use super::{
    client::{
        discover_legacy_assets, group_legacy_sources, inspect_saved_shield_output,
        inspect_shield_confirmation, prepare_shield_batches, quote_shield_fee, submit_saved_shield,
        FeeQuoteDisplay, LegacySourceIdentity, MigrationEndpoints, PreparedShieldBatch,
        RecipientIdentity, ShieldBatch, SubmitIdentity,
    },
    prepare_package, LegacyIdentity, LegacyMigrationBatchPlan, LegacyMigrationJournal,
    LegacyMigrationJournalSeed, LegacyMigrationObservation, LegacyMigrationRequest,
    SecretLegacyKey,
};
use crate::{
    domain::{LegacyMigrationPhase, LegacyMigrationPreflight, LegacyMigrationStatus},
    service::{read_rpc, ServiceError},
};
use quil_client::{
    commands::token::{TokenCtx, DEFAULT_MAX_AUTOMATIC_FEE},
    rpc::ConnectOpts,
};
use std::{collections::HashMap, path::Path, path::PathBuf, sync::Mutex};
use zeroize::Zeroizing;

struct PendingMigration {
    public: LegacyMigrationPreflight,
    package: crate::domain::LegacyMigrationPackage,
    prepared: Vec<PreparedShieldBatch>,
    read: TokenCtx,
    submit: TokenCtx,
    recipient: RecipientIdentity,
    sender: SubmitIdentity,
}

#[derive(Default)]
struct PendingState {
    approvals: HashMap<String, PendingMigration>,
    transient_sources: HashMap<String, Zeroizing<String>>,
}

pub struct LegacyMigrationCoordinator {
    home: PathBuf,
    pending: Mutex<PendingState>,
}

impl Default for LegacyMigrationCoordinator {
    fn default() -> Self {
        Self::new(
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(".")),
        )
    }
}

impl LegacyMigrationCoordinator {
    pub fn new(home: PathBuf) -> Self {
        Self {
            home,
            pending: Mutex::new(PendingState::default()),
        }
    }

    pub async fn preflight(
        &self,
        active_config: &Path,
        request: LegacyMigrationRequest,
    ) -> Result<LegacyMigrationPreflight, ServiceError> {
        let active_config = canonical_dir(active_config)?;
        let (source_secret, destination, allow_initialize) = request.into_parts();
        let package = prepare_package(
            &self.home,
            Some(&active_config),
            destination.clone(),
            &source_secret,
            allow_initialize,
        )
        .map_err(migration_error)?;
        let active = quil_config::load_config(&active_config)
            .map_err(|error| ServiceError::Config(error.to_string()))?;
        let submit_endpoint = active.effective_listen_grpc_multiaddr().to_owned();
        let read_endpoint = read_rpc::load(&active_config, &submit_endpoint)?;
        let running_peer_id = LegacyIdentity::parse(&active.p2p.peer_priv_key)
            .map_err(migration_error)?
            .summary()
            .peer_id
            .clone();

        let source_hex = source_secret.normalized();
        let read = token_context(
            &active_config,
            &read_endpoint,
            &submit_endpoint,
            Some(&source_hex),
        )?;
        let submit = token_context(&active_config, &submit_endpoint, &submit_endpoint, None)?;
        let recipient_context = token_context(
            &package.recipient_config_dir,
            &read_endpoint,
            &submit_endpoint,
            None,
        )?;
        let source = LegacySourceIdentity::load(&read).map_err(migration_error)?;
        let recipient = RecipientIdentity::load(&recipient_context).map_err(migration_error)?;
        let sender = SubmitIdentity::new(submit.key_manager.clone()).map_err(migration_error)?;
        let endpoints = MigrationEndpoints::new(&read, &submit).map_err(migration_error)?;
        let assets = discover_legacy_assets(&endpoints, &source)
            .await
            .map_err(migration_error)?;
        let unshielded = assets
            .coins
            .into_iter()
            .filter(|coin| !coin.shielded)
            .map(|coin| (coin.address, coin.amount))
            .collect::<Vec<_>>();
        if unshielded.is_empty() {
            return Err(ServiceError::InvalidInput(
                "This old identity has no unshielded QUIL".into(),
            ));
        }
        let shards = endpoints.live_shards().await.map_err(migration_error)?;
        let grouped = group_legacy_sources(unshielded, &shards, 96).map_err(migration_error)?;
        if grouped.batches.is_empty() {
            return Err(ServiceError::InvalidInput(
                "No legacy QUIL is currently covered by a live shard".into(),
            ));
        }
        let source_amount = grouped
            .batches
            .iter()
            .flat_map(|batch| &batch.sources)
            .try_fold(0_u128, |sum, source| sum.checked_add(source.amount))
            .ok_or_else(|| ServiceError::InvalidInput("Legacy QUIL amount overflow".into()))?;
        let mut fees = Vec::with_capacity(grouped.batches.len());
        for batch in &grouped.batches {
            fees.push(
                quote_shield_fee(&endpoints, batch.sources.len(), FeeQuoteDisplay::Silent)
                    .await
                    .map_err(migration_error)?,
            );
        }
        let fee_total = fees
            .iter()
            .try_fold(0_u128, |sum, fee| sum.checked_add(*fee))
            .ok_or_else(|| ServiceError::InvalidInput("Migration fee overflow".into()))?;
        let estimated_net_amount = source_amount.checked_sub(fee_total).ok_or_else(|| {
            ServiceError::InvalidInput("Legacy QUIL does not cover migration fees".into())
        })?;
        let prepared = prepare_shield_batches(&source, &recipient, &grouped.batches, &fees)
            .await
            .map_err(migration_error)?;
        let approval_id = format!(
            "{}-{:016x}",
            source_secret.summary().fingerprint,
            rand::random::<u64>()
        );
        let mut warnings = Vec::new();
        if !grouped.uncovered.is_empty() {
            warnings.push(format!(
                "{} legacy outputs are outside current live shards",
                grouped.uncovered.len()
            ));
        }
        let public = LegacyMigrationPreflight {
            approval_id: approval_id.clone(),
            source: source_secret.summary().clone(),
            source_count: prepared
                .iter()
                .map(|batch| batch.source_addresses.len())
                .sum(),
            source_amount,
            batch_count: prepared.len(),
            destination,
            recipient_config_dir: package.recipient_config_dir.clone(),
            config_path: package.config_path.clone(),
            keys_path: package.keys_path.clone(),
            recipient_address: package.recipient_address.clone(),
            active_config_dir: active_config.clone(),
            read_endpoint,
            submit_endpoint,
            running_peer_id,
            fees,
            estimated_net_amount,
            warnings,
        };
        self.pending.lock().unwrap().approvals.insert(
            approval_id,
            PendingMigration {
                public: public.clone(),
                package,
                prepared,
                read,
                submit,
                recipient,
                sender,
            },
        );
        Ok(public)
    }

    pub async fn start(
        &self,
        approved: LegacyMigrationPreflight,
    ) -> Result<LegacyMigrationStatus, ServiceError> {
        let pending = self
            .pending
            .lock()
            .unwrap()
            .approvals
            .remove(&approved.approval_id)
            .ok_or_else(|| ServiceError::InvalidInput("Migration approval expired".into()))?;
        if pending.public != approved {
            return Err(ServiceError::InvalidInput(
                "Migration approval contents changed".into(),
            ));
        }
        let plans = pending
            .prepared
            .iter()
            .map(|batch| LegacyMigrationBatchPlan {
                source_addresses: batch.source_addresses.clone(),
                shard: batch.shard.clone(),
                fee: batch.fee,
            })
            .collect();
        let mut journal = LegacyMigrationJournal::create(LegacyMigrationJournalSeed {
            source: approved.source.clone(),
            task_dir: pending.package.task_dir.clone(),
            active_config_dir: approved.active_config_dir.clone(),
            recipient_config_dir: approved.recipient_config_dir.clone(),
            config_path: pending.package.config_path.clone(),
            keys_path: pending.package.keys_path.clone(),
            source_key_persisted: pending.package.generated,
            read_endpoint: approved.read_endpoint.clone(),
            submit_endpoint: approved.submit_endpoint.clone(),
            running_peer_id: approved.running_peer_id.clone(),
            recipient_address: approved.recipient_address.clone(),
            batches: plans,
            scan_cursor: None,
        })
        .map_err(migration_error)?;
        submit_current(&mut journal, &pending, 0).await?;
        let status = journal.status().clone();
        self.remember_source(
            &approved.source.fingerprint,
            Zeroizing::new(pending.read.node_config.p2p.peer_priv_key.clone()),
        );
        Ok(status)
    }

    pub fn load(&self, fingerprint: &str) -> Result<Option<LegacyMigrationStatus>, ServiceError> {
        let task = self.task_dir(fingerprint)?;
        if !task.join("migration-journal.json").is_file() {
            return Ok(None);
        }
        LegacyMigrationJournal::load(&task, self.has_source(fingerprint))
            .map(|journal| Some(journal.status().clone()))
            .map_err(migration_error)
    }

    pub async fn advance(
        &self,
        fingerprint: &str,
        source_key: Option<SecretLegacyKey>,
    ) -> Result<LegacyMigrationStatus, ServiceError> {
        let source_key = source_key.map(SecretLegacyKey::into_identity);
        if let Some(identity) = &source_key {
            if identity.summary().fingerprint != fingerprint {
                return Err(ServiceError::InvalidInput(
                    "Old key does not belong to this migration".into(),
                ));
            }
            self.remember_source(fingerprint, identity.normalized());
        }
        let transient_source = self.transient_source(fingerprint);
        let mut journal = LegacyMigrationJournal::load_for_advance(
            &self.task_dir(fingerprint)?,
            transient_source.is_some(),
        )
        .map_err(migration_error)?;
        if matches!(
            journal.status().phase,
            LegacyMigrationPhase::AwaitingSourceKey
        ) {
            return Ok(journal.status().clone());
        }
        let action = advance_action(&journal.status().phase);
        if matches!(action, AdvanceAction::Return) {
            return Ok(journal.status().clone());
        }
        let identity = match transient_source {
            Some(source) => LegacyIdentity::parse(&source).map_err(migration_error)?,
            None if journal.status().source_key_persisted => {
                let config = quil_config::load_config(&journal.status().recipient_config_dir)
                    .map_err(|error| ServiceError::Config(error.to_string()))?;
                LegacyIdentity::parse(&config.p2p.peer_priv_key).map_err(migration_error)?
            }
            None => return Ok(journal.status().clone()),
        };
        let status = journal.status().clone();
        let source_hex = identity.normalized();
        let read = token_context(
            &status.active_config_dir,
            &status.read_endpoint,
            &status.submit_endpoint,
            Some(&source_hex),
        )?;
        let submit = token_context(
            &status.active_config_dir,
            &status.submit_endpoint,
            &status.submit_endpoint,
            None,
        )?;
        let source = LegacySourceIdentity::load(&read).map_err(migration_error)?;
        let endpoints = MigrationEndpoints::new(&read, &submit).map_err(migration_error)?;
        if matches!(action, AdvanceAction::Observe) {
            let addresses = status.batches[status.current_batch]
                .source_addresses
                .iter()
                .map(|address| {
                    let bytes = hex::decode(address).map_err(migration_error)?;
                    bytes
                        .try_into()
                        .map_err(|_| ServiceError::InvalidInput("Invalid migration source".into()))
                })
                .collect::<Result<Vec<[u8; 32]>, ServiceError>>()?;
            let observed = inspect_shield_confirmation(&endpoints, &source, &addresses)
                .await
                .map_err(migration_error)?;
            let all_consumed = observed.consumed.len() == addresses.len();
            let target_output_found = if all_consumed {
                let recipient_context = token_context(
                    &status.recipient_config_dir,
                    &status.read_endpoint,
                    &status.submit_endpoint,
                    None,
                )?;
                let recipient =
                    RecipientIdentity::load(&recipient_context).map_err(migration_error)?;
                let operation = journal.current_saved_operation().map_err(migration_error)?;
                inspect_saved_shield_output(&endpoints, &recipient, &operation)
                    .await
                    .map_err(migration_error)?
            } else {
                false
            };
            journal
                .record_observation(LegacyMigrationObservation {
                    consumed_sources: observed.consumed,
                    target_output_found,
                    scan_cursor: None,
                })
                .map_err(migration_error)?;
        }
        if matches!(journal.status().phase, LegacyMigrationPhase::Prepared) {
            prepare_and_submit_current(&mut journal, &source, &endpoints, &submit).await?;
        }
        let status = journal.status().clone();
        if matches!(status.phase, LegacyMigrationPhase::Complete) {
            self.forget_source(fingerprint);
        }
        Ok(status)
    }

    pub async fn rebroadcast(
        &self,
        fingerprint: &str,
        confirmed: bool,
    ) -> Result<LegacyMigrationStatus, ServiceError> {
        let mut journal = LegacyMigrationJournal::load(
            &self.task_dir(fingerprint)?,
            self.has_source(fingerprint),
        )
        .map_err(migration_error)?;
        let status = journal.status().clone();
        let action = recovery_action(&status.phase)?;
        let read = token_context(
            &status.active_config_dir,
            &status.read_endpoint,
            &status.submit_endpoint,
            None,
        )?;
        let submit = token_context(
            &status.active_config_dir,
            &status.submit_endpoint,
            &status.submit_endpoint,
            None,
        )?;
        let recipient_context = token_context(
            &status.recipient_config_dir,
            &status.read_endpoint,
            &status.submit_endpoint,
            None,
        )?;
        let recipient = RecipientIdentity::load(&recipient_context).map_err(migration_error)?;
        let sender = SubmitIdentity::new(submit.key_manager.clone()).map_err(migration_error)?;
        let endpoints = MigrationEndpoints::new(&read, &submit).map_err(migration_error)?;
        match action {
            RecoveryAction::RebroadcastUnknown => {
                journal
                    .rebroadcast_saved_operation(confirmed, |operation| async {
                        submit_saved_shield(&endpoints, &recipient, &sender, operation).await
                    })
                    .await
            }
            RecoveryAction::RetryRejected => {
                journal
                    .retry_rejected_saved_operation(confirmed, |operation| async {
                        submit_saved_shield(&endpoints, &recipient, &sender, operation).await
                    })
                    .await
            }
        }
        .map_err(migration_error)?;
        Ok(journal.status().clone())
    }

    fn task_dir(&self, fingerprint: &str) -> Result<PathBuf, ServiceError> {
        if fingerprint.len() != 32
            || !fingerprint
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ServiceError::InvalidInput(
                "Invalid legacy migration fingerprint".into(),
            ));
        }
        Ok(self
            .home
            .join(".qclient-console/legacy-wallets")
            .join(format!("ed448-{fingerprint}")))
    }

    fn remember_source(&self, fingerprint: &str, source: Zeroizing<String>) {
        self.pending
            .lock()
            .unwrap()
            .transient_sources
            .insert(fingerprint.into(), source);
    }

    fn transient_source(&self, fingerprint: &str) -> Option<Zeroizing<String>> {
        self.pending
            .lock()
            .unwrap()
            .transient_sources
            .get(fingerprint)
            .map(|source| Zeroizing::new(source.to_string()))
    }

    fn forget_source(&self, fingerprint: &str) {
        self.pending
            .lock()
            .unwrap()
            .transient_sources
            .remove(fingerprint);
    }

    fn has_source(&self, fingerprint: &str) -> bool {
        self.pending
            .lock()
            .unwrap()
            .transient_sources
            .contains_key(fingerprint)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum RecoveryAction {
    RebroadcastUnknown,
    RetryRejected,
}

#[derive(Debug, PartialEq, Eq)]
enum AdvanceAction {
    PrepareAndSubmit,
    Observe,
    Return,
}

fn advance_action(phase: &LegacyMigrationPhase) -> AdvanceAction {
    match phase {
        LegacyMigrationPhase::Prepared => AdvanceAction::PrepareAndSubmit,
        LegacyMigrationPhase::AwaitingChainConfirmation { .. }
        | LegacyMigrationPhase::SubmissionUnknown => AdvanceAction::Observe,
        _ => AdvanceAction::Return,
    }
}

fn recovery_action(phase: &LegacyMigrationPhase) -> Result<RecoveryAction, ServiceError> {
    match phase {
        LegacyMigrationPhase::SubmissionUnknown => Ok(RecoveryAction::RebroadcastUnknown),
        LegacyMigrationPhase::RecoverableError => Ok(RecoveryAction::RetryRejected),
        _ => Err(ServiceError::InvalidInput(
            "Migration has no recoverable saved submission".into(),
        )),
    }
}

async fn submit_current(
    journal: &mut LegacyMigrationJournal,
    pending: &PendingMigration,
    index: usize,
) -> Result<(), ServiceError> {
    let endpoints =
        MigrationEndpoints::new(&pending.read, &pending.submit).map_err(migration_error)?;
    journal
        .submit_current(&pending.prepared[index].operation, |operation| async {
            submit_saved_shield(&endpoints, &pending.recipient, &pending.sender, operation).await
        })
        .await
        .map_err(migration_error)
}

async fn prepare_and_submit_current(
    journal: &mut LegacyMigrationJournal,
    source: &LegacySourceIdentity,
    endpoints: &MigrationEndpoints<'_>,
    submit: &TokenCtx,
) -> Result<(), ServiceError> {
    journal.recover_prepared_orphan().map_err(migration_error)?;
    let status = journal.status().clone();
    let assets = discover_legacy_assets(endpoints, source)
        .await
        .map_err(migration_error)?;
    let record = &status.batches[status.current_batch];
    let sources = record
        .source_addresses
        .iter()
        .map(|address| {
            let address: [u8; 32] = hex::decode(address)
                .map_err(migration_error)?
                .try_into()
                .map_err(|_| ServiceError::InvalidInput("Invalid migration source".into()))?;
            let coin = assets
                .coins
                .iter()
                .find(|coin| coin.address == address && !coin.shielded)
                .ok_or_else(|| {
                    ServiceError::InvalidInput(
                        "A pending migration source is no longer available".into(),
                    )
                })?;
            Ok(quil_lattice_ct::confidential::shield::ShieldSource {
                address,
                amount: coin.amount,
            })
        })
        .collect::<Result<Vec<_>, ServiceError>>()?;
    let shard = record.shard.chars().map(|bit| bit == '1').collect();
    let batch = ShieldBatch { shard, sources };
    let recipient_context = token_context(
        &status.recipient_config_dir,
        &status.read_endpoint,
        &status.submit_endpoint,
        None,
    )?;
    let recipient = RecipientIdentity::load(&recipient_context).map_err(migration_error)?;
    let sender = SubmitIdentity::new(submit.key_manager.clone()).map_err(migration_error)?;
    let prepared = prepare_shield_batches(source, &recipient, &[batch], &[record.fee])
        .await
        .map_err(migration_error)?;
    journal
        .submit_current(&prepared[0].operation, |operation| async {
            submit_saved_shield(endpoints, &recipient, &sender, operation).await
        })
        .await
        .map_err(migration_error)
}

fn token_context(
    config_dir: &Path,
    read_endpoint: &str,
    submit_endpoint: &str,
    peer_override: Option<&str>,
) -> Result<TokenCtx, ServiceError> {
    let mut config = quil_config::load_config(config_dir)
        .map_err(|error| ServiceError::Config(error.to_string()))?;
    if let Some(peer) = peer_override {
        config.p2p.peer_priv_key = peer.into();
    }
    let key_manager = quil_client::keys::build_key_manager(&config, config_dir)
        .map_err(|error| ServiceError::Key(error.to_string()))?;
    let submit_key_manager = key_manager.clone();
    let mut read_flags = vec![
        "--config".into(),
        config_dir.to_string_lossy().into_owned(),
        "--read-rpc".into(),
        read_endpoint.into(),
    ];
    // Follow-up commands must accept the same plaintext read node as the console.
    if !quil_client::rpc::is_local_multiaddr(read_endpoint).map_err(migration_error)? {
        read_flags.push("--insecure-read-rpc".into());
    }
    Ok(TokenCtx {
        node_config: config,
        config_dir: config_dir.into(),
        key_manager,
        connect_opts: ConnectOpts {
            listen_grpc_multiaddr: read_endpoint.into(),
            ..Default::default()
        },
        submit_opts: ConnectOpts {
            listen_grpc_multiaddr: submit_endpoint.into(),
            ..Default::default()
        },
        submit_key_manager,
        submit_config_dir: config_dir.into(),
        submit_identity: Default::default(),
        peer_id_bytes: Vec::new(),
        max_fee: DEFAULT_MAX_AUTOMATIC_FEE,
        read_flags,
    })
}

fn canonical_dir(path: &Path) -> Result<PathBuf, ServiceError> {
    let path =
        std::fs::canonicalize(path).map_err(|error| ServiceError::Config(error.to_string()))?;
    if !path.is_dir() {
        return Err(ServiceError::Config(
            "Configuration path is not a directory".into(),
        ));
    }
    Ok(path)
}

fn migration_error(error: impl std::fmt::Display) -> ServiceError {
    ServiceError::Operation(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_context_preserves_wallet_and_read_node_in_follow_up_flags() {
        let home = tempfile::tempdir().unwrap();
        let config_dir = home.path().join("wallet config");
        let mut config = quil_config::Config::default();
        config.key.key_store_file.path = config_dir.join("keys.yml").to_string_lossy().into_owned();
        config.key.key_store_file.encryption_key = hex::encode([1; 32]);
        quil_config::save_config(&config_dir, &config).unwrap();
        let submit_endpoint = "/ip4/127.0.0.1/tcp/8337";

        for (read_endpoint, remote) in [
            ("/ip4/127.0.0.1/tcp/8837", false),
            ("/ip4/192.0.2.89/tcp/8837", true),
        ] {
            let context = token_context(&config_dir, read_endpoint, submit_endpoint, None).unwrap();
            let mut expected = vec![
                "--config".to_string(),
                config_dir.to_string_lossy().into_owned(),
                "--read-rpc".to_string(),
                read_endpoint.to_string(),
            ];
            if remote {
                expected.push("--insecure-read-rpc".to_string());
            }
            assert_eq!(context.read_flags, expected);
            assert_eq!(context.config_dir, config_dir);
            assert_eq!(context.submit_config_dir, config_dir);
            assert_eq!(context.connect_opts.listen_grpc_multiaddr, read_endpoint);
            assert_eq!(context.submit_opts.listen_grpc_multiaddr, submit_endpoint);
            assert!(std::sync::Arc::ptr_eq(
                &context.key_manager,
                &context.submit_key_manager
            ));
        }
    }

    #[test]
    fn transient_source_survives_start_boundary_until_completion() {
        let home = tempfile::tempdir().unwrap();
        let coordinator = LegacyMigrationCoordinator::new(home.path().into());
        coordinator.remember_source("fingerprint", Zeroizing::new("secret".into()));

        assert_eq!(
            coordinator
                .transient_source("fingerprint")
                .as_deref()
                .map(String::as_str),
            Some("secret")
        );
        coordinator.forget_source("fingerprint");
        assert!(coordinator.transient_source("fingerprint").is_none());
    }

    #[test]
    fn task_paths_accept_only_canonical_fingerprints() {
        let home = tempfile::tempdir().unwrap();
        let coordinator = LegacyMigrationCoordinator::new(home.path().into());
        assert!(coordinator.task_dir(&"ab".repeat(16)).is_ok());
        for value in ["../escape", "AB", "", &"a".repeat(31)] {
            assert!(coordinator.task_dir(value).is_err());
        }
    }

    #[test]
    fn recovery_action_matches_the_saved_submission_phase() {
        assert_eq!(
            recovery_action(&LegacyMigrationPhase::SubmissionUnknown).unwrap(),
            RecoveryAction::RebroadcastUnknown
        );
        assert_eq!(
            recovery_action(&LegacyMigrationPhase::RecoverableError).unwrap(),
            RecoveryAction::RetryRejected
        );
        assert!(recovery_action(&LegacyMigrationPhase::Prepared).is_err());
    }

    #[test]
    fn prepared_advance_resumes_at_prepare_and_submit_without_observing() {
        assert_eq!(
            advance_action(&LegacyMigrationPhase::Prepared),
            AdvanceAction::PrepareAndSubmit
        );
        assert_eq!(
            advance_action(&LegacyMigrationPhase::AwaitingChainConfirmation { batch: 1, total: 2 }),
            AdvanceAction::Observe
        );
        assert_eq!(
            advance_action(&LegacyMigrationPhase::SubmissionUnknown),
            AdvanceAction::Observe
        );
        assert_eq!(
            advance_action(&LegacyMigrationPhase::Complete),
            AdvanceAction::Return
        );
    }
}
