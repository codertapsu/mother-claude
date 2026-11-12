//! A run's work folder (docs/PLAN-MACHINES.md Phase 4, decision 1): a
//! worktree of the target's checkout, set to each snapshot the source
//! sends — tracked and untracked files as the snapshot holds them, the
//! files git ignores kept, the checkout itself never touched, no hook run.

use super::*;
use crate::machines::code::{fetch_snapshot, reset_work_tree, top_of, CodeError};

const WORK: &str = "wk_0123456789abcdef";

/// A snapshot of `from` for the work folder, fetched into `to`'s
/// repository; its commit.
fn send(w: &World, from: &Path, to: &Path, epoch: u64, name: &str) -> String {
    let git = runner();
    let dir = w.dir.join(name);
    let mut req = SnapshotRequest::new(WORK, epoch, "machine-a", dir.join("code"));
    req.haves = haves(&git, to, Some(WORK)).expect("haves").haves;
    snapshot(&git, from, &req).expect("snapshot");
    let f = fetch_snapshot(&git, to, &dir.join("code"), WORK).expect("fetched");
    f.meta.snap
}

fn files_under(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), String::new())];
    while let Some((d, prefix)) = stack.pop() {
        for e in std::fs::read_dir(&d).expect("read dir").flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if prefix.is_empty() && name == ".git" {
                continue;
            }
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            if e.path().is_dir() {
                stack.push((e.path(), rel));
            } else {
                out.push(rel);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn the_work_folder_takes_each_snapshot_and_keeps_what_git_ignores() {
    let w = World::new();
    w.seed_with(&[BASE_FILES]);
    w.clones();
    w.hooks();
    let git = runner();
    let before_dst = w.fx.git(&w.dst, &["status", "--porcelain", "-uall"]);
    let dst_head = w.fx.git(&w.dst, &["rev-parse", "HEAD"]);
    // The session's uncommitted change and a new file go along.
    put(&w.src, "a.txt", "alpha, changed here\n");
    put(&w.src, "new/untracked.txt", "untracked\n");
    let snap1 = send(&w, &w.src, &w.dst, 1, "r1");
    let folder = w
        .dir
        .join("appdata")
        .join("machines")
        .join("work")
        .join(WORK);
    let made = reset_work_tree(&git, &w.dst, &folder, &snap1).expect("reset");
    assert!(made.made, "the worktree is made on first use");
    assert_eq!(
        std::fs::read_to_string(folder.join("a.txt")).unwrap(),
        "alpha, changed here\n"
    );
    assert_eq!(
        std::fs::read_to_string(folder.join("new/untracked.txt")).unwrap(),
        "untracked\n"
    );
    // A build leaves files git ignores; something else leaves an untracked
    // file the next snapshot does not hold.
    put(&folder, "build/out.bin", "cached build output");
    put(&folder, "debug.log", "a log");
    put(&folder, "stray.txt", "not in any snapshot");
    // The session changes again: a file goes, another changes.
    std::fs::remove_file(w.src.join("new/untracked.txt")).unwrap();
    put(&w.src, "b.txt", "bravo, changed\n");
    let snap2 = send(&w, &w.src, &w.dst, 2, "r2");
    let again = reset_work_tree(&git, &w.dst, &folder, &snap2).expect("reset again");
    assert!(!again.made, "the same worktree is kept");
    assert_eq!(
        std::fs::read_to_string(folder.join("build/out.bin")).unwrap(),
        "cached build output",
        "ignored files stay between runs"
    );
    assert!(folder.join("debug.log").is_file(), "an ignored log stays");
    assert!(
        !folder.join("stray.txt").exists(),
        "an untracked file not in the snapshot goes"
    );
    assert!(
        !folder.join("new/untracked.txt").exists(),
        "a file the snapshot no longer holds goes"
    );
    assert_eq!(
        std::fs::read_to_string(folder.join("b.txt")).unwrap(),
        "bravo, changed\n"
    );
    let mut want: Vec<String> =
        w.fx.git(&w.dst, &["ls-tree", "-r", "--name-only", &snap2])
            .lines()
            .map(String::from)
            .collect();
    want.extend(["build/out.bin".to_string(), "debug.log".to_string()]);
    want.sort();
    assert_eq!(
        files_under(&folder),
        want,
        "the snapshot's files, and what git ignores"
    );
    // The user's checkout there: untouched.
    assert_eq!(
        w.fx.git(&w.dst, &["status", "--porcelain", "-uall"]),
        before_dst
    );
    assert_eq!(w.fx.git(&w.dst, &["rev-parse", "HEAD"]), dst_head);
    assert_eq!(
        std::fs::read_to_string(w.dst.join("a.txt")).unwrap(),
        "alpha\n"
    );
    assert!(w.fired().is_empty(), "no hook ran: {:?}", w.fired());
    // The worktree's own top is the folder (a run's `cwd` starts there).
    assert!(crate::claude::paths::same_dir(
        &top_of(&git, &folder).unwrap(),
        &folder
    ));
}

#[test]
fn a_folder_that_is_not_the_work_folder_is_made_again() {
    let w = World::new();
    w.seed_with(&[BASE_FILES]);
    w.clones();
    let git = runner();
    let snap = send(&w, &w.src, &w.dst, 1, "r1");
    let folder = w.dir.join("work").join(WORK);
    // Something else sits there (a copy, a half-made folder).
    put(&folder, "junk.txt", "junk");
    let made = reset_work_tree(&git, &w.dst, &folder, &snap).expect("reset");
    assert!(made.made);
    assert!(!folder.join("junk.txt").exists());
    assert!(folder.join("a.txt").is_file());
    // Gone meanwhile (its admin entry left behind): made again.
    std::fs::remove_dir_all(&folder).unwrap();
    assert!(
        reset_work_tree(&git, &w.dst, &folder, &snap)
            .expect("again")
            .made
    );
    assert!(folder.join("a.txt").is_file());
    // Not a commit id: refused before git sees it.
    assert!(matches!(
        reset_work_tree(&git, &w.dst, &folder, "--help"),
        Err(CodeError::Invalid { .. })
    ));
    // Another work folder's snapshot: refused.
    let dir = w.dir.join("r1").join("code");
    assert!(matches!(
        fetch_snapshot(&git, &w.dst, &dir, "wk_ffffffffffffffff"),
        Err(CodeError::WrongSession)
    ));
}
