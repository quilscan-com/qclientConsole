use quil_keys::FileKeyManager;
use quil_types::{
    crypto::Signer,
    proto::{
        global::{
            message_request::Request, MessageBundle, MessageRequest, ProverConfirm, ProverLeave,
            ProverPause, ProverReject, ProverResume,
        },
        keys::Bls48581AddressedSignature,
    },
};
use zeroize::Zeroizing;

use crate::domain::ProverAction;

pub fn node_auth_domain(domain: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(b"NODE_AUTHENTICATION".len() + domain.len());
    result.extend_from_slice(b"NODE_AUTHENTICATION");
    result.extend_from_slice(domain);
    result
}

pub(super) fn falcon_signer(
    manager: &FileKeyManager,
    key_id: &str,
    expected_public: &[u8],
) -> anyhow::Result<std::sync::Arc<quil_crypto::FalconSigner>> {
    let secret = Zeroizing::new(manager.get_secret_key_bytes_by_id(key_id)?);
    let public = manager.get_public_key_bytes_by_id(key_id)?;
    anyhow::ensure!(
        secret.len() == quil_crypto::FALCON_SIGNING_KEY_LEN
            && public.len() == quil_crypto::FALCON_PUBLIC_KEY_LEN,
        "selected reward key is not a Falcon-512 keypair"
    );
    anyhow::ensure!(
        public == expected_public,
        "selected reward public key changed after preflight"
    );
    anyhow::ensure!(
        quil_crypto::falcon_public_from_signing_key(&secret).as_deref() == Some(public.as_slice()),
        "selected Falcon reward keypair does not match"
    );
    Ok(std::sync::Arc::new(quil_crypto::FalconSigner::from_bytes(
        &secret, &public,
    )))
}

pub fn sign_bundle(
    manager: &FileKeyManager,
    domain: &[u8],
    bundle: &MessageBundle,
) -> anyhow::Result<Vec<u8>> {
    let payload = quil_execution::message_envelope::proto_message_bundle_to_canonical_bytes(bundle)
        .map_err(|error| anyhow::anyhow!("Cannot encode message: {error}"))?;
    let signer: Box<dyn Signer> = manager
        .get_signer_by_id("q-peer-key")
        .map_err(|error| anyhow::anyhow!("Cannot read q-peer-key: {error}"))?;
    signer
        .sign_with_domain(&payload, &node_auth_domain(domain))
        .map_err(|error| anyhow::anyhow!("Cannot sign authenticated message: {error}"))
}

pub fn signed_request(
    manager: &FileKeyManager,
    action: &ProverAction,
    frame: u64,
) -> anyhow::Result<MessageRequest> {
    use quil_execution::global_intrinsic::prover_verify::{
        multi_filter_signing_message, prover_confirm_domain, prover_leave_domain,
        prover_pause_domain, prover_reject_domain, prover_resume_domain,
        single_filter_signing_message,
    };

    let request = match action {
        ProverAction::Join { .. } => anyhow::bail!("Joining a shard uses the RequestJoin RPC"),
        ProverAction::Leave { filters } => Request::Leave(ProverLeave {
            filters: filters.clone(),
            frame_number: frame,
            public_key_signature_bls48581: Some(addressed_signature(
                manager,
                &multi_filter_signing_message(filters, frame),
                &prover_leave_domain().map_err(|error| anyhow::anyhow!(error.to_string()))?,
            )?),
        }),
        ProverAction::Confirm { filters } => {
            #[allow(deprecated)]
            let confirm = ProverConfirm {
                filter: Vec::new(),
                frame_number: frame,
                public_key_signature_bls48581: Some(addressed_signature(
                    manager,
                    &multi_filter_signing_message(filters, frame),
                    &prover_confirm_domain().map_err(|error| anyhow::anyhow!(error.to_string()))?,
                )?),
                filters: filters.clone(),
                leaf_roots: Vec::new(),
                release: None,
            };
            Request::Confirm(confirm)
        }
        ProverAction::Reject { filters } => {
            #[allow(deprecated)]
            let reject = ProverReject {
                filter: Vec::new(),
                frame_number: frame,
                public_key_signature_bls48581: Some(addressed_signature(
                    manager,
                    &multi_filter_signing_message(filters, frame),
                    &prover_reject_domain().map_err(|error| anyhow::anyhow!(error.to_string()))?,
                )?),
                filters: filters.clone(),
            };
            Request::Reject(reject)
        }
        ProverAction::Pause { filter } => Request::Pause(ProverPause {
            filter: filter.clone(),
            frame_number: frame,
            public_key_signature_bls48581: Some(addressed_signature(
                manager,
                &single_filter_signing_message(filter, frame),
                &prover_pause_domain().map_err(|error| anyhow::anyhow!(error.to_string()))?,
            )?),
        }),
        ProverAction::Resume { filter } => Request::Resume(ProverResume {
            filter: filter.clone(),
            frame_number: frame,
            public_key_signature_bls48581: Some(addressed_signature(
                manager,
                &single_filter_signing_message(filter, frame),
                &prover_resume_domain().map_err(|error| anyhow::anyhow!(error.to_string()))?,
            )?),
        }),
    };
    Ok(MessageRequest {
        request: Some(request),
        timestamp: 0,
    })
}

fn addressed_signature(
    manager: &FileKeyManager,
    message: &[u8],
    domain: &[u8],
) -> anyhow::Result<Bls48581AddressedSignature> {
    let signer: Box<dyn Signer> = manager
        .get_signer_by_id("q-prover-key")
        .map_err(|error| anyhow::anyhow!("Cannot read q-prover-key: {error}"))?;
    let public_key = signer.public_key();
    let address = quil_crypto::poseidon::hash_bytes_to_32(public_key)
        .map_err(|error| anyhow::anyhow!("Cannot derive Prover address: {error}"))?
        .to_vec();
    let signature = signer
        .sign_with_domain(message, domain)
        .map_err(|error| anyhow::anyhow!("Cannot sign request: {error}"))?;
    Ok(Bls48581AddressedSignature { signature, address })
}

#[cfg(test)]
mod tests {
    use quil_crypto::FalconKeyConstructor;
    use quil_keys::FileKeyManager;
    use quil_types::{
        crypto::{KeyType, Signer},
        proto::global::{message_request::Request, MessageBundle},
    };

    use crate::domain::ProverAction;

    use super::*;

    #[test]
    fn outer_auth_domain_prefixes_target_domain() {
        let target = vec![0xff; 32];

        let domain = node_auth_domain(&target);

        assert_eq!(&domain[..19], b"NODE_AUTHENTICATION");
        assert_eq!(&domain[19..], target);
    }

    #[test]
    fn leave_request_signature_verifies_with_node_primitives() {
        let dir = tempfile::tempdir().unwrap();
        let manager = FileKeyManager::new(
            dir.path().join("keys.yml"),
            "",
            "q-prover-key".into(),
            Box::new(FalconKeyConstructor),
        )
        .unwrap();
        manager.create_falcon_key("q-prover-key").unwrap();
        let filters = vec![vec![0xaa; 32]];
        let action = ProverAction::Leave {
            filters: filters.clone(),
        };

        let request = signed_request(&manager, &action, 123).unwrap();
        let Request::Leave(leave) = request.request.unwrap() else {
            panic!("expected leave request");
        };
        let signature = leave.public_key_signature_bls48581.unwrap();
        let signer: Box<dyn Signer> = manager.get_signer_by_id("q-prover-key").unwrap();
        let message = quil_execution::global_intrinsic::prover_verify::multi_filter_signing_message(
            &filters, 123,
        );
        let domain =
            quil_execution::global_intrinsic::prover_verify::prover_leave_domain().unwrap();

        assert!(manager
            .validate_signature(
                KeyType::Falcon512,
                signer.public_key(),
                &message,
                &signature.signature,
                &domain,
            )
            .unwrap());
    }

    #[test]
    fn outer_bundle_signature_verifies_with_peer_key() {
        let dir = tempfile::tempdir().unwrap();
        let manager = FileKeyManager::new(
            dir.path().join("keys.yml"),
            "",
            "q-prover-key".into(),
            Box::new(FalconKeyConstructor),
        )
        .unwrap();
        let seed = [55_u8; 57];
        let public = quil_crypto::Ed448Signer::derive_public(&seed).unwrap();
        let mut peer_key = seed.to_vec();
        peer_key.extend_from_slice(&public);
        manager.set_peer_priv_key_hex(&hex::encode(peer_key));
        let bundle = MessageBundle {
            requests: Vec::new(),
            timestamp: 456,
        };
        let target = vec![0xff; 32];

        let signature = sign_bundle(&manager, &target, &bundle).unwrap();
        let payload =
            quil_execution::message_envelope::proto_message_bundle_to_canonical_bytes(&bundle)
                .unwrap();

        assert!(manager
            .validate_signature(
                KeyType::Ed448,
                &public,
                &payload,
                &signature,
                &node_auth_domain(&target),
            )
            .unwrap());
    }

    #[test]
    fn reward_mint_signer_uses_the_selected_falcon_keypair() {
        let dir = tempfile::tempdir().unwrap();
        let manager = FileKeyManager::new(
            dir.path().join("keys.yml"),
            "",
            "q-prover-key".into(),
            Box::new(FalconKeyConstructor),
        )
        .unwrap();
        manager.create_falcon_key("reward-key").unwrap();
        let expected = manager.get_public_key_bytes_by_id("reward-key").unwrap();

        let signer = falcon_signer(&manager, "reward-key", &expected).unwrap();

        assert_eq!(signer.key_type(), KeyType::Falcon512);
        assert_eq!(signer.public_key(), expected);
        assert!(falcon_signer(&manager, "reward-key", &[9; 897]).is_err());
    }
}
