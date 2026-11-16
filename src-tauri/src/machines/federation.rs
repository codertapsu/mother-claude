//! Seeing other machines' sessions (docs/PLAN-MACHINES.md §5.6, D2).
//!
//! For each paired machine this one keeps a WebSocket to the other's own
//! `/ws` over the peer link (its `view` grant decides whether it is
//! accepted), and republishes what arrives on the local bus as
//! `{t:"remote", machine, frame}`:
//!
//! * its session list (`sessions` full and diffs) — cached here, so a
//!   dashboard that connects later gets it at once — its pending items and
//!   their resolutions, its moves, and its runs' and tasks' changes
//!   (`{t:"machines", what:"runs"|"tasks"}`, so a page showing one of its
//!   sessions refetches its background tasks): to everyone;
//! * a session's transcript, stream and agents: only to the clients
//!   subscribed to that machine's session (`sub` with `machine`); the
//!   subscriptions are counted here and forwarded to the other machine.
//!
//! When the link drops the machine is marked offline (`{t:"offline", since}`)
//! and its sessions stay listed, greyed, until it is back. Nothing is copied
//! to disk: seeing is remote.
//!
//! Which of its sessions show here is this computer's choice
//! (`PeerRecord::sessions`: all but the ones stopped, or only the ones
//! chosen): every list, count and pending item republished here leaves the
//! others out — a diff left with nothing for them is not sent — while the
//! cache keeps them all, for the choice's own list ([`choices`]). A changed
//! choice sends every dashboard the list again ([`republish`]). The other
//! computer is never told, and a hidden session's page still opens: its
//! subscriptions are not filtered.
//!
//! The links run only while Machines is on here: none is dialled while it
//! is off, they start when it turns on ([`wake`]) and close when it turns
//! off ([`stop`]). (The other computer refuses its `/ws` while Machines is
//! off there.)

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, Weak};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use ws_client::tungstenite::Message;

use super::store::SessionsShown;
use crate::server::ws::ServerEvent;
use crate::state::{AppState, Inner};

/// What this machine knows of another's sessions.
#[derive(Default)]
struct Remote {
    /// Session summaries by id, as the other machine sent them.
    sessions: BTreeMap<String, Value>,
    /// A whole list of them arrived (until then: "Loading its sessions…").
    loaded: bool,
    online: bool,
    /// Offline since (ms), once it was heard and lost.
    since: Option<i64>,
    /// Frames to send on the open WebSocket.
    tx: Option<mpsc::UnboundedSender<String>>,
    /// Local subscribers per `(session, agent)`.
    subs: HashMap<(String, Option<String>), usize>,
    /// A task keeps (or tries to keep) the link.
    running: bool,
    /// That task, to stop it when Machines turns off.
    task: Option<tokio::task::AbortHandle>,
}

/// The federation's state (`MachinesState::federation`).
#[derive(Default)]
pub struct Federation {
    remotes: Mutex<HashMap<String, Remote>>,
    /// Machines turned on: start the links now rather than at the next tick.
    wake: std::sync::Arc<tokio::sync::Notify>,
}

impl Federation {
    fn with<R>(&self, f: impl FnOnce(&mut HashMap<String, Remote>) -> R) -> R {
        f(&mut self.remotes.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Whether the link to `machine` is up.
    pub fn online(&self, machine: &str) -> bool {
        self.with(|r| r.get(machine).is_some_and(|m| m.online))
    }

    /// How many of `machine`'s sessions show here (`shown`, its choice),
    /// and how many of those run.
    pub fn counts(&self, machine: &str, shown: &SessionsShown) -> Option<(usize, usize)> {
        self.with(|r| {
            r.get(machine).map(|m| {
                let list = shown_list(m.sessions.iter(), shown);
                let running = list
                    .iter()
                    .filter(|s| s.get("live").and_then(Value::as_bool).unwrap_or(false))
                    .count();
                (list.len(), running)
            })
        })
    }

    /// How many of `machine`'s sessions, as last heard, do not show here.
    pub fn hidden(&self, machine: &str, shown: &SessionsShown) -> usize {
        if shown.is_default() {
            return 0;
        }
        self.with(|r| {
            r.get(machine)
                .map(|m| shown.hidden(m.sessions.keys().map(String::as_str)))
                .unwrap_or(0)
        })
    }
}

/// This computer's choice of `machine`'s sessions. Read before the
/// federation's lock is taken, never under it (the view reads the store,
/// then the federation).
fn shown_of(state: &AppState, machine: &str) -> SessionsShown {
    state.machines.store_read(|f| {
        f.peer(machine)
            .map(|p| p.sessions.clone())
            .unwrap_or_default()
    })
}

/// Every paired computer's choice, by id.
fn shown_all(state: &AppState) -> HashMap<String, SessionsShown> {
    state.machines.store_read(|f| {
        f.peers
            .iter()
            .map(|p| (p.id.clone(), p.sessions.clone()))
            .collect()
    })
}

/// The sessions of a list that show here.
fn shown_list<'a>(
    sessions: impl Iterator<Item = (&'a String, &'a Value)>,
    shown: &SessionsShown,
) -> Vec<&'a Value> {
    sessions
        .filter(|(id, _)| shown.shows(id))
        .map(|(_, s)| s)
        .collect()
}

/// What dashboards here get of a `sessions` frame from another computer:
/// only the sessions that show here; a diff left with nothing to upsert or
/// remove is not sent.
fn shown_sessions_frame(frame: &Value, shown: &SessionsShown) -> Option<Value> {
    if shown.is_default() {
        return Some(frame.clone());
    }
    let full = frame.get("full").and_then(Value::as_bool).unwrap_or(false);
    let mut out = frame.clone();
    if let Some(list) = out
        .get_mut(if full { "sessions" } else { "upsert" })
        .and_then(Value::as_array_mut)
    {
        list.retain(|s| {
            s.get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| shown.shows(id))
        });
    }
    let empty = |key: &str| {
        out.get(key)
            .and_then(Value::as_array)
            .is_none_or(|a| a.is_empty())
    };
    if !full && empty("upsert") && empty("remove") {
        return None;
    }
    Some(out)
}

/// Whether a `pending` or `pendingResolved` frame from another computer is
/// about a session that shows here (one it does not name: yes).
fn pending_shows(frame: &Value, shown: &SessionsShown) -> bool {
    let session = match frame.get("t").and_then(Value::as_str) {
        Some("pending") => frame.pointer("/item/sessionId"),
        _ => frame.get("session"),
    };
    session
        .and_then(Value::as_str)
        .is_none_or(|id| shown.shows(id))
}

/// Whether `machine`'s session `session` runs there, as its sessions last
/// said (`None`: not heard of).
pub fn session_live(state: &AppState, machine: &str, session: &str) -> Option<bool> {
    state.machines.federation.with(|r| {
        r.get(machine)?
            .sessions
            .get(session)
            .map(|s| s.get("live").and_then(Value::as_bool).unwrap_or(false))
    })
}

/// The frames a dashboard gets when it connects: every other machine's
/// sessions as last heard, then whether it is online.
pub fn initial_frames(state: &AppState) -> Vec<ServerEvent> {
    let shown = shown_all(state);
    state.machines.federation.with(|r| {
        let mut out = Vec::new();
        for (machine, m) in r.iter() {
            let choice = shown.get(machine).cloned().unwrap_or_default();
            out.push(ServerEvent::Remote {
                machine: machine.clone(),
                frame: json!({
                    "t": "sessions",
                    "full": true,
                    "sessions": shown_list(m.sessions.iter(), &choice),
                    "seq": 0,
                }),
                session: None,
            });
            out.push(ServerEvent::Remote {
                machine: machine.clone(),
                frame: if m.online {
                    json!({ "t": "online" })
                } else {
                    json!({ "t": "offline", "since": m.since })
                },
                session: None,
            });
        }
        out
    })
}

/// Every paired computer's sessions as last heard, for a dashboard that
/// cannot keep a WebSocket (the polling fallback's `GET
/// /api/machines/sessions`): per computer whether it is online, since when
/// it is not, whether a whole list arrived, and its sessions, each stamped
/// with `machine`. Ordered by computer, then session, so the same state
/// always reads the same (the route's ETag). Computers no longer paired are
/// left out.
pub fn snapshot(state: &AppState) -> Vec<Value> {
    let names: HashMap<String, String> = state.machines.store_read(|f| {
        f.peers
            .iter()
            .map(|p| (p.id.clone(), p.display_name().to_string()))
            .collect()
    });
    let shown = shown_all(state);
    state.machines.federation.with(|r| {
        let mut ids: Vec<&String> = r.keys().filter(|id| names.contains_key(*id)).collect();
        ids.sort();
        ids.into_iter()
            .map(|id| {
                let m = &r[id];
                let choice = shown.get(id).cloned().unwrap_or_default();
                let sessions: Vec<Value> = shown_list(m.sessions.iter(), &choice)
                    .into_iter()
                    .map(|s| {
                        let mut s = s.clone();
                        if let Some(o) = s.as_object_mut() {
                            o.insert("machine".into(), json!(id));
                        }
                        s
                    })
                    .collect();
                json!({
                    "machine": id,
                    "name": names.get(id),
                    "online": m.online,
                    "since": if m.online { None } else { m.since },
                    "loaded": m.loaded,
                    "sessions": sessions,
                })
            })
            .collect()
    })
}

/// A local client subscribes to `machine`'s session.
pub fn subscribe(
    state: &AppState,
    machine: &str,
    session: &str,
    agent: Option<&str>,
    since: Option<&str>,
) {
    let first = state.machines.federation.with(|r| {
        let m = r.entry(machine.to_string()).or_default();
        let n = m
            .subs
            .entry((session.to_string(), agent.map(str::to_string)))
            .or_insert(0);
        *n += 1;
        (*n == 1).then(|| m.tx.clone()).flatten()
    });
    // The first subscriber asks the other machine; later ones load the page
    // themselves and get the live frames from then on.
    if let Some(tx) = first {
        let _ = tx.send(sub_frame("sub", session, agent, since));
    }
}

/// A local client unsubscribes.
pub fn unsubscribe(state: &AppState, machine: &str, session: &str, agent: Option<&str>) {
    let last = state.machines.federation.with(|r| {
        let m = r.get_mut(machine)?;
        let key = (session.to_string(), agent.map(str::to_string));
        let n = m.subs.get_mut(&key)?;
        *n = n.saturating_sub(1);
        if *n == 0 {
            m.subs.remove(&key);
            m.tx.clone()
        } else {
            None
        }
    });
    if let Some(tx) = last {
        let _ = tx.send(sub_frame("unsub", session, agent, None));
    }
}

fn sub_frame(t: &str, session: &str, agent: Option<&str>, since: Option<&str>) -> String {
    let mut f = json!({ "t": t, "session": session });
    if let Some(a) = agent {
        f["agent"] = json!(a);
    }
    if let Some(s) = since {
        f["since"] = json!(s);
    }
    f.to_string()
}

/// Forgets a machine (revoked): its sessions leave every dashboard.
pub fn forget(state: &AppState, machine: &str) {
    let tx = state
        .machines
        .federation
        .with(|r| r.remove(machine).and_then(|m| m.tx));
    drop(tx);
    state.broadcast(ServerEvent::Remote {
        machine: machine.to_string(),
        frame: json!({ "t": "sessions", "full": true, "sessions": [], "seq": 0 }),
        session: None,
    });
    state.broadcast(ServerEvent::Remote {
        machine: machine.to_string(),
        frame: json!({ "t": "forgotten" }),
        session: None,
    });
}

// ─── which of its sessions show here ────────────────────────────────────────────

/// `machine`'s sessions as last heard, every one — the ones not shown here
/// too — each with whether it shows here (`synced`), most recently active
/// first: `{mode, loaded, sessions: [{id, name, title, projectName, cwd,
/// live, status, lastActivityAt, synced}]}` (`GET /api/machines/{id}/
/// sessions/sync`).
pub fn choices(state: &AppState, machine: &str) -> Value {
    let shown = shown_of(state, machine);
    let (loaded, mut sessions) = state.machines.federation.with(|r| {
        let Some(m) = r.get(machine) else {
            return (false, Vec::new());
        };
        let list: Vec<Value> = m
            .sessions
            .iter()
            .map(|(id, s)| {
                let field = |k: &str| s.get(k).cloned().unwrap_or(Value::Null);
                json!({
                    "id": id,
                    "name": field("name"),
                    "title": field("title"),
                    "projectName": field("projectName"),
                    "cwd": field("cwd"),
                    "live": s.get("live").and_then(Value::as_bool).unwrap_or(false),
                    "status": field("status"),
                    "lastActivityAt": field("lastActivityAt"),
                    "synced": shown.shows(id),
                })
            })
            .collect();
        (m.loaded, list)
    });
    let at = |v: &Value| v["lastActivityAt"].as_i64().unwrap_or(0);
    sessions.sort_by_key(|s| std::cmp::Reverse(at(s)));
    json!({ "mode": shown.mode, "loaded": loaded, "sessions": sessions })
}

/// Which of `machine`'s sessions show here changed: every dashboard gets
/// its list again (sessions newly hidden leave it, newly shown join it),
/// then whether it is online (a list alone would say it is).
pub fn republish(state: &AppState, machine: &str) {
    let shown = shown_of(state, machine);
    let frames = state.machines.federation.with(|r| {
        let m = r.get(machine)?;
        Some([
            json!({
                "t": "sessions",
                "full": true,
                "sessions": shown_list(m.sessions.iter(), &shown),
                "seq": 0,
            }),
            if m.online {
                json!({ "t": "online" })
            } else {
                json!({ "t": "offline", "since": m.since })
            },
        ])
    });
    for frame in frames.into_iter().flatten() {
        state.broadcast(ServerEvent::Remote {
            machine: machine.to_string(),
            frame,
            session: None,
        });
    }
}

// ─── the links ─────────────────────────────────────────────────────────────────

/// Machines turned on here: the links start now.
pub fn wake(state: &AppState) {
    state.machines.federation.wake.notify_one();
}

/// Machines turned off here: every link closes, and each machine shows as
/// offline (its sessions as last heard stay cached for when it is back).
pub fn stop(state: &AppState) {
    let stopped: Vec<String> = state.machines.federation.with(|r| {
        r.iter_mut()
            .filter_map(|(id, m)| {
                if let Some(t) = m.task.take() {
                    t.abort();
                }
                m.running = false;
                m.tx = None;
                m.online.then(|| id.clone())
            })
            .collect()
    });
    for machine in stopped {
        went_offline(state, &machine);
    }
}

/// Keeps a link to every paired machine while the transport runs and
/// Machines is on here.
pub(crate) async fn run(state: Weak<Inner>) {
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        let wake = match state.upgrade() {
            Some(st) => st.machines.federation.wake.clone(),
            None => break,
        };
        tokio::select! {
            _ = tick.tick() => {}
            _ = wake.notified() => {}
        }
        let Some(st) = state.upgrade() else { break };
        if !st.machines.enabled() {
            continue;
        }
        let peers: Vec<String> = st
            .machines
            .store_read(|f| f.peers.iter().map(|p| p.id.clone()).collect());
        for machine in peers {
            st.machines.federation.with(|r| {
                let m = r.entry(machine.clone()).or_default();
                if !std::mem::replace(&mut m.running, true) {
                    m.task = Some(tokio::spawn(keep_link(state.clone(), machine)).abort_handle());
                }
            });
        }
    }
}

async fn keep_link(state: Weak<Inner>, machine: String) {
    let mut backoff = Duration::from_secs(1);
    loop {
        let Some(st) = state.upgrade() else { return };
        if !st.machines.enabled() {
            // Turned off since: [`stop`] closed the others; this one ends.
            st.machines.federation.with(|r| {
                if let Some(m) = r.get_mut(&machine) {
                    m.running = false;
                    m.task = None;
                }
            });
            return;
        }
        let Some(key) = st
            .machines
            .store_read(|f| f.peer(&machine).map(|p| p.key.clone()))
        else {
            forget(&st, &machine);
            return;
        };
        match link(&st, &machine, &key).await {
            Ok(()) => backoff = Duration::from_secs(1),
            Err(e) => {
                tracing::debug!(machine, error = %e, "no link to the other machine's sessions")
            }
        }
        went_offline(&st, &machine);
        drop(st);
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

fn went_offline(state: &AppState, machine: &str) {
    let changed = state.machines.federation.with(|r| {
        let m = r.entry(machine.to_string()).or_default();
        m.tx = None;
        let was = std::mem::replace(&mut m.online, false);
        if was {
            m.since = Some(super::now());
        }
        was.then_some(m.since)
    });
    if let Some(since) = changed {
        state.broadcast(ServerEvent::Remote {
            machine: machine.to_string(),
            frame: json!({ "t": "offline", "since": since }),
            session: None,
        });
    }
}

/// One WebSocket to the other machine's `/ws`, until it ends.
async fn link(state: &AppState, machine: &str, key: &str) -> Result<(), String> {
    let conn = super::transport::connection_to(state, key).await?;
    let (send, recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
    let (ws, _) = ws_client::client_async("ws://peer/ws", tokio::io::join(recv, send))
        .await
        .map_err(|e| format!("the other machine refused its sessions: {e}"))?;
    let (mut sink, mut stream) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    // Inbox frames (pending items), then every session a dashboard here
    // watches.
    let _ = tx.send(json!({ "t": "subInbox" }).to_string());
    let resubs: Vec<(String, Option<String>)> = state.machines.federation.with(|r| {
        let m = r.entry(machine.to_string()).or_default();
        m.tx = Some(tx.clone());
        m.online = true;
        m.since = None;
        m.subs.keys().cloned().collect()
    });
    for (s, a) in resubs {
        let _ = tx.send(sub_frame("sub", &s, a.as_deref(), None));
    }
    state.broadcast(ServerEvent::Remote {
        machine: machine.to_string(),
        frame: json!({ "t": "online" }),
        session: None,
    });
    let writer = tokio::spawn(async move {
        while let Some(text) = rx.recv().await {
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });
    let result = loop {
        let Some(msg) = stream.next().await else {
            break Ok(());
        };
        let text = match msg {
            Ok(Message::Text(t)) => t,
            Ok(Message::Close(_)) => break Ok(()),
            Ok(_) => continue,
            Err(e) => break Err(e.to_string()),
        };
        let Ok(frame) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        handle(state, machine, &tx, frame);
    };
    writer.abort();
    result
}

/// One frame from the other machine.
fn handle(state: &AppState, machine: &str, tx: &mpsc::UnboundedSender<String>, frame: Value) {
    let t = frame.get("t").and_then(Value::as_str).unwrap_or("");
    let session_scoped = || {
        frame
            .get("session")
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let send = |frame: Value, session: Option<String>| {
        state.broadcast(ServerEvent::Remote {
            machine: machine.to_string(),
            frame,
            session,
        });
    };
    let publish = |session: Option<String>| send(frame.clone(), session);
    match t {
        "ping" => {
            let _ = tx.send(json!({ "t": "pong" }).to_string());
        }
        "sessions" => {
            // Kept whole; passed on with only the sessions shown here.
            cache_sessions(state, machine, &frame);
            if let Some(f) = shown_sessions_frame(&frame, &shown_of(state, machine)) {
                send(f, None);
            }
        }
        // What waits on someone there, for the sessions shown here.
        "pending" | "pendingResolved" => {
            if pending_shows(&frame, &shown_of(state, machine)) {
                publish(None);
            }
        }
        // A move it runs (to or from anywhere), as its own dashboards see it.
        "resync" | "move" => publish(None),
        "transcript" | "transcriptReset" | "stream" | "agents" => {
            if let Some(session) = session_scoped() {
                publish(Some(session));
            }
        }
        // Its runs or tasks changed (a session's background tasks there):
        // the pages showing its sessions refetch, as for this computer's.
        "machines" if forwarded_machines(&frame) => publish(None),
        // Its own plan usage, accounts, notices, the rest of its machines:
        // not this machine's business (yet).
        _ => {}
    }
}

/// The `machines` frames of another computer passed on to this one's
/// dashboards: its runs' and its tasks' (`{t:"machines", what:"runs"}`),
/// nothing else of its Machines.
fn forwarded_machines(frame: &Value) -> bool {
    matches!(
        frame.get("what").and_then(Value::as_str),
        Some("runs" | "tasks")
    )
}

fn cache_sessions(state: &AppState, machine: &str, frame: &Value) {
    state.machines.federation.with(|r| {
        let m = r.entry(machine.to_string()).or_default();
        let id_of = |s: &Value| s.get("id").and_then(Value::as_str).map(str::to_string);
        if frame.get("full").and_then(Value::as_bool).unwrap_or(false) {
            m.loaded = true;
            m.sessions = frame
                .get("sessions")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|s| Some((id_of(s)?, s.clone())))
                        .collect()
                })
                .unwrap_or_default();
            return;
        }
        if let Some(list) = frame.get("upsert").and_then(Value::as_array) {
            for s in list {
                if let Some(id) = id_of(s) {
                    m.sessions.insert(id, s.clone());
                }
            }
        }
        if let Some(list) = frame.get("remove").and_then(Value::as_array) {
            for id in list.iter().filter_map(Value::as_str) {
                m.sessions.remove(id);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sub_frames_carry_only_what_is_set() {
        assert_eq!(
            serde_json::from_str::<Value>(&sub_frame("sub", "s1", None, None)).unwrap(),
            json!({"t": "sub", "session": "s1"})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&sub_frame("sub", "s1", Some("a1"), Some("12"))).unwrap(),
            json!({"t": "sub", "session": "s1", "agent": "a1", "since": "12"})
        );
    }

    /// The polling fallback's view: paired computers only, each session
    /// stamped with its computer, whether a whole list arrived, offline
    /// since when — and the same state always reads the same.
    #[test]
    fn the_snapshot_lists_paired_computers_sessions_stamped() {
        const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        const GONE: &str = "cccccccccccccccccccccccccccccccc";
        let tmp = tempfile::tempdir().unwrap();
        let st = crate::state::Inner::new(
            crate::claude::ClaudeHome::with_base(tmp.path().join("home")),
            crate::state::ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            crate::server::auth::Auth::ephemeral(),
        );
        let mut peer = crate::machines::pairing::peer_record(
            &crate::machines::protocol::MachineInfo {
                id: B.into(),
                name: "DESKTOP-PC".into(),
                os: "windows".into(),
                os_version: None,
                versions: crate::machines::protocol::Versions {
                    app: "0.11.0".into(),
                    protocol: crate::machines::protocol::PROTOCOL,
                    claude: None,
                },
                key: "d".repeat(64),
                shell: None,
            },
            crate::machines::grants::Grants::defaults(),
            None,
        );
        peer.alias = Some("Desk".into());
        st.machines.store_write(|f| f.peers.push(peer));
        assert!(snapshot(&st).is_empty(), "nothing heard yet");

        let tx = mpsc::unbounded_channel().0;
        handle(
            &st,
            B,
            &tx,
            json!({"t": "sessions", "full": true, "seq": 1,
                   "sessions": [{"id": "s2", "live": true}, {"id": "s1"}]}),
        );
        // A computer no longer paired is never listed.
        handle(
            &st,
            GONE,
            &tx,
            json!({"t": "sessions", "full": true, "seq": 1, "sessions": [{"id": "x"}]}),
        );
        st.machines
            .federation
            .with(|r| r.get_mut(B).unwrap().online = true);
        let v = snapshot(&st);
        assert_eq!(
            v,
            vec![json!({
                "machine": B, "name": "Desk", "online": true, "since": null, "loaded": true,
                "sessions": [{"id": "s1", "machine": B}, {"id": "s2", "live": true, "machine": B}],
            })]
        );
        assert_eq!(snapshot(&st), v, "the same state reads the same");

        // A diff, then the link drops: its sessions stay, offline since then.
        handle(
            &st,
            B,
            &tx,
            json!({"t": "sessions", "seq": 2, "upsert": [{"id": "s3"}], "remove": ["s2"]}),
        );
        went_offline(&st, B);
        let v = snapshot(&st);
        assert_eq!(v[0]["online"], false);
        assert!(v[0]["since"].as_i64().is_some());
        let ids: Vec<&str> = v[0]["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|s| s["id"].as_str())
            .collect();
        assert_eq!(ids, ["s1", "s3"]);
    }

    /// Another computer's runs and tasks changes reach this computer's
    /// dashboards as its frames (a remote session's page refetches its
    /// background tasks); nothing else of its Machines does.
    #[test]
    fn its_runs_and_tasks_frames_are_passed_on_and_nothing_else_of_machines() {
        const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let tmp = tempfile::tempdir().unwrap();
        let st = crate::state::Inner::new(
            crate::claude::ClaudeHome::with_base(tmp.path().join("home")),
            crate::state::ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            crate::server::auth::Auth::ephemeral(),
        );
        let mut rx = st.bus.subscribe();
        let tx = mpsc::unbounded_channel().0;
        for what in ["runs", "peers", "tasks", "setup", "roles"] {
            handle(&st, B, &tx, json!({ "t": "machines", "what": what }));
        }
        let mut got = Vec::new();
        while let Ok(e) = rx.try_recv() {
            if let ServerEvent::Remote {
                machine,
                frame,
                session,
            } = &*e
            {
                assert_eq!(machine, B);
                assert_eq!(*session, None, "to every dashboard");
                got.push(frame["what"].as_str().unwrap_or_default().to_string());
            }
        }
        assert_eq!(got, ["runs", "tasks"]);
    }

    const S1: &str = "a0ee0000-1111-4222-8333-444455556661";
    const S2: &str = "a0ee0000-1111-4222-8333-444455556662";

    /// A state with `B` paired, its sessions shown as `choice` says.
    fn paired_with_choice(
        choice: crate::machines::store::SessionsShown,
    ) -> (tempfile::TempDir, AppState) {
        const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let tmp = tempfile::tempdir().unwrap();
        let st = crate::state::Inner::new(
            crate::claude::ClaudeHome::with_base(tmp.path().join("home")),
            crate::state::ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            crate::server::auth::Auth::ephemeral(),
        );
        let mut peer = crate::machines::pairing::peer_record(
            &crate::machines::protocol::MachineInfo {
                id: B.into(),
                name: "DESKTOP-PC".into(),
                os: "windows".into(),
                os_version: None,
                versions: crate::machines::protocol::Versions {
                    app: "0.15.0".into(),
                    protocol: crate::machines::protocol::PROTOCOL,
                    claude: None,
                },
                key: "d".repeat(64),
                shell: None,
            },
            crate::machines::grants::Grants::defaults(),
            None,
        );
        peer.sessions = choice;
        st.machines.store_write(|f| f.peers.push(peer));
        (tmp, st)
    }

    /// The `remote` frames dashboards got, in order.
    fn got(rx: &mut tokio::sync::broadcast::Receiver<std::sync::Arc<ServerEvent>>) -> Vec<Value> {
        let mut out = Vec::new();
        while let Ok(e) = rx.try_recv() {
            if let ServerEvent::Remote { frame, .. } = &*e {
                out.push(frame.clone());
            }
        }
        out
    }

    fn ids(list: &Value) -> Vec<&str> {
        list.as_array()
            .map(|l| l.iter().filter_map(|s| s["id"].as_str()).collect())
            .unwrap_or_default()
    }

    /// A session stopped here leaves every list, count and pending item
    /// dashboards get, while the cache keeps it for the choice's own list;
    /// a diff left with nothing for them is not sent.
    #[test]
    fn a_stopped_session_is_left_out_of_what_dashboards_get() {
        const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let mut choice = crate::machines::store::SessionsShown::default();
        choice.set(S1, false).unwrap();
        let (_tmp, st) = paired_with_choice(choice);
        let mut rx = st.bus.subscribe();
        let tx = mpsc::unbounded_channel().0;

        handle(
            &st,
            B,
            &tx,
            json!({"t": "sessions", "full": true, "seq": 1, "sessions": [
                {"id": S1, "live": true, "lastActivityAt": 2, "title": "Hidden"},
                {"id": S2, "live": true, "lastActivityAt": 1, "title": "Shown"},
            ]}),
        );
        let frames = got(&mut rx);
        assert_eq!(frames.len(), 1);
        assert_eq!(ids(&frames[0]["sessions"]), [S2]);

        // Diffs: one only about the stopped session is not sent; one about
        // both carries the shown one; a removal passes.
        handle(
            &st,
            B,
            &tx,
            json!({"t": "sessions", "seq": 2, "upsert": [{"id": S1, "live": false}]}),
        );
        assert!(got(&mut rx).is_empty());
        handle(
            &st,
            B,
            &tx,
            json!({"t": "sessions", "seq": 3, "upsert": [{"id": S1}, {"id": S2, "live": true, "lastActivityAt": 3}]}),
        );
        let frames = got(&mut rx);
        assert_eq!(ids(&frames[0]["upsert"]), [S2]);
        handle(
            &st,
            B,
            &tx,
            json!({"t": "sessions", "seq": 4, "upsert": [], "remove": [S1]}),
        );
        assert_eq!(got(&mut rx)[0]["remove"], json!([S1]));
        handle(
            &st,
            B,
            &tx,
            json!({"t": "sessions", "seq": 5, "upsert": [
                {"id": S1, "live": true, "lastActivityAt": 2, "title": "Hidden"},
            ]}),
        );
        assert!(got(&mut rx).is_empty());

        // What waits on someone there: only for the shown session.
        for frame in [
            json!({"t": "pending", "item": {"id": "p1", "sessionId": S1}}),
            json!({"t": "pending", "item": {"id": "p2", "sessionId": S2}}),
            json!({"t": "pendingResolved", "id": "p1", "session": S1}),
            json!({"t": "pendingResolved", "id": "p2", "session": S2}),
            json!({"t": "pending", "item": {"id": "p3"}}),
        ] {
            handle(&st, B, &tx, frame);
        }
        let frames = got(&mut rx);
        let seen: Vec<String> = frames
            .iter()
            .map(|f| {
                f.pointer("/item/id")
                    .or_else(|| f.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            })
            .collect();
        assert_eq!(seen, ["p2", "p2", "p3"]);

        // Counts, the first frames of a dashboard, the polling fallback.
        let shown = shown_of(&st, B);
        assert_eq!(st.machines.federation.counts(B, &shown), Some((1, 1)));
        assert_eq!(st.machines.federation.hidden(B, &shown), 1);
        let first = initial_frames(&st);
        let ServerEvent::Remote { frame, .. } = &first[0] else {
            panic!("a remote frame")
        };
        assert_eq!(ids(&frame["sessions"]), [S2]);
        assert_eq!(ids(&snapshot(&st)[0]["sessions"]), [S2]);

        // The choice's own list has both, most recently active first.
        let c = choices(&st, B);
        assert_eq!(c["mode"], "all");
        assert_eq!(c["loaded"], true);
        assert_eq!(ids(&c["sessions"]), [S2, S1]);
        assert_eq!(c["sessions"][1]["synced"], false);
        assert_eq!(c["sessions"][1]["title"], "Hidden");

        // Synced again: every dashboard gets the whole list, then the link's
        // state (a list alone would say it is online).
        st.machines.store_write(|f| {
            f.peer_mut(B).unwrap().sessions.set(S1, true).unwrap();
        });
        republish(&st, B);
        let frames = got(&mut rx);
        assert_eq!(frames.len(), 2);
        let mut both = ids(&frames[0]["sessions"]);
        both.sort();
        assert_eq!(both, [S1, S2]);
        assert_eq!(frames[1]["t"], "offline");
    }

    /// Only the chosen ones: nothing shows until one is chosen, and a new
    /// session there stays out.
    #[test]
    fn only_the_chosen_sessions_show_in_chosen_mode() {
        const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let mut choice = crate::machines::store::SessionsShown {
            mode: crate::machines::store::ShowMode::Chosen,
            ..Default::default()
        };
        choice.set(S2, true).unwrap();
        let (_tmp, st) = paired_with_choice(choice);
        let mut rx = st.bus.subscribe();
        let tx = mpsc::unbounded_channel().0;
        handle(
            &st,
            B,
            &tx,
            json!({"t": "sessions", "full": true, "seq": 1,
                   "sessions": [{"id": S1}, {"id": S2}]}),
        );
        assert_eq!(ids(&got(&mut rx)[0]["sessions"]), [S2]);
        assert_eq!(
            st.machines.federation.hidden(B, &shown_of(&st, B)),
            1,
            "the one not chosen"
        );
        // An unknown computer's frames are untouched (nothing chosen there).
        assert!(shown_of(&st, "cccccccccccccccccccccccccccccccc").shows(S1));
    }

    #[test]
    fn subscriptions_are_counted_per_session() {
        let f = Federation::default();
        f.with(|r| {
            let m = r.entry("m".into()).or_default();
            *m.subs.entry(("s".into(), None)).or_insert(0) += 2;
        });
        assert_eq!(f.with(|r| r["m"].subs[&("s".to_string(), None)]), 2);
    }
}
