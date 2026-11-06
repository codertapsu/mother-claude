//! What tasks cost and how they ended (docs/PLAN-BROWSER-AND-TEAMS.md §7.2):
//! the records the router's policy is tuned on.
//!
//! **Contract** (the teams side calls [`record_outcome`] whenever a task ends
//! in any way). The body here only logs; the scheduler side stores the
//! records, joins them with each session's token usage from the transcript
//! index, and reports cost per completed task by class, model and effort.

use serde::{Deserialize, Serialize};

use super::router::{ModelChoice, TaskClass};
use crate::state::AppState;

/// How a task ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskResult {
    Done,
    Failed,
    /// Handed to a more capable model or effort.
    Escalated,
    Cancelled,
}

/// One task's end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskOutcome {
    pub task_id: String,
    pub project: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<TaskClass>,
    pub choice: ModelChoice,
    /// The session that worked on it (its token usage is in the index).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Epoch milliseconds.
    pub started_at: i64,
    pub ended_at: i64,
    pub result: TaskResult,
    /// Whether the integration gate (tests) passed, when one ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_passed: Option<bool>,
    #[serde(default)]
    pub escalations: u8,
}

/// Record how a task ended.
pub fn record_outcome(_state: &AppState, outcome: &TaskOutcome) {
    tracing::info!(
        task = %outcome.task_id,
        result = ?outcome.result,
        model = %outcome.choice.model,
        "task ended"
    );
}
