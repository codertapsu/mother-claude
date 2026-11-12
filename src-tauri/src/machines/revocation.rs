//! *Revoke everywhere* (docs/PLAN-MACHINES.md §5.4, §10, Phase 5): a lost or
//! stolen computer is cut from all your computers from any one of them.
//!
//! * On the computer where you press it, the computer is revoked at once
//!   (its key forgotten, its connections cut: [`super::revoke_with`]) and
//!   every other paired computer is asked to do the same (`POST
//!   /peer/revoke`). The request is kept in `machines.json`
//!   (`revocations`) until each one took it: an offline computer is told
//!   when it is next heard (its hello, either way), a computer too old to
//!   take it when it has updated.
//! * A computer asked by a paired one applies it at once — it only ever
//!   removes trust — audits it and shows a desktop notice ("MacBook revoked
//!   DESKTOP-X on all your computers"). It never adds anything: a computer
//!   it doesn't know is left unknown, and the request can name neither the
//!   computer that asks nor the one asked (a third computer only).
//! * Nothing is passed on: the computer that asks asks every one itself.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{ids, transport};
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Where another computer asks this one to revoke a third.
pub const PEER_PATH: &str = "/peer/revoke";
/// A revocation every computer took is listed this long (ms).
const KEEP_DONE_MS: i64 = 7 * 24 * 3600 * 1000;
/// Revocations kept at most (the oldest done ones go first).
const MAX_KEPT: usize = 32;
/// How long one computer is given to take it right away.
const DELIVER_TIMEOUT: Duration = Duration::from_secs(6);

/// A computer revoked here and on the others.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Revocation {
    /// The computer revoked: its id and key, and its name here then.
    pub machine: String,
    pub key: String,
    #[serde(default)]
    pub name: String,
    pub at: i64,
    /// The paired computers still to take it.
    #[serde(default)]
    pub pending: Vec<String>,
    /// Those that took it (it is gone there, or they never knew it).
    #[serde(default)]
    pub done: Vec<String>,
    /// Among the pending, those whose Mother Claude can't take it until it
    /// updates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub too_old: Vec<String>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl Revocation {
    /// Every id and key well formed (`machines.json` is read field by field).
    pub fn well_formed(&self) -> bool {
        ids::is_machine_id(&self.machine)
            && ids::is_key(&self.key)
            && self
                .pending
                .iter()
                .chain(&self.done)
                .chain(&self.too_old)
                .all(|m| ids::is_machine_id(m))
    }
}

/// `POST /peer/revoke`: the computer to revoke, by id and key (either
/// matching is enough: it only removes trust).
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeRequest {
    pub machine: String,
    pub key: String,
    /// Its name on the asking computer (shown when this one doesn't name
    /// it).
    #[serde(default)]
    pub name: Option<String>,
}

// ─── on the computer where it is pressed ─────────────────────────────────────

/// `POST /api/machines/{id}/revoke-everywhere`: revokes `machine` here, then
/// asks every other paired computer to (each given a few seconds now; the
/// rest are told when they are next heard). `Err` when it isn't paired.
pub async fn revoke_everywhere(state: &AppState, machine: &str) -> ApiResult<()> {
    let target = state
        .machines
        .store_read(|f| f.peer(machine).cloned())
        .ok_or_else(|| ApiError::not_found("machines_unknown", "Not a paired computer."))?;
    let others: Vec<String> = state.machines.store_read(|f| {
        f.peers
            .iter()
            .filter(|p| p.id != target.id)
            .map(|p| p.id.clone())
            .collect()
    });
    let name = target.display_name().to_string();
    super::revoke_with(
        state,
        machine,
        Some(json!({ "everywhere": true, "asked": others })),
    );
    let rev = Revocation {
        machine: target.id.clone(),
        key: target.key.clone(),
        name,
        at: super::now(),
        pending: others.clone(),
        done: Vec::new(),
        too_old: Vec::new(),
        extra: BTreeMap::new(),
    };
    state.machines.store_write(|f| {
        f.revocations
            .retain(|r| r.machine != rev.machine && r.key != rev.key);
        f.revocations.push(rev);
        prune(f);
    });
    let sends = others.iter().map(|p| deliver_to(state, p));
    futures_util::future::join_all(sends).await;
    super::publish(state, "peers");
    Ok(())
}

/// What became of one request.
enum Outcome {
    /// It took it (or never knew the computer).
    Done,
    /// Its Mother Claude can't take it yet (it has no such route).
    TooOld,
    /// It refused for good (it no longer counts this computer as paired):
    /// it is not asked again.
    Refused,
    /// Not reached: asked again when it is next heard.
    Later,
}

/// Tells `peer` every revocation it has not taken yet: on a hello either
/// way, and right after *Revoke everywhere*.
pub async fn deliver_to(state: &AppState, peer: &str) {
    let due: Vec<Revocation> = state.machines.store_read(|f| {
        f.revocations
            .iter()
            .filter(|r| r.pending.iter().any(|p| p == peer))
            .cloned()
            .collect()
    });
    if due.is_empty() {
        return;
    }
    let mut changed = false;
    for r in due {
        let body = json!({ "machine": r.machine, "key": r.key, "name": r.name });
        let answer = tokio::time::timeout(
            DELIVER_TIMEOUT,
            transport::request(state, peer, "POST", PEER_PATH, Some(body), DELIVER_TIMEOUT),
        )
        .await;
        let outcome = match answer {
            Ok(Ok((s, _))) if (200..300).contains(&s) => Outcome::Done,
            Ok(Ok((404, _))) => Outcome::TooOld,
            Ok(Ok((403 | 422, _))) => Outcome::Refused,
            _ => Outcome::Later,
        };
        changed |= state.machines.store_write(|f| {
            let Some(x) = f
                .revocations
                .iter_mut()
                .find(|x| x.machine == r.machine && x.key == r.key)
            else {
                return false;
            };
            match outcome {
                Outcome::Done | Outcome::Refused => {
                    x.pending.retain(|p| p != peer);
                    x.too_old.retain(|p| p != peer);
                    if matches!(outcome, Outcome::Done) && !x.done.iter().any(|d| d == peer) {
                        x.done.push(peer.to_string());
                    }
                    true
                }
                Outcome::TooOld if !x.too_old.iter().any(|p| p == peer) => {
                    x.too_old.push(peer.to_string());
                    true
                }
                _ => false,
            }
        });
        if matches!(outcome, Outcome::Done) {
            tracing::info!(peer, revoked = %r.machine, "a revocation was taken");
        }
    }
    if changed {
        super::publish(state, "peers");
    }
}

/// Whether some revocation still waits for `peer` (cheap: checked on every
/// hello).
pub fn pending_for(state: &AppState, peer: &str) -> bool {
    state.machines.store_read(|f| {
        f.revocations
            .iter()
            .any(|r| r.pending.iter().any(|p| p == peer))
    })
}

/// `peer` is no longer paired here: nothing waits for it any more.
pub fn forget_peer(f: &mut super::store::MachinesFile, peer: &str) {
    for r in &mut f.revocations {
        r.pending.retain(|p| p != peer);
        r.too_old.retain(|p| p != peer);
    }
    prune(f);
}

/// Drops the revocations every computer took a while ago, and the oldest
/// done ones past [`MAX_KEPT`].
fn prune(f: &mut super::store::MachinesFile) {
    let now = super::now();
    f.revocations
        .retain(|r| !r.pending.is_empty() || now - r.at < KEEP_DONE_MS);
    while f.revocations.len() > MAX_KEPT {
        match f.revocations.iter().position(|r| r.pending.is_empty()) {
            Some(i) => f.revocations.remove(i),
            None => f.revocations.remove(0),
        };
    }
}

/// `GET /api/machines` `revocations`: newest first, each with the
/// computers that took it and those still to be told (online or not, too
/// old or not).
pub fn view(state: &AppState) -> Vec<Value> {
    let (list, names): (Vec<Revocation>, HashMap<String, String>) =
        state.machines.store_read(|f| {
            (
                f.revocations.clone(),
                f.peers
                    .iter()
                    .map(|p| (p.id.clone(), p.display_name().to_string()))
                    .collect(),
            )
        });
    let name = |id: &str| {
        names
            .get(id)
            .cloned()
            .unwrap_or_else(|| id.chars().take(8).collect())
    };
    let mut out: Vec<(i64, Value)> = list
        .iter()
        .map(|r| {
            let pending: Vec<Value> = r
                .pending
                .iter()
                .map(|p| {
                    json!({
                        "id": p,
                        "name": name(p),
                        "online": state.machines.link(p).online,
                        "tooOld": r.too_old.contains(p),
                    })
                })
                .collect();
            let done: Vec<Value> = r
                .done
                .iter()
                .map(|p| json!({ "id": p, "name": name(p) }))
                .collect();
            (
                r.at,
                json!({
                    "machine": r.machine,
                    "name": r.name,
                    "key": ids::fingerprint(&r.key),
                    "at": r.at,
                    "pending": pending,
                    "done": done,
                }),
            )
        })
        .collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.0));
    out.into_iter().map(|(_, v)| v).collect()
}

// ─── on a computer asked to revoke a third ───────────────────────────────────

/// `POST /peer/revoke` from the paired computer `from`: revokes the computer
/// named — by its id or its key — when this computer has it paired; refuses
/// a request naming this computer or the one asking; adds nothing.
pub fn receive(state: &AppState, from: &str, req: RevokeRequest) -> ApiResult<Value> {
    if !ids::is_machine_id(&req.machine) || !ids::is_key(&req.key) {
        return Err(ApiError::bad_request("Malformed request."));
    }
    let own = super::identity(state).map_err(ApiError::internal)?;
    if req.machine == own.id || req.key == own.key_hex() {
        return Err(ApiError::unprocessable(
            "revoke_self",
            "A computer can't be asked to revoke itself.",
        ));
    }
    let asker_key = state
        .machines
        .store_read(|f| f.peer(from).map(|p| p.key.clone()));
    if req.machine == from || asker_key.as_deref() == Some(req.key.as_str()) {
        return Err(ApiError::unprocessable(
            "revoke_asker",
            "A computer asks the others to revoke a third computer, never itself.",
        ));
    }
    let targets: Vec<String> = state.machines.store_read(|f| {
        f.peers
            .iter()
            .filter(|p| p.id == req.machine || p.key == req.key)
            .map(|p| p.id.clone())
            .collect()
    });
    if targets.is_empty() {
        return Ok(json!({ "revoked": false }));
    }
    let by = super::peer_name(state, from);
    for t in targets {
        let name = super::peer_name(state, &t);
        super::revoke_with(
            state,
            &t,
            Some(json!({ "everywhere": true, "by": from, "byName": by })),
        );
        crate::server::ws::publish_desktop_notice(
            state,
            crate::claude::model::NoticeLevel::Warning,
            format!("{by} revoked {name} on all your computers."),
        );
    }
    Ok(json!({ "revoked": true }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machines::store::MachinesFile;

    fn rev(machine: char, pending: &[char], at: i64) -> Revocation {
        let id = |c: char| c.to_string().repeat(32);
        Revocation {
            machine: id(machine),
            key: machine.to_string().repeat(64),
            name: "X".into(),
            at,
            pending: pending.iter().map(|c| id(*c)).collect(),
            done: Vec::new(),
            too_old: Vec::new(),
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn revocations_are_read_one_by_one() {
        let good = serde_json::to_value(rev('a', &['b'], 5)).unwrap();
        let f = MachinesFile::from_value(&json!({
            "revocations": [good, {"machine": "nope", "key": "x", "at": 1}, {"oops": true}]
        }));
        assert_eq!(f.revocations.len(), 1);
        assert_eq!(f.revocations[0].pending, ["b".repeat(32)]);
        let saved = serde_json::to_value(&f).unwrap();
        assert_eq!(MachinesFile::from_value(&saved), f);
        assert!(serde_json::to_value(MachinesFile::default())
            .unwrap()
            .get("revocations")
            .is_none());
    }

    #[test]
    fn a_revocation_waits_until_every_computer_took_it() {
        let now = crate::machines::now();
        let mut f = MachinesFile {
            revocations: vec![
                rev('a', &['b', 'c'], now - KEEP_DONE_MS * 2),
                rev('d', &[], now - KEEP_DONE_MS * 2),
                rev('e', &[], now),
            ],
            ..MachinesFile::default()
        };
        prune(&mut f);
        let left: Vec<char> = f
            .revocations
            .iter()
            .map(|r| r.machine.chars().next().unwrap())
            .collect();
        assert_eq!(left, ['a', 'e'], "an old one still pending stays");
        forget_peer(&mut f, &"b".repeat(32));
        assert_eq!(f.revocations[0].pending, ["c".repeat(32)]);
        forget_peer(&mut f, &"c".repeat(32));
        prune(&mut f);
        assert_eq!(f.revocations.len(), 1, "nothing waits for the old one");
    }
}
