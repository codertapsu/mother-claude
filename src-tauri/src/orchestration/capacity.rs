//! Admission control (docs/PLAN-BROWSER-AND-TEAMS.md §9): whether a role
//! session may start now, and why it waits otherwise.
//!
//! **Contract** (the teams side calls [`admit`] before it launches a role
//! session, with the role sessions it runs now). The body here is the first
//! rule, a fixed cap; the scheduler replaces it (memory headroom, plan usage
//! windows, budgets, per-project and user caps, priorities) without changing
//! these types.

use serde::{Deserialize, Serialize};

use crate::state::AppState;

/// The role sessions running at once when nothing else says otherwise: the
/// user's cap's default (§9).
pub const DEFAULT_USER_CAP: usize = 3;

/// A role session the teams side wants to start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRequest {
    /// The project folder.
    pub project: String,
    /// The role id.
    pub role: String,
    /// The model it would run on (after [`super::router::choose`]).
    pub model: String,
    /// 0 (highest) … 3.
    #[serde(default)]
    pub priority: u8,
    /// The account it runs under (`None`: the project's default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// The lead (CEO) of a team: admitted ahead of workers.
    #[serde(default)]
    pub lead: bool,
}

/// A role session that runs now (the teams side knows them).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningWorker {
    pub session_id: String,
    pub project: String,
    pub role: String,
    pub model: String,
    #[serde(default)]
    pub lead: bool,
}

/// May it start?
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Admission {
    Admit,
    /// Not now: each reason in the user's words ("3 role sessions run
    /// already (your limit)", "Opus weekly usage at 91%").
    Wait {
        reasons: Vec<String>,
    },
}

impl Admission {
    pub fn is_admit(&self) -> bool {
        matches!(self, Self::Admit)
    }
}

/// Decide whether `req` may start while `running` run.
pub fn admit(_state: &AppState, req: &StartRequest, running: &[RunningWorker]) -> Admission {
    let workers = running.iter().filter(|w| !w.lead).count();
    if req.lead || workers < DEFAULT_USER_CAP {
        return Admission::Admit;
    }
    Admission::Wait {
        reasons: vec![format!(
            "{workers} role sessions run already (the limit is {DEFAULT_USER_CAP})"
        )],
    }
}
