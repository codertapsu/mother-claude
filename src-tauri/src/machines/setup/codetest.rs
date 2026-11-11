//! The readiness test's git round trip (docs/PLAN-MACHINES.md §5.14, step
//! 3): a scratch repository in this computer's data dir — never a project —
//! with a commit and a dirty tree (staged, unstaged, untracked and ignored
//! files), snapshotted as a move snapshots one (`code::snapshot`, a
//! bundle), sent to the other computer over the peer link, landed there in
//! a fresh scratch repository (`code::apply`, the checkout landing) and
//! proven file by file (`code::verify`); then the same the other way. Both
//! computers delete their scratch folders after.
//!
//! Decision (the plan leaves the transfer open, "the chosen way"): the
//! round trip always travels machine to machine. A scratch repository has
//! no remote, and the test never pushes to one of yours: with *Through your
//! git remote* chosen, each project's remote is checked on demand instead
//! (`POST /api/machines/setup/remote`) and again by each move's preflight.

use std::path::{Path, PathBuf};
use std::time::Instant;

use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};

use super::TestLine;
use crate::machines::code::{
    apply, snapshot, verify, ApplyRequest, GitRunner, Landing, Outcome, SnapshotRequest, BUNDLE,
    META,
};
use crate::machines::transport;
use crate::state::AppState;

/// The session id every test snapshot carries.
pub const SESSION: &str = "mc-readiness-test";
/// Scratch folders, inside the data dir.
pub const SCRATCH: &str = "machines/test";
/// Where another computer's half is asked for.
pub const PEER_PATH: &str = "/peer/setup/code-test";
/// A package larger than this is refused (a scratch bundle is a few KB).
const MAX_BUNDLE: usize = 4 << 20;
/// Test halves one computer runs at once.
const MAX_RUNNING: usize = 4;

/// A folder of its own for one half of a test, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(data_dir: &Path) -> Result<Self, String> {
        let base = data_dir.join(SCRATCH);
        sweep(&base);
        let dir = base.join(uuid::Uuid::new_v4().simple().to_string());
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Could not make a scratch folder: {e}"))?;
        Ok(Self(dir))
    }
}

/// Scratch folders older than an hour: left by a test the app did not
/// finish (it quit, or crashed). Removed.
fn sweep(base: &Path) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    let hour = std::time::Duration::from_secs(3600);
    for e in entries.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > hour);
        if old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir_all(&self.0) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(dir = %self.0.display(), error = %e, "a readiness test's scratch folder was left behind");
            }
        }
    }
}

fn write(dir: &Path, rel: &str, content: &str) -> Result<(), String> {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&p, content).map_err(|e| e.to_string())
}

/// A scratch repository with one commit and a dirty tree, snapshotted for
/// the other computer (blocking): the snapshot's record and its bundle.
pub fn make(git: &GitRunner, data_dir: &Path, machine: &str) -> Result<(Value, Vec<u8>), String> {
    let scratch = Scratch::new(data_dir)?;
    let src = scratch.0.join("src");
    std::fs::create_dir_all(&src).map_err(|e| e.to_string())?;
    let op = git.begin().map_err(|e| e.to_string())?;
    let run = |args: &[&str]| op.git(&src, args).map(|_| ()).map_err(|e| e.to_string());
    run(&["-c", "init.defaultBranch=main", "init", "-q"])?;
    write(&src, "README.md", "# Readiness test\n")?;
    write(&src, "src/main.txt", "one\ntwo\n")?;
    write(&src, "docs/with space.txt", "spaces in a name\n")?;
    write(&src, ".gitignore", "*.log\n")?;
    run(&["add", "-A"])?;
    run(&[
        "-c",
        "user.name=Mother Claude",
        "-c",
        "user.email=mother-claude@localhost",
        "commit",
        "-q",
        "-m",
        "Readiness test",
    ])?;
    // What a move carries: staged, unstaged, untracked; never ignored.
    write(&src, "staged.txt", "staged\n")?;
    run(&["add", "staged.txt"])?;
    write(&src, "src/main.txt", "one\ntwo\nthree\n")?;
    write(&src, "notes/todo.md", "- [ ] move\n")?;
    write(&src, "debug.log", "ignored\n")?;
    drop(op);
    let out = scratch.0.join("move");
    let req = SnapshotRequest::new(SESSION, 1, machine, &out);
    snapshot(git, &src, &req).map_err(|e| e.to_string())?;
    let meta: Value = serde_json::from_slice(
        &std::fs::read(out.join(META)).map_err(|e| format!("The snapshot's record: {e}"))?,
    )
    .map_err(|e| format!("The snapshot's record: {e}"))?;
    let bundle = std::fs::read(out.join(BUNDLE)).map_err(|e| format!("The bundle: {e}"))?;
    Ok((meta, bundle))
}

/// What landing a test package found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Landed {
    pub outcome: Outcome,
    pub fails: Vec<String>,
}

/// Lands another computer's package in a fresh scratch repository and
/// proves it (blocking). Only a scratch folder in the data dir is written.
pub fn land(
    git: &GitRunner,
    data_dir: &Path,
    meta: &Value,
    bundle: &[u8],
    machine: &str,
) -> Result<Landed, String> {
    if meta["session"] != SESSION {
        return Err("That is not a readiness test.".into());
    }
    let scratch = Scratch::new(data_dir)?;
    let dst = scratch.0.join("dst");
    let in_dir = scratch.0.join("in");
    std::fs::create_dir_all(&dst).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&in_dir).map_err(|e| e.to_string())?;
    std::fs::write(
        in_dir.join(META),
        serde_json::to_vec(meta).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(in_dir.join(BUNDLE), bundle).map_err(|e| e.to_string())?;
    {
        let op = git.begin().map_err(|e| e.to_string())?;
        op.git(&dst, &["-c", "init.defaultBranch=main", "init", "-q"])
            .map_err(|e| e.to_string())?;
    }
    let mut req = ApplyRequest::new(SESSION, Landing::Checkout);
    req.machine = machine.to_string();
    let applied = apply(git, &dst, &in_dir, &req).map_err(|e| e.to_string())?;
    let report = verify(git, &applied.path, &in_dir).map_err(|e| e.to_string())?;
    Ok(Landed {
        outcome: report.outcome,
        fails: report.fails,
    })
}

/// A package on the wire: the record, and the bundle in base64.
#[derive(Debug, Deserialize)]
pub struct Package {
    pub meta: Value,
    pub bundle: String,
}

pub fn package(meta: Value, bundle: &[u8]) -> Value {
    json!({
        "meta": meta,
        "bundle": base64::engine::general_purpose::STANDARD.encode(bundle),
    })
}

pub fn unpack(p: &Package) -> Result<Vec<u8>, String> {
    if p.bundle.len() > MAX_BUNDLE * 4 / 3 + 4 {
        return Err("The test package is too large.".into());
    }
    base64::engine::general_purpose::STANDARD
        .decode(p.bundle.as_bytes())
        .map_err(|_| "The test package is not readable.".to_string())
}

/// One half at a time per slot: `None` when [`MAX_RUNNING`] already run.
pub fn begin_half(state: &AppState) -> Option<HalfGuard> {
    super::with_cache(state, |c| {
        (c.code_tests < MAX_RUNNING).then(|| {
            c.code_tests += 1;
        })
    })?;
    Some(HalfGuard(state.clone()))
}

pub struct HalfGuard(AppState);

impl Drop for HalfGuard {
    fn drop(&mut self) {
        super::with_cache(&self.0, |c| c.code_tests = c.code_tests.saturating_sub(1));
    }
}

fn data_dir(state: &AppState) -> Result<PathBuf, String> {
    state
        .auth
        .config_dir
        .clone()
        .ok_or_else(|| "This computer keeps no data folder.".to_string())
}

/// `GET /peer/setup/code-test`: this computer's half of the way back — a
/// scratch snapshot for the asking computer to land.
pub async fn peer_make(state: &AppState) -> Result<Value, String> {
    let _guard = begin_half(state).ok_or("A readiness test runs already.")?;
    let dir = data_dir(state)?;
    let name = crate::machines::this_name(state);
    let (meta, bundle) = tokio::task::spawn_blocking(move || make(&GitRunner::new(), &dir, &name))
        .await
        .map_err(|e| e.to_string())??;
    Ok(package(meta, &bundle))
}

/// `POST /peer/setup/code-test`: lands the asking computer's package here
/// and says how it went.
pub async fn peer_land(state: &AppState, p: Package) -> Result<Value, String> {
    let _guard = begin_half(state).ok_or("A readiness test runs already.")?;
    let bundle = unpack(&p)?;
    let dir = data_dir(state)?;
    let name = crate::machines::this_name(state);
    let started = Instant::now();
    let landed =
        tokio::task::spawn_blocking(move || land(&GitRunner::new(), &dir, &p.meta, &bundle, &name))
            .await
            .map_err(|e| e.to_string())??;
    Ok(json!({
        "outcome": landed.outcome,
        "fails": landed.fails,
        "ms": started.elapsed().as_millis() as u64,
    }))
}

fn landed_ok(outcome: &str) -> bool {
    matches!(outcome, "pass" | "limit")
}

/// One way: this computer's snapshot landed there.
async fn there(state: &AppState, machine: &str) -> Result<(), String> {
    let _guard = begin_half(state).ok_or("A readiness test runs already here.")?;
    let dir = data_dir(state)?;
    let name = crate::machines::this_name(state);
    let (meta, bundle) = tokio::task::spawn_blocking(move || make(&GitRunner::new(), &dir, &name))
        .await
        .map_err(|e| e.to_string())??;
    let (status, v) = transport::request(
        state,
        machine,
        "POST",
        PEER_PATH,
        Some(package(meta, &bundle)),
        std::time::Duration::from_secs(90),
    )
    .await?;
    match status {
        200 if landed_ok(v["outcome"].as_str().unwrap_or("")) => Ok(()),
        200 => Err(format!(
            "the code came out different ({})",
            v["fails"]
                .as_array()
                .map(|f| f
                    .iter()
                    .filter_map(Value::as_str)
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(", "))
                .unwrap_or_default()
        )),
        404 => Err("older".into()),
        _ => Err(v
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("it refused")
            .to_string()),
    }
}

/// The other way: its snapshot landed here.
async fn back(state: &AppState, machine: &str) -> Result<(), String> {
    let (status, v) = transport::request(
        state,
        machine,
        "GET",
        PEER_PATH,
        None,
        std::time::Duration::from_secs(90),
    )
    .await?;
    match status {
        200 => {}
        404 => return Err("older".into()),
        _ => {
            return Err(v
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("it refused")
                .to_string())
        }
    }
    let p: Package =
        serde_json::from_value(v).map_err(|_| "it answered something unexpected".to_string())?;
    let _guard = begin_half(state).ok_or("A readiness test runs already here.")?;
    let bundle = unpack(&p)?;
    let dir = data_dir(state)?;
    let name = crate::machines::this_name(state);
    let landed =
        tokio::task::spawn_blocking(move || land(&GitRunner::new(), &dir, &p.meta, &bundle, &name))
            .await
            .map_err(|e| e.to_string())??;
    if matches!(landed.outcome, Outcome::Pass | Outcome::Limit) {
        Ok(())
    } else {
        Err(format!(
            "the code came out different ({})",
            landed
                .fails
                .into_iter()
                .take(3)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

/// The `code` line: a snapshot of a scratch repository each way between
/// this computer and `machine`, landed and proven.
pub async fn round_trip(state: &AppState, machine: &str, name: &str) -> TestLine {
    let started = Instant::now();
    let to = there(state, machine).await;
    let from = back(state, machine).await;
    let ms = (started.elapsed().as_secs_f64() * 1000.0).round();
    let line = |state: &str, detail: String| TestLine {
        id: "code".into(),
        state: state.into(),
        detail,
        ms: Some(ms),
        mibps: None,
    };
    match (to, from) {
        (Ok(()), Ok(())) => line("ok", format!("Both ways in {:.1} s.", ms / 1000.0)),
        (Err(a), Err(b)) if a == "older" && b == "older" => line(
            "skipped",
            format!("Update Mother Claude on {name} to test moving code."),
        ),
        (to, from) => {
            let word = |r: &Result<(), String>| match r {
                Ok(()) => "✓".to_string(),
                Err(e) if e == "older" => format!("update Mother Claude on {name}"),
                Err(e) => e.clone(),
            };
            line(
                "failed",
                format!("To {name}: {} · From {name}: {}", word(&to), word(&from)),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scratch_snapshot_lands_and_proves_itself() {
        let data = tempfile::tempdir().unwrap();
        let git = GitRunner::new().isolated();
        let (meta, bundle) = make(&git, data.path(), "MacBook").expect("a snapshot");
        assert_eq!(meta["session"], SESSION);
        assert!(!bundle.is_empty());
        // Over the wire and back.
        let p: Package = serde_json::from_value(package(meta, &bundle)).unwrap();
        let bytes = unpack(&p).unwrap();
        assert_eq!(bytes, bundle);
        let landed = land(&git, data.path(), &p.meta, &bytes, "DESKTOP-PC").expect("landed");
        assert_eq!(landed.outcome, Outcome::Pass, "{:?}", landed.fails);
        // Both scratch folders are gone.
        let left: Vec<_> = std::fs::read_dir(data.path().join(SCRATCH))
            .map(|d| d.flatten().collect())
            .unwrap_or_default();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn a_package_that_is_not_a_test_is_refused() {
        let data = tempfile::tempdir().unwrap();
        let git = GitRunner::new().isolated();
        let e = land(
            &git,
            data.path(),
            &json!({ "session": "real-session" }),
            b"x",
            "pc",
        )
        .unwrap_err();
        assert_eq!(e, "That is not a readiness test.");
        let bad = Package {
            meta: json!({}),
            bundle: "!!".into(),
        };
        assert!(unpack(&bad).is_err());
    }
}
