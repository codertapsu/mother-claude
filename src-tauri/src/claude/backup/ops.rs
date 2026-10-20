//! The backup engine wired to the app: the passphrase (in the secret store),
//! where sets go, the jobs that push, restore and pull, and the schedule that
//! backs up idle accounts (docs/ACCOUNTS.md P4.3, P5.1).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{BackupError, PullItem, Set};
use crate::claude::accounts::{Account, BackupSettings};
use crate::claude::secrets::{Secret, SecretName, SecretStore};
use crate::claude::transfer::{self, JobState};
use crate::state::AppState;

/// How often the schedule looks.
const TICK: Duration = Duration::from_secs(5 * 60);
/// An account is idle when none of its sessions was active this long.
pub const IDLE_FOR_MS: i64 = 10 * 60_000;
/// Shortest passphrase of the user's own.
pub const PASSPHRASE_MIN: usize = 12;

/// `<app data dir>/backup-state.json`: when each account was last pushed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct LocalState {
    accounts: BTreeMap<String, AccountState>,
    last_pull: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct AccountState {
    last_push: Option<i64>,
    last_error: Option<String>,
    /// Sessions in the set after the last push.
    sessions: Option<usize>,
}

fn state_path(state: &AppState) -> Option<PathBuf> {
    state
        .auth
        .config_dir
        .as_ref()
        .map(|d| d.join("backup-state.json"))
}

fn load(state: &AppState) -> LocalState {
    state_path(state)
        .as_deref()
        .and_then(crate::persist::read_json)
        .unwrap_or_default()
}

fn save(state: &AppState, s: &LocalState) {
    if let Some(p) = state_path(state) {
        if let Err(e) = crate::persist::write_json_private(&p, s) {
            tracing::warn!(error = %e, "could not save the backup state");
        }
    }
}

fn stored(store: &dyn SecretStore, name: &SecretName) -> Option<Secret> {
    store.get(name).ok().flatten().filter(|p| !p.is_empty())
}

/// The backup passphrase, if one is set.
pub fn passphrase(state: &AppState) -> Option<Secret> {
    stored(
        state.accounts.secret_store().as_ref(),
        &SecretName::BackupPassphrase,
    )
}

/// The new passphrases unfinished changes left pending (one per line): some
/// sets may have one of them already.
fn pending_in(store: &dyn SecretStore) -> Vec<Secret> {
    stored(store, &SecretName::BackupPassphrasePending)
        .map(|p| {
            p.expose()
                .lines()
                .filter(|l| !l.is_empty())
                .map(Secret::new)
                .collect()
        })
        .unwrap_or_default()
}

/// What sets open with: the passphrase, then those unfinished changes left
/// pending.
pub fn passphrases_in(store: &dyn SecretStore) -> Option<Vec<Secret>> {
    let mut out = vec![stored(store, &SecretName::BackupPassphrase)?];
    for p in pending_in(store) {
        if !out.contains(&p) {
            out.push(p);
        }
    }
    Some(out)
}

pub fn passphrases(state: &AppState) -> Option<Vec<Secret>> {
    passphrases_in(state.accounts.secret_store().as_ref())
}

/// A passphrase change did not finish: until one does, sets may need a
/// pending passphrase (the UI asks to set it again).
pub fn passphrase_change_unfinished(state: &AppState) -> bool {
    !pending_in(state.accounts.secret_store().as_ref()).is_empty()
}

/// A passphrase to write down: eight groups of four from an alphabet without
/// look-alikes (160 bits).
pub fn generate_passphrase() -> String {
    const ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
    let bytes = crate::server::auth::random_bytes(32);
    let chars: Vec<char> = bytes
        .iter()
        .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
        .collect();
    chars
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

/// `<app data dir>/backup-machine.json`: this install's machine id.
const MACHINE_FILE: &str = "backup-machine.json";

#[derive(Debug, Serialize, Deserialize)]
struct StoredMachine {
    id: String,
    /// A hash of the hardware's own id, when the platform has one: a copy of
    /// the app's data made by Migration Assistant or a backup on another Mac
    /// keeps this file but not the hardware, and gets an id of its own.
    #[serde(default)]
    hardware: Option<String>,
}

/// This install's machine id: random, made once, kept in the app data dir
/// (0600). A copy of it on other hardware is replaced (§ machine identity
/// in docs/BACKUP.md).
pub fn machine_id_in(data_dir: &Path, hardware: Option<String>) -> Result<String, BackupError> {
    let path = data_dir.join(MACHINE_FILE);
    if let Some(s) = crate::persist::read_json::<StoredMachine>(&path) {
        let same_hardware = match (&s.hardware, &hardware) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        };
        if super::is_machine_id(&s.id) && same_hardware {
            return Ok(s.id);
        }
    }
    let id: String = crate::server::auth::random_bytes(16)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    crate::persist::write_json_private(
        &path,
        &StoredMachine {
            id: id.clone(),
            hardware,
        },
    )
    .map_err(|e| BackupError::Io(format!("writing {}: {e}", path.display())))?;
    Ok(id)
}

/// A hash of the hardware's id (macOS: the host UUID; Linux: the machine
/// id), never the id itself.
pub fn hardware_fingerprint() -> Option<String> {
    let raw: Option<String> = {
        #[cfg(target_os = "macos")]
        {
            let mut uuid = [0u8; 16];
            let wait = libc::timespec {
                tv_sec: 1,
                tv_nsec: 0,
            };
            // SAFETY: gethostuuid writes exactly 16 bytes into `uuid`.
            let rc = unsafe { libc::gethostuuid(uuid.as_mut_ptr(), &wait) };
            (rc == 0 && uuid != [0u8; 16])
                .then(|| uuid.iter().map(|b| format!("{b:02x}")).collect())
        }
        #[cfg(target_os = "linux")]
        {
            std::fs::read_to_string("/etc/machine-id")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            None
        }
    };
    raw.map(|r| crate::claude::secrets::sha256_hex(format!("mother-claude machine:{r}").as_bytes()))
}

/// This machine as the sets know it.
pub fn this_machine(state: &AppState) -> Result<super::ThisMachine, BackupError> {
    let name = super::host_name();
    let id = match state.auth.config_dir.as_deref() {
        Some(dir) => machine_id_in(dir, hardware_fingerprint())?,
        // No data dir (some tests): a stable id from the name.
        None => crate::claude::secrets::sha256_hex(name.as_bytes())[..32].to_string(),
    };
    Ok(super::ThisMachine { id, name })
}

/// The accounts with a set of their own: the Default and each directory
/// account. A token account's sessions are in the Default's directory, so
/// in the Default's set.
fn set_owners(state: &AppState) -> Vec<Account> {
    state
        .accounts
        .homes()
        .into_iter()
        .filter_map(|(id, _)| state.accounts.get(&id))
        .collect()
}

/// The account whose set holds `a`'s sessions: itself, or the Default for a
/// token account.
fn set_owner(state: &AppState, a: Account) -> Account {
    match a.kind {
        crate::claude::accounts::AccountKind::Token => state
            .accounts
            .get(crate::claude::accounts::DEFAULT_ID)
            .unwrap_or(a),
        crate::claude::accounts::AccountKind::Directory => a,
    }
}

/// Why a passphrase change did not go through.
#[derive(Debug)]
pub struct PassphraseFailure {
    pub error: BackupError,
    /// Some set has the new passphrase already: whoever asked for the change
    /// must learn it (a generated one is shown with the error), and it stays
    /// kept as pending, so every set still opens.
    pub applied: bool,
}

impl From<BackupError> for PassphraseFailure {
    fn from(error: BackupError) -> Self {
        Self {
            error,
            applied: false,
        }
    }
}

/// What a passphrase change did.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PassphraseChange {
    /// Sets now under the new passphrase.
    pub rekeyed: Vec<String>,
    /// Sets in the folder of no account here that open with none of this
    /// machine's passphrases: left as they are.
    pub left: Vec<String>,
}

/// Every set in the folder, by folder name.
fn folder_sets(folder: &Path) -> Vec<(String, Set)> {
    let mut out: Vec<(String, Set)> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let set = Set::new(folder, &name);
            (set.exists() || set.key_missing()).then_some((name, set))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Set (or change) the passphrase. Every set in the folder is re-keyed with
/// it, so nothing already backed up becomes unreadable, and the change is
/// all or nothing as far as it can be:
///
/// 1. every set is opened first (with the new passphrase: done already; else
///    with the current one or a pending one) and nothing changes if a set of
///    an account here opens with none;
/// 2. the new passphrase is kept as *pending* while the sets are re-keyed,
///    so a crash or a failed write leaves every set openable with one of the
///    two kept;
/// 3. only then does it become the passphrase.
pub fn set_passphrase(
    state: &AppState,
    new: &Secret,
) -> Result<PassphraseChange, PassphraseFailure> {
    let ours: Vec<String> = set_owners(state).into_iter().map(|a| a.name).collect();
    change_passphrase(
        state.accounts.secret_store().as_ref(),
        state.accounts.backup_settings().folder.as_deref(),
        &ours,
        new,
    )
}

/// [`set_passphrase`] on a secret store and a folder; `ours`: the names of
/// the sets of this machine's accounts (the others are left as they are when
/// they open with none of its passphrases).
pub fn change_passphrase(
    store: &dyn SecretStore,
    folder: Option<&Path>,
    ours: &[String],
    new: &Secret,
) -> Result<PassphraseChange, PassphraseFailure> {
    if new.expose().chars().count() < PASSPHRASE_MIN {
        return Err(BackupError::Invalid(format!(
            "a passphrase has at least {PASSPHRASE_MIN} characters"
        ))
        .into());
    }
    if new.expose().contains(['\n', '\r']) {
        return Err(BackupError::Invalid("a passphrase is one line".into()).into());
    }
    let earlier = pending_in(store);
    let mut known: Vec<Secret> = passphrases_in(store).unwrap_or_default();
    for p in &earlier {
        if !known.contains(p) {
            known.push(p.clone());
        }
    }
    let mut change = PassphraseChange::default();
    // Some set has the new one already (an earlier attempt).
    let mut had_new = false;
    let mut todo = Vec::new();
    if let Some(folder) = folder {
        for (name, set) in folder_sets(folder) {
            let mine = ours.contains(&name);
            if set.key_missing() {
                if mine {
                    return Err(BackupError::KeyMissing(set.dir.display().to_string()).into());
                }
                change.left.push(name);
                continue;
            }
            match super::open_keys(&set, new) {
                Ok(_) => {
                    had_new = true;
                    continue;
                }
                Err(BackupError::WrongPassphrase) => {}
                Err(e) => return Err(e.into()),
            }
            match super::open_keys_any(&set, &known) {
                Ok(keys) => todo.push((name, set, keys)),
                Err(BackupError::WrongPassphrase) | Err(BackupError::NoPassphrase) if !mine => {
                    change.left.push(name)
                }
                Err(BackupError::WrongPassphrase) | Err(BackupError::NoPassphrase) => {
                    return Err(BackupError::Invalid(format!(
                        "the backup set {name} opens with neither this machine's passphrase nor \
                         the new one; enter the passphrase it was made with"
                    ))
                    .into())
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
    // Kept with any an earlier unfinished change left: until this one is
    // through, a set may still need one of those.
    let mut keep: Vec<&str> = earlier.iter().map(Secret::expose).collect();
    if !keep.contains(&new.expose()) {
        keep.push(new.expose());
    }
    store
        .set(
            &SecretName::BackupPassphrasePending,
            &Secret::new(keep.join("\n")),
        )
        .map_err(|e| BackupError::Io(e.to_string()))?;
    for (name, set, keys) in &todo {
        if let Err(error) = super::write_identity(set, keys, new) {
            return Err(PassphraseFailure {
                error,
                applied: had_new || !change.rekeyed.is_empty(),
            });
        }
        change.rekeyed.push(name.clone());
    }
    let applied = had_new || !change.rekeyed.is_empty();
    store
        .set(&SecretName::BackupPassphrase, new)
        .map_err(|e| PassphraseFailure {
            error: BackupError::Io(e.to_string()),
            applied,
        })?;
    if let Err(e) = store.delete(&SecretName::BackupPassphrasePending) {
        tracing::warn!(error = %e, "could not forget the pending backup passphrase");
    }
    Ok(change)
}

/// A folder the UI offers.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub label: String,
    pub path: String,
}

/// iCloud Drive and the Google Drive (and other File Provider) folders that
/// exist on this machine, each with a `Mother Claude` folder inside.
pub fn suggestions() -> Vec<Suggestion> {
    let Some(home) = crate::claude::user_home_dir() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let icloud = home.join("Library/Mobile Documents/com~apple~CloudDocs");
    if icloud.is_dir() {
        out.push(Suggestion {
            label: "iCloud Drive".into(),
            path: icloud.join("Mother Claude").to_string_lossy().into_owned(),
        });
    }
    for e in std::fs::read_dir(home.join("Library/CloudStorage"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let name = e.file_name().to_string_lossy().into_owned();
        let (label, root) = if let Some(rest) = name.strip_prefix("GoogleDrive-") {
            // "My Drive" (or its translation) is the folder that syncs.
            let drive = std::fs::read_dir(e.path())
                .into_iter()
                .flatten()
                .flatten()
                .map(|d| d.path())
                .find(|p| {
                    p.is_dir()
                        && !p
                            .file_name()
                            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
                })
                .unwrap_or_else(|| e.path());
            (format!("Google Drive ({rest})"), drive)
        } else if let Some(rest) = name.strip_prefix("OneDrive-") {
            (format!("OneDrive ({rest})"), e.path())
        } else if name.starts_with("Dropbox") {
            ("Dropbox".to_string(), e.path())
        } else {
            continue;
        };
        out.push(Suggestion {
            label,
            path: root.join("Mother Claude").to_string_lossy().into_owned(),
        });
    }
    #[cfg(windows)]
    {
        let icloud = home.join("iCloudDrive");
        if icloud.is_dir() {
            out.push(Suggestion {
                label: "iCloud Drive".into(),
                path: icloud.join("Mother Claude").to_string_lossy().into_owned(),
            });
        }
    }
    out
}

/// One account's set as `GET /api/backup` shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetView {
    pub account: String,
    pub name: String,
    pub dir: String,
    pub exists: bool,
    /// A token account's sessions are in the Default's directory, so in the
    /// Default's set: its id here (and that set's details above).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_with: Option<String>,
    /// A set is in the folder but its key is not (deleted, or not downloaded
    /// yet): nothing can be pushed or read until it is back.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub key_missing: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_push: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sessions: Option<usize>,
    /// Other machines that pushed to it, and when.
    pub machines: Vec<super::Machine>,
}

/// `GET /api/backup`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupView {
    pub settings: BackupSettings,
    pub passphrase_set: bool,
    /// A passphrase change did not finish (some sets may have the new one):
    /// set the passphrase again.
    pub passphrase_change_unfinished: bool,
    pub host: String,
    pub sets: Vec<SetView>,
    pub suggestions: Vec<Suggestion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_pull: Option<i64>,
    /// The data jobs of the last backups, restores and pulls.
    pub jobs: Vec<transfer::JobView>,
}

pub fn view(state: &AppState) -> BackupView {
    let settings = state.accounts.backup_settings();
    let local = load(state);
    let host = super::host_name();
    let sets = match &settings.folder {
        Some(folder) => state
            .accounts
            .accounts()
            .into_iter()
            .map(|a| {
                let owner = set_owner(state, a.clone());
                let set = Set::new(folder, &owner.name);
                let mine = local.accounts.get(&owner.id).cloned().unwrap_or_default();
                let machines = super::read_info(&set)
                    .map(|i| i.machines)
                    .unwrap_or_default();
                SetView {
                    account: a.id.clone(),
                    name: a.name.clone(),
                    dir: set.dir.to_string_lossy().into_owned(),
                    exists: set.exists(),
                    shared_with: (owner.id != a.id).then(|| owner.id.clone()),
                    key_missing: set.key_missing(),
                    last_push: mine.last_push,
                    last_error: mine.last_error,
                    sessions: mine.sessions,
                    machines,
                }
            })
            .collect(),
        None => Vec::new(),
    };
    BackupView {
        settings,
        passphrase_set: passphrase(state).is_some(),
        passphrase_change_unfinished: passphrase_change_unfinished(state),
        host,
        sets,
        suggestions: suggestions(),
        last_pull: local.last_pull,
        jobs: state
            .jobs
            .list()
            .into_iter()
            .filter(|j| matches!(j.kind.as_str(), "backup" | "restore" | "pull"))
            .collect(),
    }
}

fn is_live(state: &AppState, id: &str) -> bool {
    state.live.is_live(id) || state.control.controls(id) || state.bridge.account_of(id).is_some()
}

/// What every backup job needs first: the folder, and what sets open with.
fn prerequisites(state: &AppState) -> Result<(PathBuf, Vec<Secret>), BackupError> {
    let folder = state
        .accounts
        .backup_settings()
        .folder
        .ok_or(BackupError::NoFolder)?;
    let pass = passphrases(state).ok_or(BackupError::NoPassphrase)?;
    Ok((folder, pass))
}

/// Why there is nothing to read in `set`: no set at all, or one without its
/// key here.
fn no_set_error(set: &Set, a: &Account, folder: &Path) -> Result<(), BackupError> {
    if set.exists() {
        return Ok(());
    }
    Err(if set.key_missing() {
        BackupError::KeyMissing(set.dir.display().to_string())
    } else {
        BackupError::Invalid(format!(
            "there is no backup of {} in {}",
            a.name,
            folder.display()
        ))
    })
}

fn account(state: &AppState, id: &str) -> Result<Account, BackupError> {
    state
        .accounts
        .get(id)
        .ok_or_else(|| BackupError::Invalid(format!("no account {id:?}")))
}

/// The ids of the accounts with a set of their own (what "back up all" and
/// "pull all" go through).
pub fn set_owner_ids(state: &AppState) -> Vec<String> {
    set_owners(state).into_iter().map(|a| a.id).collect()
}

fn finish(state: &AppState, job: &str, result: Result<Value, BackupError>) {
    let at = chrono::Utc::now().timestamp_millis();
    state.jobs.update(job, |v| {
        v.finished_at = Some(at);
        match result {
            Ok(r) => {
                v.state = JobState::Done;
                v.result = Some(r);
            }
            Err(e) => {
                v.state = JobState::Failed;
                v.error = Some(e.to_string());
                v.error_code = Some(e.code().to_string());
            }
        }
    });
}

/// Backups and pulls running or waiting for their turn, per set: a second
/// one of a set gets the first's job id instead of a queue of its own (the
/// schedule ticks every five minutes; a first push to iCloud takes longer).
static IN_FLIGHT: std::sync::LazyLock<std::sync::Mutex<BTreeMap<String, String>>> =
    std::sync::LazyLock::new(Default::default);

fn in_flight() -> std::sync::MutexGuard<'static, BTreeMap<String, String>> {
    IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner())
}

/// A set's claim on its one backup (or pull) job: released when the job's
/// task ends, however it ends.
struct Flight(String);

impl Drop for Flight {
    fn drop(&mut self) {
        in_flight().remove(&self.0);
    }
}

/// `Err(job)`: the job running or queued for `key`; else `job` is it now.
fn claim(key: &str, job: &str) -> Result<Flight, String> {
    let mut map = in_flight();
    if let Some(running) = map.get(key) {
        return Err(running.clone());
    }
    map.insert(key.to_string(), job.to_string());
    Ok(Flight(key.to_string()))
}

fn flight_key(kind: &str, set: &Set) -> String {
    format!("{kind}:{}", set.dir.display())
}

/// What the session list calls each session, for the index.
async fn titles(state: &AppState) -> BTreeMap<String, String> {
    state
        .sessions()
        .await
        .into_iter()
        .filter_map(|s| {
            let named = Some(s.name.clone()).filter(|n| !n.is_empty() && *n != s.project_name);
            let title = s
                .title
                .clone()
                .or(named)
                .or_else(|| s.last_prompt.clone())?;
            Some((s.id, super::clip_title(&title, super::TITLE_MAX)))
        })
        .collect()
}

/// Back up an account (a data job; one backup job at a time with the rest).
/// A token account is backed up with the Default, whose directory it uses;
/// a set that is being backed up already returns that job.
pub async fn push(state: &AppState, account_id: &str) -> Result<String, BackupError> {
    let (folder, pass) = prerequisites(state)?;
    let a = set_owner(state, account(state, account_id)?);
    let home = state
        .accounts
        .home(&a.id)
        .ok_or_else(|| BackupError::Invalid("the account has no directory".into()))?;
    let machine = this_machine(state)?;
    let set = Set::new(&folder, &a.name);
    let key = flight_key("backup", &set);
    let job = transfer::new_job_id();
    let flight = match claim(&key, &job) {
        Ok(f) => f,
        Err(running) => return Ok(running),
    };
    let progress = state.jobs.start(&job, "backup", 0, 0);
    state.jobs.update(&job, |v| v.account = Some(a.id.clone()));
    let st = state.clone();
    let id = job.clone();
    tokio::spawn(async move {
        let _flight = flight;
        let _one = st.jobs.lock.lock().await;
        let titles = titles(&st).await;
        let st2 = st.clone();
        let account_id = a.id.clone();
        let result = tokio::task::spawn_blocking(move || {
            let live = |s: &str| is_live(&st2, s);
            let title_of = |s: &str| titles.get(s).cloned();
            super::push(&super::PushInputs {
                home: &home,
                set: &set,
                account_name: &a.name,
                passphrases: &pass,
                machine: &machine,
                is_live: &live,
                title_of: &title_of,
                progress: &progress,
            })
        })
        .await
        .unwrap_or_else(|e| Err(BackupError::Io(e.to_string())));
        let mut local = load(&st);
        let s = local.accounts.entry(account_id).or_default();
        match &result {
            Ok(report) => {
                s.last_push = Some(chrono::Utc::now().timestamp_millis());
                s.last_error = (!report.failed.is_empty())
                    .then(|| format!("{} session(s) could not be archived", report.failed.len()));
                s.sessions = Some(report.archived.len() + report.unchanged);
            }
            Err(e) => s.last_error = Some(e.to_string()),
        }
        save(&st, &local);
        finish(
            &st,
            &id,
            result.map(|r| serde_json::to_value(r).unwrap_or(Value::Null)),
        );
    });
    Ok(job)
}

/// Back up every set (the schedule's and "Back up now"'s): the Default and
/// each directory account, never the Default's directory twice.
pub async fn push_all(state: &AppState) -> Result<Vec<String>, BackupError> {
    prerequisites(state)?;
    let mut jobs = Vec::new();
    for id in set_owner_ids(state) {
        jobs.push(push(state, &id).await?);
    }
    Ok(jobs)
}

/// One session of the set, compared with the target account's directory.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexView {
    pub account: String,
    pub name: String,
    pub set: String,
    pub machines: Vec<super::Machine>,
    pub sessions: Vec<PullItem>,
    pub config: Vec<super::ConfigEntry>,
}

/// `GET /api/backup/index`: the set of `account` (a token account's: the
/// Default's), compared with `into`'s directory (default: the same account).
pub async fn index(
    state: &AppState,
    account_id: &str,
    into: Option<&str>,
) -> Result<IndexView, BackupError> {
    let (folder, pass) = prerequisites(state)?;
    let a = account(state, account_id)?;
    let owner = set_owner(state, a.clone());
    let target = account(state, into.unwrap_or(account_id))?;
    let home = state
        .accounts
        .home(&target.id)
        .ok_or_else(|| BackupError::Invalid("the account has no directory".into()))?;
    let set = Set::new(&folder, &owner.name);
    no_set_error(&set, &owner, &folder)?;
    let machine = this_machine(state)?;
    // What this machine had is about the set's own directory only.
    let same_dir = state
        .accounts
        .home(&owner.id)
        .is_some_and(|h| h.base() == home.base());
    let st = state.clone();
    tokio::task::spawn_blocking(move || {
        let keys = super::open_keys_any(&set, &pass)?;
        let index = super::read_index(&set, &keys)?;
        // Display only.
        let machines = super::read_info(&set)
            .map(|i| i.machines)
            .unwrap_or_default();
        let live = |s: &str| is_live(&st, s);
        let sessions = super::compare(
            &index,
            &home,
            if same_dir { &machine.id } else { "" },
            &live,
            chrono::Utc::now().timestamp_millis(),
        );
        Ok(IndexView {
            account: a.id.clone(),
            name: a.name.clone(),
            set: set.dir.to_string_lossy().into_owned(),
            machines,
            sessions,
            config: index.config,
        })
    })
    .await
    .map_err(|e| BackupError::Io(e.to_string()))?
}

/// Restore sessions of `account`'s set (a token account's: the Default's)
/// into `into`'s directory (a data job with an undo).
pub async fn restore(
    state: &AppState,
    account_id: &str,
    into: &str,
    ids: Vec<String>,
    trust: bool,
) -> Result<String, BackupError> {
    let (folder, pass) = prerequisites(state)?;
    let a = set_owner(state, account(state, account_id)?);
    let target = account(state, into)?;
    let home = state
        .accounts
        .home(&target.id)
        .ok_or_else(|| BackupError::Invalid("the account has no directory".into()))?;
    let undo_root = transfer::ops::undo_root(state)
        .ok_or_else(|| BackupError::Invalid("this build has no data directory".into()))?;
    let job = transfer::new_job_id();
    let progress = state.jobs.start(&job, "restore", ids.len() as u64, 0);
    state.jobs.update(&job, |v| {
        v.account = Some(a.id.clone());
        v.to = Some(target.id.clone());
    });
    let st = state.clone();
    let id = job.clone();
    tokio::spawn(async move {
        let _one = st.jobs.lock.lock().await;
        let set = Set::new(&folder, &a.name);
        let st2 = st.clone();
        let id2 = id.clone();
        let result = tokio::task::spawn_blocking(move || {
            let live = |s: &str| is_live(&st2, s);
            super::restore(&super::RestoreInputs {
                set: &set,
                passphrases: &pass,
                target: &home,
                ids: &ids,
                trust,
                is_live: &live,
                undo_root: &undo_root,
                job_id: &id2,
                progress: &progress,
                kind: "restore",
            })
        })
        .await
        .unwrap_or_else(|e| Err(BackupError::Io(e.to_string())));
        let undo = result
            .as_ref()
            .ok()
            .filter(|(_, m)| !m.created.is_empty())
            .map(|(_, m)| m.created_at + transfer::UNDO_KEEP.as_millis() as i64);
        let sessions = result
            .as_ref()
            .map(|(r, _)| r.restored.clone())
            .unwrap_or_default();
        st.jobs.update(&id, |v| {
            v.undo_until = undo;
            v.sessions = sessions;
        });
        finish(
            &st,
            &id,
            result.map(|(r, _)| serde_json::to_value(r).unwrap_or(Value::Null)),
        );
        rediscover(&st).await;
    });
    Ok(job)
}

/// Unpack the newest configuration bundle of `account`'s set for review
/// (this machine's own, else the newest of any machine).
pub async fn extract_config(state: &AppState, account_id: &str) -> Result<PathBuf, BackupError> {
    let (folder, pass) = prerequisites(state)?;
    let a = set_owner(state, account(state, account_id)?);
    let into = state
        .auth
        .config_dir
        .as_ref()
        .map(|d| d.join("restored-config").join(&a.name))
        .ok_or_else(|| BackupError::Invalid("this build has no data directory".into()))?;
    let machine = this_machine(state)?;
    tokio::task::spawn_blocking(move || {
        super::extract_config(&Set::new(&folder, &a.name), &pass, &into, Some(&machine.id))
    })
    .await
    .map_err(|e| BackupError::Io(e.to_string()))?
}

/// Pull `account`'s set into its own directory (§5.11; a data job with an
/// undo; a token account's is the Default's). `ids`: only these sessions. A
/// set that is being pulled already returns that job.
pub async fn pull(
    state: &AppState,
    account_id: &str,
    ids: Option<Vec<String>>,
    trust: bool,
) -> Result<String, BackupError> {
    let (folder, pass) = prerequisites(state)?;
    let a = set_owner(state, account(state, account_id)?);
    let home = state
        .accounts
        .home(&a.id)
        .ok_or_else(|| BackupError::Invalid("the account has no directory".into()))?;
    let set = Set::new(&folder, &a.name);
    no_set_error(&set, &a, &folder)?;
    let undo_root = transfer::ops::undo_root(state)
        .ok_or_else(|| BackupError::Invalid("this build has no data directory".into()))?;
    let machine = this_machine(state)?;
    let key = flight_key("pull", &set);
    let job = transfer::new_job_id();
    let flight = match claim(&key, &job) {
        Ok(f) => f,
        Err(running) => return Ok(running),
    };
    let progress = state.jobs.start(&job, "pull", 0, 0);
    state.jobs.update(&job, |v| v.account = Some(a.id.clone()));
    let st = state.clone();
    let id = job.clone();
    tokio::spawn(async move {
        let _flight = flight;
        let _one = st.jobs.lock.lock().await;
        let st2 = st.clone();
        let id2 = id.clone();
        let result = tokio::task::spawn_blocking(move || {
            let live = |s: &str| is_live(&st2, s);
            super::pull(&super::PullInputs {
                set: &set,
                passphrases: &pass,
                home: &home,
                machine: &machine,
                ids: ids.as_deref(),
                trust,
                is_live: &live,
                undo_root: &undo_root,
                job_id: &id2,
                progress: &progress,
            })
        })
        .await
        .unwrap_or_else(|e| Err(BackupError::Io(e.to_string())));
        let undo = result
            .as_ref()
            .ok()
            .filter(|(_, m)| !m.created.is_empty() || !m.moved.is_empty())
            .map(|(_, m)| m.created_at + transfer::UNDO_KEEP.as_millis() as i64);
        let sessions = result
            .as_ref()
            .map(|(r, _)| r.restored.iter().chain(&r.updated).cloned().collect())
            .unwrap_or_default();
        st.jobs.update(&id, |v| {
            v.undo_until = undo;
            v.sessions = sessions;
        });
        if result.is_ok() {
            let mut local = load(&st);
            local.last_pull = Some(chrono::Utc::now().timestamp_millis());
            save(&st, &local);
        }
        finish(
            &st,
            &id,
            result.map(|(r, _)| serde_json::to_value(r).unwrap_or(Value::Null)),
        );
        rediscover(&st).await;
    });
    Ok(job)
}

async fn rediscover(state: &AppState) {
    let st = state.clone();
    let _ = tokio::task::spawn_blocking(move || {
        st.index.discover_homes(&st.accounts.homes());
    })
    .await;
    crate::server::monitor::tick_once(state).await;
}

/// Whether no session in `owner`'s directory (its own and, for the Default,
/// its token accounts') was active in the last ten minutes.
async fn idle(state: &AppState, owner: &str, now: i64) -> bool {
    let Some(dir) = state.accounts.home(owner).map(|h| h.base().to_path_buf()) else {
        return true;
    };
    let sessions = state.sessions().await;
    !sessions.iter().any(|s| {
        let account = s
            .account
            .as_ref()
            .map_or(crate::claude::accounts::DEFAULT_ID, |a| a.id.as_str());
        let here = state
            .accounts
            .home(account)
            .is_some_and(|h| h.base() == dir.as_path());
        here && (s.last_activity_at.is_some_and(|t| now - t < IDLE_FOR_MS)
            || (s.live && s.status == crate::claude::model::SessionStatus::Working))
    })
}

/// The schedule: every `everyHours`, each idle set is backed up; at launch,
/// with "pull on launch" and nothing live, every set is pulled.
pub fn start(state: &AppState) {
    let st = state.clone();
    tokio::spawn(async move {
        // Let the first refresh find what is live.
        tokio::time::sleep(Duration::from_secs(30)).await;
        let settings = st.accounts.backup_settings();
        if settings.pull_on_launch
            && settings.folder.is_some()
            && passphrase(&st).is_some()
            && !st.sessions().await.iter().any(|s| s.live)
        {
            for id in set_owner_ids(&st) {
                if let Err(e) = pull(&st, &id, None, false).await {
                    tracing::info!(account = %id, error = %e, "pull on launch skipped");
                }
            }
        }
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let settings = st.accounts.backup_settings();
            if settings.every_hours == 0 || settings.folder.is_none() || passphrase(&st).is_none() {
                continue;
            }
            let every = i64::from(settings.every_hours) * 3600 * 1000;
            let now = chrono::Utc::now().timestamp_millis();
            let local = load(&st);
            for id in set_owner_ids(&st) {
                let due = local
                    .accounts
                    .get(&id)
                    .and_then(|s| s.last_push)
                    .is_none_or(|t| now - t >= every);
                // A set still being backed up is not queued again (`push`).
                if due && idle(&st, &id, now).await {
                    if let Err(e) = push(&st, &id).await {
                        tracing::info!(account = %id, error = %e, "scheduled backup skipped");
                    }
                }
            }
        }
    });
}

/// An account was renamed: its set follows it (the set is named after it).
pub fn on_renamed(state: &AppState, old_name: &str, new_name: &str) {
    let Some(folder) = state.accounts.backup_settings().folder else {
        return;
    };
    let (from, to) = (folder.join(old_name), folder.join(new_name));
    if from.is_dir() && !to.exists() {
        if let Err(e) = std::fs::rename(&from, &to) {
            tracing::warn!(error = %e, "could not rename a backup set after its account");
        }
    }
}
