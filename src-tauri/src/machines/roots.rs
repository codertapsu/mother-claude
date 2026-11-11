//! Roots (docs/PLAN-MACHINES.md §5.12, Phase 3a decision 12): a named folder
//! with one path per machine — `projects` is
//! `/Users/marcus/development/projects` on the MacBook and
//! `D:\development\projects` on the Windows desktop — so a project under a
//! root on one machine is the same project, at the same relative path, under
//! that root on another. A folder pair (docs/BACKUP.md "Folder pairs") is a
//! root with two entries.
//!
//! * **Kept** in `machines.json` (`roots`), keyed by machine id (the backup's
//!   per-install id, `identity.rs`). A removed root stays as a tombstone, so
//!   no copy of it brings it back.
//! * **Replicated** to every paired machine, as the whole list with its
//!   tombstones: when they change here (`POST /peer/roots`, answered with the
//!   receiver's list), and when a hello shows the other machine's differ
//!   (`hello.rootsDigest`). They merge as folder pairs do: per root the
//!   last writer wins (`updatedAt`, then the higher machine id), and a
//!   removal sticks. A machine takes another's roots only when it gave that
//!   machine the `move` grant, for the path they came by — roots steer where
//!   a move lands and where a session from there continues — and lets it read
//!   them (`GET /peer/roots`) with `view`.
//! * **Imported** from the folder pairs the backup sets know, each pair once,
//!   as a two-entry root whose id is made from the pair's: every machine that
//!   imports a pair makes the same root, so the copies merge and a removed
//!   one stays removed. A pair removed in the backups removes its root, unless
//!   the root was edited since.
//! * **Mapping** ([`map_between`], [`map_here`]): roots first — the longest
//!   matching path wins, a Windows path whatever its case and separators
//!   (`claude::paths::rebase`) — then the folder pairs, so nothing that mapped
//!   before stops mapping.
//!
//! Backup sets keep their pairs as they are; writing roots back into them
//! comes later.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::grants::Grant;
use super::protocol::Hello;
use super::store::PeerRecord;
use super::transport::{self, PathKind};
use crate::claude::backup::pairs::{is_pair_id, FolderPair};
use crate::claude::paths::{self, PathForm};
use crate::server::context::ClientContext;
use crate::server::devices::audit::AuditEntry;
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// What a machine lists in its hello when it keeps roots.
pub const CAPABILITY: &str = "roots";
/// Where another machine's roots are read and sent.
pub const PEER_PATH: &str = "/peer/roots";
/// At most this many roots (removed ones not counted) made here; the roots
/// of several machines together may be more.
pub const MAX_ROOTS: usize = 64;
/// Live roots kept, from every machine: another machine's roots that would
/// make more are refused.
const MAX_KEPT: usize = 1024;
/// A root's name has 1 to this many characters.
pub const MAX_NAME: usize = 40;
/// A path has at most this many characters.
pub const MAX_PATH: usize = 1024;
/// The machines one root may name.
pub const MAX_MACHINES: usize = 32;
/// Removed roots kept; beyond it the earliest removed go first.
const MAX_TOMBSTONES: usize = 1024;
/// The most roots one message from another machine may carry.
const MAX_INCOMING: usize = 2048;

// ─── the model ─────────────────────────────────────────────────────────────────

/// One root.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Root {
    /// `rt_` and 16 hex characters, made once: every copy keeps it.
    pub id: String,
    pub name: String,
    /// Machine id → its folder, as that machine writes paths. Empty once
    /// removed.
    #[serde(default)]
    pub paths: BTreeMap<String, String>,
    /// When it was last written (ms), on the clock of the machine that wrote
    /// it.
    #[serde(default)]
    pub updated_at: i64,
    /// The machine that wrote it last.
    #[serde(default)]
    pub by: String,
    /// The folder pair it was imported from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pair: Option<String>,
    /// Removed: kept as a tombstone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed: Option<Removed>,
    /// Fields a newer version wrote; kept as they were.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

/// When, and by which machine, a root was removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Removed {
    pub at: i64,
    #[serde(default)]
    pub by: String,
}

/// `rt_` and 16 lowercase hex characters.
pub fn is_root_id(s: &str) -> bool {
    s.strip_prefix("rt_")
        .is_some_and(|h| h.len() == 16 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

pub fn new_root_id() -> String {
    let hex: String = crate::server::auth::random_bytes(8)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("rt_{hex}")
}

/// The root folder pair `pair` becomes: the same id on every machine.
fn root_id_of_pair(pair: &str) -> Option<String> {
    is_pair_id(pair).then(|| format!("rt_{}", &pair[3..]))
}

/// A root's name as kept: trimmed, 1 to 40 characters, no control
/// character.
pub fn clean_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("Name the folder.".into());
    }
    if name.chars().count() > MAX_NAME {
        return Err(format!(
            "A folder's name has at most {MAX_NAME} characters."
        ));
    }
    if name.chars().any(char::is_control) {
        return Err("A folder's name has no control characters.".into());
    }
    Ok(name.to_string())
}

/// A path a root may hold, whichever machine wrote it: absolute in its own
/// form, no `.` or `..`, no control character, not too long.
pub fn well_formed_path(p: &str) -> bool {
    paths::is_absolute_anywhere(p)
        && !paths::has_dot_segments(p)
        && !p.chars().any(char::is_control)
        && p.chars().count() <= MAX_PATH
}

impl Root {
    pub fn live(&self) -> bool {
        self.removed.is_none()
    }

    pub fn path_of(&self, machine: &str) -> Option<&str> {
        self.paths.get(machine).map(String::as_str)
    }

    /// What `machines.json` or another machine may hold: a root id, a clean
    /// name, machine ids with well-formed paths (at least one while live).
    /// Anything else is dropped when read.
    pub fn well_formed(&self) -> bool {
        is_root_id(&self.id)
            && clean_name(&self.name).is_ok_and(|n| n == self.name)
            && self.paths.len() <= MAX_MACHINES
            && (!self.live() || !self.paths.is_empty())
            && self
                .paths
                .iter()
                .all(|(m, p)| super::ids::is_machine_id(m) && well_formed_path(p))
            && self.pair.as_deref().is_none_or(is_pair_id)
    }
}

/// A root as it is kept: a tombstone without its paths.
fn as_kept(mut r: Root) -> Root {
    if !r.live() {
        r.paths.clear();
    }
    r
}

fn canonical(r: &Root) -> String {
    serde_json::to_string(r).unwrap_or_default()
}

/// Which of two copies of one root is kept (`Greater`: `a`): a removal over
/// a live copy (of two removals the earlier), else the later write —
/// `updatedAt`, then the higher machine id, then the larger content — so
/// every machine keeps the same copy whatever order they arrive in.
fn preferred(a: &Root, b: &Root) -> Ordering {
    let content = || canonical(a).cmp(&canonical(b));
    match (&a.removed, &b.removed) {
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (Some(x), Some(y)) => {
            y.at.cmp(&x.at)
                .then_with(|| x.by.cmp(&y.by))
                .then_with(content)
        }
        (None, None) => a
            .updated_at
            .cmp(&b.updated_at)
            .then_with(|| a.by.cmp(&b.by))
            .then_with(content),
    }
}

/// Merges `incoming` — another machine's roots, or a copy of them — into
/// `roots`: a root joins by its id, the preferred copy ([`preferred`]) is
/// kept, and a removal sticks. Returns the ids that changed.
pub fn merge(roots: &mut Vec<Root>, incoming: &[Root]) -> Vec<String> {
    let mut changed = BTreeSet::new();
    for r in incoming {
        let r = as_kept(r.clone());
        match roots.iter_mut().find(|q| q.id == r.id) {
            Some(q) => {
                if preferred(&r, q) == Ordering::Greater {
                    *q = r;
                    changed.insert(q.id.clone());
                }
            }
            None => {
                changed.insert(r.id.clone());
                roots.push(r);
            }
        }
    }
    tidy(roots);
    changed
        .into_iter()
        .filter(|id| roots.iter().any(|r| &r.id == id))
        .collect()
}

/// Sorted by id; at most [`MAX_TOMBSTONES`] tombstones, the earliest removed
/// dropped first.
fn tidy(roots: &mut Vec<Root>) {
    let dead = roots.iter().filter(|r| !r.live()).count();
    if dead > MAX_TOMBSTONES {
        let mut gone: Vec<(i64, String)> = roots
            .iter()
            .filter_map(|r| r.removed.as_ref().map(|x| (x.at, r.id.clone())))
            .collect();
        gone.sort();
        let drop: BTreeSet<String> = gone
            .into_iter()
            .take(dead - MAX_TOMBSTONES)
            .map(|(_, id)| id)
            .collect();
        roots.retain(|r| !drop.contains(&r.id));
    }
    roots.sort_by(|a, b| a.id.cmp(&b.id));
}

/// What a machine's roots are, in one string: equal digests, equal roots.
fn digest_of(roots: &[Root]) -> String {
    crate::claude::secrets::sha256_hex(&serde_json::to_vec(roots).unwrap_or_default())
}

// ─── mapping ───────────────────────────────────────────────────────────────────

/// `path`, as machine `from` writes it, mapped to machine `to` by the live
/// root with a path on both whose `from` path is the longest one `path` is
/// under (`None`: no root maps it, or `from` is `to`).
pub fn map_in(roots: &[Root], from: &str, to: &str, path: &str) -> Option<String> {
    if from == to {
        return None;
    }
    best(roots.iter().filter(|r| r.live()).filter_map(|r| {
        let there = r.path_of(from)?;
        let here = r.path_of(to)?;
        Some((r, there, paths::rebase(path, there, here)?))
    }))
}

/// `path`, as some machine other than `here` wrote it, mapped to `here` by
/// the live roots: of every other machine's path `path` is under, the
/// longest.
pub fn map_from_any(roots: &[Root], here: &str, path: &str) -> Option<String> {
    best(
        roots
            .iter()
            .filter(|r| r.live())
            .filter_map(|r| r.path_of(here).map(|h| (r, h)))
            .flat_map(|(r, h)| {
                r.paths
                    .iter()
                    .filter(move |(m, _)| m.as_str() != here)
                    .filter_map(move |(_, there)| {
                        Some((r, there.as_str(), paths::rebase(path, there, h)?))
                    })
            }),
    )
}

/// Of the mappings found, the one by the deepest root path; a tie goes to
/// the root written last, then to the larger id.
fn best<'a>(found: impl Iterator<Item = (&'a Root, &'a str, String)>) -> Option<String> {
    found
        .max_by(|(ra, ta, _), (rb, tb, _)| {
            paths::depth(ta)
                .cmp(&paths::depth(tb))
                .then_with(|| ra.updated_at.cmp(&rb.updated_at))
                .then_with(|| ra.id.cmp(&rb.id))
        })
        .map(|(_, _, mapped)| mapped)
}

/// This machine's id, made on first use: the backup's per-install id, the
/// one `identity` and every hello carry.
pub fn here_id(state: &AppState) -> Option<String> {
    if let Some(i) = state.machines.ident.get() {
        return Some(i.id.clone());
    }
    crate::claude::backup::ops::this_machine(state)
        .ok()
        .map(|m| m.id)
}

/// This machine's id when it has one, without making it: before it has one
/// no root can name it.
fn known_here_id(state: &AppState) -> Option<String> {
    if let Some(i) = state.machines.ident.get() {
        return Some(i.id.clone());
    }
    crate::claude::backup::ops::known_machine_id(state)
}

/// The live roots, by name.
pub fn roots(state: &AppState) -> Vec<Root> {
    let mut out: Vec<Root> = state
        .machines
        .store_read(|f| f.roots.iter().filter(|r| r.live()).cloned().collect());
    out.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    out
}

/// Every root with the tombstones, as they are replicated.
pub fn all(state: &AppState) -> Vec<Root> {
    state.machines.store_read(|f| f.roots.clone())
}

/// The digest of this machine's roots (the hello's `rootsDigest`).
pub fn digest(state: &AppState) -> String {
    state.machines.store_read(|f| digest_of(&f.roots))
}

/// `path`, as machine `from_machine` writes it, mapped to machine
/// `to_machine`: by the roots first (the longest matching root path), then
/// by the folder pairs the backup sets know, so nothing that mapped before
/// stops mapping. `None`: neither maps it, or the two are one machine.
pub fn map_between(
    state: &AppState,
    from_machine: &str,
    to_machine: &str,
    path: &str,
) -> Option<String> {
    if from_machine == to_machine {
        return None;
    }
    state
        .machines
        .store_read(|f| map_in(&f.roots, from_machine, to_machine, path))
        .or_else(|| {
            let pairs = crate::claude::backup::ops::cached_pairs(state);
            crate::claude::backup::pairs::map_between(&pairs, from_machine, to_machine, path)
        })
}

/// `path`, as machine `from_machine` writes it, mapped to this machine
/// ([`map_between`]).
pub fn map_here(state: &AppState, from_machine: &str, path: &str) -> Option<String> {
    let here = known_here_id(state)?;
    map_between(state, from_machine, &here, path)
}

/// `path`, recorded by another machine that is not known by name (a session
/// a backup brought), mapped to this machine by the roots alone: of every
/// other machine's root path it is under, the longest. The caller falls back
/// on the folder pairs (`claude::paths::local_pairs`).
pub fn map_here_from_any(state: &AppState, path: &str) -> Option<String> {
    let here = known_here_id(state)?;
    state
        .machines
        .store_read(|f| map_from_any(&f.roots, &here, path))
}

// ─── importing folder pairs ────────────────────────────────────────────────────

/// What an import did.
#[derive(Debug, Default, PartialEq)]
struct Imported {
    /// Roots made from pairs.
    added: Vec<String>,
    /// Roots whose pair was removed in the backups.
    removed: Vec<String>,
    /// Pairs seen for the first time.
    noted: Vec<String>,
}

impl Imported {
    fn roots_changed(&self) -> bool {
        !self.added.is_empty() || !self.removed.is_empty()
    }
}

/// The root a live folder pair becomes: the pair's id, folders, time and
/// maker, so every machine that imports it makes the same root.
fn root_of_pair(p: &FolderPair, id: String) -> Root {
    Root {
        id,
        name: pair_name(p),
        paths: p.folders.clone(),
        updated_at: p.created_at,
        by: p.by.clone(),
        pair: Some(p.id.clone()),
        removed: None,
        extra: BTreeMap::new(),
    }
}

/// An imported pair's name: the last folder name of its maker's side (else
/// of its first side), the same on every machine.
fn pair_name(p: &FolderPair) -> String {
    p.folder_of(&p.by)
        .or_else(|| p.folders.values().next().map(String::as_str))
        .and_then(paths::last_component)
        .and_then(|n| clean_name(&n.chars().take(MAX_NAME).collect::<String>()).ok())
        .unwrap_or_else(|| "Folder pair".to_string())
}

/// Imports `pairs` (removed ones too) into `roots`: a live pair seen for the
/// first time becomes a root unless one with its id is there (live, or
/// removed: a removal sticks); a removed pair removes its root while the
/// root is as it was imported. `imported` is every pair seen, so none is
/// imported twice.
fn import_into(
    roots: &mut Vec<Root>,
    imported: &mut Vec<String>,
    pairs: &[FolderPair],
) -> Imported {
    let mut out = Imported::default();
    for p in pairs {
        let Some(id) = root_id_of_pair(&p.id) else {
            continue;
        };
        let first = !imported.contains(&p.id);
        match &p.removed {
            None if !p.valid() => continue,
            None => {
                if first && roots.iter().all(|r| r.id != id) {
                    let root = root_of_pair(p, id.clone());
                    if root.well_formed() {
                        roots.push(root);
                        out.added.push(id);
                    }
                }
            }
            Some(gone) => {
                if let Some(r) = roots.iter_mut().find(|r| {
                    r.id == id
                        && r.live()
                        && r.pair.as_deref() == Some(p.id.as_str())
                        && r.updated_at == p.created_at
                }) {
                    r.removed = Some(Removed {
                        at: gone.at,
                        by: gone.by.clone(),
                    });
                    r.paths.clear();
                    out.removed.push(id);
                }
            }
        }
        if first {
            imported.push(p.id.clone());
            out.noted.push(p.id.clone());
        }
    }
    if out.roots_changed() {
        tidy(roots);
    }
    out
}

/// The folder pairs the backup sets know (`pairs`, removed ones too), made
/// roots: each pair once. Called whenever the backups' pairs are read (at
/// launch, after a push, pull, restore or pair edit).
pub fn import_pairs(state: &AppState, pairs: &[FolderPair]) {
    if pairs.is_empty() {
        return;
    }
    let any = state.machines.store_read(|f| {
        let (mut r, mut i) = (f.roots.clone(), f.roots_imported.clone());
        import_into(&mut r, &mut i, pairs) != Imported::default()
    });
    if !any {
        return;
    }
    let done = state
        .machines
        .store_write(|f| import_into(&mut f.roots, &mut f.roots_imported, pairs));
    if done.roots_changed() {
        state.devices.audit.record(AuditEntry {
            action: "machines.roots.import".into(),
            detail: Some(json!({ "added": done.added, "removed": done.removed })),
            ..AuditEntry::now()
        });
        changed(state, None);
    }
}

// ─── this machine's dashboard ──────────────────────────────────────────────────

/// A machine a root may name.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownMachine {
    pub id: String,
    pub name: String,
    /// `macos`, `windows`, `linux`, …, when known: how its paths are written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    /// This machine.
    pub here: bool,
    /// A paired machine.
    pub paired: bool,
}

/// The machines roots may name: this one, the paired ones, those the backup
/// sets know and those a root names already.
fn known_machines(state: &AppState, here: &str) -> BTreeMap<String, KnownMachine> {
    let mut out: BTreeMap<String, KnownMachine> = BTreeMap::new();
    out.insert(
        here.to_string(),
        KnownMachine {
            id: here.to_string(),
            name: super::this_name(state),
            os: Some(super::protocol::this_os().to_string()),
            here: true,
            paired: false,
        },
    );
    let (peers, named): (Vec<(String, String, String)>, BTreeSet<String>) =
        state.machines.store_read(|f| {
            (
                f.peers
                    .iter()
                    .map(|p| (p.id.clone(), p.display_name().to_string(), p.os.clone()))
                    .collect(),
                f.roots
                    .iter()
                    .filter(|r| r.live())
                    .flat_map(|r| r.paths.keys().cloned())
                    .collect(),
            )
        });
    for (id, name, os) in peers {
        out.entry(id.clone()).or_insert(KnownMachine {
            id,
            name,
            os: Some(os).filter(|o| !o.is_empty()),
            here: false,
            paired: true,
        });
    }
    for m in crate::claude::backup::ops::pairs_view(state).machines {
        if !super::ids::is_machine_id(&m.id) {
            continue;
        }
        let slot = out.entry(m.id.clone()).or_insert(KnownMachine {
            id: m.id.clone(),
            name: String::new(),
            os: None,
            here: false,
            paired: false,
        });
        if slot.name.is_empty() {
            slot.name = m.name;
        }
        if slot.os.is_none() {
            slot.os = m.os;
        }
    }
    for id in named {
        out.entry(id.clone()).or_insert(KnownMachine {
            id,
            name: String::new(),
            os: None,
            here: false,
            paired: false,
        });
    }
    out
}

/// `GET /api/roots`, and the answer to `PUT`: this machine's id, the
/// machines a root may name (this one first) and the live roots, by name.
pub fn view(state: &AppState) -> Value {
    let here = here_id(state);
    let machines = here
        .as_deref()
        .map(|h| known_machines(state, h))
        .unwrap_or_default();
    let mut list: Vec<&KnownMachine> = machines.values().collect();
    list.sort_by(|a, b| {
        b.here
            .cmp(&a.here)
            .then(b.paired.cmp(&a.paired))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.id.cmp(&b.id))
    });
    json!({
        "machine": here,
        "machines": list,
        "roots": roots(state),
        "max": MAX_ROOTS,
    })
}

/// `PUT /api/roots`: roots to add or change and to remove, all or nothing.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Change {
    /// A root without `id` is new; one with an `id` gets this name and
    /// these paths (the whole map: a machine left out loses its path).
    #[serde(default)]
    pub upsert: Vec<RootInput>,
    /// Ids of roots to remove.
    #[serde(default)]
    pub remove: Vec<String>,
}

/// A root as asked for.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RootInput {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    /// Machine id → its folder, written the way that machine writes paths
    /// (this machine's may start with `~`).
    #[serde(default)]
    pub paths: BTreeMap<String, String>,
}

/// What a change is checked against.
struct Known {
    here: String,
    machines: BTreeMap<String, KnownMachine>,
    /// This machine's home folder, for `~`.
    home: Option<PathBuf>,
}

impl Known {
    fn os_of(&self, machine: &str) -> Option<&str> {
        self.machines.get(machine).and_then(|m| m.os.as_deref())
    }

    fn label(&self, machine: &str) -> String {
        match self.machines.get(machine) {
            Some(m) if m.here => "This computer".to_string(),
            Some(m) if !m.name.is_empty() => m.name.clone(),
            _ => machine.chars().take(8).collect(),
        }
    }
}

/// A root's path for `machine`, as typed: this machine's by its own rules
/// (`~` for the home folder, as every folder typed for it), another
/// machine's in the form its system writes (by its OS when known, else the
/// path's own) — then written plainly (`paths::clean_folder`).
fn clean_path(known: &Known, machine: &str, raw: &str) -> Result<String, String> {
    let typed = raw.trim();
    if typed.is_empty() {
        return Err("name the folder".into());
    }
    if typed.chars().any(char::is_control) {
        return Err("a folder has no control characters".into());
    }
    if typed.chars().count() > MAX_PATH {
        return Err(format!("a folder's path has at most {MAX_PATH} characters"));
    }
    if machine == known.here {
        let expanded = paths::expand_home(Path::new(typed), known.home.as_deref())
            .ok_or_else(|| "there is no home folder to expand ~ in".to_string())?;
        if !expanded.is_absolute() {
            return Err(paths::not_a_full_path(typed));
        }
        return paths::clean_folder(&expanded.to_string_lossy(), PathForm::here());
    }
    let form = known
        .os_of(machine)
        .map(PathForm::of_os)
        .or_else(|| PathForm::of_path(typed))
        .ok_or_else(|| format!("{typed} is not a full path"))?;
    paths::clean_folder(typed, form)
}

fn clean_paths(
    known: &Known,
    raw: &BTreeMap<String, String>,
) -> ApiResult<BTreeMap<String, String>> {
    if raw.is_empty() {
        return Err(ApiError::unprocessable(
            "roots_no_path",
            "Give the folder's path on at least one computer.",
        ));
    }
    if raw.len() > MAX_MACHINES {
        return Err(ApiError::unprocessable(
            "roots_bad_request",
            format!("A folder names at most {MAX_MACHINES} computers."),
        ));
    }
    let mut out = BTreeMap::new();
    for (machine, path) in raw {
        if !known.machines.contains_key(machine) {
            return Err(ApiError::unprocessable(
                "roots_unknown_machine",
                "That is not one of your computers: pair it first.",
            )
            .with("machine", machine.as_str()));
        }
        let clean = clean_path(known, machine, path).map_err(|why| {
            ApiError::unprocessable(
                "roots_bad_path",
                format!("{}: {why}.", known.label(machine)),
            )
            .with("machine", machine.as_str())
            .with("path", path.as_str())
        })?;
        out.insert(machine.clone(), clean);
    }
    Ok(out)
}

/// What a change did, for its audit line.
#[derive(Debug, Default, PartialEq)]
struct Applied {
    added: Vec<(String, String)>,
    changed: Vec<(String, String)>,
    removed: Vec<(String, String)>,
}

impl Applied {
    fn is_empty(&self) -> bool {
        self.added.is_empty() && self.changed.is_empty() && self.removed.is_empty()
    }

    fn detail(&self) -> Value {
        let list = |l: &[(String, String)]| -> Vec<Value> {
            l.iter()
                .map(|(id, name)| json!({ "id": id, "name": name }))
                .collect()
        };
        json!({
            "added": list(&self.added),
            "changed": list(&self.changed),
            "removed": list(&self.removed),
        })
    }
}

/// The live root `id`, to change: `404 root_not_found`, or `409
/// root_removed` when it was removed here or on another machine.
fn live_mut<'a>(roots: &'a mut [Root], id: &str) -> ApiResult<&'a mut Root> {
    match roots.iter_mut().find(|r| r.id == id) {
        Some(r) if r.live() => Ok(r),
        Some(_) => Err(ApiError::conflict(
            "root_removed",
            "That folder was removed, on this computer or another: add it again as a new one.",
        )
        .with("id", id)),
        None => {
            Err(ApiError::not_found("root_not_found", "No folder with that id.").with("id", id))
        }
    }
}

/// Applies `change` to `roots` as this machine (`known.here`) at `now`: all
/// of it, or nothing and why.
fn apply_change(
    roots: &mut Vec<Root>,
    change: &Change,
    known: &Known,
    now: i64,
) -> ApiResult<Applied> {
    if change.upsert.len() + change.remove.len() > MAX_ROOTS * 2 {
        return Err(ApiError::unprocessable(
            "roots_bad_request",
            format!("At most {} changes at once.", MAX_ROOTS * 2),
        ));
    }
    let mut named = BTreeSet::new();
    for id in change
        .upsert
        .iter()
        .filter_map(|u| u.id.as_deref())
        .chain(change.remove.iter().map(String::as_str))
    {
        if !is_root_id(id) {
            return Err(
                ApiError::unprocessable("roots_bad_request", "That is not a folder's id.")
                    .with("id", id),
            );
        }
        if !named.insert(id) {
            return Err(ApiError::unprocessable(
                "roots_bad_request",
                "A folder is named twice in one change.",
            )
            .with("id", id));
        }
    }
    let mut out = Applied::default();
    for id in &change.remove {
        let r = live_mut(roots, id)?;
        r.removed = Some(Removed {
            at: now,
            by: known.here.clone(),
        });
        r.paths.clear();
        out.removed.push((id.clone(), r.name.clone()));
    }
    // Ids whose name is new here (added or renamed): names stay unique.
    let mut renamed: Vec<String> = Vec::new();
    for u in &change.upsert {
        let name = clean_name(&u.name).map_err(|m| {
            ApiError::unprocessable("roots_bad_name", m).with("name", u.name.as_str())
        })?;
        let paths = clean_paths(known, &u.paths)?;
        match u.id.as_deref() {
            Some(id) => {
                let r = live_mut(roots, id)?;
                if r.name == name && r.paths == paths {
                    continue;
                }
                if r.name != name {
                    renamed.push(id.to_string());
                }
                r.name = name;
                r.paths = paths;
                // Later than the copy it replaces, whatever the clocks say.
                r.updated_at = now.max(r.updated_at.saturating_add(1));
                r.by = known.here.clone();
                out.changed.push((r.id.clone(), r.name.clone()));
            }
            None => {
                let id = new_root_id();
                renamed.push(id.clone());
                out.added.push((id.clone(), name.clone()));
                roots.push(Root {
                    id,
                    name,
                    paths,
                    updated_at: now,
                    by: known.here.clone(),
                    pair: None,
                    removed: None,
                    extra: BTreeMap::new(),
                });
            }
        }
    }
    if !out.added.is_empty() && roots.iter().filter(|r| r.live()).count() > MAX_ROOTS {
        return Err(ApiError::unprocessable(
            "roots_too_many",
            format!("At most {MAX_ROOTS} folders: remove one first."),
        ));
    }
    for id in &renamed {
        let Some(r) = roots.iter().find(|r| &r.id == id) else {
            continue;
        };
        let key = r.name.to_lowercase();
        if roots
            .iter()
            .any(|o| o.live() && o.id != r.id && o.name.to_lowercase() == key)
        {
            return Err(ApiError::unprocessable(
                "roots_duplicate_name",
                format!("A folder is already named \"{}\".", r.name),
            )
            .with("name", r.name.as_str()));
        }
    }
    tidy(roots);
    Ok(out)
}

/// `PUT /api/roots` by `ctx` (the desktop or a paired device): checked whole
/// first, so a refused change writes nothing; a change is audited, named to
/// the desktop when a device made it, shown on every dashboard and sent to
/// the other machines. Answers the view. Blocking.
pub fn change(state: &AppState, ctx: &ClientContext, req: &Change) -> ApiResult<Value> {
    let here = here_id(state)
        .ok_or_else(|| ApiError::internal("This computer's id could not be read."))?;
    let known = Known {
        machines: known_machines(state, &here),
        here,
        home: crate::claude::user_home_dir(),
    };
    let now = super::now();
    let dry = state
        .machines
        .store_read(|f| apply_change(&mut f.roots.clone(), req, &known, now))?;
    if dry.is_empty() {
        return Ok(view(state));
    }
    let applied = state.machines.store_write(|f| {
        let mut next = f.roots.clone();
        let applied = apply_change(&mut next, req, &known, now)?;
        f.roots = next;
        Ok::<_, ApiError>(applied)
    })?;
    if !applied.is_empty() {
        state.devices.audit.record(AuditEntry {
            detail: Some(applied.detail()),
            ..AuditEntry::from_ctx(ctx, "machines.roots")
        });
        crate::server::auth::note_device_action(
            state,
            ctx,
            "changed the folders your computers share",
        );
        changed(state, None);
    }
    Ok(view(state))
}

/// Roots changed here: every dashboard refetches (`{t:"machines",
/// what:"roots"}`) and the other machines get them (but `except`, where
/// they just came from).
fn changed(state: &AppState, except: Option<&str>) {
    super::publish(state, "roots");
    send_all(state, except);
}

// ─── the other machines ────────────────────────────────────────────────────────

/// What `GET /peer/roots` and `POST /peer/roots` answer: this machine's
/// roots with their tombstones, and their digest.
pub fn message(state: &AppState) -> Value {
    let roots = all(state);
    json!({ "digest": digest_of(&roots), "roots": roots })
}

/// Another machine's roots (a list), each checked; a malformed one is
/// skipped.
fn parse_incoming(v: &Value) -> Result<Vec<Root>, String> {
    let list = v.as_array().ok_or("roots must be a list")?;
    if list.len() > MAX_INCOMING {
        return Err(format!("at most {MAX_INCOMING} roots at once"));
    }
    Ok(list
        .iter()
        .filter_map(|e| match serde_json::from_value::<Root>(e.clone()) {
            Ok(r) if r.well_formed() => Some(r),
            _ => {
                tracing::debug!("a malformed root from another computer was skipped");
                None
            }
        })
        .collect())
}

/// `incoming` merged into a copy of `kept`: the new list and the ids that
/// changed, `None` when nothing does. `Err` when it would keep more live
/// roots than [`MAX_KEPT`].
fn merged_copy(kept: &[Root], incoming: &[Root]) -> Result<Option<Merged>, String> {
    let mut next = kept.to_vec();
    let ids = merge(&mut next, incoming);
    if ids.is_empty() {
        return Ok(None);
    }
    if next.iter().filter(|r| r.live()).count() > MAX_KEPT {
        return Err(format!("more than {MAX_KEPT} folders"));
    }
    Ok(Some(Merged { roots: next, ids }))
}

/// A merge's outcome ([`merged_copy`]).
struct Merged {
    roots: Vec<Root>,
    ids: Vec<String>,
}

/// The roots `machine` sent (it holds this machine's `move` grant: the
/// caller checked), merged here. A change is audited, shown on every
/// dashboard and passed on to the other paired machines. Returns whether
/// anything changed; `Err`: not a list, or more live roots than are kept
/// (nothing of it taken).
pub fn receive(state: &AppState, machine: &str, roots: &Value) -> Result<bool, String> {
    let incoming = parse_incoming(roots)?;
    // On a copy first: nothing is written when nothing changes.
    if state
        .machines
        .store_read(|f| merged_copy(&f.roots, &incoming))?
        .is_none()
    {
        return Ok(false);
    }
    let merged = state.machines.store_write(|f| {
        let merged = merged_copy(&f.roots, &incoming)?;
        Ok::<_, String>(merged.map(|m| {
            f.roots = m.roots;
            m.ids
        }))
    })?;
    let Some(changed_ids) = merged else {
        return Ok(false);
    };
    state.devices.audit.record(AuditEntry {
        action: "machine.roots".into(),
        device_id: Some(machine.to_string()),
        device_name: Some(super::peer_name(state, machine)),
        detail: Some(json!({ "roots": changed_ids })),
        ..AuditEntry::now()
    });
    changed(state, Some(machine));
    Ok(true)
}

/// Whether the link to `machine` runs on the local network now (the grants'
/// `lan` column), as the transport judges a request that comes in.
fn lan_now(state: &AppState, machine: &str) -> bool {
    let key = state
        .machines
        .store_read(|f| f.peer(machine).map(|p| p.key.clone()));
    key.and_then(|k| state.machines.live_connection(&k))
        .map(|c| transport::path_of(&c) == PathKind::Lan)
        .unwrap_or(false)
}

/// `machine`'s answer to a send or a fetch: its roots, taken when it may
/// change roots here (this machine's `move` grant, for the path it answered
/// by).
fn take_answer(state: &AppState, machine: &str, answer: &Value) {
    if !state
        .machines
        .allows(machine, Grant::Move, lan_now(state, machine))
    {
        return;
    }
    if let Err(e) = receive(state, machine, &answer["roots"]) {
        tracing::debug!(machine, error = %e, "another computer's roots were malformed");
    }
}

/// Sends this machine's roots (with the tombstones) to `machine`, and takes
/// its answer ([`take_answer`]).
pub async fn send(state: &AppState, machine: &str) -> Result<(), String> {
    let body = json!({ "roots": all(state) });
    let (status, answer) = transport::request(
        state,
        machine,
        "POST",
        PEER_PATH,
        Some(body),
        transport::REQUEST_TIMEOUT,
    )
    .await?;
    if status != 200 {
        return Err(format!("it answered {status}"));
    }
    take_answer(state, machine, &answer);
    Ok(())
}

/// Reads `machine`'s roots and takes them ([`take_answer`]).
async fn fetch(state: &AppState, machine: &str) -> Result<(), String> {
    let (status, answer) = transport::request(
        state,
        machine,
        "GET",
        PEER_PATH,
        None,
        transport::REQUEST_TIMEOUT,
    )
    .await?;
    if status != 200 {
        return Err(format!("it answered {status}"));
    }
    take_answer(state, machine, &answer);
    Ok(())
}

/// Whether `peer` may take this machine's roots: it keeps roots (or has not
/// said yet), and what it last said this machine may do there includes
/// `move` on some path (or it has not said yet). It decides itself.
fn may_send_to(peer: &PeerRecord) -> bool {
    let keeps = peer.capabilities.is_empty() || peer.capabilities.iter().any(|c| c == CAPABILITY);
    keeps
        && peer
            .their_grants
            .is_none_or(|g| g.moves.lan || g.moves.internet)
}

/// Sends this machine's roots to every paired machine that is online and
/// may take them ([`may_send_to`]) — but `except` — in the background.
pub fn send_all(state: &AppState, except: Option<&str>) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let peers: Vec<String> = state.machines.store_read(|f| {
        f.peers
            .iter()
            .filter(|p| Some(p.id.as_str()) != except && may_send_to(p))
            .map(|p| p.id.clone())
            .collect()
    });
    for machine in peers {
        if !state.machines.link(&machine).online {
            continue;
        }
        let st = state.clone();
        handle.spawn(async move {
            if let Err(e) = send(&st, &machine).await {
                tracing::debug!(machine, error = %e, "roots not sent");
            }
        });
    }
}

/// One exchange of roots with a machine, as last made.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Exchange {
    mine: String,
    theirs: Option<String>,
    may_send: bool,
    accept: bool,
}

/// The last exchange with each machine: a hello starts another only when
/// something changed since (either digest, or the grants either way).
#[derive(Default)]
pub struct Exchanges {
    last: Mutex<HashMap<String, Exchange>>,
}

impl Exchanges {
    fn begin(&self, machine: &str, ex: Exchange) -> bool {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if last.get(machine) == Some(&ex) {
            return false;
        }
        last.insert(machine.to_string(), ex);
        true
    }

    fn forget(&self, machine: &str) {
        self.last
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(machine);
    }
}

/// After a hello either way: when `machine` keeps roots and its differ from
/// this machine's, they are exchanged in the background — sent when it lets
/// this machine change its roots (its answer taken when this machine lets it
/// change these), else fetched when this machine lets it — unless nothing
/// changed since the last exchange.
pub fn after_hello(state: &AppState, machine: &str, hello: &Hello) {
    if !hello.capabilities.iter().any(|c| c == CAPABILITY) {
        return;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let mine = digest(state);
    if hello.roots_digest.as_deref() == Some(mine.as_str()) {
        return;
    }
    let lan = lan_now(state, machine);
    let grants = state.machines.store_read(|f| {
        f.peer(machine).map(|p| {
            (
                p.their_grants.is_none_or(|g| g.allows(Grant::Move, lan)),
                p.grants.allows(Grant::Move, lan),
            )
        })
    });
    let Some((may_send, accept)) = grants else {
        return;
    };
    if !may_send && !accept {
        return;
    }
    let exchange = Exchange {
        mine,
        theirs: hello.roots_digest.clone(),
        may_send,
        accept,
    };
    if !state.machines.roots_exchanges.begin(machine, exchange) {
        return;
    }
    let st = state.clone();
    let machine = machine.to_string();
    handle.spawn(async move {
        let done = if may_send {
            send(&st, &machine).await
        } else {
            fetch(&st, &machine).await
        };
        if let Err(e) = done {
            // Tried again on the next hello.
            st.machines.roots_exchanges.forget(&machine);
            tracing::debug!(machine, error = %e, "roots not exchanged");
        }
    });
}

#[cfg(test)]
mod tests;
