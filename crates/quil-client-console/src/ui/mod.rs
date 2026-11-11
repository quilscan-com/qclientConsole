mod actions;
mod alias_actions;
mod alias_edit;
mod aliases;
mod archive_shards;
mod capabilities;
pub mod chrome;
mod confidential_wallet;
mod config_path;
mod configs;
mod confirm;
mod detail;
mod diagnostics;
mod diagnostics_identity;
mod health;
mod help;
mod home;
mod hypergraph;
mod key_actions;
mod key_create;
mod keys;
mod legacy_assets;
mod log_export;
mod logs;
mod node_status;
mod other;
mod peer_resolve;
mod pending_escrows;
pub(crate) mod prover;
mod remote_health;
mod result;
mod rewards;
mod runtime;
mod section;
pub(crate) mod shards;
pub mod text;
pub mod theme;
mod wallet_actions;
mod wallet_operations;

use ratatui::Frame;

use crate::app::{App, Page};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    match app.page {
        Page::Home => home::draw(frame, app),
        Page::NodeStatus => node_status::draw(frame, app),
        Page::Section(section) => section::draw(frame, app, section),
        Page::Configs => configs::draw(frame, app),
        Page::ConfigPath => config_path::draw(frame, app),
        Page::Runtime => runtime::draw(frame, app),
        Page::Diagnostics => diagnostics::draw(frame, app),
        Page::DiagnosticsIdentity => diagnostics_identity::draw(frame, app),
        Page::RemoteHealth => remote_health::draw(frame, app),
        Page::Health => health::draw(frame, app),
        Page::PeerResolve => peer_resolve::draw(frame, app),
        Page::ConfidentialWallet => confidential_wallet::draw(frame, app),
        Page::PendingEscrows => pending_escrows::draw(frame, app),
        Page::WalletActions
        | Page::WalletForm
        | Page::WalletConfirm
        | Page::WalletProgress
        | Page::WalletResult => wallet_actions::draw(frame, app),
        Page::TokenTransfer | Page::DeployToken => capabilities::draw(frame, app),
        Page::Capabilities
        | Page::CapabilityForm
        | Page::CapabilityConfirm
        | Page::CapabilityTask => capabilities::draw(frame, app),
        Page::Logs => logs::draw(frame, app),
        Page::LogExport => log_export::draw(frame, app),
        Page::Keys => keys::draw(frame, app),
        Page::KeyActions => key_actions::draw(frame, app),
        Page::KeyCreate => key_create::draw(frame, app),
        Page::LegacyAssets => legacy_assets::draw(frame, app),
        Page::Hypergraph => hypergraph::draw(frame, app),
        Page::Aliases => aliases::draw(frame, app),
        Page::AliasActions => alias_actions::draw(frame, app),
        Page::AliasEdit => alias_edit::draw(frame, app),
        Page::Prover => prover::draw(frame, app),
        Page::ProverActions => actions::draw(frame, app),
        Page::Shards => shards::draw(frame, app),
        Page::ArchiveShards => archive_shards::draw(frame, app),
        Page::Rewards => rewards::draw(frame, app),
        Page::RewardMintConfirm => rewards::draw_confirm(frame, app),
        Page::Confirm => confirm::draw(frame, app),
        Page::OperationResult => result::draw(frame, app),
        Page::Help => help::draw(frame, app),
        Page::Other => other::draw(frame, app),
    }
}

#[cfg(test)]
mod tests;
