use std::{path::Path, time::Duration};

use async_trait::async_trait;
use num_bigint::BigUint;
use quil_types::proto::node::{
    node_service_client::NodeServiceClient, FrameCadence, FrameWindow, GetNodeInfoRequest,
    GetWorkerInfoRequest, NodeInfoResponse, WorkerInfoResponse,
};
use tonic::transport::{Channel, Endpoint};

use super::{ConsoleService, ServiceError};
use crate::domain::{
    AllocationSnapshot, EpochSnapshot, FrameCadenceSnapshot, FrameWindowSnapshot, NodeSnapshot,
    RecoverySnapshot, SessionSealSnapshot, WorkerExecutionSnapshot, WorkerSnapshot,
};

pub(super) const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub(super) const RPC_TIMEOUT: Duration = Duration::from_secs(15);
pub(super) const MAX_MESSAGE_SIZE: usize = 100 * 1024 * 1024;

#[derive(Default)]
pub struct NodeConsoleService {
    log_exports: super::logs::LogExportStore,
    explorer: super::explorer::ExplorerDirectory,
    legacy_migrations: super::legacy_migration::LegacyMigrationCoordinator,
}

impl NodeConsoleService {
    pub fn with_home(home: std::path::PathBuf) -> Self {
        Self {
            legacy_migrations: super::legacy_migration::LegacyMigrationCoordinator::new(home),
            ..Default::default()
        }
    }
}

pub(super) fn grpc_address(multiaddr: &str) -> Result<String, ServiceError> {
    let parts = multiaddr.trim_matches('/').split('/').collect::<Vec<_>>();
    if parts.len() < 4 {
        return Err(ServiceError::Config(format!(
            "Unsupported RPC address: {multiaddr}"
        )));
    }

    let mut host = None;
    let mut port = None;
    for pair in parts.chunks_exact(2) {
        match pair[0] {
            "ip4" | "ip6" | "dns" | "dns4" | "dns6" => host = Some(pair[1]),
            "tcp" => port = Some(pair[1]),
            _ => {}
        }
    }

    let (host, port) = host
        .zip(port)
        .ok_or_else(|| ServiceError::Config(format!("Unsupported RPC address: {multiaddr}")))?;
    let host = match host {
        "0.0.0.0" => "127.0.0.1".into(),
        "::" => "[::1]".into(),
        value if value.contains(':') => format!("[{value}]"),
        value => value.into(),
    };
    Ok(format!("{host}:{port}"))
}

pub(super) async fn connect_client(
    address: &str,
) -> Result<NodeServiceClient<Channel>, ServiceError> {
    let endpoint = Endpoint::from_shared(format!("http://{address}"))
        .map_err(|error| ServiceError::Config(error.to_string()))?
        .connect_timeout(CONNECT_TIMEOUT);
    let channel = tokio::time::timeout(CONNECT_TIMEOUT, endpoint.connect())
        .await
        .map_err(|_| ServiceError::Timeout)?
        .map_err(|error| ServiceError::Connect(error.to_string()))?;
    Ok(NodeServiceClient::new(channel)
        .max_decoding_message_size(MAX_MESSAGE_SIZE)
        .max_encoding_message_size(MAX_MESSAGE_SIZE))
}

fn map_snapshot(
    config_name: &str,
    rpc_address: &str,
    info: NodeInfoResponse,
    workers: Result<WorkerInfoResponse, String>,
) -> NodeSnapshot {
    let (worker_info, worker_info_error) = match workers {
        Ok(response) => (response.worker_info, None),
        Err(error) => (Vec::new(), Some(error)),
    };
    let mut worker_snapshots = worker_info
        .into_iter()
        .map(|worker| WorkerSnapshot {
            core_id: worker.core_id,
            filter_hex: hex::encode(worker.filter),
            available_storage: worker.available_storage,
            total_storage: worker.total_storage,
            manually_managed: worker.manually_managed,
            execution: worker.execution.map(map_execution),
        })
        .collect::<Vec<_>>();
    worker_snapshots.sort_by_key(|worker| worker.core_id);

    NodeSnapshot {
        config_name: config_name.into(),
        rpc_address: rpc_address.into(),
        connected: true,
        peer_id: info.peer_id,
        peer_seniority: (!info.peer_seniority.is_empty())
            .then(|| BigUint::from_bytes_be(&info.peer_seniority).to_string()),
        version: info.version,
        patch_number: info.patch_number,
        last_received_frame: info.last_received_frame,
        last_global_head_frame: info.last_global_head_frame,
        reachable: info.reachable,
        running_workers: info.running_workers,
        allocated_workers: info.allocated_workers,
        peer_score: info.peer_score,
        epoch: (info.epoch_length_frames > 0).then_some(EpochSnapshot {
            current: info.current_epoch,
            length_frames: info.epoch_length_frames,
        }),
        frame_cadence: info.frame_cadence.map(map_frame_cadence),
        allocations: info
            .shard_allocations
            .into_iter()
            .map(|allocation| AllocationSnapshot {
                filter_hex: hex::encode(allocation.filter),
                status: allocation.status.into(),
                join_frame_number: allocation.join_frame_number,
                join_confirm_frame_number: allocation.join_confirm_frame_number,
                leave_frame_number: allocation.leave_frame_number,
                leave_confirm_frame_number: allocation.leave_confirm_frame_number,
                last_active_frame_number: allocation.last_active_frame_number,
                epoch: allocation.epoch,
            })
            .collect(),
        workers: worker_snapshots,
        worker_info_error,
    }
}

fn map_frame_cadence(value: FrameCadence) -> FrameCadenceSnapshot {
    FrameCadenceSnapshot {
        last_observed_unix_seconds: value.last_observed_unix_seconds,
        hour: value.hour.map(map_frame_window),
        day: value.day.map(map_frame_window),
    }
}

fn map_frame_window(value: FrameWindow) -> FrameWindowSnapshot {
    FrameWindowSnapshot {
        window_seconds: value.window_seconds,
        observed_seconds: value.observed_seconds,
        advanced_frames: value.advanced_frames,
    }
}

fn map_execution(value: quil_types::proto::node::WorkerExecution) -> WorkerExecutionSnapshot {
    WorkerExecutionSnapshot {
        state: value.state,
        blocker: non_empty(value.blocker),
        materialized_frame: value.materialized_frame,
        last_advance_unix_ms: value.last_advance_unix_ms,
        observed_unix_ms: value.observed_unix_ms,
        awaited_frame: value.awaited_frame,
        recovery: value.recovery.map(|recovery| RecoverySnapshot {
            phase: recovery.phase,
            phase_since_unix_ms: recovery.phase_since_unix_ms,
            tree_phase: recovery.tree_phase,
            installed_leaves: recovery.installed_leaves,
            planned_leaves: recovery.planned_leaves,
            leaves_observed_unix_ms: recovery.leaves_observed_unix_ms,
            last_outcome: non_empty(recovery.last_outcome),
            last_outcome_unix_ms: recovery.last_outcome_unix_ms,
            last_error: non_empty(recovery.last_error),
            last_error_unix_ms: recovery.last_error_unix_ms,
            source: non_empty(recovery.source),
            attempts: recovery.attempts,
            consecutive_failures: recovery.consecutive_failures,
            attempt_started_unix_ms: recovery.attempt_started_unix_ms,
            permit_wait_ms: recovery.permit_wait_ms,
            attempt_ms: recovery.attempt_ms,
            attempt_installed_leaves: recovery.attempt_installed_leaves,
            failures_by_class: {
                let mut failures = recovery.failures_by_class.into_iter().collect::<Vec<_>>();
                failures.sort_by(|left, right| left.0.cmp(&right.0));
                failures
            },
            longest_permit_wait_ms: recovery.longest_permit_wait_ms,
        }),
        seal: value.seal.map(|seal| SessionSealSnapshot {
            generation: seal.generation,
            checkpoint: seal.checkpoint,
            view: seal.view,
            global_executed: seal.global_executed,
            drain: seal.drain,
            drain_headers: seal.drain_headers,
            finalized_unix_ms: seal.finalized_unix_ms,
            last_submitted_unix_ms: seal.last_submitted_unix_ms,
            submissions: seal.submissions,
            recorded_unix_ms: seal.recorded_unix_ms,
        }),
    }
}

fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

#[async_trait]
impl ConsoleService for NodeConsoleService {
    async fn snapshot(
        &self,
        config_name: &str,
        config_dir: &Path,
    ) -> Result<NodeSnapshot, ServiceError> {
        let config = quil_config::load_config(config_dir)
            .map_err(|error| ServiceError::Config(error.to_string()))?;
        let address = grpc_address(config.effective_listen_grpc_multiaddr())?;
        let mut client = connect_client(&address).await?;
        let info = tokio::time::timeout(
            RPC_TIMEOUT,
            client.get_node_info(tonic::Request::new(GetNodeInfoRequest::default())),
        )
        .await
        .map_err(|_| ServiceError::Timeout)?
        .map_err(|error| ServiceError::Rpc(error.to_string()))?
        .into_inner();
        let workers = match tokio::time::timeout(
            RPC_TIMEOUT,
            client.get_worker_info(tonic::Request::new(GetWorkerInfoRequest::default())),
        )
        .await
        {
            Ok(Ok(response)) => Ok(response.into_inner()),
            Ok(Err(error)) => Err(format!("GetWorkerInfo RPC failed: {error}")),
            Err(_) => Err(format!(
                "GetWorkerInfo RPC timed out ({} seconds)",
                RPC_TIMEOUT.as_secs()
            )),
        };

        Ok(map_snapshot(config_name, &address, info, workers))
    }

    async fn execute_prover(
        &self,
        config_dir: &Path,
        action: crate::domain::ProverAction,
    ) -> Result<crate::domain::OperationResult, ServiceError> {
        super::prover::execute(config_dir, action).await
    }

    async fn diagnostics_dial(
        &self,
        address: &str,
    ) -> Result<crate::domain::DiagnosticsReport, ServiceError> {
        super::diagnostics::dial(address).await
    }

    async fn inspect_diagnostics_identity(
        &self,
        path: &Path,
    ) -> Result<crate::domain::DiagnosticsIdentitySnapshot, ServiceError> {
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || super::diagnostics_identity::inspect(&path))
            .await
            .map_err(|error| {
                ServiceError::DiagnosticsIdentity(format!("Read task failed: {error}"))
            })?
    }

    async fn create_diagnostics_identity(
        &self,
        path: &Path,
    ) -> Result<crate::domain::DiagnosticsIdentitySnapshot, ServiceError> {
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || super::diagnostics_identity::create(&path))
            .await
            .map_err(|error| {
                ServiceError::DiagnosticsIdentity(format!("Create task failed: {error}"))
            })?
    }

    async fn resolve_peer(
        &self,
        config_dir: &Path,
        peer_id: &str,
    ) -> Result<crate::domain::PeerAddressReport, ServiceError> {
        super::diagnostics::resolve_peer(config_dir, peer_id).await
    }

    async fn health_report(
        &self,
        config_dir: &Path,
        target: &crate::domain::HealthTarget,
    ) -> Result<crate::domain::HealthSnapshot, ServiceError> {
        super::health::read(config_dir, target).await
    }

    async fn read_logs(
        &self,
        config_dir: &Path,
        core_id: u32,
        lines: usize,
    ) -> Result<crate::domain::LogSnapshot, ServiceError> {
        super::logs::read(config_dir, core_id, lines).await
    }

    async fn prepare_log_export(
        &self,
        config_dir: &Path,
        options: crate::domain::LogExportOptions,
    ) -> Result<crate::domain::LogExportPreview, ServiceError> {
        self.log_exports.prepare(config_dir, options).await
    }

    async fn finish_log_export(
        &self,
        id: u64,
        commit: bool,
    ) -> Result<crate::domain::OperationResult, ServiceError> {
        self.log_exports.finish(id, commit).await
    }

    async fn runtime_status(&self) -> Result<crate::domain::RuntimeSnapshot, ServiceError> {
        super::runtime::read().await
    }

    async fn list_keys(
        &self,
        config_dir: &Path,
    ) -> Result<Vec<crate::domain::KeySnapshot>, ServiceError> {
        super::keys::list(config_dir).await
    }

    async fn mutate_key(
        &self,
        config_dir: &Path,
        mutation: crate::domain::KeyMutation,
    ) -> Result<crate::domain::OperationResult, ServiceError> {
        super::keys::mutate(config_dir, mutation).await
    }

    async fn list_aliases(
        &self,
        config_dir: &Path,
    ) -> Result<Vec<crate::domain::AliasSnapshot>, ServiceError> {
        super::aliases::list(config_dir).await
    }

    async fn mutate_alias(
        &self,
        config_dir: &Path,
        mutation: crate::domain::AliasMutation,
    ) -> Result<crate::domain::OperationResult, ServiceError> {
        super::aliases::mutate(config_dir, mutation).await
    }

    async fn reward_summary(
        &self,
        config_dir: &Path,
    ) -> Result<crate::domain::RewardSnapshot, ServiceError> {
        super::rewards::read(config_dir).await
    }

    async fn reward_mint_preflight(
        &self,
        config_dir: &Path,
    ) -> Result<crate::domain::RewardMintPreflight, ServiceError> {
        super::wallet::reward_mint_preflight(config_dir).await
    }

    async fn start_reward_mint(
        &self,
        config_dir: &Path,
        approved: crate::domain::RewardMintPreflight,
    ) -> Result<crate::domain::RewardMintStatus, ServiceError> {
        super::wallet::start_reward_mint(config_dir, approved).await
    }

    async fn load_reward_mint(
        &self,
        config_dir: &Path,
    ) -> Result<Option<crate::domain::RewardMintStatus>, ServiceError> {
        super::wallet::load_reward_mint(config_dir).await
    }

    async fn advance_reward_mint(
        &self,
        config_dir: &Path,
    ) -> Result<crate::domain::RewardMintStatus, ServiceError> {
        super::wallet::advance_reward_mint(config_dir).await
    }

    async fn rebroadcast_reward_mint(
        &self,
        config_dir: &Path,
        confirmed: bool,
    ) -> Result<crate::domain::RewardMintStatus, ServiceError> {
        super::wallet::rebroadcast_reward_mint(config_dir, confirmed).await
    }

    async fn preflight_legacy_migration(
        &self,
        active_config: &Path,
        request: super::legacy_migration::LegacyMigrationRequest,
    ) -> Result<crate::domain::LegacyMigrationPreflight, ServiceError> {
        self.legacy_migrations
            .preflight(active_config, request)
            .await
    }

    async fn start_legacy_migration(
        &self,
        approved: crate::domain::LegacyMigrationPreflight,
    ) -> Result<crate::domain::LegacyMigrationStatus, ServiceError> {
        self.legacy_migrations.start(approved).await
    }

    async fn load_legacy_migration(
        &self,
        fingerprint: &str,
    ) -> Result<Option<crate::domain::LegacyMigrationStatus>, ServiceError> {
        self.legacy_migrations.load(fingerprint)
    }

    async fn advance_legacy_migration(
        &self,
        fingerprint: &str,
        source_key: Option<super::legacy_migration::SecretLegacyKey>,
    ) -> Result<crate::domain::LegacyMigrationStatus, ServiceError> {
        self.legacy_migrations
            .advance(fingerprint, source_key)
            .await
    }

    async fn rebroadcast_legacy_migration(
        &self,
        fingerprint: &str,
        confirmed: bool,
    ) -> Result<crate::domain::LegacyMigrationStatus, ServiceError> {
        self.legacy_migrations
            .rebroadcast(fingerprint, confirmed)
            .await
    }

    async fn legacy_assets(
        &self,
        config_dir: &Path,
    ) -> Result<crate::domain::LegacyAssetsSnapshot, ServiceError> {
        super::legacy_assets::read(config_dir).await
    }

    async fn confidential_wallet(
        &self,
        config_dir: &Path,
    ) -> Result<crate::domain::ConfidentialWalletSnapshot, ServiceError> {
        super::wallet::read(config_dir).await
    }

    async fn pending_escrows(
        &self,
        config_dir: &Path,
    ) -> Result<crate::domain::PendingEscrowsSnapshot, ServiceError> {
        super::wallet::read_escrows(config_dir).await
    }

    async fn preflight_wallet_action(
        &self,
        config_dir: &Path,
        request: crate::domain::WalletActionRequest,
    ) -> Result<crate::domain::WalletPreflight, ServiceError> {
        super::wallet::preflight_action(config_dir, request).await
    }

    async fn execute_wallet_action(
        &self,
        config_dir: &Path,
        request: crate::domain::WalletActionRequest,
        approved: crate::domain::WalletPreflight,
    ) -> Result<crate::domain::WalletOperationResult, ServiceError> {
        super::wallet::execute_action(config_dir, request, approved).await
    }

    async fn query_hypergraph(
        &self,
        config_dir: &Path,
        kind: crate::domain::HypergraphKind,
        address: &str,
    ) -> Result<crate::domain::HypergraphSnapshot, ServiceError> {
        super::hypergraph::read(config_dir, kind, address).await
    }

    async fn shard_info(
        &self,
        config_dir: &Path,
        include_all: bool,
    ) -> Result<crate::domain::ShardInfoSnapshot, ServiceError> {
        super::shards::read(config_dir, include_all, &self.explorer).await
    }

    async fn archive_shards(
        &self,
        config_dir: &Path,
    ) -> Result<crate::domain::ArchiveShardSnapshot, ServiceError> {
        super::archive_shards::read(config_dir).await
    }
}

#[cfg(test)]
mod tests {
    use quil_types::proto::node::{
        FrameCadence, FrameWindow, NodeInfoResponse, SessionSeal, ShardAllocationInfo,
        ShardRecovery, WorkerExecution, WorkerInfo, WorkerInfoResponse,
    };

    use super::*;
    use crate::domain::{
        AllocationStatus, EpochSnapshot, FrameCadenceSnapshot, FrameWindowSnapshot,
        RecoverySnapshot, SessionSealSnapshot,
    };

    #[test]
    fn parses_supported_grpc_multiaddresses() {
        assert_eq!(
            grpc_address("/ip4/127.0.0.1/tcp/8337").unwrap(),
            "127.0.0.1:8337"
        );
        assert_eq!(
            grpc_address("/ip4/0.0.0.0/tcp/8337").unwrap(),
            "127.0.0.1:8337"
        );
        assert_eq!(
            grpc_address("/dns4/node.local/tcp/8337").unwrap(),
            "node.local:8337"
        );
        assert_eq!(grpc_address("/ip6/::1/tcp/8337").unwrap(), "[::1]:8337");
        assert!(grpc_address("/unix/node.sock").is_err());
    }

    #[test]
    fn maps_node_and_worker_responses_without_text_parsing() {
        let info = NodeInfoResponse {
            peer_id: "peer-1".into(),
            version: vec![2, 1, 0],
            patch_number: vec![25],
            running_workers: 1,
            allocated_workers: 1,
            last_global_head_frame: 907_773,
            shard_allocations: vec![ShardAllocationInfo {
                filter: vec![0xaa],
                status: 2,
                ..Default::default()
            }],
            ..Default::default()
        };
        let workers = WorkerInfoResponse {
            worker_info: vec![
                WorkerInfo {
                    core_id: 3,
                    filter: vec![0xbb],
                    ..Default::default()
                },
                WorkerInfo {
                    core_id: 1,
                    filter: vec![0xaa],
                    execution: Some(WorkerExecution {
                        state: "running".into(),
                        materialized_frame: Some(907_770),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ],
        };

        let snapshot = map_snapshot("mainnet", "127.0.0.1:8337", info, Ok(workers));
        assert_eq!(snapshot.peer_id, "peer-1");
        assert_eq!(snapshot.allocations[0].status, AllocationStatus::Active);
        assert_eq!(
            snapshot
                .workers
                .iter()
                .map(|worker| worker.core_id)
                .collect::<Vec<_>>(),
            vec![1, 3]
        );
        assert_eq!(snapshot.workers[0].core_id, 1);
        assert_eq!(snapshot.workers[0].materialized_frame(), Some(907_770));
    }

    #[test]
    fn preserves_full_node_allocation_worker_recovery_and_seal_snapshot() {
        let info = NodeInfoResponse {
            peer_id: "peer-full".into(),
            peer_score: 99,
            version: vec![2, 1, 0],
            peer_seniority: vec![0xff; 16],
            running_workers: 1,
            allocated_workers: 2,
            patch_number: vec![25],
            last_received_frame: 900,
            last_global_head_frame: 899,
            reachable: true,
            current_epoch: 42,
            epoch_length_frames: 720,
            frame_cadence: Some(FrameCadence {
                last_observed_unix_seconds: 1_000,
                hour: Some(FrameWindow {
                    window_seconds: 3_600,
                    observed_seconds: 3_590,
                    advanced_frames: 360,
                }),
                day: Some(FrameWindow {
                    window_seconds: 86_400,
                    observed_seconds: 86_000,
                    advanced_frames: 8_600,
                }),
            }),
            shard_allocations: vec![ShardAllocationInfo {
                filter: vec![0xaa; 32],
                status: 2,
                join_frame_number: 101,
                join_confirm_frame_number: 102,
                leave_frame_number: 103,
                leave_confirm_frame_number: 104,
                last_active_frame_number: 105,
                epoch: 41,
            }],
            ..Default::default()
        };
        let workers = WorkerInfoResponse {
            worker_info: vec![WorkerInfo {
                core_id: 7,
                filter: vec![0xaa; 32],
                available_storage: 400,
                total_storage: 1_000,
                manually_managed: true,
                execution: Some(WorkerExecution {
                    state: "blocked".into(),
                    blocker: "waiting for transport".into(),
                    materialized_frame: Some(90),
                    last_advance_unix_ms: 11,
                    observed_unix_ms: 12,
                    awaited_frame: Some(100),
                    recovery: Some(ShardRecovery {
                        phase: "replaying_frames".into(),
                        phase_since_unix_ms: 10,
                        tree_phase: 2,
                        installed_leaves: 40,
                        planned_leaves: 100,
                        leaves_observed_unix_ms: 13,
                        last_outcome: "retrying".into(),
                        last_outcome_unix_ms: 14,
                        last_error: "archive timeout".into(),
                        last_error_unix_ms: 15,
                        source: "archive-1".into(),
                        attempts: 4,
                        consecutive_failures: 1,
                        attempt_started_unix_ms: 16,
                        permit_wait_ms: 17,
                        attempt_ms: 18,
                        attempt_installed_leaves: 19,
                        failures_by_class: std::collections::HashMap::from([
                            ("connect".into(), 2),
                            ("archive_state".into(), 1),
                        ]),
                        longest_permit_wait_ms: 20,
                    }),
                    seal: Some(SessionSeal {
                        generation: 3,
                        checkpoint: 88,
                        view: 89,
                        global_executed: 87,
                        drain: "partial".into(),
                        drain_headers: 8,
                        finalized_unix_ms: 21,
                        last_submitted_unix_ms: 22,
                        submissions: 2,
                        recorded_unix_ms: 23,
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }],
        };

        let snapshot = map_snapshot("mainnet", "127.0.0.1:8337", info, Ok(workers));
        assert_eq!(
            snapshot,
            NodeSnapshot {
                config_name: "mainnet".into(),
                rpc_address: "127.0.0.1:8337".into(),
                connected: true,
                peer_id: "peer-full".into(),
                peer_seniority: Some("340282366920938463463374607431768211455".into()),
                version: vec![2, 1, 0],
                patch_number: vec![25],
                last_received_frame: 900,
                last_global_head_frame: 899,
                reachable: true,
                running_workers: 1,
                allocated_workers: 2,
                peer_score: 99,
                epoch: Some(EpochSnapshot {
                    current: 42,
                    length_frames: 720,
                }),
                frame_cadence: Some(FrameCadenceSnapshot {
                    last_observed_unix_seconds: 1_000,
                    hour: Some(FrameWindowSnapshot {
                        window_seconds: 3_600,
                        observed_seconds: 3_590,
                        advanced_frames: 360,
                    }),
                    day: Some(FrameWindowSnapshot {
                        window_seconds: 86_400,
                        observed_seconds: 86_000,
                        advanced_frames: 8_600,
                    }),
                }),
                allocations: vec![AllocationSnapshot {
                    filter_hex: "aa".repeat(32),
                    status: AllocationStatus::Active,
                    join_frame_number: 101,
                    join_confirm_frame_number: 102,
                    leave_frame_number: 103,
                    leave_confirm_frame_number: 104,
                    last_active_frame_number: 105,
                    epoch: 41,
                }],
                workers: vec![WorkerSnapshot {
                    core_id: 7,
                    filter_hex: "aa".repeat(32),
                    available_storage: 400,
                    total_storage: 1_000,
                    manually_managed: true,
                    execution: Some(WorkerExecutionSnapshot {
                        state: "blocked".into(),
                        blocker: Some("waiting for transport".into()),
                        materialized_frame: Some(90),
                        last_advance_unix_ms: 11,
                        observed_unix_ms: 12,
                        awaited_frame: Some(100),
                        recovery: Some(RecoverySnapshot {
                            phase: "replaying_frames".into(),
                            phase_since_unix_ms: 10,
                            tree_phase: 2,
                            installed_leaves: 40,
                            planned_leaves: 100,
                            leaves_observed_unix_ms: 13,
                            last_outcome: Some("retrying".into()),
                            last_outcome_unix_ms: 14,
                            last_error: Some("archive timeout".into()),
                            last_error_unix_ms: 15,
                            source: Some("archive-1".into()),
                            attempts: 4,
                            consecutive_failures: 1,
                            attempt_started_unix_ms: 16,
                            permit_wait_ms: 17,
                            attempt_ms: 18,
                            attempt_installed_leaves: 19,
                            failures_by_class: vec![
                                ("archive_state".into(), 1),
                                ("connect".into(), 2),
                            ],
                            longest_permit_wait_ms: 20,
                        }),
                        seal: Some(SessionSealSnapshot {
                            generation: 3,
                            checkpoint: 88,
                            view: 89,
                            global_executed: 87,
                            drain: "partial".into(),
                            drain_headers: 8,
                            finalized_unix_ms: 21,
                            last_submitted_unix_ms: 22,
                            submissions: 2,
                            recorded_unix_ms: 23,
                        }),
                    }),
                }],
                worker_info_error: None,
            }
        );
    }

    #[test]
    fn distinguishes_empty_worker_response_from_worker_rpc_failure() {
        let empty = map_snapshot(
            "mainnet",
            "127.0.0.1:8337",
            NodeInfoResponse::default(),
            Ok(WorkerInfoResponse::default()),
        );
        assert!(empty.workers.is_empty());
        assert_eq!(empty.worker_info_error, None);

        let failed = map_snapshot(
            "mainnet",
            "127.0.0.1:8337",
            NodeInfoResponse {
                peer_id: "node-still-available".into(),
                ..Default::default()
            },
            Err("GetWorkerInfo RPC failed: unavailable".into()),
        );
        assert_eq!(failed.peer_id, "node-still-available");
        assert!(failed.workers.is_empty());
        assert_eq!(
            failed.worker_info_error.as_deref(),
            Some("GetWorkerInfo RPC failed: unavailable")
        );
    }

    #[test]
    fn leaves_absent_optional_node_fields_absent() {
        let snapshot = map_snapshot(
            "legacy",
            "127.0.0.1:8337",
            NodeInfoResponse::default(),
            Ok(WorkerInfoResponse::default()),
        );

        assert_eq!(snapshot.peer_seniority, None);
        assert_eq!(snapshot.frame_cadence, None);
        assert_eq!(snapshot.epoch, None);
    }

    #[test]
    fn preserves_genesis_epoch_when_epoch_length_is_present() {
        let snapshot = map_snapshot(
            "genesis",
            "127.0.0.1:8337",
            NodeInfoResponse {
                current_epoch: 0,
                epoch_length_frames: 720,
                ..Default::default()
            },
            Ok(WorkerInfoResponse::default()),
        );

        assert_eq!(
            snapshot.epoch,
            Some(EpochSnapshot {
                current: 0,
                length_frames: 720,
            })
        );
    }
}
