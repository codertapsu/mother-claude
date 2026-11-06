//! The model router (docs/PLAN-BROWSER-AND-TEAMS.md §7, docs/ROUTING.md):
//! which model and effort a task runs on.
//!
//! **Contract** (the teams side calls [`choose`] when it starts a role session
//! or hands a task to one). The types are as fixed before the work began;
//! the policy, in order of precedence (§7.1):
//!
//! 1. **Your override** on the task or the role: always wins.
//! 2. **Escalation**: a task's own escalations ([`TaskFacts::escalation`];
//!    [`escalation_for`] counts them from recorded failed gates) and its
//!    class's in this project (raised when a task of the class escalates,
//!    one step back after two clean completions) — whichever is higher —
//!    climb the policy's ladder (effort before model; Fable only when the
//!    policy allows it).
//! 3. **The budget gate**: Opus and Fable work runs on Sonnet at high effort
//!    while the account's Opus weekly window is at 85 % or more, or the
//!    project's daily budget is nearly spent; Fable runs only when you
//!    choose it.
//! 4. **The task class** → the policy's model and effort.
//! 5. **The role's default**, else Sonnet at medium.
//!
//! Every decision carries its reason; the last ones are kept for the
//! Capacity page.

use serde::{Deserialize, Serialize};

use super::limits::Limits;
use super::policy::{self, Policy};
use super::scheduler::{lock, now_ms, ROUTES_KEEP};
use crate::state::AppState;

/// What kind of work a task is (§7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskClass {
    Plan,
    Architecture,
    Review,
    Implement,
    Test,
    Fix,
    Search,
    Summarize,
    Classify,
    Ops,
    Other,
}

/// A model (a Claude Code alias — `opus`, `sonnet`, `haiku`, `fable` — or a
/// full id) and an effort level (`low` … `max`; `None`: the model's default).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelChoice {
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

impl ModelChoice {
    pub fn new(model: &str, effort: Option<&str>) -> Self {
        Self {
            model: model.to_string(),
            effort: effort.map(str::to_string),
        }
    }
}

/// What the router knows about a task when it chooses.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskFacts {
    /// The project folder.
    pub project: String,
    /// The role it is for (a template's role id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<TaskClass>,
    /// The role's model and effort, from its template.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_default: Option<ModelChoice>,
    /// Set by the user on the task or the role: always wins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_override: Option<ModelChoice>,
    /// How many times this task was escalated (a failed gate, a reviewer's
    /// "needs deeper reasoning").
    #[serde(default)]
    pub escalation: u8,
    /// 0 (highest) … 3.
    #[serde(default)]
    pub priority: u8,
}

/// The router's answer, with the reason the UI shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub choice: ModelChoice,
    pub reason: String,
}

/// What the budget gate and the escalation state read, gathered by
/// [`choose`] (synthetic in tests).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RouteSignals {
    /// The Opus weekly window (or the weekly one when the plan has no Opus
    /// window): percent used and its name.
    pub opus_weekly: Option<(f64, &'static str)>,
    /// The project's daily budget: spent and limit (estimated USD).
    pub budget: Option<(f64, f64)>,
    /// The task class's escalation in this project.
    pub class_level: u8,
}

/// A routing decision as the Capacity page lists it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteNote {
    pub at: i64,
    pub project: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<TaskClass>,
    pub choice: ModelChoice,
    pub reason: String,
}

fn steps_label(n: u8) -> String {
    if n == 1 {
        "1 step".into()
    } else {
        format!("{n} steps")
    }
}

fn pct_label(p: f64) -> String {
    if p.fract() == 0.0 {
        format!("{p:.0}")
    } else {
        format!("{p:.1}")
    }
}

/// The decision itself: pure, every signal given.
pub fn decide_route(
    policy: &Policy,
    limits: &Limits,
    s: &RouteSignals,
    facts: &TaskFacts,
) -> Decision {
    // 1. Your override.
    if let Some(choice) = &facts.user_override {
        return Decision {
            choice: choice.clone(),
            reason: "set by you".into(),
        };
    }

    // 4–5. The base: the class's policy, the role's default, the default.
    let class_choice = facts
        .class
        .and_then(|c| policy.for_class(c).map(|ch| (c, ch)));
    let (base, base_reason) = match (class_choice, &facts.role_default) {
        (Some((c, ch)), _) => (
            ch.clone(),
            format!("the policy for {} tasks", policy::class_label(c)),
        ),
        (None, Some(rd)) => (rd.clone(), "the role's default".to_string()),
        (None, None) => (
            ModelChoice::new("sonnet", Some("medium")),
            "the default for a task without a role".to_string(),
        ),
    };

    // 2. Escalation: the task's own or its class's, whichever is higher.
    let steps = facts.escalation.max(s.class_level);
    if steps > 0 {
        let (choice, mut reason) = match policy.escalate(&base, steps) {
            Some(up) => {
                let why = if facts.escalation >= s.class_level {
                    format!(
                        "escalated {} from {}",
                        steps_label(steps),
                        policy::describe(&base)
                    )
                } else {
                    format!(
                        "{} tasks in this project escalated recently ({} from {}; {} clean \
                         completions step back down)",
                        facts.class.map(policy::class_label).unwrap_or("these"),
                        steps_label(steps),
                        policy::describe(&base),
                        policy.clean_to_step_down
                    )
                };
                (up, why)
            }
            None => (
                base.clone(),
                format!("{base_reason}; already at the top of the escalation ladder"),
            ),
        };
        // Fable only when you choose it (or the policy lets escalation climb there).
        if policy::tier(&choice.model) >= 3 && !policy.fable_on_escalation {
            let opus = ModelChoice::new("opus", Some("xhigh"));
            reason.push_str("; Fable only when you choose it, so Opus at xhigh");
            return Decision {
                choice: opus,
                reason,
            };
        }
        return Decision { choice, reason };
    }

    // 3. Fable only on your explicit choice.
    let (mut choice, mut reason) = (base, base_reason);
    if policy::tier(&choice.model) >= 3 {
        let effort = choice.effort.clone().unwrap_or_else(|| "high".into());
        reason =
            format!("{reason}, but Fable runs only when you choose it: Opus at {effort} instead");
        choice = ModelChoice::new("opus", Some(&effort));
    }

    // 3. The budget gate, for Opus-tier work.
    if policy::tier(&choice.model) >= 2 {
        let gate = &policy.gate_choice;
        let gate_text = policy::describe(gate);
        if let Some((pct, label)) = s.opus_weekly.filter(|(p, _)| *p >= limits.opus_gate_pct) {
            return Decision {
                choice: gate.clone(),
                reason: format!(
                    "the {label} window is at {} % (the gate is {} %): {gate_text} instead of {}",
                    pct_label(pct),
                    pct_label(limits.opus_gate_pct),
                    policy::describe(&choice)
                ),
            };
        }
        if let Some((spent, limit)) = s.budget.filter(|(_, l)| *l > 0.0) {
            let share = spent / limit * 100.0;
            if share >= limits.budgets.near_pct {
                return Decision {
                    choice: gate.clone(),
                    reason: format!(
                        "the project's daily budget is {:.0} % spent (${spent:.2} of \
                         ${limit:.2}, estimated): {gate_text} instead of {}",
                        share.floor(),
                        policy::describe(&choice)
                    ),
                };
            }
        }
    }
    Decision { choice, reason }
}

/// The Opus weekly window of an account: `seven_day_opus` when the plan has
/// one, else the weekly window (which then limits Opus too).
pub fn opus_weekly_of(state: &AppState, account: &str) -> Option<(f64, &'static str)> {
    let usage = crate::claude::usage::current_for(state, account);
    if !usage.available {
        return None;
    }
    let find = |k: &str| usage.windows.iter().find(|w| w.key == k).map(|w| w.pct);
    find("seven_day_opus")
        .map(|p| (p, "Opus weekly"))
        .or_else(|| find("seven_day").map(|p| (p, "weekly")))
}

/// Gather what a decision about `facts` reads now.
pub fn route_signals(state: &AppState, facts: &TaskFacts) -> RouteSignals {
    let limits = state.scheduler.limits();
    let account = state.accounts.account_for_cwd(&facts.project);
    RouteSignals {
        opus_weekly: opus_weekly_of(state, &account),
        budget: limits
            .project_budget(&facts.project)
            .map(|limit| (state.scheduler.spend().project_today(&facts.project), limit)),
        class_level: facts
            .class
            .map(|c| lock(&state.scheduler.learned).class_level(&facts.project, c))
            .unwrap_or(0),
    }
}

/// Choose a model and effort for a task.
pub fn choose(state: &AppState, facts: &TaskFacts) -> Decision {
    let policy = state.scheduler.policy();
    let limits = state.scheduler.limits();
    let signals = route_signals(state, facts);
    let decision = decide_route(&policy, &limits, &signals, facts);
    let mut routes = lock(&state.scheduler.routes);
    routes.push_front(RouteNote {
        at: now_ms(),
        project: facts.project.clone(),
        role: facts.role.clone(),
        class: facts.class,
        choice: decision.choice.clone(),
        reason: decision.reason.clone(),
    });
    routes.truncate(ROUTES_KEEP);
    decision
}

/// The escalation steps a task earned from its recorded failed gates
/// (every [`Policy::gates_per_step`] failed gates, one step), for
/// [`TaskFacts::escalation`] when the caller does not count them itself.
pub fn escalation_for(state: &AppState, task_id: &str) -> u8 {
    let policy = state.scheduler.policy();
    lock(&state.scheduler.learned).task_steps(task_id, &policy)
}

/// The `--fallback-model` for a choice (`opus` → `sonnet`), if the policy
/// gives its model family one.
pub fn fallback_for(state: &AppState, choice: &ModelChoice) -> Option<String> {
    state.scheduler.policy().fallback_for(choice)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mc(m: &str, e: &str) -> ModelChoice {
        ModelChoice::new(m, Some(e))
    }

    fn facts(class: Option<TaskClass>) -> TaskFacts {
        TaskFacts {
            project: "/Users/me/app".into(),
            role: Some("backend".into()),
            class,
            ..Default::default()
        }
    }

    fn route(s: &RouteSignals, f: &TaskFacts) -> Decision {
        decide_route(&Policy::default(), &Limits::default(), s, f)
    }

    #[test]
    fn your_override_always_wins() {
        let mut f = facts(Some(TaskClass::Review));
        f.user_override = Some(mc("fable", "max"));
        f.escalation = 3;
        let s = RouteSignals {
            opus_weekly: Some((99.0, "Opus weekly")),
            ..Default::default()
        };
        let d = route(&s, &f);
        assert_eq!(d.choice, mc("fable", "max"));
        assert_eq!(d.reason, "set by you");
    }

    #[test]
    fn the_class_policy_comes_before_the_role_default() {
        let mut f = facts(Some(TaskClass::Review));
        f.role_default = Some(mc("sonnet", "low"));
        let d = route(&RouteSignals::default(), &f);
        assert_eq!(d.choice, mc("opus", "high"));
        assert_eq!(d.reason, "the policy for review tasks");
        // `other` has no policy entry: the role's default.
        let mut f = facts(Some(TaskClass::Other));
        f.role_default = Some(mc("sonnet", "low"));
        let d = route(&RouteSignals::default(), &f);
        assert_eq!(d.choice, mc("sonnet", "low"));
        assert_eq!(d.reason, "the role's default");
        let d = route(&RouteSignals::default(), &facts(None));
        assert_eq!(d.choice, mc("sonnet", "medium"));
        assert_eq!(d.reason, "the default for a task without a role");
    }

    #[test]
    fn escalation_climbs_and_says_from_where() {
        let mut f = facts(Some(TaskClass::Implement));
        f.escalation = 1;
        let d = route(&RouteSignals::default(), &f);
        assert_eq!(d.choice, mc("sonnet", "high"));
        assert_eq!(d.reason, "escalated 1 step from Sonnet · medium");
        f.escalation = 2;
        assert_eq!(
            route(&RouteSignals::default(), &f).choice,
            mc("opus", "high")
        );
        // Escalation comes before the budget gate.
        let gated = RouteSignals {
            opus_weekly: Some((95.0, "Opus weekly")),
            ..Default::default()
        };
        assert_eq!(route(&gated, &f).choice, mc("opus", "high"));
        // At the top, it stays.
        f.escalation = 9;
        let d = route(&RouteSignals::default(), &f);
        assert_eq!(d.choice, mc("opus", "xhigh"));
        let mut top = facts(Some(TaskClass::Review));
        top.role_default = None;
        top.escalation = 1;
        let d = decide_route(
            &Policy {
                classes: [("review".to_string(), mc("opus", "xhigh"))].into(),
                ..Policy::default()
            },
            &Limits::default(),
            &RouteSignals::default(),
            &top,
        );
        assert_eq!(d.choice, mc("opus", "xhigh"));
        assert!(d.reason.contains("already at the top"), "{}", d.reason);
    }

    #[test]
    fn a_class_escalation_applies_to_new_tasks_of_the_class() {
        let s = RouteSignals {
            class_level: 1,
            ..Default::default()
        };
        let d = route(&s, &facts(Some(TaskClass::Fix)));
        assert_eq!(d.choice, mc("sonnet", "high"));
        assert!(
            d.reason
                .starts_with("fix tasks in this project escalated recently"),
            "{}",
            d.reason
        );
        // A task's own, higher escalation wins without double counting.
        let mut f = facts(Some(TaskClass::Fix));
        f.escalation = 2;
        assert_eq!(route(&s, &f).choice, mc("opus", "high"));
    }

    #[test]
    fn the_budget_gate_steps_opus_down_to_sonnet_high() {
        let f = facts(Some(TaskClass::Plan));
        let s = RouteSignals {
            opus_weekly: Some((91.0, "Opus weekly")),
            ..Default::default()
        };
        let d = route(&s, &f);
        assert_eq!(d.choice, mc("sonnet", "high"));
        assert_eq!(
            d.reason,
            "the Opus weekly window is at 91 % (the gate is 85 %): Sonnet · high instead of Opus · high"
        );
        // Below the gate: Opus.
        let s = RouteSignals {
            opus_weekly: Some((84.9, "Opus weekly")),
            ..Default::default()
        };
        assert_eq!(route(&s, &f).choice, mc("opus", "high"));
        // A nearly spent budget gates too.
        let s = RouteSignals {
            budget: Some((17.2, 20.0)),
            ..Default::default()
        };
        let d = route(&s, &f);
        assert_eq!(d.choice, mc("sonnet", "high"));
        assert!(
            d.reason
                .starts_with("the project's daily budget is 86 % spent"),
            "{}",
            d.reason
        );
        // Sonnet work is never gated.
        let s = RouteSignals {
            opus_weekly: Some((99.0, "weekly")),
            budget: Some((30.0, 20.0)),
            ..Default::default()
        };
        assert_eq!(
            route(&s, &facts(Some(TaskClass::Implement))).choice,
            mc("sonnet", "medium")
        );
    }

    #[test]
    fn fable_runs_only_when_you_choose_it() {
        let mut f = facts(None);
        f.role_default = Some(mc("fable", "high"));
        let d = route(&RouteSignals::default(), &f);
        assert_eq!(d.choice, mc("opus", "high"));
        assert!(
            d.reason.contains("Fable runs only when you choose it"),
            "{}",
            d.reason
        );
        f.user_override = Some(mc("fable", "high"));
        assert_eq!(
            route(&RouteSignals::default(), &f).choice,
            mc("fable", "high")
        );
    }
}
