//! The model router (docs/PLAN-BROWSER-AND-TEAMS.md §7): which model and
//! effort a task runs on.
//!
//! **Contract** (the teams side calls [`choose`] when it starts a role session
//! or hands a task to one). The body here is the first, static rule; the
//! policy engine replaces it without changing these types.

use serde::{Deserialize, Serialize};

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

/// Choose a model and effort for a task.
pub fn choose(_state: &AppState, facts: &TaskFacts) -> Decision {
    if let Some(choice) = &facts.user_override {
        return Decision {
            choice: choice.clone(),
            reason: "set by you".into(),
        };
    }
    if let Some(choice) = &facts.role_default {
        return Decision {
            choice: choice.clone(),
            reason: "the role's default".into(),
        };
    }
    Decision {
        choice: ModelChoice::new("sonnet", Some("medium")),
        reason: "the default for a task without a role".into(),
    }
}
