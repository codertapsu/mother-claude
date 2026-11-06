//! The scheduler's settings (docs/ROUTING.md): caps, the memory reserve,
//! the plan-window thresholds, budgets, alerts, which accounts pay per
//! token, and the optional Haiku classifier — `<app data dir>/scheduler.json`
//! (0600, atomic, through `crate::persist`), written only when the user
//! changes something.
//!
//! Read tolerantly: each field is taken on its own, so one bad value falls
//! back to its default instead of losing the file; fields a newer version
//! wrote are kept and written back unchanged.
//!
//! An update ([`Limits::patched`]) names only what changes: a key present
//! replaces the current value (`null` turns an optional cap off), objects
//! (`budgets`, `alerts`) merge key by key, and the maps (`projectCaps`,
//! `budgets.projects`, `budgets.roles`, `billing`) are replaced whole.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::capacity::DEFAULT_USER_CAP;
use crate::claude::paths;

/// The settings file in the app data dir.
pub const FILE_NAME: &str = "scheduler.json";
/// Role sessions in one project at once unless it has its own cap (plan §9
/// `repo_cap`: 4 editing roles).
pub const DEFAULT_PROJECT_CAP: usize = 4;
/// Above this share of the 5-hour window only the lead starts (§9).
pub const DEFAULT_LEAD_ONLY_PCT: f64 = 85.0;
/// Above this share of the 5-hour window nothing starts (§9).
pub const DEFAULT_STOP_PCT: f64 = 95.0;
/// Above this share of the Opus weekly window the router steps Opus work
/// down to Sonnet at high effort (§7.1).
pub const DEFAULT_OPUS_GATE_PCT: f64 = 85.0;
/// A role session's `--max-budget-usd` on accounts that pay per token
/// unless the role has its own (the plan's template example: $5).
pub const DEFAULT_SESSION_USD: f64 = 5.0;
/// Budget alerts (and the router's budget gate) start here.
pub const DEFAULT_NEAR_PCT: f64 = 80.0;
/// The most entries one map of the settings holds.
const MAP_MAX: usize = 200;
/// The largest budget accepted, in USD.
const USD_MAX: f64 = 1_000_000.0;

/// Whether an account pays per token or through a plan's usage windows,
/// when the user says so (auto-detected otherwise, `budget::billing`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Billing {
    PerToken,
    Plan,
}

/// Budgets, in estimated US dollars.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Budgets {
    /// Every project's daily budget unless it has its own (`None`: none).
    pub project_daily_usd: Option<f64>,
    /// A project's own daily budget, by project folder.
    pub projects: BTreeMap<String, f64>,
    /// One task's budget (`None`: none).
    pub task_usd: Option<f64>,
    /// A role session's `--max-budget-usd` on pay-per-token accounts
    /// (`None`: none).
    pub session_usd: Option<f64>,
    /// A role's own session budget, by role id.
    pub roles: BTreeMap<String, f64>,
    /// Alerts at this share and at 100 %; the router's budget gate starts
    /// here too.
    pub near_pct: f64,
}

impl Default for Budgets {
    fn default() -> Self {
        Self {
            project_daily_usd: None,
            projects: BTreeMap::new(),
            task_usd: None,
            session_usd: Some(DEFAULT_SESSION_USD),
            roles: BTreeMap::new(),
            near_pct: DEFAULT_NEAR_PCT,
        }
    }
}

/// Where budget alerts go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Alerts {
    /// A notice on the desktop.
    pub desktop: bool,
    /// A push notification to paired phones.
    pub push: bool,
}

impl Default for Alerts {
    fn default() -> Self {
        Self {
            desktop: true,
            push: true,
        }
    }
}

/// The scheduler's settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    /// Role sessions (workers) running at once; the lead is not counted.
    pub user_cap: usize,
    /// Role sessions running in one project at once unless it has its own.
    pub project_cap: usize,
    /// A project's own cap, by project folder.
    pub project_caps: BTreeMap<String, usize>,
    /// Memory that always stays free, in MB.
    pub reserve_mb: u64,
    /// What one role session needs, in MB (`None`: measured).
    pub session_mb: Option<u64>,
    /// Above this share of the 5-hour window only the lead starts.
    pub lead_only_pct: f64,
    /// Above this share of the 5-hour window nothing starts.
    pub stop_pct: f64,
    /// Above this share of the Opus weekly window Opus work runs on Sonnet.
    pub opus_gate_pct: f64,
    pub budgets: Budgets,
    pub alerts: Alerts,
    /// Ask Claude Haiku to classify tasks that arrive without a class
    /// (spends tokens; needs an API key for the direct Messages API path).
    pub haiku_classifier: bool,
    /// Accounts the user says pay per token or through a plan.
    pub billing: BTreeMap<String, Billing>,
    /// Fields a newer version wrote, kept as they were.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            user_cap: DEFAULT_USER_CAP,
            project_cap: DEFAULT_PROJECT_CAP,
            project_caps: BTreeMap::new(),
            reserve_mb: super::host::DEFAULT_RESERVE / super::host::MIB,
            session_mb: None,
            lead_only_pct: DEFAULT_LEAD_ONLY_PCT,
            stop_pct: DEFAULT_STOP_PCT,
            opus_gate_pct: DEFAULT_OPUS_GATE_PCT,
            budgets: Budgets::default(),
            alerts: Alerts::default(),
            haiku_classifier: false,
            billing: BTreeMap::new(),
            extra: BTreeMap::new(),
        }
    }
}

/// Why an update was refused: the field (a JSON path) and what is wrong, in
/// the user's words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid {
    pub field: String,
    pub message: String,
}

impl Invalid {
    fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

/// Keys whose value is a map replaced whole by an update.
const WHOLE: [&str; 4] = ["projectCaps", "projects", "roles", "billing"];

/// Take `obj[key]` into `target` when it parses; keep the default otherwise.
fn take<T: DeserializeOwned>(obj: &Map<String, Value>, key: &str, target: &mut T) {
    if let Some(v) = obj.get(key) {
        match serde_json::from_value::<T>(v.clone()) {
            Ok(value) => *target = value,
            Err(e) => tracing::warn!(key, error = %e, "ignoring a bad scheduler setting"),
        }
    }
}

/// A map whose good entries are kept and bad ones dropped.
fn take_map<V: DeserializeOwned>(
    obj: &Map<String, Value>,
    key: &str,
    target: &mut BTreeMap<String, V>,
) {
    let Some(entries) = obj.get(key).and_then(Value::as_object) else {
        return;
    };
    target.clear();
    for (k, v) in entries {
        match serde_json::from_value::<V>(v.clone()) {
            Ok(value) => {
                target.insert(k.clone(), value);
            }
            Err(e) => tracing::warn!(key, error = %e, "ignoring a bad scheduler setting entry"),
        }
    }
}

const KNOWN: [&str; 12] = [
    "userCap",
    "projectCap",
    "projectCaps",
    "reserveMb",
    "sessionMb",
    "leadOnlyPct",
    "stopPct",
    "opusGatePct",
    "budgets",
    "alerts",
    "haikuClassifier",
    "billing",
];

impl Limits {
    /// `<data_dir>/scheduler.json`.
    pub fn path(data_dir: &Path) -> PathBuf {
        data_dir.join(FILE_NAME)
    }

    /// The saved settings, or the defaults (no file, unreadable file).
    pub fn load(path: Option<&Path>) -> Self {
        path.and_then(crate::persist::read_json::<Value>)
            .map(|v| Self::from_value(&v))
            .unwrap_or_default()
    }

    /// Settings from JSON, field by field: a bad value keeps its default.
    pub fn from_value(v: &Value) -> Self {
        let mut out = Self::default();
        let Some(obj) = v.as_object() else {
            return out;
        };
        take(obj, "userCap", &mut out.user_cap);
        take(obj, "projectCap", &mut out.project_cap);
        take_map(obj, "projectCaps", &mut out.project_caps);
        take(obj, "reserveMb", &mut out.reserve_mb);
        take(obj, "sessionMb", &mut out.session_mb);
        take(obj, "leadOnlyPct", &mut out.lead_only_pct);
        take(obj, "stopPct", &mut out.stop_pct);
        take(obj, "opusGatePct", &mut out.opus_gate_pct);
        if let Some(b) = obj.get("budgets").and_then(Value::as_object) {
            take(b, "projectDailyUsd", &mut out.budgets.project_daily_usd);
            take_map(b, "projects", &mut out.budgets.projects);
            take(b, "taskUsd", &mut out.budgets.task_usd);
            take(b, "sessionUsd", &mut out.budgets.session_usd);
            take_map(b, "roles", &mut out.budgets.roles);
            take(b, "nearPct", &mut out.budgets.near_pct);
        }
        if let Some(a) = obj.get("alerts").and_then(Value::as_object) {
            take(a, "desktop", &mut out.alerts.desktop);
            take(a, "push", &mut out.alerts.push);
        }
        take(obj, "haikuClassifier", &mut out.haiku_classifier);
        take_map(obj, "billing", &mut out.billing);
        out.extra = obj
            .iter()
            .filter(|(k, _)| !KNOWN.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        // A value that loaded but is out of range falls back too.
        if out.validate().is_err() {
            out.repair();
        }
        out
    }

    /// Reset whatever is out of range to its default (a hand-edited file).
    fn repair(&mut self) {
        let d = Self::default();
        if !(1..=16).contains(&self.user_cap) {
            self.user_cap = d.user_cap;
        }
        if !(1..=16).contains(&self.project_cap) {
            self.project_cap = d.project_cap;
        }
        self.project_caps
            .retain(|k, v| clean_project(k).is_ok() && (1..=16).contains(v));
        if self.reserve_mb > 1024 * 1024 {
            self.reserve_mb = d.reserve_mb;
        }
        if self.session_mb.is_some_and(|m| !(50..=65_536).contains(&m)) {
            self.session_mb = None;
        }
        if !pct_ok(self.lead_only_pct)
            || !pct_ok(self.stop_pct)
            || self.stop_pct < self.lead_only_pct
        {
            self.lead_only_pct = d.lead_only_pct;
            self.stop_pct = d.stop_pct;
        }
        if !pct_ok(self.opus_gate_pct) {
            self.opus_gate_pct = d.opus_gate_pct;
        }
        let b = &mut self.budgets;
        if b.project_daily_usd.is_some_and(|v| !usd_ok(v)) {
            b.project_daily_usd = None;
        }
        b.projects
            .retain(|k, v| clean_project(k).is_ok() && usd_ok(*v));
        if b.task_usd.is_some_and(|v| !usd_ok(v)) {
            b.task_usd = None;
        }
        if b.session_usd.is_some_and(|v| !usd_ok(v)) {
            b.session_usd = Some(DEFAULT_SESSION_USD);
        }
        b.roles.retain(|k, v| role_ok(k) && usd_ok(*v));
        if !(1.0..=99.0).contains(&b.near_pct) || !b.near_pct.is_finite() {
            b.near_pct = DEFAULT_NEAR_PCT;
        }
        self.billing
            .retain(|k, _| crate::claude::accounts::is_valid_id(k));
    }

    /// Whether every value is in range, else the first that is not.
    pub fn validate(&self) -> Result<(), Invalid> {
        if !(1..=16).contains(&self.user_cap) {
            return Err(Invalid::new(
                "userCap",
                "Your limit must be between 1 and 16.",
            ));
        }
        if !(1..=16).contains(&self.project_cap) {
            return Err(Invalid::new(
                "projectCap",
                "A project's limit must be between 1 and 16.",
            ));
        }
        if self.project_caps.len() > MAP_MAX {
            return Err(Invalid::new("projectCaps", "Too many projects."));
        }
        for (project, cap) in &self.project_caps {
            clean_project(project).map_err(|m| Invalid::new("projectCaps", m))?;
            if !(1..=16).contains(cap) {
                return Err(Invalid::new(
                    "projectCaps",
                    "A project's limit must be between 1 and 16.",
                ));
            }
        }
        if self.reserve_mb > 1024 * 1024 {
            return Err(Invalid::new(
                "reserveMb",
                "The memory reserve must be at most 1 TB.",
            ));
        }
        if self.session_mb.is_some_and(|m| !(50..=65_536).contains(&m)) {
            return Err(Invalid::new(
                "sessionMb",
                "A session's memory must be between 50 MB and 64 GB (or measured).",
            ));
        }
        for (field, v) in [
            ("leadOnlyPct", self.lead_only_pct),
            ("stopPct", self.stop_pct),
            ("opusGatePct", self.opus_gate_pct),
        ] {
            if !pct_ok(v) {
                return Err(Invalid::new(
                    field,
                    "A percentage must be between 1 and 100.",
                ));
            }
        }
        if self.stop_pct < self.lead_only_pct {
            return Err(Invalid::new(
                "stopPct",
                "Nothing-starts must be at or above lead-only.",
            ));
        }
        let b = &self.budgets;
        for (field, v) in [
            ("budgets.projectDailyUsd", b.project_daily_usd),
            ("budgets.taskUsd", b.task_usd),
            ("budgets.sessionUsd", b.session_usd),
        ] {
            if v.is_some_and(|v| !usd_ok(v)) {
                return Err(Invalid::new(
                    field,
                    "A budget must be more than $0 and at most $1,000,000.",
                ));
            }
        }
        if b.projects.len() > MAP_MAX || b.roles.len() > MAP_MAX {
            return Err(Invalid::new("budgets", "Too many budgets."));
        }
        for (project, v) in &b.projects {
            clean_project(project).map_err(|m| Invalid::new("budgets.projects", m))?;
            if !usd_ok(*v) {
                return Err(Invalid::new(
                    "budgets.projects",
                    "A budget must be more than $0 and at most $1,000,000.",
                ));
            }
        }
        for (role, v) in &b.roles {
            if !role_ok(role) {
                return Err(Invalid::new(
                    "budgets.roles",
                    "A role id is letters, digits, '-', '_' or '.', up to 64.",
                ));
            }
            if !usd_ok(*v) {
                return Err(Invalid::new(
                    "budgets.roles",
                    "A budget must be more than $0 and at most $1,000,000.",
                ));
            }
        }
        if !(1.0..=99.0).contains(&b.near_pct) || !b.near_pct.is_finite() {
            return Err(Invalid::new(
                "budgets.nearPct",
                "The alert threshold must be between 1 and 99 %.",
            ));
        }
        if self.billing.len() > MAP_MAX {
            return Err(Invalid::new("billing", "Too many accounts."));
        }
        if let Some(id) = self
            .billing
            .keys()
            .find(|k| !crate::claude::accounts::is_valid_id(k))
        {
            return Err(Invalid::new("billing", format!("No account {id:?}.")));
        }
        Ok(())
    }

    /// These settings with `patch` applied (see the module docs), the
    /// project folders written plainly, and validated. Keys this version
    /// does not know are ignored (never stored).
    pub fn patched(&self, patch: &Value) -> Result<Self, Invalid> {
        let Some(patch) = patch.as_object() else {
            return Err(Invalid::new(
                "",
                "Send the settings to change as an object.",
            ));
        };
        check_types(patch)?;
        let known: Map<String, Value> = patch
            .iter()
            .filter(|(k, _)| KNOWN.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let mut current = serde_json::to_value(self)
            .map_err(|e| Invalid::new("", format!("Could not read the settings: {e}")))?;
        merge(&mut current, &known);
        let mut next: Self = serde_json::from_value(current)
            .map_err(|e| Invalid::new("", format!("Invalid settings: {e}")))?;
        next.project_caps = clean_keys(&next.project_caps, "projectCaps")?;
        next.budgets.projects = clean_keys(&next.budgets.projects, "budgets.projects")?;
        next.validate()?;
        Ok(next)
    }

    /// Save to `path` (0600, atomic).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        crate::persist::write_json_private(path, self)
    }

    /// The cap of role sessions in `project`.
    pub fn project_cap_for(&self, project: &str) -> usize {
        find(&self.project_caps, project)
            .copied()
            .unwrap_or(self.project_cap)
    }

    /// The daily budget of `project` (its own, else every project's).
    pub fn project_budget(&self, project: &str) -> Option<f64> {
        find(&self.budgets.projects, project)
            .copied()
            .or(self.budgets.project_daily_usd)
    }

    /// Bytes that always stay free.
    pub fn reserve_bytes(&self) -> u64 {
        self.reserve_mb.saturating_mul(super::host::MIB)
    }
}

/// `map[project]`, the key compared as a recorded path (a Windows path
/// without regard to case or separators).
pub fn find<'a, V>(map: &'a BTreeMap<String, V>, project: &str) -> Option<&'a V> {
    map.get(project).or_else(|| {
        map.iter()
            .find(|(k, _)| paths::same_path(k, project))
            .map(|(_, v)| v)
    })
}

/// Merge `patch` into `current`: objects key by key, the [`WHOLE`] maps and
/// everything else replaced.
fn merge(current: &mut Value, patch: &Map<String, Value>) {
    let Some(obj) = current.as_object_mut() else {
        *current = Value::Object(patch.clone());
        return;
    };
    for (k, v) in patch {
        match (obj.get_mut(k), v) {
            (Some(cur @ Value::Object(_)), Value::Object(p)) if !WHOLE.contains(&k.as_str()) => {
                merge(cur, p)
            }
            _ => {
                obj.insert(k.clone(), v.clone());
            }
        }
    }
}

/// `v` as a `T`, or the field's error in the user's words.
fn check<T: DeserializeOwned>(v: &Value, field: &str) -> Result<(), Invalid> {
    serde_json::from_value::<T>(v.clone())
        .map(|_| ())
        .map_err(|e| Invalid::new(field, format!("Invalid {field}: {e}")))
}

/// Every known key of an update has the type its setting takes, so a
/// refusal names its field.
fn check_types(patch: &Map<String, Value>) -> Result<(), Invalid> {
    for (k, v) in patch {
        match k.as_str() {
            "userCap" | "projectCap" => check::<usize>(v, k)?,
            "projectCaps" => check::<BTreeMap<String, usize>>(v, k)?,
            "reserveMb" => check::<u64>(v, k)?,
            "sessionMb" => check::<Option<u64>>(v, k)?,
            "leadOnlyPct" | "stopPct" | "opusGatePct" => check::<f64>(v, k)?,
            "haikuClassifier" => check::<bool>(v, k)?,
            "billing" => check::<BTreeMap<String, Billing>>(v, k)?,
            "alerts" => {
                let o = v
                    .as_object()
                    .ok_or_else(|| Invalid::new(k, "Invalid alerts: send an object."))?;
                for (ak, av) in o {
                    if matches!(ak.as_str(), "desktop" | "push") {
                        check::<bool>(av, &format!("alerts.{ak}"))?;
                    }
                }
            }
            "budgets" => {
                let o = v
                    .as_object()
                    .ok_or_else(|| Invalid::new(k, "Invalid budgets: send an object."))?;
                for (bk, bv) in o {
                    let field = format!("budgets.{bk}");
                    match bk.as_str() {
                        "projectDailyUsd" | "taskUsd" | "sessionUsd" => {
                            check::<Option<f64>>(bv, &field)?
                        }
                        "projects" | "roles" => check::<BTreeMap<String, f64>>(bv, &field)?,
                        "nearPct" => check::<f64>(bv, &field)?,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// A project folder written plainly (no trailing separator), or why not.
pub fn clean_project(raw: &str) -> Result<String, String> {
    let p = raw.trim();
    if p.is_empty() || p.contains('\0') || p.len() > 4096 {
        return Err("Name a project by its full folder path.".into());
    }
    if !paths::is_absolute_anywhere(p) {
        return Err(paths::not_a_full_path(p));
    }
    if paths::has_dot_segments(p) {
        return Err(format!(
            "{p} names . or .., which a project folder does not."
        ));
    }
    Ok(paths::trim_end_separators(p).to_string())
}

fn clean_keys<V: Clone>(
    map: &BTreeMap<String, V>,
    field: &str,
) -> Result<BTreeMap<String, V>, Invalid> {
    let mut out = BTreeMap::new();
    for (k, v) in map {
        let clean = clean_project(k).map_err(|m| Invalid::new(field, m))?;
        out.insert(clean, v.clone());
    }
    Ok(out)
}

fn pct_ok(v: f64) -> bool {
    v.is_finite() && (1.0..=100.0).contains(&v)
}

fn usd_ok(v: f64) -> bool {
    v.is_finite() && v > 0.0 && v <= USD_MAX
}

/// A role id as templates name them.
pub fn role_ok(role: &str) -> bool {
    !role.is_empty()
        && role.len() <= 64
        && role
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_follow_the_plan() {
        let d = Limits::default();
        assert_eq!(d.user_cap, 3);
        assert_eq!(d.project_cap, 4);
        assert_eq!(d.reserve_mb, 2048);
        assert_eq!(
            (d.lead_only_pct, d.stop_pct, d.opus_gate_pct),
            (85.0, 95.0, 85.0)
        );
        assert_eq!(d.budgets.session_usd, Some(5.0));
        assert_eq!(d.budgets.project_daily_usd, None);
        assert!(d.alerts.desktop && d.alerts.push);
        assert!(!d.haiku_classifier);
        assert!(d.validate().is_ok());
    }

    #[test]
    fn a_bad_field_keeps_its_default_and_unknown_fields_survive() {
        let l = Limits::from_value(&json!({
            "userCap": "five",
            "projectCap": 6,
            "projectCaps": { "/Users/me/app": 2, "/Users/me/bad": "x", "relative/x": 3 },
            "stopPct": 97,
            "budgets": { "sessionUsd": null, "taskUsd": -1, "projects": { "/p": 20 } },
            "alerts": { "push": false },
            "billing": { "default": "perToken", "acc_000000000001": "nope" },
            "futureThing": { "a": 1 }
        }));
        assert_eq!(l.user_cap, 3, "a string is not a cap");
        assert_eq!(l.project_cap, 6);
        // The relative folder is dropped when the loaded file is repaired.
        assert_eq!(l.project_caps.len(), 1);
        assert_eq!(l.project_caps["/Users/me/app"], 2);
        assert_eq!(l.stop_pct, 97.0);
        assert_eq!(l.budgets.session_usd, None, "null turns the cap off");
        assert_eq!(l.budgets.task_usd, None, "a negative budget is dropped");
        assert_eq!(l.budgets.projects["/p"], 20.0);
        assert!(l.alerts.desktop && !l.alerts.push);
        assert_eq!(l.billing.len(), 1);
        assert_eq!(l.billing["default"], Billing::PerToken);
        assert_eq!(l.extra["futureThing"], json!({ "a": 1 }));
        // Written back with the unknown field.
        let v = serde_json::to_value(&l).unwrap();
        assert_eq!(v["futureThing"], json!({ "a": 1 }));
        assert!(v["budgets"]["sessionUsd"].is_null());
        // Garbage is the defaults.
        assert_eq!(Limits::from_value(&json!("nope")), Limits::default());
        assert_eq!(Limits::from_value(&json!({ "userCap": 99 })).user_cap, 3);
    }

    #[test]
    fn an_update_names_only_what_changes() {
        let base = Limits::default();
        let next = base
            .patched(&json!({
                "userCap": 5,
                "budgets": { "projects": { "/Users/me/app/": 12.5 } },
                "projectCaps": { "C:\\dev\\app\\": 2 }
            }))
            .unwrap();
        assert_eq!(next.user_cap, 5);
        assert_eq!(
            next.budgets.projects["/Users/me/app"], 12.5,
            "trailing / dropped"
        );
        assert_eq!(next.budgets.session_usd, Some(5.0), "untouched");
        assert_eq!(next.project_caps["C:\\dev\\app"], 2);
        assert_eq!(
            next.project_cap_for("c:/DEV/app"),
            2,
            "Windows paths compare loosely"
        );
        assert_eq!(next.project_cap_for("/elsewhere"), 4);
        // A map is replaced whole; null turns a cap off.
        let again = next
            .patched(&json!({ "budgets": { "projects": {}, "sessionUsd": null } }))
            .unwrap();
        assert!(again.budgets.projects.is_empty());
        assert_eq!(again.budgets.session_usd, None);
        assert_eq!(again.user_cap, 5);
    }

    #[test]
    fn an_update_out_of_range_is_refused_with_its_field() {
        let base = Limits::default();
        let bad = |patch: Value| base.patched(&patch).unwrap_err();
        assert_eq!(bad(json!({ "userCap": 0 })).field, "userCap");
        assert_eq!(bad(json!({ "userCap": 17 })).field, "userCap");
        assert_eq!(bad(json!({ "stopPct": 80 })).field, "stopPct");
        assert_eq!(
            bad(json!({ "budgets": { "projectDailyUsd": 0 } })).field,
            "budgets.projectDailyUsd"
        );
        assert_eq!(
            bad(json!({ "budgets": { "projects": { "relative": 5 } } })).field,
            "budgets.projects"
        );
        assert_eq!(
            bad(json!({ "budgets": { "roles": { "bad role!": 5 } } })).field,
            "budgets.roles"
        );
        assert_eq!(
            bad(json!({ "billing": { "nobody": "plan" } })).field,
            "billing"
        );
        assert_eq!(bad(json!({ "sessionMb": 10 })).field, "sessionMb");
        assert_eq!(bad(json!({ "userCap": "x" })).field, "userCap");
        assert_eq!(
            bad(json!({ "budgets": { "nearPct": "x" } })).field,
            "budgets.nearPct"
        );
        assert_eq!(bad(json!({ "alerts": { "push": 1 } })).field, "alerts.push");
        assert!(base.patched(&json!([1])).is_err());
        // Unknown keys are not stored.
        let next = base.patched(&json!({ "stash": "anything" })).unwrap();
        assert!(next.extra.is_empty());
    }

    #[test]
    fn saved_settings_round_trip_privately() {
        let dir = tempfile::tempdir().unwrap();
        let path = Limits::path(dir.path());
        assert_eq!(Limits::load(Some(&path)), Limits::default());
        let l = Limits::default()
            .patched(&json!({ "userCap": 2, "haikuClassifier": true }))
            .unwrap();
        l.save(&path).unwrap();
        assert_eq!(Limits::load(Some(&path)), l);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(Limits::load(Some(&path)), Limits::default());
    }
}
