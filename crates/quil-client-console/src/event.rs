use crate::domain::{
    AliasSnapshot, ArchiveShardSnapshot, ConfidentialWalletSnapshot, DiagnosticsIdentitySnapshot,
    DiagnosticsReport, HealthSnapshot, HypergraphSnapshot, KeySnapshot, LegacyAssetsSnapshot,
    LegacyMigrationPreflight, LegacyMigrationStatus, LogExportPreview, LogSnapshot,
    OperationResult, PeerAddressReport, PendingEscrowsSnapshot, RewardMintPreflight,
    RewardMintStatus, RewardSnapshot, RuntimeSnapshot, ShardInfoSnapshot, WalletOperationResult,
    WalletPreflight,
};

#[derive(Clone, PartialEq, Eq)]
pub struct SensitiveFilePayload(zeroize::Zeroizing<String>);

impl SensitiveFilePayload {
    pub fn new(value: String) -> Self {
        Self(zeroize::Zeroizing::new(value))
    }

    pub fn into_string(mut self) -> String {
        std::mem::take(&mut *self.0)
    }
}

impl std::fmt::Debug for SensitiveFilePayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SensitiveFilePayload(<redacted>)")
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct SensitiveTaskRedactions(Vec<zeroize::Zeroizing<String>>);

impl SensitiveTaskRedactions {
    pub fn new(values: Vec<String>) -> Self {
        Self::from_zeroizing(
            values
                .into_iter()
                .filter(|value| !value.is_empty())
                .map(zeroize::Zeroizing::new)
                .collect(),
        )
    }

    pub(crate) fn from_zeroizing(mut values: Vec<zeroize::Zeroizing<String>>) -> Self {
        values.retain(|value| !value.is_empty());
        values.sort_by(|left, right| {
            right
                .len()
                .cmp(&left.len())
                .then_with(|| left.as_str().cmp(right.as_str()))
        });
        values.dedup_by(|left, right| left.as_str() == right.as_str());
        Self(values)
    }

    pub fn redact(&self, text: &str) -> String {
        self.0.iter().fold(text.to_owned(), |redacted, value| {
            if value.is_empty() {
                redacted
            } else {
                redacted.replace(value.as_str(), "[redacted]")
            }
        })
    }

    pub fn redact_task(
        &self,
        mut task: crate::capability::CapabilityTask,
    ) -> crate::capability::CapabilityTask {
        task.stdout = self.redact(&task.stdout);
        task.stderr = self.redact(&task.stderr);
        task
    }
}

impl std::fmt::Debug for SensitiveTaskRedactions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SensitiveTaskRedactions")
            .field("value_count", &self.0.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct CapabilityTaskCompletion {
    task: crate::capability::CapabilityTask,
    redactions: SensitiveTaskRedactions,
}

impl CapabilityTaskCompletion {
    pub fn new(
        task: crate::capability::CapabilityTask,
        redactions: SensitiveTaskRedactions,
    ) -> Self {
        Self { task, redactions }
    }

    pub fn into_display_task(self) -> crate::capability::CapabilityTask {
        self.redactions.redact_task(self.task)
    }
}

impl std::fmt::Debug for CapabilityTaskCompletion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CapabilityTaskCompletion")
            .field("id", &self.task.id)
            .field("capability_id", &self.task.capability_id)
            .field("state", &self.task.state)
            .field("exit_code", &self.task.exit_code)
            .field("output", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    Up,
    Down,
    PageUp,
    PageDown,
    Left,
    Right,
    Enter,
    Character(char),
    Backspace,
    BackTab,
    Back,
    OpenConfigPath,
    Refresh,
    Help,
    Quit,
    Resize(u16, u16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceEvent {
    CapabilityTaskStarted {
        request_id: u64,
        capability_id: String,
        result: Result<crate::capability::CapabilityTask, String>,
    },
    CapabilityTaskFinished {
        request_id: u64,
        completion: CapabilityTaskCompletion,
    },
    RefreshFinished {
        generation: u64,
        result: Result<(), String>,
    },
    ConfigActivationFinished {
        generation: u64,
        result: Result<(), String>,
        persistence_error: Option<String>,
    },
    OperationFinished {
        generation: u64,
        result: Result<OperationResult, String>,
    },
    ProverOperationFinished {
        id: u64,
        result: Result<OperationResult, String>,
    },
    DiagnosticsFinished {
        generation: u64,
        result: Result<DiagnosticsReport, String>,
    },
    DiagnosticsIdentityFinished {
        generation: u64,
        result: Result<DiagnosticsIdentitySnapshot, String>,
    },
    PeerResolveFinished {
        generation: u64,
        result: Result<PeerAddressReport, String>,
    },
    HealthFinished {
        generation: u64,
        result: Result<HealthSnapshot, String>,
    },
    RuntimeFinished {
        generation: u64,
        result: Result<RuntimeSnapshot, String>,
    },
    LogFinished {
        generation: u64,
        result: Result<LogSnapshot, String>,
    },
    LogExportPrepared {
        generation: u64,
        result: Result<LogExportPreview, String>,
    },
    LogExportFinished {
        generation: u64,
        committed: bool,
        result: Result<OperationResult, String>,
    },
    KeysFinished {
        generation: u64,
        result: Result<Vec<KeySnapshot>, String>,
    },
    AliasesFinished {
        generation: u64,
        result: Result<Vec<AliasSnapshot>, String>,
    },
    RewardsFinished {
        generation: u64,
        result: Result<RewardSnapshot, String>,
    },
    RewardMintPreflightFinished {
        generation: u64,
        result: Result<RewardMintPreflight, String>,
    },
    RewardMintStarted {
        generation: u64,
        result: Result<RewardMintStatus, String>,
    },
    RewardMintLoaded {
        generation: u64,
        result: Result<Option<RewardMintStatus>, String>,
    },
    RewardMintAdvanced {
        generation: u64,
        result: Result<RewardMintStatus, String>,
    },
    RewardMintRebroadcasted {
        generation: u64,
        result: Result<RewardMintStatus, String>,
    },
    LegacyAssetsFinished {
        generation: u64,
        result: Result<LegacyAssetsSnapshot, String>,
    },
    LegacyMigrationLoaded {
        generation: u64,
        result: Result<Option<LegacyMigrationStatus>, String>,
    },
    LegacyMigrationPreflightFinished {
        generation: u64,
        result: Result<LegacyMigrationPreflight, String>,
    },
    LegacyMigrationStarted {
        generation: u64,
        result: Result<LegacyMigrationStatus, String>,
    },
    LegacyMigrationAdvanced {
        generation: u64,
        result: Result<LegacyMigrationStatus, String>,
    },
    LegacyMigrationRebroadcasted {
        generation: u64,
        result: Result<LegacyMigrationStatus, String>,
    },
    LegacyMigrationFileLoaded {
        generation: u64,
        kind: crate::app::LegacyMigrationFileKind,
        result: Result<SensitiveFilePayload, String>,
    },
    ConfidentialWalletFinished {
        generation: u64,
        result: Result<ConfidentialWalletSnapshot, String>,
    },
    PendingEscrowsFinished {
        generation: u64,
        result: Result<PendingEscrowsSnapshot, String>,
    },
    WalletPreflightFinished {
        generation: u64,
        result: Result<WalletPreflight, String>,
    },
    WalletExecutionFinished {
        generation: u64,
        result: Result<WalletOperationResult, String>,
    },
    HypergraphFinished {
        generation: u64,
        result: Result<HypergraphSnapshot, String>,
    },
    ShardsFinished {
        generation: u64,
        result: Result<ShardInfoSnapshot, String>,
    },
    ArchiveShardsFinished {
        generation: u64,
        result: Result<ArchiveShardSnapshot, String>,
    },
    AliasMutationFinished {
        generation: u64,
        result: Result<OperationResult, String>,
    },
    KeyMutationFinished {
        generation: u64,
        result: Result<OperationResult, String>,
    },
}
