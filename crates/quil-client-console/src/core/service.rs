use std::path::PathBuf;

use crate::{
    domain::{
        AliasSnapshot, ArchiveShardSnapshot, ConfidentialWalletSnapshot,
        DiagnosticsIdentitySnapshot, DiagnosticsReport, HealthSnapshot, HealthTarget,
        HypergraphKind, HypergraphSnapshot, KeySnapshot, LegacyAssetsSnapshot,
        LegacyMigrationPreflight, LegacyMigrationStatus, LogExportOptions, LogExportPreview,
        LogSnapshot, PeerAddressReport, PendingEscrowsSnapshot, RewardMintPreflight,
        RewardMintStatus, RewardSnapshot, RuntimeSnapshot, ShardInfoSnapshot, WalletActionRequest,
        WalletPreflight,
    },
    service::ServiceError,
};

use super::{ConsoleCore, OperationOrigin};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewardMintRun {
    pub config_dir: PathBuf,
    pub status: RewardMintStatus,
}

impl ConsoleCore {
    pub async fn preflight_legacy_migration(
        &self,
        request: crate::service::legacy_migration::LegacyMigrationRequest,
    ) -> Result<LegacyMigrationPreflight, ServiceError> {
        let active_config = self.active_config_path()?;
        self.service()
            .preflight_legacy_migration(&active_config, request)
            .await
    }

    pub async fn start_legacy_migration(
        &self,
        origin: OperationOrigin,
        approved: LegacyMigrationPreflight,
    ) -> Result<LegacyMigrationStatus, ServiceError> {
        let _lease = self
            .operations()
            .try_begin(origin, "legacy-migration")
            .map_err(|error| ServiceError::Conflict(error.to_string()))?;
        self.service().start_legacy_migration(approved).await
    }

    pub async fn load_legacy_migration(
        &self,
        fingerprint: &str,
    ) -> Result<Option<LegacyMigrationStatus>, ServiceError> {
        self.service().load_legacy_migration(fingerprint).await
    }

    pub async fn advance_legacy_migration(
        &self,
        origin: OperationOrigin,
        fingerprint: &str,
        source_key: Option<crate::service::legacy_migration::SecretLegacyKey>,
    ) -> Result<LegacyMigrationStatus, ServiceError> {
        let _lease = self
            .operations()
            .try_begin(origin, "legacy-migration-advance")
            .map_err(|error| ServiceError::Conflict(error.to_string()))?;
        self.service()
            .advance_legacy_migration(fingerprint, source_key)
            .await
    }

    pub async fn rebroadcast_legacy_migration(
        &self,
        origin: OperationOrigin,
        fingerprint: &str,
        confirmed: bool,
    ) -> Result<LegacyMigrationStatus, ServiceError> {
        if !confirmed {
            return Err(ServiceError::InvalidInput(
                "Legacy migration rebroadcast requires confirmation".into(),
            ));
        }
        let _lease = self
            .operations()
            .try_begin(origin, "legacy-migration-rebroadcast")
            .map_err(|error| ServiceError::Conflict(error.to_string()))?;
        self.service()
            .rebroadcast_legacy_migration(fingerprint, true)
            .await
    }

    pub async fn diagnostics_dial(
        &self,
        address: String,
    ) -> Result<DiagnosticsReport, ServiceError> {
        self.service().diagnostics_dial(&address).await
    }

    pub async fn inspect_diagnostics_identity(
        &self,
        path: PathBuf,
    ) -> Result<DiagnosticsIdentitySnapshot, ServiceError> {
        self.service().inspect_diagnostics_identity(&path).await
    }

    pub async fn runtime_status(&self) -> Result<RuntimeSnapshot, ServiceError> {
        self.service().runtime_status().await
    }

    pub async fn resolve_peer(&self, peer_id: String) -> Result<PeerAddressReport, ServiceError> {
        let config_dir = self.active_config_path()?;
        self.service().resolve_peer(&config_dir, &peer_id).await
    }

    pub async fn health_report(
        &self,
        target: HealthTarget,
    ) -> Result<HealthSnapshot, ServiceError> {
        let config_dir = match self.active_config_path() {
            Ok(path) => path,
            Err(_)
                if matches!(
                    target,
                    HealthTarget::Peer {
                        identity_path: Some(_),
                        ..
                    }
                ) =>
            {
                PathBuf::new()
            }
            Err(error) => return Err(error),
        };
        self.service().health_report(&config_dir, &target).await
    }

    pub async fn read_logs(&self, core_id: u32, lines: usize) -> Result<LogSnapshot, ServiceError> {
        let config_dir = self.active_config_path()?;
        self.service().read_logs(&config_dir, core_id, lines).await
    }

    pub async fn prepare_log_export(
        &self,
        options: LogExportOptions,
    ) -> Result<LogExportPreview, ServiceError> {
        let config_dir = self.active_config_path()?;
        self.service()
            .prepare_log_export(&config_dir, options)
            .await
    }

    pub async fn list_keys(&self) -> Result<Vec<KeySnapshot>, ServiceError> {
        let config_dir = self.active_config_path()?;
        self.service().list_keys(&config_dir).await
    }

    pub async fn list_aliases(&self) -> Result<Vec<AliasSnapshot>, ServiceError> {
        let config_dir = self.active_config_path()?;
        self.service().list_aliases(&config_dir).await
    }

    pub async fn reward_summary(&self) -> Result<RewardSnapshot, ServiceError> {
        let config_dir = self.active_config_path()?;
        self.service().reward_summary(&config_dir).await
    }

    pub async fn reward_mint_preflight(&self) -> Result<RewardMintPreflight, ServiceError> {
        let config_dir = self.active_config_path()?;
        self.service().reward_mint_preflight(&config_dir).await
    }

    pub async fn mint_rewards(
        &self,
        origin: OperationOrigin,
    ) -> Result<RewardMintRun, ServiceError> {
        let _lease = self
            .operations()
            .try_begin(origin, "reward-mint")
            .map_err(|error| ServiceError::Conflict(error.to_string()))?;
        let config_dir = self.active_config_path()?;
        if let Some(status) = self.service().load_reward_mint(&config_dir).await? {
            return Ok(RewardMintRun { config_dir, status });
        }
        let approved = self.service().reward_mint_preflight(&config_dir).await?;
        let config_dir = approved.config_dir.clone();
        let status = self
            .service()
            .start_reward_mint(&config_dir, approved)
            .await?;
        Ok(RewardMintRun { config_dir, status })
    }

    pub async fn load_reward_mint(&self) -> Result<Option<RewardMintStatus>, ServiceError> {
        let config_dir = self.active_config_path()?;
        self.service().load_reward_mint(&config_dir).await
    }

    pub async fn advance_reward_mint(&self) -> Result<RewardMintStatus, ServiceError> {
        let config_dir = self.active_config_path()?;
        self.advance_reward_mint_at(&config_dir).await
    }

    pub async fn load_reward_mint_run(&self) -> Result<Option<RewardMintRun>, ServiceError> {
        let config_dir = self.active_config_path()?;
        Ok(self
            .service()
            .load_reward_mint(&config_dir)
            .await?
            .map(|status| RewardMintRun { config_dir, status }))
    }

    pub async fn advance_reward_mint_at(
        &self,
        config_dir: &std::path::Path,
    ) -> Result<RewardMintStatus, ServiceError> {
        self.service().advance_reward_mint(config_dir).await
    }

    pub async fn rebroadcast_reward_mint(
        &self,
        origin: OperationOrigin,
        confirmed: bool,
    ) -> Result<RewardMintRun, ServiceError> {
        let _lease = self
            .operations()
            .try_begin(origin, "reward-mint-rebroadcast")
            .map_err(|error| ServiceError::Conflict(error.to_string()))?;
        let config_dir = self.active_config_path()?;
        let status = self
            .service()
            .rebroadcast_reward_mint(&config_dir, confirmed)
            .await?;
        Ok(RewardMintRun { config_dir, status })
    }

    pub async fn preflight_wallet_action(
        &self,
        request: WalletActionRequest,
    ) -> Result<WalletPreflight, ServiceError> {
        let config_dir = self.active_config_path()?;
        self.service()
            .preflight_wallet_action(&config_dir, request)
            .await
    }

    pub async fn legacy_assets(&self) -> Result<LegacyAssetsSnapshot, ServiceError> {
        let config_dir = self.active_config_path()?;
        self.service().legacy_assets(&config_dir).await
    }

    pub async fn confidential_wallet(&self) -> Result<ConfidentialWalletSnapshot, ServiceError> {
        let config_dir = self.active_config_path()?;
        self.service().confidential_wallet(&config_dir).await
    }

    pub async fn pending_escrows(&self) -> Result<PendingEscrowsSnapshot, ServiceError> {
        let config_dir = self.active_config_path()?;
        self.service().pending_escrows(&config_dir).await
    }

    pub async fn query_hypergraph(
        &self,
        kind: HypergraphKind,
        address: String,
    ) -> Result<HypergraphSnapshot, ServiceError> {
        let config_dir = self.active_config_path()?;
        self.service()
            .query_hypergraph(&config_dir, kind, &address)
            .await
    }

    pub async fn shard_info(&self, include_all: bool) -> Result<ShardInfoSnapshot, ServiceError> {
        let config_dir = self.active_config_path()?;
        self.service().shard_info(&config_dir, include_all).await
    }

    pub async fn archive_shards(&self) -> Result<ArchiveShardSnapshot, ServiceError> {
        let config_dir = self.active_config_path()?;
        self.service().archive_shards(&config_dir).await
    }
}
