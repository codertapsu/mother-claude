//! Subagents, workflow agents, background shells and monitors (design §3.4).
//!
//! Owner: **B2 (data layer)** — STUB. What B2 implements: reading
//! `projects/<enc>/<sid>/subagents/agent-<id>.meta.json` + tailing running
//! agents' `agent-<id>.jsonl`, workflow journals, background task output
//! (`/private/tmp/claude-<uid>/<enc>/<sid>/tasks/<taskId>.output`, tail 16 KB,
//! on demand), teams; and publishing changes with
//! `crate::server::ws::publish_agents` / `publish_fleet`.
//!
//! AppState field: `state.agents`.

use super::model::{AgentInfo, FleetSession, WorkflowRun};

/// Agent tracking state. STUB: knows no agents.
#[derive(Debug, Default)]
pub struct AgentTracker {}

impl AgentTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// A session's agents and workflow runs (`GET /sessions/{id}/agents`,
    /// `SessionDetail.agentsDetail/workflows`). STUB.
    pub fn for_session(&self, _session_id: &str) -> (Vec<AgentInfo>, Vec<WorkflowRun>) {
        (Vec::new(), Vec::new())
    }

    /// Running agents across all sessions (`GET /agents`, `fleet` frame).
    /// STUB.
    pub fn fleet(&self) -> Vec<FleetSession> {
        Vec::new()
    }
}
