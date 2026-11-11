use crate::i18n::UiText;
use crate::{
    allocation::{allocation_lifecycle, ActionTiming, AllocationAction},
    config::ConfigEntry,
    core::CoreStateSnapshot,
    domain::{
        protected_key_id, AliasMutation, AllocationStatus, HealthTarget, HypergraphKind,
        KeyMutation, LogExportLevel, LogExportOptions, ProverAction, CREATABLE_KEY_TYPES,
    },
    event::InputEvent,
    i18n::{allocation_lifecycle_status_label, format_allocation_action_hint, t, Locale},
    settings::{parse_port, ConsoleSettings, WebSettings},
};

use super::{navigation::*, state::*};

#[cfg(test)]
use super::service_reducer::reduce_service;

#[cfg(test)]
use crate::domain::{
    AliasSnapshot, DiagnosticsIdentitySnapshot, HypergraphSnapshot, KeySnapshot,
    LegacyAssetsSnapshot, LogSnapshot, NodeSnapshot, PeerAddressReport, RewardSnapshot,
    ShardInfoSnapshot,
};

impl App {
    pub const fn locale(&self) -> Locale {
        self.locale
    }

    pub fn set_console_settings(&mut self, settings: ConsoleSettings) {
        self.other.settings = settings.web;
        self.locale = settings.language;
        self.other.port_input = settings.web.port.to_string();
        self.other.editing_port = false;
        self.other.in_flight = false;
        self.other.error = None;
        self.other.previous_locale = None;
        self.other.queued = None;
        self.other.generation = self.other.generation.wrapping_add(1);
    }

    pub fn set_console_settings_load_error(&mut self, error: String) {
        self.set_console_settings(ConsoleSettings::default());
        self.other.error = Some(error);
    }

    pub fn take_console_settings_request(&mut self) -> Option<ConsoleSettingsRequest> {
        self.other.queued.take()
    }

    pub fn queue_read_rpc_load(&mut self, config_dir: std::path::PathBuf) {
        self.other.read_rpc_generation = self.other.read_rpc_generation.wrapping_add(1);
        self.other.read_rpc_config_dir = Some(config_dir.clone());
        self.other.read_rpc_override = None;
        self.other.effective_read_rpc.clear();
        self.other.read_rpc_input.clear();
        self.other.editing_read_rpc = false;
        self.other.read_rpc_in_flight = true;
        self.other.error = None;
        self.other.queued_read_rpc = Some(ReadRpcRequest {
            generation: self.other.read_rpc_generation,
            config_dir,
            value: None,
        });
    }

    pub fn take_read_rpc_request(&mut self) -> Option<ReadRpcRequest> {
        self.other.queued_read_rpc.take()
    }

    pub fn apply_read_rpc_result(
        &mut self,
        generation: u64,
        result: Result<ReadRpcSelection, String>,
    ) {
        if generation != self.other.read_rpc_generation || !self.other.read_rpc_in_flight {
            return;
        }
        self.other.read_rpc_in_flight = false;
        match result {
            Ok(selection) => {
                self.other.read_rpc_input = selection.override_address.clone().unwrap_or_default();
                self.other.read_rpc_override = selection.override_address;
                self.other.effective_read_rpc = selection.effective_address;
                self.other.error = None;
            }
            Err(error) => self.other.error = Some(error),
        }
    }

    fn queue_read_rpc_save(&mut self) {
        let Some(config_dir) = self.other.read_rpc_config_dir.clone() else {
            self.other.error =
                Some(crate::i18n::t(self.locale(), UiText::SelectANodeConfigurationFirst).into());
            return;
        };
        let value = self.other.read_rpc_input.trim();
        let value = (!value.is_empty()).then(|| value.to_owned());
        self.other.read_rpc_generation = self.other.read_rpc_generation.wrapping_add(1);
        self.other.read_rpc_in_flight = true;
        self.other.editing_read_rpc = false;
        self.other.error = None;
        self.other.queued_read_rpc = Some(ReadRpcRequest {
            generation: self.other.read_rpc_generation,
            config_dir,
            value: Some(value),
        });
    }

    pub fn apply_console_settings_result(
        &mut self,
        generation: u64,
        result: Result<ConsoleSettings, String>,
    ) {
        if generation != self.other.generation || !self.other.in_flight {
            return;
        }
        self.other.in_flight = false;
        self.other.queued = None;
        match result {
            Ok(settings) => {
                self.other.settings = settings.web;
                self.locale = settings.language;
                self.other.error = None;
            }
            Err(error) => {
                if let Some(previous) = self.other.previous_locale {
                    self.locale = previous;
                }
                self.other.error = Some(error);
            }
        }
        self.other.previous_locale = None;
        self.other.port_input = self.other.settings.port.to_string();
    }

    fn queue_console_settings(&mut self, settings: ConsoleSettings) {
        if self.other.in_flight {
            return;
        }
        self.other.previous_locale = Some(self.locale);
        self.locale = settings.language;
        self.other.generation = self.other.generation.wrapping_add(1);
        self.other.in_flight = true;
        self.other.editing_port = false;
        self.other.error = None;
        self.other.queued = Some(ConsoleSettingsRequest {
            generation: self.other.generation,
            settings,
        });
    }

    pub(crate) fn narrow_prover_detail_open(&self) -> bool {
        self.page == Page::Prover
            && self.viewport_width < 70
            && self.expanded_allocation == Some(self.allocation_index)
    }

    fn scroll_prover_detail(&mut self, down: bool, step: usize) {
        let limit = crate::ui::prover::detail_scroll_limit(self);
        let current = self.prover_detail_scroll.min(limit);
        self.prover_detail_scroll = if down {
            current.saturating_add(step).min(limit)
        } else {
            current.saturating_sub(step)
        };
    }

    fn scroll_shard_detail(&mut self, down: bool, step: usize) {
        let (limit, visible) = crate::ui::shards::detail_scroll_bounds(self);
        let step = step.min(visible.max(1));
        let current = self.shards.detail_scroll.min(limit);
        self.shards.detail_scroll = if down {
            current.saturating_add(step).min(limit)
        } else {
            current.saturating_sub(step)
        };
    }

    pub fn page(&self) -> Page {
        self.page
    }

    pub fn open(&mut self, page: Page) {
        if self.page != page && (self.page == Page::Shards || page == Page::Shards) {
            self.shards.detail_scroll = 0;
        }
        if self.page != page && (self.page == Page::Prover || page == Page::Prover) {
            self.prover_detail_scroll = 0;
        }
        if page == Page::Health && self.page != page {
            self.health.detail = false;
        }
        if self.route_stack.is_empty() {
            self.route_stack.push(self.page);
        }
        if self.page != page {
            self.narrow_read_detail = false;
            self.route_stack.push(page);
            self.page = page;
        }
    }

    pub fn begin_action(&mut self, action: ProverAction) {
        if self.operation_in_flight {
            return;
        }
        self.clear_pending_confirmation();
        self.pending_action = Some(action);
        self.confirm_choice = ConfirmChoice::Cancel;
        self.open(Page::Confirm);
    }

    pub fn set_configs(&mut self, configs: Vec<ConfigEntry>) {
        let selected = configs
            .iter()
            .position(|entry| entry.is_default)
            .unwrap_or_default();
        self.configs = configs;
        self.config_index = selected;
        self.active_config_index = selected;
        self.queued_config_index = None;
    }

    pub fn active_config(&self) -> Option<&ConfigEntry> {
        self.configs.get(self.active_config_index)
    }

    pub fn require_config_path(&mut self, input: String, error: Option<String>) {
        self.config_path_input = input;
        self.config_path_error = error;
        self.config_path_required = true;
        self.queued_config_path = None;
        self.route_stack.clear();
        self.route_stack.push(Page::ConfigPath);
        self.page = Page::ConfigPath;
    }

    pub fn open_config_path(&mut self) {
        self.config_path_input = self
            .active_config()
            .map(|entry| entry.path.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.config_path_error = None;
        self.config_path_required = false;
        self.queued_config_path = None;
        self.open(Page::ConfigPath);
    }

    pub fn take_config_path_request(&mut self) -> Option<ConfigPathRequest> {
        self.queued_config_path.take()
    }

    pub fn apply_config_entry(&mut self, entry: ConfigEntry) {
        self.activate_config_entry(entry);
    }

    pub fn activate_config_entry(&mut self, entry: ConfigEntry) {
        let existing_index = self
            .configs
            .iter()
            .position(|existing| existing.path == entry.path);
        self.clear_config_scoped_state();
        self.config_path_input = entry.path.to_string_lossy().into_owned();
        if let Some(index) = existing_index {
            self.configs[index] = entry;
            self.config_index = index;
            self.active_config_index = index;
        } else {
            self.configs = vec![entry];
            self.config_index = 0;
            self.active_config_index = 0;
        }
        self.queued_config_index = None;
        self.config_path_error = None;
        self.config_path_required = false;
        self.queued_config_path = None;
        self.route_stack.clear();
        self.route_stack.push(Page::Home);
        self.page = Page::Home;
    }

    pub(crate) fn apply_core_state_projection(
        &mut self,
        state: &CoreStateSnapshot,
    ) -> (bool, bool) {
        let config_changed = match (&state.active_config, self.active_config()) {
            (Some(core), Some(active)) => core.path != active.path,
            (Some(_), None) => true,
            (None, Some(_)) => true,
            (None, None) => false,
        };

        if config_changed {
            if let Some(entry) = state.active_config.clone() {
                self.activate_config_entry(entry);
            } else {
                self.clear_config_scoped_state();
                self.configs.clear();
                self.config_index = 0;
                self.active_config_index = 0;
            }
        } else if let (Some(core), Some(active)) =
            (state.active_config.as_ref(), self.active_config())
        {
            if core != active {
                self.configs[self.active_config_index] = core.clone();
            }
        }

        let snapshot_changed = self.snapshot != state.node_snapshot
            || self.applied_snapshot_revision != state.snapshot_revision;
        if snapshot_changed {
            self.applied_snapshot_revision = state.snapshot_revision;
            self.loading = false;
            self.prover_detail_scroll = 0;
            self.cancel_pending_prover_confirmation_for_refresh();
            self.snapshot = state.node_snapshot.clone();
            self.error = None;
            if let Some(snapshot) = &self.snapshot {
                self.prover_operations.reconcile(snapshot);
                self.allocation_index = self
                    .allocation_index
                    .min(snapshot.allocations.len().saturating_sub(1));
                self.expanded_allocation = None;
                self.action_index = self
                    .action_index
                    .min(available_actions(self).len().saturating_sub(1));
            }
        }

        if config_changed && state.node_snapshot.is_some() {
            self.queue_reward_mint_load();
        }

        (config_changed, snapshot_changed)
    }

    pub fn apply_config_path_error(&mut self, error: String) {
        self.config_path_error = Some(error);
        self.queued_config_path = None;
        self.page = Page::ConfigPath;
        if self.route_stack.last() != Some(&Page::ConfigPath) {
            self.route_stack.push(Page::ConfigPath);
        }
    }

    pub fn take_config_selection(&mut self) -> Option<ConfigEntry> {
        let index = self.queued_config_index.take()?;
        if index == self.active_config_index {
            return None;
        }
        self.configs.get(index).cloned()
    }

    pub fn take_operation(&mut self) -> Option<OperationRequest> {
        self.queued_operation.take()
    }

    pub fn take_prover_operation(&mut self) -> Option<ProverOperationRequest> {
        self.prover_operations.take_request()
    }

    pub fn take_diagnostics_request(&mut self) -> Option<DiagnosticsRequest> {
        self.queued_diagnostics.take()
    }

    pub fn take_diagnostics_identity_request(&mut self) -> Option<DiagnosticsIdentityRequest> {
        self.diagnostics_identity.queued.take()
    }

    pub fn set_diagnostics_identity_path(&mut self, home: &std::path::Path) {
        if self.diagnostics_identity.path.is_empty() {
            self.diagnostics_identity.path = home
                .join("qclient-console-diagnostics.key")
                .to_string_lossy()
                .into_owned();
        }
    }

    pub fn take_peer_resolve_request(&mut self) -> Option<PeerResolveRequest> {
        self.peer_resolve.queued.take()
    }

    pub fn take_health_request(&mut self) -> Option<HealthRequest> {
        self.health.queued.take()
    }

    pub fn take_runtime_request(&mut self) -> Option<RuntimeRequest> {
        self.runtime.queued.take()
    }

    pub fn take_log_request(&mut self) -> Option<LogRequest> {
        self.queued_log.take()
    }

    pub fn take_log_export_request(&mut self) -> Option<LogExportRequest> {
        self.log_export.queued_prepare.take()
    }

    pub fn take_log_export_finish(&mut self) -> Option<LogExportFinishRequest> {
        self.log_export.queued_finish.take()
    }

    pub fn take_key_request(&mut self) -> Option<KeyRequest> {
        self.queued_key.take()
    }

    pub fn take_key_mutation(&mut self) -> Option<KeyMutationRequest> {
        self.queued_key_mutation.take()
    }

    pub fn take_alias_request(&mut self) -> Option<AliasRequest> {
        self.queued_alias.take()
    }

    pub fn take_alias_mutation(&mut self) -> Option<AliasMutationRequest> {
        self.queued_alias_mutation.take()
    }

    pub fn take_reward_request(&mut self) -> Option<RewardRequest> {
        self.queued_reward.take()
    }

    pub fn take_reward_mint_preflight(&mut self) -> Option<RewardMintReadRequest> {
        self.reward_mint.queued_preflight.take()
    }

    pub fn take_reward_mint_start(&mut self) -> Option<RewardMintStartRequest> {
        self.reward_mint.queued_start.take()
    }

    pub fn take_reward_mint_load(&mut self) -> Option<RewardMintReadRequest> {
        self.reward_mint.queued_load.take()
    }

    pub fn take_reward_mint_advance(&mut self) -> Option<RewardMintReadRequest> {
        self.reward_mint.queued_advance.take()
    }

    pub fn take_reward_mint_rebroadcast(&mut self) -> Option<RewardMintReadRequest> {
        self.reward_mint.queued_rebroadcast.take()
    }

    pub fn reward_mint_should_poll(&self) -> bool {
        self.reward_mint
            .status
            .as_ref()
            .is_some_and(|status| status.next_poll_in_secs.is_some() && !status.phase.is_complete())
            && !self.reward_mint.in_flight
    }

    pub fn take_legacy_request(&mut self) -> Option<LegacyAssetsRequest> {
        self.queued_legacy.take()
    }

    pub fn take_legacy_migration_load(&mut self) -> Option<LegacyMigrationReadWork> {
        if !std::mem::take(&mut self.legacy_migration.queued_load) {
            return None;
        }
        Some(LegacyMigrationReadWork {
            generation: self.legacy_migration.generation,
            fingerprint: self.legacy_migration.fingerprint.clone()?,
        })
    }

    pub fn take_legacy_migration_preflight(
        &mut self,
    ) -> Option<Result<LegacyMigrationPreflightWork, String>> {
        if !std::mem::take(&mut self.legacy_migration.queued_preflight) {
            return None;
        }
        let source = match crate::service::legacy_migration::SecretLegacyKey::parse(
            self.legacy_migration.secret.expose(),
        ) {
            Ok(source) => source,
            Err(error) => return Some(Err(error.to_string())),
        };
        self.legacy_migration.secret.clear();
        Some(Ok(LegacyMigrationPreflightWork {
            generation: self.legacy_migration.generation,
            request: crate::service::legacy_migration::LegacyMigrationRequest::new(
                source,
                self.legacy_migration.destination(),
                true,
            ),
        }))
    }

    pub fn take_legacy_migration_start(&mut self) -> Option<LegacyMigrationStartWork> {
        if !std::mem::take(&mut self.legacy_migration.queued_start) {
            return None;
        }
        Some(LegacyMigrationStartWork {
            generation: self.legacy_migration.generation,
            approved: self.legacy_migration.preflight.clone()?,
        })
    }

    pub fn take_legacy_migration_advance(
        &mut self,
    ) -> Option<Result<LegacyMigrationAdvanceWork, String>> {
        if !std::mem::take(&mut self.legacy_migration.queued_advance) {
            return None;
        }
        let source = if std::mem::take(&mut self.legacy_migration.queued_advance_with_source) {
            match crate::service::legacy_migration::SecretLegacyKey::parse(
                self.legacy_migration.secret.expose(),
            ) {
                Ok(source) => {
                    self.legacy_migration.secret.clear();
                    Some(source)
                }
                Err(error) => return Some(Err(error.to_string())),
            }
        } else {
            None
        };
        Some(Ok(LegacyMigrationAdvanceWork {
            generation: self.legacy_migration.generation,
            fingerprint: self.legacy_migration.fingerprint.clone()?,
            source,
        }))
    }

    pub fn take_legacy_migration_rebroadcast(&mut self) -> Option<LegacyMigrationReadWork> {
        if !std::mem::take(&mut self.legacy_migration.queued_rebroadcast) {
            return None;
        }
        Some(LegacyMigrationReadWork {
            generation: self.legacy_migration.generation,
            fingerprint: self.legacy_migration.fingerprint.clone()?,
        })
    }

    pub fn take_legacy_migration_file(&mut self) -> Option<LegacyMigrationFileWork> {
        let status = self.legacy_migration.status.as_ref()?;
        if !status.source_key_persisted {
            self.legacy_migration.queued_file = None;
            return None;
        }
        Some(LegacyMigrationFileWork {
            generation: self.legacy_migration.generation,
            recipient_config_dir: status.recipient_config_dir.clone(),
            kind: self.legacy_migration.queued_file.take()?,
        })
    }

    pub fn legacy_migration_should_poll(&self) -> bool {
        self.legacy_migration.step == LegacyMigrationStep::Progress
            && !self.legacy_migration.in_flight
            && self.legacy_migration.status.as_ref().is_some_and(|status| {
                matches!(
                    status.phase,
                    crate::domain::LegacyMigrationPhase::Prepared
                        | crate::domain::LegacyMigrationPhase::Migrating { .. }
                        | crate::domain::LegacyMigrationPhase::AwaitingChainConfirmation { .. }
                        | crate::domain::LegacyMigrationPhase::SubmissionUnknown
                )
            })
    }

    pub(crate) fn queue_legacy_migration_advance(&mut self) {
        if !self.legacy_migration_should_poll() {
            return;
        }
        self.legacy_migration.generation = self.legacy_migration.generation.wrapping_add(1);
        self.legacy_migration.in_flight = true;
        self.legacy_migration.queued_advance = true;
    }

    pub fn take_confidential_wallet_request(&mut self) -> Option<ConfidentialWalletRequest> {
        self.confidential_wallet.queued.take()
    }

    pub fn take_pending_escrows_request(&mut self) -> Option<PendingEscrowsRequest> {
        self.pending_escrows.queued.take()
    }

    pub fn take_wallet_preflight_request(&mut self) -> Option<WalletPreflightRequest> {
        self.wallet.queued_preflight.take()
    }

    pub fn take_wallet_execute_request(&mut self) -> Option<WalletExecuteRequest> {
        self.wallet.queued_execute.take()
    }

    pub fn open_wallet_action(&mut self, action: crate::domain::WalletAction) {
        if self.wallet.in_flight {
            return;
        }
        self.wallet.action = action;
        self.wallet.form_field = 0;
        self.wallet.editing = false;
        self.wallet.confirmation = ConfirmChoice::Cancel;
        self.wallet.request = None;
        self.wallet.preflight = None;
        self.wallet.result = None;
        self.wallet.error = None;
        self.open(Page::WalletForm);
    }

    fn queue_wallet_preflight(&mut self) {
        if self.wallet.in_flight {
            return;
        }
        let amount = match self.wallet.action {
            crate::domain::WalletAction::Transfer
            | crate::domain::WalletAction::PendingTransfer => {
                match crate::domain::WalletAmount::parse(&self.wallet.form_amount) {
                    Ok(amount) => Some(amount),
                    Err(error) => {
                        self.wallet.error = Some(error);
                        return;
                    }
                }
            }
            crate::domain::WalletAction::Accept | crate::domain::WalletAction::Reject => None,
        };
        let maximum_fee_base_units = match self.wallet.form_maximum_fee.trim().parse::<u128>() {
            Ok(value) => value,
            Err(error) => {
                self.wallet.error =
                    Some(format!("Maximum fee must be a base-unit integer: {error}"));
                return;
            }
        };
        let explicit_fee_base_units = if self.wallet.form_explicit_fee.trim().is_empty() {
            None
        } else {
            match self.wallet.form_explicit_fee.trim().parse::<u128>() {
                Ok(value) => Some(value),
                Err(error) => {
                    self.wallet.error =
                        Some(format!("Explicit fee must be a base-unit integer: {error}"));
                    return;
                }
            }
        };
        if self.wallet.form_target.trim().is_empty() {
            self.wallet.error = Some("Recipient address is required".into());
            return;
        }
        let refund_after_global_frame = if self.wallet.action
            == crate::domain::WalletAction::PendingTransfer
            && !self.wallet.form_refund_frame.trim().is_empty()
        {
            match self.wallet.form_refund_frame.trim().parse::<u64>() {
                Ok(frame) => Some(frame),
                Err(error) => {
                    self.wallet.error = Some(format!("Refund frame must be an integer: {error}"));
                    return;
                }
            }
        } else {
            None
        };
        let request = crate::domain::WalletActionRequest {
            action: self.wallet.action,
            application_hex: self.wallet.form_application.trim().to_owned(),
            target: self.wallet.form_target.trim().to_owned(),
            amount,
            explicit_fee_base_units,
            maximum_fee_base_units,
            refund_after_global_frame,
        };
        self.wallet.generation = self.wallet.generation.wrapping_add(1);
        self.wallet.in_flight = true;
        self.wallet.error = None;
        self.wallet.request = Some(request.clone());
        self.wallet.queued_preflight = Some(WalletPreflightRequest {
            generation: self.wallet.generation,
            request,
        });
    }

    fn queue_wallet_execute(&mut self) {
        if self.wallet.in_flight {
            return;
        }
        let (Some(request), Some(approved)) =
            (self.wallet.request.clone(), self.wallet.preflight.clone())
        else {
            self.wallet.error = Some("Wallet preflight is missing".into());
            return;
        };
        self.wallet.generation = self.wallet.generation.wrapping_add(1);
        self.wallet.in_flight = true;
        self.wallet.error = None;
        self.wallet.queued_execute = Some(WalletExecuteRequest {
            generation: self.wallet.generation,
            request,
            approved,
        });
        self.open(Page::WalletProgress);
    }

    pub fn take_hypergraph_request(&mut self) -> Option<HypergraphRequest> {
        self.hypergraph.queued.take()
    }

    pub fn take_shard_request(&mut self) -> Option<ShardRequest> {
        self.shards.queued.take()
    }

    pub(crate) fn take_node_refresh_request(&mut self) -> bool {
        std::mem::take(&mut self.node_refresh_requested)
    }

    pub fn take_archive_request(&mut self) -> Option<ArchiveShardRequest> {
        self.archive_shards.queued.take()
    }

    pub fn take_capability_request(&mut self) -> Option<CapabilityRequest> {
        self.capabilities.queued.take()
    }

    pub fn is_text_editing(&self) -> bool {
        self.page == Page::ConfigPath
            || (self.page == Page::Other
                && (self.other.editing_port || self.other.editing_read_rpc))
            || (self.page == Page::Diagnostics && self.diagnostics_editing)
            || (self.page == Page::DiagnosticsIdentity && self.diagnostics_identity.editing)
            || (self.page == Page::RemoteHealth && self.remote_health.editing)
            || (self.page == Page::PeerResolve && self.peer_resolve.editing)
            || (self.page == Page::AliasEdit && self.alias_form_editing)
            || (self.page == Page::KeyCreate && self.key_form_editing)
            || (self.page == Page::Hypergraph && self.hypergraph.editing)
            || (self.page == Page::WalletForm && self.wallet.editing)
            || (self.page == Page::CapabilityForm && self.capabilities.editing)
            || (self.page == Page::LegacyAssets
                && matches!(
                    self.legacy_migration.step,
                    LegacyMigrationStep::Identity | LegacyMigrationStep::Progress
                )
                && (self.legacy_migration.step == LegacyMigrationStep::Identity
                    || self.legacy_migration.status.as_ref().is_some_and(|status| {
                        matches!(
                            status.phase,
                            crate::domain::LegacyMigrationPhase::AwaitingSourceKey
                        )
                    })))
            || (self.page == Page::LegacyAssets
                && self.legacy_migration.step == LegacyMigrationStep::Destination
                && self.legacy_migration.destination == LegacyDestinationChoice::ExistingConfig)
    }

    fn queue_log(&mut self) {
        let cores = log_core_ids(self);
        self.log_core_index %= cores.len();
        self.log_generation = self.log_generation.wrapping_add(1);
        self.log_in_flight = true;
        self.log_scroll = 0;
        self.log_error = None;
        self.queued_log = Some(LogRequest {
            generation: self.log_generation,
            core_id: cores[self.log_core_index],
            lines: 500,
        });
    }

    fn queue_log_export_preview(&mut self) {
        if self.log_export.in_flight {
            return;
        }
        self.log_export.generation = self.log_export.generation.wrapping_add(1);
        self.log_export.in_flight = true;
        self.log_export.preview = None;
        self.log_export.error = None;
        self.log_export.choice = ConfirmChoice::Cancel;
        self.log_export.queued_prepare = Some(LogExportRequest {
            generation: self.log_export.generation,
            options: LogExportOptions {
                window_seconds: LOG_EXPORT_WINDOWS[self.log_export.window_index],
                min_level: LogExportLevel::ALL[self.log_export.level_index],
            },
        });
    }

    fn queue_log_export_finish(&mut self, commit: bool) {
        if self.log_export.in_flight {
            return;
        }
        let Some(preview) = self.log_export.preview.take() else {
            return;
        };
        self.log_export.in_flight = true;
        self.log_export.error = None;
        self.log_export.queued_finish = Some(LogExportFinishRequest {
            generation: self.log_export.generation,
            id: preview.id,
            commit,
        });
    }

    pub(super) fn queue_keys(&mut self) {
        if self.key_in_flight {
            return;
        }
        self.key_generation = self.key_generation.wrapping_add(1);
        self.key_in_flight = true;
        self.key_error = None;
        self.queued_key = Some(KeyRequest {
            generation: self.key_generation,
        });
    }

    pub(super) fn queue_aliases(&mut self) {
        if self.alias_in_flight {
            return;
        }
        self.alias_generation = self.alias_generation.wrapping_add(1);
        self.alias_in_flight = true;
        self.alias_error = None;
        self.queued_alias = Some(AliasRequest {
            generation: self.alias_generation,
        });
    }

    fn queue_rewards(&mut self) {
        if self.reward_in_flight {
            return;
        }
        self.reward_generation = self.reward_generation.wrapping_add(1);
        self.reward_in_flight = true;
        self.reward_error = None;
        self.queued_reward = Some(RewardRequest {
            generation: self.reward_generation,
        });
    }

    fn queue_reward_mint_preflight(&mut self) {
        if self.reward_mint.in_flight || self.reward_mint.status.is_some() {
            return;
        }
        self.reward_mint.generation = self.reward_mint.generation.wrapping_add(1);
        self.reward_mint.in_flight = true;
        self.reward_mint.error = None;
        self.reward_mint.queued_preflight = Some(RewardMintReadRequest {
            generation: self.reward_mint.generation,
        });
    }

    fn queue_reward_mint_start(&mut self) {
        if self.reward_mint.in_flight {
            return;
        }
        let Some(approved) = self.reward_mint.preflight.clone() else {
            return;
        };
        self.reward_mint.generation = self.reward_mint.generation.wrapping_add(1);
        self.reward_mint.in_flight = true;
        self.reward_mint.error = None;
        self.reward_mint.queued_start = Some(RewardMintStartRequest {
            generation: self.reward_mint.generation,
            approved,
        });
    }

    pub(crate) fn queue_reward_mint_load(&mut self) {
        if self.reward_mint.in_flight || self.reward_mint.status.is_some() {
            return;
        }
        self.reward_mint.generation = self.reward_mint.generation.wrapping_add(1);
        self.reward_mint.in_flight = true;
        self.reward_mint.error = None;
        self.reward_mint.queued_load = Some(RewardMintReadRequest {
            generation: self.reward_mint.generation,
        });
    }

    pub(crate) fn queue_reward_mint_advance(&mut self) {
        if !self.reward_mint_should_poll() {
            return;
        }
        self.reward_mint.generation = self.reward_mint.generation.wrapping_add(1);
        self.reward_mint.in_flight = true;
        self.reward_mint.queued_advance = Some(RewardMintReadRequest {
            generation: self.reward_mint.generation,
        });
    }

    fn queue_reward_mint_rebroadcast(&mut self) {
        if self.reward_mint.in_flight
            || !self
                .reward_mint
                .status
                .as_ref()
                .is_some_and(|status| status.can_rebroadcast)
        {
            return;
        }
        self.reward_mint.generation = self.reward_mint.generation.wrapping_add(1);
        self.reward_mint.in_flight = true;
        self.reward_mint.error = None;
        self.reward_mint.rebroadcast_armed = false;
        self.reward_mint.queued_rebroadcast = Some(RewardMintReadRequest {
            generation: self.reward_mint.generation,
        });
    }

    fn queue_legacy_assets(&mut self) {
        if self.legacy_in_flight {
            return;
        }
        self.legacy_generation = self.legacy_generation.wrapping_add(1);
        self.legacy_in_flight = true;
        self.legacy_error = None;
        self.queued_legacy = Some(LegacyAssetsRequest {
            generation: self.legacy_generation,
        });
    }

    fn begin_legacy_migration(&mut self) {
        self.legacy_migration.clear_and_invalidate();
        self.legacy_migration.step = LegacyMigrationStep::Identity;
    }

    fn validate_legacy_identity_and_load(&mut self) {
        match crate::service::legacy_migration::LegacyIdentity::parse(
            self.legacy_migration.secret.expose(),
        ) {
            Ok(identity) => {
                self.legacy_migration.masked_identity = Some(identity.summary().masked_key.clone());
                self.legacy_migration.fingerprint = Some(identity.summary().fingerprint.clone());
                self.legacy_migration.error = None;
                self.legacy_migration.in_flight = true;
                self.legacy_migration.generation = self.legacy_migration.generation.wrapping_add(1);
                self.legacy_migration.queued_load = true;
            }
            Err(error) => self.legacy_migration.error = Some(error.to_string()),
        }
    }

    fn queue_legacy_migration_preflight(&mut self) {
        if self.legacy_migration.in_flight
            || (self.legacy_migration.destination == LegacyDestinationChoice::ExistingConfig
                && self.legacy_migration.existing_path.trim().is_empty())
        {
            return;
        }
        self.legacy_migration.generation = self.legacy_migration.generation.wrapping_add(1);
        self.legacy_migration.in_flight = true;
        self.legacy_migration.error = None;
        self.legacy_migration.queued_preflight = true;
    }

    fn queue_legacy_migration_start(&mut self) {
        if self.legacy_migration.in_flight || self.legacy_migration.preflight.is_none() {
            return;
        }
        self.legacy_migration.generation = self.legacy_migration.generation.wrapping_add(1);
        self.legacy_migration.in_flight = true;
        self.legacy_migration.error = None;
        self.legacy_migration.queued_start = true;
    }

    fn queue_legacy_migration_source_resume(&mut self) {
        if self.legacy_migration.in_flight {
            return;
        }
        self.legacy_migration.generation = self.legacy_migration.generation.wrapping_add(1);
        self.legacy_migration.in_flight = true;
        self.legacy_migration.error = None;
        self.legacy_migration.queued_advance = true;
        self.legacy_migration.queued_advance_with_source = true;
    }

    fn queue_legacy_migration_rebroadcast(&mut self) {
        if self.legacy_migration.in_flight {
            return;
        }
        self.legacy_migration.generation = self.legacy_migration.generation.wrapping_add(1);
        self.legacy_migration.in_flight = true;
        self.legacy_migration.error = None;
        self.legacy_migration.recovery_armed = false;
        self.legacy_migration.queued_rebroadcast = true;
    }

    pub(super) fn queue_confidential_wallet(&mut self) {
        if self.confidential_wallet.in_flight {
            return;
        }
        self.confidential_wallet.generation = self.confidential_wallet.generation.wrapping_add(1);
        self.confidential_wallet.in_flight = true;
        self.confidential_wallet.error = None;
        self.confidential_wallet.queued = Some(ConfidentialWalletRequest {
            generation: self.confidential_wallet.generation,
        });
    }

    pub(super) fn queue_pending_escrows(&mut self) {
        if self.pending_escrows.in_flight {
            return;
        }
        self.pending_escrows.generation = self.pending_escrows.generation.wrapping_add(1);
        self.pending_escrows.in_flight = true;
        self.pending_escrows.error = None;
        self.pending_escrows.queued = Some(PendingEscrowsRequest {
            generation: self.pending_escrows.generation,
        });
    }

    fn queue_hypergraph(&mut self) {
        if self.hypergraph.in_flight {
            return;
        }
        let address = self.hypergraph.address.trim().to_string();
        if address.is_empty() {
            self.hypergraph.error =
                Some(crate::i18n::t(self.locale(), UiText::EnterA64ByteAddressOrLocalAlias).into());
            return;
        }
        self.hypergraph.generation = self.hypergraph.generation.wrapping_add(1);
        self.hypergraph.in_flight = true;
        self.hypergraph.snapshot = None;
        self.hypergraph.error = None;
        self.hypergraph.entry_index = 0;
        self.hypergraph.queued = Some(HypergraphRequest {
            generation: self.hypergraph.generation,
            kind: self.hypergraph.kind,
            address,
        });
    }

    fn queue_peer_resolve(&mut self) {
        if self.peer_resolve.in_flight {
            return;
        }
        let peer_id = self.peer_resolve.peer_id.trim().to_string();
        if peer_id.is_empty() {
            self.peer_resolve.error =
                Some(crate::i18n::t(self.locale(), UiText::EnterAPeerID).into());
            return;
        }
        self.peer_resolve.generation = self.peer_resolve.generation.wrapping_add(1);
        self.peer_resolve.in_flight = true;
        self.peer_resolve.report = None;
        self.peer_resolve.error = None;
        self.peer_resolve.index = 0;
        self.peer_resolve.queued = Some(PeerResolveRequest {
            generation: self.peer_resolve.generation,
            peer_id,
        });
    }

    fn queue_health_for_target(&mut self, target: HealthTarget) {
        if self.health.target != target {
            self.health.target = target;
            self.health.snapshot = None;
            self.health.error = None;
            self.health.index = 0;
            self.health.detail = false;
            self.health.in_flight = false;
            self.health.queued = None;
        }
        // A changed target always queues a new generation, invalidating the old read.
        self.queue_health();
    }

    fn queue_health(&mut self) {
        if self.health.in_flight {
            return;
        }
        self.health.generation = self.health.generation.wrapping_add(1);
        self.health.in_flight = true;
        self.health.error = None;
        self.health.queued = Some(HealthRequest {
            generation: self.health.generation,
            target: self.health.target.clone(),
        });
    }

    fn queue_diagnostics_identity(&mut self, create: bool) {
        if self.diagnostics_identity.in_flight {
            return;
        }
        let path = self.diagnostics_identity.path.trim().to_string();
        if path.is_empty() {
            self.diagnostics_identity.error = Some(
                crate::i18n::t(self.locale(), UiText::EnterTheDiagnosticIdentityFilePath).into(),
            );
            return;
        }
        self.diagnostics_identity.generation = self.diagnostics_identity.generation.wrapping_add(1);
        self.diagnostics_identity.in_flight = true;
        self.diagnostics_identity.error = None;
        self.diagnostics_identity.queued = Some(DiagnosticsIdentityRequest {
            generation: self.diagnostics_identity.generation,
            path,
            create,
        });
    }

    fn open_remote_health(&mut self) {
        let Some(report) = self.peer_resolve.report.as_ref() else {
            self.peer_resolve.error =
                Some(crate::i18n::t(self.locale(), UiText::QueryPeerAddressesFirst).into());
            return;
        };
        let Some(address) = report.addresses.get(self.peer_resolve.index) else {
            self.peer_resolve.error =
                Some(crate::i18n::t(self.locale(), UiText::SelectAnAddress).into());
            return;
        };
        if address.kind != "stream" {
            self.peer_resolve.error = Some(
                crate::i18n::t(
                    self.locale(),
                    UiText::HealthReportsCanOnlyBeReadThroughAStreamAddress,
                )
                .into(),
            );
            return;
        }
        let Some(host_port) = address.host_port.as_ref() else {
            self.peer_resolve.error = Some(
                crate::i18n::t(
                    self.locale(),
                    UiText::TheStreamAddressDoesNotContainAUsableHostAndPort,
                )
                .into(),
            );
            return;
        };
        let target = HealthTarget::Peer {
            peer_id: report.peer_id.clone(),
            address: host_port.clone(),
            identity_path: self
                .diagnostics_identity
                .snapshot
                .as_ref()
                .map(|identity| identity.path.clone()),
        };
        self.open(Page::Health);
        self.queue_health_for_target(target);
    }

    fn queue_direct_health(&mut self) {
        let Some(identity) = self.diagnostics_identity.snapshot.as_ref() else {
            self.remote_health.error = Some(
                crate::i18n::t(
                    self.locale(),
                    UiText::LoadOrCreateAnIdentityOnTheDiagnosticIdentityPageFirst,
                )
                .into(),
            );
            return;
        };
        let peer_id = self.remote_health.peer_id.trim();
        if peer_id.is_empty() {
            self.remote_health.error =
                Some(crate::i18n::t(self.locale(), UiText::EnterTheTargetPeerID).into());
            return;
        }
        let address = self.remote_health.address.trim();
        if address.is_empty() {
            self.remote_health.error = Some(
                crate::i18n::t(
                    self.locale(),
                    UiText::EnterTheTargetStreamAddressForExampleHost8336,
                )
                .into(),
            );
            return;
        }
        let target = HealthTarget::Peer {
            peer_id: peer_id.to_string(),
            address: address.to_string(),
            identity_path: Some(identity.path.clone()),
        };
        self.remote_health.error = None;
        self.open(Page::Health);
        self.queue_health_for_target(target);
    }

    fn queue_runtime(&mut self) {
        if self.runtime.in_flight {
            return;
        }
        self.runtime.generation = self.runtime.generation.wrapping_add(1);
        self.runtime.in_flight = true;
        self.runtime.error = None;
        self.runtime.queued = Some(RuntimeRequest {
            generation: self.runtime.generation,
        });
    }

    fn queue_shards(&mut self) {
        self.shards.detail_scroll = 0;
        if self.shards.in_flight {
            return;
        }
        self.shards.generation = self.shards.generation.wrapping_add(1);
        self.shards.in_flight = true;
        self.shards.snapshot = None;
        self.shards.error = None;
        self.shards.notice = None;
        self.shards.index = 0;
        self.shards.core_index = 0;
        self.shards.focus = ShardFocus::Directory;
        self.node_refresh_requested = true;
        self.shards.queued = Some(ShardRequest {
            generation: self.shards.generation,
            include_all: self.shards.scope.include_all(),
        });
    }

    fn queue_archive_shards(&mut self) {
        if self.archive_shards.in_flight {
            return;
        }
        self.archive_shards.generation = self.archive_shards.generation.wrapping_add(1);
        self.archive_shards.in_flight = true;
        self.archive_shards.snapshot = None;
        self.archive_shards.error = None;
        self.archive_shards.index = 0;
        self.archive_shards.detail = false;
        self.archive_shards.queued = Some(ArchiveShardRequest {
            generation: self.archive_shards.generation,
        });
    }

    fn open_page(&mut self, page: Page) {
        let capability_id = match page {
            Page::TokenTransfer => Some("token.transfer"),
            Page::DeployToken => Some("deploy.token"),
            _ => None,
        };
        if let Some(capability_id) = capability_id {
            begin_capability(self, capability_id);
            return;
        }
        if page == Page::Prover {
            self.expanded_allocation = None;
        }
        self.open(page);
        match page {
            Page::Runtime => self.queue_runtime(),
            Page::Health => self.queue_health_for_target(HealthTarget::SelfNode),
            Page::Logs => self.queue_log(),
            Page::Keys => self.queue_keys(),
            Page::Aliases => self.queue_aliases(),
            Page::Rewards => {
                self.queue_rewards();
                self.queue_reward_mint_load();
            }
            Page::LegacyAssets => self.begin_legacy_migration(),
            Page::ConfidentialWallet => self.queue_confidential_wallet(),
            Page::PendingEscrows => self.queue_pending_escrows(),
            Page::Shards => self.queue_shards(),
            Page::ArchiveShards => self.queue_archive_shards(),
            _ => {}
        }
    }

    fn begin_alias_mutation(&mut self, mutation: AliasMutation) {
        if self.operation_in_flight {
            return;
        }
        self.clear_pending_confirmation();
        self.pending_alias_mutation = Some(mutation);
        self.confirm_choice = ConfirmChoice::Cancel;
        self.open(Page::Confirm);
    }

    fn begin_key_mutation(&mut self, mutation: KeyMutation) {
        if self.operation_in_flight {
            return;
        }
        self.clear_pending_confirmation();
        self.pending_key_mutation = Some(mutation);
        self.confirm_choice = ConfirmChoice::Cancel;
        self.open(Page::Confirm);
    }

    fn back(&mut self) {
        self.shards.detail_scroll = 0;
        self.prover_detail_scroll = 0;
        if self.page == Page::Confirm {
            self.clear_pending_confirmation();
        }
        if self.route_stack.len() > 1 {
            self.route_stack.pop();
            self.page = *self.route_stack.last().expect("root route remains");
        } else {
            self.page = Page::Home;
        }
    }

    pub(super) fn finish_operation(&mut self, message: String, succeeded: Option<bool>) {
        self.clear_pending_confirmation();
        self.operation_message = Some(message);
        self.operation_succeeded = succeeded;
        self.operation_in_flight = false;
        while matches!(
            self.route_stack.last(),
            Some(
                Page::Confirm
                    | Page::ProverActions
                    | Page::AliasActions
                    | Page::AliasEdit
                    | Page::KeyActions
                    | Page::KeyCreate
            )
        ) {
            self.route_stack.pop();
        }
        if self.route_stack.is_empty() {
            self.route_stack.push(Page::Home);
        }
        self.route_stack.push(Page::OperationResult);
        self.page = Page::OperationResult;
    }

    fn clear_pending_confirmation(&mut self) {
        self.pending_action = None;
        self.pending_alias_mutation = None;
        self.pending_key_mutation = None;
    }

    fn return_after_background_operation(&mut self) {
        self.clear_pending_confirmation();
        while matches!(
            self.route_stack.last(),
            Some(Page::Confirm | Page::ProverActions)
        ) {
            self.route_stack.pop();
        }
        if self.route_stack.is_empty() {
            self.route_stack.push(Page::Home);
        }
        self.page = *self.route_stack.last().expect("root route remains");
    }

    pub(crate) fn cancel_pending_prover_confirmation_for_refresh(&mut self) {
        if self.operation_in_flight
            || (self.pending_action.is_none() && self.page != Page::ProverActions)
        {
            return;
        }
        self.clear_pending_confirmation();
        self.confirm_choice = ConfirmChoice::Cancel;
        while matches!(
            self.route_stack.last(),
            Some(Page::Confirm | Page::ProverActions)
        ) {
            self.route_stack.pop();
        }
        if self.route_stack.last() != Some(&Page::Prover) {
            self.route_stack.push(Page::Prover);
        }
        self.page = Page::Prover;
        self.expanded_allocation = None;
        self.prover_detail_scroll = 0;
    }

    pub(crate) fn begin_node_refresh(&mut self) {
        self.prover_detail_scroll = 0;
        self.refresh_generation = self.refresh_generation.wrapping_add(1);
        self.loading = true;
        self.error = None;
    }

    fn clear_config_scoped_state(&mut self) {
        self.prover_detail_scroll = 0;
        self.narrow_read_detail = false;
        self.refresh_generation = self.refresh_generation.wrapping_add(1);
        self.loading = false;
        self.node_refresh_requested = false;
        self.snapshot = None;
        self.error = None;
        self.allocation_index = 0;
        self.expanded_allocation = None;
        self.action_index = 0;

        self.operation_generation = self.operation_generation.wrapping_add(1);
        self.operation_in_flight = false;
        self.queued_operation = None;
        self.pending_action = None;
        self.confirm_choice = ConfirmChoice::Cancel;
        self.operation_message = None;
        self.operation_succeeded = None;
        self.prover_operations.clear_config_scoped();

        self.log_generation = self.log_generation.wrapping_add(1);
        self.log_in_flight = false;
        self.log_snapshot = None;
        self.log_error = None;
        self.queued_log = None;
        self.log_core_index = 0;
        self.log_scroll = 0;

        self.log_export.generation = self.log_export.generation.wrapping_add(1);
        self.log_export.in_flight = false;
        self.log_export.preview = None;
        self.log_export.error = None;
        self.log_export.queued_prepare = None;
        self.log_export.queued_finish = None;
        self.log_export.choice = ConfirmChoice::Cancel;

        self.key_generation = self.key_generation.wrapping_add(1);
        self.key_in_flight = false;
        self.keys.clear();
        self.key_index = 0;
        self.key_error = None;
        self.queued_key = None;
        self.key_action_index = 0;
        self.key_form_field = 0;
        self.key_form_editing = false;
        self.key_form_id.clear();
        self.key_form_type_index = 0;
        self.key_form_error = None;
        self.key_mutation_generation = self.key_mutation_generation.wrapping_add(1);
        self.queued_key_mutation = None;
        self.pending_key_mutation = None;

        self.alias_generation = self.alias_generation.wrapping_add(1);
        self.alias_in_flight = false;
        self.aliases.clear();
        self.alias_index = 0;
        self.alias_error = None;
        self.queued_alias = None;
        self.alias_action_index = 0;
        self.alias_form_field = 0;
        self.alias_form_editing = false;
        self.alias_form_name.clear();
        self.alias_form_address.clear();
        self.alias_form_kind.clear();
        self.alias_form_error = None;
        self.alias_mutation_generation = self.alias_mutation_generation.wrapping_add(1);
        self.queued_alias_mutation = None;
        self.pending_alias_mutation = None;

        self.reward_generation = self.reward_generation.wrapping_add(1);
        self.reward_in_flight = false;
        self.reward_snapshot = None;
        self.reward_error = None;
        self.queued_reward = None;
        self.reward_mint.generation = self.reward_mint.generation.wrapping_add(1);
        self.reward_mint.in_flight = false;
        self.reward_mint.preflight = None;
        self.reward_mint.status = None;
        self.reward_mint.error = None;
        self.reward_mint.confirmation = ConfirmChoice::Cancel;
        self.reward_mint.rebroadcast_armed = false;
        self.reward_mint.queued_preflight = None;
        self.reward_mint.queued_start = None;
        self.reward_mint.queued_load = None;
        self.reward_mint.queued_advance = None;
        self.reward_mint.queued_rebroadcast = None;

        self.legacy_generation = self.legacy_generation.wrapping_add(1);
        self.legacy_in_flight = false;
        self.legacy_snapshot = None;
        self.legacy_error = None;
        self.legacy_index = 0;
        self.queued_legacy = None;
        self.legacy_migration.clear_and_invalidate();

        self.confidential_wallet.generation = self.confidential_wallet.generation.wrapping_add(1);
        self.confidential_wallet.in_flight = false;
        self.confidential_wallet.snapshot = None;
        self.confidential_wallet.error = None;
        self.confidential_wallet.coin_index = 0;
        self.confidential_wallet.queued = None;

        self.pending_escrows.generation = self.pending_escrows.generation.wrapping_add(1);
        self.pending_escrows.in_flight = false;
        self.pending_escrows.snapshot = None;
        self.pending_escrows.error = None;
        self.pending_escrows.index = 0;
        self.pending_escrows.queued = None;

        let wallet_generation = self.wallet.generation.wrapping_add(1);
        self.wallet = WalletWorkflowState::default();
        self.wallet.generation = wallet_generation;

        self.hypergraph.generation = self.hypergraph.generation.wrapping_add(1);
        self.hypergraph.in_flight = false;
        self.hypergraph.snapshot = None;
        self.hypergraph.error = None;
        self.hypergraph.entry_index = 0;
        self.hypergraph.queued = None;

        self.shards.generation = self.shards.generation.wrapping_add(1);
        self.shards.in_flight = false;
        self.shards.snapshot = None;
        self.shards.error = None;
        self.shards.index = 0;
        self.shards.queued = None;
        self.node_refresh_requested = false;
        self.shards.detail_scroll = 0;

        self.archive_shards.generation = self.archive_shards.generation.wrapping_add(1);
        self.archive_shards.in_flight = false;
        self.archive_shards.snapshot = None;
        self.archive_shards.error = None;
        self.archive_shards.index = 0;
        self.archive_shards.detail = false;
        self.archive_shards.queued = None;

        self.peer_resolve.generation = self.peer_resolve.generation.wrapping_add(1);
        self.peer_resolve.in_flight = false;
        self.peer_resolve.report = None;
        self.peer_resolve.error = None;
        self.peer_resolve.index = 0;
        self.peer_resolve.queued = None;

        self.health.generation = self.health.generation.wrapping_add(1);
        self.health.in_flight = false;
        self.health.snapshot = None;
        self.health.error = None;
        self.health.index = 0;
        self.health.detail = false;
        self.health.queued = None;
    }
}

pub fn available_actions(app: &App) -> Vec<ProverAction> {
    if app.loading || app.error.is_some() {
        return Vec::new();
    }
    let Some(snapshot) = &app.snapshot else {
        return Vec::new();
    };
    let Some(epoch) = snapshot
        .epoch
        .as_ref()
        .filter(|epoch| epoch.length_frames > 0)
    else {
        return Vec::new();
    };
    let Some(allocation) = snapshot.allocations.get(app.allocation_index) else {
        return Vec::new();
    };
    let Ok(filter) = hex::decode(&allocation.filter_hex) else {
        return Vec::new();
    };
    if !(32..=66).contains(&filter.len()) {
        return Vec::new();
    }
    if app.prover_operations.conflicts_filter(&filter) {
        return Vec::new();
    }
    let lifecycle = allocation_lifecycle(allocation, snapshot.last_received_frame, Some(epoch));
    let worker = snapshot
        .workers
        .iter()
        .find(|worker| worker.filter_hex == allocation.filter_hex);
    if matches!(
        allocation.status,
        AllocationStatus::Rejected | AllocationStatus::Kicked | AllocationStatus::Historic
    ) {
        if snapshot.worker_info_error.is_some() {
            return Vec::new();
        }
        return worker.map_or_else(Vec::new, |worker| {
            vec![ProverAction::Join {
                filters: vec![filter],
                delegate: Vec::new(),
                worker_ids: vec![worker.core_id],
            }]
        });
    }

    let Some(hint) = lifecycle.next_action else {
        return Vec::new();
    };
    if hint.timing != ActionTiming::Now {
        return Vec::new();
    }
    hint.actions
        .into_iter()
        .filter_map(|action| match action {
            AllocationAction::Reject => Some(ProverAction::Reject {
                filters: vec![filter.clone()],
            }),
            AllocationAction::Confirm => Some(ProverAction::Confirm {
                filters: vec![filter.clone()],
            }),
            AllocationAction::Pause => Some(ProverAction::Pause {
                filter: filter.clone(),
            }),
            AllocationAction::Resume => Some(ProverAction::Resume {
                filter: filter.clone(),
            }),
            AllocationAction::Leave => Some(ProverAction::Leave {
                filters: vec![filter.clone()],
            }),
            AllocationAction::Renew
            | AllocationAction::Activate
            | AllocationAction::Depart
            | AllocationAction::Expire => None,
        })
        .collect()
}

fn is_narrow_read_detail_page(page: Page) -> bool {
    matches!(
        page,
        Page::ConfidentialWallet
            | Page::PendingEscrows
            | Page::LegacyAssets
            | Page::Hypergraph
            | Page::Shards
            | Page::PeerResolve
    )
}

fn narrow_read_detail_available(app: &App) -> bool {
    match app.page {
        Page::ConfidentialWallet => app
            .confidential_wallet
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| !snapshot.coins.is_empty()),
        Page::PendingEscrows => app
            .pending_escrows
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.identity_available && !snapshot.escrows.is_empty()),
        Page::LegacyAssets => app
            .legacy_snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.identity_available && !snapshot.coins.is_empty()),
        Page::Hypergraph => app
            .hypergraph
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| !snapshot.entries.is_empty()),
        Page::Shards => app
            .shards
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| !snapshot.shards.is_empty()),
        Page::PeerResolve => app
            .peer_resolve
            .report
            .as_ref()
            .is_some_and(|report| !report.addresses.is_empty()),
        _ => false,
    }
}

fn eligible_join_core_ids(app: &App) -> Vec<u32> {
    let Some(snapshot) = app
        .snapshot
        .as_ref()
        .filter(|snapshot| snapshot.worker_info_error.is_none())
    else {
        return Vec::new();
    };
    let mut cores = snapshot
        .workers
        .iter()
        .filter(|worker| {
            worker.manually_managed
                && worker.filter_hex.is_empty()
                && !app.prover_operations.conflicts_core(worker.core_id)
        })
        .map(|worker| worker.core_id)
        .collect::<Vec<_>>();
    cores.sort_unstable();
    cores
}

pub(crate) fn selected_shard_allocation_index(app: &App) -> Option<usize> {
    let filter = &app
        .shards
        .snapshot
        .as_ref()?
        .shards
        .get(app.shards.index)?
        .filter_hex;
    app.snapshot
        .as_ref()?
        .allocations
        .iter()
        .position(|allocation| allocation.filter_hex == *filter)
}

fn shard_action_unavailable_notice(app: &App) -> String {
    let locale = app.locale();
    if app.loading {
        return t(locale, UiText::LoadingNodeStatusMessage).into();
    }
    if let Some(error) = app.error.as_deref() {
        return crate::i18n::localize_service_error(locale, error).into_owned();
    }
    let Some(snapshot) = app.snapshot.as_ref() else {
        return t(locale, UiText::NodeStatusNotLoadedRToRefresh).into();
    };
    let Some(allocation) =
        selected_shard_allocation_index(app).and_then(|index| snapshot.allocations.get(index))
    else {
        return t(locale, UiText::NoMatchingLocalAllocation).into();
    };
    let Some(epoch) = snapshot
        .epoch
        .as_ref()
        .filter(|epoch| epoch.length_frames > 0)
    else {
        return t(locale, UiText::AllocationLifecycleUnavailable).into();
    };
    let Ok(filter) = hex::decode(&allocation.filter_hex) else {
        return t(locale, UiText::SelectedFilterInvalid).into();
    };
    if !(32..=66).contains(&filter.len()) {
        return t(locale, UiText::SelectedFilterInvalid).into();
    }
    if app.prover_operations.conflicts_filter(&filter) {
        return t(locale, UiText::OperationAlreadyInProgress).into();
    }

    let lifecycle = allocation_lifecycle(allocation, snapshot.last_received_frame, Some(epoch));
    let status = format!(
        "{}: {}",
        t(locale, UiText::LocalAllocationStatus),
        allocation_lifecycle_status_label(locale, lifecycle.status),
    );
    if let Some(next) = lifecycle.next_action.as_ref().filter(|hint| {
        hint.actions.iter().any(|action| {
            matches!(
                action,
                AllocationAction::Reject
                    | AllocationAction::Confirm
                    | AllocationAction::Pause
                    | AllocationAction::Resume
                    | AllocationAction::Leave
            )
        })
    }) {
        return format!(
            "{status} · {}: {}",
            t(locale, UiText::NextAction),
            format_allocation_action_hint(locale, next, true),
        );
    }
    if let Some(default) = lifecycle.default_action.as_ref() {
        return format!(
            "{status} · {} · {}: {}",
            t(locale, UiText::NoManualActionRequired),
            t(locale, UiText::DefaultAction),
            format_allocation_action_hint(locale, default, true),
        );
    }
    format!(
        "{status} · {}",
        t(
            locale,
            UiText::NoActionsAreAvailableForTheCurrentAllocationStateRefreshAndCheckAgain,
        )
    )
}

fn background_operation_core(app: &App, action: &ProverAction) -> Option<u32> {
    match action {
        ProverAction::Join { worker_ids, .. } => worker_ids.first().copied(),
        ProverAction::Leave { filters } => {
            let filter = filters.first().map(hex::encode)?;
            app.snapshot
                .as_ref()?
                .workers
                .iter()
                .find(|worker| worker.filter_hex == filter)
                .map(|worker| worker.core_id)
        }
        _ => None,
    }
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn handle_wallet_input(app: &mut App, event: &InputEvent) -> bool {
    if app.wallet.in_flight
        && matches!(
            app.page,
            Page::WalletForm | Page::WalletProgress | Page::WalletConfirm
        )
    {
        if *event == InputEvent::Quit {
            app.should_quit = true;
        }
        return true;
    }
    match app.page {
        Page::WalletActions => match event {
            InputEvent::Up => {
                app.wallet.action_index = (app.wallet.action_index + 1) % 2;
                true
            }
            InputEvent::Down => {
                app.wallet.action_index = (app.wallet.action_index + 1) % 2;
                true
            }
            InputEvent::Enter => {
                let action = if app.wallet.action_index == 0 {
                    crate::domain::WalletAction::Transfer
                } else {
                    crate::domain::WalletAction::PendingTransfer
                };
                app.open_wallet_action(action);
                true
            }
            InputEvent::Back => {
                app.back();
                true
            }
            _ => false,
        },
        Page::WalletForm if app.wallet.editing => match event {
            InputEvent::Character('\t') | InputEvent::Enter => {
                app.wallet.editing = false;
                true
            }
            InputEvent::Character(character) => {
                let field = match app.wallet.form_field {
                    0 => &mut app.wallet.form_application,
                    1 => &mut app.wallet.form_target,
                    2 => &mut app.wallet.form_amount,
                    3 => &mut app.wallet.form_explicit_fee,
                    4 => &mut app.wallet.form_maximum_fee,
                    5 => &mut app.wallet.form_refund_frame,
                    _ => return true,
                };
                field.push(*character);
                app.wallet.error = None;
                true
            }
            InputEvent::Backspace => {
                let field = match app.wallet.form_field {
                    0 => &mut app.wallet.form_application,
                    1 => &mut app.wallet.form_target,
                    2 => &mut app.wallet.form_amount,
                    3 => &mut app.wallet.form_explicit_fee,
                    4 => &mut app.wallet.form_maximum_fee,
                    5 => &mut app.wallet.form_refund_frame,
                    _ => return true,
                };
                field.pop();
                app.wallet.error = None;
                true
            }
            InputEvent::Back => {
                app.wallet.editing = false;
                true
            }
            _ => true,
        },
        Page::WalletForm => match event {
            InputEvent::Up => {
                app.wallet.form_field = app.wallet.form_field.saturating_sub(1);
                true
            }
            InputEvent::Down | InputEvent::Character('\t') => {
                let last = if app.wallet.action == crate::domain::WalletAction::PendingTransfer {
                    6
                } else {
                    5
                };
                app.wallet.form_field = (app.wallet.form_field + 1).min(last);
                true
            }
            InputEvent::Enter
                if matches!(
                    app.wallet.action,
                    crate::domain::WalletAction::Accept | crate::domain::WalletAction::Reject
                ) && matches!(app.wallet.form_field, 1 | 2) =>
            {
                true
            }
            InputEvent::Enter
                if app.wallet.form_field
                    < if app.wallet.action == crate::domain::WalletAction::PendingTransfer {
                        6
                    } else {
                        5
                    } =>
            {
                app.wallet.editing = true;
                true
            }
            InputEvent::Enter => {
                app.queue_wallet_preflight();
                true
            }
            InputEvent::Back => {
                app.back();
                true
            }
            _ => false,
        },
        Page::WalletConfirm => match event {
            InputEvent::Left | InputEvent::Right | InputEvent::Up | InputEvent::Down => {
                app.wallet.confirmation = match app.wallet.confirmation {
                    ConfirmChoice::Cancel => ConfirmChoice::Confirm,
                    ConfirmChoice::Confirm => ConfirmChoice::Cancel,
                };
                true
            }
            InputEvent::Enter if app.wallet.confirmation == ConfirmChoice::Confirm => {
                app.queue_wallet_execute();
                true
            }
            InputEvent::Enter | InputEvent::Back => {
                app.wallet.request = None;
                app.wallet.preflight = None;
                app.back();
                true
            }
            _ => false,
        },
        Page::WalletProgress => true,
        Page::WalletResult => match event {
            InputEvent::Refresh => {
                app.queue_confidential_wallet();
                app.queue_pending_escrows();
                true
            }
            InputEvent::Enter | InputEvent::Back => {
                while app.route_stack.last().is_some_and(|page| {
                    matches!(
                        page,
                        Page::WalletActions
                            | Page::WalletForm
                            | Page::WalletConfirm
                            | Page::WalletProgress
                            | Page::WalletResult
                    )
                }) {
                    app.route_stack.pop();
                }
                app.page = app
                    .route_stack
                    .last()
                    .copied()
                    .unwrap_or(Page::ConfidentialWallet);
                true
            }
            _ => false,
        },
        Page::ConfidentialWallet if *event == InputEvent::Character('a') => {
            app.open(Page::WalletActions);
            true
        }
        Page::PendingEscrows if *event == InputEvent::Character('a') => {
            let selected = app
                .pending_escrows
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.escrows.get(app.pending_escrows.index))
                .cloned();
            if let Some(escrow) = selected
                .filter(|escrow| escrow.branch == crate::domain::PendingEscrowBranch::Recipient)
            {
                app.open_wallet_action(crate::domain::WalletAction::Accept);
                app.wallet.form_target = escrow.address_hex;
                app.wallet.form_amount.clear();
            }
            true
        }
        Page::PendingEscrows if *event == InputEvent::Character('x') => {
            let selected = app
                .pending_escrows
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.escrows.get(app.pending_escrows.index))
                .cloned();
            let current = app
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.last_global_head_frame);
            if let (Some(escrow), Some(current)) = (selected, current) {
                let eligible = escrow.branch == crate::domain::PendingEscrowBranch::Refund
                    && current.saturating_sub(
                        quil_execution::token_intrinsic::constants::GLOBAL_ANCHOR_SAFETY_MARGIN,
                    ) >= escrow.refund_after_global_frame;
                if eligible {
                    app.open_wallet_action(crate::domain::WalletAction::Reject);
                    app.wallet.form_target = escrow.address_hex;
                    app.wallet.form_amount.clear();
                }
            }
            true
        }
        _ => false,
    }
}

fn handle_legacy_migration_input(app: &mut App, event: &InputEvent) -> bool {
    if app.page != Page::LegacyAssets {
        return false;
    }
    if app.legacy_migration.step == LegacyMigrationStep::Inactive {
        if matches!(event, InputEvent::Character('m' | 'M')) {
            app.begin_legacy_migration();
            return true;
        }
        return false;
    }
    if *event == InputEvent::Quit {
        app.legacy_migration.clear_and_invalidate();
        app.should_quit = true;
        return true;
    }
    if matches!(event, InputEvent::Resize(_, _)) {
        return false;
    }
    if app.legacy_migration.viewer_kind.is_some() {
        if matches!(event, InputEvent::Back | InputEvent::Enter) {
            app.legacy_migration.close_viewer();
        }
        return true;
    }
    if app.legacy_migration.in_flight {
        return true;
    }
    match app.legacy_migration.step {
        LegacyMigrationStep::Identity => match event {
            InputEvent::Character(character) if character.is_ascii_hexdigit() => {
                app.legacy_migration.secret.push(*character);
                app.legacy_migration.error = None;
                true
            }
            InputEvent::Backspace => {
                app.legacy_migration.secret.pop();
                app.legacy_migration.error = None;
                true
            }
            InputEvent::Enter => {
                app.validate_legacy_identity_and_load();
                true
            }
            InputEvent::Back => {
                app.legacy_migration.clear_and_invalidate();
                true
            }
            _ => true,
        },
        LegacyMigrationStep::Destination => match event {
            InputEvent::Left | InputEvent::Up => {
                app.legacy_migration.destination = app.legacy_migration.destination.previous();
                true
            }
            InputEvent::Right | InputEvent::Down => {
                app.legacy_migration.destination = app.legacy_migration.destination.next();
                true
            }
            InputEvent::Character(character)
                if app.legacy_migration.destination == LegacyDestinationChoice::ExistingConfig =>
            {
                app.legacy_migration.existing_path.push(*character);
                app.legacy_migration.error = None;
                true
            }
            InputEvent::Backspace
                if app.legacy_migration.destination == LegacyDestinationChoice::ExistingConfig =>
            {
                app.legacy_migration.existing_path.pop();
                true
            }
            InputEvent::Enter => {
                if app.legacy_migration.destination == LegacyDestinationChoice::ExistingConfig
                    && app.legacy_migration.existing_path.trim().is_empty()
                {
                    app.legacy_migration.error = Some("Recipient config path is required".into());
                } else {
                    app.queue_legacy_migration_preflight();
                }
                true
            }
            InputEvent::Back => {
                app.legacy_migration.step = LegacyMigrationStep::Identity;
                true
            }
            _ => true,
        },
        LegacyMigrationStep::Preflight => match event {
            InputEvent::Left | InputEvent::Right | InputEvent::Up | InputEvent::Down => {
                app.legacy_migration.confirmation = match app.legacy_migration.confirmation {
                    ConfirmChoice::Cancel => ConfirmChoice::Confirm,
                    ConfirmChoice::Confirm => ConfirmChoice::Cancel,
                };
                true
            }
            InputEvent::Enter if app.legacy_migration.confirmation == ConfirmChoice::Confirm => {
                app.queue_legacy_migration_start();
                true
            }
            InputEvent::Enter | InputEvent::Back => {
                app.legacy_migration.preflight = None;
                app.legacy_migration.confirmation = ConfirmChoice::Cancel;
                app.legacy_migration.step = LegacyMigrationStep::Destination;
                true
            }
            _ => true,
        },
        LegacyMigrationStep::Progress => {
            let phase = app
                .legacy_migration
                .status
                .as_ref()
                .map(|status| status.phase.clone());
            match (phase, event) {
                (
                    Some(crate::domain::LegacyMigrationPhase::AwaitingSourceKey),
                    InputEvent::Character(character),
                ) if character.is_ascii_hexdigit() => {
                    app.legacy_migration.secret.push(*character);
                    true
                }
                (
                    Some(crate::domain::LegacyMigrationPhase::AwaitingSourceKey),
                    InputEvent::Backspace,
                ) => {
                    app.legacy_migration.secret.pop();
                    true
                }
                (
                    Some(crate::domain::LegacyMigrationPhase::AwaitingSourceKey),
                    InputEvent::Enter,
                ) => {
                    app.queue_legacy_migration_source_resume();
                    true
                }
                (
                    Some(
                        crate::domain::LegacyMigrationPhase::SubmissionUnknown
                        | crate::domain::LegacyMigrationPhase::RecoverableError,
                    ),
                    InputEvent::Enter,
                ) => {
                    if app.legacy_migration.recovery_armed {
                        app.queue_legacy_migration_rebroadcast();
                    } else {
                        app.legacy_migration.recovery_armed = true;
                    }
                    true
                }
                (_, InputEvent::Back) if app.legacy_migration.recovery_armed => {
                    app.legacy_migration.recovery_armed = false;
                    true
                }
                (_, InputEvent::Back) => {
                    app.legacy_migration.clear_and_invalidate();
                    true
                }
                _ => true,
            }
        }
        LegacyMigrationStep::Files
            if !app
                .legacy_migration
                .status
                .as_ref()
                .is_some_and(|status| status.source_key_persisted) =>
        {
            if *event == InputEvent::Back {
                app.legacy_migration.clear_and_invalidate();
            }
            true
        }
        LegacyMigrationStep::Files => match event {
            InputEvent::Character('c' | 'C') => {
                if app.legacy_migration.file_confirmation == Some(LegacyMigrationFileKind::Config) {
                    app.legacy_migration.queued_file = Some(LegacyMigrationFileKind::Config);
                    app.legacy_migration.in_flight = true;
                } else {
                    app.legacy_migration.file_confirmation = Some(LegacyMigrationFileKind::Config);
                }
                true
            }
            InputEvent::Character('k' | 'K') => {
                if app.legacy_migration.file_confirmation == Some(LegacyMigrationFileKind::Keys) {
                    app.legacy_migration.queued_file = Some(LegacyMigrationFileKind::Keys);
                    app.legacy_migration.in_flight = true;
                } else {
                    app.legacy_migration.file_confirmation = Some(LegacyMigrationFileKind::Keys);
                }
                true
            }
            InputEvent::Back => {
                app.legacy_migration.clear_and_invalidate();
                true
            }
            _ => true,
        },
        LegacyMigrationStep::Inactive => false,
    }
}

fn handle_reward_mint_input(app: &mut App, event: &InputEvent) -> bool {
    if app.page == Page::RewardMintConfirm {
        if app.reward_mint.in_flight {
            if *event == InputEvent::Quit {
                app.should_quit = true;
            }
            return !matches!(event, InputEvent::Resize(_, _));
        }
        return match event {
            InputEvent::Left | InputEvent::Right | InputEvent::Up | InputEvent::Down => {
                app.reward_mint.confirmation = match app.reward_mint.confirmation {
                    ConfirmChoice::Cancel => ConfirmChoice::Confirm,
                    ConfirmChoice::Confirm => ConfirmChoice::Cancel,
                };
                true
            }
            InputEvent::Enter if app.reward_mint.confirmation == ConfirmChoice::Confirm => {
                app.queue_reward_mint_start();
                true
            }
            InputEvent::Enter | InputEvent::Back => {
                app.reward_mint.preflight = None;
                app.reward_mint.confirmation = ConfirmChoice::Cancel;
                app.back();
                true
            }
            _ => false,
        };
    }
    if app.page == Page::Rewards && matches!(event, InputEvent::Character('m' | 'M')) {
        let eligible = app.reward_snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.proof_verified
                && snapshot
                    .claimable_base_units
                    .is_some_and(|amount| amount > 0)
        });
        if eligible && app.reward_mint.status.is_none() {
            app.queue_reward_mint_preflight();
        }
        return true;
    }
    if app.page == Page::Rewards && matches!(event, InputEvent::Character('b' | 'B')) {
        let available = app
            .reward_mint
            .status
            .as_ref()
            .is_some_and(|status| status.can_rebroadcast);
        if available && !app.reward_mint.in_flight {
            if app.reward_mint.rebroadcast_armed {
                app.queue_reward_mint_rebroadcast();
            } else {
                app.reward_mint.rebroadcast_armed = true;
            }
        }
        return true;
    }
    if app.page == Page::Rewards
        && app.reward_mint.rebroadcast_armed
        && matches!(event, InputEvent::Back)
    {
        app.reward_mint.rebroadcast_armed = false;
        return true;
    }
    false
}

fn native_capability_page(route: &str) -> Option<Page> {
    Some(match route {
        "confidential-wallet" => Page::ConfidentialWallet,
        "legacy-assets" => Page::LegacyAssets,
        "pending-escrows" => Page::PendingEscrows,
        "rewards" => Page::Rewards,
        "prover" => Page::Prover,
        "shards" => Page::Shards,
        "archive-shards" => Page::ArchiveShards,
        "aliases" => Page::Aliases,
        "hypergraph" => Page::Hypergraph,
        "health" => Page::Health,
        "peer-resolve" => Page::PeerResolve,
        "diagnostics" => Page::Diagnostics,
        "logs" => Page::Logs,
        "log-export" => Page::LogExport,
        _ => return None,
    })
}

fn capability_field_error(app: &App) -> Option<(usize, String)> {
    let capability = app.capabilities.current()?;
    if matches!(capability.id.as_str(), "node.manage" | "node.prover.manage")
        && app.capabilities.values.value("render", 0) != "true"
    {
        let index = capability
            .fields
            .iter()
            .position(|field| field.id == "render")
            .unwrap_or_default();
        return Some((
            index,
            "Only one-shot render mode is available here; use the native Node or Prover page for interactive management."
                .into(),
        ));
    }
    for (index, field) in capability.fields.iter().enumerate() {
        let values = app.capabilities.values.get(&field.id).unwrap_or_default();
        if field.required
            && (values.is_empty() || values.iter().any(|value| value.trim().is_empty()))
        {
            return Some((
                index,
                crate::i18n::capability_required_error(app.locale(), &field.label),
            ));
        }
        for value in values.iter().filter(|value| !value.trim().is_empty()) {
            match field.kind {
                crate::capability::CapabilityFieldKind::Integer
                    if value.parse::<u128>().is_err() =>
                {
                    return Some((
                        index,
                        crate::i18n::capability_unsigned_error(app.locale(), &field.label),
                    ));
                }
                crate::capability::CapabilityFieldKind::Boolean
                    if !matches!(*value, "true" | "false") =>
                {
                    return Some((
                        index,
                        crate::i18n::capability_boolean_error(app.locale(), &field.label),
                    ));
                }
                _ => {}
            }
        }
    }
    None
}

fn open_capability_task(app: &mut App) {
    while matches!(
        app.route_stack.last(),
        Some(Page::CapabilityForm | Page::CapabilityConfirm | Page::CapabilityTask)
    ) {
        app.route_stack.pop();
    }
    if app.route_stack.is_empty() {
        app.route_stack.push(Page::Capabilities);
    }
    app.route_stack.push(Page::CapabilityTask);
    app.page = Page::CapabilityTask;
}

fn queue_current_capability(app: &mut App, confirmed: bool) {
    let Some(capability) = app.capabilities.current().cloned() else {
        return;
    };
    let request_id = app.capabilities.allocate_request_id();
    let sensitive_field_ids = capability
        .fields
        .iter()
        .filter(|field| field.sensitive)
        .map(|field| field.id.clone())
        .collect();
    let values = std::mem::take(&mut app.capabilities.values);
    app.capabilities.queued = Some(CapabilityRequest {
        request_id,
        capability_id: capability.id,
        values,
        sensitive_field_ids,
        confirmed,
    });
    app.capabilities.pending_request_id = Some(request_id);
    app.capabilities.current_request_id = Some(request_id);
    app.capabilities.current_task_id = None;
    app.capabilities.current_task = None;
    app.capabilities.task_scroll = 0;
    app.capabilities.error = None;
    open_capability_task(app);
}

fn submit_capability_form(app: &mut App) {
    if let Some((field_index, error)) = capability_field_error(app) {
        app.capabilities.field_index = field_index;
        app.capabilities.error = Some(error);
        return;
    }
    let read_only = app
        .capabilities
        .current()
        .is_some_and(|capability| capability.read_only);
    app.capabilities.error = None;
    if read_only {
        queue_current_capability(app, false);
    } else {
        app.capabilities.confirmation = ConfirmChoice::Cancel;
        app.open(Page::CapabilityConfirm);
    }
}

fn open_capability(app: &mut App, capability: crate::capability::CapabilitySpec) {
    match &capability.target {
        crate::capability::CapabilityTarget::Native { route } => {
            if let Some(page) = native_capability_page(route) {
                app.open_page(page);
            } else {
                app.capabilities.error =
                    Some(crate::i18n::capability_route_error(app.locale(), route));
            }
        }
        crate::capability::CapabilityTarget::EmbeddedCli { .. }
            if !capability.fields.is_empty() =>
        {
            app.open(Page::CapabilityForm);
        }
        crate::capability::CapabilityTarget::EmbeddedCli { .. } if capability.read_only => {
            queue_current_capability(app, false);
        }
        crate::capability::CapabilityTarget::EmbeddedCli { .. } => {
            app.capabilities.confirmation = ConfirmChoice::Cancel;
            app.open(Page::CapabilityConfirm);
        }
    }
}

fn begin_capability(app: &mut App, id: &str) {
    let Some(capability) = app.capabilities.begin(id) else {
        app.capabilities.error = Some(format!("qclient capability {id} is unavailable"));
        app.open(Page::Capabilities);
        return;
    };
    open_capability(app, capability);
}

fn begin_selected_capability(app: &mut App) {
    let Some(capability) = app.capabilities.begin_selected() else {
        return;
    };
    open_capability(app, capability);
}

fn handle_capability_input(app: &mut App, event: &InputEvent) -> bool {
    if !matches!(
        app.page,
        Page::Capabilities | Page::CapabilityForm | Page::CapabilityConfirm | Page::CapabilityTask
    ) {
        return false;
    }
    if matches!(event, InputEvent::Resize(_, _)) {
        return false;
    }
    if *event == InputEvent::Quit {
        app.capabilities.clear_sensitive_state();
        app.should_quit = true;
        return true;
    }

    match app.page {
        Page::Capabilities => match event {
            InputEvent::Left => app.capabilities.select_group(-1),
            InputEvent::Right => app.capabilities.select_group(1),
            InputEvent::Up => app.capabilities.select_item(-1),
            InputEvent::Down => app.capabilities.select_item(1),
            InputEvent::PageUp => {
                let max_scroll = app
                    .capabilities
                    .max_detail_scroll(app.viewport_width, app.viewport_height);
                app.capabilities.detail_scroll = app
                    .capabilities
                    .detail_scroll
                    .min(max_scroll)
                    .saturating_sub(5)
            }
            InputEvent::PageDown => {
                let max_scroll = app
                    .capabilities
                    .max_detail_scroll(app.viewport_width, app.viewport_height);
                app.capabilities.detail_scroll = app
                    .capabilities
                    .detail_scroll
                    .saturating_add(5)
                    .min(max_scroll)
            }
            InputEvent::Enter => begin_selected_capability(app),
            InputEvent::Character('t' | 'T') if app.capabilities.current_task.is_some() => {
                app.open(Page::CapabilityTask)
            }
            InputEvent::Back => app.back(),
            _ => {}
        },
        Page::CapabilityForm if app.capabilities.editing => {
            let field_id = app
                .capabilities
                .current()
                .and_then(|capability| capability.fields.get(app.capabilities.field_index))
                .map(|field| field.id.clone());
            match (event, field_id) {
                (InputEvent::Character('\t'), _) | (InputEvent::BackTab, _) => {
                    app.capabilities.editing = false;
                    let fields = app
                        .capabilities
                        .current()
                        .map_or(0, |capability| capability.fields.len());
                    if fields > 0 {
                        app.capabilities.field_index = match event {
                            InputEvent::BackTab => {
                                (app.capabilities.field_index + fields) % (fields + 1)
                            }
                            _ => (app.capabilities.field_index + 1) % (fields + 1),
                        };
                    }
                }
                (InputEvent::Character(character), Some(id)) => {
                    app.capabilities.values.push_char(&id, *character);
                    app.capabilities.error = None;
                }
                (InputEvent::Backspace, Some(id)) => {
                    app.capabilities.values.pop_char(&id);
                    app.capabilities.error = None;
                }
                (InputEvent::Enter, _) | (InputEvent::Back, _) => {
                    app.capabilities.editing = false;
                }
                _ => {}
            }
        }
        Page::CapabilityForm => {
            let fields = app
                .capabilities
                .current()
                .map_or(0, |capability| capability.fields.len());
            match event {
                InputEvent::Up | InputEvent::BackTab => {
                    app.capabilities.field_index =
                        (app.capabilities.field_index + fields) % (fields + 1);
                }
                InputEvent::Down | InputEvent::Character('\t') => {
                    app.capabilities.field_index =
                        (app.capabilities.field_index + 1) % (fields + 1);
                }
                InputEvent::Back => {
                    app.capabilities.clear_draft();
                    app.back();
                }
                InputEvent::Enter if app.capabilities.field_index == fields => {
                    submit_capability_form(app)
                }
                InputEvent::Enter | InputEvent::Left | InputEvent::Right => {
                    let field = app.capabilities.current().and_then(|capability| {
                        capability.fields.get(app.capabilities.field_index).cloned()
                    });
                    if let Some(field) = field {
                        if field.kind == crate::capability::CapabilityFieldKind::Boolean {
                            app.capabilities.values.toggle_boolean(&field.id);
                        } else {
                            app.capabilities.editing = true;
                        }
                        app.capabilities.error = None;
                    }
                }
                InputEvent::Character('a' | 'A') => {
                    let field = app.capabilities.current().and_then(|capability| {
                        capability.fields.get(app.capabilities.field_index).cloned()
                    });
                    if let Some(field) = field.filter(|field| field.repeated) {
                        app.capabilities.values.push_empty(&field.id);
                    }
                }
                InputEvent::Character('x' | 'X') => {
                    let field = app.capabilities.current().and_then(|capability| {
                        capability.fields.get(app.capabilities.field_index).cloned()
                    });
                    if let Some(field) = field.filter(|field| field.repeated) {
                        app.capabilities.values.pop_extra(&field.id);
                    }
                }
                _ => {}
            }
        }
        Page::CapabilityConfirm => match event {
            InputEvent::Left | InputEvent::Right | InputEvent::Up | InputEvent::Down => {
                app.capabilities.confirmation = match app.capabilities.confirmation {
                    ConfirmChoice::Cancel => ConfirmChoice::Confirm,
                    ConfirmChoice::Confirm => ConfirmChoice::Cancel,
                }
            }
            InputEvent::Enter => match app.capabilities.confirmation {
                ConfirmChoice::Cancel => app.back(),
                ConfirmChoice::Confirm => queue_current_capability(app, true),
            },
            InputEvent::Back => app.back(),
            _ => {}
        },
        Page::CapabilityTask => match event {
            InputEvent::Up => {
                app.capabilities.task_scroll = app.capabilities.task_scroll.saturating_sub(1).min(
                    app.capabilities
                        .max_task_scroll(app.viewport_width, app.viewport_height),
                )
            }
            InputEvent::Down => {
                app.capabilities.task_scroll = app.capabilities.task_scroll.saturating_add(1).min(
                    app.capabilities
                        .max_task_scroll(app.viewport_width, app.viewport_height),
                )
            }
            InputEvent::PageUp => {
                app.capabilities.task_scroll = app.capabilities.task_scroll.saturating_sub(10).min(
                    app.capabilities
                        .max_task_scroll(app.viewport_width, app.viewport_height),
                )
            }
            InputEvent::PageDown => {
                app.capabilities.task_scroll = app.capabilities.task_scroll.saturating_add(10).min(
                    app.capabilities
                        .max_task_scroll(app.viewport_width, app.viewport_height),
                )
            }
            InputEvent::Back => app.back(),
            _ => {}
        },
        _ => unreachable!(),
    }
    true
}

pub fn reduce_input(app: &mut App, event: InputEvent) {
    if !matches!(
        app.page,
        Page::Prover | Page::Shards | Page::Capabilities | Page::CapabilityTask
    ) && matches!(event, InputEvent::PageUp | InputEvent::PageDown)
    {
        return;
    }
    if handle_legacy_migration_input(app, &event)
        || handle_reward_mint_input(app, &event)
        || handle_wallet_input(app, &event)
        || handle_capability_input(app, &event)
    {
        return;
    }
    if let InputEvent::Resize(width, height) = event {
        app.viewport_width = width;
        app.viewport_height = height;
        app.prover_detail_scroll = 0;
        app.shards.detail_scroll = 0;
        app.capabilities.clamp_detail_scroll(width, height);
        app.capabilities.clamp_task_scroll(width, height);
        if width >= 70 {
            app.health.detail = false;
            app.expanded_allocation = None;
            app.archive_shards.detail = false;
            app.narrow_read_detail = false;
        }
        return;
    }
    if app.page == Page::Confirm && app.operation_in_flight {
        if event == InputEvent::Quit {
            app.should_quit = true;
        }
        return;
    }
    if app.page == Page::LogExport && app.log_export.in_flight {
        if event == InputEvent::Quit {
            app.should_quit = true;
        }
        return;
    }
    if app.page == Page::ConfigPath {
        if app.config_path_in_flight {
            if event == InputEvent::Quit {
                app.should_quit = true;
            }
            return;
        }
        match event {
            InputEvent::Character(character) => {
                app.config_path_input.push(character);
                app.config_path_error = None;
            }
            InputEvent::Backspace => {
                app.config_path_input.pop();
                app.config_path_error = None;
            }
            InputEvent::Enter => {
                let path = app.config_path_input.trim().to_owned();
                if path.is_empty() {
                    app.config_path_error = Some(
                        crate::i18n::t(app.locale(), UiText::EnterAConfigurationDirectory).into(),
                    );
                } else if app.queued_config_path.is_none() {
                    app.config_path_error = None;
                    app.queued_config_path = Some(ConfigPathRequest { path });
                    app.config_path_in_flight = true;
                }
            }
            InputEvent::Back if !app.config_path_required => app.back(),
            InputEvent::Quit => app.should_quit = true,
            _ => {}
        }
        return;
    }
    if app.page == Page::Other {
        match event {
            InputEvent::Quit => app.should_quit = true,
            InputEvent::Back if app.other.editing_port => {
                app.other.editing_port = false;
                app.other.port_input = app.other.settings.port.to_string();
                app.other.error = None;
            }
            InputEvent::Back if app.other.editing_read_rpc => {
                app.other.editing_read_rpc = false;
                app.other.read_rpc_input = app.other.read_rpc_override.clone().unwrap_or_default();
                app.other.error = None;
            }
            InputEvent::Back => app.back(),
            _ if app.other.in_flight || app.other.read_rpc_in_flight => {}
            InputEvent::Character(character)
                if app.other.editing_port && character.is_ascii_digit() =>
            {
                app.other.port_input.push(character);
                app.other.error = None;
            }
            InputEvent::Backspace if app.other.editing_port => {
                app.other.port_input.pop();
                app.other.error = None;
            }
            InputEvent::Enter if app.other.editing_port => {
                match parse_port(&app.other.port_input) {
                    Ok(port) => app.queue_console_settings(ConsoleSettings {
                        web: WebSettings {
                            port,
                            ..app.other.settings
                        },
                        language: app.locale(),
                    }),
                    Err(_) => {
                        app.other.error = Some(
                            crate::i18n::t(
                                app.locale(),
                                UiText::WebPortMustBeAnIntegerFrom1To65535,
                            )
                            .into(),
                        )
                    }
                }
            }
            _ if app.other.editing_port => {}
            InputEvent::Character(character) if app.other.editing_read_rpc => {
                app.other.read_rpc_input.push(character);
                app.other.error = None;
            }
            InputEvent::Backspace if app.other.editing_read_rpc => {
                app.other.read_rpc_input.pop();
                app.other.error = None;
            }
            InputEvent::Enter if app.other.editing_read_rpc => app.queue_read_rpc_save(),
            _ if app.other.editing_read_rpc => {}
            InputEvent::Up | InputEvent::Down => {
                app.other.selected = match event {
                    InputEvent::Up => app.other.selected.checked_sub(1).unwrap_or(3),
                    InputEvent::Down => (app.other.selected + 1) % 4,
                    _ => unreachable!(),
                };
            }
            InputEvent::Enter if app.other.selected == 0 => {
                app.queue_console_settings(ConsoleSettings {
                    web: WebSettings {
                        enabled: !app.other.settings.enabled,
                        ..app.other.settings
                    },
                    language: app.locale(),
                });
            }
            InputEvent::Enter if app.other.selected == 1 => {
                app.other.port_input = app.other.settings.port.to_string();
                app.other.editing_port = true;
                app.other.error = None;
            }
            InputEvent::Enter if app.other.selected == 2 => {
                app.other.read_rpc_input = app.other.read_rpc_override.clone().unwrap_or_default();
                app.other.editing_read_rpc = true;
                app.other.error = None;
            }
            InputEvent::Enter | InputEvent::Right if app.other.selected == 3 => app
                .queue_console_settings(ConsoleSettings {
                    web: app.other.settings,
                    language: app.locale().next(),
                }),
            InputEvent::Left if app.other.selected == 3 => {
                app.queue_console_settings(ConsoleSettings {
                    web: app.other.settings,
                    language: app.locale().previous(),
                })
            }
            InputEvent::Help => app.open(Page::Help),
            _ => {}
        }
        return;
    }
    if app.page == Page::Hypergraph && app.hypergraph.editing {
        match event {
            InputEvent::Character(character) => {
                app.hypergraph.address.push(character);
                app.hypergraph.error = None;
            }
            InputEvent::Backspace => {
                app.hypergraph.address.pop();
                app.hypergraph.error = None;
            }
            InputEvent::Enter => {
                app.hypergraph.editing = false;
                app.queue_hypergraph();
            }
            InputEvent::Back => app.hypergraph.editing = false,
            InputEvent::Quit => app.should_quit = true,
            _ => {}
        }
        return;
    }
    if app.page == Page::Diagnostics && app.diagnostics_editing {
        match event {
            InputEvent::Character(character) => app.diagnostics_address.push(character),
            InputEvent::Backspace => {
                app.diagnostics_address.pop();
            }
            InputEvent::Enter => {
                let address = app.diagnostics_address.trim().to_string();
                if address.is_empty() {
                    app.diagnostics_error =
                        Some(crate::i18n::t(app.locale(), UiText::EnterANodeMultiaddr).into());
                } else if !app.diagnostics_in_flight {
                    app.diagnostics_editing = false;
                    app.diagnostics_generation = app.diagnostics_generation.wrapping_add(1);
                    app.diagnostics_in_flight = true;
                    app.diagnostics_report = None;
                    app.diagnostics_error = None;
                    app.queued_diagnostics = Some(DiagnosticsRequest {
                        generation: app.diagnostics_generation,
                        address,
                    });
                }
            }
            InputEvent::Back => app.diagnostics_editing = false,
            InputEvent::Quit => app.should_quit = true,
            _ => {}
        }
        return;
    }
    if app.page == Page::DiagnosticsIdentity && app.diagnostics_identity.editing {
        match event {
            InputEvent::Character(character) => {
                app.diagnostics_identity.path.push(character);
                app.diagnostics_identity.error = None;
            }
            InputEvent::Backspace => {
                app.diagnostics_identity.path.pop();
                app.diagnostics_identity.error = None;
            }
            InputEvent::Enter | InputEvent::Back => {
                app.diagnostics_identity.editing = false;
            }
            InputEvent::Quit => app.should_quit = true,
            _ => {}
        }
        return;
    }
    if app.page == Page::RemoteHealth && app.remote_health.editing {
        let value = if app.remote_health.field_index == 0 {
            &mut app.remote_health.peer_id
        } else {
            &mut app.remote_health.address
        };
        match event {
            InputEvent::Character(character) => value.push(character),
            InputEvent::Backspace => {
                value.pop();
            }
            InputEvent::Enter | InputEvent::Back => app.remote_health.editing = false,
            InputEvent::Quit => app.should_quit = true,
            _ => {}
        }
        app.remote_health.error = None;
        return;
    }
    if app.page == Page::PeerResolve && app.peer_resolve.editing {
        match event {
            InputEvent::Character(character) => {
                app.peer_resolve.peer_id.push(character);
                app.peer_resolve.error = None;
            }
            InputEvent::Backspace => {
                app.peer_resolve.peer_id.pop();
                app.peer_resolve.error = None;
            }
            InputEvent::Enter => {
                app.peer_resolve.editing = false;
                app.queue_peer_resolve();
            }
            InputEvent::Back => app.peer_resolve.editing = false,
            InputEvent::Quit => app.should_quit = true,
            _ => {}
        }
        return;
    }
    if app.page == Page::AliasEdit && app.alias_form_editing {
        let value = match app.alias_form_field {
            0 => &mut app.alias_form_name,
            1 => &mut app.alias_form_address,
            _ => &mut app.alias_form_kind,
        };
        match event {
            InputEvent::Character(character) => value.push(character),
            InputEvent::Backspace => {
                value.pop();
            }
            InputEvent::Enter | InputEvent::Back => app.alias_form_editing = false,
            InputEvent::Quit => app.should_quit = true,
            _ => {}
        }
        app.alias_form_error = None;
        return;
    }
    if app.page == Page::KeyCreate && app.key_form_editing {
        match event {
            InputEvent::Character(character) => app.key_form_id.push(character),
            InputEvent::Backspace => {
                app.key_form_id.pop();
            }
            InputEvent::Enter | InputEvent::Back => app.key_form_editing = false,
            InputEvent::Quit => app.should_quit = true,
            _ => {}
        }
        app.key_form_error = None;
        return;
    }
    if app.viewport_width <= 69 && app.narrow_read_detail && is_narrow_read_detail_page(app.page) {
        match event {
            InputEvent::Back => {
                app.narrow_read_detail = false;
                app.shards.detail_scroll = 0;
            }
            InputEvent::Up if app.page == Page::Shards => app.scroll_shard_detail(false, 1),
            InputEvent::Down if app.page == Page::Shards => app.scroll_shard_detail(true, 1),
            InputEvent::PageUp if app.page == Page::Shards => app.scroll_shard_detail(false, 5),
            InputEvent::PageDown if app.page == Page::Shards => app.scroll_shard_detail(true, 5),
            InputEvent::Quit => app.should_quit = true,
            InputEvent::Character('e' | 'E') if app.page == Page::Hypergraph => {
                app.narrow_read_detail = false;
                app.hypergraph.editing = true;
                app.hypergraph.error = None;
            }
            InputEvent::Character('e' | 'E') if app.page == Page::PeerResolve => {
                app.narrow_read_detail = false;
                app.peer_resolve.editing = true;
                app.peer_resolve.error = None;
            }
            _ => {}
        }
        return;
    }
    match event {
        InputEvent::Left | InputEvent::Right if app.page == Page::Confirm => {
            app.confirm_choice = match app.confirm_choice {
                ConfirmChoice::Cancel => ConfirmChoice::Confirm,
                ConfirmChoice::Confirm => ConfirmChoice::Cancel,
            };
        }
        InputEvent::PageUp if app.page == Page::Prover => {
            app.scroll_prover_detail(false, 5);
        }
        InputEvent::PageDown if app.page == Page::Prover => {
            app.scroll_prover_detail(true, 5);
        }
        InputEvent::PageUp if app.page == Page::Shards => app.scroll_shard_detail(false, 5),
        InputEvent::PageDown if app.page == Page::Shards => app.scroll_shard_detail(true, 5),
        InputEvent::Up if app.narrow_prover_detail_open() => {
            app.scroll_prover_detail(false, 1);
        }
        InputEvent::Down if app.narrow_prover_detail_open() => {
            app.scroll_prover_detail(true, 1);
        }
        InputEvent::Up if app.page == Page::Prover => {
            let count = allocation_count(app);
            if count > 0 {
                app.allocation_index = (app.allocation_index + count - 1) % count;
            }
            app.expanded_allocation = None;
            app.prover_detail_scroll = 0;
        }
        InputEvent::Down if app.page == Page::Prover => {
            let count = allocation_count(app);
            if count > 0 {
                app.allocation_index = (app.allocation_index + 1) % count;
            }
            app.expanded_allocation = None;
            app.prover_detail_scroll = 0;
        }
        InputEvent::Up if app.page == Page::ProverActions => {
            let count = available_actions(app).len();
            if count > 0 {
                app.action_index = (app.action_index + count - 1) % count;
            }
        }
        InputEvent::Down if app.page == Page::ProverActions => {
            let count = available_actions(app).len();
            if count > 0 {
                app.action_index = (app.action_index + 1) % count;
            }
        }
        InputEvent::Up if app.page == Page::Configs => {
            let count = app.configs.len();
            if count > 0 {
                app.config_index = (app.config_index + count - 1) % count;
            }
        }
        InputEvent::Down if app.page == Page::Configs => {
            let count = app.configs.len();
            if count > 0 {
                app.config_index = (app.config_index + 1) % count;
            }
        }
        InputEvent::Up if app.page == Page::Keys => {
            let count = app.keys.len();
            if count > 0 {
                app.key_index = (app.key_index + count - 1) % count;
            }
        }
        InputEvent::Down if app.page == Page::Keys => {
            let count = app.keys.len();
            if count > 0 {
                app.key_index = (app.key_index + 1) % count;
            }
        }
        InputEvent::Up if app.page == Page::KeyActions => {
            let count = 1 + usize::from(key_delete_allowed(app));
            app.key_action_index = (app.key_action_index + count - 1) % count;
        }
        InputEvent::Down if app.page == Page::KeyActions => {
            let count = 1 + usize::from(key_delete_allowed(app));
            app.key_action_index = (app.key_action_index + 1) % count;
        }
        InputEvent::Up if app.page == Page::KeyCreate => {
            app.key_form_field = (app.key_form_field + 2) % 3;
        }
        InputEvent::Down if app.page == Page::KeyCreate => {
            app.key_form_field = (app.key_form_field + 1) % 3;
        }
        InputEvent::Left if app.page == Page::KeyCreate && app.key_form_field == 1 => {
            app.key_form_type_index = (app.key_form_type_index + CREATABLE_KEY_TYPES.len() - 1)
                % CREATABLE_KEY_TYPES.len();
        }
        InputEvent::Right if app.page == Page::KeyCreate && app.key_form_field == 1 => {
            app.key_form_type_index = (app.key_form_type_index + 1) % CREATABLE_KEY_TYPES.len();
        }
        InputEvent::Up if app.page == Page::Aliases => {
            let count = app.aliases.len();
            if count > 0 {
                app.alias_index = (app.alias_index + count - 1) % count;
            }
        }
        InputEvent::Down if app.page == Page::Aliases => {
            let count = app.aliases.len();
            if count > 0 {
                app.alias_index = (app.alias_index + 1) % count;
            }
        }
        InputEvent::Up if app.page == Page::LegacyAssets => {
            let count = app
                .legacy_snapshot
                .as_ref()
                .map(|snapshot| snapshot.coins.len())
                .unwrap_or_default();
            if count > 0 {
                app.legacy_index = (app.legacy_index + count - 1) % count;
            }
        }
        InputEvent::Down if app.page == Page::LegacyAssets => {
            let count = app
                .legacy_snapshot
                .as_ref()
                .map(|snapshot| snapshot.coins.len())
                .unwrap_or_default();
            if count > 0 {
                app.legacy_index = (app.legacy_index + 1) % count;
            }
        }
        InputEvent::Up if app.page == Page::ConfidentialWallet => {
            let count = app
                .confidential_wallet
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.coins.len())
                .unwrap_or_default();
            if count > 0 {
                app.confidential_wallet.coin_index =
                    (app.confidential_wallet.coin_index + count - 1) % count;
            }
        }
        InputEvent::Down if app.page == Page::ConfidentialWallet => {
            let count = app
                .confidential_wallet
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.coins.len())
                .unwrap_or_default();
            if count > 0 {
                app.confidential_wallet.coin_index =
                    (app.confidential_wallet.coin_index + 1) % count;
            }
        }
        InputEvent::Up if app.page == Page::PendingEscrows => {
            let count = app
                .pending_escrows
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.escrows.len())
                .unwrap_or_default();
            if count > 0 {
                app.pending_escrows.index = (app.pending_escrows.index + count - 1) % count;
            }
        }
        InputEvent::Up if app.page == Page::Health => {
            let count = app
                .health
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.workers.len())
                .unwrap_or_default();
            if count > 0 {
                app.health.index = (app.health.index + count - 1) % count;
            }
        }
        InputEvent::Down if app.page == Page::Health => {
            let count = app
                .health
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.workers.len())
                .unwrap_or_default();
            if count > 0 {
                app.health.index = (app.health.index + 1) % count;
            }
        }
        InputEvent::Down if app.page == Page::PendingEscrows => {
            let count = app
                .pending_escrows
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.escrows.len())
                .unwrap_or_default();
            if count > 0 {
                app.pending_escrows.index = (app.pending_escrows.index + 1) % count;
            }
        }
        InputEvent::Up if app.page == Page::Hypergraph => {
            let count = app
                .hypergraph
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.entries.len())
                .unwrap_or_default();
            if count > 0 {
                app.hypergraph.entry_index = (app.hypergraph.entry_index + count - 1) % count;
            } else {
                app.hypergraph.field_index = (app.hypergraph.field_index + 1) % 2;
            }
        }
        InputEvent::Down if app.page == Page::Hypergraph => {
            let count = app
                .hypergraph
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.entries.len())
                .unwrap_or_default();
            if count > 0 {
                app.hypergraph.entry_index = (app.hypergraph.entry_index + 1) % count;
            } else {
                app.hypergraph.field_index = (app.hypergraph.field_index + 1) % 2;
            }
        }
        InputEvent::Left | InputEvent::Right
            if app.page == Page::Hypergraph && app.hypergraph.field_index == 0 =>
        {
            app.hypergraph.kind = match app.hypergraph.kind {
                HypergraphKind::Vertex => HypergraphKind::Hyperedge,
                HypergraphKind::Hyperedge => HypergraphKind::Vertex,
            };
        }
        InputEvent::Character('\t') if app.page == Page::Shards => {
            if app.shards.scope == ShardScope::All {
                app.shards.focus = match app.shards.focus {
                    ShardFocus::Directory => ShardFocus::FreeCores,
                    ShardFocus::FreeCores => ShardFocus::Directory,
                };
                app.shards.notice = None;
            }
        }
        InputEvent::Up if app.page == Page::Shards && app.shards.focus == ShardFocus::FreeCores => {
            let count = eligible_join_core_ids(app).len();
            if count > 0 {
                app.shards.core_index = (app.shards.core_index + count - 1) % count;
            }
        }
        InputEvent::Down
            if app.page == Page::Shards && app.shards.focus == ShardFocus::FreeCores =>
        {
            let count = eligible_join_core_ids(app).len();
            if count > 0 {
                app.shards.core_index = (app.shards.core_index + 1) % count;
            }
        }
        InputEvent::Up if app.page == Page::Shards => {
            let count = app
                .shards
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.shards.len())
                .unwrap_or_default();
            if count > 0 {
                let index = (app.shards.index + count - 1) % count;
                if index != app.shards.index {
                    app.shards.detail_scroll = 0;
                }
                app.shards.index = index;
            }
        }
        InputEvent::Down if app.page == Page::Shards => {
            let count = app
                .shards
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.shards.len())
                .unwrap_or_default();
            if count > 0 {
                let index = (app.shards.index + 1) % count;
                if index != app.shards.index {
                    app.shards.detail_scroll = 0;
                }
                app.shards.index = index;
            }
        }
        InputEvent::Left | InputEvent::Right
            if app.page == Page::Shards && app.shards.focus == ShardFocus::Directory =>
        {
            if !app.shards.in_flight {
                app.shards.scope = match app.shards.scope {
                    ShardScope::Allocated => ShardScope::All,
                    ShardScope::All => ShardScope::Allocated,
                };
                app.queue_shards();
            }
        }
        InputEvent::Up if app.page == Page::ArchiveShards => {
            let count = app
                .archive_shards
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.archives.len())
                .unwrap_or_default();
            if count > 0 {
                app.archive_shards.index = (app.archive_shards.index + count - 1) % count;
            }
            app.archive_shards.detail = false;
        }
        InputEvent::Down if app.page == Page::ArchiveShards => {
            let count = app
                .archive_shards
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.archives.len())
                .unwrap_or_default();
            if count > 0 {
                app.archive_shards.index = (app.archive_shards.index + 1) % count;
            }
            app.archive_shards.detail = false;
        }
        InputEvent::Up if app.page == Page::PeerResolve => {
            let count = app
                .peer_resolve
                .report
                .as_ref()
                .map(|report| report.addresses.len())
                .unwrap_or_default();
            if count > 0 {
                app.peer_resolve.index = (app.peer_resolve.index + count - 1) % count;
            }
        }
        InputEvent::Down if app.page == Page::PeerResolve => {
            let count = app
                .peer_resolve
                .report
                .as_ref()
                .map(|report| report.addresses.len())
                .unwrap_or_default();
            if count > 0 {
                app.peer_resolve.index = (app.peer_resolve.index + 1) % count;
            }
        }
        InputEvent::Up if app.page == Page::AliasActions => {
            let count = 1 + usize::from(!app.aliases.is_empty());
            app.alias_action_index = (app.alias_action_index + count - 1) % count;
        }
        InputEvent::Down if app.page == Page::AliasActions => {
            let count = 1 + usize::from(!app.aliases.is_empty());
            app.alias_action_index = (app.alias_action_index + 1) % count;
        }
        InputEvent::Up if app.page == Page::AliasEdit => {
            app.alias_form_field = (app.alias_form_field + 3) % 4;
        }
        InputEvent::Down if app.page == Page::AliasEdit => {
            app.alias_form_field = (app.alias_form_field + 1) % 4;
        }
        InputEvent::Up if app.page == Page::Logs => {
            app.log_scroll = app.log_scroll.saturating_add(1);
        }
        InputEvent::Down if app.page == Page::Logs => {
            app.log_scroll = app.log_scroll.saturating_sub(1);
        }
        InputEvent::Left if app.page == Page::Logs => {
            let count = log_core_ids(app).len();
            app.log_core_index = (app.log_core_index + count - 1) % count;
            app.queue_log();
        }
        InputEvent::Right if app.page == Page::Logs => {
            let count = log_core_ids(app).len();
            app.log_core_index = (app.log_core_index + 1) % count;
            app.queue_log();
        }
        InputEvent::Up if app.page == Page::LogExport && app.log_export.preview.is_none() => {
            app.log_export.field_index = (app.log_export.field_index + 1) % 2;
        }
        InputEvent::Down if app.page == Page::LogExport && app.log_export.preview.is_none() => {
            app.log_export.field_index = (app.log_export.field_index + 1) % 2;
        }
        InputEvent::Left | InputEvent::Right
            if app.page == Page::LogExport && app.log_export.preview.is_some() =>
        {
            app.log_export.choice = match app.log_export.choice {
                ConfirmChoice::Cancel => ConfirmChoice::Confirm,
                ConfirmChoice::Confirm => ConfirmChoice::Cancel,
            };
        }
        InputEvent::Left if app.page == Page::LogExport => {
            if app.log_export.field_index == 0 {
                app.log_export.window_index =
                    (app.log_export.window_index + LOG_EXPORT_WINDOWS.len() - 1)
                        % LOG_EXPORT_WINDOWS.len();
            } else {
                app.log_export.level_index =
                    (app.log_export.level_index + LogExportLevel::ALL.len() - 1)
                        % LogExportLevel::ALL.len();
            }
        }
        InputEvent::Right if app.page == Page::LogExport => {
            if app.log_export.field_index == 0 {
                app.log_export.window_index =
                    (app.log_export.window_index + 1) % LOG_EXPORT_WINDOWS.len();
            } else {
                app.log_export.level_index =
                    (app.log_export.level_index + 1) % LogExportLevel::ALL.len();
            }
        }
        InputEvent::Up if app.page == Page::Home => {
            app.menu_index =
                (app.menu_index % ROOT_MENU.len() + ROOT_MENU.len() - 1) % ROOT_MENU.len();
        }
        InputEvent::Down if app.page == Page::Home => {
            app.menu_index = (app.menu_index % ROOT_MENU.len() + 1) % ROOT_MENU.len()
        }
        InputEvent::Up if matches!(app.page, Page::Section(_)) => {
            let Page::Section(section) = app.page else {
                unreachable!()
            };
            let count = section.items().len();
            app.section_index = (app.section_index % count + count - 1) % count;
        }
        InputEvent::Down if matches!(app.page, Page::Section(_)) => {
            let Page::Section(section) = app.page else {
                unreachable!()
            };
            let count = section.items().len();
            app.section_index = (app.section_index % count + 1) % count;
        }
        InputEvent::Up if app.page == Page::DiagnosticsIdentity => {
            app.diagnostics_identity.action_index = (app.diagnostics_identity.action_index + 2) % 3;
        }
        InputEvent::Down if app.page == Page::DiagnosticsIdentity => {
            app.diagnostics_identity.action_index = (app.diagnostics_identity.action_index + 1) % 3;
        }
        InputEvent::Up if app.page == Page::RemoteHealth => {
            app.remote_health.field_index = (app.remote_health.field_index + 2) % 3;
        }
        InputEvent::Down if app.page == Page::RemoteHealth => {
            app.remote_health.field_index = (app.remote_health.field_index + 1) % 3;
        }
        InputEvent::Up | InputEvent::Down | InputEvent::Left | InputEvent::Right => {}
        InputEvent::Enter if app.page == Page::Home => match ROOT_MENU
            .get(app.menu_index)
            .map(|destination| destination.route)
        {
            Some(RootRoute::Overview) | None => {}
            Some(RootRoute::Section(section)) => {
                app.section_index = 0;
                app.open(Page::Section(section));
            }
            Some(RootRoute::Capabilities) => app.open(Page::Capabilities),
            Some(RootRoute::Help) => app.open(Page::Help),
            Some(RootRoute::Other) => app.open(Page::Other),
        },
        InputEvent::Enter if matches!(app.page, Page::Section(_)) => {
            let Page::Section(section) = app.page else {
                unreachable!()
            };
            if let Some(item) = section.items().get(app.section_index) {
                app.open_page(item.page);
            }
        }
        InputEvent::Enter if app.page == Page::Configs => {
            if app.config_index < app.configs.len()
                && app.active_config_index != app.config_index
                && app.queued_config_index != Some(app.config_index)
            {
                app.queued_config_index = Some(app.config_index);
            }
        }
        InputEvent::OpenConfigPath if app.page == Page::Configs => app.open_config_path(),
        InputEvent::Enter if app.page == Page::Diagnostics => {
            if !app.diagnostics_in_flight {
                app.diagnostics_editing = true;
                app.diagnostics_error = None;
            }
        }
        InputEvent::Enter if app.page == Page::DiagnosticsIdentity => {
            match app.diagnostics_identity.action_index {
                0 => app.diagnostics_identity.editing = true,
                1 => app.queue_diagnostics_identity(false),
                _ => app.queue_diagnostics_identity(true),
            }
        }
        InputEvent::Enter if app.page == Page::RemoteHealth => {
            if app.remote_health.field_index < 2 {
                app.remote_health.editing = true;
            } else {
                app.queue_direct_health();
            }
        }
        InputEvent::Enter
            if app.viewport_width <= 69
                && narrow_read_detail_available(app)
                && !(app.page == Page::Shards && app.shards.focus == ShardFocus::FreeCores) =>
        {
            app.narrow_read_detail = true;
            if app.page == Page::Shards {
                app.shards.detail_scroll = 0;
            }
        }
        InputEvent::Character('e' | 'E')
            if app.page == Page::PeerResolve && !app.peer_resolve.in_flight =>
        {
            app.narrow_read_detail = false;
            app.peer_resolve.editing = true;
            app.peer_resolve.error = None;
        }
        InputEvent::Character('e' | 'E')
            if app.page == Page::Hypergraph && !app.hypergraph.in_flight =>
        {
            app.narrow_read_detail = false;
            app.hypergraph.field_index = 1;
            app.hypergraph.editing = true;
            app.hypergraph.error = None;
        }
        InputEvent::Enter if app.page == Page::PeerResolve => {
            if !app.peer_resolve.in_flight {
                app.peer_resolve.editing = true;
                app.peer_resolve.error = None;
            }
        }
        InputEvent::Character('h' | 'H') if app.page == Page::PeerResolve => {
            app.open_remote_health();
        }
        InputEvent::Enter if app.page == Page::Hypergraph => {
            if !app.hypergraph.in_flight {
                if app.hypergraph.field_index == 0 {
                    app.hypergraph.field_index = 1;
                }
                app.hypergraph.editing = true;
                app.hypergraph.error = None;
            }
        }
        InputEvent::Enter if app.page == Page::Logs => app.queue_log(),
        InputEvent::Enter if app.page == Page::LogExport => {
            if app.log_export.preview.is_some() {
                app.queue_log_export_finish(app.log_export.choice == ConfirmChoice::Confirm);
            } else {
                app.queue_log_export_preview();
            }
        }
        InputEvent::Enter if app.page == Page::Keys && !app.key_in_flight => {
            app.key_action_index = 0;
            app.open(Page::KeyActions);
        }
        InputEvent::Enter if app.page == Page::KeyActions => {
            if app.key_action_index == 0 {
                app.key_form_field = 0;
                app.key_form_editing = false;
                app.key_form_id.clear();
                app.key_form_type_index = 0;
                app.key_form_error = None;
                app.open(Page::KeyCreate);
            } else if key_delete_allowed(app) {
                if let Some(key) = app.keys.get(app.key_index) {
                    app.begin_key_mutation(KeyMutation::Delete { id: key.id.clone() });
                }
            }
        }
        InputEvent::Enter if app.page == Page::KeyCreate => match app.key_form_field {
            0 => app.key_form_editing = true,
            1 => {
                app.key_form_type_index = (app.key_form_type_index + 1) % CREATABLE_KEY_TYPES.len();
            }
            _ => match key_mutation_from_form(app) {
                Ok(mutation) => app.begin_key_mutation(mutation),
                Err(error) => app.key_form_error = Some(error),
            },
        },
        InputEvent::Enter if app.page == Page::Aliases && !app.alias_in_flight => {
            app.alias_action_index = 0;
            app.open(Page::AliasActions);
        }
        InputEvent::Enter if app.page == Page::AliasActions => {
            if app.alias_action_index == 0 {
                app.alias_form_field = 0;
                app.alias_form_editing = false;
                app.alias_form_name.clear();
                app.alias_form_address.clear();
                app.alias_form_kind.clear();
                app.alias_form_error = None;
                app.open(Page::AliasEdit);
            } else if let Some(alias) = app.aliases.get(app.alias_index) {
                app.begin_alias_mutation(AliasMutation::Delete {
                    name: alias.name.clone(),
                });
            }
        }
        InputEvent::Enter if app.page == Page::AliasEdit => {
            if app.alias_form_field < 3 {
                app.alias_form_editing = true;
            } else {
                match alias_mutation_from_form(app) {
                    Ok(mutation) => app.begin_alias_mutation(mutation),
                    Err(error) => app.alias_form_error = Some(error),
                }
            }
        }
        InputEvent::Enter if app.page == Page::Prover && app.viewport_width <= 69 => {
            app.prover_detail_scroll = 0;
            if allocation_count(app) > 0 {
                app.expanded_allocation = if app.expanded_allocation == Some(app.allocation_index) {
                    None
                } else {
                    Some(app.allocation_index)
                };
            }
        }
        InputEvent::Enter if app.page == Page::ArchiveShards && app.viewport_width <= 69 => {
            if app
                .archive_shards
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| !snapshot.archives.is_empty())
            {
                app.archive_shards.detail = true;
            }
        }
        InputEvent::Character('a' | 'A') if app.page == Page::Prover => {
            if !available_actions(app).is_empty() {
                app.action_index = 0;
                app.open(Page::ProverActions);
            }
        }
        InputEvent::Character('a' | 'A')
            if app.page == Page::Shards && app.shards.focus == ShardFocus::Directory =>
        {
            let allocation = selected_shard_allocation_index(app);
            let actions_available = allocation.is_some_and(|index| {
                app.allocation_index = index;
                !available_actions(app).is_empty()
            });
            if actions_available {
                app.action_index = 0;
                app.shards.notice = None;
                app.open(Page::ProverActions);
            } else {
                app.shards.notice = Some(shard_action_unavailable_notice(app));
            }
        }
        InputEvent::Enter if app.page == Page::ProverActions => {
            if let Some(action) = available_actions(app).get(app.action_index).cloned() {
                app.begin_action(action);
            }
        }
        InputEvent::Enter if app.page == Page::Shards => match app.shards.focus {
            ShardFocus::Directory => {
                if app.shards.scope == ShardScope::All {
                    if eligible_join_core_ids(app).is_empty() {
                        app.shards.notice =
                            Some(crate::i18n::t(app.locale(), UiText::NoIdleCoreAvailable).into());
                    } else {
                        app.shards.focus = ShardFocus::FreeCores;
                        app.shards.notice = None;
                    }
                }
            }
            ShardFocus::FreeCores => {
                let target = app
                    .shards
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.shards.get(app.shards.index));
                let core = eligible_join_core_ids(app)
                    .get(app.shards.core_index)
                    .copied();
                match (target, core) {
                    (Some(target), Some(core)) if !target.is_allocated => {
                        match hex::decode(&target.filter_hex) {
                            Ok(filter) if (32..=66).contains(&filter.len()) => {
                                app.begin_action(ProverAction::Join {
                                    filters: vec![filter],
                                    delegate: Vec::new(),
                                    worker_ids: vec![core],
                                });
                            }
                            _ => {
                                app.shards.notice = Some(
                                    crate::i18n::t(app.locale(), UiText::SelectedFilterInvalid)
                                        .into(),
                                )
                            }
                        }
                    }
                    (Some(_), Some(_)) => {
                        app.shards.notice = Some(
                            crate::i18n::t(app.locale(), UiText::FilterAlreadyAllocated).into(),
                        )
                    }
                    _ => {
                        app.shards.notice =
                            Some(crate::i18n::t(app.locale(), UiText::SelectIdleCore).into())
                    }
                }
            }
        },
        InputEvent::Enter if app.page == Page::OperationResult => app.back(),
        InputEvent::Enter if app.page == Page::Confirm => {
            if app.confirm_choice == ConfirmChoice::Confirm && !app.operation_in_flight {
                if let Some(action) = app.pending_action.clone() {
                    if matches!(
                        action,
                        ProverAction::Join { .. } | ProverAction::Leave { .. }
                    ) {
                        let core_id = background_operation_core(app, &action);
                        match app
                            .prover_operations
                            .enqueue(action, core_id, unix_seconds())
                        {
                            Ok(_) => app.return_after_background_operation(),
                            Err(_) => {
                                app.operation_message = Some(
                                    crate::i18n::t(
                                        app.locale(),
                                        UiText::OperationAlreadyInProgress,
                                    )
                                    .into(),
                                );
                                app.operation_succeeded = Some(false);
                            }
                        }
                    } else {
                        app.operation_generation = app.operation_generation.wrapping_add(1);
                        app.queued_operation = Some(OperationRequest {
                            generation: app.operation_generation,
                            action,
                        });
                        app.operation_in_flight = true;
                    }
                } else if let Some(mutation) = app.pending_alias_mutation.clone() {
                    app.alias_mutation_generation = app.alias_mutation_generation.wrapping_add(1);
                    app.queued_alias_mutation = Some(AliasMutationRequest {
                        generation: app.alias_mutation_generation,
                        mutation,
                    });
                    app.operation_in_flight = true;
                } else if let Some(mutation) = app.pending_key_mutation.clone() {
                    app.key_mutation_generation = app.key_mutation_generation.wrapping_add(1);
                    app.queued_key_mutation = Some(KeyMutationRequest {
                        generation: app.key_mutation_generation,
                        mutation,
                    });
                    app.operation_in_flight = true;
                }
            } else if app.confirm_choice == ConfirmChoice::Cancel {
                app.clear_pending_confirmation();
                app.back();
            }
        }
        InputEvent::Enter if app.page == Page::Health && app.viewport_width <= 69 => {
            if app
                .health
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| !snapshot.workers.is_empty())
            {
                app.health.detail = !app.health.detail;
            }
        }
        InputEvent::Enter
        | InputEvent::Character(_)
        | InputEvent::Backspace
        | InputEvent::OpenConfigPath => {}
        InputEvent::Back if app.page == Page::LogExport && app.log_export.in_flight => {}
        InputEvent::Back if app.page == Page::LogExport && app.log_export.preview.is_some() => {
            app.queue_log_export_finish(false);
            app.back();
        }
        InputEvent::Back
            if app.page == Page::Health && app.viewport_width <= 69 && app.health.detail =>
        {
            app.health.detail = false;
        }
        InputEvent::Back if app.narrow_prover_detail_open() => {
            app.expanded_allocation = None;
            app.prover_detail_scroll = 0;
        }
        InputEvent::Back
            if app.page == Page::ArchiveShards
                && app.viewport_width <= 69
                && app.archive_shards.detail =>
        {
            app.archive_shards.detail = false;
        }
        InputEvent::Back => app.back(),
        InputEvent::Refresh => match app.page {
            Page::Home | Page::NodeStatus | Page::Prover => app.begin_node_refresh(),
            Page::Runtime => app.queue_runtime(),
            Page::Health => app.queue_health(),
            Page::PeerResolve => app.queue_peer_resolve(),
            Page::Logs => app.queue_log(),
            Page::LogExport if app.log_export.preview.is_none() => {
                app.queue_log_export_preview();
            }
            Page::Keys => app.queue_keys(),
            Page::Aliases => app.queue_aliases(),
            Page::Rewards => app.queue_rewards(),
            Page::LegacyAssets => app.queue_legacy_assets(),
            Page::ConfidentialWallet => app.queue_confidential_wallet(),
            Page::PendingEscrows => app.queue_pending_escrows(),
            Page::Hypergraph => app.queue_hypergraph(),
            Page::Shards => app.queue_shards(),
            Page::ArchiveShards => app.queue_archive_shards(),
            Page::Section(_)
            | Page::Configs
            | Page::ConfigPath
            | Page::Diagnostics
            | Page::DiagnosticsIdentity
            | Page::RemoteHealth
            | Page::WalletActions
            | Page::WalletForm
            | Page::WalletConfirm
            | Page::WalletProgress
            | Page::WalletResult
            | Page::TokenTransfer
            | Page::Capabilities
            | Page::CapabilityForm
            | Page::CapabilityConfirm
            | Page::CapabilityTask
            | Page::DeployToken
            | Page::RewardMintConfirm
            | Page::LogExport
            | Page::KeyActions
            | Page::KeyCreate
            | Page::AliasActions
            | Page::AliasEdit
            | Page::ProverActions
            | Page::Confirm
            | Page::OperationResult
            | Page::Other
            | Page::Help => {}
        },
        InputEvent::Help => app.open(Page::Help),
        InputEvent::Quit => app.should_quit = true,
        InputEvent::Resize(_, _)
        | InputEvent::PageUp
        | InputEvent::PageDown
        | InputEvent::BackTab => {}
    }
}

fn alias_mutation_from_form(app: &App) -> Result<AliasMutation, String> {
    let name = app.alias_form_name.trim();
    if name.is_empty() {
        return Err(crate::i18n::t(app.locale(), UiText::EnterAnAlias).into());
    }
    if name.chars().any(char::is_whitespace) {
        return Err(crate::i18n::t(app.locale(), UiText::AliasesCannotContainSpaces).into());
    }
    let address = app.alias_form_address.trim();
    let address_without_prefix = address.strip_prefix("0x").unwrap_or(address);
    if address_without_prefix.is_empty() {
        return Err(crate::i18n::t(app.locale(), UiText::EnterAHexadecimalAddress).into());
    }
    hex::decode(address_without_prefix).map_err(|_| {
        crate::i18n::t(app.locale(), UiText::AddressIsNotValidHexadecimal).to_string()
    })?;
    Ok(AliasMutation::Put {
        name: name.to_string(),
        address_hex: address_without_prefix.to_string(),
        kind: app.alias_form_kind.trim().to_string(),
    })
}

fn key_delete_allowed(app: &App) -> bool {
    app.keys
        .get(app.key_index)
        .is_some_and(|key| !protected_key_id(&key.id))
}

fn key_mutation_from_form(app: &App) -> Result<KeyMutation, String> {
    let id = app.key_form_id.trim();
    if id.is_empty() {
        return Err(crate::i18n::t(app.locale(), UiText::EnterAKeyName).into());
    }
    if id.chars().any(char::is_whitespace) {
        return Err(crate::i18n::t(app.locale(), UiText::KeyNamesCannotContainSpaces).into());
    }
    if protected_key_id(id) {
        return Err(
            crate::i18n::t(app.locale(), UiText::QNamesAreReservedForBuiltInNodeKeys).into(),
        );
    }
    if app.keys.iter().any(|key| key.id == id) {
        return Err(match app.locale() {
            Locale::English => format!("Key {id} already exists"),
            Locale::TraditionalChinese => format!("金鑰 {id} 已存在"),
            Locale::Japanese => format!("キー {id} はすでに存在します"),
            Locale::French => format!("La clé {id} existe déjà"),
        });
    }
    let key_type = CREATABLE_KEY_TYPES
        .get(app.key_form_type_index)
        .map(|(key_type, _)| *key_type)
        .ok_or_else(|| crate::i18n::t(app.locale(), UiText::SelectAKeyType).to_string())?;
    Ok(KeyMutation::Create {
        id: id.to_string(),
        key_type,
    })
}

fn allocation_count(app: &App) -> usize {
    app.snapshot
        .as_ref()
        .map(|snapshot| snapshot.allocations.len())
        .unwrap_or_default()
}

pub fn log_core_ids(app: &App) -> Vec<u32> {
    let mut cores = vec![0];
    if let Some(snapshot) = &app.snapshot {
        cores.extend(snapshot.workers.iter().map(|worker| worker.core_id));
    }
    cores.sort_unstable();
    cores.dedup();
    cores
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::ServiceEvent;

    fn narrow_read_only_page(page: Page) -> App {
        use crate::domain::{
            ConfidentialCoinSnapshot, ConfidentialWalletSnapshot, HypergraphEntrySnapshot,
            HypergraphKind, HypergraphSnapshot, LegacyAssetsSnapshot, LegacyCoinSnapshot,
            PeerAddressReport, PeerAddressSnapshot, PeerReach, PendingEscrowBranch,
            PendingEscrowSnapshot, PendingEscrowsSnapshot, ShardInfoSnapshot, ShardSnapshot,
            ShardSummarySnapshot,
        };

        let mut app = App {
            page,
            route_stack: vec![Page::Home, page],
            viewport_width: 60,
            ..Default::default()
        };
        match page {
            Page::ConfidentialWallet => {
                app.confidential_wallet.snapshot = Some(ConfidentialWalletSnapshot {
                    address_hex: "wallet🧪".repeat(20),
                    total_base_units: 8_000_000_001,
                    coins: vec![ConfidentialCoinSnapshot {
                        address_hex: "coin🧪".repeat(20),
                        amount_base_units: 8_000_000_001,
                    }],
                    recent_operations: Vec::new(),
                });
            }
            Page::PendingEscrows => {
                app.pending_escrows.snapshot = Some(PendingEscrowsSnapshot {
                    identity_available: true,
                    unavailable_reason: None,
                    escrows: vec![PendingEscrowSnapshot {
                        address_hex: "escrow🧪".repeat(16),
                        amount_base_units: 8_000_000_002,
                        branch: PendingEscrowBranch::Refund,
                        refund_after_global_frame: 987_654,
                    }],
                    recent_operations: Vec::new(),
                });
            }
            Page::LegacyAssets => {
                app.legacy_snapshot = Some(LegacyAssetsSnapshot {
                    identity_available: true,
                    coins: vec![LegacyCoinSnapshot {
                        address_hex: "legacy🧪".repeat(16),
                        amount_base_units: 8_000_000_003,
                        shielded: false,
                    }],
                    unshielded_total_base_units: 8_000_000_003,
                    unshielded_count: 1,
                    shielded_count: 0,
                });
            }
            Page::Hypergraph => {
                app.hypergraph.snapshot = Some(HypergraphSnapshot {
                    kind: HypergraphKind::Vertex,
                    address_hex: "vertex🧪".repeat(16),
                    set_type: "test-set".into(),
                    phase_type: "joining".into(),
                    shard_l1_hex: "l1🧪".repeat(12),
                    shard_l2_hex: "l2🧪".repeat(12),
                    present: Some(true),
                    entries: vec![HypergraphEntrySnapshot {
                        key_hex: "key-unicode🧪".repeat(16),
                        value_hex: "value-unicode🧪".repeat(16),
                    }],
                });
            }
            Page::Shards => {
                app.shards.snapshot = Some(ShardInfoSnapshot {
                    include_all: true,
                    frame_number: 42,
                    difficulty: 7,
                    world_state: "ready".into(),
                    pomw_basis: "1".into(),
                    allocation_scoring_strategy: "balanced".into(),
                    directory_collected_at_unix_seconds: None,
                    directory_stale: false,
                    directory_error: None,
                    directory_invalid_rows: 0,
                    network_provers: None,
                    summary: ShardSummarySnapshot::default(),
                    shards: vec![ShardSnapshot {
                        filter_hex: "filter-unicode🧪".repeat(16),
                        active_provers: 9,
                        ring: Some(2),
                        ring_estimated: false,
                        official: false,
                        shard_size: "1.25 GiB".into(),
                        data_shards: 12,
                        is_allocated: true,
                        materialized_frame: 40,
                        latest_frame: 42,
                        allocation_reason: Some("reward-first".into()),
                        theoretical_daily_quil: Some("12.34".into()),
                        hour_daily_quil: Some("11.22".into()),
                        day_daily_quil: Some("10.11".into()),
                        prover_counts: None,
                    }],
                });
            }
            Page::PeerResolve => {
                app.peer_resolve.report = Some(PeerAddressReport {
                    peer_id: "QmUnicode🧪".repeat(16),
                    addresses: vec![PeerAddressSnapshot {
                        kind: "stream".into(),
                        host_port: Some("node.example.test:8336".into()),
                        reach: PeerReach::Public,
                        multiaddr: format!("/dns4/{}/tcp/8336", "node🧪".repeat(24)),
                        filter: "filter-unicode🧪".repeat(16),
                    }],
                });
            }
            _ => panic!("unsupported page"),
        }
        app
    }

    #[test]
    fn narrow_read_only_lists_drill_into_details_without_queueing_requests() {
        for page in [
            Page::ConfidentialWallet,
            Page::PendingEscrows,
            Page::LegacyAssets,
            Page::Hypergraph,
            Page::Shards,
            Page::PeerResolve,
        ] {
            let mut app = narrow_read_only_page(page);

            reduce_input(&mut app, InputEvent::Enter);

            assert!(app.narrow_read_detail, "{page:?}");
            assert!(!app.hypergraph.editing, "{page:?}");
            assert!(!app.peer_resolve.editing, "{page:?}");
            assert!(app.take_confidential_wallet_request().is_none());
            assert!(app.take_pending_escrows_request().is_none());
            assert!(app.take_hypergraph_request().is_none());
            assert!(app.take_shard_request().is_none());
            assert!(app.take_peer_resolve_request().is_none());

            reduce_input(&mut app, InputEvent::Back);
            assert!(!app.narrow_read_detail, "{page:?}");
            assert_eq!(app.page, page, "{page:?}");
        }
    }

    #[test]
    fn wide_read_only_enter_never_arms_hidden_detail_and_resize_clears_it() {
        for page in [
            Page::ConfidentialWallet,
            Page::PendingEscrows,
            Page::LegacyAssets,
            Page::Hypergraph,
            Page::Shards,
            Page::PeerResolve,
        ] {
            let mut app = narrow_read_only_page(page);
            app.viewport_width = 80;
            reduce_input(&mut app, InputEvent::Enter);
            assert!(!app.narrow_read_detail, "{page:?}");

            let mut app = narrow_read_only_page(page);
            reduce_input(&mut app, InputEvent::Enter);
            assert!(app.narrow_read_detail, "{page:?}");
            reduce_input(&mut app, InputEvent::Resize(80, 24));
            assert!(!app.narrow_read_detail, "{page:?}");
        }
    }

    #[test]
    fn result_pages_use_e_to_edit_without_queueing_or_losing_enter_detail() {
        for page in [Page::Hypergraph, Page::PeerResolve] {
            for edit_key in ['e', 'E'] {
                let mut app = narrow_read_only_page(page);

                reduce_input(&mut app, InputEvent::Enter);
                assert!(app.narrow_read_detail, "{page:?}");

                reduce_input(&mut app, InputEvent::Character(edit_key));

                assert!(!app.narrow_read_detail, "{page:?} {edit_key}");
                match page {
                    Page::Hypergraph => assert!(app.hypergraph.editing, "{edit_key}"),
                    Page::PeerResolve => assert!(app.peer_resolve.editing, "{edit_key}"),
                    _ => unreachable!(),
                }
                assert!(app.take_hypergraph_request().is_none(), "{page:?}");
                assert!(app.take_peer_resolve_request().is_none(), "{page:?}");
            }
        }
    }

    fn read_only_selection(app: &App) -> usize {
        match app.page {
            Page::ConfidentialWallet => app.confidential_wallet.coin_index,
            Page::PendingEscrows => app.pending_escrows.index,
            Page::LegacyAssets => app.legacy_index,
            Page::Hypergraph => app.hypergraph.entry_index,
            Page::Shards => app.shards.index,
            Page::PeerResolve => app.peer_resolve.index,
            _ => panic!("unsupported page"),
        }
    }

    fn assert_no_read_only_request(app: &mut App) {
        assert!(app.take_confidential_wallet_request().is_none());
        assert!(app.take_pending_escrows_request().is_none());
        assert!(app.take_legacy_request().is_none());
        assert!(app.take_hypergraph_request().is_none());
        assert!(app.take_shard_request().is_none());
        assert!(app.take_peer_resolve_request().is_none());
    }

    #[test]
    fn narrow_read_only_details_isolate_hidden_list_keys_and_requests() {
        for page in [
            Page::ConfidentialWallet,
            Page::PendingEscrows,
            Page::LegacyAssets,
            Page::Hypergraph,
            Page::Shards,
            Page::PeerResolve,
        ] {
            let mut app = narrow_read_only_page(page);
            match page {
                Page::ConfidentialWallet => {
                    let snapshot = app.confidential_wallet.snapshot.as_mut().unwrap();
                    snapshot.coins.push(snapshot.coins[0].clone());
                }
                Page::PendingEscrows => {
                    let snapshot = app.pending_escrows.snapshot.as_mut().unwrap();
                    snapshot.escrows.push(snapshot.escrows[0].clone());
                }
                Page::LegacyAssets => {
                    let snapshot = app.legacy_snapshot.as_mut().unwrap();
                    snapshot.coins.push(snapshot.coins[0].clone());
                }
                Page::Hypergraph => {
                    let snapshot = app.hypergraph.snapshot.as_mut().unwrap();
                    snapshot.entries.push(snapshot.entries[0].clone());
                }
                Page::Shards => {
                    let snapshot = app.shards.snapshot.as_mut().unwrap();
                    snapshot.shards.push(snapshot.shards[0].clone());
                }
                Page::PeerResolve => {
                    let report = app.peer_resolve.report.as_mut().unwrap();
                    report.addresses.push(report.addresses[0].clone());
                }
                _ => unreachable!(),
            }
            reduce_input(&mut app, InputEvent::Enter);
            assert!(app.narrow_read_detail, "{page:?}");
            let selection = read_only_selection(&app);
            let kind = app.hypergraph.kind;
            let scope = app.shards.scope;

            for event in [
                InputEvent::Up,
                InputEvent::Down,
                InputEvent::Left,
                InputEvent::Right,
                InputEvent::Enter,
                InputEvent::Refresh,
            ] {
                let event_name = format!("{event:?}");
                reduce_input(&mut app, event);
                assert!(app.narrow_read_detail, "{page:?} {event_name}");
                assert_eq!(
                    read_only_selection(&app),
                    selection,
                    "{page:?} {event_name}"
                );
                assert_eq!(app.hypergraph.kind, kind, "{page:?} {event_name}");
                assert_eq!(app.shards.scope, scope, "{page:?} {event_name}");
                assert_no_read_only_request(&mut app);
            }
        }
    }

    #[test]
    fn required_config_path_page_cannot_back_into_an_unusable_home() {
        let mut app = App::default();
        app.require_config_path("/tmp/node".into(), Some("invalid configuration".into()));

        reduce_input(&mut app, InputEvent::Back);

        assert_eq!(app.page, Page::ConfigPath);
        assert!(app.config_path_required);
    }

    #[test]
    fn config_path_form_accepts_spaces_and_q_characters() {
        let mut app = App::default();
        app.require_config_path(String::new(), None);

        for character in "/tmp/q node Q".chars() {
            reduce_input(&mut app, InputEvent::Character(character));
        }

        assert_eq!(app.config_path_input, "/tmp/q node Q");
        assert!(app.is_text_editing());
    }

    #[test]
    fn config_path_form_backspace_deletes_the_last_character() {
        let mut app = App::default();
        app.require_config_path("/tmp/node-x".into(), None);

        reduce_input(&mut app, InputEvent::Backspace);

        assert_eq!(app.config_path_input, "/tmp/node-");
    }

    #[test]
    fn config_path_form_quit_sets_should_quit() {
        let mut app = App::default();
        app.require_config_path("/tmp/node".into(), None);

        reduce_input(&mut app, InputEvent::Quit);

        assert!(app.should_quit);
        assert_eq!(app.page, Page::ConfigPath);
    }

    #[test]
    fn config_path_form_is_locked_during_validation_except_for_quit() {
        let mut app = App::default();
        app.require_config_path("/tmp/node".into(), None);
        app.config_path_in_flight = true;
        let initial_stack = app.route_stack.clone();

        for event in [
            InputEvent::Character('x'),
            InputEvent::Backspace,
            InputEvent::Enter,
            InputEvent::Back,
            InputEvent::OpenConfigPath,
        ] {
            reduce_input(&mut app, event);
        }

        assert_eq!(app.config_path_input, "/tmp/node");
        assert!(app.queued_config_path.is_none());
        assert_eq!(app.page, Page::ConfigPath);
        assert_eq!(app.route_stack, initial_stack);

        reduce_input(&mut app, InputEvent::Quit);
        assert!(app.should_quit);
    }

    #[test]
    fn empty_config_path_submission_shows_an_error() {
        let mut app = App::default();
        app.require_config_path("   ".into(), None);

        reduce_input(&mut app, InputEvent::Enter);

        assert_eq!(
            app.config_path_error.as_deref(),
            Some("Enter a configuration directory")
        );
        assert!(app.take_config_path_request().is_none());
    }

    #[test]
    fn config_path_form_queues_trimmed_path_only_once() {
        let mut app = App::default();
        app.require_config_path(" /tmp/node config ".into(), None);

        reduce_input(&mut app, InputEvent::Enter);
        reduce_input(&mut app, InputEvent::Character('x'));
        reduce_input(&mut app, InputEvent::Enter);

        assert_eq!(
            app.take_config_path_request(),
            Some(ConfigPathRequest {
                path: "/tmp/node config".into(),
            })
        );
        assert!(app.take_config_path_request().is_none());
    }

    #[test]
    fn config_path_submission_immediately_locks_the_original_input() {
        let mut app = App::default();
        app.require_config_path(" /tmp/node-a ".into(), None);

        reduce_input(&mut app, InputEvent::Enter);

        assert!(app.config_path_in_flight);
        for event in [
            InputEvent::Character('x'),
            InputEvent::Backspace,
            InputEvent::Enter,
            InputEvent::Back,
        ] {
            reduce_input(&mut app, event);
        }
        assert_eq!(app.config_path_input, " /tmp/node-a ");
        assert_eq!(app.page, Page::ConfigPath);
        assert_eq!(
            app.take_config_path_request(),
            Some(ConfigPathRequest {
                path: "/tmp/node-a".into(),
            })
        );
    }

    #[test]
    fn configs_can_open_a_prefilled_optional_config_path_page() {
        let mut app = App::default();
        app.set_configs(vec![ConfigEntry {
            name: "current".into(),
            path: "/tmp/current config".into(),
            is_default: true,
        }]);
        app.open(Page::Configs);
        app.config_path_error = Some("old error".into());

        reduce_input(&mut app, InputEvent::OpenConfigPath);

        assert_eq!(app.page, Page::ConfigPath);
        assert_eq!(app.config_path_input, "/tmp/current config");
        assert!(app.config_path_error.is_none());
        assert!(!app.config_path_required);
    }

    #[test]
    fn applying_config_path_replaces_config_and_clears_config_scoped_state() {
        let mut app = App {
            page: Page::Logs,
            route_stack: vec![Page::Home, Page::Configs, Page::ConfigPath, Page::Logs],
            configs: vec![ConfigEntry {
                name: "old".into(),
                path: "/tmp/old".into(),
                is_default: true,
            }],
            config_index: 3,
            active_config_index: 2,
            config_path_input: "/tmp/new/.config".into(),
            config_path_error: Some("old error".into()),
            config_path_required: true,
            refresh_generation: 7,
            loading: true,
            snapshot: Some(NodeSnapshot::default()),
            error: Some("old refresh error".into()),
            log_snapshot: Some(LogSnapshot {
                core_id: 0,
                path: "old.log".into(),
                lines: vec!["old".into()],
            }),
            keys: vec![KeySnapshot {
                id: "old-key".into(),
                key_type: 0,
                public_key_hex: "aa".into(),
            }],
            aliases: vec![AliasSnapshot {
                name: "old-alias".into(),
                address_hex: "bb".into(),
                kind: String::new(),
            }],
            key_action_index: 2,
            key_form_field: 2,
            key_form_editing: true,
            key_form_id: "old-key-form".into(),
            key_form_type_index: 4,
            key_form_error: Some("old key error".into()),
            key_mutation_generation: 11,
            queued_key_mutation: Some(KeyMutationRequest {
                generation: 11,
                mutation: KeyMutation::Delete {
                    id: "old-key".into(),
                },
            }),
            pending_key_mutation: Some(KeyMutation::Delete {
                id: "pending-key".into(),
            }),
            alias_action_index: 1,
            alias_form_field: 3,
            alias_form_editing: true,
            alias_form_name: "old-name".into(),
            alias_form_address: "aabb".into(),
            alias_form_kind: "account".into(),
            alias_form_error: Some("old alias error".into()),
            alias_mutation_generation: 13,
            queued_alias_mutation: Some(AliasMutationRequest {
                generation: 13,
                mutation: AliasMutation::Delete {
                    name: "old-name".into(),
                },
            }),
            pending_alias_mutation: Some(AliasMutation::Delete {
                name: "pending-name".into(),
            }),
            operation_message: Some("old result".into()),
            operation_succeeded: Some(true),
            ..Default::default()
        };

        app.apply_config_entry(ConfigEntry {
            name: ".config".into(),
            path: "/tmp/new/.config".into(),
            is_default: true,
        });

        assert_eq!(app.configs.len(), 1);
        assert_eq!(
            app.active_config().unwrap().path,
            std::path::PathBuf::from("/tmp/new/.config")
        );
        assert_eq!(app.config_index, 0);
        assert_eq!(app.active_config_index, 0);
        assert_eq!(app.refresh_generation, 8);
        assert!(!app.loading);
        assert!(app.snapshot.is_none());
        assert!(app.error.is_none());
        assert!(app.log_snapshot.is_none());
        assert!(app.keys.is_empty());
        assert!(app.aliases.is_empty());
        assert_eq!(app.key_action_index, 0);
        assert_eq!(app.key_form_field, 0);
        assert!(!app.key_form_editing);
        assert!(app.key_form_id.is_empty());
        assert_eq!(app.key_form_type_index, 0);
        assert!(app.key_form_error.is_none());
        assert_eq!(app.key_mutation_generation, 12);
        assert!(app.queued_key_mutation.is_none());
        assert!(app.pending_key_mutation.is_none());
        assert_eq!(app.alias_action_index, 0);
        assert_eq!(app.alias_form_field, 0);
        assert!(!app.alias_form_editing);
        assert!(app.alias_form_name.is_empty());
        assert!(app.alias_form_address.is_empty());
        assert!(app.alias_form_kind.is_empty());
        assert!(app.alias_form_error.is_none());
        assert_eq!(app.alias_mutation_generation, 14);
        assert!(app.queued_alias_mutation.is_none());
        assert!(app.pending_alias_mutation.is_none());
        assert!(app.operation_message.is_none());
        assert!(app.operation_succeeded.is_none());
        assert!(app.config_path_error.is_none());
        assert!(!app.config_path_required);
        assert_eq!(app.page, Page::Home);
        assert_eq!(app.route_stack, vec![Page::Home]);
    }

    #[test]
    fn applying_config_path_invalidates_an_older_refresh_result() {
        let mut app = App {
            refresh_generation: 7,
            loading: true,
            ..Default::default()
        };

        app.apply_config_entry(ConfigEntry {
            name: "new".into(),
            path: "/tmp/new".into(),
            is_default: true,
        });
        reduce_service(
            &mut app,
            ServiceEvent::RefreshFinished {
                generation: 7,
                result: Ok(()),
            },
        );

        assert_eq!(app.refresh_generation, 8);
        assert!(!app.loading);
        assert!(app.snapshot.is_none());
    }

    #[test]
    fn config_path_error_keeps_input_and_page() {
        let mut app = App::default();
        app.require_config_path("/tmp/broken".into(), None);

        app.apply_config_path_error("missing keys.yml".into());

        assert_eq!(app.page, Page::ConfigPath);
        assert_eq!(app.config_path_input, "/tmp/broken");
        assert_eq!(app.config_path_error.as_deref(), Some("missing keys.yml"));
    }

    #[test]
    fn optional_config_path_page_returns_to_configs_on_escape() {
        let mut app = App::default();
        app.open(Page::Configs);
        app.open_config_path();

        reduce_input(&mut app, InputEvent::Back);

        assert_eq!(app.page, Page::Configs);
    }

    #[test]
    fn destructive_action_defaults_to_cancel() {
        let mut app = App::default();
        app.begin_action(crate::domain::ProverAction::Leave {
            filters: vec![vec![0xaa; 32]],
        });

        assert_eq!(app.confirm_choice, ConfirmChoice::Cancel);
        reduce_input(&mut app, InputEvent::Enter);

        assert!(app.take_operation().is_none());
        assert_eq!(app.page(), Page::Home);
    }

    #[test]
    fn confirmed_action_is_queued_only_once() {
        let mut app = App::default();
        app.begin_action(crate::domain::ProverAction::Pause {
            filter: vec![0xbb; 32],
        });
        app.confirm_choice = ConfirmChoice::Confirm;

        reduce_input(&mut app, InputEvent::Enter);
        assert!(app.take_operation().is_some());
        reduce_input(&mut app, InputEvent::Enter);
        assert!(app.take_operation().is_none());
    }

    #[test]
    fn in_flight_confirmation_locks_navigation_and_duplicate_actions() {
        for event in [
            InputEvent::Left,
            InputEvent::Right,
            InputEvent::Up,
            InputEvent::Down,
            InputEvent::Enter,
            InputEvent::Back,
            InputEvent::Refresh,
            InputEvent::Character('q'),
            InputEvent::Backspace,
        ] {
            let action = crate::domain::ProverAction::Pause {
                filter: vec![0xbb; 32],
            };
            let mut app = App {
                page: Page::Confirm,
                route_stack: vec![Page::Home, Page::ProverActions, Page::Confirm],
                pending_action: Some(action.clone()),
                confirm_choice: ConfirmChoice::Confirm,
                operation_in_flight: true,
                ..Default::default()
            };

            reduce_input(&mut app, event);

            assert_eq!(app.page, Page::Confirm);
            assert_eq!(app.confirm_choice, ConfirmChoice::Confirm);
            assert_eq!(app.pending_action.as_ref(), Some(&action));
            assert!(app.queued_operation.is_none());
            assert!(!app.loading);
        }

        let mut app = App {
            page: Page::Confirm,
            operation_in_flight: true,
            ..Default::default()
        };
        reduce_input(&mut app, InputEvent::Quit);
        assert!(app.should_quit);
    }

    #[test]
    fn in_flight_log_export_locks_form_navigation_and_duplicate_actions() {
        for event in [
            InputEvent::Left,
            InputEvent::Right,
            InputEvent::Up,
            InputEvent::Down,
            InputEvent::Enter,
            InputEvent::Back,
            InputEvent::Refresh,
            InputEvent::Character('q'),
            InputEvent::Backspace,
        ] {
            let preview = crate::domain::LogExportPreview {
                id: 7,
                destination: "/tmp/export.tar.gz".into(),
                token_map_path: "/tmp/export.map.txt".into(),
                window_since: "from".into(),
                window_until: "until".into(),
                source: "node.log".into(),
                lines_read: 10,
                lines_kept: 8,
                redactions: vec![("peer_id".into(), 2, 1)],
                samples: vec!["sample".into()],
                read_errors: 0,
            };
            let mut app = App {
                page: Page::LogExport,
                route_stack: vec![Page::Home, Page::Logs, Page::LogExport],
                log_export: LogExportState {
                    in_flight: true,
                    field_index: 1,
                    window_index: 2,
                    level_index: 2,
                    choice: ConfirmChoice::Confirm,
                    preview: Some(preview.clone()),
                    ..Default::default()
                },
                ..Default::default()
            };

            reduce_input(&mut app, event);

            assert_eq!(app.page, Page::LogExport);
            assert_eq!(app.log_export.field_index, 1);
            assert_eq!(app.log_export.window_index, 2);
            assert_eq!(app.log_export.level_index, 2);
            assert_eq!(app.log_export.choice, ConfirmChoice::Confirm);
            assert_eq!(app.log_export.preview.as_ref(), Some(&preview));
            assert!(app.log_export.queued_prepare.is_none());
            assert!(app.log_export.queued_finish.is_none());
        }

        let mut app = App {
            page: Page::LogExport,
            log_export: LogExportState {
                in_flight: true,
                ..Default::default()
            },
            ..Default::default()
        };
        reduce_input(&mut app, InputEvent::Quit);
        assert!(app.should_quit);
    }

    #[test]
    fn leaving_confirmation_cannot_leak_an_old_action_into_the_next_one() {
        let mut app = App::default();
        app.begin_action(crate::domain::ProverAction::Pause {
            filter: vec![0xaa; 32],
        });
        reduce_input(&mut app, InputEvent::Back);

        app.begin_alias_mutation(AliasMutation::Delete {
            name: "wallet".into(),
        });
        app.confirm_choice = ConfirmChoice::Confirm;
        reduce_input(&mut app, InputEvent::Enter);

        assert!(app.take_operation().is_none());
        assert!(matches!(
            app.take_alias_mutation().map(|request| request.mutation),
            Some(AliasMutation::Delete { name }) if name == "wallet"
        ));
    }

    #[test]
    fn active_allocation_offers_only_pause_and_leave() {
        let app = app_with_status(crate::domain::AllocationStatus::Active);

        let labels = available_actions(&app)
            .iter()
            .map(crate::domain::ProverAction::label)
            .collect::<Vec<_>>();

        assert_eq!(labels, vec!["Pause shard", "Leave shard"]);
    }

    #[test]
    fn joining_allocation_offers_confirm_and_reject() {
        let app = app_with_status(crate::domain::AllocationStatus::Joining);

        let labels = available_actions(&app)
            .iter()
            .map(crate::domain::ProverAction::label)
            .collect::<Vec<_>>();

        assert_eq!(labels, vec!["Reject proposal", "Confirm proposal"]);
    }

    #[test]
    fn prover_joining_before_confirmation_window_has_no_wire_action() {
        let mut app = app_with_status(crate::domain::AllocationStatus::Joining);
        let snapshot = app.snapshot.as_mut().unwrap();
        snapshot.last_received_frame = 150;
        snapshot.allocations[0].join_frame_number = 150;

        assert!(available_actions(&app).is_empty());
    }

    #[test]
    fn prover_joining_in_confirmation_window_offers_hint_actions() {
        let mut app = app_with_status(crate::domain::AllocationStatus::Joining);
        let snapshot = app.snapshot.as_mut().unwrap();
        snapshot.last_received_frame = 250;
        snapshot.allocations[0].join_frame_number = 150;

        let labels = available_actions(&app)
            .iter()
            .map(crate::domain::ProverAction::label)
            .collect::<Vec<_>>();

        assert_eq!(labels, vec!["Reject proposal", "Confirm proposal"]);
    }

    #[test]
    fn prover_leaving_in_confirmation_window_offers_hint_actions() {
        let mut app = app_with_status(crate::domain::AllocationStatus::Leaving);
        let snapshot = app.snapshot.as_mut().unwrap();
        snapshot.last_received_frame = 250;
        snapshot.allocations[0].leave_frame_number = 150;

        let labels = available_actions(&app)
            .iter()
            .map(crate::domain::ProverAction::label)
            .collect::<Vec<_>>();

        assert_eq!(labels, vec!["Reject proposal", "Confirm proposal"]);
    }

    #[test]
    fn prover_leaving_before_confirmation_window_has_no_wire_action() {
        let mut app = app_with_status(crate::domain::AllocationStatus::Leaving);
        let snapshot = app.snapshot.as_mut().unwrap();
        snapshot.last_received_frame = 150;
        snapshot.allocations[0].leave_frame_number = 150;

        assert!(available_actions(&app).is_empty());
    }

    #[test]
    fn prover_selection_is_stable_by_allocation_index_and_reaches_an_orphan() {
        let allocations = (0..8)
            .map(|index| crate::domain::AllocationSnapshot {
                filter_hex: format!("{index:02x}").repeat(32),
                status: crate::domain::AllocationStatus::Joining,
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let workers = allocations[..7]
            .iter()
            .enumerate()
            .map(|(index, allocation)| crate::domain::WorkerSnapshot {
                core_id: index as u32,
                filter_hex: allocation.filter_hex.clone(),
                available_storage: 0,
                total_storage: 0,
                manually_managed: false,
                execution: None,
            })
            .collect();
        let mut app = App {
            page: Page::Prover,
            snapshot: Some(NodeSnapshot {
                epoch: Some(crate::domain::EpochSnapshot {
                    current: 2,
                    length_frames: 100,
                }),
                last_received_frame: 250,
                allocations,
                workers,
                ..Default::default()
            }),
            ..Default::default()
        };

        for _ in 0..7 {
            reduce_input(&mut app, InputEvent::Down);
        }

        assert_eq!(app.allocation_index, 7);
        assert_eq!(
            available_actions(&app)
                .iter()
                .map(crate::domain::ProverAction::label)
                .collect::<Vec<_>>(),
            vec!["Reject proposal", "Confirm proposal"]
        );
    }

    #[test]
    fn prover_actions_bind_workers_only_by_exact_filter() {
        let allocation_filter = "ab".repeat(32);
        let partial_filter = format!("{}cd", "ab".repeat(31));
        let app = App {
            page: Page::Prover,
            snapshot: Some(NodeSnapshot {
                epoch: Some(crate::domain::EpochSnapshot {
                    current: 2,
                    length_frames: 100,
                }),
                last_received_frame: 250,
                allocations: vec![crate::domain::AllocationSnapshot {
                    filter_hex: allocation_filter,
                    status: crate::domain::AllocationStatus::Historic,
                    ..Default::default()
                }],
                workers: vec![crate::domain::WorkerSnapshot {
                    core_id: 9,
                    filter_hex: partial_filter,
                    available_storage: 0,
                    total_storage: 0,
                    manually_managed: false,
                    execution: None,
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        assert!(available_actions(&app).is_empty());
    }

    #[test]
    fn prover_join_fallback_requires_a_known_terminal_state_and_exact_worker() {
        fn app_for(
            status: crate::domain::AllocationStatus,
            worker_filter: Option<String>,
            worker_info_error: Option<String>,
        ) -> App {
            let allocation_filter = "ab".repeat(32);
            App {
                page: Page::Prover,
                snapshot: Some(NodeSnapshot {
                    epoch: Some(crate::domain::EpochSnapshot {
                        current: 2,
                        length_frames: 100,
                    }),
                    last_received_frame: 250,
                    allocations: vec![crate::domain::AllocationSnapshot {
                        filter_hex: allocation_filter,
                        status,
                        ..Default::default()
                    }],
                    workers: worker_filter
                        .map(|filter_hex| crate::domain::WorkerSnapshot {
                            core_id: 42,
                            filter_hex,
                            available_storage: 0,
                            total_storage: 0,
                            manually_managed: false,
                            execution: None,
                        })
                        .into_iter()
                        .collect(),
                    worker_info_error,
                    ..Default::default()
                }),
                ..Default::default()
            }
        }

        let exact_filter = "ab".repeat(32);
        assert!(available_actions(&app_for(
            crate::domain::AllocationStatus::Unknown,
            Some(exact_filter.clone()),
            None,
        ))
        .is_empty());

        for status in [
            crate::domain::AllocationStatus::Rejected,
            crate::domain::AllocationStatus::Kicked,
            crate::domain::AllocationStatus::Historic,
        ] {
            assert_eq!(
                available_actions(&app_for(status, Some(exact_filter.clone()), None)),
                vec![crate::domain::ProverAction::Join {
                    filters: vec![vec![0xab; 32]],
                    delegate: Vec::new(),
                    worker_ids: vec![42],
                }],
                "{status:?}"
            );
        }

        for (name, worker_filter, worker_error) in [
            ("filter mismatch", Some("ac".repeat(32)), None),
            (
                "worker unavailable",
                Some(exact_filter.clone()),
                Some("worker rpc unavailable".into()),
            ),
            ("unbound", None, None),
        ] {
            assert!(
                available_actions(&app_for(
                    crate::domain::AllocationStatus::Historic,
                    worker_filter,
                    worker_error,
                ))
                .is_empty(),
                "{name}"
            );
        }
    }

    #[test]
    fn prover_actions_are_disabled_when_epoch_telemetry_is_missing() {
        let mut app = app_with_status(crate::domain::AllocationStatus::Joining);
        app.snapshot.as_mut().unwrap().epoch = None;

        assert!(available_actions(&app).is_empty());
    }

    #[test]
    fn prover_projection_clamps_the_allocation_selection() {
        let mut app = App {
            allocation_index: 7,
            expanded_allocation: Some(7),
            refresh_generation: 4,
            loading: true,
            ..Default::default()
        };

        app.apply_core_state_projection(&CoreStateSnapshot {
            node_snapshot: Some(NodeSnapshot {
                allocations: vec![crate::domain::AllocationSnapshot::default(); 2],
                ..Default::default()
            }),
            ..Default::default()
        });
        reduce_service(
            &mut app,
            ServiceEvent::RefreshFinished {
                generation: 4,
                result: Ok(()),
            },
        );

        assert_eq!(app.allocation_index, 1);
        assert_eq!(app.expanded_allocation, None);
    }

    #[test]
    fn prover_projection_clamps_action_selection_to_the_only_remaining_action() {
        let mut app = app_with_status(crate::domain::AllocationStatus::Active);
        app.open(Page::ProverActions);
        app.action_index = 1;
        app.loading = true;
        app.refresh_generation = 7;
        let snapshot = app_with_status(crate::domain::AllocationStatus::Rejected)
            .snapshot
            .unwrap();

        app.apply_core_state_projection(&CoreStateSnapshot {
            node_snapshot: Some(snapshot),
            ..Default::default()
        });
        reduce_service(
            &mut app,
            ServiceEvent::RefreshFinished {
                generation: 7,
                result: Ok(()),
            },
        );

        assert_eq!(app.page(), Page::Prover);
        assert_eq!(app.action_index, 0);
        reduce_input(&mut app, InputEvent::Character('a'));
        assert_eq!(app.page(), Page::ProverActions);
        reduce_input(&mut app, InputEvent::Enter);
        assert!(matches!(
            app.pending_action,
            Some(crate::domain::ProverAction::Join { ref worker_ids, .. })
                if worker_ids == &[7]
        ));
    }

    #[test]
    fn prover_projection_resets_action_selection_when_no_actions_remain() {
        let mut app = app_with_status(crate::domain::AllocationStatus::Active);
        app.open(Page::ProverActions);
        app.action_index = 1;
        app.loading = true;
        app.refresh_generation = 8;
        let snapshot = app_with_status(crate::domain::AllocationStatus::Unknown)
            .snapshot
            .unwrap();

        app.apply_core_state_projection(&CoreStateSnapshot {
            node_snapshot: Some(snapshot),
            ..Default::default()
        });
        reduce_service(
            &mut app,
            ServiceEvent::RefreshFinished {
                generation: 8,
                result: Ok(()),
            },
        );

        assert_eq!(app.page(), Page::Prover);
        assert_eq!(app.action_index, 0);
        reduce_input(&mut app, InputEvent::Enter);
        assert_eq!(app.page(), Page::Prover);
        assert!(app.pending_action.is_none());
    }

    #[test]
    fn horizontal_keys_do_not_hide_or_replace_expanded_details() {
        let mut app = App {
            page: Page::Prover,
            expanded_allocation: Some(0),
            snapshot: Some(NodeSnapshot {
                allocations: vec![crate::domain::AllocationSnapshot::default(); 2],
                ..Default::default()
            }),
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Left);
        reduce_input(&mut app, InputEvent::Right);
        assert_eq!(app.expanded_allocation, Some(0));
    }

    #[test]
    fn confirmation_refresh_is_a_noop_and_keeps_the_pending_action() {
        let mut app = app_with_status(crate::domain::AllocationStatus::Active);
        app.open(Page::ProverActions);
        app.begin_action(crate::domain::ProverAction::Pause {
            filter: vec![0xaa; 32],
        });
        let before = format!("{app:#?}");
        assert_eq!(app.page(), Page::Confirm);

        reduce_input(&mut app, InputEvent::Refresh);

        assert_eq!(format!("{app:#?}"), before);
    }

    #[test]
    fn unsupported_pages_ignore_refresh_without_state_or_queue_side_effects() {
        let pages = [
            Page::Section(Section::Node),
            Page::Section(Section::Prover),
            Page::Section(Section::Wallet),
            Page::Section(Section::Identity),
            Page::Section(Section::Data),
            Page::Section(Section::Diagnostics),
            Page::Configs,
            Page::ConfigPath,
            Page::Diagnostics,
            Page::DiagnosticsIdentity,
            Page::RemoteHealth,
            Page::KeyActions,
            Page::KeyCreate,
            Page::AliasActions,
            Page::AliasEdit,
            Page::ProverActions,
            Page::Confirm,
            Page::OperationResult,
            Page::Help,
        ];

        for page in pages {
            let mut app = App {
                page,
                route_stack: vec![Page::Home, page],
                refresh_generation: 41,
                error: Some("preserve the existing error".into()),
                pending_action: Some(crate::domain::ProverAction::Pause {
                    filter: vec![0xaa; 32],
                }),
                confirm_choice: ConfirmChoice::Confirm,
                ..Default::default()
            };
            let before = format!("{app:#?}");

            reduce_input(&mut app, InputEvent::Refresh);

            assert_eq!(format!("{app:#?}"), before, "{page:?}");
        }
    }

    #[test]
    fn node_snapshot_pages_refresh_only_the_node_snapshot() {
        for page in [Page::Home, Page::NodeStatus, Page::Prover] {
            let mut app = App {
                page,
                refresh_generation: 41,
                error: Some("old error".into()),
                ..Default::default()
            };

            reduce_input(&mut app, InputEvent::Refresh);

            assert_eq!(app.page, page);
            assert_eq!(app.refresh_generation, 42, "{page:?}");
            assert!(app.loading, "{page:?}");
            assert!(app.error.is_none(), "{page:?}");
            assert!(app.queued_log.is_none(), "{page:?}");
            assert!(app.runtime.queued.is_none(), "{page:?}");
            assert!(app.health.queued.is_none(), "{page:?}");
        }
    }

    #[test]
    fn prover_loading_blocks_opening_or_confirming_write_actions() {
        let mut app = app_with_status(crate::domain::AllocationStatus::Active);

        reduce_input(&mut app, InputEvent::Refresh);
        reduce_input(&mut app, InputEvent::Enter);
        assert_eq!(app.page(), Page::Prover);

        app.open(Page::ProverActions);
        reduce_input(&mut app, InputEvent::Enter);
        assert_eq!(app.page(), Page::ProverActions);
        assert!(app.pending_action.is_none());
    }

    #[test]
    fn refresh_finished_invalidates_an_action_created_after_refresh_started() {
        let mut app = app_with_status(crate::domain::AllocationStatus::Active);
        reduce_input(&mut app, InputEvent::Refresh);
        let generation = app.refresh_generation;
        app.begin_action(crate::domain::ProverAction::Pause {
            filter: vec![0xaa; 32],
        });
        app.confirm_choice = ConfirmChoice::Confirm;

        app.apply_core_state_projection(&CoreStateSnapshot {
            node_snapshot: Some(NodeSnapshot {
                epoch: Some(crate::domain::EpochSnapshot {
                    current: 0,
                    length_frames: 100,
                }),
                allocations: vec![crate::domain::AllocationSnapshot {
                    filter_hex: "aa".repeat(32),
                    status: crate::domain::AllocationStatus::Paused,
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        });
        reduce_service(
            &mut app,
            ServiceEvent::RefreshFinished {
                generation,
                result: Ok(()),
            },
        );

        assert_eq!(app.page(), Page::Prover);
        assert!(app.pending_action.is_none());
        assert_eq!(app.confirm_choice, ConfirmChoice::Cancel);
        app.confirm_choice = ConfirmChoice::Confirm;
        reduce_input(&mut app, InputEvent::Enter);
        assert!(app.take_operation().is_none());
    }

    #[test]
    fn refresh_finished_does_not_cancel_an_already_submitted_operation() {
        let mut app = app_with_status(crate::domain::AllocationStatus::Active);
        app.begin_action(crate::domain::ProverAction::Pause {
            filter: vec![0xaa; 32],
        });
        app.confirm_choice = ConfirmChoice::Confirm;
        reduce_input(&mut app, InputEvent::Enter);
        assert!(app.operation_in_flight);
        let pending = app.pending_action.clone();
        let generation = app.refresh_generation;

        app.apply_core_state_projection(&CoreStateSnapshot {
            node_snapshot: Some(NodeSnapshot::default()),
            ..Default::default()
        });
        reduce_service(
            &mut app,
            ServiceEvent::RefreshFinished {
                generation,
                result: Ok(()),
            },
        );

        assert_eq!(app.page(), Page::Confirm);
        assert_eq!(app.pending_action, pending);
        assert!(app.operation_in_flight);
    }

    #[test]
    fn operation_result_opens_result_page_and_clears_pending_action() {
        let mut app = App::default();
        app.begin_action(crate::domain::ProverAction::Pause {
            filter: vec![0xaa; 32],
        });
        app.confirm_choice = ConfirmChoice::Confirm;
        reduce_input(&mut app, InputEvent::Enter);
        let generation = app.take_operation().unwrap().generation;

        reduce_service(
            &mut app,
            ServiceEvent::OperationFinished {
                generation,
                result: Ok(crate::domain::OperationResult {
                    message: "Pause operation submitted".into(),
                    outcome: crate::domain::OperationOutcome::Submitted,
                }),
            },
        );

        assert_eq!(app.page(), Page::OperationResult);
        assert!(app.pending_action.is_none());
        assert_eq!(
            app.operation_message.as_deref(),
            Some("Pause operation submitted")
        );
    }

    #[test]
    fn alias_form_accepts_q_and_queues_valid_put_after_confirmation() {
        let mut app = App::default();
        app.open(Page::Aliases);
        app.open(Page::AliasActions);
        reduce_input(&mut app, InputEvent::Enter);
        assert_eq!(app.page(), Page::AliasEdit);

        reduce_input(&mut app, InputEvent::Enter);
        reduce_input(&mut app, InputEvent::Character('q'));
        reduce_input(&mut app, InputEvent::Enter);
        app.alias_form_address = "0xaabb".into();
        app.alias_form_kind = "account".into();
        app.alias_form_field = 3;
        reduce_input(&mut app, InputEvent::Enter);

        assert_eq!(app.page(), Page::Confirm);
        app.confirm_choice = ConfirmChoice::Confirm;
        reduce_input(&mut app, InputEvent::Enter);
        let request = app.take_alias_mutation().unwrap();
        assert_eq!(
            request.mutation,
            AliasMutation::Put {
                name: "q".into(),
                address_hex: "aabb".into(),
                kind: "account".into(),
            }
        );
        assert!(app.take_alias_mutation().is_none());
    }

    #[test]
    fn successful_alias_mutation_reloads_aliases() {
        let mut app = App {
            page: Page::Confirm,
            route_stack: vec![Page::Home, Page::Aliases, Page::AliasActions, Page::Confirm],
            pending_alias_mutation: Some(AliasMutation::Delete { name: "old".into() }),
            confirm_choice: ConfirmChoice::Confirm,
            ..Default::default()
        };
        reduce_input(&mut app, InputEvent::Enter);
        let generation = app.take_alias_mutation().unwrap().generation;

        reduce_service(
            &mut app,
            ServiceEvent::AliasMutationFinished {
                generation,
                result: Ok(crate::domain::OperationResult {
                    message: "Deleted".into(),
                    outcome: crate::domain::OperationOutcome::Submitted,
                }),
            },
        );

        assert_eq!(app.page(), Page::OperationResult);
        assert!(app.pending_alias_mutation.is_none());
        assert!(app.take_alias_request().is_some());
    }

    #[test]
    fn key_create_form_queues_selected_type_after_confirmation() {
        let mut app = App {
            page: Page::KeyCreate,
            route_stack: vec![Page::Home, Page::Keys, Page::KeyActions, Page::KeyCreate],
            key_form_id: "wallet-signing".into(),
            key_form_type_index: 4,
            key_form_field: 2,
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);
        assert_eq!(app.page(), Page::Confirm);
        app.confirm_choice = ConfirmChoice::Confirm;
        reduce_input(&mut app, InputEvent::Enter);

        assert_eq!(
            app.take_key_mutation().unwrap().mutation,
            KeyMutation::Create {
                id: "wallet-signing".into(),
                key_type: 8,
            }
        );
    }

    #[test]
    fn protected_node_key_has_no_delete_action() {
        let app = App {
            keys: vec![KeySnapshot {
                id: "q-prover-key".into(),
                key_type: 8,
                public_key_hex: "aa".into(),
            }],
            ..Default::default()
        };

        assert!(!key_delete_allowed(&app));
    }

    #[test]
    fn successful_key_mutation_reloads_keys() {
        let mut app = App {
            page: Page::Confirm,
            route_stack: vec![Page::Home, Page::Keys, Page::KeyActions, Page::Confirm],
            pending_key_mutation: Some(KeyMutation::Delete {
                id: "wallet-signing".into(),
            }),
            confirm_choice: ConfirmChoice::Confirm,
            ..Default::default()
        };
        reduce_input(&mut app, InputEvent::Enter);
        let generation = app.take_key_mutation().unwrap().generation;

        reduce_service(
            &mut app,
            ServiceEvent::KeyMutationFinished {
                generation,
                result: Ok(crate::domain::OperationResult {
                    message: "Deleted".into(),
                    outcome: crate::domain::OperationOutcome::Submitted,
                }),
            },
        );

        assert_eq!(app.page(), Page::OperationResult);
        assert!(app.pending_key_mutation.is_none());
        assert!(app.take_key_request().is_some());
    }

    #[test]
    fn alias_form_rejects_invalid_hex_before_confirmation() {
        let mut app = App {
            page: Page::AliasEdit,
            route_stack: vec![Page::Home, Page::AliasEdit],
            alias_form_name: "wallet".into(),
            alias_form_address: "not-hex".into(),
            alias_form_field: 3,
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);

        assert_eq!(app.page(), Page::AliasEdit);
        assert_eq!(
            app.alias_form_error.as_deref(),
            Some("Address is not valid hexadecimal")
        );
        assert!(app.pending_alias_mutation.is_none());
    }

    #[test]
    fn second_action_is_ignored_while_request_is_in_flight() {
        let first = crate::domain::ProverAction::Pause {
            filter: vec![0xaa; 32],
        };
        let second = crate::domain::ProverAction::Leave {
            filters: vec![vec![0xbb; 32]],
        };
        let mut app = App::default();
        app.begin_action(first.clone());
        app.confirm_choice = ConfirmChoice::Confirm;
        reduce_input(&mut app, InputEvent::Enter);
        assert!(app.take_operation().is_some());

        app.begin_action(second);

        assert_eq!(app.pending_action, Some(first));
    }

    fn app_with_status(status: crate::domain::AllocationStatus) -> App {
        let filter = "aa".repeat(32);
        App {
            page: Page::Prover,
            snapshot: Some(NodeSnapshot {
                epoch: Some(crate::domain::EpochSnapshot {
                    current: 0,
                    length_frames: 100,
                }),
                allocations: vec![crate::domain::AllocationSnapshot {
                    filter_hex: filter.clone(),
                    status,
                    ..Default::default()
                }],
                workers: vec![crate::domain::WorkerSnapshot {
                    core_id: 7,
                    filter_hex: filter,
                    available_storage: 0,
                    total_storage: 0,
                    manually_managed: false,
                    execution: None,
                }],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn shard_action_app(status: crate::domain::AllocationStatus) -> App {
        let mut app = app_with_status(status);
        app.page = Page::Shards;
        app.route_stack = vec![Page::Home, Page::Shards];
        app.shards.scope = ShardScope::All;
        app.shards.snapshot = Some(crate::domain::ShardInfoSnapshot {
            include_all: true,
            frame_number: 0,
            difficulty: 0,
            world_state: String::new(),
            pomw_basis: String::new(),
            allocation_scoring_strategy: String::new(),
            directory_collected_at_unix_seconds: None,
            directory_stale: false,
            directory_error: None,
            directory_invalid_rows: 0,
            network_provers: None,
            summary: Default::default(),
            shards: vec![crate::domain::ShardSnapshot {
                filter_hex: "aa".repeat(32),
                active_provers: 0,
                ring: None,
                ring_estimated: false,
                official: false,
                shard_size: String::new(),
                data_shards: 0,
                is_allocated: true,
                materialized_frame: 0,
                latest_frame: 0,
                allocation_reason: None,
                theoretical_daily_quil: None,
                hour_daily_quil: None,
                day_daily_quil: None,
                prover_counts: None,
            }],
        });
        app
    }

    #[test]
    fn shard_action_key_selects_matching_allocation_and_opens_actions() {
        let mut app = shard_action_app(crate::domain::AllocationStatus::Active);

        reduce_input(&mut app, InputEvent::Character('a'));

        assert_eq!(app.page, Page::ProverActions);
        assert_eq!(app.allocation_index, 0);
        assert_eq!(
            app.route_stack,
            vec![Page::Home, Page::Shards, Page::ProverActions]
        );
    }

    #[test]
    fn shard_action_menu_reuses_the_existing_leave_confirmation_flow() {
        let mut app = shard_action_app(crate::domain::AllocationStatus::Active);

        reduce_input(&mut app, InputEvent::Character('a'));
        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Enter);

        assert_eq!(app.page, Page::Confirm);
        assert!(matches!(
            app.pending_action,
            Some(crate::domain::ProverAction::Leave { ref filters })
                if filters == &[vec![0xaa; 32]]
        ));
    }

    #[test]
    fn shard_action_key_does_not_open_actions_for_unallocated_shard() {
        let mut app = shard_action_app(crate::domain::AllocationStatus::Active);
        let shard = &mut app.shards.snapshot.as_mut().unwrap().shards[0];
        shard.filter_hex = "bb".repeat(32);
        shard.is_allocated = false;

        reduce_input(&mut app, InputEvent::Character('a'));

        assert_eq!(app.page, Page::Shards);
        assert!(app.shards.notice.is_some());
    }

    #[test]
    fn shard_action_key_explains_future_manual_action_window() {
        let mut app = shard_action_app(crate::domain::AllocationStatus::Leaving);
        app.locale = crate::i18n::Locale::TraditionalChinese;
        let snapshot = app.snapshot.as_mut().unwrap();
        snapshot.last_received_frame = 150;
        snapshot.allocations[0].leave_frame_number = 150;

        reduce_input(&mut app, InputEvent::Character('A'));

        assert_eq!(app.page, Page::Shards);
        let notice = app.shards.notice.as_deref().unwrap();
        assert!(notice.contains("本機分配狀態: 離開"), "{notice}");
        assert!(notice.contains("拒絕/確認"), "{notice}");
        assert!(notice.contains("Frame 200"), "{notice}");
        assert!(notice.contains("剩餘 Frame 50"), "{notice}");
    }

    #[test]
    fn shard_action_key_explains_automatic_departure() {
        let mut app = shard_action_app(crate::domain::AllocationStatus::Leaving);
        app.locale = crate::i18n::Locale::TraditionalChinese;
        let snapshot = app.snapshot.as_mut().unwrap();
        snapshot.last_received_frame = 150;
        snapshot.allocations[0].leave_confirm_frame_number = 150;

        reduce_input(&mut app, InputEvent::Character('A'));

        let notice = app.shards.notice.as_deref().unwrap();
        assert!(notice.contains("本機分配狀態: 退出"), "{notice}");
        assert!(notice.contains("無需手動操作"), "{notice}");
        assert!(notice.contains("預設動作: 退出"), "{notice}");
        assert!(notice.contains("Frame 200"), "{notice}");
    }

    #[test]
    fn shard_action_key_distinguishes_unloaded_node_status_from_no_match() {
        let mut app = shard_action_app(crate::domain::AllocationStatus::Active);
        app.snapshot = None;

        reduce_input(&mut app, InputEvent::Character('A'));

        assert_eq!(
            app.shards.notice.as_deref(),
            Some(t(app.locale(), UiText::NodeStatusNotLoadedRToRefresh))
        );
    }

    #[test]
    fn shard_action_key_explains_invalid_filter_length_instead_of_offering_actions() {
        let mut app = shard_action_app(crate::domain::AllocationStatus::Active);
        app.locale = crate::i18n::Locale::TraditionalChinese;
        app.snapshot.as_mut().unwrap().allocations[0].filter_hex = "aa".into();
        app.shards.snapshot.as_mut().unwrap().shards[0].filter_hex = "aa".into();

        reduce_input(&mut app, InputEvent::Character('A'));

        let notice = app.shards.notice.as_deref().unwrap();
        assert_eq!(notice, t(app.locale(), UiText::SelectedFilterInvalid));
    }

    #[test]
    fn shard_action_key_explains_loading_and_failed_node_status() {
        let mut loading = shard_action_app(crate::domain::AllocationStatus::Active);
        loading.locale = crate::i18n::Locale::TraditionalChinese;
        loading.loading = true;
        reduce_input(&mut loading, InputEvent::Character('A'));
        assert_eq!(
            loading.shards.notice.as_deref(),
            Some(t(loading.locale(), UiText::LoadingNodeStatusMessage))
        );

        let mut failed = shard_action_app(crate::domain::AllocationStatus::Active);
        failed.error = Some("offline".into());
        reduce_input(&mut failed, InputEvent::Character('A'));
        let notice = failed.shards.notice.as_deref().unwrap();
        assert!(notice.contains("offline"), "{notice}");
        assert!(!notice.contains("Next action"), "{notice}");
    }

    #[test]
    fn back_pops_one_route_and_never_exits_at_root() {
        let mut app = App::default();
        app.open(Page::Prover);
        app.open(Page::Help);

        reduce_input(&mut app, InputEvent::Back);
        assert_eq!(app.page(), Page::Prover);
        reduce_input(&mut app, InputEvent::Back);
        reduce_input(&mut app, InputEvent::Back);

        assert_eq!(app.page(), Page::Home);
        assert!(!app.should_quit);
    }

    #[test]
    fn down_wraps_and_enter_opens_selected_page() {
        let mut app = App::default();
        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Enter);
        assert_eq!(app.page, Page::Section(Section::Node));
        reduce_input(&mut app, InputEvent::Enter);
        assert_eq!(app.page, Page::NodeStatus);
    }

    #[test]
    fn config_selection_is_queued_without_becoming_active_before_activation() {
        let mut app = App::default();
        app.set_configs(vec![
            crate::config::ConfigEntry {
                name: "mainnet".into(),
                path: "/tmp/mainnet".into(),
                is_default: true,
            },
            crate::config::ConfigEntry {
                name: "backup".into(),
                path: "/tmp/backup".into(),
                is_default: false,
            },
        ]);
        app.open(Page::Configs);

        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Enter);

        assert_eq!(app.active_config().unwrap().name, "mainnet");
        assert_eq!(app.take_config_selection().unwrap().name, "backup");
        assert_eq!(app.active_config().unwrap().name, "mainnet");
        assert!(app.take_config_selection().is_none());
    }

    #[test]
    fn queued_config_selection_does_not_interrupt_the_current_refresh_until_taken() {
        let mut app = App {
            refresh_generation: 7,
            loading: true,
            ..Default::default()
        };
        app.set_configs(vec![
            ConfigEntry {
                name: "mainnet".into(),
                path: "/tmp/mainnet".into(),
                is_default: true,
            },
            ConfigEntry {
                name: "backup".into(),
                path: "/tmp/backup".into(),
                is_default: false,
            },
        ]);
        app.open(Page::Configs);

        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Enter);
        assert_eq!(app.active_config().unwrap().name, "mainnet");
        assert_eq!(app.refresh_generation, 7);

        let active_config = app.active_config().cloned();
        app.apply_core_state_projection(&CoreStateSnapshot {
            active_config,
            node_snapshot: Some(NodeSnapshot {
                peer_id: "current-peer".into(),
                ..Default::default()
            }),
            ..Default::default()
        });
        reduce_service(
            &mut app,
            ServiceEvent::RefreshFinished {
                generation: 7,
                result: Ok(()),
            },
        );
        assert!(!app.loading);
        assert_eq!(app.snapshot.as_ref().unwrap().peer_id, "current-peer");

        assert_eq!(app.take_config_selection().unwrap().name, "backup");
        assert_eq!(app.active_config().unwrap().name, "mainnet");
        assert_eq!(app.refresh_generation, 7);
        assert_eq!(app.snapshot.as_ref().unwrap().peer_id, "current-peer");
    }

    #[test]
    fn active_config_enter_does_not_queue_a_selection() {
        let mut app = App::default();
        app.set_configs(vec![ConfigEntry {
            name: "mainnet".into(),
            path: "/tmp/mainnet".into(),
            is_default: true,
        }]);
        app.open(Page::Configs);

        reduce_input(&mut app, InputEvent::Enter);
        reduce_input(&mut app, InputEvent::Enter);

        assert!(app.queued_config_index.is_none());
        assert!(app.take_config_selection().is_none());
    }

    #[test]
    fn repeated_enter_does_not_replace_the_same_queued_config_selection() {
        let mut app = App::default();
        app.set_configs(vec![
            ConfigEntry {
                name: "mainnet".into(),
                path: "/tmp/mainnet".into(),
                is_default: true,
            },
            ConfigEntry {
                name: "backup".into(),
                path: "/tmp/backup".into(),
                is_default: false,
            },
        ]);
        app.open(Page::Configs);
        reduce_input(&mut app, InputEvent::Down);

        reduce_input(&mut app, InputEvent::Enter);
        assert_eq!(app.queued_config_index, Some(1));
        reduce_input(&mut app, InputEvent::Enter);
        assert_eq!(app.queued_config_index, Some(1));

        assert_eq!(app.take_config_selection().unwrap().name, "backup");
        assert!(app.take_config_selection().is_none());
    }

    #[test]
    fn config_candidate_keeps_all_config_scoped_data_until_activation() {
        let mut app = App::default();
        app.set_configs(vec![
            crate::config::ConfigEntry {
                name: "mainnet".into(),
                path: "/tmp/mainnet".into(),
                is_default: true,
            },
            crate::config::ConfigEntry {
                name: "backup".into(),
                path: "/tmp/backup".into(),
                is_default: false,
            },
        ]);
        app.keys.push(KeySnapshot {
            id: "old-key".into(),
            key_type: 0,
            public_key_hex: "aa".into(),
        });
        app.aliases.push(AliasSnapshot {
            name: "old-alias".into(),
            address_hex: "bb".into(),
            kind: String::new(),
        });
        app.log_snapshot = Some(LogSnapshot {
            core_id: 0,
            path: "old.log".into(),
            lines: vec!["old".into()],
        });
        app.reward_snapshot = Some(RewardSnapshot {
            identity: crate::domain::RewardIdentity::Standard,
            prover_address_hex: Some("cc".repeat(32)),
            public_key_hex: Some("dd".repeat(897)),
            legacy_account_address_hex: None,
            recipient_address_hex: Some("ee".repeat(32)),
            claimable_base_units: Some(1),
            cited_frame: Some(1),
            proof_verified: true,
            witness_error: None,
        });
        app.legacy_snapshot = Some(LegacyAssetsSnapshot {
            identity_available: true,
            ..Default::default()
        });
        app.open(Page::Configs);

        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Enter);
        assert!(app.take_config_selection().is_some());

        assert_eq!(app.keys[0].id, "old-key");
        assert_eq!(app.aliases[0].name, "old-alias");
        assert!(app.log_snapshot.is_some());
        assert!(app.reward_snapshot.is_some());
        assert!(app.legacy_snapshot.is_some());
    }

    #[test]
    fn config_candidate_is_committed_only_after_core_projection_succeeds() {
        let old = ConfigEntry {
            name: "old".into(),
            path: "/tmp/old".into(),
            is_default: true,
        };
        let candidate = ConfigEntry {
            name: "new".into(),
            path: "/tmp/new".into(),
            is_default: false,
        };
        let mut app = App {
            snapshot: Some(NodeSnapshot {
                config_name: "old".into(),
                peer_id: "old-peer".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        app.set_configs(vec![old, candidate.clone()]);
        app.open(Page::Configs);
        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Enter);

        assert_eq!(app.take_config_selection(), Some(candidate.clone()));
        assert_eq!(app.active_config().unwrap().name, "old");
        assert_eq!(app.snapshot.as_ref().unwrap().peer_id, "old-peer");

        app.begin_node_refresh();
        let activated = NodeSnapshot {
            config_name: "new".into(),
            peer_id: "new-peer".into(),
            connected: true,
            ..Default::default()
        };
        app.apply_core_state_projection(&CoreStateSnapshot {
            active_config: Some(candidate),
            node_snapshot: Some(activated),
            ..Default::default()
        });
        let generation = app.refresh_generation;
        reduce_service(
            &mut app,
            ServiceEvent::ConfigActivationFinished {
                generation,
                result: Ok(()),
                persistence_error: None,
            },
        );

        assert_eq!(app.active_config().unwrap().name, "new");
        assert_eq!(app.snapshot.as_ref().unwrap().peer_id, "new-peer");
        assert_eq!(app.config_path_input, "/tmp/new");
        assert!(app.take_reward_mint_load().is_some());
    }

    #[test]
    fn config_switch_keeps_tracked_prover_ids_unique_for_late_completions() {
        let old = ConfigEntry {
            name: "old".into(),
            path: "/tmp/old".into(),
            is_default: true,
        };
        let new = ConfigEntry {
            name: "new".into(),
            path: "/tmp/new".into(),
            is_default: false,
        };
        let action = ProverAction::Pause {
            filter: vec![0x44; 32],
        };
        let mut app = App::default();
        app.set_configs(vec![old]);
        let old_id = app
            .prover_operations
            .enqueue(action.clone(), None, 1)
            .unwrap();
        app.take_prover_operation().unwrap();

        app.apply_core_state_projection(&CoreStateSnapshot {
            config_generation: 1,
            active_config: Some(new),
            ..Default::default()
        });
        let new_id = app
            .prover_operations
            .enqueue(action.clone(), None, 2)
            .unwrap();

        assert_ne!(new_id, old_id);
        reduce_service(
            &mut app,
            ServiceEvent::ProverOperationFinished {
                id: old_id,
                result: Err("old config failure".into()),
            },
        );
        assert_eq!(
            app.prover_operations.items[0].phase,
            ProverOperationPhase::Submitting
        );
        assert!(app.prover_operations.conflicts_filter(&action.filters()[0]));
    }

    #[test]
    fn failed_config_activation_preserves_the_active_config_and_snapshot() {
        let old = ConfigEntry {
            name: "old".into(),
            path: "/tmp/old".into(),
            is_default: true,
        };
        let mut app = App {
            snapshot: Some(NodeSnapshot {
                config_name: "old".into(),
                peer_id: "old-peer".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        app.set_configs(vec![old]);
        app.begin_node_refresh();
        let generation = app.refresh_generation;

        reduce_service(
            &mut app,
            ServiceEvent::ConfigActivationFinished {
                generation,
                result: Err("candidate unavailable".into()),
                persistence_error: None,
            },
        );

        assert_eq!(app.active_config().unwrap().name, "old");
        assert_eq!(app.snapshot.as_ref().unwrap().peer_id, "old-peer");
        assert_eq!(
            app.config_path_error.as_deref(),
            Some("candidate unavailable")
        );
        assert!(app.take_reward_mint_load().is_none());
    }

    #[test]
    fn config_activation_persistence_error_still_commits_the_core_state() {
        let old = ConfigEntry {
            name: "old".into(),
            path: "/tmp/old".into(),
            is_default: true,
        };
        let candidate = ConfigEntry {
            name: "new".into(),
            path: "/tmp/new".into(),
            is_default: false,
        };
        let mut app = App::default();
        app.set_configs(vec![old]);
        app.begin_node_refresh();
        app.apply_core_state_projection(&CoreStateSnapshot {
            active_config: Some(candidate),
            node_snapshot: Some(NodeSnapshot {
                config_name: "new".into(),
                peer_id: "new-peer".into(),
                connected: true,
                ..Default::default()
            }),
            ..Default::default()
        });
        let generation = app.refresh_generation;

        reduce_service(
            &mut app,
            ServiceEvent::ConfigActivationFinished {
                generation,
                result: Ok(()),
                persistence_error: Some("cannot remember config".into()),
            },
        );

        assert_eq!(app.active_config().unwrap().name, "new");
        assert_eq!(app.snapshot.as_ref().unwrap().peer_id, "new-peer");
        assert_eq!(app.error.as_deref(), Some("cannot remember config"));
        assert!(app.take_reward_mint_load().is_some());
    }

    #[test]
    fn key_and_alias_management_wait_for_current_reload() {
        let mut keys = App {
            page: Page::Keys,
            route_stack: vec![Page::Home, Page::Keys],
            key_in_flight: true,
            keys: vec![KeySnapshot {
                id: "stale".into(),
                key_type: 0,
                public_key_hex: "aa".into(),
            }],
            ..Default::default()
        };
        reduce_input(&mut keys, InputEvent::Enter);
        assert_eq!(keys.page(), Page::Keys);

        let mut aliases = App {
            page: Page::Aliases,
            route_stack: vec![Page::Home, Page::Aliases],
            alias_in_flight: true,
            aliases: vec![AliasSnapshot {
                name: "stale".into(),
                address_hex: "aa".into(),
                kind: String::new(),
            }],
            ..Default::default()
        };
        reduce_input(&mut aliases, InputEvent::Enter);
        assert_eq!(aliases.page(), Page::Aliases);
    }

    #[test]
    fn empty_config_page_does_not_queue_a_selection() {
        let mut app = App::default();
        app.open(Page::Configs);

        reduce_input(&mut app, InputEvent::Enter);

        assert!(app.take_config_selection().is_none());
    }

    #[test]
    fn diagnostics_edit_mode_accepts_q_without_navigating_back() {
        let mut app = App::default();
        app.open(Page::Diagnostics);

        reduce_input(&mut app, InputEvent::Enter);
        reduce_input(&mut app, InputEvent::Character('q'));

        assert_eq!(app.page(), Page::Diagnostics);
        assert!(app.diagnostics_editing);
        assert_eq!(app.diagnostics_address, "q");
    }

    #[test]
    fn diagnostics_submit_queues_one_request() {
        let mut app = App::default();
        app.open(Page::Diagnostics);
        reduce_input(&mut app, InputEvent::Enter);
        for character in "/ip4/127.0.0.1/tcp/8336".chars() {
            reduce_input(&mut app, InputEvent::Character(character));
        }

        reduce_input(&mut app, InputEvent::Enter);

        let request = app.take_diagnostics_request().unwrap();
        assert_eq!(request.address, "/ip4/127.0.0.1/tcp/8336");
        assert!(app.take_diagnostics_request().is_none());
        assert!(app.diagnostics_in_flight);
    }

    #[test]
    fn peer_resolve_submit_queues_one_request_and_selects_results() {
        let mut app = App::default();
        app.open(Page::PeerResolve);
        reduce_input(&mut app, InputEvent::Enter);
        for character in "peer-1".chars() {
            reduce_input(&mut app, InputEvent::Character(character));
        }

        reduce_input(&mut app, InputEvent::Enter);
        let request = app.take_peer_resolve_request().unwrap();
        assert_eq!(request.peer_id, "peer-1");
        assert!(app.peer_resolve.in_flight);

        reduce_service(
            &mut app,
            ServiceEvent::PeerResolveFinished {
                generation: request.generation,
                result: Ok(crate::domain::PeerAddressReport {
                    peer_id: "peer-1".into(),
                    addresses: vec![
                        crate::domain::PeerAddressSnapshot {
                            kind: "stream".into(),
                            host_port: Some("127.0.0.1:8336".into()),
                            reach: crate::domain::PeerReach::Private,
                            multiaddr: "/ip4/127.0.0.1/tcp/8336".into(),
                            filter: "master".into(),
                        },
                        crate::domain::PeerAddressSnapshot {
                            kind: "pubsub".into(),
                            host_port: Some("node.example:8336".into()),
                            reach: crate::domain::PeerReach::Unknown,
                            multiaddr: "/dns4/node.example/udp/8336/quic-v1".into(),
                            filter: "aa".repeat(32),
                        },
                    ],
                }),
            },
        );
        reduce_input(&mut app, InputEvent::Down);

        assert_eq!(app.peer_resolve.index, 1);
        assert!(!app.peer_resolve.in_flight);
    }

    #[test]
    fn opening_rewards_queues_a_read_and_refreshes_it() {
        let mut app = App {
            menu_index: 3,
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);
        reduce_input(&mut app, InputEvent::Enter);

        assert_eq!(app.page(), Page::Rewards);
        assert_eq!(app.take_reward_request().unwrap().generation, 1);
        assert!(app.reward_in_flight);

        app.reward_in_flight = false;
        reduce_input(&mut app, InputEvent::Refresh);
        assert_eq!(app.take_reward_request().unwrap().generation, 2);
    }

    #[test]
    fn opening_legacy_assets_enters_migration_identity_without_queuing_a_read() {
        let mut app = App {
            menu_index: 3,
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);
        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Enter);

        assert_eq!(app.page(), Page::LegacyAssets);
        assert_eq!(app.legacy_migration.step, LegacyMigrationStep::Identity);
        assert!(app.take_legacy_request().is_none());
        assert!(!app.legacy_in_flight);
    }

    #[test]
    fn opening_confidential_wallet_queues_a_read() {
        let mut app = App {
            menu_index: 3,
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);
        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Enter);

        assert_eq!(app.page(), Page::ConfidentialWallet);
        assert_eq!(
            app.take_confidential_wallet_request(),
            Some(ConfidentialWalletRequest { generation: 1 })
        );
        assert!(app.confidential_wallet.in_flight);
    }

    #[test]
    fn opening_pending_transfers_queues_an_independent_read() {
        let mut app = App {
            menu_index: 3,
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);
        for _ in 0..4 {
            reduce_input(&mut app, InputEvent::Down);
        }
        reduce_input(&mut app, InputEvent::Enter);

        assert_eq!(app.page(), Page::PendingEscrows);
        assert_eq!(
            app.take_pending_escrows_request(),
            Some(PendingEscrowsRequest { generation: 1 })
        );
        assert!(app.pending_escrows.in_flight);
    }

    #[test]
    fn opening_health_report_queues_an_independent_read() {
        let mut app = App {
            menu_index: 6,
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);
        reduce_input(&mut app, InputEvent::Enter);

        assert_eq!(app.page(), Page::Health);
        assert_eq!(
            app.take_health_request(),
            Some(HealthRequest {
                generation: 1,
                target: crate::domain::HealthTarget::SelfNode,
            })
        );
        assert!(app.health.in_flight);
    }

    #[test]
    fn health_failed_refresh_closes_the_unavailable_detail_before_back() {
        let mut app = App {
            page: Page::Health,
            route_stack: vec![Page::Home, Page::Health],
            health: HealthState {
                detail: true,
                generation: 1,
                ..Default::default()
            },
            ..Default::default()
        };
        reduce_service(
            &mut app,
            ServiceEvent::HealthFinished {
                generation: 1,
                result: Err("unavailable".into()),
            },
        );
        reduce_input(&mut app, InputEvent::Back);
        assert_eq!(app.page, Page::Home);
    }

    #[test]
    fn peer_stream_address_opens_remote_health_without_typing_another_command() {
        let mut app = App {
            page: Page::PeerResolve,
            route_stack: vec![Page::Home, Page::PeerResolve],
            peer_resolve: PeerResolveState {
                peer_id: "QmTarget".into(),
                report: Some(PeerAddressReport {
                    peer_id: "QmTarget".into(),
                    addresses: vec![crate::domain::PeerAddressSnapshot {
                        kind: "stream".into(),
                        host_port: Some("node.example:8336".into()),
                        reach: crate::domain::PeerReach::Public,
                        multiaddr: "/dns/node.example/tcp/8336".into(),
                        filter: "master".into(),
                    }],
                }),
                ..Default::default()
            },
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Character('h'));

        assert_eq!(app.page(), Page::Health);
        assert_eq!(
            app.take_health_request(),
            Some(HealthRequest {
                generation: 1,
                target: crate::domain::HealthTarget::Peer {
                    peer_id: "QmTarget".into(),
                    address: "node.example:8336".into(),
                    identity_path: None,
                },
            })
        );
    }

    #[test]
    fn peer_health_rejects_a_pubsub_address_without_leaving_the_page() {
        let mut app = App {
            page: Page::PeerResolve,
            route_stack: vec![Page::Home, Page::PeerResolve],
            peer_resolve: PeerResolveState {
                peer_id: "QmTarget".into(),
                report: Some(PeerAddressReport {
                    peer_id: "QmTarget".into(),
                    addresses: vec![crate::domain::PeerAddressSnapshot {
                        kind: "pubsub".into(),
                        host_port: Some("node.example:8337".into()),
                        reach: crate::domain::PeerReach::Public,
                        multiaddr: "/dns/node.example/tcp/8337".into(),
                        filter: "master".into(),
                    }],
                }),
                ..Default::default()
            },
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Character('h'));

        assert_eq!(app.page(), Page::PeerResolve);
        assert!(app.take_health_request().is_none());
        assert_eq!(
            app.peer_resolve.error.as_deref(),
            Some("Health reports can only be read through a stream address")
        );
    }

    #[test]
    fn diagnostics_identity_can_be_loaded_and_is_used_for_remote_health() {
        let mut app = App {
            page: Page::DiagnosticsIdentity,
            route_stack: vec![Page::Home, Page::DiagnosticsIdentity],
            diagnostics_identity: DiagnosticsIdentityState {
                path: "/tmp/diagnostics.key".into(),
                action_index: 1,
                ..Default::default()
            },
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);
        let request = app.take_diagnostics_identity_request().unwrap();
        assert_eq!(request.path, "/tmp/diagnostics.key");
        assert!(!request.create);

        reduce_service(
            &mut app,
            ServiceEvent::DiagnosticsIdentityFinished {
                generation: request.generation,
                result: Ok(DiagnosticsIdentitySnapshot {
                    path: request.path.clone(),
                    peer_id: "QmRequester".into(),
                }),
            },
        );
        app.page = Page::PeerResolve;
        app.route_stack.push(Page::PeerResolve);
        app.peer_resolve.report = Some(PeerAddressReport {
            peer_id: "QmTarget".into(),
            addresses: vec![crate::domain::PeerAddressSnapshot {
                kind: "stream".into(),
                host_port: Some("node.example:8336".into()),
                reach: crate::domain::PeerReach::Public,
                multiaddr: "/dns4/node.example/tcp/8336".into(),
                filter: "master".into(),
            }],
        });

        reduce_input(&mut app, InputEvent::Character('h'));

        assert_eq!(
            app.take_health_request().unwrap().target,
            HealthTarget::Peer {
                peer_id: "QmTarget".into(),
                address: "node.example:8336".into(),
                identity_path: Some("/tmp/diagnostics.key".into()),
            }
        );
    }

    #[test]
    fn diagnostics_identity_path_editing_accepts_q_and_create_is_explicit() {
        let mut app = App {
            page: Page::DiagnosticsIdentity,
            diagnostics_identity: DiagnosticsIdentityState {
                path: String::new(),
                ..Default::default()
            },
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);
        for character in "/tmp/qclient.key".chars() {
            reduce_input(&mut app, InputEvent::Character(character));
        }
        reduce_input(&mut app, InputEvent::Enter);
        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Enter);

        let request = app.take_diagnostics_identity_request().unwrap();
        assert_eq!(request.path, "/tmp/qclient.key");
        assert!(request.create);
        assert_eq!(app.page(), Page::DiagnosticsIdentity);
    }

    #[test]
    fn failed_identity_change_keeps_the_previously_enabled_identity() {
        let active = DiagnosticsIdentitySnapshot {
            path: "/tmp/active.key".into(),
            peer_id: "QmActive".into(),
        };
        let mut app = App {
            page: Page::DiagnosticsIdentity,
            diagnostics_identity: DiagnosticsIdentityState {
                path: "/tmp/existing.key".into(),
                generation: 7,
                in_flight: true,
                snapshot: Some(active.clone()),
                ..Default::default()
            },
            ..Default::default()
        };

        reduce_service(
            &mut app,
            ServiceEvent::DiagnosticsIdentityFinished {
                generation: 7,
                result: Err("File already exists".into()),
            },
        );

        assert_eq!(app.diagnostics_identity.snapshot, Some(active));
        assert_eq!(
            app.diagnostics_identity.error.as_deref(),
            Some("File already exists")
        );
    }

    #[test]
    fn direct_remote_health_uses_explicit_target_and_independent_identity() {
        let mut app = App {
            page: Page::RemoteHealth,
            diagnostics_identity: DiagnosticsIdentityState {
                snapshot: Some(DiagnosticsIdentitySnapshot {
                    path: "/tmp/diagnostics.key".into(),
                    peer_id: "QmRequester".into(),
                }),
                ..Default::default()
            },
            remote_health: RemoteHealthState {
                peer_id: "QmTarget".into(),
                address: "node.example:8336".into(),
                field_index: 2,
                ..Default::default()
            },
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);

        assert_eq!(app.page(), Page::Health);
        assert_eq!(
            app.take_health_request().unwrap().target,
            HealthTarget::Peer {
                peer_id: "QmTarget".into(),
                address: "node.example:8336".into(),
                identity_path: Some("/tmp/diagnostics.key".into()),
            }
        );
    }

    #[test]
    fn direct_remote_health_requires_an_enabled_identity() {
        let mut app = App {
            page: Page::RemoteHealth,
            remote_health: RemoteHealthState {
                peer_id: "QmTarget".into(),
                address: "node.example:8336".into(),
                field_index: 2,
                ..Default::default()
            },
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);

        assert_eq!(app.page(), Page::RemoteHealth);
        assert!(app.take_health_request().is_none());
        assert!(app
            .remote_health
            .error
            .as_deref()
            .unwrap()
            .contains("Diagnostic Identity"));
    }

    #[test]
    fn repeated_read_refresh_does_not_start_parallel_requests() {
        let mut app = App {
            page: Page::LegacyAssets,
            route_stack: vec![Page::Home, Page::LegacyAssets],
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Refresh);
        assert!(app.take_legacy_request().is_some());
        reduce_input(&mut app, InputEvent::Refresh);

        assert!(app.take_legacy_request().is_none());
        assert_eq!(app.legacy_generation, 1);
    }

    #[test]
    fn stale_diagnostics_result_is_ignored() {
        let mut app = App {
            diagnostics_generation: 2,
            diagnostics_in_flight: true,
            ..Default::default()
        };

        reduce_service(
            &mut app,
            ServiceEvent::DiagnosticsFinished {
                generation: 1,
                result: Ok(crate::domain::DiagnosticsReport {
                    address: "old".into(),
                    peer_id: "peer".into(),
                    transport: "TCP".into(),
                    connection_ms: 12,
                    ping_ms: Some(4),
                    agent_version: None,
                    protocol_version: None,
                    observed_address: None,
                    listen_addresses: Vec::new(),
                    protocols: Vec::new(),
                }),
            },
        );

        assert!(app.diagnostics_in_flight);
        assert!(app.diagnostics_report.is_none());
    }

    #[test]
    fn stale_reward_result_is_ignored_after_config_switch() {
        let mut app = App {
            reward_generation: 3,
            reward_in_flight: true,
            ..Default::default()
        };

        reduce_service(
            &mut app,
            ServiceEvent::RewardsFinished {
                generation: 2,
                result: Ok(RewardSnapshot {
                    identity: crate::domain::RewardIdentity::Standard,
                    prover_address_hex: Some("aa".repeat(32)),
                    public_key_hex: Some("bb".repeat(897)),
                    legacy_account_address_hex: None,
                    recipient_address_hex: Some("cc".repeat(32)),
                    claimable_base_units: Some(8_000_000_000),
                    cited_frame: Some(42),
                    proof_verified: true,
                    witness_error: None,
                }),
            },
        );

        assert!(app.reward_in_flight);
        assert!(app.reward_snapshot.is_none());
    }

    #[test]
    fn stale_legacy_assets_result_is_ignored_after_config_switch() {
        let mut app = App {
            legacy_generation: 3,
            legacy_in_flight: true,
            ..Default::default()
        };

        reduce_service(
            &mut app,
            ServiceEvent::LegacyAssetsFinished {
                generation: 2,
                result: Ok(LegacyAssetsSnapshot {
                    identity_available: true,
                    ..Default::default()
                }),
            },
        );

        assert!(app.legacy_in_flight);
        assert!(app.legacy_snapshot.is_none());
    }

    #[test]
    fn opening_logs_queues_master_log_then_switches_to_worker() {
        let mut app = App {
            page: Page::Home,
            menu_index: 6,
            snapshot: Some(NodeSnapshot {
                workers: vec![crate::domain::WorkerSnapshot {
                    core_id: 7,
                    filter_hex: String::new(),
                    available_storage: 0,
                    total_storage: 0,
                    manually_managed: false,
                    execution: None,
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);
        for _ in 0..4 {
            reduce_input(&mut app, InputEvent::Down);
        }
        reduce_input(&mut app, InputEvent::Enter);
        let master = app.take_log_request().unwrap();
        assert_eq!(master.core_id, 0);

        reduce_input(&mut app, InputEvent::Right);
        let worker = app.take_log_request().unwrap();
        assert_eq!(worker.core_id, 7);
    }

    #[test]
    fn log_export_requires_preview_and_explicit_write_choice() {
        let mut app = App {
            menu_index: 6,
            ..Default::default()
        };
        reduce_input(&mut app, InputEvent::Enter);
        for _ in 0..5 {
            reduce_input(&mut app, InputEvent::Down);
        }
        reduce_input(&mut app, InputEvent::Enter);
        assert_eq!(app.page(), Page::LogExport);

        reduce_input(&mut app, InputEvent::Enter);
        let request = app.take_log_export_request().unwrap();
        assert_eq!(request.options.window_seconds, 2 * 60 * 60);
        assert_eq!(request.options.min_level, LogExportLevel::Info);

        reduce_service(
            &mut app,
            ServiceEvent::LogExportPrepared {
                generation: request.generation,
                result: Ok(crate::domain::LogExportPreview {
                    id: 9,
                    destination: "/tmp/export".into(),
                    token_map_path: "/tmp/export.map.txt".into(),
                    window_since: "from".into(),
                    window_until: "until".into(),
                    source: "2 log files".into(),
                    lines_read: 20,
                    lines_kept: 8,
                    redactions: Vec::new(),
                    samples: Vec::new(),
                    read_errors: 0,
                }),
            },
        );
        assert_eq!(app.log_export.choice, ConfirmChoice::Cancel);

        reduce_input(&mut app, InputEvent::Right);
        reduce_input(&mut app, InputEvent::Enter);
        assert_eq!(
            app.take_log_export_finish(),
            Some(LogExportFinishRequest {
                generation: request.generation,
                id: 9,
                commit: true,
            })
        );
    }

    #[test]
    fn stale_log_result_does_not_replace_current_log() {
        let mut app = App {
            log_generation: 3,
            log_in_flight: true,
            ..Default::default()
        };

        reduce_service(
            &mut app,
            ServiceEvent::LogFinished {
                generation: 2,
                result: Ok(crate::domain::LogSnapshot {
                    core_id: 0,
                    path: "/old/master.log".into(),
                    lines: vec!["old".into()],
                }),
            },
        );

        assert!(app.log_in_flight);
        assert!(app.log_snapshot.is_none());
    }

    #[test]
    fn opening_keys_queues_read_only_key_list() {
        let mut app = App {
            page: Page::Home,
            menu_index: 4,
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);
        reduce_input(&mut app, InputEvent::Enter);

        assert_eq!(app.page(), Page::Keys);
        assert!(app.take_key_request().is_some());
        assert!(app.key_in_flight);
    }

    fn scrollable_prover(width: u16) -> App {
        App {
            page: Page::Prover,
            viewport_width: width,
            viewport_height: 12,
            snapshot: Some(NodeSnapshot {
                allocations: vec![
                    crate::domain::AllocationSnapshot {
                        filter_hex: "abcdef".repeat(100),
                        ..Default::default()
                    };
                    2
                ],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn prover_page_keys_only_scroll_details_and_clamp_at_both_ends() {
        for width in [120, 80, 60, 42] {
            let mut app = scrollable_prover(width);
            if width < 70 {
                reduce_input(&mut app, InputEvent::Enter);
            }
            let before = format!("{app:?}");
            reduce_input(&mut app, InputEvent::PageDown);
            assert_eq!(app.prover_detail_scroll, 5, "width {width}");
            for _ in 0..200 {
                reduce_input(&mut app, InputEvent::PageDown);
            }
            let end = app.prover_detail_scroll;
            assert!(end > 5 && end < 200, "width {width}: {end}");
            reduce_input(&mut app, InputEvent::PageDown);
            assert_eq!(app.prover_detail_scroll, end);
            reduce_input(&mut app, InputEvent::PageUp);
            assert_eq!(app.prover_detail_scroll, end - 5);
            app.prover_detail_scroll = usize::MAX;
            reduce_input(&mut app, InputEvent::PageUp);
            assert_eq!(app.prover_detail_scroll, end - 5);
            for _ in 0..200 {
                reduce_input(&mut app, InputEvent::PageUp);
            }
            assert_eq!(app.prover_detail_scroll, 0);
            assert_eq!(
                format!("{app:?}"),
                before,
                "scrolling must not queue requests or change selection"
            );
        }
    }

    #[test]
    fn narrow_prover_arrows_scroll_only_after_opening_details() {
        let mut app = scrollable_prover(60);
        reduce_input(&mut app, InputEvent::PageDown);
        assert_eq!(app.prover_detail_scroll, 0);
        reduce_input(&mut app, InputEvent::Down);
        assert_eq!(app.allocation_index, 1);
        app.prover_detail_scroll = 4;
        reduce_input(&mut app, InputEvent::Enter);
        assert_eq!(app.prover_detail_scroll, 0);
        for _ in 0..200 {
            reduce_input(&mut app, InputEvent::Down);
        }
        let end = app.prover_detail_scroll;
        assert!(end > 0);
        assert_eq!(app.allocation_index, 1);
        assert_eq!(app.expanded_allocation, Some(1));
        reduce_input(&mut app, InputEvent::Up);
        assert_eq!(app.prover_detail_scroll, end - 1);
        reduce_input(&mut app, InputEvent::Back);
        assert_eq!(app.page, Page::Prover);
        assert_eq!(app.expanded_allocation, None);
        assert_eq!(app.prover_detail_scroll, 0);
        reduce_input(&mut app, InputEvent::Up);
        assert_eq!(app.allocation_index, 0);
    }

    #[test]
    fn wide_prover_arrows_select_allocations_and_reset_scroll() {
        for width in [80, 120] {
            let mut app = scrollable_prover(width);
            for event in [InputEvent::Down, InputEvent::Up] {
                app.prover_detail_scroll = 5;
                let previous = app.allocation_index;
                reduce_input(&mut app, event);
                assert_ne!(app.allocation_index, previous);
                assert_eq!(app.prover_detail_scroll, 0);
            }
        }
    }

    #[test]
    fn prover_scroll_resets_on_detail_toggle_refresh_resize_and_navigation() {
        let mut app = scrollable_prover(60);
        for event in [
            InputEvent::Enter,
            InputEvent::Enter,
            InputEvent::Refresh,
            InputEvent::Resize(80, 24),
        ] {
            app.prover_detail_scroll = 5;
            reduce_input(&mut app, event);
            assert_eq!(app.prover_detail_scroll, 0);
        }
        assert_eq!((app.viewport_width, app.viewport_height), (80, 24));
        assert!(app.loading);
        assert_eq!(app.refresh_generation, 1);
        app.prover_detail_scroll = 5;
        app.open(Page::Help);
        assert_eq!(app.prover_detail_scroll, 0);
        app.prover_detail_scroll = 5;
        reduce_input(&mut app, InputEvent::Back);
        assert_eq!(app.page, Page::Prover);
        assert_eq!(app.prover_detail_scroll, 0);
        app.prover_detail_scroll = 5;
        reduce_input(&mut app, InputEvent::Back);
        assert_eq!(app.prover_detail_scroll, 0);
        app.prover_detail_scroll = 5;
        app.open(Page::Prover);
        assert_eq!(app.prover_detail_scroll, 0);
    }

    #[test]
    fn prover_scroll_resets_on_config_switch_and_core_projection() {
        let mut app = scrollable_prover(80);
        app.prover_detail_scroll = 5;
        app.apply_core_state_projection(&CoreStateSnapshot {
            node_snapshot: Some(NodeSnapshot::default()),
            ..Default::default()
        });
        assert_eq!(app.prover_detail_scroll, 0);
        app.prover_detail_scroll = 5;
        app.apply_config_entry(ConfigEntry {
            name: "new".into(),
            path: "/tmp/new/.config".into(),
            is_default: false,
        });
        assert_eq!(app.prover_detail_scroll, 0);
    }

    #[test]
    fn prover_scroll_actions_keep_existing_action_and_refresh_semantics() {
        let mut app = app_with_status(crate::domain::AllocationStatus::Active);
        app.viewport_width = 60;
        app.expanded_allocation = Some(0);
        app.prover_detail_scroll = 5;
        reduce_input(&mut app, InputEvent::Character('a'));
        assert_eq!(app.page, Page::ProverActions);
        assert_eq!(app.expanded_allocation, Some(0));
        assert_eq!(app.prover_detail_scroll, 0);
        assert!(app.queued_operation.is_none());
        reduce_input(&mut app, InputEvent::Back);
        app.prover_detail_scroll = 5;
        reduce_input(&mut app, InputEvent::Refresh);
        assert!(app.loading);
        assert_eq!(app.prover_detail_scroll, 0);
        assert!(app.queued_operation.is_none());
    }

    #[test]
    fn page_keys_do_not_clear_errors_in_editing_forms() {
        for page in [Page::RemoteHealth, Page::AliasEdit, Page::KeyCreate] {
            let mut app = App {
                page,
                alias_form_editing: true,
                alias_form_error: Some("keep alias error".into()),
                key_form_editing: true,
                key_form_error: Some("keep key error".into()),
                ..Default::default()
            };
            app.remote_health.editing = true;
            app.remote_health.error = Some("keep health error".into());
            let before = format!("{app:?}");
            reduce_input(&mut app, InputEvent::PageDown);
            reduce_input(&mut app, InputEvent::PageUp);
            assert_eq!(format!("{app:?}"), before, "{page:?}");
        }
    }

    #[test]
    fn page_keys_are_no_ops_on_unsupported_pages() {
        for page in [
            Page::Home,
            Page::NodeStatus,
            Page::Configs,
            Page::ConfigPath,
            Page::Help,
            Page::ProverActions,
            Page::Confirm,
            Page::Logs,
            Page::Diagnostics,
            Page::Runtime,
            Page::Health,
            Page::Keys,
            Page::Aliases,
            Page::Rewards,
            Page::OperationResult,
            Page::Shards,
            Page::Hypergraph,
        ] {
            let mut app = App {
                page,
                ..Default::default()
            };
            let before = format!("{app:?}");
            reduce_input(&mut app, InputEvent::PageDown);
            reduce_input(&mut app, InputEvent::PageUp);
            assert_eq!(format!("{app:?}"), before, "{page:?}");
        }
    }

    #[test]
    fn down_on_prover_moves_allocation_selection() {
        let mut app = App {
            page: Page::Prover,
            snapshot: Some(NodeSnapshot {
                allocations: vec![
                    crate::domain::AllocationSnapshot {
                        filter_hex: "aa".repeat(32),
                        ..Default::default()
                    },
                    crate::domain::AllocationSnapshot {
                        filter_hex: "bb".repeat(32),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Down);

        assert_eq!(app.allocation_index, 1);
        assert_eq!(app.menu_index, 0);
    }

    #[test]
    fn enter_toggles_selected_allocation_details_in_place() {
        let mut app = App {
            page: Page::Prover,
            snapshot: Some(NodeSnapshot {
                allocations: vec![crate::domain::AllocationSnapshot::default()],
                ..Default::default()
            }),
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);
        assert_eq!(app.page(), Page::Prover);
        assert_eq!(app.expanded_allocation, Some(0));

        reduce_input(&mut app, InputEvent::Enter);
        assert_eq!(app.expanded_allocation, None);
    }

    #[test]
    fn narrow_prover_back_closes_detail_before_leaving_page() {
        let mut app = App {
            page: Page::Prover,
            viewport_width: 60,
            expanded_allocation: Some(0),
            snapshot: Some(NodeSnapshot {
                allocations: vec![crate::domain::AllocationSnapshot::default()],
                ..Default::default()
            }),
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Back);

        assert_eq!(app.page(), Page::Prover);
        assert_eq!(app.expanded_allocation, None);
    }

    #[test]
    fn resizing_prover_to_two_panes_clears_narrow_detail_mode() {
        let mut app = App {
            page: Page::Prover,
            viewport_width: 60,
            expanded_allocation: Some(0),
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Resize(80, 24));

        assert_eq!(app.viewport_width, 80);
        assert_eq!(app.expanded_allocation, None);
    }

    #[test]
    fn wide_prover_enter_does_not_arm_detail_for_a_later_narrow_resize() {
        let mut app = App {
            page: Page::Prover,
            viewport_width: 80,
            snapshot: Some(NodeSnapshot {
                allocations: vec![crate::domain::AllocationSnapshot::default()],
                ..Default::default()
            }),
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);
        assert_eq!(app.expanded_allocation, None);

        reduce_input(&mut app, InputEvent::Resize(60, 24));
        assert_eq!(app.expanded_allocation, None);
    }

    #[test]
    fn moving_selection_collapses_expanded_allocation() {
        let mut app = App {
            page: Page::Prover,
            viewport_width: 80,
            expanded_allocation: Some(0),
            snapshot: Some(NodeSnapshot {
                allocations: vec![crate::domain::AllocationSnapshot::default(); 2],
                ..Default::default()
            }),
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Down);

        assert_eq!(app.allocation_index, 1);
        assert_eq!(app.expanded_allocation, None);
    }

    #[test]
    fn a_opens_actions_without_changing_expansion() {
        let mut app = app_with_status(crate::domain::AllocationStatus::Active);
        app.expanded_allocation = Some(0);

        reduce_input(&mut app, InputEvent::Character('a'));

        assert_eq!(app.page(), Page::ProverActions);
        assert_eq!(app.expanded_allocation, Some(0));
    }

    #[test]
    fn arrows_on_help_do_not_change_hidden_menu_selection() {
        let mut app = App {
            page: Page::Help,
            menu_index: 1,
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Down);

        assert_eq!(app.menu_index, 1);
    }

    #[test]
    fn hypergraph_query_starts_once_after_address_editing() {
        let mut app = App {
            page: Page::Hypergraph,
            hypergraph: HypergraphState {
                address: "ab".repeat(64),
                ..Default::default()
            },
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);
        assert!(app.hypergraph.editing);
        reduce_input(&mut app, InputEvent::Enter);

        assert!(app.hypergraph.in_flight);
        let request = app.take_hypergraph_request().expect("query queued");
        assert_eq!(request.kind, HypergraphKind::Vertex);
        assert_eq!(request.address, "ab".repeat(64));

        reduce_input(&mut app, InputEvent::Enter);
        assert!(app.take_hypergraph_request().is_none());
    }

    #[test]
    fn hypergraph_form_focus_moves_before_results_and_entries_keep_arrow_navigation() {
        let mut app = App {
            page: Page::Hypergraph,
            ..Default::default()
        };

        assert_eq!(app.hypergraph.field_index, 0);
        reduce_input(&mut app, InputEvent::Down);
        assert_eq!(app.hypergraph.field_index, 1);
        reduce_input(&mut app, InputEvent::Up);
        assert_eq!(app.hypergraph.field_index, 0);

        app.hypergraph.snapshot = Some(HypergraphSnapshot {
            kind: HypergraphKind::Vertex,
            address_hex: String::new(),
            set_type: String::new(),
            phase_type: String::new(),
            shard_l1_hex: String::new(),
            shard_l2_hex: String::new(),
            present: Some(true),
            entries: vec![
                crate::domain::HypergraphEntrySnapshot {
                    key_hex: "01".into(),
                    value_hex: "02".into(),
                },
                crate::domain::HypergraphEntrySnapshot {
                    key_hex: "03".into(),
                    value_hex: "04".into(),
                },
            ],
        });
        reduce_input(&mut app, InputEvent::Down);
        assert_eq!(app.hypergraph.field_index, 0);
        assert_eq!(app.hypergraph.entry_index, 1);
    }

    #[test]
    fn empty_hypergraph_query_shows_validation_error() {
        let mut app = App {
            page: Page::Hypergraph,
            hypergraph: HypergraphState {
                editing: true,
                ..Default::default()
            },
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);

        assert!(!app.hypergraph.in_flight);
        assert_eq!(
            app.hypergraph.error.as_deref(),
            Some("Enter a 64-byte address or local alias")
        );
    }

    #[test]
    fn stale_hypergraph_result_is_ignored() {
        let mut app = App {
            hypergraph: HypergraphState {
                generation: 2,
                in_flight: true,
                ..Default::default()
            },
            ..Default::default()
        };

        reduce_service(
            &mut app,
            ServiceEvent::HypergraphFinished {
                generation: 1,
                result: Err("old".into()),
            },
        );

        assert!(app.hypergraph.in_flight);
        assert!(app.hypergraph.error.is_none());
    }

    fn archive_snapshot(endpoint_count: usize) -> crate::domain::ArchiveShardSnapshot {
        crate::domain::ArchiveShardSnapshot {
            summary: crate::domain::ArchiveConsensus::AllAgree,
            total: endpoint_count,
            answered: endpoint_count,
            archives: (0..endpoint_count)
                .map(|index| crate::domain::ArchiveEndpointSnapshot {
                    endpoint: format!("archive-{index}"),
                    shard_count: Some(index + 1),
                    frozen_count: Some(0),
                    digest: Some(format!("digest-{index}")),
                    status: "ok".into(),
                    extra_filters: Vec::new(),
                    missing_filters: Vec::new(),
                    error: None,
                })
                .collect(),
        }
    }

    #[test]
    fn archive_refresh_does_not_overlap() {
        let mut app = App {
            page: Page::ArchiveShards,
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Refresh);
        let first = app.take_archive_request().expect("archive read queued");
        reduce_input(&mut app, InputEvent::Refresh);

        assert_eq!(first.generation, 1);
        assert!(app.take_archive_request().is_none());
        assert_eq!(app.archive_shards.generation, 1);
        assert!(app.archive_shards.in_flight);
    }

    #[test]
    fn archive_stale_result_is_ignored_after_config_activation() {
        let mut app = App {
            page: Page::Configs,
            configs: vec![
                ConfigEntry {
                    name: "old".into(),
                    path: "/old".into(),
                    is_default: true,
                },
                ConfigEntry {
                    name: "new".into(),
                    path: "/new".into(),
                    is_default: false,
                },
            ],
            config_index: 1,
            active_config_index: 0,
            archive_shards: ArchiveShardPageState {
                generation: 4,
                in_flight: true,
                snapshot: Some(archive_snapshot(1)),
                error: Some("old error".into()),
                index: 3,
                detail: true,
                queued: Some(ArchiveShardRequest { generation: 4 }),
            },
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);
        assert_eq!(app.archive_shards.generation, 4);
        assert!(app.archive_shards.in_flight);
        let candidate = app.take_config_selection().unwrap();

        assert_eq!(app.archive_shards.generation, 4);
        assert!(app.archive_shards.in_flight);
        assert!(app.archive_shards.snapshot.is_some());

        app.begin_node_refresh();
        app.apply_core_state_projection(&CoreStateSnapshot {
            active_config: Some(candidate),
            node_snapshot: Some(NodeSnapshot::default()),
            ..Default::default()
        });
        let generation = app.refresh_generation;
        reduce_service(
            &mut app,
            ServiceEvent::ConfigActivationFinished {
                generation,
                result: Ok(()),
                persistence_error: None,
            },
        );

        assert_eq!(app.archive_shards.generation, 5);
        assert!(!app.archive_shards.in_flight);
        assert!(app.archive_shards.snapshot.is_none());
        assert!(app.archive_shards.error.is_none());
        assert_eq!(app.archive_shards.index, 0);
        assert!(app.take_archive_request().is_none());

        reduce_service(
            &mut app,
            ServiceEvent::ArchiveShardsFinished {
                generation: 4,
                result: Ok(archive_snapshot(2)),
            },
        );

        assert!(app.archive_shards.snapshot.is_none());
        assert!(!app.archive_shards.in_flight);
    }

    #[test]
    fn archive_selection_wraps_over_returned_endpoints() {
        let mut app = App {
            page: Page::ArchiveShards,
            archive_shards: ArchiveShardPageState {
                snapshot: Some(archive_snapshot(2)),
                ..Default::default()
            },
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Up);
        assert_eq!(app.archive_shards.index, 1);
        reduce_input(&mut app, InputEvent::Down);
        assert_eq!(app.archive_shards.index, 0);
        reduce_input(&mut app, InputEvent::Down);
        assert_eq!(app.archive_shards.index, 1);
    }

    #[test]
    fn narrow_archive_enter_and_back_toggle_detail_without_a_request() {
        let mut app = App {
            page: Page::ArchiveShards,
            viewport_width: 60,
            archive_shards: ArchiveShardPageState {
                snapshot: Some(archive_snapshot(2)),
                ..Default::default()
            },
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);
        assert!(app.archive_shards.detail);
        assert!(app.take_archive_request().is_none());

        reduce_input(&mut app, InputEvent::Back);
        assert_eq!(app.page(), Page::ArchiveShards);
        assert!(!app.archive_shards.detail);
    }

    #[test]
    fn moving_archive_selection_and_widening_clear_narrow_detail() {
        let mut app = App {
            page: Page::ArchiveShards,
            viewport_width: 60,
            archive_shards: ArchiveShardPageState {
                snapshot: Some(archive_snapshot(2)),
                detail: true,
                ..Default::default()
            },
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Down);
        assert_eq!(app.archive_shards.index, 1);
        assert!(!app.archive_shards.detail);

        app.archive_shards.detail = true;
        reduce_input(&mut app, InputEvent::Resize(80, 24));
        assert!(!app.archive_shards.detail);
    }

    #[test]
    fn wide_archive_enter_does_not_arm_detail_for_a_later_narrow_resize() {
        let mut app = App {
            page: Page::ArchiveShards,
            viewport_width: 80,
            archive_shards: ArchiveShardPageState {
                snapshot: Some(archive_snapshot(1)),
                ..Default::default()
            },
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);
        assert!(!app.archive_shards.detail);

        reduce_input(&mut app, InputEvent::Resize(60, 24));
        assert!(!app.archive_shards.detail);
    }

    #[test]
    fn archive_entering_menu_queues_request() {
        let mut app = App {
            menu_index: 1,
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);
        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Enter);

        assert_eq!(app.page(), Page::ArchiveShards);
        assert_eq!(
            app.take_archive_request(),
            Some(ArchiveShardRequest { generation: 1 })
        );
        assert!(app.archive_shards.in_flight);
    }

    #[test]
    fn archive_matching_result_clamps_selection_and_clears_error() {
        let mut app = App {
            archive_shards: ArchiveShardPageState {
                generation: 7,
                in_flight: true,
                error: Some("old".into()),
                index: 9,
                ..Default::default()
            },
            ..Default::default()
        };

        reduce_service(
            &mut app,
            ServiceEvent::ArchiveShardsFinished {
                generation: 7,
                result: Ok(archive_snapshot(2)),
            },
        );

        assert!(!app.archive_shards.in_flight);
        assert_eq!(app.archive_shards.index, 1);
        assert_eq!(app.archive_shards.snapshot, Some(archive_snapshot(2)));
        assert!(app.archive_shards.error.is_none());
    }

    #[test]
    fn archive_matching_error_clears_snapshot_and_selection() {
        let mut app = App {
            archive_shards: ArchiveShardPageState {
                generation: 8,
                in_flight: true,
                snapshot: Some(archive_snapshot(2)),
                index: 1,
                ..Default::default()
            },
            ..Default::default()
        };

        reduce_service(
            &mut app,
            ServiceEvent::ArchiveShardsFinished {
                generation: 8,
                result: Err("unavailable".into()),
            },
        );

        assert!(!app.archive_shards.in_flight);
        assert!(app.archive_shards.snapshot.is_none());
        assert_eq!(app.archive_shards.index, 0);
        assert_eq!(app.archive_shards.error.as_deref(), Some("unavailable"));
    }

    #[test]
    fn shard_scope_switch_waits_for_current_read_then_queries_all() {
        let mut app = App {
            page: Page::Shards,
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Refresh);
        assert!(app.take_node_refresh_request());
        let first = app.take_shard_request().expect("allocated query queued");
        assert!(!first.include_all);

        reduce_input(&mut app, InputEvent::Right);
        assert_eq!(app.shards.scope, ShardScope::Allocated);

        reduce_service(
            &mut app,
            ServiceEvent::ShardsFinished {
                generation: first.generation,
                result: Ok(ShardInfoSnapshot {
                    include_all: false,
                    frame_number: 1,
                    difficulty: 2,
                    world_state: "1.0 MB".into(),
                    pomw_basis: "0".into(),
                    allocation_scoring_strategy: String::new(),
                    directory_collected_at_unix_seconds: None,
                    directory_stale: false,
                    directory_error: None,
                    directory_invalid_rows: 0,
                    network_provers: None,
                    summary: Default::default(),
                    shards: Vec::new(),
                }),
            },
        );
        reduce_input(&mut app, InputEvent::Right);

        assert_eq!(app.shards.scope, ShardScope::All);
        assert!(app.take_shard_request().unwrap().include_all);
    }

    #[test]
    fn stale_refresh_does_not_change_current_status_or_snapshot() {
        let mut app = App {
            refresh_generation: 2,
            loading: true,
            error: Some("current".into()),
            snapshot: Some(NodeSnapshot {
                peer_id: "new".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        reduce_service(
            &mut app,
            ServiceEvent::RefreshFinished {
                generation: 1,
                result: Ok(()),
            },
        );
        assert!(app.loading);
        assert_eq!(app.error.as_deref(), Some("current"));
        assert_eq!(app.snapshot.as_ref().unwrap().peer_id, "new");
    }

    #[test]
    fn refresh_error_keeps_last_valid_snapshot() {
        let mut app = app_with_status(crate::domain::AllocationStatus::Active);
        app.refresh_generation = 3;
        app.snapshot.as_mut().unwrap().peer_id = "kept".into();
        reduce_service(
            &mut app,
            ServiceEvent::RefreshFinished {
                generation: 3,
                result: Err("timeout".into()),
            },
        );
        assert_eq!(app.snapshot.as_ref().unwrap().peer_id, "kept");
        assert_eq!(app.error.as_deref(), Some("timeout"));
        assert!(
            available_actions(&app).is_empty(),
            "a retained stale snapshot must never re-enable writes"
        );
    }

    #[test]
    fn successful_refresh_clears_staleness_and_restores_snapshot_actions() {
        let mut app = app_with_status(crate::domain::AllocationStatus::Active);
        app.error = Some("old refresh failed".into());
        app.loading = true;
        app.refresh_generation = 4;
        let fresh = app_with_status(crate::domain::AllocationStatus::Active)
            .snapshot
            .unwrap();

        app.apply_core_state_projection(&CoreStateSnapshot {
            node_snapshot: Some(fresh),
            ..Default::default()
        });
        reduce_service(
            &mut app,
            ServiceEvent::RefreshFinished {
                generation: 4,
                result: Ok(()),
            },
        );

        assert!(app.error.is_none());
        assert!(!app.loading);
        assert_eq!(available_actions(&app).len(), 2);
    }

    #[test]
    fn operation_failure_does_not_make_a_fresh_snapshot_stale() {
        let mut app = app_with_status(crate::domain::AllocationStatus::Active);
        app.operation_message = Some("pause rpc failed".into());
        app.operation_succeeded = Some(false);

        assert!(app.error.is_none());
        assert_eq!(available_actions(&app).len(), 2);
    }

    #[test]
    fn unknown_operation_result_reaches_the_indeterminate_result_state() {
        let mut app = App {
            operation_generation: 7,
            operation_in_flight: true,
            ..Default::default()
        };

        reduce_service(
            &mut app,
            ServiceEvent::OperationFinished {
                generation: 7,
                result: Ok(crate::domain::OperationResult {
                    message: "Result unknown; refresh the status".into(),
                    outcome: crate::domain::OperationOutcome::Unknown,
                }),
            },
        );

        assert_eq!(app.page(), Page::OperationResult);
        assert_eq!(app.operation_succeeded, None);
        assert_eq!(
            app.operation_message.as_deref(),
            Some("Result unknown; refresh the status")
        );
    }

    #[test]
    fn root_menu_includes_other_before_help() {
        assert_eq!(
            ROOT_MENU
                .map(|item| crate::i18n::zh_tw(item.label))
                .as_slice(),
            [
                "系統概覽",
                "節點管理",
                "Prover 與分片",
                "錢包與 Token",
                "金鑰與地址",
                "資料與應用",
                "診斷與日誌",
                "QClient 指令",
                "其他",
                "說明"
            ]
        );
        assert_eq!(
            ROOT_MENU.map(|item| item.route),
            [
                RootRoute::Overview,
                RootRoute::Section(Section::Node),
                RootRoute::Section(Section::Prover),
                RootRoute::Section(Section::Wallet),
                RootRoute::Section(Section::Identity),
                RootRoute::Section(Section::Data),
                RootRoute::Section(Section::Diagnostics),
                RootRoute::Capabilities,
                RootRoute::Other,
                RootRoute::Help,
            ]
        );
    }

    #[test]
    fn sections_only_contain_implemented_pages() {
        let expected = [
            (
                Section::Node,
                &[
                    Page::NodeStatus,
                    Page::Configs,
                    Page::Runtime,
                    Page::ArchiveShards,
                ][..],
            ),
            (Section::Prover, &[Page::Prover, Page::Shards][..]),
            (
                Section::Wallet,
                &[
                    Page::Rewards,
                    Page::TokenTransfer,
                    Page::LegacyAssets,
                    Page::ConfidentialWallet,
                    Page::PendingEscrows,
                ][..],
            ),
            (Section::Identity, &[Page::Keys, Page::Aliases][..]),
            (Section::Data, &[Page::Hypergraph, Page::DeployToken][..]),
            (
                Section::Diagnostics,
                &[
                    Page::Health,
                    Page::Diagnostics,
                    Page::DiagnosticsIdentity,
                    Page::PeerResolve,
                    Page::Logs,
                    Page::LogExport,
                    Page::RemoteHealth,
                ][..],
            ),
        ];

        for (section, pages) in expected {
            assert_eq!(
                section
                    .items()
                    .iter()
                    .map(|item| item.page)
                    .collect::<Vec<_>>(),
                pages,
                "{section:?}"
            );
        }
    }

    #[test]
    fn overview_enter_stays_home_without_growing_route_stack() {
        let mut app = App::default();

        reduce_input(&mut app, InputEvent::Enter);

        assert_eq!(app.page(), Page::Home);
        assert!(app.route_stack.is_empty());
    }

    #[test]
    fn node_status_is_reachable_from_node_section_and_backtracks_one_level() {
        let mut app = App {
            menu_index: 1,
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);
        assert_eq!(app.page(), Page::Section(Section::Node));

        reduce_input(&mut app, InputEvent::Enter);
        assert_eq!(app.page(), Page::NodeStatus);

        reduce_input(&mut app, InputEvent::Back);
        assert_eq!(app.page(), Page::Section(Section::Node));
        reduce_input(&mut app, InputEvent::Back);
        assert_eq!(app.page(), Page::Home);
    }

    #[test]
    fn section_selection_is_independent_wraps_and_opens_selected_page() {
        let mut app = App {
            menu_index: 1,
            allocation_index: 7,
            key_index: 9,
            section_index: 8,
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);
        assert_eq!(app.page(), Page::Section(Section::Node));
        assert_eq!(app.section_index, 0);
        assert_eq!(app.allocation_index, 7);
        assert_eq!(app.key_index, 9);

        reduce_input(&mut app, InputEvent::Up);
        assert_eq!(app.section_index, 3);
        reduce_input(&mut app, InputEvent::Enter);
        assert_eq!(app.page(), Page::ArchiveShards);
        assert_eq!(
            app.take_archive_request(),
            Some(ArchiveShardRequest { generation: 1 })
        );
    }

    #[test]
    fn back_returns_from_page_to_section_then_home_and_ctrl_c_exits() {
        let mut app = App {
            menu_index: 4,
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);
        reduce_input(&mut app, InputEvent::Enter);
        assert_eq!(app.page(), Page::Keys);
        assert!(app.take_key_request().is_some());

        reduce_input(&mut app, InputEvent::Back);
        assert_eq!(app.page(), Page::Section(Section::Identity));
        reduce_input(&mut app, InputEvent::Back);
        assert_eq!(app.page(), Page::Home);
        assert!(!app.should_quit);

        reduce_input(&mut app, InputEvent::Quit);
        assert!(app.should_quit);
    }

    #[test]
    fn every_existing_top_level_page_is_reachable_through_sections() {
        let pages = ROOT_MENU
            .iter()
            .filter_map(|destination| match destination.route {
                RootRoute::Section(section) => Some(section),
                RootRoute::Overview
                | RootRoute::Capabilities
                | RootRoute::Other
                | RootRoute::Help => None,
            })
            .flat_map(Section::items)
            .map(|item| item.page)
            .collect::<Vec<_>>();

        assert_eq!(
            pages,
            [
                Page::NodeStatus,
                Page::Configs,
                Page::Runtime,
                Page::ArchiveShards,
                Page::Prover,
                Page::Shards,
                Page::Rewards,
                Page::TokenTransfer,
                Page::LegacyAssets,
                Page::ConfidentialWallet,
                Page::PendingEscrows,
                Page::Keys,
                Page::Aliases,
                Page::Hypergraph,
                Page::DeployToken,
                Page::Health,
                Page::Diagnostics,
                Page::DiagnosticsIdentity,
                Page::PeerResolve,
                Page::Logs,
                Page::LogExport,
                Page::RemoteHealth,
            ]
        );
    }

    #[test]
    fn section_labels_match_the_i18n_catalog() {
        use crate::i18n::{zh_tw, MessageKey};

        let expected = [
            (
                Section::Node,
                [
                    MessageKey::MenuOverview,
                    MessageKey::MenuConfigs,
                    MessageKey::MenuRuntime,
                    MessageKey::MenuArchiveShards,
                ]
                .as_slice(),
            ),
            (
                Section::Prover,
                [MessageKey::MenuProver, MessageKey::MenuShards].as_slice(),
            ),
            (
                Section::Wallet,
                [
                    MessageKey::MenuRewards,
                    MessageKey::MenuTransfer,
                    MessageKey::MenuLegacyAssets,
                    MessageKey::MenuConfidentialWallet,
                    MessageKey::MenuPendingEscrows,
                ]
                .as_slice(),
            ),
            (
                Section::Identity,
                [MessageKey::MenuKeys, MessageKey::MenuAliases].as_slice(),
            ),
            (
                Section::Data,
                [MessageKey::MenuHypergraph, MessageKey::MenuDeployToken].as_slice(),
            ),
            (
                Section::Diagnostics,
                [
                    MessageKey::MenuHealth,
                    MessageKey::MenuDiagnostics,
                    MessageKey::MenuDiagnosticsIdentity,
                    MessageKey::MenuPeerResolve,
                    MessageKey::MenuLogs,
                    MessageKey::MenuLogExport,
                    MessageKey::MenuRemoteHealth,
                ]
                .as_slice(),
            ),
        ];

        for (section, keys) in expected {
            let labels = section
                .items()
                .iter()
                .map(|item| item.label)
                .collect::<Vec<_>>();
            assert_eq!(labels, keys);
            assert!(labels.into_iter().all(|key| !zh_tw(key).is_empty()));
        }
    }

    #[test]
    fn invalid_root_index_does_not_panic_or_navigate() {
        let mut app = App {
            menu_index: usize::MAX,
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);

        assert_eq!(app.page(), Page::Home);
        assert!(app.route_stack.is_empty());
    }

    #[test]
    fn invalid_section_index_does_not_panic_or_navigate() {
        let mut app = App {
            page: Page::Section(Section::Wallet),
            route_stack: vec![Page::Home, Page::Section(Section::Wallet)],
            section_index: usize::MAX,
            ..Default::default()
        };

        reduce_input(&mut app, InputEvent::Enter);

        assert_eq!(app.page(), Page::Section(Section::Wallet));
        assert!(app.take_reward_request().is_none());
        assert!(app.take_legacy_request().is_none());
    }
}

#[cfg(test)]
mod web_settings_tests {
    use super::*;
    use crate::settings::{ConsoleSettings, WebSettings};

    fn other_app() -> App {
        let mut app = App::default();
        app.open(Page::Other);
        app
    }

    #[test]
    fn console_settings_defaults_and_loaded_settings_are_committed() {
        let mut app = other_app();
        assert_eq!(app.other.settings, WebSettings::default());
        assert_eq!(app.locale(), Locale::English);
        assert_eq!(app.other.port_input, "8888");
        let loaded = ConsoleSettings {
            web: WebSettings {
                enabled: true,
                port: 9000,
            },
            language: Locale::TraditionalChinese,
        };
        app.set_console_settings(loaded);
        assert_eq!(app.other.settings.port, loaded.web.port);
        assert!(app.other.settings.enabled);
        assert_eq!(app.locale(), loaded.language);
        assert_eq!(app.other.port_input, "9000");
    }

    #[test]
    fn web_toggle_queues_disabled_then_enabled_settings() {
        let mut app = other_app();
        reduce_input(&mut app, InputEvent::Enter);
        assert!(app.other.settings.enabled);
        let disabled = app.take_console_settings_request().unwrap();
        assert!(!disabled.settings.web.enabled);
        app.apply_console_settings_result(disabled.generation, Ok(disabled.settings));
        assert!(!app.other.settings.enabled);

        reduce_input(&mut app, InputEvent::Enter);
        let enabled = app.take_console_settings_request().unwrap();
        assert!(enabled.settings.web.enabled);
        app.apply_console_settings_result(enabled.generation, Ok(enabled.settings));
        assert!(app.other.settings.enabled);
    }

    #[test]
    fn language_switch_is_global_before_settings_are_persisted() {
        let mut app = other_app();

        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Enter);
        let request = app.take_console_settings_request().unwrap();
        assert!(request.settings.web.enabled);
        assert_eq!(request.settings.language, Locale::TraditionalChinese);
        assert_eq!(app.locale(), Locale::TraditionalChinese);

        app.apply_console_settings_result(request.generation, Ok(request.settings));
        assert_eq!(app.locale(), Locale::TraditionalChinese);
        assert!(app.other.settings.enabled);
    }

    #[test]
    fn failed_language_save_restores_the_previous_global_locale() {
        let mut app = other_app();
        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Enter);
        let request = app.take_console_settings_request().unwrap();

        assert_eq!(app.locale(), Locale::TraditionalChinese);
        app.apply_console_settings_result(request.generation, Err("cannot save".into()));

        assert_eq!(app.locale(), Locale::English);
        assert_eq!(app.other.error.as_deref(), Some("cannot save"));
    }

    #[test]
    fn language_cycles_through_all_four_locales() {
        let mut app = other_app();
        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Down);
        for expected in [
            Locale::TraditionalChinese,
            Locale::Japanese,
            Locale::French,
            Locale::English,
        ] {
            reduce_input(&mut app, InputEvent::Enter);
            let request = app.take_console_settings_request().unwrap();
            assert_eq!(request.settings.language, expected);
            assert_eq!(app.locale(), expected);
            app.apply_console_settings_result(request.generation, Ok(request.settings));
        }
    }

    #[test]
    fn language_row_supports_forward_and_reverse_navigation() {
        let mut app = other_app();
        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Down);

        reduce_input(&mut app, InputEvent::Left);
        let previous = app.take_console_settings_request().unwrap();
        assert_eq!(previous.settings.language, Locale::French);
        app.apply_console_settings_result(previous.generation, Ok(previous.settings));

        reduce_input(&mut app, InputEvent::Right);
        let next = app.take_console_settings_request().unwrap();
        assert_eq!(next.settings.language, Locale::English);
        app.apply_console_settings_result(next.generation, Ok(next.settings));
    }

    #[test]
    fn web_settings_port_edit_accepts_ascii_digits_and_cancel_restores_committed_port() {
        let mut app = other_app();
        reduce_input(&mut app, InputEvent::Down);
        assert_eq!(app.other.selected, 1);
        reduce_input(&mut app, InputEvent::Enter);
        assert!(app.is_text_editing());
        for character in ['q', '?', '３', ' ', '-'] {
            reduce_input(&mut app, InputEvent::Character(character));
        }
        assert_eq!(app.other.port_input, "8888");
        reduce_input(&mut app, InputEvent::Backspace);
        reduce_input(&mut app, InputEvent::Character('1'));
        assert_eq!(app.other.port_input, "8881");
        reduce_input(&mut app, InputEvent::Down);
        assert_eq!(app.other.selected, 1);
        reduce_input(&mut app, InputEvent::Back);
        assert_eq!(app.page, Page::Other);
        assert!(!app.is_text_editing());
        assert_eq!(app.other.port_input, "8888");
        reduce_input(&mut app, InputEvent::Back);
        assert_eq!(app.page, Page::Home);
    }

    #[test]
    fn web_port_validation_and_save_preserve_web_state() {
        let mut app = other_app();
        reduce_input(&mut app, InputEvent::Down);
        reduce_input(&mut app, InputEvent::Enter);
        for invalid in ["", "0", "65536"] {
            app.other.port_input = invalid.into();
            reduce_input(&mut app, InputEvent::Enter);
            assert!(app.other.error.is_some());
            assert!(app.other.editing_port);
            assert!(!app.other.in_flight);
            assert!(app.take_console_settings_request().is_none());
        }
        app.other.port_input = "65535".into();
        reduce_input(&mut app, InputEvent::Enter);
        let request = app.take_console_settings_request().unwrap();
        assert_eq!(
            request.settings.web,
            WebSettings {
                enabled: true,
                port: 65535,
            },
        );
        assert!(!app.is_text_editing());
        assert_eq!(app.other.settings.port, 8888);
        app.apply_console_settings_result(request.generation, Err("read only".into()));
        assert_eq!(app.other.settings.port, 8888);
        assert_eq!(app.other.port_input, "8888");
        assert_eq!(app.other.error.as_deref(), Some("read only"));
    }

    #[test]
    fn read_rpc_editor_saves_an_override_and_empty_input_restores_the_node_default() {
        let mut app = other_app();
        let config_dir = std::path::PathBuf::from("/nodes/mainnet/.config");
        app.queue_read_rpc_load(config_dir.clone());
        let load = app.take_read_rpc_request().unwrap();
        app.apply_read_rpc_result(
            load.generation,
            Ok(ReadRpcSelection {
                override_address: None,
                effective_address: "/ip4/127.0.0.1/tcp/8337".into(),
            }),
        );
        assert_eq!(app.other.effective_read_rpc, "/ip4/127.0.0.1/tcp/8337");

        app.other.selected = 2;
        reduce_input(&mut app, InputEvent::Enter);
        assert!(app.other.editing_read_rpc);
        app.other.read_rpc_input = "/ip4/192.0.2.89/tcp/8337".into();
        reduce_input(&mut app, InputEvent::Enter);
        let save = app.take_read_rpc_request().unwrap();
        assert_eq!(save.config_dir, config_dir);
        assert_eq!(save.value, Some(Some("/ip4/192.0.2.89/tcp/8337".into())));

        app.apply_read_rpc_result(
            save.generation,
            Ok(ReadRpcSelection {
                override_address: Some("/ip4/192.0.2.89/tcp/8337".into()),
                effective_address: "/ip4/192.0.2.89/tcp/8337".into(),
            }),
        );
        app.other.selected = 2;
        reduce_input(&mut app, InputEvent::Enter);
        app.other.read_rpc_input.clear();
        reduce_input(&mut app, InputEvent::Enter);
        assert_eq!(app.take_read_rpc_request().unwrap().value, Some(None));
    }

    #[test]
    fn localized_validation_errors_cover_all_supported_languages() {
        for (locale, expected) in [
            (
                Locale::English,
                [
                    "Enter a configuration directory",
                    "Address is not valid hexadecimal",
                    "Enter a 64-byte address or local alias",
                    "Enter a Peer ID",
                    "Load or create an identity on the Diagnostic Identity page first",
                    "Web port must be an integer from 1 to 65535",
                ],
            ),
            (
                Locale::TraditionalChinese,
                [
                    "請輸入設定目錄",
                    "地址不是有效的十六進位值",
                    "請輸入 64 位元組地址或本機別名",
                    "請輸入 Peer ID",
                    "請先在「診斷身分」頁面讀取或建立身分",
                    "Web 連接埠必須是 1 到 65535 之間的整數",
                ],
            ),
            (
                Locale::Japanese,
                [
                    "設定ディレクトリを入力してください",
                    "アドレスが有効な16進数ではありません",
                    "64 バイトのアドレスまたはローカルの別名を入力してください",
                    "Peer ID を入力してください",
                    "診断アイデンティティページでアイデンティティを読み込むか作成する必要があります",
                    "Webポートは1から65535の整数でなければなりません",
                ],
            ),
            (
                Locale::French,
                [
                    "Veuillez entrer un répertoire de configuration",
                    "L'adresse n'est pas une valeur hexadécimale valide",
                    "Veuillez entrer une adresse de 64 octets ou un alias local",
                    "Veuillez entrer un Peer ID",
                    "Vous devez d'abord charger ou créer un identifiant sur la page d'identifiant de diagnostic.",
                    "Le port Web doit être un entier entre 1 et 65535",
                ],
            ),
        ] {
            let localized = |mut app: App| {
                app.locale = locale;
                app
            };

            let mut config = localized(App::default());
            config.require_config_path("".into(), None);
            reduce_input(&mut config, InputEvent::Enter);
            assert_eq!(config.config_path_error.as_deref(), Some(expected[0]));

            let mut alias = localized(App {
                page: Page::AliasEdit,
                alias_form_name: "wallet".into(),
                alias_form_address: "not-hex".into(),
                alias_form_field: 3,
                ..Default::default()
            });
            reduce_input(&mut alias, InputEvent::Enter);
            assert_eq!(alias.alias_form_error.as_deref(), Some(expected[1]));

            let mut hypergraph = localized(App {
                page: Page::Hypergraph,
                hypergraph: HypergraphState {
                    editing: true,
                    ..Default::default()
                },
                ..Default::default()
            });
            reduce_input(&mut hypergraph, InputEvent::Enter);
            assert_eq!(hypergraph.hypergraph.error.as_deref(), Some(expected[2]));

            let mut peer = localized(App {
                page: Page::PeerResolve,
                peer_resolve: PeerResolveState {
                    editing: true,
                    ..Default::default()
                },
                ..Default::default()
            });
            reduce_input(&mut peer, InputEvent::Enter);
            assert_eq!(peer.peer_resolve.error.as_deref(), Some(expected[3]));

            let mut remote = localized(App {
                page: Page::RemoteHealth,
                remote_health: RemoteHealthState {
                    peer_id: "QmTarget".into(),
                    address: "node.example:8336".into(),
                    field_index: 2,
                    ..Default::default()
                },
                ..Default::default()
            });
            reduce_input(&mut remote, InputEvent::Enter);
            assert_eq!(remote.remote_health.error.as_deref(), Some(expected[4]));

            let mut other = localized(other_app());
            reduce_input(&mut other, InputEvent::Down);
            reduce_input(&mut other, InputEvent::Enter);
            other.other.port_input = "0".into();
            reduce_input(&mut other, InputEvent::Enter);
            assert_eq!(other.other.error.as_deref(), Some(expected[5]));
        }
    }

    #[test]
    fn tab_moves_from_directory_to_free_cores() {
        let mut app = shard_join_fixture();

        reduce_input(&mut app, InputEvent::Character('\t'));

        assert_eq!(app.shards.focus, ShardFocus::FreeCores);
    }

    #[test]
    fn enter_builds_join_for_selected_filter_and_core() {
        let mut app = shard_join_fixture();
        app.shards.focus = ShardFocus::FreeCores;

        reduce_input(&mut app, InputEvent::Enter);

        assert!(matches!(
            app.pending_action,
            Some(ProverAction::Join { ref worker_ids, .. }) if worker_ids == &vec![3]
        ));
        assert_eq!(app.page, Page::Confirm);
    }

    #[test]
    fn confirmed_join_returns_to_shards_and_locks_only_its_resources() {
        let mut app = shard_join_fixture();
        app.shards.focus = ShardFocus::FreeCores;
        reduce_input(&mut app, InputEvent::Enter);
        app.confirm_choice = ConfirmChoice::Confirm;

        reduce_input(&mut app, InputEvent::Enter);

        let filter = hex::decode(format!("{}0530", "11".repeat(32))).unwrap();
        assert_eq!(app.page, Page::Shards);
        assert!(!app.operation_in_flight);
        assert!(app.prover_operations.conflicts_filter(&filter));
        assert!(app.prover_operations.conflicts_core(3));
        assert!(!app.prover_operations.conflicts_core(6));
    }

    #[test]
    fn leave_and_join_can_be_in_flight_together() {
        let mut app = App::default();
        let leave = ProverAction::Leave {
            filters: vec![vec![0x11; 32]],
        };
        let join = ProverAction::Join {
            filters: vec![vec![0x22; 32]],
            delegate: Vec::new(),
            worker_ids: vec![3],
        };

        app.prover_operations.enqueue(leave, Some(7), 1).unwrap();
        app.prover_operations.enqueue(join, Some(3), 1).unwrap();

        assert_eq!(app.prover_operations.items.len(), 2);
    }

    fn shard_join_fixture() -> App {
        use crate::domain::{
            NodeSnapshot, ShardInfoSnapshot, ShardSnapshot, ShardSummarySnapshot, WorkerSnapshot,
        };

        let filter = format!("{}0530", "11".repeat(32));
        App {
            page: Page::Shards,
            route_stack: vec![Page::Home, Page::Shards],
            viewport_width: 120,
            snapshot: Some(NodeSnapshot {
                workers: vec![WorkerSnapshot {
                    core_id: 3,
                    filter_hex: String::new(),
                    available_storage: 1,
                    total_storage: 1,
                    manually_managed: true,
                    execution: None,
                }],
                ..Default::default()
            }),
            shards: ShardPageState {
                scope: ShardScope::All,
                snapshot: Some(ShardInfoSnapshot {
                    include_all: true,
                    frame_number: 900,
                    difficulty: 1,
                    world_state: "1.0 GB".into(),
                    pomw_basis: "1".into(),
                    allocation_scoring_strategy: "reward".into(),
                    directory_collected_at_unix_seconds: Some(1),
                    directory_stale: false,
                    directory_error: None,
                    directory_invalid_rows: 0,
                    network_provers: Some(1),
                    summary: ShardSummarySnapshot::default(),
                    shards: vec![ShardSnapshot {
                        filter_hex: filter,
                        active_provers: 1,
                        ring: Some(0),
                        ring_estimated: true,
                        official: true,
                        shard_size: "1.0 MB".into(),
                        data_shards: 1,
                        is_allocated: false,
                        materialized_frame: 0,
                        latest_frame: 900,
                        allocation_reason: None,
                        theoretical_daily_quil: Some("1.0".into()),
                        hour_daily_quil: None,
                        day_daily_quil: None,
                        prover_counts: None,
                    }],
                }),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn legacy_migration_test_key(seed: u8) -> String {
        let seed = [seed; 57];
        let public = quil_p2p::ed448_identity::derive_public_key(&seed);
        format!("{}{}", hex::encode(seed), hex::encode(public))
    }

    #[test]
    fn legacy_migration_identity_is_redacted_and_routes_to_destination() {
        let key = legacy_migration_test_key(7);
        let mut app = App {
            page: Page::LegacyAssets,
            ..Default::default()
        };
        reduce_input(&mut app, InputEvent::Character('m'));
        for character in key.chars() {
            reduce_input(&mut app, InputEvent::Character(character));
        }
        assert!(!format!("{app:?}").contains(&key));
        assert!(app.clone().legacy_migration.secret.expose().is_empty());

        reduce_input(&mut app, InputEvent::Enter);
        let generation = app.legacy_migration.generation;
        let request = app.take_legacy_migration_load().unwrap();
        assert_eq!(request.fingerprint.len(), 32);
        reduce_service(
            &mut app,
            crate::event::ServiceEvent::LegacyMigrationLoaded {
                generation,
                result: Ok(None),
            },
        );
        assert_eq!(app.legacy_migration.step, LegacyMigrationStep::Destination);
        assert_eq!(
            app.legacy_migration.destination,
            LegacyDestinationChoice::CurrentNode
        );

        reduce_input(&mut app, InputEvent::Right);
        assert_eq!(
            app.legacy_migration.destination,
            LegacyDestinationChoice::ExistingConfig
        );
        reduce_input(&mut app, InputEvent::Right);
        assert_eq!(
            app.legacy_migration.destination,
            LegacyDestinationChoice::CreateNew
        );
    }

    #[test]
    fn closing_legacy_file_viewer_zeroizes_visible_text() {
        let mut state = LegacyMigrationTuiState::default();
        state.viewer.replace("encryptionKey: secret".into());
        state.viewer_kind = Some(LegacyMigrationFileKind::Config);
        state.close_viewer();
        assert!(state.viewer.expose().is_empty());
        assert!(state.viewer_kind.is_none());
        assert!(!format!("{state:?}").contains("encryptionKey"));
    }

    #[test]
    fn legacy_migration_quit_invalidates_generation_and_clears_sensitive_state() {
        let mut app = App {
            page: Page::LegacyAssets,
            ..Default::default()
        };
        app.legacy_migration.step = LegacyMigrationStep::Files;
        app.legacy_migration.generation = 41;
        app.legacy_migration.secret.push('a');
        app.legacy_migration.existing_path = "/secret/recipient".into();
        app.legacy_migration
            .viewer
            .replace("encryptionKey: secret".into());
        app.legacy_migration.viewer_kind = Some(LegacyMigrationFileKind::Keys);

        reduce_input(&mut app, InputEvent::Quit);

        assert!(app.should_quit);
        assert_eq!(app.legacy_migration.generation, 42);
        assert_eq!(app.legacy_migration.step, LegacyMigrationStep::Inactive);
        assert!(app.legacy_migration.secret.expose().is_empty());
        assert!(app.legacy_migration.existing_path.is_empty());
        assert!(app.legacy_migration.viewer.expose().is_empty());
        assert!(app.legacy_migration.viewer_kind.is_none());
    }

    #[test]
    fn legacy_migration_generation_never_resets_when_configuration_changes() {
        let mut app = App::default();
        app.legacy_migration.generation = 41;

        app.clear_config_scoped_state();
        assert_eq!(app.legacy_migration.generation, 42);

        app.begin_legacy_migration();
        assert_eq!(app.legacy_migration.generation, 43);
    }
}
