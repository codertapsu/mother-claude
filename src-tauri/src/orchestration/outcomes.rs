//! Task outcomes (docs/ROUTING.md): how each task ended and what it cost,
//! kept for the reports and for the router's escalation state.
//!
//! * **Storage**: `<app data dir>/outcomes.jsonl`, one JSON record per line
//!   (0600), appended; at 4 MB it becomes `outcomes.1.jsonl` (the one before
//!   is dropped). Both are read at startup, tolerantly: a line that does
//!   not parse is skipped. A record is written when the task ends and again
//!   once its token usage was joined; the last line of a task end wins.
//! * **Learned state** ([`Learned`]), rebuilt from the records at startup:
//!   the failed gates of each task (every `gatesPerStep` of them earn one
//!   escalation step, with the reason recorded) and each (project, task
//!   class)'s escalation level — raised when a task of the class escalates,
//!   one step back after `cleanToStepDown` clean completions.

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::metrics::{TaskOutcome, TaskResult};
use super::policy::{self, Policy};
use super::router::TaskClass;
use super::scheduler::lock;
use crate::claude::index::window::WindowUsage;
use crate::claude::paths;

/// The log in the app data dir, and its previous generation.
pub const FILE_NAME: &str = "outcomes.jsonl";
pub const OLD_NAME: &str = "outcomes.1.jsonl";
/// The log rotates at this size.
const ROTATE_AT: u64 = 4 * 1024 * 1024;
/// Records kept in memory (the reports cover these).
const KEEP: usize = 20_000;
/// Escalation notes kept.
const NOTES_KEEP: usize = 200;
/// Tasks whose failed gates are remembered.
const GATES_KEEP: usize = 10_000;
/// The most escalation steps a class can carry.
pub const LEVEL_MAX: u8 = 6;

/// One task's end with what was learned about it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutcomeRecord {
    #[serde(flatten)]
    pub outcome: TaskOutcome,
    /// Epoch ms the end was recorded.
    pub recorded_at: i64,
    /// The account the session ran under.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// Whether that account pays per token (else a plan's usage windows pay
    /// and `costUsd` is what the work would have cost through the API).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pays_per_token: Option<bool>,
    /// The session's token usage within the task's time, from its
    /// transcripts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<WindowUsage>,
    /// Estimated USD of `usage`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// Estimated share of the 5-hour window it used, in percentage points
    /// (plan accounts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_pct: Option<f64>,
    /// Why usage is missing or incomplete.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// When this end escalated the task: why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalation: Option<String>,
    /// Whether the usage join ran.
    #[serde(default)]
    pub joined: bool,
}

impl OutcomeRecord {
    /// What identifies one task end (the join rewrites the same end).
    pub fn key(&self) -> (String, i64, String) {
        (
            self.outcome.task_id.clone(),
            self.outcome.ended_at,
            format!("{:?}", self.outcome.result),
        )
    }
}

/// The records in memory and the log on disk.
#[derive(Debug)]
pub struct OutcomeStore {
    path: Option<PathBuf>,
    records: Mutex<VecDeque<OutcomeRecord>>,
    file: Mutex<()>,
}

impl OutcomeStore {
    /// Load `<data_dir>/outcomes.jsonl` (and its previous generation);
    /// `None`: in memory only.
    pub fn open(data_dir: Option<&Path>) -> Self {
        let path = data_dir.map(|d| d.join(FILE_NAME));
        let mut records = VecDeque::new();
        if let Some(dir) = data_dir {
            records = load(&[dir.join(OLD_NAME), dir.join(FILE_NAME)]).into();
        }
        while records.len() > KEEP {
            records.pop_front();
        }
        Self {
            path,
            records: Mutex::new(records),
            file: Mutex::new(()),
        }
    }

    /// Every record, oldest first.
    pub fn all(&self) -> Vec<OutcomeRecord> {
        lock(&self.records).iter().cloned().collect()
    }

    /// Records recorded at or after `since`.
    pub fn since(&self, since: i64) -> Vec<OutcomeRecord> {
        lock(&self.records)
            .iter()
            .filter(|r| r.recorded_at >= since || r.outcome.ended_at >= since)
            .cloned()
            .collect()
    }

    /// Add a new end (memory and log).
    pub fn push(&self, record: OutcomeRecord) {
        self.append(&record);
        let mut records = lock(&self.records);
        records.push_back(record);
        while records.len() > KEEP {
            records.pop_front();
        }
    }

    /// Replace an end with what was learned since (memory and log).
    pub fn update(&self, record: OutcomeRecord) {
        self.append(&record);
        let key = record.key();
        let mut records = lock(&self.records);
        match records.iter_mut().rev().find(|r| r.key() == key) {
            Some(r) => *r = record,
            None => records.push_back(record),
        }
    }

    fn append(&self, record: &OutcomeRecord) {
        let Some(path) = &self.path else {
            return;
        };
        let _g = lock(&self.file);
        if let Err(e) = append_line(path, record) {
            tracing::warn!(path = %path.display(), error = %e, "could not record a task outcome");
        }
    }
}

/// Append one record, rotating the log first when it is full.
fn append_line(path: &Path, record: &OutcomeRecord) -> std::io::Result<()> {
    if std::fs::metadata(path).is_ok_and(|m| m.len() >= ROTATE_AT) {
        let old = path.with_file_name(OLD_NAME);
        std::fs::rename(path, &old)?;
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut line = serde_json::to_vec(record)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    line.push(b'\n');
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(&line)?;
    f.flush()?;
    crate::persist::restrict(path);
    Ok(())
}

/// Read logs (oldest first): unparsable lines skipped, the last line of
/// each end kept, ordered by when they were recorded.
pub fn load(paths: &[PathBuf]) -> Vec<OutcomeRecord> {
    let mut order: Vec<(String, i64, String)> = Vec::new();
    let mut by_key: HashMap<(String, i64, String), OutcomeRecord> = HashMap::new();
    for path in paths {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "could not read task outcomes");
                continue;
            }
        };
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let Ok(r) = serde_json::from_str::<OutcomeRecord>(line) else {
                continue;
            };
            let key = r.key();
            if by_key.insert(key.clone(), r).is_none() {
                order.push(key);
            }
        }
    }
    let mut out: Vec<OutcomeRecord> = order
        .into_iter()
        .filter_map(|k| by_key.remove(&k))
        .collect();
    out.sort_by_key(|r| r.recorded_at);
    out
}

/// A (project, class)'s escalation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassState {
    /// Steps up the ladder new tasks of the class start with.
    pub level: u8,
    /// Clean completions since the last change.
    pub clean: u8,
}

/// An escalation (or a step back down), with its reason.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EscalationNote {
    pub at: i64,
    pub task_id: String,
    pub project: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<TaskClass>,
    /// `up` or `down`.
    pub direction: &'static str,
    /// The task's (up) or the class's (down) steps afterwards.
    pub steps: u8,
    pub reason: String,
}

/// A class's escalation as the Capacity page lists it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassView {
    pub project: String,
    pub class: TaskClass,
    pub level: u8,
    pub clean: u8,
}

/// What the router learned from outcomes.
#[derive(Debug, Default)]
pub struct Learned {
    /// (project key, class) → state; the project as last written.
    classes: HashMap<(String, TaskClass), (String, ClassState)>,
    /// Task id → (failed gates, last at).
    failed_gates: HashMap<String, (u32, i64)>,
    notes: VecDeque<EscalationNote>,
}

impl Learned {
    /// Note how a task ended; returns why it escalated the task, if it did.
    pub fn note(&mut self, o: &TaskOutcome, policy: &Policy, at: i64) -> Option<String> {
        let gates_per_step = u32::from(policy.gates_per_step.max(1));
        let mut escalated: Option<(u8, String)> = None;
        if o.gate_passed == Some(false) {
            let entry = self
                .failed_gates
                .entry(o.task_id.clone())
                .or_insert((0, at));
            entry.0 += 1;
            entry.1 = at;
            let n = entry.0;
            if n.is_multiple_of(gates_per_step) {
                let steps = u8::try_from(n / gates_per_step).unwrap_or(u8::MAX);
                escalated = Some((
                    steps.min(LEVEL_MAX),
                    format!(
                        "{n} failed gates: escalated to {} step{}",
                        steps,
                        if steps == 1 { "" } else { "s" }
                    ),
                ));
            }
            self.prune_gates();
        }
        if escalated.is_none() && o.result == TaskResult::Escalated {
            let steps = o
                .escalations
                .max(1)
                .max(self.task_steps(&o.task_id, policy));
            escalated = Some((
                steps.min(LEVEL_MAX),
                "handed to a more capable model or effort".into(),
            ));
        }
        let class_key = o
            .class
            .map(|c| ((paths::path_key(&o.project), c), o.project.clone()));
        if let Some((steps, reason)) = escalated {
            if let Some((key, project)) = class_key {
                let (name, st) = self
                    .classes
                    .entry(key)
                    .or_insert((project.clone(), ClassState::default()));
                *name = project;
                st.level = st.level.max(steps).min(LEVEL_MAX);
                st.clean = 0;
            }
            self.push_note(EscalationNote {
                at,
                task_id: o.task_id.clone(),
                project: o.project.clone(),
                class: o.class,
                direction: "up",
                steps,
                reason: reason.clone(),
            });
            return Some(reason);
        }
        let Some((key, project)) = class_key else {
            if matches!(o.result, TaskResult::Done | TaskResult::Cancelled) {
                self.failed_gates.remove(&o.task_id);
            }
            return None;
        };
        let clean = o.result == TaskResult::Done
            && o.gate_passed != Some(false)
            && o.escalations == 0
            && self
                .failed_gates
                .get(&o.task_id)
                .is_none_or(|(n, _)| *n == 0);
        let (name, st) = self
            .classes
            .entry(key)
            .or_insert((project.clone(), ClassState::default()));
        *name = project.clone();
        match o.result {
            TaskResult::Done if clean => {
                st.clean = st.clean.saturating_add(1);
                if st.level > 0 && st.clean >= policy.clean_to_step_down.max(1) {
                    st.level -= 1;
                    st.clean = 0;
                    let level = st.level;
                    let class = o.class;
                    self.push_note(EscalationNote {
                        at,
                        task_id: o.task_id.clone(),
                        project,
                        class,
                        direction: "down",
                        steps: level,
                        reason: format!(
                            "{} clean completions: {} tasks step back down",
                            policy.clean_to_step_down,
                            class.map(policy::class_label).unwrap_or("these")
                        ),
                    });
                }
            }
            TaskResult::Done | TaskResult::Failed => st.clean = 0,
            TaskResult::Escalated | TaskResult::Cancelled => {}
        }
        if matches!(o.result, TaskResult::Done | TaskResult::Cancelled) {
            self.failed_gates.remove(&o.task_id);
        }
        None
    }

    fn push_note(&mut self, note: EscalationNote) {
        self.notes.push_front(note);
        self.notes.truncate(NOTES_KEEP);
    }

    fn prune_gates(&mut self) {
        if self.failed_gates.len() <= GATES_KEEP {
            return;
        }
        let mut ats: Vec<i64> = self.failed_gates.values().map(|(_, at)| *at).collect();
        ats.sort_unstable();
        let cut = ats[ats.len() / 2];
        self.failed_gates.retain(|_, (_, at)| *at > cut);
    }

    /// The escalation steps of new tasks of `class` in `project`.
    pub fn class_level(&self, project: &str, class: TaskClass) -> u8 {
        self.classes
            .get(&(paths::path_key(project), class))
            .map(|(_, s)| s.level)
            .unwrap_or(0)
    }

    /// The steps a task earned from its failed gates.
    pub fn task_steps(&self, task_id: &str, policy: &Policy) -> u8 {
        let n = self.failed_gates.get(task_id).map(|(n, _)| *n).unwrap_or(0);
        u8::try_from(n / u32::from(policy.gates_per_step.max(1)))
            .unwrap_or(u8::MAX)
            .min(LEVEL_MAX)
    }

    /// Classes that carry an escalation or count clean completions.
    pub fn classes(&self) -> Vec<ClassView> {
        let mut out: Vec<ClassView> = self
            .classes
            .iter()
            .filter(|(_, (_, s))| s.level > 0)
            .map(|((_, class), (project, s))| ClassView {
                project: project.clone(),
                class: *class,
                level: s.level,
                clean: s.clean,
            })
            .collect();
        out.sort_by(|a, b| {
            (&a.project, policy::class_name(a.class))
                .cmp(&(&b.project, policy::class_name(b.class)))
        });
        out
    }

    /// Recent escalations and steps down, newest first.
    pub fn notes(&self) -> Vec<EscalationNote> {
        self.notes.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestration::router::ModelChoice;

    fn outcome(task: &str, result: TaskResult, gate: Option<bool>) -> TaskOutcome {
        TaskOutcome {
            task_id: task.into(),
            project: "/Users/me/app".into(),
            role: Some("backend".into()),
            class: Some(TaskClass::Implement),
            choice: ModelChoice::new("sonnet", Some("medium")),
            session_id: None,
            started_at: 1_000,
            ended_at: 2_000,
            result,
            gate_passed: gate,
            escalations: 0,
        }
    }

    fn record(o: TaskOutcome, at: i64) -> OutcomeRecord {
        OutcomeRecord {
            outcome: o,
            recorded_at: at,
            account: None,
            pays_per_token: None,
            usage: None,
            cost_usd: None,
            window_pct: None,
            note: None,
            escalation: None,
            joined: false,
        }
    }

    #[test]
    fn two_failed_gates_escalate_a_task_with_the_reason_recorded() {
        let p = Policy::default();
        let mut l = Learned::default();
        assert_eq!(
            l.note(&outcome("t1", TaskResult::Failed, Some(false)), &p, 10),
            None
        );
        assert_eq!(l.task_steps("t1", &p), 0);
        let why = l.note(&outcome("t1", TaskResult::Failed, Some(false)), &p, 20);
        assert_eq!(why.as_deref(), Some("2 failed gates: escalated to 1 step"));
        assert_eq!(l.task_steps("t1", &p), 1);
        let notes = l.notes();
        assert_eq!(notes.len(), 1);
        assert_eq!((notes[0].direction, notes[0].steps), ("up", 1));
        assert_eq!(notes[0].task_id, "t1");
        // The class escalated with it: new implementation tasks start a step up.
        assert_eq!(l.class_level("/Users/me/app/", TaskClass::Implement), 1);
        assert_eq!(l.class_level("/Users/me/other", TaskClass::Implement), 0);
        // Done: the task's gates are forgotten.
        l.note(&outcome("t1", TaskResult::Done, Some(true)), &p, 30);
        assert_eq!(l.task_steps("t1", &p), 0);
    }

    #[test]
    fn two_clean_completions_step_a_class_back_down() {
        let p = Policy::default();
        let mut l = Learned::default();
        l.note(&outcome("e", TaskResult::Escalated, None), &p, 1);
        assert_eq!(l.class_level("/Users/me/app", TaskClass::Implement), 1);
        // A done task that had failed a gate is not clean.
        l.note(&outcome("x", TaskResult::Failed, Some(false)), &p, 2);
        l.note(&outcome("x", TaskResult::Done, Some(true)), &p, 3);
        assert_eq!(l.class_level("/Users/me/app", TaskClass::Implement), 1);
        l.note(&outcome("a", TaskResult::Done, Some(true)), &p, 4);
        assert_eq!(l.class_level("/Users/me/app", TaskClass::Implement), 1);
        l.note(&outcome("b", TaskResult::Done, None), &p, 5);
        assert_eq!(l.class_level("/Users/me/app", TaskClass::Implement), 0);
        let notes = l.notes();
        assert_eq!(notes[0].direction, "down");
        assert_eq!(
            notes[0].reason,
            "2 clean completions: implementation tasks step back down"
        );
        assert!(l.classes().is_empty(), "nothing escalated any more");
    }

    #[test]
    fn the_log_rotates_and_reloads_the_last_line_of_each_end() {
        let dir = tempfile::tempdir().unwrap();
        let store = OutcomeStore::open(Some(dir.path()));
        let mut r = record(outcome("t1", TaskResult::Done, Some(true)), 5);
        store.push(r.clone());
        r.cost_usd = Some(0.42);
        r.joined = true;
        store.update(r.clone());
        store.push(record(outcome("t2", TaskResult::Failed, Some(false)), 6));
        assert_eq!(store.all().len(), 2);
        assert_eq!(store.all()[0].cost_usd, Some(0.42));
        // A torn or foreign line is skipped on reload.
        let path = dir.path().join(FILE_NAME);
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(f, "{{ torn").unwrap();
        writeln!(f, "{{\"taskId\": 5}}").unwrap();
        let again = OutcomeStore::open(Some(dir.path()));
        let all = again.all();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].outcome.task_id, "t1");
        assert_eq!(all[0].cost_usd, Some(0.42));
        assert!(all[0].joined);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        // Rotation: a full log moves aside and both generations load.
        let big = vec![b'x'; ROTATE_AT as usize];
        std::fs::write(dir.path().join("pad"), &big).unwrap();
        let mut log = std::fs::read(&path).unwrap();
        log.extend_from_slice(b"\n");
        log.extend_from_slice(&big);
        std::fs::write(&path, log).unwrap();
        again.push(record(outcome("t3", TaskResult::Done, None), 7));
        assert!(dir.path().join(OLD_NAME).is_file());
        let third = OutcomeStore::open(Some(dir.path()));
        let ids: Vec<String> = third.all().into_iter().map(|r| r.outcome.task_id).collect();
        assert_eq!(ids, ["t1", "t2", "t3"]);
    }

    #[test]
    fn a_record_serializes_the_contract_fields_flat() {
        let r = record(outcome("t1", TaskResult::Escalated, Some(false)), 5);
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["taskId"], "t1");
        assert_eq!(v["result"], "escalated");
        assert_eq!(v["choice"]["model"], "sonnet");
        assert_eq!(v["recordedAt"], 5);
        assert!(v.get("usage").is_none());
        let back: OutcomeRecord = serde_json::from_value(v).unwrap();
        assert_eq!(back, r);
    }
}
