use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::domain::RewardMintPhase;

pub(super) const MINT_JOURNAL_VERSION: u32 = 5;

fn unknown_build_identity() -> String {
    "unknown".into()
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct MintJournalState {
    pub version: u32,
    #[serde(default = "unknown_build_identity")]
    pub console_version: String,
    #[serde(default = "unknown_build_identity")]
    pub build_commit: String,
    pub network: String,
    pub config_fingerprint: String,
    pub receipt: String,
    pub cited_global_frame: u64,
    pub recipient: String,
    pub reward_amount: u128,
    pub fee: u128,
    pub net_amount: u128,
    pub claimant_public_key: String,
    pub claimant_owner: String,
    pub reward_root: String,
    pub authorization_draft: Option<String>,
    pub authorization_digest: Option<String>,
    pub authorization_proof_verified: bool,
    pub phase: RewardMintPhase,
    pub authorization_file: Option<String>,
    pub claim_file: Option<String>,
    #[serde(default)]
    pub claim_submitted_unix_ms: Option<u64>,
    #[serde(default)]
    pub rebroadcast_count: u32,
    #[serde(default)]
    pub submitted_claim_txids: Vec<String>,
    pub resume_phase: Option<RewardMintPhase>,
    pub last_read_check_unix_ms: Option<u64>,
    pub read_failure_count: u32,
    pub last_error: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum MintJournalError {
    #[error("invalid mint receipt")]
    InvalidReceipt,
    #[error("mint journal I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("cannot encode mint journal: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("cannot decode mint journal: {0}")]
    Decode(#[source] serde_json::Error),
    #[error("unsupported mint journal version {0}")]
    UnsupportedVersion(u32),
    #[error("mint journal does not belong to the active network or config")]
    ContextMismatch,
}

pub(super) struct MintJournal {
    dir: PathBuf,
    receipt: String,
}

impl MintJournal {
    pub(super) fn open(config_dir: &Path, receipt: &str) -> Result<Self, MintJournalError> {
        validate_receipt(receipt)?;
        let root = config_dir.join(".qclient-console");
        let mint = root.join("mint");
        let dir = mint.join(receipt);
        create_private_dir(&root)?;
        create_private_dir(&mint)?;
        create_private_dir(&dir)?;
        Ok(Self {
            dir,
            receipt: receipt.into(),
        })
    }

    pub(super) fn dir(&self) -> &Path {
        &self.dir
    }

    pub(super) fn write_state(&self, state: &MintJournalState) -> Result<(), MintJournalError> {
        let bytes = serde_json::to_vec_pretty(state).map_err(MintJournalError::Encode)?;
        atomic_write(&self.dir, "state.json", &bytes)?;
        Ok(())
    }

    pub(super) fn read_state(&self) -> Result<MintJournalState, MintJournalError> {
        let bytes = std::fs::read(self.dir.join("state.json"))?;
        let mut state: MintJournalState =
            serde_json::from_slice(&bytes).map_err(MintJournalError::Decode)?;
        match state.version {
            3 | 4 => state.version = MINT_JOURNAL_VERSION,
            MINT_JOURNAL_VERSION => {}
            version => return Err(MintJournalError::UnsupportedVersion(version)),
        }
        Ok(state)
    }

    pub(super) fn read_state_checked(
        &self,
        network: &str,
        config_fingerprint: &str,
    ) -> Result<MintJournalState, MintJournalError> {
        let state = self.read_state()?;
        if state.network != network
            || state.config_fingerprint != config_fingerprint
            || state.receipt != self.receipt
        {
            return Err(MintJournalError::ContextMismatch);
        }
        Ok(state)
    }

    pub(super) fn write_authorization(&self, bytes: &[u8]) -> Result<(), MintJournalError> {
        atomic_write(&self.dir, "authorization.bin", bytes)?;
        Ok(())
    }

    pub(super) fn read_authorization(&self) -> Result<Vec<u8>, MintJournalError> {
        Ok(std::fs::read(self.dir.join("authorization.bin"))?)
    }

    pub(super) fn write_claim(&self, bytes: &[u8]) -> Result<(), MintJournalError> {
        atomic_write(&self.dir, "claim.bin", bytes)?;
        Ok(())
    }

    pub(super) fn read_claim(&self) -> Result<Vec<u8>, MintJournalError> {
        Ok(std::fs::read(self.dir.join("claim.bin"))?)
    }

    pub(super) fn reconcile_state(
        &self,
        mut state: MintJournalState,
    ) -> Result<MintJournalState, MintJournalError> {
        let original = state.clone();
        let authorization_exists = self.dir.join("authorization.bin").is_file();
        let claim_exists = self.dir.join("claim.bin").is_file();

        match state.phase {
            RewardMintPhase::ProvingAuthorization if authorization_exists => {
                state.phase = RewardMintPhase::AuthorizationPrepared;
                state.authorization_file = Some("authorization.bin".into());
            }
            RewardMintPhase::AuthorizationPrepared if !authorization_exists => {
                state.phase = RewardMintPhase::ProvingAuthorization;
                state.resume_phase = None;
                state.authorization_file = None;
                state.last_error = Some(
                    "Prepared authorization payload is missing; no request was submitted".into(),
                );
            }
            RewardMintPhase::AwaitingAuthorizationWitness if claim_exists => {
                state.phase = RewardMintPhase::ClaimPrepared;
                state.claim_file = Some("claim.bin".into());
            }
            RewardMintPhase::ClaimPrepared if !claim_exists => {
                state.phase = RewardMintPhase::AwaitingAuthorizationWitness;
                state.claim_file = None;
            }
            RewardMintPhase::SubmittingAuthorization => {
                state.phase = RewardMintPhase::AuthorizationUnknown;
                state.last_error = Some(
                    "Console stopped during authorization submission; the write will not be replayed"
                        .into(),
                );
            }
            RewardMintPhase::SubmittingClaim => {
                state.phase = RewardMintPhase::ClaimUnknown;
                state.last_error = Some(
                    "Console stopped during claim submission; the write will not be replayed"
                        .into(),
                );
            }
            _ => {}
        }

        if state != original {
            self.write_state(&state)?;
        }
        Ok(state)
    }

    pub(super) fn scan_unfinished(
        config_dir: &Path,
        network: &str,
        config_fingerprint: &str,
    ) -> Result<Vec<MintJournalState>, MintJournalError> {
        Self::scan_unfinished_inner(config_dir, network, config_fingerprint, true)
    }

    pub(super) fn scan_unfinished_read_only(
        config_dir: &Path,
        network: &str,
        config_fingerprint: &str,
    ) -> Result<Vec<MintJournalState>, MintJournalError> {
        Self::scan_unfinished_inner(config_dir, network, config_fingerprint, false)
    }

    fn scan_unfinished_inner(
        config_dir: &Path,
        network: &str,
        config_fingerprint: &str,
        reconcile_interrupted: bool,
    ) -> Result<Vec<MintJournalState>, MintJournalError> {
        let root = config_dir.join(".qclient-console/mint");
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut receipts = std::fs::read_dir(root)?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().is_dir())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect::<Vec<_>>();
        receipts.sort();

        let mut states = Vec::new();
        for receipt in receipts {
            validate_receipt(&receipt)?;
            let journal = Self::open(config_dir, &receipt)?;
            if !journal.dir.join("state.json").is_file() {
                // Creating the private directory is the first journal step. A crash before the
                // initial state cannot have dispatched a request, so this directory is safe to
                // ignore and must not block a later Mint.
                continue;
            }
            let state = journal.read_state()?;
            if state.receipt != receipt {
                return Err(MintJournalError::ContextMismatch);
            }
            if state.network != network || state.config_fingerprint != config_fingerprint {
                if state.phase.is_terminal()
                    || matches!(
                        state.phase,
                        RewardMintPhase::ProvingAuthorization
                            | RewardMintPhase::AuthorizationPrepared
                    )
                {
                    // Completed history and provably pre-dispatch work from an old wallet or
                    // network cannot affect the active configuration.
                    continue;
                }
                return Err(MintJournalError::ContextMismatch);
            }
            let state = if reconcile_interrupted {
                journal.reconcile_state(state)?
            } else {
                state
            };
            if state.phase == RewardMintPhase::ProvingAuthorization
                && state.authorization_draft.is_none()
                && !journal.dir.join("authorization.bin").is_file()
            {
                // No request can have been dispatched and there is no complete draft to resume.
                continue;
            }
            if !state.phase.is_terminal() {
                states.push(state);
            }
        }
        Ok(states)
    }
}

fn validate_receipt(receipt: &str) -> Result<(), MintJournalError> {
    let canonical = receipt.len() == 64
        && receipt
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if canonical {
        Ok(())
    } else {
        Err(MintJournalError::InvalidReceipt)
    }
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

    let temp_name = format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    );
    let temp = dir.join(temp_name);
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
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::RewardMintPhase;

    const RECEIPT: &str = "abababababababababababababababababababababababababababababababab";

    fn state(phase: RewardMintPhase) -> MintJournalState {
        MintJournalState {
            version: MINT_JOURNAL_VERSION,
            console_version: "0.1.0".into(),
            build_commit: "test-commit".into(),
            network: "mainnet".into(),
            config_fingerprint: "config-01".into(),
            receipt: RECEIPT.into(),
            cited_global_frame: 42,
            recipient: "QCT3-example".into(),
            reward_amount: 1_000,
            fee: 125,
            net_amount: 875,
            claimant_public_key: "11".repeat(897),
            claimant_owner: "22".repeat(32),
            reward_root: "33".repeat(32),
            authorization_draft: None,
            authorization_digest: None,
            authorization_proof_verified: false,
            phase,
            authorization_file: None,
            claim_file: None,
            claim_submitted_unix_ms: None,
            rebroadcast_count: 0,
            submitted_claim_txids: Vec::new(),
            resume_phase: None,
            last_read_check_unix_ms: None,
            read_failure_count: 0,
            last_error: None,
        }
    }

    #[test]
    fn journal_uses_private_console_directory_below_active_config() {
        let config_dir = tempfile::tempdir().unwrap();

        let journal = MintJournal::open(config_dir.path(), RECEIPT).unwrap();

        assert_eq!(
            journal.dir(),
            config_dir
                .path()
                .join(".qclient-console/mint")
                .join(RECEIPT)
        );
    }

    #[test]
    fn journal_rejects_noncanonical_or_traversing_receipts() {
        let config_dir = tempfile::tempdir().unwrap();

        for receipt in ["../escape", "ABAB", "abab", &"g".repeat(64)] {
            assert!(MintJournal::open(config_dir.path(), receipt).is_err());
        }
    }

    #[test]
    fn state_and_payloads_round_trip_with_private_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let config_dir = tempfile::tempdir().unwrap();
        let journal = MintJournal::open(config_dir.path(), RECEIPT).unwrap();
        let mut expected = state(RewardMintPhase::AuthorizationPrepared);
        expected.authorization_file = Some("authorization.bin".into());

        journal.write_authorization(b"authorization").unwrap();
        journal.write_state(&expected).unwrap();

        assert_eq!(journal.read_authorization().unwrap(), b"authorization");
        assert_eq!(journal.read_state().unwrap(), expected);
        assert_eq!(
            std::fs::metadata(journal.dir())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        for name in ["authorization.bin", "state.json"] {
            assert_eq!(
                std::fs::metadata(journal.dir().join(name))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        assert!(std::fs::read_dir(journal.dir()).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
    }

    #[test]
    fn version_three_journal_loads_with_explicit_unknown_build_identity() {
        let config_dir = tempfile::tempdir().unwrap();
        let journal = MintJournal::open(config_dir.path(), RECEIPT).unwrap();
        let mut legacy = serde_json::to_value(state(RewardMintPhase::ClaimSubmitted)).unwrap();
        legacy["version"] = 3.into();
        for field in [
            "console_version",
            "build_commit",
            "claim_submitted_unix_ms",
            "rebroadcast_count",
        ] {
            legacy.as_object_mut().unwrap().remove(field);
        }
        std::fs::write(
            journal.dir().join("state.json"),
            serde_json::to_vec(&legacy).unwrap(),
        )
        .unwrap();

        let migrated = journal.read_state().unwrap();

        assert_eq!(migrated.version, MINT_JOURNAL_VERSION);
        assert_eq!(migrated.console_version, "unknown");
        assert_eq!(migrated.build_commit, "unknown");
        assert_eq!(migrated.claim_submitted_unix_ms, None);
        assert_eq!(migrated.rebroadcast_count, 0);
        assert!(migrated.submitted_claim_txids.is_empty());
    }

    #[test]
    fn an_abandoned_temporary_file_never_replaces_the_last_valid_state() {
        let config_dir = tempfile::tempdir().unwrap();
        let journal = MintJournal::open(config_dir.path(), RECEIPT).unwrap();
        let expected = state(RewardMintPhase::AwaitingAuthorizationWitness);
        journal.write_state(&expected).unwrap();
        std::fs::write(journal.dir().join(".state.json.crashed.tmp"), b"{").unwrap();

        assert_eq!(journal.read_state().unwrap(), expected);
    }

    #[test]
    fn corrupt_unsupported_or_foreign_state_is_rejected_without_rewrite() {
        let config_dir = tempfile::tempdir().unwrap();
        let journal = MintJournal::open(config_dir.path(), RECEIPT).unwrap();
        let mut unsupported = state(RewardMintPhase::AuthorizationPrepared);
        unsupported.version += 1;
        journal.write_state(&unsupported).unwrap();
        let unsupported_bytes = std::fs::read(journal.dir().join("state.json")).unwrap();
        assert!(matches!(
            journal.read_state_checked("mainnet", "config-01"),
            Err(MintJournalError::UnsupportedVersion(_))
        ));
        assert_eq!(
            std::fs::read(journal.dir().join("state.json")).unwrap(),
            unsupported_bytes
        );

        let expected = state(RewardMintPhase::AuthorizationPrepared);
        journal.write_state(&expected).unwrap();
        assert!(matches!(
            journal.read_state_checked("testnet", "config-01"),
            Err(MintJournalError::ContextMismatch)
        ));

        std::fs::write(journal.dir().join("state.json"), b"{").unwrap();
        assert!(matches!(
            journal.read_state_checked("mainnet", "config-01"),
            Err(MintJournalError::Decode(_))
        ));
        assert_eq!(
            std::fs::read(journal.dir().join("state.json")).unwrap(),
            b"{"
        );
    }

    #[test]
    fn reconciliation_follows_durable_payloads_without_guessing_a_submission() {
        let config_dir = tempfile::tempdir().unwrap();
        let journal = MintJournal::open(config_dir.path(), RECEIPT).unwrap();
        let mut prepared = state(RewardMintPhase::ClaimPrepared);
        prepared.claim_file = Some("claim.bin".into());
        journal.write_state(&prepared).unwrap();

        let demoted = journal.reconcile_state(prepared).unwrap();
        assert_eq!(demoted.phase, RewardMintPhase::AwaitingAuthorizationWitness);
        assert_eq!(demoted.claim_file, None);

        journal.write_claim(b"claim").unwrap();
        assert_eq!(journal.read_claim().unwrap(), b"claim");
        let waiting = state(RewardMintPhase::AwaitingAuthorizationWitness);
        journal.write_state(&waiting).unwrap();
        let restored = journal.reconcile_state(waiting).unwrap();
        assert_eq!(restored.phase, RewardMintPhase::ClaimPrepared);
        assert_eq!(restored.claim_file.as_deref(), Some("claim.bin"));
    }

    #[test]
    fn interrupted_submissions_are_reconciled_to_read_only_recovery() {
        let config_dir = tempfile::tempdir().unwrap();
        let journal = MintJournal::open(config_dir.path(), RECEIPT).unwrap();

        for (interrupted, expected) in [
            (
                RewardMintPhase::SubmittingAuthorization,
                RewardMintPhase::AuthorizationUnknown,
            ),
            (
                RewardMintPhase::SubmittingClaim,
                RewardMintPhase::ClaimUnknown,
            ),
        ] {
            let mut stored = state(interrupted);
            stored.authorization_file = Some("authorization.bin".into());
            if interrupted == RewardMintPhase::SubmittingClaim {
                stored.claim_file = Some("claim.bin".into());
            }
            journal.write_state(&stored).unwrap();

            let reconciled = journal.reconcile_state(stored).unwrap();

            assert_eq!(reconciled.phase, expected);
            assert!(reconciled
                .last_error
                .as_deref()
                .unwrap()
                .contains("will not be replayed"));
        }
    }

    #[test]
    fn read_only_scan_does_not_downgrade_an_active_submission() {
        let config_dir = tempfile::tempdir().unwrap();
        let journal = MintJournal::open(config_dir.path(), RECEIPT).unwrap();
        let mut submitting = state(RewardMintPhase::SubmittingClaim);
        submitting.claim_file = Some("claim.bin".into());
        journal.write_claim(b"claim").unwrap();
        journal.write_state(&submitting).unwrap();

        let found =
            MintJournal::scan_unfinished_read_only(config_dir.path(), "mainnet", "config-01")
                .unwrap();

        assert_eq!(found, vec![submitting.clone()]);
        assert_eq!(journal.read_state().unwrap(), submitting);
    }

    #[test]
    fn incomplete_pre_dispatch_directory_does_not_block_a_new_mint() {
        let config_dir = tempfile::tempdir().unwrap();
        let journal = MintJournal::open(config_dir.path(), RECEIPT).unwrap();
        assert!(
            MintJournal::scan_unfinished(config_dir.path(), "mainnet", "config-01")
                .unwrap()
                .is_empty()
        );
        journal
            .write_state(&state(RewardMintPhase::ProvingAuthorization))
            .unwrap();

        let found =
            MintJournal::scan_unfinished(config_dir.path(), "mainnet", "config-01").unwrap();

        assert!(found.is_empty());
    }

    #[test]
    fn permanent_authorization_rejection_remains_auditable_without_blocking_a_new_mint() {
        let config_dir = tempfile::tempdir().unwrap();
        let journal = MintJournal::open(config_dir.path(), RECEIPT).unwrap();
        let mut rejected = state(RewardMintPhase::PermanentError);
        rejected.last_error = Some("authorization rejected by the node".into());
        journal.write_state(&rejected).unwrap();

        let unfinished =
            MintJournal::scan_unfinished(config_dir.path(), "mainnet", "config-01").unwrap();

        assert!(unfinished.is_empty());
        assert_eq!(journal.read_state().unwrap(), rejected);
    }

    #[test]
    fn complete_proving_draft_remains_visible_for_automatic_resume() {
        let config_dir = tempfile::tempdir().unwrap();
        let journal = MintJournal::open(config_dir.path(), RECEIPT).unwrap();
        let mut proving = state(RewardMintPhase::ProvingAuthorization);
        proving.authorization_draft = Some("00".into());
        journal.write_state(&proving).unwrap();

        let found =
            MintJournal::scan_unfinished(config_dir.path(), "mainnet", "config-01").unwrap();

        assert_eq!(found, vec![proving]);
    }

    #[test]
    fn unfinished_journals_are_scanned_in_receipt_order() {
        let config_dir = tempfile::tempdir().unwrap();
        let receipts = [
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ];
        for (index, receipt) in receipts.into_iter().enumerate() {
            let journal = MintJournal::open(config_dir.path(), receipt).unwrap();
            let mut stored = state(if index == 2 {
                RewardMintPhase::ClaimConfirmed
            } else {
                RewardMintPhase::AwaitingAuthorizationWitness
            });
            stored.receipt = receipt.into();
            journal.write_state(&stored).unwrap();
        }

        let found = MintJournal::scan_unfinished(config_dir.path(), "mainnet", "config-01")
            .unwrap()
            .into_iter()
            .map(|state| state.receipt)
            .collect::<Vec<_>>();

        assert_eq!(
            found,
            vec![receipts[1].to_string(), receipts[0].to_string()]
        );
    }

    #[test]
    fn old_wallet_history_is_ignored_but_ambiguous_delivery_is_not_hidden() {
        let config_dir = tempfile::tempdir().unwrap();
        let journal = MintJournal::open(config_dir.path(), RECEIPT).unwrap();
        let mut foreign = state(RewardMintPhase::ClaimConfirmed);
        foreign.config_fingerprint = "old-wallet".into();
        journal.write_state(&foreign).unwrap();
        assert!(
            MintJournal::scan_unfinished(config_dir.path(), "mainnet", "config-01")
                .unwrap()
                .is_empty()
        );

        foreign.phase = RewardMintPhase::AuthorizationUnknown;
        journal.write_state(&foreign).unwrap();
        assert!(matches!(
            MintJournal::scan_unfinished(config_dir.path(), "mainnet", "config-01"),
            Err(MintJournalError::ContextMismatch)
        ));
    }
}
