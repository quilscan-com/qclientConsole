use std::{path::Path, sync::Arc};

use quil_lattice_ct::confidential::{
    address::{RecipientAddress, ADDRESS_BYTES},
    memo::{
        create_escrow_recovery, create_output, open_escrow_recovery, open_output, CreatedOutput,
        EscrowRecoveryMemo,
    },
    pending_claim::{ClaimBranch, EscrowPolicy},
    pending_create::PendingCreateStatement,
    relation::membership::{InputPath, MembershipKey, MAX_DEPTH},
    transfer::{CompileLimits, Output, TransferStatement},
    MAX_PRIVATE_COINS,
};
use quil_types::{crypto::Signer, proto::node::GetNodeInfoRequest};
use zeroize::Zeroizing;

use crate::domain::{
    PendingEscrowBranch, WalletAction, WalletActionRequest, WalletOperationResult,
    WalletOperationStatus, WalletPreflight,
};

use super::{
    fees, model::select_coins, node, prover, spend, submission, OwnedCoin, ServiceError, Wallet,
};

const DEFAULT_REFUND_FRAMES: u64 = 8640;
const FEE_REBUILDS: usize = 3;

struct EscrowDestination {
    recipient: RecipientAddress,
    refund: RecipientAddress,
    amount: u128,
    policy: EscrowPolicy,
}

pub(super) async fn preflight_pending(
    config_dir: &Path,
    request: &WalletActionRequest,
) -> Result<WalletPreflight, ServiceError> {
    let application = spend::identifier(&request.application_hex, "application")?;
    let (wallet, mut client) = spend::load_wallet(config_dir, application).await?;
    let limits = spend::compile_limits();
    match request.action {
        WalletAction::PendingTransfer => {
            let amount = request
                .amount
                .ok_or_else(|| {
                    ServiceError::InvalidInput("Pending transfer amount is required".into())
                })?
                .base_units();
            let _ = parse_escrow_address(&request.target, &wallet.context)?;
            let fee = operation_fee(
                &wallet,
                request,
                &mut client,
                quil_execution::token_intrinsic::cost::Shape {
                    coins: 1,
                    markers: limits.max_inputs,
                    escrow: true,
                },
            )
            .await?;
            let refund_frame = match request.refund_after_global_frame {
                Some(frame) => frame,
                None => current_global_frame(&mut client)
                    .await?
                    .checked_add(DEFAULT_REFUND_FRAMES)
                    .ok_or_else(|| ServiceError::Wallet("Refund frame overflows".into()))?,
            };
            let coins = wallet.clone().scan_unspent(&client).await?;
            let (selected, change) = select_coins(coins, amount, fee, limits.max_inputs)?;
            pending_summary(
                request,
                amount,
                fee,
                *change,
                selected.len(),
                Some(refund_frame),
            )
        }
        WalletAction::Accept | WalletAction::Reject => {
            if request.amount.is_some() {
                return Err(ServiceError::InvalidInput(
                    "Accept and Reject derive the amount from the selected pending transfer".into(),
                ));
            }
            let escrow_address = spend::identifier(&request.target, "escrow address")?;
            let stored = fetch_escrow(&wallet, &client, escrow_address).await?;
            ensure_escrow_open(&wallet, &client, &escrow_address).await?;
            let current = if request.action == WalletAction::Reject {
                current_global_frame(&mut client).await?
            } else {
                0
            };
            let branch = if request.action == WalletAction::Accept {
                PendingEscrowBranch::Recipient
            } else {
                PendingEscrowBranch::Refund
            };
            validate_claim_branch(
                request.action,
                branch,
                stored.policy.refund_after_global_frame,
                current,
            )?;
            let recovery = claim_recovery(&stored, branch);
            let opened = open_escrow_recovery(
                &wallet.context,
                &wallet.kem_secret,
                &wallet.recipient,
                &stored.output.commitment,
                &recovery,
            )
            .map_err(|error| {
                ServiceError::Wallet(format!(
                    "Selected pending transfer is not owned by this wallet: {error:?}"
                ))
            })?;
            let fee = operation_fee(
                &wallet,
                request,
                &mut client,
                quil_execution::token_intrinsic::cost::Shape {
                    coins: 1,
                    markers: 1,
                    escrow: false,
                },
            )
            .await?;
            let output = opened
                .amount()
                .checked_sub(fee)
                .filter(|amount| *amount > 0)
                .ok_or_else(|| {
                    ServiceError::Wallet("Pending transfer amount must exceed the fee".into())
                })?;
            pending_summary(
                request,
                opened.amount(),
                fee,
                output,
                1,
                Some(stored.policy.refund_after_global_frame),
            )
        }
        WalletAction::Transfer => Err(ServiceError::InvalidInput(
            "Transfer must use the transfer service".into(),
        )),
    }
}

pub(super) async fn execute_pending(
    config_dir: &Path,
    request: &WalletActionRequest,
    approved: &WalletPreflight,
) -> Result<WalletOperationResult, ServiceError> {
    validate_pending_approval(request, approved)?;
    let application = spend::identifier(&request.application_hex, "application")?;
    if application != quil_execution::domains::QUIL_TOKEN {
        return Err(ServiceError::Operation(
            "Custom-token pending operations require the settlement payment workflow, which is not enabled yet"
                .into(),
        ));
    }
    let (wallet, mut client) = spend::load_wallet(config_dir, application).await?;
    let signer = falcon_signer(config_dir)?;
    let limits = spend::compile_limits();

    for attempt in 1..=FEE_REBUILDS {
        let shape = match request.action {
            WalletAction::PendingTransfer => quil_execution::token_intrinsic::cost::Shape {
                coins: 1,
                markers: limits.max_inputs,
                escrow: true,
            },
            WalletAction::Accept | WalletAction::Reject => {
                quil_execution::token_intrinsic::cost::Shape {
                    coins: 1,
                    markers: 1,
                    escrow: false,
                }
            }
            WalletAction::Transfer => unreachable!("approval validation rejects transfer"),
        };
        let fee = operation_fee(&wallet, request, &mut client, shape).await?;
        if fee > approved.maximum_fee_base_units {
            return Err(ServiceError::Wallet(format!(
                "Current fee {fee} exceeds the approved maximum {}",
                approved.maximum_fee_base_units
            )));
        }
        let bytes = match request.action {
            WalletAction::PendingTransfer => {
                let amount = request.amount.expect("validated amount").base_units();
                let (recipient, recipient_authority) =
                    parse_escrow_address(&request.target, &wallet.context)?;
                let refund_after_global_frame =
                    approved.refund_after_global_frame.ok_or_else(|| {
                        ServiceError::Wallet("Approved refund frame is missing".into())
                    })?;
                let coins = wallet.clone().scan_unspent(&client).await?;
                let (selected, change) = select_coins(coins, amount, fee, limits.max_inputs)?;
                let refund_authority = signer
                    .public_key()
                    .try_into()
                    .map_err(|_| ServiceError::Wallet("Invalid refund authority key".into()))?;
                let escrow = EscrowDestination {
                    recipient,
                    refund: wallet.address.clone(),
                    amount,
                    policy: EscrowPolicy {
                        recipient: recipient_authority,
                        refund: refund_authority,
                        refund_after_global_frame,
                    },
                };
                let mut change_outputs = Vec::new();
                if *change != 0 {
                    change_outputs.push((wallet.address.clone(), *change));
                }
                create_pending(
                    wallet.clone(),
                    &client,
                    selected,
                    escrow,
                    change_outputs,
                    fee,
                    limits,
                )
                .await?
            }
            WalletAction::Accept | WalletAction::Reject => {
                let escrow_address = spend::identifier(&request.target, "escrow address")?;
                let stored = fetch_escrow(&wallet, &client, escrow_address).await?;
                ensure_escrow_open(&wallet, &client, &escrow_address).await?;
                if Some(stored.policy.refund_after_global_frame)
                    != approved.refund_after_global_frame
                {
                    return Err(ServiceError::InvalidInput(
                        "Selected pending transfer changed after confirmation".into(),
                    ));
                }
                let (branch, global_frame) = if request.action == WalletAction::Accept {
                    (ClaimBranch::Recipient, None)
                } else {
                    let current = current_global_frame(&mut client).await?;
                    validate_claim_branch(
                        request.action,
                        PendingEscrowBranch::Refund,
                        stored.policy.refund_after_global_frame,
                        current,
                    )?;
                    (ClaimBranch::Refund, Some(current))
                };
                let recovery = claim_recovery(
                    &stored,
                    if branch == ClaimBranch::Recipient {
                        PendingEscrowBranch::Recipient
                    } else {
                        PendingEscrowBranch::Refund
                    },
                );
                let opened = open_escrow_recovery(
                    &wallet.context,
                    &wallet.kem_secret,
                    &wallet.recipient,
                    &stored.output.commitment,
                    &recovery,
                )
                .map_err(|error| {
                    ServiceError::Wallet(format!("Cannot open pending transfer: {error:?}"))
                })?;
                if opened.amount() != approved.amount_base_units {
                    return Err(ServiceError::InvalidInput(
                        "Pending-transfer amount changed after confirmation".into(),
                    ));
                }
                let amount = opened
                    .amount()
                    .checked_sub(fee)
                    .filter(|amount| *amount > 0)
                    .ok_or_else(|| {
                        ServiceError::Wallet("Pending transfer amount must exceed the fee".into())
                    })?;
                drop(opened);
                let signer: Arc<dyn Signer> = signer.clone();
                create_pending_claim(
                    wallet.clone(),
                    escrow_address,
                    stored,
                    branch,
                    global_frame,
                    signer,
                    vec![(wallet.address.clone(), amount)],
                    fee,
                    1,
                )
                .await?
            }
            WalletAction::Transfer => unreachable!("approval validation rejects transfer"),
        };
        match fees::refresh_submission_fee(&mut client, &wallet.network, &bytes).await {
            Ok(()) => {
                let submitted =
                    submission::submit(config_dir, &wallet, &client, request.action, bytes).await?;
                let outcome = submitted.outcome;
                let operation = match request.action {
                    WalletAction::PendingTransfer => "Pending transfer",
                    WalletAction::Accept => "Pending transfer acceptance",
                    WalletAction::Reject => "Pending transfer refund",
                    WalletAction::Transfer => unreachable!(),
                };
                let message = match submitted.operation.phase {
                    WalletOperationStatus::Committed => submitted
                        .operation
                        .detail
                        .clone()
                        .unwrap_or_else(|| format!("{operation} committed by GLOBAL")),
                    WalletOperationStatus::Rejected => submitted
                        .operation
                        .detail
                        .clone()
                        .unwrap_or_else(|| format!("{operation} rejected by GLOBAL")),
                    WalletOperationStatus::Submitting => {
                        format!("{operation} is being submitted")
                    }
                    WalletOperationStatus::Submitted => format!(
                        "{operation} submitted; finalized inclusion is not yet confirmed"
                    ),
                    WalletOperationStatus::Unknown => format!(
                        "{operation} submission result is unknown; refresh wallet state before trying again"
                    ),
                };
                return Ok(WalletOperationResult {
                    action: request.action,
                    outcome,
                    message,
                    operation: submitted.operation,
                });
            }
            Err(_) if request.explicit_fee_base_units.is_none() && attempt < FEE_REBUILDS => {}
            Err(error) => return Err(error),
        }
    }
    Err(ServiceError::Operation(
        "Pending operation could not be rebuilt within the fee refresh limit".into(),
    ))
}

fn falcon_signer(config_dir: &Path) -> Result<Arc<quil_crypto::FalconSigner>, ServiceError> {
    let config = quil_config::load_config(config_dir)
        .map_err(|error| ServiceError::Config(error.to_string()))?;
    let manager = prover::build_key_manager(&config, config_dir)?;
    let secret = Zeroizing::new(
        manager
            .get_secret_key_bytes_by_id("q-prover-key")
            .map_err(|error| ServiceError::Wallet(format!("q-prover-key secret: {error}")))?,
    );
    let public = manager
        .get_public_key_bytes_by_id("q-prover-key")
        .map_err(|error| ServiceError::Wallet(format!("q-prover-key public: {error}")))?;
    if secret.len() != quil_crypto::FALCON_SIGNING_KEY_LEN
        || public.len() != quil_crypto::FALCON_PUBLIC_KEY_LEN
        || quil_crypto::falcon_public_from_signing_key(&secret).as_deref()
            != Some(public.as_slice())
    {
        return Err(ServiceError::Wallet(
            "q-prover-key is not a matching Falcon-512 keypair".into(),
        ));
    }
    Ok(Arc::new(quil_crypto::FalconSigner::from_bytes(
        &secret, &public,
    )))
}

#[allow(clippy::too_many_arguments)]
fn prepare_pending_create(
    wallet: &Wallet,
    coins: &[([u8; 32], &Output)],
    root: &quil_lattice_ct::confidential::coin_tree::RootRecord,
    paths: &[quil_lattice_ct::confidential::coin_tree::AuthPath],
    escrow: &EscrowDestination,
    change: &[(&RecipientAddress, u128)],
    fee: u128,
    limits: CompileLimits,
) -> Result<
    (
        PendingCreateStatement,
        quil_lattice_ct::confidential::relation::CompiledAmountRelation,
    ),
    ServiceError,
> {
    if escrow.refund.context() != &wallet.context
        || wallet.application != quil_execution::domains::QUIL_TOKEN && fee != 0
        || change.len() >= limits.max_outputs.min(MAX_PRIVATE_COINS)
    {
        return Err(ServiceError::Wallet(
            "Pending transfer dimensions, context, or fee are invalid".into(),
        ));
    }
    let mut recipients = Vec::with_capacity(change.len() + 1);
    recipients.push((&escrow.recipient, escrow.amount));
    recipients.extend_from_slice(change);
    let (funding, relation, refund_recovery) = prepare_funding(
        wallet,
        coins,
        root,
        paths,
        &recipients,
        fee,
        limits,
        &escrow.refund,
    )?;
    let statement = PendingCreateStatement {
        funding,
        policy: escrow.policy.clone(),
        refund_recovery,
    };
    let context = statement.context_bytes().map_err(|error| {
        ServiceError::Wallet(format!("Cannot bind pending-transfer context: {error:?}"))
    })?;
    Ok((statement, relation.with_transaction_context(&context)))
}

#[allow(clippy::too_many_arguments)]
async fn create_pending(
    wallet: Arc<Wallet>,
    client: &quil_types::proto::node::node_service_client::NodeServiceClient<
        tonic::transport::Channel,
    >,
    coins: Vec<OwnedCoin>,
    escrow: EscrowDestination,
    change: Vec<(RecipientAddress, u128)>,
    fee: u128,
    limits: CompileLimits,
) -> Result<Vec<u8>, ServiceError> {
    use quil_lattice_ct::confidential::{pending_create::PendingCreate, relation::backend::native};

    let permit = spend::PROVING
        .acquire()
        .await
        .map_err(|error| ServiceError::Wallet(format!("Proving unavailable: {error}")))?;
    let selected: Vec<_> = coins
        .iter()
        .map(|coin| (coin.address, &coin.output))
        .collect();
    let (root, paths) = wallet.witnesses(client, &selected).await?;
    drop(selected);
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let selected: Vec<_> = coins
            .iter()
            .map(|coin| (coin.address, &coin.output))
            .collect();
        let change: Vec<_> = change
            .iter()
            .map(|(address, amount)| (address, *amount))
            .collect();
        let (statement, relation) = prepare_pending_create(
            &wallet, &selected, &root, &paths, &escrow, &change, fee, limits,
        )?;
        let budget =
            quil_execution::token_intrinsic::dispatch::TokenPolicy::for_network(0).native_budget;
        let proof = native::prove(&relation, budget).map_err(|error| {
            ServiceError::Wallet(format!("Pending-transfer proving failed: {error:?}"))
        })?;
        drop(relation);
        let operation = PendingCreate { statement, proof };
        let bytes = operation.encode().map_err(|error| {
            ServiceError::Wallet(format!("Pending-transfer encoding failed: {error:?}"))
        })?;
        let public = operation
            .statement
            .public_relation(limits)
            .map_err(|error| {
                ServiceError::Wallet(format!(
                    "Pending-transfer verification setup failed: {error:?}"
                ))
            })?;
        let valid = native::verify_owned(public, &operation.proof, budget).map_err(|error| {
            ServiceError::Wallet(format!("Pending-transfer verification failed: {error:?}"))
        })?;
        if !valid {
            return Err(ServiceError::Wallet(
                "Generated pending transfer failed public verification".into(),
            ));
        }
        Ok(bytes)
    })
    .await
    .map_err(|error| ServiceError::Wallet(format!("Pending proving task failed: {error}")))?
}

#[allow(clippy::too_many_arguments)]
fn prepare_funding(
    wallet: &Wallet,
    coins: &[([u8; 32], &Output)],
    root: &quil_lattice_ct::confidential::coin_tree::RootRecord,
    paths: &[quil_lattice_ct::confidential::coin_tree::AuthPath],
    recipients: &[(&RecipientAddress, u128)],
    fee: u128,
    limits: CompileLimits,
    refund: &RecipientAddress,
) -> Result<
    (
        TransferStatement,
        quil_lattice_ct::confidential::relation::CompiledAmountRelation,
        EscrowRecoveryMemo,
    ),
    ServiceError,
> {
    if root.context != wallet.context
        || coins.is_empty()
        || coins.len() > limits.max_inputs.min(MAX_PRIVATE_COINS)
        || paths.len() != coins.len()
        || recipients.is_empty()
        || recipients.len() > limits.max_outputs.min(MAX_PRIVATE_COINS)
        || root.depth == 0
        || usize::from(root.depth) > limits.max_depth.min(MAX_DEPTH)
    {
        return Err(ServiceError::Wallet(
            "Pending transfer dimensions exceed configured limits".into(),
        ));
    }
    if coins
        .iter()
        .map(|(address, _)| address)
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != coins.len()
    {
        return Err(ServiceError::Wallet("Duplicate selected Coins".into()));
    }
    if recipients
        .iter()
        .any(|(address, _)| address.context() != &wallet.context)
    {
        return Err(ServiceError::Wallet(
            "Recipient belongs to another network or application".into(),
        ));
    }
    let opened = coins
        .iter()
        .map(|(_, output)| {
            open_output(
                &wallet.context,
                &wallet.kem_secret,
                &wallet.recipient,
                output,
            )
            .map_err(|error| ServiceError::Wallet(format!("Cannot open input Coin: {error:?}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if total(opened.iter().map(|coin| coin.amount))
        != total(
            recipients
                .iter()
                .map(|(_, amount)| *amount)
                .chain(std::iter::once(fee)),
        )
    {
        return Err(ServiceError::Wallet(
            "Inputs must equal outputs plus fee".into(),
        ));
    }
    let key = MembershipKey::derive(&wallet.context);
    let images = opened
        .iter()
        .map(|coin| {
            key.key_image(&coin.secrets.owner)
                .identity_bytes()
                .map_err(|error| ServiceError::Wallet(format!("Invalid input image: {error:?}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut refund_recovery = None;
    let created = recipients
        .iter()
        .enumerate()
        .map(|(index, (address, amount))| {
            if index == 0 {
                let escrow = create_escrow_recovery(&wallet.context, address, refund, *amount)
                    .map_err(|error| {
                        ServiceError::Wallet(format!(
                            "Cannot create pending-transfer recovery: {error:?}"
                        ))
                    })?;
                refund_recovery = Some(escrow.refund);
                return Ok(CreatedOutput {
                    opening: escrow.opening,
                    output: Output {
                        commitment: escrow.commitment,
                        owner: escrow.recipient.owner,
                        memo: escrow.recipient.ciphertext,
                    },
                });
            }
            create_output(&wallet.context, address, *amount).map_err(|error| {
                ServiceError::Wallet(format!("Cannot create change output: {error:?}"))
            })
        })
        .collect::<Result<Vec<_>, ServiceError>>()?;
    let statement = TransferStatement {
        network: wallet.network,
        application: wallet.application,
        depth: root.depth,
        root: root.root.clone(),
        images,
        outputs: created.iter().map(|coin| coin.output.clone()).collect(),
        fee,
    };
    let input_witness: Vec<_> = opened
        .iter()
        .zip(coins)
        .map(|(coin, (_, output))| (coin.amount, &coin.secrets.opening, &output.commitment))
        .collect();
    let output_witness: Vec<_> = recipients
        .iter()
        .zip(&created)
        .map(|((_, amount), coin)| (*amount, &coin.opening))
        .collect();
    let input_paths: Vec<_> = opened
        .iter()
        .zip(paths)
        .map(|(coin, path)| InputPath {
            owner: &coin.secrets.owner,
            siblings: &path.siblings,
            right: &path.right,
        })
        .collect();
    let relation = statement
        .private_relation(&input_witness, &output_witness, &input_paths, limits)
        .map_err(|error| {
            ServiceError::Wallet(format!("Invalid pending-transfer witness: {error:?}"))
        })?;
    Ok((
        statement,
        relation,
        refund_recovery.expect("first output always creates refund recovery"),
    ))
}

fn total(amounts: impl Iterator<Item = u128>) -> Zeroizing<[u128; 2]> {
    let mut sum = Zeroizing::new([0u128; 2]);
    for amount in amounts {
        let (low, carry) = sum[0].overflowing_add(amount);
        sum[0] = low;
        sum[1] += u128::from(carry);
    }
    sum
}

#[allow(clippy::too_many_arguments)]
fn prepare_pending_claim(
    wallet: &Wallet,
    escrow_address: [u8; 32],
    escrow: &quil_execution::token_intrinsic::escrow::StoredEscrow,
    branch: ClaimBranch,
    global_frame: Option<u64>,
    signer: &dyn Signer,
    recipients: &[(&RecipientAddress, u128)],
    fee: u128,
    max_outputs: usize,
) -> Result<
    (
        quil_lattice_ct::confidential::pending_claim::PendingClaimStatement,
        [u8; 666],
        quil_lattice_ct::confidential::relation::CompiledAmountRelation,
    ),
    ServiceError,
> {
    use quil_lattice_ct::confidential::pending_claim::PendingClaimStatement;

    if recipients.is_empty() || recipients.len() > max_outputs.min(MAX_PRIVATE_COINS - 1) {
        return Err(ServiceError::Wallet(
            "Claim outputs exceed configured limits".into(),
        ));
    }
    if recipients
        .iter()
        .any(|(address, _)| address.context() != &wallet.context)
    {
        return Err(ServiceError::Wallet(
            "Claim recipient belongs to another network or application".into(),
        ));
    }
    if wallet.application != quil_execution::domains::QUIL_TOKEN && fee != 0 {
        return Err(ServiceError::Wallet(
            "Custom-token outflow cannot pay a QUIL fee".into(),
        ));
    }
    let expected_address = quil_execution::token_intrinsic::escrow::create_escrow(
        &wallet.context,
        escrow.frame_number,
        &escrow.output,
        &escrow.policy,
        &escrow.refund_recovery,
    )
    .map_err(|error| ServiceError::Wallet(format!("Invalid escrow content: {error}")))?
    .0;
    if expected_address != escrow_address {
        return Err(ServiceError::Wallet(
            "Escrow content address mismatch".into(),
        ));
    }
    let authority = match branch {
        ClaimBranch::Recipient => &escrow.policy.recipient,
        ClaimBranch::Refund => escrow
            .policy
            .key_for_branch(
                branch,
                global_frame
                    .ok_or_else(|| {
                        ServiceError::InvalidInput(
                            "Refund preparation requires a global frame reference".into(),
                        )
                    })?
                    .saturating_sub(
                        quil_execution::token_intrinsic::constants::GLOBAL_ANCHOR_SAFETY_MARGIN,
                    ),
            )
            .ok_or_else(|| {
                ServiceError::InvalidInput(
                    "Refund deadline has not been reached at the shards' anchor".into(),
                )
            })?,
    };
    if signer.key_type() != quil_types::crypto::KeyType::Falcon512
        || signer.public_key() != authority.as_slice()
    {
        return Err(ServiceError::Wallet(
            "Signer does not match selected escrow authority".into(),
        ));
    }
    let recovery = match branch {
        ClaimBranch::Recipient => EscrowRecoveryMemo {
            owner: escrow.output.owner,
            ciphertext: escrow.output.memo,
        },
        ClaimBranch::Refund => escrow.refund_recovery.clone(),
    };
    let opened = open_escrow_recovery(
        &wallet.context,
        &wallet.kem_secret,
        &wallet.recipient,
        &escrow.output.commitment,
        &recovery,
    )
    .map_err(|error| ServiceError::Wallet(format!("Cannot open escrow: {error:?}")))?;
    if recipients
        .iter()
        .try_fold(fee, |sum, (_, amount)| sum.checked_add(*amount))
        != Some(opened.amount())
    {
        return Err(ServiceError::Wallet(
            "Escrow amount must equal outputs plus fee".into(),
        ));
    }
    let created = recipients
        .iter()
        .map(|(address, amount)| {
            create_output(&wallet.context, address, *amount).map_err(|error| {
                ServiceError::Wallet(format!("Cannot create claim output: {error:?}"))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let statement = PendingClaimStatement {
        network: wallet.network,
        application: wallet.application,
        escrow_address,
        source: escrow.output.commitment.clone(),
        policy: escrow.policy.clone(),
        branch,
        fee,
        outputs: created.iter().map(|output| output.output.clone()).collect(),
    };
    let canonical = statement
        .context_bytes()
        .map_err(|error| ServiceError::Wallet(format!("Cannot bind claim context: {error:?}")))?;
    let signature: [u8; 666] = signer
        .sign_with_domain(&canonical, &wallet.context)
        .map_err(|error| ServiceError::Wallet(format!("Cannot sign claim: {error}")))?
        .try_into()
        .map_err(|_| ServiceError::Wallet("Invalid claim signature length".into()))?;
    if !quil_crypto::falcon_verify(authority, &signature, &canonical, &wallet.context) {
        return Err(ServiceError::Wallet(
            "Claim signature did not verify".into(),
        ));
    }
    let openings: Vec<_> = recipients
        .iter()
        .zip(&created)
        .map(|((_, amount), output)| (*amount, &output.opening))
        .collect();
    let relation = statement
        .private_relation((opened.amount(), opened.opening()), &openings, max_outputs)
        .map_err(|error| ServiceError::Wallet(format!("Invalid claim witness: {error:?}")))?;
    Ok((statement, signature, relation))
}

#[allow(clippy::too_many_arguments)]
async fn create_pending_claim(
    wallet: Arc<Wallet>,
    escrow_address: [u8; 32],
    escrow: quil_execution::token_intrinsic::escrow::StoredEscrow,
    branch: ClaimBranch,
    global_frame: Option<u64>,
    signer: Arc<dyn Signer>,
    recipients: Vec<(RecipientAddress, u128)>,
    fee: u128,
    max_outputs: usize,
) -> Result<Vec<u8>, ServiceError> {
    use quil_lattice_ct::confidential::{
        pending_claim::PendingClaim,
        relation::backend::native::{self, NativeBudget},
    };

    let permit = spend::PROVING
        .acquire()
        .await
        .map_err(|error| ServiceError::Wallet(format!("Proving unavailable: {error}")))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let destinations: Vec<_> = recipients
            .iter()
            .map(|(address, amount)| (address, *amount))
            .collect();
        let (statement, signature, relation) = prepare_pending_claim(
            &wallet,
            escrow_address,
            &escrow,
            branch,
            global_frame,
            signer.as_ref(),
            &destinations,
            fee,
            max_outputs,
        )?;
        let budget = NativeBudget {
            max_native_bytes: 1 << 30,
        };
        let proof = native::prove(&relation, budget).map_err(|error| {
            ServiceError::Wallet(format!("Pending-claim proving failed: {error:?}"))
        })?;
        drop(relation);
        let claim = PendingClaim {
            statement,
            signature,
            proof,
        };
        let bytes = claim.encode().map_err(|error| {
            ServiceError::Wallet(format!("Pending-claim encoding failed: {error:?}"))
        })?;
        let public = claim
            .statement
            .public_relation(max_outputs)
            .map_err(|error| {
                ServiceError::Wallet(format!(
                    "Pending-claim verification setup failed: {error:?}"
                ))
            })?;
        let valid = native::verify_owned(public, &claim.proof, budget).map_err(|error| {
            ServiceError::Wallet(format!("Pending-claim verification failed: {error:?}"))
        })?;
        if !valid {
            return Err(ServiceError::Wallet(
                "Generated pending claim failed public verification".into(),
            ));
        }
        Ok(bytes)
    })
    .await
    .map_err(|error| ServiceError::Wallet(format!("Pending-claim task failed: {error}")))?
}

fn pending_summary(
    request: &WalletActionRequest,
    amount: u128,
    fee: u128,
    change: u128,
    input_count: usize,
    refund_after_global_frame: Option<u64>,
) -> Result<WalletPreflight, ServiceError> {
    if fee > request.maximum_fee_base_units {
        return Err(ServiceError::Wallet(format!(
            "Fee {fee} exceeds the approved maximum {}",
            request.maximum_fee_base_units
        )));
    }
    let mut summary = WalletPreflight {
        action: request.action,
        application_hex: hex::encode(spend::identifier(&request.application_hex, "application")?),
        target: request.target.trim().to_owned(),
        amount_base_units: amount,
        fee_base_units: fee,
        maximum_fee_base_units: request.maximum_fee_base_units,
        change_base_units: change,
        input_count,
        refund_after_global_frame,
        approval_id: [0; 32],
    };
    summary.approval_id = spend::approval_id(&summary);
    Ok(summary)
}

fn validate_pending_approval(
    request: &WalletActionRequest,
    approved: &WalletPreflight,
) -> Result<(), ServiceError> {
    if !matches!(
        request.action,
        WalletAction::PendingTransfer | WalletAction::Accept | WalletAction::Reject
    ) {
        return Err(ServiceError::InvalidInput(
            "Pending service received a direct transfer".into(),
        ));
    }
    let request_amount = request.amount.map(|amount| amount.base_units());
    let amount_matches = match request.action {
        WalletAction::PendingTransfer => request_amount == Some(approved.amount_base_units),
        WalletAction::Accept | WalletAction::Reject => request_amount.is_none(),
        WalletAction::Transfer => false,
    };
    let refund_matches = match request.action {
        WalletAction::PendingTransfer => request
            .refund_after_global_frame
            .is_none_or(|frame| Some(frame) == approved.refund_after_global_frame),
        WalletAction::Accept | WalletAction::Reject => request.refund_after_global_frame.is_none(),
        WalletAction::Transfer => false,
    };
    if approved.approval_id != spend::approval_id(approved)
        || approved.action != request.action
        || approved.application_hex
            != hex::encode(spend::identifier(&request.application_hex, "application")?)
        || approved.target != request.target.trim()
        || !amount_matches
        || approved.maximum_fee_base_units != request.maximum_fee_base_units
        || !refund_matches
        || request
            .explicit_fee_base_units
            .is_some_and(|fee| approved.fee_base_units != fee)
    {
        return Err(ServiceError::InvalidInput(
            "Wallet confirmation no longer matches the requested pending operation".into(),
        ));
    }
    Ok(())
}

async fn operation_fee(
    wallet: &Wallet,
    request: &WalletActionRequest,
    client: &mut quil_types::proto::node::node_service_client::NodeServiceClient<
        tonic::transport::Channel,
    >,
    shape: quil_execution::token_intrinsic::cost::Shape,
) -> Result<u128, ServiceError> {
    if wallet.application != quil_execution::domains::QUIL_TOKEN {
        let fee = request.explicit_fee_base_units.unwrap_or(0);
        if fee != 0 {
            return Err(ServiceError::InvalidInput(
                "Custom-token outflow cannot pay a QUIL fee directly".into(),
            ));
        }
        return Ok(0);
    }
    let growth = fees::growth_for_shape(&wallet.network, &wallet.application, shape)?;
    fees::quoted_quil_fee_for_cost(
        client,
        &wallet.network,
        request.explicit_fee_base_units,
        growth,
        false,
        request.maximum_fee_base_units,
    )
    .await
}

fn parse_escrow_address(
    value: &str,
    context: &[u8; 32],
) -> Result<(RecipientAddress, [u8; quil_crypto::FALCON_PUBLIC_KEY_LEN]), ServiceError> {
    let bytes =
        hex::decode(value.trim().strip_prefix("0x").unwrap_or(value.trim())).map_err(|error| {
            ServiceError::InvalidInput(format!("Invalid pending-transfer address: {error}"))
        })?;
    if bytes.len() != ADDRESS_BYTES + quil_crypto::FALCON_PUBLIC_KEY_LEN {
        return Err(ServiceError::InvalidInput(
            "Pending-transfer address must contain a confidential address and Falcon public key"
                .into(),
        ));
    }
    let address = RecipientAddress::decode(&bytes[..ADDRESS_BYTES], context).map_err(|error| {
        ServiceError::InvalidInput(format!("Invalid pending-transfer recipient: {error:?}"))
    })?;
    let authority = bytes[ADDRESS_BYTES..]
        .try_into()
        .map_err(|_| ServiceError::InvalidInput("Invalid Falcon authority".into()))?;
    Ok((address, authority))
}

async fn current_global_frame(
    client: &mut quil_types::proto::node::node_service_client::NodeServiceClient<
        tonic::transport::Channel,
    >,
) -> Result<u64, ServiceError> {
    tokio::time::timeout(
        node::RPC_TIMEOUT,
        client.get_node_info(GetNodeInfoRequest::default()),
    )
    .await
    .map_err(|_| ServiceError::Timeout)?
    .map_err(|error| ServiceError::Rpc(error.to_string()))
    .map(|response| response.into_inner().last_global_head_frame)
}

async fn fetch_escrow(
    wallet: &Wallet,
    client: &quil_types::proto::node::node_service_client::NodeServiceClient<
        tonic::transport::Channel,
    >,
    escrow_address: [u8; 32],
) -> Result<quil_execution::token_intrinsic::escrow::StoredEscrow, ServiceError> {
    let mut address = wallet.application.to_vec();
    address.extend_from_slice(&escrow_address);
    let response = tokio::time::timeout(
        node::RPC_TIMEOUT,
        client
            .clone()
            .max_decoding_message_size(
                quil_execution::token_intrinsic::escrow::MAX_ESCROW_BLOB_BYTES + 1024,
            )
            .get_vertex_data(quil_types::proto::node::GetVertexDataRequest {
                address,
                full_data: true,
            }),
    )
    .await
    .map_err(|_| ServiceError::Timeout)?
    .map_err(|error| ServiceError::Rpc(error.to_string()))?
    .into_inner();
    if response.present != Some(true)
        || response.set_type != "vertex"
        || response.phase_type != "adds"
        || response.shard_l2 != wallet.application
        || response.shard_l1
            != quil_hypergraph::addressing::get_bloom_filter_indices(&wallet.application, 256, 3)
        || !response.entries.is_empty()
    {
        return Err(ServiceError::Wallet(
            "Pending-transfer record has the wrong shard, phase, or representation".into(),
        ));
    }
    quil_execution::token_intrinsic::escrow::decode_escrow_blob(
        &response.raw_data,
        &wallet.context,
        &escrow_address,
    )
    .map_err(|error| ServiceError::Wallet(format!("Invalid pending-transfer record: {error}")))
}

async fn ensure_escrow_open(
    wallet: &Wallet,
    client: &quil_types::proto::node::node_service_client::NodeServiceClient<
        tonic::transport::Channel,
    >,
    escrow_address: &[u8; 32],
) -> Result<(), ServiceError> {
    let record = quil_execution::token_intrinsic::global_commit::escrow_address(
        &wallet.application,
        escrow_address,
    )
    .map_err(|error| ServiceError::Wallet(error.to_string()))?;
    let (present, raw_data) = wallet.global_record(client, &record).await?;
    let open = present
        && !quil_execution::token_intrinsic::global_commit::parse_escrow_record(&raw_data)
            .map_err(|error| {
                ServiceError::Wallet(format!("Invalid pending-transfer record: {error}"))
            })?
            .2;
    if !open {
        return Err(ServiceError::Wallet(
            "Pending transfer is absent or already consumed".into(),
        ));
    }
    Ok(())
}

fn claim_recovery(
    escrow: &quil_execution::token_intrinsic::escrow::StoredEscrow,
    branch: PendingEscrowBranch,
) -> EscrowRecoveryMemo {
    match branch {
        PendingEscrowBranch::Recipient => EscrowRecoveryMemo {
            owner: escrow.output.owner,
            ciphertext: escrow.output.memo,
        },
        PendingEscrowBranch::Refund => escrow.refund_recovery.clone(),
    }
}

pub(super) fn validate_claim_branch(
    action: WalletAction,
    branch: PendingEscrowBranch,
    refund_after_global_frame: u64,
    current_global_frame: u64,
) -> Result<(), ServiceError> {
    match (action, branch) {
        (WalletAction::Accept, PendingEscrowBranch::Recipient) => Ok(()),
        (WalletAction::Accept, PendingEscrowBranch::Refund) => Err(ServiceError::InvalidInput(
            "Accept requires the recipient recovery branch".into(),
        )),
        (WalletAction::Reject, PendingEscrowBranch::Recipient) => Err(ServiceError::InvalidInput(
            "Reject requires the refund recovery branch".into(),
        )),
        (WalletAction::Reject, PendingEscrowBranch::Refund)
            if current_global_frame.saturating_sub(
                quil_execution::token_intrinsic::constants::GLOBAL_ANCHOR_SAFETY_MARGIN,
            ) >= refund_after_global_frame =>
        {
            Ok(())
        }
        (WalletAction::Reject, PendingEscrowBranch::Refund) => Err(ServiceError::InvalidInput(
            format!(
                "Refund is not eligible until global frame {refund_after_global_frame}; current frame is {current_global_frame}"
            ),
        )),
        _ => Err(ServiceError::InvalidInput(
            "Only Accept and Reject can claim a pending transfer".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use quil_crypto::sntrup761;
    use quil_lattice_ct::confidential::{
        coin_tree::{CoinRecord, CoinTree},
        memo::{create_escrow_recovery, create_output},
        pending_claim::{ClaimBranch, EscrowPolicy},
        transfer::{CompileLimits, Output},
    };
    use quil_types::crypto::Signer;
    use zeroize::Zeroizing;

    use crate::domain::{PendingEscrowBranch, WalletAction, WalletActionRequest, WalletAmount};

    use super::{
        parse_escrow_address, pending_summary, prepare_pending_create, validate_claim_branch,
        validate_pending_approval, EscrowDestination, Wallet,
    };

    fn pending_request(target: String) -> WalletActionRequest {
        WalletActionRequest {
            action: WalletAction::PendingTransfer,
            application_hex: hex::encode(quil_execution::domains::QUIL_TOKEN),
            target,
            amount: Some(WalletAmount::parse("11").unwrap()),
            explicit_fee_base_units: None,
            maximum_fee_base_units: 20,
            refund_after_global_frame: Some(500),
        }
    }

    #[test]
    fn reject_requires_the_refund_branch_and_expired_global_frame() {
        let margin = quil_execution::token_intrinsic::constants::GLOBAL_ANCHOR_SAFETY_MARGIN;
        let error = validate_claim_branch(
            WalletAction::Reject,
            PendingEscrowBranch::Recipient,
            100,
            100,
        )
        .unwrap_err();
        assert!(error.to_string().contains("refund"));
        assert!(
            validate_claim_branch(WalletAction::Reject, PendingEscrowBranch::Refund, 100, 100)
                .is_err()
        );
        assert!(validate_claim_branch(
            WalletAction::Reject,
            PendingEscrowBranch::Refund,
            100,
            100 + margin
        )
        .is_ok());
    }

    #[test]
    fn accept_requires_the_recipient_branch_but_has_no_deadline() {
        assert!(validate_claim_branch(
            WalletAction::Accept,
            PendingEscrowBranch::Recipient,
            u64::MAX,
            0
        )
        .is_ok());
        assert!(validate_claim_branch(
            WalletAction::Accept,
            PendingEscrowBranch::Refund,
            0,
            u64::MAX
        )
        .is_err());
    }

    #[test]
    fn pending_address_binds_recipient_and_falcon_authority() {
        let keys = sntrup761::Sntrup761KeyPair::generate();
        let wallet =
            Wallet::from_keys([1; 32], [2; 32], &keys.public, Zeroizing::new(keys.secret)).unwrap();
        let authority = [7; quil_crypto::FALCON_PUBLIC_KEY_LEN];
        let mut encoded = wallet.address.encode().to_vec();
        encoded.extend_from_slice(&authority);

        let (recipient, parsed_authority) =
            parse_escrow_address(&format!("0x{}", hex::encode(encoded)), &wallet.context).unwrap();

        assert_eq!(recipient.encode(), wallet.address.encode());
        assert_eq!(parsed_authority, authority);
        assert!(parse_escrow_address("00", &wallet.context).is_err());
        assert!(parse_escrow_address(
            &format!(
                "{}{}",
                hex::encode(wallet.address.encode()),
                hex::encode(authority)
            ),
            &[9; 32]
        )
        .is_err());
    }

    #[test]
    fn changed_pending_request_is_rejected_after_confirmation() {
        let request = pending_request("ab".repeat(160));
        let summary = pending_summary(&request, 11, 2, 4, 1, Some(500)).unwrap();
        let mut changed = request;
        changed.refund_after_global_frame = Some(501);

        assert!(validate_pending_approval(&changed, &summary).is_err());
    }

    #[test]
    fn pending_relation_binds_escrow_policy_and_change() {
        let network = [1; 32];
        let application = quil_execution::domains::QUIL_TOKEN;
        let sender_keys = sntrup761::Sntrup761KeyPair::generate();
        let sender = Wallet::from_keys(
            network,
            application,
            &sender_keys.public,
            Zeroizing::new(sender_keys.secret),
        )
        .unwrap();
        let recipient_keys = sntrup761::Sntrup761KeyPair::generate();
        let recipient = Wallet::from_keys(
            network,
            application,
            &recipient_keys.public,
            Zeroizing::new(recipient_keys.secret),
        )
        .unwrap();
        let output = create_output(&sender.context, &sender.address, 21)
            .unwrap()
            .output;
        let records = vec![CoinRecord {
            address: [3; 32],
            owner: output.owner,
            commitment: output.commitment.clone(),
            position: 0,
        }];
        let record = &records[0];
        let tree = CoinTree::build(&sender.context, &records, 1, 8).unwrap();
        let root = tree.root_at_depth(1).unwrap();
        let path = tree.auth_path(&record.address, 1).unwrap();
        let authority = [7; quil_crypto::FALCON_PUBLIC_KEY_LEN];
        let policy = EscrowPolicy {
            recipient: authority,
            refund: [8; quil_crypto::FALCON_PUBLIC_KEY_LEN],
            refund_after_global_frame: 500,
        };
        let escrow = EscrowDestination {
            recipient: recipient.address.clone(),
            refund: sender.address.clone(),
            amount: 11,
            policy: policy.clone(),
        };

        let (statement, relation) = prepare_pending_create(
            &sender,
            &[(record.address, &output)],
            &root,
            &[path],
            &escrow,
            &[(&sender.address, 8)],
            2,
            CompileLimits {
                max_inputs: 1,
                max_outputs: 2,
                max_depth: 1,
            },
        )
        .unwrap();

        assert!(relation.validate_local_witness());
        assert_eq!(statement.policy, policy);
        assert_eq!(statement.funding.fee, 2);
        assert_eq!(statement.funding.outputs.len(), 2);
    }

    #[test]
    fn claim_relation_binds_content_address_authority_and_output() {
        let network = [1; 32];
        let application = quil_execution::domains::QUIL_TOKEN;
        let keys = sntrup761::Sntrup761KeyPair::generate();
        let wallet = Wallet::from_keys(
            network,
            application,
            &keys.public,
            Zeroizing::new(keys.secret),
        )
        .unwrap();
        let signer = quil_crypto::FalconSigner::generate();
        let refund_signer = quil_crypto::FalconSigner::generate();
        let recovery =
            create_escrow_recovery(&wallet.context, &wallet.address, &wallet.address, 21).unwrap();
        let stored = quil_execution::token_intrinsic::escrow::StoredEscrow {
            frame_number: 1,
            output: Output {
                commitment: recovery.commitment,
                owner: recovery.recipient.owner,
                memo: recovery.recipient.ciphertext,
            },
            policy: EscrowPolicy {
                recipient: signer.public_key().try_into().unwrap(),
                refund: refund_signer.public_key().try_into().unwrap(),
                refund_after_global_frame: 100,
            },
            refund_recovery: recovery.refund,
        };
        let address = quil_execution::token_intrinsic::escrow::create_escrow(
            &wallet.context,
            stored.frame_number,
            &stored.output,
            &stored.policy,
            &stored.refund_recovery,
        )
        .unwrap()
        .0;

        let (statement, signature, relation) = super::prepare_pending_claim(
            &wallet,
            address,
            &stored,
            ClaimBranch::Recipient,
            None,
            &signer,
            &[(&wallet.address, 19)],
            2,
            1,
        )
        .unwrap();

        assert!(relation.validate_local_witness());
        assert!(quil_crypto::falcon_verify(
            signer.public_key(),
            &signature,
            &statement.context_bytes().unwrap(),
            &wallet.context,
        ));
        assert_eq!(statement.escrow_address, address);
        assert_eq!(statement.outputs.len(), 1);
    }
}
