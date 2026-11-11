use std::{collections::BTreeSet, ffi::OsString, io::Read};

use serde::{Deserialize, Serialize};

pub(crate) const STDIN_MARKER: &str = "--qclient-console-private-stdin-v1";
pub(crate) const ARG_PLACEHOLDER: &str = "__QCLIENT_CONSOLE_PRIVATE_STDIN_V1__";
const PROTOCOL_VERSION: u8 = 1;
const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SensitiveArgument {
    pub(crate) index: usize,
    pub(crate) value: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SensitivePayload {
    version: u8,
    arguments: Vec<SensitiveArgument>,
}

pub(crate) fn encode(arguments: Vec<SensitiveArgument>) -> Result<Vec<u8>, String> {
    let payload = serde_json::to_vec(&SensitivePayload {
        version: PROTOCOL_VERSION,
        arguments,
    })
    .map_err(|_| "cannot encode private capability input".to_owned())?;
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err("private capability input is too large".to_owned());
    }
    Ok(payload)
}

pub(crate) fn restore_process_args(
    mut args: Vec<OsString>,
    reader: impl Read,
) -> Result<Vec<OsString>, String> {
    let markers: Vec<_> = args
        .iter()
        .enumerate()
        .filter_map(|(index, argument)| (argument == STDIN_MARKER).then_some(index))
        .collect();
    if markers.is_empty() {
        return Ok(args);
    }
    if markers != [1] {
        return Err("invalid private capability input marker".to_owned());
    }
    args.remove(1);

    let mut payload = Vec::new();
    reader
        .take((MAX_PAYLOAD_BYTES + 1) as u64)
        .read_to_end(&mut payload)
        .map_err(|_| "cannot read private capability input".to_owned())?;
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err("private capability input is too large".to_owned());
    }
    let payload: SensitivePayload = serde_json::from_slice(&payload)
        .map_err(|_| "invalid private capability input".to_owned())?;
    if payload.version != PROTOCOL_VERSION || payload.arguments.is_empty() {
        return Err("invalid private capability input".to_owned());
    }
    let placeholder_count = args
        .iter()
        .filter(|argument| *argument == ARG_PLACEHOLDER)
        .count();
    if payload.arguments.len() != placeholder_count {
        return Err("invalid private capability input".to_owned());
    }

    let mut seen = BTreeSet::new();
    for argument in payload.arguments {
        if !seen.insert(argument.index) {
            return Err("invalid private capability input".to_owned());
        }
        let process_index = argument
            .index
            .checked_add(1)
            .ok_or_else(|| "invalid private capability input".to_owned())?;
        let destination = args
            .get_mut(process_index)
            .ok_or_else(|| "invalid private capability input".to_owned())?;
        if destination != ARG_PLACEHOLDER {
            return Err("invalid private capability input".to_owned());
        }
        *destination = OsString::from(argument.value);
    }
    if args.iter().any(|argument| argument == ARG_PLACEHOLDER) {
        return Err("invalid private capability input".to_owned());
    }

    Ok(args)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn restores_multiple_utf8_values_without_putting_them_in_process_arguments() {
        let first = "private-päss-🔐";
        let second = "ключ-δεύτερο";
        let payload = encode(vec![
            SensitiveArgument {
                index: 2,
                value: first.into(),
            },
            SensitiveArgument {
                index: 4,
                value: second.into(),
            },
        ])
        .unwrap();
        let process_args = vec![
            "qclient-console".into(),
            STDIN_MARKER.into(),
            "--signature-check=false".into(),
            "key".into(),
            ARG_PLACEHOLDER.into(),
            "--another".into(),
            ARG_PLACEHOLDER.into(),
        ];
        assert!(!process_args
            .iter()
            .any(|argument: &OsString| { argument == first || argument == second }));

        let restored = restore_process_args(process_args, Cursor::new(payload)).unwrap();

        assert_eq!(restored[3], first);
        assert_eq!(restored[5], second);
        assert!(!restored.iter().any(|argument| argument == STDIN_MARKER));
    }

    #[test]
    fn malformed_protocol_never_echoes_private_input() {
        let private_value = "do-not-echo-private";
        let args = vec!["qclient-console".into(), STDIN_MARKER.into()];

        let error = restore_process_args(args, Cursor::new(format!("{{malformed:{private_value}")))
            .unwrap_err();

        assert!(!error.contains(private_value));
        assert_eq!(error, "invalid private capability input");
    }

    #[test]
    fn protocol_rejects_duplicate_or_non_placeholder_destinations() {
        let private_value = "never-in-error";
        let payload = encode(vec![
            SensitiveArgument {
                index: 1,
                value: private_value.into(),
            },
            SensitiveArgument {
                index: 1,
                value: private_value.into(),
            },
        ])
        .unwrap();
        let args = vec![
            "qclient-console".into(),
            STDIN_MARKER.into(),
            "--signature-check=false".into(),
            ARG_PLACEHOLDER.into(),
        ];

        let result = restore_process_args(args, Cursor::new(payload));
        assert!(result.is_err(), "duplicate destinations must fail closed");
        let error = result.unwrap_err();

        assert_eq!(error, "invalid private capability input");
        assert!(!error.contains(private_value));
    }

    #[test]
    fn protocol_rejects_unmapped_placeholders_without_echoing_private_input() {
        let private_value = "only-one-private-value-private";
        let payload = encode(vec![SensitiveArgument {
            index: 1,
            value: private_value.into(),
        }])
        .unwrap();
        let args = vec![
            "qclient-console".into(),
            STDIN_MARKER.into(),
            "--signature-check=false".into(),
            ARG_PLACEHOLDER.into(),
            ARG_PLACEHOLDER.into(),
        ];

        let result = restore_process_args(args, Cursor::new(payload));
        assert!(result.is_err(), "partial payload must fail closed");
        let error = result.unwrap_err();

        assert_eq!(error, "invalid private capability input");
        assert!(!error.contains(private_value));
    }

    #[test]
    fn protocol_rejects_empty_argument_payload_for_a_placeholder() {
        let payload = encode(Vec::new()).unwrap();
        let args = vec![
            "qclient-console".into(),
            STDIN_MARKER.into(),
            "--signature-check=false".into(),
            ARG_PLACEHOLDER.into(),
        ];

        let error = restore_process_args(args, Cursor::new(payload)).unwrap_err();

        assert_eq!(error, "invalid private capability input");
    }
}
