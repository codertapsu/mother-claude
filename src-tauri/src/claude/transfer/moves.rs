//! Move data anywhere (docs/ACCOUNTS.md §13): the `items` body of
//! `POST /api/data/move`. A request names a source and a target location —
//! an account, or a set of a storage — and any mix of items; its plan says
//! everything it would do, and one data job per source does it.
//!
//! | from → to | an account in another directory | one in the same directory | a storage |
//! |---|---|---|---|
//! | an account | move or copy (the transfer engine) | assign (a move; a copy is refused) | copy (a push of just those) or move (archive) |
//! | a storage | copy (a restore) or move (a restore, then out of the set) | | copy or move between sets (re-encrypted, checked) |
//!
//! With only `items.sessions`, `from` may be left out: each conversation goes
//! from where it is (its directory; in the shared folder, the account it is
//! attributed to), one job per source. Bulk skips, it never refuses for one
//! conversation: what cannot go is listed with its code (§13.3 rule 2).

use std::collections::{BTreeMap, HashSet};

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Value};

use super::custom::{Custom, CustomItem};
use super::{JobState, JobView, Mode, Skipped, TransferError};
use crate::claude::accounts::{AccountKind, DEFAULT_ID};
use crate::claude::backup::{self, storage::Storage, BackupError, Set};
use crate::claude::summaries::{acting_account, session_account};
use crate::claude::ClaudeHome;
use crate::state::AppState;

/// The most entries a plan's lists carry (`truncated` beyond).
pub const LIST_CAP: usize = 1000;

/// Where data lives (§13.1): an account, or a set of a storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    Account(String),
    Storage {
        storage: String,
        set: Option<String>,
    },
}

impl Serialize for Location {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Account(id) => json!({ "account": id }).serialize(s),
            Self::Storage { storage, set } => match set {
                Some(set) => json!({ "storage": storage, "set": set }).serialize(s),
                None => json!({ "storage": storage }).serialize(s),
            },
        }
    }
}

impl<'de> Deserialize<'de> for Location {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let bad = || {
            D::Error::custom(
                "a location is an account id, {\"account\": id} or {\"storage\": id, \"set\": name}",
            )
        };
        match Value::deserialize(d)? {
            Value::String(id) => Ok(Self::Account(id)),
            Value::Object(o) => match (o.get("account"), o.get("storage")) {
                (Some(Value::String(id)), None) => Ok(Self::Account(id.clone())),
                (None, Some(Value::String(storage))) => Ok(Self::Storage {
                    storage: storage.clone(),
                    set: match o.get("set") {
                        None | Some(Value::Null) => None,
                        Some(Value::String(s)) => Some(s.clone()),
                        Some(_) => return Err(bad()),
                    },
                }),
                _ => Err(bad()),
            },
            _ => Err(bad()),
        }
    }
}

/// `items` of the request.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemsBody {
    #[serde(default)]
    pub sessions: Vec<String>,
    #[serde(default)]
    pub projects: Vec<String>,
    /// Every conversation of `from`.
    #[serde(default)]
    pub all: bool,
    #[serde(default)]
    pub customizations: Vec<Custom>,
}

impl ItemsBody {
    fn is_empty(&self) -> bool {
        self.sessions.is_empty()
            && self.projects.is_empty()
            && !self.all
            && self.customizations.is_empty()
    }
}

/// The `items` body of `POST /api/data/move` (§13.4).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveRequest {
    #[serde(default)]
    pub from: Option<Location>,
    pub to: Location,
    pub items: ItemsBody,
    pub mode: Mode,
    /// Carry the projects' trust and entries.
    #[serde(default)]
    pub trust: bool,
    /// Customizations: replace what the target has.
    #[serde(default)]
    pub replace_existing: bool,
    #[serde(default)]
    pub dry_run: bool,
}

/// What a plan does with one conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    Move,
    Copy,
    Assign,
    Archive,
    Restore,
}

/// One conversation of a plan.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanSession {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub files: usize,
    pub bytes: u64,
    pub from: Location,
    pub action: Action,
}

/// The plan of a request (the dry run's answer, §13.4).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MovePlan {
    /// Left out when the request left it out (each conversation names its
    /// own).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<Location>,
    pub to: Location,
    pub mode: Mode,
    pub jobs: usize,
    pub sessions: Vec<PlanSession>,
    pub skipped: Vec<Skipped>,
    pub projects: Vec<String>,
    pub adds_project_entries: Vec<String>,
    pub removes_project_entries: Vec<String>,
    pub history_lines: usize,
    pub customizations: Vec<CustomItem>,
    pub warnings: Vec<String>,
    pub files: usize,
    pub bytes: u64,
    pub writes_shared: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    #[serde(skip)]
    groups: Vec<Group>,
    /// What was skipped before any source was known (the first job reports
    /// it).
    #[serde(skip)]
    unsourced: Vec<Skipped>,
}

/// What one job does.
#[derive(Debug, Clone)]
enum Work {
    /// Between two directories (the transfer engine), planned again when it
    /// runs.
    Transfer {
        from: String,
        to: String,
        items: super::Items,
    },
    /// Between accounts that share a directory: each conversation recorded
    /// as `to`'s; with the account it was before.
    Assign {
        from: String,
        to: String,
        sessions: Vec<(String, String)>,
    },
    /// Into a set: a push of these (copy), or a move (archive).
    Into {
        from: String,
        storage: Storage,
        set: String,
        ids: Vec<String>,
    },
    /// Out of a set into an account: a restore, and for a move the entries
    /// out of the set.
    OutOf {
        storage: Storage,
        set: String,
        to: String,
        ids: Vec<String>,
    },
    /// From one set to another: re-encrypted to the target's key and
    /// checked; for a move the entries then leave the source set.
    Between {
        from: Storage,
        from_set: String,
        to: Storage,
        to_set: String,
        ids: Vec<String>,
    },
    /// Everything was skipped: a job that only says so.
    Nothing,
}

/// One job of a plan, with what its dry run found.
#[derive(Debug, Clone)]
struct Group {
    work: Work,
    mode: Mode,
    trust: bool,
    from: Location,
    to: Location,
    sessions: Vec<PlanSession>,
    skipped: Vec<Skipped>,
    projects: Vec<String>,
    adds: Vec<String>,
    removes: Vec<String>,
    history_lines: usize,
    customizations: Vec<CustomItem>,
    warnings: Vec<String>,
    writes_shared: bool,
    /// What it would write: files and bytes.
    files: usize,
    bytes: u64,
}

impl Group {
    fn new(work: Work, mode: Mode, trust: bool, from: Location, to: Location) -> Self {
        Self {
            work,
            mode,
            trust,
            from,
            to,
            sessions: Vec::new(),
            skipped: Vec::new(),
            projects: Vec::new(),
            adds: Vec::new(),
            removes: Vec::new(),
            history_lines: 0,
            customizations: Vec::new(),
            warnings: Vec::new(),
            writes_shared: false,
            files: 0,
            bytes: 0,
        }
    }

    /// Files and bytes from its conversations.
    fn tally(&mut self) {
        self.files = self.sessions.iter().map(|s| s.files).sum();
        self.bytes = self.sessions.iter().map(|s| s.bytes).sum();
    }
}

fn invalid(why: impl Into<String>) -> TransferError {
    TransferError::Invalid(why.into())
}

/// An account that exists.
fn account_of(
    state: &AppState,
    id: &str,
) -> Result<crate::claude::accounts::Account, TransferError> {
    state
        .accounts
        .get(id)
        .ok_or_else(|| TransferError::NotFound(format!("no account {id:?}")))
}

fn home_of(state: &AppState, id: &str) -> Result<ClaudeHome, TransferError> {
    state
        .accounts
        .home(id)
        .ok_or_else(|| TransferError::NotFound(format!("no account {id:?}")))
}

fn same_dir(a: &ClaudeHome, b: &ClaudeHome) -> bool {
    let canon = |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(a.base()) == canon(b.base())
}

/// The name of the set an account's conversations go to by default: its own
/// directory's (a token account's is the Default's).
fn default_set(state: &AppState, account: &str) -> Result<String, TransferError> {
    let a = account_of(state, account)?;
    Ok(backup::ops::set_owner(state, a).name)
}

/// Whether a conversation of `from`'s directory is `from`'s. In the shared
/// folder, while token accounts exist, only those attributed to it are; a
/// directory account's are all its own.
fn belongs(state: &AppState, from: &str, id: &str) -> Result<(), (String, String)> {
    let shared = state
        .accounts
        .home(from)
        .is_some_and(|h| same_dir(&h, state.accounts.default_home()));
    if !shared || !state.accounts.has_token_accounts() {
        return Ok(());
    }
    let acting = acting_account(state, id);
    if acting == from {
        return Ok(());
    }
    let name = state
        .accounts
        .get(&acting)
        .map(|a| a.name)
        .unwrap_or(acting);
    Err((
        "not_found".to_string(),
        format!("it is {name}'s conversation; move it from there"),
    ))
}

/// Where a conversation is now: the account whose directory holds it (in the
/// shared folder, the account it is attributed to). `None`: nowhere known.
fn location_of(state: &AppState, id: &str) -> Option<String> {
    let home = state.index.home_account_of(id)?;
    if home == DEFAULT_ID {
        Some(acting_account(state, id))
    } else {
        Some(home)
    }
}

fn cap<T>(v: &mut Vec<T>, truncated: &mut bool) {
    if v.len() > LIST_CAP {
        v.truncate(LIST_CAP);
        *truncated = true;
    }
}

/// Plan a request: every check, nothing written (it may open a set, which
/// takes the passphrase). Refuses only what is wrong with the request as a
/// whole; what cannot go is skipped.
pub async fn plan(state: &AppState, req: &MoveRequest) -> Result<MovePlan, TransferError> {
    if req.items.is_empty() {
        return Err(invalid(
            "name what to move: items.sessions, items.projects, items.all or \
             items.customizations",
        ));
    }
    let items = &req.items;
    if req.from.is_none()
        && (!items.projects.is_empty() || items.all || !items.customizations.is_empty())
    {
        return Err(invalid(
            "name `from` for projects, everything or customizations; only conversations go \
             from where each one is",
        ));
    }
    let storage_side = matches!(req.to, Location::Storage { .. })
        || matches!(req.from, Some(Location::Storage { .. }));
    if !items.customizations.is_empty() {
        if req.mode == Mode::Move {
            return Err(invalid(
                "customizations are only copied, never moved: choose Copy, or leave them out",
            ));
        }
        if storage_side {
            return Err(invalid("customizations go between accounts only"));
        }
    }
    let titles = backup::ops::titles(state).await;
    let mut groups: Vec<Group> = Vec::new();
    let mut skipped: Vec<Skipped> = Vec::new();
    match &req.from {
        Some(from) => {
            groups.push(plan_one(state, req, from, &req.to, items, true, &titles).await?);
        }
        None => {
            // Each conversation from where it is, one job per source.
            let mut by_source: BTreeMap<String, Vec<String>> = BTreeMap::new();
            let mut seen = HashSet::new();
            for id in &items.sessions {
                if !seen.insert(id.clone()) {
                    continue;
                }
                if !crate::claude::ids::is_session_id(id) {
                    skipped.push(Skipped::session(
                        id,
                        "invalid_transfer",
                        format!("{id:?} is not a session id"),
                    ));
                    continue;
                }
                match location_of(state, id) {
                    Some(source) => by_source.entry(source).or_default().push(id.clone()),
                    None => skipped.push(Skipped::session(
                        id,
                        "not_found",
                        "no account has this conversation",
                    )),
                }
            }
            for (source, ids) in by_source {
                let one = ItemsBody {
                    sessions: ids.clone(),
                    ..ItemsBody::default()
                };
                let from = Location::Account(source.clone());
                match plan_one(state, req, &from, &req.to, &one, false, &titles).await {
                    // A source with nothing to do is no job: what it
                    // skipped is listed with the rest.
                    Ok(g) if g.sessions.is_empty() => skipped.extend(g.skipped),
                    Ok(g) => groups.push(g),
                    // A source that cannot go as a whole: its conversations
                    // are skipped with that code (bulk never refuses).
                    Err(e) => skipped.extend(
                        ids.iter()
                            .map(|id| Skipped::session(id, e.code(), e.to_string())),
                    ),
                }
            }
        }
    }
    Ok(aggregate(req, groups, skipped))
}

/// A backup engine's skip as Move data lists it (a bad id is
/// `invalid_transfer` here, as everywhere else in Move data).
fn skipped_of(s: &backup::Skip) -> Skipped {
    let code = match s.code.as_str() {
        "invalid_id" => "invalid_transfer",
        other => other,
    };
    Skipped::session(&s.id, code, s.reason.clone())
}

/// The plan of all jobs together.
fn aggregate(req: &MoveRequest, groups: Vec<Group>, unsourced: Vec<Skipped>) -> MovePlan {
    let mut skipped = unsourced.clone();
    let mut plan = MovePlan {
        from: req.from.clone(),
        to: req.to.clone(),
        mode: req.mode,
        jobs: 0,
        sessions: Vec::new(),
        skipped: Vec::new(),
        projects: Vec::new(),
        adds_project_entries: Vec::new(),
        removes_project_entries: Vec::new(),
        history_lines: 0,
        customizations: Vec::new(),
        warnings: Vec::new(),
        files: 0,
        bytes: 0,
        writes_shared: false,
        truncated: false,
        groups: Vec::new(),
        unsourced,
    };
    for g in &groups {
        plan.sessions.extend(g.sessions.iter().cloned());
        skipped.extend(g.skipped.iter().cloned());
        plan.projects.extend(g.projects.iter().cloned());
        plan.adds_project_entries.extend(g.adds.iter().cloned());
        plan.removes_project_entries
            .extend(g.removes.iter().cloned());
        plan.history_lines += g.history_lines;
        plan.customizations.extend(g.customizations.iter().cloned());
        plan.warnings.extend(g.warnings.iter().cloned());
        plan.files += g.files;
        plan.bytes += g.bytes;
        plan.writes_shared |= g.writes_shared;
    }
    plan.skipped = skipped;
    plan.jobs = groups.len().max(1);
    plan.groups = groups;
    let mut t = false;
    cap(&mut plan.sessions, &mut t);
    cap(&mut plan.skipped, &mut t);
    cap(&mut plan.projects, &mut t);
    cap(&mut plan.adds_project_entries, &mut t);
    cap(&mut plan.removes_project_entries, &mut t);
    cap(&mut plan.customizations, &mut t);
    cap(&mut plan.warnings, &mut t);
    plan.truncated = t;
    plan
}

/// The ids a source's items name in its directory: the conversations, every
/// conversation of the projects that is the source's, everything of it.
/// `all` becomes projects (each recorded folder) plus the conversations with
/// no folder, so a project's history and entry can go with it.
fn expand_all(state: &AppState, from: &str, home: &ClaudeHome) -> (Vec<String>, Vec<String>) {
    let mut projects: Vec<String> = Vec::new();
    let mut loose = Vec::new();
    for id in backup::session_ids(home) {
        if belongs(state, from, &id).is_err() {
            continue;
        }
        let cwd = super::transcripts_of(home, &id)
            .first()
            .and_then(|t| super::recorded_cwd(t));
        match cwd {
            Some(c) if std::path::Path::new(&c).is_absolute() => {
                if !projects.contains(&c) {
                    projects.push(c);
                }
            }
            _ => loose.push(id),
        }
    }
    projects.sort();
    (projects, loose)
}

/// The conversations of a folder in `home` that are the source's.
fn sessions_of_projects(
    state: &AppState,
    from: &str,
    home: &ClaudeHome,
    projects: &[String],
) -> (Vec<String>, Vec<Skipped>) {
    let mut ids = Vec::new();
    let mut skipped = Vec::new();
    for raw in projects {
        let cwd = match raw.trim_end_matches('/') {
            "" => "/".to_string(),
            c => c.to_string(),
        };
        if !std::path::Path::new(&cwd).is_absolute() {
            skipped.push(Skipped::project(
                raw,
                "invalid_transfer",
                "a project is named by its absolute folder",
            ));
            continue;
        }
        let of: Vec<String> = super::project_sessions(home, &cwd)
            .into_iter()
            .filter(|id| belongs(state, from, id).is_ok())
            .collect();
        if of.is_empty() {
            skipped.push(Skipped::project(
                &cwd,
                "not_found",
                format!("no conversations of {cwd} here"),
            ));
        }
        ids.extend(of);
    }
    (ids, skipped)
}

/// Plan one source's job. `explicit`: the request named `from` (a source
/// that cannot go is then refused, else its conversations are skipped).
async fn plan_one(
    state: &AppState,
    req: &MoveRequest,
    from: &Location,
    to: &Location,
    items: &ItemsBody,
    explicit: bool,
    titles: &BTreeMap<String, String>,
) -> Result<Group, TransferError> {
    let title = |id: &str| titles.get(id).cloned();
    match (from, to) {
        (Location::Account(a), Location::Account(b)) => {
            account_of(state, a)?;
            account_of(state, b)?;
            let (ha, hb) = (home_of(state, a)?, home_of(state, b)?);
            if a == b {
                if explicit {
                    return Err(TransferError::SameDirectory);
                }
                let mut g = Group::new(
                    Work::Assign {
                        from: a.clone(),
                        to: b.clone(),
                        sessions: Vec::new(),
                    },
                    req.mode,
                    req.trust,
                    from.clone(),
                    to.clone(),
                );
                g.skipped = items
                    .sessions
                    .iter()
                    .map(|id| Skipped::session(id, "already_exists", "it is there already"))
                    .collect();
                return Ok(g);
            }
            if same_dir(&ha, &hb) {
                if req.mode == Mode::Copy || !items.customizations.is_empty() {
                    return Err(TransferError::SameDirectory);
                }
                return plan_assign(state, req, a, b, &ha, items, title);
            }
            plan_transfer(state, req, a, b, ha, hb, items, title).await
        }
        (Location::Account(a), Location::Storage { storage, set }) => {
            account_of(state, a)?;
            let home = home_of(state, a)?;
            let storage = backup::storage::get(state, storage).map_err(TransferError::from)?;
            storage.require_available().map_err(TransferError::from)?;
            let set = match set {
                Some(s) => backup::clean_set_name(s).map_err(TransferError::from)?,
                None => default_set(state, a)?,
            };
            plan_into(state, req, a, &home, storage, set, items, title).await
        }
        (Location::Storage { storage, set }, Location::Account(b)) => {
            account_of(state, b)?;
            let set = set
                .as_deref()
                .ok_or_else(|| invalid("name the set to move from (`from.set`)"))?;
            let (storage, set_name, set) =
                backup::ops::storage_set(state, storage, set).map_err(TransferError::from)?;
            let home = home_of(state, b)?;
            plan_out_of(state, req, storage, set_name, set, b, &home, items).await
        }
        (
            Location::Storage {
                storage: s1,
                set: set1,
            },
            Location::Storage {
                storage: s2,
                set: set2,
            },
        ) => {
            let set1 = set1
                .as_deref()
                .ok_or_else(|| invalid("name the set to move from (`from.set`)"))?;
            let (from_storage, from_name, from_set) =
                backup::ops::storage_set(state, s1, set1).map_err(TransferError::from)?;
            let to_storage = backup::storage::get(state, s2).map_err(TransferError::from)?;
            to_storage
                .require_available()
                .map_err(TransferError::from)?;
            let to_name = match set2 {
                Some(s) => backup::clean_set_name(s).map_err(TransferError::from)?,
                None => from_name.clone(),
            };
            let to_set = to_storage.set(&to_name);
            let canon =
                |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
            if canon(&from_set.dir) == canon(&to_set.dir) {
                return Err(invalid("that is the same set"));
            }
            plan_between(
                state,
                req,
                (from_storage, from_name, from_set),
                (to_storage, to_name, to_set),
                items,
            )
            .await
        }
    }
}

/// A copy or move from one set to another.
async fn plan_between(
    state: &AppState,
    req: &MoveRequest,
    from: (Storage, String, Set),
    to: (Storage, String, Set),
    items: &ItemsBody,
) -> Result<Group, TransferError> {
    let pass =
        backup::ops::passphrases(state).ok_or(TransferError::from(BackupError::NoPassphrase))?;
    let (items2, from_set, to_set) = (items.clone(), from.2.clone(), to.2.clone());
    let (entries, skips, empty) = tokio::task::spawn_blocking(move || {
        backup::plan_between(
            &from_set,
            &to_set,
            &pass,
            (!items2.all).then_some(items2.sessions.as_slice()),
            &items2.projects,
        )
    })
    .await
    .map_err(|e| TransferError::Io(e.to_string()))?
    .map_err(TransferError::from)?;
    let from_loc = Location::Storage {
        storage: from.0.id.clone(),
        set: Some(from.1.clone()),
    };
    let to_loc = Location::Storage {
        storage: to.0.id.clone(),
        set: Some(to.1.clone()),
    };
    let mut g = Group::new(
        Work::Between {
            from: from.0,
            from_set: from.1,
            to: to.0,
            to_set: to.1,
            ids: entries.iter().map(|e| e.id.clone()).collect(),
        },
        req.mode,
        req.trust,
        from_loc.clone(),
        to_loc,
    );
    g.sessions = entries
        .iter()
        .map(|e| PlanSession {
            id: e.id.clone(),
            cwd: e.cwd.clone(),
            title: e.title.clone(),
            files: e.files as usize,
            bytes: e.archive_bytes,
            from: from_loc.clone(),
            action: match req.mode {
                Mode::Move => Action::Move,
                Mode::Copy => Action::Copy,
            },
        })
        .collect();
    g.skipped = skips
        .iter()
        .map(skipped_of)
        .chain(empty.iter().map(|cwd| {
            Skipped::project(
                cwd,
                "not_found",
                format!("the set has no conversations of {cwd}"),
            )
        }))
        .collect();
    g.tally();
    Ok(g)
}

/// An assign between accounts that share a directory.
fn plan_assign(
    state: &AppState,
    req: &MoveRequest,
    a: &str,
    b: &str,
    home: &ClaudeHome,
    items: &ItemsBody,
    title: impl Fn(&str) -> Option<String>,
) -> Result<Group, TransferError> {
    let mut g = Group::new(
        Work::Assign {
            from: a.to_string(),
            to: b.to_string(),
            sessions: Vec::new(),
        },
        req.mode,
        req.trust,
        Location::Account(a.to_string()),
        Location::Account(b.to_string()),
    );
    let (mut ids, skipped) = sessions_of_projects(state, a, home, &items.projects);
    g.skipped = skipped;
    if items.all {
        ids.extend(
            backup::session_ids(home)
                .into_iter()
                .filter(|id| belongs(state, a, id).is_ok()),
        );
    }
    ids.extend(items.sessions.iter().cloned());
    let mut assigned = Vec::new();
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(id.clone()) {
            continue;
        }
        if !crate::claude::ids::is_session_id(&id) {
            g.skipped.push(Skipped::session(
                &id,
                "invalid_transfer",
                format!("{id:?} is not a session id"),
            ));
            continue;
        }
        if super::transcripts_of(home, &id).is_empty() {
            g.skipped.push(Skipped::session(
                &id,
                "not_found",
                "it is not in this account's directory",
            ));
            continue;
        }
        let acting = acting_account(state, &id);
        if acting == b {
            g.skipped.push(Skipped::session(
                &id,
                "already_exists",
                "it is the target account's already",
            ));
            continue;
        }
        if let Err((code, message)) = belongs(state, a, &id) {
            g.skipped.push(Skipped::session(&id, &code, message));
            continue;
        }
        if super::ops::is_live(state, &id) {
            g.skipped
                .push(Skipped::of(&id, &TransferError::SessionLive(id.clone())));
            continue;
        }
        let cwd = super::transcripts_of(home, &id)
            .first()
            .and_then(|t| super::recorded_cwd(t));
        g.sessions.push(PlanSession {
            id: id.clone(),
            cwd,
            title: title(&id),
            files: 0,
            bytes: 0,
            from: Location::Account(a.to_string()),
            action: Action::Assign,
        });
        assigned.push((id.clone(), session_account(state, &id)));
    }
    g.work = Work::Assign {
        from: a.to_string(),
        to: b.to_string(),
        sessions: assigned,
    };
    Ok(g)
}

/// A move or copy between two directories.
#[allow(clippy::too_many_arguments)]
async fn plan_transfer(
    state: &AppState,
    req: &MoveRequest,
    a: &str,
    b: &str,
    ha: ClaudeHome,
    hb: ClaudeHome,
    items: &ItemsBody,
    title: impl Fn(&str) -> Option<String>,
) -> Result<Group, TransferError> {
    let st = state.clone();
    let (a2, req2, items2) = (a.to_string(), req.clone(), items.clone());
    let (work_items, plan, skipped) = tokio::task::spawn_blocking(move || {
        let mut projects = items2.projects.clone();
        let mut sessions = items2.sessions.clone();
        if items2.all {
            let (p, loose) = expand_all(&st, &a2, &ha);
            projects.extend(p);
            sessions.extend(loose);
        }
        let engine = super::Items {
            sessions,
            projects,
            mode: req2.mode,
            trust: req2.trust,
            customs: items2.customizations.clone(),
            replace_existing: req2.replace_existing,
        };
        let live = |id: &str| super::ops::is_live(&st, id);
        let ctx = super::Context {
            from: &ha,
            to: &hb,
            is_live: &live,
        };
        let owns = |id: &str| belongs(&st, &a2, id);
        let (plan, skipped) = super::plan_items(&ctx, &engine, &owns)?;
        Ok::<_, TransferError>((engine, plan, skipped))
    })
    .await
    .map_err(|e| TransferError::Io(e.to_string()))??;
    let mut g = Group::new(
        Work::Transfer {
            from: a.to_string(),
            to: b.to_string(),
            items: work_items,
        },
        req.mode,
        req.trust,
        Location::Account(a.to_string()),
        Location::Account(b.to_string()),
    );
    g.sessions = plan
        .sessions
        .iter()
        .map(|s| PlanSession {
            id: s.id.clone(),
            cwd: s.cwd.clone(),
            title: title(&s.id),
            files: s.files.len(),
            bytes: s.files.iter().map(|f| f.size).sum(),
            from: Location::Account(a.to_string()),
            action: match req.mode {
                Mode::Move => Action::Move,
                Mode::Copy => Action::Copy,
            },
        })
        .collect();
    g.skipped = skipped;
    g.projects = plan.projects.clone();
    g.adds = plan.added_entries();
    g.removes = plan.removed_entries().to_vec();
    g.history_lines = plan.history_lines;
    g.customizations = plan
        .customs
        .as_ref()
        .map(|c| c.items.clone())
        .unwrap_or_default();
    g.warnings = plan.warnings.clone();
    g.writes_shared = plan.writes_shared;
    g.files = plan.files;
    g.bytes = plan.bytes;
    Ok(g)
}

/// A copy (a push of some) or a move (archive) into a set.
#[allow(clippy::too_many_arguments)]
async fn plan_into(
    state: &AppState,
    req: &MoveRequest,
    a: &str,
    home: &ClaudeHome,
    storage: Storage,
    set_name: String,
    items: &ItemsBody,
    title: impl Fn(&str) -> Option<String>,
) -> Result<Group, TransferError> {
    let pass = backup::ops::passphrases(state).unwrap_or_default();
    // What this machine had of each (a dry run never makes an id).
    let machine = backup::ops::known_machine_id(state).unwrap_or_default();
    let st = state.clone();
    let (a2, items2, home2) = (a.to_string(), items.clone(), home.clone());
    let set = storage.set(&set_name);
    let (ids, planned, skipped) = tokio::task::spawn_blocking(move || {
        let (mut ids, mut skipped) = sessions_of_projects(&st, &a2, &home2, &items2.projects);
        if items2.all {
            ids.extend(
                backup::session_ids(&home2)
                    .into_iter()
                    .filter(|id| belongs(&st, &a2, id).is_ok()),
            );
        }
        for id in &items2.sessions {
            match belongs(&st, &a2, id) {
                Ok(()) => ids.push(id.clone()),
                Err((code, message)) => skipped.push(Skipped::session(id, &code, message)),
            }
        }
        let live = |id: &str| super::ops::is_live(&st, id);
        let (planned, skips) = backup::plan_push_some(&set, &pass, &home2, &ids, &machine, &live)?;
        skipped.extend(skips.into_iter().map(|s| skipped_of(&s)));
        Ok::<_, BackupError>((ids, planned, skipped))
    })
    .await
    .map_err(|e| TransferError::Io(e.to_string()))?
    .map_err(TransferError::from)?;
    let to = Location::Storage {
        storage: storage.id.clone(),
        set: Some(set_name.clone()),
    };
    let mut g = Group::new(
        Work::Into {
            from: a.to_string(),
            storage,
            set: set_name,
            ids: Vec::new(),
        },
        req.mode,
        req.trust,
        Location::Account(a.to_string()),
        to,
    );
    let going: HashSet<&str> = planned.iter().map(|p| p.id.as_str()).collect();
    if let Work::Into { ids: work_ids, .. } = &mut g.work {
        let mut seen = HashSet::new();
        *work_ids = ids
            .into_iter()
            .filter(|id| going.contains(id.as_str()) && seen.insert(id.clone()))
            .collect();
    }
    g.sessions = planned
        .into_iter()
        .map(|p| PlanSession {
            title: title(&p.id),
            id: p.id,
            cwd: p.cwd,
            files: p.files,
            bytes: p.bytes,
            from: Location::Account(a.to_string()),
            action: match req.mode {
                Mode::Move => Action::Archive,
                Mode::Copy => Action::Copy,
            },
        })
        .collect();
    g.skipped = skipped;
    g.tally();
    Ok(g)
}

/// A restore out of a set (copy), then for a move its entries out of it.
#[allow(clippy::too_many_arguments)]
async fn plan_out_of(
    state: &AppState,
    req: &MoveRequest,
    storage: Storage,
    set_name: String,
    set: Set,
    b: &str,
    home: &ClaudeHome,
    items: &ItemsBody,
) -> Result<Group, TransferError> {
    let pass =
        backup::ops::passphrases(state).ok_or(TransferError::from(BackupError::NoPassphrase))?;
    let st = state.clone();
    let (items2, home2, set2) = (items.clone(), home.clone(), set.clone());
    let (entries, skips, empty) = tokio::task::spawn_blocking(move || {
        let live = |id: &str| super::ops::is_live(&st, id);
        backup::plan_restore(
            &set2,
            &pass,
            &home2,
            (!items2.all).then_some(items2.sessions.as_slice()),
            &items2.projects,
            &live,
        )
    })
    .await
    .map_err(|e| TransferError::Io(e.to_string()))?
    .map_err(TransferError::from)?;
    let from = Location::Storage {
        storage: storage.id.clone(),
        set: Some(set_name.clone()),
    };
    let mut g = Group::new(
        Work::OutOf {
            storage,
            set: set_name,
            to: b.to_string(),
            ids: entries.iter().map(|e| e.id.clone()).collect(),
        },
        req.mode,
        req.trust,
        from.clone(),
        Location::Account(b.to_string()),
    );
    g.sessions = entries
        .iter()
        .map(|e| PlanSession {
            id: e.id.clone(),
            cwd: e.cwd.clone(),
            title: e.title.clone(),
            files: e.files as usize,
            bytes: e.archive_bytes,
            from: from.clone(),
            action: Action::Restore,
        })
        .collect();
    g.skipped = skips
        .into_iter()
        .map(|s| skipped_of(&s))
        .chain(empty.iter().map(|cwd| {
            Skipped::project(
                cwd,
                "not_found",
                format!("the set has no conversations of {cwd}"),
            )
        }))
        .collect();
    if req.trust {
        g.adds = entries
            .iter()
            .filter_map(|e| e.cwd.clone())
            .filter(|c| super::project_entry(&home.claude_json(), c).is_none())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        g.writes_shared = !g.adds.is_empty();
    }
    let days = backup::cleanup_days(home);
    let now = chrono::Utc::now().timestamp_millis();
    g.warnings = entries
        .iter()
        .filter_map(|e| {
            backup::prune_warning(e.newest_mtime, days, now)
                .map(|w| format!("{}: {w}", e.title.clone().unwrap_or_else(|| e.id.clone())))
        })
        .collect();
    g.tally();
    Ok(g)
}

// ─────────────────────────────── running it ───────────────────────────────

/// The job view a group starts with.
fn start_view(state: &AppState, job: &str, g: &Group) {
    let kind = match g.mode {
        Mode::Move => "move",
        Mode::Copy => "copy",
    };
    drop(state.jobs.start(job, kind, g.files as u64, g.bytes));
    let sessions: Vec<String> = g.sessions.iter().map(|s| s.id.clone()).collect();
    state.jobs.update(job, |v| {
        v.sessions = sessions;
        match &g.from {
            Location::Account(a) => v.from = Some(a.clone()).filter(|a| !a.is_empty()),
            Location::Storage { storage, set } => {
                v.from_storage = Some(storage.clone());
                v.set = set.clone();
            }
        }
        match &g.to {
            Location::Account(b) => v.to = Some(b.clone()),
            Location::Storage { storage, set } => {
                v.to_storage = Some(storage.clone());
                // Storage to storage: `set` is the source's.
                if v.from_storage.is_some() {
                    v.to_set = set.clone();
                } else {
                    v.set = set.clone();
                }
            }
        }
    });
}

/// Start the plan's jobs (one per source; one that does nothing when every
/// conversation was skipped) and run them one after another in the
/// background. Returns their views as they start.
pub fn start(state: &AppState, plan: MovePlan) -> Vec<JobView> {
    let mut groups = plan.groups;
    if let Some(first) = groups.first_mut() {
        // What the request skipped before any source was known goes with
        // the first job.
        let mut skipped = plan.unsourced;
        skipped.append(&mut first.skipped);
        first.skipped = skipped;
    } else {
        let mut g = Group::new(
            Work::Nothing,
            plan.mode,
            false,
            plan.from
                .clone()
                .unwrap_or(Location::Account(String::new())),
            plan.to.clone(),
        );
        g.skipped = plan.unsourced;
        groups.push(g);
    }
    let ids: Vec<String> = groups.iter().map(|_| super::new_job_id()).collect();
    for (g, id) in groups.iter().zip(&ids) {
        start_view(state, id, g);
    }
    let views = ids.iter().filter_map(|id| state.jobs.get(id)).collect();
    let st = state.clone();
    tokio::spawn(async move {
        for (g, id) in groups.into_iter().zip(ids) {
            let result = run_group(&st, &id, g).await;
            if let Err(e) = &result {
                tracing::warn!(job = %id, error = %e, "moving data failed");
            }
            super::ops::rediscover(&st).await;
        }
    });
    views
}

/// The `skipped` of a job's result (at most [`LIST_CAP`]).
fn skipped_json(s: &[Skipped]) -> Value {
    serde_json::to_value(&s[..s.len().min(LIST_CAP)]).unwrap_or(Value::Null)
}

/// Finish a job's view.
fn finish(state: &AppState, job: &str, result: Result<(Value, Vec<String>, bool), TransferError>) {
    let at = chrono::Utc::now().timestamp_millis();
    state.jobs.update(job, |v| {
        v.finished_at = Some(at);
        match result {
            Ok((r, sessions, undo)) => {
                v.state = JobState::Done;
                v.result = Some(r);
                v.sessions = sessions;
                v.files_done = v.files_total;
                v.bytes_done = v.bytes_total;
                v.undo_until = undo.then(|| at + super::UNDO_KEEP.as_millis() as i64);
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

/// Run one job, with the data jobs' lock (it may wait behind a backup), its
/// plan checked again first.
async fn run_group(state: &AppState, job: &str, g: Group) -> Result<(), TransferError> {
    let one = state.jobs.lock.lock().await;
    let root =
        super::ops::undo_root(state).ok_or_else(|| invalid("this build has no data directory"))?;
    let result = match g.work.clone() {
        Work::Transfer { from, to, items } => {
            run_transfer(state, job, &root, &g, &from, &to, items).await
        }
        Work::Assign { from, to, sessions } => {
            run_assign(state, job, &root, &g, &from, &to, sessions)
        }
        Work::Into {
            from,
            storage,
            set,
            ids,
        } => run_into(state, job, &root, &g, &from, storage, set, ids).await,
        Work::OutOf {
            storage,
            set,
            to,
            ids,
        } => run_out_of(state, job, &root, &g, storage, set, &to, ids).await,
        Work::Between {
            from,
            from_set,
            to,
            to_set,
            ids,
        } => run_between(state, job, &root, &g, (from, from_set), (to, to_set), ids).await,
        Work::Nothing => {
            let key = match (&g.to, g.mode) {
                (Location::Storage { .. }, Mode::Move) => "archived",
                (_, Mode::Move) => "moved",
                (_, Mode::Copy) => "copied",
            };
            Ok((
                json!({ key: Vec::<String>::new(), "skipped": skipped_json(&g.skipped) }),
                Vec::new(),
                false,
            ))
        }
    };
    drop(one);
    let failed = result.as_ref().err().cloned();
    finish(state, job, result);
    match failed {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

async fn run_transfer(
    state: &AppState,
    job: &str,
    root: &std::path::Path,
    g: &Group,
    from: &str,
    to: &str,
    items: super::Items,
) -> Result<(Value, Vec<String>, bool), TransferError> {
    let (ha, hb) = (home_of(state, from)?, home_of(state, to)?);
    let st = state.clone();
    let (job2, root2, from2, to2) = (
        job.to_string(),
        root.to_path_buf(),
        from.to_string(),
        to.to_string(),
    );
    let p2 = state.jobs.progress(job).unwrap_or_default();
    let (plan, skipped, manifest) = tokio::task::spawn_blocking(move || {
        super::prune_undo(&root2);
        let live = |id: &str| super::ops::is_live(&st, id);
        let ctx = super::Context {
            from: &ha,
            to: &hb,
            is_live: &live,
        };
        let owns = |id: &str| belongs(&st, &from2, id);
        // Planned again: what changed while it waited is skipped now.
        let (mut plan, skipped) = super::plan_items(&ctx, &items, &owns)?;
        plan.from_account = Some(from2.clone());
        plan.to_account = Some(to2.clone());
        if plan.is_empty() {
            return Ok((plan, skipped, None));
        }
        let m = super::run(&plan, &root2, &job2, &p2, &live)?;
        Ok::<_, TransferError>((plan, skipped, Some(m)))
    })
    .await
    .map_err(|e| TransferError::Io(e.to_string()))??;
    let ids: Vec<String> = plan.sessions.iter().map(|s| s.id.clone()).collect();
    // Into a token account's shared folder: recorded as that account's.
    if state
        .accounts
        .get(to)
        .is_some_and(|a| a.kind == AccountKind::Token)
    {
        for id in &ids {
            state.accounts.remember_session(id, to);
        }
    }
    let key = match g.mode {
        Mode::Move => "moved",
        Mode::Copy => "copied",
    };
    let mut result = json!({ key: ids, "skipped": skipped_json(&skipped) });
    if let Some(cp) = &plan.customs {
        result["customizations"] = serde_json::to_value(&cp.items).unwrap_or(Value::Null);
    }
    if !plan.warnings.is_empty() {
        result["warnings"] = json!(plan.warnings);
    }
    Ok((result, ids, manifest.is_some()))
}

fn run_assign(
    state: &AppState,
    job: &str,
    root: &std::path::Path,
    g: &Group,
    from: &str,
    to: &str,
    sessions: Vec<(String, String)>,
) -> Result<(Value, Vec<String>, bool), TransferError> {
    let mut skipped = g.skipped.clone();
    let mut done: Vec<(String, String)> = Vec::new();
    for (id, previous) in sessions {
        // Checked again: it may have started, or been given to another
        // account, while the job waited.
        if super::ops::is_live(state, &id) {
            skipped.push(Skipped::of(&id, &TransferError::SessionLive(id.clone())));
            continue;
        }
        if acting_account(state, &id) != from && session_account(state, &id) != previous {
            skipped.push(Skipped::session(
                &id,
                "session_changed",
                "it was given to another account meanwhile",
            ));
            continue;
        }
        state.accounts.remember_session(&id, to);
        done.push((id, previous));
    }
    let ids: Vec<String> = done.iter().map(|(id, _)| id.clone()).collect();
    if !done.is_empty() {
        let dir = state.accounts.default_home().base().to_path_buf();
        let m = super::Manifest {
            sessions: ids.clone(),
            from_account: Some(from.to_string()),
            to_account: Some(to.to_string()),
            assigned: done,
            ..super::Manifest::new(
                job,
                Mode::Move,
                super::What::Session { id: ids[0].clone() },
                dir.clone(),
                dir,
            )
        };
        let undo_dir = root.join(job);
        std::fs::create_dir_all(&undo_dir)
            .map_err(|e| TransferError::Io(format!("creating {}: {e}", undo_dir.display())))?;
        super::private_dir(root);
        super::private_dir(&undo_dir);
        crate::persist::write_json_private(&undo_dir.join("manifest.json"), &m)
            .map_err(|e| TransferError::Io(format!("writing {}: {e}", undo_dir.display())))?;
    }
    Ok((
        json!({ "assigned": ids, "skipped": skipped_json(&skipped) }),
        ids.clone(),
        !ids.is_empty(),
    ))
}

#[allow(clippy::too_many_arguments)]
async fn run_into(
    state: &AppState,
    job: &str,
    root: &std::path::Path,
    g: &Group,
    from: &str,
    storage: Storage,
    set_name: String,
    ids: Vec<String>,
) -> Result<(Value, Vec<String>, bool), TransferError> {
    let home = home_of(state, from)?;
    storage.require_available().map_err(TransferError::from)?;
    let pass =
        backup::ops::passphrases(state).ok_or(TransferError::from(BackupError::NoPassphrase))?;
    let machine = backup::ops::this_machine(state).map_err(TransferError::from)?;
    let titles = backup::ops::titles(state).await;
    let st = state.clone();
    let (job2, root2, from2) = (job.to_string(), root.to_path_buf(), from.to_string());
    let mode = g.mode;
    let set = storage.set(&set_name);
    let storage_ref = super::StorageRef {
        storage: storage.id.clone(),
        set: set_name.clone(),
        dir: set.dir.clone(),
        into: true,
    };
    let progress = state.jobs.progress(job).unwrap_or_default();
    let (report, manifest) = tokio::task::spawn_blocking(move || {
        let live = |id: &str| super::ops::is_live(&st, id);
        let title_of = |id: &str| titles.get(id).cloned();
        let push = backup::PushInputs {
            home: &home,
            set: &set,
            account_name: &set_name,
            passphrases: &pass,
            machine: &machine,
            is_live: &live,
            title_of: &title_of,
            progress: &progress,
        };
        match mode {
            Mode::Copy => backup::push_some(&push, &ids, false).map(|r| (r, None)),
            Mode::Move => backup::move_into_set(&backup::MoveIntoInputs {
                push,
                ids: &ids,
                undo_root: &root2,
                job_id: &job2,
                storage: storage_ref,
                from_account: Some(from2),
            }),
        }
    })
    .await
    .map_err(|e| TransferError::Io(e.to_string()))?
    .map_err(TransferError::from)?;
    let mut skipped = g.skipped.clone();
    skipped.extend(report.skipped.iter().map(skipped_of));
    let (key, ids) = match g.mode {
        Mode::Move => ("archived", report.archived.clone()),
        Mode::Copy => ("copied", report.copied.clone()),
    };
    let mut result = json!({ key: ids, "skipped": skipped_json(&skipped) });
    if !report.warnings.is_empty() {
        result["warnings"] = json!(report.warnings);
    }
    Ok((result, ids, manifest.is_some()))
}

#[allow(clippy::too_many_arguments)]
async fn run_out_of(
    state: &AppState,
    job: &str,
    root: &std::path::Path,
    g: &Group,
    storage: Storage,
    set_name: String,
    to: &str,
    ids: Vec<String>,
) -> Result<(Value, Vec<String>, bool), TransferError> {
    let home = home_of(state, to)?;
    storage.require_available().map_err(TransferError::from)?;
    let pass =
        backup::ops::passphrases(state).ok_or(TransferError::from(BackupError::NoPassphrase))?;
    let st = state.clone();
    let (job2, root2, to2) = (job.to_string(), root.to_path_buf(), to.to_string());
    let (mode, trust) = (g.mode, g.trust);
    let set = storage.set(&set_name);
    let storage_ref = super::StorageRef {
        storage: storage.id.clone(),
        set: set_name.clone(),
        dir: set.dir.clone(),
        into: false,
    };
    let progress = state.jobs.progress(job).unwrap_or_default();
    let (report, manifest) = tokio::task::spawn_blocking(move || {
        let live = |id: &str| super::ops::is_live(&st, id);
        let kind = match mode {
            Mode::Move => "move",
            Mode::Copy => "copy",
        };
        let keep_until =
            chrono::Utc::now().timestamp_millis() + super::UNDO_KEEP.as_millis() as i64;
        backup::move_out_of_set(
            &backup::RestoreInputs {
                set: &set,
                passphrases: &pass,
                target: &home,
                ids: &ids,
                trust,
                is_live: &live,
                undo_root: &root2,
                job_id: &job2,
                progress: &progress,
                kind,
            },
            mode == Mode::Move,
            keep_until,
            storage_ref,
            Some(to2),
        )
    })
    .await
    .map_err(|e| TransferError::Io(e.to_string()))?
    .map_err(TransferError::from)?;
    // Nothing came: nothing to undo.
    if manifest.is_empty() {
        let _ = std::fs::remove_dir_all(root.join(job));
    }
    let mut skipped = g.skipped.clone();
    skipped.extend(report.skipped.iter().map(skipped_of));
    let warnings: Vec<String> = report
        .warnings
        .iter()
        .map(|w| {
            if w.id.is_empty() {
                w.text.clone()
            } else {
                format!("{}: {}", w.id, w.text)
            }
        })
        .collect();
    let mut result = json!({ "restored": report.restored, "skipped": skipped_json(&skipped) });
    if !warnings.is_empty() {
        result["warnings"] = json!(warnings);
    }
    Ok((result, report.restored.clone(), !manifest.is_empty()))
}

/// Run a copy or move between two sets.
async fn run_between(
    state: &AppState,
    job: &str,
    root: &std::path::Path,
    g: &Group,
    from: (Storage, String),
    to: (Storage, String),
    ids: Vec<String>,
) -> Result<(Value, Vec<String>, bool), TransferError> {
    from.0.require_available().map_err(TransferError::from)?;
    to.0.require_available().map_err(TransferError::from)?;
    let pass =
        backup::ops::passphrases(state).ok_or(TransferError::from(BackupError::NoPassphrase))?;
    let progress = state.jobs.progress(job).unwrap_or_default();
    let (job2, root2) = (job.to_string(), root.to_path_buf());
    let remove = g.mode == Mode::Move;
    let (report, manifest) = tokio::task::spawn_blocking(move || {
        let (from_set, to_set) = (from.0.set(&from.1), to.0.set(&to.1));
        let keep_until =
            chrono::Utc::now().timestamp_millis() + super::UNDO_KEEP.as_millis() as i64;
        backup::between_sets(&backup::BetweenInputs {
            from: &from_set,
            to: &to_set,
            to_name: &to.1,
            passphrases: &pass,
            ids: &ids,
            remove,
            keep_until,
            undo_root: &root2,
            job_id: &job2,
            from_ref: super::StorageRef {
                storage: from.0.id.clone(),
                set: from.1.clone(),
                dir: from_set.dir.clone(),
                into: false,
            },
            to_ref: super::StorageRef {
                storage: to.0.id.clone(),
                set: to.1.clone(),
                dir: to_set.dir.clone(),
                into: true,
            },
            progress: &progress,
        })
    })
    .await
    .map_err(|e| TransferError::Io(e.to_string()))?
    .map_err(TransferError::from)?;
    let mut skipped = g.skipped.clone();
    skipped.extend(report.skipped.iter().map(skipped_of));
    let key = match g.mode {
        Mode::Move => "moved",
        Mode::Copy => "copied",
    };
    let mut result = json!({ key: report.copied, "skipped": skipped_json(&skipped) });
    if !report.warnings.is_empty() {
        result["warnings"] = json!(report.warnings);
    }
    Ok((result, report.copied.clone(), manifest.is_some()))
}

/// Undo a copy or move between two sets (see [`backup::undo_between`]).
pub fn undo_between_sets(
    root: &std::path::Path,
    job_id: &str,
    passphrases: Option<&[crate::claude::secrets::Secret]>,
) -> Result<super::Manifest, TransferError> {
    let mut m = super::manifest(root, job_id)
        .ok_or_else(|| TransferError::NotFound("nothing to undo for that transfer".into()))?;
    if m.undone_at.is_some() {
        return Err(TransferError::UndoConflict(
            "this transfer was undone already".into(),
        ));
    }
    let pass = passphrases.ok_or(TransferError::from(BackupError::NoPassphrase))?;
    backup::undo_between(&m, pass).map_err(TransferError::from)?;
    m.undone_at = Some(chrono::Utc::now().timestamp_millis());
    crate::persist::write_json_private(&root.join(job_id).join("manifest.json"), &m)
        .map_err(|e| TransferError::Io(e.to_string()))?;
    Ok(m)
}

/// Undo an assign: each conversation back to the account it was, while it is
/// still the one this job gave it to and does not run.
pub fn undo_assign(
    state: &AppState,
    root: &std::path::Path,
    job_id: &str,
) -> Result<super::Manifest, TransferError> {
    let mut m = super::manifest(root, job_id)
        .ok_or_else(|| TransferError::NotFound("nothing to undo for that transfer".into()))?;
    if m.undone_at.is_some() {
        return Err(TransferError::UndoConflict(
            "this transfer was undone already".into(),
        ));
    }
    if let Some((id, _)) = m
        .assigned
        .iter()
        .find(|(id, _)| super::ops::is_live(state, id))
    {
        return Err(TransferError::SessionLive(id.clone()));
    }
    let to = m.to_account.clone().unwrap_or_default();
    for (id, previous) in &m.assigned {
        if acting_account(state, id) == to {
            state.accounts.remember_session(id, previous);
        }
    }
    m.undone_at = Some(chrono::Utc::now().timestamp_millis());
    crate::persist::write_json_private(&root.join(job_id).join("manifest.json"), &m)
        .map_err(|e| TransferError::Io(e.to_string()))?;
    Ok(m)
}

/// What a request's audit line says (never a secret: ids, names, counts).
pub fn audit_detail(req: &MoveRequest, plan: &MovePlan, jobs: &[JobView]) -> Value {
    json!({
        "jobs": jobs.iter().map(|j| j.id.clone()).collect::<Vec<_>>(),
        "from": req.from, "to": req.to, "mode": req.mode,
        "trust": req.trust, "replaceExisting": req.replace_existing,
        "sessions": plan.sessions.len(), "skipped": plan.skipped.len(),
        "projects": plan.projects.len(),
        "customizations": req.items.customizations.iter().map(|c| c.key()).collect::<Vec<_>>(),
        "files": plan.files, "bytes": plan.bytes,
    })
}

/// Whether a request needs the dangerous right (§13.9): a folder's trust,
/// the settings or MCP servers. `Some(what)` names it.
pub fn dangerous_part(req: &MoveRequest) -> Option<&'static str> {
    if req.trust {
        return Some("Carrying a folder's trust");
    }
    req.items
        .customizations
        .iter()
        .any(|c| c.dangerous())
        .then_some("Copying settings or MCP servers")
}
