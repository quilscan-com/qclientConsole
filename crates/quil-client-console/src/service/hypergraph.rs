use std::path::Path;

use quil_types::proto::node::{
    GetHyperedgeDataRequest, GetHyperedgeDataResponse, GetVertexDataRequest, GetVertexDataResponse,
    VertexDataEntry,
};

use super::{aliases, node, ServiceError};
use crate::domain::{AliasSnapshot, HypergraphEntrySnapshot, HypergraphKind, HypergraphSnapshot};

pub(super) async fn read(
    config_dir: &Path,
    kind: HypergraphKind,
    input: &str,
) -> Result<HypergraphSnapshot, ServiceError> {
    let address = match decode_address(input) {
        Ok(address) => address,
        Err(_) => resolve_address(input, &aliases::list(config_dir).await?)?,
    };
    let config = quil_config::load_config(config_dir)
        .map_err(|error| ServiceError::Config(error.to_string()))?;
    let rpc_address = node::grpc_address(config.effective_listen_grpc_multiaddr())?;
    let mut client = node::connect_client(&rpc_address).await?;

    match kind {
        HypergraphKind::Vertex => {
            let response = tokio::time::timeout(
                node::RPC_TIMEOUT,
                client.get_vertex_data(tonic::Request::new(GetVertexDataRequest {
                    address: address.clone(),
                    full_data: false,
                })),
            )
            .await
            .map_err(|_| ServiceError::Timeout)?
            .map_err(|error| ServiceError::Hypergraph(error.to_string()))?
            .into_inner();
            Ok(map_vertex(address, response))
        }
        HypergraphKind::Hyperedge => {
            let response = tokio::time::timeout(
                node::RPC_TIMEOUT,
                client.get_hyperedge_data(tonic::Request::new(GetHyperedgeDataRequest {
                    address: address.clone(),
                    full_data: false,
                })),
            )
            .await
            .map_err(|_| ServiceError::Timeout)?
            .map_err(|error| ServiceError::Hypergraph(error.to_string()))?
            .into_inner();
            Ok(map_hyperedge(address, response))
        }
    }
}

fn resolve_address(input: &str, aliases: &[AliasSnapshot]) -> Result<Vec<u8>, ServiceError> {
    if let Some(alias) = aliases.iter().find(|alias| alias.name == input.trim()) {
        return decode_address(&alias.address_hex);
    }
    decode_address(input)
}

fn decode_address(input: &str) -> Result<Vec<u8>, ServiceError> {
    let input = input.trim().strip_prefix("0x").unwrap_or(input.trim());
    let address = hex::decode(input).map_err(|_| {
        ServiceError::InvalidInput("Enter a 64-byte hexadecimal address or local alias".into())
    })?;
    if address.len() != 64 {
        return Err(ServiceError::InvalidInput(format!(
            "Hypergraph address must be 64 bytes; got {} bytes",
            address.len()
        )));
    }
    Ok(address)
}

fn map_vertex(address: Vec<u8>, response: GetVertexDataResponse) -> HypergraphSnapshot {
    HypergraphSnapshot {
        kind: HypergraphKind::Vertex,
        address_hex: hex::encode(address),
        set_type: response.set_type,
        phase_type: response.phase_type,
        shard_l1_hex: hex::encode(response.shard_l1),
        shard_l2_hex: hex::encode(response.shard_l2),
        present: response.present,
        entries: map_entries(response.entries),
    }
}

fn map_hyperedge(address: Vec<u8>, response: GetHyperedgeDataResponse) -> HypergraphSnapshot {
    HypergraphSnapshot {
        kind: HypergraphKind::Hyperedge,
        address_hex: hex::encode(address),
        set_type: response.set_type,
        phase_type: response.phase_type,
        shard_l1_hex: hex::encode(response.shard_l1),
        shard_l2_hex: hex::encode(response.shard_l2),
        present: None,
        entries: map_entries(response.entries),
    }
}

fn map_entries(entries: Vec<VertexDataEntry>) -> Vec<HypergraphEntrySnapshot> {
    entries
        .into_iter()
        .map(|entry| HypergraphEntrySnapshot {
            key_hex: hex::encode(entry.key),
            value_hex: hex::encode(entry.value),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use quil_types::proto::node::{GetVertexDataResponse, VertexDataEntry};

    use super::*;
    use crate::domain::{AliasSnapshot, HypergraphKind};

    #[test]
    fn resolves_full_address_from_hex_or_alias() {
        let address = vec![0xab; 64];
        let aliases = vec![AliasSnapshot {
            name: "my-vertex".into(),
            address_hex: hex::encode(&address),
            kind: "vertex".into(),
        }];

        assert_eq!(
            resolve_address(&hex::encode(&address), &aliases).unwrap(),
            address
        );
        assert_eq!(resolve_address("my-vertex", &aliases).unwrap(), address);
        assert!(resolve_address("abcd", &aliases).is_err());
    }

    #[test]
    fn maps_vertex_response_without_guessing_presence() {
        let snapshot = map_vertex(
            vec![0xcd; 64],
            GetVertexDataResponse {
                entries: vec![VertexDataEntry {
                    key: vec![1],
                    value: vec![2, 3],
                }],
                set_type: "vertex".into(),
                phase_type: "adds".into(),
                shard_l1: vec![4, 5, 6],
                shard_l2: vec![7; 32],
                present: Some(false),
                ..Default::default()
            },
        );

        assert_eq!(snapshot.kind, HypergraphKind::Vertex);
        assert_eq!(snapshot.present, Some(false));
        assert_eq!(snapshot.entries[0].key_hex, "01");
        assert_eq!(snapshot.entries[0].value_hex, "0203");
    }
}
