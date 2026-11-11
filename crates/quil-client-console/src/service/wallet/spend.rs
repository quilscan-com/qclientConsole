use std::{path::Path, sync::Arc};

use quil_crypto::sntrup761;
use quil_lattice_ct::confidential::{
    address::RecipientAddress,
    memo::{create_output, open_output},
    relation::membership::{InputPath, MembershipKey, MAX_DEPTH},
    transfer::{network_identifier, CompileLimits, Output, Transfer, TransferStatement},
    MAX_PRIVATE_COINS,
};
use sha3::{Digest, Sha3_256};
use zeroize::Zeroizing;

use crate::domain::{
    WalletAction, WalletActionRequest, WalletOperationResult, WalletOperationStatus,
    WalletPreflight,
};

use super::{fees, model::select_coins, node, prover, submission, OwnedCoin, ServiceError, Wallet};

pub(super) static PROVING: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
const FEE_REBUILDS: usize = 3;

pub(super) async fn preflight_transfer(
    config_dir: &Path,
    request: &WalletActionRequest,
) -> Result<WalletPreflight, ServiceError> {
    validate_transfer_request(request)?;
    let application = identifier(&request.application_hex, "application")?;
    let (wallet, mut client) = load_wallet(config_dir, application).await?;
    parse_recipient(&request.target, &wallet.context)?;
    let limits = compile_limits();
    let fee = transfer_fee(&wallet, request, &mut client, limits).await?;
    let amount = request.amount.expect("validated amount").base_units();
    let coins = wallet.clone().scan_unspent(&client).await?;
    let (selected, change) = select_coins(coins, amount, fee, limits.max_inputs)?;
    transfer_summary(request, fee, *change, selected.len())
}

pub(super) async fn execute_transfer(
    config_dir: &Path,
    request: &WalletActionRequest,
    approved: &WalletPreflight,
) -> Result<WalletOperationResult, ServiceError> {
    validate_approval(request, approved)?;
    let application = identifier(&request.application_hex, "application")?;
    if application != quil_execution::domains::QUIL_TOKEN {
        return Err(ServiceError::Operation(
            "Custom-token transfers require the settlement payment workflow, which is not enabled yet"
                .into(),
        ));
    }
    let (wallet, mut client) = load_wallet(config_dir, application).await?;
    let recipient = parse_recipient(&request.target, &wallet.context)?;
    let limits = compile_limits();
    let amount = request.amount.expect("validated amount").base_units();

    for attempt in 1..=FEE_REBUILDS {
        let fee = transfer_fee(&wallet, request, &mut client, limits).await?;
        if fee > approved.maximum_fee_base_units {
            return Err(ServiceError::Wallet(format!(
                "Current fee {fee} exceeds the approved maximum {}",
                approved.maximum_fee_base_units
            )));
        }
        let coins = wallet.clone().scan_unspent(&client).await?;
        let (selected, change) = select_coins(coins, amount, fee, limits.max_inputs)?;
        let mut destinations = vec![(recipient.clone(), amount)];
        if *change != 0 {
            destinations.push((wallet.address.clone(), *change));
        }
        let bytes =
            create_transfer(wallet.clone(), &client, selected, destinations, fee, limits).await?;
        match fees::refresh_submission_fee(&mut client, &wallet.network, &bytes).await {
            Ok(()) => {
                let submitted =
                    submission::submit(config_dir, &wallet, &client, WalletAction::Transfer, bytes)
                        .await?;
                let outcome = submitted.outcome;
                let message = match submitted.operation.phase {
                    WalletOperationStatus::Committed => submitted
                        .operation
                        .detail
                        .clone()
                        .unwrap_or_else(|| "Confidential transfer committed by GLOBAL".into()),
                    WalletOperationStatus::Rejected => submitted
                        .operation
                        .detail
                        .clone()
                        .unwrap_or_else(|| "Confidential transfer rejected by GLOBAL".into()),
                    WalletOperationStatus::Submitting => {
                        "Confidential transfer is being submitted".into()
                    }
                    WalletOperationStatus::Submitted => {
                        "Confidential transfer submitted; finalized inclusion is not yet confirmed"
                            .into()
                    }
                    WalletOperationStatus::Unknown => {
                        "Submission result is unknown; refresh wallet state before trying again"
                            .into()
                    }
                };
                return Ok(WalletOperationResult {
                    action: WalletAction::Transfer,
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
        "Transfer could not be rebuilt within the fee refresh limit".into(),
    ))
}

pub(super) async fn load_wallet(
    config_dir: &Path,
    application: [u8; 32],
) -> Result<
    (
        Arc<Wallet>,
        quil_types::proto::node::node_service_client::NodeServiceClient<tonic::transport::Channel>,
    ),
    ServiceError,
> {
    let config = quil_config::load_config(config_dir)
        .map_err(|error| ServiceError::Config(error.to_string()))?;
    let manager = prover::build_key_manager(&config, config_dir)?;
    let secret = Zeroizing::new(
        manager
            .get_secret_key_bytes_by_id("q-onion-key")
            .map_err(|error| ServiceError::Wallet(format!("q-onion-key secret: {error}")))?,
    );
    let public = manager
        .get_public_key_bytes_by_id("q-onion-key")
        .map_err(|error| ServiceError::Wallet(format!("q-onion-key public: {error}")))?;
    if public.len() != sntrup761::SNTRUP761_PUBLIC_KEY_LEN
        || secret.len() != sntrup761::SNTRUP761_SECRET_KEY_LEN
    {
        return Err(ServiceError::Wallet(
            "q-onion-key is not a valid sntrup761 keypair".into(),
        ));
    }
    let wallet = Arc::new(Wallet::from_keys(
        network_identifier(config.p2p.network),
        application,
        &public,
        secret,
    )?);
    let read_rpc =
        super::super::read_rpc::load(config_dir, config.effective_listen_grpc_multiaddr())?;
    let address = node::grpc_address(&read_rpc)?;
    Ok((wallet, node::connect_client(&address).await?))
}

async fn transfer_fee(
    wallet: &Wallet,
    request: &WalletActionRequest,
    client: &mut quil_types::proto::node::node_service_client::NodeServiceClient<
        tonic::transport::Channel,
    >,
    limits: CompileLimits,
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
    let growth = fees::growth_for_shape(
        &wallet.network,
        &wallet.application,
        quil_execution::token_intrinsic::cost::Shape {
            coins: 2,
            markers: limits.max_inputs,
            escrow: false,
        },
    )?;
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

async fn create_transfer(
    wallet: Arc<Wallet>,
    client: &quil_types::proto::node::node_service_client::NodeServiceClient<
        tonic::transport::Channel,
    >,
    coins: Vec<OwnedCoin>,
    recipients: Vec<(RecipientAddress, u128)>,
    fee: u128,
    limits: CompileLimits,
) -> Result<Vec<u8>, ServiceError> {
    use quil_lattice_ct::confidential::relation::backend::native;

    let permit = PROVING
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
        let destinations: Vec<_> = recipients
            .iter()
            .map(|(address, amount)| (address, *amount))
            .collect();
        let (statement, relation) = prepare_transfer(
            &wallet,
            &selected,
            &root,
            &paths,
            &destinations,
            fee,
            limits,
        )?;
        let budget =
            quil_execution::token_intrinsic::dispatch::TokenPolicy::for_network(0).native_budget;
        let proof = native::prove(&relation, budget)
            .map_err(|error| ServiceError::Wallet(format!("Transfer proving failed: {error:?}")))?;
        drop(relation);
        let transfer = Transfer { statement, proof };
        let bytes = transfer.encode().map_err(|error| {
            ServiceError::Wallet(format!("Transfer encoding failed: {error:?}"))
        })?;
        let public = transfer
            .statement
            .public_relation(limits)
            .map_err(|error| {
                ServiceError::Wallet(format!("Transfer verification setup failed: {error:?}"))
            })?;
        let valid = native::verify_owned(public, &transfer.proof, budget).map_err(|error| {
            ServiceError::Wallet(format!("Transfer verification failed: {error:?}"))
        })?;
        if !valid {
            return Err(ServiceError::Wallet(
                "Generated transfer failed public verification".into(),
            ));
        }
        Ok(bytes)
    })
    .await
    .map_err(|error| ServiceError::Wallet(format!("Transfer proving task failed: {error}")))?
}

fn prepare_transfer(
    wallet: &Wallet,
    coins: &[([u8; 32], &Output)],
    root: &quil_lattice_ct::confidential::coin_tree::RootRecord,
    paths: &[quil_lattice_ct::confidential::coin_tree::AuthPath],
    recipients: &[(&RecipientAddress, u128)],
    fee: u128,
    limits: CompileLimits,
) -> Result<
    (
        TransferStatement,
        quil_lattice_ct::confidential::relation::CompiledAmountRelation,
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
            "Transfer dimensions exceed configured limits".into(),
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
    let created = recipients
        .iter()
        .map(|(address, amount)| {
            create_output(&wallet.context, address, *amount).map_err(|error| {
                ServiceError::Wallet(format!("Cannot create recipient output: {error:?}"))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
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
        .map_err(|error| ServiceError::Wallet(format!("Invalid transfer witness: {error:?}")))?;
    Ok((statement, relation))
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

pub(super) fn compile_limits() -> CompileLimits {
    quil_execution::token_intrinsic::dispatch::TokenPolicy::for_network(0).limits
}

fn parse_recipient(value: &str, context: &[u8; 32]) -> Result<RecipientAddress, ServiceError> {
    let bytes = hex::decode(value.trim().strip_prefix("0x").unwrap_or(value.trim()))
        .map_err(|error| ServiceError::InvalidInput(format!("Invalid recipient: {error}")))?;
    RecipientAddress::decode(&bytes, context)
        .map_err(|error| ServiceError::InvalidInput(format!("Invalid recipient: {error:?}")))
}

pub(super) fn identifier(value: &str, label: &str) -> Result<[u8; 32], ServiceError> {
    hex::decode(value.trim().strip_prefix("0x").unwrap_or(value.trim()))
        .map_err(|error| ServiceError::InvalidInput(format!("Invalid {label}: {error}")))?
        .try_into()
        .map_err(|_| ServiceError::InvalidInput(format!("{label} must be exactly 32 bytes")))
}

fn validate_transfer_request(request: &WalletActionRequest) -> Result<(), ServiceError> {
    if request.action != WalletAction::Transfer {
        return Err(ServiceError::InvalidInput(
            "Transfer service received another wallet action".into(),
        ));
    }
    if request.amount.is_none() {
        return Err(ServiceError::InvalidInput(
            "Transfer amount is required".into(),
        ));
    }
    if request.refund_after_global_frame.is_some() {
        return Err(ServiceError::InvalidInput(
            "A direct transfer cannot set a refund frame".into(),
        ));
    }
    Ok(())
}

fn validate_approval(
    request: &WalletActionRequest,
    approved: &WalletPreflight,
) -> Result<(), ServiceError> {
    validate_transfer_request(request)?;
    if approved.approval_id != approval_id(approved)
        || approved.action != request.action
        || approved.application_hex != normalized_hex_32(&request.application_hex, "application")?
        || approved.target != request.target.trim()
        || approved.amount_base_units != request.amount.expect("validated amount").base_units()
        || approved.maximum_fee_base_units != request.maximum_fee_base_units
        || approved.refund_after_global_frame != request.refund_after_global_frame
        || request
            .explicit_fee_base_units
            .is_some_and(|fee| approved.fee_base_units != fee)
    {
        return Err(ServiceError::InvalidInput(
            "Wallet confirmation no longer matches the requested transfer".into(),
        ));
    }
    Ok(())
}

pub(super) fn transfer_summary(
    request: &WalletActionRequest,
    fee: u128,
    change: u128,
    input_count: usize,
) -> Result<WalletPreflight, ServiceError> {
    if request.action != WalletAction::Transfer {
        return Err(ServiceError::InvalidInput(
            "Transfer preflight received another wallet action".into(),
        ));
    }
    let amount = request
        .amount
        .ok_or_else(|| ServiceError::InvalidInput("Transfer amount is required".into()))?
        .base_units();
    if fee > request.maximum_fee_base_units {
        return Err(ServiceError::Wallet(format!(
            "Fee {fee} exceeds the approved maximum {}",
            request.maximum_fee_base_units
        )));
    }
    if input_count == 0 {
        return Err(ServiceError::Wallet(
            "Transfer must spend at least one Coin".into(),
        ));
    }
    let mut summary = WalletPreflight {
        action: request.action,
        application_hex: normalized_hex_32(&request.application_hex, "application")?,
        target: request.target.trim().to_owned(),
        amount_base_units: amount,
        fee_base_units: fee,
        maximum_fee_base_units: request.maximum_fee_base_units,
        change_base_units: change,
        input_count,
        refund_after_global_frame: request.refund_after_global_frame,
        approval_id: [0; 32],
    };
    summary.approval_id = approval_id(&summary);
    Ok(summary)
}

pub(super) fn approval_id(summary: &WalletPreflight) -> [u8; 32] {
    let mut digest = Sha3_256::new();
    digest.update(b"qclient-console/wallet-preflight/v1\0");
    digest.update([match summary.action {
        WalletAction::Transfer => 0,
        WalletAction::PendingTransfer => 1,
        WalletAction::Accept => 2,
        WalletAction::Reject => 3,
    }]);
    put_bytes(&mut digest, summary.application_hex.as_bytes());
    put_bytes(&mut digest, summary.target.as_bytes());
    digest.update(summary.amount_base_units.to_be_bytes());
    digest.update(summary.fee_base_units.to_be_bytes());
    digest.update(summary.maximum_fee_base_units.to_be_bytes());
    digest.update(summary.change_base_units.to_be_bytes());
    digest.update((summary.input_count as u64).to_be_bytes());
    match summary.refund_after_global_frame {
        Some(frame) => {
            digest.update([1]);
            digest.update(frame.to_be_bytes());
        }
        None => digest.update([0]),
    }
    digest.finalize().into()
}

fn put_bytes(digest: &mut Sha3_256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

fn normalized_hex_32(value: &str, label: &str) -> Result<String, ServiceError> {
    let bytes = hex::decode(value.trim().strip_prefix("0x").unwrap_or(value.trim()))
        .map_err(|error| ServiceError::InvalidInput(format!("Invalid {label}: {error}")))?;
    let _: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| ServiceError::InvalidInput(format!("{label} must be exactly 32 bytes")))?;
    Ok(hex::encode(bytes))
}

#[cfg(test)]
mod tests {
    use quil_crypto::sntrup761;
    use quil_lattice_ct::confidential::{
        coin_tree::{CoinRecord, CoinTree},
        memo::{create_output, open_output},
        transfer::CompileLimits,
    };
    use zeroize::Zeroizing;

    use crate::domain::{WalletAction, WalletActionRequest, WalletAmount};

    use super::{approval_id, prepare_transfer, transfer_summary, validate_approval, Wallet};

    fn fixture_request() -> WalletActionRequest {
        WalletActionRequest {
            action: WalletAction::Transfer,
            application_hex: hex::encode(quil_execution::domains::QUIL_TOKEN),
            target: "ab".repeat(32),
            amount: Some(WalletAmount::parse("11").unwrap()),
            explicit_fee_base_units: None,
            maximum_fee_base_units: 20,
            refund_after_global_frame: None,
        }
    }

    #[test]
    fn transfer_summary_contains_the_exact_approved_spend() {
        let summary = transfer_summary(&fixture_request(), 2, 9, 2).unwrap();
        assert_eq!(summary.amount_base_units, 11);
        assert_eq!(summary.fee_base_units, 2);
        assert_eq!(summary.change_base_units, 9);
        assert_eq!(summary.input_count, 2);
        assert_ne!(summary.approval_id, [0; 32]);
        assert_eq!(summary.approval_id, approval_id(&summary));
    }

    #[test]
    fn approval_fingerprint_detects_modified_fee_ceiling() {
        let mut summary = transfer_summary(&fixture_request(), 2, 9, 2).unwrap();
        let original = summary.approval_id;
        summary.maximum_fee_base_units += 1;
        assert_ne!(approval_id(&summary), original);
    }

    #[test]
    fn modified_request_is_rejected_after_confirmation() {
        let request = fixture_request();
        let summary = transfer_summary(&request, 2, 9, 2).unwrap();
        let mut changed = request;
        changed.target = "cd".repeat(32);
        assert!(validate_approval(&changed, &summary).is_err());
    }

    #[test]
    fn read_rpc_override_is_private_per_node_configuration() {
        let config = tempfile::tempdir().unwrap();
        let fallback = "/ip4/127.0.0.1/tcp/8337";

        assert_eq!(
            super::super::read_rpc::load(config.path(), fallback).unwrap(),
            fallback
        );
        super::super::read_rpc::save(config.path(), Some("/ip4/192.0.2.89/tcp/8337")).unwrap();
        assert_eq!(
            super::super::read_rpc::load(config.path(), fallback).unwrap(),
            "/ip4/192.0.2.89/tcp/8337"
        );
        assert!(super::super::read_rpc::save(config.path(), Some("not-a-multiaddr")).is_err());
        super::super::read_rpc::save(config.path(), None).unwrap();
        assert_eq!(
            super::super::read_rpc::load(config.path(), fallback).unwrap(),
            fallback
        );
    }

    #[test]
    fn transfer_relation_uses_owned_inputs_recipient_and_change() {
        let network = [1; 32];
        let application = [2; 32];
        let keys = sntrup761::Sntrup761KeyPair::generate();
        let wallet = Wallet::from_keys(
            network,
            application,
            &keys.public,
            Zeroizing::new(keys.secret),
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
        let outputs: Vec<_> = [13u128, 8]
            .into_iter()
            .map(|amount| {
                create_output(&wallet.context, &wallet.address, amount)
                    .unwrap()
                    .output
            })
            .collect();
        let records: Vec<_> = outputs
            .iter()
            .enumerate()
            .map(|(index, output)| CoinRecord {
                address: [index as u8 + 1; 32],
                owner: output.owner,
                commitment: output.commitment.clone(),
                position: index as u64,
            })
            .collect();
        let tree = CoinTree::build(&wallet.context, &records, 1, 8).unwrap();
        let root = tree.root_at_depth(1).unwrap();
        let paths: Vec<_> = records
            .iter()
            .map(|record| tree.auth_path(&record.address, 1).unwrap())
            .collect();
        let coins: Vec<_> = records
            .iter()
            .zip(&outputs)
            .map(|(record, output)| (record.address, output))
            .collect();
        let destinations = [(&recipient.address, 12), (&wallet.address, 7)];
        let limits = CompileLimits {
            max_inputs: 2,
            max_outputs: 2,
            max_depth: 1,
        };

        let (statement, relation) =
            prepare_transfer(&wallet, &coins, &root, &paths, &destinations, 2, limits).unwrap();

        assert!(relation.validate_local_witness());
        assert_eq!(
            open_output(
                &recipient.context,
                &recipient.kem_secret,
                &recipient.recipient,
                &statement.outputs[0]
            )
            .unwrap()
            .amount,
            12
        );
        assert_eq!(
            open_output(
                &wallet.context,
                &wallet.kem_secret,
                &wallet.recipient,
                &statement.outputs[1]
            )
            .unwrap()
            .amount,
            7
        );
        assert_eq!(statement.fee, 2);
        assert_ne!(statement.images[0], statement.images[1]);
    }
}
