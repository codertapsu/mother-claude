//! Seeing other machines' sessions (docs/PLAN-MACHINES.md §5.6, D2).
//!
//! For each paired machine this one keeps a WebSocket to the other's own
//! `/ws` over the peer link (its `view` grant decides whether it is
//! accepted), and republishes what arrives on the local bus as
//! `{t:"remote", machine, frame}`:
//!
//! * its session list (`sessions` full and diffs) — cached here, so a
//!   dashboard that connects later gets it at once — its pending items and
//!   their resolutions: to everyone;
//! * a session's transcript, stream and agents: only to the clients
//!   subscribed to that machine's session (`sub` with `machine`); the
//!   subscriptions are counted here and forwarded to the other machine.
//!
//! When the link drops the machine is marked offline (`{t:"offline", since}`)
//! and its sessions stay listed, greyed, until it is back. Nothing is copied
//! to disk: seeing is remote.
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

    /// How many of `machine`'s sessions are known, and how many run.
    pub fn counts(&self, machine: &str) -> Option<(usize, usize)> {
        self.with(|r| {
            r.get(machine).map(|m| {
                let running = m
                    .sessions
                    .values()
                    .filter(|s| s.get("live").and_then(Value::as_bool).unwrap_or(false))
                    .count();
                (m.sessions.len(), running)
            })
        })
    }
}

/// The frames a dashboard gets when it connects: every other machine's
/// sessions as last heard, then whether it is online.
pub fn initial_frames(state: &AppState) -> Vec<ServerEvent> {
    state.machines.federation.with(|r| {
        let mut out = Vec::new();
        for (machine, m) in r.iter() {
            out.push(ServerEvent::Remote {
                machine: machine.clone(),
                frame: json!({
                    "t": "sessions",
                    "full": true,
                    "sessions": m.sessions.values().collect::<Vec<_>>(),
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
    state.machines.federation.with(|r| {
        let mut ids: Vec<&String> = r.keys().filter(|id| names.contains_key(*id)).collect();
        ids.sort();
        ids.into_iter()
            .map(|id| {
                let m = &r[id];
                let sessions: Vec<Value> = m
                    .sessions
                    .values()
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
    let publish = |session: Option<String>| {
        state.broadcast(ServerEvent::Remote {
            machine: machine.to_string(),
            frame: frame.clone(),
            session,
        });
    };
    match t {
        "ping" => {
            let _ = tx.send(json!({ "t": "pong" }).to_string());
        }
        "sessions" => {
            cache_sessions(state, machine, &frame);
            publish(None);
        }
        // A move it runs (to or from anywhere), as its own dashboards see it.
        "pending" | "pendingResolved" | "resync" | "move" => publish(None),
        "transcript" | "transcriptReset" | "stream" | "agents" => {
            if let Some(session) = session_scoped() {
                publish(Some(session));
            }
        }
        // Its own plan usage, accounts, notices, machines: not this
        // machine's business (yet).
        _ => {}
    }
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
