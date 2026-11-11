use super::client::SubmitShieldOutcome;
use crate::domain::{
    LegacyIdentitySummary, LegacyMigrationBatchRecord, LegacyMigrationPhase, LegacyMigrationStatus,
};
use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const SCHEMA_VERSION: u32 = 2;
const JOURNAL_FILE: &str = "migration-journal.json";

pub type LegacyMigrationSubmitOutcome = SubmitShieldOutcome;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyMigrationBatchPlan {
    pub source_addresses: Vec<[u8; 32]>,
    pub shard: Vec<bool>,
    pub fee: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyMigrationJournalSeed {
    pub source: LegacyIdentitySummary,
    pub task_dir: PathBuf,
    pub active_config_dir: PathBuf,
    pub recipient_config_dir: PathBuf,
    pub config_path: PathBuf,
    pub keys_path: PathBuf,
    pub source_key_persisted: bool,
    pub read_endpoint: String,
    pub submit_endpoint: String,
    pub running_peer_id: String,
    pub recipient_address: String,
    pub batches: Vec<LegacyMigrationBatchPlan>,
    pub scan_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyMigrationObservation {
    pub consumed_sources: Vec<[u8; 32]>,
    pub target_output_found: bool,
    pub scan_cursor: Option<String>,
}

/// Durable state for one old-QUIL migration. Cryptographic preparation,
/// confirmation reads and Send remain in quil-client; this type only enforces
/// persistence and replay policy around those calls.
pub struct LegacyMigrationJournal {
    task_dir: PathBuf,
    status: LegacyMigrationStatus,
}

impl LegacyMigrationJournal {
    pub fn create(seed: LegacyMigrationJournalSeed) -> Result<Self> {
        ensure!(!seed.batches.is_empty(), "migration has no batches");
        ensure!(
            seed.batches
                .iter()
                .all(|batch| !batch.source_addresses.is_empty()),
            "migration batch has no sources"
        );
        ensure!(
            !seed.read_endpoint.trim().is_empty() && !seed.submit_endpoint.trim().is_empty(),
            "migration endpoints are required"
        );
        quil_config::create_private_dir_all(&seed.task_dir)?;
        let task_dir = canonical_dir(&seed.task_dir, "task")?;
        let journal_path = task_dir.join(JOURNAL_FILE);
        ensure!(!journal_path.exists(), "migration journal already exists");
        let active_config_dir = canonical_dir(&seed.active_config_dir, "active config")?;
        let recipient_config_dir = canonical_dir(&seed.recipient_config_dir, "recipient config")?;
        let (config_path, keys_path) =
            bound_recipient_paths(&recipient_config_dir, &seed.config_path, &seed.keys_path)?;
        let now = now_millis();
        let status = LegacyMigrationStatus {
            schema_version: SCHEMA_VERSION,
            source: seed.source,
            task_dir: task_dir.clone(),
            active_config_dir,
            recipient_config_dir,
            config_path,
            keys_path,
            source_key_persisted: seed.source_key_persisted,
            read_endpoint: seed.read_endpoint,
            submit_endpoint: seed.submit_endpoint,
            running_peer_id: seed.running_peer_id,
            recipient_address: seed.recipient_address,
            batches: seed
                .batches
                .into_iter()
                .map(|batch| LegacyMigrationBatchRecord {
                    source_addresses: batch
                        .source_addresses
                        .into_iter()
                        .map(hex::encode)
                        .collect(),
                    shard: batch
                        .shard
                        .into_iter()
                        .map(|bit| if bit { '1' } else { '0' })
                        .collect(),
                    fee: batch.fee,
                    operation_file: None,
                    operation_sha256: None,
                })
                .collect(),
            current_batch: 0,
            phase: LegacyMigrationPhase::Prepared,
            resume_phase: None,
            scan_cursor: seed.scan_cursor,
            console_version: env!("CARGO_PKG_VERSION").into(),
            build_commit: option_env!("QCLIENT_CONSOLE_BUILD_COMMIT")
                .unwrap_or("unknown")
                .into(),
            created_unix_ms: now,
            updated_unix_ms: now,
            error: None,
        };
        let mut journal = Self { task_dir, status };
        journal.persist()?;
        Ok(journal)
    }

    /// Load and validate saved state, applying source-key availability only to
    /// the returned view. This read path never rewrites the journal.
    pub fn load(task_dir: &Path, source_key_available: bool) -> Result<Self> {
        let mut journal = Self::load_saved(task_dir)?;
        journal.project_source_key(source_key_available)?;
        Ok(journal)
    }

    fn load_saved(task_dir: &Path) -> Result<Self> {
        let task_dir = canonical_dir(task_dir, "task")?;
        let bytes = std::fs::read(task_dir.join(JOURNAL_FILE))?;
        let status: LegacyMigrationStatus =
            serde_json::from_slice(&bytes).context("invalid migration journal")?;
        let journal = Self { task_dir, status };
        journal.validate()?;
        Ok(journal)
    }

    /// Writer-only restart reconciliation. An interrupted submission becomes
    /// Unknown here, but the saved operation is never replayed automatically.
    pub fn load_for_advance(task_dir: &Path, source_key_available: bool) -> Result<Self> {
        let mut journal = Self::load_saved(task_dir)?;
        if matches!(journal.status.phase, LegacyMigrationPhase::Migrating { .. }) {
            journal.status.phase = LegacyMigrationPhase::SubmissionUnknown;
            journal.status.error = Some(
                "Console stopped during submission; the saved operation was not replayed".into(),
            );
            journal.persist()?;
        }
        if source_key_available
            && matches!(
                journal.status.phase,
                LegacyMigrationPhase::AwaitingSourceKey
            )
        {
            journal.resume_with_source_key()?;
        } else if !source_key_available
            && !journal.status.source_key_persisted
            && !matches!(journal.status.phase, LegacyMigrationPhase::Complete)
            && !matches!(
                journal.status.phase,
                LegacyMigrationPhase::AwaitingSourceKey
            )
        {
            journal.status.resume_phase = Some(journal.status.phase.clone());
            journal.status.phase = LegacyMigrationPhase::AwaitingSourceKey;
            journal.persist()?;
        }
        Ok(journal)
    }

    fn project_source_key(&mut self, source_key_available: bool) -> Result<()> {
        if source_key_available
            && matches!(self.status.phase, LegacyMigrationPhase::AwaitingSourceKey)
        {
            self.status.phase = self
                .status
                .resume_phase
                .take()
                .ok_or_else(|| anyhow::anyhow!("missing source-key resume phase"))?;
        } else if !source_key_available
            && !self.status.source_key_persisted
            && !matches!(self.status.phase, LegacyMigrationPhase::Complete)
            && !matches!(self.status.phase, LegacyMigrationPhase::AwaitingSourceKey)
        {
            self.status.resume_phase = Some(self.status.phase.clone());
            self.status.phase = LegacyMigrationPhase::AwaitingSourceKey;
        }
        Ok(())
    }

    pub fn status(&self) -> &LegacyMigrationStatus {
        &self.status
    }

    /// Read the current saved operation after validating its fixed filename,
    /// private file mode and journaled digest.
    pub fn current_saved_operation(&self) -> Result<Vec<u8>> {
        self.read_current_operation()
    }

    /// Remove an operation that reached disk before its Prepared journal
    /// metadata could be persisted. Send cannot have started in this state.
    pub fn recover_prepared_orphan(&self) -> Result<bool> {
        ensure!(
            matches!(self.status.phase, LegacyMigrationPhase::Prepared),
            "migration is not prepared"
        );
        let record = &self.status.batches[self.status.current_batch];
        ensure!(
            record.operation_file.is_none() && record.operation_sha256.is_none(),
            "prepared migration already records an operation"
        );
        let path = self
            .task_dir
            .join(operation_name(self.status.current_batch));
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        validate_private_operation_file(&metadata)?;
        std::fs::remove_file(&path)?;
        quil_config::sync_parent_dir(&path);
        Ok(true)
    }

    /// Persist the exact operation and Migrating state before invoking Send.
    pub async fn submit_current<F, Fut>(&mut self, operation: &[u8], submit: F) -> Result<()>
    where
        F: FnOnce(Vec<u8>) -> Fut,
        Fut: std::future::Future<Output = Result<LegacyMigrationSubmitOutcome>>,
    {
        ensure!(
            matches!(self.status.phase, LegacyMigrationPhase::Prepared),
            "migration is not ready to submit"
        );
        ensure!(!operation.is_empty(), "migration operation is empty");
        self.save_current_operation(operation)?;
        self.mark_migrating()?;
        let outcome = submit(operation.to_vec()).await;
        self.record_submit_result(outcome)
    }

    /// Replay is deliberately limited to an explicitly confirmed Unknown.
    pub async fn rebroadcast_saved_operation<F, Fut>(
        &mut self,
        confirmed: bool,
        submit: F,
    ) -> Result<()>
    where
        F: FnOnce(Vec<u8>) -> Fut,
        Fut: std::future::Future<Output = Result<LegacyMigrationSubmitOutcome>>,
    {
        ensure!(confirmed, "saved-operation replay requires confirmation");
        ensure!(
            matches!(self.status.phase, LegacyMigrationPhase::SubmissionUnknown),
            "only an unknown submission can be replayed"
        );
        let operation = self.read_current_operation()?;
        self.mark_migrating()?;
        let outcome = submit(operation).await;
        self.record_submit_result(outcome)
    }

    /// A definite rejection has a separate, explicit retry path so it cannot
    /// be confused with acknowledgement of an ambiguous submission.
    pub async fn retry_rejected_saved_operation<F, Fut>(
        &mut self,
        confirmed: bool,
        submit: F,
    ) -> Result<()>
    where
        F: FnOnce(Vec<u8>) -> Fut,
        Fut: std::future::Future<Output = Result<LegacyMigrationSubmitOutcome>>,
    {
        ensure!(confirmed, "rejected-operation retry requires confirmation");
        ensure!(
            matches!(self.status.phase, LegacyMigrationPhase::RecoverableError),
            "only a rejected submission can use rejected-operation retry"
        );
        let operation = self.read_current_operation()?;
        self.mark_migrating()?;
        let outcome = submit(operation).await;
        self.record_submit_result(outcome)
    }

    /// Record read-side evidence. Unknown operations stay Unknown unless all
    /// source addresses are consumed and the target output is visible.
    pub fn record_observation(&mut self, observation: LegacyMigrationObservation) -> Result<()> {
        ensure!(
            matches!(
                self.status.phase,
                LegacyMigrationPhase::SubmissionUnknown
                    | LegacyMigrationPhase::AwaitingChainConfirmation { .. }
            ),
            "migration is not awaiting chain observation"
        );
        self.status.scan_cursor = observation.scan_cursor;
        let batch = &self.status.batches[self.status.current_batch];
        let all_consumed = batch.source_addresses.iter().all(|address| {
            observation
                .consumed_sources
                .iter()
                .any(|consumed| hex::encode(consumed) == *address)
        });
        if all_consumed && observation.target_output_found {
            self.status.error = None;
            if self.status.current_batch + 1 == self.status.batches.len() {
                self.status.phase = LegacyMigrationPhase::Complete;
            } else {
                self.status.current_batch += 1;
                self.status.phase = LegacyMigrationPhase::Prepared;
            }
        }
        self.persist()
    }

    pub fn resume_with_source_key(&mut self) -> Result<()> {
        ensure!(
            matches!(self.status.phase, LegacyMigrationPhase::AwaitingSourceKey),
            "migration is not waiting for its source key"
        );
        self.status.phase = self
            .status
            .resume_phase
            .take()
            .ok_or_else(|| anyhow::anyhow!("missing source-key resume phase"))?;
        self.persist()
    }

    fn mark_migrating(&mut self) -> Result<()> {
        self.status.phase = LegacyMigrationPhase::Migrating {
            batch: self.status.current_batch + 1,
            total: self.status.batches.len(),
        };
        self.status.error = None;
        self.persist()
    }

    fn record_submit_outcome(&mut self, outcome: LegacyMigrationSubmitOutcome) -> Result<()> {
        match outcome {
            SubmitShieldOutcome::Accepted => {
                self.status.phase = LegacyMigrationPhase::AwaitingChainConfirmation {
                    batch: self.status.current_batch + 1,
                    total: self.status.batches.len(),
                };
                self.status.error = None;
            }
            SubmitShieldOutcome::Rejected(error) => {
                self.status.phase = LegacyMigrationPhase::RecoverableError;
                self.status.error = Some(sanitize_error(&error));
            }
            SubmitShieldOutcome::Unknown(error) => {
                self.status.phase = LegacyMigrationPhase::SubmissionUnknown;
                self.status.error = Some(sanitize_error(&error));
            }
        }
        self.persist()
    }

    fn record_submit_result(
        &mut self,
        outcome: Result<LegacyMigrationSubmitOutcome>,
    ) -> Result<()> {
        match outcome {
            Ok(outcome) => self.record_submit_outcome(outcome),
            Err(error) => {
                self.record_submit_outcome(SubmitShieldOutcome::Unknown(error.to_string()))
            }
        }
    }

    fn save_current_operation(&mut self, operation: &[u8]) -> Result<()> {
        let name = operation_name(self.status.current_batch);
        let path = self.task_dir.join(&name);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) => {
                validate_private_operation_file(&metadata)?;
                ensure!(
                    std::fs::read(&path)? == operation,
                    "saved migration operation differs"
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                crate::service::private_file::write_atomic(&path, operation)?;
            }
            Err(error) => return Err(error.into()),
        }
        let record = &mut self.status.batches[self.status.current_batch];
        record.operation_file = Some(name);
        record.operation_sha256 = Some(hex::encode(Sha256::digest(operation)));
        Ok(())
    }

    fn read_current_operation(&self) -> Result<Vec<u8>> {
        let record = &self.status.batches[self.status.current_batch];
        let name = record
            .operation_file
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("migration operation was not saved"))?;
        ensure!(
            name == operation_name(self.status.current_batch),
            "migration operation filename is invalid"
        );
        let path = self.task_dir.join(name);
        validate_private_operation_file(&std::fs::symlink_metadata(&path)?)?;
        let operation = std::fs::read(path)?;
        ensure!(
            record.operation_sha256.as_deref()
                == Some(hex::encode(Sha256::digest(&operation)).as_str()),
            "saved migration operation hash differs"
        );
        Ok(operation)
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.status.schema_version == SCHEMA_VERSION,
            "unsupported migration journal schema"
        );
        ensure!(
            self.status.task_dir == self.task_dir,
            "migration journal task path differs"
        );
        let recipient_config_dir =
            canonical_dir(&self.status.recipient_config_dir, "recipient config")?;
        ensure!(
            self.status.recipient_config_dir == recipient_config_dir,
            "migration recipient path differs"
        );
        let (config_path, keys_path) = bound_recipient_paths(
            &recipient_config_dir,
            &self.status.config_path,
            &self.status.keys_path,
        )?;
        ensure!(
            self.status.config_path == config_path && self.status.keys_path == keys_path,
            "migration recipient files differ"
        );
        ensure!(!self.status.batches.is_empty(), "migration has no batches");
        ensure!(
            self.status.current_batch < self.status.batches.len(),
            "migration batch cursor is invalid"
        );
        validate_phase_coordinates(
            &self.status.phase,
            self.status.current_batch,
            self.status.batches.len(),
        )?;
        if let Some(phase) = &self.status.resume_phase {
            ensure!(
                matches!(self.status.phase, LegacyMigrationPhase::AwaitingSourceKey),
                "migration resume phase exists outside source-key recovery"
            );
            validate_phase_coordinates(
                phase,
                self.status.current_batch,
                self.status.batches.len(),
            )?;
        } else {
            ensure!(
                !matches!(self.status.phase, LegacyMigrationPhase::AwaitingSourceKey),
                "migration source-key recovery has no resume phase"
            );
        }
        for (index, batch) in self.status.batches.iter().enumerate() {
            ensure!(
                !batch.source_addresses.is_empty(),
                "migration batch has no sources"
            );
            ensure!(
                batch.source_addresses.iter().all(|address| {
                    address.len() == 64
                        && address
                            .chars()
                            .all(|character| character.is_ascii_hexdigit())
                        && address.to_ascii_lowercase() == *address
                }),
                "migration source address is invalid"
            );
            ensure!(
                batch.shard.chars().all(|bit| bit == '0' || bit == '1'),
                "migration shard is invalid"
            );
            ensure!(
                batch.operation_file.is_some() == batch.operation_sha256.is_some(),
                "migration operation metadata is incomplete"
            );
            if batch.operation_file.is_some() {
                ensure!(
                    batch.operation_file.as_deref() == Some(operation_name(index).as_str()),
                    "migration operation filename is invalid"
                );
                let path = self.task_dir.join(batch.operation_file.as_deref().unwrap());
                validate_private_operation_file(&std::fs::symlink_metadata(&path)?)?;
                let bytes = std::fs::read(path)?;
                ensure!(
                    batch.operation_sha256.as_deref()
                        == Some(hex::encode(Sha256::digest(bytes)).as_str()),
                    "saved migration operation hash differs"
                );
            }
        }
        Ok(())
    }

    fn persist(&mut self) -> Result<()> {
        self.status.updated_unix_ms = now_millis();
        let bytes = serde_json::to_vec_pretty(&self.status)?;
        crate::service::private_file::write_atomic(&self.task_dir.join(JOURNAL_FILE), &bytes)?;
        Ok(())
    }
}

fn canonical_dir(path: &Path, label: &str) -> Result<PathBuf> {
    let canonical = std::fs::canonicalize(path)
        .with_context(|| format!("cannot resolve migration {label} directory"))?;
    ensure!(
        canonical.is_dir(),
        "migration {label} path is not a directory"
    );
    Ok(canonical)
}

fn bound_recipient_paths(
    recipient_config_dir: &Path,
    config_path: &Path,
    keys_path: &Path,
) -> Result<(PathBuf, PathBuf)> {
    let config_path = canonical_file(config_path, "recipient config")?;
    let expected_config =
        canonical_file(&recipient_config_dir.join("config.yml"), "recipient config")?;
    ensure!(
        config_path == expected_config,
        "migration config path is not bound to the recipient"
    );
    let config = quil_config::load_config(recipient_config_dir)
        .context("cannot load migration recipient config")?;
    let resolved_keys =
        quil_keys::resolve_keystore_path(&config.key.key_store_file, recipient_config_dir)
            .context("cannot resolve migration recipient keystore")?;
    let keys_path = canonical_file(keys_path, "recipient keystore")?;
    let expected_keys = canonical_file(&resolved_keys, "recipient keystore")?;
    ensure!(
        keys_path == expected_keys,
        "migration keys path is not bound to the recipient"
    );
    ensure!(
        config_path != keys_path,
        "migration config and keys paths conflict"
    );
    Ok((config_path, keys_path))
}

fn canonical_file(path: &Path, label: &str) -> Result<PathBuf> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("cannot inspect migration {label} file"))?;
    ensure!(
        metadata.file_type().is_file(),
        "migration {label} path is not a regular file"
    );
    std::fs::canonicalize(path).with_context(|| format!("cannot resolve migration {label} file"))
}

fn operation_name(batch: usize) -> String {
    format!("migration-operation-{:04}.bin", batch + 1)
}

fn validate_private_operation_file(metadata: &std::fs::Metadata) -> Result<()> {
    ensure!(
        metadata.file_type().is_file(),
        "migration operation is not a regular file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o777 == 0o600,
            "migration operation is not private"
        );
    }
    Ok(())
}

fn validate_phase_coordinates(
    phase: &LegacyMigrationPhase,
    current_batch: usize,
    total: usize,
) -> Result<()> {
    if let LegacyMigrationPhase::Migrating {
        batch,
        total: phase_total,
    }
    | LegacyMigrationPhase::AwaitingChainConfirmation {
        batch,
        total: phase_total,
    } = phase
    {
        ensure!(
            *batch == current_batch + 1 && *phase_total == total,
            "migration phase batch coordinates are inconsistent"
        );
    }
    Ok(())
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn sanitize_error(error: &str) -> String {
    let chars = error
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<Vec<_>>();
    let mut sanitized = String::new();
    let mut index = 0;
    while index < chars.len() && sanitized.chars().count() < 512 {
        if chars[index].is_ascii_hexdigit() {
            let start = index;
            while index < chars.len() && chars[index].is_ascii_hexdigit() {
                index += 1;
            }
            if index - start >= 64 {
                sanitized.push_str("[redacted]");
            } else {
                sanitized.extend(&chars[start..index]);
            }
        } else {
            sanitized.push(chars[index]);
            index += 1;
        }
    }
    let sanitized = sanitized.trim();
    if sanitized.is_empty() {
        "submission failed".into()
    } else {
        sanitized.into()
    }
}
