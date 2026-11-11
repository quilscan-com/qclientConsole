use std::{collections::BTreeSet, future::Future, time::Duration};

use quil_lattice_ct::confidential::{
    coin_tree::{AuthPath, RootRecord},
    relation::membership::{MembershipKey, Node},
    transfer::Output,
};
use quil_types::proto::node::{GetCoinWitnessesRequest, GetCoinWitnessesResponse};

use super::{ServiceError, Wallet};

const WITNESS_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MAX_WITNESS_RESPONSE_BYTES: usize = (1 << 20) - 1;

impl Wallet {
    pub(super) async fn witnesses(
        &self,
        client: &quil_types::proto::node::node_service_client::NodeServiceClient<
            tonic::transport::Channel,
        >,
        coins: &[([u8; 32], &Output)],
    ) -> Result<(RootRecord, Vec<AuthPath>), ServiceError> {
        if coins.is_empty() || coins.len() > 4 {
            return Err(ServiceError::Wallet(
                "Select one to four Coins per witness request".into(),
            ));
        }
        if coins
            .iter()
            .map(|(address, _)| address)
            .collect::<BTreeSet<_>>()
            .len()
            != coins.len()
        {
            return Err(ServiceError::Wallet("Duplicate selected Coins".into()));
        }

        let client = client
            .clone()
            .max_decoding_message_size(MAX_WITNESS_RESPONSE_BYTES);
        let request = GetCoinWitnessesRequest {
            domain: self.application.to_vec(),
            addresses: coins.iter().map(|(address, _)| address.to_vec()).collect(),
        };
        let response = retry_witness_request(WITNESS_TIMEOUT, || {
            let mut client = client.clone();
            let request = request.clone();
            async move {
                client
                    .get_coin_witnesses(request)
                    .await
                    .map(tonic::Response::into_inner)
            }
        })
        .await
        .map_err(|error| ServiceError::Rpc(error.to_string()))?;

        decode_witnesses(&self.network, &self.context, coins, response)
    }
}

async fn retry_witness_request<T, F, Fut>(
    timeout: Duration,
    mut request: F,
) -> Result<T, tonic::Status>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, tonic::Status>>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    let mut delay = Duration::from_millis(250);
    loop {
        let result = tokio::time::timeout_at(deadline, request())
            .await
            .map_err(|_| {
                tonic::Status::deadline_exceeded(
                    "Witness preparation deadline exceeded; retry after synchronization",
                )
            })?;
        match result {
            Ok(value) => return Ok(value),
            Err(error)
                if matches!(
                    error.code(),
                    tonic::Code::Unavailable | tonic::Code::ResourceExhausted
                ) =>
            {
                tokio::time::timeout_at(deadline, tokio::time::sleep(delay))
                    .await
                    .map_err(|_| {
                        tonic::Status::deadline_exceeded(
                            "Witness preparation deadline exceeded; retry after synchronization",
                        )
                    })?;
                delay = delay.saturating_mul(2).min(Duration::from_secs(5));
            }
            Err(error) => return Err(error),
        }
    }
}

fn decode_witnesses(
    network: &[u8; 32],
    context: &[u8; 32],
    coins: &[([u8; 32], &Output)],
    response: GetCoinWitnessesResponse,
) -> Result<(RootRecord, Vec<AuthPath>), ServiceError> {
    if response.network.as_slice() != network {
        return Err(ServiceError::Wallet("Witness network mismatch".into()));
    }
    let root = RootRecord::decode(&response.root_record, context)
        .map_err(|error| ServiceError::Wallet(format!("Invalid witness root: {error:?}")))?;
    if response.witnesses.len() != coins.len() {
        return Err(ServiceError::Wallet("Witness count mismatch".into()));
    }

    let key = MembershipKey::derive(context);
    let mut paths = Vec::with_capacity(coins.len());
    for ((address, output), witness) in coins.iter().zip(response.witnesses) {
        if witness.address.as_slice() != address || !witness.found {
            return Err(ServiceError::Wallet(
                "Selected Coin is missing or mismatched".into(),
            ));
        }
        if witness.siblings.len() != usize::from(root.depth)
            || witness.right.len() != usize::from(root.depth)
        {
            return Err(ServiceError::Wallet("Witness depth mismatch".into()));
        }
        let siblings = witness
            .siblings
            .iter()
            .map(|bytes| {
                Node::from_bytes(bytes).map_err(|error| {
                    ServiceError::Wallet(format!("Invalid witness node: {error:?}"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let owner = Node::from_identity_bytes(&output.owner)
            .map_err(|error| ServiceError::Wallet(format!("Invalid Coin owner: {error:?}")))?;
        let mut node = key.leaf(&owner, &output.commitment);
        for (sibling, right) in siblings.iter().zip(&witness.right) {
            node = if *right {
                key.parent(sibling, &node)
            } else {
                key.parent(&node, sibling)
            };
        }
        if node != root.root {
            return Err(ServiceError::Wallet(
                "Witness does not authenticate the selected Coin".into(),
            ));
        }
        paths.push(AuthPath {
            siblings,
            right: witness.right,
        });
    }
    Ok((root, paths))
}

#[cfg(test)]
mod tests {
    use quil_lattice_ct::confidential::{
        coin_tree::{CoinRecord, CoinTree},
        relation::membership::{Node, IDENTITY_BYTES},
        transfer::{parameter_context, Output, MEMO_BYTES},
        AmountCommitment, COMMITMENT_BYTES,
    };
    use quil_types::proto::node::{CoinWitness, GetCoinWitnessesResponse};

    use super::*;

    fn fixture() -> (
        [u8; 32],
        [u8; 32],
        [u8; 32],
        Output,
        GetCoinWitnessesResponse,
    ) {
        let network = [1; 32];
        let application = [2; 32];
        let context = parameter_context(&network, &application);
        let address = [3; 32];
        let output = Output {
            commitment: AmountCommitment::from_bytes(&[0; COMMITMENT_BYTES]).unwrap(),
            owner: [4; IDENTITY_BYTES],
            memo: [5; MEMO_BYTES],
        };
        let tree = CoinTree::build(
            &context,
            &[CoinRecord {
                address,
                owner: output.owner,
                commitment: output.commitment.clone(),
                position: 0,
            }],
            1,
            4,
        )
        .unwrap();
        let root = tree.root_at_depth(1).unwrap();
        let path = tree.auth_path(&address, 1).unwrap();
        let response = GetCoinWitnessesResponse {
            network: network.to_vec(),
            root_record: root.encode().unwrap().to_vec(),
            witnesses: vec![CoinWitness {
                address: address.to_vec(),
                found: true,
                siblings: path
                    .siblings
                    .iter()
                    .map(|node| node.to_bytes().to_vec())
                    .collect(),
                right: path.right,
            }],
        };
        (network, application, address, output, response)
    }

    #[test]
    fn witness_response_is_bound_to_network_root_and_selected_output() {
        let (network, application, address, output, response) = fixture();
        let context = parameter_context(&network, &application);
        assert!(
            decode_witnesses(&network, &context, &[(address, &output)], response.clone()).is_ok()
        );

        let mut bad_network = response.clone();
        bad_network.network[0] ^= 1;
        assert!(decode_witnesses(&network, &context, &[(address, &output)], bad_network).is_err());

        let mut bad_path = response.clone();
        bad_path.witnesses[0].siblings[0] = Node::from_identity_bytes(&[7; IDENTITY_BYTES])
            .unwrap()
            .to_bytes()
            .to_vec();
        assert!(decode_witnesses(&network, &context, &[(address, &output)], bad_path).is_err());

        let mut other = output.clone();
        other.owner[0] ^= 1;
        assert!(decode_witnesses(&network, &context, &[(address, &other)], response).is_err());
    }
}
