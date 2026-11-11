//! This machine's identity (docs/PLAN-MACHINES.md §5.2): the per-install
//! machine id the backup already keeps, a name, and the machine key — an
//! Ed25519 key made at first use, kept in the OS secret store
//! (`machine-key`), never in a file of ours, a log, a frame or a backup. Its
//! public half is this machine's address on the peer transport.

use std::sync::OnceLock;

use iroh::SecretKey;

use crate::claude::secrets::{Secret, SecretName, SecretStore};
use crate::state::AppState;

/// Who this machine is.
#[derive(Clone)]
pub struct Identity {
    pub id: String,
    /// The host's name (the default until the user picks one).
    pub host_name: String,
    pub key: SecretKey,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("id", &self.id)
            .field("key", &self.key.public().to_string())
            .finish_non_exhaustive()
    }
}

impl Identity {
    /// The machine key's public half, hex: what peers pin.
    pub fn key_hex(&self) -> String {
        self.key.public().to_string()
    }
}

/// Loads this machine's identity, making the key on first use.
pub fn load(state: &AppState) -> Result<Identity, String> {
    let machine = crate::claude::backup::ops::this_machine(state)
        .map_err(|e| format!("This machine's id could not be read: {e}"))?;
    let key = load_or_create_key(state.accounts.secret_store().as_ref())?;
    Ok(Identity {
        id: machine.id,
        host_name: machine.name,
        key,
    })
}

fn load_or_create_key(store: &dyn SecretStore) -> Result<SecretKey, String> {
    match store.get(&SecretName::MachineKey) {
        Ok(Some(secret)) => {
            if let Some(key) = parse_key(secret.expose()) {
                return Ok(key);
            }
            tracing::warn!("the stored machine key is malformed; making a new one");
        }
        Ok(None) => {}
        Err(e) => return Err(format!("The machine key could not be read: {e}")),
    }
    let key = SecretKey::generate();
    let hex: String = key.to_bytes().iter().map(|b| format!("{b:02x}")).collect();
    store
        .set(&SecretName::MachineKey, &Secret::new(hex))
        .map_err(|e| format!("The machine key could not be saved: {e}"))?;
    Ok(key)
}

fn parse_key(hex: &str) -> Option<SecretKey> {
    let hex = hex.trim();
    if hex.len() != 64 {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(SecretKey::from_bytes(&bytes))
}

/// The OS's own name and version (`macOS 27.2 …`, `Windows 11 Pro …`).
pub fn os_version() -> Option<String> {
    static CACHE: OnceLock<Option<String>> = OnceLock::new();
    CACHE
        .get_or_init(|| sysinfo::System::long_os_version().ok())
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::secrets::store_for;

    #[test]
    fn the_key_is_made_once_and_kept() {
        let store = store_for(None);
        let a = load_or_create_key(store.as_ref()).unwrap();
        let b = load_or_create_key(store.as_ref()).unwrap();
        assert_eq!(a.public(), b.public());
    }

    #[test]
    fn a_malformed_stored_key_is_replaced() {
        let store = store_for(None);
        store
            .set(&SecretName::MachineKey, &Secret::new("not hex"))
            .unwrap();
        let a = load_or_create_key(store.as_ref()).unwrap();
        let stored = store.get(&SecretName::MachineKey).unwrap().unwrap();
        assert_eq!(parse_key(stored.expose()).unwrap().public(), a.public());
    }
}
