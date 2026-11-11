//! Explicit Console grouping and write safety.
use super::{CapabilityGroup, CapabilitySpec};

// Everything outside this allowlist is a confirmed write, including new commands.
// diagnostics.fetch downloads a bundle; token.prefunded prunes and rewrites its
// settlement file. Both deliberately stay writes.
const READ_ONLY: &[&str] = &[
    "token.account",
    "token.info",
    "token.balance",
    "token.coins",
    "token.legacy",
    "token.confidential-address",
    "token.inclusion",
    "token.escrows",
    "token.entitlements",
    "token.payment-address",
    "token.settlement-claim",
    "node.prover.status",
    "node.prover.shards",
    "node.prover.shardinfo",
    "node.archive.shards",
    "node.diagnostics.health",
    "node.diagnostics.resolve",
    "node.diagnostics.status",
    "node.diagnostics.pending",
    "node.diagnostics.list",
    "node.doctor",
    "node.log.view",
    "node.info",
    "alias.list",
    "alias.get",
    "alias.resolve",
    "alias.find",
    "key.list",
    "hypergraph.get.vertex",
    "hypergraph.get.hyperedge",
    "message.retrieve",
    "message.show",
    "config.print",
];

pub(super) fn apply(item: &mut CapabilitySpec) {
    let id = item.id.as_str();
    if matches!(id, "node.manage" | "node.prover.manage") {
        if let Some(render) = item.fields.iter_mut().find(|field| field.id == "render") {
            render.default_values = vec!["true".into()];
        }
    }
    item.group = if id.starts_with("token.") {
        CapabilityGroup::Wallet
    } else if id.starts_with("node.prover.") || id == "node.manage" {
        CapabilityGroup::Prover
    } else if id.starts_with("node.diagnostics.")
        || id == "node.doctor"
        || id.starts_with("node.log.")
        || id.starts_with("diagnostics.")
    {
        CapabilityGroup::Diagnostics
    } else if id.starts_with("node.") {
        CapabilityGroup::Node
    } else if id.starts_with("alias.") || id.starts_with("key.") {
        CapabilityGroup::Identity
    } else if ["hypergraph.", "message.", "deploy.", "compute."]
        .iter()
        .any(|prefix| id.starts_with(prefix))
    {
        CapabilityGroup::Data
    } else {
        // config.*, update, download-signatures, link, and future top-level paths.
        CapabilityGroup::System
    };
    item.read_only = READ_ONLY.contains(&id);
    item.confirmation = (!item.read_only)
        .then(|| format!("Confirm {} on the active node configuration", item.title));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::{CapabilityGroup, CapabilityTarget};

    #[test]
    fn policy_ids_resolve_to_discovered_capabilities() {
        let catalog = crate::capability::CapabilityCatalog::discover().unwrap();
        for id in READ_ONLY {
            assert!(
                catalog.get(id).is_some(),
                "policy references missing capability {id}"
            );
        }
        for id in READ_ONLY {
            let item = catalog.get(id).unwrap();
            assert!(item.read_only, "{id}");
            assert!(item.confirmation.is_none(), "{id}");
        }
    }

    #[test]
    fn unknown_commands_default_to_system_writes_with_confirmation() {
        let mut item = CapabilitySpec {
            id: "future.inspect".into(),
            group: CapabilityGroup::Wallet,
            title: "inspect".into(),
            description: String::new(),
            read_only: true,
            confirmation: None,
            fields: vec![],
            target: CapabilityTarget::EmbeddedCli {
                command_path: vec!["future".into(), "inspect".into()],
            },
        };
        apply(&mut item);
        assert_eq!(item.group, CapabilityGroup::System);
        assert!(!item.read_only);
        assert_eq!(
            item.confirmation.as_deref(),
            Some("Confirm inspect on the active node configuration")
        );
        assert!(matches!(item.target, CapabilityTarget::EmbeddedCli { .. }));
    }
}
