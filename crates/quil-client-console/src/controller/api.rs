use std::{path::PathBuf, sync::Arc};

use crate::{
    app::App,
    domain,
    service::{ConsoleService, ServiceError},
};

pub async fn refresh_once(
    service: Arc<dyn ConsoleService>,
    config_name: String,
    config_dir: PathBuf,
) -> Result<domain::NodeSnapshot, ServiceError> {
    service.snapshot(&config_name, &config_dir).await
}

pub async fn refresh_core_once(
    core: crate::core::ConsoleCore,
) -> Result<domain::NodeSnapshot, ServiceError> {
    core.refresh().await
}

pub async fn execute_once(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    action: domain::ProverAction,
) -> Result<domain::OperationResult, ServiceError> {
    service.execute_prover(&config_dir, action).await
}

pub async fn diagnostics_once(
    service: Arc<dyn ConsoleService>,
    address: String,
) -> Result<domain::DiagnosticsReport, ServiceError> {
    service.diagnostics_dial(&address).await
}

pub async fn inspect_diagnostics_identity_once(
    service: Arc<dyn ConsoleService>,
    path: PathBuf,
) -> Result<domain::DiagnosticsIdentitySnapshot, ServiceError> {
    service.inspect_diagnostics_identity(&path).await
}

pub async fn create_diagnostics_identity_once(
    service: Arc<dyn ConsoleService>,
    path: PathBuf,
) -> Result<domain::DiagnosticsIdentitySnapshot, ServiceError> {
    service.create_diagnostics_identity(&path).await
}

pub async fn peer_resolve_once(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    peer_id: String,
) -> Result<domain::PeerAddressReport, ServiceError> {
    service.resolve_peer(&config_dir, &peer_id).await
}

pub async fn health_once(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    target: domain::HealthTarget,
) -> Result<domain::HealthSnapshot, ServiceError> {
    service.health_report(&config_dir, &target).await
}

pub async fn runtime_once(
    service: Arc<dyn ConsoleService>,
) -> Result<domain::RuntimeSnapshot, ServiceError> {
    service.runtime_status().await
}

pub async fn logs_once(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    core_id: u32,
    lines: usize,
) -> Result<domain::LogSnapshot, ServiceError> {
    service.read_logs(&config_dir, core_id, lines).await
}

pub async fn prepare_log_export_once(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    options: domain::LogExportOptions,
) -> Result<domain::LogExportPreview, ServiceError> {
    service.prepare_log_export(&config_dir, options).await
}

pub async fn finish_log_export_once(
    service: Arc<dyn ConsoleService>,
    id: u64,
    commit: bool,
) -> Result<domain::OperationResult, ServiceError> {
    service.finish_log_export(id, commit).await
}

pub async fn keys_once(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
) -> Result<Vec<domain::KeySnapshot>, ServiceError> {
    service.list_keys(&config_dir).await
}

pub async fn mutate_key_once(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    mutation: domain::KeyMutation,
) -> Result<domain::OperationResult, ServiceError> {
    service.mutate_key(&config_dir, mutation).await
}

pub async fn aliases_once(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
) -> Result<Vec<domain::AliasSnapshot>, ServiceError> {
    service.list_aliases(&config_dir).await
}

pub async fn rewards_once(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
) -> Result<domain::RewardSnapshot, ServiceError> {
    service.reward_summary(&config_dir).await
}

pub async fn reward_mint_preflight_once(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
) -> Result<domain::RewardMintPreflight, ServiceError> {
    service.reward_mint_preflight(&config_dir).await
}

pub async fn start_reward_mint_once(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    approved: domain::RewardMintPreflight,
) -> Result<domain::RewardMintStatus, ServiceError> {
    service.start_reward_mint(&config_dir, approved).await
}

pub async fn load_reward_mint_once(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
) -> Result<Option<domain::RewardMintStatus>, ServiceError> {
    service.load_reward_mint(&config_dir).await
}

pub async fn advance_reward_mint_once(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
) -> Result<domain::RewardMintStatus, ServiceError> {
    service.advance_reward_mint(&config_dir).await
}

pub async fn rebroadcast_reward_mint_once(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
) -> Result<domain::RewardMintStatus, ServiceError> {
    service.rebroadcast_reward_mint(&config_dir, true).await
}

pub async fn legacy_assets_once(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
) -> Result<domain::LegacyAssetsSnapshot, ServiceError> {
    service.legacy_assets(&config_dir).await
}

pub async fn confidential_wallet_once(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
) -> Result<domain::ConfidentialWalletSnapshot, ServiceError> {
    service.confidential_wallet(&config_dir).await
}

pub async fn pending_escrows_once(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
) -> Result<domain::PendingEscrowsSnapshot, ServiceError> {
    service.pending_escrows(&config_dir).await
}

pub async fn preflight_wallet_action_once(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    request: domain::WalletActionRequest,
) -> Result<domain::WalletPreflight, ServiceError> {
    service.preflight_wallet_action(&config_dir, request).await
}

pub async fn execute_wallet_action_once(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    request: domain::WalletActionRequest,
    approved: domain::WalletPreflight,
) -> Result<domain::WalletOperationResult, ServiceError> {
    service
        .execute_wallet_action(&config_dir, request, approved)
        .await
}

pub async fn hypergraph_once(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    kind: domain::HypergraphKind,
    address: String,
) -> Result<domain::HypergraphSnapshot, ServiceError> {
    service.query_hypergraph(&config_dir, kind, &address).await
}

pub async fn shards_once(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    include_all: bool,
) -> Result<domain::ShardInfoSnapshot, ServiceError> {
    service.shard_info(&config_dir, include_all).await
}

pub async fn archive_shards_once(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
) -> Result<domain::ArchiveShardSnapshot, ServiceError> {
    service.archive_shards(&config_dir).await
}

pub async fn mutate_alias_once(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    mutation: domain::AliasMutation,
) -> Result<domain::OperationResult, ServiceError> {
    service.mutate_alias(&config_dir, mutation).await
}

pub fn request_refresh(app: &mut App, in_flight: &mut bool) -> Option<u64> {
    app.cancel_pending_prover_confirmation_for_refresh();
    if *in_flight {
        return None;
    }
    app.begin_node_refresh();
    *in_flight = true;
    Some(app.refresh_generation)
}

pub fn request_config_refresh(app: &mut App, in_flight: &mut bool) -> Option<u64> {
    if *in_flight {
        return None;
    }
    request_refresh(app, in_flight)
}
