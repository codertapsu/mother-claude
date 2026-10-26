//! Deleting conversations and projects in bulk (docs/ACCOUNTS.md §14.3): a
//! move with no target.
//!
//! **What goes** is what a move takes from its source (§1.4): every
//! `projects/*/<id>.jsonl` of the conversation in that directory, its
//! `projects/<enc>/<id>/` folder, `file-history/<id>/`, `session-env/<id>`
//! and its todo lists — and a project folder its conversations left empty
//! (with a `.DS_Store` Finder left there). Never `history.jsonl` or
//! `.claude.json`: prompt history and a folder's settings and trust stay, so a
//! delete writes no shared file and runs while Claude Code runs elsewhere in
//! the same directory.
//!
//! **Where from:** with `from`, that account's conversations (in the shared
//! folder, those attributed to it); without, every directory that has them —
//! one job per directory.
//!
//! **Skipped, never refused:** a conversation that runs (a living pidfile,
//! read again right before it goes and once more after; an owned session; a
//! bridge conversation open in the host) or is not there.
//!
//! **Undo:** each conversation's files are renamed into `<app data
//! dir>/undo/<job>/` (copied and removed across volumes), listed in the
//! manifest before they move; the job's Undo is a transfer's, which puts
//! every file back and refuses where something new is there now. Without
//! undo the directory is removed at the end of the job, once every file was
//! taken out. A conversation that cannot go whole is put back and skipped; if
//! even that fails, the undo directory stays and the job offers Undo.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::moves::{Location, LIST_CAP};
use super::{JobState, JobView, Manifest, Mode, Moved, Progress, Skipped, TransferError, What};
use crate::claude::accounts::{AccountKind, DEFAULT_ID, UNREGISTERED_TOKEN_ID};
use crate::claude::{disk, ClaudeHome};
use crate::state::AppState;

/// A delete job's kind, and its manifest's.
pub const KIND: &str = "delete";

fn io(what: &str, path: &Path, e: std::io::Error) -> TransferError {
    TransferError::Io(format!("{what} {}: {e}", path.display()))
}

// ─────────────────────────────── the request ───────────────────────────────

/// `items` of the request.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteItems {
    #[serde(default)]
    pub sessions: Vec<String>,
    /// Absolute folders: every conversation of each.
    #[serde(default)]
    pub projects: Vec<String>,
}

fn keep_undo() -> bool {
    true
}

/// The body of `POST /api/data/delete`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteRequest {
    /// An account: only its conversations. Left out: every directory that
    /// has them.
    #[serde(default)]
    pub from: Option<Location>,
    pub items: DeleteItems,
    /// Keep a copy for seven days, so the job can be undone (the default).
    #[serde(default = "keep_undo")]
    pub undo: bool,
    #[serde(default)]
    pub dry_run: bool,
}

// ─────────────────────────────── the plan ────────────────────────────────

/// One conversation of a plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanSession {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The account whose conversation it is (in the shared folder, the one
    /// it is attributed to).
    pub account: String,
    /// Its files, and their size.
    pub files: usize,
    pub bytes: u64,
}

/// Something a delete leaves where it is, and why: a bulk skip (`id` or
/// `cwd`, `code`, `message`) with the conversation's title when it has one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeleteSkip {
    #[serde(flatten)]
    pub skip: Skipped,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl DeleteSkip {
    fn titled(skip: Skipped, titles: &BTreeMap<String, String>) -> Self {
        let title = skip.id.as_ref().and_then(|id| titles.get(id)).cloned();
        Self { skip, title }
    }
}

/// The dry run's answer: what goes, what is skipped and why, the totals.
/// Each list holds at most 1000 entries (`truncated` beyond); its `…Total`
/// counts them all.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletePlan {
    /// The account named (an id); left out when the request left it out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    pub undo: bool,
    /// One per directory with something to delete (at least 1).
    pub jobs: usize,
    pub sessions: Vec<PlanSession>,
    pub skipped: Vec<DeleteSkip>,
    /// The projects named that something of goes.
    pub projects: Vec<String>,
    /// The `projects/<enc>` folders that would be left empty, and go too.
    pub folders: Vec<String>,
    pub sessions_total: usize,
    pub skipped_total: usize,
    pub projects_total: usize,
    pub folders_total: usize,
    pub files: usize,
    pub bytes: u64,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    #[serde(skip)]
    groups: Vec<Group>,
    /// What was skipped before any directory was known (the first job
    /// reports it).
    #[serde(skip)]
    unsourced: Vec<DeleteSkip>,
}

/// One directory's job.
#[derive(Debug, Clone)]
struct Group {
    /// The account the job names: `from`, else the directory's.
    account: String,
    /// The account whose directory it is (`default` for the shared folder).
    dir_account: String,
    home: ClaudeHome,
    /// With `from`: only its conversations, checked again as each one goes.
    only: Option<String>,
    sessions: Vec<PlanSession>,
    skipped: Vec<DeleteSkip>,
    projects: Vec<String>,
    folders: Vec<PathBuf>,
}

impl Group {
    fn new(account: &str, dir_account: &str, home: ClaudeHome, only: Option<String>) -> Self {
        Self {
            account: account.to_string(),
            dir_account: dir_account.to_string(),
            home,
            only,
            sessions: Vec::new(),
            skipped: Vec::new(),
            projects: Vec::new(),
            folders: Vec::new(),
        }
    }

    fn ids(&self) -> Vec<String> {
        self.sessions.iter().map(|s| s.id.clone()).collect()
    }
}

/// The conversations a living process of `home` runs now, read from its
/// pidfiles (the roster may be a tick behind). Only `<pid>.json` files are
/// read; one that cannot be parsed (being rewritten) is left to the roster.
fn running_in(home: &ClaudeHome) -> HashSet<String> {
    let mut out = HashSet::new();
    for e in std::fs::read_dir(home.sessions_dir())
        .into_iter()
        .flatten()
        .flatten()
    {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(pid) = name
            .strip_suffix(".json")
            .filter(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|p| p.parse::<u32>().ok())
        else {
            continue;
        };
        if !crate::claude::live::pid_exists(pid) {
            continue;
        }
        let Ok(bytes) = std::fs::read(e.path()) else {
            continue;
        };
        if let Some(id) = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|v| v.get("sessionId")?.as_str().map(str::to_string))
        {
            out.insert(id);
        }
    }
    out
}

/// The project folders of a conversation (its transcripts' folders).
fn folders_of(home: &ClaudeHome, id: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = super::transcripts_of(home, id)
        .iter()
        .filter_map(|t| t.parent().map(Path::to_path_buf))
        .collect();
    out.dedup();
    out
}

/// Whether a project folder holds nothing but what `going` takes (and a
/// `.DS_Store`).
fn left_empty(folder: &Path, going: &HashSet<&str>) -> bool {
    std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .all(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name == ".DS_Store" || going.contains(name.strip_suffix(".jsonl").unwrap_or(&name))
        })
}

/// The project folders taking `ids` would leave empty.
fn emptied_folders(home: &ClaudeHome, ids: &[String]) -> Vec<PathBuf> {
    let going: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let mut folders: Vec<PathBuf> = ids.iter().flat_map(|id| folders_of(home, id)).collect();
    folders.sort();
    folders.dedup();
    folders.retain(|f| left_empty(f, &going));
    folders
}

fn account_name(state: &AppState, id: &str) -> String {
    state
        .accounts
        .get(id)
        .map(|a| a.name)
        .unwrap_or_else(|| id.to_string())
}

/// The plan's blocking part: the directories, and in each what goes; every
/// skip titled from `titles`.
fn plan_dirs(
    state: &AppState,
    from: Option<&str>,
    items: &DeleteItems,
    titles: &BTreeMap<String, String>,
) -> Result<(Vec<Group>, Vec<DeleteSkip>), TransferError> {
    let titled = |s: Skipped| DeleteSkip::titled(s, titles);
    let mut unsourced: Vec<DeleteSkip> = Vec::new();
    let mut ids: Vec<String> = Vec::new();
    let mut named: HashSet<&str> = HashSet::new();
    for id in &items.sessions {
        if !named.insert(id.as_str()) {
            continue;
        }
        if !crate::claude::ids::is_session_id(id) {
            unsourced.push(titled(Skipped::session(
                id,
                "invalid_transfer",
                format!("{id:?} is not a session id"),
            )));
            continue;
        }
        ids.push(id.clone());
    }
    let mut cwds: Vec<String> = Vec::new();
    for raw in &items.projects {
        match disk::clean_cwd(raw) {
            Some(c) if !cwds.contains(&c) => cwds.push(c),
            Some(_) => {}
            None => unsourced.push(titled(Skipped::project(
                raw,
                "invalid_transfer",
                "a project is named by its absolute folder",
            ))),
        }
    }
    let dirs: Vec<(String, ClaudeHome, Option<String>)> = match from {
        Some(a) => {
            let home = state
                .accounts
                .home(a)
                .ok_or_else(|| TransferError::NotFound(format!("no account {a:?}")))?;
            let dir_account = state
                .accounts
                .account_for_dir(home.base())
                .unwrap_or_else(|| DEFAULT_ID.to_string());
            vec![(dir_account, home, Some(a.to_string()))]
        }
        None => state
            .accounts
            .homes()
            .into_iter()
            .map(|(a, h)| (a, h, None))
            .collect(),
    };
    let mut found_ids: HashSet<String> = HashSet::new();
    let mut found_cwds: HashSet<String> = HashSet::new();
    let mut groups = Vec::new();
    for (dir_account, home, only) in dirs {
        let account = only.clone().unwrap_or_else(|| dir_account.clone());
        let owner = |id: &str| disk::owner_in(state, &dir_account, id);
        let ours = |id: &str| only.as_deref().is_none_or(|a| owner(id) == a);
        let mut g = Group::new(&account, &dir_account, home.clone(), only.clone());
        let mut skipped: Vec<Skipped> = Vec::new();
        let mut wanted: Vec<String> = Vec::new();
        let mut taken: HashSet<String> = HashSet::new();
        for cwd in &cwds {
            let mine: Vec<String> = super::project_sessions(&home, cwd)
                .into_iter()
                .filter(|id| ours(id))
                .collect();
            if mine.is_empty() {
                if only.is_some() {
                    skipped.push(Skipped::project(
                        cwd,
                        "not_found",
                        format!("no conversations of {cwd} here"),
                    ));
                }
                continue;
            }
            found_cwds.insert(cwd.clone());
            g.projects.push(cwd.clone());
            for id in mine {
                if taken.insert(id.clone()) {
                    wanted.push(id);
                }
            }
        }
        for id in &ids {
            if super::transcripts_of(&home, id).is_empty() {
                if only.is_some() {
                    skipped.push(Skipped::session(
                        id,
                        "not_found",
                        format!("it is not in {}'s directory", account_name(state, &account)),
                    ));
                }
                continue;
            }
            found_ids.insert(id.clone());
            if !ours(id) {
                skipped.push(Skipped::session(
                    id,
                    "not_found",
                    format!(
                        "it is {}'s conversation; delete it from there",
                        account_name(state, &owner(id))
                    ),
                ));
                continue;
            }
            if taken.insert(id.clone()) {
                wanted.push(id.clone());
            }
        }
        let running = running_in(&home);
        for id in wanted {
            if super::ops::is_live(state, &id) || running.contains(&id) {
                skipped.push(Skipped::of(&id, &TransferError::SessionLive(id.clone())));
                continue;
            }
            match super::plan_session(&home, &home, &id) {
                Ok(p) => g.sessions.push(PlanSession {
                    account: owner(&id),
                    cwd: p.cwd,
                    title: titles.get(&id).cloned(),
                    files: p.files.len(),
                    bytes: p.files.iter().map(|f| f.size).sum(),
                    id,
                }),
                Err(e) => skipped.push(Skipped::of(&id, &e)),
            }
        }
        g.skipped = skipped.into_iter().map(titled).collect();
        g.folders = emptied_folders(&home, &g.ids());
        groups.push(g);
    }
    if from.is_none() {
        for id in ids.iter().filter(|id| !found_ids.contains(*id)) {
            unsourced.push(titled(Skipped::session(
                id,
                "not_found",
                "no account has this conversation",
            )));
        }
        for cwd in cwds.iter().filter(|c| !found_cwds.contains(*c)) {
            unsourced.push(titled(Skipped::project(
                cwd,
                "not_found",
                format!("no account has conversations of {cwd}"),
            )));
        }
    }
    // A directory with nothing to delete is no job: what it skipped is
    // listed with the rest.
    let (groups, idle): (Vec<Group>, Vec<Group>) =
        groups.into_iter().partition(|g| !g.sessions.is_empty());
    for g in idle {
        unsourced.extend(g.skipped);
    }
    Ok((groups, unsourced))
}

/// Plan a request: every check, nothing touched. Refuses only what is wrong
/// with the request as a whole; what cannot go is skipped.
pub async fn plan(state: &AppState, req: &DeleteRequest) -> Result<DeletePlan, TransferError> {
    if req.items.sessions.is_empty() && req.items.projects.is_empty() {
        return Err(TransferError::Invalid(
            "name what to delete: items.sessions or items.projects".into(),
        ));
    }
    let from = match &req.from {
        None => None,
        // A token no account holds: its conversations are the Default's for
        // every decision (D8), as the chip's account names it.
        Some(Location::Account(a)) if a == UNREGISTERED_TOKEN_ID => Some(DEFAULT_ID.to_string()),
        Some(Location::Account(a)) => {
            state
                .accounts
                .get(a)
                .ok_or_else(|| TransferError::NotFound(format!("no account {a:?}")))?;
            Some(a.clone())
        }
        Some(Location::Storage { .. }) => {
            return Err(TransferError::Invalid(
                "a delete takes conversations out of an account's directory: name an account"
                    .into(),
            ))
        }
    };
    let titles = crate::claude::backup::ops::titles(state).await;
    let (st, items, from2) = (state.clone(), req.items.clone(), from.clone());
    let (groups, unsourced) =
        tokio::task::spawn_blocking(move || plan_dirs(&st, from2.as_deref(), &items, &titles))
            .await
            .map_err(|e| TransferError::Io(e.to_string()))??;
    Ok(aggregate(req, from, groups, unsourced))
}

fn cap<T>(v: &mut Vec<T>, truncated: &mut bool) {
    if v.len() > LIST_CAP {
        v.truncate(LIST_CAP);
        *truncated = true;
    }
}

/// The plan of every job together.
fn aggregate(
    req: &DeleteRequest,
    from: Option<String>,
    groups: Vec<Group>,
    unsourced: Vec<DeleteSkip>,
) -> DeletePlan {
    // The same conversation skipped in two directories for the same reason
    // is listed once.
    let mut seen: HashSet<(Option<String>, Option<String>, String)> = HashSet::new();
    let mut skipped: Vec<DeleteSkip> = unsourced
        .iter()
        .chain(groups.iter().flat_map(|g| &g.skipped))
        .filter(|s| {
            let k = &s.skip;
            seen.insert((k.id.clone(), k.cwd.clone(), k.code.clone()))
        })
        .cloned()
        .collect();
    let mut projects: Vec<String> = Vec::new();
    for p in groups.iter().flat_map(|g| &g.projects) {
        if !projects.contains(p) {
            projects.push(p.clone());
        }
    }
    let mut sessions: Vec<PlanSession> = groups.iter().flat_map(|g| g.sessions.clone()).collect();
    let mut folders: Vec<String> = groups
        .iter()
        .flat_map(|g| g.folders.iter().map(|f| f.to_string_lossy().into_owned()))
        .collect();
    let (sessions_total, skipped_total, projects_total, folders_total) =
        (sessions.len(), skipped.len(), projects.len(), folders.len());
    let files = sessions.iter().map(|s| s.files).sum();
    let bytes = sessions.iter().map(|s| s.bytes).sum();
    let mut truncated = false;
    cap(&mut sessions, &mut truncated);
    cap(&mut skipped, &mut truncated);
    cap(&mut projects, &mut truncated);
    cap(&mut folders, &mut truncated);
    DeletePlan {
        from,
        undo: req.undo,
        jobs: groups.len().max(1),
        sessions,
        skipped,
        projects,
        folders,
        sessions_total,
        skipped_total,
        projects_total,
        folders_total,
        files,
        bytes,
        truncated,
        groups,
        unsourced,
    }
}

// ─────────────────────────────── taking them ───────────────────────────────

/// What taking one directory's conversations needs.
pub struct TakeInputs<'a> {
    pub home: &'a ClaudeHome,
    pub ids: &'a [String],
    pub undo_root: &'a Path,
    pub job_id: &'a str,
    /// The account the job names (its manifest's).
    pub account: &'a str,
    /// Keep the undo copy; else it is removed once every file was taken.
    pub keep_undo: bool,
    /// Whether a conversation runs anywhere the app knows of (the roster,
    /// an owned session, a bridge conversation); this directory's pidfiles
    /// are read here as well.
    pub is_live: &'a dyn Fn(&str) -> bool,
    /// Whether a conversation may still go (`Err`: skipped so).
    pub may_go: &'a dyn Fn(&str) -> Result<(), Skipped>,
    /// The token account a conversation is attributed to, if any (its undo
    /// records it again).
    pub attributed: &'a dyn Fn(&str) -> Option<String>,
    pub progress: &'a Progress,
}

/// What a delete did.
#[derive(Debug, Default)]
pub struct TakeReport {
    pub deleted: Vec<String>,
    pub skipped: Vec<Skipped>,
    pub warnings: Vec<String>,
    /// While an undo is kept.
    pub manifest: Option<Manifest>,
}

/// One conversation's outcome.
enum Taken {
    Gone,
    Skipped(Skipped),
}

/// Put moved files back, last first.
fn put_back_all(moved: &[Moved], undo_dir: &Path) -> Result<(), TransferError> {
    for mv in moved.iter().rev() {
        super::put_back(&undo_dir.join(&mv.saved), &mv.original)?;
    }
    Ok(())
}

/// Take a project folder its conversations left empty (a `.DS_Store` Finder
/// left there goes into the undo directory first; when it cannot, the folder
/// stays). Whether it went.
fn take_folder(
    folder: &Path,
    home: &ClaudeHome,
    undo_dir: &Path,
    m: &mut Manifest,
    save: &dyn Fn(&Manifest) -> Result<(), TransferError>,
) -> Result<bool, TransferError> {
    if !super::is_plain_dir(folder) || !left_empty(folder, &HashSet::new()) {
        return Ok(false);
    }
    let ds_store = folder.join(".DS_Store");
    if super::is_plain_file(&ds_store) {
        let rel = ds_store.strip_prefix(home.base()).unwrap_or(&ds_store);
        m.moved.push(Moved {
            original: ds_store.clone(),
            saved: PathBuf::from("moved").join(rel),
        });
        save(m)?;
        if super::stash(&ds_store, undo_dir, rel).is_err() {
            m.moved.pop();
            return Ok(false);
        }
    }
    Ok(std::fs::remove_dir(folder).is_ok())
}

/// Take one conversation: checked right before, its files listed in the
/// manifest, then moved; put back and skipped when it cannot go whole or
/// started meanwhile. `Err` only when what moved could not be put back (or
/// the manifest not written).
fn take_one(
    inp: &TakeInputs<'_>,
    undo_dir: &Path,
    m: &mut Manifest,
    save: &dyn Fn(&Manifest) -> Result<(), TransferError>,
    id: &str,
) -> Result<Taken, TransferError> {
    let home = inp.home;
    let runs = || (inp.is_live)(id) || running_in(home).contains(id);
    if runs() {
        return Ok(Taken::Skipped(Skipped::of(
            id,
            &TransferError::SessionLive(id.to_string()),
        )));
    }
    if let Err(s) = (inp.may_go)(id) {
        return Ok(Taken::Skipped(s));
    }
    let folders = folders_of(home, id);
    let token_account = (inp.attributed)(id);
    let plan = match super::plan_session(home, home, id) {
        Ok(p) => p,
        Err(e) => return Ok(Taken::Skipped(Skipped::of(id, &e))),
    };
    let rel = |p: &Path| p.strip_prefix(home.base()).unwrap_or(p).to_path_buf();
    // The undo lists what will move before it moves.
    let before = m.moved.len();
    let planned: Vec<Moved> = plan
        .files
        .iter()
        .map(|f| Moved {
            original: f.from.clone(),
            saved: PathBuf::from("moved").join(rel(&f.from)),
        })
        .collect();
    m.moved.extend(planned.iter().cloned());
    m.sessions.push(id.to_string());
    if let Err(e) = save(m) {
        m.moved.truncate(before);
        m.sessions.pop();
        return Err(e);
    }
    let mut moved = 0;
    let result = plan.files.iter().try_for_each(|f| {
        super::stash(&f.from, undo_dir, &rel(&f.from))?;
        moved += 1;
        inp.progress.files_done.fetch_add(1, Ordering::Relaxed);
        inp.progress.bytes_done.fetch_add(f.size, Ordering::Relaxed);
        Ok::<(), TransferError>(())
    });
    // One that started meanwhile may have opened a file before it moved:
    // all of it goes back.
    let failed = match result {
        Err(e) => Some(Skipped::of(id, &e)),
        Ok(()) if runs() => Some(Skipped::of(id, &TransferError::SessionLive(id.to_string()))),
        Ok(()) => None,
    };
    if let Some(skip) = failed {
        if let Err(back) = put_back_all(&planned[..moved], undo_dir) {
            return Err(TransferError::Incomplete(format!(
                "{} could not be deleted ({}), and putting it back failed too ({back}); \
                 nothing is lost: undo this job to finish.",
                id, skip.message
            )));
        }
        m.moved.truncate(before);
        m.sessions.pop();
        save(m)?;
        return Ok(Taken::Skipped(skip));
    }
    // Its own folders when nothing else is in them, then a project folder
    // it left empty.
    for (dir, _) in plan.dirs.iter().rev() {
        super::remove_empty_dirs(dir);
        if !dir.exists() {
            m.moved_dirs.push(dir.clone());
        }
    }
    for folder in folders {
        if take_folder(&folder, home, undo_dir, m, save)? {
            m.moved_dirs.push(folder);
        }
    }
    if let Some(account) = token_account {
        m.token_accounts.push((id.to_string(), account));
    }
    save(m)?;
    Ok(Taken::Gone)
}

/// Delete one directory's conversations. Blocking.
pub fn take(inp: &TakeInputs<'_>) -> Result<TakeReport, TransferError> {
    let home = inp.home;
    let undo_dir = inp.undo_root.join(inp.job_id);
    std::fs::create_dir_all(&undo_dir).map_err(|e| io("creating", &undo_dir, e))?;
    super::private_dir(inp.undo_root);
    super::private_dir(&undo_dir);
    let first = inp.ids.first().cloned().unwrap_or_default();
    // A delete has no target: its files went into its own undo directory.
    let mut m = Manifest {
        kind: Some(KIND.to_string()),
        from_config: Some(home.claude_json()),
        from_account: Some(inp.account.to_string()),
        ..Manifest::new(
            inp.job_id,
            Mode::Move,
            What::Session { id: first },
            home.base().to_path_buf(),
            undo_dir.clone(),
        )
    };
    let manifest_path = undo_dir.join("manifest.json");
    let save = |m: &Manifest| -> Result<(), TransferError> {
        super::fail_point("save")?;
        crate::persist::write_json_private(&manifest_path, m)
            .map_err(|e| io("writing", &undo_dir, e))
    };
    if let Err(e) = save(&m) {
        let _ = std::fs::remove_dir_all(&undo_dir);
        return Err(e);
    }
    let mut report = TakeReport::default();
    for id in inp.ids {
        match take_one(inp, &undo_dir, &mut m, &save, id) {
            Ok(Taken::Gone) => report.deleted.push(id.clone()),
            Ok(Taken::Skipped(s)) => report.skipped.push(s),
            Err(e) if m.moved.is_empty() => {
                let _ = std::fs::remove_dir_all(&undo_dir);
                return Err(e);
            }
            Err(e) => {
                // What was taken is only in the undo directory now: it stays,
                // and the job offers Undo.
                let _ = crate::persist::write_json_private(&manifest_path, &m);
                tracing::error!(job = inp.job_id, error = %e, "a delete stopped; its undo directory stays");
                return Err(match e {
                    TransferError::Incomplete(_) => e,
                    other => TransferError::Incomplete(format!(
                        "{other}. What was deleted so far is kept: undo this job to put it back."
                    )),
                });
            }
        }
    }
    if report.deleted.is_empty() {
        let _ = std::fs::remove_dir_all(&undo_dir);
        return Ok(report);
    }
    if !inp.keep_undo {
        // Gone for good: the manifest first, so no Undo is offered for what
        // is being removed.
        let gone =
            std::fs::remove_file(&manifest_path).and_then(|()| std::fs::remove_dir_all(&undo_dir));
        if let Err(e) = gone {
            tracing::warn!(job = inp.job_id, error = %e, "a delete's files could not all be removed");
            report.warnings.push(format!(
                "Some deleted files could not be removed from {} ({e}); they go with the undo \
                 copies after seven days.",
                undo_dir.display()
            ));
        }
        return Ok(report);
    }
    report.manifest = Some(m);
    Ok(report)
}

// ─────────────────────────────── jobs ───────────────────────────────

/// The job view a group starts with.
fn start_view(state: &AppState, job: &str, g: &Group) {
    let files = g.sessions.iter().map(|s| s.files as u64).sum();
    let bytes = g.sessions.iter().map(|s| s.bytes).sum();
    drop(state.jobs.start(job, KIND, files, bytes));
    let sessions = g.ids();
    state.jobs.update(job, |v| {
        v.sessions = sessions;
        v.account = Some(g.account.clone()).filter(|a| !a.is_empty());
    });
}

/// Start the plan's jobs (one per directory; one that only reports what was
/// skipped when nothing can go) and run them one after another in the
/// background. Returns their views as they start.
pub fn start(state: &AppState, plan: DeletePlan) -> Vec<JobView> {
    let keep = plan.undo;
    let mut groups = plan.groups;
    match groups.first_mut() {
        Some(first) => {
            let mut skipped = plan.unsourced;
            skipped.append(&mut first.skipped);
            first.skipped = skipped;
        }
        None => {
            let account = plan.from.clone().unwrap_or_default();
            let home = state.accounts.default_home().clone();
            let mut g = Group::new(&account, DEFAULT_ID, home, None);
            g.skipped = plan.unsourced;
            groups.push(g);
        }
    }
    let ids: Vec<String> = groups.iter().map(|_| super::new_job_id()).collect();
    for (g, id) in groups.iter().zip(&ids) {
        start_view(state, id, g);
    }
    let views = ids.iter().filter_map(|id| state.jobs.get(id)).collect();
    let st = state.clone();
    tokio::spawn(async move {
        for (g, id) in groups.into_iter().zip(ids) {
            let nothing = g.sessions.is_empty();
            if let Err(e) = run_group(&st, &id, g, keep).await {
                tracing::warn!(job = %id, error = %e, "deleting conversations failed");
            }
            if !nothing {
                super::ops::rediscover(&st).await;
            }
        }
    });
    views
}

/// Run one job, with the data jobs' lock (it may wait behind a backup or a
/// move).
async fn run_group(state: &AppState, job: &str, g: Group, keep: bool) -> Result<(), TransferError> {
    let one = state.jobs.lock.lock().await;
    let result = if g.sessions.is_empty() {
        Ok(TakeReport::default())
    } else {
        match super::ops::undo_root(state) {
            Some(root) => run_locked(state, job, root, &g, keep).await,
            None => Err(TransferError::Invalid(
                "this build has no data directory".into(),
            )),
        }
    };
    drop(one);
    if let Ok(r) = &result {
        // An API server no longer joins or names a conversation that is gone.
        crate::bridge::forget_deleted(state, &r.deleted).await;
    }
    let failed = result.as_ref().err().cloned();
    finish(state, job, &g, result);
    failed.map_or(Ok(()), Err)
}

async fn run_locked(
    state: &AppState,
    job: &str,
    root: PathBuf,
    g: &Group,
    keep: bool,
) -> Result<TakeReport, TransferError> {
    let (st, g2, job2) = (state.clone(), g.clone(), job.to_string());
    let progress = state.jobs.progress(job).unwrap_or_default();
    tokio::task::spawn_blocking(move || {
        super::prune_undo(&root);
        let ids = g2.ids();
        let live = |id: &str| super::ops::is_live(&st, id);
        // Given to another account meanwhile (an assign), it is not the one
        // the request named any more.
        let may_go = |id: &str| match &g2.only {
            Some(a) if disk::owner_in(&st, &g2.dir_account, id) != *a => Err(Skipped::session(
                id,
                "session_changed",
                "it was given to another account meanwhile",
            )),
            _ => Ok(()),
        };
        let attributed = |id: &str| {
            let owner = disk::owner_in(&st, &g2.dir_account, id);
            st.accounts
                .get(&owner)
                .is_some_and(|a| a.kind == AccountKind::Token)
                .then_some(owner)
        };
        take(&TakeInputs {
            home: &g2.home,
            ids: &ids,
            undo_root: &root,
            job_id: &job2,
            account: &g2.account,
            keep_undo: keep,
            is_live: &live,
            may_go: &may_go,
            attributed: &attributed,
            progress: &progress,
        })
    })
    .await
    .map_err(|e| TransferError::Io(e.to_string()))?
}

/// Finish a job's view: `result: {deleted, skipped, warnings?}`, and Undo
/// while a copy is kept.
fn finish(state: &AppState, job: &str, g: &Group, result: Result<TakeReport, TransferError>) {
    let at = chrono::Utc::now().timestamp_millis();
    state.jobs.update(job, |v| {
        v.finished_at = Some(at);
        match result {
            Ok(r) => {
                // A conversation skipped as it was about to go is titled as
                // the plan titled it.
                let titles: BTreeMap<String, String> = g
                    .sessions
                    .iter()
                    .filter_map(|s| Some((s.id.clone(), s.title.clone()?)))
                    .collect();
                let mut skipped = g.skipped.clone();
                skipped.extend(
                    r.skipped
                        .into_iter()
                        .map(|s| DeleteSkip::titled(s, &titles)),
                );
                let mut out = json!({
                    "deleted": r.deleted,
                    "skipped": skipped[..skipped.len().min(LIST_CAP)],
                });
                if !r.warnings.is_empty() {
                    out["warnings"] = json!(r.warnings);
                }
                v.state = JobState::Done;
                v.result = Some(out);
                v.sessions = r.deleted.clone();
                v.files_done = v.files_total;
                v.bytes_done = v.bytes_total;
                v.undo_until = r
                    .manifest
                    .is_some()
                    .then(|| at + super::UNDO_KEEP.as_millis() as i64);
            }
            Err(e) => {
                v.state = JobState::Failed;
                v.error = Some(e.to_string());
                v.error_code = Some(e.code().to_string());
                if matches!(e, TransferError::Incomplete(_)) {
                    v.undo_until = Some(at + super::UNDO_KEEP.as_millis() as i64);
                }
            }
        }
    });
}

/// What a request's audit line says (ids and counts, never a path).
pub fn audit_detail(plan: &DeletePlan, jobs: &[JobView]) -> Value {
    json!({
        "jobs": jobs.iter().map(|j| j.id.clone()).collect::<Vec<_>>(),
        "from": plan.from,
        "undo": plan.undo,
        "sessions": plan.sessions_total,
        "skipped": plan.skipped_total,
        "projects": plan.projects_total,
        "files": plan.files,
        "bytes": plan.bytes,
    })
}
