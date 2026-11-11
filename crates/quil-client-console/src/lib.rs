pub mod allocation;
pub mod app;
pub mod capability;
pub mod config;
pub mod console_release;
mod controller;
pub mod core;
pub mod domain;
pub mod event;
pub mod i18n;
pub mod launcher;
pub mod qclient;
pub mod service;
pub mod settings;
pub mod terminal;
pub mod ui;
pub mod web;

pub use controller::{
    advance_reward_mint_once, aliases_once, archive_shards_once, confidential_wallet_once,
    create_diagnostics_identity_once, diagnostics_once, execute_once, finish_log_export_once,
    health_once, hypergraph_once, inspect_diagnostics_identity_once, keys_once, legacy_assets_once,
    load_reward_mint_once, logs_once, mutate_alias_once, mutate_key_once, peer_resolve_once,
    pending_escrows_once, prepare_log_export_once, refresh_core_once, refresh_once,
    request_config_refresh, request_refresh, reward_mint_preflight_once, rewards_once, run,
    run_with_signature_check, runtime_once, shards_once, start_reward_mint_once,
};

#[cfg(test)]
pub(crate) use controller::map_key;
