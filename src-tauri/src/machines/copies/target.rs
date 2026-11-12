//! The holder's half of a copy (docs/PLAN-MACHINES.md §5.9): answering an
//! offer with what it holds (nothing written), taking a round's blob in
//! chunks, applying it — an append only onto the very prefix both sides
//! agree on, a whole file only as its SHA-256 says — keeping the cap, and
//! the two ways a copy leaves the data dir for a config directory: a move
//! that lands from it, and *Continue here from the copy* (a take over while
//! the session's home is offline). Both land as a pull does
//! (`session_pack::land`: the folder mapped through the roots, a conflict
//! copy beside a session that went on here, an undo folder).
//!
//! Everything the source sends is checked before the file system sees it:
//! ids, every path (one of the session's own files, `backup::
//! allowed_entry`), sizes, offsets, hashes, marks, settings.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::check::{self, Mark};
use super::store::{self, FileState, Held, Table};
use super::wire::{Applied, CopyThere, HeldFile, Holdings, Offer, Op, OpKind};
use super::{allowed_path, is_account_dir, is_sha256, join_rel, key};
use crate::claude::backup::session_pack::{self, LandInputs, Landed};
use crate::machines::chunks::{self, AppendError, Refusal};
use crate::machines::lease;
use crate::machines::moves::note::{self, Note};
use crate::machines::moves::{self, Carried, LandingPart};
use crate::machines::{self as m, protocol};
use crate::server::context::ClientContext;
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

fn bad(message: impl Into<String>) -> ApiError {
    ApiError::unprocessable("invalid_copy", message)
}

/// The cap of the copies held for one source computer.
pub fn cap(state: &AppState) -> u64 {
    state.machines.settings().copies_cap_bytes
}

/// Whether the session's lease names this computer (its copy is never
/// taken away by the cap).
fn lives_here(state: &AppState, session: &str) -> bool {
    let me = m::self_id(state).unwrap_or_default();
    lease::lease_of(state, session).is_some_and(|l| l.epoch > 0 && l.home == me)
}

/// Creates `dir` and every folder up to it below `base`, owner only.
fn make_dirs(base: &Path, dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut d = dir.to_path_buf();
    loop {
        crate::claude::transfer::private_dir(&d);
        if d == base || !d.pop() || !d.starts_with(base) {
            break;
        }
    }
    Ok(())
}

// ─── an offer ──────────────────────────────────────────────────────────────────

fn check_files(session: &str, files: &[super::wire::FileNow]) -> ApiResult<()> {
    if files.len() > super::MAX_FILES {
        return Err(bad("Too many files for a copy."));
    }
    let mut seen = std::collections::HashSet::new();
    for f in files {
        if !allowed_path(&f.path, session) || f.size > super::MAX_FILE || !seen.insert(&f.path) {
            return Err(bad(format!(
                "{:?} is not one of this session's files.",
                f.path
            )));
        }
    }
    Ok(())
}

/// `POST /peer/copies/{session}/offer`: what this computer holds of the
/// copy (nothing is written; the session's lease there is learnt).
pub async fn offer(
    state: &AppState,
    from: &str,
    lan: bool,
    session: &str,
    offer: Offer,
) -> ApiResult<Holdings> {
    if !crate::claude::ids::is_session_id(session) {
        return Err(bad("Not a session id."));
    }
    check_files(session, &offer.files)?;
    let me = m::self_id(state).unwrap_or_default();
    let here = m::this_name(state);
    // It lives here, and the other computer knows no newer home: no copy.
    if let Some(l) = lease::lease_of(state, session).filter(|l| l.epoch > 0 && l.home == me) {
        let newer = offer.lease.as_ref().is_some_and(|w| {
            w.session == session && lease::wins(w.epoch, &w.home, l.epoch, &l.home)
        });
        if !newer {
            return Err(ApiError::conflict(
                "copy_lives_here",
                format!("This session lives on {here} now: it is not copied there."),
            ));
        }
    }
    if let Some(w) = offer.lease.as_ref().filter(|w| w.session == session) {
        let changed = lease::accept(state, from, lan, std::slice::from_ref(w)).await;
        moves::on_leases_changed(state, &changed).await;
    }
    let cap = cap(state);
    if offer.bytes > cap {
        return Err(ApiError::conflict(
            "copy_too_large",
            format!(
                "It is larger ({}) than {here} keeps of {}'s copies ({}).",
                human(offer.bytes),
                m::peer_name(state, from),
                human(cap)
            ),
        )
        .with("cap", cap));
    }
    prune_incoming(state, from);
    let held = state
        .machines
        .copies
        .held_read(|i| i.copies.get(&key(from, session)).cloned());
    let files = match &held {
        Some(h) => {
            let dir = state
                .machines
                .copies
                .copy_dir(from, &h.account_dir, session)
                .ok_or_else(|| ApiError::internal("No data directory."))?;
            let d = dir.clone();
            let sid = session.to_string();
            let table = tokio::task::spawn_blocking(move || store::load_table(&d, &sid))
                .await
                .map_err(|e| ApiError::internal(e.to_string()))?;
            table
                .into_iter()
                .map(|(path, f)| HeldFile {
                    path,
                    size: f.size,
                    mtime: f.mtime,
                    mark: f.mark,
                })
                .collect()
        }
        None => Vec::new(),
    };
    Ok(Holdings {
        files,
        cap,
        copied_at: held.map(|h| h.copied_at).filter(|t| *t > 0),
    })
}

/// A size as people read it.
pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1000.0 && u < UNITS.len() - 1 {
        v /= 1000.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} bytes")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

/// Blobs of rounds that never finished, older than an hour, go.
fn prune_incoming(state: &AppState, from: &str) {
    let Some(dir) = state
        .machines
        .copies
        .root()
        .map(|r| r.join(".incoming").join(from))
    else {
        return;
    };
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t < cutoff);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

// ─── a round's bytes ───────────────────────────────────────────────────────────

/// `PUT /peer/copies/{session}/rounds/{round}?offset=N&total=T`: the next
/// chunk of a round's blob, appended when `offset` is what this computer has
/// (else `409` with what it has).
pub async fn put_chunk(
    state: &AppState,
    from: &str,
    session: &str,
    round: &str,
    offset: u64,
    total: u64,
    bytes: axum::body::Bytes,
) -> ApiResult<Value> {
    if !crate::claude::ids::is_session_id(session) || !super::is_round_id(round) {
        return Err(bad("Not a copy's round."));
    }
    if total > cap(state).saturating_add(64 << 20) {
        return Err(ApiError::conflict(
            "copy_too_large",
            "This round is larger than this computer keeps of yours.",
        ));
    }
    if bytes.len() > chunks::CHUNK {
        return Err(bad("A chunk is at most 8 MiB."));
    }
    let path = state
        .machines
        .copies
        .incoming(from, round)
        .ok_or_else(|| ApiError::internal("No data directory."))?;
    let _one = state.machines.copies.chunks.lock().await;
    let p = path.clone();
    let written = tokio::task::spawn_blocking(move || {
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
            crate::claude::transfer::private_dir(parent);
        }
        chunks::append(&p, offset, &bytes, total)
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| match e {
        AppendError::Refused(Refusal::Offset(size)) => ApiError::conflict(
            "offset_mismatch",
            "This computer has another part of the round.",
        )
        .with("size", size),
        AppendError::Refused(Refusal::TooLong) => bad("The round is longer than it said."),
        AppendError::Refused(Refusal::TooLarge) => bad("A chunk is at most 8 MiB."),
        AppendError::Io(e) => ApiError::internal(format!("It could not be written here: {e}")),
    })?;
    Ok(json!({ "size": written }))
}

// ─── applying a round ──────────────────────────────────────────────────────────

fn check_apply(session: &str, a: &super::wire::Apply) -> ApiResult<()> {
    if !is_account_dir(&a.account_id)
        || a.account.name.chars().count() > 100
        || a.cwd.len() > 4096
        || (!a.cwd.is_empty() && !crate::claude::paths::is_absolute_anywhere(&a.cwd))
        || a.title.as_deref().is_some_and(|t| t.len() > 1000)
        || a.ops.len() > super::MAX_FILES
        || a.files > super::MAX_FILES
    {
        return Err(bad(
            "The copy names an account or a folder this computer can't take.",
        ));
    }
    moves::check_settings(&a.settings)?;
    let blob = a.blob.as_ref().map(|b| b.size).unwrap_or(0);
    if a.blob.as_ref().is_some_and(|b| !is_sha256(&b.sha256)) {
        return Err(bad("The round's blob has no hash."));
    }
    let mut seen = std::collections::HashSet::new();
    for op in &a.ops {
        if !allowed_path(&op.path, session) || !seen.insert(&op.path) {
            return Err(bad(format!(
                "{:?} is not one of this session's files.",
                op.path
            )));
        }
        let fits = op.offset.checked_add(op.len).is_some_and(|end| end <= blob);
        match op.kind {
            OpKind::Remove => {}
            OpKind::Whole => {
                if !fits || !is_sha256(&op.sha256) || op.len > super::MAX_FILE {
                    return Err(bad(format!("{} came without its bytes.", op.path)));
                }
            }
            OpKind::Append => {
                let (Some(base), Some(after)) = (&op.base, &op.after) else {
                    return Err(bad(format!("{} came without its marks.", op.path)));
                };
                if !fits
                    || !is_sha256(&op.sha256)
                    || !base.well_formed()
                    || !after.well_formed()
                    || base.size.checked_add(op.len) != Some(after.size)
                    || after.size > super::MAX_FILE
                {
                    return Err(bad(format!("{} came with marks that don't fit.", op.path)));
                }
            }
        }
    }
    Ok(())
}

/// Copies `len` bytes of `blob` from `offset` into `w`, hashing them.
fn copy_part(
    blob: &mut std::fs::File,
    offset: u64,
    len: u64,
    w: &mut impl std::io::Write,
) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Seek, SeekFrom};
    blob.seek(SeekFrom::Start(offset))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut left = len;
    while left > 0 {
        let take = (left as usize).min(buf.len());
        blob.read_exact(&mut buf[..take])?;
        h.update(&buf[..take]);
        w.write_all(&buf[..take])?;
        left -= take as u64;
    }
    w.flush()?;
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Appends an op's bytes onto the file the holder has, only when it has the
/// prefix both sides agree on; the result must have the source's mark.
fn apply_append(file: &Path, op: &Op, blob: Option<&mut std::fs::File>) -> Result<Mark, String> {
    let (Some(base), Some(after)) = (&op.base, &op.after) else {
        return Err("no marks".into());
    };
    let len = std::fs::metadata(file)
        .map(|m| m.len())
        .map_err(|_| "missing")?;
    if len != base.size || check::mark_of(file, base.size).as_ref() != Some(base) {
        return Err("not the prefix it was said to be".into());
    }
    if op.len > 0 {
        let blob = blob.ok_or("no bytes")?;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(file)
            .map_err(|e| e.to_string())?;
        let sha = copy_part(blob, op.offset, op.len, &mut f).map_err(|e| e.to_string())?;
        crate::persist::sync_file(&f).map_err(|e| e.to_string())?;
        drop(f);
        if sha != op.sha256 {
            truncate(file, base.size);
            return Err("its bytes are not what was sent".into());
        }
    }
    if check::mark_of(file, after.size).as_ref() != Some(after) {
        truncate(file, base.size);
        return Err("the result is not what the source has".into());
    }
    Ok(after.clone())
}

fn truncate(file: &Path, size: u64) {
    if let Ok(f) = std::fs::OpenOptions::new().write(true).open(file) {
        let _ = f.set_len(size);
    }
}

/// Writes an op's bytes as the whole file (a temp file beside it, checked,
/// synced, renamed over it).
fn apply_whole(base: &Path, file: &Path, op: &Op, blob: &mut std::fs::File) -> Result<(), String> {
    let parent = file.parent().ok_or("no folder")?;
    make_dirs(base, parent).map_err(|e| e.to_string())?;
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = parent.join(format!(".{name}.mc-{}.tmp", uuid::Uuid::new_v4().simple()));
    let written = (|| -> Result<(), String> {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp).map_err(|e| e.to_string())?;
        let sha = copy_part(blob, op.offset, op.len, &mut f).map_err(|e| e.to_string())?;
        crate::persist::sync_file(&f).map_err(|e| e.to_string())?;
        drop(f);
        if sha != op.sha256 {
            return Err("its bytes are not what was sent".into());
        }
        std::fs::rename(&tmp, file).map_err(|e| e.to_string())
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// Applies a round's ops to the copy in `dir` (blocking): each file on its
/// own; one that does not fit is taken out of the table (the next round
/// sends it whole) and named in the answer.
fn apply_ops(dir: &Path, table: &mut Table, blob: Option<&Path>, ops: &[Op]) -> Vec<String> {
    let mut resend = Vec::new();
    let mut blob = blob.and_then(|p| std::fs::File::open(p).ok());
    for op in ops {
        let file = join_rel(dir, &op.path);
        let result = match op.kind {
            OpKind::Remove => {
                let _ = std::fs::remove_file(&file);
                table.remove(&op.path);
                continue;
            }
            OpKind::Append => apply_append(&file, op, blob.as_mut()).map(|mark| FileState {
                size: mark.size,
                mtime: op.mtime,
                mark: Some(mark),
                sha256: None,
            }),
            OpKind::Whole => match blob.as_mut() {
                None => Err("no bytes".to_string()),
                Some(b) => apply_whole(dir, &file, op, b).map(|()| FileState {
                    size: op.len,
                    mtime: op.mtime,
                    mark: if op.log {
                        check::mark_of(&file, op.len)
                    } else {
                        None
                    },
                    sha256: Some(op.sha256.clone()),
                }),
            },
        };
        match result {
            Ok(state) => {
                crate::persist::restrict(&file);
                table.insert(op.path.clone(), state);
            }
            Err(why) => {
                tracing::info!(path = %op.path, why, "a copy's file goes again whole");
                table.remove(&op.path);
                resend.push(op.path.clone());
            }
        }
    }
    resend
}

/// `POST /peer/copies/{session}/rounds/{round}/apply`: the round's blob is
/// checked whole, then each file applied; the index and the cap follow.
pub async fn apply(
    state: &AppState,
    from: &str,
    session: &str,
    round: &str,
    body: super::wire::Apply,
) -> ApiResult<Applied> {
    if !crate::claude::ids::is_session_id(session) || !super::is_round_id(round) {
        return Err(bad("Not a copy's round."));
    }
    check_apply(session, &body)?;
    let copies = &state.machines.copies;
    let k = key(from, session);
    let lock_ = copies.lock_of("in", &k);
    let _one = lock_.lock().await;
    let blob_path = copies
        .incoming(from, round)
        .ok_or_else(|| ApiError::internal("No data directory."))?;
    let result = apply_locked(state, from, session, &blob_path, &body).await;
    let _ = std::fs::remove_file(&blob_path);
    let _ = std::fs::remove_file(chunks::part_of(&blob_path));
    result
}

async fn apply_locked(
    state: &AppState,
    from: &str,
    session: &str,
    blob_path: &Path,
    body: &super::wire::Apply,
) -> ApiResult<Applied> {
    let copies = &state.machines.copies;
    let k = key(from, session);
    if copies.take_fail("apply") {
        return Err(ApiError::internal("A test stopped the copy here."));
    }
    // The blob, whole and as announced.
    let blob = match &body.blob {
        Some(b) => {
            let (p, size, sha) = (blob_path.to_path_buf(), b.size, b.sha256.clone());
            tokio::task::spawn_blocking(move || chunks::finish(&p, size, &sha))
                .await
                .map_err(|e| ApiError::internal(e.to_string()))?
                .map_err(|why| {
                    ApiError::conflict(
                        "copy_incomplete",
                        format!("The round did not arrive whole: {why}."),
                    )
                })?;
            Some(blob_path.to_path_buf())
        }
        None => {
            if body.ops.iter().any(|o| o.len > 0) {
                return Err(bad("The round's bytes are missing."));
            }
            None
        }
    };
    // Its folder: the source's account's (moved there when that changed).
    let dir = copies
        .copy_dir(from, &body.account_id, session)
        .ok_or_else(|| ApiError::internal("No data directory."))?;
    let root = copies
        .root()
        .ok_or_else(|| ApiError::internal("No data directory."))?;
    let before = copies.held_read(|i| i.copies.get(&k).cloned());
    if let Some(old) = before
        .as_ref()
        .filter(|h| h.account_dir != body.account_id)
        .and_then(|h| copies.copy_dir(from, &h.account_dir, session))
    {
        if old.exists() && !dir.exists() {
            if let Some(parent) = dir.parent() {
                make_dirs(&root, parent).map_err(|e| ApiError::internal(e.to_string()))?;
            }
            std::fs::rename(&old, &dir).map_err(|e| ApiError::internal(e.to_string()))?;
        }
    }
    let (d, r, sid, ops) = (
        dir.clone(),
        root.clone(),
        session.to_string(),
        body.ops.clone(),
    );
    let (table, resend) =
        tokio::task::spawn_blocking(move || -> std::io::Result<(Table, Vec<String>)> {
            make_dirs(&r, &d)?;
            let mut table = store::load_table(&d, &sid);
            let resend = apply_ops(&d, &mut table, blob.as_deref(), &ops);
            store::save_table(&d, &table)?;
            Ok((table, resend))
        })
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .map_err(|e| ApiError::internal(format!("The copy could not be written here: {e}")))?;
    let now = m::now();
    let complete = resend.is_empty();
    let bytes: u64 = table.values().map(|f| f.size).sum();
    let transcript = table
        .iter()
        .filter(|(p, _)| {
            let parts: Vec<&str> = p.split('/').collect();
            parts.len() == 3 && parts[0] == "projects" && parts[2] == format!("{session}.jsonl")
        })
        .max_by_key(|(_, f)| f.mtime)
        .and_then(|(_, f)| f.mark.clone());
    let held = copies.held_write(|i| {
        let e = i.copies.entry(k.clone()).or_insert_with(|| Held {
            created_at: now,
            ..Held::default()
        });
        e.machine = from.to_string();
        e.session = session.to_string();
        e.account = body.account.clone();
        e.account_dir = body.account_id.clone();
        e.title = body.title.clone();
        e.cwd = body.cwd.clone();
        e.settings = body.settings.clone();
        e.bytes = bytes;
        e.files = table.len();
        e.transcript = transcript;
        e.turn_at = body.turn_at;
        if complete {
            e.copied_at = now;
        }
        e.complete = complete;
        e.clone()
    });
    let evicted = enforce_cap(state, from, &k).await;
    super::audit(
        state,
        "machine.copy.received",
        from,
        session,
        json!({
            "bytes": held.bytes,
            "files": held.files,
            "ops": body.ops.len(),
            "sent": body.blob.as_ref().map(|b| b.size).unwrap_or(0),
            "resend": resend.len(),
            "evicted": evicted.len(),
        }),
    );
    super::publish(state);
    Ok(Applied {
        copied_at: now,
        resend,
        bytes: held.bytes,
        files: held.files,
        complete,
        evicted,
    })
}

/// Keeps the copies of `from` within the cap: the one refreshed longest ago
/// first, never one whose session lives here, never `keep`.
async fn enforce_cap(state: &AppState, from: &str, keep: &str) -> Vec<String> {
    let cap = cap(state);
    let copies: Vec<Held> = state.machines.copies.held_read(|i| {
        i.copies
            .values()
            .filter(|h| h.machine == from)
            .cloned()
            .collect()
    });
    let candidates: Vec<store::Candidate> = copies
        .iter()
        .map(|h| store::Candidate {
            key: key(&h.machine, &h.session),
            bytes: h.bytes,
            copied_at: h.copied_at,
            protected: lives_here(state, &h.session),
        })
        .collect();
    let mut out = Vec::new();
    for k in store::to_evict(&candidates, cap, keep) {
        let Some(h) = copies.iter().find(|h| key(&h.machine, &h.session) == k) else {
            continue;
        };
        // A copy in the middle of its own round is left to the next one.
        let lock_ = state.machines.copies.lock_of("in", &k);
        let Ok(_busy) = lock_.try_lock() else {
            continue;
        };
        remove_copy(state, &h.machine, &h.session).await;
        super::audit(
            state,
            "machine.copy.evicted",
            &h.machine,
            &h.session,
            json!({ "bytes": h.bytes, "cap": cap }),
        );
        out.push(k);
    }
    out
}

/// Takes a copy away (its folder and its entry).
async fn remove_copy(state: &AppState, machine: &str, session: &str) -> Option<Held> {
    let copies = &state.machines.copies;
    let k = key(machine, session);
    let held = copies.held_write(|i| i.copies.remove(&k))?;
    if let Some(dir) = copies.copy_dir(machine, &held.account_dir, session) {
        let _ = tokio::task::spawn_blocking(move || std::fs::remove_dir_all(dir)).await;
    }
    Some(held)
}

/// `GET /peer/copies/{session}`: whether this computer holds a copy.
pub fn state_of(state: &AppState, from: &str, session: &str) -> CopyThere {
    match state
        .machines
        .copies
        .held_read(|i| i.copies.get(&key(from, session)).cloned())
    {
        Some(h) => CopyThere {
            held: true,
            copied_at: Some(h.copied_at).filter(|t| *t > 0),
            turn_at: Some(h.turn_at).filter(|t| *t > 0),
            bytes: h.bytes,
            files: h.files,
            complete: h.complete,
        },
        None => CopyThere::default(),
    }
}

/// `DELETE /peer/copies/{session}`: the source keeps no copy here any more.
pub async fn dropped(state: &AppState, from: &str, session: &str) -> Value {
    let lock_ = state.machines.copies.lock_of("in", &key(from, session));
    let _one = lock_.lock().await;
    let gone = remove_copy(state, from, session).await;
    if let Some(h) = &gone {
        super::audit(
            state,
            "machine.copy.dropped",
            from,
            session,
            json!({ "bytes": h.bytes }),
        );
        super::publish(state);
    }
    json!({ "dropped": gone.is_some() })
}

/// `DELETE /api/machines/copies/held.<machine>.<session>`: a copy held here
/// goes (its source copies it again after its next turn unless it stops).
pub async fn remove_here(
    state: &AppState,
    ctx: &ClientContext,
    machine: &str,
    session: &str,
) -> ApiResult<()> {
    let lock_ = state.machines.copies.lock_of("in", &key(machine, session));
    let _one = lock_.lock().await;
    let held = remove_copy(state, machine, session)
        .await
        .ok_or_else(|| ApiError::not_found("copy_not_found", "No such copy here."))?;
    state
        .devices
        .audit
        .record(crate::server::devices::audit::AuditEntry {
            device_id: Some(machine.to_string()),
            device_name: Some(m::peer_name(state, machine)),
            detail: Some(json!({ "session": session, "bytes": held.bytes })),
            ..crate::server::devices::audit::AuditEntry::from_ctx(ctx, "machine.copy.removed")
        });
    super::publish(state);
    Ok(())
}

/// The copies held here (`held` of `GET /api/machines/copies`).
pub fn held_view(state: &AppState) -> Vec<Value> {
    let enabled = state.machines.enabled();
    let mut all: Vec<Held> = state
        .machines
        .copies
        .held_read(|i| i.copies.values().cloned().collect());
    all.sort_by_key(|h| std::cmp::Reverse(h.copied_at.max(h.created_at)));
    all.iter()
        .map(|h| {
            let online = state.machines.link(&h.machine).online;
            let here = lives_here(state, &h.session);
            json!({
                "id": format!("held.{}.{}", h.machine, h.session),
                "machine": h.machine,
                "machineName": m::peer_name(state, &h.machine),
                "online": online,
                "session": h.session,
                "title": h.title,
                "cwd": h.cwd,
                "account": h.account.name,
                "bytes": h.bytes,
                "files": h.files,
                "copiedAt": Some(h.copied_at).filter(|t| *t > 0),
                "turnAt": Some(h.turn_at).filter(|t| *t > 0),
                "createdAt": h.created_at,
                "complete": h.complete,
                "livesHere": here,
                "canContinue": enabled && !online && !here,
            })
        })
        .collect()
}

// ─── leaving the data dir ──────────────────────────────────────────────────────

/// A move of a session this computer holds a copy of lands from it: exactly
/// the files the source listed (each checked by size and SHA-256), packed
/// into `out` as the session's package.
pub async fn pack_for_move(
    state: &AppState,
    from: &str,
    session: &str,
    files: &[(String, u64, String)],
    out: &Path,
) -> ApiResult<()> {
    let copies = &state.machines.copies;
    let held = copies
        .held_read(|i| i.copies.get(&key(from, session)).cloned())
        .ok_or_else(|| {
            ApiError::conflict(
                "move_copy_missing",
                "This computer has no copy of it any more: try the move again.",
            )
        })?;
    let dir = copies
        .copy_dir(from, &held.account_dir, session)
        .ok_or_else(|| ApiError::internal("No data directory."))?;
    let lock_ = copies.lock_of("in", &key(from, session));
    let _one = lock_.lock().await;
    let (d, sid, list, cwd, o) = (
        dir,
        session.to_string(),
        files.to_vec(),
        held.cwd.clone(),
        out.to_path_buf(),
    );
    tokio::task::spawn_blocking(move || {
        session_pack::pack_listed(
            &d,
            &sid,
            &list,
            Some(&cwd).filter(|c| !c.is_empty()).map(String::as_str),
            &o,
        )
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map(|_| ())
    .map_err(|e| {
        ApiError::conflict(
            "move_copy_differs",
            format!("This computer's copy is not what the session has: {e}"),
        )
    })
}

/// What a take over from a copy landed.
#[derive(Clone, Debug)]
pub struct TakenOver {
    pub account: String,
    pub cwd: String,
    pub settings: Carried,
    pub landing: LandingPart,
    pub copied_at: Option<i64>,
    pub complete: bool,
    pub warnings: Vec<String>,
}

/// *Continue here from the copy* (decision 6): the copy of `session` that
/// `from` (its home, offline) kept here lands in this computer's config
/// directory as a pull lands a session — under the account of the same
/// name, in the folder the roots map its folder to (or `folder`, chosen),
/// undo kept — with the move note for its next message. The caller moves
/// the lease and resumes it.
pub async fn land_for_take_over(
    state: &AppState,
    from: &str,
    session: &str,
    folder: Option<&str>,
) -> ApiResult<TakenOver> {
    let copies = &state.machines.copies;
    let k = key(from, session);
    let held = copies
        .held_read(|i| i.copies.get(&k).cloned())
        .ok_or_else(|| {
            ApiError::conflict("no_copy", "This computer keeps no copy of this session.")
        })?;
    let here = m::this_name(state);
    let (account, name) = moves::resolve_account(state, &held.account)
        .map_err(|c| ApiError::conflict(&c.code, c.message.clone()))?;
    moves::account_ready(state, &account, &name)
        .map_err(|c| ApiError::conflict(&c.code, c.message.clone()))?;
    let home = state
        .accounts
        .home(&account)
        .unwrap_or_else(|| state.home.clone());
    let found = moves::folder::folder_there(state, from, &held.cwd, folder, None)
        .map_err(|e| ApiError::unprocessable("invalid_folder", e.0))?;
    let Some(folder) = found.path else {
        return Err(ApiError::conflict(
            "folder_elsewhere",
            format!(
                "{here} has no folder for {}: choose its folder here.",
                held.cwd
            ),
        )
        .with("candidates", json!(found.candidates)));
    };
    if !crate::claude::owned::trusted_in(
        &home.claude_json(),
        &home.projects_dir(),
        Path::new(&folder),
    ) {
        return Err(ApiError::conflict(
            "folder_untrusted",
            format!(
                "Claude Code on {here} doesn't trust {folder} yet: open it once with Claude Code here, then try again."
            ),
        ));
    }
    if crate::claude::transfer::ops::is_live(state, session) {
        return Err(ApiError::conflict(
            "session_live",
            format!("{here} runs this session right now."),
        ));
    }
    let dir = copies
        .copy_dir(from, &held.account_dir, session)
        .ok_or_else(|| ApiError::internal("No data directory."))?;
    let undo_root = crate::claude::transfer::ops::undo_root(state)
        .ok_or_else(|| ApiError::internal("No data directory."))?;
    let package = copies
        .outgoing(&format!("takeover-{session}.tar.gz"))
        .ok_or_else(|| ApiError::internal("No data directory."))?;
    let _job = tokio::time::timeout(std::time::Duration::from_secs(60), state.jobs.lock.lock())
        .await
        .map_err(|_| {
            ApiError::conflict(
                "data_job_running",
                "A backup, restore or transfer is running here; try again when it has finished.",
            )
        })?;
    let lock_ = copies.lock_of("in", &k);
    let _one = lock_.lock().await;
    let job = crate::claude::transfer::new_job_id();
    let host = crate::claude::backup::host_name();
    let project_dir = crate::claude::paths::project_dir_name(&folder);
    let (d, sid, pkg, h, ur, j, acc) = (
        dir,
        session.to_string(),
        package.clone(),
        home.clone(),
        undo_root,
        job.clone(),
        account.clone(),
    );
    let landed = tokio::task::spawn_blocking(move || {
        let r = (|| {
            session_pack::pack(
                &crate::claude::home::ClaudeHome::for_account_dir(&d),
                &sid,
                &pkg,
                &[],
            )?;
            session_pack::land(&LandInputs {
                archive: &pkg,
                id: &sid,
                home: &h,
                project_dir: Some(&project_dir),
                undo_root: &ur,
                job_id: &j,
                trust_key: None,
                host: &host,
                account: Some(&acc),
            })
        })();
        let _ = std::fs::remove_file(&pkg);
        r
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::conflict(e.code(), format!("The copy could not land here: {e}")))?;
    let (report, _) = landed;
    if state
        .accounts
        .get(&account)
        .is_some_and(|a| a.kind == crate::claude::accounts::AccountKind::Token)
    {
        state.accounts.remember_session(session, &account);
    }
    // The note for its next message: where it was, where it goes on.
    let me = m::self_id(state).unwrap_or_default();
    let from_os = state
        .machines
        .store_read(|f| f.peer(from).map(|p| p.os.clone()))
        .unwrap_or_default();
    let mut text = note::text(&Note {
        from_os: &from_os,
        from_path: &held.cwd,
        to_os: protocol::this_os(),
        to_path: &folder,
        worktree_of: None,
        as_copy: false,
        recipe: &[],
    });
    text.push_str(
        " The code here is this computer's project folder as it is: changes made on the other \
         computer since the last move are not here.",
    );
    state.machines.leases.set_note(session, text, &me);
    crate::claude::transfer::ops::rediscover(state).await;
    let mut warnings = Vec::new();
    if !held.complete {
        warnings.push(
            "The last copy did not finish: part of the last turn may be missing.".to_string(),
        );
    }
    let landing = LandingPart {
        job: Some(job),
        action: match &report.landed {
            Landed::New => "new",
            Landed::FastForward => "fastForward",
            Landed::Same => "same",
            Landed::Conflict { .. } => "conflict",
        }
        .to_string(),
        conflict: match &report.landed {
            Landed::Conflict { copy } => Some(copy.display().to_string()),
            _ => None,
        },
        trusted: false,
        transcript_bytes: held.transcript.as_ref().map(|t| t.size).unwrap_or(0),
        transcript_sha256: String::new(),
    };
    if let Landed::Conflict { copy } = &report.landed {
        warnings.push(format!(
            "It had gone on on {here} too: those turns are kept beside it ({}).",
            copy.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        ));
    }
    warnings.extend(report.warnings);
    super::audit(
        state,
        "machine.copy.landed",
        from,
        session,
        json!({ "action": landing.action, "bytes": held.bytes }),
    );
    Ok(TakenOver {
        account,
        cwd: folder,
        settings: held.settings.clone(),
        landing,
        copied_at: Some(held.copied_at).filter(|t| *t > 0),
        complete: held.complete,
        warnings,
    })
}

/// The copy of `session` held here, from whichever computer kept it (the
/// newest).
pub fn held_of(state: &AppState, session: &str) -> Option<Held> {
    state.machines.copies.held_read(|i| {
        i.copies
            .values()
            .filter(|h| h.session == session)
            .max_by_key(|h| h.copied_at)
            .cloned()
    })
}

/// The copy's folder, for tests and the take over.
pub fn dir_of(state: &AppState, machine: &str, session: &str) -> Option<PathBuf> {
    let h = state
        .machines
        .copies
        .held_read(|i| i.copies.get(&key(machine, session)).cloned())?;
    state
        .machines
        .copies
        .copy_dir(machine, &h.account_dir, session)
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: &str = "0f1e2d3c-4b5a-4968-8776-655443322110";

    fn op_whole(path: &str, offset: u64, bytes: &[u8], log: bool) -> Op {
        Op {
            path: path.into(),
            kind: OpKind::Whole,
            offset,
            len: bytes.len() as u64,
            sha256: crate::claude::secrets::sha256_hex(bytes),
            base: None,
            after: None,
            mtime: 7,
            log,
        }
    }

    /// Whole files, then an append onto the agreed prefix; an append onto
    /// another prefix, or bytes that are not what was said, are refused and
    /// the file goes again whole.
    #[test]
    fn a_round_appends_only_onto_the_agreed_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("copy");
        let t = format!("projects/-a/{S}.jsonl");
        let todo = format!("todos/{S}.json");
        let first = b"{\"n\":1}\n".repeat(100);
        let blob1 = tmp.path().join("b1");
        let mut b = first.clone();
        b.extend_from_slice(b"[1]");
        std::fs::write(&blob1, &b).unwrap();
        let mut table = Table::new();
        let resend = apply_ops(
            &dir,
            &mut table,
            Some(&blob1),
            &[
                op_whole(&t, 0, &first, true),
                op_whole(&todo, first.len() as u64, b"[1]", false),
            ],
        );
        assert!(resend.is_empty(), "{resend:?}");
        assert_eq!(std::fs::read(join_rel(&dir, &t)).unwrap(), first);
        assert_eq!(
            table[&t].mark,
            check::mark_of_bytes(&first, first.len() as u64)
        );
        assert_eq!(table[&todo].sha256.as_deref().map(str::len), Some(64));

        // A turn later: the new bytes alone.
        let more = b"{\"n\":2}\n".repeat(50);
        let mut whole = first.clone();
        whole.extend_from_slice(&more);
        let blob2 = tmp.path().join("b2");
        std::fs::write(&blob2, &more).unwrap();
        let append = Op {
            path: t.clone(),
            kind: OpKind::Append,
            offset: 0,
            len: more.len() as u64,
            sha256: crate::claude::secrets::sha256_hex(&more),
            base: check::mark_of_bytes(&first, first.len() as u64),
            after: check::mark_of_bytes(&whole, whole.len() as u64),
            mtime: 8,
            log: true,
        };
        let resend = apply_ops(
            &dir,
            &mut table,
            Some(&blob2),
            std::slice::from_ref(&append),
        );
        assert!(resend.is_empty());
        assert_eq!(std::fs::read(join_rel(&dir, &t)).unwrap(), whole);
        assert_eq!(table[&t].size, whole.len() as u64);

        // The same append again: the copy is no longer at that prefix.
        let resend = apply_ops(
            &dir,
            &mut table,
            Some(&blob2),
            std::slice::from_ref(&append),
        );
        assert_eq!(resend, std::slice::from_ref(&t));
        assert!(!table.contains_key(&t), "it goes again whole");
        assert_eq!(
            std::fs::read(join_rel(&dir, &t)).unwrap(),
            whole,
            "nothing was appended"
        );

        // Bytes that are not what was said: put back as they were.
        std::fs::write(join_rel(&dir, &t), &first).unwrap();
        let mut lie = append.clone();
        lie.sha256 = crate::claude::secrets::sha256_hex(b"other");
        let resend = apply_ops(&dir, &mut table, Some(&blob2), &[lie]);
        assert_eq!(resend, std::slice::from_ref(&t));
        assert_eq!(std::fs::read(join_rel(&dir, &t)).unwrap(), first);

        // A file the source has no more goes.
        let resend = apply_ops(
            &dir,
            &mut table,
            None,
            &[Op {
                path: todo.clone(),
                kind: OpKind::Remove,
                ..Op::default()
            }],
        );
        assert!(resend.is_empty());
        assert!(!join_rel(&dir, &todo).exists());
        assert!(!table.contains_key(&todo));
    }

    /// Everything a round names is checked before the file system sees it:
    /// its paths, offsets in the blob, hashes and marks, its account and
    /// folder.
    #[test]
    fn a_round_that_names_anything_else_is_refused_whole() {
        use super::super::wire::{Apply, Blob};
        let t = format!("projects/-a/{S}.jsonl");
        let mark = |n: u64| Mark {
            size: n,
            check: "a".repeat(64),
        };
        let good = Apply {
            account_id: "default".into(),
            cwd: if cfg!(windows) {
                r"D:\dev\app".into()
            } else {
                "/Users/m/app".into()
            },
            blob: Some(Blob {
                size: 10,
                sha256: "b".repeat(64),
            }),
            ops: vec![Op {
                path: t.clone(),
                kind: OpKind::Append,
                offset: 0,
                len: 10,
                sha256: "c".repeat(64),
                base: Some(mark(5)),
                after: Some(mark(15)),
                mtime: 1,
                log: true,
            }],
            ..Apply::default()
        };
        assert!(check_apply(S, &good).is_ok());
        let refused = |f: &dyn Fn(&mut Apply)| {
            let mut a = good.clone();
            f(&mut a);
            check_apply(S, &a).unwrap_err().code
        };
        assert_eq!(
            refused(&|a| a.ops[0].path = "settings.json".into()),
            "invalid_copy"
        );
        assert_eq!(
            refused(&|a| a.ops[0].path = "projects/-a/../../x.jsonl".into()),
            "invalid_copy"
        );
        assert_eq!(
            refused(&|a| a.ops[0].offset = 1),
            "invalid_copy",
            "past the blob"
        );
        assert_eq!(refused(&|a| a.ops[0].base = None), "invalid_copy");
        assert_eq!(
            refused(&|a| a.ops[0].after = Some(mark(16))),
            "invalid_copy"
        );
        assert_eq!(refused(&|a| a.ops[0].sha256 = "x".into()), "invalid_copy");
        assert_eq!(refused(&|a| a.account_id = "../up".into()), "invalid_copy");
        assert_eq!(refused(&|a| a.cwd = "relative/app".into()), "invalid_copy");
        assert_eq!(
            refused(&|a| a.ops.push(a.ops[0].clone())),
            "invalid_copy",
            "a file twice"
        );
        assert_eq!(
            refused(&|a| a.blob.as_mut().unwrap().sha256 = "nope".into()),
            "invalid_copy"
        );
        // A removal names a file and nothing more.
        let mut removal = good.clone();
        removal.blob = None;
        removal.ops = vec![Op {
            path: format!("todos/{S}.json"),
            kind: OpKind::Remove,
            ..Op::default()
        }];
        assert!(check_apply(S, &removal).is_ok());
    }

    #[test]
    fn sizes_read_as_people_read_them() {
        assert_eq!(human(12), "12 bytes");
        assert_eq!(human(2_000_000_000), "2.0 GB");
        assert_eq!(human(1_500), "1.5 KB");
    }
}
