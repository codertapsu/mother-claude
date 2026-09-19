//! Starting, stopping and configuring the bridge from the dashboard.
//!
//! The bridge does not listen until you ask it to. That is deliberate: starting
//! it publishes an unauthenticated port that can drive Claude with your account,
//! and the moment that happens should be a thing you did, not a side effect of
//! opening the app.
//!
//! What you choose at start time becomes the *defaults* for every conversation
//! the bridge creates — model, effort, working directory, permission mode — and
//! every one of them can still be overridden per request. You can also nominate
//! a conversation for new messages to join, so a client that only ever sends
//! `{"message": "…"}` keeps one continuous thread instead of a new one each time.
//!
//! These handlers live on the **dashboard** API (`/api/bridge/…`), not on the
//! bridge itself — you have to be able to start something that is not running.

use std::net::SocketAddr;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::oneshot;

use super::error::{BridgeError, BridgeResult};
use super::routes::threads::{EFFORT_LEVELS, PERMISSION_MODES, SETTING_SOURCES};
use crate::state::{AppState, ServerEvent};

/// Defaults applied to every conversation the bridge creates.
///
/// Every field is optional and every one is overridable per request; `None`
/// means "whatever the user's own Claude settings say", which is the same rule
/// the dashboard's launch form follows.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BridgeDefaults {
    /// Working directory for new conversations.
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// "on" | "off".
    pub thinking: Option<String>,
    pub permission_mode: Option<String>,
    /// Which of `user` / `project` / `local` settings to load. `[]` isolates.
    pub setting_sources: Option<Vec<String>>,
    pub system_prompt_append: Option<String>,
    /// The conversation new messages join when the caller does not name one.
    ///
    /// `None` means the first message creates a conversation and everything
    /// after it joins that one, which is what a client sending only
    /// `{"message": "…"}` almost always wants.
    pub default_thread: Option<String>,
}

fn one_of(field: &str, value: &Option<String>, allowed: &[&str]) -> BridgeResult<()> {
    if let Some(v) = value {
        if !allowed.contains(&v.as_str()) {
            return Err(BridgeError::invalid_field(
                field,
                format!(
                    "Unknown {field} {v:?}; expected one of {}.",
                    allowed.join(", ")
                ),
            ));
        }
    }
    Ok(())
}

impl BridgeDefaults {
    /// Reject anything the runtime would reject later, while the person is
    /// still looking at the form.
    pub fn validate(&self) -> BridgeResult<()> {
        one_of("effort", &self.effort, &EFFORT_LEVELS)?;
        one_of("permissionMode", &self.permission_mode, &PERMISSION_MODES)?;
        one_of("thinking", &self.thinking, &["on", "off"])?;
        if let Some(sources) = &self.setting_sources {
            for source in sources {
                if !SETTING_SOURCES.contains(&source.as_str()) {
                    return Err(BridgeError::invalid_field(
                        "settingSources",
                        format!(
                            "Unknown setting source {source:?}; expected one of {}.",
                            SETTING_SOURCES.join(", ")
                        ),
                    ));
                }
            }
        }
        if let Some(cwd) = &self.cwd {
            if !cwd.trim().is_empty() && !std::path::Path::new(cwd).is_dir() {
                return Err(BridgeError::invalid_field(
                    "cwd",
                    format!("{cwd} is not a directory on this machine."),
                ));
            }
        }
        Ok(())
    }

    /// Blank strings from a web form mean "unset", not "empty".
    pub fn normalized(mut self) -> Self {
        let blank = |v: &mut Option<String>| {
            if v.as_deref().map(str::trim).is_some_and(str::is_empty) {
                *v = None;
            }
        };
        blank(&mut self.cwd);
        blank(&mut self.model);
        blank(&mut self.effort);
        blank(&mut self.thinking);
        blank(&mut self.permission_mode);
        blank(&mut self.system_prompt_append);
        blank(&mut self.default_thread);
        self
    }
}

/// A bound, serving bridge listener.
pub struct Listener {
    pub port: u16,
    pub started_at: i64,
    /// Dropped or fired to unbind the port.
    shutdown: Option<oneshot::Sender<()>>,
}

impl std::fmt::Debug for Listener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listener")
            .field("port", &self.port)
            .field("started_at", &self.started_at)
            .finish()
    }
}

impl Listener {
    fn stop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

/// Start the bridge listener with `defaults`, replacing any previous run.
pub async fn start(state: &AppState, defaults: BridgeDefaults) -> BridgeResult<Value> {
    let config = state.bridge.config.clone();
    if !config.enabled {
        return Err(BridgeError::forbidden(
            "bridge_disabled",
            "The bridge is disabled for this process (MOTHER_CLAUDE_BRIDGE=0).",
        ));
    }
    let defaults = defaults.normalized();
    defaults.validate()?;

    // A nominated conversation must actually exist, or the first message would
    // fail in a way that looks like the bridge is broken rather than like a
    // stale pick.
    if let Some(thread) = &defaults.default_thread {
        if !super::thread_exists(state, thread).await {
            return Err(BridgeError::invalid_field(
                "defaultThread",
                format!("No conversation {thread} — pick another, or leave it unset."),
            )
            .with("thread_id", thread.clone()));
        }
    }

    let mut guard = state.bridge.listener.lock().await;
    if let Some(listener) = guard.as_mut() {
        // Restarting in place: keep the port, take the new defaults.
        listener.stop();
        *guard = None;
    }

    *state.bridge.defaults.write().await = defaults.clone();
    *state.bridge.active_thread.write().await = defaults.default_thread.clone();

    let Some(port) = config.port else {
        // No dedicated port configured: the `/v1` mount is the whole surface,
        // and "started" is still meaningful because it ungates those routes.
        *guard = Some(Listener {
            port: 0,
            started_at: crate::claude::registry::now_ms(),
            shutdown: None,
        });
        drop(guard);
        announce(state, "started").await;
        return Ok(status(state).await);
    };

    // Bind before reporting success, so "port already in use" is an error the
    // person sees on the button they pressed.
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let socket = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
        BridgeError::conflict(
            "port_unavailable",
            format!("Could not bind {addr}: {e}. Another process may already hold it."),
        )
        .with("port", port)
    })?;

    let app = super::listener_router(state.clone(), config.require_token);
    let (tx, rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let served = axum::serve(
            socket,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let _ = rx.await;
        })
        .await;
        if let Err(e) = served {
            tracing::error!(error = %e, "Claude bridge listener stopped");
        } else {
            tracing::info!("Claude bridge listener closed");
        }
    });

    *guard = Some(Listener {
        port,
        started_at: crate::claude::registry::now_ms(),
        shutdown: Some(tx),
    });
    drop(guard);

    announce(state, "started").await;
    Ok(status(state).await)
}

/// Stop the listener. Conversations already created stay alive — stopping the
/// door does not evict the people inside it.
pub async fn stop(state: &AppState) -> BridgeResult<Value> {
    let mut guard = state.bridge.listener.lock().await;
    if let Some(listener) = guard.as_mut() {
        listener.stop();
    }
    *guard = None;
    drop(guard);
    announce(state, "stopped").await;
    Ok(status(state).await)
}

async fn announce(state: &AppState, what: &str) {
    let running = state.bridge.is_running().await;
    tracing::info!(running, "Claude bridge {what}");
    state.broadcast(ServerEvent::Notice(format!("Claude bridge {what}")));
}

/// Everything the control screen needs in one payload.
pub async fn status(state: &AppState) -> Value {
    let config = &state.bridge.config;
    let listener = state.bridge.listener.lock().await;
    let running = listener.is_some();
    let port = listener.as_ref().map(|l| l.port).or(config.port);
    let started_at = listener.as_ref().map(|l| l.started_at);
    drop(listener);

    let url = port
        .filter(|p| *p != 0)
        .map(|p| format!("http://127.0.0.1:{p}"));
    // Projected to the dashboard's camelCase convention. `ThreadRecord` is
    // snake_case because the bridge's own API mirrors the Codex one; this
    // payload belongs to `/api/*`, which is camelCase throughout.
    let threads: Vec<Value> = state
        .bridge
        .threads()
        .await
        .into_iter()
        .map(|t| {
            json!({
                "threadId": t.thread_id,
                "cwd": t.cwd,
                "createdAt": t.created_at,
                "model": t.model,
                "effort": t.effort,
                "permissionMode": t.permission_mode,
                "title": t.title,
                "persisted": t.persisted,
                "activeTurn": t.active_turn,
            })
        })
        .collect();
    let active = state.bridge.active_thread.read().await.clone();
    let auth = state.claude_auth.read().await.clone();

    json!({
        "enabled": config.enabled,
        "running": running,
        "port": port,
        "url": url,
        "startedAt": started_at,
        "requireToken": config.require_token,
        "hostBuilt": super::host::host_entry().is_some(),
        "hostRunning": state.bridge.host.is_running().await,
        "messagesApi": state.bridge.messages.is_configured(),
        "claudeAuthenticated": auth.logged_in,
        "claudeAccount": auth.summary(),
        "activeOperations": state.bridge.ops.active_count(),
        "defaults": state.bridge.defaults.read().await.clone(),
        "activeThread": active,
        "threads": threads,
        "docsUrl": url.as_ref().map(|u| format!("{u}/docs/")),
        "consoleUrl": url.as_ref().map(|u| format!("{u}/example/")),
    })
}

// --- dashboard handlers ----------------------------------------------------

/// `GET /api/bridge`
pub async fn get_status(State(state): State<AppState>) -> Response {
    axum::Json(status(&state).await).into_response()
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartBody {
    #[serde(default)]
    pub defaults: BridgeDefaults,
}

/// `POST /api/bridge/start`
pub async fn post_start(
    State(state): State<AppState>,
    super::routes::Body(body): super::routes::Body<StartBody>,
) -> BridgeResult<Response> {
    Ok(axum::Json(start(&state, body.defaults).await?).into_response())
}

/// `POST /api/bridge/stop`
pub async fn post_stop(State(state): State<AppState>) -> BridgeResult<Response> {
    Ok(axum::Json(stop(&state).await?).into_response())
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActiveThreadBody {
    /// `null` clears it, so the next message starts a fresh conversation.
    pub thread_id: Option<String>,
}

/// `POST /api/bridge/active-thread` — change which conversation new messages join.
pub async fn post_active_thread(
    State(state): State<AppState>,
    super::routes::Body(body): super::routes::Body<ActiveThreadBody>,
) -> BridgeResult<Response> {
    if let Some(thread) = &body.thread_id {
        if !super::thread_exists(&state, thread).await {
            return Err(BridgeError::thread_not_found(thread));
        }
    }
    *state.bridge.active_thread.write().await = body.thread_id.clone();
    state
        .bridge
        .update_defaults(|d| d.default_thread = body.thread_id.clone())
        .await;
    Ok(axum::Json(status(&state).await).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_form_fields_mean_unset() {
        let defaults = BridgeDefaults {
            cwd: Some("   ".into()),
            model: Some(String::new()),
            effort: Some("high".into()),
            ..Default::default()
        }
        .normalized();
        assert!(defaults.cwd.is_none());
        assert!(defaults.model.is_none());
        assert_eq!(defaults.effort.as_deref(), Some("high"));
    }

    #[test]
    fn invalid_choices_are_named() {
        let bad = BridgeDefaults {
            effort: Some("turbo".into()),
            ..Default::default()
        };
        assert_eq!(
            bad.validate().unwrap_err().body()["error"]["field"],
            "effort"
        );

        let bad = BridgeDefaults {
            permission_mode: Some("whatever".into()),
            ..Default::default()
        };
        assert_eq!(
            bad.validate().unwrap_err().body()["error"]["field"],
            "permissionMode"
        );

        let bad = BridgeDefaults {
            setting_sources: Some(vec!["nope".into()]),
            ..Default::default()
        };
        assert_eq!(
            bad.validate().unwrap_err().body()["error"]["field"],
            "settingSources"
        );

        let bad = BridgeDefaults {
            cwd: Some("/definitely/not/here".into()),
            ..Default::default()
        };
        assert_eq!(bad.validate().unwrap_err().body()["error"]["field"], "cwd");
    }

    #[test]
    fn a_plausible_configuration_validates() {
        let good = BridgeDefaults {
            cwd: Some(std::env::temp_dir().to_string_lossy().into_owned()),
            model: Some("haiku".into()),
            effort: Some("low".into()),
            thinking: Some("off".into()),
            permission_mode: Some("acceptEdits".into()),
            setting_sources: Some(vec!["user".into(), "project".into()]),
            system_prompt_append: Some("Be terse.".into()),
            default_thread: None,
        };
        assert!(good.validate().is_ok());
    }

    #[test]
    fn the_start_body_rejects_typos() {
        assert!(
            serde_json::from_value::<StartBody>(json!({ "defaults": { "model": "haiku" } }))
                .is_ok()
        );
        assert!(serde_json::from_value::<StartBody>(json!({})).is_ok());
        let err = serde_json::from_value::<StartBody>(json!({ "defaults": { "modl": "haiku" } }))
            .unwrap_err();
        assert!(err.to_string().starts_with("unknown field"));
    }
}
