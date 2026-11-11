//! Folders across computers (docs/PLAN-MACHINES.md §5.14 step 5): the git
//! repositories each computer knows — its projects, and the folders right
//! under its roots — with their `origin`, matched by origin:
//!
//! * the same origin under folders of the same name on both proposes a
//!   root: `~/development/projects/app` ↔ `D:\development\projects\app`
//!   proposes `~/development/projects` ↔ `D:\development\projects` (the
//!   user accepts each one, `PUT /api/roots`);
//! * a repository on one computer only offers *Clone it there* when a root
//!   maps its folder there (an asked action on the other computer, `asks`;
//!   on this one, `POST /api/machines/clone` from its desktop).
//!
//! Every git call goes through `code::GitRunner` (hooks never run) and never
//! prompts (`GIT_TERMINAL_PROMPT=0`, Git Credential Manager non-interactive,
//! no askpass). An origin is kept and shown without credentials.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::claude::paths;
use crate::machines::code::{CodeError, Git, GitRunner};
use crate::machines::roots::{self, Root};
use crate::machines::transport;
use crate::state::AppState;

/// Where another computer's repositories are read.
pub const PEER_PATH: &str = "/peer/origins";
/// A scan is reused this long (ms) unless a fresh one is asked for, and
/// this long when one is.
const REPOS_FRESH_MS: i64 = 60_000;
const FRESH_AGAIN_MS: i64 = 10_000;
/// Remote checks another computer may have running here at once.
const MAX_REMOTE_CHECKS: usize = 4;
/// Folders looked at under each root, and repositories kept, at most.
const MAX_PER_ROOT: usize = 300;
const MAX_REPOS: usize = 500;
/// How long a remote may take to answer.
pub const REMOTE_TIMEOUT: Duration = Duration::from_secs(30);

// ─── origins ───────────────────────────────────────────────────────────────────

/// `origin` as kept and shown: without credentials (`https://user:token@
/// host/x` → `https://host/x`; ssh keeps its user, `git@host:x`), and only
/// a remote git can reach — `https`, `http`, `ssh`, `git`, `file` or the
/// `user@host:path` form. `None` for anything else (an option, a local
/// path, `ext::…`).
pub fn sanitize_origin(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty()
        || s.len() > 2048
        || s.starts_with('-')
        || s.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return None;
    }
    if let Ok(mut u) = url::Url::parse(s) {
        match u.scheme() {
            "https" | "http" => {
                let _ = u.set_username("");
                let _ = u.set_password(None);
            }
            "ssh" | "git+ssh" | "ssh+git" | "git" => {
                let _ = u.set_password(None);
            }
            "file" => {}
            _ => return None,
        }
        if u.scheme() != "file" && u.host_str().is_none_or(str::is_empty) {
            return None;
        }
        return Some(u.to_string());
    }
    // `[user@]host:path`, as ssh remotes are often written (a one-letter
    // host is a Windows drive, which git reads as a local path).
    let (left, path) = s.split_once(':')?;
    let (user, host) = match left.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, left),
    };
    let plain = |t: &str| {
        t.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
    };
    let host_ok = host.len() > 1 && plain(host);
    let user_ok = user.is_none_or(|u| !u.is_empty() && plain(u));
    let path_ok = !path.is_empty() && !path.starts_with("//") && !path.contains(['\\', '@']);
    (host_ok && user_ok && path_ok).then(|| s.to_string())
}

/// What two origins are compared by: host and path, lower case, without
/// `.git`, a trailing `/` or the scheme — `git@github.com:You/App.git`,
/// `https://github.com/you/app` and `ssh://git@github.com/you/app.git` are
/// one repository.
pub fn origin_key(origin: &str) -> Option<String> {
    let s = sanitize_origin(origin)?;
    let (host, path) = match url::Url::parse(&s) {
        Ok(u) if u.scheme() == "file" => ("file".to_string(), u.path().to_string()),
        Ok(u) => (u.host_str()?.to_string(), u.path().to_string()),
        Err(_) => {
            let (left, path) = s.split_once(':')?;
            (left.rsplit('@').next()?.to_string(), path.to_string())
        }
    };
    let path = path.trim_matches('/');
    let path = path
        .strip_suffix(".git")
        .unwrap_or(path)
        .trim_end_matches('/');
    if path.is_empty() {
        return None;
    }
    Some(format!("{}/{}", host.to_lowercase(), path.to_lowercase()))
}

/// A repository's name for a list: its folder's.
pub fn name_of(path: &str) -> String {
    paths::last_component(path).unwrap_or(path).to_string()
}

// ─── this computer's repositories ──────────────────────────────────────────────

/// A git repository (a folder holding `.git`) and its origin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    pub path: String,
    #[serde(default)]
    pub origin: Option<String>,
}

/// A worktree Claude Code made (`.claude/worktrees/…`): the same
/// repository as its project, not another.
fn in_claude_worktree(p: &str) -> bool {
    let k = p.replace('\\', "/").to_lowercase();
    k.contains("/.claude/worktrees/")
}

/// The repositories among `projects` and the folders right under
/// `root_dirs`, each with its origin (blocking; git through `git`).
pub fn scan(git: &GitRunner, projects: &[String], root_dirs: &[String]) -> Vec<Repo> {
    let mut candidates: Vec<PathBuf> = projects.iter().map(PathBuf::from).collect();
    for r in root_dirs {
        if let Ok(entries) = std::fs::read_dir(r) {
            candidates.extend(
                entries
                    .flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                    .take(MAX_PER_ROOT)
                    .map(|e| e.path()),
            );
        }
    }
    let Ok(op) = git.begin() else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for c in candidates {
        if out.len() >= MAX_REPOS {
            break;
        }
        let s = c.to_string_lossy().into_owned();
        if in_claude_worktree(&s) || !seen.insert(paths::path_key(&s)) || !c.join(".git").exists() {
            continue;
        }
        let origin = op
            .cmd(&c)
            .args(["config", "--get", "remote.origin.url"])
            .output()
            .ok()
            .filter(|o| o.success)
            .and_then(|o| sanitize_origin(String::from_utf8_lossy(&o.stdout).trim()));
        out.push(Repo { path: s, origin });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// This computer's repositories: its projects (`GET /api/projects`) and the
/// folders right under its roots. A scan of the same roots is reused for a
/// minute, and for 10 s even when a fresh one is asked for (another
/// computer asking again and again runs git once).
pub async fn local_repos(state: &AppState, fresh: bool) -> Vec<Repo> {
    let now = crate::machines::now();
    let here = roots::here_id(state);
    let root_dirs: Vec<String> = roots::roots(state)
        .iter()
        .filter_map(|r| r.path_of(here.as_deref()?).map(str::to_string))
        .collect();
    let reuse_for = if fresh {
        FRESH_AGAIN_MS
    } else {
        REPOS_FRESH_MS
    };
    let cached = super::with_cache(state, |c| c.repos.clone());
    if let Some((_, _, list)) =
        cached.filter(|(at, dirs, _)| now - at < reuse_for && *dirs == root_dirs)
    {
        return list;
    }
    let projects: Vec<String> = crate::server::routes::data::project_list(state)
        .await
        .into_iter()
        .map(|p| p.path)
        .collect();
    let dirs = root_dirs.clone();
    let list = tokio::task::spawn_blocking(move || scan(&GitRunner::new(), &projects, &dirs))
        .await
        .unwrap_or_default();
    super::with_cache(state, |c| c.repos = Some((now, root_dirs, list.clone())));
    list
}

/// A remote check another computer asked for (`/peer/setup/remote`): at
/// most [`MAX_REMOTE_CHECKS`] at once, each released when dropped.
pub struct RemoteSlot(AppState);

pub fn remote_slot(state: &AppState) -> Option<RemoteSlot> {
    super::with_cache(state, |c| {
        (c.remote_checks < MAX_REMOTE_CHECKS).then(|| c.remote_checks += 1)
    })?;
    Some(RemoteSlot(state.clone()))
}

impl Drop for RemoteSlot {
    fn drop(&mut self) {
        super::with_cache(&self.0, |c| {
            c.remote_checks = c.remote_checks.saturating_sub(1)
        });
    }
}

/// `GET /peer/origins`: this computer's repositories with their origins,
/// and whether each root's folder is here (the readiness test's folders).
pub async fn peer_answer(state: &AppState, fresh: bool) -> Value {
    let repos = local_repos(state, fresh).await;
    let here = roots::here_id(state);
    let roots: Vec<Value> = roots::roots(state)
        .iter()
        .filter_map(|r| {
            let p = r.path_of(here.as_deref()?)?;
            Some(json!({ "id": r.id, "exists": Path::new(p).is_dir() }))
        })
        .collect();
    json!({ "repos": repos, "roots": roots })
}

/// Another computer's answer to `GET /peer/origins`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Origins {
    #[serde(default)]
    pub repos: Vec<Repo>,
    #[serde(default)]
    pub roots: Vec<RootHere>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RootHere {
    pub id: String,
    #[serde(default)]
    pub exists: bool,
}

/// Another computer's repositories: `Err("older")` from a build without
/// them, else why not.
pub async fn fetch(state: &AppState, machine: &str, fresh: bool) -> Result<Origins, String> {
    let path = if fresh {
        format!("{PEER_PATH}?fresh=true")
    } else {
        PEER_PATH.to_string()
    };
    let (status, v) =
        transport::request(state, machine, "GET", &path, None, Duration::from_secs(60)).await?;
    match status {
        200 => {
            let mut o: Origins = serde_json::from_value(v)
                .map_err(|_| "it answered something unexpected".to_string())?;
            // From another computer: only what is well formed.
            o.repos.retain(|r| roots::well_formed_path(&r.path));
            for r in &mut o.repos {
                r.origin = r.origin.as_deref().and_then(sanitize_origin);
            }
            Ok(o)
        }
        404 => Err("older".into()),
        403 => Err("refused".into()),
        s => Err(format!("it answered {s}")),
    }
}

// ─── matching ──────────────────────────────────────────────────────────────────

/// A repository on both computers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pair {
    pub name: String,
    pub origin: String,
    pub here: String,
    pub there: String,
}

/// A root to add (or a path to add to an existing root), from the
/// repositories it would map.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    /// `pr_` and 12 hex characters, from the two folders.
    pub id: String,
    pub name: String,
    pub here: String,
    pub there: String,
    /// The root that names `here` on this computer and nothing on the other
    /// yet: accepting adds the other's folder to it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    pub repos: Vec<Pair>,
}

/// A repository on one computer only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lone {
    pub name: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// The root holding it that names the other computer too, its path in
    /// it, and the folder that maps it there: what a clone there needs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mapped: Option<String>,
}

/// Two computers' repositories, matched.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub proposals: Vec<Proposal>,
    /// On both, and the roots (or folder pairs) map one to the other.
    pub matched: Vec<Pair>,
    /// On both, but the roots map this computer's elsewhere there.
    pub mismatched: Vec<Pair>,
    pub only_here: Vec<Lone>,
    pub only_there: Vec<Lone>,
}

/// A recorded path's parent folder; `None` at a root of the file system
/// (too broad to propose) or for a path without one.
pub fn parent_of(path: &str) -> Option<String> {
    let t = paths::trim_end_separators(path);
    let seps: &[char] = if paths::is_windows_absolute(t) {
        &['/', '\\']
    } else {
        &['/']
    };
    let i = t.rfind(seps)?;
    let parent = &t[..i];
    let parent = if parent.is_empty() || (parent.len() == 2 && parent.ends_with(':')) {
        &t[..=i]
    } else {
        parent
    };
    (paths::depth(parent) > 0 && paths::is_absolute_anywhere(parent)).then(|| parent.to_string())
}

/// `path` inside `base`, `/`-separated (`None`: not inside, or `base`
/// itself).
pub fn rel_inside(path: &str, base: &str) -> Option<String> {
    let r = paths::rebase(path, base, "/")?;
    let r = r.trim_start_matches('/');
    (!r.is_empty()).then(|| r.to_string())
}

/// The deepest live root that holds `path` on machine `from` and names
/// machine `to` too: its id, `path` inside it, and the same folder on `to`.
pub fn root_for(
    roots: &[Root],
    from: &str,
    to: &str,
    path: &str,
) -> Option<(String, String, String)> {
    roots
        .iter()
        .filter(|r| r.live())
        .filter_map(|r| {
            let f = r.path_of(from)?;
            let t = r.path_of(to)?;
            let mapped = paths::rebase(path, f, t)?;
            Some((r, f, mapped))
        })
        .max_by_key(|(_, f, _)| paths::depth(f))
        .and_then(|(r, f, mapped)| Some((r.id.clone(), rel_inside(path, f)?, mapped)))
}

fn lone(repo: &Repo, found: Option<(String, String, String)>) -> Lone {
    let (root, rel, mapped) = match found {
        Some((r, rel, m)) => (Some(r), Some(rel), Some(m)),
        None => (None, None, None),
    };
    Lone {
        name: name_of(&repo.path),
        path: repo.path.clone(),
        origin: repo.origin.clone(),
        root,
        rel,
        mapped,
    }
}

fn proposal_id(here: &str, there: &str) -> String {
    let h = crate::claude::secrets::sha256_hex(
        format!("{}\n{}", paths::path_key(here), paths::path_key(there)).as_bytes(),
    );
    format!("pr_{}", &h[..12])
}

/// Matches this computer's repositories (`mine`, machine `here`) with the
/// other's (`theirs`, machine `there`) by origin. `to_there` maps a path of
/// this computer to the other by the roots and folder pairs.
pub fn plan(
    here: &str,
    there: &str,
    roots: &[Root],
    mine: &[Repo],
    theirs: &[Repo],
    to_there: &dyn Fn(&str) -> Option<String>,
) -> Plan {
    let mut out = Plan::default();
    let mut by_key: BTreeMap<String, Vec<&Repo>> = BTreeMap::new();
    for t in theirs {
        if let Some(k) = t.origin.as_deref().and_then(origin_key) {
            by_key.entry(k).or_default().push(t);
        }
    }
    let mut mine_keys = HashSet::new();
    let mut groups: Vec<Proposal> = Vec::new();
    for m in mine {
        let Some(origin) = m.origin.as_deref() else {
            continue;
        };
        let Some(key) = origin_key(origin) else {
            continue;
        };
        mine_keys.insert(key.clone());
        let Some(ts) = by_key.get(&key) else {
            out.only_here
                .push(lone(m, root_for(roots, here, there, &m.path)));
            continue;
        };
        let pair = |t: &Repo| Pair {
            name: name_of(&m.path),
            origin: origin.to_string(),
            here: m.path.clone(),
            there: t.path.clone(),
        };
        let mapped = to_there(&m.path);
        if let Some(t) = ts.iter().copied().find(|t| {
            mapped
                .as_deref()
                .is_some_and(|p| paths::same_path(p, &t.path))
        }) {
            out.matched.push(pair(t));
            continue;
        }
        if mapped.is_some() {
            out.mismatched.push(pair(ts[0]));
            continue;
        }
        for t in ts.iter().copied() {
            let same_name = name_of(&m.path).to_lowercase() == name_of(&t.path).to_lowercase();
            let (h, th) = match (same_name, parent_of(&m.path), parent_of(&t.path)) {
                (true, Some(h), Some(th)) => (h, th),
                _ => (m.path.clone(), t.path.clone()),
            };
            match groups
                .iter_mut()
                .find(|g| paths::same_path(&g.here, &h) && paths::same_path(&g.there, &th))
            {
                Some(g) => g.repos.push(pair(t)),
                None => groups.push(Proposal {
                    id: proposal_id(&h, &th),
                    name: String::new(),
                    here: h,
                    there: th,
                    root: None,
                    repos: vec![pair(t)],
                }),
            }
        }
    }
    for t in theirs {
        let Some(k) = t.origin.as_deref().and_then(origin_key) else {
            continue;
        };
        if !mine_keys.contains(&k) {
            out.only_there
                .push(lone(t, root_for(roots, there, here, &t.path)));
        }
    }
    // A folder under another proposal's, at the same place in it, is that
    // proposal's.
    groups.sort_by(|a, b| {
        paths::depth(&a.here)
            .cmp(&paths::depth(&b.here))
            .then_with(|| a.here.cmp(&b.here))
    });
    let mut kept: Vec<Proposal> = Vec::new();
    for g in groups {
        if let Some(k) = kept.iter_mut().find(|k| {
            paths::rebase(&g.here, &k.here, &k.there)
                .is_some_and(|p| paths::same_path(&p, &g.there))
        }) {
            k.repos.extend(g.repos);
            continue;
        }
        kept.push(g);
    }
    // A root that names the folder here already: extended when it names
    // nothing there, else left as the user made it.
    let live: Vec<&Root> = roots.iter().filter(|r| r.live()).collect();
    let mut taken: Vec<String> = live.iter().map(|r| r.name.to_lowercase()).collect();
    for mut p in kept {
        let existing = live.iter().find(|r| {
            r.path_of(here)
                .is_some_and(|h| paths::same_path(h, &p.here))
        });
        match existing {
            Some(r) if r.path_of(there).is_some() => {
                out.mismatched.extend(p.repos);
                continue;
            }
            Some(r) => {
                p.root = Some(r.id.clone());
                p.name = r.name.clone();
            }
            None => {
                let base: String = roots::clean_name(&name_of(&p.here))
                    .unwrap_or_else(|_| "Projects".to_string())
                    .chars()
                    .take(roots::MAX_NAME - 3)
                    .collect();
                let mut name = base.clone();
                let mut n = 2;
                while taken.contains(&name.to_lowercase()) {
                    name = format!("{base} {n}");
                    n += 1;
                }
                taken.push(name.to_lowercase());
                p.name = name;
            }
        }
        p.repos.sort_by_key(|r| r.name.to_lowercase());
        out.proposals.push(p);
    }
    out.proposals.sort_by(|a, b| {
        b.repos
            .len()
            .cmp(&a.repos.len())
            .then_with(|| a.name.cmp(&b.name))
    });
    for l in [&mut out.only_here, &mut out.only_there] {
        l.sort_by_key(|r| r.name.to_lowercase());
    }
    out
}

/// `GET /api/machines/folders`: this computer's repositories matched with
/// every paired computer's, the roots each match proposes, and the ones on
/// one side only.
pub async fn view(state: &AppState, fresh: bool) -> Value {
    let here = crate::machines::machine_info(state).id;
    let mine = local_repos(state, fresh).await;
    let roots_now = roots::roots(state);
    let peers: Vec<(String, String)> = state.machines.store_read(|f| {
        f.peers
            .iter()
            .map(|p| (p.id.clone(), p.display_name().to_string()))
            .collect()
    });
    let answers =
        futures_util::future::join_all(peers.iter().map(|(id, _)| fetch(state, id, fresh))).await;
    let mut list = Vec::new();
    for ((id, name), r) in peers.iter().zip(answers) {
        let online = state.machines.link(id).online;
        match r {
            Ok(o) => {
                let to_there = |p: &str| roots::map_between(state, &here, id, p);
                let p = plan(&here, id, &roots_now, &mine, &o.repos, &to_there);
                let mut v = serde_json::to_value(&p).unwrap_or(Value::Null);
                v["machine"] = json!(id);
                v["name"] = json!(name);
                v["online"] = json!(online);
                v["error"] = Value::Null;
                list.push(v);
            }
            Err(e) => list.push(json!({
                "machine": id, "name": name, "online": online,
                "error": match e.as_str() {
                    "older" => format!("Update Mother Claude on {name} to see its projects."),
                    "refused" => format!("{name} does not let this computer see it."),
                    _ => format!("Can't reach {name} now: {e}"),
                },
                "proposals": [], "matched": [], "mismatched": [], "onlyHere": [], "onlyThere": [],
            })),
        }
    }
    let no_origin: Vec<Value> = mine
        .iter()
        .filter(|r| r.origin.is_none())
        .map(|r| json!({ "name": name_of(&r.path), "path": r.path }))
        .collect();
    json!({
        "machine": here,
        "repos": mine.len(),
        "noOrigin": no_origin,
        "peers": list,
    })
}

// ─── cloning and reaching a remote ─────────────────────────────────────────────

/// A git call that never prompts: Git Credential Manager non-interactive,
/// no askpass program (git itself runs with `GIT_TERMINAL_PROMPT=0`).
fn no_prompt(g: Git<'_>) -> Git<'_> {
    g.env("GCM_INTERACTIVE", "never")
        .env("GIT_ASKPASS", "")
        .env("SSH_ASKPASS_REQUIRE", "never")
}

/// What went wrong reaching a remote, in a sentence (never git's own
/// output, which can name the remote's URL).
pub fn problem(stderr: &str) -> &'static str {
    let s = stderr.to_ascii_lowercase();
    let any = |words: &[&str]| words.iter().any(|w| s.contains(w));
    if any(&[
        "terminal prompts disabled",
        "could not read username",
        "could not read password",
        "authentication failed",
        "authentication required",
        "access denied",
        "invalid username or password",
        "returned error: 401",
        "returned error: 403",
    ]) {
        "The remote asked for a sign-in: sign in to it on this computer (Git Credential Manager, or gh auth login), then try again."
    } else if any(&[
        "permission denied (publickey",
        "host key verification failed",
    ]) {
        "The remote refused this computer's SSH key."
    } else if any(&[
        "could not resolve host",
        "couldn't resolve host",
        "could not resolve hostname",
        "connection timed out",
        "connection refused",
        "failed to connect",
        "network is unreachable",
        "no route to host",
    ]) {
        "Can't reach the remote from this computer."
    } else if any(&[
        "repository not found",
        "does not appear to be a git repository",
        "not found",
    ]) {
        "The remote has no such repository, or this computer may not see it."
    } else {
        "git could not reach the remote."
    }
}

/// `git clone <origin> <folder>` (blocking): the folder must be free (absent
/// or empty) and is removed again when the clone fails. Submodules are not
/// followed; hooks never run; nothing prompts.
pub fn clone_into(git: &GitRunner, origin: &str, folder: &Path) -> Result<(), String> {
    if sanitize_origin(origin).as_deref() != Some(origin) {
        return Err("That is not a git remote Mother Claude can clone.".into());
    }
    if !super::asks::free_folder(folder) {
        return Err(format!("{} is there already.", folder.display()));
    }
    let existed = folder.exists();
    let parent = folder
        .parent()
        .ok_or_else(|| "That folder has no parent folder.".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("Could not create {}: {e}", parent.display()))?;
    let op = git.begin().map_err(|e| e.to_string())?;
    let out = no_prompt(
        op.cmd(parent)
            .args(["clone", "-q", "--no-recurse-submodules", "--"])
            .arg(origin)
            .arg(folder),
    )
    .output();
    match out {
        Ok(o) if o.success => Ok(()),
        Ok(o) => {
            if !existed {
                let _ = std::fs::remove_dir_all(folder);
            }
            Err(problem(&o.stderr).to_string())
        }
        Err(CodeError::NoGit(_)) => Err("git isn't installed on this computer.".into()),
        Err(e) => Err(e.to_string()),
    }
}

/// Whether a project's remote answers without a prompt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteCheck {
    /// `ok` | `failed`.
    pub state: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

impl RemoteCheck {
    fn failed(detail: &str, origin: Option<String>) -> Self {
        Self {
            state: "failed".into(),
            detail: detail.into(),
            origin,
        }
    }
}

/// `git ls-remote origin HEAD` in `repo`, without a prompt (blocking): the
/// check *Through your git remote* needs on both computers (plan §5.14),
/// made per project, on demand only.
pub fn remote_answers(git: &GitRunner, repo: &Path) -> RemoteCheck {
    remote_named_answers(git, repo, "origin")
}

/// [`remote_answers`] for the remote `remote` (a checked name: a move's).
pub fn remote_named_answers(git: &GitRunner, repo: &Path, remote: &str) -> RemoteCheck {
    if !crate::machines::code::valid_remote_name(remote) {
        return RemoteCheck::failed("That is not a remote's name.", None);
    }
    let op = match git.begin() {
        Ok(op) => op,
        Err(e) => return RemoteCheck::failed(&e.to_string(), None),
    };
    let origin = match op
        .cmd(repo)
        .args(["config", "--get", &format!("remote.{remote}.url")])
        .output()
    {
        Ok(o) if o.success => sanitize_origin(String::from_utf8_lossy(&o.stdout).trim()),
        Ok(_) => {
            return RemoteCheck::failed(
                &format!("This project has no remote named {remote}."),
                None,
            )
        }
        Err(CodeError::NoGit(_)) => {
            return RemoteCheck::failed("git isn't installed on this computer.", None)
        }
        Err(e) => return RemoteCheck::failed(&e.to_string(), None),
    };
    let out = no_prompt(op.cmd(repo).args(["ls-remote", "-q", remote, "HEAD"])).output();
    match out {
        Ok(o) if o.success => RemoteCheck {
            state: "ok".into(),
            detail: "The remote answers without a prompt.".into(),
            origin,
        },
        Ok(o) => RemoteCheck::failed(problem(&o.stderr), origin),
        Err(e) => RemoteCheck::failed(&e.to_string(), origin),
    }
}

/// [`remote_answers`] off the runtime, within [`REMOTE_TIMEOUT`].
pub async fn remote_answers_async(repo: PathBuf) -> RemoteCheck {
    remote_named_answers_async(GitRunner::new(), repo, "origin".into()).await
}

/// [`remote_named_answers`] off the runtime, within [`REMOTE_TIMEOUT`] (a
/// move's preflight: with the git moves run).
pub async fn remote_named_answers_async(
    git: GitRunner,
    repo: PathBuf,
    remote: String,
) -> RemoteCheck {
    let run = tokio::task::spawn_blocking(move || remote_named_answers(&git, &repo, &remote));
    match tokio::time::timeout(REMOTE_TIMEOUT, run).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => RemoteCheck::failed(&e.to_string(), None),
        Err(_) => RemoteCheck::failed("The remote did not answer in 30 seconds.", None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap as Map;

    #[test]
    fn origins_lose_their_credentials_and_odd_ones_are_refused() {
        assert_eq!(
            sanitize_origin("https://user:ghp_secret@github.com/you/app.git").as_deref(),
            Some("https://github.com/you/app.git")
        );
        assert_eq!(
            sanitize_origin("https://ghp_token@github.com/you/app").as_deref(),
            Some("https://github.com/you/app")
        );
        assert_eq!(
            sanitize_origin("ssh://git:pw@github.com/you/app.git").as_deref(),
            Some("ssh://git@github.com/you/app.git")
        );
        assert_eq!(
            sanitize_origin("git@github.com:you/app.git").as_deref(),
            Some("git@github.com:you/app.git")
        );
        assert_eq!(
            sanitize_origin("file:///srv/git/app.git").as_deref(),
            Some("file:///srv/git/app.git")
        );
        for bad in [
            "",
            "--upload-pack=touch /tmp/x",
            "ext::sh -c touch% /tmp/x",
            "/srv/git/app.git",
            r"C:\repos\app.git",
            "C:/repos/app.git",
            "https://github.com/a b",
            "fd::7",
            "user:pw@host:repo.git",
        ] {
            assert_eq!(sanitize_origin(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn one_repository_has_one_key_whatever_its_url() {
        let k = origin_key("git@github.com:You/App.git").unwrap();
        assert_eq!(k, "github.com/you/app");
        assert_eq!(origin_key("https://github.com/you/app").unwrap(), k);
        assert_eq!(origin_key("https://github.com/you/app.git/").unwrap(), k);
        assert_eq!(origin_key("ssh://git@github.com/you/app.git").unwrap(), k);
        assert_eq!(origin_key("https://tok@GitHub.com/you/app.git").unwrap(), k);
        assert_ne!(origin_key("https://gitlab.com/you/app").unwrap(), k);
        assert_eq!(origin_key("https://github.com/"), None);
    }

    #[test]
    fn parents_and_paths_inside_a_folder() {
        assert_eq!(
            parent_of("/Users/m/dev/projects/app").as_deref(),
            Some("/Users/m/dev/projects")
        );
        assert_eq!(
            parent_of(r"D:\dev\projects\app\").as_deref(),
            Some(r"D:\dev\projects")
        );
        assert_eq!(parent_of("/app"), None, "the file system's root");
        assert_eq!(parent_of(r"D:\app"), None);
        assert_eq!(rel_inside("/a/b/c/d", "/a/b").as_deref(), Some("c/d"));
        assert_eq!(rel_inside(r"D:\a\B\c", r"d:\a\b").as_deref(), Some("c"));
        assert_eq!(rel_inside("/a/b", "/a/b"), None);
        assert_eq!(rel_inside("/x/y", "/a"), None);
    }

    const MAC: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const PC: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn repo(path: &str, origin: Option<&str>) -> Repo {
        Repo {
            path: path.into(),
            origin: origin.map(str::to_string),
        }
    }

    fn root(id: &str, name: &str, paths: &[(&str, &str)]) -> Root {
        Root {
            id: id.into(),
            name: name.into(),
            paths: paths
                .iter()
                .map(|(m, p)| (m.to_string(), p.to_string()))
                .collect::<Map<_, _>>(),
            updated_at: 1,
            by: MAC.into(),
            pair: None,
            removed: None,
            extra: Map::new(),
        }
    }

    fn by_roots(roots: &[Root]) -> impl Fn(&str) -> Option<String> + '_ {
        move |p: &str| roots::map_in(roots, MAC, PC, p)
    }

    #[test]
    fn the_same_origins_under_the_same_names_propose_their_parents() {
        let mine = vec![
            repo(
                "/Users/m/development/projects/mother-claude",
                Some("git@github.com:you/mother-claude.git"),
            ),
            repo(
                "/Users/m/development/projects/tray-tool",
                Some("https://github.com/you/tray-tool"),
            ),
            repo(
                "/Users/m/development/projects/org/lib",
                Some("https://github.com/org/lib.git"),
            ),
            repo(
                "/Users/m/development/projects/side-project",
                Some("https://github.com/you/side-project"),
            ),
            repo("/Users/m/notes", None),
        ];
        let theirs = vec![
            repo(
                r"D:\development\projects\mother-claude",
                Some("https://github.com/you/mother-claude.git"),
            ),
            repo(
                r"D:\development\projects\tray-tool",
                Some("https://tok@github.com/you/tray-tool.git"),
            ),
            repo(
                r"D:\development\projects\org\lib",
                Some("git@github.com:org/lib.git"),
            ),
            repo(
                r"D:\development\projects\win-tools",
                Some("https://github.com/you/win-tools"),
            ),
        ];
        let roots: Vec<Root> = Vec::new();
        let p = plan(MAC, PC, &roots, &mine, &theirs, &by_roots(&roots));
        assert_eq!(p.proposals.len(), 1, "{:#?}", p.proposals);
        let one = &p.proposals[0];
        assert_eq!(one.here, "/Users/m/development/projects");
        assert_eq!(one.there, r"D:\development\projects");
        assert_eq!(one.name, "projects");
        assert!(one.id.starts_with("pr_"));
        assert_eq!(
            one.repos
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>(),
            ["lib", "mother-claude", "tray-tool"],
            "org/lib is under the same folder, at the same place"
        );
        assert_eq!(p.only_here.len(), 1);
        assert_eq!(p.only_here[0].name, "side-project");
        assert_eq!(p.only_here[0].root, None, "no root maps it yet");
        assert_eq!(p.only_there.len(), 1);
        assert_eq!(p.only_there[0].name, "win-tools");
        assert!(p.matched.is_empty() && p.mismatched.is_empty());
    }

    #[test]
    fn a_root_maps_matches_and_offers_clones_both_ways() {
        let roots = vec![root(
            "rt_1111111111111111",
            "projects",
            &[(MAC, "/Users/m/dev"), (PC, r"D:\dev")],
        )];
        let mine = vec![
            repo("/Users/m/dev/app", Some("https://github.com/you/app")),
            repo("/Users/m/dev/side", Some("https://github.com/you/side")),
            repo("/Users/m/other/thing", Some("https://github.com/you/thing")),
        ];
        let theirs = vec![
            repo(r"D:\dev\app", Some("https://github.com/you/app.git")),
            repo(r"D:\dev\win", Some("https://github.com/you/win")),
            repo(r"E:\elsewhere\thing", Some("https://github.com/you/thing")),
        ];
        let p = plan(MAC, PC, &roots, &mine, &theirs, &by_roots(&roots));
        assert_eq!(p.matched.len(), 1);
        assert_eq!(p.matched[0].name, "app");
        let side = &p.only_here[0];
        assert_eq!(side.name, "side");
        assert_eq!(side.root.as_deref(), Some("rt_1111111111111111"));
        assert_eq!(side.rel.as_deref(), Some("side"));
        assert_eq!(
            side.mapped.as_deref(),
            Some(r"D:\dev\side"),
            "where a clone lands there"
        );
        let win = &p.only_there[0];
        assert_eq!(win.rel.as_deref(), Some("win"));
        assert_eq!(win.mapped.as_deref(), Some("/Users/m/dev/win"));
        // `thing` is on both, outside the root: their parents are proposed.
        assert_eq!(p.proposals.len(), 1);
        assert_eq!(p.proposals[0].here, "/Users/m/other");
        assert_eq!(p.proposals[0].there, r"E:\elsewhere");
        assert_eq!(p.proposals[0].name, "other");

        // Under other folder names, the two folders themselves.
        let theirs = vec![repo(
            r"E:\elsewhere\thing-win",
            Some("https://github.com/you/thing"),
        )];
        let mine = vec![repo(
            "/Users/m/other/thing",
            Some("https://github.com/you/thing"),
        )];
        let p = plan(MAC, PC, &roots, &mine, &theirs, &by_roots(&roots));
        assert_eq!(p.proposals[0].here, "/Users/m/other/thing");
        assert_eq!(p.proposals[0].there, r"E:\elsewhere\thing-win");
        assert_eq!(p.proposals[0].name, "thing");
    }

    #[test]
    fn a_root_already_here_is_extended_and_one_mapping_elsewhere_is_kept() {
        let half = vec![root(
            "rt_2222222222222222",
            "projects",
            &[(MAC, "/Users/m/dev")],
        )];
        let mine = vec![repo("/Users/m/dev/app", Some("https://github.com/you/app"))];
        let theirs = vec![repo(r"D:\code\app", Some("https://github.com/you/app"))];
        let p = plan(MAC, PC, &half, &mine, &theirs, &by_roots(&half));
        assert_eq!(p.proposals[0].root.as_deref(), Some("rt_2222222222222222"));
        assert_eq!(p.proposals[0].name, "projects");
        assert_eq!(p.proposals[0].there, r"D:\code");

        let elsewhere = vec![root(
            "rt_3333333333333333",
            "projects",
            &[(MAC, "/Users/m/dev"), (PC, r"D:\dev")],
        )];
        let p = plan(MAC, PC, &elsewhere, &mine, &theirs, &by_roots(&elsewhere));
        assert!(p.proposals.is_empty());
        assert_eq!(p.mismatched.len(), 1, "the roots send it where it is not");

        // A name a root has already is not proposed twice.
        let named = vec![root("rt_4444444444444444", "dev", &[(MAC, "/x/dev")])];
        let mine = vec![repo("/Users/m/dev/app", Some("https://github.com/you/app"))];
        let theirs = vec![repo(r"D:\dev\app", Some("https://github.com/you/app"))];
        let p = plan(MAC, PC, &named, &mine, &theirs, &by_roots(&named));
        assert_eq!(p.proposals[0].name, "dev 2");
    }

    #[test]
    fn remote_problems_are_told_in_words_without_the_url() {
        let cases = [
            ("fatal: could not read Username for 'https://github.com': terminal prompts disabled", "sign-in"),
            ("remote: Invalid username or password.\nfatal: Authentication failed for 'https://x'", "sign-in"),
            ("git@github.com: Permission denied (publickey).\r\nfatal: Could not read from remote repository.", "SSH key"),
            ("fatal: unable to access 'https://nohost.invalid/x/': Could not resolve host: nohost.invalid", "reach"),
            ("remote: Repository not found.\nfatal: repository 'https://github.com/x/y/' not found", "no such repository"),
            ("fatal: 'origin' does not appear to be a git repository", "no such repository"),
            ("something else", "could not reach"),
        ];
        for (stderr, word) in cases {
            let p = problem(stderr);
            assert!(p.contains(word), "{stderr:?} → {p}");
            assert!(!p.contains("http"), "never the URL: {p}");
        }
    }

    #[test]
    fn another_computer_runs_only_a_few_remote_checks_or_code_tests_at_once() {
        let state = crate::state::Inner::new(
            crate::claude::ClaudeHome::with_base(std::env::temp_dir()),
            crate::state::ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            crate::server::auth::Auth::ephemeral(),
        );
        let slots: Vec<RemoteSlot> = (0..MAX_REMOTE_CHECKS)
            .map(|_| remote_slot(&state).expect("a slot"))
            .collect();
        assert!(remote_slot(&state).is_none(), "all taken");
        drop(slots);
        assert!(remote_slot(&state).is_some(), "released when dropped");
        let halves: Vec<_> = (0..4)
            .map(|_| super::super::codetest::begin_half(&state).expect("a half"))
            .collect();
        assert!(super::super::codetest::begin_half(&state).is_none());
        drop(halves);
        assert!(super::super::codetest::begin_half(&state).is_some());
    }

    #[test]
    fn worktrees_claude_code_made_are_not_other_repositories() {
        assert!(in_claude_worktree("/Users/m/app/.claude/worktrees/role-a"));
        assert!(in_claude_worktree(r"D:\app\.claude\worktrees\x"));
        assert!(!in_claude_worktree("/Users/m/app"));
    }
}
