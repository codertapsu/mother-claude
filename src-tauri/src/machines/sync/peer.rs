//! The other computer's half of code sync (`/peer/sync/*`, docs/PLAN-
//! MACHINES.md §5.15): its state when asked, a step's files (the moves'
//! chunks, `machines::chunks`), the landing in its checkout — parked at the
//! last synced state, else a conflict — agreeing on identical changes, a
//! conflict's pause, sending when asked, losing a resolution, and being
//! told the other side confirmed or turned it off.
//!
//! Everything the other computer sends is checked before git or the file
//! system sees it: the pair must be one it shares with that computer, file
//! names come from a fixed list, sizes and hashes are checked, states are
//! object ids and branch names.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{json, Value};

use super::step::{self, git};
use super::{audit, name_of, update, Conflict, Direction, Last, Pair, Reason, Status, Trigger};
use crate::machines::chunks::{self, AppendError, Refusal};
use crate::machines::code::{self, ApplyRequest, ApplyStop, Changes, Landing, Reset, TreeState};
use crate::machines::moves::{self, ignored};
use crate::machines::{self as m};
use crate::server::context::ClientContext;
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// At most this many files in a step.
const MAX_FILES: usize = 10_000;
/// The largest file a step may hold.
const MAX_FILE: u64 = 1 << 40;
/// The step's record in its folder.
const MANIFEST: &str = "manifest.json";

/// The pair `id` this computer shares with `from` (`404 sync_unknown`
/// otherwise, as if there were none).
pub fn pair_of(state: &AppState, id: &str, from: &str) -> ApiResult<Pair> {
    state
        .machines
        .sync
        .pair(id)
        .filter(|p| p.machine == from)
        .ok_or_else(super::routes::unknown)
}

/// The pair, on on both computers here.
fn active(state: &AppState, id: &str, from: &str) -> ApiResult<Pair> {
    let p = pair_of(state, id, from)?;
    if !p.here_on {
        return Err(super::routes::unknown());
    }
    if !p.peer_on {
        return Err(ApiError::conflict(
            "sync_off",
            "Code sync is not on for this project on both computers.",
        ));
    }
    Ok(p)
}

fn busy(name: &str) -> ApiError {
    ApiError::conflict(
        "sync_busy",
        format!("A step of {name} runs on this computer: try again in a moment."),
    )
}

fn in_dir(state: &AppState, id: &str) -> ApiResult<PathBuf> {
    state
        .machines
        .sync
        .dir_of(id)
        .map(|d| d.join("in"))
        .ok_or_else(|| ApiError::internal("This build has no data folder."))
}

// ─── its state ─────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
pub struct StateQuery {
    /// The asking computer's branch: this computer says its own tip of it.
    #[serde(default)]
    pub branch: Option<String>,
}

/// `GET /peer/sync/{id}`: this checkout's state and what the asking
/// computer needs to decide (whether it changed since the last synced
/// state, what it has for a bundle, its tip of the asker's branch).
pub async fn state_for(state: &AppState, from: &str, id: &str, q: StateQuery) -> ApiResult<Value> {
    let p = pair_of(state, id, from)?;
    if !p.here_on {
        return Ok(json!({ "on": false }));
    }
    let g = git(state);
    let folder = PathBuf::from(&p.folder);
    let last = p.last.as_ref().map(|l| l.state.clone());
    let branch = q.branch.filter(|b| code::valid_branch(b));
    let (g2, f2, id2) = (g.clone(), folder.clone(), id.to_string());
    let read = tokio::task::spawn_blocking(move || {
        let c = code::read_state(&g2, &f2)?;
        let tip = match &branch {
            Some(b) => code::tip_of(&g2, &f2, b)?,
            None => None,
        };
        let haves = code::haves(&g2, &f2, Some(&id2))?;
        let summary = code::changes(&g2, &f2, last.as_ref(), &c.state)?;
        Ok::<_, code::CodeError>((c, tip, haves, summary))
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let move_running = super::move_running_in(state, &p.folder);
    let base = json!({
        "on": true,
        "peerOn": p.peer_on,
        "lastStep": p.last.as_ref().map(|l| l.step),
        "moveRunning": move_running,
        "conflict": matches!(p.status.reason(), Some(Reason::Conflict)),
        "resetOk": p.reset_ok,
    });
    let mut v = base;
    match read {
        Ok((c, tip, haves, summary)) => {
            let changed = p.last.as_ref().is_none_or(|l| l.state != c.state);
            v["state"] = json!(c.state);
            v["clean"] = json!(c.changed == 0);
            v["changed"] = json!(changed);
            v["tip"] = json!(tip);
            v["haves"] = json!(haves.haves);
            v["shallow"] = json!(haves.shallow);
            v["summary"] = json!(summary);
        }
        Err(e) => {
            v["error"] = json!(step::unreadable(state, &p.folder, &e));
        }
    }
    Ok(v)
}

// ─── a step's files ────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSpec {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

/// `POST /peer/sync/{id}/steps`: a step is coming.
#[derive(Clone, Debug, Default, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Begin {
    pub step: u64,
    pub files: Vec<FileSpec>,
    /// The files git ignores that travel with it, under the top.
    #[serde(default)]
    pub ignored: Vec<String>,
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn check_begin(b: &Begin, dir: &Path) -> ApiResult<()> {
    let bad = |m: &str| ApiError::unprocessable("sync_bad_step", m.to_string());
    if b.files.len() > MAX_FILES {
        return Err(bad("Too many files."));
    }
    let mut seen = std::collections::HashSet::new();
    for f in &b.files {
        if moves::file_path(dir, &f.name).is_none()
            || f.name == crate::claude::backup::session_pack::FILE
            || !seen.insert(f.name.as_str())
        {
            return Err(bad("A file this computer does not take."));
        }
        if f.size > MAX_FILE || !is_sha256(&f.sha256) {
            return Err(bad("A file's size or hash."));
        }
    }
    if !b.files.iter().any(|f| f.name == code::META) {
        return Err(bad("The step has no record."));
    }
    if !ignored::valid_list(&b.ignored)
        || (!b.ignored.is_empty() && !b.files.iter().any(|f| f.name == ignored::FILE))
    {
        return Err(bad("The files git ignores it names."));
    }
    Ok(())
}

fn load_manifest(dir: &Path) -> Option<Begin> {
    crate::persist::read_json(&dir.join(MANIFEST))
}

fn have_of(dir: &Path, name: &str) -> u64 {
    moves::file_path(dir, name).map_or(0, |p| chunks::have(&p))
}

/// `POST /peer/sync/{id}/steps`: the step's files are announced; what
/// arrived of the same step before stays (a broken transfer goes on).
pub async fn begin(state: &AppState, from: &str, id: &str, b: Begin) -> ApiResult<Value> {
    active(state, id, from)?;
    let dir = in_dir(state, id)?;
    check_begin(&b, &dir)?;
    let same = load_manifest(&dir).is_some_and(|old| {
        old.step == b.step
            && old.files.len() == b.files.len()
            && old
                .files
                .iter()
                .zip(&b.files)
                .all(|(x, y)| x.name == y.name && x.size == y.size && x.sha256 == y.sha256)
    });
    if !same {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| ApiError::internal(e.to_string()))?;
        crate::claude::transfer::private_dir(&dir);
        crate::persist::write_json_private(&dir.join(MANIFEST), &b)
            .map_err(|e| ApiError::internal(e.to_string()))?;
    }
    let files: Vec<Value> = b
        .files
        .iter()
        .map(|f| json!({ "name": f.name, "have": have_of(&dir, &f.name).min(f.size) }))
        .collect();
    Ok(json!({ "files": files }))
}

/// `GET /peer/sync/{id}/files/{name}`: what this computer has of a file.
pub async fn file_size(state: &AppState, from: &str, id: &str, name: &str) -> ApiResult<Value> {
    active(state, id, from)?;
    let dir = in_dir(state, id)?;
    let mf = load_manifest(&dir).ok_or_else(super::routes::unknown)?;
    let f = mf
        .files
        .iter()
        .find(|f| f.name == name)
        .ok_or_else(|| ApiError::not_found("not_found", "No such file in this step."))?;
    Ok(json!({ "name": name, "size": have_of(&dir, name).min(f.size), "total": f.size }))
}

/// `PUT /peer/sync/{id}/files/{name}?offset=N`: the next chunk of a file.
pub async fn put_chunk(
    state: &AppState,
    from: &str,
    id: &str,
    name: &str,
    offset: u64,
    bytes: axum::body::Bytes,
) -> ApiResult<Value> {
    active(state, id, from)?;
    let dir = in_dir(state, id)?;
    let mf = load_manifest(&dir).ok_or_else(|| {
        ApiError::conflict("sync_no_step", "No step of this project is coming here.")
    })?;
    let f = mf
        .files
        .iter()
        .find(|f| f.name == name)
        .cloned()
        .ok_or_else(|| ApiError::not_found("not_found", "No such file in this step."))?;
    let path = moves::file_path(&dir, name)
        .ok_or_else(|| ApiError::unprocessable("sync_bad_step", "No such file."))?;
    let _one = state.machines.moves.chunks.lock().await;
    let written =
        tokio::task::spawn_blocking(move || chunks::append(&path, offset, &bytes, f.size))
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?
            .map_err(|e| match e {
                AppendError::Refused(Refusal::Offset(size)) => ApiError::conflict(
                    "offset_mismatch",
                    "This computer has another part of the file.",
                )
                .with("size", size),
                AppendError::Refused(Refusal::TooLong) => ApiError::unprocessable(
                    "sync_bad_step",
                    "The file is longer than the step said.",
                ),
                AppendError::Refused(Refusal::TooLarge) => {
                    ApiError::unprocessable("sync_bad_step", "A chunk is at most 8 MiB.")
                }
                AppendError::Io(e) => {
                    ApiError::internal(format!("The file could not be written here: {e}"))
                }
            })?;
    Ok(json!({ "name": name, "size": written }))
}

// ─── landing ───────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
pub struct LandBody {
    #[serde(default)]
    pub step: u64,
}

/// `POST /peer/sync/{id}/land`: every file checked, the snapshot landed in
/// this checkout — parked at the last synced state, else a conflict
/// (`409 sync_changed`, nothing changed) — the files git ignores it
/// carries written (this computer's `dangerous` grant for the sender), and
/// the new state recorded. A failure after the landing puts it back.
pub async fn land(
    state: &AppState,
    ctx: &ClientContext,
    from: &str,
    id: &str,
    body: LandBody,
) -> ApiResult<Value> {
    let p = active(state, id, from)?;
    let name = name_of(&p.folder);
    let lock = state.machines.sync.lock_of(id);
    let _g = lock.try_lock_owned().map_err(|_| busy(&name))?;
    let p = active(state, id, from)?;
    let dir = in_dir(state, id)?;
    let mf = load_manifest(&dir)
        .filter(|mf| mf.step == body.step)
        .ok_or_else(|| {
            ApiError::conflict(
                "sync_no_step",
                "That step did not come here: send it again.",
            )
        })?;
    // Every file whole and as announced.
    let (d, files) = (dir.clone(), mf.files.clone());
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        for f in &files {
            let path = moves::file_path(&d, &f.name).ok_or("a file has no place")?;
            chunks::finish(&path, f.size, &f.sha256)?;
        }
        Ok(())
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|why| {
        ApiError::conflict(
            "sync_incomplete",
            format!("The step did not arrive whole: {why}."),
        )
    })?;
    let code_dir = dir.join("code");
    let meta: code::SnapshotMeta = crate::persist::read_json(&code_dir.join(code::META))
        .filter(|m: &code::SnapshotMeta| m.validate().is_ok() && m.session == id)
        .ok_or_else(|| ApiError::unprocessable("sync_bad_step", "The step's record."))?;
    let sent = TreeState {
        branch: meta.branch.clone(),
        head: meta.base.clone(),
        idx_tree: meta.idx_tree.clone(),
        wt_tree: meta.wt_tree.clone(),
    };
    let here = m::this_name(state);
    let peer = m::peer_name(state, from);
    // The code, parked at the last synced state.
    let mut req = ApplyRequest::new(id, Landing::Checkout);
    req.machine = here.clone();
    req.parked = p.last.as_ref().map(|l| l.state.clone());
    req.reset = if p.reset_ok {
        Reset::Always
    } else {
        match p.last.as_ref().and_then(|l| l.state.head.clone()) {
            Some(h) => Reset::IfTip(h),
            None => Reset::Never,
        }
    };
    let g = git(state);
    let folder = PathBuf::from(&p.folder);
    let applied = {
        let (g, f, cd) = (g.clone(), folder.clone(), code_dir.clone());
        tokio::task::spawn_blocking(move || code::apply(&g, &f, &cd, &req))
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?
    };
    let applied = match applied {
        Ok(a) => a,
        Err(ApplyStop::NotParked { what }) => {
            return Err(ApiError::conflict(
                "sync_changed",
                format!("{name} changed on {here} meanwhile ({what})."),
            ))
        }
        Err(ApplyStop::Dirty { .. }) => {
            return Err(ApiError::conflict(
                "sync_changed",
                format!("{name} has changes of its own on {here}."),
            ))
        }
        Err(ApplyStop::Diverged { branch, .. }) => {
            return Err(ApiError::conflict(
                "sync_diverged",
                format!("{branch} has commits on {here} that {peer} lacks."),
            ))
        }
        Err(ApplyStop::Failed(e)) => {
            return Err(ApiError::unprocessable(
                "sync_failed",
                format!("{name} could not land on {here}: {e}"),
            ))
        }
        Err(stop) => {
            return Err(ApiError::conflict(
                "sync_blocked",
                format!("{name} can't land on {here}: {stop}"),
            ))
        }
    };
    // Put back on a failure from here on: the files git ignores first.
    let undo = |carried: bool| {
        let (g, f, cd, d) = (g.clone(), folder.clone(), code_dir.clone(), dir.clone());
        async move {
            let _ = tokio::task::spawn_blocking(move || {
                if carried {
                    let _ = ignored::undo(&d);
                }
                code::undo(&g, &f, &cd)
            })
            .await;
        }
    };
    let mut carried = false;
    let mut ignored_skipped = false;
    if !mf.ignored.is_empty() {
        if ctx.may_dangerous() {
            let (top, archive, list, d) = (
                applied.path.clone(),
                dir.join(ignored::FILE),
                mf.ignored.clone(),
                dir.clone(),
            );
            let r = tokio::task::spawn_blocking(move || ignored::land(&top, &archive, &list, &d))
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r);
            match r {
                Ok(_) => carried = true,
                Err(why) => {
                    undo(false).await;
                    return Err(ApiError::conflict(
                        "sync_blocked",
                        format!("The files git ignores can't land on {here}: {why}"),
                    ));
                }
            }
        } else {
            ignored_skipped = true;
        }
    }
    if state.machines.sync.take_fail("land") {
        undo(carried).await;
        return Err(ApiError::new(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "injected",
            "A test stopped the step after its landing.",
        ));
    }
    // The state it holds now: as sent, but for what this computer cannot
    // hold (an exec bit, a symbolic link held as a file); an edit made in
    // the meantime is not taken as synced.
    let recorded = {
        let (g, f, s) = (g.clone(), folder.clone(), sent.clone());
        tokio::task::spawn_blocking(move || match code::read_state(&g, &f) {
            Ok(c) if code::limits_only(&g, &f, &s, &c.state).unwrap_or(false) => c.state,
            _ => s,
        })
        .await
        .unwrap_or_else(|_| sent.clone())
    };
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
    let step_no = mf.step;
    let Some(q) = update(state, id, |q| {
        q.last = Some(Last {
            state: recorded.clone(),
            snapshot: Some(meta.snap.clone()),
            step: step_no,
            at: now,
            direction: Direction::In,
            files: files_changed,
        });
        q.conflict = None;
        q.reset_ok = false;
        q.status = Status::InSync { at: now };
    }) else {
        undo(carried).await;
        return Err(super::routes::unknown());
    };
    audit(
        state,
        "machine.sync.landed",
        &q,
        json!({
            "step": step_no,
            "files": files_changed,
            "ignored": if carried { mf.ignored.len() } else { 0 },
            "ignoredSkipped": ignored_skipped,
            "action": applied.action,
        }),
    );
    if carried {
        keep_replaced(state, id, &dir);
    }
    step::tidy(state, id, &folder, &dir).await;
    Ok(json!({ "landed": true, "state": recorded, "files": files_changed }))
}

/// How many sets of replaced ignored files a pair keeps.
const REPLACED_KEPT: usize = 10;

/// The files git ignores a step replaced here (this computer's own copies)
/// outlive the step: kept under `<app data>/machines/sync/<id>/replaced/
/// <time>`, the newest [`REPLACED_KEPT`].
fn keep_replaced(state: &AppState, id: &str, in_dir: &Path) {
    let from = in_dir.join(ignored::KEPT);
    let has_files = std::fs::read_dir(&from).is_ok_and(|mut d| d.next().is_some());
    let Some(base) = state.machines.sync.dir_of(id).map(|d| d.join("replaced")) else {
        return;
    };
    if !has_files {
        return;
    }
    if std::fs::create_dir_all(&base).is_err() {
        return;
    }
    crate::claude::transfer::private_dir(&base);
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%3f").to_string();
    if let Err(e) = std::fs::rename(&from, base.join(&stamp)) {
        tracing::warn!(error = %e, "files git ignores a step replaced could not be kept");
        return;
    }
    let mut sets: Vec<PathBuf> = std::fs::read_dir(&base)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    sets.sort();
    while sets.len() > REPLACED_KEPT {
        let _ = std::fs::remove_dir_all(sets.remove(0));
    }
}

// ─── the rest of a step ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AgreeBody {
    pub state: TreeState,
    #[serde(default)]
    pub step: u64,
}

/// `POST /peer/sync/{id}/agree`: both computers made the same changes:
/// recorded as synced when this checkout still holds them.
pub async fn agree(state: &AppState, from: &str, id: &str, b: AgreeBody) -> ApiResult<Value> {
    let p = active(state, id, from)?;
    b.state
        .validate()
        .map_err(|e| ApiError::unprocessable("sync_bad_step", e.to_string()))?;
    let lock = state.machines.sync.lock_of(id);
    let _g = lock
        .try_lock_owned()
        .map_err(|_| busy(&name_of(&p.folder)))?;
    let (g, f) = (git(state), PathBuf::from(&p.folder));
    let cur = tokio::task::spawn_blocking(move || code::read_state(&g, &f))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .map_err(|e| ApiError::conflict("sync_changed", e.to_string()))?;
    if cur.state != b.state {
        return Err(ApiError::conflict(
            "sync_changed",
            format!("{} changed here meanwhile.", name_of(&p.folder)),
        ));
    }
    let now = m::now();
    if let Some(q) = update(state, id, |q| {
        q.last = Some(Last {
            state: b.state.clone(),
            snapshot: None,
            step: b.step,
            at: now,
            direction: Direction::Both,
            files: 0,
        });
        q.conflict = None;
        q.reset_ok = false;
        q.status = Status::InSync { at: now };
    }) {
        audit(state, "machine.sync.agreed", &q, json!({ "step": b.step }));
    }
    Ok(json!({ "ok": true }))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PauseBody {
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub first: bool,
    /// This computer's changes, as the asker saw them.
    #[serde(default)]
    pub yours: Changes,
    /// The asker's own.
    #[serde(default)]
    pub mine: Changes,
}

/// What another computer says of a side's changes, its ids checked.
fn clean_changes(mut c: Changes) -> Changes {
    if c.branch.as_deref().is_some_and(|b| !code::valid_branch(b)) {
        c.branch = None;
    }
    if c.head.as_deref().is_some_and(|h| !code::is_oid(h)) {
        c.head = None;
    }
    c
}

/// `POST /peer/sync/{id}/pause`: the other computer met a conflict: this
/// side pauses too, with both sides its way round.
pub async fn pause(state: &AppState, from: &str, id: &str, b: PauseBody) -> ApiResult<Value> {
    let p = active(state, id, from)?;
    if b.reason != "conflict" {
        return Err(ApiError::unprocessable(
            "sync_bad_reason",
            "Only a conflict pauses both computers.",
        ));
    }
    let name = name_of(&p.folder);
    let detail = if b.first {
        format!("{name} differs on both computers: choose which one's to keep.")
    } else {
        format!("{name} changed on both computers: choose which one's to keep.")
    };
    let c = Conflict {
        at: m::now(),
        here: clean_changes(b.yours),
        there: clean_changes(b.mine),
        first: b.first,
    };
    if let Some(q) = update(state, id, |q| {
        q.conflict = Some(c.clone());
        q.status = Status::paused(Reason::Conflict, detail.clone());
    }) {
        audit(
            state,
            "machine.sync.conflict",
            &q,
            json!({ "here": c.here.files, "there": c.there.files, "first": c.first }),
        );
    }
    Ok(json!({ "paused": true }))
}

#[derive(Debug, Default, Deserialize)]
pub struct PushBody {
    /// The other computer's desktop kept this side in a conflict.
    #[serde(default)]
    pub resolved: bool,
}

/// `POST /peer/sync/{id}/push`: the other computer did not change and
/// this one did (or a resolution chose this side): a step from here now.
pub async fn push(state: &AppState, from: &str, id: &str, b: PushBody) -> ApiResult<Value> {
    let p = active(state, id, from)?;
    if b.resolved {
        audit(
            state,
            "machine.sync.resolved",
            &p,
            json!({ "keep": "here", "by": "there" }),
        );
    }
    let outcome = step::locked(state, id, Trigger::Now, true).await;
    let p = state
        .machines
        .sync
        .pair(id)
        .ok_or_else(super::routes::unknown)?;
    Ok(json!({ "outcome": outcome.as_str(), "status": p.status }))
}

/// `POST /peer/sync/{id}/lose`: a resolution chose the other computer's
/// state: this checkout's is kept under a ref and goes back to the synced
/// state (refused, with nothing changed, when anything could be lost).
pub async fn lose(state: &AppState, from: &str, id: &str) -> ApiResult<Value> {
    let p = active(state, id, from)?;
    if !matches!(p.status.reason(), Some(Reason::Conflict)) {
        return Err(ApiError::conflict(
            "sync_no_conflict",
            "It is not paused for a conflict here.",
        ));
    }
    let kept = step::lose_here(state, id).await?;
    Ok(json!({ "kept": {
        "ref": kept.ref_name,
        "commit": kept.commit,
        "files": kept.files,
    } }))
}

#[derive(Debug, Deserialize)]
pub struct ConfirmBody {
    /// The confirming computer's folder.
    pub folder: String,
}

/// `POST /peer/sync/{id}/confirm`: the other computer's desktop confirmed
/// what this one asked: it is on on both.
pub async fn confirm(state: &AppState, from: &str, id: &str, b: ConfirmBody) -> ApiResult<Value> {
    let p = pair_of(state, id, from)?;
    if !p.here_on {
        return Err(super::routes::unknown());
    }
    let folder = b.folder.trim().to_string();
    if !crate::claude::paths::is_absolute_anywhere(&folder) || folder.len() > 4096 {
        return Err(ApiError::unprocessable(
            "sync_bad_folder",
            "Not a folder's path.",
        ));
    }
    if let Some(q) = update(state, id, |q| {
        q.peer_on = true;
        q.there = folder.clone();
        if !matches!(q.status, Status::InSync { .. }) {
            q.status = Status::Syncing { since: m::now() };
        }
    }) {
        audit(
            state,
            "machine.sync.on",
            &q,
            json!({ "confirmedThere": true }),
        );
    }
    super::watch::refresh(state);
    Ok(json!({ "on": true }))
}

#[derive(Debug, Default, Deserialize)]
pub struct OffBody {
    /// `off` (turned off there) or `declined` (*Not now* there).
    #[serde(default)]
    pub reason: String,
}

/// `POST /peer/sync/{id}/off`: the other computer turned it off (or said
/// *Not now*): nothing syncs; the pair stays listed here, off, until it is
/// turned on again or removed.
pub async fn off(state: &AppState, from: &str, id: &str, b: OffBody) -> ApiResult<Value> {
    let Ok(p) = pair_of(state, id, from) else {
        // Asked, not answered yet: the request goes.
        crate::machines::setup::asks::withdraw_sync(state, from, id);
        return Ok(json!({ "off": true }));
    };
    let peer = m::peer_name(state, from);
    let detail = if b.reason == "declined" {
        format!("{peer} said not now.")
    } else {
        format!("Turned off on {peer}.")
    };
    if let Some(q) = update(state, id, |q| {
        q.peer_on = false;
        q.conflict = None;
        q.status = Status::Off {
            detail: Some(detail.clone()),
        };
    }) {
        audit(
            state,
            "machine.sync.off",
            &q,
            json!({ "by": "there", "reason": if b.reason == "declined" { "declined" } else { "off" } }),
        );
    }
    let _ = p;
    super::watch::refresh(state);
    Ok(json!({ "off": true }))
}
