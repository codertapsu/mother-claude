//! Starting a moved session again — on the target once it lives there, or on
//! the source when a move did not go ahead or was undone — as an owned
//! session with the settings it ran with: always an explicit mode, and one
//! the starting person may launch. Until the app turns resumes on
//! ([`super::Moves::enable_resumes`]) they are only recorded, as the setup's
//! fixes are: tests never start `claude`.

use serde_json::json;

use super::Carried;
use crate::claude::control::SpawnOptions;
use crate::claude::owned::{self, Caller};
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// What to start.
pub(super) struct Spec<'a> {
    /// The session to resume.
    pub session: &'a str,
    /// A copy: forked under this new id (`--fork-session`).
    pub fork_as: Option<&'a str>,
    pub cwd: &'a str,
    pub account: &'a str,
    pub settings: &'a Carried,
    /// The person behind it may launch a mode without approvals.
    pub may_dangerous: bool,
    /// Messages to hand it once it runs.
    pub held: &'a [String],
}

/// Starts it; returns the id it runs under.
pub(super) async fn start(state: &AppState, spec: &Spec<'_>) -> ApiResult<String> {
    let id = spec.fork_as.unwrap_or(spec.session).to_string();
    let caller = Caller {
        may_dangerous: spec.may_dangerous,
        // The folder was checked by the move (its trust there, or carried).
        desktop: true,
    };
    let mode = owned::launch_mode(
        Some(
            spec.settings
                .permission_mode
                .as_deref()
                .unwrap_or("default"),
        ),
        caller,
    )?;
    let account_env = state
        .accounts
        .env(spec.account)
        .map_err(owned::account_error)?;
    if !state.machines.moves.resumes_live() {
        // The launch is checked as it would be (the lease, a move under
        // way), then only recorded.
        crate::machines::lease::require_here(state, &id)?;
        state.machines.moves.record_resume(json!({
            "session": spec.session,
            "id": id,
            "fork": spec.fork_as.is_some(),
            "cwd": spec.cwd,
            "account": spec.account,
            "model": spec.settings.model,
            "effort": spec.settings.effort,
            "thinking": spec.settings.thinking,
            "permissionMode": mode,
            "ultracode": spec.settings.ultracode,
            "held": spec.held.len(),
            "note": state.machines.leases.note_of(&id).is_some(),
        }));
        return Ok(id);
    }
    let opts = SpawnOptions {
        cwd: spec.cwd.to_string(),
        model: spec.settings.model.clone(),
        effort: spec.settings.effort.clone(),
        thinking: spec.settings.thinking.clone(),
        permission_mode: Some(mode),
        resume: Some(spec.session.to_string()),
        fork: spec.fork_as.is_some(),
        session_id: spec.fork_as.map(str::to_string),
        allow_bypass: spec.settings.bypass_available && spec.may_dangerous,
        caller_may_dangerous: spec.may_dangerous,
        account: Some(spec.account.to_string()),
        account_env,
        quiet: true,
        ..Default::default()
    };
    let id = state
        .control
        .spawn(state, opts)
        .await
        .map_err(owned::spawn_error)?;
    if spec.settings.ultracode {
        if let Err(e) = owned::set_ultracode(state, &id, true).await {
            tracing::debug!(session = %id, error = %e.message, "ultracode after a move");
        }
    }
    for text in spec.held {
        if let Err(e) = owned::send_as(state, spec.may_dangerous, &id, text, &[]).await {
            crate::server::ws::publish_notice(
                state,
                crate::claude::model::NoticeLevel::Warning,
                format!(
                    "A message that was queued could not be handed to the moved session: {}",
                    e.message
                ),
                Some(&id),
            );
        }
    }
    Ok(id)
}

/// The settings an owned session runs with now (what a switch of account
/// carries too).
pub(super) fn carried(live: &crate::claude::control::OwnedLive) -> Carried {
    Carried {
        model: crate::claude::switch::carried_model(live),
        effort: crate::claude::switch::carried_effort(live),
        thinking: live.thinking.map(|t| {
            match t {
                crate::claude::control::OwnedThinking::Off => "off",
                crate::claude::control::OwnedThinking::On(_) => "on",
            }
            .to_string()
        }),
        permission_mode: Some(
            live.permission_mode
                .clone()
                .unwrap_or_else(|| "default".to_string()),
        ),
        ultracode: live.ultracode_wanted(),
        bypass_available: live.bypass_available,
    }
}

/// The settings a session nobody runs ran with, as its row shows them.
pub(super) fn carried_of(s: &crate::claude::model::SessionSummary) -> Carried {
    Carried {
        model: s
            .model
            .as_ref()
            .map(|m| m.id.clone())
            .filter(|m| !crate::claude::pricing::is_synthetic(m))
            .and_then(|m| owned::validate_model(&m).ok()),
        effort: s
            .effort
            .clone()
            .filter(|e| owned::EFFORT_LEVELS.contains(&e.as_str())),
        thinking: None,
        permission_mode: s
            .permission_mode
            .clone()
            .filter(|m| owned::PERMISSION_MODES.contains(&m.as_str())),
        ultracode: false,
        bypass_available: false,
    }
}

/// The error of a resume, in a record's words.
pub(super) fn words(e: &ApiError) -> String {
    e.message.clone()
}
