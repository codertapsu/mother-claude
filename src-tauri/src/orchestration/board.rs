//! The task board of a project (docs/TEAMS.md "The board"; plan §6.2): task
//! packets, claims, progress, results and history — the one truth the lead,
//! the role sessions and the user share.
//!
//! One board per project, in `<app data dir>/boards/<key>.json` (0600,
//! written atomically through [`crate::persist`]; `key` is a hash of the
//! folder), never in the project or an account's config directory.
//!
//! The operations are pure functions of a [`Board`] and an [`Actor`], so
//! who may do what is decided (and tested) here:
//!
//! * the **user** (desktop, a paired device, a local script) and the **lead**
//!   create, edit, reprioritise, reassign, cancel, accept and reopen tasks;
//! * a **worker** (a role session) claims a task of its own role, reports
//!   progress on the task it holds, marks it blocked, and finishes it —
//!   into `review` when the template reviews, else `done` — or fails it.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::router::ModelChoice;

/// Tasks kept per board; the oldest ended ones go first beyond it.
pub const MAX_TASKS: usize = 2000;
/// Progress notes kept per task (the newest).
pub const MAX_NOTES: usize = 50;
/// History entries kept per task (the newest).
pub const MAX_HISTORY: usize = 100;
const TITLE_MAX: usize = 200;
const GOAL_MAX: usize = 8000;
const ITEM_MAX: usize = 500;
const ITEMS_MAX: usize = 50;
const FILES_MAX: usize = 500;
const NOTE_MAX: usize = 2000;
const SUMMARY_MAX: usize = 8000;

/// Where a task is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Claimed,
    InProgress,
    /// Finished by its worker; waits for the lead's review.
    Review,
    Done,
    Failed,
    Cancelled,
    /// Written, nobody holds it. (An unknown status read from a newer
    /// version's board reads as this.)
    #[default]
    #[serde(other)]
    Todo,
}

impl TaskStatus {
    pub fn ended(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }

    /// Someone holds it and works on it.
    pub fn held(self) -> bool {
        matches!(self, Self::Claimed | Self::InProgress)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::Claimed => "claimed",
            Self::InProgress => "in_progress",
            Self::Review => "review",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "todo" => Self::Todo,
            "claimed" => Self::Claimed,
            "in_progress" => Self::InProgress,
            "review" => Self::Review,
            "done" => Self::Done,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }

    pub const ALL: [TaskStatus; 7] = [
        Self::Todo,
        Self::Claimed,
        Self::InProgress,
        Self::Review,
        Self::Done,
        Self::Failed,
        Self::Cancelled,
    ];
}

/// A progress note.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Note {
    pub at: i64,
    pub by: String,
    pub text: String,
}

/// One line of a task's history.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HistoryEntry {
    pub at: i64,
    pub by: String,
    /// `created`, `claimed`, `status`, `priority`, `role`, `edited`,
    /// `blocked`, `unblocked`, `escalated`, `model`, `assigned`.
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// What a finished task produced.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TaskReport {
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub changed_files: Vec<String>,
    /// Whether the integration gate passed, when one ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate_passed: Option<bool>,
}

/// One task: a packet and everything that happened to it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Task {
    pub id: u64,
    pub title: String,
    pub goal: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub acceptance: Vec<String>,
    /// The role it is for (a template's role id).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// A task class (`router::TaskClass`, camelCase).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    /// 0 (highest) … 3.
    pub priority: u8,
    pub status: TaskStatus,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<String>,
    /// The session that holds it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claimed_by: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub progress: Vec<Note>,
    /// Why it waits (a question for the lead or the user).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<TaskReport>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<HistoryEntry>,
    /// The model and effort it was handed to a session with, and why.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_reason: Option<String>,
    /// Set by the user: always wins (`router::TaskFacts::user_override`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_override: Option<ModelChoice>,
    /// Times it was sent back for a stronger model or more effort.
    pub escalations: u8,
    pub created_at: i64,
    pub updated_at: i64,
    pub created_by: String,
    /// When it was first claimed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Task {
    fn log(&mut self, at: i64, by: &str, action: &str, detail: Option<String>) {
        self.history.push(HistoryEntry {
            at,
            by: by.to_string(),
            action: action.to_string(),
            detail,
        });
        if self.history.len() > MAX_HISTORY {
            let excess = self.history.len() - MAX_HISTORY;
            self.history.drain(..excess);
        }
        self.updated_at = at;
    }

    /// The row `task_list` and the board columns show.
    pub fn row(&self) -> TaskRow {
        TaskRow {
            id: self.id,
            title: self.title.clone(),
            role: self.role.clone(),
            status: self.status,
            priority: self.priority,
            class: self.class.clone(),
            claimed_by: self.claimed_by.clone(),
            blocked: self.blocked.clone(),
            escalations: self.escalations,
            updated_at: self.updated_at,
        }
    }
}

/// A task in a list.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRow {
    pub id: u64,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub status: TaskStatus,
    pub priority: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claimed_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked: Option<String>,
    pub escalations: u8,
    pub updated_at: i64,
}

/// A project's board.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Board {
    pub project: String,
    pub next_id: u64,
    pub tasks: Vec<Task>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Who acts on the board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    /// The dashboard (desktop, a paired device, a local script).
    User(String),
    /// A role session of the team; `lead` for the team's lead.
    Session {
        session: String,
        role: String,
        lead: bool,
    },
    /// Mother Claude itself (handing a task to a session it starts).
    System,
}

impl Actor {
    /// How the history names it.
    pub fn label(&self) -> String {
        match self {
            Self::User(who) => format!("user ({who})"),
            Self::Session { role, lead, .. } if *lead => format!("{role} (lead)"),
            Self::Session { role, .. } => role.clone(),
            Self::System => "Mother Claude".to_string(),
        }
    }

    fn manages(&self) -> bool {
        matches!(self, Self::User(_) | Self::Session { lead: true, .. })
    }

    fn session(&self) -> Option<&str> {
        match self {
            Self::Session { session, .. } => Some(session),
            _ => None,
        }
    }
}

/// Why a board operation was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BoardError {
    #[error("No task {0} on this board.")]
    NotFound(u64),
    #[error("{0}")]
    Denied(String),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Conflict(String),
}

impl BoardError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "not_found",
            Self::Denied(_) => "denied",
            Self::Invalid(_) => "invalid_arguments",
            Self::Conflict(_) => "conflict",
        }
    }
}

/// What happened to a task (one operation can do several things).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeKind {
    Created,
    Claimed,
    Started,
    Note,
    Blocked,
    Unblocked,
    Review,
    Done,
    Failed,
    Cancelled,
    Reopened,
    Escalated,
    Edited,
    Reprioritised,
    Reassigned,
}

/// A task after an operation, and what the operation did.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub task: Task,
    pub kinds: Vec<ChangeKind>,
    pub actor: Actor,
    /// Its status before.
    pub before: TaskStatus,
    /// The session that held it before (a reassignment or cancel tells it).
    pub held_by: Option<String>,
}

impl Change {
    pub fn is(&self, k: ChangeKind) -> bool {
        self.kinds.contains(&k)
    }

    /// It ended with this operation.
    pub fn ended(&self) -> bool {
        self.task.status.ended() && !self.before.ended()
    }
}

/// Text as the board keeps it: control characters out (line breaks and
/// tabs stay), trimmed, at most `max` characters.
pub fn clean(s: &str, max: usize) -> String {
    let kept: String = s
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect();
    kept.trim().chars().take(max).collect()
}

fn clean_list(items: &[String], max_items: usize, max_len: usize) -> Vec<String> {
    items
        .iter()
        .map(|s| clean(s, max_len))
        .filter(|s| !s.is_empty())
        .take(max_items)
        .collect()
}

fn valid_class(c: &str) -> bool {
    super::templates::TASK_CLASSES.contains(&c)
}

/// A new task.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TaskDraft {
    pub title: String,
    pub goal: String,
    pub acceptance: Vec<String>,
    pub role: Option<String>,
    pub class: Option<String>,
    pub priority: Option<u8>,
    pub files: Vec<String>,
    pub constraints: Vec<String>,
    /// The user's model choice for it.
    pub model: Option<ModelChoice>,
}

/// A change to a task. Absent fields are left alone.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TaskPatch {
    pub status: Option<TaskStatus>,
    /// A progress note (or, from the lead or the user, a note to the worker).
    pub note: Option<String>,
    /// Non-empty: blocked, with the reason; empty: no longer blocked.
    pub blocked: Option<String>,
    pub priority: Option<u8>,
    pub role: Option<String>,
    pub title: Option<String>,
    pub goal: Option<String>,
    pub acceptance: Option<Vec<String>>,
    pub files: Option<Vec<String>>,
    pub constraints: Option<Vec<String>>,
    pub class: Option<String>,
    /// With `status: todo`: the next run gets a stronger model or more
    /// effort (the escalation count rises).
    pub escalate: bool,
    /// The user's model choice (`{"model": null}`-like clearing is not
    /// offered: set another).
    pub model: Option<ModelChoice>,
}

/// How a worker finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    #[default]
    Done,
    Failed,
}

impl Board {
    pub fn new(project: &str) -> Self {
        Self {
            project: project.to_string(),
            next_id: 1,
            ..Default::default()
        }
    }

    pub fn get(&self, id: u64) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }

    fn get_mut(&mut self, id: u64) -> Result<&mut Task, BoardError> {
        self.tasks
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or(BoardError::NotFound(id))
    }

    /// Counts per status.
    pub fn counts(&self) -> BTreeMap<&'static str, usize> {
        let mut out: BTreeMap<&'static str, usize> =
            TaskStatus::ALL.iter().map(|s| (s.as_str(), 0)).collect();
        for t in &self.tasks {
            *out.entry(t.status.as_str()).or_default() += 1;
        }
        out
    }

    /// The task a role should take next: `todo`, for that role, not blocked,
    /// highest priority first, then oldest.
    pub fn next_for(&self, role: &str) -> Option<&Task> {
        self.tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Todo && t.role.as_deref() == Some(role))
            .filter(|t| t.blocked.is_none())
            .min_by_key(|t| (t.priority, t.id))
    }

    /// The task a session holds (claimed or in progress), if any.
    pub fn held_by(&self, session: &str) -> Option<&Task> {
        self.tasks
            .iter()
            .filter(|t| t.status.held() && t.claimed_by.as_deref() == Some(session))
            .min_by_key(|t| (t.priority, t.id))
    }

    /// Create a task. Workers do not create tasks: they ask the lead.
    pub fn create(
        &mut self,
        draft: TaskDraft,
        actor: &Actor,
        roles: &[String],
        now: i64,
    ) -> Result<Change, BoardError> {
        if !actor.manages() && *actor != Actor::System {
            return Err(BoardError::Denied(
                "Only the lead (or the user) writes tasks; send the lead a message instead.".into(),
            ));
        }
        let title = clean(&draft.title, TITLE_MAX);
        if title.is_empty() {
            return Err(BoardError::Invalid("A task needs a title.".into()));
        }
        let role = draft
            .role
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty());
        if let Some(r) = role {
            if !roles.iter().any(|x| x == r) {
                return Err(BoardError::Invalid(format!(
                    "No role {r:?} in this team (roles: {}).",
                    roles.join(", ")
                )));
            }
        }
        if let Some(c) = &draft.class {
            if !valid_class(c) {
                return Err(BoardError::Invalid(format!(
                    "{c:?} is not a task class ({}).",
                    super::templates::TASK_CLASSES.join(", ")
                )));
            }
        }
        let priority = draft.priority.unwrap_or(2);
        if priority > 3 {
            return Err(BoardError::Invalid("Priority is 0 (highest) to 3.".into()));
        }
        let id = self.next_id.max(1);
        self.next_id = id + 1;
        let by = actor.label();
        let mut task = Task {
            id,
            title,
            goal: clean(&draft.goal, GOAL_MAX),
            acceptance: clean_list(&draft.acceptance, ITEMS_MAX, ITEM_MAX),
            role: role.map(str::to_string),
            class: draft.class,
            priority,
            status: TaskStatus::Todo,
            files: clean_list(&draft.files, FILES_MAX, ITEM_MAX),
            constraints: clean_list(&draft.constraints, ITEMS_MAX, ITEM_MAX),
            model_override: draft.model,
            created_at: now,
            updated_at: now,
            created_by: by.clone(),
            ..Default::default()
        };
        task.log(
            now,
            &by,
            "created",
            task.role.clone().map(|r| format!("for {r}")),
        );
        self.tasks.push(task.clone());
        self.prune();
        Ok(Change {
            task,
            kinds: vec![ChangeKind::Created],
            actor: actor.clone(),
            before: TaskStatus::Todo,
            held_by: None,
        })
    }

    /// A session takes a task of its role. Claiming the task it already
    /// holds is a no-op.
    pub fn claim(&mut self, id: u64, actor: &Actor, now: i64) -> Result<Change, BoardError> {
        let (session, role) = match actor {
            Actor::Session { session, role, .. } => (session.clone(), role.clone()),
            _ => {
                return Err(BoardError::Denied(
                    "Only a role session claims tasks (assign the role instead).".into(),
                ))
            }
        };
        let task = self.get_mut(id)?;
        if task.status.held() && task.claimed_by.as_deref() == Some(session.as_str()) {
            return Ok(Change {
                task: task.clone(),
                kinds: Vec::new(),
                actor: actor.clone(),
                before: task.status,
                held_by: Some(session),
            });
        }
        if task.status != TaskStatus::Todo {
            return Err(BoardError::Conflict(format!(
                "Task {id} is {}, not todo: it cannot be claimed.",
                task.status.as_str()
            )));
        }
        if let Some(r) = &task.role {
            if r != &role {
                return Err(BoardError::Denied(format!(
                    "Task {id} is for the {r} role, not {role}."
                )));
            }
        }
        if task.blocked.is_some() {
            return Err(BoardError::Conflict(format!(
                "Task {id} is blocked: {}",
                task.blocked.clone().unwrap_or_default()
            )));
        }
        let before = task.status;
        task.status = TaskStatus::Claimed;
        task.claimed_by = Some(session.clone());
        task.started_at.get_or_insert(now);
        if task.role.is_none() {
            task.role = Some(role.clone());
        }
        let by = actor.label();
        task.log(now, &by, "claimed", None);
        Ok(Change {
            task: task.clone(),
            kinds: vec![ChangeKind::Claimed],
            actor: actor.clone(),
            before,
            held_by: None,
        })
    }

    /// Mother Claude hands a task to the session it starts (or wakes) for
    /// it: claimed for that session.
    pub fn assign(&mut self, id: u64, session: &str, now: i64) -> Result<Change, BoardError> {
        let task = self.get_mut(id)?;
        if task.status != TaskStatus::Todo {
            return Err(BoardError::Conflict(format!(
                "Task {id} is {}, not todo.",
                task.status.as_str()
            )));
        }
        let before = task.status;
        task.status = TaskStatus::Claimed;
        task.claimed_by = Some(session.to_string());
        task.started_at.get_or_insert(now);
        task.log(now, "Mother Claude", "assigned", Some(short_id(session)));
        Ok(Change {
            task: task.clone(),
            kinds: vec![ChangeKind::Claimed],
            actor: Actor::System,
            before,
            held_by: None,
        })
    }

    /// Record the model a task was handed over with.
    pub fn set_model(&mut self, id: u64, choice: ModelChoice, reason: &str, now: i64) {
        if let Ok(task) = self.get_mut(id) {
            let detail = match &choice.effort {
                Some(e) => format!("{} @ {e} ({reason})", choice.model),
                None => format!("{} ({reason})", choice.model),
            };
            if task.model.as_ref() != Some(&choice) {
                task.log(now, "Mother Claude", "model", Some(detail));
            }
            task.model = Some(choice);
            task.model_reason = Some(reason.chars().take(200).collect());
        }
    }

    /// Change a task.
    pub fn update(
        &mut self,
        id: u64,
        patch: TaskPatch,
        actor: &Actor,
        roles: &[String],
        now: i64,
    ) -> Result<Change, BoardError> {
        let manages = actor.manages();
        let task = self.get_mut(id)?;
        let before = task.status;
        let held_by = task.claimed_by.clone();
        let holder = actor.session().is_some() && actor.session() == task.claimed_by.as_deref();
        if !manages && !holder {
            return Err(BoardError::Denied(format!(
                "Task {id} is not yours: only the session that holds it, the lead or the user \
                 can change it."
            )));
        }
        let by = actor.label();
        let mut kinds = Vec::new();
        // What only the lead or the user may change.
        let managed = patch.priority.is_some()
            || patch.role.is_some()
            || patch.title.is_some()
            || patch.goal.is_some()
            || patch.acceptance.is_some()
            || patch.constraints.is_some()
            || patch.class.is_some()
            || patch.model.is_some()
            || patch.escalate;
        if managed && !manages {
            return Err(BoardError::Denied(
                "Only the lead or the user changes a task's packet, priority, role or model."
                    .into(),
            ));
        }
        if let Some(p) = patch.priority {
            if p > 3 {
                return Err(BoardError::Invalid("Priority is 0 (highest) to 3.".into()));
            }
            if p != task.priority {
                task.log(
                    now,
                    &by,
                    "priority",
                    Some(format!("P{} → P{p}", task.priority)),
                );
                task.priority = p;
                kinds.push(ChangeKind::Reprioritised);
            }
        }
        if let Some(r) = patch.role.as_deref().map(str::trim) {
            if !roles.iter().any(|x| x == r) {
                return Err(BoardError::Invalid(format!(
                    "No role {r:?} in this team (roles: {}).",
                    roles.join(", ")
                )));
            }
            if task.role.as_deref() != Some(r) {
                task.log(now, &by, "role", Some(r.to_string()));
                task.role = Some(r.to_string());
                // Whoever held it no longer does.
                if task.status.held() {
                    task.status = TaskStatus::Todo;
                    task.claimed_by = None;
                }
                kinds.push(ChangeKind::Reassigned);
            }
        }
        if let Some(c) = &patch.class {
            if !valid_class(c) {
                return Err(BoardError::Invalid(format!("{c:?} is not a task class.")));
            }
            task.class = Some(c.clone());
            kinds.push(ChangeKind::Edited);
        }
        let mut edited = false;
        if let Some(t) = &patch.title {
            let t = clean(t, TITLE_MAX);
            if t.is_empty() {
                return Err(BoardError::Invalid("A task needs a title.".into()));
            }
            task.title = t;
            edited = true;
        }
        if let Some(g) = &patch.goal {
            task.goal = clean(g, GOAL_MAX);
            edited = true;
        }
        if let Some(a) = &patch.acceptance {
            task.acceptance = clean_list(a, ITEMS_MAX, ITEM_MAX);
            edited = true;
        }
        if let Some(c) = &patch.constraints {
            task.constraints = clean_list(c, ITEMS_MAX, ITEM_MAX);
            edited = true;
        }
        if let Some(f) = &patch.files {
            task.files = clean_list(f, FILES_MAX, ITEM_MAX);
            edited = true;
        }
        if let Some(m) = &patch.model {
            task.model_override = Some(m.clone());
            task.log(
                now,
                &by,
                "model",
                Some(format!("{} (set by {by})", m.model)),
            );
            edited = true;
        }
        if edited {
            task.log(now, &by, "edited", None);
            kinds.push(ChangeKind::Edited);
        }
        if let Some(b) = &patch.blocked {
            let b = clean(b, NOTE_MAX);
            if b.is_empty() {
                if task.blocked.take().is_some() {
                    task.log(now, &by, "unblocked", None);
                    kinds.push(ChangeKind::Unblocked);
                }
            } else {
                task.log(now, &by, "blocked", Some(b.clone()));
                task.blocked = Some(b);
                kinds.push(ChangeKind::Blocked);
            }
        }
        if let Some(n) = &patch.note {
            let n = clean(n, NOTE_MAX);
            if !n.is_empty() {
                task.progress.push(Note {
                    at: now,
                    by: by.clone(),
                    text: n,
                });
                if task.progress.len() > MAX_NOTES {
                    let excess = task.progress.len() - MAX_NOTES;
                    task.progress.drain(..excess);
                }
                task.updated_at = now;
                kinds.push(ChangeKind::Note);
            }
        }
        if let Some(to) = patch.status {
            if to != task.status {
                let from = task.status;
                let allowed = match (from, to) {
                    // The holder works, gives it back, or (through
                    // `finish`) finishes it.
                    (TaskStatus::Claimed, TaskStatus::InProgress) => true,
                    (TaskStatus::Claimed | TaskStatus::InProgress, TaskStatus::Todo) => true,
                    // The rest is the lead's or the user's.
                    (_, TaskStatus::Cancelled) => manages && !from.ended(),
                    (TaskStatus::Review, TaskStatus::Done) => manages,
                    (
                        TaskStatus::Review | TaskStatus::Done | TaskStatus::Failed,
                        TaskStatus::Todo,
                    )
                    | (TaskStatus::Cancelled, TaskStatus::Todo) => manages,
                    (s, TaskStatus::Failed) => manages && !s.ended(),
                    (s, TaskStatus::Done) => manages && s.held(),
                    _ => false,
                };
                if !allowed {
                    return Err(BoardError::Conflict(format!(
                        "Task {id} cannot go from {} to {}{}.",
                        from.as_str(),
                        to.as_str(),
                        if manages {
                            ""
                        } else {
                            " (finish it with task_done)"
                        }
                    )));
                }
                task.status = to;
                task.log(
                    now,
                    &by,
                    "status",
                    Some(format!("{} → {}", from.as_str(), to.as_str())),
                );
                match to {
                    TaskStatus::InProgress => kinds.push(ChangeKind::Started),
                    TaskStatus::Todo => {
                        task.claimed_by = None;
                        task.ended_at = None;
                        if from.ended() || from == TaskStatus::Review {
                            kinds.push(ChangeKind::Reopened);
                        }
                        if patch.escalate {
                            task.escalations = task.escalations.saturating_add(1);
                            task.log(
                                now,
                                &by,
                                "escalated",
                                Some(format!("step {}", task.escalations)),
                            );
                            kinds.push(ChangeKind::Escalated);
                        }
                    }
                    TaskStatus::Done => {
                        task.ended_at = Some(now);
                        task.blocked = None;
                        kinds.push(ChangeKind::Done);
                    }
                    TaskStatus::Failed => {
                        task.ended_at = Some(now);
                        kinds.push(ChangeKind::Failed);
                    }
                    TaskStatus::Cancelled => {
                        task.ended_at = Some(now);
                        task.claimed_by = None;
                        kinds.push(ChangeKind::Cancelled);
                    }
                    _ => {}
                }
            }
        } else if patch.escalate {
            return Err(BoardError::Invalid(
                "Escalate goes with sending the task back (status todo).".into(),
            ));
        }
        Ok(Change {
            task: task.clone(),
            kinds,
            actor: actor.clone(),
            before,
            held_by,
        })
    }

    /// The holder (or the lead) finishes a task: `review` when the team
    /// reviews and a worker finished it, else `done`; or `failed`.
    pub fn finish(
        &mut self,
        id: u64,
        report: TaskReport,
        outcome: Outcome,
        review: bool,
        actor: &Actor,
        now: i64,
    ) -> Result<Change, BoardError> {
        let task = self.get_mut(id)?;
        let before = task.status;
        let held_by = task.claimed_by.clone();
        let holder = actor.session().is_some() && actor.session() == task.claimed_by.as_deref();
        let lead = matches!(actor, Actor::Session { lead: true, .. });
        if !holder && !lead {
            return Err(BoardError::Denied(format!(
                "Task {id} is held by another session (or nobody): claim it first."
            )));
        }
        if !(task.status.held() || (lead && task.status == TaskStatus::Todo)) {
            return Err(BoardError::Conflict(format!(
                "Task {id} is {}: there is nothing to finish.",
                task.status.as_str()
            )));
        }
        let summary = clean(&report.summary, SUMMARY_MAX);
        if summary.is_empty() {
            return Err(BoardError::Invalid(
                "Say what was done (or why it failed) in the summary.".into(),
            ));
        }
        let to = match outcome {
            Outcome::Failed => TaskStatus::Failed,
            Outcome::Done if review && !lead => TaskStatus::Review,
            Outcome::Done => TaskStatus::Done,
        };
        task.result = Some(TaskReport {
            summary,
            branch: report
                .branch
                .map(|b| clean(&b, 200))
                .filter(|b| !b.is_empty()),
            changed_files: clean_list(&report.changed_files, FILES_MAX, ITEM_MAX),
            gate_passed: report.gate_passed,
        });
        let by = actor.label();
        task.log(
            now,
            &by,
            "status",
            Some(format!("{} → {}", task.status.as_str(), to.as_str())),
        );
        task.status = to;
        task.blocked = None;
        let kind = match to {
            TaskStatus::Review => ChangeKind::Review,
            TaskStatus::Failed => {
                task.ended_at = Some(now);
                ChangeKind::Failed
            }
            _ => {
                task.ended_at = Some(now);
                ChangeKind::Done
            }
        };
        Ok(Change {
            task: task.clone(),
            kinds: vec![kind],
            actor: actor.clone(),
            before,
            held_by,
        })
    }

    /// Tasks a session held go back to `todo` (its process ended without
    /// finishing them, or its role was taken away).
    pub fn release(&mut self, session: &str, why: &str, now: i64) -> Vec<Change> {
        let mut out = Vec::new();
        for task in self
            .tasks
            .iter_mut()
            .filter(|t| t.status.held() && t.claimed_by.as_deref() == Some(session))
        {
            let before = task.status;
            task.status = TaskStatus::Todo;
            task.claimed_by = None;
            task.log(now, "Mother Claude", "released", Some(why.to_string()));
            out.push(Change {
                task: task.clone(),
                kinds: vec![ChangeKind::Reopened],
                actor: Actor::System,
                before,
                held_by: Some(session.to_string()),
            });
        }
        out
    }

    /// Keep the board bounded: drop the oldest ended tasks beyond
    /// [`MAX_TASKS`].
    fn prune(&mut self) {
        if self.tasks.len() <= MAX_TASKS {
            return;
        }
        let mut ended: Vec<(i64, u64)> = self
            .tasks
            .iter()
            .filter(|t| t.status.ended())
            .map(|t| (t.ended_at.unwrap_or(t.updated_at), t.id))
            .collect();
        ended.sort();
        let excess = self.tasks.len() - MAX_TASKS;
        let drop: Vec<u64> = ended.into_iter().take(excess).map(|(_, id)| id).collect();
        self.tasks.retain(|t| !drop.contains(&t.id));
    }
}

/// The first eight characters of a session id.
pub fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

// ─────────────────────────────── the store ───────────────────────────────

/// The boards, one per project, cached in memory and written on every
/// change.
#[derive(Debug, Default)]
pub struct BoardStore {
    dir: Option<PathBuf>,
    boards: Mutex<HashMap<String, Board>>,
}

/// The file name of a project's board: a hash of the folder as Claude Code
/// compares it (a Windows folder in any case, with either separator).
pub fn board_key(project: &str) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(crate::claude::paths::path_key(project).as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

impl BoardStore {
    pub fn new(data_dir: Option<&Path>) -> Self {
        Self {
            dir: data_dir.map(|d| d.join("boards")),
            boards: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Board>> {
        self.boards.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn path(&self, key: &str) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(format!("{key}.json")))
    }

    fn load<'a>(&self, map: &'a mut HashMap<String, Board>, project: &str) -> &'a mut Board {
        let key = board_key(project);
        map.entry(key.clone()).or_insert_with(|| {
            self.path(&key)
                .and_then(|p| crate::persist::read_json::<Board>(&p))
                .filter(|b| crate::claude::paths::same_path(&b.project, project))
                .unwrap_or_else(|| Board::new(project))
        })
    }

    /// Read a project's board.
    pub fn read<R>(&self, project: &str, f: impl FnOnce(&Board) -> R) -> R {
        let mut map = self.lock();
        f(self.load(&mut map, project))
    }

    /// Change a project's board; written when `f` succeeds. A failed write
    /// is logged (the change stays in memory and goes out with the next).
    pub fn mutate<R, E>(
        &self,
        project: &str,
        f: impl FnOnce(&mut Board) -> Result<R, E>,
    ) -> Result<R, E> {
        let mut map = self.lock();
        let board = self.load(&mut map, project);
        let out = f(board)?;
        if let Some(path) = self.path(&board_key(project)) {
            if let Err(e) = crate::persist::write_json_private(&path, &*board) {
                tracing::warn!(error = %e, "could not save the task board");
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roles() -> Vec<String> {
        ["lead", "frontend", "backend"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn lead() -> Actor {
        Actor::Session {
            session: "s-lead".into(),
            role: "lead".into(),
            lead: true,
        }
    }

    fn worker(session: &str, role: &str) -> Actor {
        Actor::Session {
            session: session.into(),
            role: role.into(),
            lead: false,
        }
    }

    fn draft(title: &str, role: &str) -> TaskDraft {
        TaskDraft {
            title: title.into(),
            goal: "Do it".into(),
            role: Some(role.into()),
            ..Default::default()
        }
    }

    #[test]
    fn a_task_goes_from_packet_to_done_through_review() {
        let mut b = Board::new("/p");
        let c = b
            .create(draft("Login form", "frontend"), &lead(), &roles(), 1)
            .unwrap();
        assert_eq!(c.task.id, 1);
        assert_eq!(c.task.priority, 2, "default priority");
        assert!(c.is(ChangeKind::Created));
        // Workers do not write tasks.
        assert_eq!(
            b.create(
                draft("x", "frontend"),
                &worker("w1", "frontend"),
                &roles(),
                1
            )
            .unwrap_err()
            .code(),
            "denied"
        );
        // Only the frontend role claims it.
        assert_eq!(
            b.claim(1, &worker("w2", "backend"), 2).unwrap_err().code(),
            "denied"
        );
        let c = b.claim(1, &worker("w1", "frontend"), 2).unwrap();
        assert_eq!(c.task.status, TaskStatus::Claimed);
        assert_eq!(c.task.claimed_by.as_deref(), Some("w1"));
        assert_eq!(c.task.started_at, Some(2));
        // Claiming again is a no-op; another session cannot.
        assert!(b
            .claim(1, &worker("w1", "frontend"), 3)
            .unwrap()
            .kinds
            .is_empty());
        assert_eq!(
            b.claim(1, &worker("w9", "frontend"), 3).unwrap_err().code(),
            "conflict"
        );
        // The holder reports progress and moves it along.
        let c = b
            .update(
                1,
                TaskPatch {
                    status: Some(TaskStatus::InProgress),
                    note: Some("form drafted".into()),
                    ..Default::default()
                },
                &worker("w1", "frontend"),
                &roles(),
                4,
            )
            .unwrap();
        assert!(c.is(ChangeKind::Started) && c.is(ChangeKind::Note));
        // A worker cannot reprioritise, edit or mark done directly.
        for patch in [
            TaskPatch {
                priority: Some(0),
                ..Default::default()
            },
            TaskPatch {
                title: Some("x".into()),
                ..Default::default()
            },
        ] {
            assert_eq!(
                b.update(1, patch, &worker("w1", "frontend"), &roles(), 5)
                    .unwrap_err()
                    .code(),
                "denied"
            );
        }
        assert_eq!(
            b.update(
                1,
                TaskPatch {
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
                &worker("w1", "frontend"),
                &roles(),
                5
            )
            .unwrap_err()
            .code(),
            "conflict"
        );
        // Another worker cannot touch it at all.
        assert_eq!(
            b.update(
                1,
                TaskPatch {
                    note: Some("hi".into()),
                    ..Default::default()
                },
                &worker("w2", "backend"),
                &roles(),
                5
            )
            .unwrap_err()
            .code(),
            "denied"
        );
        // Finished by the worker: review first.
        let c = b
            .finish(
                1,
                TaskReport {
                    summary: "Form and validation".into(),
                    branch: Some("worktree-frontend".into()),
                    changed_files: vec!["src/app/login.ts".into()],
                    gate_passed: None,
                },
                Outcome::Done,
                true,
                &worker("w1", "frontend"),
                6,
            )
            .unwrap();
        assert_eq!(c.task.status, TaskStatus::Review);
        assert!(c.is(ChangeKind::Review) && !c.ended());
        // The lead accepts it.
        let c = b
            .update(
                1,
                TaskPatch {
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
                &lead(),
                &roles(),
                7,
            )
            .unwrap();
        assert!(c.is(ChangeKind::Done) && c.ended());
        assert_eq!(c.task.ended_at, Some(7));
        assert!(c.task.history.iter().any(|h| h.action == "claimed"));
    }

    #[test]
    fn the_lead_sends_work_back_with_an_escalation_and_the_user_manages() {
        let mut b = Board::new("/p");
        b.create(draft("A", "backend"), &lead(), &roles(), 1)
            .unwrap();
        b.assign(1, "w1", 2).unwrap();
        let c = b
            .finish(
                1,
                TaskReport {
                    summary: "tried".into(),
                    ..Default::default()
                },
                Outcome::Failed,
                true,
                &worker("w1", "backend"),
                3,
            )
            .unwrap();
        assert_eq!(c.task.status, TaskStatus::Failed);
        assert!(c.ended());
        let c = b
            .update(
                1,
                TaskPatch {
                    status: Some(TaskStatus::Todo),
                    escalate: true,
                    ..Default::default()
                },
                &lead(),
                &roles(),
                4,
            )
            .unwrap();
        assert!(c.is(ChangeKind::Reopened) && c.is(ChangeKind::Escalated));
        assert_eq!(c.task.escalations, 1);
        assert!(c.task.claimed_by.is_none());
        // Escalate needs a send-back.
        assert_eq!(
            b.update(
                1,
                TaskPatch {
                    escalate: true,
                    ..Default::default()
                },
                &lead(),
                &roles(),
                5
            )
            .unwrap_err()
            .code(),
            "invalid_arguments"
        );
        // The user reprioritises, reassigns and cancels.
        let user = Actor::User("phone".into());
        let c = b
            .update(
                1,
                TaskPatch {
                    priority: Some(0),
                    role: Some("frontend".into()),
                    ..Default::default()
                },
                &user,
                &roles(),
                6,
            )
            .unwrap();
        assert!(c.is(ChangeKind::Reprioritised) && c.is(ChangeKind::Reassigned));
        assert_eq!(b.next_for("frontend").map(|t| t.id), Some(1));
        assert_eq!(
            b.update(
                1,
                TaskPatch {
                    role: Some("nobody".into()),
                    ..Default::default()
                },
                &user,
                &roles(),
                7
            )
            .unwrap_err()
            .code(),
            "invalid_arguments"
        );
        let c = b
            .update(
                1,
                TaskPatch {
                    status: Some(TaskStatus::Cancelled),
                    ..Default::default()
                },
                &user,
                &roles(),
                8,
            )
            .unwrap();
        assert!(c.is(ChangeKind::Cancelled) && c.ended());
        assert!(b.next_for("frontend").is_none());
    }

    #[test]
    fn blocked_tasks_wait_and_released_ones_return() {
        let mut b = Board::new("/p");
        b.create(draft("A", "frontend"), &lead(), &roles(), 1)
            .unwrap();
        b.create(
            TaskDraft {
                priority: Some(0),
                ..draft("B", "frontend")
            },
            &lead(),
            &roles(),
            2,
        )
        .unwrap();
        assert_eq!(b.next_for("frontend").map(|t| t.id), Some(2), "P0 first");
        b.assign(2, "w1", 3).unwrap();
        assert_eq!(b.held_by("w1").map(|t| t.id), Some(2));
        let c = b
            .update(
                2,
                TaskPatch {
                    blocked: Some("Which API?".into()),
                    ..Default::default()
                },
                &worker("w1", "frontend"),
                &roles(),
                4,
            )
            .unwrap();
        assert!(c.is(ChangeKind::Blocked));
        let c = b
            .update(
                2,
                TaskPatch {
                    blocked: Some(String::new()),
                    ..Default::default()
                },
                &lead(),
                &roles(),
                5,
            )
            .unwrap();
        assert!(c.is(ChangeKind::Unblocked));
        let released = b.release("w1", "its session ended", 6);
        assert_eq!(released.len(), 1);
        assert_eq!(b.get(2).unwrap().status, TaskStatus::Todo);
        assert!(b.held_by("w1").is_none());
    }

    #[test]
    fn text_is_cleaned_and_bounded() {
        let mut b = Board::new("/p");
        let c = b
            .create(
                TaskDraft {
                    title: format!("  bell\u{7} {}", "x".repeat(500)),
                    acceptance: vec!["".into(), " one ".into()],
                    priority: Some(1),
                    class: Some("implement".into()),
                    ..Default::default()
                },
                &Actor::User("mac".into()),
                &roles(),
                1,
            )
            .unwrap();
        assert!(!c.task.title.contains('\u{7}'));
        assert_eq!(c.task.title.chars().count(), 200);
        assert_eq!(c.task.acceptance, vec!["one".to_string()]);
        assert_eq!(
            b.create(
                TaskDraft {
                    title: "x".into(),
                    class: Some("dance".into()),
                    ..Default::default()
                },
                &lead(),
                &roles(),
                1
            )
            .unwrap_err()
            .code(),
            "invalid_arguments"
        );
        assert_eq!(
            b.create(
                TaskDraft {
                    title: "x".into(),
                    priority: Some(9),
                    ..Default::default()
                },
                &lead(),
                &roles(),
                1
            )
            .unwrap_err()
            .code(),
            "invalid_arguments"
        );
    }

    #[test]
    fn the_store_writes_privately_and_reads_tolerantly() {
        let dir = tempfile::tempdir().unwrap();
        let store = BoardStore::new(Some(dir.path()));
        store
            .mutate("/work/app", |b| {
                b.create(draft("A", "frontend"), &lead(), &roles(), 1)
            })
            .unwrap();
        let file = dir
            .path()
            .join("boards")
            .join(format!("{}.json", board_key("/work/app")));
        assert!(file.is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        // A newer version's status and fields read without failing.
        let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        v["tasks"][0]["status"] = "parked".into();
        v["tasks"][0]["sparkle"] = true.into();
        std::fs::write(&file, v.to_string()).unwrap();
        let again = BoardStore::new(Some(dir.path()));
        again.read("/work/app", |b| {
            assert_eq!(b.tasks[0].status, TaskStatus::Todo);
            assert_eq!(b.tasks[0].extra["sparkle"], true);
            assert_eq!(b.next_id, 2);
        });
        // A Windows folder is one board in any case and with either
        // separator.
        assert_eq!(board_key(r"C:\Work\App"), board_key("c:/work/app"));
        assert_ne!(board_key("/work/app"), board_key("/work/App"));
    }
}
