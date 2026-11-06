//! What the teams send to the dashboard, and the hook events they observe.
//!
//! * The `teams` WebSocket frame (everyone): a team's whole view when it
//!   changed (roles, states, queue reasons, board counts), one task of a
//!   board when it changed, a team that was removed, or "the templates
//!   changed: refetch".
//! * The plugin's observe-only events (docs/TEAMS.md "Hooks"):
//!   `TeammateIdle`, `TaskCreated`, `TaskCompleted` (interactive agent
//!   teams — Mother Claude shows them, it does not run them),
//!   `PreModelSwitch` / `PostModelSwitch` (a role session's model, for the
//!   team view and the router's records) and `ConfigChange` (a role session
//!   changing settings is audited). `WorktreeCreate` / `WorktreeRemove` are
//!   deliberately NOT registered: a hook on either replaces Claude Code's own
//!   git worktree handling for every session on the machine (verified,
//!   docs/TEAMS.md "Hooks"); worktrees are read from the role sessions
//!   themselves instead.

use serde::Serialize;
use serde_json::json;

use super::board::Task;
use super::teams::{Observed, TeamView};
use crate::server::hooks::HookPayload;
use crate::state::AppState;

/// `{t:"teams", team?, teamId?, task?, removed?, templates?}`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamsFrame {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<TeamView>,
    /// The team whose board `task` is on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<Task>,
    /// A team that was removed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removed: Option<String>,
    /// The templates changed: refetch them.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub templates: bool,
}

/// The hook events the teams observe.
pub const OBSERVED_EVENTS: [&str; 6] = [
    "TeammateIdle",
    "TaskCreated",
    "TaskCompleted",
    "PreModelSwitch",
    "PostModelSwitch",
    "ConfigChange",
];

fn text(v: &crate::claude::index::loose::LStr, max: usize) -> Option<String> {
    v.get()
        .map(|s| {
            s.chars()
                .filter(|c| !c.is_control())
                .take(max)
                .collect::<String>()
        })
        .filter(|s: &String| !s.trim().is_empty())
}

/// One observe-only hook event (from `server::hooks::dispatch_from`, which
/// already checked the session id and that it is not an owned session's).
pub fn on_hook(state: &AppState, p: &HookPayload) {
    let name = p.hook_event_name.as_str();
    if !OBSERVED_EVENTS.contains(&name) {
        return;
    }
    let ev = Observed {
        at: crate::claude::registry::now_ms(),
        session_id: p.session_id.clone(),
        event: name.to_string(),
        teammate: text(&p.teammate_name, 80),
        team: text(&p.team_name, 80),
        task_id: text(&p.task_id, 80),
        subject: text(&p.task_subject, 200),
        from_model: text(&p.from_model, 80),
        to_model: text(&p.to_model, 80),
        source: p.source.clone().map(|s| s.chars().take(40).collect()),
    };
    // A role session's own events matter to its team.
    if let Some(b) = state.teams.binding(&p.session_id) {
        match name {
            "PostModelSwitch" => {
                if let Some(to) = ev.to_model.clone() {
                    state.teams.update(&b.team, |t| {
                        if let Some(s) = t.roles.get_mut(&b.role) {
                            let effort = s.model.as_ref().and_then(|m| m.effort.clone());
                            s.model = Some(super::router::ModelChoice { model: to, effort });
                            s.model_reason = Some("switched in the session".into());
                        }
                    });
                }
            }
            "ConfigChange" => {
                state
                    .devices
                    .audit
                    .record(crate::server::devices::audit::AuditEntry {
                        action: "team.config_change".into(),
                        principal: Some("session".into()),
                        detail: Some(json!({
                            "session": p.session_id,
                            "team": b.team,
                            "role": b.role,
                            "source": ev.source,
                        })),
                        ..crate::server::devices::audit::AuditEntry::now()
                    });
            }
            _ => {}
        }
        state.teams.nudge();
    }
    state.teams.observe(ev);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frame_carries_only_what_changed() {
        let f = TeamsFrame {
            templates: true,
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&f).unwrap(),
            json!({ "templates": true })
        );
        let f = TeamsFrame {
            removed: Some("tm_x".into()),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&f).unwrap(),
            json!({ "removed": "tm_x" })
        );
    }

    #[test]
    fn the_plugin_registers_the_observed_events_and_never_a_worktree_hook() {
        let hooks: serde_json::Value = serde_json::from_str(include_str!(
            "../../assets/claude-plugin/mother-claude/hooks/hooks.json"
        ))
        .unwrap();
        let hooks = hooks["hooks"].as_object().unwrap();
        for event in OBSERVED_EVENTS {
            let h = &hooks[event][0]["hooks"][0];
            assert_eq!(h["type"], "command", "{event}");
            assert_eq!(
                h["command"], "${CLAUDE_PLUGIN_ROOT}/scripts/mc-hook",
                "{event}"
            );
            assert_eq!(h["args"][0], event);
            assert_eq!(h["args"][2], "observe", "{event} only observes");
            assert!(h["timeout"].as_u64().unwrap() > 0);
        }
        // A WorktreeCreate hook replaces Claude Code's git worktrees for
        // every session on the machine (and fails them when it prints no
        // path): never registered.
        assert!(!hooks.contains_key("WorktreeCreate"));
        assert!(!hooks.contains_key("WorktreeRemove"));
    }
}
