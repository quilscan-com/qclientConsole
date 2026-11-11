use std::path::Path;

use quil_types::proto::global::{ListLegacyCoinsRequest, ListLegacyCoinsResponse};

use super::{node, read_rpc, ServiceError};
use crate::domain::{LegacyAssetsSnapshot, LegacyCoinSnapshot};

const MAX_PAGES_PER_OWNER: usize = 2048;

pub(super) async fn read(config_dir: &Path) -> Result<LegacyAssetsSnapshot, ServiceError> {
    let config = quil_config::load_config(config_dir)
        .map_err(|error| ServiceError::Config(error.to_string()))?;
    let Some(owners) = owner_addresses(&config.p2p.peer_priv_key) else {
        return Ok(LegacyAssetsSnapshot::default());
    };

    let read_rpc = read_rpc::load(config_dir, config.effective_listen_grpc_multiaddr())?;
    let address = node::grpc_address(&read_rpc)?;
    let mut client = node::connect_client(&address).await?;
    let mut coins = Vec::new();
    for owner in owners {
        let mut after = Vec::new();
        let mut complete = false;
        for _ in 0..MAX_PAGES_PER_OWNER {
            let response = tokio::time::timeout(
                node::RPC_TIMEOUT,
                client.list_legacy_coins(ListLegacyCoinsRequest {
                    domain: quil_execution::domains::QUIL_TOKEN.to_vec(),
                    owner: owner.to_vec(),
                    after: after.clone(),
                }),
            )
            .await
            .map_err(|_| ServiceError::Timeout)?
            .map_err(map_rpc_error)?
            .into_inner();
            let page = decode_page(&after, response)?;
            coins.extend(page.coins);
            if !page.has_more {
                complete = true;
                break;
            }
            after = page.next_cursor;
        }
        if !complete {
            return Err(ServiceError::LegacyAssets(format!(
                "An address exceeded {MAX_PAGES_PER_OWNER} pages; stopped reading"
            )));
        }
    }

    coins.sort_by(|left, right| left.address_hex.cmp(&right.address_hex));
    summarize(coins)
}

fn owner_addresses(peer_private_key_hex: &str) -> Option<Vec<[u8; 32]>> {
    let identity =
        quil_p2p::ed448_identity::Ed448Identity::from_config_hex(peer_private_key_hex).ok()?;
    let by_key = quil_crypto::poseidon::hash_bytes_to_32(&identity.public_key).ok()?;
    let by_peer = quil_crypto::poseidon::hash_bytes_to_32(&identity.peer_id_bytes).ok()?;
    let mut owners = vec![by_key, by_peer];
    owners.dedup();
    Some(owners)
}

fn map_rpc_error(status: tonic::Status) -> ServiceError {
    if status.code() == tonic::Code::Unimplemented {
        ServiceError::LegacyAssets(
            "This node does not support legacy asset queries; upgrade the node".into(),
        )
    } else {
        ServiceError::Rpc(status.message().into())
    }
}

struct DecodedPage {
    coins: Vec<LegacyCoinSnapshot>,
    next_cursor: Vec<u8>,
    has_more: bool,
}

fn decode_page(
    after: &[u8],
    response: ListLegacyCoinsResponse,
) -> Result<DecodedPage, ServiceError> {
    let mut cursor = after.to_vec();
    let mut coins = Vec::with_capacity(response.coins.len());
    for coin in response.coins {
        if coin.address.len() != 32 || coin.address.as_slice() <= cursor.as_slice() {
            return Err(ServiceError::LegacyAssets(
                "The node returned an invalid or non-increasing Coin address".into(),
            ));
        }
        let amount = u128::from_le_bytes(
            coin.amount
                .as_slice()
                .try_into()
                .map_err(|_| ServiceError::LegacyAssets("Invalid Coin amount length".into()))?,
        );
        cursor = coin.address.clone();
        coins.push(LegacyCoinSnapshot {
            address_hex: hex::encode(coin.address),
            amount_base_units: amount,
            shielded: coin.shielded,
        });
    }
    if response.has_more && coins.is_empty() {
        return Err(ServiceError::LegacyAssets(
            "Legacy asset pagination did not advance".into(),
        ));
    }
    Ok(DecodedPage {
        coins,
        next_cursor: cursor,
        has_more: response.has_more,
    })
}

fn summarize(coins: Vec<LegacyCoinSnapshot>) -> Result<LegacyAssetsSnapshot, ServiceError> {
    let mut snapshot = LegacyAssetsSnapshot {
        identity_available: true,
        coins,
        ..Default::default()
    };
    for coin in &snapshot.coins {
        if coin.shielded {
            snapshot.shielded_count += 1;
        } else {
            snapshot.unshielded_count += 1;
            snapshot.unshielded_total_base_units = snapshot
                .unshielded_total_base_units
                .checked_add(coin.amount_base_units)
                .ok_or_else(|| ServiceError::LegacyAssets("Legacy asset total overflow".into()))?;
        }
    }
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use quil_types::proto::global::LegacyCoin;

    use super::*;

    #[test]
    fn derives_both_legacy_owner_forms_only_for_ed448() {
        let identity = quil_p2p::ed448_identity::Ed448Identity::generate().unwrap();
        let owners = owner_addresses(&identity.to_config_hex()).unwrap();

        assert_eq!(owners.len(), 2);
        assert_eq!(owner_addresses(""), None);
    }

    #[test]
    fn decodes_strictly_ordered_page_and_summarizes_spendable_total() {
        let page = decode_page(
            &[],
            ListLegacyCoinsResponse {
                coins: vec![coin(1, 8_000_000_000, false), coin(2, 4_000_000_000, true)],
                has_more: false,
                ..Default::default()
            },
        )
        .unwrap();
        let snapshot = summarize(page.coins).unwrap();

        assert_eq!(snapshot.unshielded_total_base_units, 8_000_000_000);
        assert_eq!(snapshot.unshielded_count, 1);
        assert_eq!(snapshot.shielded_count, 1);
    }

    #[test]
    fn rejects_non_advancing_pages() {
        let result = decode_page(
            &[2; 32],
            ListLegacyCoinsResponse {
                coins: vec![coin(1, 1, false)],
                has_more: true,
                ..Default::default()
            },
        );

        assert!(result.is_err());
    }

    fn coin(last_byte: u8, amount: u128, shielded: bool) -> LegacyCoin {
        let mut address = vec![0; 32];
        address[31] = last_byte;
        LegacyCoin {
            address,
            amount: amount.to_le_bytes().to_vec(),
            shielded,
            ..Default::default()
        }
    }
}
