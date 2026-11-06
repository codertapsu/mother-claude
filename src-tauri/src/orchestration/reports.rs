//! Reports on task outcomes (docs/ROUTING.md, plan §7.2): cost and tokens
//! per completed task by class × model × effort, the escalation rate, the
//! gate pass rate and the cache-hit ratio — as JSON (`GET
//! /api/routing/report`) and CSV.
//!
//! Every task end is one attempt in the group of the model and effort it
//! ran on, so a failed Sonnet attempt counts against Sonnet. The per-class
//! rows add every attempt of a class up: what a completed task of the class
//! cost all told, cascades included. Money figures are estimates (the
//! transcripts priced with the first-party price table); on plan accounts
//! they are what the work would have cost through the API, and the window
//! share is the estimated part of the 5-hour window.

use std::collections::BTreeMap;

use serde::Serialize;

use super::classify::ClassifierStats;
use super::metrics::TaskResult;
use super::outcomes::{ClassView, EscalationNote, OutcomeRecord};
use super::policy;
use crate::claude::paths;

/// What every money figure is.
pub const ESTIMATE_NOTE: &str = "Estimates: token usage from the sessions' transcripts priced at \
first-party API rates. On plan accounts (Pro, Max, Team) the work is paid in usage windows, so \
dollars are what it would have cost through the API.";

/// One group of task ends.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    /// A class name, `unclassified`, or `all`.
    pub class: String,
    /// The model as chosen, or `all`.
    pub model: String,
    /// The effort as chosen, `default`, or `all`.
    pub effort: String,
    /// Task ends (attempts).
    pub tasks: u64,
    pub completed: u64,
    pub failed: u64,
    pub escalated: u64,
    pub cancelled: u64,
    /// Ends whose usage is known.
    pub with_usage: u64,
    /// Estimated USD of the ends with usage.
    pub cost_usd: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_per_completed_usd: Option<f64>,
    pub tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_per_completed: Option<u64>,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    /// Cache reads among all input tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_hit_ratio: Option<f64>,
    pub gates: u64,
    pub gates_passed: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate_pass_rate: Option<f64>,
    /// Escalated ends among all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub escalation_rate: Option<f64>,
    /// Estimated 5-hour window share (percentage points), plan accounts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_pct_per_completed: Option<f64>,
    /// Wall time of a completed task, on average.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minutes_per_completed: Option<f64>,
    /// Ends on accounts that pay per token.
    pub per_token: u64,
    #[serde(skip)]
    completed_ms: i64,
}

impl Row {
    fn new(class: &str, model: &str, effort: &str) -> Self {
        Self {
            class: class.into(),
            model: model.into(),
            effort: effort.into(),
            ..Default::default()
        }
    }

    fn add(&mut self, r: &OutcomeRecord) {
        let o = &r.outcome;
        self.tasks += 1;
        match o.result {
            TaskResult::Done => {
                self.completed += 1;
                self.completed_ms += (o.ended_at - o.started_at).max(0);
            }
            TaskResult::Failed => self.failed += 1,
            TaskResult::Escalated => self.escalated += 1,
            TaskResult::Cancelled => self.cancelled += 1,
        }
        if let Some(g) = o.gate_passed {
            self.gates += 1;
            self.gates_passed += u64::from(g);
        }
        if r.pays_per_token == Some(true) {
            self.per_token += 1;
        }
        if let Some(u) = &r.usage {
            self.with_usage += 1;
            self.cost_usd += r.cost_usd.unwrap_or(u.cost_usd);
            self.input += u.input;
            self.output += u.output;
            self.cache_read += u.cache_read;
            self.cache_write += u.cache_write;
            self.tokens += u.tokens();
        }
        if let Some(w) = r.window_pct {
            self.window_pct = Some(self.window_pct.unwrap_or(0.0) + w);
        }
    }

    fn finish(mut self) -> Self {
        let per = |v: f64, n: u64| (n > 0).then(|| v / n as f64);
        self.cost_usd = round(self.cost_usd, 6);
        self.cost_per_completed_usd = (self.with_usage > 0)
            .then(|| per(self.cost_usd, self.completed))
            .flatten()
            .map(|c| round(c, 6));
        self.tokens_per_completed =
            (self.with_usage > 0 && self.completed > 0).then(|| self.tokens / self.completed);
        let inputs = self.input + self.cache_read + self.cache_write;
        self.cache_hit_ratio =
            (inputs > 0).then(|| round(self.cache_read as f64 / inputs as f64, 4));
        self.gate_pass_rate = per(self.gates_passed as f64, self.gates).map(|r| round(r, 4));
        self.escalation_rate = per(self.escalated as f64, self.tasks).map(|r| round(r, 4));
        self.window_pct = self.window_pct.map(|w| round(w, 1));
        self.window_pct_per_completed = self
            .window_pct
            .and_then(|w| per(w, self.completed))
            .map(|w| round(w, 2));
        self.minutes_per_completed =
            per(self.completed_ms as f64 / 60_000.0, self.completed).map(|m| round(m, 1));
        self
    }
}

fn round(v: f64, places: i32) -> f64 {
    let f = 10f64.powi(places);
    (v * f).round() / f
}

/// The model as reported: lowercase, trimmed.
fn model_key(m: &str) -> String {
    m.trim().to_ascii_lowercase()
}

/// A report.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub generated_at: i64,
    /// Ends since this time (epoch ms).
    pub since: i64,
    pub days: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// By class × model × effort.
    pub rows: Vec<Row>,
    /// By class, every model and effort together.
    pub classes: Vec<Row>,
    pub totals: Row,
    /// Escalations and steps down in the period, newest first.
    pub escalations: Vec<EscalationNote>,
    /// Task classes carrying an escalation now.
    pub learned: Vec<ClassView>,
    pub classifier: ClassifierStats,
    /// Ends whose usage is still being read.
    pub pending: u64,
    pub estimate_note: &'static str,
}

/// The groups of `records` (ended at or after `since`, in `project` when
/// given): by class × model × effort, by class, and in total.
pub fn build(
    records: &[OutcomeRecord],
    since: i64,
    project: Option<&str>,
) -> (Vec<Row>, Vec<Row>, Row, u64) {
    let mut rows: BTreeMap<(String, String, String), Row> = BTreeMap::new();
    let mut classes: BTreeMap<String, Row> = BTreeMap::new();
    let mut totals = Row::new("all", "all", "all");
    let mut pending = 0;
    for r in records.iter().filter(|r| {
        r.outcome.ended_at >= since
            && project.is_none_or(|p| paths::is_within_recorded(&r.outcome.project, p))
    }) {
        let o = &r.outcome;
        let class = o
            .class
            .map(|c| policy::class_name(c).to_string())
            .unwrap_or_else(|| "unclassified".into());
        let model = model_key(&o.choice.model);
        let effort = o.choice.effort.clone().unwrap_or_else(|| "default".into());
        rows.entry((class.clone(), model.clone(), effort.clone()))
            .or_insert_with(|| Row::new(&class, &model, &effort))
            .add(r);
        classes
            .entry(class.clone())
            .or_insert_with(|| Row::new(&class, "all", "all"))
            .add(r);
        totals.add(r);
        pending += u64::from(!r.joined);
    }
    // Classes in the policy's order, unclassified last.
    let order = |c: &str| {
        policy::CLASSES
            .iter()
            .position(|k| policy::class_name(*k) == c)
            .unwrap_or(usize::MAX)
    };
    let mut rows: Vec<Row> = rows.into_values().map(Row::finish).collect();
    rows.sort_by(|a, b| {
        order(&a.class)
            .cmp(&order(&b.class))
            .then(policy::tier(&a.model).cmp(&policy::tier(&b.model)))
            .then(a.model.cmp(&b.model))
            .then(policy::effort_rank(Some(&a.effort)).cmp(&policy::effort_rank(Some(&b.effort))))
    });
    let mut classes: Vec<Row> = classes.into_values().map(Row::finish).collect();
    classes.sort_by_key(|r| order(&r.class));
    (rows, classes, totals.finish(), pending)
}

// ─────────────────────────────── CSV ──────────────────────────────────────────

/// One CSV cell: quoted when needed, and a text cell that a spreadsheet
/// would run as a formula gets a leading `'`.
fn cell(s: &str) -> String {
    let guarded = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{s}")
    } else {
        s.to_string()
    };
    if guarded.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", guarded.replace('"', "\"\""))
    } else {
        guarded
    }
}

fn num(v: Option<f64>) -> String {
    v.map(|v| format!("{v}")).unwrap_or_default()
}

/// The groups as CSV (every money column is named as an estimate).
pub fn rows_csv(rows: &[Row], classes: &[Row], totals: &Row) -> String {
    let mut out = String::from(
        "class,model,effort,tasks,completed,failed,escalated,cancelled,est_cost_usd,\
         est_cost_per_completed_usd,tokens,tokens_per_completed,input,output,cache_read,\
         cache_write,cache_hit_ratio,gate_pass_rate,escalation_rate,est_window_pct,\
         est_window_pct_per_completed,minutes_per_completed,per_token_ends\r\n",
    );
    for r in rows.iter().chain(classes).chain(std::iter::once(totals)) {
        let fields = [
            cell(&r.class),
            cell(&r.model),
            cell(&r.effort),
            r.tasks.to_string(),
            r.completed.to_string(),
            r.failed.to_string(),
            r.escalated.to_string(),
            r.cancelled.to_string(),
            format!("{}", r.cost_usd),
            num(r.cost_per_completed_usd),
            r.tokens.to_string(),
            r.tokens_per_completed
                .map(|t| t.to_string())
                .unwrap_or_default(),
            r.input.to_string(),
            r.output.to_string(),
            r.cache_read.to_string(),
            r.cache_write.to_string(),
            num(r.cache_hit_ratio),
            num(r.gate_pass_rate),
            num(r.escalation_rate),
            num(r.window_pct),
            num(r.window_pct_per_completed),
            num(r.minutes_per_completed),
            r.per_token.to_string(),
        ];
        out.push_str(&fields.join(","));
        out.push_str("\r\n");
    }
    out
}

fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

/// Every task end as CSV, oldest first.
pub fn outcomes_csv(records: &[OutcomeRecord], since: i64, project: Option<&str>) -> String {
    let mut out = String::from(
        "task_id,project,role,class,model,effort,result,gate_passed,escalations,started_at,\
         ended_at,minutes,account,pays_per_token,input,output,cache_read,cache_write,\
         est_cost_usd,est_window_pct,escalation,note\r\n",
    );
    for r in records.iter().filter(|r| {
        r.outcome.ended_at >= since
            && project.is_none_or(|p| paths::is_within_recorded(&r.outcome.project, p))
    }) {
        let o = &r.outcome;
        let u = r.usage.as_ref();
        let fields = [
            cell(&o.task_id),
            cell(&o.project),
            cell(o.role.as_deref().unwrap_or("")),
            cell(o.class.map(policy::class_name).unwrap_or("")),
            cell(&o.choice.model),
            cell(o.choice.effort.as_deref().unwrap_or("")),
            cell(&format!("{:?}", o.result).to_ascii_lowercase()),
            o.gate_passed.map(|g| g.to_string()).unwrap_or_default(),
            o.escalations.to_string(),
            iso(o.started_at),
            iso(o.ended_at),
            format!(
                "{}",
                round((o.ended_at - o.started_at).max(0) as f64 / 60_000.0, 1)
            ),
            cell(r.account.as_deref().unwrap_or("")),
            r.pays_per_token.map(|p| p.to_string()).unwrap_or_default(),
            u.map(|u| u.input.to_string()).unwrap_or_default(),
            u.map(|u| u.output.to_string()).unwrap_or_default(),
            u.map(|u| u.cache_read.to_string()).unwrap_or_default(),
            u.map(|u| u.cache_write.to_string()).unwrap_or_default(),
            num(r.cost_usd),
            num(r.window_pct),
            cell(r.escalation.as_deref().unwrap_or("")),
            cell(r.note.as_deref().unwrap_or("")),
        ];
        out.push_str(&fields.join(","));
        out.push_str("\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::index::window::WindowUsage;
    use crate::orchestration::metrics::TaskOutcome;
    use crate::orchestration::router::{ModelChoice, TaskClass};

    #[allow(clippy::too_many_arguments)]
    fn rec(
        task: &str,
        class: Option<TaskClass>,
        model: &str,
        effort: &str,
        result: TaskResult,
        gate: Option<bool>,
        cost: Option<f64>,
        minutes: i64,
    ) -> OutcomeRecord {
        OutcomeRecord {
            outcome: TaskOutcome {
                task_id: task.into(),
                project: "/Users/me/app".into(),
                role: None,
                class,
                choice: ModelChoice::new(model, Some(effort)),
                session_id: None,
                started_at: 1_000,
                ended_at: 1_000 + minutes * 60_000,
                result,
                gate_passed: gate,
                escalations: 0,
            },
            recorded_at: 2_000,
            account: Some("default".into()),
            pays_per_token: Some(true),
            usage: cost.map(|c| WindowUsage {
                input: 1_000,
                output: 500,
                cache_read: 8_000,
                cache_write: 1_000,
                messages: 3,
                cost_usd: c,
                ..Default::default()
            }),
            cost_usd: cost,
            window_pct: None,
            note: None,
            escalation: None,
            joined: true,
        }
    }

    #[test]
    fn cost_per_completed_task_counts_every_attempt() {
        use TaskResult::*;
        let impl_ = Some(TaskClass::Implement);
        let records = vec![
            // Sonnet: one failed attempt ($1), one escalated ($1), two done ($0.5 each).
            rec(
                "a",
                impl_,
                "sonnet",
                "medium",
                Failed,
                Some(false),
                Some(1.0),
                10,
            ),
            rec(
                "a",
                impl_,
                "sonnet",
                "medium",
                Escalated,
                Some(false),
                Some(1.0),
                10,
            ),
            rec(
                "b",
                impl_,
                "sonnet",
                "medium",
                Done,
                Some(true),
                Some(0.5),
                20,
            ),
            rec("c", impl_, "sonnet", "medium", Done, None, Some(0.5), 40),
            // The escalated task done on Opus ($2).
            rec("a", impl_, "opus", "high", Done, Some(true), Some(2.0), 30),
            // Unknown usage still counts as an attempt.
            rec("d", None, "haiku", "low", Cancelled, None, None, 1),
        ];
        let (rows, classes, totals, pending) = build(&records, 0, None);
        assert_eq!(pending, 0);
        assert_eq!(rows.len(), 3);
        let sonnet = &rows[0];
        assert_eq!(
            (sonnet.class.as_str(), sonnet.model.as_str()),
            ("implement", "sonnet")
        );
        assert_eq!(
            (
                sonnet.tasks,
                sonnet.completed,
                sonnet.failed,
                sonnet.escalated
            ),
            (4, 2, 1, 1)
        );
        assert_eq!(sonnet.cost_usd, 3.0);
        assert_eq!(sonnet.cost_per_completed_usd, Some(1.5));
        assert_eq!(sonnet.escalation_rate, Some(0.25));
        assert_eq!(sonnet.gate_pass_rate, Some(0.3333));
        assert_eq!(sonnet.minutes_per_completed, Some(30.0));
        // 8,000 cache reads of 10,000 input tokens per end.
        assert_eq!(sonnet.cache_hit_ratio, Some(0.8));
        assert_eq!(sonnet.tokens_per_completed, Some(4 * 10_500 / 2));
        let opus = &rows[1];
        assert_eq!(opus.model, "opus");
        assert_eq!(opus.cost_per_completed_usd, Some(2.0));
        // The class all told: $5 for 3 completed tasks.
        assert_eq!(classes[0].class, "implement");
        assert_eq!(classes[0].cost_usd, 5.0);
        assert_eq!(classes[0].completed, 3);
        assert_eq!(classes[0].cost_per_completed_usd, Some(1.666667));
        assert_eq!(classes[1].class, "unclassified");
        assert_eq!(classes[1].cost_per_completed_usd, None, "nothing completed");
        assert_eq!(totals.tasks, 6);
        assert_eq!(totals.with_usage, 5);
        assert_eq!(totals.per_token, 6);
        // Filters: by time and by project.
        assert_eq!(build(&records, i64::MAX, None).2.tasks, 0);
        assert_eq!(build(&records, 0, Some("/Users/me/other")).2.tasks, 0);
        assert_eq!(build(&records, 0, Some("/Users/me/app")).2.tasks, 6);
    }

    #[test]
    fn csv_is_quoted_and_safe_to_open() {
        assert_eq!(cell("plain"), "plain");
        assert_eq!(cell("a,b"), "\"a,b\"");
        assert_eq!(cell("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(cell("=HYPERLINK(1)"), "'=HYPERLINK(1)");
        assert_eq!(cell("-1"), "'-1");
        let records = vec![rec(
            "=evil",
            Some(TaskClass::Fix),
            "sonnet",
            "medium",
            TaskResult::Done,
            Some(true),
            Some(0.25),
            5,
        )];
        let (rows, classes, totals, _) = build(&records, 0, None);
        let csv = rows_csv(&rows, &classes, &totals);
        let lines: Vec<&str> = csv.split("\r\n").filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 4, "header, one group, one class, the total");
        assert!(lines[0].starts_with("class,model,effort,tasks"));
        assert!(lines[0].contains("est_cost_usd"));
        assert!(lines[1].starts_with("fix,sonnet,medium,1,1,0,0,0,0.25,0.25,"));
        assert!(lines[3].starts_with("all,all,all,1,"));
        let raw = outcomes_csv(&records, 0, None);
        assert!(
            raw.contains("'=evil,/Users/me/app,,fix,sonnet,medium,done,true,0,"),
            "{raw}"
        );
    }
}
