use std::{
    path::Path,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use quil_lattice_ct::confidential::{
    memo::create_output,
    mint::{Mint, MintStatement, RewardClaim},
    mint_claim::MintClaim,
    relation::CompiledAmountRelation,
};
use quil_types::{
    crypto::{KeyType, Signer},
    proto::node::{
        node_service_client::NodeServiceClient, GetMintAuthorizationWitnessRequest,
        GetMintAuthorizationWitnessResponse, GetProverRewardWitnessRequest,
        GetProverRewardWitnessResponse,
    },
};
use sha3::{Digest, Sha3_256};

use super::{
    decision::{self, OperationDecision},
    fees,
    mint_journal::{MintJournal, MintJournalError, MintJournalState, MINT_JOURNAL_VERSION},
    model::OwnedCoin,
    spend, submission, ServiceError, Wallet,
};
use crate::{
    domain::{RewardMintPhase, RewardMintPreflight, RewardMintStatus, WalletSubmissionOutcome},
    service::{node, prover, rewards, signing},
};

use tonic::transport::Channel;

const CLAIM_REBROADCAST_AFTER_MS: u64 = 10 * 60 * 1_000;

pub(super) struct PreparedAuthorization {
    pub(super) statement: MintStatement,
    pub(super) signatures: Vec<[u8; 666]>,
    pub(super) receipt: [u8; 32],
    relation: CompiledAmountRelation,
}

pub(super) fn decode_reward_claim(
    public_key: [u8; 897],
    response: GetProverRewardWitnessResponse,
) -> Result<(u64, [u8; 32], RewardClaim), ServiceError> {
    if !response.found {
        return Err(ServiceError::Rewards(
            "No claimable Prover reward was found".into(),
        ));
    }
    let reward_root: [u8; 32] = response.reward_root.try_into().map_err(|_| {
        ServiceError::Rewards("Reward witness does not contain a valid 32-byte root".into())
    })?;
    let value = u128::from_le_bytes(response.value.try_into().map_err(|_| {
        ServiceError::Rewards("Reward witness amount must be exactly 16 bytes".into())
    })?);
    let owner = quil_crypto::poseidon::hash_bytes_to_32(&public_key)
        .map_err(|error| ServiceError::Rewards(error.to_string()))?;
    quil_execution::token_intrinsic::reward_witness::verify_reward_membership(
        &owner,
        value,
        &reward_root,
        &response.forest_proof,
    )
    .map_err(|error| ServiceError::Rewards(format!("Reward witness validation failed: {error}")))?;
    Ok((
        response.cited_frame,
        reward_root,
        RewardClaim {
            owner,
            value,
            public_key,
            forest_proof: response.forest_proof,
        },
    ))
}

pub(super) fn prepare_authorization(
    wallet: &Wallet,
    cited_frame: u64,
    reward_root: [u8; 32],
    reward: RewardClaim,
    signer: Arc<dyn Signer>,
    fee: u128,
) -> Result<PreparedAuthorization, ServiceError> {
    if wallet.application != quil_execution::domains::QUIL_TOKEN {
        return Err(ServiceError::Wallet(
            "Reward Mint is available only for QUIL".into(),
        ));
    }
    if signer.key_type() != KeyType::Falcon512
        || signer.public_key() != reward.public_key
        || quil_crypto::poseidon::hash_bytes_to_32(&reward.public_key)
            .map_err(|error| ServiceError::Wallet(error.to_string()))?
            != reward.owner
    {
        return Err(ServiceError::Wallet(
            "Reward claimant does not match the selected Falcon key".into(),
        ));
    }
    let net_amount = reward
        .value
        .checked_sub(fee)
        .filter(|amount| *amount > 0)
        .ok_or_else(|| ServiceError::Wallet("Claimable reward must exceed the Mint fee".into()))?;
    let created = create_output(&wallet.context, &wallet.address, net_amount)
        .map_err(|error| ServiceError::Wallet(format!("Cannot create Mint output: {error:?}")))?;
    let statement = MintStatement {
        network: wallet.network,
        application: wallet.application,
        cited_frame,
        reward_root,
        fee,
        claims: vec![reward],
        outputs: vec![created.output],
    };
    let context = statement
        .context_bytes()
        .map_err(|error| ServiceError::Wallet(format!("Invalid Mint context: {error:?}")))?;
    let signature: [u8; 666] = signer
        .sign_with_domain(&context, &wallet.context)
        .map_err(|error| ServiceError::Wallet(format!("Cannot sign Mint: {error}")))?
        .try_into()
        .map_err(|_| ServiceError::Wallet("Falcon Mint signature has the wrong length".into()))?;
    if !quil_crypto::falcon_verify(
        &statement.claims[0].public_key,
        &signature,
        &context,
        &wallet.context,
    ) {
        return Err(ServiceError::Wallet(
            "Generated Mint claimant signature did not verify".into(),
        ));
    }
    let relation = statement
        .private_relation(&[(net_amount, &created.opening)], 1, 1)
        .map_err(|error| ServiceError::Wallet(format!("Invalid Mint relation: {error:?}")))?;
    let receipt = quil_execution::token_intrinsic::mint_authorization::receipt_address(&statement)
        .map_err(|error| ServiceError::Wallet(format!("Cannot derive Mint receipt: {error}")))?;
    Ok(PreparedAuthorization {
        statement,
        signatures: vec![signature],
        receipt,
        relation,
    })
}

pub(super) async fn prove_authorization(
    prepared: PreparedAuthorization,
) -> Result<Vec<u8>, ServiceError> {
    use quil_lattice_ct::confidential::relation::backend::native;

    let permit = spend::PROVING
        .acquire()
        .await
        .map_err(|error| ServiceError::Wallet(format!("Proving unavailable: {error}")))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let policy = quil_execution::token_intrinsic::dispatch::TokenPolicy::for_network(0);
        let proof = native::prove(&prepared.relation, policy.native_budget)
            .map_err(|error| ServiceError::Wallet(format!("Mint proving failed: {error:?}")))?;
        drop(prepared.relation);
        let mint = Mint {
            statement: prepared.statement,
            signatures: prepared.signatures,
            proof,
        };
        let bytes = mint
            .encode()
            .map_err(|error| ServiceError::Wallet(format!("Mint encoding failed: {error:?}")))?;
        let public = mint.statement.public_relation(1, 1).map_err(|error| {
            ServiceError::Wallet(format!("Mint verification setup failed: {error:?}"))
        })?;
        let valid =
            native::verify_owned(public, &mint.proof, policy.native_budget).map_err(|error| {
                ServiceError::Wallet(format!("Mint verification failed: {error:?}"))
            })?;
        if !valid {
            return Err(ServiceError::Wallet(
                "Generated Mint failed public verification".into(),
            ));
        }
        Ok(bytes)
    })
    .await
    .map_err(|error| ServiceError::Wallet(format!("Mint proving task failed: {error}")))?
}

fn encode_authorization_draft(prepared: &PreparedAuthorization) -> Result<String, ServiceError> {
    let mut placeholder_proof = vec![0_u8; 40];
    placeholder_proof[..8].copy_from_slice(b"QPF6\0\0\0\0");
    Mint {
        statement: prepared.statement.clone(),
        signatures: prepared.signatures.clone(),
        proof: placeholder_proof,
    }
    .encode()
    .map(hex::encode)
    .map_err(|error| ServiceError::Wallet(format!("Cannot persist Mint draft: {error:?}")))
}

fn recover_authorization_draft(
    wallet: &Wallet,
    state: &MintJournalState,
    claimant_public_key: &[u8; 897],
) -> Result<PreparedAuthorization, ServiceError> {
    let bytes = hex::decode(
        state
            .authorization_draft
            .as_deref()
            .ok_or_else(|| ServiceError::Wallet("Mint authorization draft is missing".into()))?,
    )
    .map_err(|_| ServiceError::Wallet("Mint authorization draft is invalid".into()))?;
    let mint = validate_authorization(wallet, state, &bytes, claimant_public_key, false)?;
    let opened = quil_lattice_ct::confidential::memo::open_output(
        &wallet.context,
        &wallet.kem_secret,
        &wallet.recipient,
        &mint.statement.outputs[0],
    )
    .map_err(|error| ServiceError::Wallet(format!("Cannot reopen Mint draft: {error:?}")))?;
    let relation = mint
        .statement
        .private_relation(&[(opened.amount, &opened.secrets.opening)], 1, 1)
        .map_err(|error| ServiceError::Wallet(format!("Invalid Mint draft relation: {error:?}")))?;
    let receipt =
        quil_execution::token_intrinsic::mint_authorization::receipt_address(&mint.statement)
            .map_err(|error| {
                ServiceError::Wallet(format!("Invalid Mint draft receipt: {error}"))
            })?;
    Ok(PreparedAuthorization {
        statement: mint.statement,
        signatures: mint.signatures,
        receipt,
        relation,
    })
}

pub(super) fn prepare_claim(
    statement: &MintStatement,
    expected_receipt: [u8; 32],
    response: GetMintAuthorizationWitnessResponse,
) -> Result<MintClaim, ServiceError> {
    let claim = quil_execution::token_intrinsic::mint_authorization::claim_from_witness(
        statement,
        quil_types::store::MintAuthorizationWitnessData {
            found: response.found,
            cited_frame: response.cited_frame,
            global_root: response.global_root,
            forest_proof: response.forest_proof,
        },
    )
    .map_err(|error| {
        ServiceError::Wallet(format!(
            "Mint authorization witness validation failed: {error}"
        ))
    })?;
    if claim.receipt != expected_receipt || claim.outputs != statement.outputs {
        return Err(ServiceError::Wallet(
            "Mint claim does not match the prepared authorization".into(),
        ));
    }
    Ok(claim)
}

pub(super) fn claim_is_finalized(claim: &MintClaim, coins: &[OwnedCoin]) -> bool {
    claim
        .outputs
        .iter()
        .all(|expected| coins.iter().any(|coin| coin.output == *expected))
}

pub(super) fn confirmation_id(preflight: &RewardMintPreflight) -> [u8; 32] {
    let mut digest = Sha3_256::new();
    digest.update(b"qclient-console/reward-mint-preflight/v1\0");
    put_bytes(&mut digest, preflight.network.as_bytes());
    put_bytes(
        &mut digest,
        preflight.config_dir.to_string_lossy().as_bytes(),
    );
    put_bytes(&mut digest, preflight.recipient.as_bytes());
    digest.update(preflight.reward_amount.to_be_bytes());
    digest.update(preflight.estimated_fee.to_be_bytes());
    digest.update(preflight.net_amount.to_be_bytes());
    digest.update(preflight.cited_global_frame.to_be_bytes());
    digest.finalize().into()
}

fn put_bytes(digest: &mut Sha3_256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_be_bytes());
    digest.update(bytes);
}

pub(super) fn next_read_delay(failures: u32) -> u64 {
    match failures {
        0 => 15,
        1 => 30,
        _ => 60,
    }
}

struct LiveMint {
    preflight: RewardMintPreflight,
    config_fingerprint: String,
    wallet: Arc<Wallet>,
    client: NodeServiceClient<Channel>,
    signer: Arc<dyn Signer>,
    reward_root: [u8; 32],
    reward: RewardClaim,
}

struct MintLease {
    _file: std::fs::File,
}

impl MintLease {
    fn try_acquire(config_dir: &Path) -> Result<Option<Self>, ServiceError> {
        let root = config_dir.join(".qclient-console");
        std::fs::create_dir_all(&root)
            .map_err(|error| ServiceError::Wallet(format!("Cannot create Mint lock: {error}")))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).map_err(
                |error| ServiceError::Wallet(format!("Cannot secure Mint lock directory: {error}")),
            )?;
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .open(root.join("mint.lock"))
                .map_err(|error| ServiceError::Wallet(format!("Cannot open Mint lock: {error}")))?;
            match file.try_lock() {
                Ok(()) => Ok(Some(Self { _file: file })),
                Err(std::fs::TryLockError::WouldBlock) => Ok(None),
                Err(std::fs::TryLockError::Error(error)) => Err(ServiceError::Wallet(format!(
                    "Cannot acquire Mint lock: {error}"
                ))),
            }
        }
        #[cfg(not(unix))]
        {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(root.join("mint.lock"))
                .map_err(|error| ServiceError::Wallet(format!("Cannot open Mint lock: {error}")))?;
            match file.try_lock() {
                Ok(()) => Ok(Some(Self { _file: file })),
                Err(std::fs::TryLockError::WouldBlock) => Ok(None),
                Err(std::fs::TryLockError::Error(error)) => Err(ServiceError::Wallet(format!(
                    "Cannot acquire Mint lock: {error}"
                ))),
            }
        }
    }
}

pub(super) async fn preflight(config_dir: &Path) -> Result<RewardMintPreflight, ServiceError> {
    let live = load_live_mint(config_dir).await?;
    let unfinished = MintJournal::scan_unfinished_read_only(
        &live.preflight.config_dir,
        &live.preflight.network,
        &live.config_fingerprint,
    )
    .map_err(journal_error)?;
    if unfinished.len() > 1 {
        return Err(ServiceError::Wallet(
            "More than one unfinished Reward Mint journal exists for this configuration".into(),
        ));
    }
    if !unfinished.is_empty() {
        return Err(ServiceError::Wallet(
            "An unfinished Reward Mint already exists and will resume automatically".into(),
        ));
    }
    Ok(live.preflight)
}

pub(super) async fn start(
    config_dir: &Path,
    approved: RewardMintPreflight,
) -> Result<RewardMintStatus, ServiceError> {
    let _lease = MintLease::try_acquire(config_dir)?.ok_or_else(|| {
        ServiceError::Wallet("Another Console is already advancing Reward Mint".into())
    })?;
    let mut live = load_live_mint(config_dir).await?;
    if live.preflight != approved || approved.confirmation_id != confirmation_id(&approved) {
        return Err(ServiceError::InvalidInput(
            "Reward Mint confirmation is stale; review the latest reward and fee".into(),
        ));
    }
    let unfinished = MintJournal::scan_unfinished(
        &approved.config_dir,
        &approved.network,
        &live.config_fingerprint,
    )
    .map_err(journal_error)?;
    if !unfinished.is_empty() {
        return Err(ServiceError::Wallet(
            "An unfinished Reward Mint already exists for this configuration".into(),
        ));
    }

    let prepared = prepare_authorization(
        &live.wallet,
        approved.cited_global_frame,
        live.reward_root,
        live.reward,
        live.signer,
        approved.estimated_fee,
    )?;
    let receipt = hex::encode(prepared.receipt);
    let canonical = approved.config_dir.clone();
    let journal = MintJournal::open(&approved.config_dir, &receipt).map_err(journal_error)?;
    let claimant = prepared
        .statement
        .claims
        .first()
        .ok_or_else(|| ServiceError::Wallet("Prepared Mint has no reward claimant".into()))?;
    let mut state = MintJournalState {
        version: MINT_JOURNAL_VERSION,
        console_version: env!("CARGO_PKG_VERSION").into(),
        build_commit: option_env!("QCLIENT_CONSOLE_BUILD_COMMIT")
            .unwrap_or("unknown")
            .into(),
        network: approved.network,
        config_fingerprint: live.config_fingerprint,
        receipt,
        cited_global_frame: approved.cited_global_frame,
        recipient: approved.recipient,
        reward_amount: approved.reward_amount,
        fee: approved.estimated_fee,
        net_amount: approved.net_amount,
        claimant_public_key: hex::encode(claimant.public_key),
        claimant_owner: hex::encode(claimant.owner),
        reward_root: hex::encode(prepared.statement.reward_root),
        authorization_draft: Some(encode_authorization_draft(&prepared)?),
        authorization_digest: None,
        authorization_proof_verified: false,
        phase: RewardMintPhase::ProvingAuthorization,
        authorization_file: None,
        claim_file: None,
        claim_submitted_unix_ms: None,
        rebroadcast_count: 0,
        submitted_claim_txids: Vec::new(),
        resume_phase: None,
        last_read_check_unix_ms: None,
        read_failure_count: 0,
        last_error: None,
    };
    journal.write_state(&state).map_err(journal_error)?;
    let authorization = prove_authorization(prepared).await?;
    journal
        .write_authorization(&authorization)
        .map_err(journal_error)?;
    state.phase = RewardMintPhase::AuthorizationPrepared;
    state.authorization_file = Some("authorization.bin".into());
    state.authorization_digest = Some(authorization_digest(&authorization));
    state.authorization_proof_verified = true;
    journal.write_state(&state).map_err(journal_error)?;
    if let Err(error) =
        fees::refresh_submission_fee(&mut live.client, &live.wallet.network, &authorization).await
    {
        return record_read_failure(
            &journal,
            state,
            &format!("Authorization fee check failed before submission: {error}"),
        );
    }
    let prepared_submission =
        match submission::prepare(&canonical, &live.wallet, authorization).await {
            Ok(prepared) => prepared,
            Err(error) => {
                return record_read_failure(
                    &journal,
                    state,
                    &format!("Authorization could not be prepared for submission: {error}"),
                )
            }
        };
    state.phase = RewardMintPhase::SubmittingAuthorization;
    journal.write_state(&state).map_err(journal_error)?;
    state.phase = match submission::dispatch(prepared_submission).await {
        Ok(WalletSubmissionOutcome::Submitted) => RewardMintPhase::AwaitingAuthorizationWitness,
        Ok(WalletSubmissionOutcome::Unknown) => RewardMintPhase::AuthorizationUnknown,
        Err(error) => {
            state.last_error = Some(error.to_string());
            RewardMintPhase::PermanentError
        }
    };
    if state.phase == RewardMintPhase::AuthorizationUnknown {
        state.last_error = Some(
            "Authorization submission result is unknown; only witness reads will be retried".into(),
        );
    }
    journal.write_state(&state).map_err(journal_error)?;
    Ok(status(&state))
}

pub(super) async fn load(config_dir: &Path) -> Result<Option<RewardMintStatus>, ServiceError> {
    let (canonical, network, fingerprint) = load_journal_context(config_dir)?;
    let lease = MintLease::try_acquire(config_dir)?;
    let states = if lease.is_some() {
        MintJournal::scan_unfinished(&canonical, &network, &fingerprint)
    } else {
        MintJournal::scan_unfinished_read_only(&canonical, &network, &fingerprint)
    }
    .map_err(journal_error)?;
    match states.as_slice() {
        [] => Ok(None),
        [state] => Ok(Some(status(state))),
        _ => Err(ServiceError::Wallet(
            "More than one unfinished Reward Mint journal exists for this configuration".into(),
        )),
    }
}

pub(super) async fn rebroadcast(
    config_dir: &Path,
    confirmed: bool,
) -> Result<RewardMintStatus, ServiceError> {
    if !confirmed {
        return Err(ServiceError::InvalidInput(
            "Explicit confirmation is required before rebroadcasting a saved Mint claim".into(),
        ));
    }
    let _lease = MintLease::try_acquire(config_dir)?.ok_or_else(|| {
        ServiceError::Conflict("Another Console is already advancing Reward Mint".into())
    })?;
    let (canonical, network, fingerprint, wallet, mut client, claimant_public_key) =
        load_runtime(config_dir).await?;
    let states =
        MintJournal::scan_unfinished(&canonical, &network, &fingerprint).map_err(journal_error)?;
    let mut state = match states.as_slice() {
        [state] => state.clone(),
        [] => {
            return Err(ServiceError::Wallet(
                "No unfinished Reward Mint exists for this configuration".into(),
            ))
        }
        _ => {
            return Err(ServiceError::Wallet(
                "More than one unfinished Reward Mint journal exists for this configuration".into(),
            ))
        }
    };
    let journal = MintJournal::open(&canonical, &state.receipt).map_err(journal_error)?;
    let saved_claim_bytes = journal.read_claim().map_err(journal_error)?;
    validate_claim(&wallet, &state, &saved_claim_bytes)?;

    match claim_decision(&wallet, &client, &saved_claim_bytes).await? {
        Some(OperationDecision::Committed { frame }) => {
            apply_claim_observation(
                &mut state,
                Some(OperationDecision::Committed { frame }),
                false,
            );
            journal.write_state(&state).map_err(journal_error)?;
            return Ok(status(&state));
        }
        Some(OperationDecision::Rejected { .. }) => {}
        None if !claim_rebroadcast_available(
            state.phase,
            state.resume_phase,
            state.claim_submitted_unix_ms,
            now_millis(),
        ) =>
        {
            return Err(ServiceError::Conflict(
                "The saved Mint claim is still pending; wait for the recovery timeout before rebroadcasting"
                    .into(),
            ));
        }
        None => {}
    }

    // The source attempt must be part of the durable history before claim.bin can be
    // overwritten. This is deliberately after the eligibility check: a premature Web call
    // against ClaimPrepared must not poison the first real submission.
    remember_claim_attempt(&mut state, &saved_claim_bytes);
    journal.write_state(&state).map_err(journal_error)?;

    let authorization = journal.read_authorization().map_err(journal_error)?;
    let digest = authorization_digest(&authorization);
    let mint = validate_authorization_async(
        wallet.clone(),
        state.clone(),
        authorization,
        claimant_public_key,
    )
    .await?;
    checkpoint_authorization_verification(&journal, &mut state, digest)?;
    let receipt = hex::decode(&state.receipt)
        .map_err(|error| ServiceError::Wallet(format!("Invalid Mint receipt: {error}")))?;
    let response = tokio::time::timeout(
        node::RPC_TIMEOUT,
        client
            .clone()
            .max_decoding_message_size(64 * 1024)
            .get_mint_authorization_witness(GetMintAuthorizationWitnessRequest { receipt }),
    )
    .await
    .map_err(|_| ServiceError::Timeout)?
    .map_err(|error| ServiceError::Rpc(error.to_string()))?
    .into_inner();
    if !response.found {
        return Err(ServiceError::Wallet(
            "The saved Mint authorization is not visible on the configured read RPC".into(),
        ));
    }
    let receipt =
        quil_execution::token_intrinsic::mint_authorization::receipt_address(&mint.statement)
            .map_err(|error| ServiceError::Wallet(error.to_string()))?;
    let claim = prepare_claim(&mint.statement, receipt, response)?;
    let claim_bytes = claim
        .encode()
        .map_err(|error| ServiceError::Wallet(format!("Mint claim encoding failed: {error:?}")))?;
    ensure_rebuilt_claim_is_new(&state, &saved_claim_bytes, &claim_bytes)?;
    journal.write_claim(&claim_bytes).map_err(journal_error)?;
    state.phase = RewardMintPhase::ClaimPrepared;
    state.resume_phase = None;
    state.claim_file = Some("claim.bin".into());
    state.rebroadcast_count = state.rebroadcast_count.saturating_add(1);
    state.last_error = None;
    journal.write_state(&state).map_err(journal_error)?;
    submit_claim(
        &canonical,
        &wallet,
        &mut client,
        &journal,
        &mut state,
        claim_bytes,
    )
    .await?;
    Ok(status(&state))
}

fn claim_txid_hex(claim: &[u8]) -> String {
    hex::encode(quil_execution::token_intrinsic::global_commit::tx_id(claim))
}

fn remember_claim_attempt(state: &mut MintJournalState, claim: &[u8]) {
    let txid = claim_txid_hex(claim);
    if !state.submitted_claim_txids.contains(&txid) {
        state.submitted_claim_txids.push(txid);
    }
}

fn ensure_rebuilt_claim_is_new(
    state: &MintJournalState,
    previous: &[u8],
    rebuilt: &[u8],
) -> Result<(), ServiceError> {
    let rebuilt_txid = claim_txid_hex(rebuilt);
    if claim_txid_hex(previous) == rebuilt_txid
        || state.submitted_claim_txids.contains(&rebuilt_txid)
    {
        return Err(ServiceError::Conflict(
            "The rebuilt Mint claim has a TxID that was already attempted; refusing to submit it again"
                .into(),
        ));
    }
    Ok(())
}

pub(super) async fn advance(config_dir: &Path) -> Result<RewardMintStatus, ServiceError> {
    let Some(_lease) = MintLease::try_acquire(config_dir)? else {
        let mut current = load(config_dir).await?.ok_or_else(|| {
            ServiceError::Wallet("No unfinished Reward Mint exists for this configuration".into())
        })?;
        current.detail = Some("Another Console is currently advancing Reward Mint".into());
        current.next_poll_in_secs = Some(15);
        return Ok(current);
    };
    let (canonical, network, fingerprint, wallet, mut client, claimant_public_key) =
        load_runtime(config_dir).await?;
    let states =
        MintJournal::scan_unfinished(&canonical, &network, &fingerprint).map_err(journal_error)?;
    let mut state = match states.as_slice() {
        [state] => state.clone(),
        [] => {
            return Err(ServiceError::Wallet(
                "No unfinished Reward Mint exists for this configuration".into(),
            ))
        }
        _ => {
            return Err(ServiceError::Wallet(
                "More than one unfinished Reward Mint journal exists for this configuration".into(),
            ))
        }
    };
    let journal = MintJournal::open(&canonical, &state.receipt).map_err(journal_error)?;

    if state.phase == RewardMintPhase::RecoverableError {
        state.phase = state
            .resume_phase
            .take()
            .ok_or_else(|| ServiceError::Wallet("Mint recovery phase is missing".into()))?;
        state.last_error = None;
        journal.write_state(&state).map_err(journal_error)?;
    }
    if state.phase == RewardMintPhase::SubmittingAuthorization {
        state.phase = RewardMintPhase::AuthorizationUnknown;
        state.last_error = Some(
            "Console stopped during authorization submission; the write will not be replayed"
                .into(),
        );
        journal.write_state(&state).map_err(journal_error)?;
    }
    if state.phase == RewardMintPhase::SubmittingClaim {
        state.phase = RewardMintPhase::ClaimUnknown;
        state.last_error =
            Some("Console stopped during claim submission; the write will not be replayed".into());
        journal.write_state(&state).map_err(journal_error)?;
    }

    match state.phase {
        RewardMintPhase::ProvingAuthorization => {
            let prepared = recover_authorization_draft(&wallet, &state, &claimant_public_key)?;
            let authorization = prove_authorization(prepared).await?;
            journal
                .write_authorization(&authorization)
                .map_err(journal_error)?;
            state.phase = RewardMintPhase::AuthorizationPrepared;
            state.authorization_file = Some("authorization.bin".into());
            state.authorization_digest = Some(authorization_digest(&authorization));
            state.authorization_proof_verified = true;
            state.last_error = None;
            journal.write_state(&state).map_err(journal_error)?;
        }
        RewardMintPhase::AuthorizationPrepared => {
            let authorization = journal.read_authorization().map_err(journal_error)?;
            let digest = authorization_digest(&authorization);
            validate_authorization_async(
                wallet.clone(),
                state.clone(),
                authorization.clone(),
                claimant_public_key,
            )
            .await?;
            checkpoint_authorization_verification(&journal, &mut state, digest)?;
            if let Err(error) =
                fees::refresh_submission_fee(&mut client, &wallet.network, &authorization).await
            {
                return record_read_failure(
                    &journal,
                    state,
                    &format!("Authorization fee check failed before submission: {error}"),
                );
            }
            let prepared_submission =
                match submission::prepare(&canonical, &wallet, authorization).await {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        return record_read_failure(
                            &journal,
                            state,
                            &format!("Authorization could not be prepared for submission: {error}"),
                        )
                    }
                };
            state.phase = RewardMintPhase::SubmittingAuthorization;
            journal.write_state(&state).map_err(journal_error)?;
            state.phase = match submission::dispatch(prepared_submission).await {
                Ok(WalletSubmissionOutcome::Submitted) => {
                    RewardMintPhase::AwaitingAuthorizationWitness
                }
                Ok(WalletSubmissionOutcome::Unknown) => RewardMintPhase::AuthorizationUnknown,
                Err(error) => {
                    state.last_error = Some(error.to_string());
                    RewardMintPhase::PermanentError
                }
            };
            if state.phase == RewardMintPhase::AuthorizationUnknown {
                state.last_error = Some(
                    "Authorization submission result is unknown; waiting for its witness".into(),
                );
            }
            journal.write_state(&state).map_err(journal_error)?;
        }
        RewardMintPhase::AwaitingAuthorizationWitness | RewardMintPhase::AuthorizationUnknown => {
            let authorization = journal.read_authorization().map_err(journal_error)?;
            let digest = authorization_digest(&authorization);
            let mint = validate_authorization_async(
                wallet.clone(),
                state.clone(),
                authorization,
                claimant_public_key,
            )
            .await?;
            checkpoint_authorization_verification(&journal, &mut state, digest)?;
            let receipt = hex::decode(&state.receipt)
                .map_err(|error| ServiceError::Wallet(error.to_string()))?;
            let response = tokio::time::timeout(
                node::RPC_TIMEOUT,
                client
                    .clone()
                    .max_decoding_message_size(64 * 1024)
                    .get_mint_authorization_witness(GetMintAuthorizationWitnessRequest { receipt }),
            )
            .await;
            let response = match response {
                Err(_) => {
                    return record_read_failure(
                        &journal,
                        state,
                        "Authorization witness query timed out",
                    )
                }
                Ok(Err(error)) => {
                    return record_read_failure(
                        &journal,
                        state,
                        &format!("Authorization witness query failed: {error}"),
                    )
                }
                Ok(Ok(response)) => response.into_inner(),
            };
            state.last_read_check_unix_ms = Some(now_millis());
            state.read_failure_count = 0;
            if !response.found {
                journal.write_state(&state).map_err(journal_error)?;
                return Ok(status(&state));
            }
            let receipt = quil_execution::token_intrinsic::mint_authorization::receipt_address(
                &mint.statement,
            )
            .map_err(|error| ServiceError::Wallet(error.to_string()))?;
            let claim = prepare_claim(&mint.statement, receipt, response)?;
            let claim_bytes = claim.encode().map_err(|error| {
                ServiceError::Wallet(format!("Mint claim encoding failed: {error:?}"))
            })?;
            journal.write_claim(&claim_bytes).map_err(journal_error)?;
            state.phase = RewardMintPhase::ClaimPrepared;
            state.claim_file = Some("claim.bin".into());
            state.last_error = None;
            journal.write_state(&state).map_err(journal_error)?;
            submit_claim(
                &canonical,
                &wallet,
                &mut client,
                &journal,
                &mut state,
                claim_bytes,
            )
            .await?;
        }
        RewardMintPhase::ClaimPrepared => {
            let claim_bytes = journal.read_claim().map_err(journal_error)?;
            validate_claim(&wallet, &state, &claim_bytes)?;
            submit_claim(
                &canonical,
                &wallet,
                &mut client,
                &journal,
                &mut state,
                claim_bytes,
            )
            .await?;
        }
        RewardMintPhase::ClaimSubmitted
        | RewardMintPhase::ClaimAccepted
        | RewardMintPhase::ClaimUnknown => {
            let claim_bytes = journal.read_claim().map_err(journal_error)?;
            let claim = validate_claim(&wallet, &state, &claim_bytes)?;
            let decision = match claim_decision(&wallet, &client, &claim_bytes).await {
                Ok(decision) => decision,
                Err(error) => {
                    return record_read_failure(
                        &journal,
                        state,
                        &format!("Claim decision query failed: {error}"),
                    )
                }
            };
            let credited = if matches!(decision, Some(OperationDecision::Rejected { .. })) {
                false
            } else {
                let coins = match wallet.clone().scan_unspent(&client).await {
                    Ok(coins) => coins,
                    Err(error) => {
                        return record_read_failure(
                            &journal,
                            state,
                            &format!("Claim finality scan failed: {error}"),
                        )
                    }
                };
                claim_is_finalized(&claim, &coins)
            };
            state.last_read_check_unix_ms = Some(now_millis());
            apply_claim_observation(&mut state, decision, credited);
            journal.write_state(&state).map_err(journal_error)?;
        }
        RewardMintPhase::PermanentError
        | RewardMintPhase::ClaimRejected
        | RewardMintPhase::ClaimConfirmed => {}
        RewardMintPhase::Preflighting
        | RewardMintPhase::ReadyToConfirm
        | RewardMintPhase::ProvingClaim
        | RewardMintPhase::RecoverableError
        | RewardMintPhase::SubmittingAuthorization
        | RewardMintPhase::SubmittingClaim => {
            return Err(ServiceError::Wallet(format!(
                "Reward Mint cannot advance from {:?}",
                state.phase
            )))
        }
    }
    Ok(status(&state))
}

async fn load_live_mint(config_dir: &Path) -> Result<LiveMint, ServiceError> {
    let config = quil_config::load_config(config_dir)
        .map_err(|error| ServiceError::Config(error.to_string()))?;
    let canonical = std::fs::canonicalize(config_dir)
        .map_err(|error| ServiceError::Config(error.to_string()))?;
    let manager = prover::build_key_manager(&config, &canonical)?;
    let resolved = rewards::resolve_prover_identity(&manager.list_keys())?;
    let rewards::ResolvedRewardIdentity::Falcon { key_id, public_key } = resolved else {
        return Err(ServiceError::Rewards(match resolved {
            rewards::ResolvedRewardIdentity::Unavailable => {
                "No Falcon reward identity is available".into()
            }
            rewards::ResolvedRewardIdentity::Ambiguous { candidates } => {
                format!("Reward identity is ambiguous: {candidates} Falcon keys are available")
            }
            rewards::ResolvedRewardIdentity::Falcon { .. } => unreachable!(),
        }));
    };
    let public: [u8; 897] = public_key
        .as_slice()
        .try_into()
        .map_err(|_| ServiceError::Key("Selected Falcon public key has the wrong length".into()))?;
    let signer = signing::falcon_signer(&manager, &key_id, &public_key)
        .map_err(|error| ServiceError::Key(error.to_string()))?;
    let signer: Arc<dyn Signer> = signer;
    let (wallet, mut client) =
        spend::load_wallet(&canonical, quil_execution::domains::QUIL_TOKEN).await?;
    let owner = quil_crypto::poseidon::hash_bytes_to_32(&public)
        .map_err(|error| ServiceError::Rewards(error.to_string()))?;
    let response = tokio::time::timeout(
        node::RPC_TIMEOUT,
        client
            .clone()
            .max_decoding_message_size(64 * 1024)
            .get_prover_reward_witness(GetProverRewardWitnessRequest {
                domain: quil_execution::domains::QUIL_TOKEN.to_vec(),
                owner_prover_address: owner.to_vec(),
            }),
    )
    .await
    .map_err(|_| ServiceError::Timeout)?
    .map_err(|error| ServiceError::Rpc(error.to_string()))?
    .into_inner();
    let (cited_frame, reward_root, reward) = decode_reward_claim(public, response)?;
    let growth = fees::growth_for_shape(
        &wallet.network,
        &wallet.application,
        quil_execution::token_intrinsic::cost::Shape {
            coins: 1,
            markers: 1,
            escrow: false,
        },
    )?;
    let fee = fees::quoted_quil_fee_for_cost(
        &mut client,
        &wallet.network,
        None,
        growth,
        true,
        reward.value.saturating_sub(1),
    )
    .await?;
    let net_amount = reward
        .value
        .checked_sub(fee)
        .filter(|amount| *amount > 0)
        .ok_or_else(|| ServiceError::Wallet("Claimable reward must exceed the Mint fee".into()))?;
    let network = config.p2p.network.to_string();
    let fingerprint = config_fingerprint(&canonical, &network, &wallet.address.encode());
    let mut preflight = RewardMintPreflight {
        confirmation_id: [0; 32],
        network,
        config_dir: canonical,
        recipient: hex::encode(wallet.address.encode()),
        reward_amount: reward.value,
        estimated_fee: fee,
        net_amount,
        cited_global_frame: cited_frame,
    };
    preflight.confirmation_id = confirmation_id(&preflight);
    Ok(LiveMint {
        preflight,
        config_fingerprint: fingerprint,
        wallet,
        client,
        signer,
        reward_root,
        reward,
    })
}

async fn load_runtime(
    config_dir: &Path,
) -> Result<
    (
        std::path::PathBuf,
        String,
        String,
        Arc<Wallet>,
        NodeServiceClient<Channel>,
        [u8; 897],
    ),
    ServiceError,
> {
    let config = quil_config::load_config(config_dir)
        .map_err(|error| ServiceError::Config(error.to_string()))?;
    let canonical = std::fs::canonicalize(config_dir)
        .map_err(|error| ServiceError::Config(error.to_string()))?;
    let manager = prover::build_key_manager(&config, &canonical)?;
    let resolved = rewards::resolve_prover_identity(&manager.list_keys())?;
    let rewards::ResolvedRewardIdentity::Falcon { public_key, .. } = resolved else {
        return Err(ServiceError::Rewards(match resolved {
            rewards::ResolvedRewardIdentity::Unavailable => {
                "No Falcon reward identity is available".into()
            }
            rewards::ResolvedRewardIdentity::Ambiguous { candidates } => {
                format!("Reward identity is ambiguous: {candidates} Falcon keys are available")
            }
            rewards::ResolvedRewardIdentity::Falcon { .. } => unreachable!(),
        }));
    };
    let claimant_public_key: [u8; 897] = public_key
        .as_slice()
        .try_into()
        .map_err(|_| ServiceError::Key("Selected Falcon public key has the wrong length".into()))?;
    let (wallet, client) =
        spend::load_wallet(&canonical, quil_execution::domains::QUIL_TOKEN).await?;
    let network = config.p2p.network.to_string();
    let fingerprint = config_fingerprint(&canonical, &network, &wallet.address.encode());
    Ok((
        canonical,
        network,
        fingerprint,
        wallet,
        client,
        claimant_public_key,
    ))
}

fn load_journal_context(
    config_dir: &Path,
) -> Result<(std::path::PathBuf, String, String), ServiceError> {
    let config = quil_config::load_config(config_dir)
        .map_err(|error| ServiceError::Config(error.to_string()))?;
    let canonical = std::fs::canonicalize(config_dir)
        .map_err(|error| ServiceError::Config(error.to_string()))?;
    let manager = prover::build_key_manager(&config, &canonical)?;
    let secret = zeroize::Zeroizing::new(
        manager
            .get_secret_key_bytes_by_id("q-onion-key")
            .map_err(|error| ServiceError::Wallet(format!("q-onion-key secret: {error}")))?,
    );
    let public = manager
        .get_public_key_bytes_by_id("q-onion-key")
        .map_err(|error| ServiceError::Wallet(format!("q-onion-key public: {error}")))?;
    let wallet = Wallet::from_keys(
        quil_lattice_ct::confidential::transfer::network_identifier(config.p2p.network),
        quil_execution::domains::QUIL_TOKEN,
        &public,
        secret,
    )?;
    let network = config.p2p.network.to_string();
    let fingerprint = config_fingerprint(&canonical, &network, &wallet.address.encode());
    Ok((canonical, network, fingerprint))
}

fn config_fingerprint(config_dir: &Path, network: &str, recipient: &[u8]) -> String {
    let mut digest = Sha3_256::new();
    digest.update(b"qclient-console/mint-config/v1\0");
    put_bytes(&mut digest, config_dir.to_string_lossy().as_bytes());
    put_bytes(&mut digest, network.as_bytes());
    put_bytes(&mut digest, recipient);
    hex::encode(digest.finalize())
}

async fn validate_authorization_async(
    wallet: Arc<Wallet>,
    state: MintJournalState,
    bytes: Vec<u8>,
    claimant_public_key: [u8; 897],
) -> Result<Mint, ServiceError> {
    let current_digest = authorization_digest(&bytes);
    let verify_native_proof = !state.authorization_proof_verified
        || state.authorization_digest.as_deref() != Some(current_digest.as_str());
    tokio::task::spawn_blocking(move || {
        validate_authorization(
            &wallet,
            &state,
            &bytes,
            &claimant_public_key,
            verify_native_proof,
        )
    })
    .await
    .map_err(|error| ServiceError::Wallet(format!("Mint verification task failed: {error}")))?
}

fn validate_authorization(
    wallet: &Wallet,
    state: &MintJournalState,
    bytes: &[u8],
    claimant_public_key: &[u8; 897],
    verify_native_proof: bool,
) -> Result<Mint, ServiceError> {
    let digest = authorization_digest(bytes);
    if state
        .authorization_digest
        .as_ref()
        .is_some_and(|expected| expected != &digest)
    {
        return Err(ServiceError::Wallet(
            "Persisted Mint authorization digest does not match its journal".into(),
        ));
    }
    let mint = Mint::decode(bytes, &wallet.network, &wallet.application).map_err(|error| {
        ServiceError::Wallet(format!(
            "Persisted Mint authorization is invalid: {error:?}"
        ))
    })?;
    let receipt =
        quil_execution::token_intrinsic::mint_authorization::receipt_address(&mint.statement)
            .map_err(|error| ServiceError::Wallet(error.to_string()))?;
    let expected_reward_root: [u8; 32] = hex::decode(&state.reward_root)
        .map_err(|_| ServiceError::Wallet("Mint journal reward root is invalid".into()))?
        .try_into()
        .map_err(|_| {
            ServiceError::Wallet("Mint journal reward root has the wrong length".into())
        })?;
    if hex::encode(receipt) != state.receipt
        || mint.statement.cited_frame != state.cited_global_frame
        || mint.statement.fee != state.fee
        || mint.statement.total().ok() != Some(state.reward_amount)
        || mint.statement.outputs.len() != 1
    {
        return Err(ServiceError::Wallet(
            "Persisted Mint authorization does not match its journal".into(),
        ));
    }
    validate_claimant_binding(
        &mint.statement,
        mint.signatures.len(),
        state,
        claimant_public_key,
        expected_reward_root,
    )?;
    let context = mint
        .statement
        .context_bytes()
        .map_err(|error| ServiceError::Wallet(format!("Invalid Mint context: {error:?}")))?;
    if mint.signatures.len() != mint.statement.claims.len()
        || !mint
            .statement
            .claims
            .iter()
            .zip(&mint.signatures)
            .all(|(claim, signature)| {
                quil_crypto::poseidon::hash_bytes_to_32(&claim.public_key).ok() == Some(claim.owner)
                    && quil_crypto::falcon_verify(
                        &claim.public_key,
                        signature,
                        &context,
                        &wallet.context,
                    )
                    && quil_execution::token_intrinsic::reward_witness::verify_reward_membership(
                        &claim.owner,
                        claim.value,
                        &mint.statement.reward_root,
                        &claim.forest_proof,
                    )
                    .is_ok()
            })
    {
        return Err(ServiceError::Wallet(
            "Persisted Mint reward proof or claimant signature is invalid".into(),
        ));
    }
    if verify_native_proof {
        let public = mint
            .statement
            .public_relation(1, 1)
            .map_err(|error| ServiceError::Wallet(format!("Invalid Mint relation: {error:?}")))?;
        let budget =
            quil_execution::token_intrinsic::dispatch::TokenPolicy::for_network(0).native_budget;
        if !quil_lattice_ct::confidential::relation::backend::native::verify_owned(
            public,
            &mint.proof,
            budget,
        )
        .map_err(|error| ServiceError::Wallet(format!("Mint verification failed: {error:?}")))?
        {
            return Err(ServiceError::Wallet(
                "Persisted Mint authorization proof did not verify".into(),
            ));
        }
    }
    validate_output_for_wallet(wallet, state, &mint.statement.outputs[0])?;
    Ok(mint)
}

fn authorization_digest(bytes: &[u8]) -> String {
    let mut digest = Sha3_256::new();
    digest.update(b"qclient-console/mint-authorization/v1\0");
    digest.update(bytes);
    hex::encode(digest.finalize())
}

fn checkpoint_authorization_verification(
    journal: &MintJournal,
    state: &mut MintJournalState,
    digest: String,
) -> Result<(), ServiceError> {
    if !state.authorization_proof_verified || state.authorization_digest.as_deref() != Some(&digest)
    {
        state.authorization_digest = Some(digest);
        state.authorization_proof_verified = true;
        journal.write_state(state).map_err(journal_error)?;
    }
    Ok(())
}

fn validate_claimant_binding(
    statement: &MintStatement,
    signature_count: usize,
    state: &MintJournalState,
    claimant_public_key: &[u8; 897],
    expected_reward_root: [u8; 32],
) -> Result<(), ServiceError> {
    if statement.claims.len() != 1
        || signature_count != 1
        || statement.reward_root != expected_reward_root
    {
        return Err(ServiceError::Wallet(
            "Persisted Mint does not contain the exact confirmed reward claim".into(),
        ));
    }
    let claim = &statement.claims[0];
    if claim.public_key != *claimant_public_key
        || state.claimant_public_key != hex::encode(claim.public_key)
        || state.claimant_owner != hex::encode(claim.owner)
        || claim.value != state.reward_amount
    {
        return Err(ServiceError::Wallet(
            "Persisted Mint claimant does not match the active Falcon reward identity".into(),
        ));
    }
    Ok(())
}

fn validate_claim(
    wallet: &Wallet,
    state: &MintJournalState,
    bytes: &[u8],
) -> Result<MintClaim, ServiceError> {
    let claim =
        MintClaim::decode(bytes, &wallet.network, &wallet.application).map_err(|error| {
            ServiceError::Wallet(format!("Persisted Mint claim is invalid: {error:?}"))
        })?;
    if hex::encode(claim.receipt) != state.receipt
        || claim.fee != state.fee
        || claim.outputs.len() != 1
    {
        return Err(ServiceError::Wallet(
            "Persisted Mint claim does not match its journal".into(),
        ));
    }
    quil_execution::token_intrinsic::mint_authorization::verify_membership(
        &claim.network,
        &claim.application,
        &claim.receipt,
        &claim.outputs,
        claim.fee,
        &claim.global_root,
        &claim.forest_proof,
    )
    .map_err(|error| {
        ServiceError::Wallet(format!("Persisted Mint claim witness is invalid: {error}"))
    })?;
    validate_output_for_wallet(wallet, state, &claim.outputs[0])?;
    Ok(claim)
}

fn validate_output_for_wallet(
    wallet: &Wallet,
    state: &MintJournalState,
    output: &quil_lattice_ct::confidential::transfer::Output,
) -> Result<(), ServiceError> {
    if state.recipient != hex::encode(wallet.address.encode()) {
        return Err(ServiceError::Wallet(
            "Mint journal recipient is not the current configuration wallet".into(),
        ));
    }
    let opened = quil_lattice_ct::confidential::memo::open_output(
        &wallet.context,
        &wallet.kem_secret,
        &wallet.recipient,
        output,
    )
    .map_err(|error| {
        ServiceError::Wallet(format!(
            "Mint output is not owned by the current wallet: {error:?}"
        ))
    })?;
    if opened.amount != state.net_amount {
        return Err(ServiceError::Wallet(
            "Mint output amount does not match its journal".into(),
        ));
    }
    Ok(())
}

async fn submit_claim(
    config_dir: &Path,
    wallet: &Wallet,
    client: &mut NodeServiceClient<Channel>,
    journal: &MintJournal,
    state: &mut MintJournalState,
    bytes: Vec<u8>,
) -> Result<(), ServiceError> {
    let claim_txid = claim_txid_hex(&bytes);
    if let Err(error) = fees::refresh_submission_fee(client, &wallet.network, &bytes).await {
        state.phase = RewardMintPhase::RecoverableError;
        state.resume_phase = Some(RewardMintPhase::ClaimPrepared);
        state.last_error = Some(format!("Claim fee check failed before submission: {error}"));
        journal.write_state(state).map_err(journal_error)?;
        return Ok(());
    }
    let prepared_submission = match submission::prepare(config_dir, wallet, bytes).await {
        Ok(prepared) => prepared,
        Err(error) => {
            state.phase = RewardMintPhase::RecoverableError;
            state.resume_phase = Some(RewardMintPhase::ClaimPrepared);
            state.last_error = Some(format!(
                "Claim could not be prepared for submission: {error}"
            ));
            journal.write_state(state).map_err(journal_error)?;
            return Ok(());
        }
    };
    if state.submitted_claim_txids.contains(&claim_txid) {
        return Err(ServiceError::Conflict(
            "This Mint claim TxID was already attempted; refusing to submit it again".into(),
        ));
    }
    state.submitted_claim_txids.push(claim_txid);
    state.phase = RewardMintPhase::SubmittingClaim;
    state.claim_submitted_unix_ms = Some(now_millis());
    journal.write_state(state).map_err(journal_error)?;
    state.phase = match submission::dispatch(prepared_submission).await {
        Ok(WalletSubmissionOutcome::Submitted) => RewardMintPhase::ClaimSubmitted,
        Ok(WalletSubmissionOutcome::Unknown) => RewardMintPhase::ClaimUnknown,
        Err(error) => {
            state.last_error = Some(error.to_string());
            RewardMintPhase::ClaimRejected
        }
    };
    if state.phase == RewardMintPhase::ClaimUnknown {
        state.last_error =
            Some("Claim submission result is unknown; the write will not be replayed".into());
    }
    journal.write_state(state).map_err(journal_error)?;
    Ok(())
}

async fn claim_decision(
    wallet: &Wallet,
    client: &NodeServiceClient<Channel>,
    claim_bytes: &[u8],
) -> Result<Option<OperationDecision>, ServiceError> {
    let tx_id = quil_execution::token_intrinsic::global_commit::tx_id(claim_bytes);
    let address = quil_execution::token_intrinsic::global_commit::decision_address(
        &wallet.application,
        &tx_id,
    )
    .map_err(|error| ServiceError::Wallet(error.to_string()))?;
    let (present, raw_data) = wallet.global_record(client, &address).await?;
    if !present {
        return Ok(None);
    }
    decision::parse_record(&raw_data)
        .map(Some)
        .map_err(|error| ServiceError::Wallet(format!("Invalid claim decision record: {error}")))
}

fn apply_claim_observation(
    state: &mut MintJournalState,
    decision: Option<OperationDecision>,
    credited: bool,
) {
    state.read_failure_count = 0;
    state.resume_phase = None;
    state.last_error = None;
    if credited {
        state.phase = RewardMintPhase::ClaimConfirmed;
        return;
    }
    match decision {
        Some(OperationDecision::Committed { frame }) => {
            state.phase = RewardMintPhase::ClaimAccepted;
            state.last_error = Some(format!(
                "Claim was accepted at GLOBAL frame {frame}; its Coin is not visible yet. Verify that the configured read RPC carries complete QUIL token state"
            ));
        }
        Some(OperationDecision::Rejected { frame, reason }) => {
            state.phase = RewardMintPhase::ClaimRejected;
            state.last_error = Some(format!(
                "Claim was rejected at GLOBAL frame {frame}: {reason}"
            ));
        }
        None => {}
    }
}

fn record_read_failure(
    journal: &MintJournal,
    mut state: MintJournalState,
    message: &str,
) -> Result<RewardMintStatus, ServiceError> {
    let resume = state.phase;
    state.read_failure_count = state.read_failure_count.saturating_add(1);
    state.last_read_check_unix_ms = Some(now_millis());
    state.phase = RewardMintPhase::RecoverableError;
    state.resume_phase = Some(resume);
    state.last_error = Some(message.into());
    journal.write_state(&state).map_err(journal_error)?;
    Ok(status(&state))
}

fn status(state: &MintJournalState) -> RewardMintStatus {
    let polling = phase_needs_poll(state.phase);
    RewardMintStatus {
        receipt: state.receipt.clone(),
        recipient: state.recipient.clone(),
        phase: state.phase,
        resume_phase: state.resume_phase,
        detail: state.last_error.clone(),
        next_poll_in_secs: polling.then(|| next_read_delay(state.read_failure_count)),
        can_rebroadcast: claim_rebroadcast_available(
            state.phase,
            state.resume_phase,
            state.claim_submitted_unix_ms,
            now_millis(),
        ),
    }
}

fn phase_needs_poll(phase: RewardMintPhase) -> bool {
    matches!(
        phase,
        RewardMintPhase::ProvingAuthorization
            | RewardMintPhase::AuthorizationPrepared
            | RewardMintPhase::AwaitingAuthorizationWitness
            | RewardMintPhase::AuthorizationUnknown
            | RewardMintPhase::ClaimPrepared
            | RewardMintPhase::ClaimSubmitted
            | RewardMintPhase::ClaimAccepted
            | RewardMintPhase::ClaimUnknown
            | RewardMintPhase::RecoverableError
    )
}

fn claim_rebroadcast_available(
    phase: RewardMintPhase,
    resume_phase: Option<RewardMintPhase>,
    submitted_unix_ms: Option<u64>,
    now_unix_ms: u64,
) -> bool {
    if phase == RewardMintPhase::ClaimRejected {
        return true;
    }
    let waiting = matches!(
        phase,
        RewardMintPhase::ClaimSubmitted | RewardMintPhase::ClaimUnknown
    ) || (phase == RewardMintPhase::RecoverableError
        && matches!(
            resume_phase,
            Some(RewardMintPhase::ClaimSubmitted | RewardMintPhase::ClaimUnknown)
        ));
    waiting
        && submitted_unix_ms.is_none_or(|submitted| {
            now_unix_ms.saturating_sub(submitted) >= CLAIM_REBROADCAST_AFTER_MS
        })
}

fn journal_error(error: MintJournalError) -> ServiceError {
    ServiceError::Wallet(error.to_string())
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use quil_lattice_ct::confidential::{
        mint::RewardClaim,
        mint_claim::MintClaim,
        relation::membership::IDENTITY_BYTES,
        transfer::{Output, MEMO_BYTES},
        AmountCommitment, COMMITMENT_BYTES,
    };
    use quil_types::crypto::Signer;
    use sha3::{Digest, Sha3_384};
    use zeroize::Zeroizing;

    use super::{
        apply_claim_observation, claim_is_finalized, claim_rebroadcast_available, confirmation_id,
        decode_reward_claim, encode_authorization_draft, ensure_rebuilt_claim_is_new,
        next_read_delay, phase_needs_poll, prepare_authorization, prepare_claim,
        remember_claim_attempt, validate_claimant_binding, MintLease, OperationDecision,
        CLAIM_REBROADCAST_AFTER_MS,
    };
    use crate::domain::{RewardMintPhase, RewardMintPreflight};
    use crate::service::wallet::decision;
    use crate::service::wallet::{
        mint_journal::{MintJournalState, MINT_JOURNAL_VERSION},
        model::OwnedCoin,
        Wallet,
    };

    fn wallet() -> Wallet {
        let keys = quil_crypto::sntrup761::Sntrup761KeyPair::generate();
        Wallet::from_keys(
            [1; 32],
            quil_execution::domains::QUIL_TOKEN,
            &keys.public,
            Zeroizing::new(keys.secret),
        )
        .unwrap()
    }

    fn output(marker: u8) -> Output {
        Output {
            commitment: AmountCommitment::from_bytes(&[marker; COMMITMENT_BYTES]).unwrap(),
            owner: [marker; IDENTITY_BYTES],
            memo: [marker; MEMO_BYTES],
        }
    }

    fn test_journal_state() -> MintJournalState {
        MintJournalState {
            version: MINT_JOURNAL_VERSION,
            console_version: "0.1.0".into(),
            build_commit: "test-commit".into(),
            network: "mainnet".into(),
            config_fingerprint: "config".into(),
            receipt: "ab".repeat(32),
            cited_global_frame: 7,
            recipient: "recipient".into(),
            reward_amount: 19,
            fee: 2,
            net_amount: 17,
            claimant_public_key: "11".repeat(897),
            claimant_owner: "22".repeat(32),
            reward_root: "33".repeat(32),
            authorization_draft: None,
            authorization_digest: None,
            authorization_proof_verified: true,
            phase: RewardMintPhase::ClaimSubmitted,
            authorization_file: Some("authorization.bin".into()),
            claim_file: Some("claim.bin".into()),
            claim_submitted_unix_ms: Some(1_000),
            rebroadcast_count: 0,
            submitted_claim_txids: Vec::new(),
            resume_phase: None,
            last_read_check_unix_ms: None,
            read_failure_count: 0,
            last_error: None,
        }
    }

    #[test]
    fn prepares_authorization_for_current_wallet_and_falcon_claimant() {
        let wallet = wallet();
        let signer = Arc::new(quil_crypto::FalconSigner::generate());
        let public_key: [u8; 897] = signer.public_key().try_into().unwrap();
        let reward = RewardClaim {
            owner: quil_crypto::poseidon::hash_bytes_to_32(&public_key).unwrap(),
            value: 19,
            public_key,
            forest_proof: vec![1],
        };
        let signer: Arc<dyn Signer> = signer;

        let prepared = prepare_authorization(&wallet, 7, [8; 32], reward, signer, 2).unwrap();

        assert_eq!(prepared.statement.network, wallet.network);
        assert_eq!(prepared.statement.application, wallet.application);
        assert_eq!(prepared.statement.cited_frame, 7);
        assert_eq!(prepared.statement.fee, 2);
        assert_eq!(prepared.statement.total().unwrap(), 19);
        assert_eq!(prepared.statement.outputs.len(), 1);
        assert_eq!(
            prepared.receipt,
            quil_execution::token_intrinsic::mint_authorization::receipt_address(
                &prepared.statement
            )
            .unwrap()
        );
        assert!(quil_crypto::falcon_verify(
            &public_key,
            &prepared.signatures[0],
            &prepared.statement.context_bytes().unwrap(),
            &wallet.context,
        ));
        let opened = quil_lattice_ct::confidential::memo::open_output(
            &wallet.context,
            &wallet.kem_secret,
            &wallet.recipient,
            &prepared.statement.outputs[0],
        )
        .unwrap();
        assert_eq!(opened.amount, 17);
        let draft = hex::decode(encode_authorization_draft(&prepared).unwrap()).unwrap();
        let decoded = quil_lattice_ct::confidential::mint::Mint::decode(
            &draft,
            &wallet.network,
            &wallet.application,
        )
        .unwrap();
        assert_eq!(decoded.statement, prepared.statement);
        assert_eq!(decoded.signatures, prepared.signatures);
    }

    #[test]
    fn configuration_lease_prevents_parallel_mint_advancement() {
        let config_dir = tempfile::tempdir().unwrap();
        let first = MintLease::try_acquire(config_dir.path()).unwrap().unwrap();
        assert!(MintLease::try_acquire(config_dir.path()).unwrap().is_none());
        drop(first);
        assert!(MintLease::try_acquire(config_dir.path()).unwrap().is_some());
    }

    #[test]
    fn persisted_authorization_is_bound_to_one_active_falcon_claimant() {
        let wallet = wallet();
        let signer = Arc::new(quil_crypto::FalconSigner::generate());
        let public_key: [u8; 897] = signer.public_key().try_into().unwrap();
        let reward = RewardClaim {
            owner: quil_crypto::poseidon::hash_bytes_to_32(&public_key).unwrap(),
            value: 19,
            public_key,
            forest_proof: vec![1],
        };
        let signer: Arc<dyn Signer> = signer;
        let prepared = prepare_authorization(&wallet, 7, [8; 32], reward, signer, 2).unwrap();
        let claim = &prepared.statement.claims[0];
        let state = MintJournalState {
            version: MINT_JOURNAL_VERSION,
            console_version: "0.1.0".into(),
            build_commit: "test-commit".into(),
            network: "mainnet".into(),
            config_fingerprint: "config".into(),
            receipt: hex::encode(prepared.receipt),
            cited_global_frame: 7,
            recipient: hex::encode(wallet.address.encode()),
            reward_amount: 19,
            fee: 2,
            net_amount: 17,
            claimant_public_key: hex::encode(claim.public_key),
            claimant_owner: hex::encode(claim.owner),
            reward_root: hex::encode(prepared.statement.reward_root),
            authorization_draft: None,
            authorization_digest: None,
            authorization_proof_verified: false,
            phase: RewardMintPhase::AuthorizationPrepared,
            authorization_file: Some("authorization.bin".into()),
            claim_file: None,
            claim_submitted_unix_ms: None,
            rebroadcast_count: 0,
            submitted_claim_txids: Vec::new(),
            resume_phase: None,
            last_read_check_unix_ms: None,
            read_failure_count: 0,
            last_error: None,
        };

        assert!(validate_claimant_binding(
            &prepared.statement,
            prepared.signatures.len(),
            &state,
            &public_key,
            [8; 32],
        )
        .is_ok());
        assert!(validate_claimant_binding(
            &prepared.statement,
            prepared.signatures.len(),
            &state,
            &[9; 897],
            [8; 32],
        )
        .is_err());

        let mut multiple_claims = prepared.statement;
        multiple_claims
            .claims
            .push(multiple_claims.claims[0].clone());
        assert!(
            validate_claimant_binding(&multiple_claims, 2, &state, &public_key, [8; 32]).is_err()
        );
    }

    #[test]
    fn rejects_reward_response_that_is_absent_or_structurally_invalid() {
        let public = [7; 897];
        assert!(decode_reward_claim(
            public,
            quil_types::proto::node::GetProverRewardWitnessResponse::default(),
        )
        .is_err());
        assert!(decode_reward_claim(
            public,
            quil_types::proto::node::GetProverRewardWitnessResponse {
                found: true,
                reward_root: vec![1; 31],
                value: 1_u128.to_le_bytes().to_vec(),
                ..Default::default()
            },
        )
        .is_err());
    }

    #[test]
    fn claim_preparation_rejects_missing_or_mismatched_witness() {
        let wallet = wallet();
        let signer = Arc::new(quil_crypto::FalconSigner::generate());
        let public_key: [u8; 897] = signer.public_key().try_into().unwrap();
        let reward = RewardClaim {
            owner: quil_crypto::poseidon::hash_bytes_to_32(&public_key).unwrap(),
            value: 19,
            public_key,
            forest_proof: vec![1],
        };
        let signer: Arc<dyn Signer> = signer;
        let prepared = prepare_authorization(&wallet, 7, [8; 32], reward, signer, 2).unwrap();

        assert!(prepare_claim(
            &prepared.statement,
            prepared.receipt,
            quil_types::proto::node::GetMintAuthorizationWitnessResponse::default(),
        )
        .is_err());
        assert!(prepare_claim(
            &prepared.statement,
            prepared.receipt,
            quil_types::proto::node::GetMintAuthorizationWitnessResponse {
                found: true,
                cited_frame: 9,
                global_root: vec![4; 31],
                forest_proof: vec![1, 0, 0, 0],
            },
        )
        .is_err());
    }

    #[test]
    fn claim_preparation_accepts_only_the_exact_authorization_record() {
        let wallet = wallet();
        let signer = Arc::new(quil_crypto::FalconSigner::generate());
        let public_key: [u8; 897] = signer.public_key().try_into().unwrap();
        let reward = RewardClaim {
            owner: quil_crypto::poseidon::hash_bytes_to_32(&public_key).unwrap(),
            value: 19,
            public_key,
            forest_proof: vec![1],
        };
        let signer: Arc<dyn Signer> = signer;
        let prepared = prepare_authorization(&wallet, 7, [8; 32], reward, signer, 2).unwrap();
        let context = quil_lattice_ct::confidential::transfer::parameter_context(
            &prepared.statement.network,
            &prepared.statement.application,
        );
        let mut outputs_hash = Sha3_384::new();
        outputs_hash.update(b"quil/mint/authorized-outputs/v4\0");
        outputs_hash.update(context);
        outputs_hash.update((prepared.statement.outputs.len() as u16).to_le_bytes());
        for output in &prepared.statement.outputs {
            outputs_hash.update(output.commitment.to_bytes());
            outputs_hash.update(output.owner);
            outputs_hash.update(output.memo);
        }
        let mut tag = b"quil/mint/authorization-record/v4\0".to_vec();
        tag.extend_from_slice(&context);
        let kind = quil_crypto::poseidon::hash_bytes_to_32(&tag).unwrap();
        let mut tree = quil_tries::VectorCommitmentTree::new();
        for (key, value) in [
            (vec![0xff; 32], kind.to_vec()),
            (vec![0], context.to_vec()),
            (vec![4], outputs_hash.finalize().to_vec()),
            (vec![8], prepared.statement.fee.to_be_bytes().to_vec()),
        ] {
            tree.insert(&key, &value, &[], &num_bigint::BigInt::from(value.len()))
                .unwrap();
        }
        let blob = quil_tries::serialize_go_tree(tree.root.as_ref()).unwrap();
        let forest = quil_forest::Forest::in_memory();
        let root = forest
            .commit_shard_phase_raw(
                b"console-mint-test",
                quil_forest::Phase::VertexAdds,
                0,
                [(
                    prepared.receipt.to_vec(),
                    quil_tries::vertex_leaf_value(&blob).unwrap(),
                )],
            )
            .unwrap();
        let mut address = quil_execution::domains::GLOBAL.to_vec();
        address.extend_from_slice(&prepared.receipt);
        let membership = forest
            .build_vertex_membership_proof(
                b"console-mint-test",
                quil_forest::Phase::VertexAdds,
                0,
                &address,
                &blob,
            )
            .unwrap();
        let proof = quil_forest::MembershipProof {
            inputs: vec![membership],
        }
        .to_bytes();
        let response = quil_types::proto::node::GetMintAuthorizationWitnessResponse {
            found: true,
            cited_frame: 9,
            global_root: root.to_vec(),
            forest_proof: proof,
        };

        let claim = prepare_claim(&prepared.statement, prepared.receipt, response.clone()).unwrap();

        assert_eq!(claim.receipt, prepared.receipt);
        assert_eq!(claim.outputs, prepared.statement.outputs);
        let mut changed = prepared.statement.clone();
        changed.outputs[0].memo[0] ^= 1;
        assert!(prepare_claim(&changed, prepared.receipt, response).is_err());
    }

    #[test]
    fn finality_requires_an_exact_claim_output_not_only_the_owner() {
        let expected = output(1);
        let claim = MintClaim {
            network: [1; 32],
            application: quil_execution::domains::QUIL_TOKEN,
            cited_global_frame: 9,
            global_root: [4; 32],
            receipt: [5; 32],
            fee: 2,
            outputs: vec![expected.clone()],
            forest_proof: vec![1],
        };
        let owned = |output: Output| OwnedCoin {
            address: [9; 32],
            output,
            amount: Zeroizing::new(17),
            image: [8; IDENTITY_BYTES],
        };

        assert!(!claim_is_finalized(&claim, &[owned(output(2))]));

        let mut wrong_memo = expected.clone();
        wrong_memo.memo[0] ^= 1;
        assert!(!claim_is_finalized(&claim, &[owned(wrong_memo)]));

        assert!(claim_is_finalized(&claim, &[owned(expected)]));
    }

    #[test]
    fn malformed_claim_decision_records_are_rejected() {
        assert!(decision::parse_record(b"not-a-tree").is_err());
    }

    #[test]
    fn claim_rebroadcast_needs_a_terminal_rejection_or_a_ten_minute_timeout() {
        let submitted = 1_000;
        assert!(!claim_rebroadcast_available(
            RewardMintPhase::ClaimSubmitted,
            None,
            Some(submitted),
            submitted + CLAIM_REBROADCAST_AFTER_MS - 1,
        ));
        assert!(claim_rebroadcast_available(
            RewardMintPhase::ClaimSubmitted,
            None,
            Some(submitted),
            submitted + CLAIM_REBROADCAST_AFTER_MS,
        ));
        assert!(claim_rebroadcast_available(
            RewardMintPhase::ClaimRejected,
            None,
            None,
            submitted,
        ));
        assert!(claim_rebroadcast_available(
            RewardMintPhase::ClaimUnknown,
            None,
            None,
            submitted,
        ));
        assert!(!claim_rebroadcast_available(
            RewardMintPhase::ClaimAccepted,
            None,
            Some(submitted),
            submitted + CLAIM_REBROADCAST_AFTER_MS,
        ));
        assert!(!claim_rebroadcast_available(
            RewardMintPhase::ClaimPrepared,
            None,
            None,
            submitted,
        ));
        assert!(!claim_rebroadcast_available(
            RewardMintPhase::RecoverableError,
            Some(RewardMintPhase::ClaimPrepared),
            None,
            submitted,
        ));
        assert!(claim_rebroadcast_available(
            RewardMintPhase::RecoverableError,
            Some(RewardMintPhase::ClaimUnknown),
            Some(submitted),
            submitted + CLAIM_REBROADCAST_AFTER_MS,
        ));
    }

    #[test]
    fn mint_recovery_refuses_to_submit_the_same_claim_txid_twice() {
        let claim = b"canonical saved mint claim";
        let mut state = test_journal_state();
        remember_claim_attempt(&mut state, claim);
        remember_claim_attempt(&mut state, b"second witness claim");

        let error =
            ensure_rebuilt_claim_is_new(&state, b"second witness claim", claim).unwrap_err();
        assert!(error.to_string().contains("already attempted"));
        assert!(ensure_rebuilt_claim_is_new(
            &state,
            b"second witness claim",
            b"fresh witness claim"
        )
        .is_ok());
    }

    #[test]
    fn claim_observation_distinguishes_pending_accepted_rejected_and_credited() {
        let state = MintJournalState {
            version: MINT_JOURNAL_VERSION,
            console_version: "0.1.0".into(),
            build_commit: "test-commit".into(),
            network: "mainnet".into(),
            config_fingerprint: "config".into(),
            receipt: "ab".repeat(32),
            cited_global_frame: 7,
            recipient: "recipient".into(),
            reward_amount: 19,
            fee: 2,
            net_amount: 17,
            claimant_public_key: "11".repeat(897),
            claimant_owner: "22".repeat(32),
            reward_root: "33".repeat(32),
            authorization_draft: None,
            authorization_digest: None,
            authorization_proof_verified: true,
            phase: RewardMintPhase::ClaimSubmitted,
            authorization_file: Some("authorization.bin".into()),
            claim_file: Some("claim.bin".into()),
            claim_submitted_unix_ms: Some(1_000),
            rebroadcast_count: 0,
            submitted_claim_txids: Vec::new(),
            resume_phase: None,
            last_read_check_unix_ms: None,
            read_failure_count: 3,
            last_error: Some("old read failure".into()),
        };

        let mut pending = state.clone();
        apply_claim_observation(&mut pending, None, false);
        assert_eq!(pending.phase, RewardMintPhase::ClaimSubmitted);
        assert_eq!(pending.last_error, None);

        let mut accepted = state.clone();
        apply_claim_observation(
            &mut accepted,
            Some(OperationDecision::Committed { frame: 88 }),
            false,
        );
        assert_eq!(accepted.phase, RewardMintPhase::ClaimAccepted);
        assert!(accepted.last_error.as_deref().unwrap().contains("88"));

        let mut rejected = state.clone();
        apply_claim_observation(
            &mut rejected,
            Some(OperationDecision::Rejected {
                frame: 89,
                reason: "invalid witness".into(),
            }),
            false,
        );
        assert_eq!(rejected.phase, RewardMintPhase::ClaimRejected);
        assert!(rejected
            .last_error
            .as_deref()
            .unwrap()
            .contains("invalid witness"));

        let mut credited = state;
        apply_claim_observation(
            &mut credited,
            Some(OperationDecision::Committed { frame: 90 }),
            true,
        );
        assert_eq!(credited.phase, RewardMintPhase::ClaimConfirmed);
        assert_eq!(credited.last_error, None);
        assert_eq!(credited.read_failure_count, 0);
    }

    #[test]
    fn confirmation_fingerprint_binds_every_displayed_mint_field() {
        let mut preflight = RewardMintPreflight {
            confirmation_id: [0; 32],
            network: "mainnet".into(),
            config_dir: "/node/config".into(),
            recipient: "qct3-recipient".into(),
            reward_amount: 1_000,
            estimated_fee: 25,
            net_amount: 975,
            cited_global_frame: 42,
        };
        let original = confirmation_id(&preflight);
        preflight.estimated_fee += 1;
        preflight.net_amount -= 1;

        assert_ne!(confirmation_id(&preflight), original);
    }

    #[test]
    fn witness_read_backoff_caps_at_sixty_seconds() {
        assert_eq!(next_read_delay(0), 15);
        assert_eq!(next_read_delay(1), 30);
        assert_eq!(next_read_delay(2), 60);
        assert_eq!(next_read_delay(20), 60);
    }

    #[test]
    fn every_durable_recovery_phase_is_scheduled() {
        for phase in [
            RewardMintPhase::ProvingAuthorization,
            RewardMintPhase::AuthorizationPrepared,
            RewardMintPhase::AwaitingAuthorizationWitness,
            RewardMintPhase::AuthorizationUnknown,
            RewardMintPhase::ClaimPrepared,
            RewardMintPhase::ClaimSubmitted,
            RewardMintPhase::ClaimUnknown,
            RewardMintPhase::RecoverableError,
        ] {
            assert!(phase_needs_poll(phase), "{phase:?} must resume");
        }
        for phase in [
            RewardMintPhase::SubmittingAuthorization,
            RewardMintPhase::SubmittingClaim,
            RewardMintPhase::ClaimConfirmed,
            RewardMintPhase::PermanentError,
        ] {
            assert!(
                !phase_needs_poll(phase),
                "{phase:?} must be reconciled first"
            );
        }
    }
}
