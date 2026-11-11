use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

use argon2::{
    password_hash::{rand_core::OsRng as SaltRng, PasswordHash, PasswordHasher, SaltString},
    Argon2, PasswordVerifier,
};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};

const SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Debug)]
pub(super) enum AuthError {
    AlreadyInitialized,
    Storage(io::Error),
}

#[derive(Deserialize, Serialize)]
struct AuthFile {
    password_hash: String,
}

pub(super) struct AuthStore {
    path: PathBuf,
    password_hash: Mutex<Option<String>>,
    sessions: Mutex<HashMap<String, Instant>>,
}

impl AuthStore {
    pub(super) fn new(home: PathBuf) -> io::Result<Self> {
        let path = home.join(".qclient-console/web-auth.json");
        let password_hash = match fs::read(&path) {
            Ok(bytes) => Some(
                serde_json::from_slice::<AuthFile>(&bytes)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
                    .password_hash,
            ),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };

        Ok(Self {
            path,
            password_hash: Mutex::new(password_hash),
            sessions: Mutex::new(HashMap::new()),
        })
    }

    pub(super) fn initialized(&self) -> bool {
        self.password_hash
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }

    pub(super) fn setup(&self, password: &str) -> Result<(), AuthError> {
        let mut stored = self
            .password_hash
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if stored.is_some() {
            return Err(AuthError::AlreadyInitialized);
        }

        let salt = SaltString::generate(&mut SaltRng);
        let password_hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|error| AuthError::Storage(io::Error::other(error.to_string())))?
            .to_string();
        self.write_auth_file(&password_hash)
            .map_err(AuthError::Storage)?;
        *stored = Some(password_hash);
        Ok(())
    }

    pub(super) fn authenticate(&self, password: &str) -> bool {
        let stored = self
            .password_hash
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(stored) = stored.as_deref() else {
            return false;
        };
        let Ok(hash) = PasswordHash::new(stored) else {
            return false;
        };
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    }

    pub(super) fn create_session(&self) -> String {
        self.create_session_at(Instant::now())
    }

    pub(super) fn create_session_at(&self, now: Instant) -> String {
        let mut bytes = [0_u8; 32];
        OsRng.fill_bytes(&mut bytes);
        let token = hex::encode(bytes);
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(token.clone(), now);
        token
    }

    pub(super) fn validate(&self, token: &str) -> bool {
        self.validate_at(token, Instant::now())
    }

    pub(super) fn validate_at(&self, token: &str, now: Instant) -> bool {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(last_seen) = sessions.get(token).copied() else {
            return false;
        };
        if now.checked_duration_since(last_seen).unwrap_or_default() >= SESSION_IDLE_TIMEOUT {
            sessions.remove(token);
            return false;
        }
        sessions.insert(token.to_owned(), now);
        true
    }

    pub(super) fn logout(&self, token: &str) {
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(token);
    }

    fn write_auth_file(&self, password_hash: &str) -> io::Result<()> {
        let directory = self.path.parent().expect("auth path always has a parent");
        fs::create_dir_all(directory)?;

        let mut suffix = [0_u8; 8];
        OsRng.fill_bytes(&mut suffix);
        let temporary = directory.join(format!(".web-auth.json.{}.tmp", hex::encode(suffix)));
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            let bytes = serde_json::to_vec(&AuthFile {
                password_hash: password_hash.to_owned(),
            })
            .map_err(io::Error::other)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &self.path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::AuthStore;

    #[test]
    fn sessions_expire_after_thirty_idle_minutes_and_slide_on_use() {
        let home = tempfile::tempdir().unwrap();
        let store = AuthStore::new(home.path().to_path_buf()).unwrap();
        let started = Instant::now();
        let token = store.create_session_at(started);

        assert!(store.validate_at(&token, started + Duration::from_secs(29 * 60)));
        assert!(store.validate_at(&token, started + Duration::from_secs(58 * 60)));
        assert!(!store.validate_at(&token, started + Duration::from_secs(88 * 60)));
    }
}
