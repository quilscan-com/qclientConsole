use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use quil_config::Config;
use quil_keys::FileKeyManager;
use quil_types::proto::{
    global::MessageBundle,
    node::{GetNodeInfoRequest, RequestJoinRequest, SendRequest},
};

use crate::domain::ProverAction;

use super::{node, signing, ServiceError};

const JOIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);
const MIN_FILTER_BYTES: usize = 32;
const MAX_FILTER_BYTES: usize = 66;

fn operation_timeout(action: &ProverAction) -> std::time::Duration {
    if matches!(action, ProverAction::Join { .. }) {
        JOIN_TIMEOUT
    } else {
        node::RPC_TIMEOUT
    }
}

fn join_timeout_result() -> crate::domain::OperationResult {
    crate::domain::OperationResult {
        message: "Response timed out; the result is unknown. The node may still be working. Refresh status and do not submit again".into(),
        outcome: crate::domain::OperationOutcome::Unknown,
    }
}

fn join_request(action: &ProverAction) -> Result<RequestJoinRequest, ServiceError> {
    let ProverAction::Join {
        filters,
        delegate,
        worker_ids,
    } = action
    else {
        return Err(ServiceError::InvalidInput(
            "Action is not a shard join".into(),
        ));
    };
    if filters.is_empty()
        || filters
            .iter()
            .any(|filter| !(MIN_FILTER_BYTES..=MAX_FILTER_BYTES).contains(&filter.len()))
    {
        return Err(ServiceError::InvalidInput(
            "Filter must be between 32 and 66 bytes".into(),
        ));
    }
    if !delegate.is_empty() && delegate.len() != 32 {
        return Err(ServiceError::InvalidInput(
            "Delegate must be empty or exactly 32 bytes".into(),
        ));
    }
    if !worker_ids.is_empty() && worker_ids.len() != filters.len() {
        return Err(ServiceError::InvalidInput(
            "Each Filter must have a matching Worker".into(),
        ));
    }
    Ok(RequestJoinRequest {
        filters: filters.clone(),
        delegate: delegate.clone(),
        worker_ids: worker_ids.clone(),
    })
}

pub(super) async fn execute(
    config_dir: &Path,
    action: ProverAction,
) -> Result<crate::domain::OperationResult, ServiceError> {
    validate_filters(action.filters())?;
    let config = quil_config::load_config(config_dir)
        .map_err(|error| ServiceError::Config(error.to_string()))?;
    let address = node::grpc_address(config.effective_listen_grpc_multiaddr())?;
    let mut client = node::connect_client(&address).await?;

    if matches!(action, ProverAction::Join { .. }) {
        let request = join_request(&action)?;
        let response = tokio::time::timeout(
            operation_timeout(&action),
            client.request_join(tonic::Request::new(request)),
        )
        .await;
        match response {
            Err(_) => return Ok(join_timeout_result()),
            Ok(Err(error)) => return Err(ServiceError::Operation(error.to_string())),
            Ok(Ok(_)) => {}
        }
        return Ok(crate::domain::OperationResult {
            message: "Shard join request submitted; the node is computing the proof".into(),
            outcome: crate::domain::OperationOutcome::Submitted,
        });
    }

    let info = tokio::time::timeout(
        node::RPC_TIMEOUT,
        client.get_node_info(tonic::Request::new(GetNodeInfoRequest::default())),
    )
    .await
    .map_err(|_| ServiceError::Timeout)?
    .map_err(|error| ServiceError::Rpc(error.to_string()))?
    .into_inner();
    let manager = build_key_manager(&config, config_dir)?;
    let request = signing::signed_request(&manager, &action, info.last_global_head_frame)
        .map_err(|error| ServiceError::Operation(error.to_string()))?;
    let bundle = MessageBundle {
        requests: vec![request],
        timestamp: now_millis(),
    };
    let domain = vec![0xff; 32];
    let authentication = signing::sign_bundle(&manager, &domain, &bundle)
        .map_err(|error| ServiceError::Operation(error.to_string()))?;
    tokio::time::timeout(
        operation_timeout(&action),
        client.send(tonic::Request::new(SendRequest {
            domain,
            request: Some(bundle),
            authentication,
            delivery_data: Vec::new(),
        })),
    )
    .await
    .map_err(|_| ServiceError::Timeout)?
    .map_err(|error| ServiceError::Operation(error.to_string()))?;

    Ok(crate::domain::OperationResult {
        message: format!(
            "{} request submitted; waiting for on-chain state update",
            action.localized_label(crate::i18n::Locale::English)
        ),
        outcome: crate::domain::OperationOutcome::Submitted,
    })
}

fn validate_filters(filters: &[Vec<u8>]) -> Result<(), ServiceError> {
    if filters.is_empty()
        || filters
            .iter()
            .any(|filter| !(MIN_FILTER_BYTES..=MAX_FILTER_BYTES).contains(&filter.len()))
    {
        return Err(ServiceError::InvalidInput(
            "Filter must be between 32 and 66 bytes".into(),
        ));
    }
    Ok(())
}

pub(super) fn build_key_manager(
    config: &Config,
    config_dir: &Path,
) -> Result<FileKeyManager, ServiceError> {
    let keys_path = resolve_keys_path(config_dir, Path::new(&config.key.key_store_file.path));
    let proving_key_id = if config.engine.proving_key_id.is_empty() {
        "default-proving-key".to_string()
    } else {
        config.engine.proving_key_id.clone()
    };
    let manager = FileKeyManager::new(
        keys_path,
        &config.key.key_store_file.encryption_key,
        proving_key_id,
        Box::new(quil_crypto::FalconKeyConstructor),
    )
    .map_err(|error| ServiceError::Key(error.to_string()))?;
    manager.set_peer_priv_key_hex(&config.p2p.peer_priv_key);
    Ok(manager)
}

fn resolve_keys_path(config_dir: &Path, configured_path: &Path) -> PathBuf {
    if configured_path.as_os_str().is_empty() {
        return config_dir.join("keys.yml");
    }
    if configured_path.is_absolute() {
        return configured_path.to_path_buf();
    }

    let inside_config = config_dir.join(configured_path);
    if inside_config.exists() {
        return inside_config;
    }

    let beside_config = config_dir
        .parent()
        .map(|parent| parent.join(configured_path))
        .filter(|path| path.exists());
    if let Some(path) = beside_config {
        return path;
    }

    configured_path.to_path_buf()
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_node_relative_keys_path_from_an_explicit_config_directory() {
        let root = tempfile::tempdir().unwrap();
        let config_dir = root.path().join(".config");
        std::fs::create_dir_all(&config_dir).unwrap();
        let keys = config_dir.join("keys.yml");
        std::fs::write(&keys, "null:\n").unwrap();

        assert_eq!(
            resolve_keys_path(&config_dir, Path::new(".config/keys.yml")),
            keys
        );
    }

    #[test]
    fn join_keeps_selected_worker_and_filter_together() {
        let action = ProverAction::Join {
            filters: vec![vec![0xaa; 32]],
            delegate: vec![0xbb; 32],
            worker_ids: vec![7],
        };

        let request = join_request(&action).unwrap();

        assert_eq!(request.filters, vec![vec![0xaa; 32]]);
        assert_eq!(request.delegate, vec![0xbb; 32]);
        assert_eq!(request.worker_ids, vec![7]);
    }

    #[test]
    fn join_accepts_a_full_shard_filter_with_prefix_bytes() {
        let action = ProverAction::Join {
            filters: vec![vec![0xaa; 35]],
            delegate: Vec::new(),
            worker_ids: vec![7],
        };

        let request = join_request(&action).unwrap();

        assert_eq!(request.filters[0].len(), 35);
    }

    #[test]
    fn join_rejects_mismatched_worker_selection() {
        let action = ProverAction::Join {
            filters: vec![vec![0xaa; 32], vec![0xbb; 32]],
            delegate: Vec::new(),
            worker_ids: vec![7],
        };

        assert!(join_request(&action).is_err());
    }

    #[test]
    fn join_uses_a_longer_timeout_than_ordinary_prover_rpcs() {
        let join = ProverAction::Join {
            filters: vec![vec![0xaa; 32]],
            delegate: Vec::new(),
            worker_ids: vec![7],
        };
        let pause = ProverAction::Pause {
            filter: vec![0xaa; 32],
        };

        assert_eq!(operation_timeout(&join).as_secs(), 90);
        assert_eq!(operation_timeout(&pause), node::RPC_TIMEOUT);
    }

    #[test]
    fn join_timeout_reports_an_unknown_outcome_and_safe_next_step() {
        let result = join_timeout_result();

        assert_eq!(result.outcome, crate::domain::OperationOutcome::Unknown);
        for expected in [
            "Response timed out",
            "result is unknown",
            "node may still be working",
            "Refresh status",
            "do not submit again",
        ] {
            assert!(result.message.contains(expected), "{result:?}");
        }
    }
}
