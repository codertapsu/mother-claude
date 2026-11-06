//! The routing policy (docs/ROUTING.md, plan §7.1): which model and effort
//! each task class gets, the escalation ladder, where the budget gate sends
//! Opus work, and the `--fallback-model` of each model — in
//! `<app data dir>/routing-policy.json` (0600, atomic), edited on the
//! desktop with validation; resetting removes the file.
//!
//! Defaults (the first template of §7.1): planning, architecture and review
//! on `opus` at `high`; implementation, tests and fixes on `sonnet` at
//! `medium`; search, summaries and classification on `haiku` at `low`;
//! operations on `sonnet` at `medium`. The ladder climbs effort before
//! model: haiku·low → haiku·high → sonnet·medium → sonnet·high → opus·high
//! → opus·xhigh, and Fable only when the user allows escalation to reach it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::limits::Invalid;
use super::router::{ModelChoice, TaskClass};

/// The policy file in the app data dir.
pub const FILE_NAME: &str = "routing-policy.json";
/// Effort levels, lowest first.
pub const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
/// The model aliases Claude Code takes, cheapest first.
pub const ALIASES: [&str; 4] = ["haiku", "sonnet", "opus", "fable"];
/// Every task class, in display order.
pub const CLASSES: [TaskClass; 11] = [
    TaskClass::Plan,
    TaskClass::Architecture,
    TaskClass::Review,
    TaskClass::Implement,
    TaskClass::Test,
    TaskClass::Fix,
    TaskClass::Search,
    TaskClass::Summarize,
    TaskClass::Classify,
    TaskClass::Ops,
    TaskClass::Other,
];
/// Ladder bounds.
const LADDER_MIN: usize = 2;
const LADDER_MAX: usize = 12;

/// A class's JSON name (`plan`, `summarize`, …).
pub fn class_name(c: TaskClass) -> &'static str {
    match c {
        TaskClass::Plan => "plan",
        TaskClass::Architecture => "architecture",
        TaskClass::Review => "review",
        TaskClass::Implement => "implement",
        TaskClass::Test => "test",
        TaskClass::Fix => "fix",
        TaskClass::Search => "search",
        TaskClass::Summarize => "summarize",
        TaskClass::Classify => "classify",
        TaskClass::Ops => "ops",
        TaskClass::Other => "other",
    }
}

/// A class in words ("review tasks").
pub fn class_label(c: TaskClass) -> &'static str {
    match c {
        TaskClass::Plan => "planning",
        TaskClass::Architecture => "architecture",
        TaskClass::Review => "review",
        TaskClass::Implement => "implementation",
        TaskClass::Test => "test",
        TaskClass::Fix => "fix",
        TaskClass::Search => "search",
        TaskClass::Summarize => "summary",
        TaskClass::Classify => "classification",
        TaskClass::Ops => "operations",
        TaskClass::Other => "other",
    }
}

/// A class by its JSON name (also `summarise`).
pub fn class_from(name: &str) -> Option<TaskClass> {
    let n = name.trim().to_ascii_lowercase();
    if n == "summarise" {
        return Some(TaskClass::Summarize);
    }
    CLASSES.into_iter().find(|c| class_name(*c) == n)
}

/// The model family of an alias or a full id: `haiku`, `sonnet`, `opus` or
/// `fable` (Mythos counts as Fable's tier).
pub fn family(model: &str) -> Option<&'static str> {
    let m = model.trim().to_ascii_lowercase();
    if m.contains("fable") || m.contains("mythos") {
        Some("fable")
    } else if m.contains("opus") {
        Some("opus")
    } else if m.contains("sonnet") {
        Some("sonnet")
    } else if m.contains("haiku") {
        Some("haiku")
    } else {
        None
    }
}

/// Tier of a model: haiku 0, sonnet 1, opus 2, fable 3 (unknown: sonnet's).
pub fn tier(model: &str) -> u8 {
    match family(model) {
        Some("haiku") => 0,
        Some("opus") => 2,
        Some("fable") => 3,
        _ => 1,
    }
}

/// Rank of an effort (no effort: the model's default, ranked as medium).
pub fn effort_rank(effort: Option<&str>) -> u8 {
    effort
        .and_then(|e| EFFORTS.iter().position(|x| *x == e))
        .map(|i| i as u8)
        .unwrap_or(1)
}

/// Where a choice sits: tier first, then effort.
pub fn rank(c: &ModelChoice) -> (u8, u8) {
    (tier(&c.model), effort_rank(c.effort.as_deref()))
}

/// A model Claude Code takes: an alias or a full `claude-…` id.
pub fn valid_model(m: &str) -> bool {
    if ALIASES.contains(&m) {
        return true;
    }
    let core = m.strip_suffix("[1m]").unwrap_or(m);
    core.len() <= 64
        && core.starts_with("claude-")
        && core.len() > "claude-".len()
        && core
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '.'))
}

pub fn valid_effort(e: &str) -> bool {
    EFFORTS.contains(&e)
}

/// "Opus · high", "Sonnet", "Opus 4.8 · xhigh".
pub fn describe(c: &ModelChoice) -> String {
    let model = if ALIASES.contains(&c.model.as_str()) {
        let mut chars = c.model.chars();
        chars
            .next()
            .map(|f| f.to_uppercase().chain(chars).collect())
            .unwrap_or_default()
    } else {
        crate::claude::pricing::model_label(&c.model)
    };
    match &c.effort {
        Some(e) => format!("{model} · {e}"),
        None => model,
    }
}

fn mc(model: &str, effort: &str) -> ModelChoice {
    ModelChoice::new(model, Some(effort))
}

/// The routing policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
    /// Task class (its JSON name) → model and effort; a class without an
    /// entry runs on the role's default.
    pub classes: BTreeMap<String, ModelChoice>,
    /// The escalation ladder, lowest rung first.
    pub ladder: Vec<ModelChoice>,
    /// Whether escalation may climb onto a Fable rung (otherwise Fable runs
    /// only when the user chooses it).
    pub fable_on_escalation: bool,
    /// Failed gates of one task that earn it one escalation step.
    pub gates_per_step: u8,
    /// Clean completions that take an escalated task class one step back.
    pub clean_to_step_down: u8,
    /// Where the budget gate sends Opus and Fable work.
    pub gate_choice: ModelChoice,
    /// `--fallback-model` by model family (`opus` → `sonnet`).
    pub fallbacks: BTreeMap<String, String>,
}

impl Default for Policy {
    fn default() -> Self {
        let mut classes = BTreeMap::new();
        for (c, choice) in [
            (TaskClass::Plan, mc("opus", "high")),
            (TaskClass::Architecture, mc("opus", "high")),
            (TaskClass::Review, mc("opus", "high")),
            (TaskClass::Implement, mc("sonnet", "medium")),
            (TaskClass::Test, mc("sonnet", "medium")),
            (TaskClass::Fix, mc("sonnet", "medium")),
            (TaskClass::Search, mc("haiku", "low")),
            (TaskClass::Summarize, mc("haiku", "low")),
            (TaskClass::Classify, mc("haiku", "low")),
            (TaskClass::Ops, mc("sonnet", "medium")),
        ] {
            classes.insert(class_name(c).to_string(), choice);
        }
        Self {
            classes,
            ladder: vec![
                mc("haiku", "low"),
                mc("haiku", "high"),
                mc("sonnet", "medium"),
                mc("sonnet", "high"),
                mc("opus", "high"),
                mc("opus", "xhigh"),
                mc("fable", "high"),
            ],
            fable_on_escalation: false,
            gates_per_step: 2,
            clean_to_step_down: 2,
            gate_choice: mc("sonnet", "high"),
            fallbacks: BTreeMap::from([
                ("opus".to_string(), "sonnet".to_string()),
                ("fable".to_string(), "opus".to_string()),
            ]),
        }
    }
}

fn choice_ok(c: &ModelChoice) -> Result<(), String> {
    if !valid_model(&c.model) {
        return Err(format!(
            "{:?} is not a model (opus, sonnet, haiku, fable or a full claude-… id).",
            c.model
        ));
    }
    if let Some(e) = &c.effort {
        if !valid_effort(e) {
            return Err(format!(
                "{e:?} is not an effort (low, medium, high, xhigh or max)."
            ));
        }
    }
    Ok(())
}

impl Policy {
    /// `<data_dir>/routing-policy.json`.
    pub fn path(data_dir: &Path) -> PathBuf {
        data_dir.join(FILE_NAME)
    }

    /// The saved policy, or the defaults. Read tolerantly: a bad class
    /// entry or rung is dropped, a bad field keeps its default.
    pub fn load(path: Option<&Path>) -> Self {
        path.and_then(crate::persist::read_json::<Value>)
            .map(|v| Self::from_value(&v))
            .unwrap_or_default()
    }

    pub fn from_value(v: &Value) -> Self {
        let mut out = Self::default();
        let Some(obj) = v.as_object() else {
            return out;
        };
        if let Some(classes) = obj.get("classes").and_then(Value::as_object) {
            out.classes = classes
                .iter()
                .filter_map(|(k, v)| {
                    let class = class_from(k)?;
                    let choice: ModelChoice = serde_json::from_value(v.clone()).ok()?;
                    choice_ok(&choice).ok()?;
                    Some((class_name(class).to_string(), choice))
                })
                .collect();
        }
        if let Some(rungs) = obj.get("ladder").and_then(Value::as_array) {
            let ladder: Vec<ModelChoice> = rungs
                .iter()
                .filter_map(|r| serde_json::from_value::<ModelChoice>(r.clone()).ok())
                .filter(|r| choice_ok(r).is_ok())
                .collect();
            if (LADDER_MIN..=LADDER_MAX).contains(&ladder.len()) {
                out.ladder = ladder;
            }
        }
        if let Some(b) = obj.get("fableOnEscalation").and_then(Value::as_bool) {
            out.fable_on_escalation = b;
        }
        if let Some(n) = obj
            .get("gatesPerStep")
            .and_then(Value::as_u64)
            .filter(|n| (1..=5).contains(n))
        {
            out.gates_per_step = n as u8;
        }
        if let Some(n) = obj
            .get("cleanToStepDown")
            .and_then(Value::as_u64)
            .filter(|n| (1..=10).contains(n))
        {
            out.clean_to_step_down = n as u8;
        }
        if let Some(c) = obj
            .get("gateChoice")
            .and_then(|v| serde_json::from_value::<ModelChoice>(v.clone()).ok())
            .filter(|c| choice_ok(c).is_ok())
        {
            out.gate_choice = c;
        }
        if let Some(f) = obj.get("fallbacks").and_then(Value::as_object) {
            out.fallbacks = f
                .iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                .filter(|(k, v)| fallback_ok(k, v).is_ok())
                .collect();
        }
        out
    }

    /// Whether the policy is usable, else the first problem and its field.
    pub fn validate(&self) -> Result<(), Invalid> {
        for (k, choice) in &self.classes {
            if class_from(k).is_none_or(|c| class_name(c) != k) {
                return Err(Invalid {
                    field: "classes".into(),
                    message: format!("{k:?} is not a task class."),
                });
            }
            choice_ok(choice).map_err(|m| Invalid {
                field: format!("classes.{k}"),
                message: m,
            })?;
        }
        if !(LADDER_MIN..=LADDER_MAX).contains(&self.ladder.len()) {
            return Err(Invalid {
                field: "ladder".into(),
                message: format!("The ladder has {LADDER_MIN} to {LADDER_MAX} rungs."),
            });
        }
        for (i, rung) in self.ladder.iter().enumerate() {
            choice_ok(rung).map_err(|m| Invalid {
                field: format!("ladder.{i}"),
                message: m,
            })?;
        }
        if self.ladder.windows(2).any(|w| rank(&w[1]) <= rank(&w[0])) {
            return Err(Invalid {
                field: "ladder".into(),
                message: "Each rung must be above the one before it (a stronger model, or \
                          the same model at a higher effort)."
                    .into(),
            });
        }
        if !(1..=5).contains(&self.gates_per_step) {
            return Err(Invalid {
                field: "gatesPerStep".into(),
                message: "Between 1 and 5 failed gates per step.".into(),
            });
        }
        if !(1..=10).contains(&self.clean_to_step_down) {
            return Err(Invalid {
                field: "cleanToStepDown".into(),
                message: "Between 1 and 10 clean completions.".into(),
            });
        }
        choice_ok(&self.gate_choice).map_err(|m| Invalid {
            field: "gateChoice".into(),
            message: m,
        })?;
        if tier(&self.gate_choice.model) >= 2 {
            return Err(Invalid {
                field: "gateChoice".into(),
                message: "The budget gate must send Opus work to a cheaper model.".into(),
            });
        }
        for (k, v) in &self.fallbacks {
            fallback_ok(k, v).map_err(|m| Invalid {
                field: "fallbacks".into(),
                message: m,
            })?;
        }
        Ok(())
    }

    /// A policy sent by the desktop, validated.
    pub fn parse(v: &Value) -> Result<Self, Invalid> {
        let p: Self = serde_json::from_value(v.clone()).map_err(|e| Invalid {
            field: String::new(),
            message: format!("Invalid policy: {e}"),
        })?;
        p.validate()?;
        Ok(p)
    }

    /// Save to `path` (0600, atomic).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        crate::persist::write_json_private(path, self)
    }

    /// The policy's choice for a class.
    pub fn for_class(&self, class: TaskClass) -> Option<&ModelChoice> {
        self.classes.get(class_name(class))
    }

    /// The rungs escalation may use.
    fn rungs(&self) -> Vec<&ModelChoice> {
        self.ladder
            .iter()
            .filter(|r| self.fable_on_escalation || tier(&r.model) < 3)
            .collect()
    }

    /// `base` moved `steps` rungs up the ladder: from the highest rung at
    /// or below it. `None` when it cannot climb (already at the top).
    pub fn escalate(&self, base: &ModelChoice, steps: u8) -> Option<ModelChoice> {
        if steps == 0 {
            return None;
        }
        let rungs = self.rungs();
        let here = rank(base);
        // Index of the highest rung at or below `base` (+1, so 0 = below all).
        let pos = rungs
            .iter()
            .rposition(|r| rank(r) <= here)
            .map_or(0, |i| i + 1);
        let target = (pos + usize::from(steps)).min(rungs.len());
        if target == 0 {
            return None;
        }
        let up = rungs[target - 1];
        (rank(up) > here).then(|| up.clone())
    }

    /// The `--fallback-model` for a choice, if its family has one.
    pub fn fallback_for(&self, choice: &ModelChoice) -> Option<String> {
        self.fallbacks.get(family(&choice.model)?).cloned()
    }
}

fn fallback_ok(family_key: &str, value: &str) -> Result<(), String> {
    if !ALIASES.contains(&family_key) {
        return Err(format!("{family_key:?} is not a model family."));
    }
    let models: Vec<&str> = value.split(',').map(str::trim).collect();
    if models.is_empty() || models.len() > 3 || models.iter().any(|m| !valid_model(m)) {
        return Err(format!(
            "{value:?}: up to three models, comma-separated (e.g. \"sonnet\")."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_default_policy_is_the_plans_first_template() {
        let p = Policy::default();
        assert!(p.validate().is_ok());
        let of = |c| p.for_class(c).cloned();
        assert_eq!(of(TaskClass::Review), Some(mc("opus", "high")));
        assert_eq!(of(TaskClass::Implement), Some(mc("sonnet", "medium")));
        assert_eq!(of(TaskClass::Fix), Some(mc("sonnet", "medium")));
        assert_eq!(of(TaskClass::Search), Some(mc("haiku", "low")));
        assert_eq!(of(TaskClass::Ops), Some(mc("sonnet", "medium")));
        assert_eq!(of(TaskClass::Other), None);
        assert_eq!(p.fallback_for(&mc("opus", "high")), Some("sonnet".into()));
        assert_eq!(
            p.fallback_for(&mc("claude-opus-5-5", "high")),
            Some("sonnet".into())
        );
        assert_eq!(p.fallback_for(&mc("haiku", "low")), None);
    }

    #[test]
    fn escalation_climbs_effort_before_model_and_never_onto_fable_unasked() {
        let p = Policy::default();
        let up = |m: &str, e: &str, steps| p.escalate(&mc(m, e), steps);
        assert_eq!(up("sonnet", "medium", 1), Some(mc("sonnet", "high")));
        assert_eq!(up("sonnet", "medium", 2), Some(mc("opus", "high")));
        assert_eq!(up("haiku", "low", 1), Some(mc("haiku", "high")));
        // Between rungs: from the highest rung below it.
        assert_eq!(up("sonnet", "low", 1), Some(mc("sonnet", "medium")));
        // The top without Fable is opus·xhigh.
        assert_eq!(up("opus", "high", 5), Some(mc("opus", "xhigh")));
        assert_eq!(up("opus", "xhigh", 1), None);
        assert_eq!(up("opus", "max", 1), None, "above the ladder");
        assert_eq!(up("sonnet", "medium", 0), None);
        // With Fable allowed, the last rung is reachable.
        let fable = Policy {
            fable_on_escalation: true,
            ..Policy::default()
        };
        assert_eq!(
            fable.escalate(&mc("opus", "xhigh"), 1),
            Some(mc("fable", "high"))
        );
        // A full id ranks by its family.
        assert_eq!(
            p.escalate(&mc("claude-sonnet-5-5", "high"), 1),
            Some(mc("opus", "high"))
        );
    }

    #[test]
    fn validation_names_the_field() {
        let p = Policy {
            ladder: vec![mc("opus", "high"), mc("sonnet", "high")],
            ..Policy::default()
        };
        assert_eq!(p.validate().unwrap_err().field, "ladder");
        let mut p = Policy::default();
        p.classes.insert("plan".into(), mc("gpt-5", "high"));
        assert_eq!(p.validate().unwrap_err().field, "classes.plan");
        let mut p = Policy::default();
        p.classes.insert("plan".into(), mc("opus", "ultra"));
        assert_eq!(p.validate().unwrap_err().field, "classes.plan");
        let mut p = Policy::default();
        p.classes.insert("dance".into(), mc("opus", "high"));
        assert_eq!(p.validate().unwrap_err().field, "classes");
        let p = Policy {
            gate_choice: mc("opus", "low"),
            ..Policy::default()
        };
        assert_eq!(p.validate().unwrap_err().field, "gateChoice");
        let mut p = Policy::default();
        p.fallbacks
            .insert("opus".into(), "sonnet,haiku,opus,fable".into());
        assert_eq!(p.validate().unwrap_err().field, "fallbacks");
        let p = Policy {
            gates_per_step: 0,
            ..Policy::default()
        };
        assert_eq!(p.validate().unwrap_err().field, "gatesPerStep");
        assert!(Policy::parse(&json!({ "classes": 1 })).is_err());
        assert!(valid_model("claude-opus-4-8[1m]"));
        assert!(!valid_model("claude-"));
        assert!(!valid_model("Opus"));
    }

    #[test]
    fn a_saved_policy_loads_tolerantly() {
        let p = Policy::from_value(&json!({
            "classes": {
                "plan": { "model": "sonnet", "effort": "high" },
                "summarise": { "model": "haiku" },
                "dance": { "model": "opus" },
                "fix": { "model": "nope" }
            },
            "ladder": [{ "model": "haiku" }],
            "gatesPerStep": 9,
            "fallbacks": { "opus": "sonnet", "sonnet": "gpt" }
        }));
        assert_eq!(p.classes.len(), 2);
        assert_eq!(p.classes["plan"], mc("sonnet", "high"));
        assert_eq!(p.classes["summarize"], ModelChoice::new("haiku", None));
        assert_eq!(
            p.ladder,
            Policy::default().ladder,
            "one rung is not a ladder"
        );
        assert_eq!(p.gates_per_step, 2);
        assert_eq!(p.fallbacks.len(), 1);
        assert_eq!(Policy::from_value(&json!(null)), Policy::default());

        let dir = tempfile::tempdir().unwrap();
        let path = Policy::path(dir.path());
        let mut edited = Policy::default();
        edited.classes.remove("ops");
        edited.save(&path).unwrap();
        assert_eq!(Policy::load(Some(&path)), edited);
    }

    #[test]
    fn names_and_descriptions() {
        for c in CLASSES {
            assert_eq!(class_from(class_name(c)), Some(c));
            let json = serde_json::to_value(c).unwrap();
            assert_eq!(json, json!(class_name(c)), "the contract's serde name");
        }
        assert_eq!(describe(&mc("opus", "high")), "Opus · high");
        assert_eq!(describe(&ModelChoice::new("sonnet", None)), "Sonnet");
        assert_eq!(
            describe(&mc("claude-opus-4-8", "xhigh")),
            "Opus 4.8 · xhigh"
        );
        assert_eq!(tier("claude-mythos-5-1"), 3);
        assert_eq!(effort_rank(None), 1);
    }
}
