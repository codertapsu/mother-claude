//! What copies keep on disk (all in the app data dir, 0600, written whole
//! and atomically, read entry by entry so one bad entry never loses the
//! rest; fields from a newer build survive a save):
//!
//! - `machines/copies.json` — this computer's side: which computers each
//!   session keeps a copy on (its own choice; a project's is in its Machines
//!   options) and how each of those copies stands (copied at, paused: why);
//! - `machines/copies/index.json` — the copies held here: per copy its
//!   source computer, account, folder there, sizes, its transcript's mark,
//!   the last turn's time;
//! - `<copy>/.mc/files.json` — what a copy holds of each file (its size, the
//!   source's time of it, its mark or its SHA-256).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::check::Mark;
use crate::machines::moves::wire::AccountAsk;
use crate::machines::moves::Carried;

/// `machines/copies.json`.
pub const OUT_FILE: &str = "copies.json";
/// `machines/copies/index.json`.
pub const INDEX_FILE: &str = "index.json";
/// A copy's own record of its files.
pub const TABLE_FILE: &str = "files.json";
/// The format of these files.
pub const VERSION: u32 = 1;

// ─── this computer's side ──────────────────────────────────────────────────────

/// Where one copy this computer keeps elsewhere stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    /// Marked, not copied yet.
    #[default]
    Pending,
    /// A turn runs, or the session has not been quiet long enough.
    Waiting,
    /// A round runs now.
    Copying,
    /// The copy there is as the session was after its last turn.
    Copied,
    /// Something stops it (said in `reason` and `detail`).
    Paused,
}

/// Why a copy waits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    /// The other computer can't be reached.
    Offline,
    /// Machines is off on this computer or the other.
    MachinesOff,
    /// The other computer has not given this one its `move` grant.
    NoGrant,
    /// The other computer's Mother Claude can't keep copies yet.
    Unsupported,
    /// The session lives on another computer now.
    LivesElsewhere,
    /// It is larger than the other computer keeps for this one.
    TooLarge,
    /// Its files changed while they were read: it goes again once quiet.
    Changed,
    /// Something else (said in `detail`).
    Error,
}

/// One copy's state, as screens show it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PairStatus {
    pub state: State,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Reason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Since when it is in this state (ms).
    pub since: i64,
    /// When the copy there was last made current (ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copied_at: Option<i64>,
    /// The time of the turn it holds (the transcript's last write, ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_at: Option<i64>,
    /// What the copy there holds.
    pub bytes: u64,
    pub files: usize,
}

/// A session's own choice of computers (it wins over its project's).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Kept {
    pub machines: Vec<String>,
    pub updated_at: i64,
}

/// `machines/copies.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutFile {
    pub version: u32,
    /// Session → its own choice.
    pub sessions: BTreeMap<String, Kept>,
    /// Session → computer → how its copy there stands.
    pub status: BTreeMap<String, BTreeMap<String, PairStatus>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl OutFile {
    fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("machines").join(OUT_FILE)
    }

    pub fn load(data_dir: Option<&Path>) -> Self {
        data_dir
            .and_then(|d| crate::persist::read_json::<Value>(&Self::path(d)))
            .map(|v| Self::from_value(&v))
            .unwrap_or(OutFile {
                version: VERSION,
                ..OutFile::default()
            })
    }

    /// Read entry by entry: a bad one is skipped (with a warning).
    pub fn from_value(v: &Value) -> Self {
        let mut out = OutFile {
            version: VERSION,
            ..OutFile::default()
        };
        let Some(obj) = v.as_object() else {
            return out;
        };
        let machine = crate::machines::ids::is_machine_id;
        let session = crate::claude::ids::is_session_id;
        for (sid, k) in obj
            .get("sessions")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            match serde_json::from_value::<Kept>(k.clone()) {
                Ok(mut kept) if session(sid) => {
                    kept.machines.retain(|m| machine(m));
                    kept.machines.dedup();
                    out.sessions.insert(sid.clone(), kept);
                }
                _ => tracing::warn!("machines/copies.json: a session's choice was skipped"),
            }
        }
        for (sid, per) in obj
            .get("status")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            if !session(sid) {
                continue;
            }
            let mut kept = BTreeMap::new();
            for (m, s) in per.as_object().into_iter().flatten() {
                if let (true, Ok(s)) = (machine(m), serde_json::from_value::<PairStatus>(s.clone()))
                {
                    kept.insert(m.clone(), s);
                }
            }
            if !kept.is_empty() {
                out.status.insert(sid.clone(), kept);
            }
        }
        for (k, val) in obj {
            if !matches!(k.as_str(), "version" | "sessions" | "status") {
                out.extra.insert(k.clone(), val.clone());
            }
        }
        out
    }

    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        crate::persist::write_json_private(&Self::path(data_dir), self)
    }

    /// How the copy of `session` on `machine` stands.
    pub fn status_of(&self, session: &str, machine: &str) -> Option<&PairStatus> {
        self.status.get(session).and_then(|m| m.get(machine))
    }
}

// ─── the copies held here ──────────────────────────────────────────────────────

/// One copy held here.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Held {
    /// The computer it is a copy of (its home, as it was copied).
    pub machine: String,
    pub session: String,
    /// Its account there, by name (matched here as a move matches it).
    pub account: AccountAsk,
    /// Its account's id there: the copy's folder.
    pub account_dir: String,
    pub title: Option<String>,
    /// Where it works there.
    pub cwd: String,
    /// The settings it ran with there.
    pub settings: Carried,
    /// What the copy holds.
    pub bytes: u64,
    pub files: usize,
    /// Its transcript's acknowledged size and check.
    pub transcript: Option<Mark>,
    /// The time of the turn it holds (ms, there).
    pub turn_at: i64,
    /// When it was last made current here (ms).
    pub copied_at: i64,
    pub created_at: i64,
    /// The last round went through whole (else part of a turn may be
    /// missing until the next one).
    pub complete: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Held {
    fn well_formed(&self) -> bool {
        crate::machines::ids::is_machine_id(&self.machine)
            && crate::claude::ids::is_session_id(&self.session)
            && super::is_account_dir(&self.account_dir)
            && self.transcript.as_ref().is_none_or(Mark::well_formed)
    }
}

/// `machines/copies/index.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeldIndex {
    pub version: u32,
    /// `<source machine>/<session>` → the copy.
    pub copies: BTreeMap<String, Held>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl HeldIndex {
    fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("machines").join("copies").join(INDEX_FILE)
    }

    pub fn load(data_dir: Option<&Path>) -> Self {
        data_dir
            .and_then(|d| crate::persist::read_json::<Value>(&Self::path(d)))
            .map(|v| Self::from_value(&v))
            .unwrap_or(HeldIndex {
                version: VERSION,
                ..HeldIndex::default()
            })
    }

    pub fn from_value(v: &Value) -> Self {
        let mut out = HeldIndex {
            version: VERSION,
            ..HeldIndex::default()
        };
        let Some(obj) = v.as_object() else {
            return out;
        };
        for (_, c) in obj
            .get("copies")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            match serde_json::from_value::<Held>(c.clone()) {
                Ok(h) if h.well_formed() => {
                    out.copies.insert(super::key(&h.machine, &h.session), h);
                }
                _ => tracing::warn!("machines/copies/index.json: a copy was skipped"),
            }
        }
        for (k, val) in obj {
            if !matches!(k.as_str(), "version" | "copies") {
                out.extra.insert(k.clone(), val.clone());
            }
        }
        out
    }

    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        let path = Self::path(data_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            crate::claude::transfer::private_dir(parent);
        }
        crate::persist::write_json_private(&path, self)
    }

    /// The bytes the copies of `machine` hold here.
    pub fn bytes_of(&self, machine: &str) -> u64 {
        self.copies
            .values()
            .filter(|h| h.machine == machine)
            .map(|h| h.bytes)
            .sum()
    }
}

// ─── what a copy holds of each file ────────────────────────────────────────────

/// What a copy holds of one file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FileState {
    pub size: u64,
    /// The source's time of the file when it was copied (ms).
    pub mtime: i64,
    /// An append-only file's mark at its size.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mark: Option<Mark>,
    /// A whole file's SHA-256, as it arrived.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

/// A copy's files: path (`/`-separated, under the copy) → what it holds.
pub type Table = BTreeMap<String, FileState>;

fn table_path(copy_dir: &Path) -> PathBuf {
    copy_dir.join(".mc").join(TABLE_FILE)
}

/// A copy's table (empty when it has none, or it can't be read: the next
/// round then sends every file whole).
pub fn load_table(copy_dir: &Path, session: &str) -> Table {
    let Some(v) = crate::persist::read_json::<Value>(&table_path(copy_dir)) else {
        return Table::new();
    };
    let mut out = Table::new();
    for (path, f) in v
        .get("files")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        match serde_json::from_value::<FileState>(f.clone()) {
            Ok(s) if super::allowed_path(path, session) => {
                out.insert(path.clone(), s);
            }
            _ => tracing::warn!("a copy's file record was skipped"),
        }
    }
    out
}

pub fn save_table(copy_dir: &Path, table: &Table) -> std::io::Result<()> {
    let path = table_path(copy_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        crate::claude::transfer::private_dir(parent);
    }
    crate::persist::write_json_private(
        &path,
        &serde_json::json!({ "version": VERSION, "files": table }),
    )
}

// ─── the cap ───────────────────────────────────────────────────────────────────

/// A copy the cap may take away.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub key: String,
    pub bytes: u64,
    /// When it was last made current (the oldest goes first).
    pub copied_at: i64,
    /// Its session's lease names this computer: never taken away.
    pub protected: bool,
}

/// The copies to take away so that one source computer's copies fit in
/// `cap`: the one refreshed longest ago first, never a protected one, never
/// `keep` (the copy just written). What still does not fit stays (the
/// protected ones and `keep` are never lost to the cap).
pub fn to_evict(copies: &[Candidate], cap: u64, keep: &str) -> Vec<String> {
    let mut total: u64 = copies.iter().map(|c| c.bytes).sum();
    if total <= cap {
        return Vec::new();
    }
    let mut order: Vec<&Candidate> = copies
        .iter()
        .filter(|c| !c.protected && c.key != keep)
        .collect();
    order.sort_by(|a, b| a.copied_at.cmp(&b.copied_at).then(a.key.cmp(&b.key)));
    let mut out = Vec::new();
    for c in order {
        if total <= cap {
            break;
        }
        total = total.saturating_sub(c.bytes);
        out.push(c.key.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const M: &str = "0123456789abcdef0123456789abcdef";
    const S1: &str = "0f1e2d3c-4b5a-4968-8776-655443322110";
    const S2: &str = "1f1e2d3c-4b5a-4968-8776-655443322110";

    fn cand(key: &str, bytes: u64, at: i64, protected: bool) -> Candidate {
        Candidate {
            key: key.into(),
            bytes,
            copied_at: at,
            protected,
        }
    }

    #[test]
    fn the_oldest_copy_goes_first_and_a_protected_one_never() {
        let copies = [
            cand("a", 400, 30, false),
            cand("b", 400, 10, false),
            cand("c", 400, 20, false),
            cand("new", 400, 40, false),
        ];
        // 1600 bytes, 1000 allowed: the two refreshed longest ago go.
        assert_eq!(to_evict(&copies, 1000, "new"), ["b", "c"]);
        // Everything fits: nothing goes.
        assert!(to_evict(&copies, 1600, "new").is_empty());
        // The oldest is a session that lives here now: kept; the next goes.
        let copies = [
            cand("a", 400, 30, false),
            cand("b", 400, 10, true),
            cand("c", 400, 20, false),
            cand("new", 400, 40, false),
        ];
        assert_eq!(to_evict(&copies, 1000, "new"), ["c", "a"]);
        // Only protected ones and the new one are left: over the cap, kept.
        let copies = [cand("b", 900, 10, true), cand("new", 900, 40, false)];
        assert!(to_evict(&copies, 1000, "new").is_empty());
        // Ties go by key, so every computer decides alike.
        let copies = [
            cand("y", 500, 10, false),
            cand("x", 500, 10, false),
            cand("new", 500, 40, false),
        ];
        assert_eq!(to_evict(&copies, 1000, "new"), ["x"]);
    }

    #[test]
    fn the_files_are_read_entry_by_entry() {
        let out = OutFile::from_value(&json!({
            "version": 1,
            "sessions": {
                S1: { "machines": [M, "nope", M], "updatedAt": 3 },
                "bad": { "machines": [M] },
            },
            "status": {
                S1: { M: { "state": "copied", "copiedAt": 5, "bytes": 10, "files": 2 },
                      "short": { "state": "copied" } },
                S2: { M: { "state": "teleported" } },
            },
            "later": true,
        }));
        assert_eq!(out.sessions.len(), 1);
        assert_eq!(out.sessions[S1].machines, [M]);
        let s = out.status_of(S1, M).unwrap();
        assert_eq!(s.state, State::Copied);
        assert_eq!(s.copied_at, Some(5));
        assert!(
            out.status_of(S2, M).is_none(),
            "an unknown state is skipped"
        );
        let saved = serde_json::to_value(&out).unwrap();
        assert_eq!(saved["later"], true);
        assert_eq!(OutFile::from_value(&saved), out);

        let idx = HeldIndex::from_value(&json!({
            "copies": {
                "x": { "machine": M, "session": S1, "accountDir": "default", "bytes": 7,
                       "account": { "name": "Default", "isDefault": true } },
                "y": { "machine": M, "session": S2, "accountDir": "../up" },
                "z": { "machine": "nope", "session": S2, "accountDir": "default" },
            }
        }));
        assert_eq!(idx.copies.len(), 1);
        let h = &idx.copies[&super::super::key(M, S1)];
        assert!(h.account.is_default);
        assert_eq!(idx.bytes_of(M), 7);
        assert_eq!(idx.bytes_of(&"f".repeat(32)), 0);
    }

    #[test]
    fn a_copys_table_keeps_only_the_sessions_own_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = Table::new();
        let good = format!("projects/-x/{S1}.jsonl");
        t.insert(
            good.clone(),
            FileState {
                size: 3,
                mtime: 1,
                mark: None,
                sha256: Some("a".repeat(64)),
            },
        );
        save_table(dir.path(), &t).unwrap();
        // A record naming another file is dropped on reading.
        let path = table_path(dir.path());
        let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        v["files"]["settings.json"] = json!({ "size": 1 });
        std::fs::write(&path, v.to_string()).unwrap();
        let back = load_table(dir.path(), S1);
        assert_eq!(back.len(), 1);
        assert_eq!(back[&good].size, 3);
        assert!(load_table(&dir.path().join("none"), S1).is_empty());
    }
}
