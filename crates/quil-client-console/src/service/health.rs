use std::{path::Path, str::FromStr};

use quil_types::proto::diagnostics::{
    Blocker, GetHealthReportRequest, HealthReport, Phase, RecoveryFailure, RecoveryPhase,
    StoreIoReason, StoreIoVerdict,
};

use super::{archive_shards, diagnostics_identity, node, prover, ServiceError};
use crate::domain::{
    HealthAffectedShardSnapshot, HealthConditionSnapshot, HealthSnapshot, HealthStoreIoSnapshot,
    HealthTarget, HealthWorkerSnapshot,
};

pub(super) async fn read(
    config_dir: &Path,
    target: &HealthTarget,
) -> Result<HealthSnapshot, ServiceError> {
    let (private_key, own_peer, config) = requester_auth(config_dir, target)?;
    let (peer_id, address, label) = match target {
        HealthTarget::SelfNode => (
            own_peer,
            archive_shards::local_stream_addr(
                config.as_ref().expect("self health loads node config"),
            )?,
            "local node".to_string(),
        ),
        HealthTarget::Peer {
            peer_id, address, ..
        } => {
            let peer = quil_p2p::PeerId::from_str(peer_id.trim()).map_err(|error| {
                ServiceError::InvalidInput(format!("Invalid target Peer ID: {error}"))
            })?;
            if address.trim().is_empty() {
                return Err(ServiceError::InvalidInput(
                    "Target stream address is empty".into(),
                ));
            }
            (peer, address.trim().to_string(), format!("Peer {peer_id}"))
        }
    };
    let mut client = tokio::time::timeout(
        node::RPC_TIMEOUT,
        quil_rpc::diagnostics_service::connect_peer(&address, &private_key, peer_id),
    )
    .await
    .map_err(|_| {
        ServiceError::Health(format!(
            "Timed out connecting to the {label} diagnostics service"
        ))
    })?
    .map_err(|error| {
        ServiceError::Health(format!(
            "Cannot connect to the {label} diagnostics service: {error}"
        ))
    })?;
    let report = tokio::time::timeout(
        node::RPC_TIMEOUT,
        client.get_health_report(GetHealthReportRequest {}),
    )
    .await
    .map_err(|_| ServiceError::Health(format!("Timed out reading the {label} health report")))?
    .map_err(|error| {
        ServiceError::Health(format!(
            "The {label} did not return a health report: {error}"
        ))
    })?
    .into_inner();

    Ok(map_report(report))
}

fn requester_auth(
    config_dir: &Path,
    target: &HealthTarget,
) -> Result<(Vec<u8>, quil_p2p::PeerId, Option<quil_config::Config>), ServiceError> {
    let identity_path = match target {
        HealthTarget::Peer { identity_path, .. } => identity_path.as_deref(),
        HealthTarget::SelfNode => None,
    };
    let config = if identity_path.is_none() || matches!(target, HealthTarget::SelfNode) {
        Some(
            quil_config::load_config(config_dir)
                .map_err(|error| ServiceError::Config(error.to_string()))?,
        )
    } else {
        None
    };
    let private_key = if let Some(path) = identity_path {
        diagnostics_identity::load_key(Path::new(path))?
    } else {
        let config = config.as_ref().expect("node identity loads node config");
        let manager = prover::build_key_manager(config, config_dir)?;
        manager
            .get_signer_by_id("q-prover-key")
            .map_err(|error| ServiceError::Key(error.to_string()))?
            .private_key()
            .to_vec()
    };
    let own_peer = quil_p2p::Keypair::falcon_from_bytes(&private_key)
        .map_err(|error| ServiceError::Key(format!("Invalid Falcon network identity: {error}")))?
        .public()
        .to_peer_id();
    Ok((private_key, own_peer, config))
}

fn map_report(report: HealthReport) -> HealthSnapshot {
    let store_io = report.store_io.map(|io| {
        let figures = quil_rpc::health_conditions::store_io_figures(&io);
        HealthStoreIoSnapshot {
            verdict: store_io_verdict_label(io.verdict),
            slow_reason: store_io_reason_label(io.slow_reason),
            slow_secs: io.slow_secs,
            interval_secs: io.interval_secs,
            ended_unix_ms: io.ended_unix_ms,
            canary_reads: io.canary_reads,
            canary_p50_us: io.canary_p50_us,
            canary_p99_us: io.canary_p99_us,
            canary_max_us: io.canary_max_us,
            canary_pending_us: io.canary_pending_us,
            disk_reads: io.disk_reads,
            disk_writes: io.disk_writes,
            disk_read_await_us: io.disk_read_await_us,
            disk_write_await_us: io.disk_write_await_us,
            disk_busy_pct: io.disk_busy_pct,
            figures,
        }
    });
    let conditions_reported = report.health.is_some();
    let (conditions, unobserved_conditions) = report.health.map_or_else(
        || (Vec::new(), Vec::new()),
        |health| {
            let unobserved = health
                .sources
                .iter()
                .filter(|source| source.observed_unix_ms == 0)
                .map(|source| quil_rpc::health_conditions::kind_label(source.kind).to_string())
                .collect();
            let conditions = health
                .conditions
                .into_iter()
                .map(|condition| {
                    let notice = quil_rpc::health_conditions::notice_text(&condition);
                    HealthConditionSnapshot {
                        kind: quil_rpc::health_conditions::kind_label(condition.kind).into(),
                        severity: quil_rpc::health_conditions::severity_label(condition.severity)
                            .into(),
                        scope: quil_rpc::health_conditions::scope_label(condition.scope).into(),
                        since_unix_ms: condition.since_unix_ms,
                        affected: condition.affected,
                        tracked: condition.tracked,
                        threshold: condition.threshold,
                        notice,
                        action: quil_rpc::health_conditions::action_text(condition.action).into(),
                        shards: condition
                            .shards
                            .into_iter()
                            .map(|shard| HealthAffectedShardSnapshot {
                                filter_hex: hex::encode(shard.filter),
                                core_id: shard.core_id,
                                frame: shard.frame,
                                unchanged_since_unix_ms: shard.unchanged_since_unix_ms,
                                window_secs: shard.window_secs,
                                lag_frames: shard.lag_frames,
                            })
                            .collect(),
                        omitted_shards: condition.omitted_shards,
                    }
                })
                .collect();
            (conditions, unobserved)
        },
    );

    HealthSnapshot {
        release: release(&report.version, report.patch, report.subpatch),
        build_fingerprint: report
            .build_fingerprint
            .get(..4)
            .map(hex::encode)
            .unwrap_or_else(|| "unknown".into()),
        global_head_frame: report.global_head_frame,
        generated_unix_ms: report.generated_unix_ms,
        omitted_workers: report.omitted_workers,
        worker_capacity: report.worker_capacity,
        master_store_write_failed: report.master_store_write_failed,
        store_io,
        conditions_reported,
        conditions,
        unobserved_conditions,
        workers: report
            .workers
            .into_iter()
            .map(|worker| HealthWorkerSnapshot {
                core_id: worker.core_id,
                filter_hex: hex::encode(worker.filter),
                allocated: worker.allocated,
                draining: worker.draining,
                phase: phase_label(worker.phase).into(),
                blocker: blocker_label(worker.blocker).into(),
                observed_unix_ms: worker.observed_unix_ms,
                materialized_frame: worker.materialized_frame,
                last_advance_unix_ms: worker.last_advance_unix_ms,
                committee_head_frame: worker.committee_head_frame,
                committed_generation: worker.committed_generation,
                recovery_phase: recovery_phase_label(worker.recovery_phase).into(),
                recovery_attempts: worker.recovery_attempts,
                recovery_consecutive_failures: worker.recovery_consecutive_failures,
                recovery_failures: worker
                    .recovery_failures
                    .into_iter()
                    .map(|failure| {
                        (
                            recovery_failure_label(failure.class).to_string(),
                            failure.count,
                        )
                    })
                    .collect(),
                last_failure: recovery_failure_label(worker.last_failure).into(),
                last_failure_unix_ms: worker.last_failure_unix_ms,
                last_success_unix_ms: worker.last_success_unix_ms,
                longest_permit_wait_ms: worker.longest_permit_wait_ms,
                seal_generation: worker.seal_generation,
                seal_recorded_unix_ms: worker.seal_recorded_unix_ms,
                join_confirm_epoch: worker.join_confirm_epoch,
                activation_frame: worker.activation_frame,
                awaited_frame: worker.awaited_frame,
            })
            .collect(),
    }
}

fn store_io_verdict_label(value: i32) -> String {
    StoreIoVerdict::try_from(value).map_or_else(
        |_| format!("unknown ({value})"),
        |verdict| proto_enum_label(verdict.as_str_name(), "STORE_IO_VERDICT_"),
    )
}

fn store_io_reason_label(value: i32) -> String {
    StoreIoReason::try_from(value).map_or_else(
        |_| format!("unknown ({value})"),
        |reason| proto_enum_label(reason.as_str_name(), "STORE_IO_REASON_"),
    )
}

fn proto_enum_label(name: &str, prefix: &str) -> String {
    name.strip_prefix(prefix)
        .unwrap_or(name)
        .to_ascii_lowercase()
        .replace('_', " ")
}

fn release(version: &[u8], patch: u32, subpatch: u32) -> String {
    let major = version.first().copied().unwrap_or_default();
    let minor = version.get(1).copied().unwrap_or_default();
    let revision = version.get(2).copied().unwrap_or_default();
    if subpatch == 0 {
        format!("{major}.{minor}.{revision}.{patch}")
    } else {
        format!("{major}.{minor}.{revision}.{patch}.{subpatch}")
    }
}

fn phase_label(value: i32) -> &'static str {
    match Phase::try_from(value).unwrap_or(Phase::Unrecognized) {
        Phase::Unspecified => "unobserved",
        Phase::Starting => "starting",
        Phase::Waiting => "waiting",
        Phase::Syncing => "syncing",
        Phase::Serving => "serving",
        Phase::Closing => "closing",
        Phase::Handoff => "handoff",
        Phase::Blocked => "blocked",
        Phase::Stopped => "stopped",
        Phase::Running => "running",
        Phase::Unknown => "unknown",
        Phase::Unrecognized => "unrecognized",
        Phase::Joining => "joining",
    }
}

fn blocker_label(value: i32) -> &'static str {
    match Blocker::try_from(value).unwrap_or(Blocker::Unrecognized) {
        Blocker::None => "none",
        Blocker::Unrecognized => "unrecognized",
        Blocker::WaitingForTransport => "waiting for transport",
        Blocker::RestartingConsensusHost => "restarting consensus host",
        Blocker::EngineExited => "engine exited",
        Blocker::AwaitingSuccessor => "awaiting successor shard",
        Blocker::StagingShardData => "staging shard data",
        Blocker::AwaitingClosingCertificate => "awaiting closing certificate",
        Blocker::AwaitingAllocationActivation => "awaiting Allocation activation",
        Blocker::AwaitingCommitteeSession => "awaiting committee session",
        Blocker::AwaitingShardReanchor => "awaiting shard re-anchor",
        Blocker::AwaitingGlobalStateSync => "awaiting GLOBAL state sync",
        Blocker::CheckpointMismatch => "checkpoint mismatch",
        Blocker::StateUnavailable => "state unavailable",
        Blocker::CommitteeAdmissionUnavailable => "committee admission unavailable",
        Blocker::ConsensusStartupUnavailable => "consensus startup unavailable",
        Blocker::SessionIdentityUnavailable => "session identity unavailable",
        Blocker::SessionStateUnavailable => "session state unavailable",
        Blocker::StorageWriteFailed => "storage writes stopped",
    }
}

fn recovery_phase_label(value: i32) -> &'static str {
    match RecoveryPhase::try_from(value).unwrap_or(RecoveryPhase::Unrecognized) {
        RecoveryPhase::None => "not started",
        RecoveryPhase::Idle => "idle",
        RecoveryPhase::WaitingForPermit => "waiting for recovery permit",
        RecoveryPhase::ReplayingFrames => "replaying frames",
        RecoveryPhase::FetchingCheckpoint => "fetching checkpoint",
        RecoveryPhase::InstallingLeaves => "installing leaf data",
        RecoveryPhase::InstallingCheckpoint => "installing checkpoint",
        RecoveryPhase::Starting => "starting",
        RecoveryPhase::Unrecognized => "unrecognized",
    }
}

fn recovery_failure_label(value: i32) -> &'static str {
    match RecoveryFailure::try_from(value).unwrap_or(RecoveryFailure::Other) {
        RecoveryFailure::None => "none",
        RecoveryFailure::NoArchiveKnown => "remote Archive unavailable",
        RecoveryFailure::Connect => "remote Archive connection failed",
        RecoveryFailure::ArchiveRequest => "remote Archive request failed",
        RecoveryFailure::ArchiveState => "remote Archive state unavailable",
        RecoveryFailure::ReplayMismatch => "replay mismatch",
        RecoveryFailure::Other => "other error",
        RecoveryFailure::GlobalSync => "GLOBAL state sync installing",
        RecoveryFailure::ProofVerification => "proof verification failed",
        RecoveryFailure::EngineBusy => "execution engine busy",
    }
}

#[cfg(test)]
mod tests {
    use quil_types::proto::diagnostics::{
        AffectedShard, ConditionAction, ConditionKind, ConditionScope, ConditionSeverity,
        ConditionSource, FailureCount, HealthCondition, HealthConditions, StoreIoHealth,
        StoreIoReason, StoreIoVerdict, WorkerHealth,
    };

    use super::*;

    #[test]
    fn maps_health_report_without_inventing_missing_frames() {
        let report = map_report(HealthReport {
            version: vec![2, 1, 0],
            patch: 25,
            subpatch: 99,
            build_fingerprint: vec![0xab; 32],
            global_head_frame: 42,
            generated_unix_ms: 1_000,
            omitted_workers: 2,
            workers: vec![WorkerHealth {
                core_id: 7,
                filter: vec![0xcd; 32],
                allocated: true,
                phase: Phase::Serving as i32,
                blocker: Blocker::None as i32,
                materialized_frame: Some(41),
                recovery_phase: RecoveryPhase::Idle as i32,
                recovery_attempts: 3,
                recovery_failures: vec![FailureCount {
                    class: RecoveryFailure::Connect as i32,
                    count: 1,
                }],
                ..Default::default()
            }],
            ..Default::default()
        });

        assert_eq!(report.release, "2.1.0.25.99");
        assert_eq!(report.build_fingerprint, "abababab");
        assert_eq!(report.workers[0].phase, "serving");
        assert_eq!(report.workers[0].materialized_frame, Some(41));
        assert_eq!(report.workers[0].committee_head_frame, None);
        assert_eq!(
            report.workers[0].recovery_failures,
            vec![("remote Archive connection failed".into(), 1)]
        );
    }

    #[test]
    fn maps_latest_report_and_store_io_fields_without_losing_measurements() {
        let mut store_io = StoreIoHealth {
            slow_secs: 121,
            interval_secs: 60,
            ended_unix_ms: 9_000,
            canary_reads: 59,
            canary_p50_us: 1_500,
            canary_p99_us: 75_000,
            canary_max_us: 90_000,
            canary_pending_us: 1_200_000,
            disk_reads: Some(100),
            disk_writes: Some(50),
            disk_read_await_us: Some(21_000),
            disk_write_await_us: Some(101_000),
            disk_busy_pct: Some(87),
            ..Default::default()
        };
        store_io.set_verdict(StoreIoVerdict::Slow);
        store_io.set_slow_reason(StoreIoReason::DiskReadAwait);
        let expected_figures = quil_rpc::health_conditions::store_io_figures(&store_io);

        let report = map_report(HealthReport {
            worker_capacity: 6,
            master_store_write_failed: true,
            store_io: Some(store_io),
            workers: vec![WorkerHealth {
                core_id: 7,
                draining: true,
                ..Default::default()
            }],
            ..Default::default()
        });

        assert_eq!(report.worker_capacity, 6);
        assert!(report.master_store_write_failed);
        assert!(report.workers[0].draining);
        let store_io = report.store_io.expect("latest report includes store I/O");
        assert_eq!(store_io.verdict, "slow");
        assert_eq!(store_io.slow_reason, "disk read await");
        assert_eq!(store_io.slow_secs, 121);
        assert_eq!(store_io.interval_secs, 60);
        assert_eq!(store_io.ended_unix_ms, 9_000);
        assert_eq!(store_io.canary_reads, 59);
        assert_eq!(store_io.canary_p50_us, 1_500);
        assert_eq!(store_io.canary_p99_us, 75_000);
        assert_eq!(store_io.canary_max_us, 90_000);
        assert_eq!(store_io.canary_pending_us, 1_200_000);
        assert_eq!(store_io.disk_reads, Some(100));
        assert_eq!(store_io.disk_writes, Some(50));
        assert_eq!(store_io.disk_read_await_us, Some(21_000));
        assert_eq!(store_io.disk_write_await_us, Some(101_000));
        assert_eq!(store_io.disk_busy_pct, Some(87));
        assert_eq!(store_io.figures, expected_figures);
    }

    #[test]
    fn maps_condition_labels_text_evidence_and_unobserved_sources() {
        let affected = AffectedShard {
            filter: vec![0xcd; 32],
            core_id: 7,
            frame: 41,
            unchanged_since_unix_ms: 800,
            window_secs: 60,
            lag_frames: 101,
        };
        let severities = [
            ConditionSeverity::Info,
            ConditionSeverity::Warning,
            ConditionSeverity::Error,
        ];
        let conditions = severities
            .into_iter()
            .enumerate()
            .map(|(index, severity)| HealthCondition {
                kind: ConditionKind::LocalExecutionStopped as i32,
                severity: severity as i32,
                scope: ConditionScope::ThisNode as i32,
                action: ConditionAction::CheckBlockers as i32,
                since_unix_ms: 1_000 + index as u64,
                affected: 1,
                tracked: 3,
                threshold: 60,
                shards: vec![affected.clone()],
                omitted_shards: 2,
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let expected_notices = conditions
            .iter()
            .map(quil_rpc::health_conditions::notice_text)
            .collect::<Vec<_>>();

        let report = map_report(HealthReport {
            health: Some(HealthConditions {
                conditions,
                sources: vec![
                    ConditionSource {
                        kind: ConditionKind::LocalExecutionStopped as i32,
                        observed_unix_ms: 2_000,
                        tracked: 3,
                    },
                    ConditionSource {
                        kind: ConditionKind::PeerAppFramesUnchanged as i32,
                        observed_unix_ms: 0,
                        tracked: 0,
                    },
                ],
            }),
            ..Default::default()
        });

        assert!(report.conditions_reported);
        assert_eq!(
            report
                .conditions
                .iter()
                .map(|condition| condition.severity.as_str())
                .collect::<Vec<_>>(),
            vec!["info", "warning", "error"]
        );
        assert_eq!(
            report
                .conditions
                .iter()
                .map(|condition| condition.notice.as_str())
                .collect::<Vec<_>>(),
            expected_notices
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        );
        let condition = &report.conditions[0];
        assert_eq!(condition.kind, "local_execution_stopped");
        assert_eq!(condition.scope, "this node");
        assert_eq!(condition.since_unix_ms, 1_000);
        assert_eq!(condition.affected, 1);
        assert_eq!(condition.tracked, 3);
        assert_eq!(condition.threshold, 60);
        assert_eq!(condition.omitted_shards, 2);
        assert_eq!(
            condition.action,
            quil_rpc::health_conditions::action_text(ConditionAction::CheckBlockers as i32)
        );
        assert_eq!(condition.shards.len(), 1);
        let shard = &condition.shards[0];
        assert_eq!(shard.filter_hex, hex::encode([0xcd; 32]));
        assert_eq!(shard.core_id, 7);
        assert_eq!(shard.frame, 41);
        assert_eq!(shard.unchanged_since_unix_ms, 800);
        assert_eq!(shard.window_secs, 60);
        assert_eq!(shard.lag_frames, 101);
        assert_eq!(
            report.unobserved_conditions,
            vec!["peer_app_frames_unchanged"]
        );
    }

    #[test]
    fn old_reports_keep_optional_health_sections_absent() {
        let report = map_report(HealthReport::default());

        assert_eq!(report.worker_capacity, 0);
        assert!(!report.master_store_write_failed);
        assert!(report.store_io.is_none());
        assert!(!report.conditions_reported);
        assert!(report.conditions.is_empty());
        assert!(report.unobserved_conditions.is_empty());
    }

    #[test]
    fn archive_recovery_labels_describe_a_remote_dependency_without_hiding_blocked() {
        for failure in [
            RecoveryFailure::NoArchiveKnown,
            RecoveryFailure::Connect,
            RecoveryFailure::ArchiveRequest,
            RecoveryFailure::ArchiveState,
        ] {
            assert!(recovery_failure_label(failure as i32).starts_with("remote Archive "));
        }
        let report = map_report(HealthReport {
            workers: vec![WorkerHealth {
                phase: Phase::Blocked as i32,
                blocker: Blocker::ConsensusStartupUnavailable as i32,
                last_failure: RecoveryFailure::ArchiveState as i32,
                ..Default::default()
            }],
            ..Default::default()
        });
        assert_eq!(report.workers[0].phase, "blocked");
        assert_eq!(report.workers[0].blocker, "consensus startup unavailable");
        assert_eq!(
            report.workers[0].last_failure,
            "remote Archive state unavailable"
        );
    }

    #[test]
    fn storage_write_failure_is_reported_explicitly() {
        assert_eq!(
            blocker_label(Blocker::StorageWriteFailed as i32),
            "storage writes stopped"
        );
    }

    #[test]
    fn current_wait_blockers_are_reported_explicitly() {
        for (blocker, expected) in [
            (
                Blocker::AwaitingCommitteeSession,
                "awaiting committee session",
            ),
            (Blocker::AwaitingShardReanchor, "awaiting shard re-anchor"),
            (
                Blocker::AwaitingGlobalStateSync,
                "awaiting GLOBAL state sync",
            ),
        ] {
            assert_eq!(blocker_label(blocker as i32), expected);
        }
    }

    #[test]
    fn global_sync_wait_is_reported_as_local_installation() {
        assert_eq!(
            recovery_failure_label(RecoveryFailure::GlobalSync as i32),
            "GLOBAL state sync installing"
        );
    }

    #[test]
    fn independent_identity_does_not_read_the_node_config_or_prover_key() {
        let dir = tempfile::tempdir().unwrap();
        let identity_path = dir.path().join("diagnostics.key");
        let identity = diagnostics_identity::create(&identity_path).unwrap();
        let target = HealthTarget::Peer {
            peer_id: identity.peer_id.clone(),
            address: "127.0.0.1:8336".into(),
            identity_path: Some(identity_path.to_string_lossy().into_owned()),
        };

        let (_, peer_id, config) = requester_auth(&dir.path().join("missing-config"), &target)
            .expect("independent identity must not load node config");

        assert_eq!(peer_id.to_string(), identity.peer_id);
        assert!(config.is_none());
    }
}
