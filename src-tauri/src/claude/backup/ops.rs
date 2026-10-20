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
use crate::claude::secrets::{Secret, SecretName};
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

/// The backup passphrase, if one is set.
pub fn passphrase(state: &AppState) -> Option<Secret> {
    state
        .accounts
        .secret_store()
        .get(&SecretName::BackupPassphrase)
        .ok()
        .flatten()
        .filter(|p| !p.is_empty())
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

/// Every account's set in the folder.
fn sets(folder: &Path, accounts: &[Account]) -> Vec<(Account, Set)> {
    accounts
        .iter()
        .map(|a| (a.clone(), Set::new(folder, &a.name)))
        .collect()
}

/// Set (or change) the passphrase. Existing sets are re-keyed with it, so
/// nothing already backed up becomes unreadable.
pub fn set_passphrase(state: &AppState, new: &Secret) -> Result<(), BackupError> {
    if new.expose().chars().count() < PASSPHRASE_MIN {
        return Err(BackupError::Invalid(format!(
            "a passphrase has at least {PASSPHRASE_MIN} characters"
        )));
    }
    if let (Some(old), Some(folder)) = (passphrase(state), state.accounts.backup_settings().folder)
    {
        for (_, set) in sets(&folder, &state.accounts.accounts()) {
            if set.exists() {
                super::rekey(&set, &old, new)?;
            }
        }
    }
    state
        .accounts
        .secret_store()
        .set(&SecretName::BackupPassphrase, new)
        .map_err(|e| BackupError::Io(e.to_string()))
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
        Some(folder) => sets(folder, &state.accounts.accounts())
            .into_iter()
            .map(|(a, set)| {
                let mine = local.accounts.get(&a.id).cloned().unwrap_or_default();
                let machines = super::read_info(&set)
                    .map(|i| i.machines)
                    .unwrap_or_default();
                SetView {
                    account: a.id.clone(),
                    name: a.name.clone(),
                    dir: set.dir.to_string_lossy().into_owned(),
                    exists: set.exists(),
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

/// What every backup job needs first.
fn prerequisites(state: &AppState) -> Result<(PathBuf, Secret), BackupError> {
    let folder = state
        .accounts
        .backup_settings()
        .folder
        .ok_or(BackupError::NoFolder)?;
    let pass = passphrase(state).ok_or(BackupError::NoPassphrase)?;
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

/// Back up one account (a data job; one backup job at a time with the rest).
pub async fn push(state: &AppState, account_id: &str) -> Result<String, BackupError> {
    let (folder, pass) = prerequisites(state)?;
    let a = account(state, account_id)?;
    let home = state
        .accounts
        .home(&a.id)
        .ok_or_else(|| BackupError::Invalid("the account has no directory".into()))?;
    let job = transfer::new_job_id();
    let progress = state.jobs.start(&job, "backup", 0, 0);
    let st = state.clone();
    let id = job.clone();
    tokio::spawn(async move {
        let _one = st.jobs.lock.lock().await;
        let set = Set::new(&folder, &a.name);
        let host = super::host_name();
        let st2 = st.clone();
        let account_id = a.id.clone();
        let result = tokio::task::spawn_blocking(move || {
            let live = |s: &str| is_live(&st2, s);
            super::push(&super::PushInputs {
                home: &home,
                set: &set,
                account_name: &a.name,
                passphrase: &pass,
                host: &host,
                is_live: &live,
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

/// Back up every account (the schedule's and "Back up now"'s).
pub async fn push_all(state: &AppState) -> Result<Vec<String>, BackupError> {
    prerequisites(state)?;
    let mut jobs = Vec::new();
    for a in state.accounts.accounts() {
        jobs.push(push(state, &a.id).await?);
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

/// `GET /api/backup/index`: the set of `account`, compared with `into`'s
/// directory (default: the same account).
pub async fn index(
    state: &AppState,
    account_id: &str,
    into: Option<&str>,
) -> Result<IndexView, BackupError> {
    let (folder, pass) = prerequisites(state)?;
    let a = account(state, account_id)?;
    let target = account(state, into.unwrap_or(account_id))?;
    let home = state
        .accounts
        .home(&target.id)
        .ok_or_else(|| BackupError::Invalid("the account has no directory".into()))?;
    let set = Set::new(&folder, &a.name);
    no_set_error(&set, &a, &folder)?;
    let st = state.clone();
    tokio::task::spawn_blocking(move || {
        let keys = super::open_keys(&set, &pass)?;
        let index = super::read_index(&set, &keys)?;
        // Display only.
        let machines = super::read_info(&set)
            .map(|i| i.machines)
            .unwrap_or_default();
        let live = |s: &str| is_live(&st, s);
        let sessions = super::compare(
            &index,
            &home,
            &super::host_name(),
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

/// Restore sessions of `account`'s set into `into`'s directory (a data job
/// with an undo).
pub async fn restore(
    state: &AppState,
    account_id: &str,
    into: &str,
    ids: Vec<String>,
    trust: bool,
) -> Result<String, BackupError> {
    let (folder, pass) = prerequisites(state)?;
    let a = account(state, account_id)?;
    let target = account(state, into)?;
    let home = state
        .accounts
        .home(&target.id)
        .ok_or_else(|| BackupError::Invalid("the account has no directory".into()))?;
    let undo_root = transfer::ops::undo_root(state)
        .ok_or_else(|| BackupError::Invalid("this build has no data directory".into()))?;
    let job = transfer::new_job_id();
    let progress = state.jobs.start(&job, "restore", ids.len() as u64, 0);
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
                passphrase: &pass,
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

/// Unpack the newest configuration bundle of `account`'s set for review.
pub async fn extract_config(state: &AppState, account_id: &str) -> Result<PathBuf, BackupError> {
    let (folder, pass) = prerequisites(state)?;
    let a = account(state, account_id)?;
    let into = state
        .auth
        .config_dir
        .as_ref()
        .map(|d| d.join("restored-config").join(&a.name))
        .ok_or_else(|| BackupError::Invalid("this build has no data directory".into()))?;
    tokio::task::spawn_blocking(move || {
        super::extract_config(&Set::new(&folder, &a.name), &pass, &into)
    })
    .await
    .map_err(|e| BackupError::Io(e.to_string()))?
}

/// Pull `account`'s set into its own directory (§5.11; a data job with an
/// undo). `ids`: only these sessions.
pub async fn pull(
    state: &AppState,
    account_id: &str,
    ids: Option<Vec<String>>,
    trust: bool,
) -> Result<String, BackupError> {
    let (folder, pass) = prerequisites(state)?;
    let a = account(state, account_id)?;
    let home = state
        .accounts
        .home(&a.id)
        .ok_or_else(|| BackupError::Invalid("the account has no directory".into()))?;
    no_set_error(&Set::new(&folder, &a.name), &a, &folder)?;
    let undo_root = transfer::ops::undo_root(state)
        .ok_or_else(|| BackupError::Invalid("this build has no data directory".into()))?;
    let job = transfer::new_job_id();
    let progress = state.jobs.start(&job, "pull", 0, 0);
    let st = state.clone();
    let id = job.clone();
    tokio::spawn(async move {
        let _one = st.jobs.lock.lock().await;
        let set = Set::new(&folder, &a.name);
        let st2 = st.clone();
        let id2 = id.clone();
        let result = tokio::task::spawn_blocking(move || {
            let live = |s: &str| is_live(&st2, s);
            super::pull(&super::PullInputs {
                set: &set,
                passphrase: &pass,
                home: &home,
                host: &super::host_name(),
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

/// Whether no session of `account` was active in the last ten minutes.
async fn idle(state: &AppState, account: &str, now: i64) -> bool {
    let sessions = state.sessions().await;
    !sessions.iter().any(|s| {
        let mine = s
            .account
            .as_ref()
            .map_or(crate::claude::accounts::DEFAULT_ID, |a| a.id.as_str())
            == account;
        mine && (s.last_activity_at.is_some_and(|t| now - t < IDLE_FOR_MS)
            || (s.live && s.status == crate::claude::model::SessionStatus::Working))
    })
}

/// The schedule: every `everyHours`, each idle account is backed up; at
/// launch, with "pull on launch" and nothing live, every account is pulled.
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
            for a in st.accounts.accounts() {
                if let Err(e) = pull(&st, &a.id, None, false).await {
                    tracing::info!(account = %a.id, error = %e, "pull on launch skipped");
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
            for a in st.accounts.accounts() {
                let due = local
                    .accounts
                    .get(&a.id)
                    .and_then(|s| s.last_push)
                    .is_none_or(|t| now - t >= every);
                if due && idle(&st, &a.id, now).await {
                    if let Err(e) = push(&st, &a.id).await {
                        tracing::info!(account = %a.id, error = %e, "scheduled backup skipped");
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
