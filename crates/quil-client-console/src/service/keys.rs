use std::path::Path;

use super::{prover, ServiceError};
use crate::domain::{
    protected_key_id, KeyMutation, KeySnapshot, OperationResult, CREATABLE_KEY_TYPES,
};

pub(super) async fn list(config_dir: &Path) -> Result<Vec<KeySnapshot>, ServiceError> {
    let config = quil_config::load_config(config_dir)
        .map_err(|error| ServiceError::Config(error.to_string()))?;
    let manager = prover::build_key_manager(&config, config_dir)?;
    Ok(manager
        .list_keys()
        .into_iter()
        .map(|key| KeySnapshot {
            id: key.id,
            key_type: key.key_type,
            public_key_hex: hex::encode(key.public_key),
        })
        .collect())
}

pub(super) async fn mutate(
    config_dir: &Path,
    mutation: KeyMutation,
) -> Result<OperationResult, ServiceError> {
    let config_dir = config_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let config = quil_config::load_config(&config_dir)
            .map_err(|error| ServiceError::Config(error.to_string()))?;
        let manager = prover::build_key_manager(&config, &config_dir)?;
        match mutation {
            KeyMutation::Create { id, key_type } => {
                let id = validate_id(&id)?;
                if !CREATABLE_KEY_TYPES
                    .iter()
                    .any(|(supported, _)| *supported == key_type)
                {
                    return Err(ServiceError::InvalidInput("Unsupported key type".into()));
                }
                if manager.list_keys().iter().any(|key| key.id == id) {
                    return Err(ServiceError::Key(format!("Key {id} already exists")));
                }
                manager
                    .create_signing_key(&id, key_type)
                    .map_err(|error| ServiceError::Key(error.to_string()))?;
                Ok(OperationResult {
                    message: format!("Key {id} created"),
                    outcome: crate::domain::OperationOutcome::Submitted,
                })
            }
            KeyMutation::Delete { id } => {
                if protected_key_id(&id) {
                    return Err(ServiceError::Key(
                        "Built-in q-* keys define node identity and cannot be deleted from the console".into(),
                    ));
                }
                let deleted = manager
                    .delete_key(&id)
                    .map_err(|error| ServiceError::Key(error.to_string()))?;
                if !deleted {
                    return Err(ServiceError::Key(format!("Key {id} does not exist")));
                }
                Ok(OperationResult {
                    message: format!("Key {id} deleted"),
                    outcome: crate::domain::OperationOutcome::Submitted,
                })
            }
        }
    })
    .await
    .map_err(|error| ServiceError::Key(error.to_string()))?
}

fn validate_id(id: &str) -> Result<String, ServiceError> {
    let id = id.trim();
    if id.is_empty() {
        return Err(ServiceError::InvalidInput(
            "Key name cannot be empty".into(),
        ));
    }
    if id.chars().any(char::is_whitespace) {
        return Err(ServiceError::InvalidInput(
            "Key name cannot contain whitespace".into(),
        ));
    }
    if protected_key_id(id) {
        return Err(ServiceError::InvalidInput(
            "q-* names are reserved for built-in node keys".into(),
        ));
    }
    Ok(id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_custom_key_ids() {
        assert_eq!(validate_id(" wallet ").unwrap(), "wallet");
        assert!(validate_id("").is_err());
        assert!(validate_id("two words").is_err());
        assert!(validate_id("q-custom-key").is_err());
    }

    #[test]
    fn protects_node_identity_keys() {
        assert!(protected_key_id("q-prover-key"));
        assert!(protected_key_id("q-onion-key"));
        assert!(!protected_key_id("wallet-signing"));
    }

    #[tokio::test]
    async fn creates_lists_and_deletes_custom_key_in_temporary_store() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("config.yml"), "{}").unwrap();

        mutate(
            directory.path(),
            KeyMutation::Create {
                id: "wallet-signing".into(),
                key_type: 0,
            },
        )
        .await
        .unwrap();
        let keys = list(directory.path()).await.unwrap();
        assert!(keys.iter().any(|key| key.id == "wallet-signing"));

        mutate(
            directory.path(),
            KeyMutation::Delete {
                id: "wallet-signing".into(),
            },
        )
        .await
        .unwrap();
        assert!(list(directory.path()).await.unwrap().is_empty());
    }
}
