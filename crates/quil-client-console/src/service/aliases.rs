use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::{de, ser::SerializeMap, Deserialize, Deserializer, Serialize, Serializer};

use super::ServiceError;
use crate::domain::{AliasMutation, AliasSnapshot, OperationResult};

#[derive(Debug, Clone, PartialEq, Eq)]
struct AliasEntry {
    address: Vec<u8>,
    kind: String,
}

impl Serialize for AliasEntry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(1 + usize::from(!self.kind.is_empty())))?;
        map.serialize_entry("address", &hex::encode(&self.address))?;
        if !self.kind.is_empty() {
            map.serialize_entry("type", &self.kind)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for AliasEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Address(String),
            Entry {
                address: String,
                #[serde(rename = "type", default)]
                kind: String,
            },
        }

        let (address, kind) = match Repr::deserialize(deserializer)? {
            Repr::Address(address) => (address, String::new()),
            Repr::Entry { address, kind } => (address, kind),
        };
        Ok(Self {
            address: decode_address(&address).map_err(de::Error::custom)?,
            kind,
        })
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct AliasFile {
    #[serde(default)]
    aliases: BTreeMap<String, AliasEntry>,
}

#[derive(Debug, Deserialize)]
struct NodeAliasConfig {
    alias: Option<AliasConfig>,
}

#[derive(Debug, Deserialize)]
struct AliasConfig {
    #[serde(rename = "aliasFile")]
    file: Option<AliasFileConfig>,
}

#[derive(Debug, Deserialize)]
struct AliasFileConfig {
    #[serde(default)]
    path: String,
    #[serde(rename = "createIfMissing", default)]
    create_if_missing: bool,
}

pub(super) async fn list(config_dir: &Path) -> Result<Vec<AliasSnapshot>, ServiceError> {
    let config_dir = config_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let (_, file) = load_file(&config_dir, false)?;
        Ok(file
            .aliases
            .into_iter()
            .map(|(name, entry)| AliasSnapshot {
                name,
                address_hex: hex::encode(entry.address),
                kind: entry.kind,
            })
            .collect())
    })
    .await
    .map_err(|error| ServiceError::Aliases(error.to_string()))?
}

pub(super) async fn mutate(
    config_dir: &Path,
    mutation: AliasMutation,
) -> Result<OperationResult, ServiceError> {
    let config_dir = config_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let (path, mut file) = load_file(&config_dir, true)?;
        let message = match mutation {
            AliasMutation::Put {
                name,
                address_hex,
                kind,
            } => {
                let name = validate_name(&name)?;
                let address = decode_address(&address_hex)?;
                file.aliases.insert(
                    name.clone(),
                    AliasEntry {
                        address,
                        kind: kind.trim().to_string(),
                    },
                );
                format!("Alias {name} saved")
            }
            AliasMutation::Delete { name } => {
                if file.aliases.remove(&name).is_none() {
                    return Err(ServiceError::Aliases(format!(
                        "Alias {name} does not exist"
                    )));
                }
                format!("Alias {name} deleted")
            }
        };
        save_file(&path, &file)?;
        Ok(OperationResult {
            message,
            outcome: crate::domain::OperationOutcome::Submitted,
        })
    })
    .await
    .map_err(|error| ServiceError::Aliases(error.to_string()))?
}

fn load_file(
    config_dir: &Path,
    create_missing: bool,
) -> Result<(PathBuf, AliasFile), ServiceError> {
    let (path, create_if_missing) = alias_file_config(config_dir);
    match std::fs::read_to_string(&path) {
        Ok(contents) => {
            let file = serde_yaml::from_str(&contents)
                .map_err(|error| ServiceError::Aliases(error.to_string()))?;
            Ok((path, file))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !create_missing => {
            Ok((path, AliasFile::default()))
        }
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                && create_missing
                && create_if_missing =>
        {
            let file = AliasFile::default();
            save_file(&path, &file)?;
            Ok((path, file))
        }
        Err(error) => Err(ServiceError::Aliases(error.to_string())),
    }
}

fn save_file(path: &Path, file: &AliasFile) -> Result<(), ServiceError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| ServiceError::Aliases(error.to_string()))?;
    }
    let contents =
        serde_yaml::to_string(file).map_err(|error| ServiceError::Aliases(error.to_string()))?;
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, contents)
        .map_err(|error| ServiceError::Aliases(error.to_string()))?;
    std::fs::rename(&temporary, path).map_err(|error| ServiceError::Aliases(error.to_string()))
}

fn alias_file_config(config_dir: &Path) -> (PathBuf, bool) {
    let parsed = std::fs::read_to_string(config_dir.join("config.yml"))
        .ok()
        .and_then(|contents| serde_yaml::from_str::<NodeAliasConfig>(&contents).ok())
        .and_then(|config| config.alias)
        .and_then(|config| config.file);
    match parsed {
        Some(file) if !file.path.is_empty() => (PathBuf::from(file.path), file.create_if_missing),
        _ => (config_dir.join("alias.yml"), true),
    }
}

fn validate_name(name: &str) -> Result<String, ServiceError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ServiceError::InvalidInput("Alias cannot be empty".into()));
    }
    if name.chars().any(char::is_whitespace) {
        return Err(ServiceError::InvalidInput(
            "Alias cannot contain whitespace".into(),
        ));
    }
    Ok(name.to_string())
}

fn decode_address(value: &str) -> Result<Vec<u8>, ServiceError> {
    let value = value.trim().strip_prefix("0x").unwrap_or(value.trim());
    if value.is_empty() {
        return Err(ServiceError::InvalidInput("Address cannot be empty".into()));
    }
    hex::decode(value).map_err(|error| {
        ServiceError::InvalidInput(format!("Address is not valid hexadecimal: {error}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_dir() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("config.yml"), "{}").unwrap();
        directory
    }

    #[tokio::test]
    async fn reads_scalar_and_mapping_aliases_in_sorted_order() {
        let directory = config_dir();
        std::fs::write(
            directory.path().join("alias.yml"),
            "aliases:\n  zebra: aabb\n  alpha:\n    address: 00ff\n    type: vertex\n",
        )
        .unwrap();

        let aliases = list(directory.path()).await.unwrap();

        assert_eq!(aliases[0].name, "alpha");
        assert_eq!(aliases[0].address_hex, "00ff");
        assert_eq!(aliases[0].kind, "vertex");
        assert_eq!(aliases[1].name, "zebra");
    }

    #[tokio::test]
    async fn put_and_delete_round_trip_atomically() {
        let directory = config_dir();

        mutate(
            directory.path(),
            AliasMutation::Put {
                name: "wallet".into(),
                address_hex: "0xaabb".into(),
                kind: "account".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(list(directory.path()).await.unwrap().len(), 1);

        mutate(
            directory.path(),
            AliasMutation::Delete {
                name: "wallet".into(),
            },
        )
        .await
        .unwrap();
        assert!(list(directory.path()).await.unwrap().is_empty());
        assert!(!directory.path().join("alias.tmp").exists());
    }

    #[tokio::test]
    async fn listing_missing_alias_file_is_read_only() {
        let directory = config_dir();
        let path = directory.path().join("alias.yml");

        assert!(list(directory.path()).await.unwrap().is_empty());
        assert!(!path.exists(), "a read-only list must not create alias.yml");
    }

    #[tokio::test]
    async fn rejects_invalid_input_before_writing() {
        let directory = config_dir();

        let result = mutate(
            directory.path(),
            AliasMutation::Put {
                name: "bad name".into(),
                address_hex: "not-hex".into(),
                kind: String::new(),
            },
        )
        .await;

        assert!(result.is_err());
        assert!(list(directory.path()).await.unwrap().is_empty());
    }
}
