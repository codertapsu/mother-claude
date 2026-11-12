//! `machines.json` (0600, app data dir): this machine's chosen name, whether
//! Machines is on, the paired machines, the settings of D10 and the roots
//! (docs/PLAN-MACHINES.md §6). Read field by field, so one bad field (or
//! peer, or root) never loses the rest; fields from a newer version survive
//! a save.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::grants::Grants;
use super::protocol::Versions;
use super::roots::Root;

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

/// A field read on its own: a value this build cannot read is its default,
/// so one bad field never loses the rest (`machines.json` comes from older
/// and newer builds alike).
fn lenient<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    let v = Value::deserialize(d)?;
    Ok(serde_json::from_value(v).unwrap_or_default())
}

/// How a session's code travels when it moves (D10, plan §5.10).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CodeTransfer {
    /// Machine to machine: a git bundle over the peer link (the default).
    /// `direct` is the plan's first name for it (§6), read as this.
    #[default]
    #[serde(alias = "direct")]
    Bundle,
    /// Through the project's git remote: hidden refs pushed there.
    Remote,
}

impl CodeTransfer {
    pub fn as_str(self) -> &'static str {
        match self {
            CodeTransfer::Bundle => "bundle",
            CodeTransfer::Remote => "remote",
        }
    }
}

/// Where a moved session's code lands on the target (D10).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CodeApply {
    /// The project's checkout when it has no changes of its own (the
    /// default).
    #[default]
    Checkout,
    /// Always a separate worktree; the checkout is never touched.
    Worktree,
}

impl CodeApply {
    pub fn as_str(self) -> &'static str {
        match self {
            CodeApply::Checkout => "checkout",
            CodeApply::Worktree => "worktree",
        }
    }
}

/// The code options of D10, the same on every machine (offered to the
/// others like the relay): how code travels, where it lands, and whether a
/// move's hidden refs stay on the remote. Each field is read on its own.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeSettings {
    #[serde(default, deserialize_with = "lenient")]
    pub transfer: CodeTransfer,
    #[serde(default, deserialize_with = "lenient")]
    pub apply: CodeApply,
    /// Keep a move's hidden refs on the remote after the target fetched
    /// them (off: they are deleted there, kept locally for undo).
    #[serde(default, deserialize_with = "lenient")]
    pub keep_remote_refs: bool,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl CodeSettings {
    /// The same choices (fields a newer build added do not count).
    pub fn same_as(&self, other: &CodeSettings) -> bool {
        self.transfer == other.transfer
            && self.apply == other.apply
            && self.keep_remote_refs == other.keep_remote_refs
    }
}

/// A project's own code options; a field left out follows the settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeOverride {
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub transfer: Option<CodeTransfer>,
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub apply: Option<CodeApply>,
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub keep_remote_refs: Option<bool>,
}

impl CodeOverride {
    pub fn is_empty(&self) -> bool {
        self.transfer.is_none() && self.apply.is_none() && self.keep_remote_refs.is_none()
    }

    /// `base` with this project's choices over it.
    pub fn over(&self, base: &CodeSettings) -> CodeSettings {
        CodeSettings {
            transfer: self.transfer.unwrap_or(base.transfer),
            apply: self.apply.unwrap_or(base.apply),
            keep_remote_refs: self.keep_remote_refs.unwrap_or(base.keep_remote_refs),
            extra: base.extra.clone(),
        }
    }
}

/// What to run after a move, per system (plan §5.10, decision 8: it joins
/// the move note as a request to Claude, never run by Mother Claude).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AfterMove {
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub macos: Vec<String>,
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub windows: Vec<String>,
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub linux: Vec<String>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl AfterMove {
    pub fn is_empty(&self) -> bool {
        self.macos.is_empty() && self.windows.is_empty() && self.linux.is_empty()
    }

    /// The commands for a system as machines name it (`macos`, `windows`,
    /// `linux`).
    pub fn for_os(&self, os: &str) -> &[String] {
        match os {
            "macos" => &self.macos,
            "windows" => &self.windows,
            "linux" => &self.linux,
            _ => &[],
        }
    }
}

/// A project's recipe: what to run after a move, and the files git ignores
/// that still travel with it (opt-in; setting them is a dangerous act).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recipe {
    #[serde(default, deserialize_with = "lenient")]
    pub after_move: AfterMove,
    /// Paths relative to the project, `/`-separated, that a move carries
    /// although git ignores them (`.env.local`).
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub carry_ignored: Vec<String>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl Recipe {
    pub fn is_empty(&self) -> bool {
        self.after_move.is_empty() && self.carry_ignored.is_empty()
    }
}

/// One project's Machines settings (*Project settings › Machines*), kept
/// under a root-relative key (`rt_…/app`) when a root holds the project,
/// else its absolute path here (`setup::options`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSettings {
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "CodeOverride::is_empty"
    )]
    pub code: CodeOverride,
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub recipe: Option<Recipe>,
    /// The computers its sessions keep a copy on (docs/PLAN-MACHINES.md
    /// §5.9): their ids; a session's own choice wins.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub copies: Vec<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub updated_at: i64,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl ProjectSettings {
    pub fn is_empty(&self) -> bool {
        self.code.is_empty()
            && self.recipe.as_ref().is_none_or(Recipe::is_empty)
            && self.copies.is_empty()
    }
}

/// The settings of D10. Unknown fields survive a save.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub relay: RelaySetting,
    /// Hold a keep-awake assertion while another machine uses this one.
    #[serde(default = "yes")]
    pub keep_awake: bool,
    /// How code travels and where it lands (the defaults of every move).
    #[serde(default, deserialize_with = "lenient")]
    pub code: CodeSettings,
    /// What this computer keeps at most of the copies of each other
    /// computer's sessions (docs/PLAN-MACHINES.md §5.9): the copy refreshed
    /// longest ago goes first. This computer's own: not offered.
    #[serde(default = "copies_cap", deserialize_with = "cap_or_default")]
    pub copies_cap_bytes: u64,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

fn yes() -> bool {
    true
}

fn copies_cap() -> u64 {
    super::copies::DEFAULT_CAP
}

/// A cap read on its own: one out of bounds, or not a number, is the
/// default.
fn cap_or_default<'de, D>(d: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Value::deserialize(d)?;
    Ok(v.as_u64()
        .filter(|n| (super::copies::MIN_CAP..=super::copies::MAX_CAP).contains(n))
        .unwrap_or_else(copies_cap))
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            relay: RelaySetting::default(),
            keep_awake: true,
            code: CodeSettings::default(),
            copies_cap_bytes: copies_cap(),
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
    /// The shell a run uses there, as it last said (Phase 4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    /// The computers it is paired with (their ids), as its last hello said
    /// (plan §5.12: what an introduction is offered for).
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub paired_with: Vec<String>,
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
    /// Roots (§5.12): a named folder with one path per machine, replicated
    /// to the paired machines; a removed one stays as a tombstone
    /// (`roots.rs`). Sorted by id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<Root>,
    /// The folder pairs already imported as roots (their ids): none is
    /// imported twice.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots_imported: Vec<String>,
    /// Each project's own Machines settings (code options, recipe), by
    /// `rt_…/<path in the root>` or the project's absolute path here
    /// (`setup::options`). This machine's own: not replicated.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub projects: BTreeMap<String, ProjectSettings>,
    /// *Revoke everywhere* (Phase 5, `revocation`): each computer revoked
    /// here and on the others, with the paired computers still to be told —
    /// kept until each was, so an offline one applies it when it is back.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub revocations: Vec<super::revocation::Revocation>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

const KNOWN: [&str; 9] = [
    "name",
    "enabled",
    "peers",
    "settings",
    "setupStep",
    "roots",
    "rootsImported",
    "projects",
    "revocations",
];

/// A key `projects` may hold: 1 to 1100 characters, no control character.
pub fn is_project_key(k: &str) -> bool {
    !k.trim().is_empty() && k.chars().count() <= 1100 && !k.chars().any(char::is_control)
}

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
        if let Some(roots) = obj.get("roots").and_then(Value::as_array) {
            let mut kept = Vec::new();
            for r in roots {
                match serde_json::from_value::<Root>(r.clone()) {
                    Ok(root) if root.well_formed() => kept.push(root),
                    Ok(_) => tracing::warn!("machines.json: a malformed root was skipped"),
                    Err(e) => tracing::warn!(error = %e, "machines.json: a root was skipped"),
                }
            }
            // One copy per id, sorted, as a merge keeps them.
            super::roots::merge(&mut out.roots, &kept);
        }
        take(obj, "rootsImported", &mut out.roots_imported);
        out.roots_imported
            .retain(|id| crate::claude::backup::pairs::is_pair_id(id));
        if let Some(projects) = obj.get("projects").and_then(Value::as_object) {
            for (key, v) in projects {
                match serde_json::from_value::<ProjectSettings>(v.clone()) {
                    Ok(p) if is_project_key(key) => {
                        out.projects.insert(key.clone(), p);
                    }
                    Ok(_) => tracing::warn!("machines.json: a project with a bad key was skipped"),
                    Err(e) => tracing::warn!(error = %e, "machines.json: a project was skipped"),
                }
            }
        }
        if let Some(list) = obj.get("revocations").and_then(Value::as_array) {
            for r in list {
                match serde_json::from_value::<super::revocation::Revocation>(r.clone()) {
                    Ok(rev) if rev.well_formed() => out.revocations.push(rev),
                    Ok(_) => tracing::warn!("machines.json: a malformed revocation was skipped"),
                    Err(e) => tracing::warn!(error = %e, "machines.json: a revocation was skipped"),
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
    fn roots_are_read_one_by_one_and_saved_once() {
        let good = json!({
            "id": "rt_1111111111111111", "name": "projects",
            "paths": { ID_A: "/Users/m/dev" }, "updatedAt": 5, "by": ID_A
        });
        let f = MachinesFile::from_value(&json!({
            "roots": [
                good,
                {"id": "rt_bad"},
                {"id": "rt_2222222222222222", "name": "x", "paths": {"nope": "/a"}, "updatedAt": 1}
            ],
            "rootsImported": ["fp_1111111111111111", "junk"],
        }));
        assert_eq!(
            f.roots.len(),
            1,
            "a malformed root is skipped, the rest kept"
        );
        assert_eq!(f.roots_imported, ["fp_1111111111111111"]);
        assert!(f.extra.is_empty(), "known fields are not kept twice");
        let saved = serde_json::to_value(&f).unwrap();
        assert_eq!(saved["roots"][0]["name"], "projects");
        assert_eq!(MachinesFile::from_value(&saved), f);
        let empty = serde_json::to_value(MachinesFile::default()).unwrap();
        assert!(empty.get("roots").is_none() && empty.get("rootsImported").is_none());
    }

    #[test]
    fn the_code_options_default_and_read_field_by_field() {
        let f = MachinesFile::from_value(&json!({ "settings": {"relay": {"mode": "none"}} }));
        assert_eq!(f.settings.code, CodeSettings::default());
        assert_eq!(f.settings.code.transfer, CodeTransfer::Bundle);
        assert_eq!(f.settings.code.apply, CodeApply::Checkout);
        assert!(!f.settings.code.keep_remote_refs);

        // `direct` (the plan's first name) is a bundle; a bad field is its
        // default and never loses the relay or the other fields.
        let f = MachinesFile::from_value(&json!({ "settings": {
            "relay": {"mode": "none"},
            "code": {"transfer": "direct", "apply": "teleport", "keepRemoteRefs": true, "later": 1}
        }}));
        assert_eq!(f.settings.relay.mode, RelayMode::None);
        assert_eq!(f.settings.code.transfer, CodeTransfer::Bundle);
        assert_eq!(f.settings.code.apply, CodeApply::Checkout, "unknown value");
        assert!(f.settings.code.keep_remote_refs);
        let saved = serde_json::to_value(&f).unwrap();
        assert_eq!(saved["settings"]["code"]["transfer"], "bundle");
        assert_eq!(
            saved["settings"]["code"]["later"], 1,
            "a newer field survives"
        );

        let f = MachinesFile::from_value(&json!({ "settings": {"code": "remote"} }));
        assert_eq!(f.settings.code, CodeSettings::default(), "not an object");
        let f = MachinesFile::from_value(
            &json!({ "settings": {"code": {"transfer": "remote", "apply": "worktree"}} }),
        );
        assert_eq!(f.settings.code.transfer, CodeTransfer::Remote);
        assert_eq!(f.settings.code.apply, CodeApply::Worktree);
    }

    #[test]
    fn projects_are_read_one_by_one() {
        let f = MachinesFile::from_value(&json!({ "projects": {
            "rt_1111111111111111/app": {
                "code": {"transfer": "remote", "apply": "nope"},
                "recipe": {"afterMove": {"windows": ["npm ci"], "macos": "npm ci"}, "carryIgnored": [".env.local"]},
                "updatedAt": 5
            },
            "/Users/m/dev/x": {"code": {"apply": "worktree"}},
            "bad\u{0007}key": {"code": {"apply": "worktree"}},
            "/Users/m/dev/y": "not an object"
        }}));
        assert_eq!(f.projects.len(), 2, "{:?}", f.projects.keys());
        let app = &f.projects["rt_1111111111111111/app"];
        assert_eq!(app.code.transfer, Some(CodeTransfer::Remote));
        assert_eq!(app.code.apply, None, "an unknown value is left out");
        let recipe = app.recipe.as_ref().unwrap();
        assert_eq!(recipe.after_move.windows, ["npm ci"]);
        assert!(recipe.after_move.macos.is_empty(), "not a list");
        assert_eq!(recipe.carry_ignored, [".env.local"]);
        assert_eq!(
            f.projects["/Users/m/dev/x"]
                .code
                .over(&CodeSettings::default())
                .apply,
            CodeApply::Worktree
        );
        let saved = serde_json::to_value(&f).unwrap();
        assert_eq!(MachinesFile::from_value(&saved), f);
        assert!(serde_json::to_value(MachinesFile::default())
            .unwrap()
            .get("projects")
            .is_none());
    }

    #[test]
    fn the_copies_cap_defaults_and_stays_within_its_bounds() {
        let two_gib = 2 * 1024 * 1024 * 1024;
        let f = MachinesFile::from_value(&json!({ "settings": {"relay": {"mode": "none"}} }));
        assert_eq!(f.settings.copies_cap_bytes, two_gib);
        for bad in [json!(5), json!("big"), json!(-1), json!(u64::MAX)] {
            let f = MachinesFile::from_value(
                &json!({ "settings": {"relay": {"mode": "none"}, "copiesCapBytes": bad}}),
            );
            assert_eq!(f.settings.copies_cap_bytes, two_gib, "{bad}");
            assert_eq!(f.settings.relay.mode, RelayMode::None, "the rest is kept");
        }
        let f =
            MachinesFile::from_value(&json!({ "settings": {"copiesCapBytes": 500_000_000u64} }));
        assert_eq!(f.settings.copies_cap_bytes, 500_000_000);
        let saved = serde_json::to_value(&f).unwrap();
        assert_eq!(saved["settings"]["copiesCapBytes"], 500_000_000);
        // A project's copies, read and saved with it.
        let f = MachinesFile::from_value(&json!({ "projects": {
            "/Users/m/dev/x": {"copies": [ID_A], "code": {"apply": "worktree"}},
            "/Users/m/dev/y": {"copies": "nope"},
        }}));
        assert_eq!(f.projects["/Users/m/dev/x"].copies, [ID_A]);
        assert!(f.projects["/Users/m/dev/y"].copies.is_empty());
        assert!(!f.projects["/Users/m/dev/x"].is_empty());
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
