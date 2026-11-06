//! From a `browser_*` tool call to a command in the user's browser.
//!
//! [`perform`], for one call:
//!
//! 1. the calling session (`ToolCtx.caller.session_id`; none → refused);
//! 2. its Stop, if the user pressed one in the last minute → refused;
//! 3. its lease ([`ensure_lease`]: the existing one, or `lease.open` in the
//!    browser to use — the pinned one, else the most recently active);
//! 4. the page: what the desktop knows about the tab (asked with `tabs.list`
//!    when it knows nothing);
//! 5. the site mode of the page (or of the target URL), before anything is
//!    sent: `off` / `read` refusals name the origin and how to change it;
//! 6. typing into a password, card or one-time-code field → refused;
//! 7. [`danger::classify`] → an approval through `crate::pending` (desktop,
//!    a phone holding the dangerous grant, or the side panel; first answer
//!    wins), unless a standing grant covers it;
//! 8. the `command` frame, with its deadline; a lost lease is reopened once,
//!    a `navigate` with no tab opens one;
//! 9. what the result teaches (URL, title, refs, the active tab), a check
//!    that content came from a site the session may read, redaction;
//! 10. an activity entry and an audit line (never values or page content).

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::activity::{self, Entry};
use super::danger::{self, Class, Danger, DangerKind};
use super::hub::{Conn, RequestError};
use super::leases::{Lease, Stop};
use super::page::TabInfo;
use super::sites::{self, GrantAction, Resolved, SiteMode};
use super::Approval;
use crate::mcp::{Caller, ToolCtx, ToolError};
use crate::pending::{
    self, PendingItem, PendingKind, PendingSource, PermissionSuggestion, WaitEnd,
};
use crate::state::AppState;

/// How long an approval waits for an answer.
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);
/// How long `lease.open` waits for `lease.opened`.
pub const LEASE_OPEN_TIMEOUT: Duration = Duration::from_secs(10);
/// A command's deadline unless the tool says otherwise.
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;
/// The longest deadline a tool may ask for.
pub const MAX_TIMEOUT_MS: u64 = 120_000;
/// Extra time over a command's deadline before the desktop gives up (the
/// extension answers `timeout` itself 500 ms before it).
const GRACE: Duration = Duration::from_millis(1500);

/// One tool call, translated.
#[derive(Debug, Clone)]
pub struct Call {
    pub tool: &'static str,
    pub action: &'static str,
    pub params: Value,
    pub tab: Option<i64>,
    pub timeout_ms: u64,
    /// `navigate` / `tabs.open`: where to.
    pub target_url: Option<String>,
    /// What it does, for an approval card: "Click “Sign in”".
    pub summary: String,
    /// `upload`: the files, already checked (read after the approval).
    pub upload: Option<Vec<super::upload::UploadFile>>,
}

impl Call {
    pub fn new(tool: &'static str, action: &'static str, params: Value) -> Self {
        Self {
            tool,
            action,
            params,
            tab: None,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            target_url: None,
            summary: String::new(),
            upload: None,
        }
    }
}

/// A command that ran.
#[derive(Debug, Clone, Default)]
pub struct Done {
    /// The result's `data`, redacted by the tool that formats it.
    pub data: Value,
    /// The page afterwards, when known.
    pub url: Option<String>,
    pub title: Option<String>,
    /// A JavaScript dialog blocks the tab.
    pub dialog: Option<super::page::Dialog>,
}

// ───────────────────────────────────────────────────────── refusals

fn no_session() -> ToolError {
    ToolError::new(
        "no_session",
        "Mother Claude could not tell which Claude Code session is calling, so it cannot give \
         it a browser. Browser tools work in sessions that reach Mother Claude through its \
         plugin, or that Mother Claude started.",
    )
}

/// No browser to use, with how the user fixes it.
pub fn no_browser(state: &AppState) -> ToolError {
    let paired: Vec<String> = state
        .extension
        .browsers
        .list()
        .into_iter()
        .filter(|b| !b.revoked)
        .map(|b| b.name)
        .collect();
    if paired.is_empty() {
        ToolError::new(
            "no_browser",
            "No browser is paired with Mother Claude. Ask the user to install the Mother Claude \
             extension and pair it: in the Mother Claude app open Browser › Pair a browser, \
             then type the code it shows into the extension (docs/BROWSER.md).",
        )
    } else {
        ToolError::new(
            "no_browser",
            format!(
                "No paired browser is connected right now ({}). Ask the user to open it: the \
                 extension reconnects by itself.",
                paired.join(", ")
            ),
        )
    }
}

fn stopped(stop: &Stop) -> ToolError {
    let secs = ((stop.until - crate::claude::registry::now_ms()) / 1000).max(1);
    ToolError::new(
        "stopped",
        format!(
            "The user pressed Stop on this session's browser use. Don't retry on your own: ask \
             the user whether to continue (they can resume it in Mother Claude › Browser; \
             otherwise new browser actions are refused for another {secs} s)."
        ),
    )
}

fn mode_refusal(r: &Resolved, needs: SiteMode) -> ToolError {
    let how = match &r.sensitive {
        Some(category) => format!(
            "{} is on Mother Claude's sensitive list ({category}): only the user can open it, \
             from the Mother Claude desktop app (Browser › Site modes).",
            r.origin
        ),
        None => "The user can change it in the extension's side panel or in Mother Claude › \
                 Browser › Site modes."
            .to_string(),
    };
    if r.mode == SiteMode::Off {
        ToolError::new(
            "site_off",
            format!(
                "Browsing {} is off for Claude Code sessions (site mode `off`). {how} Don't \
                 try to reach it another way.",
                r.origin
            ),
        )
    } else {
        ToolError::new(
            "site_read_only",
            format!(
                "{} is read-only for Claude Code sessions (site mode `{}`, this needs `{}`). \
                 You may read it (snapshot, read, find, screenshot) but not act. {how}",
                r.origin,
                r.mode.as_str(),
                needs.as_str()
            ),
        )
    }
}

fn check_mode(r: &Resolved, needs: SiteMode) -> Result<(), ToolError> {
    if r.mode >= needs {
        Ok(())
    } else {
        Err(mode_refusal(r, needs))
    }
}

/// Not a website (about:blank, chrome://…, a file).
fn not_a_site(url: &str) -> ToolError {
    let shown: String = url.chars().take(60).collect();
    ToolError::new(
        "not_a_site",
        format!(
            "The tab shows {shown}, which is not a website. Open one first with \
             browser_navigate."
        ),
    )
}

fn unknown_page() -> ToolError {
    ToolError::new(
        "unknown_page",
        "Mother Claude cannot tell which page this tab shows. Take a browser_snapshot (or open \
         a page with browser_navigate) first.",
    )
}

fn secret_field(r: &str, name: &str) -> ToolError {
    let what = if name.is_empty() {
        r.to_string()
    } else {
        format!("{r} (“{name}”)")
    };
    ToolError::new(
        "secret_field",
        format!(
            "{what} is a password, card, one-time-code or other secret field. Mother Claude \
             never lets a model type into one: ask the user to type it in the browser \
             themselves, then continue."
        ),
    )
}

/// What a command's failure means, for the model.
#[derive(Debug, Clone, PartialEq)]
pub enum CmdError {
    /// The extension answered with an error.
    Ext {
        code: String,
        message: String,
        data: Value,
    },
    /// No answer before the deadline.
    Timeout,
    /// The browser went away.
    Disconnected,
}

/// The readable error for a failed command.
pub fn tool_error(e: &CmdError, page: Option<&str>) -> ToolError {
    let (code, message, data) =
        match e {
            CmdError::Timeout => {
                return ToolError::new(
                    "timeout",
                    "The browser did not answer in time. The page may still be loading: try \
                 browser_wait, then take a new browser_snapshot.",
                )
            }
            CmdError::Disconnected => return ToolError::new(
                "unavailable",
                "The browser disconnected while acting. Ask the user to check the browser; the \
                 extension reconnects by itself.",
            ),
            CmdError::Ext {
                code,
                message,
                data,
            } => (code.as_str(), message.as_str(), data),
        };
    let site = page.unwrap_or("this site");
    let text = match code {
        "stale_ref" => "That ref is gone (the page changed or navigated). Take a new \
                        browser_snapshot and use its refs."
            .to_string(),
        "not_found" => format!(
            "Nothing on the page matches{}. Check the ref or selector, or take a new \
             browser_snapshot.",
            if message.is_empty() {
                String::new()
            } else {
                format!(" ({message})")
            }
        ),
        "timeout" => "The action did not finish in time. The page may still be loading: try \
                      browser_wait, or a longer timeoutMs."
            .to_string(),
        "site_off" => format!(
            "The extension refused: browsing {site} is off. The user can change it in the \
             extension's side panel or in Mother Claude › Browser."
        ),
        "site_read_only" => format!(
            "The extension refused: {site} is read-only. The user can change it in the \
             extension's side panel or in Mother Claude › Browser."
        ),
        "no_tab" => "This session has no tab in its browser group yet: open one with \
                     browser_navigate (or browser_tabs with action \"open\")."
            .to_string(),
        "no_lease" => "The browser lost this session's tab group. Try again.".to_string(),
        "stopped" => "The user pressed Stop in the browser. Don't retry on your own: ask the \
                      user whether to continue."
            .to_string(),
        "dialog_open" => "A JavaScript dialog is open in the tab and blocks it. Read it and \
                          answer it with browser_dialog (accept or dismiss)."
            .to_string(),
        "navigation" => "The page navigated while acting. Take a new browser_snapshot.".to_string(),
        "debugger_unavailable" => "This needs the extension's optional debugger permission \
                                   (screenshots, console, trusted input). Ask the user to grant \
                                   it in the Mother Claude extension's options."
            .to_string(),
        "permission_needed" => format!(
            "The extension needs a browser permission the user has not granted{}. Ask the user \
             to grant it (the extension shows the prompt).",
            data.get("permission")
                .and_then(Value::as_str)
                .map(|p| format!(" ({p})"))
                .unwrap_or_default()
        ),
        "secret_field" => "That is a password, card, one-time-code or other secret field. \
                           Mother Claude never lets a model type into one: ask the user to \
                           type it themselves."
            .to_string(),
        "unsupported" => format!(
            "The extension cannot do this{}. It may need an update.",
            if message.is_empty() {
                String::new()
            } else {
                format!(" ({message})")
            }
        ),
        _ => {
            if message.is_empty() {
                "The browser could not do this.".to_string()
            } else {
                format!("The browser could not do this: {message}")
            }
        }
    };
    ToolError::new(code, super::redact::text(&text).into_owned())
}

// ───────────────────────────────────────────────────────── the session

fn session_of(caller: &Caller) -> Result<String, ToolError> {
    match caller.session_id.as_deref() {
        Some(id) if crate::claude::ids::is_session_id(id) => Ok(id.to_string()),
        _ => Err(no_session()),
    }
}

/// The tab group's title: the session's name and project.
pub async fn session_title(state: &AppState, session: &str) -> String {
    let short = || format!("Claude Code {}", &session[..session.len().min(8)]);
    let Some(s) = state.find_session(session).await else {
        return short();
    };
    let name = [Some(s.name.clone()), s.title.clone()]
        .into_iter()
        .flatten()
        .map(|n| n.trim().to_string())
        .find(|n| !n.is_empty() && n != session)
        .unwrap_or_else(short);
    let title = if !s.project_name.is_empty() && s.project_name != name {
        format!("{name} · {}", s.project_name)
    } else {
        name
    };
    crate::server::devices::sanitize_label(&title, 80)
}

// ───────────────────────────────────────────────────────── leases

/// The browser to open a lease in: the pinned one, else the one the
/// extension used most recently.
fn pick_browser(state: &AppState) -> Result<Arc<Conn>, ToolError> {
    let ext = &state.extension;
    if let Some(pinned) = ext.settings.get().pinned_browser {
        if let Some(conn) = ext.hub.get(&pinned) {
            return Ok(conn);
        }
        let name = ext
            .browsers
            .get(&pinned)
            .map(|b| b.name)
            .unwrap_or_else(|| "The browser Mother Claude is set to use".into());
        return Err(ToolError::new(
            "no_browser",
            format!(
                "{name} (the browser Mother Claude is set to use) is not connected. Ask the user \
                 to open it (the extension reconnects by itself), or to choose another in \
                 Mother Claude › Browser › Settings."
            ),
        ));
    }
    ext.hub.most_recent().ok_or_else(|| no_browser(state))
}

fn new_lease_id() -> String {
    let bytes = crate::server::auth::random_bytes(9);
    let s: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("l_{s}")
}

/// The session's lease and its connection: the existing one, or a new one.
pub async fn ensure_lease(
    state: &AppState,
    session: &str,
) -> Result<(Lease, Arc<Conn>), ToolError> {
    let ext = &state.extension;
    let lock = ext.lease_lock(session);
    let _held = lock.lock().await;
    if let Some(lease) = ext.leases.get(session) {
        if let Some(conn) = ext.hub.get(&lease.browser) {
            ext.leases.touch(session);
            return Ok((lease, conn));
        }
        // Its browser went away: a new lease, wherever the session can act.
        ext.leases.remove(session);
    }
    let conn = pick_browser(state)?;
    let lease = open_lease(state, session, &conn).await?;
    Ok((lease, conn))
}

/// `lease.open` → `lease.opened`.
async fn open_lease(state: &AppState, session: &str, conn: &Arc<Conn>) -> Result<Lease, ToolError> {
    let ext = &state.extension;
    let id = new_lease_id();
    let title = session_title(state, session).await;
    let color = ext.leases.pick_color(&conn.browser);
    let req = ext.hub.request_id("r");
    let frame = json!({
        "type": "lease.open",
        "id": req,
        "lease": id,
        "session": { "id": session, "title": title },
        "color": color,
    });
    let reply = conn
        .request(&req, Some(&id), &frame, LEASE_OPEN_TIMEOUT)
        .await
        .map_err(|e| match e {
            RequestError::Timeout => ToolError::new(
                "unavailable",
                "The browser did not open a tab group for this session in time. Ask the user \
                 to check the browser, then try again.",
            ),
            RequestError::Disconnected => tool_error(&CmdError::Disconnected, None),
        })?;
    let group_id = match reply.get("type").and_then(Value::as_str) {
        Some("lease.opened") => reply.get("groupId").and_then(Value::as_i64),
        _ => {
            let e = reply_error(&reply);
            return Err(tool_error(&e, None));
        }
    };
    let now = crate::claude::registry::now_ms();
    let lease = Lease {
        id,
        session: session.to_string(),
        title,
        browser: conn.browser.clone(),
        color: color.to_string(),
        group_id,
        opened_at: now,
        last_used: now,
        active_tab: None,
        seen_live: false,
    };
    ext.leases.insert(lease.clone());
    lifecycle(state, &lease, "lease.open", "ok");
    super::push_state(state, &lease.browser);
    super::notify(state, "leases");
    Ok(lease)
}

/// End a lease (`lease.close`, best effort): its session ended, it idled, or
/// the user stopped it from the desktop.
pub async fn close_lease(state: &AppState, lease: &Lease, why: &str) {
    let ext = &state.extension;
    if ext.leases.get(&lease.session).map(|l| l.id) != Some(lease.id.clone()) {
        return;
    }
    ext.leases.remove(&lease.session);
    withdraw_approvals(state, &lease.session, why).await;
    if let Some(conn) = ext.hub.get(&lease.browser) {
        conn.fail_lease(&lease.id, "stopped", why);
        let req = ext.hub.request_id("r");
        let frame = json!({
            "type": "lease.close",
            "id": req,
            "lease": lease.id,
            "closeTabs": false,
        });
        let conn = conn.clone();
        tokio::spawn(async move {
            let _ = conn
                .request(&req, None, &frame, Duration::from_secs(5))
                .await;
        });
    }
    lifecycle(state, lease, "lease.close", why);
    super::push_state(state, &lease.browser);
    super::notify(state, "leases");
}

/// The user pressed Stop (the side panel's `lease.revoked`, or the
/// desktop): in-flight commands fail `stopped`, the session's commands are
/// refused for a minute, its approvals go.
pub async fn stop_session(state: &AppState, session: &str, by: &str) -> Option<Stop> {
    let ext = &state.extension;
    let lease = ext.leases.remove(session);
    let browser = lease
        .as_ref()
        .map(|l| l.browser.clone())
        .or_else(|| ext.hub.most_recent().map(|c| c.browser.clone()))?;
    let stop = ext.leases.stop(session, &browser, by);
    if let Some(lease) = &lease {
        if let Some(conn) = ext.hub.get(&lease.browser) {
            conn.fail_lease(&lease.id, "stopped", "The user pressed Stop.");
            if by != "user" {
                // From the desktop: the extension still has the group.
                let req = ext.hub.request_id("r");
                let frame = json!({
                    "type": "lease.close",
                    "id": req,
                    "lease": lease.id,
                    "closeTabs": false,
                });
                tokio::spawn(async move {
                    let _ = conn
                        .request(&req, None, &frame, Duration::from_secs(5))
                        .await;
                });
            }
        }
        lifecycle(state, lease, "stop", by);
    }
    withdraw_approvals(state, session, "stopped by the user").await;
    super::push_state(state, &browser);
    super::notify(state, "leases");
    Some(stop)
}

async fn withdraw_approvals(state: &AppState, session: &str, why: &str) {
    let ids: Vec<String> = state
        .extension
        .approvals_for(None)
        .into_iter()
        .filter(|a| a.session == session)
        .map(|a| a.id)
        .collect();
    for id in ids {
        pending::withdraw(state, &id, why).await;
    }
}

fn lifecycle(state: &AppState, lease: &Lease, step: &str, outcome: &str) {
    activity::record(
        state,
        Entry {
            at: crate::claude::registry::now_ms(),
            session: lease.session.clone(),
            tool: step.to_string(),
            action: None,
            origin: None,
            page: None,
            outcome: outcome.to_string(),
            code: None,
            approved: false,
            ms: None,
            browser: Some(lease.browser.clone()),
        },
    );
}

// ───────────────────────────────────────────────────────── commands

fn reply_error(reply: &Value) -> CmdError {
    let err = reply.get("error");
    let pick = |k: &str| {
        err.and_then(|e| e.get(k))
            .or_else(|| reply.get(k))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .chars()
            .take(500)
            .collect::<String>()
    };
    let code = pick("code");
    CmdError::Ext {
        code: if code.is_empty() {
            "internal".into()
        } else {
            code
        },
        message: pick("message"),
        data: err
            .and_then(|e| e.get("data"))
            .or_else(|| reply.get("data"))
            .cloned()
            .unwrap_or(Value::Null),
    }
}

/// Send one `command` and read its `result`.
pub async fn command(
    state: &AppState,
    conn: &Conn,
    lease: &str,
    tab: Option<i64>,
    action: &str,
    params: &Value,
    timeout_ms: u64,
) -> Result<Value, CmdError> {
    let id = state.extension.hub.request_id("c");
    let mut frame = json!({
        "type": "command",
        "id": id,
        "lease": lease,
        "action": action,
        "params": params,
        "timeoutMs": timeout_ms,
    });
    if let Some(t) = tab {
        frame["tabId"] = json!(t);
    }
    let deadline = Duration::from_millis(timeout_ms) + GRACE;
    let reply = conn
        .request(&id, Some(lease), &frame, deadline)
        .await
        .map_err(|e| match e {
            RequestError::Timeout => CmdError::Timeout,
            RequestError::Disconnected => CmdError::Disconnected,
        })?;
    conn.mark_active();
    match reply.get("type").and_then(Value::as_str) {
        Some("result") if reply.get("ok").and_then(Value::as_bool) == Some(true) => Ok(reply
            .get("data")
            .cloned()
            .filter(|d| !d.is_null())
            .unwrap_or_else(|| json!({}))),
        _ => Err(reply_error(&reply)),
    }
}

/// Ask the extension which tabs the lease has (and learn them).
async fn refresh_tabs(state: &AppState, session: &str, lease: &Lease, conn: &Conn) {
    if let Ok(data) = command(state, conn, &lease.id, None, "tabs.list", &json!({}), 5000).await {
        learn(state, session, None, "tabs.list", &json!({}), &data);
    }
}

/// What a result teaches about the lease's tabs.
fn learn(
    state: &AppState,
    session: &str,
    tab: Option<i64>,
    action: &str,
    params: &Value,
    data: &Value,
) {
    let leases = &state.extension.leases;
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let key = leases.tab_key(session, tab);
    match action {
        "tabs.list" => {
            for t in data
                .get("tabs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(id) = t.get("tabId").and_then(Value::as_i64) else {
                    continue;
                };
                if t.get("inLease").and_then(Value::as_bool) == Some(false) {
                    continue;
                }
                if t.get("active").and_then(Value::as_bool) == Some(true)
                    && leases.get(session).and_then(|l| l.active_tab).is_none()
                {
                    leases.set_active(session, id);
                }
                leases.with_tab(session, id, |info| {
                    info.navigated(s(t, "url"), s(t, "title"))
                });
            }
        }
        "tabs.open" => {
            if let Some(id) = data.get("tabId").and_then(Value::as_i64) {
                if params.get("active").and_then(Value::as_bool) != Some(false) {
                    leases.set_active(session, id);
                }
                leases.with_tab(session, id, |info| {
                    info.navigated(s(data, "url"), s(data, "title"))
                });
            }
        }
        "tabs.select" => {
            if let Some(id) = params.get("tabId").and_then(Value::as_i64) {
                leases.set_active(session, id);
            }
        }
        "tabs.close" => {
            if let Some(id) = params.get("tabId").and_then(Value::as_i64) {
                leases.close_tab(session, id);
            }
        }
        "navigate" | "back" | "forward" | "reload" => leases.with_tab(session, key, |info| {
            if action == "reload" {
                info.refs.clear();
                info.secret_fields = false;
            }
            info.navigated(s(data, "url"), s(data, "title"));
        }),
        "snapshot" => leases.with_tab(session, key, |info| {
            info.navigated(s(data, "url"), s(data, "title"));
            if let Some(tree) = data.get("snapshot").and_then(Value::as_str) {
                info.learn_snapshot(tree, true);
            }
        }),
        "read" => leases.with_tab(session, key, |info| {
            info.navigated(s(data, "url"), s(data, "title"))
        }),
        "find" => {
            if let Some(m) = data.get("matches").and_then(Value::as_array) {
                leases.with_tab(session, key, |info| info.learn_matches(m));
            }
        }
        "click" | "dblclick" | "hover" => leases.with_tab(session, key, |info| {
            // It may have navigated: a result that names its page says so.
            if data.get("url").is_some() {
                info.navigated(s(data, "url"), s(data, "title"));
            }
            if let Some(tree) = data.get("snapshot").and_then(Value::as_str) {
                info.learn_snapshot(tree, false);
            }
        }),
        "dialog" => leases.with_tab(session, key, |info| info.dialog = None),
        _ => {}
    }
}

// ───────────────────────────────────────────────────────── approvals

fn host_label(origin: Option<&str>) -> String {
    origin.map(sites::host_of).unwrap_or("the page").to_string()
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// The card's title: "Click “Sign in” on github.com", "Open www.paypal.com".
fn approval_title(call: &Call, tab: Option<&TabInfo>, origin: Option<&str>) -> String {
    let host = host_label(origin);
    if danger::class_of(call.action) == Class::Navigate {
        return format!("{} {host}", call.summary);
    }
    let element = call.params.get("ref").and_then(Value::as_str).map(|r| {
        match tab.and_then(|t| t.get(r)).filter(|i| !i.name.is_empty()) {
            Some(i) => format!("“{}”", clip(&i.name, 60)),
            None => r.to_string(),
        }
    });
    match element {
        Some(e) => format!("{} {e} on {host}", call.summary),
        None => format!("{} on {host}", call.summary),
    }
}

/// What the card shows: the action, where, the element, and what would be
/// typed (fields the model may type into only — secret fields are refused
/// before this). The audit log never sees any of it.
fn approval_input(
    call: &Call,
    page_url: Option<&str>,
    tab: Option<&TabInfo>,
    browser: &str,
    dangers: &[Danger],
) -> Value {
    let p = &call.params;
    let element = p.get("ref").and_then(Value::as_str).and_then(|r| {
        let info = tab?.get(r)?;
        Some(json!({ "ref": r, "role": info.role, "name": info.name }))
    });
    let mut out = json!({
        "action": call.action,
        "browser": browser,
        "reasons": dangers.iter().map(|d| d.text.clone()).collect::<Vec<_>>(),
    });
    let url = call.target_url.as_deref().or(page_url);
    if let Some(u) = url.and_then(sites::url_without_query) {
        out["url"] = json!(u);
    }
    if let Some(o) = url.and_then(sites::origin_of) {
        out["origin"] = json!(o);
    }
    if let Some(e) = element {
        out["element"] = e;
    } else if let Some(sel) = p.get("selector").and_then(Value::as_str) {
        out["selector"] = json!(clip(sel, 200));
    }
    match call.action {
        "type" => {
            if let Some(t) = p.get("text").and_then(Value::as_str) {
                out["text"] = json!(clip(t, 200));
            }
            out["submit"] = json!(p.get("submit").and_then(Value::as_bool).unwrap_or(false));
        }
        "fill" => {
            let fields: Vec<Value> = p
                .get("fields")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|f| {
                    let r = f.get("ref").and_then(Value::as_str).unwrap_or_default();
                    let name = tab
                        .and_then(|t| t.get(r))
                        .map(|i| i.name.clone())
                        .unwrap_or_default();
                    let value = f
                        .get("value")
                        .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_string))
                        .unwrap_or_default();
                    json!({ "ref": r, "name": name, "value": clip(&value, 200) })
                })
                .collect();
            out["fields"] = json!(fields);
            out["submit"] = json!(p.get("submit").and_then(Value::as_bool).unwrap_or(false));
        }
        "press" => out["key"] = p.get("key").cloned().unwrap_or(Value::Null),
        "eval" => {
            let expr = p
                .get("expression")
                .and_then(Value::as_str)
                .unwrap_or_default();
            out["expression"] = json!(clip(expr, 4000));
            let lower = expr.to_ascii_lowercase();
            if [
                "cookie",
                "localstorage",
                "sessionstorage",
                "indexeddb",
                "credentials",
            ]
            .iter()
            .any(|k| lower.contains(k))
            {
                out["touchesStorage"] = json!(true);
            }
        }
        "upload" => {
            out["files"] = json!(call
                .upload
                .iter()
                .flatten()
                .map(|f| json!({ "name": f.name, "size": f.size }))
                .collect::<Vec<_>>());
        }
        _ => {}
    }
    out
}

/// The "always allow …" suggestion for the reasons a grant can cover.
fn grant_suggestion(dangers: &[Danger]) -> Option<PermissionSuggestion> {
    if dangers
        .iter()
        .any(|d| d.kind.grant().is_none() || d.origin.is_empty())
    {
        return None;
    }
    let grants: Vec<(GrantAction, &str)> = dangers
        .iter()
        .filter_map(|d| d.kind.grant().map(|g| (g, d.origin.as_str())))
        .collect();
    let first = grants.first()?;
    let what = grants
        .iter()
        .map(|(g, _)| g.label())
        .collect::<Vec<_>>()
        .join(", ");
    Some(PermissionSuggestion {
        index: 0,
        label: format!("Always allow sessions to {what} on {}", first.1),
        kind: "browserGrant".into(),
        destination: Some("browser".into()),
        standing: true,
        raw: json!({
            "type": "browserGrant",
            "grants": grants
                .iter()
                .map(|(g, o)| json!({ "action": g.as_str(), "origin": o }))
                .collect::<Vec<_>>(),
        }),
    })
}

/// Create the grants of a chosen "always allow" suggestion.
fn apply_grants(state: &AppState, raw: &Value, by: &str) {
    let grants = raw
        .get("grants")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for g in grants {
        let action = g
            .get("action")
            .and_then(Value::as_str)
            .and_then(GrantAction::parse);
        let origin = g
            .get("origin")
            .and_then(Value::as_str)
            .and_then(sites::origin_of);
        if let (Some(action), Some(origin)) = (action, origin) {
            let _ = add_grant(state, action, &origin, by, None);
        }
    }
}

/// Add a standing grant (audited). Its id.
pub fn add_grant(
    state: &AppState,
    action: GrantAction,
    origin: &str,
    by: &str,
    ctx: Option<&crate::server::context::ClientContext>,
) -> Result<String, String> {
    let id = state.extension.settings.update(|s| {
        if let Some(g) = s.grant_for(action, origin) {
            return Ok(g.id.clone());
        }
        let id = format!("g_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
        s.grants.push(sites::Grant {
            id: id.clone(),
            action,
            origin: origin.to_string(),
            created_at: crate::claude::registry::now_ms(),
            created_by: crate::server::devices::sanitize_label(by, 64),
        });
        Ok(id)
    })?;
    activity::audit_change(
        state,
        ctx,
        "browser.grant.add",
        json!({ "grant": id, "action": action.as_str(), "origin": origin, "by": by }),
    );
    super::notify(state, "grants");
    Ok(id)
}

/// A browser approval the side panel and the Browser page see while it
/// waits — forgotten however the wait ends (answered, withdrawn, timed out,
/// or its caller gone: its future dropped).
struct Tracked {
    state: AppState,
    id: String,
    browser: String,
}

impl Tracked {
    fn new(state: &AppState, approval: Approval) -> Self {
        let (id, browser) = (approval.id.clone(), approval.browser.clone());
        state.extension.track_approval(approval);
        super::push_state(state, &browser);
        super::notify(state, "approvals");
        Self {
            state: state.clone(),
            id,
            browser,
        }
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        self.state.extension.untrack_approval(&self.id);
        super::push_state(&self.state, &self.browser);
        super::notify(&self.state, "approvals");
    }
}

/// One audit line per settled approval, whoever answered it (the desktop, a
/// device, the side panel) or nobody: never its values.
fn audit_approval(
    state: &AppState,
    id: &str,
    session: &str,
    call: &Call,
    origin: Option<&str>,
    end: &WaitEnd,
) {
    let outcome = match end {
        WaitEnd::Decided(d) if d.is_allow() => "allowed",
        WaitEnd::Decided(_) => "denied",
        WaitEnd::TimedOut => "timeout",
        WaitEnd::Closed => "withdrawn",
    };
    let by = state.pending.resolution(id).and_then(|r| r.by);
    activity::audit_change(
        state,
        None,
        "browser.approval",
        json!({
            "pending": id,
            "session": session,
            "tool": call.tool,
            "action": call.action,
            "origin": origin,
            "outcome": outcome,
            "by": by,
        }),
    );
}

/// Ask a person; `Ok` when they allowed it.
#[allow(clippy::too_many_arguments)]
async fn approve(
    state: &AppState,
    session: &str,
    lease: &Lease,
    conn: &Arc<Conn>,
    call: &Call,
    page_url: Option<&str>,
    tab: Option<&TabInfo>,
    dangers: &[Danger],
) -> Result<(), ToolError> {
    let ext = &state.extension;
    // Show the user what is about to be touched (best effort, not awaited).
    if let Some(r) = call.params.get("ref").and_then(Value::as_str) {
        let id = ext.hub.request_id("c");
        let mut frame = json!({
            "type": "command",
            "id": id,
            "lease": lease.id,
            "action": "highlight",
            "params": { "ref": r, "ms": 1500 },
            "timeoutMs": 3000,
        });
        if let Some(t) = call.tab {
            frame["tabId"] = json!(t);
        }
        let (conn, lease_id) = (conn.clone(), lease.id.clone());
        tokio::spawn(async move {
            let _ = conn
                .request(&id, Some(&lease_id), &frame, Duration::from_secs(4))
                .await;
        });
    }
    let browser = ext
        .browsers
        .get(&lease.browser)
        .map(|b| b.name)
        .unwrap_or_else(|| "the browser".into());
    let origin = call
        .target_url
        .as_deref()
        .or(page_url)
        .and_then(sites::origin_of);
    let title = clip(&approval_title(call, tab, origin.as_deref()), 200);
    let reasons = dangers
        .iter()
        .map(|d| d.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let suggestion = grant_suggestion(dangers);
    let item = PendingItem {
        session_id: session.to_string(),
        kind: PendingKind::Permission,
        source: PendingSource::Browser,
        tool_name: Some(call.tool.to_string()),
        title: title.clone(),
        detail: Some(reasons.clone()),
        input: Some(approval_input(call, page_url, tab, &browser, dangers)),
        suggestions: suggestion.iter().cloned().collect(),
        dangerous: true,
        dangerous_reason: Some(reasons.clone()),
        ..Default::default()
    };
    let (guard, rx) = pending::raise(state, item).await;
    let id = guard.id().to_string();
    let tracked = Tracked::new(
        state,
        Approval {
            id: id.clone(),
            browser: lease.browser.clone(),
            lease: lease.id.clone(),
            session: session.to_string(),
            summary: title.clone(),
            detail: Some(reasons),
            created_at: crate::claude::registry::now_ms(),
        },
    );
    let end = pending::wait_end(rx, APPROVAL_TIMEOUT).await;
    drop(tracked);
    audit_approval(state, &id, session, call, origin.as_deref(), &end);
    match end {
        WaitEnd::Decided(d) if d.is_allow() => {
            if let (Some(_), Some(s)) = (d.suggestion, &suggestion) {
                let by = state
                    .pending
                    .resolution(&id)
                    .and_then(|r| r.by)
                    .and_then(|b| b.name)
                    .unwrap_or_else(|| "Mac".into());
                apply_grants(state, &s.raw, &by);
            }
            Ok(())
        }
        WaitEnd::Decided(d) => {
            let note = d
                .message
                .filter(|m| !m.trim().is_empty())
                .map(|m| format!(" Their note: {}", clip(m.trim(), 500)))
                .unwrap_or_default();
            Err(ToolError::new(
                "denied",
                format!(
                    "The user denied this browser action ({title}). Don't retry it; ask the \
                     user what to do instead.{note}"
                ),
            ))
        }
        WaitEnd::TimedOut => {
            guard.expire();
            Err(ToolError::new(
                "approval_timeout",
                format!(
                    "Nobody approved this browser action ({title}) within {} minutes. Ask the \
                     user to approve it in Mother Claude or the extension's side panel, then \
                     try again.",
                    APPROVAL_TIMEOUT.as_secs() / 60
                ),
            ))
        }
        WaitEnd::Closed => match ext.leases.stopped(session) {
            Some(stop) => Err(stopped(&stop)),
            None => Err(ToolError::new(
                "cancelled",
                "The approval for this browser action was withdrawn (the session's browser use \
                 ended). Try again if it is still needed.",
            )),
        },
    }
}

// ───────────────────────────────────────────────────────── perform

/// Run one tool call end to end (see the module docs).
pub async fn perform(ctx: &ToolCtx, call: Call) -> Result<Done, ToolError> {
    let state = &ctx.state;
    let session = session_of(&ctx.caller)?;
    let started = Instant::now();
    let mut approved = false;
    let mut origin_seen: Option<String> = None;
    let mut browser: Option<String> = None;
    let result = run(
        state,
        &session,
        &call,
        &mut approved,
        &mut origin_seen,
        &mut browser,
    )
    .await;
    let (outcome, code) = match &result {
        Ok(_) => ("ok".to_string(), None),
        Err(e) => {
            let outcome = match e.code.as_str() {
                "denied" | "stopped" | "approval_timeout" => e.code.clone(),
                "site_off" | "site_read_only" | "secret_field" | "upload_refused"
                | "screenshots_off" | "not_a_site" => "refused".to_string(),
                _ => "error".to_string(),
            };
            (outcome, Some(e.code.clone()))
        }
    };
    let page = match &result {
        Ok(done) => done.url.as_deref().and_then(sites::url_without_query),
        Err(_) => None,
    }
    .or_else(|| origin_seen.clone());
    activity::record(
        state,
        Entry {
            at: crate::claude::registry::now_ms(),
            session: session.clone(),
            tool: call.tool.to_string(),
            action: Some(call.action.to_string()),
            origin: page.as_deref().and_then(sites::origin_of).or(origin_seen),
            page,
            outcome,
            code,
            approved,
            ms: Some(started.elapsed().as_millis() as u64),
            browser,
        },
    );
    result
}

async fn run(
    state: &AppState,
    session: &str,
    call: &Call,
    approved: &mut bool,
    origin_seen: &mut Option<String>,
    browser: &mut Option<String>,
) -> Result<Done, ToolError> {
    let ext = &state.extension;
    if let Some(stop) = ext.leases.stopped(session) {
        return Err(stopped(&stop));
    }
    let (mut lease, mut conn) = ensure_lease(state, session).await?;
    *browser = Some(lease.browser.clone());
    let class = danger::class_of(call.action);

    // The page.
    let concerns_page = matches!(class, Class::Observe | Class::Act | Class::History);
    let mut tab = ext
        .leases
        .tab(session, ext.leases.tab_key(session, call.tab));
    if concerns_page && tab.as_ref().and_then(|t| t.url.as_ref()).is_none() {
        refresh_tabs(state, session, &lease, &conn).await;
        tab = ext
            .leases
            .tab(session, ext.leases.tab_key(session, call.tab));
    }
    let page_url = tab.as_ref().and_then(|t| t.url.clone());
    let page_origin = page_url.as_deref().and_then(sites::origin_of);
    let settings = ext.settings.get();
    let target_origin = call.target_url.as_deref().and_then(sites::origin_of);
    *origin_seen = if class == Class::Navigate {
        target_origin.clone()
    } else {
        page_origin.clone()
    };

    // Site modes, before anything is sent.
    let page_mode = page_origin.as_deref().map(|o| settings.resolve(o));
    match class {
        Class::Navigate => {
            if let Some(url) = &call.target_url {
                let origin = target_origin.as_deref().ok_or_else(|| {
                    ToolError::invalid(format!(
                        "{} is not a web address: only http and https URLs can be opened.",
                        clip(url, 80)
                    ))
                })?;
                check_mode(&settings.resolve(origin), SiteMode::Read)?;
            }
        }
        Class::Observe | Class::History | Class::Act => match (&page_mode, &page_url) {
            (Some(r), _) => check_mode(r, class.needs().unwrap_or(SiteMode::Act))?,
            (None, Some(url)) => return Err(not_a_site(url)),
            // Unknown page: snapshot and read report their URL (checked
            // below); waiting and scrolling show nothing; the rest wait for
            // the page to be known.
            (None, None) => {
                if !matches!(call.action, "snapshot" | "read" | "wait" | "scroll") {
                    return Err(unknown_page());
                }
            }
        },
        Class::Browser => {}
    }
    if call.action == "screenshot" && !settings.screenshots {
        return Err(ToolError::new(
            "screenshots_off",
            "Screenshots are turned off in Mother Claude (Browser › Settings). Use \
             browser_snapshot or browser_read instead.",
        ));
    }
    if let Some((r, name)) = danger::secret_target(call.action, &call.params, tab.as_ref()) {
        return Err(secret_field(&r, &name));
    }

    // Approval.
    let counted = if class == Class::Navigate {
        target_origin.clone()
    } else {
        page_origin.clone()
    };
    let limit = settings.actions_per_minute;
    let over_limit = class.counts()
        && counted
            .as_deref()
            .is_some_and(|o| ext.leases.recent_actions(session, o) >= limit as usize);
    let facts = danger::Facts {
        action: call.action,
        params: Some(&call.params),
        page_origin: page_origin.as_deref(),
        page_sensitive: page_mode.as_ref().and_then(|r| r.sensitive),
        tab: tab.as_ref(),
        target_origin: target_origin.as_deref(),
        target_sensitive: target_origin
            .as_deref()
            .and_then(|o| sites::sensitive_category(sites::host_of(o))),
        over_limit,
        limit,
        after_stop: ext.leases.after_stop(&lease.browser, session),
    };
    let dangers = danger::uncovered(danger::classify(&facts), &settings);
    if !dangers.is_empty() {
        approve(
            state,
            session,
            &lease,
            &conn,
            call,
            page_url.as_deref(),
            tab.as_ref(),
            &dangers,
        )
        .await?;
        *approved = true;
        if let Some(stop) = ext.leases.stopped(session) {
            return Err(stopped(&stop));
        }
        if dangers.iter().any(|d| d.kind == DangerKind::Burst) {
            if let Some(o) = &counted {
                ext.leases.reset_actions(session, o);
            }
        }
        // The lease may have changed while the person decided.
        let (l, c) = ensure_lease(state, session).await?;
        lease = l;
        conn = c;
    }
    if class.counts() {
        if let Some(o) = &counted {
            ext.leases.count_action(session, o);
        }
    }

    // The command.
    let mut params = call.params.clone();
    if matches!(call.action, "type" | "fill") {
        // Defence in depth: the extension refuses a secret field too.
        params["refuseSecret"] = json!(true);
    }
    if let Some(files) = &call.upload {
        params["files"] = json!(super::upload::read(files)?);
    }
    let mut reply = command(
        state,
        &conn,
        &lease.id,
        call.tab,
        call.action,
        &params,
        call.timeout_ms,
    )
    .await;
    if matches!(&reply, Err(CmdError::Ext { code, .. }) if code == "no_lease") {
        // The extension lost the lease (its service worker restarted).
        ext.leases.remove(session);
        let (l, c) = ensure_lease(state, session).await?;
        lease = l;
        conn = c;
        reply = command(
            state,
            &conn,
            &lease.id,
            call.tab,
            call.action,
            &params,
            call.timeout_ms,
        )
        .await;
    }
    if call.action == "navigate"
        && matches!(&reply, Err(CmdError::Ext { code, .. }) if code == "no_tab")
    {
        let open = json!({ "url": call.target_url, "active": true });
        reply = command(
            state,
            &conn,
            &lease.id,
            None,
            "tabs.open",
            &open,
            call.timeout_ms,
        )
        .await;
        if let Ok(data) = &reply {
            learn(state, session, None, "tabs.open", &open, data);
        }
    }
    ext.leases.touch(session);
    let mut data = reply.map_err(|e| {
        if matches!(&e, CmdError::Ext { code, .. } if code == "stopped") {
            if let Some(stop) = ext.leases.stopped(session) {
                return stopped(&stop);
            }
        }
        tool_error(&e, page_origin.as_deref().or(target_origin.as_deref()))
    })?;

    // Content must come from a site this session may read.
    let url = data.get("url").and_then(Value::as_str).map(str::to_string);
    if url.is_none() && page_origin.is_none() && matches!(call.action, "snapshot" | "read") {
        return Err(unknown_page());
    }
    // An action's snapshot of what changed may show the page it navigated
    // to: kept only when the result says which page that is and the session
    // may read it.
    if class == Class::Act && data.get("snapshot").is_some() {
        let readable = url
            .as_deref()
            .is_some_and(|u| settings.mode_of_url(u) >= SiteMode::Read);
        if !readable {
            if let Some(o) = data.as_object_mut() {
                o.remove("snapshot");
            }
        }
    }
    learn(state, session, call.tab, call.action, &call.params, &data);
    if let Some(u) = &url {
        let shows_content = matches!(class, Class::Observe | Class::History | Class::Navigate);
        if shows_content && settings.mode_of_url(u) < SiteMode::Read {
            let refusal = match sites::origin_of(u) {
                Some(o) => mode_refusal(&settings.resolve(&o), SiteMode::Read),
                None => not_a_site(u),
            };
            return Err(refusal);
        }
    }
    let after = ext
        .leases
        .tab(session, ext.leases.tab_key(session, call.tab));
    let readable = |t: &TabInfo| {
        t.url
            .as_deref()
            .is_some_and(|u| settings.mode_of_url(u) >= SiteMode::Read)
    };
    Ok(Done {
        data,
        url: url.or_else(|| after.as_ref().and_then(|t| t.url.clone())),
        title: after
            .as_ref()
            .filter(|t| readable(t))
            .and_then(|t| t.title.clone()),
        // A dialog's message is the page's: only from a site it may read.
        dialog: after.filter(|t| readable(t)).and_then(|t| t.dialog),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_readable() {
        let e = tool_error(
            &CmdError::Ext {
                code: "stale_ref".into(),
                message: "e14 is gone".into(),
                data: Value::Null,
            },
            None,
        );
        assert_eq!(e.code, "stale_ref");
        assert!(e.message.contains("browser_snapshot"));
        let e = tool_error(
            &CmdError::Ext {
                code: "permission_needed".into(),
                message: String::new(),
                data: json!({"permission": "debugger"}),
            },
            None,
        );
        assert!(e.message.contains("debugger"));
        assert_eq!(tool_error(&CmdError::Timeout, None).code, "timeout");
        assert_eq!(
            tool_error(&CmdError::Disconnected, None).code,
            "unavailable"
        );
        // An unknown code passes through, its message redacted.
        let e = tool_error(
            &CmdError::Ext {
                code: "weird".into(),
                message: "token ghp_abcdefghijklmnopqrstuvwxyz0123456789".into(),
                data: Value::Null,
            },
            None,
        );
        assert_eq!(e.code, "weird");
        assert!(!e.message.contains("ghp_"));
    }

    #[test]
    fn refusals_name_the_site_and_the_way_out() {
        let s = sites::Settings::default();
        let off = mode_refusal(&s.resolve("https://x.test"), SiteMode::Read);
        assert_eq!(off.code, "site_off");
        assert!(off.message.contains("https://x.test"));
        assert!(off.message.contains("side panel"));
        let mut s = sites::Settings::default();
        s.sites.insert("https://x.test".into(), SiteMode::Read);
        let ro = mode_refusal(&s.resolve("https://x.test"), SiteMode::Act);
        assert_eq!(ro.code, "site_read_only");
        assert!(ro.message.contains("`act`"));
        let sens = mode_refusal(&s.resolve("https://www.paypal.com"), SiteMode::Read);
        assert!(sens.message.contains("sensitive"));
        assert!(sens.message.contains("desktop"));
    }

    #[test]
    fn suggestions_cover_only_grantable_reasons() {
        let d = |kind, origin: &str| Danger {
            kind,
            origin: origin.into(),
            text: String::new(),
        };
        let s = grant_suggestion(&[d(DangerKind::Eval, "http://localhost:3000")]).unwrap();
        assert!(s.standing);
        assert_eq!(s.raw["grants"][0]["action"], "eval");
        assert!(s.label.contains("http://localhost:3000"));
        assert!(grant_suggestion(&[d(DangerKind::AfterStop, "https://x.test")]).is_none());
        assert!(grant_suggestion(&[
            d(DangerKind::Eval, "https://x.test"),
            d(DangerKind::AfterStop, "https://x.test")
        ])
        .is_none());
    }

    #[test]
    fn approval_cards_never_show_secret_fields() {
        let mut tab = TabInfo::default();
        tab.learn_snapshot("  - textbox \"Username\" [ref=e3] value=\"\"", true);
        let mut call = Call::new(
            "browser_fill_form",
            "fill",
            json!({"fields": [{"ref": "e3", "value": "marcus"}], "submit": true}),
        );
        call.summary = "Fill a form".into();
        let input = approval_input(
            &call,
            Some("https://x.test/login?next=/a&token=zzz"),
            Some(&tab),
            "Chrome",
            &[],
        );
        assert_eq!(input["url"], "https://x.test/login", "never the query");
        assert_eq!(input["origin"], "https://x.test");
        assert_eq!(input["fields"][0]["name"], "Username");
        assert_eq!(input["submit"], true);
    }
}
