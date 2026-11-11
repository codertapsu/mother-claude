//! Leases: which computer a session lives on (docs/PLAN-MACHINES.md §5.7,
//! D1 — one writer per session, across machines).
//!
//! A session's lease names its **home**, the only machine that may run (and
//! so write) it, with an **epoch** every move raises. Leases are kept per
//! session in `<app data>/machines/leases.json` (0600, written atomically,
//! read entry by entry so one bad entry never loses the rest) and exchanged
//! with every paired machine on each hello and on `POST /peer/leases` —
//! accepted only from a machine holding this one's `move` grant for the path
//! it came over: the higher epoch wins, a tie goes to the higher machine id.
//! A session without a lease is home where its files are, as before
//! Machines.
//!
//! The lease is enforced where every owned launch passes
//! (`ControlRegistry::spawn` → `409 session_lives_elsewhere` naming the
//! machine), and early in the routes that would write the session here
//! (continue, message, an account switch) for a clear answer.
//!
//! The **move note** (decision 7) waits here with the lease until the
//! session's next message takes it ([`Leases::take_note`], by `write_user`).
//! An entry may hold only a note: epoch 0 is no lease and is never exchanged.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::claude::model::LivesOn;
use crate::server::error::ApiError;
use crate::state::AppState;

/// `<app data>/machines/leases.json`.
pub const FILE: &str = "leases.json";
/// At most this many leases are sent in one exchange (the newest).
const MAX_WIRE: usize = 5000;

/// One session's lease, as this machine keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lease {
    /// The machine id of its home.
    pub home: String,
    /// Raised by every move; 0: no lease (an entry that only holds a note).
    #[serde(default)]
    pub epoch: u64,
    /// When this home took it (ms).
    #[serde(default)]
    pub since: i64,
    /// The move that made it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub move_id: Option<String>,
    /// The move note, until the session's next message carries it (this
    /// machine's only, never exchanged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A lease as machines exchange it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireLease {
    pub session: String,
    pub home: String,
    pub epoch: u64,
    #[serde(default)]
    pub since: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub move_id: Option<String>,
}

/// Whether lease `(a_epoch, a_home)` replaces `(b_epoch, b_home)`: a higher
/// epoch, or the same one and a higher machine id.
pub fn wins(a_epoch: u64, a_home: &str, b_epoch: u64, b_home: &str) -> bool {
    a_epoch > b_epoch || (a_epoch == b_epoch && a_home > b_home)
}

/// Why a launch was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LeaseRefusal {
    /// The session lives on another computer.
    #[error(
        "This session lives on {name} now: open it there, or bring it back to this computer first."
    )]
    Elsewhere { machine: String, name: String },
    /// A move of it is under way.
    #[error("This session is moving to {name} right now; it goes on there in a moment.")]
    Moving { machine: String, name: String },
}

impl LeaseRefusal {
    /// `409 session_lives_elsewhere` / `409 session_moving`, naming the
    /// machine.
    pub fn api_error(&self) -> ApiError {
        match self {
            Self::Elsewhere { machine, name } => {
                ApiError::conflict("session_lives_elsewhere", self.to_string())
                    .with("machine", machine.clone())
                    .with("name", name.clone())
            }
            Self::Moving { machine, name } => {
                ApiError::conflict("session_moving", self.to_string())
                    .with("machine", machine.clone())
                    .with("name", name.clone())
            }
        }
    }
}

/// Every lease this machine knows.
#[derive(Debug, Default)]
pub struct Leases {
    data_dir: Option<PathBuf>,
    map: Mutex<BTreeMap<String, Lease>>,
    save: Mutex<()>,
}

impl Leases {
    /// Loads `machines/leases.json`; a missing or unreadable file is empty, a
    /// bad entry skipped (with a warning).
    pub fn load(data_dir: Option<&Path>) -> Self {
        let map = data_dir
            .and_then(|d| crate::persist::read_json::<Value>(&path_in(d)))
            .map(|v| parse(&v))
            .unwrap_or_default();
        Self {
            data_dir: data_dir.map(Path::to_path_buf),
            map: Mutex::new(map),
            save: Mutex::new(()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Lease>> {
        self.map.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Changes the map and saves it (one save at a time, in order).
    fn write<R>(&self, f: impl FnOnce(&mut BTreeMap<String, Lease>) -> R) -> R {
        let _g = self.save.lock().unwrap_or_else(|e| e.into_inner());
        let (out, snapshot) = {
            let mut map = self.lock();
            let out = f(&mut map);
            (out, map.clone())
        };
        if let Some(dir) = &self.data_dir {
            if let Err(e) = crate::persist::write_json_private(&path_in(dir), &snapshot) {
                tracing::warn!(error = %e, "machines/leases.json could not be saved");
            }
        }
        out
    }

    pub fn get(&self, session: &str) -> Option<Lease> {
        self.lock().get(session).cloned()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Sets a session's lease here (a move this machine commits, a take
    /// over, an undo), keeping its note when it stays home here.
    pub fn set(&self, session: &str, mut lease: Lease, me: &str) {
        self.write(|m| {
            if lease.note.is_none() && lease.home == me {
                lease.note = m.get(session).and_then(|l| l.note.clone());
            }
            m.insert(session.to_string(), lease);
        });
    }

    /// The leases to send another machine (real ones only, newest first).
    pub fn wire(&self) -> Vec<WireLease> {
        let mut out: Vec<WireLease> = self
            .lock()
            .iter()
            .filter(|(_, l)| l.epoch > 0)
            .map(|(s, l)| WireLease {
                session: s.clone(),
                home: l.home.clone(),
                epoch: l.epoch,
                since: l.since,
                move_id: l.move_id.clone(),
            })
            .collect();
        out.sort_by_key(|w| std::cmp::Reverse(w.since));
        out.truncate(MAX_WIRE);
        out
    }

    /// One session's lease as machines exchange it.
    pub fn wire_of(&self, session: &str) -> Option<WireLease> {
        self.get(session)
            .filter(|l| l.epoch > 0)
            .map(|l| WireLease {
                session: session.to_string(),
                home: l.home,
                epoch: l.epoch,
                since: l.since,
                move_id: l.move_id,
            })
    }

    /// The note waiting for the session's next message.
    pub fn note_of(&self, session: &str) -> Option<String> {
        self.lock().get(session).and_then(|l| l.note.clone())
    }

    /// Keeps `note` for the session's next message here (an entry with no
    /// lease, epoch 0, when it has none).
    pub fn set_note(&self, session: &str, note: String, me: &str) {
        self.write(|m| match m.get_mut(session) {
            Some(l) => l.note = Some(note),
            None => {
                m.insert(
                    session.to_string(),
                    Lease {
                        home: me.to_string(),
                        epoch: 0,
                        since: super::now(),
                        move_id: None,
                        note: Some(note),
                    },
                );
            }
        });
    }

    /// The note, taken: a message carried it (saved at once).
    pub fn take_note(&self, session: &str) -> Option<String> {
        self.note_of(session)?;
        self.write(|m| {
            let note = m.get_mut(session).and_then(|l| l.note.take());
            // An entry that only held the note goes with it.
            if m.get(session).is_some_and(|l| l.epoch == 0) {
                m.remove(session);
            }
            note
        })
    }

    /// Merges leases another machine sent: each one that is well formed,
    /// names a known machine (`known`) and wins over this machine's replaces
    /// it. Returns the sessions whose lease changed.
    pub fn merge(
        &self,
        incoming: &[WireLease],
        known: &dyn Fn(&str) -> bool,
        me: &str,
    ) -> Vec<String> {
        let wanted: Vec<&WireLease> = incoming
            .iter()
            .take(MAX_WIRE)
            .filter(|w| {
                crate::claude::ids::is_session_id(&w.session)
                    && w.epoch > 0
                    && super::ids::is_machine_id(&w.home)
                    && known(&w.home)
                    && w.move_id.as_deref().is_none_or(super::moves::is_move_id)
            })
            .collect();
        if wanted.is_empty() {
            return Vec::new();
        }
        let current = self.lock().clone();
        let changes: Vec<&WireLease> = wanted
            .into_iter()
            .filter(|w| match current.get(&w.session) {
                Some(l) if l.epoch > 0 => wins(w.epoch, &w.home, l.epoch, &l.home),
                _ => true,
            })
            .collect();
        if changes.is_empty() {
            return Vec::new();
        }
        self.write(|m| {
            let mut changed = Vec::new();
            for w in changes {
                // Checked again under the lock: another merge may have won.
                if let Some(l) = m.get(&w.session) {
                    if l.epoch > 0 && !wins(w.epoch, &w.home, l.epoch, &l.home) {
                        continue;
                    }
                }
                let note = m
                    .get(&w.session)
                    .and_then(|l| l.note.clone())
                    .filter(|_| w.home == me);
                m.insert(
                    w.session.clone(),
                    Lease {
                        home: w.home.clone(),
                        epoch: w.epoch,
                        since: w.since,
                        move_id: w.move_id.clone(),
                        note,
                    },
                );
                changed.push(w.session.clone());
            }
            changed
        })
    }
}

fn path_in(data_dir: &Path) -> PathBuf {
    data_dir.join("machines").join(FILE)
}

fn parse(v: &Value) -> BTreeMap<String, Lease> {
    let mut out = BTreeMap::new();
    let Some(obj) = v.as_object() else {
        return out;
    };
    for (session, entry) in obj {
        if !crate::claude::ids::is_session_id(session) {
            tracing::warn!("machines/leases.json: an entry with a malformed id was skipped");
            continue;
        }
        match serde_json::from_value::<Lease>(entry.clone()) {
            Ok(l) if super::ids::is_machine_id(&l.home) => {
                out.insert(session.clone(), l);
            }
            Ok(_) => tracing::warn!("machines/leases.json: a lease naming no machine was skipped"),
            Err(e) => tracing::warn!(error = %e, "machines/leases.json: a lease was skipped"),
        }
    }
    out
}

// ─── against the app ───────────────────────────────────────────────────────────

/// A session's lease as this machine keeps it.
pub fn lease_of(state: &AppState, session: &str) -> Option<Lease> {
    state.machines.leases.get(session)
}

/// The move note waiting for a session's next message here.
pub fn pending_note(state: &AppState, session: &str) -> Option<String> {
    state.machines.leases.note_of(session)
}

/// The lease of a session that lives on another machine, if it does.
pub fn elsewhere(state: &AppState, session: &str) -> Option<Lease> {
    let lease = state.machines.leases.get(session).filter(|l| l.epoch > 0)?;
    let me = super::self_id(state)?;
    (lease.home != me).then_some(lease)
}

/// Whether a session may be launched (resumed in place, or started under
/// its id) here: not while it lives on another machine, nor while a move of
/// it is under way.
pub fn check_launch(state: &AppState, session: &str) -> Result<(), LeaseRefusal> {
    if let Some(to) = state.machines.moves.moving_to(session) {
        return Err(LeaseRefusal::Moving {
            name: super::peer_name(state, &to),
            machine: to,
        });
    }
    if let Some(l) = elsewhere(state, session) {
        return Err(LeaseRefusal::Elsewhere {
            name: super::peer_name(state, &l.home),
            machine: l.home,
        });
    }
    Ok(())
}

/// [`check_launch`] as the API answers it.
pub fn require_here(state: &AppState, session: &str) -> Result<(), ApiError> {
    check_launch(state, session).map_err(|r| r.api_error())
}

/// What the session list shows of a session living elsewhere.
pub fn lives_on(state: &AppState, session: &str) -> Option<LivesOn> {
    if state.machines.leases.is_empty() {
        return None;
    }
    elsewhere(state, session).map(|l| LivesOn {
        name: super::peer_name(state, &l.home),
        machine: l.home,
        since: l.since,
        epoch: l.epoch,
    })
}

/// Merges leases a paired machine sent (its hello, `/peer/leases`), when
/// it holds this machine's `move` grant for the path (`lan`) it came over,
/// and patches the sessions whose lease changed in every list. The caller
/// lets the moves waiting on a lease go on
/// ([`super::moves::on_leases_changed`]).
pub async fn accept(
    state: &AppState,
    from: &str,
    lan: bool,
    incoming: &[WireLease],
) -> Vec<String> {
    if incoming.is_empty() || !state.machines.allows(from, super::grants::Grant::Move, lan) {
        return Vec::new();
    }
    let Some(me) = super::self_id(state) else {
        return Vec::new();
    };
    let known = |m: &str| m == me || state.machines.store_read(|f| f.peer(m).is_some());
    let changed = state.machines.leases.merge(incoming, &known, &me);
    if !changed.is_empty() {
        tracing::info!(from, sessions = changed.len(), "leases changed");
        changed_here(state, &changed).await;
    }
    changed
}

/// Puts the lease of each of `sessions` on its row in every list.
pub async fn changed_here(state: &AppState, sessions: &[String]) {
    for id in sessions {
        let lives = lives_on(state, id);
        crate::server::ws::patch_session(state, id, move |s| s.lives_on = lives).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S1: &str = "0f1e2d3c-4b5a-4968-8776-655443322110";
    const S2: &str = "1f1e2d3c-4b5a-4968-8776-655443322110";
    const A: &str = "0123456789abcdef0123456789abcdef";
    const B: &str = "fedcba9876543210fedcba9876543210";

    fn wire(session: &str, home: &str, epoch: u64) -> WireLease {
        WireLease {
            session: session.into(),
            home: home.into(),
            epoch,
            since: epoch as i64,
            move_id: None,
        }
    }

    #[test]
    fn the_higher_epoch_wins_and_a_tie_goes_to_the_higher_machine() {
        assert!(wins(2, A, 1, B));
        assert!(!wins(1, B, 2, A));
        assert!(wins(3, B, 3, A), "a tie: the higher id");
        assert!(!wins(3, A, 3, B));
        assert!(!wins(3, A, 3, A), "the same lease changes nothing");
    }

    #[test]
    fn a_merge_takes_only_what_wins_and_keeps_a_note_that_stays_home() {
        let l = Leases::load(None);
        let known = |m: &str| m == A || m == B;
        assert_eq!(l.merge(&[wire(S1, B, 1)], &known, A), vec![S1.to_string()]);
        assert_eq!(l.get(S1).unwrap().home, B);
        // Older or equal-and-lower: nothing.
        assert!(l.merge(&[wire(S1, A, 1)], &known, A).is_empty());
        // Unknown machines, malformed ids and no-lease entries are ignored.
        let stranger = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        assert!(l
            .merge(
                &[wire(S1, stranger, 9), wire("../x", A, 9), wire(S2, A, 0)],
                &known,
                A
            )
            .is_empty());
        // A note waits for the session here and survives its lease coming
        // home; a lease that sends it away drops it.
        l.set_note(S2, "moved here".into(), A);
        assert_eq!(l.get(S2).unwrap().epoch, 0);
        assert!(
            l.wire().iter().all(|w| w.session != S2),
            "no lease, not sent"
        );
        assert_eq!(l.merge(&[wire(S2, A, 4)], &known, A).len(), 1);
        assert_eq!(l.note_of(S2).as_deref(), Some("moved here"));
        assert_eq!(l.take_note(S2).as_deref(), Some("moved here"));
        assert_eq!(l.take_note(S2), None, "taken once");
        assert_eq!(l.get(S2).unwrap().epoch, 4, "the lease stays");
        l.set_note(S2, "again".into(), A);
        assert_eq!(l.merge(&[wire(S2, B, 5)], &known, A).len(), 1);
        assert_eq!(l.note_of(S2), None);
    }

    /// Every owned launch passes `ControlRegistry::spawn`: a session that
    /// lives on another computer, or is moving there, is refused there —
    /// before any process starts — with the machine named.
    #[tokio::test]
    async fn spawning_a_session_that_lives_elsewhere_is_refused_before_it_starts() {
        let tmp = tempfile::tempdir().unwrap();
        let st = crate::state::Inner::new(
            crate::claude::ClaudeHome::with_base(tmp.path().join("home")),
            crate::state::ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            crate::server::auth::Auth::ephemeral(),
        );
        let me = super::super::self_id(&st).expect("an id");
        st.machines.leases.set(
            S1,
            Lease {
                home: B.to_string(),
                epoch: 2,
                since: 1,
                move_id: None,
                note: None,
            },
            &me,
        );
        let opts = |resume: &str| crate::claude::control::SpawnOptions {
            cwd: tmp.path().display().to_string(),
            resume: Some(resume.to_string()),
            ..Default::default()
        };
        let err = st.control.spawn(&st, opts(S1)).await.unwrap_err();
        let refusal = err
            .downcast_ref::<LeaseRefusal>()
            .expect("refused by the lease");
        assert!(matches!(refusal, LeaseRefusal::Elsewhere { machine, .. } if machine == B));
        let api = crate::claude::owned::spawn_error(err);
        assert_eq!(api.code, "session_lives_elsewhere");
        assert_eq!(api.extra["machine"], B);
        assert!(!st.control.controls(S1), "nothing started");
        assert!(lives_on(&st, S1).is_some_and(|l| l.machine == B));

        // Home here again (a higher epoch): no refusal from the lease.
        st.machines.leases.set(
            S1,
            Lease {
                home: me.clone(),
                epoch: 3,
                since: 2,
                move_id: None,
                note: None,
            },
            &me,
        );
        assert!(check_launch(&st, S1).is_ok());
        assert!(lives_on(&st, S1).is_none());
        // While it moves away, nothing launches it either.
        assert!(st
            .machines
            .moves
            .begin("mv_0123456789abcdef", S1, B, String::new())
            .is_some());
        assert!(matches!(
            check_launch(&st, S1),
            Err(LeaseRefusal::Moving { machine, .. }) if machine == B
        ));
        let err = st.control.spawn(&st, opts(S1)).await.unwrap_err();
        assert_eq!(
            crate::claude::owned::spawn_error(err).code,
            "session_moving"
        );
    }

    #[test]
    fn the_file_is_read_entry_by_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            serde_json::json!({
                S1: {"home": A, "epoch": 3, "since": 5},
                S2: {"home": "nope", "epoch": 1},
                "../x": {"home": A, "epoch": 1},
            })
            .to_string(),
        )
        .unwrap();
        let l = Leases::load(Some(dir.path()));
        assert_eq!(l.get(S1).unwrap().epoch, 3);
        assert!(l.get(S2).is_none());
        l.set(
            S2,
            Lease {
                home: B.to_string(),
                epoch: 1,
                since: 1,
                move_id: None,
                note: None,
            },
            A,
        );
        let again = Leases::load(Some(dir.path()));
        assert_eq!(again.get(S2).unwrap().home, B);
    }
}
