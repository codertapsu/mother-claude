//! Code sync (docs/PLAN-MACHINES.md §5.15): a project's code kept the same
//! on two computers, both ways — off by default, turned on per project.
//!
//! **The pair.** This computer's checkout (a repository's top) and the
//! folder a root (§5.12) maps it to on one paired computer, under one id
//! (`sy_` and 16 hex characters) both computers use: the "session" the
//! code machinery keys its snapshot refs on. A project syncs with one
//! other computer at a time.
//!
//! **On only when both said so** (the asked-actions rule): the desktop
//! here turns it on (`PUT /api/machines/sync`), which asks the other
//! computer (`/peer/setup/ask`, an ask of kind `sync`); its desktop
//! confirms there (`setup::asks`), and only then is it on, on both. Both
//! folders must be repositories of the same project (the same `origin`, or
//! a commit in common) and each computer must have given the other its
//! `move` grant (sync writes into its checkout). Turning it off is
//! immediate here and tells the other computer.
//!
//! **One step** (`step`): both computers keep the **last synced state**
//! (HEAD, branch, staged tree, working tree, the step's snapshot). The
//! computer whose checkout changed asks the other for its state; when only
//! it changed it sends a snapshot (a bundle without what the other has,
//! chained on the last step's) and the other lands it in its checkout —
//! parked at the last synced state, so clean by definition (F7) — and both
//! record the new state. A change there in between makes the landing stop:
//! a conflict, never an overwrite. Identical changes on both sides are no
//! conflict. Files git ignores stay put but for the project's
//! `carryIgnored` (the other computer's `dangerous` grant).
//!
//! **A conflict** (both changed) pauses the pair on both computers until
//! the desktop of either chooses which side to keep: the other side's
//! state is first kept as one commit under `refs/mother-claude/sync/<id>/
//! kept-<time>`, its checkout put back to the last synced state
//! (`code::keep_and_put_back`, refused when anything could be lost), then
//! the winner's lands as an ordinary step. Nothing is ever merged.
//!
//! **It waits, never queues**: Machines off on either computer, the other
//! computer offline, a grant missing, or a move of a session working in
//! the project under way pauses the pair and says why; the next check
//! after that ends starts from the checkouts as they are then.
//!
//! **When** (`watch`): a file watcher per pair notices a change (the
//! working tree, `.git/HEAD`, `.git/refs`, `.git/index` — not object
//! churn, not ignored paths) and a few quiet seconds later this computer
//! syncs; a check every minute is the safety net; *Sync now* runs one at
//! once. The app turns the watchers on (`MachinesState::enable_sync_watch`);
//! tests drive steps through *Sync now*.
//!
//! **Kept** in `<app data>/machines/sync.json` (0600, written whole), read
//! pair by pair: a pair this build cannot read is skipped, never the file.
//! Every step, conflict and resolution is audited on both computers (the
//! project's folder name and counts, never a file's content), and every
//! change of a pair is a `{t:"machines", what:"sync"}` frame.

pub mod on;
pub mod peer;
pub mod routes;
pub mod step;
pub mod watch;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::code::{Changes, TreeState};
use super::grants::Grant;
use crate::claude::paths;
use crate::server::devices::audit::AuditEntry;
use crate::state::AppState;

/// The file the pairs are kept in, under `<app data>/machines`.
pub const FILE: &str = "sync.json";
/// Its format.
pub const VERSION: u32 = 1;
/// Pairs kept at most.
pub const MAX_PAIRS: usize = 200;
/// Kept refs listed per pair at most (the newest).
const MAX_KEPT: usize = 20;
/// How many step snapshots each side keeps refs for (what the next bundle
/// chains on).
pub const KEEP_STEPS: usize = 3;
/// Every this many steps one goes unchained, so old steps' commits can go.
pub const CHAIN_EVERY: u64 = 32;

// ─── the pair ──────────────────────────────────────────────────────────────────

/// `sy_` and 16 lowercase hex characters (it is a ref's name part, and the
/// "session" of the code machinery).
pub fn is_sync_id(s: &str) -> bool {
    s.strip_prefix("sy_")
        .is_some_and(|h| h.len() == 16 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

pub fn new_sync_id() -> String {
    let hex: String = crate::server::auth::random_bytes(8)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("sy_{hex}")
}

/// Which way a step went, from this computer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    /// This computer's changes went there.
    #[default]
    Out,
    /// The other computer's came here.
    In,
    /// Both had made the same changes: nothing travelled.
    Both,
}

/// The last synced state, and the step that made it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Last {
    #[serde(flatten)]
    pub state: TreeState,
    /// The step's snapshot (its commit), when one travelled.
    #[serde(default)]
    pub snapshot: Option<String>,
    #[serde(default)]
    pub step: u64,
    #[serde(default)]
    pub at: i64,
    #[serde(default)]
    pub direction: Direction,
    /// Files of the working tree the step changed.
    #[serde(default)]
    pub files: usize,
}

/// Why a pair waits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    /// Both sides changed: someone chooses which to keep.
    Conflict,
    /// The other computer can't be reached.
    Offline,
    /// Machines is off on this computer or the other.
    MachinesOff,
    /// A `move` grant is missing one way or the other.
    NoGrant,
    /// A session working in the project is moving between computers.
    MoveRunning,
    /// Something else went wrong (said in `detail`).
    Error,
}

/// Where a pair stands (`status` of the view).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Status {
    /// Turned off on the other computer, or not wanted there: nothing syncs.
    Off {
        #[serde(default)]
        detail: Option<String>,
    },
    /// Waiting for the other computer's desktop to confirm.
    #[default]
    Waiting,
    /// Both checkouts hold the last synced state (as of `at`).
    InSync { at: i64 },
    /// A step runs.
    Syncing { since: i64 },
    /// Waiting for something, said in words.
    Paused {
        reason: Reason,
        detail: String,
        since: i64,
    },
}

impl Status {
    pub fn paused(reason: Reason, detail: impl Into<String>) -> Self {
        Status::Paused {
            reason,
            detail: detail.into(),
            since: super::now(),
        }
    }

    pub fn reason(&self) -> Option<Reason> {
        match self {
            Status::Paused { reason, .. } => Some(*reason),
            _ => None,
        }
    }

    /// The same state for a screen (a new `since` alone is no change).
    fn same_as(&self, other: &Status) -> bool {
        match (self, other) {
            (Status::Syncing { .. }, Status::Syncing { .. }) => true,
            (
                Status::Paused {
                    reason: a,
                    detail: x,
                    ..
                },
                Status::Paused {
                    reason: b,
                    detail: y,
                    ..
                },
            ) => a == b && x == y,
            _ => self == other,
        }
    }
}

/// Both sides of a conflict, as each computer saw its own.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub at: i64,
    pub here: Changes,
    pub there: Changes,
    /// The first sync: neither side has a synced state yet.
    #[serde(default)]
    pub first: bool,
}

/// A side's state kept by a resolution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeptRef {
    /// `refs/mother-claude/sync/<id>/kept-<time>`.
    #[serde(rename = "ref")]
    pub ref_name: String,
    /// On this computer (`here`) or the other (`there`).
    pub side: String,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub files: usize,
    pub at: i64,
}

/// One synced project, as this computer keeps it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pair {
    pub id: String,
    /// The other computer (its machine id).
    pub machine: String,
    /// The checkout here (a repository's top).
    pub folder: String,
    /// Its folder on the other computer.
    pub there: String,
    /// This computer turned it on.
    #[serde(default)]
    pub here_on: bool,
    /// The other computer did (its desktop confirmed, or it asked).
    #[serde(default)]
    pub peer_on: bool,
    /// When this computer asked the other (none: the other asked).
    #[serde(default)]
    pub asked_at: Option<i64>,
    #[serde(default)]
    pub last: Option<Last>,
    #[serde(default)]
    pub status: Status,
    #[serde(default)]
    pub conflict: Option<Conflict>,
    /// States the resolutions kept, newest last.
    #[serde(default)]
    pub kept: Vec<KeptRef>,
    /// This side lost a resolution: the next landing may reset its branch
    /// (its own commits are kept).
    #[serde(default)]
    pub reset_ok: bool,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl Pair {
    /// Both computers turned it on.
    pub fn active(&self) -> bool {
        self.here_on && self.peer_on
    }

    /// Checks a pair read from disk.
    fn well_formed(&self) -> bool {
        is_sync_id(&self.id)
            && super::ids::is_machine_id(&self.machine)
            && paths::is_absolute_anywhere(&self.folder)
            && !self.there.trim().is_empty()
            && self.there.len() <= 4096
            && self
                .last
                .as_ref()
                .is_none_or(|l| l.state.validate().is_ok())
    }
}

/// `machines/sync.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncFile {
    pub version: u32,
    pub pairs: Vec<Pair>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl SyncFile {
    fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("machines").join(FILE)
    }

    /// Reads it pair by pair: a pair this build cannot read, or a malformed
    /// one, is skipped (with a warning), never the rest.
    pub fn from_value(v: &Value) -> Self {
        let mut out = SyncFile {
            version: VERSION,
            ..SyncFile::default()
        };
        let Some(obj) = v.as_object() else {
            return out;
        };
        for p in obj
            .get("pairs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            match serde_json::from_value::<Pair>(p.clone()) {
                Ok(pair) if pair.well_formed() => {
                    if !out.pairs.iter().any(|q| q.id == pair.id) {
                        out.pairs.push(pair);
                    }
                }
                Ok(_) => tracing::warn!("machines/sync.json: a malformed pair was skipped"),
                Err(e) => tracing::warn!(error = %e, "machines/sync.json: a pair was skipped"),
            }
        }
        for (k, val) in obj {
            if k != "pairs" && k != "version" {
                out.extra.insert(k.clone(), val.clone());
            }
        }
        out
    }

    fn load(data_dir: Option<&Path>) -> Self {
        data_dir
            .and_then(|d| crate::persist::read_json::<Value>(&Self::path(d)))
            .map(|v| Self::from_value(&v))
            .unwrap_or(SyncFile {
                version: VERSION,
                ..SyncFile::default()
            })
    }

    fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        let path = Self::path(data_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        crate::persist::write_json_private(&path, self)
    }

    pub fn get(&self, id: &str) -> Option<&Pair> {
        self.pairs.iter().find(|p| p.id == id)
    }
}

// ─── at run time ───────────────────────────────────────────────────────────────

/// What a step was started by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// *Sync now*, a confirmation, a resolution, the other computer asking.
    Now,
    /// A change the watcher noticed.
    Watch,
    /// The check every minute, or the other computer coming back.
    Tick,
}

/// Code sync's state (`MachinesState::sync`).
pub struct SyncState {
    data_dir: Option<PathBuf>,
    file: Mutex<SyncFile>,
    /// One step (or landing, or resolution) per pair at a time.
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// The app turned the watchers and the minute's check on.
    watching: AtomicBool,
    pub(crate) watchers: Mutex<HashMap<String, watch::Watch>>,
    /// Pairs a check was asked for while a step ran.
    again: Mutex<HashSet<String>>,
    /// Tests: a step fails at the named point (`land`), once.
    fail_at: Mutex<Option<String>>,
    /// When a watcher-started step last paused (it waits a little after).
    pub(crate) paused_at: Mutex<HashMap<String, Instant>>,
}

impl std::fmt::Debug for SyncState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncState").finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl SyncState {
    pub fn load(data_dir: Option<&Path>) -> Self {
        Self {
            data_dir: data_dir.map(Path::to_path_buf),
            file: Mutex::new(SyncFile::load(data_dir)),
            locks: Mutex::new(HashMap::new()),
            watching: AtomicBool::new(false),
            watchers: Mutex::new(HashMap::new()),
            again: Mutex::new(HashSet::new()),
            fail_at: Mutex::new(None),
            paused_at: Mutex::new(HashMap::new()),
        }
    }

    /// Reads the pairs.
    pub fn read<R>(&self, f: impl FnOnce(&SyncFile) -> R) -> R {
        f(&lock(&self.file))
    }

    /// Changes the pairs and saves them (whole, 0600).
    pub fn write<R>(&self, f: impl FnOnce(&mut SyncFile) -> R) -> R {
        let mut file = lock(&self.file);
        let out = f(&mut file);
        if let Some(dir) = &self.data_dir {
            if let Err(e) = file.save(dir) {
                tracing::warn!(error = %e, "machines/sync.json could not be saved");
            }
        }
        out
    }

    pub fn pair(&self, id: &str) -> Option<Pair> {
        self.read(|f| f.get(id).cloned())
    }

    /// The lock of one pair's steps.
    pub(crate) fn lock_of(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        lock(&self.locks).entry(id.to_string()).or_default().clone()
    }

    pub fn watching(&self) -> bool {
        self.watching.load(Ordering::Relaxed)
    }

    pub(crate) fn set_watching(&self) {
        self.watching.store(true, Ordering::Relaxed);
    }

    pub(crate) fn ask_again(&self, id: &str) {
        lock(&self.again).insert(id.to_string());
    }

    pub(crate) fn take_again(&self, id: &str) -> bool {
        lock(&self.again).remove(id)
    }

    pub(crate) fn fail_at(&self, step: Option<&str>) {
        *lock(&self.fail_at) = step.map(str::to_string);
    }

    /// Tests: whether the step should fail at `point` now (once).
    pub(crate) fn take_fail(&self, point: &str) -> bool {
        let mut f = lock(&self.fail_at);
        if f.as_deref() == Some(point) {
            *f = None;
            true
        } else {
            false
        }
    }

    /// The folder of one pair's step files (`<app data>/machines/sync/<id>`).
    pub(crate) fn dir_of(&self, id: &str) -> Option<PathBuf> {
        is_sync_id(id)
            .then(|| {
                self.data_dir
                    .as_ref()
                    .map(|d| d.join("machines").join("sync").join(id))
            })
            .flatten()
    }
}

// ─── shared helpers ────────────────────────────────────────────────────────────

/// Pairs changed: dashboards refetch `/api/machines/sync`.
pub fn publish(state: &AppState) {
    super::publish(state, "sync");
}

/// Changes one pair, saves and tells the screens; `None` when it is gone.
pub fn update(state: &AppState, id: &str, f: impl FnOnce(&mut Pair)) -> Option<Pair> {
    let out = state.machines.sync.write(|file| {
        let p = file.pairs.iter_mut().find(|p| p.id == id)?;
        f(p);
        p.updated_at = super::now();
        if p.kept.len() > MAX_KEPT {
            let drop = p.kept.len() - MAX_KEPT;
            p.kept.drain(..drop);
        }
        Some(p.clone())
    });
    if out.is_some() {
        publish(state);
    }
    out
}

/// Sets a pair's status (and tells the screens only when it changed).
pub fn set_status(state: &AppState, id: &str, status: Status) -> Option<Pair> {
    let changed = state.machines.sync.read(|f| {
        f.get(id)
            .map(|p| !p.status.same_as(&status))
            .unwrap_or(false)
    });
    if !changed {
        return state.machines.sync.pair(id);
    }
    update(state, id, |p| p.status = status)
}

/// The project's name, for screens and audit lines.
pub fn name_of(folder: &str) -> String {
    super::setup::folders::name_of(folder)
}

/// An audit line for a pair (the project's folder name and counts, never a
/// file's content).
pub fn audit(state: &AppState, action: &str, pair: &Pair, detail: Value) {
    state.devices.audit.record(AuditEntry {
        action: action.to_string(),
        device_id: Some(pair.machine.clone()),
        device_name: Some(super::peer_name(state, &pair.machine)),
        detail: Some(json!({
            "sync": pair.id,
            "project": name_of(&pair.folder),
            "more": detail,
        })),
        ..AuditEntry::now()
    });
}

/// `path` is `folder` or inside it.
pub fn inside(path: &str, folder: &str) -> bool {
    paths::same_path(path, folder) || super::setup::folders::rel_inside(path, folder).is_some()
}

/// A move of a session working in `folder` runs (either way): sync waits.
pub fn move_running_in(state: &AppState, folder: &str) -> bool {
    use super::moves::{MoveState, Role};
    super::moves::list(state).iter().any(|r| {
        !r.state.is_final()
            && !matches!(r.state, MoveState::Committed)
            && match r.role {
                Role::Out => inside(&r.source_cwd, folder),
                Role::In => r
                    .target_folder
                    .as_deref()
                    .or(r.target_cwd.as_deref())
                    .is_some_and(|f| inside(f, folder) || inside(folder, f)),
            }
    })
}

/// Whether the link to `machine` is on the local network now (which grant
/// column its requests use).
pub fn lan_now(state: &AppState, machine: &str) -> bool {
    state.machines.link(machine).path == Some(super::transport::PathKind::Lan)
}

/// The `move` grant this computer gave `machine`, and the one it gave this
/// computer (as it last said: `None` unknown), for the path in use now.
pub fn move_grants(state: &AppState, machine: &str) -> (bool, Option<bool>) {
    let lan = lan_now(state, machine);
    state.machines.store_read(|f| match f.peer(machine) {
        Some(p) => (
            p.grants.allows(Grant::Move, lan),
            p.their_grants.map(|g| g.allows(Grant::Move, lan)),
        ),
        None => (false, Some(false)),
    })
}

// ─── the view ──────────────────────────────────────────────────────────────────

fn short(oid: &Option<String>) -> Option<String> {
    oid.as_ref().map(|o| o.chars().take(10).collect())
}

fn branch_name(b: &Option<String>) -> Option<String> {
    b.as_ref()
        .map(|b| b.strip_prefix("refs/heads/").unwrap_or(b).to_string())
}

fn changes_view(c: &Changes) -> Value {
    json!({
        "files": c.files,
        "headMoved": c.head_moved,
        "branchSwitched": c.branch_switched,
        "branch": branch_name(&c.branch),
        "head": short(&c.head),
    })
}

/// One pair, as screens get it.
pub fn pair_view(state: &AppState, p: &Pair) -> Value {
    let (os, name) = state.machines.store_read(|f| {
        f.peer(&p.machine)
            .map(|q| (q.os.clone(), q.display_name().to_string()))
            .unwrap_or_default()
    });
    let name = if name.is_empty() {
        super::peer_name(state, &p.machine)
    } else {
        name
    };
    json!({
        "id": p.id,
        "machine": p.machine,
        "machineName": name,
        "machineOs": os,
        "online": state.machines.link(&p.machine).online,
        "folder": p.folder,
        "name": name_of(&p.folder),
        "there": p.there,
        "here": { "on": p.here_on },
        "peer": { "on": p.peer_on, "asked": p.asked_at.is_some() && !p.peer_on },
        "status": p.status,
        "last": p.last.as_ref().map(|l| json!({
            "at": l.at,
            "step": l.step,
            "branch": branch_name(&l.state.branch),
            "head": short(&l.state.head),
            "direction": l.direction,
            "files": l.files,
        })),
        "conflict": p.conflict.as_ref().map(|c| json!({
            "at": c.at,
            "first": c.first,
            "here": changes_view(&c.here),
            "there": changes_view(&c.there),
        })),
        "kept": p.kept,
        "createdAt": p.created_at,
        "updatedAt": p.updated_at,
    })
}

/// `GET /api/machines/sync`: every synced project, newest first.
pub fn view(state: &AppState) -> Value {
    let mut pairs = state.machines.sync.read(|f| f.pairs.clone());
    pairs.sort_by_key(|p| std::cmp::Reverse(p.created_at));
    json!({
        "pairs": pairs.iter().map(|p| pair_view(state, p)).collect::<Vec<_>>(),
        "watching": state.machines.sync.watching(),
    })
}

/// Starts code sync's background work: the watchers and the minute's
/// check, when the app turned them on (never in tests).
pub fn start(state: &AppState) {
    if !state.machines.sync.watching() {
        return;
    }
    watch::refresh(state);
    tokio::spawn(step::tick_loop(Arc::downgrade(state)));
}

#[cfg(test)]
mod tests {
    use super::*;

    const PEER: &str = "0123456789abcdef0123456789abcdef";

    fn pair(id: &str) -> Value {
        json!({
            "id": id, "machine": PEER, "folder": if cfg!(windows) { r"D:\dev\app" } else { "/Users/m/dev/app" },
            "there": r"D:\dev\app", "hereOn": true, "peerOn": true,
            "status": { "state": "inSync", "at": 5 },
            "last": { "branch": "refs/heads/main", "head": "a".repeat(40),
                      "idxTree": "b".repeat(40), "wtTree": "c".repeat(40), "step": 3,
                      "direction": "in", "files": 2 },
            "later": { "kept": true }
        })
    }

    #[test]
    fn sync_ids_are_sy_and_16_hex() {
        let id = new_sync_id();
        assert!(is_sync_id(&id), "{id}");
        assert!(
            crate::machines::code::valid_session(&id),
            "a code session id"
        );
        for bad in [
            "sy_0123",
            "sy_0123456789ABCDEF",
            "../sy_0123456789abcdef",
            "mv_0123456789abcdef",
        ] {
            assert!(!is_sync_id(bad), "{bad}");
        }
    }

    #[test]
    fn the_file_is_read_pair_by_pair_and_keeps_what_it_does_not_know() {
        let v = json!({
            "version": 1,
            "pairs": [
                pair("sy_0123456789abcdef"),
                pair("sy_0123456789abcdef"),
                pair("not-an-id"),
                { "id": "sy_1111111111111111", "machine": "short" },
                { "nonsense": true },
                pair("sy_2222222222222222"),
            ],
            "future": [1, 2]
        });
        let f = SyncFile::from_value(&v);
        assert_eq!(
            f.pairs.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["sy_0123456789abcdef", "sy_2222222222222222"]
        );
        let p = &f.pairs[0];
        assert!(p.active());
        assert_eq!(p.status, Status::InSync { at: 5 });
        assert_eq!(p.last.as_ref().unwrap().step, 3);
        assert_eq!(p.extra.get("later"), Some(&json!({ "kept": true })));
        let back = serde_json::to_value(&f).unwrap();
        assert_eq!(back["future"], json!([1, 2]));
        assert_eq!(back["pairs"][0]["later"], json!({ "kept": true }));
        assert_eq!(
            back["pairs"][0]["wtTree"],
            Value::Null,
            "flattened under last"
        );
        assert_eq!(back["pairs"][0]["last"]["wtTree"], "c".repeat(40));
        // A bad tree id from disk: the pair is skipped.
        let mut bad = pair("sy_3333333333333333");
        bad["last"]["wtTree"] = json!("--exec=x");
        assert!(SyncFile::from_value(&json!({ "pairs": [bad] }))
            .pairs
            .is_empty());
        assert!(SyncFile::from_value(&json!([1])).pairs.is_empty());
    }

    #[test]
    fn statuses_say_what_they_wait_for() {
        let paused = Status::paused(Reason::NoGrant, "DESKTOP-PC has not given this Mac Move.");
        let v = serde_json::to_value(&paused).unwrap();
        assert_eq!(v["state"], "paused");
        assert_eq!(v["reason"], "noGrant");
        assert!(v["since"].is_i64());
        assert_eq!(
            serde_json::to_value(Status::InSync { at: 7 }).unwrap(),
            json!({ "state": "inSync", "at": 7 })
        );
        assert_eq!(
            serde_json::to_value(Status::Off { detail: None }).unwrap(),
            json!({ "state": "off", "detail": null })
        );
        assert_eq!(
            serde_json::to_value(Status::Waiting).unwrap(),
            json!({ "state": "waiting" })
        );
        // A new time alone is no change for a screen.
        let later = Status::Paused {
            reason: Reason::NoGrant,
            detail: "DESKTOP-PC has not given this Mac Move.".into(),
            since: 1,
        };
        assert!(paused.same_as(&later));
        assert!(!paused.same_as(&Status::paused(Reason::Offline, "x")));
        assert!(Status::Syncing { since: 1 }.same_as(&Status::Syncing { since: 2 }));
    }

    #[test]
    fn a_path_inside_a_folder_is_told_apart() {
        let (root, sub, other) = if cfg!(windows) {
            (r"D:\dev\app", r"D:\dev\app\src", r"D:\dev\application")
        } else {
            ("/dev/app", "/dev/app/src", "/dev/application")
        };
        assert!(inside(root, root));
        assert!(inside(sub, root));
        assert!(!inside(other, root));
        assert!(!inside(root, sub));
    }
}
