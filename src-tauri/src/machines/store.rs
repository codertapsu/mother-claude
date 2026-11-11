//! `machines.json` (0600, app data dir): this machine's chosen name, whether
//! Machines is on, the paired machines and the settings of D10
//! (docs/PLAN-MACHINES.md §6). Read field by field, so one bad field never
//! loses the rest; fields from a newer version survive a save.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::grants::Grants;
use super::protocol::Versions;

pub const FILE_NAME: &str = "machines.json";

/// How machines meet across networks (D10).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RelayMode {
    /// n0's public relays, the library's default; n0's DNS finds a peer's
    /// current relay from its key.
    #[default]
    Public,
    /// Your own `iroh-relay`: a peer is dialled as its key plus this relay,
    /// so no n0 service is asked anything.
    Custom,
    /// Direct connections only: the local network, or a machine with a
    /// public address or a mapped port.
    None,
}

impl RelayMode {
    pub fn as_str(self) -> &'static str {
        match self {
            RelayMode::Public => "public",
            RelayMode::Custom => "custom",
            RelayMode::None => "none",
        }
    }
}

/// The relay setting, the same on every machine.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelaySetting {
    #[serde(default)]
    pub mode: RelayMode,
    /// The relay's `https://` URL when `mode` is `custom`.
    #[serde(default)]
    pub url: Option<String>,
}

impl RelaySetting {
    /// Checks a setting before it is saved: a custom relay needs an
    /// `https://` (or, for a relay on this machine, `http://localhost`) URL.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.mode != RelayMode::Custom {
            return Ok(());
        }
        let Some(url) = self.url.as_deref() else {
            return Err("Enter your relay's address (https://…).");
        };
        let parsed = url::Url::parse(url).map_err(|_| "That is not a valid address.")?;
        let local = matches!(parsed.host_str(), Some("localhost") | Some("127.0.0.1"));
        if parsed.scheme() == "https" || (parsed.scheme() == "http" && local) {
            Ok(())
        } else {
            Err("Your relay's address must start with https://.")
        }
    }

    /// The same setting, comparing what matters (a URL only for `custom`).
    pub fn same_as(&self, other: &RelaySetting) -> bool {
        self.mode == other.mode
            && (self.mode != RelayMode::Custom
                || self.url.as_deref().map(normalize_url)
                    == other.url.as_deref().map(normalize_url))
    }
}

fn normalize_url(u: &str) -> String {
    u.trim().trim_end_matches('/').to_ascii_lowercase()
}

/// The settings of D10 that Phase 1 uses. Unknown fields survive a save.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub relay: RelaySetting,
    /// Hold a keep-awake assertion while another machine uses this one.
    #[serde(default = "yes")]
    pub keep_awake: bool,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            relay: RelaySetting::default(),
            keep_awake: true,
            extra: BTreeMap::new(),
        }
    }
}

/// A paired machine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerRecord {
    /// Its machine id (the per-install id).
    pub id: String,
    /// The name it gave itself.
    pub name: String,
    /// The name the user gave it here, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub os_version: Option<String>,
    /// Its machine key (the transport's endpoint id, hex): the only thing it
    /// is recognised by.
    pub key: String,
    /// What it may do on this machine.
    #[serde(default)]
    pub grants: Grants,
    /// What this machine may do on it, as it last said.
    #[serde(default)]
    pub their_grants: Option<Grants>,
    pub paired_at: i64,
    #[serde(default)]
    pub last_seen: Option<i64>,
    #[serde(default)]
    pub versions: Option<Versions>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Its relay setting, as it last said.
    #[serde(default)]
    pub relay: Option<RelaySetting>,
    /// Its last known direct addresses, tried first (mDNS is filtered on
    /// some networks).
    #[serde(default)]
    pub addrs: Vec<String>,
    /// Its last home relay.
    #[serde(default)]
    pub relay_url: Option<String>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl PeerRecord {
    /// The name to show.
    pub fn display_name(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.name)
    }
}

/// The whole file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MachinesFile {
    /// This machine's name, when the user chose one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Machines is on (the setup passed and the user turned it on).
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub peers: Vec<PeerRecord>,
    #[serde(default)]
    pub settings: Settings,
    /// The setup step the user was on when they left it (1–7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_step: Option<u8>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

const KNOWN: [&str; 5] = ["name", "enabled", "peers", "settings", "setupStep"];

fn take<T: DeserializeOwned>(obj: &Map<String, Value>, key: &str, target: &mut T) {
    if let Some(v) = obj.get(key) {
        match serde_json::from_value::<T>(v.clone()) {
            Ok(parsed) => *target = parsed,
            Err(e) => tracing::warn!(field = key, error = %e, "machines.json: field ignored"),
        }
    }
}

impl MachinesFile {
    pub fn path(data_dir: &Path) -> PathBuf {
        data_dir.join(FILE_NAME)
    }

    /// Loads the file; a missing or unreadable one is an empty file, a bad
    /// field its default, a bad peer skipped (with a warning).
    pub fn load(data_dir: Option<&Path>) -> Self {
        let Some(dir) = data_dir else {
            return Self::default();
        };
        match crate::persist::read_json::<Value>(&Self::path(dir)) {
            Some(v) => Self::from_value(&v),
            None => Self::default(),
        }
    }

    pub fn from_value(v: &Value) -> Self {
        let mut out = Self::default();
        let Some(obj) = v.as_object() else {
            return out;
        };
        take(obj, "name", &mut out.name);
        take(obj, "enabled", &mut out.enabled);
        take(obj, "settings", &mut out.settings);
        take(obj, "setupStep", &mut out.setup_step);
        if let Some(peers) = obj.get("peers").and_then(Value::as_array) {
            for p in peers {
                match serde_json::from_value::<PeerRecord>(p.clone()) {
                    Ok(rec)
                        if super::ids::is_machine_id(&rec.id) && super::ids::is_key(&rec.key) =>
                    {
                        if !out.peers.iter().any(|q| q.id == rec.id || q.key == rec.key) {
                            out.peers.push(rec);
                        }
                    }
                    Ok(_) => {
                        tracing::warn!("machines.json: a peer with a malformed id was skipped")
                    }
                    Err(e) => tracing::warn!(error = %e, "machines.json: a peer was skipped"),
                }
            }
        }
        for (k, val) in obj {
            if !KNOWN.contains(&k.as_str()) {
                out.extra.insert(k.clone(), val.clone());
            }
        }
        out
    }

    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        crate::persist::write_json_private(&Self::path(data_dir), self)
    }

    pub fn peer(&self, id: &str) -> Option<&PeerRecord> {
        self.peers.iter().find(|p| p.id == id)
    }

    pub fn peer_mut(&mut self, id: &str) -> Option<&mut PeerRecord> {
        self.peers.iter_mut().find(|p| p.id == id)
    }

    pub fn peer_by_key(&self, key: &str) -> Option<&PeerRecord> {
        self.peers.iter().find(|p| p.key == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ID_A: &str = "0123456789abcdef0123456789abcdef";
    const KEY_A: &str = "aa00000000000000000000000000000000000000000000000000000000000001";

    fn peer_json() -> Value {
        json!({
            "id": ID_A, "name": "DESKTOP-PC", "os": "windows", "key": KEY_A,
            "pairedAt": 1, "futureField": {"x": 1}
        })
    }

    #[test]
    fn a_missing_file_is_empty_and_a_peer_keeps_unknown_fields() {
        let f = MachinesFile::from_value(&json!({ "peers": [peer_json()], "later": 3 }));
        assert_eq!(f.peers.len(), 1);
        let p = &f.peers[0];
        assert_eq!(
            p.grants,
            Grants::defaults(),
            "a missing grants field is the default"
        );
        assert_eq!(p.extra.get("futureField"), Some(&json!({"x": 1})));
        assert_eq!(f.extra.get("later"), Some(&json!(3)));
        let saved = serde_json::to_value(&f).unwrap();
        assert_eq!(saved["later"], 3);
        assert_eq!(saved["peers"][0]["futureField"]["x"], 1);
    }

    #[test]
    fn one_bad_field_or_peer_never_loses_the_rest() {
        let f = MachinesFile::from_value(&json!({
            "enabled": "yes please",
            "settings": {"relay": {"mode": "none"}},
            "peers": [peer_json(), {"id": "bad"}, {"id": "../x", "name": "x", "key": KEY_A, "pairedAt": 1}]
        }));
        assert!(!f.enabled);
        assert_eq!(f.settings.relay.mode, RelayMode::None);
        assert!(f.settings.keep_awake, "keepAwake defaults on");
        assert_eq!(f.peers.len(), 1);
    }

    #[test]
    fn a_duplicate_key_is_kept_once() {
        let mut second = peer_json();
        second["id"] = json!("fedcba9876543210fedcba9876543210");
        let f = MachinesFile::from_value(&json!({ "peers": [peer_json(), second] }));
        assert_eq!(f.peers.len(), 1);
    }

    #[test]
    fn a_custom_relay_needs_an_https_url() {
        let mut r = RelaySetting {
            mode: RelayMode::Custom,
            url: None,
        };
        assert!(r.validate().is_err());
        r.url = Some("http://relay.example.com".into());
        assert!(r.validate().is_err());
        r.url = Some("https://relay.example.com/".into());
        assert!(r.validate().is_ok());
        r.url = Some("http://localhost:3340".into());
        assert!(r.validate().is_ok());
        let same = RelaySetting {
            mode: RelayMode::Custom,
            url: Some("HTTP://LOCALHOST:3340".into()),
        };
        assert!(r.same_as(&same));
        assert!(RelaySetting::default().same_as(&RelaySetting {
            mode: RelayMode::Public,
            url: Some("ignored".into())
        }));
    }
}
