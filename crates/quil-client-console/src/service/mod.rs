mod aliases;
mod archive_shards;
mod diagnostics;
mod diagnostics_identity;
mod explorer;
mod health;
mod hypergraph;
mod keys;
mod legacy_assets;
pub mod legacy_migration;
mod logs;
mod node;
mod private_file;
mod prover;
pub(crate) mod read_rpc;
mod rewards;
mod runtime;
mod shards;
mod signing;
mod wallet;

use std::path::Path;

use async_trait::async_trait;

use crate::domain::{
    AliasMutation, AliasSnapshot, ArchiveShardSnapshot, ConfidentialWalletSnapshot,
    DiagnosticsIdentitySnapshot, DiagnosticsReport, HealthSnapshot, HealthTarget, HypergraphKind,
    HypergraphSnapshot, KeyMutation, KeySnapshot, LegacyAssetsSnapshot, LegacyMigrationPreflight,
    LegacyMigrationStatus, LogExportOptions, LogExportPreview, LogSnapshot, NodeSnapshot,
    OperationResult, PeerAddressReport, PendingEscrowsSnapshot, ProverAction, RewardMintPreflight,
    RewardMintStatus, RewardSnapshot, RuntimeSnapshot, ShardInfoSnapshot, WalletActionRequest,
    WalletOperationResult, WalletPreflight,
};

pub use node::NodeConsoleService;

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    #[error("Operation conflict: {0}")]
    Conflict(String),
    #[error("Cannot read node configuration: {0}")]
    Config(String),
    #[error("Cannot connect to node: {0}")]
    Connect(String),
    #[error("Node status query failed: {0}")]
    Rpc(String),
    #[error("Node status query timed out")]
    Timeout,
    #[error("Cannot read node key: {0}")]
    Key(String),
    #[error("Operation submission failed: {0}")]
    Operation(String),
    #[error("Network diagnostics failed: {0}")]
    Diagnostics(String),
    #[error("Diagnostic identity operation failed: {0}")]
    DiagnosticsIdentity(String),
    #[error("Cannot read node health report: {0}")]
    Health(String),
    #[error("Cannot read node logs: {0}")]
    Logs(String),
    #[error("Cannot export node logs: {0}")]
    LogExport(String),
    #[error("Cannot inspect runtime environment: {0}")]
    Runtime(String),
    #[error("Address alias operation failed: {0}")]
    Aliases(String),
    #[error("Cannot read rewards: {0}")]
    Rewards(String),
    #[error("Cannot read legacy assets: {0}")]
    LegacyAssets(String),
    #[error("Cannot read Hypergraph: {0}")]
    Hypergraph(String),
    #[error("Cannot read shard information: {0}")]
    Shards(String),
    #[error("Cannot read archive shards: {0}")]
    ArchiveShards(String),
    #[error("Cannot read Confidential wallet: {0}")]
    Wallet(String),
}

#[async_trait]
pub trait ConsoleService: Send + Sync {
    async fn snapshot(
        &self,
        config_name: &str,
        config_dir: &Path,
    ) -> Result<NodeSnapshot, ServiceError>;

    async fn execute_prover(
        &self,
        config_dir: &Path,
        action: ProverAction,
    ) -> Result<OperationResult, ServiceError>;

    async fn diagnostics_dial(&self, address: &str) -> Result<DiagnosticsReport, ServiceError>;

    async fn inspect_diagnostics_identity(
        &self,
        path: &Path,
    ) -> Result<DiagnosticsIdentitySnapshot, ServiceError>;

    async fn create_diagnostics_identity(
        &self,
        path: &Path,
    ) -> Result<DiagnosticsIdentitySnapshot, ServiceError>;

    async fn resolve_peer(
        &self,
        config_dir: &Path,
        peer_id: &str,
    ) -> Result<PeerAddressReport, ServiceError>;

    async fn health_report(
        &self,
        config_dir: &Path,
        target: &HealthTarget,
    ) -> Result<HealthSnapshot, ServiceError>;

    async fn read_logs(
        &self,
        config_dir: &Path,
        core_id: u32,
        lines: usize,
    ) -> Result<LogSnapshot, ServiceError>;

    async fn prepare_log_export(
        &self,
        config_dir: &Path,
        options: LogExportOptions,
    ) -> Result<LogExportPreview, ServiceError>;

    async fn finish_log_export(
        &self,
        id: u64,
        commit: bool,
    ) -> Result<OperationResult, ServiceError>;

    async fn runtime_status(&self) -> Result<RuntimeSnapshot, ServiceError>;

    async fn list_keys(&self, config_dir: &Path) -> Result<Vec<KeySnapshot>, ServiceError>;

    async fn mutate_key(
        &self,
        config_dir: &Path,
        mutation: KeyMutation,
    ) -> Result<OperationResult, ServiceError>;

    async fn list_aliases(&self, config_dir: &Path) -> Result<Vec<AliasSnapshot>, ServiceError>;

    async fn mutate_alias(
        &self,
        config_dir: &Path,
        mutation: AliasMutation,
    ) -> Result<OperationResult, ServiceError>;

    async fn reward_summary(&self, config_dir: &Path) -> Result<RewardSnapshot, ServiceError>;

    async fn reward_mint_preflight(
        &self,
        _config_dir: &Path,
    ) -> Result<RewardMintPreflight, ServiceError> {
        Err(ServiceError::Operation(
            "Reward Mint is not available in this service".into(),
        ))
    }

    async fn start_reward_mint(
        &self,
        _config_dir: &Path,
        _approved: RewardMintPreflight,
    ) -> Result<RewardMintStatus, ServiceError> {
        Err(ServiceError::Operation(
            "Reward Mint is not available in this service".into(),
        ))
    }

    async fn load_reward_mint(
        &self,
        _config_dir: &Path,
    ) -> Result<Option<RewardMintStatus>, ServiceError> {
        Ok(None)
    }

    async fn advance_reward_mint(
        &self,
        _config_dir: &Path,
    ) -> Result<RewardMintStatus, ServiceError> {
        Err(ServiceError::Operation(
            "Reward Mint is not available in this service".into(),
        ))
    }

    async fn rebroadcast_reward_mint(
        &self,
        _config_dir: &Path,
        _confirmed: bool,
    ) -> Result<RewardMintStatus, ServiceError> {
        Err(ServiceError::Operation(
            "Reward Mint recovery is not available in this service".into(),
        ))
    }

    async fn preflight_legacy_migration(
        &self,
        _active_config: &Path,
        _request: legacy_migration::LegacyMigrationRequest,
    ) -> Result<LegacyMigrationPreflight, ServiceError> {
        Err(ServiceError::Operation(
            "Legacy migration is not available in this service".into(),
        ))
    }

    async fn start_legacy_migration(
        &self,
        _approved: LegacyMigrationPreflight,
    ) -> Result<LegacyMigrationStatus, ServiceError> {
        Err(ServiceError::Operation(
            "Legacy migration is not available in this service".into(),
        ))
    }

    async fn load_legacy_migration(
        &self,
        _fingerprint: &str,
    ) -> Result<Option<LegacyMigrationStatus>, ServiceError> {
        Ok(None)
    }

    async fn advance_legacy_migration(
        &self,
        _fingerprint: &str,
        _source_key: Option<legacy_migration::SecretLegacyKey>,
    ) -> Result<LegacyMigrationStatus, ServiceError> {
        Err(ServiceError::Operation(
            "Legacy migration recovery is not available in this service".into(),
        ))
    }

    async fn rebroadcast_legacy_migration(
        &self,
        _fingerprint: &str,
        _confirmed: bool,
    ) -> Result<LegacyMigrationStatus, ServiceError> {
        Err(ServiceError::Operation(
            "Legacy migration recovery is not available in this service".into(),
        ))
    }

    async fn legacy_assets(&self, config_dir: &Path) -> Result<LegacyAssetsSnapshot, ServiceError>;

    async fn confidential_wallet(
        &self,
        config_dir: &Path,
    ) -> Result<ConfidentialWalletSnapshot, ServiceError>;

    async fn pending_escrows(
        &self,
        config_dir: &Path,
    ) -> Result<PendingEscrowsSnapshot, ServiceError>;

    async fn preflight_wallet_action(
        &self,
        _config_dir: &Path,
        _request: WalletActionRequest,
    ) -> Result<WalletPreflight, ServiceError> {
        Err(ServiceError::Operation(
            "Wallet actions are not available in this service".into(),
        ))
    }

    async fn execute_wallet_action(
        &self,
        _config_dir: &Path,
        _request: WalletActionRequest,
        _approved: WalletPreflight,
    ) -> Result<WalletOperationResult, ServiceError> {
        Err(ServiceError::Operation(
            "Wallet actions are not available in this service".into(),
        ))
    }

    async fn query_hypergraph(
        &self,
        config_dir: &Path,
        kind: HypergraphKind,
        address: &str,
    ) -> Result<HypergraphSnapshot, ServiceError>;

    async fn shard_info(
        &self,
        config_dir: &Path,
        include_all: bool,
    ) -> Result<ShardInfoSnapshot, ServiceError>;

    async fn archive_shards(&self, config_dir: &Path)
        -> Result<ArchiveShardSnapshot, ServiceError>;
}
