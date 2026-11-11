use std::{path::Path, sync::Arc};

use prost::Message;
use quil_crypto::sntrup761;
use quil_lattice_ct::confidential::{
    address::RecipientAddress,
    coin_tree::RootRecord,
    memo::{open_escrow_recovery, open_output, EscrowRecoveryMemo, MemoError},
    relation::membership::{MembershipKey, RecipientSecret, IDENTITY_BYTES},
    transfer::{network_identifier, parameter_context, Output},
    AmountCommitment,
};
use quil_types::{
    error::QuilError,
    proto::node::{GetVertexDataRequest, ListCoinsRequest, ListCoinsResponse, ListEscrowsResponse},
};
use sha3::{
    digest::{ExtendableOutput, Update, XofReader},
    Shake256,
};
use zeroize::Zeroizing;

mod decision;
mod discovery;
pub(super) mod fees;
mod mint;
#[allow(dead_code)] // Used by the Mint orchestrator added in the next slice.
mod mint_journal;
mod model;
mod operation_journal;
mod pending;
mod spend;
pub(super) mod submission;

use model::OwnedCoin;

use super::{node, prover, read_rpc, ServiceError};
use crate::domain::{
    ConfidentialCoinSnapshot, ConfidentialWalletSnapshot, PendingEscrowBranch,
    PendingEscrowSnapshot, PendingEscrowsSnapshot, WalletAction, WalletActionRequest,
    WalletOperationResult, WalletPreflight,
};

const MAX_PAGES: usize = 2048;
const SCAN_PAGE_ENTRIES: usize = quil_types::store::MAX_SCAN_PAGE_ENTRIES;
const MAX_COINS: usize = MAX_PAGES * SCAN_PAGE_ENTRIES;
const MAX_PAGE_BYTES: usize = SCAN_PAGE_ENTRIES * 32 * 1024;

pub(super) async fn preflight_action(
    config_dir: &Path,
    request: WalletActionRequest,
) -> Result<WalletPreflight, ServiceError> {
    match request.action {
        WalletAction::Transfer => spend::preflight_transfer(config_dir, &request).await,
        WalletAction::PendingTransfer | WalletAction::Accept | WalletAction::Reject => {
            pending::preflight_pending(config_dir, &request).await
        }
    }
}

pub(super) async fn execute_action(
    config_dir: &Path,
    request: WalletActionRequest,
    approved: WalletPreflight,
) -> Result<WalletOperationResult, ServiceError> {
    match request.action {
        WalletAction::Transfer => spend::execute_transfer(config_dir, &request, &approved).await,
        WalletAction::PendingTransfer | WalletAction::Accept | WalletAction::Reject => {
            pending::execute_pending(config_dir, &request, &approved).await
        }
    }
}

pub(super) async fn reward_mint_preflight(
    config_dir: &Path,
) -> Result<crate::domain::RewardMintPreflight, ServiceError> {
    mint::preflight(config_dir).await
}

pub(super) async fn start_reward_mint(
    config_dir: &Path,
    approved: crate::domain::RewardMintPreflight,
) -> Result<crate::domain::RewardMintStatus, ServiceError> {
    mint::start(config_dir, approved).await
}

pub(super) async fn load_reward_mint(
    config_dir: &Path,
) -> Result<Option<crate::domain::RewardMintStatus>, ServiceError> {
    mint::load(config_dir).await
}

pub(super) async fn advance_reward_mint(
    config_dir: &Path,
) -> Result<crate::domain::RewardMintStatus, ServiceError> {
    mint::advance(config_dir).await
}

pub(super) async fn rebroadcast_reward_mint(
    config_dir: &Path,
    confirmed: bool,
) -> Result<crate::domain::RewardMintStatus, ServiceError> {
    mint::rebroadcast(config_dir, confirmed).await
}

pub(super) struct Wallet {
    context: [u8; 32],
    network: [u8; 32],
    application: [u8; 32],
    kem_secret: Zeroizing<Vec<u8>>,
    recipient: RecipientSecret,
    address: RecipientAddress,
}

struct Scan {
    snapshot_id: Option<[u8; 32]>,
    root: Option<RootRecord>,
    after: Option<[u8; 32]>,
    remaining_pages: usize,
    finished: bool,
}

fn load_local_wallet(
    config_dir: &Path,
    application: [u8; 32],
) -> Result<(quil_config::Config, Arc<Wallet>), ServiceError> {
    let config = quil_config::load_config(config_dir)
        .map_err(|error| ServiceError::Config(error.to_string()))?;
    let manager = prover::build_key_manager(&config, config_dir)?;
    let secret = Zeroizing::new(
        manager
            .get_secret_key_bytes_by_id("q-onion-key")
            .map_err(|error| ServiceError::Wallet(format!("q-onion-key secret：{error}")))?,
    );
    let public = manager
        .get_public_key_bytes_by_id("q-onion-key")
        .map_err(|error| ServiceError::Wallet(format!("q-onion-key public：{error}")))?;
    let wallet = Arc::new(Wallet::from_keys(
        network_identifier(config.p2p.network),
        application,
        &public,
        secret,
    )?);
    Ok((config, wallet))
}

pub(super) fn recipient_address(config_dir: &Path) -> Result<String, ServiceError> {
    let (_, wallet) = load_local_wallet(config_dir, quil_execution::domains::QUIL_TOKEN)?;
    Ok(hex::encode(wallet.address.encode()))
}

pub(super) async fn read(config_dir: &Path) -> Result<ConfidentialWalletSnapshot, ServiceError> {
    let (config, wallet) = load_local_wallet(config_dir, quil_execution::domains::QUIL_TOKEN)?;
    let read_rpc = read_rpc::load(config_dir, config.effective_listen_grpc_multiaddr())?;
    let address = node::grpc_address(&read_rpc)?;
    let client = node::connect_client(&address).await?;
    let coins = wallet.clone().scan_unspent(&client).await?;
    let total_base_units = coins.iter().try_fold(0u128, |total, coin| {
        total
            .checked_add(*coin.amount)
            .ok_or_else(|| ServiceError::Wallet("Balance exceeds u128".into()))
    })?;

    Ok(ConfidentialWalletSnapshot {
        address_hex: hex::encode(wallet.address.encode()),
        total_base_units,
        coins: coins
            .into_iter()
            .map(|coin| ConfidentialCoinSnapshot {
                address_hex: hex::encode(coin.address),
                amount_base_units: *coin.amount,
            })
            .collect(),
        recent_operations: operation_journal::OperationJournal::recover(config_dir, &client)
            .await?,
    })
}

pub(super) async fn read_escrows(
    config_dir: &Path,
) -> Result<PendingEscrowsSnapshot, ServiceError> {
    let config = quil_config::load_config(config_dir)
        .map_err(|error| ServiceError::Config(error.to_string()))?;
    let manager = prover::build_key_manager(&config, config_dir)?;
    let read_rpc = read_rpc::load(config_dir, config.effective_listen_grpc_multiaddr())?;
    let address = node::grpc_address(&read_rpc)?;
    let client = node::connect_client(&address).await?;
    let authority = match manager.get_public_key_bytes_by_id("q-prover-key") {
        Ok(authority) => authority,
        Err(QuilError::NotFound(_)) => {
            return Ok(PendingEscrowsSnapshot {
                identity_available: false,
                unavailable_reason: Some(
                    "The current configuration has no q-prover-key, so pending transfers for this Prover cannot be identified".into(),
                ),
                escrows: Vec::new(),
                recent_operations: operation_journal::OperationJournal::recover(
                    config_dir, &client,
                )
                .await?,
            });
        }
        Err(error) => return Err(ServiceError::Key(error.to_string())),
    };
    if authority.len() != quil_crypto::FALCON_PUBLIC_KEY_LEN {
        return Err(ServiceError::Wallet(
            "q-prover-key is not a valid Falcon-512 public key".into(),
        ));
    }
    let secret = Zeroizing::new(
        manager
            .get_secret_key_bytes_by_id("q-onion-key")
            .map_err(|error| ServiceError::Wallet(format!("q-onion-key secret：{error}")))?,
    );
    let public = manager
        .get_public_key_bytes_by_id("q-onion-key")
        .map_err(|error| ServiceError::Wallet(format!("q-onion-key public：{error}")))?;
    let wallet = Arc::new(Wallet::from_keys(
        network_identifier(config.p2p.network),
        quil_execution::domains::QUIL_TOKEN,
        &public,
        secret,
    )?);
    let escrows = wallet.clone().scan_escrows(&client, &authority).await?;
    Ok(PendingEscrowsSnapshot {
        identity_available: true,
        unavailable_reason: None,
        escrows,
        recent_operations: operation_journal::OperationJournal::recover(config_dir, &client)
            .await?,
    })
}

impl Wallet {
    pub(super) fn from_keys(
        network: [u8; 32],
        application: [u8; 32],
        public: &[u8],
        secret: Zeroizing<Vec<u8>>,
    ) -> Result<Self, ServiceError> {
        if public.len() != sntrup761::SNTRUP761_PUBLIC_KEY_LEN
            || secret.len() != sntrup761::SNTRUP761_SECRET_KEY_LEN
        {
            return Err(ServiceError::Wallet(
                "q-onion-key is not a valid sntrup761 keypair".into(),
            ));
        }
        let (expected, ciphertext) = sntrup761::encapsulate(public)
            .map_err(|error| ServiceError::Wallet(error.to_string()))?;
        let expected = Zeroizing::new(expected);
        let recovered = Zeroizing::new(
            sntrup761::decapsulate(&ciphertext, &secret)
                .map_err(|error| ServiceError::Wallet(error.to_string()))?,
        );
        if *expected != *recovered {
            return Err(ServiceError::Wallet(
                "q-onion-key public and private keys do not match".into(),
            ));
        }

        let context = parameter_context(&network, &application);
        let mut hash = Shake256::default();
        hash.update(b"quil/coin/wallet-recipient/v3\0");
        hash.update(&(secret.len() as u64).to_le_bytes());
        hash.update(&secret);
        let mut seed = Zeroizing::new([0u8; 32]);
        hash.finalize_xof().read(seed.as_mut());
        let recipient = RecipientSecret::from_seed(&context, &seed);
        let address = RecipientAddress::new(&context, &recipient, public).map_err(|error| {
            ServiceError::Wallet(format!("Wallet address generation failed: {error:?}"))
        })?;
        Ok(Self {
            context,
            network,
            application,
            kem_secret: secret,
            recipient,
            address,
        })
    }

    pub(super) fn address(&self) -> &RecipientAddress {
        &self.address
    }

    pub(super) fn open(
        &self,
        output: &Output,
    ) -> anyhow::Result<quil_lattice_ct::confidential::memo::OpenedOutput> {
        open_output(&self.context, &self.kem_secret, &self.recipient, output)
            .map_err(|error| anyhow::anyhow!("wallet output: {error:?}"))
    }

    pub(super) async fn unspent_outputs(
        self: Arc<Self>,
        client: &quil_types::proto::node::node_service_client::NodeServiceClient<
            tonic::transport::Channel,
        >,
    ) -> Result<Vec<Output>, ServiceError> {
        Ok(self
            .scan_unspent(client)
            .await?
            .into_iter()
            .map(|coin| coin.output)
            .collect())
    }

    fn prepare_batch_shield(
        &self,
        sources: Vec<quil_lattice_ct::confidential::shield::ShieldSource>,
        signer: &dyn quil_types::crypto::Signer,
        recipients: &[(RecipientAddress, u128)],
        fee: u128,
        max_outputs: usize,
    ) -> anyhow::Result<(
        quil_lattice_ct::confidential::shield::BatchShieldStatement,
        [u8; 114],
        quil_lattice_ct::confidential::relation::CompiledAmountRelation,
    )> {
        use quil_lattice_ct::confidential::{
            memo::create_output, shield::BatchShieldStatement, MAX_PRIVATE_COINS,
        };
        anyhow::ensure!(
            signer.key_type() == quil_types::crypto::KeyType::Ed448,
            "transparent sources require an Ed448 owner"
        );
        let public = signer
            .public_key()
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid source owner public key length"))?;
        anyhow::ensure!(
            !recipients.is_empty() && recipients.len() <= max_outputs.min(MAX_PRIVATE_COINS),
            "shield outputs exceed configured limits"
        );
        anyhow::ensure!(
            recipients
                .iter()
                .all(|(address, _)| address.context() == &self.context),
            "recipient belongs to a different network or application"
        );
        let total = sources
            .iter()
            .try_fold(0u128, |sum, source| sum.checked_add(source.amount))
            .ok_or_else(|| anyhow::anyhow!("legacy sources overflow u128"))?;
        let spent = recipients
            .iter()
            .try_fold(fee, |sum, (_, amount)| sum.checked_add(*amount));
        anyhow::ensure!(spent == Some(total), "sources must equal outputs plus fee");
        let created = recipients
            .iter()
            .map(|(address, amount)| create_output(&self.context, address, *amount))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| anyhow::anyhow!("shield output: {error:?}"))?;
        let statement = BatchShieldStatement {
            network: self.network,
            application: self.application,
            owner_public_key: public,
            sources,
            fee,
            outputs: created.iter().map(|output| output.output.clone()).collect(),
        };
        let context = statement
            .context_bytes()
            .map_err(|error| anyhow::anyhow!("batch shield context: {error:?}"))?;
        let signature: [u8; 114] = signer
            .sign(&context)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid source owner signature length"))?;
        anyhow::ensure!(
            quil_crypto::ed448_verify(&public, &context, &signature),
            "source owner signature did not verify"
        );
        let openings: Vec<_> = recipients
            .iter()
            .zip(&created)
            .map(|((_, amount), output)| (*amount, &output.opening))
            .collect();
        let relation = statement
            .private_relation(&openings, max_outputs)
            .map_err(|error| anyhow::anyhow!("batch shield relation: {error:?}"))?;
        Ok((statement, signature, relation))
    }

    pub(super) async fn create_batch_shield(
        self: Arc<Self>,
        sources: Vec<quil_lattice_ct::confidential::shield::ShieldSource>,
        signer: Arc<dyn quil_types::crypto::Signer>,
        recipients: Vec<(RecipientAddress, u128)>,
        fee: u128,
    ) -> anyhow::Result<Vec<u8>> {
        use quil_lattice_ct::confidential::{relation::backend::native, shield::BatchShield};
        let permit = spend::PROVING.acquire().await?;
        let budget =
            quil_execution::token_intrinsic::dispatch::TokenPolicy::for_network(0).native_budget;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let (statement, signature, relation) =
                self.prepare_batch_shield(sources, signer.as_ref(), &recipients, fee, 1)?;
            let proof = native::prove(&relation, budget)
                .map_err(|error| anyhow::anyhow!("batch shield proving: {error:?}"))?;
            drop(relation);
            let shield = BatchShield {
                statement,
                signature,
                proof,
            };
            let bytes = shield
                .encode()
                .map_err(|error| anyhow::anyhow!("batch shield encoding: {error:?}"))?;
            let public = shield
                .statement
                .public_relation(1)
                .map_err(|error| anyhow::anyhow!("public batch shield relation: {error:?}"))?;
            anyhow::ensure!(
                native::verify_owned(public, &shield.proof, budget)
                    .map_err(|error| anyhow::anyhow!("batch shield verification: {error:?}"))?,
                "generated batch shield failed public verification"
            );
            Ok(bytes)
        })
        .await?
    }

    async fn scan_unspent(
        self: Arc<Self>,
        client: &quil_types::proto::node::node_service_client::NodeServiceClient<
            tonic::transport::Channel,
        >,
    ) -> Result<Vec<OwnedCoin>, ServiceError> {
        let mut scan = Scan {
            snapshot_id: None,
            root: None,
            after: None,
            remaining_pages: MAX_PAGES,
            finished: false,
        };
        let mut unspent = Vec::new();
        while !scan.finished {
            if scan.remaining_pages == 0 {
                return Err(ServiceError::Wallet(format!(
                    "Coin scan exceeded {MAX_PAGES} pages; refusing to return an incomplete balance"
                )));
            }
            let response = tokio::time::timeout(
                node::RPC_TIMEOUT,
                client
                    .clone()
                    .max_decoding_message_size(MAX_PAGE_BYTES)
                    .list_coins(ListCoinsRequest {
                        domain: self.application.to_vec(),
                        snapshot_id: scan
                            .snapshot_id
                            .map(|value| value.to_vec())
                            .unwrap_or_default(),
                        after: scan.after.map(|value| value.to_vec()).unwrap_or_default(),
                        max_coins: SCAN_PAGE_ENTRIES as u32,
                    }),
            )
            .await
            .map_err(|_| ServiceError::Timeout)?
            .map_err(|error| ServiceError::Rpc(error.to_string()))?
            .into_inner();
            let page = self.decode_page(&mut scan, response)?;
            let wallet = self.clone();
            let recovered = tokio::task::spawn_blocking(move || wallet.recover_page(page))
                .await
                .map_err(|error| ServiceError::Wallet(error.to_string()))??;
            for coin in recovered {
                if self.is_unspent_hint(client, &coin.image).await? {
                    if unspent.len() >= MAX_COINS {
                        return Err(ServiceError::Wallet(format!(
                            "Wallet owns more than {MAX_COINS} Coins; refusing to return an incomplete balance"
                        )));
                    }
                    unspent.push(coin);
                }
            }
        }
        Ok(unspent)
    }

    async fn scan_escrows(
        self: Arc<Self>,
        client: &quil_types::proto::node::node_service_client::NodeServiceClient<
            tonic::transport::Channel,
        >,
        authority: &[u8],
    ) -> Result<Vec<PendingEscrowSnapshot>, ServiceError> {
        let mut scan = Scan {
            snapshot_id: None,
            root: None,
            after: None,
            remaining_pages: MAX_PAGES,
            finished: false,
        };
        let mut owned = Vec::new();
        while !scan.finished {
            if scan.remaining_pages == 0 {
                return Err(ServiceError::Wallet(format!(
                    "Pending transfers exceeded {MAX_PAGES} pages; refusing to return incomplete results"
                )));
            }
            let response = tokio::time::timeout(
                node::RPC_TIMEOUT,
                client
                    .clone()
                    .max_decoding_message_size(MAX_PAGE_BYTES)
                    .list_escrows(ListCoinsRequest {
                        domain: self.application.to_vec(),
                        snapshot_id: scan
                            .snapshot_id
                            .map(|value| value.to_vec())
                            .unwrap_or_default(),
                        after: scan.after.map(|value| value.to_vec()).unwrap_or_default(),
                        max_coins: SCAN_PAGE_ENTRIES as u32,
                    }),
            )
            .await
            .map_err(|_| ServiceError::Timeout)?
            .map_err(|error| ServiceError::Rpc(error.to_string()))?
            .into_inner();
            let wallet = self.clone();
            let authority = authority.to_vec();
            let (next_scan, recovered) = tokio::task::spawn_blocking(move || {
                let page = wallet.decode_escrow_page(&mut scan, response)?;
                let mut recovered = Vec::new();
                for (address, escrow) in page {
                    let recipient_memo = EscrowRecoveryMemo {
                        owner: escrow.output.owner,
                        ciphertext: escrow.output.memo,
                    };
                    for (branch, claimant, memo) in [
                        (
                            PendingEscrowBranch::Recipient,
                            escrow.policy.recipient.as_slice(),
                            &recipient_memo,
                        ),
                        (
                            PendingEscrowBranch::Refund,
                            escrow.policy.refund.as_slice(),
                            &escrow.refund_recovery,
                        ),
                    ] {
                        if claimant != authority.as_slice() {
                            continue;
                        }
                        if let Ok(opened) = open_escrow_recovery(
                            &wallet.context,
                            &wallet.kem_secret,
                            &wallet.recipient,
                            &escrow.output.commitment,
                            memo,
                        ) {
                            recovered.push(PendingEscrowSnapshot {
                                address_hex: hex::encode(address),
                                amount_base_units: opened.amount(),
                                branch,
                                refund_after_global_frame: escrow.policy.refund_after_global_frame,
                            });
                        }
                    }
                }
                Ok::<_, ServiceError>((scan, recovered))
            })
            .await
            .map_err(|error| ServiceError::Wallet(error.to_string()))??;
            scan = next_scan;
            for escrow in recovered {
                let escrow_address: [u8; 32] = hex::decode(&escrow.address_hex)
                    .map_err(|error| ServiceError::Wallet(error.to_string()))?
                    .try_into()
                    .map_err(|_| {
                        ServiceError::Wallet("Invalid pending-transfer address length".into())
                    })?;
                let record = quil_execution::token_intrinsic::global_commit::escrow_address(
                    &self.application,
                    &escrow_address,
                )
                .map_err(|error| ServiceError::Wallet(error.to_string()))?;
                let (present, raw_data) = self.global_record(client, &record).await?;
                let open = present
                    && !quil_execution::token_intrinsic::global_commit::parse_escrow_record(
                        &raw_data,
                    )
                    .map_err(|error| {
                        ServiceError::Wallet(format!("Invalid pending-transfer record: {error}"))
                    })?
                    .2;
                if open {
                    if owned.len() >= MAX_COINS {
                        return Err(ServiceError::Wallet(format!(
                            "More than {MAX_COINS} pending transfers belong to the current identity"
                        )));
                    }
                    owned.push(escrow);
                }
            }
        }
        Ok(owned)
    }

    fn decode_escrow_page(
        &self,
        scan: &mut Scan,
        response: ListEscrowsResponse,
    ) -> Result<
        Vec<(
            [u8; 32],
            quil_execution::token_intrinsic::escrow::StoredEscrow,
        )>,
        ServiceError,
    > {
        if response.encoded_len() > MAX_PAGE_BYTES || response.network.as_slice() != self.network {
            return Err(ServiceError::Wallet(
                "Invalid pending-transfer page size or network identifier".into(),
            ));
        }
        let snapshot_id: [u8; 32] = response.snapshot_id.try_into().map_err(|_| {
            ServiceError::Wallet("Invalid pending-transfer snapshot identifier".into())
        })?;
        if scan.snapshot_id.is_some_and(|old| old != snapshot_id) {
            return Err(ServiceError::Wallet(
                "Pending-transfer snapshot changed during the scan".into(),
            ));
        }
        let cursor = if response.cursor.is_empty() {
            None
        } else {
            Some(
                response
                    .cursor
                    .try_into()
                    .map_err(|_| ServiceError::Wallet("Invalid pending-transfer cursor".into()))?,
            )
        };
        if response.escrows.len() > SCAN_PAGE_ENTRIES
            || cursor < scan.after
            || (response.has_more && (cursor.is_none() || cursor <= scan.after))
        {
            return Err(ServiceError::Wallet(
                "Pending-transfer pagination did not advance correctly".into(),
            ));
        }
        let mut previous = scan.after;
        let mut escrows = Vec::with_capacity(response.escrows.len());
        for escrow in response.escrows {
            let address: [u8; 32] = escrow.address.try_into().map_err(|_| {
                ServiceError::Wallet("Invalid pending-transfer address length".into())
            })?;
            if previous.is_some_and(|old| address <= old) || cursor.is_none_or(|end| address > end)
            {
                return Err(ServiceError::Wallet(
                    "Invalid pending-transfer result order".into(),
                ));
            }
            let stored = quil_execution::token_intrinsic::escrow::decode_escrow_blob(
                &escrow.raw_data,
                &self.context,
                &address,
            )
            .map_err(|error| {
                ServiceError::Wallet(format!("Invalid pending-transfer content: {error}"))
            })?;
            escrows.push((address, stored));
            previous = Some(address);
        }
        scan.snapshot_id = Some(snapshot_id);
        scan.after = cursor;
        scan.remaining_pages -= 1;
        scan.finished = !response.has_more;
        Ok(escrows)
    }

    fn decode_page(
        &self,
        scan: &mut Scan,
        response: ListCoinsResponse,
    ) -> Result<Vec<([u8; 32], Output)>, ServiceError> {
        if response.encoded_len() > MAX_PAGE_BYTES || response.network.as_slice() != self.network {
            return Err(ServiceError::Wallet(
                "Invalid Coin page size or network identifier".into(),
            ));
        }
        let snapshot_id: [u8; 32] = response
            .snapshot_id
            .try_into()
            .map_err(|_| ServiceError::Wallet("Invalid Coin snapshot identifier".into()))?;
        if scan.snapshot_id.is_some_and(|old| old != snapshot_id) {
            return Err(ServiceError::Wallet(
                "Coin snapshot changed during the scan".into(),
            ));
        }
        let root = RootRecord::decode(&response.root_record, &self.context)
            .map_err(|error| ServiceError::Wallet(format!("Invalid Coin root: {error:?}")))?;
        if scan.root.as_ref().is_some_and(|old| old != &root) {
            return Err(ServiceError::Wallet(
                "Coin root changed during the scan".into(),
            ));
        }
        let cursor = if response.cursor.is_empty() {
            None
        } else {
            Some(
                response
                    .cursor
                    .try_into()
                    .map_err(|_| ServiceError::Wallet("Invalid Coin cursor".into()))?,
            )
        };
        if response.coins.len() > SCAN_PAGE_ENTRIES
            || cursor < scan.after
            || (response.has_more && (cursor.is_none() || cursor <= scan.after))
        {
            return Err(ServiceError::Wallet(
                "Coin pagination did not advance correctly".into(),
            ));
        }

        let mut previous = scan.after;
        let mut coins = Vec::with_capacity(response.coins.len());
        for coin in response.coins {
            let address: [u8; 32] = coin
                .address
                .try_into()
                .map_err(|_| ServiceError::Wallet("Invalid Coin address length".into()))?;
            if previous.is_some_and(|old| address <= old) || cursor.is_none_or(|end| address > end)
            {
                return Err(ServiceError::Wallet("Invalid Coin result order".into()));
            }
            let output = Output {
                owner: coin
                    .owner
                    .try_into()
                    .map_err(|_| ServiceError::Wallet("Invalid Coin owner length".into()))?,
                commitment: AmountCommitment::from_bytes(&coin.commitment).map_err(|error| {
                    ServiceError::Wallet(format!("Invalid Coin commitment: {error:?}"))
                })?,
                memo: coin
                    .memo
                    .try_into()
                    .map_err(|_| ServiceError::Wallet("Invalid Coin memo length".into()))?,
            };
            let (actual, _) = quil_execution::token_intrinsic::state::create_coin(
                &self.context,
                coin.frame_number,
                &output,
                coin.position,
            )
            .map_err(|error| ServiceError::Wallet(error.to_string()))?;
            if actual != address {
                return Err(ServiceError::Wallet("Coin content address mismatch".into()));
            }
            let (block, _) = quil_execution::token_intrinsic::coin_blocks::locate(coin.position);
            let belongs = quil_execution::token_intrinsic::coin_blocks::block_for_address(
                quil_execution::token_intrinsic::coin_blocks::creation_width(block),
                &address,
            )
            .is_ok_and(|expected| expected == block);
            if !belongs {
                return Err(ServiceError::Wallet(
                    "Coin position does not match its address".into(),
                ));
            }
            coins.push((address, output));
            previous = Some(address);
        }
        scan.snapshot_id = Some(snapshot_id);
        scan.root = Some(root);
        scan.after = cursor;
        scan.remaining_pages -= 1;
        scan.finished = !response.has_more;
        Ok(coins)
    }

    fn recover_page(&self, page: Vec<([u8; 32], Output)>) -> Result<Vec<OwnedCoin>, ServiceError> {
        let key = MembershipKey::derive(&self.context);
        let mut recovered = Vec::new();
        for (address, output) in page {
            let opened =
                match open_output(&self.context, &self.kem_secret, &self.recipient, &output) {
                    Ok(opened) => opened,
                    Err(MemoError::Authentication) => continue,
                    Err(error) => {
                        return Err(ServiceError::Wallet(format!(
                            "Coin memo recovery failed: {error:?}"
                        )))
                    }
                };
            let image = key
                .key_image(&opened.secrets.owner)
                .identity_bytes()
                .map_err(|error| ServiceError::Wallet(format!("Invalid Coin image: {error:?}")))?;
            recovered.push(OwnedCoin {
                address,
                output,
                amount: Zeroizing::new(opened.amount),
                image,
            });
        }
        Ok(recovered)
    }

    async fn is_unspent_hint(
        &self,
        client: &quil_types::proto::node::node_service_client::NodeServiceClient<
            tonic::transport::Channel,
        >,
        image: &[u8; IDENTITY_BYTES],
    ) -> Result<bool, ServiceError> {
        let marker = quil_execution::token_intrinsic::spent::marker_address(
            &self.network,
            &self.application,
            image,
        )
        .map_err(|error| ServiceError::Wallet(error.to_string()))?;
        let record = quil_execution::token_intrinsic::global_commit::consumed_address(
            &self.application,
            &marker,
        )
        .map_err(|error| ServiceError::Wallet(error.to_string()))?;
        Ok(!self.global_record(client, &record).await?.0)
    }

    async fn global_record(
        &self,
        client: &quil_types::proto::node::node_service_client::NodeServiceClient<
            tonic::transport::Channel,
        >,
        record: &[u8; 32],
    ) -> Result<(bool, Vec<u8>), ServiceError> {
        read_global_record(client, record).await
    }
}

pub(super) async fn read_global_record(
    client: &quil_types::proto::node::node_service_client::NodeServiceClient<
        tonic::transport::Channel,
    >,
    record: &[u8; 32],
) -> Result<(bool, Vec<u8>), ServiceError> {
    let global = quil_execution::global_schema::GLOBAL_INTRINSIC_ADDRESS;
    let mut address = global.to_vec();
    address.extend_from_slice(record);
    let response = tokio::time::timeout(
        node::RPC_TIMEOUT,
        client
            .clone()
            .max_decoding_message_size(66_560)
            .get_vertex_data(GetVertexDataRequest {
                address,
                full_data: true,
            }),
    )
    .await
    .map_err(|_| ServiceError::Timeout)?
    .map_err(|error| ServiceError::Rpc(error.to_string()))?
    .into_inner();
    let expected_l1 = quil_hypergraph::addressing::get_bloom_filter_indices(&global, 256, 3);
    if response.set_type != "vertex"
        || response.phase_type != "adds"
        || response.shard_l2 != global
        || response.shard_l1 != expected_l1
        || !response.entries.is_empty()
    {
        return Err(ServiceError::Wallet(
            "GLOBAL record returned the wrong shard or representation".into(),
        ));
    }
    let present = response.present.ok_or_else(|| {
        ServiceError::Wallet("Node did not return GLOBAL record existence".into())
    })?;
    if !present && !response.raw_data.is_empty() {
        return Err(ServiceError::Wallet(
            "A nonexistent GLOBAL record unexpectedly contains data".into(),
        ));
    }
    Ok((present, response.raw_data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use quil_execution::token_intrinsic::escrow::StoredEscrow;
    use quil_lattice_ct::confidential::{
        memo::create_escrow_recovery, pending_claim::EscrowPolicy, transfer::Output,
    };

    #[test]
    fn rejects_invalid_onion_key_lengths_before_address_derivation() {
        let error = Wallet::from_keys(
            [0; 32],
            quil_execution::domains::QUIL_TOKEN,
            &[1; 10],
            Zeroizing::new(vec![2; 10]),
        )
        .err()
        .unwrap();

        assert!(error.to_string().contains("sntrup761"));
    }

    #[test]
    fn escrow_page_is_content_bound_and_failure_does_not_advance_scan() {
        let keys = quil_crypto::sntrup761::Sntrup761KeyPair::generate();
        let wallet = Wallet::from_keys(
            [1; 32],
            quil_execution::domains::QUIL_TOKEN,
            &keys.public,
            Zeroizing::new(keys.secret),
        )
        .unwrap();
        let recovery =
            create_escrow_recovery(&wallet.context, &wallet.address, &wallet.address, 42).unwrap();
        let stored = StoredEscrow {
            frame_number: 7,
            output: Output {
                commitment: recovery.commitment,
                owner: recovery.recipient.owner,
                memo: recovery.recipient.ciphertext,
            },
            policy: EscrowPolicy {
                recipient: [2; quil_crypto::FALCON_PUBLIC_KEY_LEN],
                refund: [3; quil_crypto::FALCON_PUBLIC_KEY_LEN],
                refund_after_global_frame: 99,
            },
            refund_recovery: recovery.refund,
        };
        let (address, raw_data) = stored.encode(&wallet.context).unwrap();
        let response = ListEscrowsResponse {
            network: wallet.network.to_vec(),
            snapshot_id: vec![4; 32],
            escrows: vec![quil_types::proto::node::Escrow {
                address: address.to_vec(),
                raw_data,
            }],
            cursor: address.to_vec(),
            has_more: true,
        };
        let mut scan = Scan {
            snapshot_id: None,
            root: None,
            after: None,
            remaining_pages: 2,
            finished: false,
        };

        let mut bad = response.clone();
        bad.escrows[0].address[0] ^= 1;
        assert!(wallet.decode_escrow_page(&mut scan, bad).is_err());
        assert!(scan.snapshot_id.is_none());
        assert_eq!(scan.remaining_pages, 2);

        let decoded = wallet.decode_escrow_page(&mut scan, response).unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].0, address);
        assert_eq!(scan.snapshot_id, Some([4; 32]));
        assert_eq!(scan.remaining_pages, 1);
    }

    #[test]
    fn escrow_scan_accepts_pages_larger_than_the_legacy_eight_entries() {
        let keys = quil_crypto::sntrup761::Sntrup761KeyPair::generate();
        let wallet = Wallet::from_keys(
            [1; 32],
            quil_execution::domains::QUIL_TOKEN,
            &keys.public,
            Zeroizing::new(keys.secret),
        )
        .unwrap();
        let mut escrows = (1..=9)
            .map(|amount| {
                let recovery = create_escrow_recovery(
                    &wallet.context,
                    &wallet.address,
                    &wallet.address,
                    amount,
                )
                .unwrap();
                let stored = StoredEscrow {
                    frame_number: 7,
                    output: Output {
                        commitment: recovery.commitment,
                        owner: recovery.recipient.owner,
                        memo: recovery.recipient.ciphertext,
                    },
                    policy: EscrowPolicy {
                        recipient: [2; quil_crypto::FALCON_PUBLIC_KEY_LEN],
                        refund: [3; quil_crypto::FALCON_PUBLIC_KEY_LEN],
                        refund_after_global_frame: 99,
                    },
                    refund_recovery: recovery.refund,
                };
                let (address, raw_data) = stored.encode(&wallet.context).unwrap();
                quil_types::proto::node::Escrow {
                    address: address.to_vec(),
                    raw_data,
                }
            })
            .collect::<Vec<_>>();
        escrows.sort_by(|left, right| left.address.cmp(&right.address));
        let cursor = escrows.last().unwrap().address.clone();
        let response = ListEscrowsResponse {
            network: wallet.network.to_vec(),
            snapshot_id: vec![4; 32],
            escrows,
            cursor,
            has_more: true,
        };
        let mut scan = Scan {
            snapshot_id: None,
            root: None,
            after: None,
            remaining_pages: 2,
            finished: false,
        };

        let decoded = wallet.decode_escrow_page(&mut scan, response).unwrap();

        assert_eq!(decoded.len(), 9);
        assert_eq!(scan.remaining_pages, 1);
    }

    #[test]
    fn wallet_operation_journal_persists_exact_payload_and_unknown_outcome() {
        use super::operation_journal::{OperationJournal, OperationJournalPhase};

        let config = tempfile::tempdir().unwrap();
        let payload = b"canonical confidential operation";
        let journal = OperationJournal::create(
            config.path(),
            WalletAction::Transfer,
            quil_execution::domains::QUIL_TOKEN,
            payload,
        )
        .unwrap();
        journal
            .record_submission(OperationJournalPhase::Unknown, Some("response lost".into()))
            .unwrap();

        let states = OperationJournal::scan(config.path()).unwrap();
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].phase, OperationJournalPhase::Unknown);
        assert_eq!(states[0].detail.as_deref(), Some("response lost"));
        assert_eq!(journal.read_operation().unwrap(), payload);
        assert_eq!(
            states[0].operation_id,
            hex::encode(quil_execution::token_intrinsic::global_commit::tx_id(
                payload
            ))
        );
    }
}
