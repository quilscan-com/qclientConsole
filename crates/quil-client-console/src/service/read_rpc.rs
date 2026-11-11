use std::{
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};

use super::{node, ServiceError};

#[derive(Deserialize, Serialize)]
struct ReadRpcFile {
    read_rpc: String,
}

pub(crate) fn file(config_dir: &Path) -> PathBuf {
    config_dir.join(".qclient-console").join("read-rpc.yml")
}

pub(crate) fn load(config_dir: &Path, fallback: &str) -> Result<String, ServiceError> {
    Ok(load_override(config_dir)?.unwrap_or_else(|| fallback.into()))
}

pub(crate) fn load_override(config_dir: &Path) -> Result<Option<String>, ServiceError> {
    let path = file(config_dir);
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(ServiceError::Config(format!(
                "Cannot read {}: {error}",
                path.display()
            )))
        }
    };
    let value: ReadRpcFile = serde_yaml::from_str(&contents).map_err(|error| {
        ServiceError::Config(format!("Cannot parse {}: {error}", path.display()))
    })?;
    let value = value.read_rpc.trim();
    if value.is_empty() {
        return Ok(None);
    }
    validate(value)?;
    Ok(Some(value.into()))
}

pub(crate) fn validate(value: &str) -> Result<(), ServiceError> {
    node::grpc_address(value).map(|_| ())
}

pub(crate) fn save(config_dir: &Path, value: Option<&str>) -> Result<(), ServiceError> {
    let path = file(config_dir);
    let value = value.map(str::trim).filter(|value| !value.is_empty());
    let Some(value) = value else {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ServiceError::Config(format!(
                "Cannot remove {}: {error}",
                path.display()
            ))),
        };
    };
    validate(value)?;
    let parent = path.parent().expect("read RPC path always has a parent");
    create_private_dir(parent).map_err(|error| {
        ServiceError::Config(format!("Cannot create {}: {error}", parent.display()))
    })?;
    let contents = serde_yaml::to_string(&ReadRpcFile {
        read_rpc: value.into(),
    })
    .map_err(|error| ServiceError::Config(format!("Cannot encode read RPC: {error}")))?;
    atomic_write(parent, "read-rpc.yml", contents.as_bytes())
        .map_err(|error| ServiceError::Config(format!("Cannot write {}: {error}", path.display())))
}

fn create_private_dir(path: &Path) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700);
        match builder.create(path) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    match std::fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    Ok(())
}

fn atomic_write(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), std::io::Error> {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    let temporary = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let target = dir.join(name);
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, target)?;
        #[cfg(unix)]
        std::fs::File::open(dir)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_is_private_atomic_and_preserves_the_previous_value_on_validation_failure() {
        let config = tempfile::tempdir().unwrap();
        let first = "/ip4/127.0.0.1/tcp/8337";
        save(config.path(), Some(first)).unwrap();

        assert_eq!(load(config.path(), "fallback").unwrap(), first);
        assert!(save(config.path(), Some("not-a-multiaddr")).is_err());
        assert_eq!(load(config.path(), "fallback").unwrap(), first);
        assert_eq!(
            std::fs::read_dir(file(config.path()).parent().unwrap())
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<Vec<_>>(),
            vec![std::ffi::OsString::from("read-rpc.yml")]
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(file(config.path()).parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                std::fs::metadata(file(config.path()))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
}
