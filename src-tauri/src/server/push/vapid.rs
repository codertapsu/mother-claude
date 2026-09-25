//! The VAPID (RFC 8292) application-server key: one P-256 keypair, persisted
//! 0600 as `vapid.json` in the app data dir. The public key is what browsers
//! pass to `pushManager.subscribe({applicationServerKey})`; every push request
//! carries a JWT signed with the private key.
//!
//! Loading is tolerant: a missing, unreadable or corrupt file yields a fresh
//! key (logged). Subscriptions made against an older key are then useless, so
//! `push.json` records which key they were made with and drops them on
//! mismatch (see `store::PushFile::sanitize`).

use std::path::Path;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use web_push_native::jwt_simple::algorithms::{ECDSAP256PublicKeyLike as _, ES256KeyPair};

/// File name inside the app data dir.
pub const FILE: &str = "vapid.json";

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct VapidFile {
    /// base64url (no padding) of the 32-byte private scalar.
    private_key: String,
    /// base64url (no padding) of the 65-byte uncompressed public point.
    public_key: String,
    created_at: i64,
}

/// The keypair plus its encoded public key.
pub struct VapidKeys {
    key_pair: ES256KeyPair,
    public_key: String,
}

impl std::fmt::Debug for VapidKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the private key.
        f.debug_struct("VapidKeys")
            .field("public_key", &self.public_key)
            .finish_non_exhaustive()
    }
}

impl VapidKeys {
    /// A fresh random keypair.
    pub fn generate() -> Self {
        Self::from_pair(ES256KeyPair::generate())
    }

    /// Rebuild from the 32-byte private scalar.
    pub fn from_private_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 32 {
            return None;
        }
        ES256KeyPair::from_bytes(bytes).ok().map(Self::from_pair)
    }

    fn from_pair(key_pair: ES256KeyPair) -> Self {
        let public_key =
            URL_SAFE_NO_PAD.encode(key_pair.public_key().public_key().to_bytes_uncompressed());
        Self {
            key_pair,
            public_key,
        }
    }

    /// base64url (no padding) of the uncompressed public point — the
    /// `applicationServerKey`.
    pub fn public_key(&self) -> &str {
        &self.public_key
    }

    pub fn key_pair(&self) -> &ES256KeyPair {
        &self.key_pair
    }

    /// Load `<dir>/vapid.json`, or generate and persist (0600) a new key.
    /// `None` ⇒ in-memory only (tests).
    pub fn load_or_create(dir: Option<&Path>) -> Self {
        let Some(dir) = dir else {
            return Self::generate();
        };
        let path = dir.join(FILE);
        if path.exists() {
            match crate::persist::read_json::<VapidFile>(&path).and_then(|f| {
                URL_SAFE_NO_PAD
                    .decode(f.private_key.trim())
                    .ok()
                    .and_then(|b| Self::from_private_bytes(&b))
            }) {
                Some(keys) => {
                    crate::persist::restrict(&path);
                    return keys;
                }
                None => tracing::warn!(
                    path = %path.display(),
                    "VAPID key file is unusable; generating a new key (existing push \
                     subscriptions will need to be renewed)"
                ),
            }
        }
        let keys = Self::generate();
        let file = VapidFile {
            private_key: URL_SAFE_NO_PAD.encode(keys.key_pair.to_bytes()),
            public_key: keys.public_key.clone(),
            created_at: crate::claude::registry::now_ms(),
        };
        if let Err(e) = crate::persist::write_json_private(&path, &file) {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "could not persist the VAPID key; push subscriptions will not survive a restart"
            );
        }
        keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_persists_across_loads_with_private_mode() {
        let dir = tempfile::tempdir().unwrap();
        let a = VapidKeys::load_or_create(Some(dir.path()));
        let b = VapidKeys::load_or_create(Some(dir.path()));
        assert_eq!(a.public_key(), b.public_key());
        // 65-byte uncompressed P-256 point.
        let raw = URL_SAFE_NO_PAD.decode(a.public_key()).unwrap();
        assert_eq!(raw.len(), 65);
        assert_eq!(raw[0], 0x04);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
        // The Debug form never contains the private key.
        let file: VapidFile = crate::persist::read_json(&dir.path().join(FILE)).unwrap();
        assert!(!format!("{a:?}").contains(&file.private_key));
        assert_eq!(file.public_key, a.public_key());
    }

    #[test]
    fn corrupt_file_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE), "{not json").unwrap();
        let a = VapidKeys::load_or_create(Some(dir.path()));
        let b = VapidKeys::load_or_create(Some(dir.path()));
        assert_eq!(a.public_key(), b.public_key());
        std::fs::write(dir.path().join(FILE), r#"{"privateKey":"AAAA"}"#).unwrap();
        let c = VapidKeys::load_or_create(Some(dir.path()));
        assert_ne!(c.public_key(), a.public_key());
    }

    #[test]
    fn in_memory_keys_differ() {
        assert_ne!(
            VapidKeys::load_or_create(None).public_key(),
            VapidKeys::load_or_create(None).public_key()
        );
        assert!(VapidKeys::from_private_bytes(&[1u8; 31]).is_none());
    }
}
