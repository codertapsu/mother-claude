//! The source's half of keeping copies current (docs/PLAN-MACHINES.md
//! §5.9): which sessions keep a copy where, the turn watch (never in the
//! middle of a turn, five quiet seconds after one), and a **round** — offer
//! what it has, learn what the other computer holds, send what grew (a
//! blob in chunks), have it applied — with the copy's state for every
//! screen.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Weak;
use std::time::Duration;

use serde_json::{json, Value};

use super::check::{self, Mark};
use super::store::{PairStatus, Reason, State};
use super::wire::{self, Applied, Blob, FileNow, Holdings, Offer, Op, OpKind};
use super::{key, lock};
use crate::claude::home::ClaudeHome;
use crate::machines::chunks::{self, PutError};
use crate::machines::lease::{self, Lease};
use crate::machines::moves::wire::AccountAsk;
use crate::machines::{self as m, grants::Grant};
use crate::state::{AppState, Inner};

/// A session that ran this recently follows its project's choice.
const RECENT_MS: i64 = 30 * 60 * 1000;
/// The longest a paused copy waits before it is tried again.
const MAX_WAIT_MS: i64 = 2 * 60 * 1000;
/// How long the other computer may take to apply a round.
const APPLY_TIMEOUT: Duration = Duration::from_secs(10 * 60);

// ─── the session's files ───────────────────────────────────────────────────────

/// One of a session's files here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Local {
    pub path: PathBuf,
    /// Under the config directory, `/`-separated.
    pub rel: String,
    pub size: u64,
    pub mtime: i64,
    /// Append-only (`.jsonl`).
    pub log: bool,
}

/// A session's files and the cwd its transcript records first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scan {
    pub files: Vec<Local>,
    pub cwd: Option<String>,
}

impl Scan {
    /// Every file's bytes.
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// The same files, as large and as old: nothing wrote in between.
    pub fn same_as(&self, other: &Scan) -> bool {
        let a: BTreeMap<&str, (u64, i64)> = self
            .files
            .iter()
            .map(|f| (f.rel.as_str(), (f.size, f.mtime)))
            .collect();
        let b: BTreeMap<&str, (u64, i64)> = other
            .files
            .iter()
            .map(|f| (f.rel.as_str(), (f.size, f.mtime)))
            .collect();
        a == b
    }
}

/// The files a move packs of session `id` in `home` (`transfer::
/// plan_session`), each with its size and time. Blocking.
pub fn scan(home: &ClaudeHome, id: &str) -> Result<Scan, String> {
    let plan = crate::claude::transfer::plan_session(home, home, id).map_err(|e| e.to_string())?;
    let mut files = Vec::with_capacity(plan.files.len());
    for f in &plan.files {
        let Ok(rel) = f.from.strip_prefix(home.base()) else {
            continue;
        };
        let rel: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let rel = rel.join("/");
        if !super::allowed_path(&rel, id) {
            continue;
        }
        let meta = std::fs::metadata(&f.from).map_err(|e| format!("{rel}: {e}"))?;
        files.push(Local {
            log: rel.ends_with(".jsonl"),
            path: f.from.clone(),
            rel,
            size: meta.len(),
            mtime: super::mtime_ms(&meta),
        });
    }
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    if files.len() > super::MAX_FILES {
        return Err(format!(
            "It has more than {} files: too many to keep a copy of.",
            super::MAX_FILES
        ));
    }
    Ok(Scan {
        files,
        cwd: plan.cwd,
    })
}

/// The session's own transcript among its files (`projects/<dir>/<id>.jsonl`).
pub fn transcript_of<'a>(files: &'a [Local], id: &str) -> Option<&'a Local> {
    let name = format!("{id}.jsonl");
    files
        .iter()
        .filter(|f| {
            let parts: Vec<&str> = f.rel.split('/').collect();
            parts.len() == 3 && parts[0] == "projects" && parts[2] == name
        })
        .max_by_key(|f| f.mtime)
}

// ─── what travels ──────────────────────────────────────────────────────────────

/// What a round does with one file, before its bytes are read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// The bytes from `from` on, after what the holder has (`base`).
    Append { index: usize, from: u64, base: Mark },
    /// The whole file.
    Whole { index: usize },
    /// The source has it no more.
    Remove { path: String },
}

/// The steps that make the holder's copy what `locals` are (`mark_at`: this
/// computer's mark of a file at a size, read only when it can be used).
pub fn plan_steps(
    locals: &[Local],
    held: &[wire::HeldFile],
    mark_at: &dyn Fn(&Local, u64) -> Option<Mark>,
) -> Vec<Step> {
    let by_path: HashMap<&str, &wire::HeldFile> =
        held.iter().map(|h| (h.path.as_str(), h)).collect();
    let mut out = Vec::new();
    for (index, l) in locals.iter().enumerate() {
        let h = by_path.get(l.rel.as_str()).copied();
        // As it was copied (its size and its time): nothing to do.
        if h.is_some_and(|h| h.size == l.size && h.mtime == l.mtime) {
            continue;
        }
        if !l.log {
            out.push(Step::Whole { index });
            continue;
        }
        // A mark that is not of the size the holder says it has is no mark.
        let held_mark = h.and_then(|h| h.mark.as_ref().filter(|m| m.size == h.size));
        match check::plan(l.size, held_mark, |n| mark_at(l, n)) {
            // The same bytes, a new time: nothing travels, the time does.
            check::Plan::Same => out.push(Step::Append {
                index,
                from: l.size,
                base: held_mark.cloned().unwrap_or_default(),
            }),
            check::Plan::Append { from } => out.push(Step::Append {
                index,
                from,
                base: held_mark.cloned().unwrap_or_default(),
            }),
            check::Plan::Whole => out.push(Step::Whole { index }),
        }
    }
    let here: HashSet<&str> = locals.iter().map(|l| l.rel.as_str()).collect();
    for h in held {
        if !here.contains(h.path.as_str()) {
            out.push(Step::Remove {
                path: h.path.clone(),
            });
        }
    }
    out
}

/// A round's ops and its blob, read from the files as they are.
#[derive(Debug, Default)]
pub struct Built {
    pub ops: Vec<Op>,
    pub blob: Option<(PathBuf, Blob)>,
    pub appended: usize,
    pub whole: usize,
    pub removed: usize,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Reads what `steps` send into a blob at `out` (blocking): every part's
/// SHA-256, and the source's mark after an append. A file shorter than it
/// was is an error (it changed: the round is dropped).
pub fn build(locals: &[Local], steps: &[Step], out: &Path) -> std::io::Result<Built> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Seek, SeekFrom, Write};
    let mut built = Built::default();
    let needs_blob = steps.iter().any(|s| match s {
        Step::Append { index, from, .. } => locals[*index].size > *from,
        Step::Whole { .. } => true,
        Step::Remove { .. } => false,
    });
    let mut writer = None;
    if needs_blob {
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
            crate::claude::transfer::private_dir(parent);
        }
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        writer = Some((
            std::io::BufWriter::new(opts.open(out)?),
            Sha256::new(),
            0u64,
        ));
    }
    let mut buf = vec![0u8; 256 * 1024];
    for step in steps {
        let (l, from, base) = match step {
            Step::Remove { path } => {
                built.removed += 1;
                built.ops.push(Op {
                    path: path.clone(),
                    kind: OpKind::Remove,
                    ..Op::default()
                });
                continue;
            }
            Step::Append { index, from, base } => (&locals[*index], *from, Some(base.clone())),
            Step::Whole { index } => (&locals[*index], 0, None),
        };
        let len = l.size - from;
        let mut part = Sha256::new();
        let offset = writer.as_ref().map(|(_, _, n)| *n).unwrap_or(0);
        if len > 0 {
            let (w, whole, n) = writer
                .as_mut()
                .ok_or_else(|| std::io::Error::other("no blob"))?;
            let mut f = std::fs::File::open(&l.path)?;
            f.seek(SeekFrom::Start(from))?;
            let mut left = len;
            while left > 0 {
                let take = (left as usize).min(buf.len());
                f.read_exact(&mut buf[..take])?;
                part.update(&buf[..take]);
                whole.update(&buf[..take]);
                w.write_all(&buf[..take])?;
                left -= take as u64;
                *n += take as u64;
            }
        }
        let after = match base {
            Some(_) => Some(check::mark_of(&l.path, l.size).ok_or_else(|| {
                std::io::Error::other(format!("{} changed while it was read", l.rel))
            })?),
            None => None,
        };
        if base.is_some() {
            built.appended += 1;
        } else {
            built.whole += 1;
        }
        built.ops.push(Op {
            path: l.rel.clone(),
            kind: if base.is_some() {
                OpKind::Append
            } else {
                OpKind::Whole
            },
            offset,
            len,
            sha256: hex(&part.finalize()),
            base,
            after,
            mtime: l.mtime,
            log: l.log,
        });
    }
    if let Some((w, whole, n)) = writer {
        let file = w.into_inner().map_err(|e| e.into_error())?;
        crate::persist::sync_file(&file)?;
        drop(file);
        built.blob = Some((
            out.to_path_buf(),
            Blob {
                size: n,
                sha256: hex(&whole.finalize()),
            },
        ));
    }
    Ok(built)
}

// ─── the turn watch ────────────────────────────────────────────────────────────

/// Whether a session is quiet enough to be copied now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quiet {
    /// A turn runs (or background work it started).
    MidTurn,
    /// It changed, or its turn ended, less than the debounce ago.
    Settling,
    /// Quiet for the debounce: copy what changed.
    Yes,
}

/// One marked session's watch: whether a turn runs, since when it is quiet,
/// and which change (`gen`) each computer's copy has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Watch {
    busy: bool,
    quiet_since: Option<i64>,
    /// Its transcript's size and time, last seen.
    seen: Option<(u64, i64)>,
    /// Raised by every change seen (a turn, a write, a computer's ask).
    gen: u64,
    /// Computer → the change its copy has.
    copied: HashMap<String, u64>,
}

impl Default for Watch {
    fn default() -> Self {
        Self {
            busy: false,
            quiet_since: None,
            seen: None,
            // A new watch has something to copy everywhere.
            gen: 1,
            copied: HashMap::new(),
        }
    }
}

impl Watch {
    /// What the session is now: `busy` (the backup's rule), its transcript's
    /// size and time, at `now`, with `debounce` ms of quiet wanted.
    pub fn observe(
        &mut self,
        busy: bool,
        seen: Option<(u64, i64)>,
        now: i64,
        debounce: i64,
    ) -> Quiet {
        if busy {
            if !self.busy {
                self.gen += 1;
            }
            self.busy = true;
            self.quiet_since = None;
            self.seen = seen;
            return Quiet::MidTurn;
        }
        if self.busy {
            // The turn just ended.
            self.busy = false;
            self.gen += 1;
            self.quiet_since = Some(now);
        }
        if seen != self.seen {
            self.seen = seen;
            self.gen += 1;
            self.quiet_since = Some(now);
        }
        let since = *self.quiet_since.get_or_insert(now);
        if now - since >= debounce {
            Quiet::Yes
        } else {
            Quiet::Settling
        }
    }

    /// The change a round starting now copies.
    pub fn gen(&self) -> u64 {
        self.gen
    }

    /// `machine`'s copy lacks a change.
    pub fn due(&self, machine: &str) -> bool {
        self.copied.get(machine).copied().unwrap_or(0) < self.gen
    }

    /// `machine`'s copy has change `gen`.
    pub fn done(&mut self, machine: &str, gen: u64) {
        let e = self.copied.entry(machine.to_string()).or_insert(0);
        *e = (*e).max(gen);
    }

    /// `machine`'s copy should be looked at again (it came back, it asked
    /// for files again).
    pub fn again_for(&mut self, machine: &str) {
        self.copied.remove(machine);
    }

    /// Every copy should be looked at again.
    pub fn again(&mut self) {
        self.copied.clear();
    }
}

// ─── which sessions, where ─────────────────────────────────────────────────────

/// Where a session's copies come from: its own choice, or its project's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Session,
    Project,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Session => "session",
            Origin::Project => "project",
        }
    }
}

/// Every session this computer keeps a copy of on other computers, and
/// where: its own choice, else its project's (the sessions that ran
/// lately). Only paired computers.
pub async fn marked_now(state: &AppState) -> BTreeMap<String, (Vec<String>, Origin)> {
    let explicit = state.machines.copies.out_read(|f| f.sessions.clone());
    let paired: HashSet<String> = state
        .machines
        .store_read(|f| f.peers.iter().map(|p| p.id.clone()).collect());
    let mut out: BTreeMap<String, (Vec<String>, Origin)> = BTreeMap::new();
    for (sid, k) in &explicit {
        let ms: Vec<String> = k
            .machines
            .iter()
            .filter(|m| paired.contains(*m))
            .cloned()
            .collect();
        if !ms.is_empty() {
            out.insert(sid.clone(), (ms, Origin::Session));
        }
    }
    let any_project = state
        .machines
        .store_read(|f| f.projects.values().any(|p| !p.copies.is_empty()));
    if !any_project {
        return out;
    }
    let now = m::now();
    let recent: Vec<(String, String)> = {
        let snapshot = state.snapshot.read().await;
        snapshot
            .sessions
            .iter()
            .filter(|s| {
                !explicit.contains_key(&s.id)
                    && crate::claude::ids::is_session_id(&s.id)
                    && (s.live || s.last_activity_at.is_some_and(|t| now - t < RECENT_MS))
            })
            .map(|s| (s.id.clone(), s.cwd.clone()))
            .collect()
    };
    for (sid, cwd) in recent {
        let ms: Vec<String> = crate::machines::setup::options::effective(state, &cwd)
            .copies
            .into_iter()
            .filter(|m| paired.contains(m))
            .collect();
        if !ms.is_empty() {
            out.insert(sid, (ms, Origin::Project));
        }
    }
    out
}

/// Whether a session may be writing its files now (the backup's rule), read
/// from the session list as it is.
fn busy_now(state: &AppState, snapshot: &crate::server::ws::Snapshot, id: &str) -> bool {
    if !crate::claude::transfer::ops::is_live(state, id) {
        return false;
    }
    match snapshot.find(id) {
        Some(s) => crate::claude::transfer::ops::busy_status(s.status, s.activity.kind),
        None => true,
    }
}

/// The transcript's size and time, cheaply (the index knows where it is).
fn transcript_seen(state: &AppState, id: &str) -> Option<(u64, i64)> {
    let path = state.index.transcript_path(id)?;
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), super::mtime_ms(&meta)))
}

// ─── the loop ──────────────────────────────────────────────────────────────────

/// The copies loop: a look every [`super::TICK`] while the app runs.
pub(crate) async fn run_loop(state: Weak<Inner>) {
    let mut tick = tokio::time::interval(super::TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        let Some(st) = state.upgrade() else { break };
        pass(&st, false).await;
    }
}

/// One look at every marked session, with the rounds that are due run to
/// their end (tests drive copies with this).
pub async fn tick(state: &AppState) {
    pass(state, true).await;
}

/// One look: each marked session's turn watched; a round for each copy that
/// lacks its last change once the session is quiet (awaited when `wait`,
/// else in the background, one per copy at a time).
async fn pass(state: &AppState, wait: bool) {
    let copies = &state.machines.copies;
    let marked = marked_now(state).await;
    // Watches of sessions no longer marked go.
    lock(&copies.watches).retain(|sid, _| marked.contains_key(sid));
    if marked.is_empty() {
        return;
    }
    let now = m::now();
    if !state.machines.enabled() {
        for (sid, (ms, _)) in &marked {
            for mid in ms {
                set_status(
                    state,
                    sid,
                    mid,
                    paused(Reason::MachinesOff, "Machines is off on this computer."),
                );
            }
        }
        return;
    }
    let debounce = copies.debounce_ms();
    let mut due: Vec<(String, String, u64)> = Vec::new();
    {
        let snapshot = state.snapshot.read().await;
        for (sid, (ms, _)) in &marked {
            let busy = busy_now(state, &snapshot, sid);
            let seen = transcript_seen(state, sid);
            let mut watches = lock(&copies.watches);
            let w = watches.entry(sid.clone()).or_default();
            let quiet = w.observe(busy, seen, now, debounce);
            for mid in ms {
                if !w.due(mid) {
                    continue;
                }
                match quiet {
                    Quiet::Yes => due.push((sid.clone(), mid.clone(), w.gen())),
                    Quiet::MidTurn | Quiet::Settling => waiting(state, sid, mid),
                }
            }
        }
    }
    let retry = lock(&copies.retry).clone();
    let rounds = due
        .into_iter()
        .filter(|(sid, mid, _)| {
            retry
                .get(&(sid.clone(), mid.clone()))
                .is_none_or(|(at, _)| *at <= now)
        })
        .map(|(sid, mid, gen)| {
            let st = state.clone();
            async move { run_round(&st, &sid, &mid, gen).await }
        });
    if wait {
        futures_util::future::join_all(rounds).await;
    } else {
        for r in rounds {
            tokio::spawn(r);
        }
    }
}

/// A round and what its outcome means for the watch and the waits.
async fn run_round(state: &AppState, session: &str, machine: &str, gen: u64) {
    let copies = &state.machines.copies;
    let k = (session.to_string(), machine.to_string());
    // One at a time per copy: a round still going is left to finish.
    let lock_ = copies.lock_of("out", &key(machine, session));
    let Ok(_one) = lock_.try_lock() else {
        return;
    };
    let outcome = round(state, session, machine, false).await;
    let mut watches = lock(&copies.watches);
    match outcome {
        Outcome::Copied { complete: true, .. } => {
            lock(&copies.retry).remove(&k);
            if let Some(w) = watches.get_mut(session) {
                w.done(machine, gen);
            }
        }
        // Parts go again whole at the next look.
        Outcome::Copied {
            complete: false, ..
        }
        | Outcome::Changed
        | Outcome::Waiting => {}
        Outcome::Paused => {
            let mut retry = lock(&copies.retry);
            let tries = retry.get(&k).map(|(_, n)| *n).unwrap_or(0);
            let wait = (15_000i64 << tries.min(4)).min(MAX_WAIT_MS);
            retry.insert(k, (m::now() + wait, tries + 1));
        }
        Outcome::Skipped => {
            if let Some(w) = watches.get_mut(session) {
                w.done(machine, gen);
            }
        }
    }
}

// ─── states ────────────────────────────────────────────────────────────────────

fn paused(reason: Reason, detail: impl Into<String>) -> PairStatus {
    PairStatus {
        state: State::Paused,
        reason: Some(reason),
        detail: Some(detail.into()),
        ..PairStatus::default()
    }
}

/// Sets how a copy stands (keeping what it holds), and tells the screens —
/// only when that changed: the loop says the same thing every look.
pub(crate) fn set_status(state: &AppState, session: &str, machine: &str, next: PairStatus) {
    let merged = state.machines.copies.out_read(|f| {
        let cur = f.status_of(session, machine).cloned().unwrap_or_default();
        let mut merged = next.clone();
        if merged.copied_at.is_none() {
            merged.copied_at = cur.copied_at;
            merged.turn_at = cur.turn_at;
            merged.bytes = cur.bytes;
            merged.files = cur.files;
        }
        let same = f.status_of(session, machine).is_some()
            && cur.state == merged.state
            && cur.reason == merged.reason
            && cur.detail == merged.detail
            && cur.copied_at == merged.copied_at;
        (!same).then_some(merged)
    });
    let Some(mut merged) = merged else {
        return;
    };
    merged.since = m::now();
    state.machines.copies.out_write(|f| {
        f.status
            .entry(session.to_string())
            .or_default()
            .insert(machine.to_string(), merged);
    });
    super::publish(state);
}

/// A turn runs, or it just ended: the copy waits for it.
fn waiting(state: &AppState, session: &str, machine: &str) {
    let cur = state
        .machines
        .copies
        .out_read(|f| f.status_of(session, machine).map(|s| s.state));
    // A pause says more than a wait.
    if matches!(cur, Some(State::Paused | State::Copying)) {
        return;
    }
    set_status(
        state,
        session,
        machine,
        PairStatus {
            state: State::Waiting,
            detail: Some("It is copied once its turn ends.".into()),
            ..PairStatus::default()
        },
    );
}

// ─── a round ───────────────────────────────────────────────────────────────────

/// How a round ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The copy there is current (`complete`: every file went through).
    Copied { sent: u64, complete: bool },
    /// A turn runs: it goes once it ends.
    Waiting,
    /// Its files changed while they were read: it goes again once quiet.
    Changed,
    /// Something stops it (the copy's state says what).
    Paused,
    /// Nothing to do with this computer (no longer paired).
    Skipped,
}

/// The other computer's refusal of a round, as the copy's state.
fn refused(state: &AppState, machine: &str, status: u16, v: &Value) -> (Reason, String) {
    let name = m::peer_name(state, machine);
    let code = v
        .pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or("");
    let message = v
        .pointer("/error/message")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("{name} refused ({status})."));
    match (status, code) {
        (403, "machines_off") => (Reason::MachinesOff, format!("Machines is off on {name}.")),
        (403, "grant_missing") => (
            Reason::NoGrant,
            format!(
                "{name} has not given {} the Move permission: a copy is the first half of a move.",
                m::this_name(state)
            ),
        ),
        (404, "not_found") | (404, "") => (
            Reason::Unsupported,
            format!("{name}'s Mother Claude can't keep copies yet: update it there."),
        ),
        (409, "copy_lives_here") => (Reason::LivesElsewhere, message),
        (409, "copy_too_large") => (Reason::TooLarge, message),
        _ => (Reason::Error, message),
    }
}

/// Whether `machine` says it keeps copies (an older build, or one never
/// greeted, is greeted once first).
async fn supports(state: &AppState, machine: &str) -> bool {
    let has = |st: &AppState| {
        st.machines.store_read(|f| {
            f.peer(machine)
                .is_some_and(|p| p.capabilities.iter().any(|c| c == super::CAPABILITY))
        })
    };
    if has(state) {
        return true;
    }
    let Some(key) = state
        .machines
        .store_read(|f| f.peer(machine).map(|p| p.key.clone()))
    else {
        return false;
    };
    m::greet(state.clone(), machine.to_string(), key).await;
    has(state)
}

/// One round of the copy of `session` on `machine`: offer what is here,
/// learn what it holds, send what grew, have it applied. `for_move`: a move
/// of the session asks for it (it is frozen; the move gate is passed).
pub async fn round(state: &AppState, session: &str, machine: &str, for_move: bool) -> Outcome {
    let name = m::peer_name(state, machine);
    let pause = |reason: Reason, detail: String| {
        set_status(state, session, machine, paused(reason, detail));
        Outcome::Paused
    };
    if !state.machines.store_read(|f| f.peer(machine).is_some()) {
        return Outcome::Skipped;
    }
    if !state.machines.enabled() {
        return pause(
            Reason::MachinesOff,
            "Machines is off on this computer.".into(),
        );
    }
    if let Some(l) = lease::elsewhere(state, session) {
        let there = m::peer_name(state, &l.home);
        return pause(
            Reason::LivesElsewhere,
            format!("It lives on {there} now: its copies are kept from there."),
        );
    }
    if !for_move && state.machines.moves.moving_to(session).is_some() {
        return Outcome::Waiting;
    }
    // What it said last of the grants it gave this computer.
    let lan = state.machines.link(machine).path == Some(m::transport::PathKind::Lan);
    let their = state
        .machines
        .store_read(|f| f.peer(machine).and_then(|p| p.their_grants));
    if their.is_some_and(|g| !g.allows(Grant::Move, lan)) {
        let (r, d) = refused(
            state,
            machine,
            403,
            &json!({ "error": { "code": "grant_missing" } }),
        );
        return pause(r, d);
    }
    if !supports(state, machine).await {
        return pause(
            Reason::Unsupported,
            format!("{name}'s Mother Claude can't keep copies yet: update it there."),
        );
    }
    // Never in the middle of a turn.
    let (st, sid) = (state.clone(), session.to_string());
    let busy =
        tokio::task::spawn_blocking(move || crate::claude::transfer::ops::is_busy(&st, &sid))
            .await
            .unwrap_or(true);
    if busy {
        waiting(state, session, machine);
        return Outcome::Waiting;
    }
    set_status(
        state,
        session,
        machine,
        PairStatus {
            state: State::Copying,
            ..PairStatus::default()
        },
    );
    match round_inner(state, session, machine).await {
        Ok(o) => o,
        Err(Stop::Paused(reason, detail)) => pause(reason, detail),
        Err(Stop::Changed) => {
            set_status(
                state,
                session,
                machine,
                PairStatus {
                    state: State::Waiting,
                    reason: Some(Reason::Changed),
                    detail: Some(
                        "It changed while it was read: it goes again once it is quiet.".into(),
                    ),
                    ..PairStatus::default()
                },
            );
            Outcome::Changed
        }
    }
}

/// Why a round stopped.
enum Stop {
    Paused(Reason, String),
    Changed,
}

async fn round_inner(state: &AppState, session: &str, machine: &str) -> Result<Outcome, Stop> {
    let name = m::peer_name(state, machine);
    let account = crate::claude::summaries::acting_account(state, session);
    let home = state
        .accounts
        .home(&account)
        .unwrap_or_else(|| state.home.clone());
    let (h, sid) = (home.clone(), session.to_string());
    let before = tokio::task::spawn_blocking(move || scan(&h, &sid))
        .await
        .map_err(|e| Stop::Paused(Reason::Error, e.to_string()))?
        .map_err(|e| Stop::Paused(Reason::Error, e))?;
    if transcript_of(&before.files, session).is_none() {
        return Err(Stop::Paused(
            Reason::Error,
            "Nothing of this session is on disk yet.".into(),
        ));
    }
    // It lives here: its lease says so (made now if it had none), and the
    // other computer learns it with the offer.
    let me = m::self_id(state)
        .ok_or_else(|| Stop::Paused(Reason::Error, "This computer has no id.".into()))?;
    if state
        .machines
        .leases
        .get(session)
        .filter(|l| l.epoch > 0)
        .is_none()
    {
        state.machines.leases.set(
            session,
            Lease {
                home: me.clone(),
                epoch: 1,
                since: m::now(),
                move_id: None,
                note: None,
            },
            &me,
        );
    }
    let offer = Offer {
        files: before
            .files
            .iter()
            .map(|f| FileNow {
                path: f.rel.clone(),
                size: f.size,
                mtime: f.mtime,
                log: f.log,
            })
            .collect(),
        bytes: before.bytes(),
        lease: state.machines.leases.wire_of(session),
    };
    let body = serde_json::to_value(&offer).unwrap_or(Value::Null);
    let (status, v) = m::moves::call(
        state,
        machine,
        "POST",
        &format!("/peer/copies/{session}/offer"),
        Some(&body),
        "",
        Duration::from_secs(60),
    )
    .await
    .map_err(|e| Stop::Paused(Reason::Offline, format!("{name} can't be reached: {e}")))?;
    if status != 200 {
        let (r, d) = refused(state, machine, status, &v);
        return Err(Stop::Paused(r, d));
    }
    let holdings: Holdings = serde_json::from_value(v)
        .map_err(|e| Stop::Paused(Reason::Error, format!("{name} answered oddly: {e}")))?;
    // What travels, read from the files as they are; read again after: a
    // file that moved meanwhile drops the round.
    let round = super::new_round_id();
    let out = state
        .machines
        .copies
        .outgoing(&format!("{round}.bin"))
        .ok_or_else(|| Stop::Paused(Reason::Error, "This build has no data directory.".into()))?;
    let (h, sid, files, held, o) = (
        home.clone(),
        session.to_string(),
        before.files.clone(),
        holdings.files.clone(),
        out.clone(),
    );
    let built = tokio::task::spawn_blocking(move || -> Result<Built, Stop> {
        let mark_at = |l: &Local, n: u64| check::mark_of(&l.path, n);
        let steps = plan_steps(&files, &held, &mark_at);
        let built = build(&files, &steps, &o).map_err(|_| {
            let _ = std::fs::remove_file(&o);
            Stop::Changed
        })?;
        match scan(&h, &sid) {
            Ok(after)
                if after.same_as(&Scan {
                    files: files.clone(),
                    cwd: None,
                }) =>
            {
                Ok(built)
            }
            _ => {
                let _ = std::fs::remove_file(&o);
                Err(Stop::Changed)
            }
        }
    })
    .await
    .map_err(|e| Stop::Paused(Reason::Error, e.to_string()))??;
    let sent = built.blob.as_ref().map(|(_, b)| b.size).unwrap_or(0);
    let result = send_and_apply(state, session, machine, &round, &before, &built, &account).await;
    if let Some((p, _)) = &built.blob {
        let _ = std::fs::remove_file(p);
    }
    let applied = result?;
    let complete = applied.complete && applied.resend.is_empty();
    let turn_at = transcript_of(&before.files, session).map(|t| t.mtime);
    set_status(
        state,
        session,
        machine,
        PairStatus {
            state: State::Copied,
            reason: None,
            detail: (!complete)
                .then(|| "Part of it goes again whole at the next look.".to_string()),
            since: 0,
            copied_at: Some(m::now()),
            turn_at,
            bytes: applied.bytes,
            files: applied.files,
        },
    );
    super::audit(
        state,
        "machine.copy.sent",
        machine,
        session,
        json!({
            "round": round,
            "sent": sent,
            "appended": built.appended,
            "whole": built.whole,
            "removed": built.removed,
            "bytes": applied.bytes,
            "files": applied.files,
            "resend": applied.resend.len(),
        }),
    );
    for k in &applied.evicted {
        if let Some((_, s)) = k.split_once('/') {
            set_status(
                state,
                s,
                machine,
                paused(
                    Reason::TooLarge,
                    format!("{name} made room for newer copies: this one goes again after its next turn."),
                ),
            );
        }
    }
    Ok(Outcome::Copied { sent, complete })
}

/// Pushes the round's blob in chunks and has it applied.
async fn send_and_apply(
    state: &AppState,
    session: &str,
    machine: &str,
    round: &str,
    before: &Scan,
    built: &Built,
    account: &str,
) -> Result<Applied, Stop> {
    let name = m::peer_name(state, machine);
    if let Some((path, blob)) = &built.blob {
        let mut offset = 0u64;
        while offset < blob.size {
            let n = ((blob.size - offset) as usize).min(chunks::CHUNK);
            let p = path.clone();
            let bytes = tokio::task::spawn_blocking(move || chunks::read_chunk(&p, offset, n))
                .await
                .map_err(|e| Stop::Paused(Reason::Error, e.to_string()))?
                .map_err(|e| Stop::Paused(Reason::Error, e.to_string()))?;
            let at = format!(
                "/peer/copies/{session}/rounds/{round}?offset={offset}&total={}",
                blob.size
            );
            offset = match chunks::put(state, machine, &at, bytes, "").await {
                Ok(size) => size,
                Err(PutError::Refused(status, v)) => {
                    let (r, d) = refused(state, machine, status, &v);
                    return Err(Stop::Paused(r, d));
                }
                Err(PutError::Odd) => {
                    return Err(Stop::Paused(
                        Reason::Error,
                        format!("{name} answered oddly."),
                    ))
                }
                Err(PutError::Offline(e)) => {
                    return Err(Stop::Paused(
                        Reason::Offline,
                        format!("{name} went away while it was sent: {e}"),
                    ))
                }
            };
        }
    }
    let summary = state.find_session(session).await;
    let acc = state.accounts.get(account);
    let apply = wire::Apply {
        account: AccountAsk {
            name: acc.map(|a| a.name).unwrap_or_default(),
            is_default: account == crate::claude::accounts::DEFAULT_ID,
        },
        account_id: account.to_string(),
        cwd: summary
            .as_ref()
            .map(|s| s.cwd.clone())
            .filter(|c| !c.is_empty())
            .or_else(|| before.cwd.clone())
            .unwrap_or_default(),
        title: summary
            .as_ref()
            .map(|s| s.title.clone().unwrap_or_else(|| s.name.clone()))
            .filter(|t| !t.trim().is_empty())
            .map(|t| t.chars().take(200).collect()),
        settings: m::moves::settings_of(state, session, summary.as_ref()),
        turn_at: transcript_of(&before.files, session)
            .map(|t| t.mtime)
            .unwrap_or(0),
        bytes: before.bytes(),
        files: before.files.len(),
        blob: built.blob.as_ref().map(|(_, b)| b.clone()),
        ops: built.ops.clone(),
    };
    let body = serde_json::to_value(&apply).unwrap_or(Value::Null);
    let (status, v) = m::moves::call(
        state,
        machine,
        "POST",
        &format!("/peer/copies/{session}/rounds/{round}/apply"),
        Some(&body),
        "",
        APPLY_TIMEOUT,
    )
    .await
    .map_err(|e| {
        Stop::Paused(
            Reason::Offline,
            format!("{name} did not say it kept it: {e}"),
        )
    })?;
    if status != 200 {
        let (r, d) = refused(state, machine, status, &v);
        return Err(Stop::Paused(r, d));
    }
    serde_json::from_value(v)
        .map_err(|e| Stop::Paused(Reason::Error, format!("{name} answered oddly: {e}")))
}

// ─── a move ────────────────────────────────────────────────────────────────────

/// One file a move lands from a copy: its path, size and SHA-256 here.
pub type Listed = (String, u64, String);

/// A move of `session` to `machine`, which holds a copy of it: the copy is
/// brought current (the session is frozen), then every file here is read
/// whole for its SHA-256 — what the other computer checks its copy against
/// before it lands from it. `None` when the copy can't be used (the move
/// then sends the conversation itself).
pub async fn for_move(state: &AppState, session: &str, machine: &str) -> Option<Vec<Listed>> {
    let held = state
        .machines
        .copies
        .out_read(|f| f.status_of(session, machine).and_then(|s| s.copied_at))
        .is_some();
    if !held {
        return None;
    }
    let lock_ = state.machines.copies.lock_of("out", &key(machine, session));
    let _one = lock_.lock().await;
    match round(state, session, machine, true).await {
        Outcome::Copied { complete: true, .. } => {}
        other => {
            tracing::info!(
                session,
                ?other,
                "a move sends the conversation: its copy is not current"
            );
            return None;
        }
    }
    let account = crate::claude::summaries::acting_account(state, session);
    let home = state
        .accounts
        .home(&account)
        .unwrap_or_else(|| state.home.clone());
    let sid = session.to_string();
    tokio::task::spawn_blocking(move || -> Option<Vec<Listed>> {
        let before = scan(&home, &sid).ok()?;
        let mut out = Vec::with_capacity(before.files.len());
        for f in &before.files {
            let (size, sha) = crate::claude::transfer::sha256_file(&f.path).ok()?;
            out.push((f.rel.clone(), size, sha));
        }
        // Nothing wrote while it was read.
        let after = scan(&home, &sid).ok()?;
        after.same_as(&before).then_some(out)
    })
    .await
    .ok()
    .flatten()
}

// ─── views ─────────────────────────────────────────────────────────────────────

/// The computers a session keeps a copy on, and why (its own choice, else
/// its project's): `(machines, explicit, project's)`.
pub async fn choice_of(state: &AppState, session: &str) -> (Vec<String>, bool, Vec<String>) {
    let explicit = state
        .machines
        .copies
        .out_read(|f| f.sessions.get(session).map(|k| k.machines.clone()));
    let cwd = state
        .find_session(session)
        .await
        .map(|s| s.cwd)
        .unwrap_or_default();
    let project = if cwd.is_empty() {
        Vec::new()
    } else {
        crate::machines::setup::options::effective(state, &cwd).copies
    };
    match explicit {
        Some(ms) => (ms, true, project),
        None => (project.clone(), false, project),
    }
}

/// `GET /api/sessions/{id}/copies`: every paired computer, whether this
/// session keeps a copy there (and why), and how that copy stands.
pub async fn session_view(state: &AppState, session: &str) -> Value {
    let (on, explicit, project) = choice_of(state, session).await;
    let peers: Vec<(String, String, String, Vec<String>)> = state.machines.store_read(|f| {
        f.peers
            .iter()
            .map(|p| {
                (
                    p.id.clone(),
                    p.display_name().to_string(),
                    p.os.clone(),
                    p.capabilities.clone(),
                )
            })
            .collect()
    });
    let lives = lease::elsewhere(state, session).map(|l| l.home);
    let machines: Vec<Value> = peers
        .iter()
        .map(|(id, name, os, caps)| {
            let status = state
                .machines
                .copies
                .out_read(|f| f.status_of(session, id).cloned());
            let is_on = on.contains(id);
            json!({
                "id": id,
                "name": name,
                "os": os,
                "online": state.machines.link(id).online,
                "on": is_on,
                "from": if !is_on { Value::Null } else if explicit { json!("session") } else { json!("project") },
                "inProject": project.contains(id),
                "supported": caps.is_empty() || caps.iter().any(|c| c == super::CAPABILITY),
                "status": status,
            })
        })
        .collect();
    json!({
        "session": session,
        "explicit": explicit,
        "project": project,
        "livesOn": lives,
        "enabled": state.machines.enabled(),
        "machines": machines,
    })
}

/// The copies this computer keeps on other computers (`kept` of `GET
/// /api/machines/copies`).
pub async fn kept_view(state: &AppState) -> Vec<Value> {
    let marked = marked_now(state).await;
    let status = state.machines.copies.out_read(|f| f.status.clone());
    let mut out = Vec::new();
    for (sid, (ms, from)) in &marked {
        let title = state.find_session(sid).await.map(|s| {
            s.title
                .clone()
                .unwrap_or_else(|| s.name.clone())
                .chars()
                .take(200)
                .collect::<String>()
        });
        for mid in ms {
            let st = status.get(sid).and_then(|p| p.get(mid)).cloned();
            out.push(json!({
                "id": format!("kept.{mid}.{sid}"),
                "machine": mid,
                "machineName": m::peer_name(state, mid),
                "online": state.machines.link(mid).online,
                "session": sid,
                "title": title,
                "from": from.as_str(),
                "status": st,
            }));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(rel: &str, size: u64, mtime: i64, log: bool) -> Local {
        Local {
            path: PathBuf::from(rel),
            rel: rel.into(),
            size,
            mtime,
            log,
        }
    }

    fn held(path: &str, size: u64, mtime: i64, mark: Option<Mark>) -> wire::HeldFile {
        wire::HeldFile {
            path: path.into(),
            size,
            mtime,
            mark,
        }
    }

    /// A turn ends, then five quiet seconds: copied. A turn that starts
    /// meanwhile, or a write, waits again; a computer's copy that has the
    /// last change is not due.
    #[test]
    fn a_copy_waits_for_the_turn_and_five_quiet_seconds() {
        let d = 5_000;
        let mut w = Watch::default();
        assert!(w.due("T"), "a new watch has everything to copy");
        // Quiet from the start: after the debounce.
        assert_eq!(w.observe(false, Some((10, 1)), 0, d), Quiet::Settling);
        assert_eq!(w.observe(false, Some((10, 1)), 4_999, d), Quiet::Settling);
        assert_eq!(w.observe(false, Some((10, 1)), 5_000, d), Quiet::Yes);
        let gen = w.gen();
        w.done("T", gen);
        assert!(!w.due("T"));
        assert!(w.due("U"), "another computer's copy is still due");
        // A turn: never in the middle of one.
        assert_eq!(w.observe(true, Some((20, 2)), 6_000, d), Quiet::MidTurn);
        assert!(w.due("T"));
        assert_eq!(w.observe(true, Some((30, 3)), 20_000, d), Quiet::MidTurn);
        // It ended: the debounce starts then.
        assert_eq!(w.observe(false, Some((30, 3)), 21_000, d), Quiet::Settling);
        // A write in the quiet time starts it again.
        assert_eq!(w.observe(false, Some((40, 4)), 25_000, d), Quiet::Settling);
        assert_eq!(w.observe(false, Some((40, 4)), 29_999, d), Quiet::Settling);
        assert_eq!(w.observe(false, Some((40, 4)), 30_000, d), Quiet::Yes);
        let gen = w.gen();
        w.done("T", gen);
        // A change during the round is due again after it.
        assert_eq!(w.observe(false, Some((50, 5)), 31_000, d), Quiet::Settling);
        w.done("T", gen);
        assert!(w.due("T"));
        // A computer back online is looked at again.
        let mut w = Watch::default();
        w.done("T", w.gen());
        w.again_for("T");
        assert!(w.due("T"));
        // No debounce (tests): at once.
        let mut w = Watch::default();
        assert_eq!(w.observe(false, None, 7, 0), Quiet::Yes);
    }

    #[test]
    fn only_what_grew_travels_and_doubt_sends_the_whole_file() {
        let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let mark = |n: u64| check::mark_of_bytes(&data, n);
        let t = "projects/-a/s.jsonl";
        let sub = "projects/-a/s/subagents/agent-1.jsonl";
        let todo = "todos/s.json";
        let locals = vec![
            local(t, 300_000, 9, true),
            local(sub, 100, 5, true),
            local(todo, 30, 9, false),
        ];
        let mark_at = |l: &Local, n: u64| {
            if l.rel == t {
                mark(n)
            } else {
                None
            }
        };
        // Nothing there: everything whole.
        let steps = plan_steps(&locals, &[], &mark_at);
        assert_eq!(
            steps,
            [
                Step::Whole { index: 0 },
                Step::Whole { index: 1 },
                Step::Whole { index: 2 }
            ]
        );
        // The transcript grew; the subagent is as it was copied; the todo
        // list changed; something there is gone here.
        let held_files = vec![
            held(t, 200_000, 8, mark(200_000)),
            held(sub, 100, 5, None),
            held(todo, 30, 8, None),
            held("todos/s-old.json", 3, 1, None),
        ];
        let steps = plan_steps(&locals, &held_files, &mark_at);
        assert_eq!(
            steps,
            [
                Step::Append {
                    index: 0,
                    from: 200_000,
                    base: mark(200_000).unwrap()
                },
                Step::Whole { index: 2 },
                Step::Remove {
                    path: "todos/s-old.json".into()
                },
            ]
        );
        // What it holds is no start of the file here: whole.
        let mut wrong = held_files.clone();
        wrong[0].mark = check::mark_of_bytes(b"something else entirely", 23)
            .map(|m| Mark { size: 200_000, ..m });
        let steps = plan_steps(&locals, &wrong, &mark_at);
        assert_eq!(steps[0], Step::Whole { index: 0 });
        // Longer there than here (truncated here): whole.
        let mut more = data.clone();
        more.push(b'\n');
        let mut longer = held_files.clone();
        longer[0] = held(t, 300_001, 8, check::mark_of_bytes(&more, 300_001));
        assert_eq!(
            plan_steps(&locals, &longer, &mark_at)[0],
            Step::Whole { index: 0 }
        );
        // A mark of another size than the file it names: whole.
        longer[0] = held(t, 300_001, 8, mark(300_000));
        assert_eq!(
            plan_steps(&locals, &longer, &mark_at)[0],
            Step::Whole { index: 0 }
        );
        // The same bytes, a new time: only the time travels.
        let mut same = held_files.clone();
        same[0] = held(t, 300_000, 1, mark(300_000));
        assert_eq!(
            plan_steps(&locals, &same, &mark_at)[0],
            Step::Append {
                index: 0,
                from: 300_000,
                base: mark(300_000).unwrap()
            }
        );
    }

    #[test]
    fn a_blob_holds_the_parts_and_their_hashes() {
        let tmp = tempfile::tempdir().unwrap();
        let t = tmp.path().join("t.jsonl");
        let o = tmp.path().join("todo.json");
        let data = b"{\"a\":1}\n".repeat(1000);
        std::fs::write(&t, &data).unwrap();
        std::fs::write(&o, b"[]").unwrap();
        let locals = vec![
            Local {
                path: t.clone(),
                rel: "projects/-a/s.jsonl".into(),
                size: data.len() as u64,
                mtime: 1,
                log: true,
            },
            Local {
                path: o.clone(),
                rel: "todos/s.json".into(),
                size: 2,
                mtime: 1,
                log: false,
            },
        ];
        let base = check::mark_of_bytes(&data, 4000).unwrap();
        let steps = vec![
            Step::Append {
                index: 0,
                from: 4000,
                base: base.clone(),
            },
            Step::Whole { index: 1 },
            Step::Remove {
                path: "todos/s-old.json".into(),
            },
        ];
        let out = tmp.path().join("out").join("r.bin");
        let built = build(&locals, &steps, &out).unwrap();
        let (path, blob) = built.blob.clone().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len() as u64, blob.size);
        assert_eq!(blob.sha256, crate::claude::secrets::sha256_hex(&bytes));
        assert_eq!(&bytes[..data.len() - 4000], &data[4000..]);
        assert_eq!(&bytes[data.len() - 4000..], b"[]");
        let a = &built.ops[0];
        assert_eq!((a.kind, a.offset, a.len), (OpKind::Append, 0, 4000));
        assert_eq!(a.base.as_ref(), Some(&base));
        assert_eq!(a.after, check::mark_of_bytes(&data, data.len() as u64));
        assert_eq!(a.sha256, crate::claude::secrets::sha256_hex(&data[4000..]));
        let w = &built.ops[1];
        assert_eq!((w.kind, w.offset, w.len), (OpKind::Whole, 4000, 2));
        assert_eq!(built.ops[2].kind, OpKind::Remove);
        assert_eq!((built.appended, built.whole, built.removed), (1, 1, 1));
        // A file shorter than it was read: an error (the round is dropped).
        std::fs::write(&o, b"[").unwrap();
        assert!(build(&locals, &steps, &tmp.path().join("r2.bin")).is_err());
    }

    #[test]
    fn two_scans_agree_only_when_nothing_moved() {
        let a = Scan {
            files: vec![local("x", 1, 1, true), local("y", 2, 2, false)],
            cwd: None,
        };
        let mut b = a.clone();
        assert!(a.same_as(&b));
        b.files[1].mtime = 3;
        assert!(!a.same_as(&b));
        b.files[1].mtime = 2;
        b.files.pop();
        assert!(!a.same_as(&b));
        assert_eq!(a.bytes(), 3);
        let id = "0f1e2d3c-4b5a-4968-8776-655443322110";
        let files = vec![
            local(
                &format!("projects/-a/{id}/subagents/agent-1.jsonl"),
                1,
                9,
                true,
            ),
            local(&format!("projects/-a/{id}.jsonl"), 1, 1, true),
            local(&format!("projects/-b/{id}.jsonl"), 1, 5, true),
        ];
        assert_eq!(
            transcript_of(&files, id).map(|l| l.rel.as_str()),
            Some(format!("projects/-b/{id}.jsonl").as_str()),
            "the one written last"
        );
    }
}
