//! Keeping copies current (docs/PLAN-MACHINES.md §5.9, Phase 3b): a session
//! marked **Keep a copy on DESKTOP-PC** — itself, or every session of its
//! project — is copied there after each turn, so a move is instant and the
//! other computer can go on with it while this one sleeps.
//!
//! **Off by default; opt-in** per session (`PUT /api/sessions/{id}/copies`)
//! or per project (its Machines options, `setup::options`). The target must
//! have given this computer its `move` grant (a copy is the first half of a
//! move) and Machines must be on on both: otherwise, or while the other
//! computer is offline, the copy **pauses** and says why — nothing is
//! queued; the next round after that sends what grew meanwhile.
//!
//! **When** (`source`): after a turn ends — the session is idle or only
//! waits for a wakeup, the backup's rule (`transfer::ops::is_busy`) — and
//! five quiet seconds later; never in the middle of one: the session's files
//! are read twice around a round and a round whose files moved meanwhile is
//! dropped (it goes again once the session is quiet).
//!
//! **What**: what grew since the target's last acknowledgement, among the
//! files a move packs (`transfer::plan_session`: the transcript, its
//! subagents' and workflows' files, file-history snapshots, todos, the
//! session's environment). Append-only files (`.jsonl`) travel as the bytes
//! appended, once both sides agree on the prefix the target holds
//! ([`check`]: its size and the SHA-256 of its first and last 64 KiB —
//! truncation or a rewrite sends the whole file); other files travel whole
//! when their size or time changed. One round is a *blob* of those bytes,
//! pushed in chunks (`machines::chunks`), each part checked by SHA-256 there.
//!
//! **Where** (`target`): `<app data>/machines/copies/<source>/<account>/
//! <session>/`, laid out like a config directory (`projects/<dir>/<id>.jsonl`,
//! `file-history/<id>/…`), 0700 folders and 0600 files, with
//! `copies/index.json` naming every copy (its sizes, its transcript's mark,
//! the last turn's time, the source's folder) and `<copy>/.mc/files.json` what
//! it holds of each file. A size cap per source computer (`copiesCapBytes`,
//! 2 GiB): the copy refreshed longest ago goes first — never one of a session
//! whose lease names this computer. **A copy is never written into a config
//! directory** but by a move's or a take over's landing, as a pull writes it
//! (`session_pack::land`: undo kept, the folder mapped through the roots).
//!
//! **Moves use it** (`moves::source`): when the target holds a copy, a move
//! brings it current, checks every file's SHA-256 there and lands from it —
//! the conversation itself does not travel again. **Continue here from the
//! copy** (`moves::source::take_over`): while the session's home is offline,
//! its copy lands here, the lease takes epoch + 1 and it goes on as an owned
//! session; the home's own turns meanwhile become a conflict copy when it
//! next moves there.
//!
//! Every round is in the audit log of both computers (`machine.copy.*`: the
//! session, the sizes, never a byte of content), and every change is a
//! `{t:"machines", what:"copies"}` frame.

pub mod check;
pub mod routes;
pub mod source;
pub mod store;
pub mod target;
pub mod wire;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::server::devices::audit::AuditEntry;
use crate::state::AppState;

/// The capability a computer needs to keep (and take) copies.
pub const CAPABILITY: &str = "copies";
/// How long a session stays quiet after a turn before it is copied.
pub const DEBOUNCE: Duration = Duration::from_secs(5);
/// How often the copies loop looks at the marked sessions.
pub const TICK: Duration = Duration::from_secs(2);
/// The default size cap of the copies held for one source computer.
pub const DEFAULT_CAP: u64 = 2 * 1024 * 1024 * 1024;
/// The smallest and largest caps a user may set.
pub const MIN_CAP: u64 = 64 * 1024 * 1024;
pub const MAX_CAP: u64 = 4 * 1024 * 1024 * 1024 * 1024;
/// Files a copy holds at most.
pub const MAX_FILES: usize = 50_000;
/// The largest file a copy holds.
pub const MAX_FILE: u64 = 1 << 40;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A 64-hex SHA-256.
pub fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `cr_` and 16 lowercase hex characters: one round of a copy.
pub fn is_round_id(s: &str) -> bool {
    s.strip_prefix("cr_")
        .is_some_and(|h| h.len() == 16 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

pub fn new_round_id() -> String {
    let hex: String = crate::server::auth::random_bytes(8)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("cr_{hex}")
}

/// An account's id as a copy's folder names it: `default` or `acc_…`.
pub fn is_account_dir(s: &str) -> bool {
    s == crate::claude::accounts::DEFAULT_ID || crate::claude::accounts::is_account_id(s)
}

/// The key of a copy in the index: `<source machine>/<session>`.
pub fn key(machine: &str, session: &str) -> String {
    format!("{machine}/{session}")
}

/// Copies at run time (`MachinesState::copies`).
pub struct CopiesState {
    data_dir: Option<PathBuf>,
    /// This computer's choices and how its copies elsewhere stand
    /// (`machines/copies.json`).
    pub(crate) out: Mutex<store::OutFile>,
    /// The copies held here (`machines/copies/index.json`).
    pub(crate) held: Mutex<store::HeldIndex>,
    /// One round, landing or removal per copy at a time.
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Each marked session's turn watch.
    pub(crate) watches: Mutex<HashMap<String, source::Watch>>,
    /// When a paused pair is tried again: `(session, machine)` → (ms, tries).
    pub(crate) retry: Mutex<HashMap<(String, String), (i64, u32)>>,
    /// The app turned the loop on (tests drive rounds with [`source::tick`]).
    live: AtomicBool,
    /// How long a session stays quiet after a turn before it is copied (ms).
    debounce_ms: AtomicU64,
    /// One chunk written at a time here.
    pub(crate) chunks: tokio::sync::Mutex<()>,
    /// Tests: a round fails at the named point (`apply`: the target refuses
    /// what it got), once.
    fail_at: Mutex<Option<String>>,
}

impl std::fmt::Debug for CopiesState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CopiesState").finish_non_exhaustive()
    }
}

impl CopiesState {
    pub fn load(data_dir: Option<&Path>) -> Self {
        Self {
            data_dir: data_dir.map(Path::to_path_buf),
            out: Mutex::new(store::OutFile::load(data_dir)),
            held: Mutex::new(store::HeldIndex::load(data_dir)),
            locks: Mutex::new(HashMap::new()),
            watches: Mutex::new(HashMap::new()),
            retry: Mutex::new(HashMap::new()),
            live: AtomicBool::new(false),
            debounce_ms: AtomicU64::new(DEBOUNCE.as_millis() as u64),
            chunks: tokio::sync::Mutex::new(()),
            fail_at: Mutex::new(None),
        }
    }

    /// `<app data>/machines/copies` (the copies held here).
    pub fn root(&self) -> Option<PathBuf> {
        self.data_dir
            .as_ref()
            .map(|d| d.join("machines").join("copies"))
    }

    /// The folder of one copy held here.
    pub fn copy_dir(&self, machine: &str, account_dir: &str, session: &str) -> Option<PathBuf> {
        if !super::ids::is_machine_id(machine)
            || !is_account_dir(account_dir)
            || !crate::claude::ids::is_session_id(session)
        {
            return None;
        }
        self.root()
            .map(|r| r.join(machine).join(account_dir).join(session))
    }

    /// Where a round's blob arrives here.
    pub(crate) fn incoming(&self, machine: &str, round: &str) -> Option<PathBuf> {
        if !super::ids::is_machine_id(machine) || !is_round_id(round) {
            return None;
        }
        self.root().map(|r| {
            r.join(".incoming")
                .join(machine)
                .join(format!("{round}.bin"))
        })
    }

    /// Where this computer writes a round's blob, or a package of a copy.
    pub(crate) fn outgoing(&self, name: &str) -> Option<PathBuf> {
        self.root().map(|r| r.join(".outgoing").join(name))
    }

    /// Reads this computer's choices.
    pub fn out_read<R>(&self, f: impl FnOnce(&store::OutFile) -> R) -> R {
        f(&lock(&self.out))
    }

    /// Changes this computer's choices and saves them.
    pub fn out_write<R>(&self, f: impl FnOnce(&mut store::OutFile) -> R) -> R {
        let mut file = lock(&self.out);
        let out = f(&mut file);
        if let Some(dir) = &self.data_dir {
            if let Err(e) = file.save(dir) {
                tracing::warn!(error = %e, "machines/copies.json could not be saved");
            }
        }
        out
    }

    /// Reads the index of the copies held here.
    pub fn held_read<R>(&self, f: impl FnOnce(&store::HeldIndex) -> R) -> R {
        f(&lock(&self.held))
    }

    /// Changes the index of the copies held here and saves it.
    pub fn held_write<R>(&self, f: impl FnOnce(&mut store::HeldIndex) -> R) -> R {
        let mut file = lock(&self.held);
        let out = f(&mut file);
        if let Some(dir) = &self.data_dir {
            if let Err(e) = file.save(dir) {
                tracing::warn!(error = %e, "machines/copies/index.json could not be saved");
            }
        }
        out
    }

    /// The lock of one copy (`kind`: `in` for one held here, `out` for one
    /// this computer keeps elsewhere).
    pub(crate) fn lock_of(&self, kind: &str, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        lock(&self.locks)
            .entry(format!("{kind}:{key}"))
            .or_default()
            .clone()
    }

    pub fn live(&self) -> bool {
        self.live.load(Ordering::Relaxed)
    }

    pub(crate) fn set_live(&self) {
        self.live.store(true, Ordering::Relaxed);
    }

    /// How long a session stays quiet after a turn before it is copied.
    pub fn debounce_ms(&self) -> i64 {
        self.debounce_ms.load(Ordering::Relaxed) as i64
    }

    /// Tests: copy as soon as a turn ended (`0`), or after `ms`.
    #[doc(hidden)]
    pub fn set_debounce_ms(&self, ms: u64) {
        self.debounce_ms.store(ms, Ordering::Relaxed);
    }

    /// Tests: a round fails at `point` (`apply`), once.
    #[doc(hidden)]
    pub fn fail_at(&self, point: Option<&str>) {
        *lock(&self.fail_at) = point.map(str::to_string);
    }

    pub(crate) fn take_fail(&self, point: &str) -> bool {
        let mut f = lock(&self.fail_at);
        if f.as_deref() == Some(point) {
            *f = None;
            true
        } else {
            false
        }
    }
}

// ─── shared helpers ────────────────────────────────────────────────────────────

/// Copies changed: dashboards refetch (`{t:"machines", what:"copies"}`).
pub fn publish(state: &AppState) {
    super::publish(state, "copies");
}

/// An audit line for a copy (the session, the computer, sizes — never a
/// byte of what it holds).
pub fn audit(state: &AppState, action: &str, machine: &str, session: &str, detail: Value) {
    state.devices.audit.record(AuditEntry {
        action: action.to_string(),
        device_id: Some(machine.to_string()),
        device_name: Some(super::peer_name(state, machine)),
        detail: Some(json!({ "session": session, "more": detail })),
        ..AuditEntry::now()
    });
}

/// A file's time, in ms since the epoch (0 when unknown).
pub(crate) fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// A relative path as a copy names it (`/`-separated): one of session
/// `id`'s own files, as a session's archive may hold it.
pub fn allowed_path(rel: &str, id: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    // `projects/<dir>/<id>` is the folder of its subagents, never a file.
    let folder_named_as_file = parts.len() == 3 && parts[0] == "projects" && parts[2] == id;
    rel.len() <= 1024
        && !rel.ends_with('/')
        && !folder_named_as_file
        && crate::claude::backup::allowed_entry(rel, id)
}

/// `rel` (`/`-separated, checked) under `base`.
pub(crate) fn join_rel(base: &Path, rel: &str) -> PathBuf {
    let mut out = base.to_path_buf();
    for c in rel.split('/').filter(|c| !c.is_empty()) {
        out.push(c);
    }
    out
}

/// Starts the copies loop (the app only: tests drive [`source::tick`]).
pub fn start(state: &AppState) {
    if !state.machines.copies.live() {
        return;
    }
    tokio::spawn(source::run_loop(Arc::downgrade(state)));
}

/// A paired computer came back: its paused copies are tried at the next
/// tick instead of after their wait.
pub fn on_peer_back(state: &AppState, machine: &str) {
    lock(&state.machines.copies.retry).retain(|(_, m), _| m != machine);
    let mut watches = lock(&state.machines.copies.watches);
    for w in watches.values_mut() {
        w.again_for(machine);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: &str = "0f1e2d3c-4b5a-4968-8776-655443322110";

    #[test]
    fn ids_and_paths_are_checked() {
        let r = new_round_id();
        assert!(is_round_id(&r), "{r}");
        assert!(!is_round_id("cr_123"));
        assert!(!is_round_id("mv_0123456789abcdef"));
        assert!(is_account_dir("default"));
        assert!(is_account_dir("acc_0123456789ab"));
        assert!(!is_account_dir("../x"));
        assert!(!is_account_dir("Work"));
        assert!(is_sha256(&"a".repeat(64)));
        assert!(!is_sha256(&"A".repeat(64)));
        for ok in [
            format!("projects/-Users-m-app/{S}.jsonl"),
            format!("projects/-Users-m-app/{S}/subagents/agent-a1.jsonl"),
            format!("projects/C--dev-app/{S}/subagents/workflows/wf_1/journal.jsonl"),
            format!("file-history/{S}/abc@v2"),
            format!("session-env/{S}/hook-1.sh"),
            format!("todos/{S}-agent-{S}.json"),
        ] {
            assert!(allowed_path(&ok, S), "{ok}");
        }
        for bad in [
            "projects/-Users-m-app/other.jsonl".to_string(),
            format!("projects/../{S}.jsonl"),
            format!("projects/x/{S}/../../settings.json"),
            "settings.json".to_string(),
            ".mc/files.json".to_string(),
            format!("todos/{S}.txt"),
            format!("projects\\x\\{S}.jsonl"),
            format!("projects/x/{S}/"),
            format!("projects/x/{S}"),
        ] {
            assert!(!allowed_path(&bad, S), "{bad}");
        }
        let base = Path::new("base");
        assert_eq!(join_rel(base, "a/b.json"), base.join("a").join("b.json"));
    }

    #[test]
    fn a_copy_lives_in_its_source_account_and_session_folder() {
        let dir = tempfile::tempdir().unwrap();
        let c = CopiesState::load(Some(dir.path()));
        let m = "0123456789abcdef0123456789abcdef";
        let p = c.copy_dir(m, "default", S).unwrap();
        assert!(p.starts_with(dir.path().join("machines").join("copies")));
        assert!(p.ends_with(Path::new(m).join("default").join(S)));
        assert!(c.copy_dir("../x", "default", S).is_none());
        assert!(c.copy_dir(m, "Work", S).is_none());
        assert!(c.copy_dir(m, "default", "x").is_none());
        assert!(c.incoming(m, "cr_0123456789abcdef").is_some());
        assert!(c.incoming(m, "../x").is_none());
        assert_eq!(c.debounce_ms(), 5000);
        c.set_debounce_ms(0);
        assert_eq!(c.debounce_ms(), 0);
    }
}
