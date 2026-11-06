//! Delegation (docs/TEAMS.md "How delegation works"; plan §6.2): the lead
//! talks to role sessions itself with Claude Code's own `SendMessage` and
//! `ListAgents` (cross-session messages: local, on by default, taken
//! unattended with `crossSessionInbound: accept`); the board is the shared
//! truth; and Mother Claude tells the sessions what changed on it.
//!
//! Every board change goes through [`on_change`]: the dashboard gets the
//! task (`teams` frame), the lead (or the session that holds the task) gets
//! a short `[Mother Claude · board] …` message — through the owned-session
//! message path (queued behind a running turn) or, for a foreign session
//! that took the role, the hooks relay — every end of a task is recorded
//! with `metrics::record_outcome`, and the phone hears about tasks done,
//! failed or blocked. A lead that is not running keeps its notices for its
//! next start (which the notice brings about).

use serde_json::json;

use super::board::{Change, ChangeKind, TaskStatus};
use super::events::TeamsFrame;
use super::metrics::{self, TaskOutcome, TaskResult};
use super::router::ModelChoice;
use super::teams::Team;
use crate::server::ws::ServerEvent;
use crate::state::AppState;

/// Notices kept for a lead that is not running.
const PENDING_NOTICES_MAX: usize = 20;
/// The longest summary a notice quotes.
const QUOTE_MAX: usize = 600;

fn now_ms() -> i64 {
    crate::claude::registry::now_ms()
}

/// Publish a team's view to every client.
pub async fn publish_team(state: &AppState, team_id: &str) {
    let Some(team) = state.teams.get(team_id) else {
        return;
    };
    let view = super::teams::view(state, &team).await;
    state.broadcast(ServerEvent::Teams(Box::new(TeamsFrame {
        team: Some(view),
        ..Default::default()
    })));
}

/// Publish one task of a team's board.
pub fn publish_task(state: &AppState, team_id: &str, task: &super::board::Task) {
    state.broadcast(ServerEvent::Teams(Box::new(TeamsFrame {
        team_id: Some(team_id.to_string()),
        task: Some(task.clone()),
        ..Default::default()
    })));
}

/// Tell every client a team is gone.
pub fn publish_removed(state: &AppState, team_id: &str) {
    state.broadcast(ServerEvent::Teams(Box::new(TeamsFrame {
        removed: Some(team_id.to_string()),
        ..Default::default()
    })));
}

/// Tell every client the templates changed.
pub fn publish_templates(state: &AppState) {
    state.broadcast(ServerEvent::Teams(Box::new(TeamsFrame {
        templates: true,
        ..Default::default()
    })));
}

/// What a relayed message may take (the hooks relay takes 8,000 bytes).
const RELAY_FIT_BYTES: usize = 7_600;

/// `text` cut to `max` bytes at a character boundary, saying so.
pub fn fit(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let note = " … (cut short: task_get has the whole task)";
    let mut end = max.saturating_sub(note.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{note}", &text[..end])
}

fn quote(s: &str) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = one.chars().take(QUOTE_MAX).collect();
    if one.chars().count() > QUOTE_MAX {
        out.push('…');
    }
    out
}

/// Send `text` into the session of a role: an owned one through the
/// owned-session message path (queued while a turn runs), a foreign one
/// through the hooks relay. A lead that is not running keeps it for its
/// next start. Whether it went somewhere.
pub async fn deliver(state: &AppState, team: &Team, role: &str, text: &str) -> bool {
    let slot = team.slot(role);
    if let Some(sid) = slot.session_id.as_deref() {
        if slot.foreign {
            if crate::server::hooks::is_active(state, sid) {
                let text = fit(text, RELAY_FIT_BYTES);
                return crate::server::hooks::enqueue_relay(state, sid, &text)
                    .await
                    .is_ok();
            }
        } else if state.control.controls(sid) {
            return crate::claude::owned::send(state, sid, text, &[])
                .await
                .is_ok();
        }
    }
    let lead = team.role(role).is_some_and(|r| r.lead);
    if lead && !team.stopped {
        state.teams.with_runtime(&team.id, role, |r| {
            r.pending_notices.push(text.to_string());
            if r.pending_notices.len() > PENDING_NOTICES_MAX {
                let excess = r.pending_notices.len() - PENDING_NOTICES_MAX;
                r.pending_notices.drain(..excess);
            }
        });
        state.teams.nudge();
        return true;
    }
    false
}

/// What the lead (and the session that holds the task) is told about a
/// change: `(to the lead, to the holder)`.
pub fn notices(change: &Change) -> (Option<String>, Option<String>) {
    let t = &change.task;
    let role = t.role.as_deref().unwrap_or("no role");
    let by_lead = matches!(
        change.actor,
        super::board::Actor::Session { lead: true, .. }
    );
    let by_user = matches!(change.actor, super::board::Actor::User(_));
    let by_worker = matches!(
        change.actor,
        super::board::Actor::Session { lead: false, .. }
    );
    let who = change.actor.label();
    let head = format!("[Mother Claude · board] Task {} ({role})", t.id);
    let summary = t
        .result
        .as_ref()
        .map(|r| quote(&r.summary))
        .unwrap_or_default();
    let mut to_lead = None;
    let mut to_holder = None;
    if change.is(ChangeKind::Created) && by_user {
        to_lead = Some(format!(
            "[Mother Claude · board] The user added task {} for {role}: {} (P{}).",
            t.id, t.title, t.priority
        ));
    }
    if change.is(ChangeKind::Review) {
        let mut extra = Vec::new();
        if let Some(r) = &t.result {
            if let Some(b) = &r.branch {
                extra.push(format!("branch {b}"));
            }
            if !r.changed_files.is_empty() {
                extra.push(format!("{} files changed", r.changed_files.len()));
            }
        }
        let extra = if extra.is_empty() {
            String::new()
        } else {
            format!(" ({})", extra.join(", "))
        };
        to_lead = Some(format!(
            "{head} is ready for review{extra}: «{summary}». Review it, then task_update \
             {{\"id\": {}, \"status\": \"done\"}} — or send it back with status \"todo\" and a \
             note.",
            t.id
        ));
    }
    if change.is(ChangeKind::Done) && by_worker {
        to_lead = Some(format!("{head} is done: «{summary}»."));
    }
    if change.is(ChangeKind::Done) && by_user && !by_lead {
        to_lead = Some(format!("{head} was accepted by {who}."));
    }
    if change.is(ChangeKind::Failed) && by_worker {
        to_lead = Some(format!("{head} failed: «{summary}»."));
    }
    if change.is(ChangeKind::Blocked) && by_worker {
        to_lead = Some(format!(
            "{head} is blocked: «{}». Answer with task_update (blocked: \"\" and a note) or \
             SendMessage to its session.",
            quote(t.blocked.as_deref().unwrap_or_default())
        ));
    }
    if (change.is(ChangeKind::Cancelled)
        || change.is(ChangeKind::Reprioritised)
        || change.is(ChangeKind::Reassigned)
        || change.is(ChangeKind::Edited))
        && by_user
        && to_lead.is_none()
    {
        let what = if change.is(ChangeKind::Cancelled) {
            "was cancelled".to_string()
        } else if change.is(ChangeKind::Reassigned) {
            format!("was given to {role}")
        } else if change.is(ChangeKind::Reprioritised) {
            format!("is now P{}", t.priority)
        } else {
            "was changed".to_string()
        };
        to_lead = Some(format!("{head} {what} by {who}."));
    }
    // The session that held it.
    if change.held_by.is_some() && !by_worker {
        if change.is(ChangeKind::Cancelled) {
            to_holder = Some(format!(
                "[Mother Claude · board] Task {} was cancelled by {who}: stop working on it.",
                t.id
            ));
        } else if change.is(ChangeKind::Reassigned) {
            to_holder = Some(format!(
                "[Mother Claude · board] Task {} was given to the {role} role: stop working on \
                 it.",
                t.id
            ));
        } else if change.is(ChangeKind::Unblocked) || change.is(ChangeKind::Note) {
            let note = t
                .progress
                .last()
                .filter(|_| change.is(ChangeKind::Note))
                .map(|n| format!(" {who} says: «{}».", quote(&n.text)))
                .unwrap_or_default();
            let unblocked = if change.is(ChangeKind::Unblocked) {
                " is no longer blocked."
            } else {
                ":"
            };
            to_holder = Some(format!(
                "[Mother Claude · board] Task {}{unblocked}{note}",
                t.id
            ));
        }
    }
    // The lead never hears about what it did itself.
    if by_lead {
        to_lead = None;
    }
    (to_lead, to_holder)
}

/// Record how a task ended (every end, a send-back and an escalation too).
pub fn record(state: &AppState, team: &Team, change: &Change) {
    let t = &change.task;
    let result = if change.ended() {
        match t.status {
            TaskStatus::Done => TaskResult::Done,
            TaskStatus::Failed => TaskResult::Failed,
            _ => TaskResult::Cancelled,
        }
    } else if change.is(ChangeKind::Escalated) {
        TaskResult::Escalated
    } else if change.is(ChangeKind::Reopened) && change.before == TaskStatus::Review {
        // Sent back from review: a failed gate for the ladder.
        TaskResult::Failed
    } else {
        return;
    };
    let role = t.role.as_deref().and_then(|r| team.role(r));
    let choice = t
        .model
        .clone()
        .or_else(|| {
            role.and_then(|r| {
                r.model
                    .as_deref()
                    .map(|m| ModelChoice::new(m, r.effort.as_deref()))
            })
        })
        .unwrap_or_else(|| ModelChoice::new("unknown", None));
    let class = t
        .class
        .as_deref()
        .and_then(|c| serde_json::from_value(json!(c)).ok());
    let gate_passed = match result {
        TaskResult::Failed if change.before == TaskStatus::Review => Some(false),
        _ => t.result.as_ref().and_then(|r| r.gate_passed),
    };
    metrics::record_outcome(
        state,
        &TaskOutcome {
            task_id: outcome_id(&team.project, t.id),
            project: team.project.clone(),
            role: t.role.clone(),
            class,
            choice,
            session_id: change.held_by.clone().or_else(|| t.claimed_by.clone()),
            started_at: t.started_at.unwrap_or(t.created_at),
            ended_at: t.ended_at.unwrap_or_else(now_ms),
            result,
            gate_passed,
            escalations: t.escalations,
        },
    );
}

/// A task's id across every project, as the scheduler's records name it:
/// the board's key and the task's number (`3f2a…#12`).
pub fn outcome_id(project: &str, task: u64) -> String {
    format!("{}#{task}", super::board::board_key(project))
}

/// Everything a board change sets off. Call after every mutation.
pub async fn on_change(state: &AppState, team_id: &str, change: Change) {
    let Some(team) = state.teams.get(team_id) else {
        return;
    };
    publish_task(state, team_id, &change.task);
    let (to_lead, to_holder) = notices(&change);
    if let (Some(text), Some(lead)) = (to_lead, team.lead()) {
        deliver(state, &team, &lead.id, &text).await;
    }
    if let (Some(text), Some(holder)) = (to_holder, change.held_by.as_deref()) {
        if let Some(role) = team.role_of(holder) {
            let role = role.to_string();
            deliver(state, &team, &role, &text).await;
        }
    }
    record(state, &team, &change);
    let t = &change.task;
    let role = t.role.as_deref().unwrap_or_default();
    use crate::server::push::{on_team_event, PushKind};
    let kind = if change.is(ChangeKind::Review) {
        Some(PushKind::TaskReview)
    } else if change.is(ChangeKind::Done) {
        Some(PushKind::TaskDone)
    } else if change.is(ChangeKind::Failed) {
        Some(PushKind::TaskFailed)
    } else if change.is(ChangeKind::Blocked) {
        Some(PushKind::TaskBlocked)
    } else {
        None
    };
    if let Some(kind) = kind {
        on_team_event(state, team_id, kind, role, &t.title);
    }
    state.teams.nudge();
}

/// A role has waited for capacity long: tell the phone.
pub fn push_waiting(state: &AppState, team: &Team, role: &str, reasons: &[String]) {
    crate::server::push::on_team_event(
        state,
        &team.id,
        crate::server::push::PushKind::RoleWaiting,
        role,
        &reasons.join("; "),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestration::board::{Actor, Board, Outcome, TaskDraft, TaskPatch, TaskReport};
    use crate::orchestration::teams::{label_of, Starter};
    use std::collections::BTreeMap;

    fn team() -> Team {
        let template = super::super::templates::builtins().remove(0);
        Team {
            id: "tm_0123456789ab".into(),
            project: "/w/app".into(),
            label: label_of("/w/app"),
            template_id: template.id.clone(),
            template,
            account: "default".into(),
            overrides: BTreeMap::new(),
            created_at: 1,
            stopped: false,
            starter: Starter::default(),
            roles: BTreeMap::new(),
            brief: None,
            extra: BTreeMap::new(),
        }
    }

    fn roles() -> Vec<String> {
        team().role_ids()
    }

    fn lead() -> Actor {
        Actor::Session {
            session: "s-lead".into(),
            role: "lead".into(),
            lead: true,
        }
    }

    fn worker() -> Actor {
        Actor::Session {
            session: "s-fe".into(),
            role: "frontend".into(),
            lead: false,
        }
    }

    #[test]
    fn the_lead_hears_about_its_workers_and_the_user_never_about_itself() {
        let mut b = Board::new("/w/app");
        let c = b
            .create(
                TaskDraft {
                    title: "Login".into(),
                    role: Some("frontend".into()),
                    ..Default::default()
                },
                &lead(),
                &roles(),
                1,
            )
            .unwrap();
        assert_eq!(notices(&c), (None, None), "the lead wrote it");
        let c = b
            .create(
                TaskDraft {
                    title: "Footer".into(),
                    role: Some("frontend".into()),
                    priority: Some(1),
                    ..Default::default()
                },
                &Actor::User("phone".into()),
                &roles(),
                1,
            )
            .unwrap();
        let (lead_note, _) = notices(&c);
        assert_eq!(
            lead_note.as_deref(),
            Some("[Mother Claude · board] The user added task 2 for frontend: Footer (P1).")
        );
        b.assign(1, "s-fe", 2).unwrap();
        let c = b
            .update(
                1,
                TaskPatch {
                    blocked: Some("Which API?".into()),
                    ..Default::default()
                },
                &worker(),
                &roles(),
                3,
            )
            .unwrap();
        let (lead_note, holder) = notices(&c);
        assert!(lead_note.unwrap().contains("is blocked: «Which API?»"));
        assert!(holder.is_none());
        // The lead answers: the worker hears, the lead does not.
        let c = b
            .update(
                1,
                TaskPatch {
                    blocked: Some(String::new()),
                    note: Some("Use /api/login".into()),
                    ..Default::default()
                },
                &lead(),
                &roles(),
                4,
            )
            .unwrap();
        let (lead_note, holder) = notices(&c);
        assert!(lead_note.is_none());
        let holder = holder.unwrap();
        assert!(holder.contains("Task 1 is no longer blocked."), "{holder}");
        assert!(holder.contains("Use /api/login"), "{holder}");
        let c = b
            .finish(
                1,
                TaskReport {
                    summary: "Form done".into(),
                    branch: Some("worktree-frontend".into()),
                    changed_files: vec!["a".into(), "b".into()],
                    gate_passed: None,
                },
                Outcome::Done,
                true,
                &worker(),
                5,
            )
            .unwrap();
        let (lead_note, _) = notices(&c);
        let lead_note = lead_note.unwrap();
        assert!(lead_note.starts_with(
            "[Mother Claude · board] Task 1 (frontend) is ready for review (branch \
             worktree-frontend, 2 files changed): «Form done»."
        ));
        // A cancel by the user reaches the holder and the lead.
        b.assign(2, "s-fe", 6).unwrap();
        let c = b
            .update(
                2,
                TaskPatch {
                    status: Some(TaskStatus::Cancelled),
                    ..Default::default()
                },
                &Actor::User("phone".into()),
                &roles(),
                7,
            )
            .unwrap();
        let (lead_note, holder) = notices(&c);
        assert!(lead_note
            .unwrap()
            .ends_with("was cancelled by user (phone)."));
        assert!(holder.unwrap().contains("stop working on it"));
    }

    #[test]
    fn outcome_ids_are_stable_per_project_and_task() {
        assert_eq!(outcome_id("/w/app", 12), outcome_id("/w/app", 12));
        assert_ne!(outcome_id("/w/app", 12), outcome_id("/w/other", 12));
        assert!(outcome_id("/w/app", 12).ends_with("#12"));
    }
}
