//! Paired devices and their tokens (design §2.5, §2.6).
//!
//! Owner: **B1 (security/remote)** — STUB. What B1 implements here:
//!
//! * `devices.json` in the app data dir (0600, atomic write via
//!   `crate::persist::write_json_private`):
//!   `{devices:[{id,name,platform,tokenSha256,createdAt,lastSeenAt,lastIp,
//!   ingress,allowDangerous,revoked}], pending:[…]}`. Only SHA-256 hashes of
//!   `mcd_…` tokens are stored (`auth::sha256_hex`); compare in constant time
//!   (`auth::token_matches` on the hex digests).
//! * [`DeviceStore::authenticate`]: token → device (not revoked), updating
//!   lastSeenAt / lastIp / ingress (debounce disk writes).
//! * issue / rename / allowDangerous / revoke. Revoking MUST call
//!   `crate::server::ws::close_device(state, id)` and
//!   `crate::server::push::drop_device(state, id)`, then
//!   `crate::server::ws::publish_devices(state)`.
//!
//! AppState field: `state.devices` (constructed with
//! [`DeviceStore::load`] from `Auth::config_dir`; `None` = in-memory only, for
//! tests).

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::context::Ingress;

/// What authentication learns about a device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceAuth {
    pub id: String,
    pub name: String,
    pub allow_dangerous: bool,
}

/// A paired device as the desktop's device list shows it (never the token or
/// its hash).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingress: Option<String>,
    pub allow_dangerous: bool,
    pub revoked: bool,
}

/// The device registry. B1 replaces the internals; keep the method signatures
/// used by the foundation (`authenticate`, `get`, `list`).
#[derive(Debug, Default)]
pub struct DeviceStore {
    /// `<app data dir>/devices.json`, or `None` for an in-memory store.
    pub path: Option<PathBuf>,
}

impl DeviceStore {
    /// Load `devices.json` from `data_dir` (tolerant: missing or corrupt ⇒
    /// empty). `None` ⇒ in-memory only.
    pub fn load(data_dir: Option<&Path>) -> Self {
        Self {
            path: data_dir.map(|d| d.join("devices.json")),
        }
    }

    /// Resolve an `mcd_…` token presented from `client_ip` on `ingress`.
    /// STUB: no devices yet.
    pub fn authenticate(
        &self,
        _token: &str,
        _client_ip: IpAddr,
        _ingress: Ingress,
    ) -> Option<DeviceAuth> {
        None
    }

    /// One device by id. STUB.
    pub fn get(&self, _id: &str) -> Option<DeviceInfo> {
        None
    }

    /// Every device (including revoked ones, flagged). STUB.
    pub fn list(&self) -> Vec<DeviceInfo> {
        Vec::new()
    }
}
