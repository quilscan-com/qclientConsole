//! Console-owned legacy migration adapter built on public monorepo APIs.

use std::{collections::BTreeMap, sync::Arc};

use quil_client::commands::token::TokenCtx;
use quil_lattice_ct::confidential::{
    address::RecipientAddress,
    shield::{ShieldSource, MAX_SHIELD_SOURCES},
};
use quil_types::{
    crypto::{KeyType, Signer},
    proto::{
        global::{ListLegacyCoinsRequest, MessageBundle},
        node::{GetNodeInfoRequest, GetShardInfoRequest, SendRequest},
    },
};

use crate::service::wallet::{self, Wallet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FeeQuoteDisplay {
    Silent,
}

pub(super) struct LegacySourceIdentity {
    signer: Arc<dyn Signer>,
    public_key: [u8; 57],
    network: [u8; 32],
}

impl LegacySourceIdentity {
    pub(super) fn load(context: &TokenCtx) -> anyhow::Result<Self> {
        let raw = zeroize::Zeroizing::new(hex::decode(&context.node_config.p2p.peer_priv_key)?);
        anyhow::ensure!(
            raw.len() == 114,
            "peer key must be 57-byte seed plus 57-byte public key"
        );
        let signer = Arc::new(quil_crypto::Ed448Signer::from_bytes(
            &raw[..57],
            &raw[57..],
        )?) as Arc<dyn Signer>;
        let public_key: [u8; 57] = signer
            .public_key()
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid Ed448 public key"))?;
        let challenge = b"qclient-console/legacy-source-key-check";
        anyhow::ensure!(
            quil_crypto::ed448_verify(&public_key, challenge, &signer.sign(challenge)?),
            "legacy Ed448 public and private keys do not match"
        );
        Ok(Self {
            signer,
            public_key,
            network: network(context),
        })
    }

    fn owner_addresses(&self) -> anyhow::Result<[[u8; 32]; 2]> {
        Ok([
            quil_crypto::poseidon::hash_bytes_to_32(&self.public_key)
                .map_err(|error| anyhow::anyhow!("legacy owner: {error}"))?,
            quil_crypto::poseidon::hash_bytes_to_32(
                &quil_crypto::peer_id_multihash_from_ed448_pubkey(&self.public_key),
            )
            .map_err(|error| anyhow::anyhow!("legacy peer owner: {error}"))?,
        ])
    }
}

pub(super) struct RecipientIdentity {
    wallet: Arc<Wallet>,
    network: [u8; 32],
}

impl RecipientIdentity {
    pub(super) fn load(context: &TokenCtx) -> anyhow::Result<Self> {
        let secret = zeroize::Zeroizing::new(
            context
                .key_manager
                .get_secret_key_bytes_by_id("q-onion-key")?,
        );
        let public = context
            .key_manager
            .get_public_key_bytes_by_id("q-onion-key")?;
        let network = network(context);
        let wallet = Wallet::from_keys(
            network,
            quil_execution::domains::QUIL_TOKEN,
            &public,
            secret,
        )
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        Ok(Self {
            wallet: Arc::new(wallet),
            network,
        })
    }

    pub(super) fn address(&self) -> &RecipientAddress {
        self.wallet.address()
    }
}

pub(super) struct SubmitIdentity {
    keys: Arc<quil_keys::FileKeyManager>,
}

impl SubmitIdentity {
    pub(super) fn new(keys: Arc<quil_keys::FileKeyManager>) -> anyhow::Result<Self> {
        let signer = keys.get_signer_by_id("q-peer-key")?;
        anyhow::ensure!(
            signer.key_type() == KeyType::Ed448,
            "Send requires an Ed448 q-peer-key"
        );
        let challenge = b"qclient-console/legacy-submit-key-check";
        anyhow::ensure!(
            quil_crypto::ed448_verify(signer.public_key(), challenge, &signer.sign(challenge)?),
            "q-peer public and private keys do not match"
        );
        drop(signer);
        Ok(Self { keys })
    }
}

fn network(context: &TokenCtx) -> [u8; 32] {
    quil_lattice_ct::confidential::transfer::network_identifier(context.node_config.p2p.network)
}

pub(super) struct MigrationEndpoints<'a> {
    read: &'a TokenCtx,
    submit: &'a TokenCtx,
}

impl<'a> MigrationEndpoints<'a> {
    pub(super) fn new(read: &'a TokenCtx, submit: &'a TokenCtx) -> anyhow::Result<Self> {
        anyhow::ensure!(
            network(read) == network(submit),
            "migration endpoints belong to different networks"
        );
        Ok(Self { read, submit })
    }

    pub(super) async fn live_shards(&self) -> anyhow::Result<Vec<Vec<bool>>> {
        let client = self.read.connect().await?;
        ensure_shields_active(&client, self.read.node_config.p2p.network).await?;
        live_shard_paths(&client, &quil_execution::domains::QUIL_TOKEN).await
    }
}

#[derive(Clone, Debug)]
pub(super) struct LegacyAsset {
    pub address: [u8; 32],
    pub amount: u128,
    pub shielded: bool,
}

#[derive(Clone, Debug)]
pub(super) struct LegacyAssetSet {
    pub coins: Vec<LegacyAsset>,
}

pub(super) async fn discover_legacy_assets(
    endpoints: &MigrationEndpoints<'_>,
    source: &LegacySourceIdentity,
) -> anyhow::Result<LegacyAssetSet> {
    anyhow::ensure!(
        source.network == network(endpoints.read),
        "source and read endpoint networks differ"
    );
    let client = endpoints.read.connect().await?;
    let mut coins = Vec::new();
    for owner in source.owner_addresses()? {
        let mut after = Vec::new();
        loop {
            let page = client
                .clone()
                .list_legacy_coins(ListLegacyCoinsRequest {
                    domain: quil_execution::domains::QUIL_TOKEN.to_vec(),
                    owner: owner.to_vec(),
                    after: after.clone(),
                })
                .await
                .map_err(|status| anyhow::anyhow!("legacy coins: {}", status.message()))?
                .into_inner();
            for coin in page.coins {
                anyhow::ensure!(
                    coin.address.len() == 32 && coin.address > after,
                    "legacy coins out of order"
                );
                let address: [u8; 32] = coin.address.as_slice().try_into().unwrap();
                let amount = u128::from_le_bytes(
                    coin.amount
                        .as_slice()
                        .try_into()
                        .map_err(|_| anyhow::anyhow!("malformed legacy coin amount"))?,
                );
                after = coin.address;
                coins.push(LegacyAsset {
                    address,
                    amount,
                    shielded: coin.shielded,
                });
            }
            if !page.has_more {
                break;
            }
            anyhow::ensure!(!after.is_empty(), "legacy coin listing did not advance");
        }
    }
    coins.sort_by_key(|coin| coin.address);
    coins.dedup_by_key(|coin| coin.address);
    Ok(LegacyAssetSet { coins })
}

#[derive(Clone, Debug)]
pub(super) struct ShieldBatch {
    pub shard: Vec<bool>,
    pub sources: Vec<ShieldSource>,
}

#[derive(Clone, Debug)]
pub(super) struct GroupedLegacySources {
    pub batches: Vec<ShieldBatch>,
    pub uncovered: Vec<[u8; 32]>,
}

pub(super) fn group_legacy_sources(
    mut coins: Vec<([u8; 32], u128)>,
    live_shards: &[Vec<bool>],
    max_per_batch: usize,
) -> anyhow::Result<GroupedLegacySources> {
    anyhow::ensure!(
        (1..=MAX_SHIELD_SOURCES).contains(&max_per_batch),
        "a batch holds 1 to {MAX_SHIELD_SOURCES} legacy coins"
    );
    anyhow::ensure!(
        live_shards.iter().all(|shard| shard.len() <= 256),
        "shard path exceeds address width"
    );
    coins.sort();
    coins.dedup_by_key(|(address, _)| *address);
    let mut groups: BTreeMap<Vec<bool>, Vec<ShieldSource>> = BTreeMap::new();
    let mut uncovered = Vec::new();
    for (address, amount) in coins {
        match deepest_shard(live_shards, &address) {
            Some(path) => groups
                .entry(path.clone())
                .or_default()
                .push(ShieldSource { address, amount }),
            None => uncovered.push(address),
        }
    }
    let batches = groups
        .into_iter()
        .flat_map(|(shard, sources)| {
            sources
                .chunks(max_per_batch)
                .map(|sources| ShieldBatch {
                    shard: shard.clone(),
                    sources: sources.to_vec(),
                })
                .collect::<Vec<_>>()
        })
        .collect();
    Ok(GroupedLegacySources { batches, uncovered })
}

#[derive(Clone, Debug)]
pub(super) struct PreparedShieldBatch {
    pub operation: Vec<u8>,
    pub source_addresses: Vec<[u8; 32]>,
    pub shard: Vec<bool>,
    pub fee: u128,
}

pub(super) async fn quote_shield_fee(
    endpoints: &MigrationEndpoints<'_>,
    source_count: usize,
    _display: FeeQuoteDisplay,
) -> anyhow::Result<u128> {
    anyhow::ensure!(
        (1..=MAX_SHIELD_SOURCES).contains(&source_count),
        "a batch holds 1 to {MAX_SHIELD_SOURCES} legacy coins"
    );
    let network = network(endpoints.submit);
    let cost = wallet::fees::growth_for_shape(
        &network,
        &quil_execution::domains::QUIL_TOKEN,
        quil_execution::token_intrinsic::cost::Shape {
            coins: 1,
            markers: source_count,
            escrow: false,
        },
    )?;
    let mut client = endpoints.submit.connect_submit().await?;
    Ok(wallet::fees::quoted_quil_fee_for_cost(
        &mut client,
        &network,
        None,
        cost,
        false,
        endpoints.submit.max_fee,
    )
    .await?)
}

pub(super) async fn prepare_shield_batches(
    source: &LegacySourceIdentity,
    recipient: &RecipientIdentity,
    batches: &[ShieldBatch],
    fees: &[u128],
) -> anyhow::Result<Vec<PreparedShieldBatch>> {
    anyhow::ensure!(
        source.network == recipient.network,
        "source and recipient networks differ"
    );
    anyhow::ensure!(batches.len() == fees.len(), "one fee is required per batch");
    let mut prepared = Vec::with_capacity(batches.len());
    for (batch, fee) in batches.iter().zip(fees) {
        anyhow::ensure!(
            !batch.sources.is_empty() && batch.sources.len() <= MAX_SHIELD_SOURCES,
            "invalid legacy shield batch"
        );
        anyhow::ensure!(
            batch.sources.iter().all(|source| deepest_shard(
                std::slice::from_ref(&batch.shard),
                &source.address
            )
            .is_some()),
            "batch sources span shards"
        );
        let total = batch
            .sources
            .iter()
            .try_fold(0u128, |sum, source| sum.checked_add(source.amount))
            .ok_or_else(|| anyhow::anyhow!("legacy sources overflow u128"))?;
        anyhow::ensure!(total > *fee, "legacy sources do not cover the fee");
        let operation = recipient
            .wallet
            .clone()
            .create_batch_shield(
                batch.sources.clone(),
                source.signer.clone(),
                vec![(recipient.address().clone(), total - fee)],
                *fee,
            )
            .await?;
        prepared.push(PreparedShieldBatch {
            operation,
            source_addresses: batch.sources.iter().map(|source| source.address).collect(),
            shard: batch.shard.clone(),
            fee: *fee,
        });
    }
    Ok(prepared)
}

#[derive(Debug, PartialEq, Eq)]
pub enum SubmitShieldOutcome {
    Accepted,
    Rejected(String),
    Unknown(String),
}

fn classify_submission_status(status: tonic::Status) -> SubmitShieldOutcome {
    use tonic::Code;
    let message = format!("{:?}: {}", status.code(), status.message());
    match status.code() {
        Code::InvalidArgument
        | Code::PermissionDenied
        | Code::FailedPrecondition
        | Code::AlreadyExists
        | Code::NotFound
        | Code::OutOfRange
        | Code::Unimplemented => SubmitShieldOutcome::Rejected(message),
        _ => SubmitShieldOutcome::Unknown(message),
    }
}

pub(super) async fn submit_saved_shield(
    endpoints: &MigrationEndpoints<'_>,
    recipient: &RecipientIdentity,
    submit: &SubmitIdentity,
    operation: Vec<u8>,
) -> anyhow::Result<SubmitShieldOutcome> {
    anyhow::ensure!(
        recipient.network == network(endpoints.submit),
        "recipient and submit endpoint networks differ"
    );
    let decoded = quil_lattice_ct::confidential::shield::AnyShield::decode(
        &operation,
        &recipient.network,
        &quil_execution::domains::QUIL_TOKEN,
    )
    .map_err(|error| anyhow::anyhow!("invalid saved shield: {error:?}"))?;
    let quil_lattice_ct::confidential::shield::AnyShield::Batch(shield) = decoded else {
        anyhow::bail!("saved migration shield must be a batch");
    };
    anyhow::ensure!(
        shield.statement.outputs.len() == 1,
        "saved migration shield must have exactly one output"
    );
    let expected_amount = shield
        .statement
        .total()
        .map_err(|error| anyhow::anyhow!("saved shield source total: {error:?}"))?
        .checked_sub(shield.statement.fee)
        .filter(|amount| *amount > 0)
        .ok_or_else(|| anyhow::anyhow!("saved shield sources must exceed the fee"))?;
    let opened = recipient.wallet.open(&shield.statement.outputs[0])?;
    anyhow::ensure!(
        opened.amount == expected_amount,
        "saved shield output amount does not equal sources minus fee"
    );
    let (domain, request) =
        wallet::submission::prepare_submission(&recipient.wallet, operation.clone())?;
    let mut client = endpoints.submit.connect_submit().await?;
    wallet::fees::refresh_submission_fee(&mut client, &recipient.network, &operation).await?;
    let bundle = MessageBundle {
        requests: vec![request],
        timestamp: quil_client::send::now_millis(),
    };
    let authentication = quil_client::send::sign_bundle(&submit.keys, &domain, &bundle)?;
    Ok(
        match client
            .send(tonic::Request::new(SendRequest {
                domain,
                request: Some(bundle),
                authentication,
                delivery_data: Vec::new(),
            }))
            .await
        {
            Ok(_) => SubmitShieldOutcome::Accepted,
            Err(status) => classify_submission_status(status),
        },
    )
}

pub(super) struct ShieldConfirmation {
    pub consumed: Vec<[u8; 32]>,
}

pub(super) async fn inspect_shield_confirmation(
    endpoints: &MigrationEndpoints<'_>,
    source: &LegacySourceIdentity,
    addresses: &[[u8; 32]],
) -> anyhow::Result<ShieldConfirmation> {
    let assets = discover_legacy_assets(endpoints, source).await?;
    Ok(ShieldConfirmation {
        consumed: addresses
            .iter()
            .filter(|address| {
                assets
                    .coins
                    .iter()
                    .any(|coin| &coin.address == *address && coin.shielded)
            })
            .copied()
            .collect(),
    })
}

pub(super) async fn inspect_saved_shield_output(
    endpoints: &MigrationEndpoints<'_>,
    recipient: &RecipientIdentity,
    operation: &[u8],
) -> anyhow::Result<bool> {
    anyhow::ensure!(
        recipient.network == network(endpoints.read),
        "recipient and read endpoint networks differ"
    );
    let decoded = quil_lattice_ct::confidential::shield::AnyShield::decode(
        operation,
        &recipient.network,
        &quil_execution::domains::QUIL_TOKEN,
    )
    .map_err(|error| anyhow::anyhow!("invalid saved shield: {error:?}"))?;
    let quil_lattice_ct::confidential::shield::AnyShield::Batch(shield) = decoded else {
        anyhow::bail!("saved migration shield must be a batch");
    };
    anyhow::ensure!(
        shield.statement.outputs.len() == 1,
        "saved migration shield must have exactly one output"
    );
    let target = &shield.statement.outputs[0];
    recipient.wallet.open(target)?;
    let client = endpoints.read.connect().await?;
    let outputs = recipient.wallet.clone().unspent_outputs(&client).await?;
    Ok(outputs.iter().any(|output| output == target))
}

fn deepest_shard<'a>(shards: &'a [Vec<bool>], address: &[u8; 32]) -> Option<&'a Vec<bool>> {
    shards
        .iter()
        .filter(|path| {
            path.iter()
                .enumerate()
                .all(|(index, bit)| (address[index / 8] & (0x80 >> (index % 8)) != 0) == *bit)
        })
        .max_by_key(|path| path.len())
}

async fn live_shard_paths(
    client: &quil_types::proto::node::node_service_client::NodeServiceClient<
        tonic::transport::Channel,
    >,
    application: &[u8; 32],
) -> anyhow::Result<Vec<Vec<bool>>> {
    let shards = client
        .clone()
        .get_shard_info(GetShardInfoRequest { include_all: true })
        .await
        .map_err(|error| anyhow::anyhow!("shard list: {}", error.message()))?
        .into_inner()
        .shards;
    Ok(shards
        .into_iter()
        .filter(|shard| shard.filter.starts_with(application) && shard.active_provers > 0)
        .filter_map(|shard| {
            quil_forest::decode_shard_filter_or_root(&shard.filter, 32).map(|(_, bits)| bits)
        })
        .collect())
}

async fn ensure_shields_active(
    client: &quil_types::proto::node::node_service_client::NodeServiceClient<
        tonic::transport::Channel,
    >,
    network: u8,
) -> anyhow::Result<()> {
    if network != 0 {
        return Ok(());
    }
    let from = quil_execution::token_intrinsic::legacy_migration::MAINNET_MIGRATED_OWNER_FRAME
        .max(quil_execution::token_intrinsic::global_commit::MAINNET_BATCH_SHIELD_FRAME);
    let head = client
        .clone()
        .get_node_info(GetNodeInfoRequest::default())
        .await
        .ok()
        .map(|response| response.into_inner().last_global_head_frame)
        .unwrap_or(0);
    if head > 0 {
        anyhow::ensure!(
            head >= from,
            "shields of legacy coins are admitted from GLOBAL frame {from}; current head is {head}"
        );
    }
    Ok(())
}
