//! A team role that runs on another computer (docs/PLAN-MACHINES.md §5.11,
//! Phase 4 decision 9; docs/TEAMS.md "A role on another computer").
//!
//! A template role may carry `machine` — a paired computer's name or id, or
//! `os:windows` / `os:macos` / `os:linux` (the first online paired computer
//! of that OS that lets this one run). The **lead's computer** keeps the
//! team, its board and its launcher; its launcher starts such a role *there*
//! through the peer link ([`lead`]); the **computer that runs it** admits it
//! with its own `capacity::admit` and starts it as an owned session in a
//! worktree of its own at the lead's code ([`host`]).
//!
//! **The binding.** The lead mints one per role and computer when it first
//! starts the role there: an opaque id (`rb_…`) and a secret. The computer
//! that runs the role keeps both in its record of it; the lead keeps the id
//! and the secret's SHA-256 in the team's role slot. A board call the role
//! session makes there reaches the lead's board only with that id, that
//! secret, from that computer, for that team and role and the role's
//! current session ([`check_binding`]); anything else is refused as if the
//! binding did not exist.
//!
//! **Who agrees.** The computer that runs it must have given the lead's its
//! `run` grant (every route of [`host`] checks it, with Machines on there);
//! a role that runs without permission prompts also needs its `dangerous`
//! grant and the right of whoever started the team (`X-MC-Acting`). The
//! board calls come the other way and need Machines on at the lead's and a
//! valid binding — the lead minted it for that computer itself.
//!
//! **What comes back.** When the role finishes a task (`task_done`), the
//! computer that runs it snapshots the worktree (its commits and what it
//! left in the working tree) and the lead fetches it into the project as
//! `refs/heads/mother-claude/<computer>/<team>/<role>` — the task code's
//! branch rules: never checked out, never merged, moved only forward — and
//! the board's task records it ([`crate::orchestration::board::RemoteWork`]).
//!
//! **Records.** Both computers audit starts, stops, removals and results
//! (the team, the role, the computer — never a task's text); the computer
//! that runs a role keeps its record in `<app data>/machines/work/roles/
//! <binding>/role.json` (0600), next to its worktree; every change is a
//! `{t:"machines", what:"roles"}` frame there and a team frame at the lead's.

pub mod host;
pub mod lead;
pub mod routes;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::target::FileSpec;
use crate::machines::code::TaskTip;
use crate::orchestration::capacity::RunningWorker;
use crate::orchestration::guard::Rule;
use crate::server::devices::audit::AuditEntry;
use crate::state::AppState;

/// The capability a computer lists when it runs team roles for others.
pub const CAPABILITY: &str = "teamRoles";
/// The folder under `machines/work/` holding the roles' folders.
pub const DIR: &str = "roles";
/// A role's record in its folder (on the computer that runs it).
pub const RECORD: &str = "role.json";
/// What arrives (the lead's code there; a result at the lead's). Its
/// leading dot keeps it apart from the worktree, whose name never starts
/// with one (`code::safe_name`).
pub const IN_DIR: &str = ".in";
/// What leaves (the lead's code, at the lead's).
pub const OUT_DIR: &str = ".out";
/// The results made, one per number (on the computer that runs it).
pub const RESULTS_DIR: &str = ".results";
/// The team's role definitions (`--agents`), on the computer that runs it.
pub const AGENTS_FILE: &str = ".agents.json";
/// The OSes `os:` may name.
pub const OSES: [&str; 3] = ["windows", "macos", "linux"];

// ─── ids and the binding ───────────────────────────────────────────────────────

/// `rb_` and 16 lowercase hex characters.
pub fn is_binding_id(s: &str) -> bool {
    s.strip_prefix("rb_")
        .is_some_and(|h| h.len() == 16 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A new binding: its id and its secret (64 hex characters).
pub fn new_binding() -> (String, String) {
    let id = format!("rb_{}", hex(&crate::server::auth::random_bytes(8)));
    (id, hex(&crate::server::auth::random_bytes(32)))
}

/// A secret as the lead keeps it.
pub fn secret_sha256(secret: &str) -> String {
    crate::claude::secrets::sha256_hex(secret.as_bytes())
}

/// What the lead keeps of a binding, and the role's session now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingKey<'a> {
    pub binding: &'a str,
    /// The computer it was minted for.
    pub machine: &'a str,
    pub team: &'a str,
    pub role: &'a str,
    pub secret_sha256: &'a str,
    /// The role's session (the one it started there).
    pub session: Option<&'a str>,
}

/// What a board call says it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingClaim<'a> {
    /// The computer the call came from (the transport's key, never the body).
    pub from: &'a str,
    pub binding: &'a str,
    pub secret: &'a str,
    pub team: &'a str,
    pub role: &'a str,
    pub session: &'a str,
}

/// Why a board call's binding was refused (logged; the caller only hears
/// that it was).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingRefusal {
    Binding,
    Computer,
    Team,
    Role,
    Secret,
    Session,
}

/// Whether a board call may act as the role `key` names: the same binding,
/// from the computer it was minted for, for its team and role, with its
/// secret and the role's current session.
pub fn check_binding(key: &BindingKey, claim: &BindingClaim) -> Result<(), BindingRefusal> {
    if !is_binding_id(claim.binding) || claim.binding != key.binding {
        return Err(BindingRefusal::Binding);
    }
    if claim.from != key.machine {
        return Err(BindingRefusal::Computer);
    }
    if claim.team != key.team {
        return Err(BindingRefusal::Team);
    }
    if claim.role != key.role {
        return Err(BindingRefusal::Role);
    }
    let sha = secret_sha256(claim.secret);
    if claim.secret.len() != 64 || !same_text(&sha, key.secret_sha256) {
        return Err(BindingRefusal::Secret);
    }
    if key.session != Some(claim.session) {
        return Err(BindingRefusal::Session);
    }
    Ok(())
}

/// Equal strings, compared without stopping at the first difference.
fn same_text(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

// ─── which computer a role's `machine` names ───────────────────────────────────

/// A role's `machine`, read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MachineSpec {
    /// `os:<os>`: the first online paired computer of that OS that lets
    /// this one run.
    Os(String),
    /// A paired computer's name (or alias) or id.
    Named(String),
}

/// `None` for an empty one (the role runs on the team's computer).
pub fn parse_spec(s: &str) -> Option<MachineSpec> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    Some(match s.strip_prefix("os:") {
        Some(os) => MachineSpec::Os(os.trim().to_ascii_lowercase()),
        None => MachineSpec::Named(s.to_string()),
    })
}

/// A paired computer as resolving a role's `machine` sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub id: String,
    /// Its own name and the name it shows here (an alias).
    pub name: String,
    pub display: String,
    pub os: String,
    pub online: bool,
    /// It gave this computer `run` on the path in use.
    pub may_run: bool,
}

/// Why no computer was picked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unresolved {
    /// No paired computer has that name or id.
    Unknown,
    /// No paired computer runs that OS.
    NoneOfOs(String),
    /// Some do, but none is online with `run` for this computer.
    NoneReady(String, Vec<String>),
}

/// The paired computer a role's `machine` names. A name or id picks that
/// computer, whatever its state (the caller says why it can't start now);
/// `os:` picks `prefer` (the computer the role ran on before) when it is of
/// that OS — a role stays where its worktree is — else the first online
/// computer of that OS that lets this one run.
pub fn pick<'a>(
    spec: &MachineSpec,
    candidates: &'a [Candidate],
    prefer: Option<&str>,
) -> Result<&'a Candidate, Unresolved> {
    match spec {
        MachineSpec::Named(n) => candidates
            .iter()
            .find(|c| c.id == *n)
            .or_else(|| {
                candidates
                    .iter()
                    .find(|c| c.display.eq_ignore_ascii_case(n) || c.name.eq_ignore_ascii_case(n))
            })
            .ok_or(Unresolved::Unknown),
        MachineSpec::Os(os) => {
            let of_os: Vec<&Candidate> = candidates.iter().filter(|c| c.os == *os).collect();
            if of_os.is_empty() {
                return Err(Unresolved::NoneOfOs(os.clone()));
            }
            if let Some(p) = prefer.and_then(|p| of_os.iter().find(|c| c.id == p)) {
                return Ok(p);
            }
            of_os
                .iter()
                .find(|c| c.online && c.may_run)
                .copied()
                .ok_or_else(|| {
                    Unresolved::NoneReady(
                        os.clone(),
                        of_os.iter().map(|c| c.display.clone()).collect(),
                    )
                })
        }
    }
}

/// An OS as a sentence names it.
pub fn os_label(os: &str) -> &str {
    match os {
        "windows" => "Windows",
        "macos" => "macOS",
        "linux" => "Linux",
        other => other,
    }
}

/// [`Unresolved`] in the user's words, for `spec` as the role names it.
pub fn unresolved_text(spec: &str, u: &Unresolved) -> String {
    match u {
        Unresolved::Unknown => format!(
            "No paired computer is called {:?}: pair it in Computers, or change the \
             role's computer.",
            spec.trim()
        ),
        Unresolved::NoneOfOs(os) => format!(
            "No paired computer runs {}: pair one in Computers, or change the role's \
             computer.",
            os_label(os)
        ),
        Unresolved::NoneReady(os, names) => {
            format!(
            "No {} computer can run the role now ({} {}): it must be online and have given this \
             computer the Run permission.",
            os_label(os),
            names.join(", "),
            if names.len() == 1 { "is not ready" } else { "are not ready" }
        )
        }
    }
}

/// The paired computers, as [`pick`] reads them.
pub fn candidates(state: &AppState) -> Vec<Candidate> {
    let peers = state.machines.store_read(|f| f.peers.clone());
    peers
        .iter()
        .map(|p| {
            let link = state.machines.link(&p.id);
            let lan = link.path == Some(crate::machines::transport::PathKind::Lan);
            Candidate {
                id: p.id.clone(),
                name: p.name.clone(),
                display: p.display_name().to_string(),
                os: p.os.clone(),
                online: link.online,
                may_run: p
                    .their_grants
                    .is_none_or(|g| g.allows(crate::machines::grants::Grant::Run, lan)),
            }
        })
        .collect()
}

/// Whether `spec` names this computer itself (its name or id).
pub fn names_this(state: &AppState, spec: &str) -> bool {
    let s = spec.trim();
    !s.is_empty()
        && (crate::machines::this_name(state).eq_ignore_ascii_case(s)
            || crate::machines::self_id(state).as_deref() == Some(s))
}

/// What a template's role names, checked against the paired computers: why
/// it could not start, when no paired computer matches (a template-time
/// warning). Online and permissions are the start's business.
pub fn spec_warning(state: &AppState, spec: &str) -> Option<String> {
    let parsed = parse_spec(spec)?;
    if names_this(state, spec) {
        return Some("That is this computer: leave `machine` out to run the role here.".into());
    }
    let all: Vec<Candidate> = candidates(state)
        .into_iter()
        .map(|c| Candidate {
            online: true,
            may_run: true,
            ..c
        })
        .collect();
    pick(&parsed, &all, None)
        .err()
        .map(|u| unresolved_text(spec, &u))
}

// ─── a role this computer runs for another one ─────────────────────────────────

/// Where a role this computer runs for another one is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HostState {
    /// The lead's code is on its way.
    Sending,
    /// The code is here: the worktree is set, the session starts.
    Starting,
    /// Its session runs (in a turn or between turns).
    Running,
    /// Its session ended (the lead stopped it, someone here did, or it
    /// ended by itself); the lead starts it again when it has work.
    Stopped,
    /// It could not start (said in `error`).
    Error,
}

/// What the lead starts a role session with (everything but the account's
/// environment and the folder, which this computer decides).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RoleSpec {
    /// `-n`: `<project>/<role>`, the role's session name.
    pub session_name: String,
    /// `--append-system-prompt`: the role's charter.
    pub charter: String,
    /// `--agents`: the team's roles as agent definitions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents: Option<Value>,
    /// The guard's globs, relative to the worktree, and read-only.
    pub paths: Vec<String>,
    pub read_only: bool,
    /// `--add-dir`, relative to the worktree.
    pub add_dirs: Vec<String>,
    pub permission_mode: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_usd: Option<f64>,
    /// 0 (highest) … 3, for admission.
    pub priority: u8,
}

/// A role this computer runs for another one, as it keeps it (`role.json`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedRecord {
    pub binding: String,
    /// The lead's computer (its id) and its name.
    pub lead: String,
    #[serde(default)]
    pub lead_name: String,
    pub team: String,
    #[serde(default)]
    pub team_label: String,
    pub role: String,
    #[serde(default)]
    pub role_title: String,
    /// The project on the lead's computer, as it writes it, and its name.
    pub project: String,
    #[serde(default)]
    pub project_name: String,
    /// This computer's checkout of it, and the role's worktree of that.
    #[serde(default)]
    pub checkout: String,
    #[serde(default)]
    pub folder: String,
    /// Its session now, and the earlier ones.
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub previous: Vec<String>,
    pub state: HostState,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub permission_mode: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    /// The account its session runs under here (its name).
    #[serde(default)]
    pub account_name: Option<String>,
    /// The guard: its globs (relative to the worktree), read-only.
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub read_only: bool,
    /// It runs without permission prompts.
    #[serde(default)]
    pub dangerous: bool,
    /// Writes the guard denied.
    #[serde(default)]
    pub denials: u32,
    /// Results made (one per `task_done` that changed something), the task
    /// the newest was made for, and how many the lead took.
    #[serde(default)]
    pub results: u32,
    #[serde(default)]
    pub result_task: Option<u64>,
    #[serde(default)]
    pub landed: u32,
    /// Why the worktree kept its own work rather than the lead's newer code
    /// at the last start.
    #[serde(default)]
    pub kept_note: Option<String>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub started_at: Option<i64>,
    #[serde(default)]
    pub stopped_at: Option<i64>,
    /// When its lead was last heard from (a look, a message, a start, an
    /// answered board call): a session it no longer looks after is stopped
    /// once idle (`host::stop_unattended`).
    #[serde(default)]
    pub heard_at: i64,
    /// Whether the lead asked for it over the local network (the `run`
    /// grant's column that let it in).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_lan: Option<bool>,
    #[serde(default)]
    pub updated_at: i64,
    // ── kept, never shown ──
    /// The binding's secret (what the lead checks).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub secret: String,
    /// The account's id here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// The code the worktree was set to last, and its tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_tree: Option<String>,
    /// Commits the lead has (results leave them out).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub haves: Vec<String>,
    /// The last result made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<TaskTip>,
    /// The newest result's files and the code it was made on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub result_files: Vec<FileSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_base: Option<String>,
    /// What its session was last started with (a message resumes it so).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<RoleSpec>,
}

/// Fields that stay in the record.
const INTERNAL: [&str; 10] = [
    "secret",
    "account",
    "base",
    "baseTree",
    "haves",
    "last",
    "resultFiles",
    "resultBase",
    "spec",
    "via_lan",
];

impl HostedRecord {
    /// What a list shows: no internals.
    pub fn view(&self) -> Value {
        let mut v = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Some(o) = v.as_object_mut() {
            for k in INTERNAL {
                o.remove(k);
            }
        }
        v
    }
}

/// A role in memory: its record and one piece of work on it at a time
/// (setting its worktree, taking a result).
pub struct Hosted {
    pub binding: String,
    record: Mutex<HostedRecord>,
    pub(crate) work: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for Hosted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hosted")
            .field("binding", &self.binding)
            .finish()
    }
}

impl Hosted {
    fn new(record: HostedRecord) -> Arc<Self> {
        Arc::new(Self {
            binding: record.binding.clone(),
            record: Mutex::new(record),
            work: tokio::sync::Mutex::new(()),
        })
    }

    pub fn record(&self) -> HostedRecord {
        self.record
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn peek<R>(&self, f: impl FnOnce(&HostedRecord) -> R) -> R {
        f(&self.record.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub(crate) fn update<R>(&self, f: impl FnOnce(&mut HostedRecord) -> R) -> R {
        let mut r = self.record.lock().unwrap_or_else(|e| e.into_inner());
        let out = f(&mut r);
        r.updated_at = crate::machines::now();
        out
    }
}

/// The roles this computer runs for others (`WorkState::roles`); at the
/// lead's, only the folders its transfers use.
pub struct Roles {
    dir: Option<PathBuf>,
    all: Mutex<HashMap<String, Arc<Hosted>>>,
    /// Incoming chunks are written one at a time.
    pub(crate) chunks: tokio::sync::Mutex<()>,
    /// Bindings whose result the lead is taking now.
    pub(crate) landing: Mutex<std::collections::HashSet<String>>,
    /// While sessions are only recorded (tests): what was started, said
    /// and stopped.
    spawns: Mutex<Vec<Value>>,
    messages: Mutex<Vec<Value>>,
    stops: Mutex<Vec<String>>,
}

impl std::fmt::Debug for Roles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Roles").finish_non_exhaustive()
    }
}

impl Roles {
    /// The records under `<work dir>/roles`.
    pub fn load(work_dir: Option<&Path>) -> Self {
        let dir = work_dir.map(|d| d.join(DIR));
        let mut all = HashMap::new();
        if let Some(d) = &dir {
            for r in load_records(d) {
                all.insert(r.binding.clone(), Hosted::new(r));
            }
        }
        Self {
            dir,
            all: Mutex::new(all),
            chunks: tokio::sync::Mutex::new(()),
            landing: Mutex::new(std::collections::HashSet::new()),
            spawns: Mutex::new(Vec::new()),
            messages: Mutex::new(Vec::new()),
            stops: Mutex::new(Vec::new()),
        }
    }

    /// A binding's own folder: its record, what arrives and leaves, its
    /// worktree.
    pub fn dir_of(&self, binding: &str) -> Option<PathBuf> {
        is_binding_id(binding)
            .then(|| self.dir.as_ref().map(|d| d.join(binding)))
            .flatten()
    }

    pub fn get(&self, binding: &str) -> Option<Arc<Hosted>> {
        self.all
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(binding)
            .cloned()
    }

    /// Every role this computer runs for another, newest first.
    pub fn all(&self) -> Vec<Arc<Hosted>> {
        let mut out: Vec<(i64, Arc<Hosted>)> = self
            .all
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .map(|h| (h.peek(|r| r.created_at), h.clone()))
            .collect();
        out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.binding.cmp(&b.1.binding)));
        out.into_iter().map(|(_, h)| h).collect()
    }

    /// The role whose session is `session` here.
    pub fn of_session(&self, session: &str) -> Option<Arc<Hosted>> {
        self.all
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .find(|h| h.peek(|r| r.session.as_deref() == Some(session)))
            .cloned()
    }

    pub(crate) fn insert(&self, record: HostedRecord) -> Option<Arc<Hosted>> {
        let mut all = self.all.lock().unwrap_or_else(|e| e.into_inner());
        if all.contains_key(&record.binding) {
            return None;
        }
        let h = Hosted::new(record);
        all.insert(h.binding.clone(), h.clone());
        Some(h)
    }

    pub(crate) fn forget(&self, binding: &str) {
        self.all
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(binding);
    }

    /// Writes a role's record (0600).
    pub(crate) fn save(&self, h: &Hosted) {
        let Some(dir) = self.dir_of(&h.binding) else {
            return;
        };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!(binding = %h.binding, error = %e, "a role's folder could not be made");
            return;
        }
        crate::claude::transfer::private_dir(&dir);
        if let Err(e) = crate::persist::write_json_private(&dir.join(RECORD), &h.record()) {
            tracing::warn!(binding = %h.binding, error = %e, "a role's record could not be saved");
        }
    }

    pub(crate) fn record_spawn(&self, v: Value) {
        self.spawns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(v);
    }

    pub fn recorded_spawns(&self) -> Vec<Value> {
        self.spawns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub(crate) fn record_message(&self, v: Value) {
        self.messages
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(v);
    }

    pub fn recorded_messages(&self) -> Vec<Value> {
        self.messages
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub(crate) fn record_stop(&self, session: &str) {
        self.stops
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(session.to_string());
    }

    pub fn recorded_stops(&self) -> Vec<String> {
        self.stops.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

fn load_records(dir: &Path) -> Vec<HostedRecord> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let now = crate::machines::now();
    let mut out = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !is_binding_id(&name) {
            continue;
        }
        let record: Option<HostedRecord> = crate::persist::read_json(&e.path().join(RECORD));
        let Some(mut r) = record.filter(|r| r.binding == name) else {
            // The lead's own transfers, or what is left of a removal.
            continue;
        };
        if matches!(
            r.state,
            HostState::Sending | HostState::Starting | HostState::Running
        ) {
            // Its session ended with the app: the lead starts it again.
            r.state = HostState::Stopped;
            r.stopped_at = Some(now);
            let _ = crate::persist::write_json_private(&e.path().join(RECORD), &r);
        }
        out.push(r);
    }
    out
}

// ─── shared ────────────────────────────────────────────────────────────────────

/// Roles changed: dashboards refetch `/api/machines/roles`.
pub fn publish(state: &AppState) {
    crate::machines::publish(state, "roles");
}

/// A role's audit line on the computer that runs it: the team, the role,
/// the lead's computer — never a task's text.
pub fn audit(state: &AppState, action: &str, r: &HostedRecord) {
    state.devices.audit.record(AuditEntry {
        action: action.to_string(),
        device_id: Some(r.lead.clone()),
        device_name: Some(crate::machines::peer_name(state, &r.lead)),
        detail: Some(json!({
            "binding": r.binding,
            "team": r.team,
            "role": r.role,
            "session": r.session,
            "folder": r.folder,
            "state": r.state,
            "permissionMode": r.permission_mode,
            "dangerous": r.dangerous,
            "results": r.results,
        })),
        ..AuditEntry::now()
    });
}

/// Whether a role's session is up here, and in a turn. (While sessions are
/// only recorded — tests — a started one is up and between turns until it
/// is stopped.)
pub fn liveness(state: &AppState, r: &HostedRecord) -> (bool, bool) {
    let Some(s) = r.session.as_deref() else {
        return (false, false);
    };
    if !state.machines.work.tasks.sessions_live() {
        return (r.state == HostState::Running, false);
    }
    match state.control.is_idle(s) {
        Some(idle) => (true, !idle),
        None => (false, false),
    }
}

/// The guard's rule for a role session this computer runs for another one:
/// the role's globs relative to its worktree here. The lead is on the other
/// computer, so the reason never names a session to message.
pub fn guard_rule(state: &AppState, session: &str) -> Option<Rule> {
    let h = state.machines.work.roles.of_session(session)?;
    let r = h.record();
    let rule = Rule {
        team: r.team,
        role: r.role,
        role_title: r.role_title,
        base: r.folder,
        globs: r.paths,
        read_only: r.read_only,
        lead_name: None,
    };
    rule.limits().then_some(rule)
}

/// Count a guard denial on a role this computer runs (the lead hears it
/// with the role's status).
pub fn note_denial(state: &AppState, session: &str) {
    if let Some(h) = state.machines.work.roles.of_session(session) {
        h.update(|r| r.denials = r.denials.saturating_add(1));
        state.machines.work.roles.save(&h);
    }
}

/// The role sessions this computer runs for others, as `capacity::admit`
/// weighs them here: they run here, so they count here (never at the
/// lead's).
pub fn running(state: &AppState) -> Vec<RunningWorker> {
    state
        .machines
        .work
        .roles
        .all()
        .into_iter()
        .filter_map(|h| {
            let r = h.record();
            if !liveness(state, &r).0 {
                return None;
            }
            Some(RunningWorker {
                session_id: r.session.clone()?,
                project: r.checkout.clone(),
                role: r.role.clone(),
                model: state
                    .control
                    .state(r.session.as_deref().unwrap_or_default())
                    .and_then(|l| l.model)
                    .or(r.model.clone())
                    .unwrap_or_default(),
                lead: false,
                for_machine: Some(if r.lead_name.is_empty() {
                    crate::machines::peer_name(state, &r.lead)
                } else {
                    r.lead_name.clone()
                }),
            })
        })
        .collect()
}

/// `GET /api/machines/roles`: the roles this computer runs for others.
pub fn view(state: &AppState) -> Value {
    let roles: Vec<Value> = state
        .machines
        .work
        .roles
        .all()
        .iter()
        .map(|h| {
            let r = h.record();
            let (alive, busy) = liveness(state, &r);
            let mut v = r.view();
            v["alive"] = json!(alive);
            v["busy"] = json!(busy);
            v["leadName"] = json!(crate::machines::peer_name(state, &r.lead));
            v
        })
        .collect();
    json!({ "roles": roles })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str, name: &str, os: &str, online: bool, may_run: bool) -> Candidate {
        Candidate {
            id: id.into(),
            name: name.into(),
            display: name.into(),
            os: os.into(),
            online,
            may_run,
        }
    }

    #[test]
    fn ids_and_secrets_have_their_shapes() {
        let (id, secret) = new_binding();
        assert!(is_binding_id(&id), "{id}");
        assert_eq!(secret.len(), 64);
        assert!(secret.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(new_binding().0, id);
        for bad in [
            "rb_",
            "rb_0123",
            "rb_0123456789ABCDEF",
            "tm_0123456789abcdef",
            "",
        ] {
            assert!(!is_binding_id(bad), "{bad}");
        }
        assert_eq!(secret_sha256(&secret).len(), 64);
    }

    #[test]
    fn a_binding_holds_only_for_its_team_role_computer_secret_and_session() {
        let (binding, secret) = new_binding();
        let sha = secret_sha256(&secret);
        let key = BindingKey {
            binding: &binding,
            machine: "b-machine",
            team: "tm_0123456789ab",
            role: "windows",
            secret_sha256: &sha,
            session: Some("s-1"),
        };
        let good = BindingClaim {
            from: "b-machine",
            binding: &binding,
            secret: &secret,
            team: "tm_0123456789ab",
            role: "windows",
            session: "s-1",
        };
        assert_eq!(check_binding(&key, &good), Ok(()));
        let (other, other_secret) = new_binding();
        let cases = [
            (
                BindingClaim {
                    binding: &other,
                    ..good.clone()
                },
                BindingRefusal::Binding,
            ),
            (
                BindingClaim {
                    from: "c-machine",
                    ..good.clone()
                },
                BindingRefusal::Computer,
            ),
            (
                BindingClaim {
                    team: "tm_ffffffffffff",
                    ..good.clone()
                },
                BindingRefusal::Team,
            ),
            (
                BindingClaim {
                    role: "lead",
                    ..good.clone()
                },
                BindingRefusal::Role,
            ),
            (
                BindingClaim {
                    secret: &other_secret,
                    ..good.clone()
                },
                BindingRefusal::Secret,
            ),
            (
                BindingClaim {
                    secret: "",
                    ..good.clone()
                },
                BindingRefusal::Secret,
            ),
            (
                BindingClaim {
                    session: "s-old",
                    ..good.clone()
                },
                BindingRefusal::Session,
            ),
        ];
        for (claim, why) in cases {
            assert_eq!(check_binding(&key, &claim), Err(why), "{claim:?}");
        }
        // No session started there yet: nothing acts for the role.
        let unstarted = BindingKey {
            session: None,
            ..key.clone()
        };
        assert_eq!(
            check_binding(&unstarted, &good),
            Err(BindingRefusal::Session)
        );
    }

    #[test]
    fn a_role_s_machine_names_a_computer_or_the_first_ready_one_of_an_os() {
        let all = vec![
            candidate("m-mac", "MacBook", "macos", true, true),
            candidate("m-win1", "DESKTOP-PC", "windows", false, true),
            candidate("m-win2", "BUILD-BOX", "windows", true, true),
            candidate("m-win3", "OLD-PC", "windows", true, false),
        ];
        assert_eq!(parse_spec("  "), None);
        assert_eq!(
            parse_spec("os:Windows"),
            Some(MachineSpec::Os("windows".into()))
        );
        let named = |s: &str| parse_spec(s).unwrap();
        // By name (any case) or id, whatever its state.
        assert_eq!(pick(&named("desktop-pc"), &all, None).unwrap().id, "m-win1");
        assert_eq!(pick(&named("m-mac"), &all, None).unwrap().id, "m-mac");
        assert_eq!(pick(&named("Nobody"), &all, None), Err(Unresolved::Unknown));
        // os: the first online one that lets this computer run…
        assert_eq!(pick(&named("os:windows"), &all, None).unwrap().id, "m-win2");
        assert_eq!(pick(&named("os:macos"), &all, None).unwrap().id, "m-mac");
        // …unless the role ran on one of that OS before: it stays there.
        assert_eq!(
            pick(&named("os:windows"), &all, Some("m-win1")).unwrap().id,
            "m-win1"
        );
        assert_eq!(
            pick(&named("os:windows"), &all, Some("m-mac")).unwrap().id,
            "m-win2",
            "a computer of another OS is no preference"
        );
        assert_eq!(
            pick(&named("os:linux"), &all, None),
            Err(Unresolved::NoneOfOs("linux".into()))
        );
        let offline = vec![
            candidate("m-win1", "DESKTOP-PC", "windows", false, true),
            candidate("m-win3", "OLD-PC", "windows", true, false),
        ];
        let e = pick(&named("os:windows"), &offline, None).unwrap_err();
        assert_eq!(
            e,
            Unresolved::NoneReady("windows".into(), vec!["DESKTOP-PC".into(), "OLD-PC".into()])
        );
        let text = unresolved_text("os:windows", &e);
        assert!(
            text.contains("No Windows computer can run the role now"),
            "{text}"
        );
        assert!(text.contains("Run permission"), "{text}");
        assert!(unresolved_text("PC-9", &Unresolved::Unknown).contains("\"PC-9\""));
    }
}
