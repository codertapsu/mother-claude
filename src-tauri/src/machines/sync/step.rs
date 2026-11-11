//! One step of code sync (docs/PLAN-MACHINES.md §5.15 "One step"), run by
//! the computer whose checkout changed, and resolving a conflict.
//!
//! A step holds the pair's lock here; the other computer's half
//! (`peer`) takes its own lock without waiting (a `409 sync_busy` makes
//! this side try again later), so two computers stepping at once never
//! wait on each other. When the other computer is the one that changed,
//! this side lets go of its lock and asks it to send (`push`).
//!
//! The decision, from the two computers' states and their last synced ones:
//!
//! | here | there | what happens |
//! |---|---|---|
//! | the same state | the same state | nothing travels: both record it |
//! | unchanged | unchanged | in sync |
//! | changed | unchanged | this side's snapshot lands there |
//! | unchanged | changed | the other computer sends |
//! | changed | changed | a conflict: paused on both |
//!
//! The first step (no synced state yet) sends this side's state when the
//! other checkout is clean and its branch is behind or the same, else asks
//! the other computer to send when this one is clean and behind, else it is
//! a conflict.

use std::path::{Path, PathBuf};
use std::sync::Weak;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use super::{
    audit, move_grants, move_running_in, name_of, set_status, update, Conflict, Direction, KeptRef,
    Last, Pair, Reason, Status, Trigger, CHAIN_EVERY, KEEP_STEPS,
};
use crate::machines::chunks::{self, PutError};
use crate::machines::code::{
    self, BranchRelation, Changes, CodeError, GitRunner, SnapshotMeta, TreeState,
};
use crate::machines::moves::{self, ignored};
use crate::machines::{self as m};
use crate::server::error::{ApiError, ApiResult};
use crate::state::{AppState, Inner};

/// How long *Sync now* waits for a step already under way.
const LOCK_WAIT: Duration = Duration::from_secs(600);
/// How long the other computer may take to read its checkout.
const STATE_TIMEOUT: Duration = Duration::from_secs(90);
/// How long a landing there may take (git on a large tree).
const LAND_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// A pause a watcher-started step met is not tried again by the watcher
/// for this long (the minute's check still does).
const PAUSE_QUIET: Duration = Duration::from_secs(30);
/// Who acts for a step: the desktop that turned the sync on.
pub const ACTING: &str = "desktop";

/// What a step did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Not on here.
    Off,
    /// The other computer has not confirmed it.
    Waiting,
    /// Neither checkout changed.
    Unchanged,
    /// This side's state landed there.
    Sent,
    /// The other computer's state landed here (it was asked to send).
    Received,
    /// Both had made the same changes.
    Agreed,
    /// The other computer changed and this one did not: it sends.
    TheyShouldSend,
    /// Both changed.
    Conflict,
    /// Waiting for something.
    Paused(Reason),
    /// A step ran already (here or there): it is tried again.
    Busy,
}

impl Outcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Outcome::Off => "off",
            Outcome::Waiting => "waiting",
            Outcome::Unchanged => "unchanged",
            Outcome::Sent => "sent",
            Outcome::Received => "received",
            Outcome::Agreed => "agreed",
            Outcome::TheyShouldSend => "theyShouldSend",
            Outcome::Conflict => "conflict",
            Outcome::Paused(_) => "paused",
            Outcome::Busy => "busy",
        }
    }
}

pub(crate) fn git(state: &AppState) -> GitRunner {
    state.machines.moves.git()
}

/// "this Mac" — how a sentence names this computer.
fn this(state: &AppState) -> String {
    m::this_name(state)
}

/// The other computer's state, as `GET /peer/sync/{id}` answers it.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct There {
    /// It has the pair and it is on there.
    pub on: bool,
    pub state: Option<TreeState>,
    /// `git status` shows nothing there.
    pub clean: bool,
    /// Its checkout is not its last synced state.
    pub changed: bool,
    /// Its last synced state's step (none: never synced).
    pub last_step: Option<u64>,
    pub summary: Changes,
    pub haves: Vec<String>,
    /// Its tip of the branch this side asked about.
    pub tip: Option<String>,
    pub move_running: bool,
    /// It waits for a conflict to be resolved.
    pub conflict: bool,
    /// It lost a resolution: its branch may be reset.
    pub reset_ok: bool,
    /// Its checkout could not be read (said in words).
    pub error: Option<String>,
}

/// Why a call to the other computer did not give an answer.
#[derive(Debug)]
pub(crate) enum Peer {
    Status(Status),
    Busy,
    /// It does not know the pair (any more).
    Unknown,
    /// Its answer, refused: code and message.
    Refused(u16, String, String),
}

/// A call to the other computer about a pair, its refusals said as a
/// pair's status.
pub(crate) async fn call(
    state: &AppState,
    p: &Pair,
    method: &str,
    path: &str,
    body: Option<&Value>,
    timeout: Duration,
) -> Result<Value, Peer> {
    let name = m::peer_name(state, &p.machine);
    let r = moves::call(state, &p.machine, method, path, body, ACTING, timeout).await;
    let (status, v) = match r {
        Ok(x) => x,
        Err(e) => {
            return Err(Peer::Status(Status::paused(
                Reason::Offline,
                format!("{name} can't be reached: {e}"),
            )))
        }
    };
    if (200..300).contains(&status) {
        return Ok(v);
    }
    let code = v
        .pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or("refused")
        .to_string();
    let message = v
        .pointer("/error/message")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("{name} refused ({status})."));
    Err(match (status, code.as_str()) {
        (403, "machines_off") => Peer::Status(Status::paused(
            Reason::MachinesOff,
            format!("Machines is off on {name}."),
        )),
        (403, "grant_missing") => Peer::Status(Status::paused(
            Reason::NoGrant,
            format!(
                "{name} has not given {} the Move permission: code sync writes into each other's project folder.",
                this(state)
            ),
        )),
        (404, "sync_unknown") => Peer::Unknown,
        (409, "sync_busy") => Peer::Busy,
        (404, _) => Peer::Status(Status::paused(
            Reason::Error,
            format!("Update Mother Claude on {name}: it does not know code sync yet."),
        )),
        _ => Peer::Refused(status, code, message),
    })
}

/// Sets the status a peer's refusal says; answers the step's outcome.
fn pause_for(state: &AppState, p: &Pair, why: Peer, trigger: Trigger) -> Outcome {
    match why {
        Peer::Status(s) => {
            let reason = s.reason().unwrap_or(Reason::Error);
            pause(state, p, s, trigger);
            Outcome::Paused(reason)
        }
        Peer::Busy => {
            state.machines.sync.ask_again(&p.id);
            Outcome::Busy
        }
        Peer::Unknown => {
            turned_off_there(state, p, None);
            Outcome::Off
        }
        Peer::Refused(_, _, message) => {
            let name = m::peer_name(state, &p.machine);
            pause(
                state,
                p,
                Status::paused(Reason::Error, format!("{name}: {message}")),
                trigger,
            );
            Outcome::Paused(Reason::Error)
        }
    }
}

fn pause(state: &AppState, p: &Pair, s: Status, trigger: Trigger) {
    if trigger != Trigger::Now {
        state
            .machines
            .sync
            .paused_at
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(p.id.clone(), std::time::Instant::now());
    }
    set_status(state, &p.id, s);
}

/// The other computer turned it off, or does not know it.
pub(crate) fn turned_off_there(state: &AppState, p: &Pair, detail: Option<String>) {
    let name = m::peer_name(state, &p.machine);
    update(state, &p.id, |q| {
        q.peer_on = false;
        q.conflict = None;
        q.status = Status::Off {
            detail: Some(detail.unwrap_or_else(|| format!("Turned off on {name}."))),
        };
    });
    super::watch::refresh(state);
}

/// What a checkout that could not be read says.
pub(crate) fn unreadable(state: &AppState, folder: &str, e: &CodeError) -> String {
    let name = name_of(folder);
    let here = this(state);
    match e {
        CodeError::InProgress(what) => {
            format!("A git operation is under way in {name} on {here} ({what}): sync waits until it ends.")
        }
        CodeError::Unmerged => {
            format!("{name} has a merge half done on {here}: sync waits until it is finished.")
        }
        CodeError::NotARepository(_) => {
            format!("{folder} is not a git repository on {here} any more.")
        }
        other => format!("{name} could not be read on {here}: {other}"),
    }
}

// ─── running one ───────────────────────────────────────────────────────────────

/// Runs one step of pair `id` (and, when the other computer is the one
/// that changed, asks it to send).
pub async fn run(state: &AppState, id: &str, trigger: Trigger) -> Outcome {
    let outcome = locked(state, id, trigger, false).await;
    match outcome {
        Outcome::TheyShouldSend => ask_to_send(state, id, trigger).await,
        o => o,
    }
}

/// [`run`] for the watcher and the minute's check: a change noticed (or a
/// step met busy) while it ran is looked at again, a few times at most.
pub(crate) async fn run_auto(state: &AppState, id: &str, trigger: Trigger) {
    run(state, id, trigger).await;
    for _ in 0..3 {
        if !state.machines.sync.take_again(id) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1500)).await;
        run(state, id, Trigger::Watch).await;
    }
}

/// The other computer sends its state (it changed, this one did not).
async fn ask_to_send(state: &AppState, id: &str, trigger: Trigger) -> Outcome {
    let Some(p) = state.machines.sync.pair(id) else {
        return Outcome::Off;
    };
    set_status(state, id, Status::Syncing { since: m::now() });
    match call(
        state,
        &p,
        "POST",
        &format!("/peer/sync/{id}/push"),
        Some(&json!({})),
        LAND_TIMEOUT,
    )
    .await
    {
        Ok(v) => {
            let theirs = v.get("outcome").and_then(Value::as_str).unwrap_or("");
            match theirs {
                "sent" => Outcome::Received,
                "conflict" => Outcome::Conflict,
                "agreed" => Outcome::Agreed,
                "unchanged" => {
                    set_status(state, id, Status::InSync { at: m::now() });
                    Outcome::Unchanged
                }
                _ => {
                    // Its own status says why (it told this side, or the next
                    // check will).
                    if let Some(p) = state.machines.sync.pair(id) {
                        if matches!(p.status, Status::Syncing { .. }) {
                            let name = m::peer_name(state, &p.machine);
                            let detail = v
                                .pointer("/status/detail")
                                .and_then(Value::as_str)
                                .map(|d| format!("{name}: {d}"))
                                .unwrap_or_else(|| format!("{name} did not send its changes."));
                            pause(state, &p, Status::paused(Reason::Error, detail), trigger);
                        }
                    }
                    Outcome::Paused(Reason::Error)
                }
            }
        }
        Err(why) => pause_for(state, &p, why, trigger),
    }
}

/// The step itself, under the pair's lock. `second`: the other computer
/// asked this one to send on a first sync because it could not — no asking
/// back.
pub(crate) async fn locked(state: &AppState, id: &str, trigger: Trigger, second: bool) -> Outcome {
    let lock = state.machines.sync.lock_of(id);
    let _guard = match trigger {
        Trigger::Now => match tokio::time::timeout(LOCK_WAIT, lock.lock_owned()).await {
            Ok(g) => g,
            Err(_) => return Outcome::Busy,
        },
        _ => match lock.try_lock_owned() {
            Ok(g) => g,
            Err(_) => {
                state.machines.sync.ask_again(id);
                return Outcome::Busy;
            }
        },
    };
    let Some(p) = state.machines.sync.pair(id) else {
        return Outcome::Off;
    };
    if !p.here_on || matches!(p.status, Status::Off { .. }) && !p.peer_on {
        return Outcome::Off;
    }
    if !p.peer_on {
        set_status(state, id, Status::Waiting);
        return Outcome::Waiting;
    }
    // The watcher leaves a conflict (and a pause it just met) to the
    // minute's check and to *Sync now*.
    if trigger == Trigger::Watch {
        let recently = state
            .machines
            .sync
            .paused_at
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .is_some_and(|t| t.elapsed() < PAUSE_QUIET);
        match p.status.reason() {
            Some(Reason::Conflict) => return Outcome::Paused(Reason::Conflict),
            Some(r) if recently => return Outcome::Paused(r),
            _ => {}
        }
    }
    let name = name_of(&p.folder);
    let peer = m::peer_name(state, &p.machine);
    let here = this(state);
    // What stops it here.
    let (ours, theirs) = move_grants(state, &p.machine);
    let gate = Gate {
        machines_on: m::using(state).is_ok(),
        move_here: move_running_in(state, &p.folder),
        ours,
        theirs,
    };
    if let Some(reason) = blocker(&gate) {
        let detail = match reason {
            Reason::MachinesOff => format!("Machines is off on {here}."),
            Reason::MoveRunning => format!(
                "A session working in {name} is moving between computers: sync waits until the move ends."
            ),
            _ if !gate.ours => format!(
                "{here} has not given {peer} the Move permission: code sync writes into each other's project folder."
            ),
            _ => format!(
                "{peer} has not given {here} the Move permission: code sync writes into each other's project folder."
            ),
        };
        pause(state, &p, Status::paused(reason, detail), trigger);
        return Outcome::Paused(reason);
    }
    // This checkout.
    let g = git(state);
    let folder = PathBuf::from(&p.folder);
    let cur = {
        let (g, f) = (g.clone(), folder.clone());
        tokio::task::spawn_blocking(move || code::read_state(&g, &f)).await
    };
    let cur = match cur {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => {
            pause(
                state,
                &p,
                Status::paused(Reason::Error, unreadable(state, &p.folder, &e)),
                trigger,
            );
            return Outcome::Paused(Reason::Error);
        }
        Err(e) => {
            pause(
                state,
                &p,
                Status::paused(Reason::Error, e.to_string()),
                trigger,
            );
            return Outcome::Paused(Reason::Error);
        }
    };
    let changed_here = p.last.as_ref().is_none_or(|l| l.state != cur.state);
    // Nothing changed here: the other computer syncs its own changes.
    if !changed_here && trigger == Trigger::Watch && matches!(p.status, Status::InSync { .. }) {
        return Outcome::Unchanged;
    }
    // The other one's.
    let q = match &cur.state.branch {
        Some(b) => format!("?branch={}", urlencoding_branch(b)),
        None => String::new(),
    };
    let there: There = match call(
        state,
        &p,
        "GET",
        &format!("/peer/sync/{id}{q}"),
        None,
        STATE_TIMEOUT,
    )
    .await
    {
        Ok(v) => serde_json::from_value(v).unwrap_or_default(),
        Err(why) => return pause_for(state, &p, why, trigger),
    };
    if !there.on {
        turned_off_there(state, &p, None);
        return Outcome::Off;
    }
    if there.move_running {
        pause(
            state,
            &p,
            Status::paused(
                Reason::MoveRunning,
                format!("A session working in {name} is moving on {peer}: sync waits until the move ends."),
            ),
            trigger,
        );
        return Outcome::Paused(Reason::MoveRunning);
    }
    if let Some(e) = &there.error {
        pause(
            state,
            &p,
            Status::paused(Reason::Error, format!("{peer}: {e}")),
            trigger,
        );
        return Outcome::Paused(Reason::Error);
    }
    let Some(their_state) = there.state.clone().filter(|s| s.validate().is_ok()) else {
        pause(
            state,
            &p,
            Status::paused(Reason::Error, format!("{peer} answered oddly.")),
            trigger,
        );
        return Outcome::Paused(Reason::Error);
    };
    let first = p.last.is_none() || there.last_step.is_none();
    let they_can_take = first
        && cur.state != their_state
        && (there.reset_ok || behind_or_same(&g, &folder, &cur.state, there.tip.as_deref()).await);
    let decision = decide(&Sides {
        ours: &cur.state,
        our_last: p.last.as_ref().map(|l| &l.state),
        our_clean: cur.changed == 0,
        theirs: &their_state,
        their_changed: there.changed,
        their_synced: there.last_step.is_some(),
        their_clean: there.clean,
        their_conflict: there.conflict,
        they_can_take,
        second,
    });
    match decision {
        Decision::Agree => agree(state, &p, &cur.state, &there, trigger).await,
        Decision::InSync => {
            update(state, id, |q| {
                q.conflict = None;
                q.status = Status::InSync { at: m::now() };
            });
            Outcome::Unchanged
        }
        Decision::Send => send(state, &p, &there, trigger).await,
        Decision::TheyShouldSend => Outcome::TheyShouldSend,
        Decision::Busy => {
            // It asked this side to send, yet it changed itself: a race.
            state.machines.sync.ask_again(id);
            Outcome::Busy
        }
        Decision::Conflict { first } => conflict(state, &p, &cur.state, &there, first).await,
    }
}

/// The local facts a step checks before it reads anything.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Gate {
    /// Machines is on here.
    pub machines_on: bool,
    /// A session working in the project is moving.
    pub move_here: bool,
    /// This computer gave the other `move` (for the path in use).
    pub ours: bool,
    /// The other gave this one `move`, as it last said (`None`: unknown).
    pub theirs: Option<bool>,
}

/// What stops a step here before anything is read: the first that holds.
pub(crate) fn blocker(g: &Gate) -> Option<Reason> {
    if !g.machines_on {
        Some(Reason::MachinesOff)
    } else if g.move_here {
        Some(Reason::MoveRunning)
    } else if !g.ours || g.theirs == Some(false) {
        Some(Reason::NoGrant)
    } else {
        None
    }
}

/// Both checkouts, as a step weighs them.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Sides<'a> {
    /// This checkout now, and its last synced state.
    pub ours: &'a TreeState,
    pub our_last: Option<&'a TreeState>,
    pub our_clean: bool,
    /// The other checkout now; whether it changed since its last synced
    /// state; whether it has one.
    pub theirs: &'a TreeState,
    pub their_changed: bool,
    pub their_synced: bool,
    pub their_clean: bool,
    /// It waits for a conflict to be resolved.
    pub their_conflict: bool,
    /// (A first sync.) Its branch is behind this side's or the same, or it
    /// may be reset: it takes this side's state without losing a commit.
    pub they_can_take: bool,
    /// The other computer asked this one to send: no asking back.
    pub second: bool,
}

/// What a step does (plan §5.15 "One step").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    /// The same state on both: both record it, nothing travels.
    Agree,
    /// Neither changed.
    InSync,
    /// Only this side changed: its snapshot lands there.
    Send,
    /// Only the other changed: it sends.
    TheyShouldSend,
    /// Both changed: paused on both until someone chooses.
    Conflict { first: bool },
    /// The other asked this side to send but changed itself: try again.
    Busy,
}

/// The decision table of the module's docs.
pub(crate) fn decide(s: &Sides) -> Decision {
    if s.our_last.is_none() || !s.their_synced {
        if s.ours == s.theirs {
            return Decision::Agree;
        }
        // The first sync: the clean side that is behind takes the other's.
        if s.their_clean && s.they_can_take {
            return Decision::Send;
        }
        if s.our_clean && !s.second {
            return Decision::TheyShouldSend;
        }
        return Decision::Conflict { first: true };
    }
    let changed_here = s.our_last != Some(s.ours);
    match (changed_here, s.their_changed) {
        // Both went back to the synced state: a conflict there is over.
        (false, false) if s.their_conflict => Decision::Agree,
        (false, false) => Decision::InSync,
        // The same changes on both: nothing travels.
        _ if s.ours == s.theirs => Decision::Agree,
        (true, false) => Decision::Send,
        (false, true) if s.second => Decision::Busy,
        (false, true) => Decision::TheyShouldSend,
        (true, true) => Decision::Conflict { first: false },
    }
}

/// A branch name in a query string (letters, digits and `-._/` stay).
fn urlencoding_branch(b: &str) -> String {
    b.bytes()
        .map(|c| match c {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'/' => {
                (c as char).to_string()
            }
            _ => format!("%{c:02X}"),
        })
        .collect()
}

/// The other computer's tip of this side's branch is behind its head, or
/// the same, or absent (so it takes this side's state without losing a
/// commit). A detached head moves no branch there.
async fn behind_or_same(g: &GitRunner, folder: &Path, ours: &TreeState, tip: Option<&str>) -> bool {
    if ours.branch.is_none() {
        return true;
    }
    let tip = match tip {
        Some(t) if code::is_oid(t) => Some(t.to_string()),
        Some(_) => return false,
        None => None,
    };
    let (g, f, base) = (g.clone(), folder.to_path_buf(), ours.head.clone());
    let r = tokio::task::spawn_blocking(move || {
        code::branch_relation(&g, &f, base.as_deref(), tip.as_deref())
    })
    .await;
    matches!(
        r,
        Ok(Ok(BranchRelation::Create
            | BranchRelation::Same
            | BranchRelation::FastForward))
    )
}

/// Both computers hold the same state: both record it, nothing travels.
async fn agree(
    state: &AppState,
    p: &Pair,
    ours: &TreeState,
    there: &There,
    trigger: Trigger,
) -> Outcome {
    let step = next_step(p, there);
    let their_state = there.state.clone().unwrap_or_default();
    match call(
        state,
        p,
        "POST",
        &format!("/peer/sync/{}/agree", p.id),
        Some(&json!({ "state": their_state, "step": step })),
        STATE_TIMEOUT,
    )
    .await
    {
        Ok(_) => {}
        Err(Peer::Refused(409, code, _)) if code == "sync_changed" => {
            state.machines.sync.ask_again(&p.id);
            return Outcome::Busy;
        }
        Err(why) => return pause_for(state, p, why, trigger),
    }
    let now = m::now();
    if let Some(q) = update(state, &p.id, |q| {
        q.last = Some(Last {
            state: ours.clone(),
            snapshot: None,
            step,
            at: now,
            direction: Direction::Both,
            files: 0,
        });
        q.conflict = None;
        q.reset_ok = false;
        q.status = Status::InSync { at: now };
    }) {
        audit(state, "machine.sync.agreed", &q, json!({ "step": step }));
    }
    Outcome::Agreed
}

fn next_step(p: &Pair, there: &There) -> u64 {
    p.last
        .as_ref()
        .map(|l| l.step)
        .unwrap_or(0)
        .max(there.last_step.unwrap_or(0))
        + 1
}

/// Both changed: paused here and there, both sides said.
async fn conflict(
    state: &AppState,
    p: &Pair,
    ours: &TreeState,
    there: &There,
    first: bool,
) -> Outcome {
    let here_changes = {
        let (g, f, since, now) = (
            git(state),
            PathBuf::from(&p.folder),
            p.last.as_ref().map(|l| l.state.clone()),
            ours.clone(),
        );
        tokio::task::spawn_blocking(move || code::changes(&g, &f, since.as_ref(), &now))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default()
    };
    let name = name_of(&p.folder);
    let detail = if first {
        format!("{name} differs on both computers: choose which one's to keep.")
    } else {
        format!("{name} changed on both computers: choose which one's to keep.")
    };
    let c = Conflict {
        at: m::now(),
        here: here_changes.clone(),
        there: there.summary.clone(),
        first,
    };
    let Some(q) = update(state, &p.id, |q| {
        q.conflict = Some(c.clone());
        q.status = Status::paused(Reason::Conflict, detail.clone());
    }) else {
        return Outcome::Off;
    };
    audit(
        state,
        "machine.sync.conflict",
        &q,
        json!({ "here": here_changes.files, "there": there.summary.files, "first": first }),
    );
    // The other computer pauses too, with the sides its way round.
    let body = json!({
        "reason": "conflict",
        "detail": detail,
        "first": first,
        "yours": there.summary,
        "mine": here_changes,
    });
    if let Err(e) = call(
        state,
        p,
        "POST",
        &format!("/peer/sync/{}/pause", p.id),
        Some(&body),
        STATE_TIMEOUT,
    )
    .await
    {
        tracing::debug!(error = ?e, "the other computer was not told of the conflict");
    }
    Outcome::Conflict
}

/// This side's state goes there: a snapshot (a bundle without what it has,
/// chained on the last steps), the files git ignores the project carries
/// (with its `dangerous` grant), landed in its checkout.
async fn send(state: &AppState, p: &Pair, there: &There, trigger: Trigger) -> Outcome {
    let id = p.id.clone();
    set_status(state, &id, Status::Syncing { since: m::now() });
    let Some(dir) = state.machines.sync.dir_of(&id).map(|d| d.join("out")) else {
        pause(
            state,
            p,
            Status::paused(Reason::Error, "This build has no data folder."),
            trigger,
        );
        return Outcome::Paused(Reason::Error);
    };
    let _ = std::fs::remove_dir_all(&dir);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        pause(
            state,
            p,
            Status::paused(Reason::Error, e.to_string()),
            trigger,
        );
        return Outcome::Paused(Reason::Error);
    }
    crate::claude::transfer::private_dir(&dir);
    let step = next_step(p, there);
    let g = git(state);
    let folder = PathBuf::from(&p.folder);
    // The snapshot.
    let mut req = code::SnapshotRequest::new(
        &id,
        (m::now().max(1)) as u64,
        &m::this_name(state),
        dir.join("code"),
    );
    req.sync = true;
    req.haves = there.haves.clone();
    req.chain = !step.is_multiple_of(CHAIN_EVERY);
    let snap = {
        let (g, f) = (g.clone(), folder.clone());
        tokio::task::spawn_blocking(move || code::snapshot(&g, &f, &req)).await
    };
    let meta: SnapshotMeta = match snap {
        Ok(Ok(m)) => m,
        Ok(Err(e)) => {
            pause(
                state,
                p,
                Status::paused(Reason::Error, unreadable(state, &p.folder, &e)),
                trigger,
            );
            return Outcome::Paused(Reason::Error);
        }
        Err(e) => {
            pause(
                state,
                p,
                Status::paused(Reason::Error, e.to_string()),
                trigger,
            );
            return Outcome::Paused(Reason::Error);
        }
    };
    let sent = TreeState {
        branch: meta.branch.clone(),
        head: meta.base.clone(),
        idx_tree: meta.idx_tree.clone(),
        wt_tree: meta.wt_tree.clone(),
    };
    // Its files.
    let mut names = vec![code::META.to_string()];
    if meta.bundle.is_some() {
        names.push(code::BUNDLE.to_string());
    }
    if let Some(l) = &meta.lfs {
        names.extend(l.carried.iter().map(|o| format!("lfs.{o}")));
    }
    // The files git ignores the project carries, when the other computer
    // lets this one carry them (its `dangerous` grant).
    let mut carried: Vec<String> = Vec::new();
    let list = crate::machines::setup::options::effective(state, &p.folder)
        .recipe
        .map(|r| r.carry_ignored)
        .unwrap_or_default();
    let may_carry = state.machines.store_read(|f| {
        f.peer(&p.machine)
            .and_then(|q| q.their_grants)
            .is_some_and(|g| {
                g.allows(
                    crate::machines::grants::Grant::Dangerous,
                    super::lan_now(state, &p.machine),
                )
            })
    });
    if !list.is_empty() && may_carry {
        let (g, f, out) = (g.clone(), folder.clone(), dir.join(ignored::FILE));
        let packed = tokio::task::spawn_blocking(move || {
            let scan = ignored::scan(&g, &f, "", &list);
            ignored::pack(&f, &scan.found, &out)
        })
        .await;
        if let Ok(Ok(packed)) = packed {
            if !packed.files.is_empty() {
                carried = packed.files.iter().map(|f| f.path.clone()).collect();
                names.push(ignored::FILE.to_string());
            }
        }
    }
    let mut files: Vec<Value> = Vec::new();
    for name in &names {
        let Some(path) = moves::file_path(&dir, name) else {
            continue;
        };
        match crate::claude::transfer::sha256_file(&path) {
            Ok((size, sha256)) => {
                files.push(json!({ "name": name, "size": size, "sha256": sha256 }))
            }
            Err(e) => {
                pause(
                    state,
                    p,
                    Status::paused(Reason::Error, format!("{name}: {e}")),
                    trigger,
                );
                return Outcome::Paused(Reason::Error);
            }
        }
    }
    // Announced, then sent from what it has of each.
    let begun = match call(
        state,
        p,
        "POST",
        &format!("/peer/sync/{id}/steps"),
        Some(&json!({ "step": step, "files": files, "ignored": carried })),
        STATE_TIMEOUT,
    )
    .await
    {
        Ok(v) => v,
        Err(why) => return pause_for(state, p, why, trigger),
    };
    let have = |name: &str| -> u64 {
        begun
            .get("files")
            .and_then(Value::as_array)
            .and_then(|l| l.iter().find(|f| f["name"] == name))
            .and_then(|f| f["have"].as_u64())
            .unwrap_or(0)
    };
    for f in &files {
        let name = f["name"].as_str().unwrap_or_default().to_string();
        let size = f["size"].as_u64().unwrap_or(0);
        let Some(path) = moves::file_path(&dir, &name) else {
            continue;
        };
        let mut offset = have(&name).min(size);
        while offset < size {
            let n = ((size - offset) as usize).min(chunks::CHUNK);
            let pth = path.clone();
            let bytes = match tokio::task::spawn_blocking(move || {
                chunks::read_chunk(&pth, offset, n)
            })
            .await
            {
                Ok(Ok(b)) => b,
                Ok(Err(e)) => {
                    pause(
                        state,
                        p,
                        Status::paused(Reason::Error, e.to_string()),
                        trigger,
                    );
                    return Outcome::Paused(Reason::Error);
                }
                Err(e) => {
                    pause(
                        state,
                        p,
                        Status::paused(Reason::Error, e.to_string()),
                        trigger,
                    );
                    return Outcome::Paused(Reason::Error);
                }
            };
            let url = format!("/peer/sync/{id}/files/{name}?offset={offset}");
            offset = match chunks::put(state, &p.machine, &url, bytes, ACTING).await {
                Ok(size) => size,
                Err(PutError::Offline(e)) => {
                    let peer = m::peer_name(state, &p.machine);
                    pause(
                        state,
                        p,
                        Status::paused(Reason::Offline, format!("{peer} can't be reached: {e}")),
                        trigger,
                    );
                    return Outcome::Paused(Reason::Offline);
                }
                Err(PutError::Odd) => {
                    let peer = m::peer_name(state, &p.machine);
                    pause(
                        state,
                        p,
                        Status::paused(Reason::Error, format!("{peer} answered oddly.")),
                        trigger,
                    );
                    return Outcome::Paused(Reason::Error);
                }
                Err(PutError::Refused(status, v)) => {
                    let message = v
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("it refused")
                        .to_string();
                    let code = v
                        .pointer("/error/code")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    return pause_for(state, p, peer_refusal(status, code, message), trigger);
                }
            };
        }
    }
    // Landed there.
    let landed = call(
        state,
        p,
        "POST",
        &format!("/peer/sync/{id}/land"),
        Some(&json!({ "step": step })),
        LAND_TIMEOUT,
    )
    .await;
    match landed {
        Ok(_) => {}
        Err(Peer::Refused(409, code, _)) if code == "sync_changed" || code == "sync_diverged" => {
            // It changed there in between: a conflict, never an overwrite.
            let there_now = There {
                changed: true,
                ..there.clone()
            };
            let refreshed = refresh_there(state, p, &sent).await.unwrap_or(there_now);
            return conflict(state, p, &sent, &refreshed, false).await;
        }
        Err(why) => return pause_for(state, p, why, trigger),
    }
    // Both hold it now.
    let files_changed = {
        let (g, f, since, now) = (
            g.clone(),
            folder.clone(),
            p.last.as_ref().map(|l| l.state.clone()),
            sent.clone(),
        );
        tokio::task::spawn_blocking(move || code::changes(&g, &f, since.as_ref(), &now))
            .await
            .ok()
            .and_then(Result::ok)
            .map(|c| c.files)
            .unwrap_or(0)
    };
    let now = m::now();
    let Some(q) = update(state, &id, |q| {
        q.last = Some(Last {
            state: sent.clone(),
            snapshot: Some(meta.snap.clone()),
            step,
            at: now,
            direction: Direction::Out,
            files: files_changed,
        });
        q.conflict = None;
        q.reset_ok = false;
        q.status = Status::InSync { at: now };
    }) else {
        return Outcome::Off;
    };
    audit(
        state,
        "machine.sync.sent",
        &q,
        json!({ "step": step, "files": files_changed, "ignored": carried.len() }),
    );
    tidy(state, &id, &folder, &dir).await;
    Outcome::Sent
}

/// The other computer's refusal of a chunk, as a call's.
fn peer_refusal(status: u16, code: String, message: String) -> Peer {
    match (status, code.as_str()) {
        (409, "sync_busy") => Peer::Busy,
        (404, "sync_unknown") => Peer::Unknown,
        (403, "machines_off") => Peer::Status(Status::paused(Reason::MachinesOff, message)),
        (403, "grant_missing") => Peer::Status(Status::paused(Reason::NoGrant, message)),
        _ => Peer::Refused(status, code, message),
    }
}

/// The other computer's state again (after its landing stopped).
async fn refresh_there(state: &AppState, p: &Pair, ours: &TreeState) -> Option<There> {
    let q = ours
        .branch
        .as_ref()
        .map(|b| format!("?branch={}", urlencoding_branch(b)))
        .unwrap_or_default();
    call(
        state,
        p,
        "GET",
        &format!("/peer/sync/{}{q}", p.id),
        None,
        STATE_TIMEOUT,
    )
    .await
    .ok()
    .and_then(|v| serde_json::from_value(v).ok())
}

/// After a step: the older steps' refs go, and the step's files.
pub(crate) async fn tidy(state: &AppState, id: &str, folder: &Path, dir: &Path) {
    let (g, f, i) = (git(state), folder.to_path_buf(), id.to_string());
    let _ =
        tokio::task::spawn_blocking(move || code::prune_snapshots(&g, &f, &i, KEEP_STEPS)).await;
    let _ = std::fs::remove_dir_all(dir);
}

// ─── resolving a conflict ──────────────────────────────────────────────────────

/// `POST /api/machines/sync/{id}/resolve {keep}`: the losing side keeps its
/// state under a ref and goes back to the synced state; then the winner's
/// lands as an ordinary step.
pub async fn resolve(state: &AppState, id: &str, keep_here: bool) -> ApiResult<Value> {
    let p = state
        .machines
        .sync
        .pair(id)
        .ok_or_else(super::routes::unknown)?;
    if !matches!(p.status.reason(), Some(Reason::Conflict)) {
        return Err(ApiError::conflict(
            "sync_no_conflict",
            "There is nothing to choose: it is not paused for a conflict.",
        ));
    }
    m::using(state)?;
    let peer = m::peer_name(state, &p.machine);
    if keep_here {
        // The other computer's state is kept there and it goes back.
        let v = match call(
            state,
            &p,
            "POST",
            &format!("/peer/sync/{id}/lose"),
            Some(&json!({})),
            LAND_TIMEOUT,
        )
        .await
        {
            Ok(v) => v,
            Err(Peer::Refused(_, code, message)) => {
                return Err(ApiError::conflict(
                    if code.is_empty() {
                        "sync_refused".into()
                    } else {
                        code
                    },
                    format!("{peer} could not set its changes aside: {message}"),
                ))
            }
            Err(Peer::Status(s)) => {
                let detail = match &s {
                    Status::Paused { detail, .. } => detail.clone(),
                    _ => format!("{peer} can't be reached."),
                };
                return Err(ApiError::conflict("sync_unreachable", detail));
            }
            Err(Peer::Busy) => {
                return Err(ApiError::conflict(
                    "sync_busy",
                    format!("{peer} is busy with this project: try again in a moment."),
                ))
            }
            Err(Peer::Unknown) => {
                turned_off_there(state, &p, None);
                return Err(ApiError::conflict(
                    "sync_off",
                    format!("{peer} does not keep this project in sync any more."),
                ));
            }
        };
        let kept = v
            .pointer("/kept/ref")
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(q) = update(state, id, |q| {
            if let Some(r) = &kept {
                q.kept.push(KeptRef {
                    ref_name: r.clone(),
                    side: "there".into(),
                    commit: v
                        .pointer("/kept/commit")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    files: v
                        .pointer("/kept/files")
                        .and_then(Value::as_u64)
                        .unwrap_or(0) as usize,
                    at: m::now(),
                });
            }
        }) {
            audit(
                state,
                "machine.sync.resolved",
                &q,
                json!({ "keep": "here", "kept": kept, "keptOn": "there" }),
            );
        }
        run(state, id, Trigger::Now).await;
    } else {
        // This computer's state is kept here and it goes back.
        let kept = lose_here(state, id).await?;
        if let Some(q) = state.machines.sync.pair(id) {
            audit(
                state,
                "machine.sync.resolved",
                &q,
                json!({ "keep": "there", "kept": kept.ref_name, "keptOn": "here" }),
            );
        }
        ask_to_send(state, id, Trigger::Now).await;
    }
    let p = state
        .machines
        .sync
        .pair(id)
        .ok_or_else(super::routes::unknown)?;
    Ok(super::pair_view(state, &p))
}

/// This side lost: everything its checkout holds kept under a ref, the
/// checkout put back to the synced state (or its HEAD, clean, when there
/// was none). Refused, with nothing changed, when anything could be lost.
pub(crate) async fn lose_here(state: &AppState, id: &str) -> ApiResult<KeptRef> {
    let lock = state.machines.sync.lock_of(id);
    let _g = lock.try_lock_owned().map_err(|_| {
        ApiError::conflict(
            "sync_busy",
            "A step of this project runs here: try again in a moment.",
        )
    })?;
    let p = state
        .machines
        .sync
        .pair(id)
        .ok_or_else(super::routes::unknown)?;
    let (g, f, to, sid, machine) = (
        git(state),
        PathBuf::from(&p.folder),
        p.last.as_ref().map(|l| l.state.clone()),
        p.id.clone(),
        m::this_name(state),
    );
    let kept = tokio::task::spawn_blocking(move || {
        code::keep_and_put_back(
            &g,
            &f,
            &code::PutBack {
                id: &sid,
                machine: &machine,
                to: to.as_ref(),
            },
        )
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::conflict("sync_put_back_refused", e.to_string()))?;
    let record = KeptRef {
        ref_name: kept.kept_ref.clone().unwrap_or_default(),
        side: "here".into(),
        commit: kept.commit.clone(),
        files: kept.files,
        at: m::now(),
    };
    let had = kept.kept_ref.is_some();
    if let Some(q) = update(state, id, |q| {
        if had {
            q.kept.push(record.clone());
        }
        q.reset_ok = true;
        q.status = Status::Syncing { since: m::now() };
    }) {
        audit(
            state,
            "machine.sync.kept",
            &q,
            json!({ "kept": kept.kept_ref, "files": kept.files, "commits": kept.commits_kept }),
        );
    }
    Ok(record)
}

// ─── the minute's check ────────────────────────────────────────────────────────

/// Every minute, while the app watches: a step of every pair that is on on
/// both computers (the safety net under the watchers).
pub(crate) async fn tick_loop(state: Weak<Inner>) {
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tick.tick().await;
    loop {
        tick.tick().await;
        let Some(st) = state.upgrade() else { break };
        let ids: Vec<String> = st.machines.sync.read(|f| {
            f.pairs
                .iter()
                .filter(|p| p.active())
                .map(|p| p.id.clone())
                .collect()
        });
        for id in ids {
            run_auto(&st, &id, Trigger::Tick).await;
        }
    }
}

/// Machines turned on here: every pair checks (while the app watches).
pub fn on_peer_back_all(state: &AppState) {
    if !state.machines.sync.watching() {
        return;
    }
    let ids: Vec<String> = state.machines.sync.read(|f| {
        f.pairs
            .iter()
            .filter(|p| p.active())
            .map(|p| p.id.clone())
            .collect()
    });
    for id in ids {
        let st = state.clone();
        tokio::spawn(async move {
            run_auto(&st, &id, Trigger::Tick).await;
        });
    }
}

/// A paired computer answered again: its pairs that waited for it check.
pub fn on_peer_back(state: &AppState, machine: &str) {
    if !state.machines.sync.watching() {
        return;
    }
    let ids: Vec<String> = state.machines.sync.read(|f| {
        f.pairs
            .iter()
            .filter(|p| p.active() && p.machine == machine)
            .filter(|p| !matches!(p.status, Status::InSync { .. }))
            .map(|p| p.id.clone())
            .collect()
    });
    for id in ids {
        let st = state.clone();
        tokio::spawn(async move {
            run_auto(&st, &id, Trigger::Tick).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(n: u8) -> TreeState {
        let c = |x: u8| format!("{x:x}").repeat(40);
        TreeState {
            branch: Some("refs/heads/main".into()),
            head: Some(c(1)),
            idx_tree: c(n),
            wt_tree: c(n),
        }
    }

    fn sides<'a>(ours: &'a TreeState, last: &'a TreeState, theirs: &'a TreeState) -> Sides<'a> {
        Sides {
            ours,
            our_last: Some(last),
            our_clean: false,
            theirs,
            their_changed: false,
            their_synced: true,
            their_clean: false,
            their_conflict: false,
            they_can_take: false,
            second: false,
        }
    }

    #[test]
    fn the_decision_table() {
        let (a, b, c) = (st(2), st(3), st(4));
        // Unchanged on both: in sync, whether the states are one or differ
        // by a limit.
        assert_eq!(decide(&sides(&a, &a, &a)), Decision::InSync);
        assert_eq!(decide(&sides(&a, &a, &b)), Decision::InSync);
        // Changed here only.
        assert_eq!(decide(&sides(&b, &a, &a)), Decision::Send);
        // Changed there only.
        let mut s = sides(&a, &a, &b);
        s.their_changed = true;
        assert_eq!(decide(&s), Decision::TheyShouldSend);
        // …but it asked this side to send: a race, tried again.
        s.second = true;
        assert_eq!(decide(&s), Decision::Busy);
        // Both changed: a conflict, never an overwrite.
        let mut s = sides(&b, &a, &c);
        s.their_changed = true;
        assert_eq!(decide(&s), Decision::Conflict { first: false });
        // Both changed the same way: no conflict.
        let mut s = sides(&c, &a, &c);
        s.their_changed = true;
        assert_eq!(decide(&s), Decision::Agree);
        // Both back at the synced state while paused: the conflict is over.
        let mut s = sides(&a, &a, &b);
        s.their_conflict = true;
        assert_eq!(decide(&s), Decision::Agree);
    }

    #[test]
    fn the_first_sync_sends_to_a_clean_side_behind() {
        let (a, b) = (st(2), st(3));
        let first = |our_clean, their_clean, they_can_take, second| {
            decide(&Sides {
                ours: &a,
                our_last: None,
                our_clean,
                theirs: &b,
                their_changed: true,
                their_synced: false,
                their_clean,
                their_conflict: false,
                they_can_take,
                second,
            })
        };
        assert_eq!(first(false, true, true, false), Decision::Send);
        // It is clean but has commits of its own: this side, clean, asks it.
        assert_eq!(first(true, true, false, false), Decision::TheyShouldSend);
        // Changes on both: the person chooses.
        assert_eq!(
            first(false, false, true, false),
            Decision::Conflict { first: true }
        );
        // Asked to send, and it cannot: no asking back.
        assert_eq!(
            first(true, false, false, true),
            Decision::Conflict { first: true }
        );
        // The same on both from the start: nothing travels.
        let same = decide(&Sides {
            our_last: None,
            their_synced: false,
            ..sides(&a, &a, &a)
        });
        assert_eq!(same, Decision::Agree);
    }

    #[test]
    fn a_step_waits_for_machines_a_move_and_both_grants() {
        let ok = Gate {
            machines_on: true,
            move_here: false,
            ours: true,
            theirs: Some(true),
        };
        assert_eq!(blocker(&ok), None);
        assert_eq!(
            blocker(&Gate { theirs: None, ..ok }),
            None,
            "not known yet: the other computer says"
        );
        assert_eq!(
            blocker(&Gate {
                machines_on: false,
                move_here: true,
                ..ok
            }),
            Some(Reason::MachinesOff)
        );
        assert_eq!(
            blocker(&Gate {
                move_here: true,
                ..ok
            }),
            Some(Reason::MoveRunning)
        );
        assert_eq!(blocker(&Gate { ours: false, ..ok }), Some(Reason::NoGrant));
        assert_eq!(
            blocker(&Gate {
                theirs: Some(false),
                ..ok
            }),
            Some(Reason::NoGrant)
        );
    }

    #[test]
    fn a_branch_name_travels_in_a_query() {
        assert_eq!(
            urlencoding_branch("refs/heads/feat/x-1.2_y"),
            "refs/heads/feat/x-1.2_y"
        );
        assert_eq!(
            urlencoding_branch("refs/heads/a b&c"),
            "refs/heads/a%20b%26c"
        );
    }
}
