//! Build the v2 [`SessionSummary`] list (design §4.1).
//!
//! Owner: **B2 (data layer)**. The foundation maps the EXISTING registry
//! ([`crate::claude::registry::Session`]) onto the contract with a minimal,
//! honest mapping so the app keeps working; B2 replaces the registry with the
//! live roster + transcript index + agents and rewrites this file.
//!
//! What stays true whatever B2 does:
//! * answerable pending items come from `state.pending` (the store is the one
//!   source of truth); transcript-derived prompts of running foreign sessions
//!   are added as `source: observed`, `answerable: false`;
//! * `control` comes from `claude::caps::compute`;
//! * the list is published with `crate::server::ws::publish_sessions`, which
//!   diffs and broadcasts.

use std::collections::HashSet;

use super::caps::{self, CapsInput, SessionFacts};
use super::model::{
    Activity, ActivityKind, AgentCounts, BackgroundTask, BackgroundTaskKind, ModelInfo,
    PendingItem, PendingKind, PendingSource, SessionDetail, SessionStatus, SessionSummary, Surface,
    Usage,
};
use super::registry::{PendingKind as LegacyKind, Session, SessionState, Surface as LegacySurface};
use crate::state::AppState;

/// Facts about a session that live outside the legacy registry row.
#[derive(Debug, Clone, Default)]
pub struct Extras {
    pub pending: Vec<PendingItem>,
    pub bridge_thread: bool,
    pub owned_handle: bool,
    /// From `caps::input_for`; `None` computes a minimal input from the row.
    pub caps: Option<CapsInput>,
}

/// The data-layer facts of a legacy registry row.
pub fn facts_of(s: &Session, owned_handle: bool, bridge_thread: bool) -> SessionFacts {
    SessionFacts {
        id: s.id.clone(),
        owned: s.owned || owned_handle || bridge_thread,
        bridge_thread,
        live: s.running || owned_handle || bridge_thread,
        kind: s.kind.clone(),
        job_id: None,
        tmux: None,
    }
}

fn surface_of(s: &Session, owned: bool) -> Surface {
    if owned {
        return Surface::MotherClaude;
    }
    if s.kind.as_deref() == Some("background") {
        return Surface::Background;
    }
    match s.surface {
        LegacySurface::Cli => Surface::Terminal,
        LegacySurface::VsCode => Surface::Vscode,
        LegacySurface::JetBrains => Surface::Jetbrains,
        LegacySurface::Desktop => Surface::Desktop,
        LegacySurface::Unknown => Surface::Unknown,
    }
}

fn status_of(s: &Session) -> SessionStatus {
    match s.state {
        SessionState::Working => SessionStatus::Working,
        SessionState::NeedsInput => SessionStatus::Waiting,
        SessionState::Idle => SessionStatus::Idle,
        SessionState::Completed => SessionStatus::Ended,
        SessionState::Failed => SessionStatus::Failed,
        SessionState::Stopped => SessionStatus::Stopped,
        SessionState::Unknown if s.running => SessionStatus::Idle,
        SessionState::Unknown => SessionStatus::Ended,
    }
}

/// The activity implied by a status (B2 replaces this with §3.5).
pub fn activity_for(status: SessionStatus, pending: &[PendingItem]) -> Activity {
    match status {
        SessionStatus::Working => Activity::new(ActivityKind::Thinking, "Working"),
        SessionStatus::Waiting => {
            let label = match pending.iter().find(|p| p.answerable).map(|p| p.kind) {
                Some(PendingKind::Question) => "Waiting for your answer",
                Some(PendingKind::Plan) => "Waiting for plan approval",
                Some(PendingKind::Permission) => "Waiting for approval",
                _ => "Waiting for you",
            };
            let mut a = Activity::new(ActivityKind::Waiting, label);
            a.tool = pending.first().and_then(|p| p.tool_name.clone());
            a
        }
        SessionStatus::Idle => Activity::new(ActivityKind::Idle, "Idle"),
        SessionStatus::Starting => Activity::new(ActivityKind::Starting, "Starting"),
        SessionStatus::Ended | SessionStatus::Stopped => {
            Activity::new(ActivityKind::Ended, "Ended")
        }
        SessionStatus::Failed => Activity::new(ActivityKind::Ended, "Failed"),
    }
}

/// A transcript-derived prompt of a running foreign session, as an observed
/// (unanswerable) pending item.
pub fn observed_pending(s: &Session) -> Option<PendingItem> {
    let p = s.pending.as_ref()?;
    if p.answerable {
        return None; // live items come from the store, not the registry
    }
    let is_plan = p.tool.as_deref() == Some("ExitPlanMode");
    let kind = if is_plan {
        PendingKind::Plan
    } else if p.kind == LegacyKind::Question {
        PendingKind::Question
    } else {
        PendingKind::Permission
    };
    let questions = (kind == PendingKind::Question).then(|| {
        vec![super::model::Question {
            question: p.prompt.clone().unwrap_or_default(),
            header: p.header.clone(),
            options: p
                .options
                .iter()
                .map(|o| super::model::QuestionOption {
                    label: o.label.clone(),
                    description: o.description.clone(),
                })
                .collect(),
            multi_select: p.multi_select,
        }]
    });
    Some(PendingItem {
        id: format!("observed:{}", p.request_id.clone().unwrap_or_default()),
        session_id: s.id.clone(),
        kind,
        source: PendingSource::Observed,
        tool_name: p.tool.clone(),
        tool_use_id: p.request_id.clone(),
        title: p
            .header
            .clone()
            .or_else(|| p.prompt.clone())
            .unwrap_or_else(|| "Waiting in the session".into()),
        detail: if is_plan { None } else { p.detail.clone() },
        plan: if is_plan { p.detail.clone() } else { None },
        questions,
        answerable: false,
        created_at: s.last_activity.unwrap_or_default(),
        ..Default::default()
    })
}

fn agent_counts(s: &Session) -> AgentCounts {
    let mut c = AgentCounts::default();
    for t in &s.tasks {
        let running = t.status == "running";
        match t.kind.as_str() {
            "agent" => {
                c.total += 1;
                c.running += running as u32;
            }
            "workflow" => c.workflows_running += running as u32,
            "bash" => c.background_running += running as u32,
            _ => {}
        }
    }
    c
}

/// Map one legacy registry row onto the contract.
pub fn from_session(s: &Session, extras: Extras) -> SessionSummary {
    let owned = s.owned || extras.owned_handle || extras.bridge_thread;
    let mut pending = extras.pending;
    if !owned && s.running {
        if let Some(observed) = observed_pending(s) {
            pending.push(observed);
        }
    }
    let mut status = status_of(s);
    if pending.iter().any(|p| p.answerable) {
        status = SessionStatus::Waiting;
    } else if status == SessionStatus::Waiting && pending.is_empty() {
        status = if s.running {
            SessionStatus::Working
        } else {
            SessionStatus::Ended
        };
    }
    let caps = caps::compute(&extras.caps.clone().unwrap_or_else(|| CapsInput {
        owned: owned && !extras.bridge_thread,
        bridge_thread: extras.bridge_thread,
        live: s.running || extras.owned_handle || extras.bridge_thread,
        background_job:
            (s.kind.as_deref() == Some("background") && s.running).then(|| s.id.clone()),
        ..Default::default()
    }));
    let model = s
        .model
        .as_ref()
        .filter(|m| !m.starts_with('<'))
        .map(|id| ModelInfo {
            id: id.clone(),
            label: super::pricing::model_label(id),
            context_window: Some(super::pricing::context_window(id)),
        });
    SessionSummary {
        id: s.id.clone(),
        name: s.title.clone().unwrap_or_else(|| s.project_name.clone()),
        title: s.title.clone(),
        project_name: s.project_name.clone(),
        cwd: s.cwd.clone(),
        surface: surface_of(s, owned),
        kind: s.kind.clone(),
        owned,
        live: s.running || extras.owned_handle || extras.bridge_thread,
        status,
        activity: activity_for(status, &pending),
        model,
        git_branch: s.git_branch.clone(),
        started_at: s.started_at,
        last_activity_at: s.last_activity,
        pid: s.pid,
        job_id: (s.kind.as_deref() == Some("background")).then(|| s.id.clone()),
        usage: Usage {
            input: s.usage.input_tokens,
            output: s.usage.output_tokens,
            cache_read: s.usage.cache_read_tokens,
            cache_write: s.usage.cache_creation_tokens,
            thinking: None,
        },
        message_count: s.message_count,
        agents: agent_counts(s),
        pending,
        control: caps,
        ..Default::default()
    }
}

/// Build every summary from the legacy registry rows, pulling pending items,
/// ownership and hook activity from the app state.
pub fn build(
    state: &AppState,
    sessions: &[Session],
    bridge_threads: &HashSet<String>,
) -> Vec<SessionSummary> {
    sessions
        .iter()
        .map(|s| {
            let bridge_thread = bridge_threads.contains(&s.id);
            let owned_handle = state.control.controls(&s.id);
            let facts = facts_of(s, owned_handle, bridge_thread);
            from_session(
                s,
                Extras {
                    pending: state.pending.list(&s.id),
                    bridge_thread,
                    owned_handle,
                    caps: Some(caps::input_for(state, &facts)),
                },
            )
        })
        .collect()
}

/// A just-spawned owned session that no data source knows yet.
pub fn synthetic_owned(id: &str, cwd: &str, started_at: i64) -> SessionSummary {
    let project_name = std::path::Path::new(cwd)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(cwd)
        .to_string();
    SessionSummary {
        id: id.to_string(),
        name: project_name.clone(),
        project_name,
        cwd: cwd.to_string(),
        surface: Surface::MotherClaude,
        owned: true,
        live: true,
        status: SessionStatus::Starting,
        activity: Activity::new(ActivityKind::Starting, "Starting"),
        started_at: Some(started_at),
        last_activity_at: Some(started_at),
        control: caps::compute(&CapsInput {
            owned: true,
            live: true,
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// `GET /api/sessions/{id}` detail. Foundation: background tasks from the
/// legacy registry row; agents/workflows from `state.agents`; queued relay
/// messages from the hook dispatcher.
pub async fn detail(state: &AppState, summary: SessionSummary) -> SessionDetail {
    let legacy = state
        .registry
        .read()
        .await
        .iter()
        .find(|s| s.id == summary.id)
        .cloned();
    let tasks = legacy
        .map(|s| {
            s.tasks
                .iter()
                .map(|t| BackgroundTask {
                    id: t.id.clone(),
                    kind: match t.kind.as_str() {
                        "agent" => BackgroundTaskKind::Agent,
                        "workflow" => BackgroundTaskKind::Workflow,
                        "monitor" => BackgroundTaskKind::Monitor,
                        _ => BackgroundTaskKind::Bash,
                    },
                    label: t.label.clone(),
                    status: t.status.clone(),
                    started_at: t.started_at,
                    ended_at: t.ended_at,
                    output_available: false,
                })
                .collect()
        })
        .unwrap_or_default();
    let (agents_detail, workflows) = state.agents.for_session(&summary.id);
    let queued_messages = crate::server::hooks::queued(state, &summary.id);
    SessionDetail {
        summary,
        agents_detail,
        workflows,
        tasks,
        queued_messages,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::registry::{PendingInput, UsageSummary};

    fn session() -> Session {
        Session {
            id: "s1".into(),
            cwd: "/p/proj".into(),
            project_name: "proj".into(),
            surface: LegacySurface::VsCode,
            owned: false,
            state: SessionState::Working,
            model: Some("claude-opus-4-8".into()),
            title: Some("Fix it".into()),
            started_at: Some(1),
            last_activity: Some(2),
            pid: Some(42),
            kind: Some("interactive".into()),
            git_branch: Some("main".into()),
            running: true,
            message_count: 7,
            usage: UsageSummary {
                input_tokens: 10,
                output_tokens: 20,
                cache_read_tokens: 30,
                cache_creation_tokens: 40,
                total_tokens: 100,
            },
            pending: None,
            tasks: Vec::new(),
            can_inject: false,
        }
    }

    #[test]
    fn maps_the_basics() {
        let s = from_session(&session(), Extras::default());
        assert_eq!(s.name, "Fix it");
        assert_eq!(s.surface, Surface::Vscode);
        assert_eq!(s.status, SessionStatus::Working);
        assert!(s.live && !s.owned);
        assert_eq!(s.model.as_ref().unwrap().label, "Opus 4.8");
        assert_eq!(s.usage.cache_write, 40);
        assert_eq!(s.message_count, 7);
        assert_eq!(
            s.control.channel,
            crate::claude::model::ControlChannel::None
        );
    }

    #[test]
    fn store_items_make_it_waiting_and_observed_items_are_unanswerable() {
        let mut legacy = session();
        legacy.pending = Some(PendingInput {
            kind: LegacyKind::Question,
            tool: Some("AskUserQuestion".into()),
            prompt: Some("Which?".into()),
            header: None,
            options: Vec::new(),
            multi_select: false,
            detail: None,
            request_id: Some("toolu_1".into()),
            answerable: false,
            dangerous: false,
        });
        let s = from_session(&legacy, Extras::default());
        assert_eq!(s.pending.len(), 1);
        assert_eq!(s.pending[0].source, PendingSource::Observed);
        assert!(!s.pending[0].answerable);
        assert_eq!(s.pending[0].kind, PendingKind::Question);

        let live_item = PendingItem {
            id: "r1".into(),
            session_id: "s1".into(),
            answerable: true,
            ..Default::default()
        };
        let s = from_session(
            &session(),
            Extras {
                pending: vec![live_item],
                ..Default::default()
            },
        );
        assert_eq!(s.status, SessionStatus::Waiting);
        assert_eq!(s.activity.kind, ActivityKind::Waiting);
    }

    #[test]
    fn owned_and_synthetic() {
        let s = from_session(
            &session(),
            Extras {
                owned_handle: true,
                ..Default::default()
            },
        );
        assert!(s.owned);
        assert_eq!(s.surface, Surface::MotherClaude);
        assert_eq!(
            s.control.channel,
            crate::claude::model::ControlChannel::Owned
        );
        let syn = synthetic_owned("n1", "/x/y", 5);
        assert_eq!(syn.project_name, "y");
        assert_eq!(syn.status, SessionStatus::Starting);
        assert!(syn.owned && syn.live);
    }
}
