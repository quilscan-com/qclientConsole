use crate::{domain::OperationOutcome, event::ServiceEvent};

use super::{App, ConfirmChoice};

pub fn reduce_service(app: &mut App, event: ServiceEvent) {
    match event {
        ServiceEvent::CapabilityTaskStarted {
            request_id,
            capability_id,
            result,
        } if app.capabilities.pending_request_id == Some(request_id) => match result {
            Ok(task) => {
                app.capabilities.pending_request_id = None;
                app.capabilities.current_request_id = Some(request_id);
                app.capabilities.current_id = Some(capability_id);
                app.capabilities.current_task_id = Some(task.id);
                app.capabilities.current_task = Some(task);
                app.capabilities.error = None;
                app.capabilities.task_scroll = 0;
            }
            Err(error) => {
                app.capabilities.pending_request_id = None;
                app.capabilities.current_request_id = Some(request_id);
                app.capabilities.current_id = Some(capability_id);
                app.capabilities.current_task_id = None;
                app.capabilities.current_task = None;
                app.capabilities.error = Some(error);
            }
        },
        ServiceEvent::CapabilityTaskStarted { .. } => {}
        ServiceEvent::CapabilityTaskFinished {
            request_id,
            completion,
        } => {
            let task = completion.into_display_task();
            let belongs_to_current = app.capabilities.current_request_id == Some(request_id)
                && app
                    .capabilities
                    .current_task_id
                    .is_none_or(|task_id| task_id == task.id);
            if belongs_to_current {
                app.capabilities.pending_request_id = None;
                app.capabilities.current_task_id = Some(task.id);
                app.capabilities.current_task = Some(task);
                app.capabilities
                    .clamp_task_scroll(app.viewport_width, app.viewport_height);
            }
        }
        ServiceEvent::RefreshFinished { generation, result }
            if generation == app.refresh_generation =>
        {
            app.loading = false;
            match result {
                Ok(()) => app.error = None,
                Err(error) => app.error = Some(error),
            }
        }
        ServiceEvent::ConfigActivationFinished {
            generation,
            result,
            persistence_error,
        } if generation == app.refresh_generation => {
            app.loading = false;
            match result {
                Ok(()) => app.error = persistence_error,
                Err(error) => app.apply_config_path_error(error),
            }
        }
        ServiceEvent::OperationFinished { generation, result }
            if generation == app.operation_generation =>
        {
            match result {
                Ok(result) => app.finish_operation(
                    result.message,
                    match result.outcome {
                        OperationOutcome::Submitted => Some(true),
                        OperationOutcome::Unknown => None,
                    },
                ),
                Err(error) => app.finish_operation(error, Some(false)),
            }
        }
        ServiceEvent::ProverOperationFinished { id, result } => {
            app.prover_operations.finish_submission(id, result);
        }
        ServiceEvent::DiagnosticsFinished { generation, result }
            if generation == app.diagnostics_generation =>
        {
            app.diagnostics_in_flight = false;
            match result {
                Ok(report) => {
                    app.diagnostics_report = Some(report);
                    app.diagnostics_error = None;
                }
                Err(error) => {
                    app.diagnostics_report = None;
                    app.diagnostics_error = Some(error);
                }
            }
        }
        ServiceEvent::DiagnosticsIdentityFinished { generation, result }
            if generation == app.diagnostics_identity.generation =>
        {
            app.diagnostics_identity.in_flight = false;
            match result {
                Ok(snapshot) => {
                    app.diagnostics_identity.path = snapshot.path.clone();
                    app.diagnostics_identity.snapshot = Some(snapshot);
                    app.diagnostics_identity.error = None;
                }
                Err(error) => app.diagnostics_identity.error = Some(error),
            }
        }
        ServiceEvent::PeerResolveFinished { generation, result }
            if generation == app.peer_resolve.generation =>
        {
            app.peer_resolve.in_flight = false;
            match result {
                Ok(report) => {
                    app.peer_resolve.index = app
                        .peer_resolve
                        .index
                        .min(report.addresses.len().saturating_sub(1));
                    app.peer_resolve.report = Some(report);
                    app.peer_resolve.error = None;
                }
                Err(error) => {
                    app.peer_resolve.report = None;
                    app.peer_resolve.index = 0;
                    app.peer_resolve.error = Some(error);
                }
            }
        }
        ServiceEvent::HealthFinished { generation, result }
            if generation == app.health.generation =>
        {
            app.health.in_flight = false;
            match result {
                Ok(snapshot) => {
                    app.health.index = app
                        .health
                        .index
                        .min(snapshot.workers.len().saturating_sub(1));
                    if snapshot.workers.is_empty() {
                        app.health.detail = false;
                    }
                    app.health.snapshot = Some(snapshot);
                    app.health.error = None;
                }
                Err(error) => {
                    if app.health.snapshot.is_none() {
                        app.health.index = 0;
                        app.health.detail = false;
                    }
                    app.health.error = Some(error);
                }
            }
        }
        ServiceEvent::RuntimeFinished { generation, result }
            if generation == app.runtime.generation =>
        {
            app.runtime.in_flight = false;
            match result {
                Ok(snapshot) => {
                    app.runtime.snapshot = Some(snapshot);
                    app.runtime.error = None;
                }
                Err(error) => {
                    app.runtime.snapshot = None;
                    app.runtime.error = Some(error);
                }
            }
        }
        ServiceEvent::LogFinished { generation, result } if generation == app.log_generation => {
            app.log_in_flight = false;
            match result {
                Ok(snapshot) => {
                    app.log_snapshot = Some(snapshot);
                    app.log_error = None;
                }
                Err(error) => {
                    app.log_snapshot = None;
                    app.log_error = Some(error);
                }
            }
        }
        ServiceEvent::LogExportPrepared { generation, result }
            if generation == app.log_export.generation =>
        {
            app.log_export.in_flight = false;
            match result {
                Ok(preview) => {
                    app.log_export.preview = Some(preview);
                    app.log_export.error = None;
                    app.log_export.choice = ConfirmChoice::Cancel;
                }
                Err(error) => {
                    app.log_export.preview = None;
                    app.log_export.error = Some(error);
                }
            }
        }
        ServiceEvent::LogExportFinished {
            generation,
            committed,
            result,
        } if generation == app.log_export.generation => {
            app.log_export.in_flight = false;
            app.log_export.preview = None;
            app.log_export.choice = ConfirmChoice::Cancel;
            match (committed, result) {
                (true, Ok(result)) => app.finish_operation(result.message, Some(true)),
                (true, Err(error)) => app.finish_operation(error, Some(false)),
                (false, Ok(_)) => app.log_export.error = None,
                (false, Err(error)) => app.log_export.error = Some(error),
            }
        }
        ServiceEvent::KeysFinished { generation, result } if generation == app.key_generation => {
            app.key_in_flight = false;
            match result {
                Ok(keys) => {
                    app.keys = keys;
                    app.key_index = app.key_index.min(app.keys.len().saturating_sub(1));
                    app.key_error = None;
                }
                Err(error) => {
                    app.keys.clear();
                    app.key_index = 0;
                    app.key_error = Some(error);
                }
            }
        }
        ServiceEvent::AliasesFinished { generation, result }
            if generation == app.alias_generation =>
        {
            app.alias_in_flight = false;
            match result {
                Ok(aliases) => {
                    app.aliases = aliases;
                    app.alias_index = app.alias_index.min(app.aliases.len().saturating_sub(1));
                    app.alias_error = None;
                }
                Err(error) => {
                    app.aliases.clear();
                    app.alias_index = 0;
                    app.alias_error = Some(error);
                }
            }
        }
        ServiceEvent::RewardsFinished { generation, result }
            if generation == app.reward_generation =>
        {
            app.reward_in_flight = false;
            match result {
                Ok(snapshot) => {
                    app.reward_snapshot = Some(snapshot);
                    app.reward_error = None;
                }
                Err(error) => {
                    app.reward_snapshot = None;
                    app.reward_error = Some(error);
                }
            }
        }
        ServiceEvent::RewardMintPreflightFinished { generation, result }
            if generation == app.reward_mint.generation =>
        {
            app.reward_mint.in_flight = false;
            match result {
                Ok(preflight) => {
                    app.reward_mint.preflight = Some(preflight);
                    app.reward_mint.confirmation = super::ConfirmChoice::Cancel;
                    app.reward_mint.error = None;
                    app.open(super::Page::RewardMintConfirm);
                }
                Err(error) => app.reward_mint.error = Some(error),
            }
        }
        ServiceEvent::RewardMintStarted { generation, result }
            if generation == app.reward_mint.generation =>
        {
            app.reward_mint.in_flight = false;
            match result {
                Ok(status) => {
                    app.reward_mint.status = Some(status);
                    app.reward_mint.rebroadcast_armed = false;
                    app.reward_mint.preflight = None;
                    app.reward_mint.error = None;
                    while app.route_stack.last() == Some(&super::Page::RewardMintConfirm) {
                        app.route_stack.pop();
                    }
                    if app.route_stack.last() != Some(&super::Page::Rewards) {
                        app.route_stack.push(super::Page::Rewards);
                    }
                    app.page = super::Page::Rewards;
                }
                Err(error) => app.reward_mint.error = Some(error),
            }
        }
        ServiceEvent::RewardMintLoaded { generation, result }
            if generation == app.reward_mint.generation =>
        {
            app.reward_mint.in_flight = false;
            match result {
                Ok(status) => {
                    app.reward_mint.status = status;
                    app.reward_mint.rebroadcast_armed = false;
                    app.reward_mint.error = None;
                }
                Err(error) => app.reward_mint.error = Some(error),
            }
        }
        ServiceEvent::RewardMintAdvanced { generation, result }
            if generation == app.reward_mint.generation =>
        {
            app.reward_mint.in_flight = false;
            match result {
                Ok(status) => {
                    app.reward_mint.status = Some(status);
                    app.reward_mint.rebroadcast_armed = false;
                    app.reward_mint.error = None;
                }
                Err(error) => app.reward_mint.error = Some(error),
            }
        }
        ServiceEvent::RewardMintRebroadcasted { generation, result }
            if generation == app.reward_mint.generation =>
        {
            app.reward_mint.in_flight = false;
            app.reward_mint.rebroadcast_armed = false;
            match result {
                Ok(status) => {
                    app.reward_mint.status = Some(status);
                    app.reward_mint.error = None;
                }
                Err(error) => app.reward_mint.error = Some(error),
            }
        }
        ServiceEvent::LegacyAssetsFinished { generation, result }
            if generation == app.legacy_generation =>
        {
            app.legacy_in_flight = false;
            match result {
                Ok(snapshot) => {
                    app.legacy_index = app.legacy_index.min(snapshot.coins.len().saturating_sub(1));
                    app.legacy_snapshot = Some(snapshot);
                    app.legacy_error = None;
                }
                Err(error) => {
                    app.legacy_snapshot = None;
                    app.legacy_index = 0;
                    app.legacy_error = Some(error);
                }
            }
        }
        ServiceEvent::LegacyMigrationLoaded { generation, result }
            if generation == app.legacy_migration.generation =>
        {
            app.legacy_migration.in_flight = false;
            match result {
                Ok(Some(status)) => {
                    app.legacy_migration.fingerprint = Some(status.source.fingerprint.clone());
                    app.legacy_migration.masked_identity = Some(status.source.masked_key.clone());
                    app.legacy_migration.step =
                        if matches!(status.phase, crate::domain::LegacyMigrationPhase::Complete) {
                            crate::app::LegacyMigrationStep::Files
                        } else {
                            crate::app::LegacyMigrationStep::Progress
                        };
                    app.legacy_migration.status = Some(status);
                    app.legacy_migration.error = None;
                }
                Ok(None) => {
                    app.legacy_migration.step = crate::app::LegacyMigrationStep::Destination;
                    app.legacy_migration.error = None;
                }
                Err(error) => app.legacy_migration.error = Some(error),
            }
        }
        ServiceEvent::LegacyMigrationPreflightFinished { generation, result }
            if generation == app.legacy_migration.generation =>
        {
            app.legacy_migration.in_flight = false;
            match result {
                Ok(preflight) => {
                    app.legacy_migration.fingerprint = Some(preflight.source.fingerprint.clone());
                    app.legacy_migration.masked_identity =
                        Some(preflight.source.masked_key.clone());
                    app.legacy_migration.preflight = Some(preflight);
                    app.legacy_migration.confirmation = ConfirmChoice::Cancel;
                    app.legacy_migration.step = crate::app::LegacyMigrationStep::Preflight;
                    app.legacy_migration.error = None;
                }
                Err(error) => app.legacy_migration.error = Some(error),
            }
        }
        ServiceEvent::LegacyMigrationStarted { generation, result }
        | ServiceEvent::LegacyMigrationAdvanced { generation, result }
        | ServiceEvent::LegacyMigrationRebroadcasted { generation, result }
            if generation == app.legacy_migration.generation =>
        {
            app.legacy_migration.in_flight = false;
            app.legacy_migration.recovery_armed = false;
            match result {
                Ok(status) => {
                    app.legacy_migration.step =
                        if matches!(status.phase, crate::domain::LegacyMigrationPhase::Complete) {
                            crate::app::LegacyMigrationStep::Files
                        } else {
                            crate::app::LegacyMigrationStep::Progress
                        };
                    app.legacy_migration.status = Some(status);
                    app.legacy_migration.error = None;
                }
                Err(error) => app.legacy_migration.error = Some(error),
            }
        }
        ServiceEvent::LegacyMigrationFileLoaded {
            generation,
            kind,
            result,
        } if generation == app.legacy_migration.generation => {
            app.legacy_migration.in_flight = false;
            app.legacy_migration.file_confirmation = None;
            match result {
                Ok(contents) => {
                    app.legacy_migration.viewer.replace(contents.into_string());
                    app.legacy_migration.viewer_kind = Some(kind);
                    app.legacy_migration.error = None;
                }
                Err(error) => app.legacy_migration.error = Some(error),
            }
        }
        ServiceEvent::ConfidentialWalletFinished { generation, result }
            if generation == app.confidential_wallet.generation =>
        {
            app.confidential_wallet.in_flight = false;
            match result {
                Ok(snapshot) => {
                    app.confidential_wallet.coin_index = app
                        .confidential_wallet
                        .coin_index
                        .min(snapshot.coins.len().saturating_sub(1));
                    app.confidential_wallet.snapshot = Some(snapshot);
                    app.confidential_wallet.error = None;
                }
                Err(error) => {
                    app.confidential_wallet.snapshot = None;
                    app.confidential_wallet.coin_index = 0;
                    app.confidential_wallet.error = Some(error);
                }
            }
        }
        ServiceEvent::PendingEscrowsFinished { generation, result }
            if generation == app.pending_escrows.generation =>
        {
            app.pending_escrows.in_flight = false;
            match result {
                Ok(snapshot) => {
                    app.pending_escrows.index = app
                        .pending_escrows
                        .index
                        .min(snapshot.escrows.len().saturating_sub(1));
                    app.pending_escrows.snapshot = Some(snapshot);
                    app.pending_escrows.error = None;
                }
                Err(error) => {
                    app.pending_escrows.snapshot = None;
                    app.pending_escrows.index = 0;
                    app.pending_escrows.error = Some(error);
                }
            }
        }
        ServiceEvent::WalletPreflightFinished { generation, result }
            if generation == app.wallet.generation =>
        {
            app.wallet.in_flight = false;
            match result {
                Ok(preflight) => {
                    app.wallet.preflight = Some(preflight);
                    app.wallet.confirmation = ConfirmChoice::Cancel;
                    app.wallet.error = None;
                    app.open(crate::app::Page::WalletConfirm);
                }
                Err(error) => {
                    app.wallet.preflight = None;
                    app.wallet.error = Some(error);
                }
            }
        }
        ServiceEvent::WalletExecutionFinished { generation, result }
            if generation == app.wallet.generation =>
        {
            app.wallet.in_flight = false;
            match result {
                Ok(result) => {
                    app.wallet.result = Some(result);
                    app.wallet.error = None;
                }
                Err(error) => {
                    app.wallet.result = None;
                    app.wallet.error = Some(error);
                }
            }
            app.queue_confidential_wallet();
            app.queue_pending_escrows();
            app.open(crate::app::Page::WalletResult);
        }
        ServiceEvent::HypergraphFinished { generation, result }
            if generation == app.hypergraph.generation =>
        {
            app.hypergraph.in_flight = false;
            match result {
                Ok(snapshot) => {
                    app.hypergraph.entry_index = app
                        .hypergraph
                        .entry_index
                        .min(snapshot.entries.len().saturating_sub(1));
                    app.hypergraph.snapshot = Some(snapshot);
                    app.hypergraph.error = None;
                }
                Err(error) => {
                    app.hypergraph.snapshot = None;
                    app.hypergraph.entry_index = 0;
                    app.hypergraph.error = Some(error);
                }
            }
        }
        ServiceEvent::ShardsFinished { generation, result }
            if generation == app.shards.generation =>
        {
            app.shards.detail_scroll = 0;
            app.shards.in_flight = false;
            match result {
                Ok(snapshot) => {
                    app.shards.index = app
                        .shards
                        .index
                        .min(snapshot.shards.len().saturating_sub(1));
                    app.shards.snapshot = Some(snapshot);
                    app.shards.error = None;
                }
                Err(error) => {
                    app.shards.snapshot = None;
                    app.shards.index = 0;
                    app.shards.error = Some(error);
                }
            }
        }
        ServiceEvent::ArchiveShardsFinished { generation, result }
            if generation == app.archive_shards.generation =>
        {
            app.archive_shards.in_flight = false;
            match result {
                Ok(snapshot) => {
                    app.archive_shards.index = app
                        .archive_shards
                        .index
                        .min(snapshot.archives.len().saturating_sub(1));
                    app.archive_shards.snapshot = Some(snapshot);
                    app.archive_shards.error = None;
                    app.archive_shards.detail = false;
                }
                Err(error) => {
                    app.archive_shards.snapshot = None;
                    app.archive_shards.index = 0;
                    app.archive_shards.detail = false;
                    app.archive_shards.error = Some(error);
                }
            }
        }
        ServiceEvent::AliasMutationFinished { generation, result }
            if generation == app.alias_mutation_generation =>
        {
            let succeeded = result.is_ok();
            match result {
                Ok(result) => app.finish_operation(result.message, Some(true)),
                Err(error) => app.finish_operation(error, Some(false)),
            }
            if succeeded {
                app.queue_aliases();
            }
        }
        ServiceEvent::KeyMutationFinished { generation, result }
            if generation == app.key_mutation_generation =>
        {
            let succeeded = result.is_ok();
            match result {
                Ok(result) => app.finish_operation(result.message, Some(true)),
                Err(error) => app.finish_operation(error, Some(false)),
            }
            if succeeded {
                app.queue_keys();
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use crate::event::ServiceEvent;

    use super::{reduce_service, App};

    #[test]
    fn service_errors_remain_raw_in_application_state() {
        let mut app = App::default();

        reduce_service(
            &mut app,
            ServiceEvent::RefreshFinished {
                generation: 0,
                result: Err("Cannot connect to node: connection refused".into()),
            },
        );

        assert_eq!(
            app.error.as_deref(),
            Some("Cannot connect to node: connection refused")
        );
    }
}
