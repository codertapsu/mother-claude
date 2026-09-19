//! Conversations: create, list, read, fork, rename, reconfigure, close.
//!
//! A "thread" here is a Claude Code session. Creating one starts a real
//! conversation in the Node host and pre-assigns its id, so the caller holds
//! the id before any model work happens — the Agent SDK's `sessionId` option
//! makes that possible. Reads fall back to the on-disk session store, so a
//! conversation stays readable after its process is gone.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use axum::extract::{ConnectInfo, Path, Query, State};
use axum::response::Response;
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};
use uuid::Uuid;

use super::{created, ok, validate_id, Body};
use crate::bridge::error::{BridgeError, BridgeResult};
use crate::bridge::host::{CREATE_TIMEOUT, RPC_TIMEOUT};
use crate::bridge::ThreadRecord;
use crate::server::auth;
use crate::state::AppState;

pub const EFFORT_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
pub const PERMISSION_MODES: [&str; 6] = [
    "default",
    "acceptEdits",
    "bypassPermissions",
    "plan",
    "dontAsk",
    "auto",
];
pub const SETTING_SOURCES: [&str; 3] = ["user", "project", "local"];

/// Options accepted when starting a conversation. Unknown fields are rejected
/// rather than silently ignored, so a typo fails loudly.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct CreateThread {
    /// Pre-assign the conversation id (must be a UUID).
    pub thread_id: Option<String>,
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// "on" | "off"; omit to inherit the user's own Claude settings.
    pub thinking: Option<String>,
    pub permission_mode: Option<String>,
    /// Continue an existing conversation instead of starting a new one.
    pub resume: Option<String>,
    /// With `resume`, branch to a new id instead of continuing in place.
    #[serde(default)]
    pub fork: bool,
    pub title: Option<String>,
    pub system_prompt_append: Option<String>,
    pub allowed_tools: Option<Vec<String>>,
    pub disallowed_tools: Option<Vec<String>>,
    pub additional_directories: Option<Vec<String>>,
    pub setting_sources: Option<Vec<String>>,
    pub mcp_servers: Option<Value>,
    /// JSON Schema for structured output; the result arrives as
    /// `structured_output` on the terminal turn result.
    pub output_schema: Option<Value>,
    pub max_turns: Option<u32>,
    pub max_budget_usd: Option<f64>,
    pub skills: Option<Value>,
    pub agents: Option<Value>,
    #[serde(default = "default_true")]
    pub include_partial_messages: bool,
}

fn default_true() -> bool {
    true
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

/// Where a conversation runs when the caller does not say.
pub fn default_cwd() -> String {
    std::env::var("MOTHER_CLAUDE_BRIDGE_CWD")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| crate::claude::user_home_dir().map(|p| p.to_string_lossy().into_owned()))
        .unwrap_or_else(|| ".".to_string())
}

impl CreateThread {
    /// Fill anything the caller left out from the defaults chosen when the
    /// bridge was started. The request always wins — these are defaults, not
    /// policy.
    pub fn with_defaults(mut self, defaults: &crate::bridge::control::BridgeDefaults) -> Self {
        self.cwd = self.cwd.or_else(|| defaults.cwd.clone());
        self.model = self.model.or_else(|| defaults.model.clone());
        self.effort = self.effort.or_else(|| defaults.effort.clone());
        self.thinking = self.thinking.or_else(|| defaults.thinking.clone());
        self.permission_mode = self
            .permission_mode
            .or_else(|| defaults.permission_mode.clone());
        self.setting_sources = self
            .setting_sources
            .or_else(|| defaults.setting_sources.clone());
        self.system_prompt_append = self
            .system_prompt_append
            .or_else(|| defaults.system_prompt_append.clone());
        self
    }

    fn validate(&self, peer: &SocketAddr, state: &AppState) -> BridgeResult<()> {
        one_of("effort", &self.effort, &EFFORT_LEVELS)?;
        one_of("permission_mode", &self.permission_mode, &PERMISSION_MODES)?;
        one_of("thinking", &self.thinking, &["on", "off"])?;
        if let Some(sources) = &self.setting_sources {
            for source in sources {
                if !SETTING_SOURCES.contains(&source.as_str()) {
                    return Err(BridgeError::invalid_field(
                        "setting_sources",
                        format!(
                            "Unknown setting source {source:?}; expected one of {}.",
                            SETTING_SOURCES.join(", ")
                        ),
                    ));
                }
            }
        }
        if let Some(id) = &self.thread_id {
            Uuid::parse_str(id).map_err(|_| {
                BridgeError::invalid_field("thread_id", "`thread_id` must be a UUID.")
            })?;
        }
        if self.fork && self.resume.is_none() {
            return Err(BridgeError::invalid_field(
                "fork",
                "`fork` requires `resume`: there is nothing to branch from otherwise.",
            ));
        }

        // `bypassPermissions` skips every approval prompt, so it is gated by
        // the same rule the dashboard uses for dangerous actions.
        if self.permission_mode.as_deref() == Some("bypassPermissions")
            && auth::dangerous_blocked(
                true,
                auth::is_loopback(peer),
                state.auth.allow_remote_dangerous,
            )
        {
            return Err(BridgeError::forbidden(
                "dangerous_blocked",
                "permission_mode `bypassPermissions` is restricted to local clients. \
                 Set MOTHER_CLAUDE_ALLOW_REMOTE_DANGEROUS=1 to override.",
            ));
        }
        Ok(())
    }

    fn host_options(&self, cwd: &str) -> Value {
        let mut options = json!({
            "cwd": cwd,
            "permissionMode": self.permission_mode.clone().unwrap_or_else(|| "default".into()),
            "includePartialMessages": self.include_partial_messages,
        });
        let set = |options: &mut Value, key: &str, value: Option<Value>| {
            if let Some(v) = value {
                options[key] = v;
            }
        };
        set(&mut options, "model", self.model.clone().map(Value::from));
        set(&mut options, "effort", self.effort.clone().map(Value::from));
        set(
            &mut options,
            "thinking",
            self.thinking.clone().map(Value::from),
        );
        set(&mut options, "title", self.title.clone().map(Value::from));
        set(&mut options, "resume", self.resume.clone().map(Value::from));
        set(
            &mut options,
            "systemPromptAppend",
            self.system_prompt_append.clone().map(Value::from),
        );
        set(
            &mut options,
            "allowedTools",
            self.allowed_tools.clone().map(|v| json!(v)),
        );
        set(
            &mut options,
            "disallowedTools",
            self.disallowed_tools.clone().map(|v| json!(v)),
        );
        set(
            &mut options,
            "additionalDirectories",
            self.additional_directories.clone().map(|v| json!(v)),
        );
        set(
            &mut options,
            "settingSources",
            self.setting_sources.clone().map(|v| json!(v)),
        );
        set(&mut options, "mcpServers", self.mcp_servers.clone());
        set(&mut options, "skills", self.skills.clone());
        set(&mut options, "agents", self.agents.clone());
        set(&mut options, "maxTurns", self.max_turns.map(Value::from));
        set(
            &mut options,
            "maxBudgetUsd",
            self.max_budget_usd.map(Value::from),
        );
        if let Some(schema) = &self.output_schema {
            options["outputFormat"] = json!({ "type": "json_schema", "schema": schema });
        }
        if self.fork {
            options["forkSession"] = json!(true);
        }
        options
    }
}

/// `POST /threads` — start a conversation and return its id.
pub async fn create(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Body(body): Body<CreateThread>,
) -> BridgeResult<Response> {
    let body = body.with_defaults(&state.bridge.defaults().await);
    body.validate(&peer, &state)?;

    let cwd = body.cwd.clone().unwrap_or_else(default_cwd);
    let path = std::path::Path::new(&cwd);
    if !path.is_dir() {
        return Err(BridgeError::invalid_field(
            "cwd",
            format!("{cwd} is not a directory on this machine."),
        ));
    }

    // Continuing in place keeps the id (and the transcript); everything else
    // gets a fresh one the caller already holds.
    let thread_id = match (&body.resume, body.fork) {
        (Some(resume), false) => resume.clone(),
        _ => body
            .thread_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
    };

    let params = json!({
        "sessionId": thread_id,
        "options": body.host_options(&cwd),
    });
    let result = state
        .bridge
        .host
        .call(&state, "session.create", params, CREATE_TIMEOUT)
        .await?;

    let record = ThreadRecord {
        thread_id: thread_id.clone(),
        cwd: cwd.clone(),
        created_at: crate::claude::registry::now_ms(),
        model: body.model.clone(),
        effort: body.effort.clone(),
        permission_mode: body
            .permission_mode
            .clone()
            .unwrap_or_else(|| "default".into()),
        title: body.title.clone(),
        persisted: body.resume.is_some(),
        active_turn: None,
    };
    state.bridge.insert_thread(record.clone()).await;

    // Surface it on the dashboard too: a bridge conversation is an owned
    // session like any other, and its approval prompts land on the same cards.
    state.mark_owned(&thread_id, &cwd, record.created_at).await;

    Ok(created(json!({
        "thread_id": thread_id,
        "status": "created",
        "cwd": result.get("cwd").and_then(Value::as_str).unwrap_or(&cwd),
        "model": record.model,
        "effort": record.effort,
        "permission_mode": record.permission_mode,
    })))
}

/// `GET /threads` — live conversations, plus what is on disk.
pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> BridgeResult<Response> {
    let live = state.bridge.threads().await;
    let live_only = params
        .get("live_only")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    let stored = if live_only {
        Value::Array(Vec::new())
    } else {
        let mut call = json!({});
        for key in ["limit", "offset"] {
            if let Some(raw) = params.get(key) {
                let parsed: u64 = raw.parse().map_err(|_| {
                    BridgeError::invalid_field(key, format!("`{key}` must be a whole number."))
                })?;
                call[key] = json!(parsed);
            }
        }
        // A bridge that has never started the host should still answer this,
        // so a runtime failure degrades to "live only" rather than a 503.
        match state
            .bridge
            .host
            .call(&state, "store.list", call, RPC_TIMEOUT)
            .await
        {
            Ok(v) => v
                .get("sessions")
                .cloned()
                .unwrap_or(Value::Array(Vec::new())),
            Err(e) => {
                tracing::warn!(error = %e, "bridge could not read the session store");
                Value::Array(Vec::new())
            }
        }
    };

    Ok(ok(json!({ "data": live, "stored": stored })))
}

/// `GET /threads/{id}` — one conversation. `?include_messages=true` inlines its
/// transcript.
pub async fn read(
    State(state): State<AppState>,
    Path(thread_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> BridgeResult<Response> {
    validate_id("thread_id", &thread_id)?;
    let live = state.bridge.thread(&thread_id).await;

    let stored = state
        .bridge
        .host
        .call(
            &state,
            "store.info",
            json!({ "sessionId": thread_id }),
            RPC_TIMEOUT,
        )
        .await
        .ok();

    if live.is_none() && stored.is_none() {
        return Err(BridgeError::thread_not_found(&thread_id));
    }

    let mut body = json!({
        "thread_id": thread_id,
        "live": live,
        "stored": stored,
        "operations": state
            .bridge
            .ops
            .for_thread(&thread_id)
            .iter()
            .map(|op| op.snapshot())
            .collect::<Vec<_>>(),
    });

    if params.get("include_messages").map(String::as_str) == Some("true") {
        let messages = state
            .bridge
            .host
            .call(
                &state,
                "store.messages",
                json!({ "sessionId": thread_id }),
                RPC_TIMEOUT,
            )
            .await?;
        body["messages"] = messages.get("messages").cloned().unwrap_or(Value::Null);
    }

    Ok(ok(body))
}

/// `GET /threads/{id}/messages` — the stored transcript.
pub async fn messages(
    State(state): State<AppState>,
    Path(thread_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> BridgeResult<Response> {
    validate_id("thread_id", &thread_id)?;
    let mut call = json!({ "sessionId": thread_id });
    for key in ["limit", "offset"] {
        if let Some(raw) = params.get(key) {
            let parsed: u64 = raw.parse().map_err(|_| {
                BridgeError::invalid_field(key, format!("`{key}` must be a whole number."))
            })?;
            call[key] = json!(parsed);
        }
    }
    if params.get("include_system_messages").map(String::as_str) == Some("true") {
        call["includeSystemMessages"] = json!(true);
    }
    Ok(ok(state
        .bridge
        .host
        .call(&state, "store.messages", call, RPC_TIMEOUT)
        .await?))
}

/// `GET /threads/{id}/context` — the live context-window breakdown.
pub async fn context(
    State(state): State<AppState>,
    Path(thread_id): Path<String>,
) -> BridgeResult<Response> {
    validate_id("thread_id", &thread_id)?;
    Ok(ok(state
        .bridge
        .host
        .call(
            &state,
            "session.contextUsage",
            json!({ "sessionId": thread_id }),
            RPC_TIMEOUT,
        )
        .await?))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NameBody {
    pub name: String,
}

/// `POST /threads/{id}/name` — set the stored title.
pub async fn rename(
    State(state): State<AppState>,
    Path(thread_id): Path<String>,
    Body(body): Body<NameBody>,
) -> BridgeResult<Response> {
    validate_id("thread_id", &thread_id)?;
    if body.name.trim().is_empty() {
        return Err(BridgeError::invalid_field(
            "name",
            "`name` must not be blank.",
        ));
    }
    state
        .bridge
        .host
        .call(
            &state,
            "store.rename",
            json!({ "sessionId": thread_id, "title": body.name }),
            RPC_TIMEOUT,
        )
        .await?;
    let title = body.name.clone();
    state
        .bridge
        .update_thread(&thread_id, move |r| r.title = Some(title))
        .await;
    Ok(ok(json!({ "thread_id": thread_id, "name": body.name })))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ForkBody {
    /// Fork only up to this message uuid.
    pub up_to_message_id: Option<String>,
    pub title: Option<String>,
}

/// `POST /threads/{id}/fork` — branch the transcript into a new conversation.
pub async fn fork(
    State(state): State<AppState>,
    Path(thread_id): Path<String>,
    Body(body): Body<ForkBody>,
) -> BridgeResult<Response> {
    validate_id("thread_id", &thread_id)?;
    let mut call = json!({ "sessionId": thread_id });
    if let Some(up_to) = &body.up_to_message_id {
        call["upToMessageId"] = json!(up_to);
    }
    if let Some(title) = &body.title {
        call["title"] = json!(title);
    }
    let result = state
        .bridge
        .host
        .call(&state, "store.fork", call, RPC_TIMEOUT)
        .await?;
    let new_id = result
        .get("sessionId")
        .and_then(Value::as_str)
        .ok_or_else(|| BridgeError::claude_error("The fork returned no conversation id."))?;
    Ok(created(json!({
        "thread_id": new_id,
        "forked_from": thread_id,
        "status": "created",
    })))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelBody {
    /// Explicit `null` restores the account default; an absent key is an error,
    /// not a silent reset — so the double `Option` is load-bearing. The outer
    /// one distinguishes "key absent" from "key present and null".
    #[serde(default, deserialize_with = "deserialize_present")]
    pub model: Option<Option<String>>,
}

/// Deserialize a present key, including an explicit `null`.
fn deserialize_present<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

/// `POST /threads/{id}/model` — switch model mid-conversation.
pub async fn set_model(
    State(state): State<AppState>,
    Path(thread_id): Path<String>,
    Body(body): Body<ModelBody>,
) -> BridgeResult<Response> {
    validate_id("thread_id", &thread_id)?;
    let model = body.model.ok_or_else(|| {
        BridgeError::invalid_field(
            "model",
            "`model` is required; send null explicitly to restore the default.",
        )
    })?;
    let result = state
        .bridge
        .host
        .call(
            &state,
            "session.setModel",
            json!({ "sessionId": thread_id, "model": model }),
            RPC_TIMEOUT,
        )
        .await?;
    let model = model.clone();
    state
        .bridge
        .update_thread(&thread_id, move |r| r.model = model)
        .await;
    Ok(ok(result))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct PermissionModeBody {
    pub permission_mode: String,
}

/// `POST /threads/{id}/permission-mode` — change approval behaviour live.
pub async fn set_permission_mode(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(thread_id): Path<String>,
    Body(body): Body<PermissionModeBody>,
) -> BridgeResult<Response> {
    validate_id("thread_id", &thread_id)?;
    one_of(
        "permission_mode",
        &Some(body.permission_mode.clone()),
        &PERMISSION_MODES,
    )?;
    if body.permission_mode == "bypassPermissions"
        && auth::dangerous_blocked(
            true,
            auth::is_loopback(&peer),
            state.auth.allow_remote_dangerous,
        )
    {
        return Err(BridgeError::forbidden(
            "dangerous_blocked",
            "permission_mode `bypassPermissions` is restricted to local clients.",
        ));
    }
    let result = state
        .bridge
        .host
        .call(
            &state,
            "session.setPermissionMode",
            json!({ "sessionId": thread_id, "mode": body.permission_mode }),
            RPC_TIMEOUT,
        )
        .await?;
    let mode = body.permission_mode.clone();
    state
        .bridge
        .update_thread(&thread_id, move |r| r.permission_mode = mode)
        .await;
    Ok(ok(result))
}

/// `DELETE /threads/{id}` — end the live conversation. The transcript stays on
/// disk; `?purge=true` deletes that too.
pub async fn close(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(thread_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> BridgeResult<Response> {
    validate_id("thread_id", &thread_id)?;
    let purge = params.get("purge").map(String::as_str) == Some("true");

    // Deleting a transcript is irreversible, so it obeys the dangerous gate.
    if purge
        && auth::dangerous_blocked(
            true,
            auth::is_loopback(&peer),
            state.auth.allow_remote_dangerous,
        )
    {
        return Err(BridgeError::forbidden(
            "dangerous_blocked",
            "Deleting a transcript is restricted to local clients.",
        ));
    }

    state
        .bridge
        .host
        .call(
            &state,
            "session.close",
            json!({ "sessionId": thread_id }),
            Duration::from_secs(30),
        )
        .await?;
    state.bridge.forget_thread(&thread_id).await;
    state.unmark_owned(&thread_id).await;

    if purge {
        state
            .bridge
            .host
            .call(
                &state,
                "store.delete",
                json!({ "sessionId": thread_id }),
                RPC_TIMEOUT,
            )
            .await?;
    }
    Ok(ok(json!({
        "thread_id": thread_id,
        "status": "closed",
        "purged": purge,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local() -> SocketAddr {
        "127.0.0.1:5000".parse().unwrap()
    }

    fn remote() -> SocketAddr {
        "192.168.1.40:5000".parse().unwrap()
    }

    fn state() -> AppState {
        crate::state::Inner::new(
            crate::claude::ClaudeHome::with_base(std::env::temp_dir()),
            crate::state::ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            auth::Auth::ephemeral(),
        )
    }

    #[test]
    fn enum_fields_are_validated_by_name() {
        let st = state();
        let body = CreateThread {
            effort: Some("turbo".into()),
            ..Default::default()
        };
        let err = body.validate(&local(), &st).unwrap_err();
        assert_eq!(err.code, "invalid_field");
        assert_eq!(err.body()["error"]["field"], "effort");

        let body = CreateThread {
            permission_mode: Some("whatever".into()),
            ..Default::default()
        };
        assert_eq!(
            body.validate(&local(), &st).unwrap_err().body()["error"]["field"],
            "permission_mode"
        );

        let body = CreateThread {
            setting_sources: Some(vec!["user".into(), "nope".into()]),
            ..Default::default()
        };
        assert_eq!(
            body.validate(&local(), &st).unwrap_err().body()["error"]["field"],
            "setting_sources"
        );
    }

    #[test]
    fn bypass_permissions_is_local_only() {
        let st = state();
        let body = CreateThread {
            permission_mode: Some("bypassPermissions".into()),
            ..Default::default()
        };
        assert!(body.validate(&local(), &st).is_ok());
        let err = body.validate(&remote(), &st).unwrap_err();
        assert_eq!(err.code, "dangerous_blocked");
        assert_eq!(err.status, axum::http::StatusCode::FORBIDDEN);
    }

    #[test]
    fn fork_without_resume_is_rejected_and_thread_ids_must_be_uuids() {
        let st = state();
        let body = CreateThread {
            fork: true,
            ..Default::default()
        };
        assert_eq!(
            body.validate(&local(), &st).unwrap_err().body()["error"]["field"],
            "fork"
        );

        let body = CreateThread {
            thread_id: Some("not-a-uuid".into()),
            ..Default::default()
        };
        assert_eq!(
            body.validate(&local(), &st).unwrap_err().body()["error"]["field"],
            "thread_id"
        );

        let body = CreateThread {
            thread_id: Some(Uuid::new_v4().to_string()),
            ..Default::default()
        };
        assert!(body.validate(&local(), &st).is_ok());
    }

    #[test]
    fn host_options_only_carry_what_was_asked_for() {
        let minimal = CreateThread {
            include_partial_messages: true,
            ..Default::default()
        };
        let options = minimal.host_options("/tmp");
        assert_eq!(options["cwd"], "/tmp");
        assert_eq!(options["permissionMode"], "default");
        assert!(options.get("model").is_none());
        assert!(options.get("resume").is_none());
        assert!(options.get("outputFormat").is_none());

        let full = CreateThread {
            model: Some("haiku".into()),
            effort: Some("low".into()),
            resume: Some("abc".into()),
            fork: true,
            output_schema: Some(json!({ "type": "object" })),
            allowed_tools: Some(vec!["Read".into()]),
            ..Default::default()
        };
        let options = full.host_options("/tmp");
        assert_eq!(options["model"], "haiku");
        assert_eq!(options["effort"], "low");
        assert_eq!(options["resume"], "abc");
        assert_eq!(options["forkSession"], true);
        assert_eq!(options["outputFormat"]["type"], "json_schema");
        assert_eq!(options["allowedTools"][0], "Read");
    }

    #[test]
    fn unknown_fields_are_rejected_at_deserialization() {
        let err = serde_json::from_value::<CreateThread>(json!({ "modl": "haiku" })).unwrap_err();
        assert!(err.to_string().starts_with("unknown field"));
    }

    #[test]
    fn start_time_defaults_fill_gaps_but_never_override() {
        let defaults = crate::bridge::control::BridgeDefaults {
            cwd: Some("/from/defaults".into()),
            model: Some("sonnet".into()),
            effort: Some("high".into()),
            permission_mode: Some("acceptEdits".into()),
            ..Default::default()
        };

        // Nothing specified: the defaults apply.
        let merged = CreateThread::default().with_defaults(&defaults);
        assert_eq!(merged.cwd.as_deref(), Some("/from/defaults"));
        assert_eq!(merged.model.as_deref(), Some("sonnet"));
        assert_eq!(merged.permission_mode.as_deref(), Some("acceptEdits"));

        // The request wins wherever it speaks.
        let merged = CreateThread {
            model: Some("haiku".into()),
            cwd: Some("/from/request".into()),
            ..Default::default()
        }
        .with_defaults(&defaults);
        assert_eq!(merged.model.as_deref(), Some("haiku"));
        assert_eq!(merged.cwd.as_deref(), Some("/from/request"));
        assert_eq!(merged.effort.as_deref(), Some("high"));
    }

    #[test]
    fn a_missing_model_key_is_not_a_silent_reset() {
        // Absent: an error, because "{}" almost always means the caller forgot.
        let body: ModelBody = serde_json::from_value(json!({})).unwrap();
        assert!(body.model.is_none());
        // Explicitly null: a deliberate reset to the account default.
        let body: ModelBody = serde_json::from_value(json!({ "model": null })).unwrap();
        assert_eq!(body.model, Some(None));
        // A real model.
        let body: ModelBody = serde_json::from_value(json!({ "model": "haiku" })).unwrap();
        assert_eq!(body.model, Some(Some("haiku".into())));
    }

    #[test]
    fn default_cwd_prefers_the_explicit_override() {
        std::env::set_var("MOTHER_CLAUDE_BRIDGE_CWD", "/tmp/somewhere");
        assert_eq!(default_cwd(), "/tmp/somewhere");
        std::env::remove_var("MOTHER_CLAUDE_BRIDGE_CWD");
        assert!(!default_cwd().is_empty());
    }
}
