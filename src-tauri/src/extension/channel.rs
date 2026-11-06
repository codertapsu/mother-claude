//! `/ws/browser`: the extension's channel (docs/BROWSER-PROTOCOL.md).
//!
//! * Mounted on the loopback listener only. A request must be machine-local
//!   (`ctx.on_machine`: loopback peer and `Host`, no proxy or tunnel
//!   header), carry exactly one `Origin: chrome-extension://<id>`, and the
//!   port must have no raw TCP tunnel pointing at it (a raw tunnel looks
//!   like this machine).
//! * Before `hello` only `pair` and `ping` are accepted; anything else is
//!   answered `error` `unauthenticated` and the connection closes. At most
//!   [`super::hub::MAX_PREAUTH`] unauthenticated connections wait at once.
//! * `hello` with a paired browser's token, from the extension it was paired
//!   with → `welcome` + `state`; the browser's older connection (if any) is
//!   told `replaced` and closed. A revoked browser gets `revoked`.
//! * Frames are JSON text, at most 8 MiB (`frame_too_large`; the request it
//!   answered fails at once); 60 s without a frame drops the connection.
//! * Replies (`result`, `lease.opened`, `ok`, `error`) go to the request
//!   waiting for their `id`; events update what the desktop knows; a Stop
//!   (`lease.revoked`) stops the session; `setSiteMode` and `answer` are the
//!   user's own clicks in the side panel.

use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use regex::Regex;
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};

use super::hub::{self, CloseReason, Conn};
use super::sites::{self, SiteMode, DEFAULT_KEY};
use super::store::{AuthFailure, BrowserInfo};
use crate::pending::{self, Decision, RespondError};
use crate::server::context::{ClientContext, Ingress, Principal};
use crate::server::error::ApiError;
use crate::state::AppState;

/// The largest frame the protocol allows.
pub const MAX_FRAME: usize = 8 * 1024 * 1024;
/// The socket's own limit: past it the connection just closes.
const HARD_FRAME: usize = 16 * 1024 * 1024;
/// A connection silent this long is dropped.
pub const SILENCE: Duration = Duration::from_secs(60);

static HEAD_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#""id"\s*:\s*"([^"\\]{1,128})""#).unwrap_or_else(|e| panic!("bad id pattern: {e}"))
});

/// `chrome-extension://` and the 32 letters `a`–`p` of an extension id.
pub fn extension_origin(origin: &str) -> bool {
    origin
        .strip_prefix("chrome-extension://")
        .is_some_and(|id| id.len() == 32 && id.bytes().all(|b| (b'a'..=b'p').contains(&b)))
}

/// A detected raw TCP tunnel points at the main port.
fn raw_tunnel(state: &AppState) -> bool {
    let port = state.config.port;
    port != 0
        && state
            .remote
            .detected()
            .iter()
            .any(|t| t.kind == "tcp" && t.upstream_port == Some(port))
}

/// `GET /ws/browser`.
pub async fn ws_browser(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    ctx: ClientContext,
) -> Response {
    if ctx.ingress != Ingress::Loopback || !ctx.on_machine {
        return ApiError::forbidden(
            "not_local",
            "The browser channel takes connections from this computer only, never through a \
             proxy or tunnel.",
        )
        .into_response();
    }
    let Some(origin) = ctx.origin.clone().filter(|o| extension_origin(o)) else {
        return ApiError::forbidden(
            "origin_not_allowed",
            "Only the Mother Claude browser extension may connect here.",
        )
        .into_response();
    };
    if raw_tunnel(&state) {
        return ApiError::forbidden(
            "tunnel_exposed",
            "A raw TCP tunnel points at this port, so the browser channel cannot tell this \
             computer from the internet. Stop the tunnel to use the browser extension.",
        )
        .into_response();
    }
    ws.max_message_size(HARD_FRAME)
        .max_frame_size(HARD_FRAME)
        .on_upgrade(move |socket| run(socket, state, ctx, origin))
}

/// What the loop does after a frame.
enum Flow {
    Continue,
    Authenticated(Arc<Conn>, watch::Receiver<Option<CloseReason>>),
    Close,
}

fn error_frame(id: Option<&str>, code: &str, message: &str) -> Value {
    let mut f = json!({ "type": "error", "code": code, "message": message });
    if let Some(id) = id {
        f["id"] = json!(id);
    }
    f
}

fn ok_frame(id: Option<&str>) -> Value {
    let mut f = json!({ "type": "ok" });
    if let Some(id) = id {
        f["id"] = json!(id);
    }
    f
}

fn queue(tx: &mpsc::Sender<String>, frame: &Value) {
    if tx.try_send(frame.to_string()).is_err() {
        tracing::debug!("browser connection queue full; frame dropped");
    }
}

/// Resolves when the watched connection must close.
async fn closed(rx: &mut watch::Receiver<Option<CloseReason>>) -> CloseReason {
    loop {
        if let Some(r) = *rx.borrow_and_update() {
            return r;
        }
        if rx.changed().await.is_err() {
            // Nobody can ask any more.
            return std::future::pending().await;
        }
    }
}

async fn run(socket: WebSocket, state: AppState, mut ctx: ClientContext, origin: String) {
    let ext = &state.extension;
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::channel::<String>(hub::QUEUE);
    let conn_id = ext.hub.next_id();
    let mut close_rx = ext.hub.enter_preauth(conn_id);
    let mut conn: Option<Arc<Conn>> = None;
    let mut last_rx = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    tick.tick().await;
    let mut farewell: Option<Value> = None;
    loop {
        tokio::select! {
            out = rx.recv() => {
                let Some(text) = out else { break };
                if sink.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
            incoming = stream.next() => {
                let Some(Ok(msg)) = incoming else { break };
                last_rx = Instant::now();
                match msg {
                    Message::Text(text) => {
                        match handle(&state, &mut ctx, &origin, conn_id, &tx, conn.as_ref(), &text).await {
                            Flow::Continue => {}
                            Flow::Authenticated(c, watch_rx) => {
                                ext.hub.leave_preauth(conn_id);
                                close_rx = watch_rx;
                                conn = Some(c);
                            }
                            Flow::Close => break,
                        }
                    }
                    Message::Binary(_) => queue(
                        &tx,
                        &error_frame(None, "bad_frame", "Frames are JSON text."),
                    ),
                    Message::Close(_) => break,
                    Message::Ping(_) | Message::Pong(_) => {}
                }
            }
            _ = tick.tick() => {
                if last_rx.elapsed() > SILENCE {
                    farewell = Some(error_frame(None, "timeout", "Nothing received for 60 s."));
                    break;
                }
            }
            reason = closed(&mut close_rx) => {
                farewell = Some(match reason {
                    CloseReason::Revoked => error_frame(
                        None,
                        "revoked",
                        "This browser was removed in Mother Claude. Pair it again to use it.",
                    ),
                    CloseReason::Replaced => error_frame(
                        None,
                        "replaced",
                        "A newer connection of this browser took over.",
                    ),
                    CloseReason::Evicted => error_frame(
                        None,
                        "too_many_connections",
                        "Too many connections are waiting to authenticate.",
                    ),
                });
                break;
            }
        }
    }
    // Flush what was queued (an `error` before closing), then close.
    while let Ok(text) = rx.try_recv() {
        if sink.send(Message::Text(text.into())).await.is_err() {
            break;
        }
    }
    if let Some(f) = farewell {
        let _ = sink.send(Message::Text(f.to_string().into())).await;
    }
    let _ = sink
        .send(Message::Close(Some(CloseFrame {
            code: 1000,
            reason: "".into(),
        })))
        .await;
    ext.hub.leave_preauth(conn_id);
    if let Some(c) = &conn {
        if ext.hub.detach(c) {
            super::notify(&state, "browsers");
        }
    }
}

/// The `id` near the start of a frame too large to parse.
fn head_id(text: &str) -> Option<String> {
    let bytes = &text.as_bytes()[..text.len().min(512)];
    let head = match std::str::from_utf8(bytes) {
        Ok(h) => h,
        Err(e) => std::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap_or_default(),
    };
    HEAD_ID.captures(head).map(|c| c[1].to_string())
}

fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

async fn handle(
    state: &AppState,
    ctx: &mut ClientContext,
    origin: &str,
    conn_id: u64,
    tx: &mpsc::Sender<String>,
    conn: Option<&Arc<Conn>>,
    text: &str,
) -> Flow {
    if text.len() > MAX_FRAME {
        let id = head_id(text);
        if let (Some(c), Some(id)) = (conn, id.as_deref()) {
            c.deliver(
                id,
                json!({
                    "type": "result",
                    "id": id,
                    "ok": false,
                    "error": {
                        "code": "frame_too_large",
                        "message": "The reply was larger than 8 MiB."
                    },
                }),
            );
        }
        queue(
            tx,
            &error_frame(
                id.as_deref(),
                "frame_too_large",
                "Frames are at most 8 MiB.",
            ),
        );
        return Flow::Continue;
    }
    let v: Value = match serde_json::from_str(text) {
        Ok(v @ Value::Object(_)) => v,
        _ => {
            queue(
                tx,
                &error_frame(None, "bad_frame", "A frame is one JSON object."),
            );
            return Flow::Continue;
        }
    };
    let kind = str_field(&v, "type").unwrap_or_default().to_string();
    let id: Option<String> = str_field(&v, "id").map(|s| s.chars().take(128).collect());
    let id = id.as_deref();

    let Some(conn) = conn else {
        return match kind.as_str() {
            "pair" => {
                pair(state, origin, tx, id, &v);
                Flow::Continue
            }
            "hello" => hello(state, ctx, origin, conn_id, tx, id, &v),
            "ping" => {
                queue(tx, &pong(id));
                Flow::Continue
            }
            _ => {
                queue(
                    tx,
                    &error_frame(
                        id,
                        "unauthenticated",
                        "Send `hello` with this browser's token first (or `pair` with a code).",
                    ),
                );
                Flow::Close
            }
        };
    };

    match kind.as_str() {
        "ping" => queue(tx, &pong(id)),
        "pong" => {}
        "result" | "lease.opened" | "ok" | "error" => {
            conn.mark_active();
            let delivered = id.is_some_and(|id| conn.deliver(id, v.clone()));
            if !delivered && kind == "error" {
                tracing::debug!(code = str_field(&v, "code"), "browser error frame");
            }
        }
        "event" => event(state, conn, &v),
        "setSiteMode" => set_site_mode(state, ctx, conn, tx, id, &v),
        "answer" => answer(state, ctx, conn, tx, id, &v).await,
        "pair" => pair(state, origin, tx, id, &v),
        "hello" => {
            // Again on an authenticated connection: fine for the same browser.
            let token = str_field(&v, "token").unwrap_or_default();
            match state.extension.browsers.authenticate(token, origin) {
                Ok(b) if b.id == conn.browser => {
                    queue(tx, &welcome(state, id, &b.id));
                    queue(tx, &super::state_frame(state, &b.id));
                }
                _ => {
                    queue(
                        tx,
                        &error_frame(id, "unauthenticated", "That token is not this browser's."),
                    );
                    return Flow::Close;
                }
            }
        }
        _ => queue(
            tx,
            &error_frame(
                id,
                "unknown_frame",
                &format!(
                    "Unknown frame type {:?}.",
                    kind.chars().take(40).collect::<String>()
                ),
            ),
        ),
    }
    Flow::Continue
}

fn pong(id: Option<&str>) -> Value {
    let mut f = json!({ "type": "pong" });
    if let Some(id) = id {
        f["id"] = json!(id);
    }
    f
}

fn welcome(state: &AppState, id: Option<&str>, browser: &str) -> Value {
    let settings = state.extension.settings.get();
    let mut f = json!({
        "type": "welcome",
        "browserId": browser,
        "protocol": super::PROTOCOL,
        "appVersion": env!("CARGO_PKG_VERSION"),
        "siteModes": settings.site_modes_frame(),
        "settings": super::settings_frame(&settings),
    });
    if let Some(id) = id {
        f["id"] = json!(id);
    }
    f
}

/// `pair` → `paired` (or `bad_code` / `expired_code` / `rate_limited`).
fn pair(state: &AppState, origin: &str, tx: &mpsc::Sender<String>, id: Option<&str>, v: &Value) {
    let ext = &state.extension;
    if let Err(e) = ext.codes.redeem(str_field(v, "code").unwrap_or_default()) {
        queue(tx, &error_frame(id, e.code(), &e.message()));
        return;
    }
    let (view, token) = ext.browsers.issue(
        str_field(v, "name").unwrap_or_default(),
        BrowserInfo::from_frame(v.get("browser")),
        str_field(v, "extensionVersion").unwrap_or_default(),
        origin,
    );
    let mut paired = json!({ "type": "paired", "token": token, "browserId": view.id });
    if let Some(id) = id {
        paired["id"] = json!(id);
    }
    queue(tx, &paired);
    super::activity::audit_change(
        state,
        None,
        "browser.paired",
        json!({ "browser": view.id, "name": view.name, "origin": origin }),
    );
    crate::server::ws::publish_desktop_notice(
        state,
        crate::claude::model::NoticeLevel::Info,
        format!(
            "\"{}\" was paired with Mother Claude. Claude Code sessions can use it within the \
             site modes you set (Browser).",
            view.name
        ),
    );
    super::notify(state, "browsers");
}

/// `hello` → `welcome` + `state`, or `error` and close.
fn hello(
    state: &AppState,
    ctx: &mut ClientContext,
    origin: &str,
    conn_id: u64,
    tx: &mpsc::Sender<String>,
    id: Option<&str>,
    v: &Value,
) -> Flow {
    let ext = &state.extension;
    let token = str_field(v, "token").unwrap_or_default();
    let browser = match ext.browsers.authenticate(token, origin) {
        Ok(b) => b,
        Err(AuthFailure::Revoked) => {
            queue(
                tx,
                &error_frame(
                    id,
                    "revoked",
                    "This browser was removed in Mother Claude. Pair it again to use it.",
                ),
            );
            return Flow::Close;
        }
        Err(AuthFailure::Unknown) => {
            queue(
                tx,
                &error_frame(
                    id,
                    "unauthenticated",
                    "This browser is not paired with Mother Claude (or its token is not valid \
                     here). Pair it again: Mother Claude › Browser › Pair a browser.",
                ),
            );
            return Flow::Close;
        }
    };
    let info = BrowserInfo::from_frame(v.get("browser"));
    let version = str_field(v, "extensionVersion");
    ext.browsers.touch(&browser.id, Some(&info), version);
    let (conn, close_rx) = Conn::new(conn_id, &browser.id, origin, tx.clone());
    if let Ok(mut caps) = conn.capabilities.lock() {
        *caps = v.get("capabilities").cloned().unwrap_or(Value::Null);
    }
    if let Ok(mut ev) = conn.extension_version.lock() {
        *ev = version
            .map(|s| crate::server::devices::sanitize_label(s, 24))
            .unwrap_or_default();
    }
    ctx.principal = Principal::Extension {
        id: browser.id.clone(),
    };
    ext.hub.attach(conn.clone());
    queue(tx, &welcome(state, id, &browser.id));
    queue(tx, &super::state_frame(state, &browser.id));
    super::notify(state, "browsers");
    Flow::Authenticated(conn, close_rx)
}

/// An event from the extension.
fn event(state: &AppState, conn: &Conn, v: &Value) {
    conn.mark_active();
    let leases = &state.extension.leases;
    let lease = str_field(v, "lease");
    let tab = v.get("tabId").and_then(Value::as_i64);
    let session = || tab.and_then(|t| leases.session_of_tab(&conn.browser, lease, t));
    let s = |k: &str| str_field(v, k).map(str::to_string);
    match str_field(v, "event").unwrap_or_default() {
        "tab.updated" => {
            if let (Some(session), Some(t)) = (session(), tab) {
                leases.with_tab(&session, t, |info| info.navigated(s("url"), s("title")));
            }
        }
        "tab.closed" => {
            if let (Some(session), Some(t)) = (session(), tab) {
                leases.close_tab(&session, t);
            }
        }
        "dialog.opened" => {
            if let (Some(session), Some(t)) = (session(), tab) {
                let message = super::redact::text(str_field(v, "message").unwrap_or_default())
                    .chars()
                    .take(300)
                    .collect();
                let kind = s("kind").unwrap_or_else(|| "alert".into());
                leases.with_tab(&session, t, |info| {
                    info.dialog = Some(super::page::Dialog { kind, message })
                });
            }
        }
        "download.started" => {
            let session = lease
                .and_then(|l| leases.by_id(l))
                .filter(|l| l.browser == conn.browser)
                .map(|l| l.session)
                .or_else(session);
            if let Some(session) = session {
                let origin = str_field(v, "url").and_then(sites::origin_of);
                super::activity::record(
                    state,
                    super::activity::Entry {
                        at: crate::claude::registry::now_ms(),
                        session,
                        tool: "download".into(),
                        action: Some("download.started".into()),
                        origin,
                        page: None,
                        outcome: "started".into(),
                        code: None,
                        approved: false,
                        ms: None,
                        browser: Some(conn.browser.clone()),
                    },
                );
            }
        }
        "lease.revoked" => {
            let found = lease
                .and_then(|l| leases.by_id(l))
                .filter(|l| l.browser == conn.browser);
            if let Some(l) = found {
                super::activity::audit_change(
                    state,
                    None,
                    "browser.stop",
                    json!({ "session": l.session, "browser": conn.browser, "by": "user" }),
                );
                let st = state.clone();
                tokio::spawn(async move {
                    super::actions::stop_session(&st, &l.session, "user").await;
                });
            }
        }
        // Resume in the side panel: the user's own click in this browser, for
        // a Stop of this browser only — as the desktop's Resume, and said on
        // the desktop as the side panel's approvals are.
        "lease.resumed" => {
            if let Some(session) = lease.and_then(|l| leases.resume_lease(&conn.browser, l)) {
                super::activity::audit_change(
                    state,
                    None,
                    "browser.resume",
                    json!({ "session": session, "browser": conn.browser, "by": "user" }),
                );
                crate::server::ws::publish_desktop_notice(
                    state,
                    crate::claude::model::NoticeLevel::Info,
                    "A session's browser use was resumed in the browser's side panel.".to_string(),
                );
                super::notify(state, "leases");
                super::push_state(state, &conn.browser);
            }
        }
        "permission.changed" => {
            let granted: Vec<String> = v
                .get("granted")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .take(200)
                .map(|p| p.chars().take(200).collect())
                .collect();
            if let Ok(mut g) = conn.permissions.lock() {
                *g = granted;
            }
            super::notify(state, "browsers");
        }
        // Events from a newer extension.
        _ => {}
    }
}

/// `setSiteMode` from the side panel.
fn set_site_mode(
    state: &AppState,
    ctx: &ClientContext,
    conn: &Conn,
    tx: &mpsc::Sender<String>,
    id: Option<&str>,
    v: &Value,
) {
    let key = match sites::normalize_key(str_field(v, "origin").unwrap_or_default()) {
        Ok(k) => k,
        Err(msg) => return queue(tx, &error_frame(id, "invalid", &msg)),
    };
    let mode = match v.get("mode") {
        None | Some(Value::Null) => None,
        Some(m) => match m.as_str().and_then(SiteMode::parse) {
            Some(mode) => Some(mode),
            None => {
                return queue(
                    tx,
                    &error_frame(id, "invalid", "The mode is one of off, read, act."),
                )
            }
        },
    };
    if mode == Some(SiteMode::Act) {
        let sensitive = key != DEFAULT_KEY
            && !key.contains("://*.")
            && sites::sensitive_category(sites::host_of(&key)).is_some();
        if sensitive || key == DEFAULT_KEY || key.contains("://*.") {
            let what = if sensitive {
                format!("{key} is a sensitive site: letting sessions act on it")
            } else if key == DEFAULT_KEY {
                "Letting sessions act on every site".to_string()
            } else {
                format!("Letting sessions act on every {key} site")
            };
            return queue(
                tx,
                &error_frame(
                    id,
                    "needs_desktop",
                    &format!("{what} is done in the Mother Claude desktop app (Browser)."),
                ),
            );
        }
    }
    let applied = state.extension.settings.update(|s| {
        match (key.as_str(), mode) {
            (DEFAULT_KEY, Some(m)) => s.default_mode = m,
            (DEFAULT_KEY, None) => s.default_mode = SiteMode::Off,
            (k, Some(m)) => {
                s.sites.insert(k.to_string(), m);
            }
            (k, None) => {
                s.sites.remove(k);
            }
        }
        Ok(())
    });
    if let Err(msg) = applied {
        return queue(tx, &error_frame(id, "internal", &msg));
    }
    super::activity::audit_change(
        state,
        Some(ctx),
        "browser.site_mode",
        json!({
            "origin": key,
            "mode": mode.map(SiteMode::as_str),
            "by": "extension",
            "browser": conn.browser,
        }),
    );
    queue(tx, &ok_frame(id));
    if mode == Some(SiteMode::Act) {
        // The extension's token lives in the browser profile: what it
        // raises is said on the desktop.
        crate::server::ws::publish_desktop_notice(
            state,
            crate::claude::model::NoticeLevel::Info,
            format!("Sessions may now act on {key} (set in the browser's side panel)."),
        );
    }
    super::broadcast_state(state);
    super::notify(state, "sites");
}

/// `answer` from the side panel: the first answer wins.
async fn answer(
    state: &AppState,
    ctx: &ClientContext,
    conn: &Conn,
    tx: &mpsc::Sender<String>,
    id: Option<&str>,
    v: &Value,
) {
    let pending_id = str_field(v, "pending").unwrap_or_default();
    let Some(approval) = state
        .extension
        .approval(pending_id)
        .filter(|a| a.browser == conn.browser)
    else {
        let code = if state.pending.is_finished(pending_id) {
            "already_answered"
        } else {
            "not_found"
        };
        return queue(
            tx,
            &error_frame(id, code, "That approval is not waiting any more."),
        );
    };
    let allow = v.get("allow").and_then(Value::as_bool) == Some(true);
    let decision = if allow {
        Decision::allow()
    } else {
        Decision::deny(Some("Denied in the browser's side panel.".into()))
    };
    match pending::respond(state, &approval.session, pending_id, decision, ctx).await {
        Ok(_) => {
            queue(tx, &ok_frame(id));
            if allow {
                // Said on the desktop too: the extension's token lives in
                // the browser profile, so an approval from it is visible.
                crate::server::ws::publish_desktop_notice(
                    state,
                    crate::claude::model::NoticeLevel::Info,
                    format!("Allowed in the browser's side panel: {}", approval.summary),
                );
            }
        }
        Err(RespondError::AlreadyDecided | RespondError::NotFound) => queue(
            tx,
            &error_frame(id, "already_answered", "Someone answered it first."),
        ),
        Err(e) => queue(tx, &error_frame(id, "refused", &format!("{e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_chrome_extension_origins() {
        assert!(extension_origin(
            "chrome-extension://abcdefghijklmnopabcdefghijklmnop"
        ));
        for bad in [
            "chrome-extension://abcdefghijklmnopabcdefghijklmnoq",
            "chrome-extension://ABCDEFGHIJKLMNOPABCDEFGHIJKLMNOP",
            "chrome-extension://abc",
            "chrome-extension://abcdefghijklmnopabcdefghijklmnop/",
            "moz-extension://abcdefghijklmnopabcdefghijklmnop",
            "http://localhost:6725",
            "null",
            "",
        ] {
            assert!(!extension_origin(bad), "{bad}");
        }
    }

    #[test]
    fn ids_are_found_in_a_large_frames_head() {
        let big = format!(
            r#"{{"type":"result","id":"c_17","ok":true,"data":{{"data":"{}"}}}}"#,
            "A".repeat(100)
        );
        assert_eq!(head_id(&big).as_deref(), Some("c_17"));
        assert_eq!(head_id(r#"{"type":"x"}"#), None);
    }
}
