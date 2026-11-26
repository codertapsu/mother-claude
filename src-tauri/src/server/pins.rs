//! Pinned sessions: the ones someone keeps at the top of the Sessions page
//! (its *Pinned* group), the same on the desktop and on every paired device.
//!
//! Mother Claude's own: `<app data dir>/pins.json` (0600, atomic), read
//! entry by entry (one bad entry is skipped, never the file); nothing in a
//! config directory. A pin names a session by its id and, for another
//! computer's session, that computer (`machine`), and keeps when it was
//! pinned: the group shows them in that order. Routes: `routes::pins`;
//! every change goes out as `{t:"pins"}`.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::claude::ids;

/// At most this many pins: past it, the one pinned longest ago goes.
pub const MAX_PINS: usize = 100;
const FILE: &str = "pins.json";

/// One pinned session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pin {
    /// The session's id (a Claude Code session id).
    pub id: String,
    /// The other computer it runs on (its id), for another computer's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    /// When it was pinned (epoch ms).
    #[serde(default)]
    pub at: i64,
}

impl Pin {
    fn is(&self, id: &str, machine: Option<&str>) -> bool {
        self.id == id && self.machine.as_deref() == machine
    }
}

/// Whether `id` (on `machine`) can be pinned: a session id, and a machine id
/// as Machines hands them out.
pub fn valid(id: &str, machine: Option<&str>) -> bool {
    ids::is_session_id(id) && machine.is_none_or(ids::is_simple_id)
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct OnDisk {
    pins: Vec<Value>,
}

/// The pins (AppState: `state.pins`).
#[derive(Debug)]
pub struct Pins {
    path: Option<PathBuf>,
    list: Mutex<Vec<Pin>>,
}

impl Pins {
    /// Load from the app data dir (`None`: memory only, for tests).
    pub fn load(data_dir: Option<&Path>) -> Self {
        let path = data_dir.map(|d| d.join(FILE));
        let mut list: Vec<Pin> = Vec::new();
        if let Some(saved) = path
            .as_deref()
            .and_then(crate::persist::read_json::<OnDisk>)
        {
            for v in saved.pins {
                match serde_json::from_value::<Pin>(v) {
                    Ok(p) if valid(&p.id, p.machine.as_deref()) => {
                        if !list.iter().any(|x| x.is(&p.id, p.machine.as_deref())) {
                            list.push(p);
                        }
                    }
                    _ => tracing::debug!("skipped a pin that is not valid"),
                }
            }
        }
        list.sort_by_key(|p| p.at);
        let over = list.len().saturating_sub(MAX_PINS);
        list.drain(..over);
        Self {
            path,
            list: Mutex::new(list),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Pin>> {
        self.list.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn save(&self, list: &[Pin]) {
        let Some(path) = &self.path else {
            return;
        };
        let on_disk = serde_json::json!({ "pins": list });
        if let Err(e) = crate::persist::write_json_private(path, &on_disk) {
            tracing::warn!(error = %e, "could not save the pinned sessions");
        }
    }

    /// The pins, in the order they were pinned.
    pub fn list(&self) -> Vec<Pin> {
        self.lock().clone()
    }

    /// Whether `id` (on `machine`) is pinned.
    pub fn is_pinned(&self, id: &str, machine: Option<&str>) -> bool {
        self.lock().iter().any(|p| p.is(id, machine))
    }

    /// Pin `id` (on `machine`) at `now`; one already pinned keeps its place.
    /// Whether anything changed.
    pub fn pin(&self, id: &str, machine: Option<&str>, now: i64) -> bool {
        let mut list = self.lock();
        if list.iter().any(|p| p.is(id, machine)) {
            return false;
        }
        list.push(Pin {
            id: id.to_string(),
            machine: machine.map(str::to_string),
            at: now,
        });
        let over = list.len().saturating_sub(MAX_PINS);
        list.drain(..over);
        self.save(&list);
        true
    }

    /// Unpin `id` (on `machine`). Whether anything changed.
    pub fn unpin(&self, id: &str, machine: Option<&str>) -> bool {
        let mut list = self.lock();
        let before = list.len();
        list.retain(|p| !p.is(id, machine));
        if list.len() == before {
            return false;
        }
        self.save(&list);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "0b9f3c1e-2d4a-4f6b-8c7d-9e0f1a2b3c4d";
    const B: &str = "1c2d3e4f-5a6b-4c7d-8e9f-0a1b2c3d4e5f";
    const MAC: &str = "6c229c70ddb9ad261a5e15885f45f62a";

    #[test]
    fn pins_keep_their_order_and_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let pins = Pins::load(Some(dir.path()));
        assert!(pins.pin(A, None, 10));
        assert!(pins.pin(B, Some(MAC), 20));
        assert!(!pins.pin(A, None, 30), "already pinned: keeps its place");
        // Another computer's copy of the same id is its own pin.
        assert!(pins.pin(A, Some(MAC), 40));
        assert!(pins.is_pinned(B, Some(MAC)) && !pins.is_pinned(B, None));

        let again = Pins::load(Some(dir.path()));
        let ids: Vec<(String, Option<String>, i64)> = again
            .list()
            .into_iter()
            .map(|p| (p.id, p.machine, p.at))
            .collect();
        assert_eq!(
            ids,
            vec![
                (A.into(), None, 10),
                (B.into(), Some(MAC.into()), 20),
                (A.into(), Some(MAC.into()), 40),
            ]
        );
        assert!(again.unpin(B, Some(MAC)));
        assert!(!again.unpin(B, Some(MAC)), "gone already");
        assert_eq!(Pins::load(Some(dir.path())).list().len(), 2);
    }

    #[test]
    fn only_session_ids_are_pinned_and_a_bad_entry_is_skipped() {
        assert!(valid(A, None) && valid(A, Some(MAC)));
        assert!(!valid("s-auth", None));
        assert!(!valid("../x", None));
        assert!(!valid(A, Some("../x")));
        assert!(!valid(&A.to_uppercase(), None));

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(FILE),
            format!(r#"{{"pins":[{{"id":"{A}","at":5}},{{"id":"nope"}},7,{{"id":"{B}","at":3,"extra":1}}]}}"#),
        )
        .unwrap();
        let pins = Pins::load(Some(dir.path()));
        let ids: Vec<String> = pins.list().into_iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![B.to_string(), A.to_string()], "by when pinned");
    }

    #[test]
    fn past_the_cap_the_oldest_pin_goes() {
        let pins = Pins::load(None);
        for i in 0..=MAX_PINS {
            let id = format!("00000000-0000-4000-8000-{i:012}");
            assert!(pins.pin(&id, None, i as i64));
        }
        let list = pins.list();
        assert_eq!(list.len(), MAX_PINS);
        assert_eq!(list[0].at, 1, "the first pinned went");
    }
}
