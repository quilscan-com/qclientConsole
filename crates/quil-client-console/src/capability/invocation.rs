use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use anyhow::bail;

use super::protocol::{self, SensitiveArgument, ARG_PLACEHOLDER, STDIN_MARKER};
use super::{CapabilityField, CapabilityFieldKind, CapabilitySpec, CapabilityTarget};

pub type CapabilityValues = BTreeMap<String, Vec<String>>;

#[derive(Clone, PartialEq, Eq)]
pub struct CapabilityInvocation {
    pub capability_id: String,
    pub argv: Vec<String>,
    pub read_only: bool,
    sensitive_arguments: Vec<SensitiveArgument>,
}

impl CapabilityInvocation {
    pub fn new(capability_id: impl Into<String>, argv: Vec<String>, read_only: bool) -> Self {
        Self {
            capability_id: capability_id.into(),
            argv,
            read_only,
            sensitive_arguments: Vec::new(),
        }
    }

    pub(crate) fn child_process(
        &self,
        signature_check: bool,
    ) -> Result<(Vec<String>, Option<Vec<u8>>), String> {
        let mut argv = vec![format!("--signature-check={signature_check}")];
        argv.extend(self.argv.iter().cloned());
        if self.sensitive_arguments.is_empty() {
            return Ok((argv, None));
        }

        let arguments = self
            .sensitive_arguments
            .iter()
            .map(|argument| SensitiveArgument {
                // The signature option is the first child argument after the
                // protocol marker has been removed.
                index: argument.index + 1,
                value: argument.value.clone(),
            })
            .collect();
        let payload = protocol::encode(arguments)?;
        argv.insert(0, STDIN_MARKER.to_owned());
        Ok((argv, Some(payload)))
    }
}

impl fmt::Debug for CapabilityInvocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CapabilityInvocation")
            .field("capability_id", &self.capability_id)
            .field("argv", &self.argv)
            .field("read_only", &self.read_only)
            .field("sensitive_argument_count", &self.sensitive_arguments.len())
            .finish()
    }
}

pub fn build_invocation(
    spec: &CapabilitySpec,
    active_config: &Path,
    values: &CapabilityValues,
    confirmed: bool,
) -> anyhow::Result<CapabilityInvocation> {
    let command_path = match &spec.target {
        CapabilityTarget::Native { .. } => {
            bail!("native capability {} cannot be invoked as argv", spec.id)
        }
        CapabilityTarget::EmbeddedCli { command_path } => command_path,
    };
    if active_config.as_os_str().is_empty() {
        bail!("active config path must not be empty");
    }
    let active_config = active_config
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("active config path must be valid UTF-8"))?;

    for field_id in values.keys() {
        if !spec.fields.iter().any(|field| field.id == *field_id) {
            bail!("unknown field {field_id} for capability {}", spec.id);
        }
    }

    if !spec.read_only && !confirmed {
        bail!("confirmation is required for capability {}", spec.id);
    }

    let one_shot_manage = matches!(spec.id.as_str(), "node.manage" | "node.prover.manage");
    if one_shot_manage
        && values.get("render").is_some_and(|values| {
            values
                .iter()
                .any(|value| !value.trim().is_empty() && value != "true")
        })
    {
        bail!(
            "capability {} only supports one-shot --render mode; use the native Node or Prover page for interactive management",
            spec.id
        );
    }

    for field in &spec.fields {
        validate_field(field, values.get(&field.id))?;
    }

    let mut argv = vec!["--network".into(), active_config.into()];
    let mut sensitive_arguments = Vec::new();
    if !spec.read_only {
        argv.push("--yes".into());
    }
    argv.extend(command_path.iter().cloned());

    let render_supplied = values
        .get("render")
        .is_some_and(|values| values.iter().any(|value| value == "true"));
    if one_shot_manage && !render_supplied {
        argv.push("--render".into());
    }

    for field in spec
        .fields
        .iter()
        .filter(|field| field.positional.is_none())
    {
        if one_shot_manage
            && field.id == "observe"
            && values.get("observe").is_none_or(|values| {
                values
                    .iter()
                    .all(|value| value.trim().is_empty() || value == "0")
            })
        {
            continue;
        }
        append_named_field(
            &mut argv,
            &mut sensitive_arguments,
            field,
            values.get(&field.id),
        )?;
    }

    let mut positional_fields: Vec<_> = spec
        .fields
        .iter()
        .filter(|field| field.positional.is_some())
        .collect();
    positional_fields.sort_by_key(|field| field.positional);
    if positional_fields.iter().any(|field| {
        values
            .get(&field.id)
            .is_some_and(|values| values.iter().any(|value| !value.trim().is_empty()))
    }) {
        argv.push("--".into());
    }
    for field in positional_fields {
        append_positional_field(
            &mut argv,
            &mut sensitive_arguments,
            field,
            values.get(&field.id),
        );
    }

    Ok(CapabilityInvocation {
        capability_id: spec.id.clone(),
        argv,
        read_only: spec.read_only,
        sensitive_arguments,
    })
}

fn validate_field(field: &CapabilityField, supplied: Option<&Vec<String>>) -> anyhow::Result<()> {
    let supplied = supplied.map(Vec::as_slice).unwrap_or_default();
    if field.required
        && (supplied.is_empty() || supplied.iter().any(|value| value.trim().is_empty()))
    {
        bail!("field {} requires a non-empty value", field.id);
    }

    for value in supplied {
        match field.kind {
            CapabilityFieldKind::Boolean if !matches!(value.as_str(), "true" | "false") => {
                bail!("field {} must be true or false", field.id)
            }
            CapabilityFieldKind::Integer if value.trim().is_empty() => {}
            CapabilityFieldKind::Integer if value.parse::<u128>().is_err() => {
                bail!("field {} must be an unsigned integer", field.id)
            }
            _ => {}
        }
    }

    let present: Vec<_> = supplied
        .iter()
        .filter(|value| !value.trim().is_empty())
        .collect();
    if !field.repeated && present.len() > 1 {
        bail!("field {} does not accept repeated values", field.id);
    }
    Ok(())
}

fn append_named_field(
    argv: &mut Vec<String>,
    sensitive_arguments: &mut Vec<SensitiveArgument>,
    field: &CapabilityField,
    supplied: Option<&Vec<String>>,
) -> anyhow::Result<()> {
    let Some(long) = field.long.as_deref() else {
        if supplied.is_some_and(|values| values.iter().any(|value| !value.trim().is_empty())) {
            bail!("field {} has no long option", field.id);
        }
        return Ok(());
    };
    let flag = format!("--{long}");

    for value in supplied
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|value| !value.trim().is_empty())
    {
        if field.kind == CapabilityFieldKind::Boolean {
            if value == "true" {
                argv.push(flag.clone());
            }
        } else {
            argv.push(flag.clone());
            append_value(argv, sensitive_arguments, field.sensitive, value);
        }
    }
    Ok(())
}

fn append_positional_field(
    argv: &mut Vec<String>,
    sensitive_arguments: &mut Vec<SensitiveArgument>,
    field: &CapabilityField,
    supplied: Option<&Vec<String>>,
) {
    for value in supplied
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|value| !value.trim().is_empty())
    {
        append_value(argv, sensitive_arguments, field.sensitive, value);
    }
}

fn append_value(
    argv: &mut Vec<String>,
    sensitive_arguments: &mut Vec<SensitiveArgument>,
    sensitive: bool,
    value: &str,
) {
    if sensitive {
        sensitive_arguments.push(SensitiveArgument {
            index: argv.len(),
            value: value.to_owned(),
        });
        argv.push(ARG_PLACEHOLDER.to_owned());
    } else {
        argv.push(value.to_owned());
    }
}
