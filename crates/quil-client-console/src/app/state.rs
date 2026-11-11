use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
};

use crate::i18n::UiText;
use crate::{
    capability::{
        CapabilityCatalog, CapabilityGroup, CapabilitySpec, CapabilityTask, CapabilityValues,
    },
    config::ConfigEntry,
    domain::{
        AliasMutation, AliasSnapshot, AllocationStatus, ArchiveShardSnapshot,
        ConfidentialWalletSnapshot, DiagnosticsIdentitySnapshot, DiagnosticsReport, HealthSnapshot,
        HealthTarget, HypergraphKind, HypergraphSnapshot, KeyMutation, KeySnapshot,
        LegacyAssetsSnapshot, LegacyMigrationDestination, LegacyMigrationPreflight,
        LegacyMigrationStatus, LogExportOptions, LogExportPreview, LogSnapshot, NodeSnapshot,
        OperationOutcome, OperationResult, PeerAddressReport, PendingEscrowsSnapshot, ProverAction,
        RewardMintPreflight, RewardMintStatus, RewardSnapshot, RuntimeSnapshot, ShardInfoSnapshot,
        WalletAction, WalletActionRequest, WalletOperationResult, WalletPreflight,
    },
    i18n::Locale,
    settings::{ConsoleSettings, WebSettings},
};

use super::Page;
use zeroize::Zeroizing;

pub const CAPABILITY_GROUPS: [CapabilityGroup; 7] = [
    CapabilityGroup::Node,
    CapabilityGroup::Prover,
    CapabilityGroup::Wallet,
    CapabilityGroup::Identity,
    CapabilityGroup::Data,
    CapabilityGroup::Diagnostics,
    CapabilityGroup::System,
];

#[derive(Clone, Default)]
pub struct CapabilityUiValues(BTreeMap<String, Vec<Zeroizing<String>>>);

impl std::fmt::Debug for CapabilityUiValues {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CapabilityUiValues")
            .field("field_count", &self.0.len())
            .finish_non_exhaustive()
    }
}

impl CapabilityUiValues {
    pub fn reset_for(&mut self, capability: &CapabilitySpec) {
        self.0.clear();
        for field in &capability.fields {
            let values = if field.default_values.is_empty() {
                vec![Zeroizing::new(String::new())]
            } else {
                field
                    .default_values
                    .iter()
                    .cloned()
                    .map(Zeroizing::new)
                    .collect()
            };
            self.0.insert(field.id.clone(), values);
        }
    }

    pub fn get(&self, id: &str) -> Option<Vec<&str>> {
        self.0
            .get(id)
            .map(|values| values.iter().map(|value| value.as_str()).collect())
    }

    pub fn value_count(&self, id: &str) -> usize {
        self.0.get(id).map_or(0, Vec::len)
    }

    pub fn value(&self, id: &str, index: usize) -> &str {
        self.0
            .get(id)
            .and_then(|values| values.get(index))
            .map(|value| value.as_str())
            .unwrap_or_default()
    }

    pub fn set_single(&mut self, id: &str, value: impl Into<String>) {
        self.0
            .insert(id.to_owned(), vec![Zeroizing::new(value.into())]);
    }

    pub fn push_empty(&mut self, id: &str) {
        self.0
            .entry(id.to_owned())
            .or_default()
            .push(Zeroizing::new(String::new()));
    }

    pub fn pop_extra(&mut self, id: &str) {
        let Some(values) = self.0.get_mut(id) else {
            return;
        };
        if values.len() > 1 {
            values.pop();
        }
    }

    pub fn push_char(&mut self, id: &str, character: char) {
        let values = self.0.entry(id.to_owned()).or_default();
        if values.is_empty() {
            values.push(Zeroizing::new(String::new()));
        }
        values.last_mut().expect("value exists").push(character);
    }

    pub fn pop_char(&mut self, id: &str) {
        if let Some(value) = self.0.get_mut(id).and_then(|values| values.last_mut()) {
            value.pop();
        }
    }

    pub fn toggle_boolean(&mut self, id: &str) {
        let next = self.value(id, 0) != "true";
        self.set_single(id, if next { "true" } else { "false" });
    }

    pub fn into_plain(self) -> CapabilityValues {
        self.into_plain_and_redactions(&[]).0
    }

    pub fn into_plain_and_redactions(
        self,
        sensitive_field_ids: &[String],
    ) -> (CapabilityValues, Vec<Zeroizing<String>>) {
        let mut redactions = Vec::new();
        let values = self
            .0
            .into_iter()
            .map(|(id, values)| {
                let sensitive = sensitive_field_ids.iter().any(|field_id| field_id == &id);
                (
                    id,
                    values
                        .into_iter()
                        .map(|mut value| {
                            let plain = std::mem::take(&mut *value);
                            if sensitive && !plain.is_empty() {
                                redactions.push(Zeroizing::new(plain.clone()));
                            }
                            plain
                        })
                        .collect(),
                )
            })
            .collect();
        (values, redactions)
    }
}

#[derive(Clone)]
pub struct CapabilityRequest {
    pub request_id: u64,
    pub capability_id: String,
    pub values: CapabilityUiValues,
    pub sensitive_field_ids: Vec<String>,
    pub confirmed: bool,
}

impl std::fmt::Debug for CapabilityRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CapabilityRequest")
            .field("request_id", &self.request_id)
            .field("capability_id", &self.capability_id)
            .field("confirmed", &self.confirmed)
            .field("sensitive_field_count", &self.sensitive_field_ids.len())
            .field("values", &"<redacted>")
            .finish()
    }
}

#[derive(Clone)]
pub struct CapabilityUiState {
    pub catalog: Vec<CapabilitySpec>,
    pub group_index: usize,
    pub item_index: usize,
    pub current_id: Option<String>,
    pub values: CapabilityUiValues,
    pub field_index: usize,
    pub editing: bool,
    pub error: Option<String>,
    pub detail_scroll: usize,
    pub confirmation: ConfirmChoice,
    pub next_request_id: u64,
    pub pending_request_id: Option<u64>,
    pub current_request_id: Option<u64>,
    pub current_task_id: Option<u64>,
    pub current_task: Option<CapabilityTask>,
    pub task_scroll: usize,
    pub queued: Option<CapabilityRequest>,
}

impl std::fmt::Debug for CapabilityUiState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CapabilityUiState")
            .field("catalog_len", &self.catalog.len())
            .field("group_index", &self.group_index)
            .field("item_index", &self.item_index)
            .field("current_id", &self.current_id)
            .field("field_index", &self.field_index)
            .field("editing", &self.editing)
            .field("error", &self.error)
            .field("detail_scroll", &self.detail_scroll)
            .field("confirmation", &self.confirmation)
            .field("pending_request_id", &self.pending_request_id)
            .field("current_request_id", &self.current_request_id)
            .field("current_task_id", &self.current_task_id)
            .field(
                "task_state",
                &self.current_task.as_ref().map(|task| task.state),
            )
            .field("task_scroll", &self.task_scroll)
            .field("queued", &self.queued)
            .finish()
    }
}

impl Default for CapabilityUiState {
    fn default() -> Self {
        let catalog = CapabilityCatalog::discover()
            .map(|catalog| catalog.iter().cloned().collect())
            .unwrap_or_default();
        Self {
            catalog,
            group_index: 0,
            item_index: 0,
            current_id: None,
            values: CapabilityUiValues::default(),
            field_index: 0,
            editing: false,
            error: None,
            detail_scroll: 0,
            confirmation: ConfirmChoice::Cancel,
            next_request_id: 1,
            pending_request_id: None,
            current_request_id: None,
            current_task_id: None,
            current_task: None,
            task_scroll: 0,
            queued: None,
        }
    }
}

impl CapabilityUiState {
    pub fn replace_catalog(&mut self, catalog: Vec<CapabilitySpec>) {
        let first_group = catalog.first().map(|capability| capability.group);
        self.catalog = catalog;
        self.group_index = first_group
            .and_then(|group| {
                CAPABILITY_GROUPS
                    .iter()
                    .position(|candidate| *candidate == group)
            })
            .unwrap_or_default();
        self.item_index = 0;
        self.current_id = None;
        self.values = CapabilityUiValues::default();
        self.detail_scroll = 0;
        self.queued = None;
        self.pending_request_id = None;
        self.current_request_id = None;
        self.error = None;
    }

    pub fn group(&self) -> CapabilityGroup {
        CAPABILITY_GROUPS[self.group_index.min(CAPABILITY_GROUPS.len() - 1)]
    }

    pub fn group_items(&self) -> Vec<&CapabilitySpec> {
        let group = self.group();
        self.catalog
            .iter()
            .filter(|capability| capability.group == group)
            .collect()
    }

    pub fn selected(&self) -> Option<&CapabilitySpec> {
        let items = self.group_items();
        items
            .get(self.item_index.min(items.len().saturating_sub(1)))
            .copied()
    }

    pub fn current(&self) -> Option<&CapabilitySpec> {
        let id = self.current_id.as_deref()?;
        self.catalog.iter().find(|capability| capability.id == id)
    }

    pub fn select_group(&mut self, delta: isize) {
        self.group_index = if delta < 0 {
            (self.group_index + CAPABILITY_GROUPS.len() - 1) % CAPABILITY_GROUPS.len()
        } else {
            (self.group_index + 1) % CAPABILITY_GROUPS.len()
        };
        self.item_index = 0;
        self.detail_scroll = 0;
        self.error = None;
    }

    pub fn select_item(&mut self, delta: isize) {
        let count = self.group_items().len();
        if count == 0 {
            self.item_index = 0;
        } else if delta < 0 {
            self.item_index = (self.item_index + count - 1) % count;
        } else {
            self.item_index = (self.item_index + 1) % count;
        }
        self.detail_scroll = 0;
        self.error = None;
    }

    pub fn begin_selected(&mut self) -> Option<CapabilitySpec> {
        let id = self.selected()?.id.clone();
        self.begin(&id)
    }

    pub fn begin(&mut self, id: &str) -> Option<CapabilitySpec> {
        let capability = self
            .catalog
            .iter()
            .find(|capability| capability.id == id)?
            .clone();
        self.current_id = Some(capability.id.clone());
        self.values.reset_for(&capability);
        self.field_index = 0;
        self.editing = false;
        self.detail_scroll = 0;
        self.error = None;
        self.confirmation = ConfirmChoice::Cancel;
        Some(capability)
    }

    pub fn allocate_request_id(&mut self) -> u64 {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        request_id
    }

    pub fn clear_draft(&mut self) {
        self.values = CapabilityUiValues::default();
        self.field_index = 0;
        self.editing = false;
        self.error = None;
        self.confirmation = ConfirmChoice::Cancel;
    }

    pub fn clear_sensitive_state(&mut self) {
        self.clear_draft();
        self.queued = None;
        self.pending_request_id = None;
    }

    pub fn max_detail_scroll(&self, viewport_width: u16, viewport_height: u16) -> usize {
        let Some(capability) = self.selected() else {
            return 0;
        };
        if viewport_width < 42 || viewport_height < 12 {
            return 0;
        }
        if viewport_width <= 69 {
            let visible = usize::from(viewport_height.saturating_sub(6).min(8));
            return 3usize
                .saturating_add(capability.fields.len().max(1))
                .saturating_sub(visible);
        }
        let list_width = viewport_width.saturating_mul(38) / 100;
        let detail_width = viewport_width.saturating_sub(list_width);
        let inner_width = usize::from(detail_width.saturating_sub(2)).max(1);
        let description_lines =
            crate::ui::text::wrapped_cell_line_count(&capability.description, inner_width);
        let error_lines = self.error.as_deref().map_or(0, |error| {
            1usize.saturating_add(crate::ui::text::wrapped_cell_line_count(error, inner_width))
        });
        let content_lines = 6usize
            .saturating_add(description_lines)
            .saturating_add(capability.fields.len())
            .saturating_add(error_lines);
        let visible = usize::from(viewport_height.saturating_sub(5)).max(1);
        content_lines.saturating_sub(visible)
    }

    pub fn clamp_detail_scroll(&mut self, viewport_width: u16, viewport_height: u16) {
        self.detail_scroll = self
            .detail_scroll
            .min(self.max_detail_scroll(viewport_width, viewport_height));
    }

    pub fn max_task_scroll(&self, viewport_width: u16, viewport_height: u16) -> usize {
        let Some(task) = &self.current_task else {
            return 0;
        };
        let width = usize::from(viewport_width.saturating_sub(2)).max(1);
        let visible = usize::from(viewport_height.saturating_sub(4)).max(1);
        let wrapped_lines = |value: &str| crate::ui::text::wrapped_cell_line_count(value, width);
        let content_lines = 6usize
            .saturating_add(wrapped_lines(&task.capability_id))
            .saturating_add(wrapped_lines(&task.stdout))
            .saturating_add(wrapped_lines(&task.stderr));
        content_lines.saturating_sub(visible)
    }

    pub fn clamp_task_scroll(&mut self, viewport_width: u16, viewport_height: u16) {
        self.task_scroll = self
            .task_scroll
            .min(self.max_task_scroll(viewport_width, viewport_height));
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConfirmChoice {
    #[default]
    Cancel,
    Confirm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationRequest {
    pub generation: u64,
    pub action: ProverAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProverOperationRequest {
    pub id: u64,
    pub action: ProverAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProverOperationPhase {
    Submitting,
    Accepted,
    Unknown,
    Failed,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedProverOperation {
    pub id: u64,
    pub action: ProverAction,
    pub filter_hex: String,
    pub core_id: Option<u32>,
    pub submitted_unix_seconds: u64,
    pub phase: ProverOperationPhase,
    pub observed: Option<AllocationStatus>,
    pub message: String,
}

impl TrackedProverOperation {
    pub fn is_pending(&self) -> bool {
        !matches!(
            self.phase,
            ProverOperationPhase::Failed | ProverOperationPhase::Complete
        )
    }
}

#[derive(Debug, Default, Clone)]
pub struct ProverOperationState {
    pub items: Vec<TrackedProverOperation>,
    next_id: u64,
    queued: VecDeque<ProverOperationRequest>,
}

impl ProverOperationState {
    pub fn enqueue(
        &mut self,
        action: ProverAction,
        core_id: Option<u32>,
        submitted_unix_seconds: u64,
    ) -> Result<u64, String> {
        let filter = action
            .filters()
            .first()
            .ok_or_else(|| "The operation has no Filter".to_string())?;
        if self.conflicts_filter(filter) {
            return Err("This Filter already has an operation in progress".into());
        }
        if core_id.is_some_and(|core_id| self.conflicts_core(core_id)) {
            return Err("This Core already has an operation in progress".into());
        }
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let id = self.next_id;
        self.items.push(TrackedProverOperation {
            id,
            filter_hex: hex::encode(filter),
            core_id,
            submitted_unix_seconds,
            phase: ProverOperationPhase::Submitting,
            observed: None,
            message: "Submitting request".into(),
            action: action.clone(),
        });
        self.queued.push_back(ProverOperationRequest { id, action });
        Ok(id)
    }

    pub fn conflicts_filter(&self, filter: &[u8]) -> bool {
        let filter = hex::encode(filter);
        self.items
            .iter()
            .any(|item| item.is_pending() && item.filter_hex == filter)
    }

    pub fn conflicts_core(&self, core_id: u32) -> bool {
        self.items
            .iter()
            .any(|item| item.is_pending() && item.core_id == Some(core_id))
    }

    pub fn has_pending(&self) -> bool {
        self.items.iter().any(TrackedProverOperation::is_pending)
    }

    pub fn take_request(&mut self) -> Option<ProverOperationRequest> {
        self.queued.pop_front()
    }

    pub(crate) fn clear_config_scoped(&mut self) {
        self.items.clear();
        self.queued.clear();
    }

    pub fn finish_submission(&mut self, id: u64, result: Result<OperationResult, String>) {
        let Some(item) = self.items.iter_mut().find(|item| item.id == id) else {
            return;
        };
        if item.phase == ProverOperationPhase::Complete {
            return;
        }
        match result {
            Ok(result) => {
                item.message = result.message;
                item.phase = match result.outcome {
                    OperationOutcome::Submitted => ProverOperationPhase::Accepted,
                    OperationOutcome::Unknown => ProverOperationPhase::Unknown,
                };
            }
            Err(error) => {
                item.message = error;
                item.phase = ProverOperationPhase::Failed;
            }
        }
    }

    pub fn reconcile(&mut self, snapshot: &NodeSnapshot) {
        for item in self.items.iter_mut().filter(|item| item.is_pending()) {
            let allocation = snapshot
                .allocations
                .iter()
                .find(|allocation| allocation.filter_hex == item.filter_hex);
            item.observed = allocation.map(|allocation| allocation.status);
            match &item.action {
                ProverAction::Join { .. }
                    if allocation.is_some_and(|value| value.status == AllocationStatus::Active) =>
                {
                    item.phase = ProverOperationPhase::Complete;
                    item.message = "Join completed".into();
                }
                ProverAction::Leave { .. }
                    if allocation
                        .is_some_and(|value| value.status == AllocationStatus::Historic)
                        || (allocation.is_none()
                            && matches!(
                                item.phase,
                                ProverOperationPhase::Accepted | ProverOperationPhase::Unknown
                            )) =>
                {
                    item.phase = ProverOperationPhase::Complete;
                    item.message = "Leave completed".into();
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod prover_operation_tests {
    use super::*;
    use crate::domain::AllocationSnapshot;

    fn join_action(filter: u8) -> ProverAction {
        ProverAction::Join {
            filters: vec![vec![filter; 32]],
            delegate: Vec::new(),
            worker_ids: vec![3],
        }
    }

    #[test]
    fn late_submission_result_cannot_downgrade_a_completed_join() {
        let mut state = ProverOperationState::default();
        let id = state.enqueue(join_action(0x11), Some(3), 1).unwrap();
        state.reconcile(&NodeSnapshot {
            allocations: vec![AllocationSnapshot {
                filter_hex: "11".repeat(32),
                status: AllocationStatus::Active,
                ..Default::default()
            }],
            ..Default::default()
        });

        state.finish_submission(
            id,
            Ok(OperationResult {
                message: "accepted later".into(),
                outcome: OperationOutcome::Submitted,
            }),
        );

        assert_eq!(state.items[0].phase, ProverOperationPhase::Complete);
        assert_eq!(state.items[0].message, "Join completed");
    }

    #[test]
    fn unknown_leave_completes_when_the_allocation_disappears() {
        let mut state = ProverOperationState::default();
        let id = state
            .enqueue(
                ProverAction::Leave {
                    filters: vec![vec![0x22; 32]],
                },
                Some(7),
                1,
            )
            .unwrap();
        state.finish_submission(
            id,
            Ok(OperationResult {
                message: "timed out".into(),
                outcome: OperationOutcome::Unknown,
            }),
        );

        state.reconcile(&NodeSnapshot::default());

        assert_eq!(state.items[0].phase, ProverOperationPhase::Complete);
        assert!(!state.conflicts_core(7));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticsRequest {
    pub generation: u64,
    pub address: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticsIdentityRequest {
    pub generation: u64,
    pub path: String,
    pub create: bool,
}

#[derive(Debug, Default, Clone)]
pub struct DiagnosticsIdentityState {
    pub path: String,
    pub editing: bool,
    pub action_index: usize,
    pub generation: u64,
    pub in_flight: bool,
    pub snapshot: Option<DiagnosticsIdentitySnapshot>,
    pub error: Option<String>,
    pub(crate) queued: Option<DiagnosticsIdentityRequest>,
}

#[derive(Debug, Default, Clone)]
pub struct RemoteHealthState {
    pub peer_id: String,
    pub address: String,
    pub field_index: usize,
    pub editing: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerResolveRequest {
    pub generation: u64,
    pub peer_id: String,
}

#[derive(Debug, Default, Clone)]
pub struct PeerResolveState {
    pub peer_id: String,
    pub editing: bool,
    pub generation: u64,
    pub in_flight: bool,
    pub report: Option<PeerAddressReport>,
    pub error: Option<String>,
    pub index: usize,
    pub(crate) queued: Option<PeerResolveRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthRequest {
    pub generation: u64,
    pub target: HealthTarget,
}

#[derive(Debug, Default, Clone)]
pub struct HealthState {
    pub target: HealthTarget,
    pub generation: u64,
    pub in_flight: bool,
    pub snapshot: Option<HealthSnapshot>,
    pub error: Option<String>,
    pub index: usize,
    pub detail: bool,
    pub(crate) queued: Option<HealthRequest>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeRequest {
    pub generation: u64,
}

#[derive(Debug, Default, Clone)]
pub struct RuntimeState {
    pub generation: u64,
    pub in_flight: bool,
    pub snapshot: Option<RuntimeSnapshot>,
    pub error: Option<String>,
    pub(crate) queued: Option<RuntimeRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPathRequest {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRequest {
    pub generation: u64,
    pub core_id: u32,
    pub lines: usize,
}

pub const LOG_EXPORT_WINDOWS: [i64; 4] = [60 * 60, 2 * 60 * 60, 6 * 60 * 60, 24 * 60 * 60];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogExportRequest {
    pub generation: u64,
    pub options: LogExportOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogExportFinishRequest {
    pub generation: u64,
    pub id: u64,
    pub commit: bool,
}

#[derive(Debug, Clone)]
pub struct LogExportState {
    pub field_index: usize,
    pub window_index: usize,
    pub level_index: usize,
    pub choice: ConfirmChoice,
    pub generation: u64,
    pub in_flight: bool,
    pub preview: Option<LogExportPreview>,
    pub error: Option<String>,
    pub(crate) queued_prepare: Option<LogExportRequest>,
    pub(crate) queued_finish: Option<LogExportFinishRequest>,
}

impl Default for LogExportState {
    fn default() -> Self {
        Self {
            field_index: 0,
            window_index: 1,
            level_index: 2,
            choice: ConfirmChoice::Cancel,
            generation: 0,
            in_flight: false,
            preview: None,
            error: None,
            queued_prepare: None,
            queued_finish: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyRequest {
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AliasRequest {
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RewardRequest {
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RewardMintReadRequest {
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewardMintStartRequest {
    pub generation: u64,
    pub approved: RewardMintPreflight,
}

#[derive(Debug, Clone)]
pub struct RewardMintState {
    pub generation: u64,
    pub in_flight: bool,
    pub preflight: Option<RewardMintPreflight>,
    pub status: Option<RewardMintStatus>,
    pub error: Option<String>,
    pub confirmation: ConfirmChoice,
    pub rebroadcast_armed: bool,
    pub(crate) queued_preflight: Option<RewardMintReadRequest>,
    pub(crate) queued_start: Option<RewardMintStartRequest>,
    pub(crate) queued_load: Option<RewardMintReadRequest>,
    pub(crate) queued_advance: Option<RewardMintReadRequest>,
    pub(crate) queued_rebroadcast: Option<RewardMintReadRequest>,
}

impl Default for RewardMintState {
    fn default() -> Self {
        Self {
            generation: 0,
            in_flight: false,
            preflight: None,
            status: None,
            error: None,
            confirmation: ConfirmChoice::Cancel,
            rebroadcast_armed: false,
            queued_preflight: None,
            queued_start: None,
            queued_load: None,
            queued_advance: None,
            queued_rebroadcast: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyAssetsRequest {
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LegacyMigrationStep {
    #[default]
    Inactive,
    Identity,
    Destination,
    Preflight,
    Progress,
    Files,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LegacyDestinationChoice {
    #[default]
    CurrentNode,
    ExistingConfig,
    CreateNew,
}

impl LegacyDestinationChoice {
    pub const fn next(self) -> Self {
        match self {
            Self::CurrentNode => Self::ExistingConfig,
            Self::ExistingConfig => Self::CreateNew,
            Self::CreateNew => Self::CurrentNode,
        }
    }

    pub const fn previous(self) -> Self {
        match self {
            Self::CurrentNode => Self::CreateNew,
            Self::ExistingConfig => Self::CurrentNode,
            Self::CreateNew => Self::ExistingConfig,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMigrationFileKind {
    Config,
    Keys,
}

/// Secret input is redacted from Debug and intentionally omitted from App clones.
#[derive(Default)]
pub struct LegacySecretDraft(Zeroizing<String>);

impl std::fmt::Debug for LegacySecretDraft {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("LegacySecretDraft(<redacted>)")
    }
}

impl Clone for LegacySecretDraft {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl LegacySecretDraft {
    pub fn push(&mut self, character: char) {
        self.0.push(character);
    }
    pub fn pop(&mut self) {
        self.0.pop();
    }
    pub fn clear(&mut self) {
        self.0.clear();
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
    pub fn masked(&self) -> String {
        let value = self.0.trim().strip_prefix("0x").unwrap_or(self.0.trim());
        if value.len() < 8 {
            "••••".into()
        } else {
            format!("{}…{}", &value[..4], &value[value.len() - 4..])
        }
    }
}

/// Full config/key contents exist only while the explicit viewer is open.
#[derive(Default)]
pub struct LegacySensitiveText(Zeroizing<String>);

impl std::fmt::Debug for LegacySensitiveText {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("LegacySensitiveText(<redacted>)")
    }
}

impl Clone for LegacySensitiveText {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl LegacySensitiveText {
    pub fn replace(&mut self, value: String) {
        self.0 = Zeroizing::new(value);
    }
    pub fn clear(&mut self) {
        self.0.clear();
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct LegacyMigrationTuiState {
    pub step: LegacyMigrationStep,
    pub secret: LegacySecretDraft,
    pub masked_identity: Option<String>,
    pub fingerprint: Option<String>,
    pub destination: LegacyDestinationChoice,
    pub existing_path: String,
    pub editing_path: bool,
    pub generation: u64,
    pub in_flight: bool,
    pub preflight: Option<LegacyMigrationPreflight>,
    pub status: Option<LegacyMigrationStatus>,
    pub error: Option<String>,
    pub confirmation: ConfirmChoice,
    pub recovery_armed: bool,
    pub file_confirmation: Option<LegacyMigrationFileKind>,
    pub viewer_kind: Option<LegacyMigrationFileKind>,
    pub viewer: LegacySensitiveText,
    pub(crate) queued_load: bool,
    pub(crate) queued_preflight: bool,
    pub(crate) queued_start: bool,
    pub(crate) queued_advance: bool,
    pub(crate) queued_advance_with_source: bool,
    pub(crate) queued_rebroadcast: bool,
    pub(crate) queued_file: Option<LegacyMigrationFileKind>,
}

impl Default for LegacyMigrationTuiState {
    fn default() -> Self {
        Self {
            step: LegacyMigrationStep::Inactive,
            secret: LegacySecretDraft::default(),
            masked_identity: None,
            fingerprint: None,
            destination: LegacyDestinationChoice::CurrentNode,
            existing_path: String::new(),
            editing_path: false,
            generation: 0,
            in_flight: false,
            preflight: None,
            status: None,
            error: None,
            confirmation: ConfirmChoice::Cancel,
            recovery_armed: false,
            file_confirmation: None,
            viewer_kind: None,
            viewer: LegacySensitiveText::default(),
            queued_load: false,
            queued_preflight: false,
            queued_start: false,
            queued_advance: false,
            queued_advance_with_source: false,
            queued_rebroadcast: false,
            queued_file: None,
        }
    }
}

impl LegacyMigrationTuiState {
    pub fn clear_and_invalidate(&mut self) {
        let generation = self.generation.wrapping_add(1);
        *self = Self::default();
        self.generation = generation;
    }

    pub fn destination(&self) -> LegacyMigrationDestination {
        match self.destination {
            LegacyDestinationChoice::CurrentNode => LegacyMigrationDestination::CurrentNode,
            LegacyDestinationChoice::ExistingConfig => {
                LegacyMigrationDestination::ExistingConfig(self.existing_path.trim().into())
            }
            LegacyDestinationChoice::CreateNew => LegacyMigrationDestination::CreateNew,
        }
    }

    pub fn close_viewer(&mut self) {
        self.viewer.clear();
        self.viewer_kind = None;
        self.file_confirmation = None;
    }
}

pub struct LegacyMigrationPreflightWork {
    pub generation: u64,
    pub request: crate::service::legacy_migration::LegacyMigrationRequest,
}

#[derive(Debug, Clone)]
pub struct LegacyMigrationStartWork {
    pub generation: u64,
    pub approved: LegacyMigrationPreflight,
}

#[derive(Debug, Clone)]
pub struct LegacyMigrationReadWork {
    pub generation: u64,
    pub fingerprint: String,
}

pub struct LegacyMigrationAdvanceWork {
    pub generation: u64,
    pub fingerprint: String,
    pub source: Option<crate::service::legacy_migration::SecretLegacyKey>,
}

#[derive(Debug, Clone)]
pub struct LegacyMigrationFileWork {
    pub generation: u64,
    pub recipient_config_dir: PathBuf,
    pub kind: LegacyMigrationFileKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfidentialWalletRequest {
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingEscrowsRequest {
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveShardRequest {
    pub generation: u64,
}

#[derive(Debug, Default, Clone)]
pub struct ConfidentialWalletState {
    pub generation: u64,
    pub in_flight: bool,
    pub snapshot: Option<ConfidentialWalletSnapshot>,
    pub error: Option<String>,
    pub coin_index: usize,
    pub(crate) queued: Option<ConfidentialWalletRequest>,
}

#[derive(Debug, Default, Clone)]
pub struct PendingEscrowsState {
    pub generation: u64,
    pub in_flight: bool,
    pub snapshot: Option<PendingEscrowsSnapshot>,
    pub error: Option<String>,
    pub index: usize,
    pub(crate) queued: Option<PendingEscrowsRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletPreflightRequest {
    pub generation: u64,
    pub request: WalletActionRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletExecuteRequest {
    pub generation: u64,
    pub request: WalletActionRequest,
    pub approved: WalletPreflight,
}

#[derive(Debug, Clone)]
pub struct WalletWorkflowState {
    pub action_index: usize,
    pub action: WalletAction,
    pub form_field: usize,
    pub editing: bool,
    pub form_application: String,
    pub form_target: String,
    pub form_amount: String,
    pub form_explicit_fee: String,
    pub form_maximum_fee: String,
    pub form_refund_frame: String,
    pub confirmation: ConfirmChoice,
    pub generation: u64,
    pub in_flight: bool,
    pub request: Option<WalletActionRequest>,
    pub preflight: Option<WalletPreflight>,
    pub result: Option<WalletOperationResult>,
    pub error: Option<String>,
    pub(crate) queued_preflight: Option<WalletPreflightRequest>,
    pub(crate) queued_execute: Option<WalletExecuteRequest>,
}

impl Default for WalletWorkflowState {
    fn default() -> Self {
        Self {
            action_index: 0,
            action: WalletAction::Transfer,
            form_field: 0,
            editing: false,
            form_application: hex::encode(quil_execution::domains::QUIL_TOKEN),
            form_target: String::new(),
            form_amount: String::new(),
            form_explicit_fee: String::new(),
            form_maximum_fee: "1000000000".into(),
            form_refund_frame: String::new(),
            confirmation: ConfirmChoice::Cancel,
            generation: 0,
            in_flight: false,
            request: None,
            preflight: None,
            result: None,
            error: None,
            queued_preflight: None,
            queued_execute: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HypergraphRequest {
    pub generation: u64,
    pub kind: HypergraphKind,
    pub address: String,
}

#[derive(Debug, Default, Clone)]
pub struct HypergraphState {
    pub kind: HypergraphKind,
    pub address: String,
    pub field_index: usize,
    pub editing: bool,
    pub generation: u64,
    pub in_flight: bool,
    pub snapshot: Option<HypergraphSnapshot>,
    pub error: Option<String>,
    pub entry_index: usize,
    pub(crate) queued: Option<HypergraphRequest>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ShardScope {
    #[default]
    Allocated,
    All,
}

impl ShardScope {
    pub fn include_all(self) -> bool {
        self == Self::All
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Allocated => "My shards",
            Self::All => "All shards",
        }
    }

    pub fn localized_label(self, locale: Locale) -> &'static str {
        match self {
            Self::Allocated => crate::i18n::t(locale, UiText::MyShards),
            Self::All => crate::i18n::t(locale, UiText::AllShards),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShardRequest {
    pub generation: u64,
    pub include_all: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ShardFocus {
    #[default]
    Directory,
    FreeCores,
}

#[derive(Debug, Default, Clone)]
pub struct ShardPageState {
    pub scope: ShardScope,
    pub generation: u64,
    pub in_flight: bool,
    pub snapshot: Option<ShardInfoSnapshot>,
    pub error: Option<String>,
    pub index: usize,
    pub focus: ShardFocus,
    pub core_index: usize,
    pub notice: Option<String>,
    pub detail_scroll: usize,
    pub(crate) queued: Option<ShardRequest>,
}

#[derive(Debug, Default, Clone)]
pub struct ArchiveShardPageState {
    pub generation: u64,
    pub in_flight: bool,
    pub snapshot: Option<ArchiveShardSnapshot>,
    pub error: Option<String>,
    pub index: usize,
    pub detail: bool,
    pub(crate) queued: Option<ArchiveShardRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasMutationRequest {
    pub generation: u64,
    pub mutation: AliasMutation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyMutationRequest {
    pub generation: u64,
    pub mutation: KeyMutation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsoleSettingsRequest {
    pub generation: u64,
    pub settings: ConsoleSettings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadRpcSelection {
    pub override_address: Option<String>,
    pub effective_address: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadRpcRequest {
    pub generation: u64,
    pub config_dir: PathBuf,
    /// `None` loads the setting; `Some(None)` clears it; `Some(Some(_))` saves an override.
    pub value: Option<Option<String>>,
}

#[derive(Debug, Clone)]
pub struct OtherState {
    pub settings: WebSettings,
    pub selected: usize,
    pub port_input: String,
    pub editing_port: bool,
    pub read_rpc_override: Option<String>,
    pub effective_read_rpc: String,
    pub read_rpc_input: String,
    pub editing_read_rpc: bool,
    pub read_rpc_in_flight: bool,
    pub read_rpc_generation: u64,
    pub read_rpc_config_dir: Option<PathBuf>,
    pub in_flight: bool,
    pub error: Option<String>,
    pub generation: u64,
    pub(crate) previous_locale: Option<Locale>,
    pub(crate) queued: Option<ConsoleSettingsRequest>,
    pub(crate) queued_read_rpc: Option<ReadRpcRequest>,
}

impl Default for OtherState {
    fn default() -> Self {
        let settings = WebSettings::default();
        Self {
            settings,
            selected: 0,
            port_input: settings.port.to_string(),
            editing_port: false,
            read_rpc_override: None,
            effective_read_rpc: String::new(),
            read_rpc_input: String::new(),
            editing_read_rpc: false,
            read_rpc_in_flight: false,
            read_rpc_generation: 0,
            read_rpc_config_dir: None,
            in_flight: false,
            error: None,
            generation: 0,
            previous_locale: None,
            queued: None,
            queued_read_rpc: None,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct App {
    pub locale: Locale,
    pub other: OtherState,
    pub capabilities: CapabilityUiState,
    pub page: Page,
    pub viewport_width: u16,
    pub viewport_height: u16,
    pub prover_detail_scroll: usize,
    pub(crate) narrow_read_detail: bool,
    pub(crate) route_stack: Vec<Page>,
    pub menu_index: usize,
    pub section_index: usize,
    pub configs: Vec<ConfigEntry>,
    pub config_index: usize,
    pub active_config_index: usize,
    pub(crate) queued_config_index: Option<usize>,
    pub config_path_input: String,
    pub config_path_error: Option<String>,
    pub config_path_required: bool,
    pub(crate) config_path_in_flight: bool,
    pub(crate) queued_config_path: Option<ConfigPathRequest>,
    pub allocation_index: usize,
    pub expanded_allocation: Option<usize>,
    pub action_index: usize,
    pub should_quit: bool,
    pub loading: bool,
    pub(crate) node_refresh_requested: bool,
    pub refresh_generation: u64,
    pub(crate) applied_snapshot_revision: u64,
    pub snapshot: Option<NodeSnapshot>,
    pub error: Option<String>,
    pub confirm_choice: ConfirmChoice,
    pub pending_action: Option<ProverAction>,
    pub operation_message: Option<String>,
    pub operation_succeeded: Option<bool>,
    pub(crate) queued_operation: Option<OperationRequest>,
    pub(crate) operation_generation: u64,
    pub operation_in_flight: bool,
    pub prover_operations: ProverOperationState,
    pub diagnostics_address: String,
    pub diagnostics_editing: bool,
    pub diagnostics_generation: u64,
    pub diagnostics_in_flight: bool,
    pub diagnostics_report: Option<DiagnosticsReport>,
    pub diagnostics_error: Option<String>,
    pub(crate) queued_diagnostics: Option<DiagnosticsRequest>,
    pub diagnostics_identity: DiagnosticsIdentityState,
    pub remote_health: RemoteHealthState,
    pub peer_resolve: PeerResolveState,
    pub health: HealthState,
    pub runtime: RuntimeState,
    pub log_core_index: usize,
    pub log_scroll: usize,
    pub log_generation: u64,
    pub log_in_flight: bool,
    pub log_snapshot: Option<LogSnapshot>,
    pub log_error: Option<String>,
    pub(crate) queued_log: Option<LogRequest>,
    pub log_export: LogExportState,
    pub keys: Vec<KeySnapshot>,
    pub key_index: usize,
    pub key_generation: u64,
    pub key_in_flight: bool,
    pub key_error: Option<String>,
    pub(crate) queued_key: Option<KeyRequest>,
    pub key_action_index: usize,
    pub key_form_field: usize,
    pub key_form_editing: bool,
    pub key_form_id: String,
    pub key_form_type_index: usize,
    pub key_form_error: Option<String>,
    pub(crate) pending_key_mutation: Option<KeyMutation>,
    pub(crate) queued_key_mutation: Option<KeyMutationRequest>,
    pub(crate) key_mutation_generation: u64,
    pub aliases: Vec<AliasSnapshot>,
    pub alias_index: usize,
    pub alias_action_index: usize,
    pub alias_generation: u64,
    pub alias_in_flight: bool,
    pub alias_error: Option<String>,
    pub(crate) queued_alias: Option<AliasRequest>,
    pub alias_form_field: usize,
    pub alias_form_editing: bool,
    pub alias_form_name: String,
    pub alias_form_address: String,
    pub alias_form_kind: String,
    pub alias_form_error: Option<String>,
    pub(crate) pending_alias_mutation: Option<AliasMutation>,
    pub(crate) queued_alias_mutation: Option<AliasMutationRequest>,
    pub(crate) alias_mutation_generation: u64,
    pub reward_generation: u64,
    pub reward_in_flight: bool,
    pub reward_snapshot: Option<RewardSnapshot>,
    pub reward_error: Option<String>,
    pub(crate) queued_reward: Option<RewardRequest>,
    pub reward_mint: RewardMintState,
    pub legacy_generation: u64,
    pub legacy_in_flight: bool,
    pub legacy_snapshot: Option<LegacyAssetsSnapshot>,
    pub legacy_error: Option<String>,
    pub legacy_index: usize,
    pub(crate) queued_legacy: Option<LegacyAssetsRequest>,
    pub legacy_migration: LegacyMigrationTuiState,
    pub confidential_wallet: ConfidentialWalletState,
    pub pending_escrows: PendingEscrowsState,
    pub wallet: WalletWorkflowState,
    pub hypergraph: HypergraphState,
    pub shards: ShardPageState,
    pub archive_shards: ArchiveShardPageState,
}
