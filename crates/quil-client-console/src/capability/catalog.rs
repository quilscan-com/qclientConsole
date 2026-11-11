use std::any::TypeId;
use std::collections::BTreeMap;

use clap::{Arg, ArgAction, Command};

use super::{
    CapabilityField, CapabilityFieldKind, CapabilityGroup, CapabilitySpec, CapabilityTarget,
};

const SENSITIVE_FIELD_IDS: &[&str] = &[
    "key_hex",
    "password",
    "private_key",
    "peer_private_key",
    "peer_priv_key",
    "secret",
];

/// The public qclient command leaves, indexed by their stable dotted command paths.
#[derive(Debug, Clone)]
pub struct CapabilityCatalog {
    items: Vec<CapabilitySpec>,
    by_id: BTreeMap<String, usize>,
}

impl CapabilityCatalog {
    pub(crate) fn empty() -> Self {
        Self {
            items: Vec::new(),
            by_id: BTreeMap::new(),
        }
    }

    pub fn discover() -> anyhow::Result<Self> {
        let mut command = crate::qclient::command();
        // Building resolves positional indices, inferred value names, and globals.
        command.build();
        let mut items = Vec::new();
        discover_leaves(&command, &[], &BTreeMap::new(), &mut items);
        for item in &mut items {
            super::policy::apply(item);
        }
        items.sort_by(|left, right| left.id.cmp(&right.id));
        let by_id = items
            .iter()
            .enumerate()
            .map(|(index, item)| (item.id.clone(), index))
            .collect();
        Ok(Self { items, by_id })
    }

    pub fn iter(&self) -> impl Iterator<Item = &CapabilitySpec> {
        self.items.iter()
    }

    pub fn get(&self, id: &str) -> Option<&CapabilitySpec> {
        self.by_id.get(id).map(|&index| &self.items[index])
    }
}

fn discover_leaves(
    command: &Command,
    path: &[String],
    inherited: &BTreeMap<String, Arg>,
    items: &mut Vec<CapabilitySpec>,
) {
    if command.is_hide_set() || command.get_name() == "help" {
        return;
    }
    let mut arguments = inherited.clone();
    for argument in command.get_arguments() {
        // Insert even hidden definitions so a closer hidden override suppresses
        // an inherited field with the same ID.
        arguments.insert(argument.get_id().to_string(), argument.clone());
    }
    if command.get_subcommands().next().is_some() {
        for child in command.get_subcommands() {
            let mut child_path = path.to_vec();
            child_path.push(child.get_name().to_owned());
            discover_leaves(child, &child_path, &arguments, items);
        }
    } else if !path.is_empty() {
        items.push(CapabilitySpec {
            id: path.join("."),
            group: CapabilityGroup::System,
            title: command
                .get_display_name()
                .unwrap_or(command.get_name())
                .to_owned(),
            description: command
                .get_long_about()
                .or_else(|| command.get_about())
                .map(ToString::to_string)
                .unwrap_or_default(),
            read_only: false,
            confirmation: None,
            fields: arguments.values().filter_map(field_from_arg).collect(),
            target: CapabilityTarget::EmbeddedCli {
                command_path: path.to_vec(),
            },
        });
    }
}

fn field_from_arg(argument: &Arg) -> Option<CapabilityField> {
    let id = argument.get_id().as_str();
    if argument.is_hide_set()
        || matches!(
            id,
            "network" | "yes" | "signature-check" | "_signature_check"
        )
        || matches!(
            argument.get_action(),
            ArgAction::Help | ArgAction::HelpShort | ArgAction::HelpLong | ArgAction::Version
        )
    {
        return None;
    }
    let parser_type = argument.get_value_parser().type_id();
    let is_unsigned_integer = parser_type == TypeId::of::<u8>()
        || parser_type == TypeId::of::<u16>()
        || parser_type == TypeId::of::<u32>()
        || parser_type == TypeId::of::<u64>()
        || parser_type == TypeId::of::<u128>()
        || parser_type == TypeId::of::<usize>();
    let kind = if matches!(
        argument.get_action(),
        ArgAction::SetTrue | ArgAction::SetFalse | ArgAction::Count
    ) {
        CapabilityFieldKind::Boolean
    } else if is_unsigned_integer {
        CapabilityFieldKind::Integer
    } else if argument.get_value_names().is_some_and(|names| {
        names.iter().any(|name| {
            let name = name.as_str().to_ascii_uppercase();
            ["PATH", "FILE", "DIR"]
                .iter()
                .any(|part| name.contains(part))
        })
    }) {
        CapabilityFieldKind::Path
    } else {
        CapabilityFieldKind::Text
    };
    Some(CapabilityField {
        id: id.to_owned(),
        label: argument.get_long().unwrap_or(id).replace(['-', '_'], " "),
        help: argument
            .get_long_help()
            .or_else(|| argument.get_help())
            .map(ToString::to_string)
            .unwrap_or_default(),
        kind,
        required: argument.is_required_set(),
        repeated: matches!(argument.get_action(), ArgAction::Append | ArgAction::Count)
            || argument
                .get_num_args()
                .is_some_and(|range| range.max_values() > 1),
        sensitive: SENSITIVE_FIELD_IDS.contains(&id)
            || id.ends_with("_password")
            || id.ends_with("_private_key")
            || id.ends_with("_priv_key")
            || id.ends_with("_secret"),
        positional: argument.get_index(),
        long: argument.get_long().map(str::to_owned),
        short: argument.get_short(),
        default_values: argument
            .get_default_values()
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn discover_with_override(nearer: Arg) -> Vec<CapabilitySpec> {
        let mut command = Command::new("root")
            .arg(
                Arg::new("config")
                    .long("config")
                    .help("Inherited config")
                    .default_value("parent"),
            )
            .subcommand(Command::new("leaf").arg(nearer));
        command.build();
        let mut items = Vec::new();
        discover_leaves(&command, &[], &BTreeMap::new(), &mut items);
        items
    }

    #[test]
    fn nearer_argument_replaces_inherited_definition() {
        let items = discover_with_override(
            Arg::new("config")
                .long("leaf-config")
                .help("Leaf config")
                .default_value("leaf"),
        );
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].fields.len(), 1);
        let field = &items[0].fields[0];
        assert_eq!(field.id, "config");
        assert_eq!(field.long.as_deref(), Some("leaf-config"));
        assert_eq!(field.help, "Leaf config");
        assert_eq!(field.default_values, ["leaf"]);
    }

    #[test]
    fn hidden_nearer_argument_suppresses_inherited_visible_field() {
        let items = discover_with_override(Arg::new("config").long("leaf-config").hide(true));
        assert_eq!(items.len(), 1);
        assert!(items[0].fields.is_empty());
    }

    #[test]
    fn unsigned_parser_types_are_integer_fields() {
        let arguments = [
            Arg::new("u8")
                .long("u8")
                .value_parser(clap::value_parser!(u8)),
            Arg::new("u16")
                .long("u16")
                .value_parser(clap::value_parser!(u16)),
            Arg::new("u32")
                .long("u32")
                .value_parser(clap::value_parser!(u32)),
            Arg::new("u64")
                .long("u64")
                .value_parser(clap::value_parser!(u64)),
            Arg::new("u128")
                .long("u128")
                .value_parser(clap::value_parser!(u128)),
            Arg::new("usize")
                .long("usize")
                .value_parser(clap::value_parser!(usize)),
        ];

        for argument in arguments {
            assert_eq!(
                field_from_arg(&argument).unwrap().kind,
                CapabilityFieldKind::Integer,
                "{}",
                argument.get_id()
            );
        }
    }

    #[test]
    fn other_typed_parsers_are_not_integer_fields() {
        let arguments = [
            Arg::new("signed")
                .long("signed")
                .value_parser(clap::value_parser!(i64)),
            Arg::new("float")
                .long("float")
                .value_parser(clap::value_parser!(f64)),
            Arg::new("ip")
                .long("ip")
                .value_parser(clap::value_parser!(std::net::IpAddr)),
        ];

        for argument in arguments {
            assert_eq!(
                field_from_arg(&argument).unwrap().kind,
                CapabilityFieldKind::Text,
                "{}",
                argument.get_id()
            );
        }
    }
}
