//! Conversations: create, list, read, fork, rename, reconfigure, close.
//!
//! A "thread" here is a Claude Code session. Creating one starts a real
//! conversation in the Node host and pre-assigns its id, so the caller holds
//! the id before any model work happens — the Agent SDK's `sessionId` option
//! makes that possible. Reads fall back to the on-disk session store, so a
//! conversation stays readable after its process is gone.

use std::collections::HashMap;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::response::Response;
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};
use uuid::Uuid;

use super::{created, ok, validate_id, Body};
use crate::bridge::error::{BridgeError, BridgeResult};
use crate::bridge::host::{CREATE_TIMEOUT, RPC_TIMEOUT};
use crate::bridge::{MountPolicy, ThreadRecord};
use crate::server::context::ClientContext;
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
    /// The Claude account to run under (docs/ACCOUNTS.md §5.5): default the
    /// bridge's start-time choice, else the project's, else the accounts'
    /// bridge default. With `resume`, another account than the
    /// conversation's always forks.
    pub account: Option<String>,
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
        self.account = self.account.or_else(|| defaults.account.clone());
        self
    }

    fn validate(&self, ctx: &ClientContext) -> BridgeResult<()> {
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
        if crate::claude::owned::is_dangerous_mode(self.permission_mode.as_deref())
            && !ctx.may_dangerous()
        {
            return Err(BridgeError::forbidden(
                "dangerous_blocked",
                "Permission modes that skip approvals (`bypassPermissions`, `dontAsk`) are \
                 restricted to local clients.",
            ));
        }
        Ok(())
    }

    /// Options that let a caller run code nobody approves: an MCP server
    /// (a stdio command), pre-approved tools, extra directories, a working
    /// directory the user never trusted, or resuming a conversation whose
    /// approvals are switched off. Only callers allowed dangerous actions may
    /// use them (design §10, B1). `caller_cwd` is the cwd the *caller* named
    /// (defaults chosen on the desktop are trusted); its trust is the one of
    /// `account`, the conversation's (each account keeps its own).
    pub async fn require_safe_options(
        &self,
        state: &AppState,
        ctx: &ClientContext,
        caller_cwd: Option<&str>,
        account: &str,
    ) -> BridgeResult<()> {
        let mut risky: Vec<&'static str> = Vec::new();
        if self
            .mcp_servers
            .as_ref()
            .is_some_and(|v| !(v.is_null() || v.as_object().is_some_and(|o| o.is_empty())))
        {
            risky.push("mcp_servers");
        }
        if self.allowed_tools.as_ref().is_some_and(|t| !t.is_empty()) {
            risky.push("allowed_tools");
        }
        if self
            .additional_directories
            .as_ref()
            .is_some_and(|d| !d.is_empty())
        {
            risky.push("additional_directories");
        }
        if let Some(resume) = &self.resume {
            if runs_without_approvals(state, resume).await {
                risky.push("resume");
            }
        }
        let untrusted = match caller_cwd {
            Some(cwd) => {
                let home = state
                    .accounts
                    .home(account)
                    .unwrap_or_else(|| state.home.clone());
                let cwd = cwd.to_string();
                !tokio::task::spawn_blocking(move || cwd_is_trusted(&home, &cwd))
                    .await
                    .unwrap_or(false)
            }
            None => false,
        };
        if ctx.may_dangerous() {
            if !risky.is_empty() || untrusted {
                ctx.note_dangerous("bridge conversation with unapproved-execution options");
            }
            return Ok(());
        }
        if let Some(first) = risky.first() {
            return Err(BridgeError::forbidden(
                "dangerous_blocked",
                format!(
                    "`{first}` can run code without an approval, so it is restricted to the \
                     Mother Claude desktop app, local clients, or a device granted dangerous \
                     actions."
                ),
            )
            .with("field", *first));
        }
        if untrusted {
            return Err(BridgeError::forbidden(
                "untrusted_cwd",
                "That folder has never been opened in Claude Code on this Mac, so its project \
                 hooks and MCP servers are untrusted. Open it in Claude Code once, or start \
                 the conversation from the desktop app.",
            )
            .with("field", "cwd"));
        }
        Ok(())
    }

    fn host_options(&self, cwd: &str, env: &crate::claude::accounts::AccountEnv) -> Value {
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
        // The account's part of the conversation's environment; the host
        // applies it to a copy of its own (never the host's itself).
        if !env.is_inherit() {
            options["env"] = json!(env.overrides());
        }
        options
    }
}

/// The config directory a conversation lives in when it is not the Default
/// account's: the host's store reads and writes look there.
pub fn store_dir(state: &AppState, thread_id: &str) -> Option<String> {
    let account = crate::claude::summaries::session_account(state, thread_id);
    let home = state.accounts.home(&account)?;
    (home.base() != state.home.base()).then(|| home.base().to_string_lossy().into_owned())
}

/// `params` with the conversation's config directory, when it needs one.
fn in_store(state: &AppState, thread_id: &str, mut params: Value) -> Value {
    if let Some(dir) = store_dir(state, thread_id) {
        params["configDir"] = json!(dir);
    }
    params
}

/// Map an account refusal onto the bridge's error envelope.
pub fn account_error(e: crate::claude::accounts::AccountError) -> BridgeError {
    use crate::claude::accounts::AccountError as E;
    match &e {
        E::NotFound(_) => BridgeError::invalid_field("account", e.to_string()),
        E::TokenMissing => BridgeError::conflict(e.code(), e.to_string()),
        _ => BridgeError::invalid_field("account", e.to_string()),
    }
}

/// The account a new conversation in `cwd` runs under: the one asked for;
/// continuing, the conversation's own (one that ran with a token no account
/// holds, or under a removed account, takes the bridge's); else the bridge's.
/// Decided before anything else, so the folder's trust is checked in that
/// account's directory.
fn conversation_account(state: &AppState, body: &CreateThread, cwd: &str) -> BridgeResult<String> {
    let asked = body
        .account
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(str::to_string);
    let account = match (&asked, &body.resume) {
        (Some(a), _) => a.clone(),
        (None, Some(resume)) => {
            let own = crate::claude::summaries::session_account(state, resume);
            if state.accounts.is_usable(&own) {
                own
            } else {
                state.accounts.account_for_bridge(cwd)
            }
        }
        (None, None) => state.accounts.account_for_bridge(cwd),
    };
    if !state.accounts.is_usable(&account) {
        return Err(account_error(
            crate::claude::accounts::AccountError::NotFound(account),
        ));
    }
    Ok(account)
}

/// How `account` may take over `resume`: another account than the
/// conversation's always forks, and one that keeps its own directory needs
/// the conversation copied there. Returns the account's environment.
async fn take_over(
    state: &AppState,
    body: &mut CreateThread,
    account: &str,
) -> BridgeResult<crate::claude::accounts::AccountEnv> {
    let env = state.accounts.env(account).map_err(account_error)?;
    if let Some(resume) = &body.resume {
        // A conversation that ran with a token no account holds is the
        // Default's here: it lives in the Default's directory.
        let source = crate::claude::summaries::acting_account(state, resume);
        let same_dir = match (state.accounts.home(&source), state.accounts.home(account)) {
            (Some(a), Some(b)) => a.base() == b.base(),
            _ => true,
        };
        if !same_dir {
            // Copied into the account's directory first, then forked there.
            use crate::claude::transfer::ops::{copy_for_continue, ContinueCopyError};
            match copy_for_continue(state, &source, account, resume).await {
                Ok(_) => {}
                Err(ContinueCopyError::ConsentRequired) => {
                    return Err(BridgeError::conflict(
                        "copy_consent_required",
                        "That account keeps its own directory: the conversation has to be \
                         copied there first. Allow copies between these two accounts on the \
                         Mother Claude desktop, then try again.",
                    )
                    .with("thread_id", resume.clone()));
                }
                Err(ContinueCopyError::Transfer(e)) => {
                    return Err(BridgeError::conflict(e.code(), e.to_string())
                        .with("thread_id", resume.clone()));
                }
            }
        }
        if account != source {
            body.fork = true;
        }
    }
    Ok(env)
}

/// Whether a conversation runs with approvals switched off
/// (bypassPermissions / dontAsk) by any account: the bridge record (kept in
/// step with the host's reports and with approvals that switch the mode),
/// the newest hook / transcript note, or the summary. Fails closed — the
/// gates on it decide whether unapproved execution is possible.
pub async fn runs_without_approvals(state: &AppState, thread_id: &str) -> bool {
    let record = state
        .bridge
        .thread(thread_id)
        .await
        .map(|t| t.permission_mode);
    let summary = state
        .find_session(thread_id)
        .await
        .and_then(|s| s.permission_mode);
    [
        record,
        crate::claude::summaries::effective_permission_mode(state, thread_id),
        summary,
    ]
    .iter()
    .any(|m| crate::claude::owned::is_dangerous_mode(m.as_deref()))
}

/// Whether the user already trusted `cwd` in Claude Code on this machine: a
/// `~/.claude.json` project entry (for it or a parent folder) with
/// `hasTrustDialogAccepted`, or transcripts recorded in exactly this folder
/// ([`crate::claude::owned::transcripts_recorded_in`] — not merely a
/// `projects/<encoded cwd>` directory, which look-alike paths share).
/// Headless runs show no trust dialog, so an untrusted folder's project hooks
/// and `.mcp.json` would run unasked.
pub fn cwd_is_trusted(home: &crate::claude::ClaudeHome, cwd: &str) -> bool {
    let raw = std::path::PathBuf::from(cwd);
    if !raw.is_absolute() {
        return false;
    }
    let canonical = std::fs::canonicalize(&raw).unwrap_or_else(|_| raw.clone());
    let forms: Vec<std::path::PathBuf> = if canonical == raw {
        vec![raw]
    } else {
        vec![raw, canonical]
    };
    let projects_dir = home.projects_dir();
    if forms
        .iter()
        .any(|p| crate::claude::owned::transcripts_recorded_in(&projects_dir, p))
    {
        return true;
    }
    let Ok(text) = std::fs::read_to_string(home.claude_json()) else {
        return false;
    };
    let Ok(json) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    let Some(projects) = json.get("projects").and_then(Value::as_object) else {
        return false;
    };
    let trusted = |p: &std::path::Path| {
        projects
            .get(p.to_string_lossy().as_ref())
            .and_then(|e| e.get("hasTrustDialogAccepted"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    forms
        .iter()
        .any(|p| p.ancestors().any(|a| a.parent().is_some() && trusted(a)))
}

/// How a conversation that names an existing session id may continue it.
#[derive(Debug, PartialEq, Eq)]
pub enum Takeover {
    /// Nobody else holds it: continue in place (same id, same transcript).
    InPlace,
    /// Another process (a terminal, VS Code) runs it: continue in a fork, or
    /// two processes would append to one transcript and its hooks would be
    /// silenced as "owned".
    Fork,
}

/// Whether session `id` may be continued by the bridge, and how. `409
/// already_owned` when Mother Claude already drives it (Path A/B).
pub async fn takeover(state: &AppState, id: &str) -> BridgeResult<Takeover> {
    if crate::claude::owned::has_handle(state, id) {
        return Err(BridgeError::conflict(
            "already_owned",
            "Mother Claude already drives that session; send it messages instead.",
        )
        .with("thread_id", id.to_string()));
    }
    Ok(if crate::claude::owned::live_elsewhere(state, id).await {
        Takeover::Fork
    } else {
        Takeover::InPlace
    })
}

/// `POST /threads` — start a conversation and return its id.
pub async fn create(
    State(state): State<AppState>,
    ctx: ClientContext,
    mount: MountPolicy,
    Body(body): Body<CreateThread>,
) -> BridgeResult<Response> {
    let caller_cwd = body.cwd.clone();
    let mut body = body.with_defaults(&mount.defaults);
    body.validate(&ctx)?;
    if let (Some(resume), false) = (body.resume.clone(), body.fork) {
        // A session still running elsewhere is continued in a fork.
        if takeover(&state, &resume).await? == Takeover::Fork {
            body.fork = true;
        }
    }

    let cwd = body.cwd.clone().unwrap_or_else(default_cwd);
    let path = std::path::Path::new(&cwd);
    if !path.is_dir() {
        return Err(BridgeError::invalid_field(
            "cwd",
            format!("{cwd} is not a directory on this machine."),
        ));
    }
    let account = conversation_account(&state, &body, &cwd)?;
    body.require_safe_options(&state, &ctx, caller_cwd.as_deref(), &account)
        .await?;
    let env = take_over(&state, &mut body, &account).await?;

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
        "options": body.host_options(&cwd, &env),
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
        account,
        server: mount.server_id().to_string(),
    };
    state.bridge.insert_thread(record.clone()).await;

    // Surface it on the dashboard too: a bridge conversation is an owned
    // session like any other, and its approval prompts land on the same cards.
    state.mark_owned(&thread_id, &cwd, record.created_at).await;

    Ok(created(json!({
        "thread_id": thread_id,
        "forked": body.fork,
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
        let homes = state.accounts.homes();
        if homes.len() == 1 {
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
        } else {
            Value::Array(stored_across_homes(&state, &homes, call).await)
        }
    };

    Ok(ok(json!({ "data": live, "stored": stored })))
}

/// Every account's stored conversations, newest first, each with its
/// `account`: one store read per config directory, the page cut from the
/// merged list.
async fn stored_across_homes(
    state: &AppState,
    homes: &[(String, crate::claude::ClaudeHome)],
    call: Value,
) -> Vec<Value> {
    let offset = call.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let limit = call
        .get("limit")
        .and_then(Value::as_u64)
        .map(|l| l as usize);
    let mut all: Vec<Value> = Vec::new();
    for (account, home) in homes {
        let mut one = json!({});
        if let Some(l) = limit {
            one["limit"] = json!(offset + l);
        }
        if home.base() != state.home.base() {
            one["configDir"] = json!(home.base().to_string_lossy());
        }
        match state
            .bridge
            .host
            .call(state, "store.list", one, RPC_TIMEOUT)
            .await
        {
            Ok(v) => {
                for mut session in v
                    .get("sessions")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                {
                    session["account"] = json!(account);
                    all.push(session);
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, account, "bridge could not read a session store");
            }
        }
    }
    all.sort_by_key(|s| {
        std::cmp::Reverse(s.get("lastModified").and_then(Value::as_i64).unwrap_or(0))
    });
    let page = all.into_iter().skip(offset);
    match limit {
        Some(l) => page.take(l).collect(),
        None => page.collect(),
    }
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
            in_store(&state, &thread_id, json!({ "sessionId": thread_id })),
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
                in_store(&state, &thread_id, json!({ "sessionId": thread_id })),
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
    let call = in_store(&state, &thread_id, call);
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
            in_store(
                &state,
                &thread_id,
                json!({ "sessionId": thread_id, "title": body.name }),
            ),
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
    let call = in_store(&state, &thread_id, call);
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
    ctx: ClientContext,
    Path(thread_id): Path<String>,
    Body(body): Body<PermissionModeBody>,
) -> BridgeResult<Response> {
    validate_id("thread_id", &thread_id)?;
    one_of(
        "permission_mode",
        &Some(body.permission_mode.clone()),
        &PERMISSION_MODES,
    )?;
    if crate::claude::owned::is_dangerous_mode(Some(&body.permission_mode)) && !ctx.may_dangerous()
    {
        return Err(BridgeError::forbidden(
            "dangerous_blocked",
            "Permission modes that skip approvals are restricted to local clients.",
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
    ctx: ClientContext,
    Path(thread_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> BridgeResult<Response> {
    validate_id("thread_id", &thread_id)?;
    let purge = params.get("purge").map(String::as_str) == Some("true");

    // Deleting a transcript is irreversible, so it obeys the dangerous gate.
    if purge && !ctx.may_dangerous() {
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
    // Where its transcript lives, before the conversation is forgotten.
    let delete = in_store(&state, &thread_id, json!({ "sessionId": thread_id }));
    state.bridge.forget_thread(&thread_id).await;
    state.unmark_owned(&thread_id).await;

    if purge {
        state
            .bridge
            .host
            .call(&state, "store.delete", delete, RPC_TIMEOUT)
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

    fn local() -> ClientContext {
        let mut h = axum::http::HeaderMap::new();
        h.insert("host", "127.0.0.1:5612".parse().unwrap());
        ClientContext::new(
            crate::server::context::Ingress::BridgeLoopback,
            "127.0.0.1:5000".parse().unwrap(),
            &h,
            false,
        )
    }

    fn remote() -> ClientContext {
        let mut h = axum::http::HeaderMap::new();
        h.insert("host", "192.168.1.2:6725".parse().unwrap());
        ClientContext::new(
            crate::server::context::Ingress::Lan,
            "192.168.1.40:5000".parse().unwrap(),
            &h,
            false,
        )
    }

    fn test_state() -> (AppState, tempfile::TempDir) {
        let home = tempfile::tempdir().unwrap();
        let st = crate::state::Inner::new(
            crate::claude::ClaudeHome::with_base(home.path()),
            crate::state::ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            crate::server::auth::Auth::ephemeral(),
        );
        (st, home)
    }

    fn record(id: &str, mode: &str) -> crate::bridge::ThreadRecord {
        crate::bridge::ThreadRecord {
            thread_id: id.into(),
            cwd: "/tmp".into(),
            created_at: 1,
            model: None,
            effort: None,
            permission_mode: mode.into(),
            title: None,
            persisted: true,
            active_turn: None,
            account: crate::claude::accounts::DEFAULT_ID.into(),
            server: crate::bridge::MAIN_ID.into(),
        }
    }

    #[tokio::test]
    async fn a_switch_to_bypass_after_start_closes_the_gates() {
        let (st, _home) = test_state();
        let id = "0a1b2c3d-0000-4000-8000-00000000000b";
        st.bridge.insert_thread(record(id, "default")).await;
        assert!(!runs_without_approvals(&st, id).await);
        assert!(
            super::super::turns::require_turn_allowed(&st, &remote(), id)
                .await
                .is_ok()
        );
        assert!(!crate::claude::owned::runs_unapproved(&st, id).await);

        // A plan approved into bypassPermissions: the delivered result
        // carries the switch.
        let item = crate::claude::model::PendingItem {
            kind: crate::claude::model::PendingKind::Plan,
            ..Default::default()
        };
        let mut d = crate::pending::Decision::allow();
        d.mode = Some("bypassPermissions".into());
        let result = crate::pending::permission_result(&item, &d);
        let mode = crate::bridge::mode_set_by(&result).unwrap();
        crate::bridge::note_thread_mode(&st, id, &mode).await;
        assert_eq!(
            st.bridge.thread(id).await.unwrap().permission_mode,
            "bypassPermissions"
        );
        assert!(
            super::super::turns::require_turn_allowed(&st, &remote(), id)
                .await
                .is_err()
        );
        assert!(crate::claude::owned::runs_unapproved(&st, id).await);

        // Fail closed: a stale "default" record does not outvote a newer
        // report of bypass.
        let other = "0a1b2c3d-0000-4000-8000-00000000000c";
        st.bridge.insert_thread(record(other, "default")).await;
        st.activity.note_permission_mode(other, "bypassPermissions");
        assert!(runs_without_approvals(&st, other).await);
        assert!(crate::claude::owned::runs_unapproved(&st, other).await);
    }

    #[tokio::test]
    async fn a_session_live_elsewhere_is_forked_not_taken_over() {
        let (st, _home) = test_state();
        let id = "0a1b2c3d-0000-4000-8000-00000000000d";
        assert_eq!(takeover(&st, id).await.unwrap(), Takeover::InPlace);
        st.live.insert_for_test(crate::claude::live::LiveSession {
            pid: Some(std::process::id()),
            session_id: Some(id.into()),
            status: Some("idle".into()),
            ..Default::default()
        });
        assert_eq!(takeover(&st, id).await.unwrap(), Takeover::Fork);
    }

    #[test]
    fn mode_switches_are_read_from_approval_results() {
        use crate::bridge::mode_set_by;
        use serde_json::json;
        assert_eq!(
            mode_set_by(&json!({ "behavior": "allow", "updatedPermissions": [
                { "type": "addRules", "destination": "session", "rules": [] },
                { "type": "setMode", "mode": "acceptEdits", "destination": "session" },
                { "type": "setMode", "mode": "bypassPermissions", "destination": "cliArg" }
            ] })),
            Some("bypassPermissions".into())
        );
        assert_eq!(
            mode_set_by(&json!({ "behavior": "allow", "updatedPermissions": [
                { "type": "setMode", "mode": "plan", "destination": "userSettings" }
            ] })),
            None
        );
        assert_eq!(
            mode_set_by(&json!({ "behavior": "deny", "updatedPermissions": [
                { "type": "setMode", "mode": "bypassPermissions" }
            ] })),
            None
        );
        assert_eq!(mode_set_by(&json!({ "behavior": "allow" })), None);
    }

    #[test]
    fn enum_fields_are_validated_by_name() {
        let body = CreateThread {
            effort: Some("turbo".into()),
            ..Default::default()
        };
        let err = body.validate(&local()).unwrap_err();
        assert_eq!(err.code, "invalid_field");
        assert_eq!(err.body()["error"]["field"], "effort");

        let body = CreateThread {
            permission_mode: Some("whatever".into()),
            ..Default::default()
        };
        assert_eq!(
            body.validate(&local()).unwrap_err().body()["error"]["field"],
            "permission_mode"
        );

        let body = CreateThread {
            setting_sources: Some(vec!["user".into(), "nope".into()]),
            ..Default::default()
        };
        assert_eq!(
            body.validate(&local()).unwrap_err().body()["error"]["field"],
            "setting_sources"
        );
    }

    #[test]
    fn bypass_permissions_is_local_only() {
        let body = CreateThread {
            permission_mode: Some("bypassPermissions".into()),
            ..Default::default()
        };
        assert!(body.validate(&local()).is_ok());
        let err = body.validate(&remote()).unwrap_err();
        assert_eq!(err.code, "dangerous_blocked");
        assert_eq!(err.status, axum::http::StatusCode::FORBIDDEN);

        // A page on another website calling the loopback bridge: tokenless for
        // ordinary work, but never for skipping approvals.
        let mut h = axum::http::HeaderMap::new();
        h.insert("host", "127.0.0.1:5612".parse().unwrap());
        h.insert("origin", "https://evil.example".parse().unwrap());
        let page = ClientContext::new(
            crate::server::context::Ingress::BridgeLoopback,
            "127.0.0.1:5000".parse().unwrap(),
            &h,
            false,
        );
        assert!(page.on_machine && !page.local);
        assert_eq!(body.validate(&page).unwrap_err().code, "dangerous_blocked");
    }

    #[test]
    fn fork_without_resume_is_rejected_and_thread_ids_must_be_uuids() {
        let body = CreateThread {
            fork: true,
            ..Default::default()
        };
        assert_eq!(
            body.validate(&local()).unwrap_err().body()["error"]["field"],
            "fork"
        );

        let body = CreateThread {
            thread_id: Some("not-a-uuid".into()),
            ..Default::default()
        };
        assert_eq!(
            body.validate(&local()).unwrap_err().body()["error"]["field"],
            "thread_id"
        );

        let body = CreateThread {
            thread_id: Some(Uuid::new_v4().to_string()),
            ..Default::default()
        };
        assert!(body.validate(&local()).is_ok());
    }

    #[test]
    fn host_options_only_carry_what_was_asked_for() {
        let minimal = CreateThread {
            include_partial_messages: true,
            ..Default::default()
        };
        let options = minimal.host_options("/tmp", &crate::claude::accounts::AccountEnv::Inherit);
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
        let options = full.host_options("/tmp", &crate::claude::accounts::AccountEnv::Inherit);
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
