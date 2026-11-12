//! Moving a session to another computer, with its code (docs/PLAN-MACHINES.md
//! §5.8, Phase 3a and the decisions it was built on).
//!
//! **Who starts it.** `POST /api/sessions/{id}/move` on the machine the
//! session lives on (its *source*, S); the other machine's screens call it
//! through the proxy (`move` grant). The *target* (T) serves `/peer/moves/*`
//! and `/peer/leases`, each needing T's `move` grant for S on the path the
//! request came over (`move_routes`).
//!
//! **The transaction** (one record per side, `machines/moves/<id>.json`,
//! walking `preflight → freezing → packaging → sending → landing → landed →
//! committed → resumed`, or ending `failed`, `cancelled` or `undone`):
//!
//! 1. *Preflight* (`source::preflight`): S reads the session and its
//!    checkout, asks T (`POST /peer/moves/preflight`: the account there by
//!    name, the folder there, its trust, T's checkout, disk, Claude Code's
//!    version) and says every blocker and warning in words; nothing is
//!    written anywhere. A blocker stops the move before anything is frozen.
//! 2. *Freezing*: an owned live session is closed and stopped as an account
//!    switch does it (after this turn, or now with an interrupt); an ended
//!    one needs nothing; a foreign live one moves only as a copy.
//! 3. *Packaging*: the session's files in the backup's layout
//!    (`backup::session_pack`), the code as a git snapshot (`code`), and the
//!    files git ignores that the project lists to carry (`ignored`: the
//!    dangerous right on both computers).
//! 4. *Sending*: each file pushed in chunks of ≤ 8 MiB
//!    (`PUT /peer/moves/{id}/files/{name}?offset=`), resumable from the size
//!    T answers; T checks every hash before landing.
//! 5. *Landing* (`target::land`): T applies the code (checkout or worktree;
//!    a stop by design fails the move before *landed*, with the reason and
//!    what it offers), writes the carried ignored files into it (what they
//!    replace kept for undo), and places the session's files with the pull's
//!    machinery (new, fast-forward, or the incoming copy with T's own beside
//!    it as a conflict copy), keeping everything for undo, and keeps the move
//!    note for the session's next message.
//! 6. *Commit*: S raises the lease (home = T, epoch + 1) and replicates it
//!    (`POST /peer/leases`); T, on accepting it, resumes the owned session
//!    in the mapped folder with the account, model, effort and mode it ran
//!    with (tests record the resume instead of starting `claude`).
//!
//! Before *landed*, any failure unfreezes S (the session resumes where it
//! was) and T drops what it received; after *landed* T keeps what it has,
//! and a lease it learns later (any hello) finishes the move. *Cancel*
//! works until *landing*; *Undo move* runs on T (stop, undo the code, take
//! the placed files back out, lease back to S, which resumes the session if
//! it ran when frozen). *Take over* moves the lease here from a home that is
//! offline. After a restart a move before *landed* is failed and unfrozen,
//! one after it finished ([`recover`]).
//!
//! Every move is in the audit log of both machines (`machine.move.*`), never
//! with a transcript's or a file's content.

pub mod folder;
pub mod ignored;
pub mod note;
mod resume;
mod source;
mod target;
pub mod wire;

/// Whether an account can run a session here (a task handed over uses it
/// too, `work::tasks`); the account of a name here, and settings another
/// computer sent, checked (a copy taken over uses them, `copies`).
pub(crate) use target::{account_ready, check_settings, resolve_account};

/// The settings a session runs (or ran) with here, as a move carries them
/// (a copy keeps them for a take over).
pub(crate) fn settings_of(
    state: &AppState,
    session: &str,
    summary: Option<&crate::claude::model::SessionSummary>,
) -> Carried {
    match crate::claude::owned::live_state(state, session) {
        Some(live) => resume::carried(&live),
        None => summary.map(resume::carried_of).unwrap_or_default(),
    }
}

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::code::GitRunner;
use crate::server::context::ClientContext;
use crate::server::error::{ApiError, ApiResult};
use crate::server::ws::ServerEvent;
use crate::state::AppState;

pub use source::{preflight, start, MoveBody, Preflight};
pub use target::{
    peer_create, peer_drop, peer_file_size, peer_land, peer_leases, peer_preflight, peer_put_chunk,
    peer_state, peer_undo,
};

/// The largest chunk a source sends in one request.
pub const CHUNK: usize = super::chunks::CHUNK;
/// How long T may take to land a move (git on a large tree, a large
/// conversation's files).
pub(crate) const LAND_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// How long a preflight may take there (`git status` and the haves).
pub(crate) const PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(90);
/// The undo of a move is offered this long (the undo folder's own life).
pub const UNDO_KEEP_MS: i64 = 7 * 24 * 3600 * 1000;

// ─── the record ────────────────────────────────────────────────────────────────

/// Where a move stands (decision 10).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MoveState {
    /// Checked; nothing frozen yet.
    Preflight,
    /// Waiting for the turn to end, interrupting it, stopping the process.
    Freezing,
    /// Packing the conversation and snapshotting the code.
    Packaging,
    /// Sending the files.
    Sending,
    /// The target places the code and the conversation.
    Landing,
    /// The target has everything; the lease has not moved yet.
    Landed,
    /// The lease names the target: the session lives there now.
    Committed,
    /// It runs there (a copy: forked there).
    Resumed,
    Failed,
    Cancelled,
    /// Undone on the target: it lives where it was again.
    Undone,
}

impl MoveState {
    /// A move in this state has ended (an undo may still follow `resumed`).
    pub fn is_final(self) -> bool {
        matches!(
            self,
            Self::Resumed | Self::Failed | Self::Cancelled | Self::Undone
        )
    }

    /// Before *landed*: a failure leaves both machines as they were.
    pub fn before_landed(self) -> bool {
        matches!(
            self,
            Self::Preflight | Self::Freezing | Self::Packaging | Self::Sending | Self::Landing
        )
    }
}

/// Which side of a move a record is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// This machine sends the session (the source).
    Out,
    /// This machine receives it (the target).
    In,
}

/// A machine as a move names it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MachineRef {
    pub id: String,
    pub name: String,
    /// `macos` | `windows` | `linux` | `other`.
    pub os: String,
}

/// The account a move runs under, on each side.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AccountPart {
    /// Its name (matched by name there, case-insensitively).
    pub name: String,
    /// It is the source's Default account.
    pub is_default: bool,
    /// Its id on the source.
    pub source: String,
    /// Its id on the target, once the target said.
    pub target: Option<String>,
}

/// The settings a session runs with, carried to where it goes on.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Carried {
    pub model: Option<String>,
    pub effort: Option<String>,
    /// `on` | `off`.
    pub thinking: Option<String>,
    pub permission_mode: Option<String>,
    pub ultracode: bool,
    /// It ran with `--allow-dangerously-skip-permissions`.
    pub bypass_available: bool,
}

/// The code of a move.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CodePart {
    /// `bundle` (machine to machine) | `remote` (hidden refs on your remote)
    /// | `none`.
    pub mode: String,
    /// `checkout` (the project folder there) | `worktree` (a separate one).
    pub apply: String,
    /// The remote of a `remote` move.
    pub remote: Option<String>,
    /// The session's folder under the repository's top (`""`: the top).
    pub rel: String,
    /// `refs/heads/…`, none when detached.
    pub branch: Option<String>,
    pub base: Option<String>,
    pub snapshot: Option<String>,
    pub index_tree: Option<String>,
    /// Paths `git status` showed on the source (untracked ones included).
    pub changed: usize,
    pub untracked: usize,
    /// The snapshot's warnings (`case-collision`, `lfs`, `submodules`, …).
    pub warnings: Vec<String>,
    /// The files git ignores that travel with it (the project's list, as
    /// found), under the repository's top.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ignored: Vec<String>,
    /// The tip of the target's branch this move replaces with the source's
    /// (*Replace the branch there*, plan §5.10): kept under the landing's
    /// undo ref, put back by *Undo move*.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaces: Option<String>,
    /// Where it landed on the target.
    pub landed: Option<super::code::Applied>,
}

/// One file of a move's package.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FileEntry {
    /// `session.tar.gz`, `meta.json`, `move.bundle`, `lfs.<oid>` or
    /// `ignored.tar.gz`.
    pub name: String,
    pub size: u64,
    pub sha256: String,
    /// Bytes the target has.
    pub sent: u64,
}

/// A blocker or a warning, in words a person understands.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Check {
    pub code: String,
    pub message: String,
    /// What fixes it, when there is something to do.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    /// What the move sheet offers instead: `worktree`, `worktree-or-reset`,
    /// `as-copy`, `choose-folder`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offer: Option<String>,
    /// More for the sheet (candidate folders, the files that changed, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Box<Value>>,
}

impl Check {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            ..Self::default()
        }
    }

    pub fn fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }

    pub fn offer(mut self, offer: &str) -> Self {
        self.offer = Some(offer.to_string());
        self
    }

    pub fn detail(mut self, detail: Value) -> Self {
        self.detail = Some(Box::new(detail));
        self
    }
}

/// When a move reached a state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub state: MoveState,
    pub at: i64,
}

/// Why a move failed, and at which step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveFailure {
    pub code: String,
    pub message: String,
    pub step: MoveState,
    /// What the target offers instead (a stop by design of its checkout).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offer: Option<String>,
}

/// What the target did with the files (target side).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LandingPart {
    /// The data job whose undo folder holds what was replaced.
    pub job: Option<String>,
    /// `new` | `fastForward` | `same` | `conflict`.
    pub action: String,
    /// This machine's own turns, kept beside it (a conflict).
    pub conflict: Option<String>,
    /// The folder's trust was carried.
    pub trusted: bool,
    /// The transcript as it landed (an undo refuses once it grew).
    pub transcript_bytes: u64,
    pub transcript_sha256: String,
}

/// A move, as both machines keep it and every screen shows it
/// (`GET /api/moves/{id}`, the `{t:"move"}` frame).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveRecord {
    /// `mv_` and 16 hex characters.
    pub id: String,
    pub role: Role,
    /// The session moved (its id on the source).
    pub session: String,
    /// The id it goes on under there: the same, or a new one for a copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_session: Option<String>,
    /// What the session list called it.
    #[serde(default)]
    pub title: Option<String>,
    pub from: MachineRef,
    pub to: MachineRef,
    pub state: MoveState,
    /// A copy that goes on there under a new id; the session stays here.
    #[serde(default)]
    pub as_copy: bool,
    /// Keep driving it from here once it runs there (the sheet's choice).
    #[serde(default)]
    pub keep_driving: bool,
    /// `idle` (after this turn) | `now` (interrupting it).
    #[serde(default)]
    pub when: String,
    #[serde(default)]
    pub account: AccountPart,
    /// The lease's epoch once committed (a copy: 0).
    #[serde(default)]
    pub epoch: u64,
    /// Where it worked on the source.
    #[serde(default)]
    pub source_cwd: String,
    /// Where it works on the target, once known.
    #[serde(default)]
    pub target_cwd: Option<String>,
    /// The project folder there (the checkout; a worktree's is beside it).
    #[serde(default)]
    pub target_folder: Option<String>,
    #[serde(default)]
    pub code: Option<CodePart>,
    #[serde(default)]
    pub files: Vec<FileEntry>,
    #[serde(default)]
    pub bytes_total: u64,
    #[serde(default)]
    pub bytes_sent: u64,
    /// The conversation's own size (the sheet's "Conversation 18 MB").
    #[serde(default)]
    pub conversation_bytes: u64,
    #[serde(default)]
    pub settings: Carried,
    /// Mother Claude ran it when it was frozen: a failure or an undo starts
    /// it again here.
    #[serde(default)]
    pub was_running: bool,
    /// Messages taken back from behind an interrupted turn: they go on
    /// there (never shown in a frame).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub held: Vec<String>,
    /// The project's recipe for the target's system, joined to the move note
    /// as a request to Claude (decision 8).
    #[serde(default)]
    pub recipe: Vec<String>,
    /// The folder's trust is carried (needs the dangerous right).
    #[serde(default)]
    pub trust: bool,
    /// The conversation landed from the copy the target keeps current
    /// (docs/PLAN-MACHINES.md §5.9): it did not travel again.
    #[serde(default)]
    pub from_copy: bool,
    /// When each state was reached.
    #[serde(default)]
    pub steps: Vec<Step>,
    #[serde(default)]
    pub error: Option<MoveFailure>,
    #[serde(default)]
    pub warnings: Vec<Check>,
    /// Target: what the landing did.
    #[serde(default)]
    pub landing: Option<LandingPart>,
    /// It landed and the lease moved, but it did not start there.
    #[serde(default)]
    pub resume_error: Option<String>,
    /// Who started it (`desktop`, `device Phone`, `machine DESKTOP-PC`).
    #[serde(default)]
    pub by: String,
    /// The person who started it had the dangerous right (a resume in a
    /// mode without approvals needs it).
    #[serde(default)]
    pub dangerous: bool,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub finished_at: Option<i64>,
    /// Until when *Undo move* is offered.
    #[serde(default)]
    pub undo_until: Option<i64>,
}

impl MoveRecord {
    /// The record as screens get it: the held messages counted, not shown.
    pub fn view(&self) -> Value {
        let mut v = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Some(obj) = v.as_object_mut() {
            obj.remove("held");
            obj.insert("heldCount".into(), json!(self.held.len()));
            obj.insert("finished".into(), json!(self.state.is_final()));
        }
        v
    }

    /// Moves to `state`, noting when.
    pub fn reach(&mut self, state: MoveState) {
        let now = super::now();
        self.state = state;
        self.updated_at = now;
        if self.steps.last().map(|s| s.state) != Some(state) {
            self.steps.push(Step { state, at: now });
        }
        if state.is_final() {
            self.finished_at = Some(now);
        }
    }

    /// The machine on the other side of this record.
    pub fn other(&self) -> &MachineRef {
        match self.role {
            Role::Out => &self.to,
            Role::In => &self.from,
        }
    }
}

/// `mv_` and 16 lowercase hex characters.
pub fn is_move_id(s: &str) -> bool {
    s.strip_prefix("mv_")
        .is_some_and(|h| h.len() == 16 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

pub fn new_move_id() -> String {
    let hex: String = crate::server::auth::random_bytes(8)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("mv_{hex}")
}

// ─── the registry ──────────────────────────────────────────────────────────────

/// A move running in this process.
#[derive(Debug, Default)]
pub(crate) struct Active {
    pub cancel: AtomicBool,
    pub wake: tokio::sync::Notify,
    /// `X-MC-Acting` for the calls to the other machine.
    pub acting: String,
}

/// Moves under way here, and the test hooks.
#[derive(Default)]
pub struct Moves {
    active: Mutex<HashMap<String, Arc<Active>>>,
    /// Sessions moving away from here right now, and where to.
    moving: Mutex<HashMap<String, String>>,
    /// One record write at a time.
    write: Mutex<()>,
    /// One landing (or undo) at a time on this machine.
    pub(crate) landing: tokio::sync::Mutex<()>,
    /// One chunk written at a time (a retried chunk never races the one
    /// still being written).
    pub(crate) chunks: tokio::sync::Mutex<()>,
    fail_at: Mutex<Option<String>>,
    resumes_live: AtomicBool,
    resumes: Mutex<Vec<Value>>,
    git: Mutex<Option<GitRunner>>,
    /// Tests: a bundle larger than this goes through the remote instead
    /// ([`BUNDLE_LIMIT`] otherwise).
    bundle_limit: Mutex<Option<u64>>,
}

/// Machine to machine, a move whose bundle would be larger than this goes
/// through the remote instead when both computers reach it without a
/// prompt (plan §5.10: 1 GB).
pub const BUNDLE_LIMIT: u64 = 1_000_000_000;

impl std::fmt::Debug for Moves {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Moves").finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Moves {
    /// The machine a session is moving to, while a move of it is under way.
    pub fn moving_to(&self, session: &str) -> Option<String> {
        lock(&self.moving).get(session).cloned()
    }

    /// Registers a move of `session` to `to`; refused while another one of
    /// it runs.
    pub(crate) fn begin(
        &self,
        id: &str,
        session: &str,
        to: &str,
        acting: String,
    ) -> Option<Arc<Active>> {
        let mut moving = lock(&self.moving);
        if moving.contains_key(session) {
            return None;
        }
        moving.insert(session.to_string(), to.to_string());
        let active = Arc::new(Active {
            acting,
            ..Active::default()
        });
        lock(&self.active).insert(id.to_string(), active.clone());
        Some(active)
    }

    /// The session is no longer moving (the move ended, or the lease gates
    /// it now).
    pub(crate) fn release(&self, session: &str) {
        lock(&self.moving).remove(session);
    }

    pub(crate) fn end(&self, id: &str, session: &str) {
        lock(&self.active).remove(id);
        self.release(session);
    }

    pub(crate) fn active(&self, id: &str) -> Option<Arc<Active>> {
        lock(&self.active).get(id).cloned()
    }

    pub(crate) fn enable_resumes(&self) {
        self.resumes_live.store(true, Ordering::Relaxed);
    }

    pub(crate) fn resumes_live(&self) -> bool {
        self.resumes_live.load(Ordering::Relaxed)
    }

    pub(crate) fn record_resume(&self, v: Value) {
        lock(&self.resumes).push(v);
    }

    pub(crate) fn recorded_resumes(&self) -> Vec<Value> {
        lock(&self.resumes).clone()
    }

    pub(crate) fn use_git(&self, git: GitRunner) {
        *lock(&self.git) = Some(git);
    }

    /// The git moves run.
    pub(crate) fn git(&self) -> GitRunner {
        lock(&self.git).clone().unwrap_or_default()
    }

    /// The bundle size over which a move goes through the remote.
    pub(crate) fn bundle_limit(&self) -> u64 {
        lock(&self.bundle_limit).unwrap_or(BUNDLE_LIMIT)
    }

    pub(crate) fn set_bundle_limit(&self, limit: Option<u64>) {
        *lock(&self.bundle_limit) = limit;
    }

    pub(crate) fn fail_at(&self, step: Option<&str>) {
        *lock(&self.fail_at) = step.map(str::to_string);
    }

    /// Tests: whether the move should fail at `step` now (once).
    pub(crate) fn take_fail(&self, step: &str) -> bool {
        let mut f = lock(&self.fail_at);
        if f.as_deref() == Some(step) {
            *f = None;
            true
        } else {
            false
        }
    }
}

// ─── records on disk ───────────────────────────────────────────────────────────

/// `<app data>/machines/moves`.
pub(crate) fn moves_dir(state: &AppState) -> Option<PathBuf> {
    state
        .auth
        .config_dir
        .as_ref()
        .map(|d| d.join("machines").join("moves"))
}

/// `<app data>/machines/worktrees`: where code lands when the project folder
/// here has changes of its own (`<root>/<session>`).
pub(crate) fn worktrees_root(state: &AppState) -> Option<PathBuf> {
    state
        .auth
        .config_dir
        .as_ref()
        .map(|d| d.join("machines").join("worktrees"))
}

/// A move's folder: its package (source) or what it received (target).
pub(crate) fn move_folder(state: &AppState, id: &str) -> Option<PathBuf> {
    is_move_id(id)
        .then(|| moves_dir(state).map(|d| d.join(id)))
        .flatten()
}

/// A move's record.
pub fn load(state: &AppState, id: &str) -> Option<MoveRecord> {
    if !is_move_id(id) {
        return None;
    }
    let path = moves_dir(state)?.join(format!("{id}.json"));
    crate::persist::read_json::<MoveRecord>(&path).filter(|r| r.id == id)
}

fn write_record(state: &AppState, rec: &MoveRecord) {
    let Some(dir) = moves_dir(state) else {
        return;
    };
    if let Err(e) = crate::persist::write_json_private(&dir.join(format!("{}.json", rec.id)), rec) {
        tracing::warn!(error = %e, id = %rec.id, "a move's record could not be saved");
    }
}

/// Saves a record and tells every screen.
pub(crate) fn save(state: &AppState, rec: &MoveRecord) {
    {
        let _g = lock(&state.machines.moves.write);
        write_record(state, rec);
    }
    state.broadcast(ServerEvent::Move { record: rec.view() });
}

/// Changes a record — read from disk and written back under one lock, so
/// every writer sees the last one — and tells every screen.
pub(crate) fn update(
    state: &AppState,
    id: &str,
    f: impl FnOnce(&mut MoveRecord),
) -> Option<MoveRecord> {
    let rec = {
        let _g = lock(&state.machines.moves.write);
        let mut rec = load(state, id)?;
        f(&mut rec);
        rec.updated_at = super::now();
        write_record(state, &rec);
        rec
    };
    state.broadcast(ServerEvent::Move { record: rec.view() });
    Some(rec)
}

/// Where a package's file is in a move's folder: `session.tar.gz`, the
/// code's `meta.json`, `move.bundle` and Git LFS objects (`lfs.<oid>`) as
/// `code::apply` reads them, and the carried ignored files'
/// `ignored.tar.gz`. Any other name is none.
pub(crate) fn file_path(dir: &std::path::Path, name: &str) -> Option<PathBuf> {
    let code = dir.join("code");
    match name {
        crate::claude::backup::session_pack::FILE | ignored::FILE => Some(dir.join(name)),
        super::code::META => Some(code.join(name)),
        super::code::BUNDLE => Some(code.join(name)),
        _ => {
            let oid = name.strip_prefix("lfs.")?;
            super::code::is_lfs_oid(oid).then(|| {
                code.join("lfs")
                    .join("objects")
                    .join(&oid[..2])
                    .join(&oid[2..4])
                    .join(oid)
            })
        }
    }
}

/// Every move this machine keeps, newest first.
fn all(state: &AppState) -> Vec<MoveRecord> {
    let Some(dir) = moves_dir(state) else {
        return Vec::new();
    };
    let mut out: Vec<MoveRecord> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let id = name.strip_suffix(".json")?;
            load(state, id)
        })
        .collect();
    out.sort_by_key(|r| std::cmp::Reverse(r.created_at));
    out
}

/// Every move this machine knows, newest first (at most 100).
pub fn list(state: &AppState) -> Vec<MoveRecord> {
    let mut out = all(state);
    out.truncate(100);
    out
}

/// The moves of one session (newest first).
pub(crate) fn of_session(state: &AppState, session: &str) -> Vec<MoveRecord> {
    list(state)
        .into_iter()
        .filter(|r| r.session == session || r.target_session.as_deref() == Some(session))
        .collect()
}

// ─── shared helpers ────────────────────────────────────────────────────────────

/// This machine as a move names it.
pub(crate) fn this_machine(state: &AppState) -> MachineRef {
    let me = super::machine_info(state);
    MachineRef {
        id: me.id,
        name: me.name,
        os: me.os,
    }
}

/// A paired machine as a move names it.
pub(crate) fn peer_ref(state: &AppState, id: &str) -> MachineRef {
    let os = state
        .machines
        .store_read(|f| f.peer(id).map(|p| p.os.clone()))
        .unwrap_or_default();
    MachineRef {
        id: id.to_string(),
        name: super::peer_name(state, id),
        os,
    }
}

/// Who acts, for records and audit lines.
pub(crate) fn who(state: &AppState, ctx: &ClientContext) -> String {
    use crate::server::context::Principal;
    match &ctx.principal {
        Principal::Desktop => "desktop".into(),
        Principal::Device { id, .. } => format!(
            "device {}",
            state.devices.get(id).map(|d| d.name).unwrap_or_default()
        ),
        Principal::Peer { machine, .. } => format!("machine {}", super::peer_name(state, machine)),
        Principal::LocalScript => "script".into(),
        _ => "someone".into(),
    }
}

/// A JSON call to another machine, with who acts.
pub(crate) async fn call(
    state: &AppState,
    machine: &str,
    method: &str,
    path: &str,
    body: Option<&Value>,
    acting: &str,
    timeout: Duration,
) -> Result<(u16, Value), String> {
    let mut headers: Vec<(&str, String)> = Vec::new();
    if !acting.is_empty() {
        headers.push((super::acting::HEADER, acting.to_string()));
    }
    let bytes = match body {
        Some(v) => {
            headers.push(("content-type", "application/json".into()));
            axum::body::Bytes::from(serde_json::to_vec(v).map_err(|e| e.to_string())?)
        }
        None => axum::body::Bytes::new(),
    };
    let (status, _, out) =
        super::transport::request_raw(state, machine, method, path, &headers, bytes, timeout)
            .await?;
    let v = if out.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&out).unwrap_or(Value::Null)
    };
    Ok((status, v))
}

/// The other machine's refusal, in its own words.
pub(crate) fn refusal(status: u16, body: &Value, name: &str) -> ApiError {
    let code = body
        .pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or("machine_refused")
        .to_string();
    let message = body
        .pointer("/error/message")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("{name} refused ({status})."));
    let st = axum::http::StatusCode::from_u16(status)
        .ok()
        .filter(|s| s.is_client_error())
        .unwrap_or(axum::http::StatusCode::BAD_GATEWAY);
    let mut e = ApiError::new(st, code, message);
    if let Some(extra) = body.pointer("/error").and_then(Value::as_object) {
        for (k, v) in extra {
            if k != "code" && k != "message" {
                e = e.with(k, v.clone());
            }
        }
    }
    e
}

/// An audit line for a move (no content: the session, the machines, the
/// state and the outcome's code).
pub(crate) fn audit(state: &AppState, action: &str, rec: &MoveRecord, detail: Value) {
    let other = rec.other();
    state
        .devices
        .audit
        .record(crate::server::devices::audit::AuditEntry {
            action: action.to_string(),
            device_id: Some(other.id.clone()),
            device_name: Some(other.name.clone()),
            detail: Some(json!({
                "move": rec.id,
                "session": rec.session,
                "from": rec.from.id,
                "to": rec.to.id,
                "state": rec.state,
                "asCopy": rec.as_copy,
                "by": rec.by,
                "more": detail,
            })),
            ..crate::server::devices::audit::AuditEntry::now()
        });
}

/// `404 move_not_found`.
pub(crate) fn not_found() -> ApiError {
    ApiError::not_found("move_not_found", "No such move here.")
}

/// Tests: the move fails here (`injected`).
pub(crate) fn injected(state: &AppState, step: &str) -> Result<(), ApiError> {
    if state.machines.moves.take_fail(step) {
        return Err(ApiError::new(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "injected",
            format!("A test stopped the move at \"{step}\"."),
        ));
    }
    Ok(())
}

/// The disk space free where `path` is (`None`: unknown).
pub(crate) fn free_bytes(path: &std::path::Path) -> Option<u64> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut free: u64 = 0;
        // SAFETY: `wide` is a NUL-terminated wide string that outlives the
        // call; the out pointer is to a live u64; the others may be null.
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut free,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        (ok != 0).then_some(free)
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        // SAFETY: `c` is a NUL-terminated path that outlives the call, and
        // `st` is a zeroed struct statvfs fills.
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        let rc = unsafe { libc::statvfs(c.as_ptr(), &mut st) };
        (rc == 0).then(|| st.f_bavail as u64 * st.f_frsize as u64)
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = path;
        None
    }
}

// ─── the API's other routes ────────────────────────────────────────────────────

/// `POST /api/moves/{id}/cancel`: until the target starts placing it.
pub async fn cancel(state: &AppState, id: &str) -> ApiResult<MoveRecord> {
    let rec = load(state, id).ok_or_else(not_found)?;
    if rec.role != Role::Out {
        return Err(ApiError::conflict(
            "move_not_cancellable",
            format!(
                "This move is {}'s to cancel: it started there.",
                rec.from.name
            ),
        ));
    }
    if rec.state.is_final() {
        return Err(ApiError::conflict(
            "move_finished",
            "This move has finished already.",
        ));
    }
    if !matches!(
        rec.state,
        MoveState::Preflight | MoveState::Freezing | MoveState::Packaging | MoveState::Sending
    ) {
        return Err(ApiError::conflict(
            "move_too_late",
            format!(
                "It is being placed on {} now: once it is there, Undo move takes it back.",
                rec.to.name
            ),
        ));
    }
    let Some(active) = state.machines.moves.active(id) else {
        return Err(ApiError::conflict(
            "move_finished",
            "This move is not running any more.",
        ));
    };
    active.cancel.store(true, Ordering::SeqCst);
    active.wake.notify_waiters();
    Ok(rec)
}

/// `POST /api/moves/{id}/undo`: runs on the target (decision 11), asked from
/// here when this machine sent it.
pub async fn undo(state: &AppState, ctx: &ClientContext, id: &str) -> ApiResult<MoveRecord> {
    let rec = load(state, id).ok_or_else(not_found)?;
    match rec.role {
        Role::In => target::undo_here(state, ctx, &rec).await,
        Role::Out => source::undo_there(state, ctx, &rec).await,
    }
}

/// `POST /api/moves/{id}/retry`: a move that landed and moved the lease but
/// did not start there is started again (nothing is sent twice).
pub async fn retry(state: &AppState, ctx: &ClientContext, id: &str) -> ApiResult<MoveRecord> {
    let rec = load(state, id).ok_or_else(not_found)?;
    match (rec.role, rec.state) {
        (Role::Out, MoveState::Committed | MoveState::Landed) => {
            source::finish(state, &rec.id, &super::acting::header_for(state, ctx)).await;
            load(state, id).ok_or_else(not_found)
        }
        (Role::In, MoveState::Committed | MoveState::Landed) => {
            target::resume_landed(state, &rec.id, ctx.may_dangerous()).await;
            load(state, id).ok_or_else(not_found)
        }
        _ => Err(ApiError::conflict(
            "move_not_retryable",
            "Only a move that is on the other computer but did not start there is tried again; \
             start a new move instead.",
        )),
    }
}

// ─── leases, restarts ──────────────────────────────────────────────────────────

/// Leases changed (a hello): a move that landed here waits for the lease
/// naming this machine and then resumes (in the background); a move this
/// machine sent that the target undid comes home (and starts again here if
/// it ran).
pub async fn on_leases_changed(state: &AppState, sessions: &[String]) {
    if sessions.is_empty() {
        return;
    }
    go_on_in(state, sessions);
    go_on_out(state, sessions).await;
}

/// The moves that landed here and that a lease now names this machine for.
fn go_on_in(state: &AppState, sessions: &[String]) {
    let Some(me) = super::self_id(state) else {
        return;
    };
    for session in sessions {
        let Some(lease) = state.machines.leases.get(session) else {
            continue;
        };
        for rec in of_session(state, session) {
            if rec.role == Role::In
                && !rec.as_copy
                && lease.home == me
                && lease.epoch == rec.epoch
                && matches!(rec.state, MoveState::Landed | MoveState::Committed)
            {
                let st = state.clone();
                let id = rec.id.clone();
                let dangerous = rec.dangerous;
                tokio::spawn(async move { target::resume_landed(&st, &id, dangerous).await });
            }
        }
    }
}

/// The moves this machine sent whose lease came back under their own id:
/// undone there.
pub(crate) async fn go_on_out(state: &AppState, sessions: &[String]) {
    let Some(me) = super::self_id(state) else {
        return;
    };
    for session in sessions {
        let Some(lease) = state.machines.leases.get(session) else {
            continue;
        };
        for rec in of_session(state, session) {
            if rec.role == Role::Out
                && lease.home == me
                && lease.move_id.as_deref() == Some(rec.id.as_str())
                && matches!(rec.state, MoveState::Committed | MoveState::Resumed)
            {
                source::came_home(state, &rec).await;
            }
        }
    }
}

/// After a hello with `machine`: a move this machine sent there that is
/// committed but not known to run there asks how it stands (it may have
/// started there since, once the lease reached it).
pub async fn follow_up(state: &AppState, machine: &str) {
    let waiting: Vec<MoveRecord> = list(state)
        .into_iter()
        .filter(|r| r.role == Role::Out && r.to.id == machine && r.state == MoveState::Committed)
        .collect();
    for rec in waiting {
        let Ok((200, v)) = call(
            state,
            machine,
            "GET",
            &format!("/peer/moves/{}", rec.id),
            None,
            "",
            Duration::from_secs(20),
        )
        .await
        else {
            continue;
        };
        if v.get("state").and_then(Value::as_str) == Some("resumed") {
            update(state, &rec.id, |r| {
                if r.state == MoveState::Committed {
                    r.resume_error = None;
                    r.reach(MoveState::Resumed);
                }
            });
        }
    }
}

/// After a restart (decision 10): a move before *landed* is failed and
/// unfrozen (this machine sent it) or dropped (it was coming here); one
/// after it is finished.
pub async fn recover(state: &AppState) {
    prune(state);
    for rec in list(state) {
        if rec.state.is_final() {
            continue;
        }
        match rec.role {
            Role::Out if rec.state.before_landed() => {
                source::fail_after_restart(state, &rec).await;
            }
            Role::Out => source::finish(state, &rec.id, "").await,
            Role::In if rec.state.before_landed() => {
                target::drop_after_restart(state, &rec).await;
            }
            Role::In => {
                let me = super::self_id(state).unwrap_or_default();
                let leased_here = state
                    .machines
                    .leases
                    .get(&rec.session)
                    .is_some_and(|l| l.home == me && l.epoch == rec.epoch);
                if rec.as_copy || leased_here || rec.state == MoveState::Committed {
                    target::resume_landed(state, &rec.id, rec.dangerous).await;
                }
            }
        }
    }
}

/// Moves that ended more than a week and a day ago go, with their folders
/// (their undo has expired with the undo folder's).
fn prune(state: &AppState) {
    let Some(dir) = moves_dir(state) else { return };
    let cutoff = super::now() - UNDO_KEEP_MS - 24 * 3600 * 1000;
    for rec in all(state) {
        let ended = rec.finished_at.unwrap_or(rec.updated_at);
        if rec.state.is_final() && ended < cutoff && rec.undo_until.is_none_or(|u| u < super::now())
        {
            let _ = std::fs::remove_file(dir.join(format!("{}.json", rec.id)));
            let _ = std::fs::remove_dir_all(dir.join(&rec.id));
        }
    }
}

/// `{moves: [...]}` for `GET /api/moves`.
pub fn list_view(state: &AppState) -> Value {
    json!({ "moves": list(state).iter().map(MoveRecord::view).collect::<Vec<_>>() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_ids_are_mv_and_16_hex() {
        let id = new_move_id();
        assert!(is_move_id(&id), "{id}");
        assert!(!is_move_id("mv_0123"));
        assert!(!is_move_id("mv_0123456789ABCDEF"));
        assert!(!is_move_id("../mv_0123456789abcdef"));
    }

    #[test]
    fn the_states_walk_in_order_and_say_when_they_end() {
        use MoveState::*;
        for s in [Preflight, Freezing, Packaging, Sending, Landing] {
            assert!(s.before_landed() && !s.is_final(), "{s:?}");
        }
        for s in [Landed, Committed] {
            assert!(!s.before_landed() && !s.is_final(), "{s:?}");
        }
        for s in [Resumed, Failed, Cancelled, Undone] {
            assert!(s.is_final(), "{s:?}");
        }
        assert_eq!(serde_json::to_value(Landed).unwrap(), "landed");
    }

    #[test]
    fn a_record_shows_its_steps_and_hides_held_messages() {
        let mut r = MoveRecord {
            id: new_move_id(),
            role: Role::Out,
            session: "s".into(),
            target_session: None,
            title: None,
            from: MachineRef::default(),
            to: MachineRef::default(),
            state: MoveState::Preflight,
            as_copy: false,
            keep_driving: true,
            when: "idle".into(),
            account: AccountPart::default(),
            epoch: 0,
            source_cwd: String::new(),
            target_cwd: None,
            target_folder: None,
            code: None,
            files: Vec::new(),
            bytes_total: 0,
            bytes_sent: 0,
            conversation_bytes: 0,
            settings: Carried::default(),
            was_running: false,
            held: vec!["secret words".into()],
            recipe: Vec::new(),
            trust: false,
            from_copy: false,
            steps: Vec::new(),
            error: None,
            warnings: Vec::new(),
            landing: None,
            resume_error: None,
            by: "desktop".into(),
            dangerous: false,
            created_at: 1,
            updated_at: 1,
            finished_at: None,
            undo_until: None,
        };
        r.reach(MoveState::Freezing);
        r.reach(MoveState::Freezing);
        r.reach(MoveState::Failed);
        assert_eq!(r.steps.len(), 2);
        assert!(r.finished_at.is_some());
        let v = r.view();
        assert!(v.get("held").is_none());
        assert_eq!(v["heldCount"], 1);
        assert_eq!(v["finished"], true);
        assert!(!v.to_string().contains("secret words"));
    }
}
