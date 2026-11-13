//! Who is calling a tool (docs/MCP.md "Identity").
//!
//! The plugin's headers helper sends `X-MC-Pid` — its `$PPID`, which is the
//! Claude Code CLI itself (verified on 2.1.319: the CLI starts the helper
//! directly) — and, in a session Mother Claude launched, `X-MC-Session` (the
//! id it gave the session through `MOTHER_CLAUDE_SESSION`). Claude Code runs
//! the helper once per connection, so both are fixed for the life of the
//! connection; the session behind a pid is looked up on every call.
//!
//! * The pid must name a running process **that holds the connection** the
//!   request came on: the process itself or one of its descendants owns the
//!   socket whose local port is the request's peer port (`ps` + `lsof`, the
//!   hooks' binding check, cached per connection for a minute). The token is
//!   in a file any process of the user can read — a session's own Bash
//!   among them — so without this a `curl` in one session could name
//!   another session's pid; with it, it can only ever be itself. A mismatch
//!   is a caller with no pid and no session; a check that cannot run (no
//!   `lsof`, Windows) changes nothing.
//! * The pid's session is the live roster's record for that pid, else
//!   `sessions/<pid>.json` read now from each account's config directory (a
//!   process newer than the last refresh).
//! * `X-MC-Session` counts only for a session Mother Claude owns, and only
//!   when nothing contradicts it: the pid's own pidfile wins (a `claude`
//!   started inside an owned session inherits its variables), and an owned
//!   session whose process the roster knows must be that pid.
//! * Anything else is a [`Caller`] without a session: tools that need one
//!   say so. No tool takes the caller for an approval: dangerous effects
//!   still go through `crate::pending`.

use std::net::SocketAddr;

use axum::http::HeaderMap;

use super::Caller;
use crate::claude::ids;
use crate::server::hooks::Binding;
use crate::state::AppState;

/// The pid header (the CLI's pid).
pub const PID_HEADER: &str = "x-mc-pid";
/// The owned-session header.
pub const SESSION_HEADER: &str = "x-mc-session";

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// The calling process, when the header names a running one.
pub fn pid_of(headers: &HeaderMap) -> Option<u32> {
    header(headers, PID_HEADER)
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|p| *p > 1 && crate::claude::live::pid_exists(*p))
}

/// The session id a request claims (a well-formed id; ownership is checked
/// by [`resolve`]).
pub fn claimed_session(headers: &HeaderMap) -> Option<&str> {
    header(headers, SESSION_HEADER).filter(|s| ids::is_session_id(s))
}

/// The session behind a running pid: `(session id, cwd)`.
pub async fn session_of_pid(state: &AppState, pid: u32) -> Option<(String, Option<String>)> {
    if let Some(rec) = state.live.by_pid(pid) {
        if let Some(sid) = rec.session_id.filter(|s| ids::is_session_id(s)) {
            return Some((sid, rec.cwd));
        }
    }
    let homes = state.accounts.homes();
    tokio::task::spawn_blocking(move || {
        homes
            .iter()
            .find_map(|(_, home)| crate::claude::live::read_pidfile_of(home, pid))
    })
    .await
    .ok()
    .flatten()
    .and_then(|rec| rec.session_id.map(|sid| (sid, rec.cwd)))
}

/// Whether the connection from `peer` belongs to `pid` (cached per
/// connection).
pub async fn binding(state: &AppState, pid: u32, peer: SocketAddr) -> Binding {
    if let Some(b) = state.mcp.cached_binding(pid, peer) {
        return b;
    }
    let b = crate::server::hooks::verify_process_binding(pid, peer).await;
    state.mcp.note_binding(pid, peer, b);
    b
}

/// Who is calling, from the request's headers and the connection it came
/// on (`peer`; `None` skips the binding check — in-process callers, tests).
pub async fn resolve(state: &AppState, headers: &HeaderMap, peer: Option<SocketAddr>) -> Caller {
    let pid = pid_of(headers);
    if let (Some(p), Some(peer)) = (pid, peer) {
        if binding(state, p, peer).await == Binding::Mismatch {
            tracing::warn!(
                pid = p,
                "an MCP request names a process it does not come from; it gets no session"
            );
            return Caller::default();
        }
    }
    let claimed = claimed_session(headers)
        .filter(|s| crate::claude::owned::is_owned(state, s))
        .map(str::to_string);
    let by_pid = match pid {
        Some(p) => session_of_pid(state, p).await,
        None => None,
    };
    if let Some((session_id, cwd)) = by_pid {
        if claimed.as_deref().is_some_and(|c| c != session_id) {
            tracing::debug!(
                claimed = claimed.as_deref().unwrap_or_default(),
                session = %session_id,
                "an MCP caller's pidfile names another session than its header; the pidfile wins"
            );
        }
        let owned = crate::claude::owned::is_owned(state, &session_id);
        let cwd = cwd.or_else(|| state.control.cwd_of(&session_id));
        return Caller {
            session_id: Some(session_id),
            pid,
            cwd,
            owned,
        };
    }
    if let Some(session_id) = claimed {
        // The owned session's own process, as far as anyone knows.
        let known_pid = state.live.get(&session_id).and_then(|l| l.pid);
        if known_pid.is_none() || known_pid == pid {
            return Caller {
                cwd: state.control.cwd_of(&session_id),
                session_id: Some(session_id),
                pid,
                owned: true,
            };
        }
        tracing::debug!(
            session = %session_id,
            "an MCP caller names an owned session whose process is another one; ignored"
        );
    }
    Caller {
        session_id: None,
        pid,
        cwd: None,
        owned: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::live::LiveSession;
    use axum::http::HeaderValue;

    const SID: &str = "0f1e2d3c-4b5a-4968-8776-655443322110";
    const OTHER: &str = "1a2b3c4d-5e6f-4a0b-8c1d-2e3f4a5b6c7d";

    fn state(base: &std::path::Path) -> AppState {
        crate::state::Inner::new(
            crate::claude::ClaudeHome::with_base(base),
            crate::state::ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            crate::server::auth::Auth::ephemeral(),
        )
    }

    fn headers(pairs: &[(&str, String)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    #[tokio::test]
    async fn a_pid_is_mapped_through_its_pidfile() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        let me = std::process::id();
        std::fs::create_dir_all(dir.path().join("sessions")).unwrap();
        std::fs::write(
            dir.path().join(format!("sessions/{me}.json")),
            serde_json::json!({"pid": me, "sessionId": SID, "cwd": "/tmp/proj"}).to_string(),
        )
        .unwrap();
        let c = resolve(&st, &headers(&[(PID_HEADER, me.to_string())]), None).await;
        assert_eq!(c.session_id.as_deref(), Some(SID));
        assert_eq!(c.cwd.as_deref(), Some("/tmp/proj"));
        assert_eq!(c.pid, Some(me));
        assert!(!c.owned);

        // The cached roster answers first.
        st.live.insert_for_test(LiveSession {
            pid: Some(me),
            session_id: Some(OTHER.into()),
            cwd: Some("/tmp/cached".into()),
            ..Default::default()
        });
        let c = resolve(&st, &headers(&[(PID_HEADER, me.to_string())]), None).await;
        assert_eq!(c.session_id.as_deref(), Some(OTHER));
    }

    #[tokio::test]
    async fn unknown_dead_or_malformed_pids_name_no_session() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        for v in ["0", "1", "abc", "-5", "99999999", ""] {
            let c = resolve(&st, &headers(&[(PID_HEADER, v.to_string())]), None).await;
            assert_eq!(c, Caller::default(), "{v:?}");
        }
        // A live process without a pidfile: known pid, no session.
        let me = std::process::id();
        let c = resolve(&st, &headers(&[(PID_HEADER, me.to_string())]), None).await;
        assert_eq!(c.pid, Some(me));
        assert_eq!(c.session_id, None);
    }

    #[tokio::test]
    async fn a_claimed_session_counts_only_when_owned_and_consistent() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        let me = std::process::id();
        // Not owned: ignored.
        let h = headers(&[(PID_HEADER, me.to_string()), (SESSION_HEADER, SID.into())]);
        assert_eq!(resolve(&st, &h, None).await.session_id, None);
        // Owned, process unknown to the roster: taken.
        st.control.note_owned(SID, true);
        let c = resolve(&st, &h, None).await;
        assert_eq!(c.session_id.as_deref(), Some(SID));
        assert!(c.owned);
        // Owned, but the roster knows another process for it: ignored.
        st.live.insert_for_test(LiveSession {
            pid: Some(4242),
            session_id: Some(SID.into()),
            ..Default::default()
        });
        assert_eq!(resolve(&st, &h, None).await.session_id, None);
        // The pid's own pidfile wins over the header (a nested `claude`).
        std::fs::create_dir_all(dir.path().join("sessions")).unwrap();
        std::fs::write(
            dir.path().join(format!("sessions/{me}.json")),
            serde_json::json!({"pid": me, "sessionId": OTHER, "cwd": "/tmp/nested"}).to_string(),
        )
        .unwrap();
        let c = resolve(&st, &h, None).await;
        assert_eq!(c.session_id.as_deref(), Some(OTHER));
        assert!(!c.owned);
        // A malformed claim is never read.
        let h = headers(&[(SESSION_HEADER, "../../etc".into())]);
        assert_eq!(claimed_session(&h), None);
    }

    /// Needs `ps` and `lsof` (macOS, most Linux); without them the check
    /// cannot run and changes nothing, so there is nothing to see.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_pid_counts_only_on_a_connection_it_holds() {
        let lsof = ["/usr/sbin/lsof", "/usr/bin/lsof", "/sbin/lsof"]
            .iter()
            .any(|p| std::path::Path::new(p).is_file());
        if !lsof {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        // What the server sees as the request's peer.
        let peer = client.local_addr().unwrap();
        let me = std::process::id();
        let mut other = crate::proc::std_command("sleep").arg("30").spawn().unwrap();
        let sleeper = other.id();
        std::fs::create_dir_all(dir.path().join("sessions")).unwrap();
        for (pid, sid) in [(me, SID), (sleeper, OTHER)] {
            std::fs::write(
                dir.path().join(format!("sessions/{pid}.json")),
                serde_json::json!({"pid": pid, "sessionId": sid, "cwd": "/tmp/p"}).to_string(),
            )
            .unwrap();
        }
        // Our own connection: we are our session.
        let c = resolve(&st, &headers(&[(PID_HEADER, me.to_string())]), Some(peer)).await;
        assert_eq!(c.session_id.as_deref(), Some(SID));
        // Another process's pid on our connection: nobody at all.
        let c = resolve(
            &st,
            &headers(&[(PID_HEADER, sleeper.to_string())]),
            Some(peer),
        )
        .await;
        assert_eq!(c, Caller::default());
        // And the verdict is kept for the connection.
        assert_eq!(
            st.mcp.cached_binding(sleeper, peer),
            Some(Binding::Mismatch)
        );
        assert_eq!(st.mcp.cached_binding(me, peer), Some(Binding::Verified));
        let _ = other.kill();
        let _ = other.wait();
        drop(client);
    }
}
