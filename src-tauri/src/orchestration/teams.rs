//! Teams (docs/TEAMS.md "Concepts"): a workflow template applied to a
//! project folder, an account and per-role overrides; each role has at most
//! one session at a time — an owned session Mother Claude launches and
//! stops, or a foreign session that took the role from the dashboard (never
//! started or stopped by Mother Claude: the hard invariant).
//!
//! Teams are kept in `<app data dir>/teams.json` (0600, atomic); what changes
//! by the second (who waits for capacity and why, starts in flight, guard
//! denials) is runtime only. `state.teams` is this module's [`Teams`].

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::board::{BoardStore, Task, TaskStatus};
use super::guard::Rule;
use super::router::ModelChoice;
use super::templates::{Integration, RoleDef, Template, TemplateStore};
use crate::state::AppState;

/// Earlier sessions remembered per role.
const PREVIOUS_KEPT: usize = 10;
/// Observed agent-team hook events kept (interactive sessions' teams).
const OBSERVED_KEPT: usize = 200;

/// Per-role changes a team is started with.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RoleOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autostart: Option<bool>,
}

impl RoleOverride {
    /// A role with these changes.
    pub fn apply(&self, role: &RoleDef) -> RoleDef {
        let mut r = role.clone();
        if let Some(m) = &self.model {
            r.model = Some(m.clone());
        }
        if let Some(e) = &self.effort {
            r.effort = Some(e.clone());
        }
        if let Some(m) = &self.permission_mode {
            r.permission_mode = Some(m.clone());
        }
        if let Some(b) = self.budget_usd {
            r.budget_usd = Some(b);
        }
        if let Some(a) = self.autostart {
            r.autostart = a;
        }
        r
    }
}

/// Who started a team: its later launches run with their rights.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Starter {
    /// The desktop (free working folders).
    pub desktop: bool,
    /// May use dangerous actions (bypass-class roles).
    pub may_dangerous: bool,
    /// For the history ("desktop", a device's name).
    pub label: String,
}

/// A role's session, as far as it is remembered.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RoleSlot {
    /// Its current (or last) session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// A foreign session took the role (never started or stopped here).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub foreign: bool,
    /// Earlier sessions of the role (forks), newest last.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub previous: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped_at: Option<i64>,
    /// What its session was launched with, and why.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_reason: Option<String>,
    /// Someone asked for it (start, `team_request_role`) even without work.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub wanted: bool,
    /// Where its worktree is, once it ran in one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// A team.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Team {
    pub id: String,
    pub project: String,
    /// The project as its role sessions' names show it (`<label>/<role>`),
    /// unique among the teams.
    pub label: String,
    pub template_id: String,
    /// The template as applied, overrides merged: editing the template
    /// later does not change this team.
    pub template: Template,
    pub account: String,
    #[serde(default)]
    pub overrides: BTreeMap<String, RoleOverride>,
    pub created_at: i64,
    /// Stopped: no session runs or starts until it is resumed.
    #[serde(default)]
    pub stopped: bool,
    #[serde(default)]
    pub starter: Starter,
    #[serde(default)]
    pub roles: BTreeMap<String, RoleSlot>,
    /// The user's brief: the lead's first message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl Team {
    pub fn lead(&self) -> Option<&RoleDef> {
        self.template.lead()
    }

    pub fn role(&self, id: &str) -> Option<&RoleDef> {
        self.template.role(id)
    }

    pub fn role_ids(&self) -> Vec<String> {
        self.template.roles.iter().map(|r| r.id.clone()).collect()
    }

    /// A role session's name (`-n`, what `SendMessage` and `ListAgents`
    /// use).
    pub fn session_name(&self, role: &str) -> String {
        format!("{}/{}", self.label, role)
    }

    pub fn slot(&self, role: &str) -> RoleSlot {
        self.roles.get(role).cloned().unwrap_or_default()
    }

    /// The role a session holds in this team.
    pub fn role_of(&self, session: &str) -> Option<&str> {
        self.roles
            .iter()
            .find(|(_, s)| s.session_id.as_deref() == Some(session))
            .map(|(r, _)| r.as_str())
    }

    /// The folder a role's globs are relative to: its worktree, else the
    /// project.
    pub fn base_of(&self, role: &RoleDef) -> String {
        if !role.worktree {
            return self.project.clone();
        }
        self.slot(&role.id)
            .worktree_path
            .unwrap_or_else(|| worktree_dir(&self.project, &role.id))
    }
}

/// Where Claude Code puts a role's worktree (`-w <role>`: verified,
/// docs/TEAMS.md "Spike").
pub fn worktree_dir(project: &str, role: &str) -> String {
    Path::new(project)
        .join(".claude")
        .join("worktrees")
        .join(role)
        .to_string_lossy()
        .into_owned()
}

/// A session's place in a team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub team: String,
    pub project: String,
    pub role: String,
    pub lead: bool,
    pub foreign: bool,
}

/// Waiting for capacity (`capacity::admit` said not now).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Waiting {
    pub reasons: Vec<String>,
    pub since: i64,
    /// The "waits for capacity" push went out for this wait.
    pub pushed: bool,
}

/// What changes by the second about a role.
#[derive(Debug, Clone, Default)]
pub struct RoleRuntime {
    pub waiting: Option<Waiting>,
    pub starting: bool,
    pub last_error: Option<String>,
    /// No new attempt before this (epoch ms) after a failed start.
    pub retry_after: Option<i64>,
    /// When its session was last seen between turns with nothing to do.
    pub idle_since: Option<i64>,
    pub denials: u32,
    /// Board notices for a lead that was not running: delivered with its
    /// next start.
    pub pending_notices: Vec<String>,
}

/// An agent-team event an interactive session's hooks reported (observe
/// only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Observed {
    pub at: i64,
    pub session_id: String,
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub teammate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Default)]
struct TeamsInner {
    teams: BTreeMap<String, Team>,
    runtime: HashMap<(String, String), RoleRuntime>,
    observed: VecDeque<Observed>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct TeamsFile {
    teams: Vec<Team>,
}

/// The teams subsystem (`state.teams`).
#[derive(Debug, Default)]
pub struct Teams {
    data_dir: Option<PathBuf>,
    pub templates: TemplateStore,
    pub boards: BoardStore,
    inner: Mutex<TeamsInner>,
    /// Nudges the launcher's loop (a board change, a start request).
    pub(crate) wake: std::sync::Arc<tokio::sync::Notify>,
    pub(crate) started: AtomicBool,
}

impl Teams {
    /// Load the teams, templates and boards of the app data dir (`None`:
    /// memory only, nothing touches the disk).
    pub fn load(data_dir: Option<&Path>) -> Self {
        let mut inner = TeamsInner::default();
        if let Some(dir) = data_dir {
            if let Some(file) = crate::persist::read_json::<TeamsFile>(&dir.join("teams.json")) {
                for t in file.teams {
                    if super::templates::validate(&t.template).ok() {
                        inner.teams.insert(t.id.clone(), t);
                    } else {
                        tracing::warn!(team = %t.id, "a saved team's template does not validate; skipped");
                    }
                }
            }
        }
        Self {
            data_dir: data_dir.map(Path::to_path_buf),
            templates: TemplateStore::load(data_dir),
            boards: BoardStore::new(data_dir),
            inner: Mutex::new(inner),
            wake: std::sync::Arc::new(tokio::sync::Notify::new()),
            started: AtomicBool::new(false),
        }
    }

    fn lock(&self) -> MutexGuard<'_, TeamsInner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The app data dir (where agent definition files go), if any.
    pub fn data_dir(&self) -> Option<&Path> {
        self.data_dir.as_deref()
    }

    fn save(&self, inner: &TeamsInner) {
        let Some(dir) = &self.data_dir else {
            return;
        };
        let file = TeamsFile {
            teams: inner.teams.values().cloned().collect(),
        };
        if let Err(e) = crate::persist::write_json_private(&dir.join("teams.json"), &file) {
            tracing::warn!(error = %e, "could not save the teams");
        }
    }

    /// Wake the launcher's loop.
    pub fn nudge(&self) {
        self.wake.notify_one();
    }

    pub fn all(&self) -> Vec<Team> {
        self.lock().teams.values().cloned().collect()
    }

    pub fn get(&self, id: &str) -> Option<Team> {
        self.lock().teams.get(id).cloned()
    }

    /// The team of a project (one per project).
    pub fn for_project(&self, project: &str) -> Option<Team> {
        self.lock()
            .teams
            .values()
            .find(|t| crate::claude::paths::same_path(&t.project, project))
            .cloned()
    }

    /// Add a team (the caller checked everything).
    pub fn insert(&self, team: Team) {
        let mut inner = self.lock();
        inner.teams.insert(team.id.clone(), team);
        self.save(&inner);
    }

    /// Forget a team (its board stays with the project).
    pub fn remove(&self, id: &str) -> Option<Team> {
        let mut inner = self.lock();
        let out = inner.teams.remove(id);
        inner.runtime.retain(|(t, _), _| t != id);
        self.save(&inner);
        out
    }

    /// Change a team and save it; `None` when there is no such team.
    pub fn update<R>(&self, id: &str, f: impl FnOnce(&mut Team) -> R) -> Option<R> {
        let mut inner = self.lock();
        let team = inner.teams.get_mut(id)?;
        let out = f(team);
        self.save(&inner);
        Some(out)
    }

    /// A role's runtime state.
    pub fn runtime(&self, team: &str, role: &str) -> RoleRuntime {
        self.lock()
            .runtime
            .get(&(team.to_string(), role.to_string()))
            .cloned()
            .unwrap_or_default()
    }

    /// Change a role's runtime state.
    pub fn with_runtime<R>(
        &self,
        team: &str,
        role: &str,
        f: impl FnOnce(&mut RoleRuntime) -> R,
    ) -> R {
        let mut inner = self.lock();
        f(inner
            .runtime
            .entry((team.to_string(), role.to_string()))
            .or_default())
    }

    /// The project label a new team on `project` gets: its folder's name,
    /// made safe for a session name, unique among the teams.
    pub fn new_label(&self, project: &str) -> String {
        let base = label_of(project);
        let inner = self.lock();
        let taken = |l: &str| {
            inner
                .teams
                .values()
                .any(|t| t.label == l && !crate::claude::paths::same_path(&t.project, project))
        };
        if !taken(&base) {
            return base;
        }
        let hash = super::board::board_key(project);
        format!("{base}-{}", &hash[..4])
    }

    /// Where a session sits in a team, if anywhere.
    pub fn binding(&self, session: &str) -> Option<Binding> {
        let inner = self.lock();
        for team in inner.teams.values() {
            if let Some(role) = team.role_of(session) {
                let slot = team.slot(role);
                return Some(Binding {
                    team: team.id.clone(),
                    project: team.project.clone(),
                    role: role.to_string(),
                    lead: team.role(role).is_some_and(|r| r.lead),
                    foreign: slot.foreign,
                });
            }
        }
        None
    }

    /// What the guard holds a session to (a role session of a running
    /// team whose role limits anything). A stopped team's sessions are not
    /// held: one continued by hand is the user's again.
    pub fn guard_rule(&self, session: &str) -> Option<Rule> {
        let inner = self.lock();
        let team = inner
            .teams
            .values()
            .filter(|t| !t.stopped)
            .find(|t| t.role_of(session).is_some())?;
        let role = team.role(team.role_of(session)?)?;
        let rule = Rule {
            team: team.id.clone(),
            role: role.id.clone(),
            role_title: role.title.clone(),
            base: team.base_of(role),
            globs: role.paths.clone(),
            read_only: role.read_only,
            lead_name: team
                .lead()
                .filter(|l| l.id != role.id)
                .map(|l| team.session_name(&l.id)),
        };
        rule.limits().then_some(rule)
    }

    /// Count a guard denial on a role.
    pub fn note_denial(&self, team: &str, role: &str) {
        self.with_runtime(team, role, |r| r.denials += 1);
    }

    /// Record an agent-team event from an interactive session.
    pub fn observe(&self, ev: Observed) {
        let mut inner = self.lock();
        inner.observed.push_back(ev);
        while inner.observed.len() > OBSERVED_KEPT {
            inner.observed.pop_front();
        }
    }

    /// The agent-team events observed, newest last.
    pub fn observed(&self) -> Vec<Observed> {
        self.lock().observed.iter().cloned().collect()
    }

    /// Bind a session to a role (a launch, a fork, a "take role"); the
    /// role's previous session is remembered.
    pub fn bind(&self, team: &str, role: &str, session: &str, foreign: bool, now: i64) {
        self.update(team, |t| {
            let slot = t.roles.entry(role.to_string()).or_default();
            if let Some(old) = slot.session_id.take().filter(|o| o != session) {
                slot.previous.push(old);
                if slot.previous.len() > PREVIOUS_KEPT {
                    let excess = slot.previous.len() - PREVIOUS_KEPT;
                    slot.previous.drain(..excess);
                }
            }
            slot.session_id = Some(session.to_string());
            slot.foreign = foreign;
            slot.started_at = Some(now);
            slot.stopped_at = None;
        });
    }
}

/// A folder's name as a session name shows it: letters, digits, `.`, `_`
/// and `-` (others become `-`), at most 40, never starting with `-`.
pub fn label_of(project: &str) -> String {
    let name = crate::claude::paths::last_component(project).unwrap_or("project");
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .take(40)
        .collect();
    while out.starts_with(['-', '.']) {
        out.remove(0);
    }
    if out.is_empty() {
        "project".to_string()
    } else {
        out
    }
}

/// A new team id.
pub fn new_team_id() -> String {
    format!("tm_{}", &uuid::Uuid::new_v4().simple().to_string()[..12])
}

/// Whether a string is a team id.
pub fn is_team_id(s: &str) -> bool {
    s.len() == 15
        && s.starts_with("tm_")
        && s[3..]
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

// ─────────────────────────────── views ───────────────────────────────

/// A role's state as the Teams page shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RoleState {
    NotStarted,
    /// In the queue: capacity said not now (see the reasons).
    Waiting,
    Starting,
    /// A turn runs.
    Running,
    /// Its session is up, between turns.
    Idle,
    /// Its session ended (it resumes when there is work).
    Stopped,
}

/// One role of a team, for the Teams page.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleView {
    pub id: String,
    pub title: String,
    pub lead: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub state: RoleState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub session_name: String,
    pub foreign: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_reason: Option<String>,
    pub permission_mode: String,
    pub worktree: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
    pub paths: Vec<String>,
    pub read_only: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_usd: Option<f64>,
    /// The budget goes to its sessions (`--max-budget-usd`): the account
    /// pays per token.
    pub budget_applied: bool,
    pub autostart: bool,
    pub classes: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub wait_reasons: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting_since: Option<i64>,
    /// The task its session holds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_task: Option<u64>,
    /// Tasks waiting for it.
    pub queued_tasks: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub guard_denials: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped_at: Option<i64>,
}

/// A team, for the Teams page.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamView {
    pub id: String,
    pub project: String,
    pub label: String,
    pub template_id: String,
    pub template_name: String,
    pub account: String,
    pub stopped: bool,
    pub created_at: i64,
    pub integration: Integration,
    pub idle_stop_minutes: u32,
    pub reviews: bool,
    pub roles: Vec<RoleView>,
    pub counts: BTreeMap<&'static str, usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
}

/// Whether a role session is alive, and whether it is in a turn.
pub(crate) async fn liveness(state: &AppState, slot: &RoleSlot) -> (bool, bool) {
    let Some(id) = slot.session_id.as_deref() else {
        return (false, false);
    };
    if slot.foreign {
        return match state.find_session(id).await {
            Some(s) if s.live => {
                use crate::claude::model::SessionStatus as S;
                (
                    true,
                    matches!(s.status, S::Working | S::Waiting | S::Starting),
                )
            }
            _ => (false, false),
        };
    }
    match state.control.is_idle(id) {
        Some(idle) => (true, !idle),
        None => (false, false),
    }
}

/// A team's view (its roles' states, the board's counts).
pub async fn view(state: &AppState, team: &Team) -> TeamView {
    let (counts, tasks): (BTreeMap<&'static str, usize>, Vec<Task>) = state
        .teams
        .boards
        .read(&team.project, |b| (b.counts(), b.tasks.clone()));
    let pays = super::launcher::pays_per_token(state, &team.account);
    let mut roles = Vec::new();
    for role in &team.template.roles {
        let slot = team.slot(&role.id);
        let rt = state.teams.runtime(&team.id, &role.id);
        let (alive, busy) = liveness(state, &slot).await;
        let role_state = if alive {
            if busy {
                RoleState::Running
            } else {
                RoleState::Idle
            }
        } else if rt.starting {
            RoleState::Starting
        } else if rt.waiting.is_some() {
            RoleState::Waiting
        } else if slot.session_id.is_some() {
            RoleState::Stopped
        } else {
            RoleState::NotStarted
        };
        let current = slot.session_id.as_deref().and_then(|sid| {
            tasks
                .iter()
                .filter(|t| t.status.held() && t.claimed_by.as_deref() == Some(sid))
                .map(|t| t.id)
                .min()
        });
        let queued = tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Todo && t.role.as_deref() == Some(role.id.as_str()))
            .count();
        // What it runs: the live session's model when it is up.
        let live_model = slot
            .session_id
            .as_deref()
            .and_then(|sid| state.control.state(sid))
            .and_then(|l| l.model);
        roles.push(RoleView {
            id: role.id.clone(),
            title: role.title.clone(),
            lead: role.lead,
            description: role.description.clone(),
            state: role_state,
            session_id: slot.session_id.clone(),
            session_name: team.session_name(&role.id),
            foreign: slot.foreign,
            model: live_model
                .or_else(|| slot.model.as_ref().map(|m| m.model.clone()))
                .or_else(|| role.model.clone()),
            effort: slot
                .model
                .as_ref()
                .and_then(|m| m.effort.clone())
                .or_else(|| role.effort.clone()),
            model_reason: slot.model_reason.clone(),
            permission_mode: role.mode().to_string(),
            worktree: role.worktree,
            worktree_path: role.worktree.then(|| team.base_of(role)),
            paths: role.paths.clone(),
            read_only: role.read_only,
            budget_usd: role.budget_usd,
            budget_applied: pays && role.budget_usd.is_some(),
            autostart: role.autostart,
            classes: role.classes.clone(),
            wait_reasons: rt
                .waiting
                .as_ref()
                .map(|w| w.reasons.clone())
                .unwrap_or_default(),
            waiting_since: rt.waiting.as_ref().map(|w| w.since),
            current_task: current,
            queued_tasks: queued,
            last_error: rt.last_error.clone(),
            guard_denials: rt.denials,
            started_at: slot.started_at,
            stopped_at: slot.stopped_at,
        });
    }
    TeamView {
        id: team.id.clone(),
        project: team.project.clone(),
        label: team.label.clone(),
        template_id: team.template_id.clone(),
        template_name: team.template.name.clone(),
        account: team.account.clone(),
        stopped: team.stopped,
        created_at: team.created_at,
        integration: team.template.integration.clone(),
        idle_stop_minutes: team.template.idle_stop_minutes(),
        reviews: team.template.reviews(),
        roles,
        counts,
        brief: team.brief.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn team(project: &str) -> Team {
        let template = super::super::templates::builtins().remove(0);
        Team {
            id: new_team_id(),
            project: project.into(),
            label: label_of(project),
            template_id: template.id.clone(),
            template,
            account: "default".into(),
            overrides: BTreeMap::new(),
            created_at: 1,
            stopped: false,
            starter: Starter::default(),
            roles: BTreeMap::new(),
            brief: None,
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn labels_and_ids_are_safe() {
        assert_eq!(label_of("/Users/me/my app!"), "my-app-");
        assert_eq!(label_of(r"C:\dev\Shop.Web"), "Shop.Web");
        assert_eq!(label_of("/x/-flag"), "flag");
        assert_eq!(label_of("/"), "project");
        let id = new_team_id();
        assert!(is_team_id(&id), "{id}");
        assert!(!is_team_id("tm_../../etc/pa"));
        assert!(!is_team_id("tm_ABCDEF123456"));
    }

    #[test]
    fn sessions_bind_to_roles_and_the_guard_rule_follows() {
        let dir = tempfile::tempdir().unwrap();
        let teams = Teams::load(Some(dir.path()));
        let t = team("/w/app");
        let id = t.id.clone();
        teams.insert(t);
        assert_eq!(teams.new_label("/w/app"), "app");
        assert_ne!(
            teams.new_label("/other/app"),
            "app",
            "two projects, two labels"
        );
        teams.bind(&id, "frontend", "s-front", false, 5);
        teams.bind(&id, "lead", "s-lead", false, 5);
        let b = teams.binding("s-front").unwrap();
        assert_eq!(
            (b.role.as_str(), b.lead, b.foreign),
            ("frontend", false, false)
        );
        assert!(teams.binding("s-lead").unwrap().lead);
        assert!(teams.binding("nobody").is_none());
        let rule = teams.guard_rule("s-front").unwrap();
        assert_eq!(rule.base, worktree_dir("/w/app", "frontend"));
        assert!(rule.globs.contains(&"src/app/**".to_string()));
        assert_eq!(rule.lead_name.as_deref(), Some("app/lead"));
        // The lead limits itself to docs; it names no lead to ask.
        assert!(teams.guard_rule("s-lead").unwrap().lead_name.is_none());
        // A stopped team holds nobody (a session continued by hand is the
        // user's again).
        teams.update(&id, |t| t.stopped = true);
        assert!(teams.guard_rule("s-front").is_none());
        teams.update(&id, |t| t.stopped = false);
        // A fork moves the role to its new session; the old one is
        // remembered and no longer bound.
        teams.bind(&id, "frontend", "s-front-2", false, 9);
        assert!(teams.binding("s-front").is_none());
        let slot = teams.get(&id).unwrap().slot("frontend");
        assert_eq!(slot.previous, vec!["s-front".to_string()]);
        // Saved privately and read back.
        let file = dir.path().join("teams.json");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let again = Teams::load(Some(dir.path()));
        assert_eq!(again.binding("s-front-2").unwrap().role, "frontend");
        assert!(again.for_project("/w/app").is_some());
        again.remove(&id);
        assert!(Teams::load(Some(dir.path())).all().is_empty());
    }

    #[test]
    fn overrides_change_only_what_they_name() {
        let t = super::super::templates::builtins().remove(0);
        let frontend = t.role("frontend").unwrap();
        let o = RoleOverride {
            model: Some("opus".into()),
            autostart: Some(true),
            ..Default::default()
        };
        let r = o.apply(frontend);
        assert_eq!(r.model.as_deref(), Some("opus"));
        assert!(r.autostart);
        assert_eq!(r.effort, frontend.effort);
        assert_eq!(r.paths, frontend.paths);
    }
}
