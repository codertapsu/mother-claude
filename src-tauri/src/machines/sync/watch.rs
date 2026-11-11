//! The project watcher (docs/PLAN-MACHINES.md §5.15 "When"): one per
//! synced project while the app watches, over the checkout's folder.
//!
//! What counts as a change: a file of the working tree that git does not
//! ignore, and `.git/HEAD`, `.git/index`, `.git/packed-refs` and
//! `.git/refs/**` (a commit, a branch switch, a stage). Object churn
//! (`.git/objects`, which a state check itself writes), Mother Claude's own
//! refs (`refs/mother-claude/`), logs and lock files never do, nor do paths git ignores — asked of `git check-ignore` once
//! per folder and remembered, so a build writing thousands of files under
//! `target/` costs one question. After a change, [`QUIET`] without another
//! one, the pair's check runs ([`Quiet`], a trailing debounce); the check
//! compares states, so its own landing's writes start nothing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Weak;
use std::time::{Duration, Instant};

use crate::machines::code::GitRunner;
use crate::state::{AppState, Inner};

/// The quiet time after a change before the check runs.
pub const QUIET: Duration = Duration::from_secs(3);
/// How long what `git check-ignore` said of a folder is trusted.
const IGNORES_FOR: Duration = Duration::from_secs(300);
/// Folders above a changed path asked about at most.
const DEPTH: usize = 8;

/// A trailing debounce: due once `quiet` passed since the last change.
#[derive(Debug, Default)]
pub struct Quiet {
    deadline: Option<Instant>,
}

impl Quiet {
    /// A change at `now`: the check waits `quiet` from here.
    pub fn touch(&mut self, now: Instant, quiet: Duration) {
        self.deadline = Some(now + quiet);
    }

    /// How long until the check is due (none: nothing changed).
    pub fn wait(&self, now: Instant) -> Option<Duration> {
        self.deadline.map(|d| d.saturating_duration_since(now))
    }

    /// Whether the check is due at `now` (then it is no longer pending).
    pub fn take(&mut self, now: Instant) -> bool {
        match self.deadline {
            Some(d) if now >= d => {
                self.deadline = None;
                true
            }
            _ => false,
        }
    }
}

/// What a changed path is to the watcher.
#[derive(Debug, PartialEq, Eq)]
pub enum Kind {
    /// Not under the checkout.
    Outside,
    /// git's own churn (objects, logs, locks, our scratch).
    Noise,
    /// HEAD, the index or a ref: always a change.
    Git,
    /// A path of the working tree (`/`-separated, under the top).
    Tree(String),
}

/// What `path` is, under the checkout `top` (both as the file system names
/// them).
pub fn classify(top: &Path, path: &Path) -> Kind {
    let Ok(rel) = path.strip_prefix(top) else {
        return Kind::Outside;
    };
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let Some(first) = parts.first() else {
        return Kind::Noise;
    };
    if first.eq_ignore_ascii_case(".git") {
        let rest = &parts[1..];
        let last = rest.last().map(String::as_str).unwrap_or_default();
        if last.ends_with(".lock") {
            return Kind::Noise;
        }
        return match rest.first().map(String::as_str) {
            Some("HEAD" | "index" | "packed-refs") if rest.len() == 1 => Kind::Git,
            // Mother Claude's own refs (a step's snapshots, kept states).
            Some("refs") if rest.get(1).map(String::as_str) == Some("mother-claude") => Kind::Noise,
            Some("refs") if rest.len() > 1 => Kind::Git,
            _ => Kind::Noise,
        };
    }
    Kind::Tree(parts.join("/"))
}

/// What `git check-ignore` said of folders, remembered for a while.
#[derive(Debug, Default)]
pub struct Ignores {
    dirs: HashMap<String, bool>,
    since: Option<Instant>,
}

impl Ignores {
    /// Whether any of `rels` (paths under `top`) is a change: not ignored,
    /// nor under an ignored folder. A `.gitignore` changing is one, and
    /// forgets what was remembered. Blocking (git may run).
    pub fn any_change(&mut self, git: &GitRunner, top: &Path, rels: &[String]) -> bool {
        if self.since.is_none_or(|t| t.elapsed() > IGNORES_FOR)
            || rels
                .iter()
                .any(|r| r == ".gitignore" || r.ends_with("/.gitignore"))
        {
            self.dirs.clear();
            self.since = Some(Instant::now());
        }
        let mut ask_dirs: Vec<String> = Vec::new();
        let mut candidates: Vec<&String> = Vec::new();
        for rel in rels {
            if rel.is_empty() {
                continue;
            }
            let parts: Vec<&str> = rel.split('/').collect();
            let mut known_ignored = false;
            for i in 1..parts.len().min(DEPTH + 1) {
                let dir = parts[..i].join("/");
                match self.dirs.get(&dir) {
                    Some(true) => {
                        known_ignored = true;
                        break;
                    }
                    Some(false) => {}
                    None => {
                        if !ask_dirs.contains(&dir) {
                            ask_dirs.push(dir);
                        }
                    }
                }
            }
            if !known_ignored {
                candidates.push(rel);
            }
        }
        if candidates.is_empty() {
            return false;
        }
        // One question: the folders not known yet (as folders) and the paths.
        let mut asked: Vec<String> = ask_dirs.iter().map(|d| format!("{d}/")).collect();
        asked.extend(candidates.iter().map(|r| r.to_string()));
        let ignored = match ignored_of(git, top, &asked) {
            Ok(list) => list,
            // git could not say: count it as a change (the check is cheap).
            Err(_) => return true,
        };
        for d in &ask_dirs {
            let as_dir = format!("{d}/");
            self.dirs
                .insert(d.clone(), ignored.iter().any(|p| *p == as_dir || p == d));
        }
        candidates.iter().any(|rel| {
            let parts: Vec<&str> = rel.split('/').collect();
            let under_ignored = (1..parts.len().min(DEPTH + 1))
                .any(|i| self.dirs.get(&parts[..i].join("/")) == Some(&true));
            !under_ignored && !ignored.iter().any(|p| p == *rel)
        })
    }
}

/// The paths of `paths` git ignores in the checkout at `top`.
fn ignored_of(git: &GitRunner, top: &Path, paths: &[String]) -> Result<Vec<String>, String> {
    let op = git.begin().map_err(|e| e.to_string())?;
    let input: Vec<u8> = paths
        .iter()
        .flat_map(|p| p.bytes().chain(std::iter::once(0)))
        .collect();
    let out = op
        .cmd(top)
        .args(["check-ignore", "-z", "--stdin"])
        .input(input)
        .output()
        .map_err(|e| e.to_string())?;
    // 0: some are ignored, 1: none is; anything else is git failing.
    if !out.success && out.code != 1 {
        return Err(out.stderr);
    }
    Ok(out
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect())
}

/// A pair's watcher: dropping it stops it.
pub struct Watch {
    _fs: crate::claude::FsWatcher,
    task: tokio::task::AbortHandle,
    pub top: PathBuf,
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Watches the checkout at `top`: `on_quiet` runs once `quiet` passed
/// without another change after one. Needs a Tokio runtime.
pub fn spawn_with(
    top: PathBuf,
    git: GitRunner,
    quiet: Duration,
    on_quiet: impl Fn() + Send + Sync + 'static,
) -> anyhow::Result<Watch> {
    let real = dunce::canonicalize(&top).unwrap_or_else(|_| top.clone());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<PathBuf>>();
    let fs = crate::claude::FsWatcher::spawn(std::slice::from_ref(&real), tx)?;
    let tops = [top.clone(), real.clone()];
    let task = tokio::spawn(async move {
        let mut quiet_for = Quiet::default();
        let mut ignores = Some(Ignores::default());
        loop {
            let wait = quiet_for.wait(Instant::now());
            tokio::select! {
                batch = rx.recv() => {
                    let Some(batch) = batch else { break };
                    let mut git_change = false;
                    let mut rels: Vec<String> = Vec::new();
                    for p in &batch {
                        let kind = tops
                            .iter()
                            .map(|t| classify(t, p))
                            .find(|k| *k != Kind::Outside)
                            .unwrap_or(Kind::Outside);
                        match kind {
                            Kind::Git => git_change = true,
                            Kind::Tree(rel) => {
                                if !rels.contains(&rel) {
                                    rels.push(rel);
                                }
                            }
                            Kind::Noise | Kind::Outside => {}
                        }
                    }
                    let changed = if git_change {
                        true
                    } else if rels.is_empty() {
                        false
                    } else {
                        let (g, t, mut ig) = (git.clone(), real.clone(), ignores.take().unwrap_or_default());
                        let joined = tokio::task::spawn_blocking(move || {
                            let any = ig.any_change(&g, &t, &rels);
                            (any, ig)
                        })
                        .await;
                        match joined {
                            Ok((any, ig)) => {
                                ignores = Some(ig);
                                any
                            }
                            Err(_) => true,
                        }
                    };
                    if changed {
                        quiet_for.touch(Instant::now(), quiet);
                    }
                }
                _ = tokio::time::sleep(wait.unwrap_or(Duration::from_secs(3600))), if wait.is_some() => {
                    if quiet_for.take(Instant::now()) {
                        on_quiet();
                    }
                }
            }
        }
    })
    .abort_handle();
    Ok(Watch { _fs: fs, task, top })
}

/// Starts a watcher for every pair that is on on both computers and stops
/// the others' (while the app watches).
pub fn refresh(state: &AppState) {
    let sync = &state.machines.sync;
    if !sync.watching() {
        return;
    }
    let want: Vec<(String, String)> = sync.read(|f| {
        f.pairs
            .iter()
            .filter(|p| p.active())
            .map(|p| (p.id.clone(), p.folder.clone()))
            .collect()
    });
    let mut watchers = sync.watchers.lock().unwrap_or_else(|e| e.into_inner());
    watchers.retain(|id, w| want.iter().any(|(i, f)| i == id && w.top == Path::new(f)));
    for (id, folder) in want {
        if watchers.contains_key(&id) {
            continue;
        }
        let weak: Weak<Inner> = std::sync::Arc::downgrade(state);
        let pair = id.clone();
        let started = spawn_with(
            PathBuf::from(&folder),
            state.machines.moves.git(),
            QUIET,
            move || {
                if let Some(st) = weak.upgrade() {
                    let id = pair.clone();
                    tokio::spawn(async move {
                        super::step::run_auto(&st, &id, super::Trigger::Watch).await;
                    });
                }
            },
        );
        match started {
            Ok(w) => {
                watchers.insert(id, w);
            }
            Err(e) => tracing::warn!(error = %e, "a synced project could not be watched"),
        }
    }
}

/// Stops every watcher (Machines turned off, the app stopping).
pub fn stop_all(state: &AppState) {
    state
        .machines
        .sync
        .watchers
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn the_quiet_time_runs_from_the_last_change() {
        let t0 = Instant::now();
        let q3 = Duration::from_secs(3);
        let mut q = Quiet::default();
        assert_eq!(q.wait(t0), None);
        assert!(!q.take(t0 + q3 * 10), "nothing changed");
        q.touch(t0, q3);
        q.touch(t0 + Duration::from_secs(2), q3);
        assert!(
            !q.take(t0 + Duration::from_secs(4)),
            "a change two seconds in"
        );
        assert_eq!(
            q.wait(t0 + Duration::from_secs(4)),
            Some(Duration::from_secs(1))
        );
        assert!(q.take(t0 + Duration::from_secs(5)));
        assert!(!q.take(t0 + Duration::from_secs(6)), "once");
    }

    #[test]
    fn git_state_counts_and_its_churn_does_not() {
        let top = Path::new("/p/app");
        let k = |rel: &str| classify(top, &top.join(rel));
        assert_eq!(k("src/main.rs"), Kind::Tree("src/main.rs".into()));
        assert_eq!(k(".git/HEAD"), Kind::Git);
        assert_eq!(k(".git/index"), Kind::Git);
        assert_eq!(k(".git/packed-refs"), Kind::Git);
        assert_eq!(k(".git/refs/heads/main"), Kind::Git);
        for noise in [
            ".git/objects/ab/cdef",
            ".git/index.lock",
            ".git/refs/heads/main.lock",
            ".git/logs/HEAD",
            ".git/FETCH_HEAD",
            ".git",
            ".git/refs",
            ".git/refs/mother-claude/moves/sy_0123456789abcdef/1/snap",
        ] {
            assert_eq!(k(noise), Kind::Noise, "{noise}");
        }
        assert_eq!(classify(top, Path::new("/elsewhere/x")), Kind::Outside);
    }

    fn repo() -> (tempfile::TempDir, PathBuf, GitRunner) {
        let tmp = tempfile::tempdir().unwrap();
        let top = dunce::canonicalize(tmp.path()).unwrap();
        let git = GitRunner::new().isolated();
        let op = git.begin().unwrap();
        op.git(&top, &["init", "-q"]).unwrap();
        std::fs::write(top.join(".gitignore"), "build/\n*.log\n").unwrap();
        (tmp, top, git)
    }

    #[test]
    fn ignored_paths_are_asked_once_per_folder() {
        let (_tmp, top, git) = repo();
        let mut ig = Ignores::default();
        let rels = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(!ig.any_change(&git, &top, &rels(&["build/a.o", "build/deep/b.o", "x.log"])));
        assert_eq!(ig.dirs.get("build"), Some(&true));
        // Under a folder known to be ignored: no git at all.
        let broken = GitRunner::with_program("/nonexistent/git");
        assert!(!ig.any_change(&broken, &top, &rels(&["build/c.o"])));
        assert!(ig.any_change(&git, &top, &rels(&["build/c.o", "src/main.rs"])));
        assert!(ig.any_change(&git, &top, &rels(&[".gitignore"])));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_burst_of_changes_runs_one_check_after_the_quiet_time() {
        let (_tmp, top, git) = repo();
        std::fs::create_dir_all(top.join("src")).unwrap();
        let fired = Arc::new(AtomicUsize::new(0));
        let f = fired.clone();
        let quiet = Duration::from_millis(600);
        let _w = spawn_with(top.clone(), git, quiet, move || {
            f.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        // Let the watcher settle (FSEvents' latency).
        tokio::time::sleep(Duration::from_millis(700)).await;
        let wait_for = |want: usize| {
            let fired = fired.clone();
            async move {
                for _ in 0..150 {
                    if fired.load(Ordering::SeqCst) >= want {
                        return true;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                false
            }
        };
        // Object churn and ignored paths: nothing.
        std::fs::create_dir_all(top.join(".git/objects/ab")).unwrap();
        std::fs::write(top.join(".git/objects/ab/cdef"), b"x").unwrap();
        std::fs::create_dir_all(top.join("build/deep")).unwrap();
        std::fs::write(top.join("build/deep/out.o"), b"x").unwrap();
        std::fs::write(top.join("debug.log"), b"x").unwrap();
        tokio::time::sleep(Duration::from_millis(2500)).await;
        let noise = fired.load(Ordering::SeqCst);
        // A burst of edits: one check, once it is quiet.
        for i in 0..4 {
            std::fs::write(top.join("src/a.txt"), format!("edit {i}")).unwrap();
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        assert!(wait_for(noise + 1).await, "the edits were noticed");
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert_eq!(
            fired.load(Ordering::SeqCst),
            noise + 1,
            "one check for the burst"
        );
        // A ref moving counts.
        std::fs::create_dir_all(top.join(".git/refs/heads")).unwrap();
        std::fs::write(top.join(".git/refs/heads/topic"), b"0000\n").unwrap();
        assert!(wait_for(noise + 2).await, "a ref change was noticed");
        assert_eq!(noise, 0, "noise started a check");
    }
}
