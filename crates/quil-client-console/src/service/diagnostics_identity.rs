use std::{io::Write, path::Path};

use crate::{domain::DiagnosticsIdentitySnapshot, service::ServiceError};

const IDENTITY_HEADER: &str =
    "# qclient-console diagnostics identity: a Falcon network key, not a prover key. PRIVATE: do not share.";

pub(super) fn create(path: &Path) -> Result<DiagnosticsIdentitySnapshot, ServiceError> {
    let key = quil_p2p::generate_falcon_signing_key();
    let snapshot = snapshot(path, &key)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|error| {
        let message = if error.kind() == std::io::ErrorKind::AlreadyExists {
            format!(
                "{} already exists; refusing to overwrite the private key",
                path.display()
            )
        } else {
            format!("Cannot create {}: {error}", path.display())
        };
        ServiceError::DiagnosticsIdentity(message)
    })?;
    let data = format!("{IDENTITY_HEADER}\n{}\n", hex::encode(key));
    if let Err(error) = file.write_all(data.as_bytes()) {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(ServiceError::DiagnosticsIdentity(format!(
            "Cannot write {}: {error}",
            path.display()
        )));
    }
    Ok(snapshot)
}

pub(super) fn inspect(path: &Path) -> Result<DiagnosticsIdentitySnapshot, ServiceError> {
    let key = load_key(path)?;
    snapshot(path, &key)
}

pub(super) fn load_key(path: &Path) -> Result<Vec<u8>, ServiceError> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        ServiceError::DiagnosticsIdentity(format!("Cannot read {}: {error}", path.display()))
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)
            .map_err(|error| {
                ServiceError::DiagnosticsIdentity(format!(
                    "Cannot read permissions for {}: {error}",
                    path.display()
                ))
            })?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            return Err(ServiceError::DiagnosticsIdentity(format!(
                "{} has unsafe permissions; set them to 600",
                path.display()
            )));
        }
    }
    let hex_text = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<String>();
    let key = hex::decode(hex_text).map_err(|_| {
        ServiceError::DiagnosticsIdentity(format!(
            "{} is not a valid diagnostic identity file",
            path.display()
        ))
    })?;
    if key.len() != quil_crypto::FALCON_SIGNING_KEY_LEN
        || quil_crypto::falcon_public_from_signing_key(&key).is_none()
    {
        return Err(ServiceError::DiagnosticsIdentity(format!(
            "{} is not a valid diagnostic identity file",
            path.display()
        )));
    }
    Ok(key)
}

fn snapshot(path: &Path, key: &[u8]) -> Result<DiagnosticsIdentitySnapshot, ServiceError> {
    let peer_id = quil_p2p::Keypair::falcon_from_bytes(key)
        .map_err(|error| {
            ServiceError::DiagnosticsIdentity(format!(
                "Invalid Falcon diagnostic identity: {error}"
            ))
        })?
        .public()
        .to_peer_id()
        .to_string();
    Ok(DiagnosticsIdentitySnapshot {
        path: path.to_string_lossy().into_owned(),
        peer_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_a_private_identity_once_and_loads_the_same_peer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diagnostics.key");

        let created = create(&path).unwrap();
        let before = std::fs::read(&path).unwrap();
        let loaded = inspect(&path).unwrap();

        assert_eq!(created, loaded);
        assert_eq!(created.path, path.to_string_lossy());
        assert!(created.peer_id.starts_with('Q'));
        assert!(
            String::from_utf8_lossy(&before).starts_with("# qclient-console diagnostics identity")
        );
        assert!(create(&path).unwrap_err().to_string().contains("overwrite"));
        assert_eq!(std::fs::read(&path).unwrap(), before);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_an_identity_readable_by_other_users() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diagnostics.key");
        create(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let error = inspect(&path).unwrap_err().to_string();
        assert!(error.contains("permissions"), "{error}");
        assert!(error.contains("600"), "{error}");
    }
}
