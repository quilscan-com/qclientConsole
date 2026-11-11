use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use tonic::transport::Channel;

use crate::domain::{WalletAction, WalletOperationSnapshot, WalletOperationStatus};

use super::{decision::OperationDecision, ServiceError};

const JOURNAL_VERSION: u32 = 1;
const RECENT_OPERATION_LIMIT: usize = 20;
const SUBMISSION_RECOVERY_AFTER_MS: u64 = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum OperationJournalPhase {
    Submitting,
    Submitted,
    Unknown,
    Committed,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct OperationJournalState {
    pub version: u32,
    pub action: String,
    pub application: String,
    #[serde(rename = "txid")]
    pub operation_id: String,
    pub phase: OperationJournalPhase,
    pub submitted_unix_ms: Option<u64>,
    pub detail: Option<String>,
    pub console_version: String,
    pub build_commit: String,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum OperationJournalError {
    #[error("wallet operation journal I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("cannot encode wallet operation journal: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("cannot decode wallet operation journal: {0}")]
    Decode(#[source] serde_json::Error),
    #[error("wallet operation journal integrity failed: {0}")]
    Integrity(String),
    #[error(
        "wallet operation {operation_id} is already tracked; refresh its GLOBAL decision instead of resubmitting"
    )]
    AlreadyTracked { operation_id: String },
    #[error(
        "wallet operation journal {operation_id} stopped before dispatch metadata was complete; rerun the exact operation to finish initialization"
    )]
    IncompleteInitialization { operation_id: String },
}

pub(super) struct OperationJournal {
    dir: PathBuf,
}

impl OperationJournal {
    pub(super) fn create(
        config_dir: &Path,
        action: WalletAction,
        application: [u8; 32],
        operation: &[u8],
    ) -> Result<Self, OperationJournalError> {
        let operation_id = hex::encode(quil_execution::token_intrinsic::global_commit::tx_id(
            operation,
        ));
        let console = config_dir.join(".qclient-console");
        let operations = console.join("operations");
        let dir = operations.join(&operation_id);
        create_private_dir(&console)?;
        create_private_dir(&operations)?;
        let journal = Self { dir };
        let _created = create_exclusive_private_dir(&journal.dir)?;
        if journal.dir.join("state.json").try_exists()? {
            let state = journal.read_state()?;
            journal.validate_state(&state)?;
            validate_metadata(&state, action, &application)?;
            return Err(OperationJournalError::AlreadyTracked { operation_id });
        }
        let operation_path = journal.dir.join("operation.bin");
        if operation_path.try_exists()? {
            if std::fs::read(&operation_path)? != operation {
                return Err(OperationJournalError::Integrity(
                    "incomplete journal operation.bin differs from the requested payload".into(),
                ));
            }
        } else if !atomic_write_new(&journal.dir, "operation.bin", operation)?
            && std::fs::read(&operation_path)? != operation
        {
            return Err(OperationJournalError::Integrity(
                "concurrent journal operation.bin differs from the requested payload".into(),
            ));
        }
        let state = OperationJournalState {
            version: JOURNAL_VERSION,
            action: action_name(action).into(),
            application: hex::encode(application),
            operation_id: operation_id.clone(),
            phase: OperationJournalPhase::Submitting,
            submitted_unix_ms: Some(now_millis()),
            detail: None,
            console_version: env!("CARGO_PKG_VERSION").into(),
            build_commit: option_env!("QCLIENT_CONSOLE_BUILD_COMMIT")
                .unwrap_or("unknown")
                .into(),
        };
        if !journal.write_initial_state(&state)? {
            let existing = journal.read_state()?;
            journal.validate_state(&existing)?;
            validate_metadata(&existing, action, &application)?;
            return Err(OperationJournalError::AlreadyTracked { operation_id });
        }
        Ok(journal)
    }

    pub(super) fn record_submission(
        &self,
        phase: OperationJournalPhase,
        detail: Option<String>,
    ) -> Result<OperationJournalState, OperationJournalError> {
        let mut state = self.read_state()?;
        state.phase = phase;
        state.detail = detail;
        state.submitted_unix_ms.get_or_insert_with(now_millis);
        self.write_state(&state)?;
        Ok(state)
    }

    pub(super) fn record_decision(
        &self,
        decision: OperationDecision,
    ) -> Result<OperationJournalState, OperationJournalError> {
        let (phase, detail) = match decision {
            OperationDecision::Committed { frame } => (
                OperationJournalPhase::Committed,
                format!("Committed at GLOBAL frame {frame}"),
            ),
            OperationDecision::Rejected { frame, reason } => (
                OperationJournalPhase::Rejected,
                format!("Rejected at GLOBAL frame {frame}: {reason}"),
            ),
        };
        self.record_submission(phase, Some(detail))
    }

    pub(super) fn read_operation(&self) -> Result<Vec<u8>, OperationJournalError> {
        Ok(std::fs::read(self.dir.join("operation.bin"))?)
    }

    pub(super) fn scan(
        config_dir: &Path,
    ) -> Result<Vec<OperationJournalState>, OperationJournalError> {
        let root = config_dir.join(".qclient-console/operations");
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut entries = std::fs::read_dir(root)?
            .collect::<Result<Vec<_>, std::io::Error>>()?
            .into_iter()
            .filter(|entry| entry.path().is_dir())
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        let mut states = Vec::with_capacity(entries.len());
        for entry in entries {
            let journal = Self { dir: entry.path() };
            if !journal.dir.join("state.json").is_file() {
                // `state.json` is durably written before `create()` returns and before dispatch.
                // A directory without it is therefore provably pre-dispatch and must not block
                // recovery of unrelated operations.
                continue;
            }
            let mut state = journal.read_state()?;
            journal.validate_state(&state)?;
            if state.phase == OperationJournalPhase::Submitting
                && state.submitted_unix_ms.is_none_or(|submitted| {
                    now_millis().saturating_sub(submitted) >= SUBMISSION_RECOVERY_AFTER_MS
                })
            {
                state.phase = OperationJournalPhase::Unknown;
                state.detail = Some(
                    "Console stopped during submission; the write will not be replayed".into(),
                );
                journal.write_state(&state)?;
            }
            states.push(state);
        }
        Ok(states)
    }

    #[cfg(test)]
    pub(super) fn recent(
        config_dir: &Path,
    ) -> Result<Vec<WalletOperationSnapshot>, OperationJournalError> {
        Self::recent_states(config_dir)?
            .into_iter()
            .map(OperationJournalState::into_snapshot)
            .collect()
    }

    pub(super) fn load_snapshot(
        config_dir: &Path,
        operation_id: &str,
    ) -> Result<WalletOperationSnapshot, OperationJournalError> {
        let decoded = hex::decode(operation_id)
            .map_err(|_| OperationJournalError::Integrity("txid is not hexadecimal".into()))?;
        if decoded.len() != 32 || hex::encode(decoded) != operation_id {
            return Err(OperationJournalError::Integrity(
                "txid is not canonical 32-byte lowercase hex".into(),
            ));
        }
        let journal = Self {
            dir: config_dir
                .join(".qclient-console/operations")
                .join(operation_id),
        };
        let state = journal.read_state()?;
        journal.validate_state(&state)?;
        state.into_snapshot()
    }

    pub(super) async fn recover(
        config_dir: &Path,
        client: &quil_types::proto::node::node_service_client::NodeServiceClient<Channel>,
    ) -> Result<Vec<WalletOperationSnapshot>, ServiceError> {
        use quil_execution::token_intrinsic::global_commit::decision_address;

        Self::recover_with_observer(config_dir, move |application, txid| {
            let client = client;
            async move {
                let record =
                    decision_address(&application, &txid).map_err(|error| error.to_string())?;
                let (present, raw_data) = super::read_global_record(client, &record)
                    .await
                    .map_err(|error| error.to_string())?;
                if !present {
                    return Ok(None);
                }
                super::decision::parse_record(&raw_data)
                    .map(Some)
                    .map_err(|error| error.to_string())
            }
        })
        .await
    }

    async fn recover_with_observer<F, Fut>(
        config_dir: &Path,
        mut observe: F,
    ) -> Result<Vec<WalletOperationSnapshot>, ServiceError>
    where
        F: FnMut([u8; 32], [u8; 32]) -> Fut,
        Fut: std::future::Future<Output = Result<Option<OperationDecision>, String>>,
    {
        let mut states = Self::scan(config_dir).map_err(journal_service_error)?;
        for state in &mut states {
            if !matches!(
                state.phase,
                OperationJournalPhase::Submitted | OperationJournalPhase::Unknown
            ) {
                continue;
            }
            let application: [u8; 32] = hex::decode(&state.application)
                .map_err(|_| ServiceError::Wallet("Invalid journal application".into()))?
                .try_into()
                .map_err(|_| ServiceError::Wallet("Invalid journal application length".into()))?;
            let txid: [u8; 32] = hex::decode(&state.operation_id)
                .map_err(|_| ServiceError::Wallet("Invalid journal txid".into()))?
                .try_into()
                .map_err(|_| ServiceError::Wallet("Invalid journal txid length".into()))?;
            let decision = match observe(application, txid).await {
                Ok(Some(decision)) => decision,
                Ok(None) => continue,
                Err(error) => {
                    state.detail = Some(format!(
                        "GLOBAL decision refresh failed; no write was retried: {error}"
                    ));
                    continue;
                }
            };
            let journal = Self {
                dir: config_dir
                    .join(".qclient-console/operations")
                    .join(&state.operation_id),
            };
            *state = journal
                .record_decision(decision)
                .map_err(journal_service_error)?;
        }
        Self::limit_states(&mut states);
        states
            .into_iter()
            .map(OperationJournalState::into_snapshot)
            .collect::<Result<Vec<_>, _>>()
            .map_err(journal_service_error)
    }

    #[cfg(test)]
    fn recent_states(
        config_dir: &Path,
    ) -> Result<Vec<OperationJournalState>, OperationJournalError> {
        let mut states = Self::scan(config_dir)?;
        Self::limit_states(&mut states);
        Ok(states)
    }

    fn limit_states(states: &mut Vec<OperationJournalState>) {
        states.sort_by(|left, right| {
            right
                .submitted_unix_ms
                .cmp(&left.submitted_unix_ms)
                .then_with(|| right.operation_id.cmp(&left.operation_id))
        });
        states.truncate(RECENT_OPERATION_LIMIT);
    }

    fn read_state(&self) -> Result<OperationJournalState, OperationJournalError> {
        let bytes = match std::fs::read(self.dir.join("state.json")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(OperationJournalError::IncompleteInitialization {
                    operation_id: self
                        .dir
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("unknown")
                        .to_owned(),
                })
            }
            Err(error) => return Err(OperationJournalError::Io(error)),
        };
        serde_json::from_slice(&bytes).map_err(OperationJournalError::Decode)
    }

    fn validate_state(&self, state: &OperationJournalState) -> Result<(), OperationJournalError> {
        if state.version != JOURNAL_VERSION {
            return Err(OperationJournalError::Integrity(format!(
                "unsupported version {}",
                state.version
            )));
        }
        let directory_txid = self
            .dir
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| OperationJournalError::Integrity("non-UTF-8 txid directory".into()))?;
        if directory_txid != state.operation_id {
            return Err(OperationJournalError::Integrity(
                "state txid does not match its directory".into(),
            ));
        }
        let txid = hex::decode(&state.operation_id)
            .map_err(|_| OperationJournalError::Integrity("txid is not hexadecimal".into()))?;
        if txid.len() != 32 {
            return Err(OperationJournalError::Integrity(
                "txid is not 32 bytes".into(),
            ));
        }
        let application = hex::decode(&state.application).map_err(|_| {
            OperationJournalError::Integrity("application is not hexadecimal".into())
        })?;
        if application.len() != 32 {
            return Err(OperationJournalError::Integrity(
                "application is not 32 bytes".into(),
            ));
        }
        if !matches!(
            state.action.as_str(),
            "transfer" | "pending_transfer" | "accept" | "reject"
        ) {
            return Err(OperationJournalError::Integrity(
                "unknown wallet action".into(),
            ));
        }
        let actual = hex::encode(quil_execution::token_intrinsic::global_commit::tx_id(
            &self.read_operation()?,
        ));
        if actual != state.operation_id {
            return Err(OperationJournalError::Integrity(
                "operation.bin does not match its txid".into(),
            ));
        }
        Ok(())
    }

    fn write_state(&self, state: &OperationJournalState) -> Result<(), OperationJournalError> {
        let bytes = serde_json::to_vec_pretty(state).map_err(OperationJournalError::Encode)?;
        atomic_write(&self.dir, "state.json", &bytes)?;
        Ok(())
    }

    fn write_initial_state(
        &self,
        state: &OperationJournalState,
    ) -> Result<bool, OperationJournalError> {
        let bytes = serde_json::to_vec_pretty(state).map_err(OperationJournalError::Encode)?;
        atomic_write_new(&self.dir, "state.json", &bytes).map_err(OperationJournalError::Io)
    }
}

fn validate_metadata(
    state: &OperationJournalState,
    action: WalletAction,
    application: &[u8; 32],
) -> Result<(), OperationJournalError> {
    if state.action != action_name(action) || state.application != hex::encode(application) {
        return Err(OperationJournalError::Integrity(
            "existing txid has different action or application metadata".into(),
        ));
    }
    Ok(())
}

fn journal_service_error(error: OperationJournalError) -> ServiceError {
    ServiceError::Wallet(error.to_string())
}

impl OperationJournalState {
    pub(super) fn into_snapshot(self) -> Result<WalletOperationSnapshot, OperationJournalError> {
        let action = match self.action.as_str() {
            "transfer" => WalletAction::Transfer,
            "pending_transfer" => WalletAction::PendingTransfer,
            "accept" => WalletAction::Accept,
            "reject" => WalletAction::Reject,
            _ => {
                return Err(OperationJournalError::Integrity(
                    "unknown wallet action".into(),
                ))
            }
        };
        let phase = match self.phase {
            OperationJournalPhase::Submitting => WalletOperationStatus::Submitting,
            OperationJournalPhase::Submitted => WalletOperationStatus::Submitted,
            OperationJournalPhase::Unknown => WalletOperationStatus::Unknown,
            OperationJournalPhase::Committed => WalletOperationStatus::Committed,
            OperationJournalPhase::Rejected => WalletOperationStatus::Rejected,
        };
        Ok(WalletOperationSnapshot {
            operation_id: self.operation_id,
            action,
            application_hex: self.application,
            phase,
            submitted_unix_ms: self.submitted_unix_ms,
            detail: self.detail,
            console_version: self.console_version,
            build_commit: self.build_commit,
        })
    }
}

fn action_name(action: WalletAction) -> &'static str {
    match action {
        WalletAction::Transfer => "transfer",
        WalletAction::PendingTransfer => "pending_transfer",
        WalletAction::Accept => "accept",
        WalletAction::Reject => "reject",
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn create_private_dir(path: &Path) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(false).mode(0o700);
        let created = match builder.create(path) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(error) => return Err(error),
        };
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
        if created {
            sync_new_directory(path)?;
        }
    }
    #[cfg(not(unix))]
    match std::fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    Ok(())
}

fn create_exclusive_private_dir(path: &Path) -> Result<bool, std::io::Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;

        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(false).mode(0o700);
        match builder.create(path) {
            Ok(()) => {
                sync_new_directory(path)?;
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(error) => Err(error),
        }
    }
    #[cfg(not(unix))]
    match std::fs::create_dir(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn sync_new_directory(path: &Path) -> Result<(), std::io::Error> {
    std::fs::File::open(path)?.sync_all()?;
    if let Some(parent) = path.parent() {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

fn atomic_write(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), std::io::Error> {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    let temp = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let target = dir.join(name);
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, &target)?;
        std::fs::File::open(dir)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}

fn atomic_write_new(dir: &Path, name: &str, bytes: &[u8]) -> Result<bool, std::io::Error> {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    let temp = dir.join(format!(
        ".{name}.{}.{}.new.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let target = dir.join(name);
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        let created = match std::fs::hard_link(&temp, &target) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(error) => return Err(error),
        };
        std::fs::remove_file(&temp)?;
        std::fs::File::open(dir)?.sync_all()?;
        Ok(created)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupted_submission_becomes_unknown_without_replaying_or_rewriting_payload() {
        let config = tempfile::tempdir().unwrap();
        let payload = b"exact operation awaiting an ambiguous send";
        let journal = OperationJournal::create(
            config.path(),
            WalletAction::PendingTransfer,
            quil_execution::domains::QUIL_TOKEN,
            payload,
        )
        .unwrap();
        let mut interrupted = journal.read_state().unwrap();
        interrupted.submitted_unix_ms = Some(0);
        journal.write_state(&interrupted).unwrap();

        let states = OperationJournal::scan(config.path()).unwrap();

        assert_eq!(states.len(), 1);
        assert_eq!(states[0].phase, OperationJournalPhase::Unknown);
        assert!(states[0]
            .detail
            .as_deref()
            .unwrap()
            .contains("will not be replayed"));
        assert_eq!(journal.read_operation().unwrap(), payload);
    }

    #[test]
    fn an_active_submission_is_not_downgraded_by_a_concurrent_read() {
        let config = tempfile::tempdir().unwrap();
        OperationJournal::create(
            config.path(),
            WalletAction::Transfer,
            quil_execution::domains::QUIL_TOKEN,
            b"operation currently being dispatched",
        )
        .unwrap();

        let states = OperationJournal::scan(config.path()).unwrap();

        assert_eq!(states.len(), 1);
        assert_eq!(states[0].phase, OperationJournalPhase::Submitting);
    }

    #[test]
    fn explicit_rejection_and_final_global_decisions_are_durable() {
        let config = tempfile::tempdir().unwrap();
        let rejected = OperationJournal::create(
            config.path(),
            WalletAction::Transfer,
            quil_execution::domains::QUIL_TOKEN,
            b"operation rejected by send RPC",
        )
        .unwrap()
        .record_submission(
            OperationJournalPhase::Rejected,
            Some("Node rejected submission: invalid operation".into()),
        )
        .unwrap();
        assert_eq!(rejected.phase, OperationJournalPhase::Rejected);
        assert_eq!(
            rejected.detail.as_deref(),
            Some("Node rejected submission: invalid operation")
        );

        let committed_journal = OperationJournal::create(
            config.path(),
            WalletAction::PendingTransfer,
            quil_execution::domains::QUIL_TOKEN,
            b"operation finalized by GLOBAL",
        )
        .unwrap();
        let committed = committed_journal
            .record_decision(OperationDecision::Committed { frame: 42 })
            .unwrap();
        assert_eq!(committed.phase, OperationJournalPhase::Committed);
        assert_eq!(
            committed.detail.as_deref(),
            Some("Committed at GLOBAL frame 42")
        );

        let rejected_journal = OperationJournal::create(
            config.path(),
            WalletAction::Accept,
            quil_execution::domains::QUIL_TOKEN,
            b"operation rejected by GLOBAL",
        )
        .unwrap();
        let rejected = rejected_journal
            .record_decision(OperationDecision::Rejected {
                frame: 43,
                reason: "already consumed".into(),
            })
            .unwrap();
        assert_eq!(rejected.phase, OperationJournalPhase::Rejected);
        assert_eq!(
            rejected.detail.as_deref(),
            Some("Rejected at GLOBAL frame 43: already consumed")
        );

        let states = OperationJournal::scan(config.path()).unwrap();
        assert_eq!(states.len(), 3);
        assert!(states.iter().any(|state| {
            state.phase == OperationJournalPhase::Committed
                && state.detail.as_deref() == Some("Committed at GLOBAL frame 42")
        }));
        assert!(states.iter().any(|state| {
            state.phase == OperationJournalPhase::Rejected
                && state.detail.as_deref() == Some("Rejected at GLOBAL frame 43: already consumed")
        }));
    }

    #[test]
    fn journal_is_private_and_refuses_a_payload_that_no_longer_matches_its_txid() {
        let config = tempfile::tempdir().unwrap();
        let payload = b"immutable canonical operation";
        let expected_txid = hex::encode(quil_execution::token_intrinsic::global_commit::tx_id(
            payload,
        ));
        OperationJournal::create(
            config.path(),
            WalletAction::Reject,
            quil_execution::domains::QUIL_TOKEN,
            payload,
        )
        .unwrap();
        let dir = config
            .path()
            .join(".qclient-console/operations")
            .join(&expected_txid);
        let state: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("state.json")).unwrap()).unwrap();
        assert_eq!(state["version"], JOURNAL_VERSION);
        assert_eq!(state["action"], "reject");
        assert_eq!(
            state["application"],
            hex::encode(quil_execution::domains::QUIL_TOKEN)
        );
        assert_eq!(state["txid"], expected_txid);
        assert_eq!(state["phase"], "submitting");
        assert!(state["submitted_unix_ms"].is_number());
        assert_eq!(state["console_version"], env!("CARGO_PKG_VERSION"));
        assert!(state["build_commit"].is_string());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(
                std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
            for name in ["operation.bin", "state.json"] {
                assert_eq!(
                    std::fs::metadata(dir.join(name))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
            }
        }

        std::fs::write(dir.join("operation.bin"), b"tampered operation").unwrap();
        let error = OperationJournal::scan(config.path()).unwrap_err();
        assert!(error.to_string().contains("does not match its txid"));
    }

    #[test]
    fn recent_operations_are_exposed_as_newest_first_domain_snapshots() {
        let config = tempfile::tempdir().unwrap();
        for index in 0..22u64 {
            let journal = OperationJournal::create(
                config.path(),
                if index % 2 == 0 {
                    WalletAction::Transfer
                } else {
                    WalletAction::Accept
                },
                quil_execution::domains::QUIL_TOKEN,
                format!("operation-{index}").as_bytes(),
            )
            .unwrap();
            let mut state = journal.read_state().unwrap();
            state.submitted_unix_ms = Some(index);
            state.phase = OperationJournalPhase::Submitted;
            journal.write_state(&state).unwrap();
        }

        let recent = OperationJournal::recent(config.path()).unwrap();

        assert_eq!(recent.len(), 20);
        assert_eq!(recent[0].submitted_unix_ms, Some(21));
        assert_eq!(recent[0].action, WalletAction::Accept);
        assert_eq!(
            recent[0].application_hex,
            hex::encode(quil_execution::domains::QUIL_TOKEN)
        );
        assert_eq!(
            recent[0].phase,
            crate::domain::WalletOperationStatus::Submitted
        );
        assert_eq!(recent[19].submitted_unix_ms, Some(2));
    }

    #[test]
    fn wallet_results_and_refresh_snapshots_carry_operation_status() {
        let operation = WalletOperationSnapshot {
            operation_id: "11".repeat(32),
            action: WalletAction::Transfer,
            application_hex: hex::encode(quil_execution::domains::QUIL_TOKEN),
            phase: WalletOperationStatus::Committed,
            submitted_unix_ms: Some(42),
            detail: Some("Committed at GLOBAL frame 99".into()),
            console_version: "0.1.0".into(),
            build_commit: "abc123".into(),
        };
        let wallet = crate::domain::ConfidentialWalletSnapshot {
            address_hex: "wallet".into(),
            total_base_units: 0,
            coins: Vec::new(),
            recent_operations: vec![operation.clone()],
        };
        let escrows = crate::domain::PendingEscrowsSnapshot {
            identity_available: true,
            unavailable_reason: None,
            escrows: Vec::new(),
            recent_operations: vec![operation.clone()],
        };
        let result = crate::domain::WalletOperationResult {
            action: WalletAction::Transfer,
            outcome: crate::domain::WalletSubmissionOutcome::Submitted,
            message: "submitted".into(),
            operation: operation.clone(),
        };

        assert_eq!(wallet.recent_operations, vec![operation.clone()]);
        assert_eq!(escrows.recent_operations, vec![operation.clone()]);
        assert_eq!(result.operation, operation);
    }

    #[test]
    fn an_existing_txid_is_never_reset_or_rewritten_for_resubmission() {
        let config = tempfile::tempdir().unwrap();
        let payload = b"operation which already has a final decision";
        let journal = OperationJournal::create(
            config.path(),
            WalletAction::Reject,
            quil_execution::domains::QUIL_TOKEN,
            payload,
        )
        .unwrap();
        journal
            .record_decision(OperationDecision::Committed { frame: 101 })
            .unwrap();
        let state_path = journal.dir.join("state.json");
        let operation_path = journal.dir.join("operation.bin");
        let state_before = std::fs::read(&state_path).unwrap();
        let operation_before = std::fs::read(&operation_path).unwrap();

        let error = match OperationJournal::create(
            config.path(),
            WalletAction::Reject,
            quil_execution::domains::QUIL_TOKEN,
            payload,
        ) {
            Ok(_) => panic!("an existing txid must not be recreated"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            OperationJournalError::AlreadyTracked { .. }
        ));
        assert_eq!(std::fs::read(state_path).unwrap(), state_before);
        assert_eq!(std::fs::read(operation_path).unwrap(), operation_before);
        let state = journal.read_state().unwrap();
        assert_eq!(state.phase, OperationJournalPhase::Committed);
        assert_eq!(
            state.detail.as_deref(),
            Some("Committed at GLOBAL frame 101")
        );
    }

    #[test]
    fn a_crash_before_state_creation_can_be_safely_initialized_once() {
        let config = tempfile::tempdir().unwrap();
        let payload = b"operation whose journal initialization was interrupted";
        let txid = hex::encode(quil_execution::token_intrinsic::global_commit::tx_id(
            payload,
        ));
        let console = config.path().join(".qclient-console");
        let operations = console.join("operations");
        let dir = operations.join(&txid);
        create_private_dir(&console).unwrap();
        create_private_dir(&operations).unwrap();
        create_private_dir(&dir).unwrap();
        atomic_write(&dir, "operation.bin", payload).unwrap();
        assert!(OperationJournal::scan(config.path()).unwrap().is_empty());

        let journal = OperationJournal::create(
            config.path(),
            WalletAction::PendingTransfer,
            quil_execution::domains::QUIL_TOKEN,
            payload,
        )
        .unwrap();

        assert_eq!(journal.read_operation().unwrap(), payload);
        assert_eq!(
            journal.read_state().unwrap().phase,
            OperationJournalPhase::Submitting
        );
        let second = OperationJournal::create(
            config.path(),
            WalletAction::PendingTransfer,
            quil_execution::domains::QUIL_TOKEN,
            payload,
        );
        assert!(matches!(
            second,
            Err(OperationJournalError::AlreadyTracked { .. })
        ));
    }

    #[tokio::test]
    async fn recovery_reconciles_all_journals_before_limiting_recent_display() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        let config = tempfile::tempdir().unwrap();
        let mut oldest_txid = String::new();
        for index in 0..22u64 {
            let journal = OperationJournal::create(
                config.path(),
                WalletAction::Transfer,
                quil_execution::domains::QUIL_TOKEN,
                format!("pending-operation-{index}").as_bytes(),
            )
            .unwrap();
            let mut state = journal.read_state().unwrap();
            state.submitted_unix_ms = Some(index);
            state.phase = OperationJournalPhase::Submitted;
            if index == 0 {
                oldest_txid = state.operation_id.clone();
            }
            journal.write_state(&state).unwrap();
        }
        let expected_oldest: [u8; 32] = hex::decode(&oldest_txid).unwrap().try_into().unwrap();
        let observed = Arc::new(AtomicUsize::new(0));
        let observed_by_query = Arc::clone(&observed);

        let recent =
            OperationJournal::recover_with_observer(config.path(), move |_application, txid| {
                observed_by_query.fetch_add(1, Ordering::Relaxed);
                std::future::ready(Ok(if txid == expected_oldest {
                    Some(OperationDecision::Committed { frame: 555 })
                } else {
                    None
                }))
            })
            .await
            .unwrap();

        assert_eq!(observed.load(Ordering::Relaxed), 22);
        assert_eq!(recent.len(), RECENT_OPERATION_LIMIT);
        assert!(recent.iter().all(|item| item.operation_id != oldest_txid));
        let states = OperationJournal::scan(config.path()).unwrap();
        let oldest = states
            .iter()
            .find(|state| state.operation_id == oldest_txid)
            .unwrap();
        assert_eq!(oldest.phase, OperationJournalPhase::Committed);
        assert_eq!(
            oldest.detail.as_deref(),
            Some("Committed at GLOBAL frame 555")
        );
        let loaded = OperationJournal::load_snapshot(config.path(), &oldest_txid).unwrap();
        assert_eq!(loaded.phase, WalletOperationStatus::Committed);
        assert_eq!(
            loaded.detail.as_deref(),
            Some("Committed at GLOBAL frame 555")
        );
    }
}
