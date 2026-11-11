//! Local legacy identity validation and recipient package preparation.
mod client;
mod config;
mod journal;
mod orchestrator;
pub use config::prepare_package;
pub use journal::{
    LegacyMigrationBatchPlan, LegacyMigrationJournal, LegacyMigrationJournalSeed,
    LegacyMigrationObservation, LegacyMigrationSubmitOutcome,
};
pub(crate) use orchestrator::LegacyMigrationCoordinator;

use crate::domain::LegacyIdentitySummary;
use anyhow::{ensure, Result};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

/// Transient old Ed448 ownership key. It is deliberately not printable,
/// cloneable, or serializable and is never written to the migration journal.
pub struct SecretLegacyKey(LegacyIdentity);

impl SecretLegacyKey {
    pub fn parse(value: &str) -> Result<Self> {
        LegacyIdentity::parse(value).map(Self)
    }

    pub(crate) fn into_identity(self) -> LegacyIdentity {
        self.0
    }

    pub(crate) fn fingerprint(&self) -> &str {
        &self.0.summary().fingerprint
    }
}

pub struct LegacyMigrationRequest {
    source: SecretLegacyKey,
    pub destination: crate::domain::LegacyMigrationDestination,
    pub allow_initialize_recipient: bool,
}

impl LegacyMigrationRequest {
    pub fn new(
        source: SecretLegacyKey,
        destination: crate::domain::LegacyMigrationDestination,
        allow_initialize_recipient: bool,
    ) -> Self {
        Self {
            source,
            destination,
            allow_initialize_recipient,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        LegacyIdentity,
        crate::domain::LegacyMigrationDestination,
        bool,
    ) {
        (
            self.source.into_identity(),
            self.destination,
            self.allow_initialize_recipient,
        )
    }
}

/// Secret ownership is deliberately neither printable, cloneable nor serializable.
/// ```compile_fail
/// use quil_client_console::service::legacy_migration::LegacyIdentity;
/// fn debug<T: std::fmt::Debug>() {} debug::<LegacyIdentity>();
/// ```
/// ```compile_fail
/// use quil_client_console::service::legacy_migration::LegacyIdentity;
/// fn cloneable<T: Clone>() {} cloneable::<LegacyIdentity>();
/// ```
/// ```compile_fail
/// use quil_client_console::service::legacy_migration::LegacyIdentity;
/// fn serialize<T: serde::Serialize>() {} serialize::<LegacyIdentity>();
/// ```
/// ```compile_fail
/// use quil_client_console::service::legacy_migration::LegacyIdentity;
/// fn deserialize<T: serde::de::DeserializeOwned>() {} deserialize::<LegacyIdentity>();
/// ```
pub struct LegacyIdentity {
    bytes: Zeroizing<Vec<u8>>,
    summary: LegacyIdentitySummary,
}

impl LegacyIdentity {
    pub fn parse(value: &str) -> Result<Self> {
        let value = value.trim();
        let value = value.strip_prefix("0x").unwrap_or(value);
        ensure!(
            value.len() == 228,
            "old peer key must contain exactly 114 bytes"
        );
        let bytes = Zeroizing::new(
            hex::decode(value).map_err(|_| anyhow::anyhow!("old peer key is not hex"))?,
        );
        let seed: &[u8; 57] = bytes[..57].try_into().expect("checked length");
        let public = quil_p2p::ed448_identity::derive_public_key(seed);
        ensure!(
            bool::from(public.ct_eq(&bytes[57..])),
            "old peer seed and public key do not match"
        );
        let normalized = Zeroizing::new(hex::encode(&*bytes));
        let peer =
            libp2p::PeerId::from_bytes(&quil_crypto::peer_id_multihash_from_ed448_pubkey(&public))?;
        let summary = LegacyIdentitySummary {
            fingerprint: hex::encode(&Sha256::digest(&public)[..16]),
            masked_key: format!("{}…{}", &normalized[..4], &normalized[224..]),
            peer_id: peer.to_string(),
        };
        Ok(Self { bytes, summary })
    }
    pub fn summary(&self) -> &LegacyIdentitySummary {
        &self.summary
    }
    fn normalized(&self) -> Zeroizing<String> {
        Zeroizing::new(hex::encode(&*self.bytes))
    }
}
