use super::LegacyIdentity;
use crate::domain::{LegacyMigrationDestination as Destination, LegacyMigrationPackage};
use anyhow::{bail, ensure, Result};
use quil_config::{create_private_dir_all, load_config, Config};
use quil_keys::FileKeyManager;
use rand::RngCore;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use zeroize::Zeroizing;

/// Prepare only local files. Existing recipients require explicit permission to initialize.
pub fn prepare_package(
    home: &Path,
    current: Option<&Path>,
    destination: Destination,
    identity: &LegacyIdentity,
    allow_initialize_recipient: bool,
) -> Result<LegacyMigrationPackage> {
    let task_dir = home
        .join(".qclient-console/legacy-wallets")
        .join(format!("ed448-{}", identity.summary().fingerprint));
    reject_symlinks(&task_dir)?;
    ensure!(
        !task_dir.exists() || task_dir.is_dir(),
        "task path is not a directory"
    );
    let generated = matches!(destination, Destination::CreateNew);
    let directory = match destination {
        Destination::CreateNew => task_dir.clone(),
        Destination::CurrentNode => current
            .ok_or_else(|| anyhow::anyhow!("no active node configuration"))?
            .to_path_buf(),
        Destination::ExistingConfig(path) => {
            if path.file_name().is_some_and(|n| n == "config.yml") {
                path.parent().unwrap_or(Path::new(".")).to_path_buf()
            } else {
                path
            }
        }
    };
    reject_symlinks(&directory)?;
    let config_path = directory.join("config.yml");
    reject_symlinks(&config_path)?;
    if !config_path.exists() {
        ensure!(generated, "recipient configuration does not exist");
        recover_unpublished_config(&directory, identity)?;
        let mut config = load_config(&directory)
            .map_err(|_| anyhow::anyhow!("cannot create recipient configuration"))?;
        config.p2p.peer_priv_key = identity.normalized().to_string();
        save_config_atomic(&directory, &config)
            .map_err(|_| anyhow::anyhow!("cannot save recipient configuration"))?;
    }
    let mut config = load_config(&directory)
        .map_err(|_| anyhow::anyhow!("cannot read recipient configuration"))?;
    if generated {
        ensure!(
            config.p2p.peer_priv_key.is_empty()
                || config.p2p.peer_priv_key == *identity.normalized(),
            "task configuration has a conflicting peer identity"
        );
    } else if !config.p2p.peer_priv_key.is_empty() {
        LegacyIdentity::parse(&config.p2p.peer_priv_key)?;
    }
    // Resolve using the node's path semantics, including its wrong-path guard.
    let mut settings = config.key.key_store_file.clone();
    settings.create_if_missing = true;
    let keys_path = quil_keys::resolve_keystore_path(&settings, &directory)
        .map_err(|_| anyhow::anyhow!("recipient keystore path conflicts with configuration"))?;
    reject_symlinks(&keys_path)?;
    ensure!(
        keys_path != config_path
            && (!config_path.exists()
                || !keys_path.exists()
                || std::fs::canonicalize(&keys_path)? != std::fs::canonicalize(&config_path)?),
        "keystore and config paths conflict"
    );
    if generated {
        ensure!(
            keys_path == std::env::current_dir()?.join(directory.join("keys.yml")),
            "generated keystore path conflicts with task directory"
        );
    }
    let missing_encryption = config.key.key_store_file.encryption_key.is_empty();
    if missing_encryption {
        ensure!(
            !keys_path.exists(),
            "keystore exists without an encryption key"
        );
        ensure!(
            !generated && allow_initialize_recipient,
            "recipient initialization requires confirmation"
        );
        let mut key = Zeroizing::new([0u8; 32]);
        rand::thread_rng().fill_bytes(&mut *key);
        config.key.key_store_file.encryption_key = hex::encode(*key);
    }
    let encryption = Zeroizing::new(
        hex::decode(&config.key.key_store_file.encryption_key)
            .map_err(|_| anyhow::anyhow!("invalid recipient encryption key"))?,
    );
    ensure!(
        encryption.len() == 32 && encryption.iter().any(|b| *b != 0),
        "invalid recipient encryption key"
    );
    let manager = Arc::new(
        FileKeyManager::new(
            keys_path.clone(),
            &config.key.key_store_file.encryption_key,
            "q-prover-key".into(),
            Box::new(quil_crypto::FalconKeyConstructor),
        )
        .map_err(|_| anyhow::anyhow!("cannot load recipient keystore"))?,
    );
    let entries = stored_key_entries(&keys_path)?;
    let resume_peer = generated && config.p2p.peer_priv_key.is_empty();
    ensure!(
        !resume_peer || entries.is_empty(),
        "empty generated peer identity with existing secrets"
    );
    const STANDARD: [(&str, u8); 6] = [
        ("q-prover-key", 8),
        ("q-onion-key", 9),
        ("q-view-key", 4),
        ("q-spend-key", 4),
        ("q-device-key", 4),
        ("q-device-pre-key", 4),
    ];
    // Loading tolerates broken signers; explicitly decrypt every entry before any writes.
    for entry in &entries {
        let secret = Zeroizing::new(
            manager
                .get_secret_key_bytes_by_id(&entry.id)
                .map_err(|_| anyhow::anyhow!("recipient keystore cannot decrypt"))?,
        );
        ensure!(
            !secret.is_empty(),
            "recipient keystore contains an empty secret"
        );
        if generated {
            if let Some((_, expected)) = STANDARD.iter().find(|(id, _)| *id == entry.id) {
                ensure!(
                    entry.key_type == *expected,
                    "generated standard key has wrong type"
                );
                match expected {
                    8 => ensure!(
                        quil_crypto::falcon_public_from_signing_key(&secret).as_deref()
                            == Some(entry.public_key.as_slice()),
                        "generated prover keypair is invalid"
                    ),
                    4 => {
                        use ed448_goldilocks_plus::{DecafPoint, Scalar, ScalarBytes};
                        ensure!(secret.len() == 57, "generated agreement key is invalid");
                        let scalar = Option::<Scalar>::from(Scalar::from_canonical_bytes(
                            &ScalarBytes::clone_from_slice(&secret),
                        ))
                        .ok_or_else(|| anyhow::anyhow!("generated agreement scalar is invalid"))?;
                        ensure!(
                            (DecafPoint::GENERATOR * scalar).compress().0.as_slice()
                                == entry.public_key,
                            "generated agreement keypair is invalid"
                        );
                    }
                    _ => (), // RecipientIdentity checks the onion keypair below.
                }
            }
        }
        if entry.id == "q-peer-key" && !config.p2p.peer_priv_key.is_empty() {
            let peer = LegacyIdentity::parse(&config.p2p.peer_priv_key)?;
            ensure!(
                entry.key_type == 0
                    && entry.public_key == peer.bytes[57..]
                    && (secret.as_slice() == &peer.bytes[..57]
                        || secret.as_slice() == peer.bytes.as_slice()),
                "recipient peer keys conflict"
            );
        }
    }
    let onion = entries.iter().find(|e| e.id == "q-onion-key");
    // Validate an existing onion before completing other missing slots.
    if onion.is_some_and(|entry| entry.key_type == 9) {
        recipient_address(config.clone(), directory.clone(), manager.clone())?;
    }
    let needs_initialization = if generated {
        STANDARD
            .iter()
            .any(|(id, _)| !entries.iter().any(|entry| entry.id == *id))
    } else {
        onion.is_none_or(|entry| entry.key_type != 9)
    };
    if resume_peer {
        config.p2p.peer_priv_key = identity.normalized().to_string();
        save_config_atomic(&directory, &config)
            .map_err(|_| anyhow::anyhow!("cannot complete generated peer configuration"))?;
    }
    if needs_initialization {
        ensure!(
            generated || allow_initialize_recipient,
            "recipient initialization requires confirmation"
        );
        // Persist encryption before writing any encrypted secrets (safe interrupted restart).
        if missing_encryption {
            save_config_atomic(&directory, &config)
                .map_err(|_| anyhow::anyhow!("cannot save recipient encryption configuration"))?;
        }
        remove_keystore_temp_symlink(&keys_path)?;
        manager
            .ensure_standard_keys()
            .map_err(|_| anyhow::anyhow!("cannot initialize recipient keys"))?;
    }
    let recipient_address = recipient_address(config, directory.clone(), manager)?;
    create_private_dir_all(&task_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&task_dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(LegacyMigrationPackage {
        task_dir,
        recipient_config_dir: directory,
        config_path,
        keys_path,
        generated,
        recipient_address,
    })
}

// quil-keys writes through `<keystore>.tmp`. A stale or planted symlink at
// that exact scratch path must be unlinked (not followed) before asking the
// dependency to initialize keys, otherwise it could truncate another file.
fn remove_keystore_temp_symlink(keys_path: &Path) -> Result<()> {
    let temporary = keys_path.with_extension("tmp");
    match std::fs::symlink_metadata(&temporary) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            std::fs::remove_file(&temporary)?;
            Ok(())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

// A crash before the first config rename can leave one complete private temp.
// Recover only that unambiguous state; never remove unrelated or concurrent files.
fn recover_unpublished_config(directory: &Path, identity: &LegacyIdentity) -> Result<()> {
    if !directory.exists() {
        return Ok(());
    }
    let mut entries = std::fs::read_dir(directory)?;
    let Some(entry) = entries.next().transpose()? else {
        return Ok(());
    };
    ensure!(
        entries.next().is_none() && entry.file_type()?.is_file(),
        "task directory contains unrelated data"
    );
    let name = entry.file_name();
    let valid_name = name
        .to_str()
        .and_then(|name| {
            let (pid, counter) = name.strip_prefix("config.yml.tmp-")?.split_once('-')?;
            let pid = pid.parse::<u32>().ok()?;
            let counter = counter.parse::<u64>().ok()?;
            Some(pid > 0 && name == format!("config.yml.tmp-{pid}-{counter}"))
        })
        .unwrap_or(false);
    ensure!(valid_name, "task directory contains unrelated data");
    let bytes = Zeroizing::new(std::fs::read(entry.path())?);
    let config: Config = serde_yaml::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("unpublished task configuration is invalid"))?;
    let encryption = Zeroizing::new(
        hex::decode(&config.key.key_store_file.encryption_key)
            .map_err(|_| anyhow::anyhow!("unpublished task encryption key is invalid"))?,
    );
    ensure!(
        encryption.len() == 32 && encryption.iter().any(|byte| *byte != 0),
        "unpublished task encryption key is invalid"
    );
    ensure!(
        Path::new(&config.key.key_store_file.path) == directory.join("keys.yml"),
        "unpublished task keystore path conflicts"
    );
    ensure!(
        config.p2p.peer_priv_key.is_empty() || config.p2p.peer_priv_key == *identity.normalized(),
        "unpublished task peer identity conflicts"
    );
    let config_path = directory.join("config.yml");
    ensure!(
        !config_path.exists(),
        "task configuration was published concurrently"
    );
    std::fs::File::open(entry.path())?.sync_all()?;
    std::fs::rename(entry.path(), &config_path)?;
    quil_config::sync_parent_dir(&config_path);
    Ok(())
}

fn recipient_address(
    config: Config,
    _directory: PathBuf,
    manager: Arc<FileKeyManager>,
) -> Result<String> {
    let secret = Zeroizing::new(
        manager
            .get_secret_key_bytes_by_id("q-onion-key")
            .map_err(|_| anyhow::anyhow!("recipient onion keypair is invalid"))?,
    );
    let public = manager
        .get_public_key_bytes_by_id("q-onion-key")
        .map_err(|_| anyhow::anyhow!("recipient onion keypair is invalid"))?;
    let wallet = crate::service::wallet::Wallet::from_keys(
        quil_lattice_ct::confidential::transfer::network_identifier(config.p2p.network),
        quil_execution::domains::QUIL_TOKEN,
        &public,
        secret,
    )
    .map_err(|_| anyhow::anyhow!("recipient onion keypair is invalid"))?;
    Ok(hex::encode(wallet.address().encode()))
}

struct StoredEntry {
    id: String,
    key_type: u8,
    public_key: Vec<u8>,
}

#[derive(serde::Deserialize)]
struct StoredKeyFile {
    id: String,
    #[serde(rename = "type")]
    key_type: u8,
    #[serde(rename = "publicKey")]
    public_key: String,
}

fn stored_key_entries(path: &Path) -> Result<Vec<StoredEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let value: serde_yaml::Value = serde_yaml::from_slice(&std::fs::read(path)?)?;
    let mapping = match value {
        serde_yaml::Value::Null => return Ok(Vec::new()),
        serde_yaml::Value::Mapping(mapping) => mapping,
        _ => bail!("recipient keystore is not a mapping"),
    };
    let mut entries = Vec::new();
    for (key, value) in mapping {
        if value.is_null() {
            continue;
        }
        let id: String = serde_yaml::from_value(key)?;
        let stored: StoredKeyFile = serde_yaml::from_value(value)?;
        ensure!(
            !id.is_empty() && stored.id == id,
            "recipient keystore entries are inconsistent"
        );
        let public_key = hex::decode(&stored.public_key)
            .map_err(|_| anyhow::anyhow!("recipient keystore public key is invalid"))?;
        entries.push(StoredEntry {
            id,
            key_type: stored.key_type,
            public_key,
        });
    }
    entries.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(entries)
}

fn save_config_atomic(directory: &Path, config: &Config) -> Result<()> {
    create_private_dir_all(directory)?;
    let bytes = serde_yaml::to_string(config)?;
    crate::service::private_file::write_atomic(&directory.join("config.yml"), bytes.as_bytes())?;
    Ok(())
}

// Refuse links, including dangling links and linked parents, before reading/writing secrets.
fn reject_symlinks(path: &Path) -> Result<()> {
    for part in path.ancestors() {
        // macOS system aliases occur in std::env::temp_dir(); they are trusted roots.
        #[cfg(target_os = "macos")]
        if (part == Path::new("/var")
            && std::fs::read_link(part).ok().as_deref() == Some(Path::new("private/var")))
            || (part == Path::new("/tmp")
                && std::fs::read_link(part).ok().as_deref() == Some(Path::new("private/tmp")))
        {
            continue;
        }
        match std::fs::symlink_metadata(part) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!("recipient path contains a symbolic link")
            }
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => bail!("cannot inspect recipient path"),
        }
    }
    Ok(())
}
