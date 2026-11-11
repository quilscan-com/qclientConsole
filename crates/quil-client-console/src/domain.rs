use crate::i18n::UiText;
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NodeSnapshot {
    pub config_name: String,
    pub rpc_address: String,
    pub connected: bool,
    pub peer_id: String,
    pub peer_seniority: Option<String>,
    pub version: Vec<u8>,
    pub patch_number: Vec<u8>,
    pub last_received_frame: u64,
    pub last_global_head_frame: u64,
    pub reachable: bool,
    pub running_workers: u32,
    pub allocated_workers: u32,
    pub peer_score: u64,
    pub epoch: Option<EpochSnapshot>,
    pub frame_cadence: Option<FrameCadenceSnapshot>,
    pub allocations: Vec<AllocationSnapshot>,
    pub workers: Vec<WorkerSnapshot>,
    pub worker_info_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpochSnapshot {
    pub current: u64,
    pub length_frames: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AllocationSnapshot {
    pub filter_hex: String,
    pub status: AllocationStatus,
    pub join_frame_number: u64,
    pub join_confirm_frame_number: u64,
    pub leave_frame_number: u64,
    pub leave_confirm_frame_number: u64,
    pub last_active_frame_number: u64,
    pub epoch: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AllocationStatus {
    #[default]
    Unknown,
    Joining,
    Active,
    Paused,
    Leaving,
    Rejected,
    Kicked,
    Historic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameCadenceSnapshot {
    pub last_observed_unix_seconds: u64,
    pub hour: Option<FrameWindowSnapshot>,
    pub day: Option<FrameWindowSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameWindowSnapshot {
    pub window_seconds: u64,
    pub observed_seconds: u64,
    pub advanced_frames: u64,
}

impl From<u32> for AllocationStatus {
    fn from(value: u32) -> Self {
        match value {
            1 => Self::Joining,
            2 => Self::Active,
            3 => Self::Paused,
            4 => Self::Leaving,
            5 => Self::Rejected,
            6 => Self::Kicked,
            7 => Self::Historic,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerSnapshot {
    pub core_id: u32,
    pub filter_hex: String,
    pub available_storage: u64,
    pub total_storage: u64,
    pub manually_managed: bool,
    pub execution: Option<WorkerExecutionSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerExecutionSnapshot {
    pub state: String,
    pub blocker: Option<String>,
    pub materialized_frame: Option<u64>,
    pub last_advance_unix_ms: u64,
    pub observed_unix_ms: u64,
    pub awaited_frame: Option<u64>,
    pub recovery: Option<RecoverySnapshot>,
    pub seal: Option<SessionSealSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoverySnapshot {
    pub phase: String,
    pub phase_since_unix_ms: u64,
    pub tree_phase: u32,
    pub installed_leaves: u64,
    pub planned_leaves: u64,
    pub leaves_observed_unix_ms: u64,
    pub last_outcome: Option<String>,
    pub last_outcome_unix_ms: u64,
    pub last_error: Option<String>,
    pub last_error_unix_ms: u64,
    pub source: Option<String>,
    pub attempts: u32,
    pub consecutive_failures: u32,
    pub attempt_started_unix_ms: u64,
    pub permit_wait_ms: u64,
    pub attempt_ms: u64,
    pub attempt_installed_leaves: u64,
    pub failures_by_class: Vec<(String, u32)>,
    pub longest_permit_wait_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSealSnapshot {
    pub generation: u64,
    pub checkpoint: u64,
    pub view: u64,
    pub global_executed: u64,
    pub drain: String,
    pub drain_headers: u32,
    pub finalized_unix_ms: u64,
    pub last_submitted_unix_ms: u64,
    pub submissions: u32,
    pub recorded_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProverAction {
    Join {
        filters: Vec<Vec<u8>>,
        delegate: Vec<u8>,
        worker_ids: Vec<u32>,
    },
    Leave {
        filters: Vec<Vec<u8>>,
    },
    Confirm {
        filters: Vec<Vec<u8>>,
    },
    Reject {
        filters: Vec<Vec<u8>>,
    },
    Pause {
        filter: Vec<u8>,
    },
    Resume {
        filter: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationResult {
    pub message: String,
    pub outcome: OperationOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RuntimeSnapshot {
    pub platform: String,
    pub manager: String,
    pub service_name: String,
    pub installed: bool,
    pub running: bool,
    pub enabled: Option<bool>,
    pub state: String,
    pub pid: Option<u32>,
    pub definition_path: String,
    pub auto_update_enabled: Option<bool>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OperationOutcome {
    #[default]
    Submitted,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticsReport {
    pub address: String,
    pub peer_id: String,
    pub transport: String,
    pub connection_ms: u128,
    pub ping_ms: Option<u128>,
    pub agent_version: Option<String>,
    pub protocol_version: Option<String>,
    pub observed_address: Option<String>,
    pub listen_addresses: Vec<PublishedAddressSnapshot>,
    pub protocols: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticsIdentitySnapshot {
    pub path: String,
    pub peer_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthSnapshot {
    pub release: String,
    pub build_fingerprint: String,
    pub global_head_frame: u64,
    pub generated_unix_ms: u64,
    pub omitted_workers: u32,
    pub workers: Vec<HealthWorkerSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum HealthTarget {
    #[default]
    SelfNode,
    Peer {
        peer_id: String,
        address: String,
        identity_path: Option<String>,
    },
}

impl HealthTarget {
    pub fn label(&self) -> String {
        match self {
            Self::SelfNode => "Local node".into(),
            Self::Peer {
                peer_id,
                address,
                identity_path,
            } => {
                let identity = if identity_path.is_some() {
                    " · independent diagnostics identity"
                } else {
                    ""
                };
                format!("Peer {} · {address}{identity}", compact_id(peer_id))
            }
        }
    }

    pub fn localized_label(&self, locale: crate::i18n::Locale) -> String {
        match self {
            Self::SelfNode => crate::i18n::t(locale, UiText::LocalNode).into(),
            Self::Peer {
                peer_id,
                address,
                identity_path,
            } => {
                let identity = if identity_path.is_some() {
                    crate::i18n::t(locale, UiText::IndependentDiagnosticIdentity)
                } else {
                    ""
                };
                format!("Peer {} · {address}{identity}", compact_id(peer_id))
            }
        }
    }
}

fn compact_id(value: &str) -> String {
    if value.chars().count() <= 18 {
        return value.into();
    }
    let head = value.chars().take(8).collect::<String>();
    let tail = value
        .chars()
        .rev()
        .take(6)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{head}…{tail}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthWorkerSnapshot {
    pub core_id: u32,
    pub filter_hex: String,
    pub allocated: bool,
    pub phase: String,
    pub blocker: String,
    pub observed_unix_ms: u64,
    pub materialized_frame: Option<u64>,
    pub last_advance_unix_ms: u64,
    pub committee_head_frame: Option<u64>,
    pub committed_generation: Option<u64>,
    pub recovery_phase: String,
    pub recovery_attempts: u32,
    pub recovery_consecutive_failures: u32,
    pub recovery_failures: Vec<(String, u32)>,
    pub last_failure: String,
    pub last_failure_unix_ms: u64,
    pub last_success_unix_ms: u64,
    pub longest_permit_wait_ms: u64,
    pub seal_generation: Option<u64>,
    pub seal_recorded_unix_ms: u64,
    pub join_confirm_epoch: Option<u64>,
    pub activation_frame: Option<u64>,
    pub awaited_frame: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedAddressSnapshot {
    pub multiaddr: String,
    pub reach: PeerReach,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerReach {
    Public,
    Private,
    Unknown,
}

impl PeerReach {
    pub fn label(self) -> &'static str {
        match self {
            Self::Public => "Public network",
            Self::Private => "Private network",
            Self::Unknown => "Unknown",
        }
    }

    pub fn localized_label(self, locale: crate::i18n::Locale) -> &'static str {
        match self {
            Self::Public => crate::i18n::t(locale, UiText::Public),
            Self::Private => crate::i18n::t(locale, UiText::Private),
            Self::Unknown => crate::i18n::t(locale, UiText::Unknown),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerAddressSnapshot {
    pub kind: String,
    pub host_port: Option<String>,
    pub reach: PeerReach,
    pub multiaddr: String,
    pub filter: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerAddressReport {
    pub peer_id: String,
    pub addresses: Vec<PeerAddressSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogSnapshot {
    pub core_id: u32,
    pub path: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogExportLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

impl LogExportLevel {
    pub const ALL: [Self; 5] = [
        Self::Trace,
        Self::Debug,
        Self::Info,
        Self::Warn,
        Self::Error,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Trace => "Trace",
            Self::Debug => "Debug",
            Self::Info => "Info",
            Self::Warn => "Warn",
            Self::Error => "Error",
        }
    }

    pub fn rank(self) -> u8 {
        match self {
            Self::Trace => 0,
            Self::Debug => 1,
            Self::Info => 2,
            Self::Warn => 3,
            Self::Error => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogExportOptions {
    pub window_seconds: i64,
    pub min_level: LogExportLevel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogExportPreview {
    pub id: u64,
    pub destination: String,
    pub token_map_path: String,
    pub window_since: String,
    pub window_until: String,
    pub source: String,
    pub lines_read: u64,
    pub lines_kept: u64,
    pub redactions: Vec<(String, u64, u64)>,
    pub samples: Vec<String>,
    pub read_errors: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySnapshot {
    pub id: String,
    pub key_type: u8,
    pub public_key_hex: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasSnapshot {
    pub name: String,
    pub address_hex: String,
    pub kind: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HypergraphKind {
    #[default]
    Vertex,
    Hyperedge,
}

impl HypergraphKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Vertex => "Vertex",
            Self::Hyperedge => "Hyperedge",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HypergraphEntrySnapshot {
    pub key_hex: String,
    pub value_hex: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HypergraphSnapshot {
    pub kind: HypergraphKind,
    pub address_hex: String,
    pub set_type: String,
    pub phase_type: String,
    pub shard_l1_hex: String,
    pub shard_l2_hex: String,
    pub present: Option<bool>,
    pub entries: Vec<HypergraphEntrySnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardInfoSnapshot {
    pub include_all: bool,
    pub frame_number: u64,
    pub difficulty: u64,
    pub world_state: String,
    pub pomw_basis: String,
    pub allocation_scoring_strategy: String,
    pub directory_collected_at_unix_seconds: Option<u64>,
    pub directory_stale: bool,
    pub directory_error: Option<String>,
    pub directory_invalid_rows: usize,
    pub network_provers: Option<u32>,
    pub summary: ShardSummarySnapshot,
    pub shards: Vec<ShardSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShardSummarySnapshot {
    pub total: usize,
    pub allocated: usize,
    pub ring_unknown: usize,
    pub active_provers: u64,
    pub data_shards: u64,
    pub theoretical_coverage: usize,
    pub theoretical_daily_quil: Option<String>,
    pub hour_coverage: usize,
    pub hour_daily_quil: Option<String>,
    pub day_coverage: usize,
    pub day_daily_quil: Option<String>,
    pub seat_concentration: Option<SeatConcentrationSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatConcentrationSnapshot {
    pub at_risk: usize,
    pub reported: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardSnapshot {
    pub filter_hex: String,
    pub active_provers: u32,
    pub ring: Option<u32>,
    pub ring_estimated: bool,
    pub official: bool,
    pub shard_size: String,
    pub data_shards: u64,
    pub is_allocated: bool,
    pub materialized_frame: u64,
    pub latest_frame: u64,
    pub allocation_reason: Option<String>,
    pub theoretical_daily_quil: Option<String>,
    pub hour_daily_quil: Option<String>,
    pub day_daily_quil: Option<String>,
    pub prover_counts: Option<ShardProverCountsSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardProverCountsSnapshot {
    pub joining: u32,
    pub activating: u32,
    pub active: u32,
    pub paused: u32,
    pub leaving: u32,
    pub departing: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveConsensus {
    AllAgree,
    Majority,
    NoMajority,
    NoAnswers,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveShardSnapshot {
    pub summary: ArchiveConsensus,
    pub total: usize,
    pub answered: usize,
    pub archives: Vec<ArchiveEndpointSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEndpointSnapshot {
    pub endpoint: String,
    pub shard_count: Option<usize>,
    pub frozen_count: Option<usize>,
    pub digest: Option<String>,
    pub status: String,
    pub extra_filters: Vec<String>,
    pub missing_filters: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RewardIdentity {
    Standard,
    Alternate { key_id: String },
    Unavailable,
    Ambiguous { candidates: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewardSnapshot {
    pub identity: RewardIdentity,
    pub prover_address_hex: Option<String>,
    pub public_key_hex: Option<String>,
    pub legacy_account_address_hex: Option<String>,
    pub recipient_address_hex: Option<String>,
    pub claimable_base_units: Option<u128>,
    pub cited_frame: Option<u64>,
    pub proof_verified: bool,
    pub witness_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RewardMintPhase {
    Preflighting,
    ReadyToConfirm,
    ProvingAuthorization,
    AuthorizationPrepared,
    SubmittingAuthorization,
    AwaitingAuthorizationWitness,
    ProvingClaim,
    ClaimPrepared,
    SubmittingClaim,
    ClaimSubmitted,
    ClaimAccepted,
    ClaimRejected,
    ClaimConfirmed,
    AuthorizationUnknown,
    ClaimUnknown,
    RecoverableError,
    PermanentError,
}

impl RewardMintPhase {
    pub fn is_complete(self) -> bool {
        self == Self::ClaimConfirmed
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::ClaimConfirmed | Self::PermanentError)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewardMintPreflight {
    pub confirmation_id: [u8; 32],
    pub network: String,
    pub config_dir: std::path::PathBuf,
    pub recipient: String,
    pub reward_amount: u128,
    pub estimated_fee: u128,
    pub net_amount: u128,
    pub cited_global_frame: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewardMintStatus {
    pub receipt: String,
    pub recipient: String,
    pub phase: RewardMintPhase,
    pub resume_phase: Option<RewardMintPhase>,
    pub detail: Option<String>,
    pub next_poll_in_secs: Option<u64>,
    pub can_rebroadcast: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyCoinSnapshot {
    pub address_hex: String,
    pub amount_base_units: u128,
    pub shielded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LegacyAssetsSnapshot {
    pub identity_available: bool,
    pub coins: Vec<LegacyCoinSnapshot>,
    pub unshielded_total_base_units: u128,
    pub unshielded_count: usize,
    pub shielded_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfidentialCoinSnapshot {
    pub address_hex: String,
    pub amount_base_units: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfidentialWalletSnapshot {
    pub address_hex: String,
    pub total_base_units: u128,
    pub coins: Vec<ConfidentialCoinSnapshot>,
    pub recent_operations: Vec<WalletOperationSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalletAction {
    Transfer,
    PendingTransfer,
    Accept,
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalletAmount(u128);

impl WalletAmount {
    pub fn parse(value: &str) -> Result<Self, String> {
        let value = value
            .trim()
            .parse::<u128>()
            .map_err(|error| format!("Amount must be a base-unit integer: {error}"))?;
        if value == 0 {
            return Err("Amount must be greater than zero".into());
        }
        Ok(Self(value))
    }

    pub fn base_units(self) -> u128 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletActionRequest {
    pub action: WalletAction,
    pub application_hex: String,
    pub target: String,
    pub amount: Option<WalletAmount>,
    pub explicit_fee_base_units: Option<u128>,
    pub maximum_fee_base_units: u128,
    pub refund_after_global_frame: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletPreflight {
    pub action: WalletAction,
    pub application_hex: String,
    pub target: String,
    pub amount_base_units: u128,
    pub fee_base_units: u128,
    pub maximum_fee_base_units: u128,
    pub change_base_units: u128,
    pub input_count: usize,
    pub refund_after_global_frame: Option<u64>,
    pub approval_id: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalletOperationPhase {
    Preflighting,
    Proving,
    Submitting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalletSubmissionOutcome {
    Submitted,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalletOperationStatus {
    Submitting,
    Submitted,
    Unknown,
    Committed,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletOperationSnapshot {
    pub operation_id: String,
    pub action: WalletAction,
    pub application_hex: String,
    pub phase: WalletOperationStatus,
    pub submitted_unix_ms: Option<u64>,
    pub detail: Option<String>,
    pub console_version: String,
    pub build_commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletOperationResult {
    pub action: WalletAction,
    pub outcome: WalletSubmissionOutcome,
    pub message: String,
    pub operation: WalletOperationSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingEscrowBranch {
    Recipient,
    Refund,
}

impl PendingEscrowBranch {
    pub fn label(self) -> &'static str {
        match self {
            Self::Recipient => "Receive",
            Self::Refund => "Refund",
        }
    }

    pub fn localized_label(self, locale: crate::i18n::Locale) -> &'static str {
        match self {
            Self::Recipient => crate::i18n::t(locale, UiText::Receive),
            Self::Refund => crate::i18n::t(locale, UiText::Refund),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingEscrowSnapshot {
    pub address_hex: String,
    pub amount_base_units: u128,
    pub branch: PendingEscrowBranch,
    pub refund_after_global_frame: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PendingEscrowsSnapshot {
    pub identity_available: bool,
    pub unavailable_reason: Option<String>,
    pub escrows: Vec<PendingEscrowSnapshot>,
    pub recent_operations: Vec<WalletOperationSnapshot>,
}

pub const QUIL_BASE_UNITS: u128 = 8_000_000_000;

pub fn format_quil_amount(value: u128) -> String {
    const SCALE: u128 = 1_000_000_000_000;
    let mut whole = value / QUIL_BASE_UNITS;
    let remainder = value % QUIL_BASE_UNITS;
    let mut fractional = (remainder * SCALE + QUIL_BASE_UNITS / 2) / QUIL_BASE_UNITS;
    if fractional == SCALE {
        whole += 1;
        fractional = 0;
    }
    format!("{whole}.{fractional:012}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AliasMutation {
    Put {
        name: String,
        address_hex: String,
        kind: String,
    },
    Delete {
        name: String,
    },
}

impl AliasMutation {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Put { .. } => "Add or update alias",
            Self::Delete { .. } => "Delete alias",
        }
    }

    pub fn localized_label(&self, locale: crate::i18n::Locale) -> &'static str {
        match self {
            Self::Put { .. } => crate::i18n::t(locale, UiText::AddOrUpdateAliasAction),
            Self::Delete { .. } => crate::i18n::t(locale, UiText::DeleteAlias),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Put { name, .. } | Self::Delete { name } => name,
        }
    }
}

impl KeySnapshot {
    pub fn type_name(&self) -> &'static str {
        match self.key_type {
            0 => "Ed448",
            1 => "X448",
            2 => "BLS48-581 G1",
            3 => "BLS48-581 G2",
            4 => "Decaf448",
            5 => "secp256k1/SHA-256",
            6 => "secp256k1/SHA-3",
            7 => "Ed25519",
            8 => "Falcon-512",
            9 => "sntrup761",
            _ => "Unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyMutation {
    Create { id: String, key_type: u8 },
    Delete { id: String },
}

impl KeyMutation {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Create { .. } => "Create key",
            Self::Delete { .. } => "Delete key",
        }
    }

    pub fn localized_label(&self, locale: crate::i18n::Locale) -> &'static str {
        match self {
            Self::Create { .. } => crate::i18n::t(locale, UiText::CreateKeyAction),
            Self::Delete { .. } => crate::i18n::t(locale, UiText::DeleteKey),
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Create { id, .. } | Self::Delete { id } => id,
        }
    }
}

pub const CREATABLE_KEY_TYPES: &[(u8, &str)] = &[
    (0, "Ed448"),
    (1, "X448"),
    (2, "BLS48-581 G1"),
    (4, "Decaf448"),
    (8, "Falcon-512"),
];

pub fn protected_key_id(id: &str) -> bool {
    id.starts_with("q-")
}

impl ProverAction {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Join { .. } => "Join shard",
            Self::Leave { .. } => "Leave shard",
            Self::Confirm { .. } => "Confirm proposal",
            Self::Reject { .. } => "Reject proposal",
            Self::Pause { .. } => "Pause shard",
            Self::Resume { .. } => "Resume shard",
        }
    }

    pub fn localized_label(&self, locale: crate::i18n::Locale) -> &'static str {
        match self {
            Self::Join { .. } => crate::i18n::t(locale, UiText::JoinShard),
            Self::Leave { .. } => crate::i18n::t(locale, UiText::LeaveShard),
            Self::Confirm { .. } => crate::i18n::t(locale, UiText::ConfirmProposal),
            Self::Reject { .. } => crate::i18n::t(locale, UiText::RejectProposal),
            Self::Pause { .. } => crate::i18n::t(locale, UiText::PauseShard),
            Self::Resume { .. } => crate::i18n::t(locale, UiText::ResumeShard),
        }
    }

    pub fn filters(&self) -> &[Vec<u8>] {
        match self {
            Self::Join { filters, .. }
            | Self::Leave { filters }
            | Self::Confirm { filters }
            | Self::Reject { filters } => filters,
            Self::Pause { filter } | Self::Resume { filter } => std::slice::from_ref(filter),
        }
    }
}

#[cfg(test)]
mod reward_tests {
    use super::*;

    #[test]
    fn formats_official_quil_base_units_without_floating_point() {
        assert_eq!(format_quil_amount(0), "0.000000000000");
        assert_eq!(format_quil_amount(1), "0.000000000125");
        assert_eq!(format_quil_amount(4_000_000_000), "0.500000000000");
        assert_eq!(format_quil_amount(8_000_000_000), "1.000000000000");
    }
}

#[cfg(test)]
mod pending_escrow_tests {
    use super::*;

    #[test]
    fn labels_pending_escrow_branches_for_people() {
        assert_eq!(PendingEscrowBranch::Recipient.label(), "Receive");
        assert_eq!(PendingEscrowBranch::Refund.label(), "Refund");
    }
}

#[cfg(test)]
mod wallet_tests {
    use super::*;

    #[test]
    fn wallet_amount_rejects_zero_and_non_decimal_input() {
        assert_eq!(WalletAmount::parse("1").unwrap().base_units(), 1);
        assert!(WalletAmount::parse("0").is_err());
        assert!(WalletAmount::parse("1.5").is_err());
        assert!(WalletAmount::parse("-1").is_err());
    }

    #[test]
    fn wallet_confirmation_keeps_the_approved_fee_ceiling() {
        let summary = WalletPreflight {
            action: WalletAction::Transfer,
            application_hex: hex::encode(quil_execution::domains::QUIL_TOKEN),
            target: "qct3-recipient".into(),
            amount_base_units: 8_000_000_000,
            fee_base_units: 100,
            maximum_fee_base_units: 120,
            change_base_units: 50,
            input_count: 1,
            refund_after_global_frame: None,
            approval_id: [7; 32],
        };

        assert_eq!(summary.action, WalletAction::Transfer);
        assert!(summary.maximum_fee_base_units >= summary.fee_base_units);
    }
}

impl WorkerSnapshot {
    pub fn state(&self) -> &str {
        self.execution
            .as_ref()
            .map(|execution| execution.state.as_str())
            .filter(|state| !state.is_empty())
            .unwrap_or("Not observed")
    }

    pub fn materialized_frame(&self) -> Option<u64> {
        self.execution
            .as_ref()
            .and_then(|execution| execution.materialized_frame)
    }
}

impl SessionSealSnapshot {
    pub fn is_recorded(&self) -> bool {
        self.recorded_unix_ms > 0
    }
}

impl NodeSnapshot {
    pub fn display_version(&self) -> String {
        let mut parts = self
            .version
            .iter()
            .take(3)
            .map(u8::to_string)
            .collect::<Vec<_>>();
        if let Some(patch) = self.patch_number.first() {
            parts.push(patch.to_string());
        }
        if parts.is_empty() {
            "Unknown".into()
        } else {
            parts.join(".")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reward_mint_phase_only_marks_confirmed_claim_as_complete() {
        assert!(!RewardMintPhase::AwaitingAuthorizationWitness.is_complete());
        assert!(!RewardMintPhase::ClaimSubmitted.is_complete());
        assert!(!RewardMintPhase::ClaimUnknown.is_complete());
        assert!(RewardMintPhase::ClaimConfirmed.is_complete());
    }

    #[test]
    fn reward_mint_terminal_admission_only_releases_confirmed_or_permanent_journals() {
        assert!(RewardMintPhase::ClaimConfirmed.is_terminal());
        assert!(RewardMintPhase::PermanentError.is_terminal());
        assert!(!RewardMintPhase::ClaimRejected.is_terminal());
        assert!(!RewardMintPhase::AuthorizationUnknown.is_terminal());
        assert!(!RewardMintPhase::ClaimUnknown.is_terminal());
    }

    #[test]
    fn reward_mint_status_preserves_receipt_recipient_and_resume_phase() {
        let status = RewardMintStatus {
            receipt: "receipt-01".into(),
            recipient: "QCT3-example".into(),
            phase: RewardMintPhase::RecoverableError,
            resume_phase: Some(RewardMintPhase::AwaitingAuthorizationWitness),
            detail: Some("temporarily unavailable".into()),
            next_poll_in_secs: Some(30),
            can_rebroadcast: false,
        };

        assert_eq!(status.receipt, "receipt-01");
        assert_eq!(status.recipient, "QCT3-example");
        assert_eq!(
            status.resume_phase,
            Some(RewardMintPhase::AwaitingAuthorizationWitness)
        );
        assert_eq!(status.next_poll_in_secs, Some(30));
    }

    #[test]
    fn reward_mint_preflight_keeps_the_complete_confirmation_summary() {
        let summary = RewardMintPreflight {
            confirmation_id: [7; 32],
            network: "mainnet".into(),
            config_dir: std::path::PathBuf::from("/node/config"),
            recipient: "QCT3-example".into(),
            reward_amount: 1_000,
            estimated_fee: 125,
            net_amount: 875,
            cited_global_frame: 42,
        };

        assert_eq!(
            summary.reward_amount - summary.estimated_fee,
            summary.net_amount
        );
        assert_eq!(summary.cited_global_frame, 42);
        assert_eq!(summary.config_dir, std::path::Path::new("/node/config"));
    }

    #[test]
    fn formats_release_and_patch() {
        let snapshot = NodeSnapshot {
            version: vec![2, 1, 0],
            patch_number: vec![25],
            ..Default::default()
        };

        assert_eq!(snapshot.display_version(), "2.1.0.25");
    }

    #[test]
    fn maps_known_and_unknown_allocation_statuses() {
        assert_eq!(AllocationStatus::from(2), AllocationStatus::Active);
        assert_eq!(AllocationStatus::from(7), AllocationStatus::Historic);
        assert_eq!(AllocationStatus::from(99), AllocationStatus::Unknown);
    }

    #[test]
    fn session_seal_keeps_recorded_timestamp_without_collapsing_it_to_a_boolean() {
        let seal = SessionSealSnapshot {
            generation: 1,
            checkpoint: 2,
            view: 3,
            global_executed: 4,
            drain: "executed".into(),
            drain_headers: 5,
            finalized_unix_ms: 6,
            last_submitted_unix_ms: 7,
            submissions: 8,
            recorded_unix_ms: 9,
        };

        assert_eq!(seal.recorded_unix_ms, 9);
        assert!(seal.is_recorded());
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LegacyMigrationDestination {
    CurrentNode,
    ExistingConfig(std::path::PathBuf),
    CreateNew,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LegacyIdentitySummary {
    pub fingerprint: String,
    pub masked_key: String,
    pub peer_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LegacyMigrationPackage {
    pub task_dir: std::path::PathBuf,
    pub recipient_config_dir: std::path::PathBuf,
    pub config_path: std::path::PathBuf,
    pub keys_path: std::path::PathBuf,
    pub generated: bool,
    pub recipient_address: String,
}

mod serde_u128_string {
    use serde::{de::Error, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u128, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(D::Error::custom)
    }
}

mod serde_u128_string_vec {
    use serde::{de::Error, Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(values: &[u128], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        values
            .iter()
            .map(u128::to_string)
            .collect::<Vec<_>>()
            .serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u128>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Vec::<String>::deserialize(deserializer)?
            .into_iter()
            .map(|value| value.parse().map_err(D::Error::custom))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LegacyMigrationPreflight {
    pub approval_id: String,
    pub source: LegacyIdentitySummary,
    pub source_count: usize,
    #[serde(with = "serde_u128_string")]
    pub source_amount: u128,
    pub batch_count: usize,
    pub destination: LegacyMigrationDestination,
    pub recipient_config_dir: std::path::PathBuf,
    pub config_path: std::path::PathBuf,
    pub keys_path: std::path::PathBuf,
    pub recipient_address: String,
    pub active_config_dir: std::path::PathBuf,
    pub read_endpoint: String,
    pub submit_endpoint: String,
    pub running_peer_id: String,
    #[serde(with = "serde_u128_string_vec")]
    pub fees: Vec<u128>,
    #[serde(with = "serde_u128_string")]
    pub estimated_net_amount: u128,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyMigrationPhase {
    Prepared,
    Migrating { batch: usize, total: usize },
    AwaitingChainConfirmation { batch: usize, total: usize },
    Complete,
    RecoverableError,
    SubmissionUnknown,
    AwaitingSourceKey,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LegacyMigrationBatchRecord {
    pub source_addresses: Vec<String>,
    pub shard: String,
    pub fee: u128,
    pub operation_file: Option<String>,
    pub operation_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LegacyMigrationStatus {
    pub schema_version: u32,
    pub source: LegacyIdentitySummary,
    pub task_dir: std::path::PathBuf,
    pub active_config_dir: std::path::PathBuf,
    pub recipient_config_dir: std::path::PathBuf,
    pub config_path: std::path::PathBuf,
    pub keys_path: std::path::PathBuf,
    pub source_key_persisted: bool,
    pub read_endpoint: String,
    pub submit_endpoint: String,
    pub running_peer_id: String,
    pub recipient_address: String,
    pub batches: Vec<LegacyMigrationBatchRecord>,
    pub current_batch: usize,
    pub phase: LegacyMigrationPhase,
    pub resume_phase: Option<LegacyMigrationPhase>,
    pub scan_cursor: Option<String>,
    pub console_version: String,
    pub build_commit: String,
    pub created_unix_ms: u64,
    pub updated_unix_ms: u64,
    pub error: Option<String>,
}
