use std::path::Path;

use quil_types::proto::node::GetProverRewardWitnessRequest;

use super::{node, prover, read_rpc, wallet, ServiceError};
use crate::domain::{RewardIdentity, RewardSnapshot};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ResolvedRewardIdentity {
    Falcon { key_id: String, public_key: Vec<u8> },
    Unavailable,
    Ambiguous { candidates: usize },
}

pub(super) async fn read(config_dir: &Path) -> Result<RewardSnapshot, ServiceError> {
    let config = quil_config::load_config(config_dir)
        .map_err(|error| ServiceError::Config(error.to_string()))?;
    let manager = prover::build_key_manager(&config, config_dir)?;
    let legacy_account = legacy_account_address(&config.p2p.peer_priv_key);
    let recipient_address = Some(wallet::recipient_address(config_dir)?);
    let resolved = resolve_prover_identity(&manager.list_keys())?;
    let ResolvedRewardIdentity::Falcon { key_id, public_key } = resolved else {
        return Ok(RewardSnapshot {
            identity: match resolved {
                ResolvedRewardIdentity::Unavailable => RewardIdentity::Unavailable,
                ResolvedRewardIdentity::Ambiguous { candidates } => {
                    RewardIdentity::Ambiguous { candidates }
                }
                ResolvedRewardIdentity::Falcon { .. } => unreachable!(),
            },
            prover_address_hex: None,
            public_key_hex: None,
            legacy_account_address_hex: legacy_account,
            recipient_address_hex: recipient_address,
            claimable_base_units: None,
            cited_frame: None,
            proof_verified: false,
            witness_error: None,
        });
    };
    let public: [u8; 897] = public_key.try_into().map_err(|_| {
        ServiceError::Key(format!("Key {key_id} is not a valid Falcon-512 public key"))
    })?;
    let owner = quil_crypto::poseidon::hash_bytes_to_32(&public)
        .map_err(|error| ServiceError::Rewards(error.to_string()))?;
    let identity = if key_id == "q-prover-key" {
        RewardIdentity::Standard
    } else {
        RewardIdentity::Alternate { key_id }
    };
    let snapshot = RewardSnapshot {
        identity,
        prover_address_hex: Some(hex::encode(owner)),
        public_key_hex: Some(hex::encode(public)),
        legacy_account_address_hex: legacy_account,
        recipient_address_hex: recipient_address,
        claimable_base_units: None,
        cited_frame: None,
        proof_verified: false,
        witness_error: None,
    };
    let read_rpc = match read_rpc::load(config_dir, config.effective_listen_grpc_multiaddr()) {
        Ok(value) => value,
        Err(error) => return Ok(mark_witness_unavailable(snapshot, error.to_string())),
    };
    let address = match node::grpc_address(&read_rpc) {
        Ok(value) => value,
        Err(error) => return Ok(mark_witness_unavailable(snapshot, error.to_string())),
    };
    let client = match node::connect_client(&address).await {
        Ok(value) => value,
        Err(error) => return Ok(mark_witness_unavailable(snapshot, error.to_string())),
    };
    let response = match tokio::time::timeout(
        node::RPC_TIMEOUT,
        client
            .max_decoding_message_size(64 * 1024)
            .get_prover_reward_witness(GetProverRewardWitnessRequest {
                domain: quil_execution::domains::QUIL_TOKEN.to_vec(),
                owner_prover_address: owner.to_vec(),
            }),
    )
    .await
    {
        Ok(Ok(response)) => response.into_inner(),
        Ok(Err(error)) => {
            return Ok(mark_witness_unavailable(
                snapshot,
                grpc_witness_error(&error),
            ))
        }
        Err(_) => {
            return Ok(mark_witness_unavailable(
                snapshot,
                "Reward witness query timed out",
            ))
        }
    };
    decode_response(snapshot, owner, response)
}

fn mark_witness_unavailable(
    mut snapshot: RewardSnapshot,
    reason: impl Into<String>,
) -> RewardSnapshot {
    snapshot.witness_error = Some(reason.into());
    snapshot
}

fn grpc_witness_error(error: &tonic::Status) -> String {
    let message = error.message().trim();
    if message.is_empty() {
        format!("Reward witness RPC failed: {:?}", error.code())
    } else {
        message.to_owned()
    }
}

pub(super) fn resolve_prover_identity(
    keys: &[quil_keys::KeyEntry],
) -> Result<ResolvedRewardIdentity, ServiceError> {
    if let Some(key) = keys
        .iter()
        .find(|key| key.id == "q-prover-key" && key.key_type == 8)
    {
        validate_falcon_public(key)?;
        return Ok(ResolvedRewardIdentity::Falcon {
            key_id: key.id.clone(),
            public_key: key.public_key.clone(),
        });
    }

    let candidates = keys
        .iter()
        .filter(|key| key.key_type == 8)
        .collect::<Vec<_>>();
    match candidates.as_slice() {
        [] => Ok(ResolvedRewardIdentity::Unavailable),
        [key] => {
            validate_falcon_public(key)?;
            Ok(ResolvedRewardIdentity::Falcon {
                key_id: key.id.clone(),
                public_key: key.public_key.clone(),
            })
        }
        keys => Ok(ResolvedRewardIdentity::Ambiguous {
            candidates: keys.len(),
        }),
    }
}

fn validate_falcon_public(key: &quil_keys::KeyEntry) -> Result<(), ServiceError> {
    if key.public_key.len() == 897 {
        Ok(())
    } else {
        Err(ServiceError::Key(format!(
            "Falcon-512 public key {} has the wrong length: expected 897 bytes, got {}",
            key.id,
            key.public_key.len()
        )))
    }
}

fn legacy_account_address(peer_private_key_hex: &str) -> Option<String> {
    let identity =
        quil_p2p::ed448_identity::Ed448Identity::from_config_hex(peer_private_key_hex).ok()?;
    quil_crypto::poseidon::hash_bytes_to_32(&identity.peer_id_bytes)
        .ok()
        .map(hex::encode)
}

fn decode_response(
    mut snapshot: RewardSnapshot,
    owner: [u8; 32],
    response: quil_types::proto::node::GetProverRewardWitnessResponse,
) -> Result<RewardSnapshot, ServiceError> {
    if !response.found {
        return Ok(snapshot);
    }

    let root: [u8; 32] = response
        .reward_root
        .as_slice()
        .try_into()
        .map_err(|_| ServiceError::Rewards("Reward proof has no valid 32-byte root".into()))?;
    let value =
        u128::from_le_bytes(response.value.as_slice().try_into().map_err(|_| {
            ServiceError::Rewards("Reward amount is not a valid 16-byte value".into())
        })?);
    quil_execution::token_intrinsic::reward_witness::verify_reward_membership(
        &owner,
        value,
        &root,
        &response.forest_proof,
    )
    .map_err(|error| ServiceError::Rewards(format!("Reward proof validation failed: {error}")))?;

    snapshot.claimable_base_units = Some(value);
    snapshot.cited_frame = Some(response.cited_frame);
    snapshot.proof_verified = true;
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(id: &str, key_type: u8, public_len: usize) -> quil_keys::KeyEntry {
        quil_keys::KeyEntry {
            id: id.into(),
            key_type,
            public_key: vec![7; public_len],
        }
    }

    #[test]
    fn missing_falcon_identity_is_not_applicable() {
        let resolved = resolve_prover_identity(&[]).unwrap();

        assert_eq!(resolved, ResolvedRewardIdentity::Unavailable);
    }

    #[test]
    fn unique_falcon_identity_is_used_when_standard_id_is_absent() {
        let resolved =
            resolve_prover_identity(&[key("legacy-prover", 2, 74), key("pool-falcon", 8, 897)])
                .unwrap();

        assert_eq!(
            resolved,
            ResolvedRewardIdentity::Falcon {
                key_id: "pool-falcon".into(),
                public_key: vec![7; 897],
            }
        );
    }

    #[test]
    fn multiple_falcon_identities_are_ambiguous() {
        let resolved =
            resolve_prover_identity(&[key("falcon-a", 8, 897), key("falcon-b", 8, 897)]).unwrap();

        assert_eq!(
            resolved,
            ResolvedRewardIdentity::Ambiguous { candidates: 2 }
        );
    }

    #[test]
    fn absent_witness_is_unavailable_not_a_fake_zero_balance() {
        let snapshot = RewardSnapshot {
            identity: RewardIdentity::Standard,
            prover_address_hex: Some(hex::encode([9; 32])),
            public_key_hex: Some(hex::encode([7; 897])),
            legacy_account_address_hex: None,
            recipient_address_hex: None,
            claimable_base_units: None,
            cited_frame: None,
            proof_verified: false,
            witness_error: None,
        };
        let snapshot = decode_response(
            snapshot,
            [9; 32],
            quil_types::proto::node::GetProverRewardWitnessResponse::default(),
        )
        .unwrap();

        assert_eq!(snapshot.prover_address_hex, Some(hex::encode([9; 32])));
        assert_eq!(snapshot.claimable_base_units, None);
        assert_eq!(snapshot.cited_frame, None);
        assert!(!snapshot.proof_verified);
    }

    #[test]
    fn unavailable_witness_keeps_local_identity_and_wallet_visible() {
        let snapshot = RewardSnapshot {
            identity: RewardIdentity::Standard,
            prover_address_hex: Some(hex::encode([9; 32])),
            public_key_hex: Some(hex::encode([7; 897])),
            legacy_account_address_hex: None,
            recipient_address_hex: Some(hex::encode([8; 32])),
            claimable_base_units: None,
            cited_frame: None,
            proof_verified: false,
            witness_error: None,
        };

        let snapshot =
            mark_witness_unavailable(snapshot, "historical global reward root unavailable");

        assert_eq!(snapshot.prover_address_hex, Some(hex::encode([9; 32])));
        assert_eq!(snapshot.recipient_address_hex, Some(hex::encode([8; 32])));
        assert_eq!(
            snapshot.witness_error.as_deref(),
            Some("historical global reward root unavailable")
        );
        assert_eq!(snapshot.claimable_base_units, None);
    }

    #[test]
    fn empty_grpc_message_still_produces_a_visible_witness_error() {
        let error = tonic::Status::new(tonic::Code::Unavailable, "");

        let message = grpc_witness_error(&error);

        assert!(!message.trim().is_empty());
        assert!(message.to_ascii_lowercase().contains("unavailable"));
    }

    #[test]
    fn malformed_found_witness_is_rejected() {
        let snapshot = RewardSnapshot {
            identity: RewardIdentity::Standard,
            prover_address_hex: Some(hex::encode([9; 32])),
            public_key_hex: Some(hex::encode([7; 897])),
            legacy_account_address_hex: None,
            recipient_address_hex: None,
            claimable_base_units: None,
            cited_frame: None,
            proof_verified: false,
            witness_error: None,
        };
        let result = decode_response(
            snapshot,
            [9; 32],
            quil_types::proto::node::GetProverRewardWitnessResponse {
                found: true,
                reward_root: vec![1; 31],
                value: 1_u128.to_le_bytes().to_vec(),
                ..Default::default()
            },
        );

        assert!(result.is_err());
    }

    #[test]
    fn legacy_account_exists_only_for_a_valid_ed448_identity() {
        let identity = quil_p2p::ed448_identity::Ed448Identity::generate().unwrap();
        let expected = quil_crypto::poseidon::hash_bytes_to_32(&identity.peer_id_bytes).unwrap();

        assert_eq!(
            legacy_account_address(&identity.to_config_hex()),
            Some(hex::encode(expected))
        );
        assert_eq!(legacy_account_address(""), None);
        assert_eq!(legacy_account_address("not-hex"), None);
    }
}
