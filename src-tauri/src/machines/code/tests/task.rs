//! A task handed to another computer (docs/PLAN-MACHINES.md Phase 4,
//! decision 6): its worktree at the handed-over snapshot, what its session
//! did coming back as a branch on the asking computer — moved only
//! forward, never checked out, never over someone's work — and nothing
//! when it changed nothing.

use super::*;
use crate::machines::code::{
    drop_task_refs, fetch_snapshot, land_task_branch, remove_task_tree, reset_work_tree,
    task_branch, task_result, TaskBranchError, TaskResult, TaskResultRequest, TaskTip,
};

const TASK: &str = "task_0123456789abcdef";

/// The asking computer's snapshot of `from`, fetched into `to`: the
/// handed-over snapshot's record.
fn hand_over(w: &World, from: &Path, to: &Path) -> SnapshotMeta {
    let git = runner();
    let dir = w.dir.join("handed").join("code");
    let mut req = SnapshotRequest::new(TASK, 1, "machine-a", &dir);
    req.haves = haves(&git, to, Some(TASK)).expect("haves").haves;
    snapshot(&git, from, &req).expect("snapshot");
    fetch_snapshot(&git, to, &dir, TASK).expect("fetched").meta
}

/// The task's result in `folder`, its record and bundle in `<dir>/<name>`.
fn result(
    w: &World,
    folder: &Path,
    handed: &SnapshotMeta,
    last: Option<&TaskTip>,
    epoch: u64,
) -> (TaskResult, PathBuf) {
    let git = runner();
    let out = w.dir.join(format!("result-{epoch}")).join("code");
    let req = TaskResultRequest {
        task: TASK.into(),
        epoch,
        machine: "machine-b".into(),
        out_dir: out.clone(),
        base: handed.snap.clone(),
        base_tree: handed.wt_tree.clone(),
        last: last.cloned(),
        haves: haves(&git, &w.src, Some(TASK)).expect("haves").haves,
    };
    (task_result(&git, folder, &req).expect("result"), out)
}

fn made(r: TaskResult) -> (SnapshotMeta, TaskTip) {
    match r {
        TaskResult::Made { meta, tip } => (*meta, tip),
        TaskResult::Unchanged => panic!("it changed something"),
    }
}

fn branch() -> String {
    task_branch("machine-b", TASK).expect("a branch name")
}

#[test]
fn a_task_s_work_comes_back_as_a_branch_that_only_moves_forward() {
    let w = World::new();
    w.seed_with(&[BASE_FILES]);
    w.clones();
    w.hooks();
    let git = runner();
    // The asking session's uncommitted change and a new file go along.
    put(&w.src, "a.txt", "alpha, changed by the asking session\n");
    put(&w.src, "notes/new.txt", "untracked there\n");
    let src_before = probe(&w.fx, &w.src);
    let dst_before = probe(&w.fx, &w.dst);
    let handed = hand_over(&w, &w.src, &w.dst);
    let folder = w
        .dir
        .join("appdata")
        .join("machines")
        .join("work")
        .join("tasks")
        .join(TASK)
        .join("app");
    reset_work_tree(&git, &w.dst, &folder, &handed.snap).expect("the task's worktree");
    assert_eq!(
        std::fs::read_to_string(folder.join("a.txt")).unwrap(),
        "alpha, changed by the asking session\n",
        "the worktree holds the asking session's code"
    );
    assert!(folder.join("notes/new.txt").is_file());
    // Before the session did anything: nothing to bring back.
    assert!(matches!(
        result(&w, &folder, &handed, None, 10).0,
        TaskResult::Unchanged
    ));
    // The session commits a change and leaves another uncommitted.
    put(&folder, "c.txt", "charlie, committed by the task\n");
    w.fx.git(&folder, &["add", "c.txt"]);
    w.fx.git(&folder, &["commit", "-q", "-m", "the task's commit"]);
    put(&folder, "d.txt", "delta, left uncommitted\n");
    put(&folder, "made/by-task.txt", "a new file\n");
    let (r, out1) = result(&w, &folder, &handed, None, 11);
    let (meta1, tip1) = made(r);
    assert_eq!(
        tip1.tip, tip1.snap,
        "uncommitted work: the snapshot is the tip"
    );
    assert_eq!(meta1.base, tip1.head);
    let b = branch();
    let (_, landed) =
        land_task_branch(&git, &w.src, &out1, TASK, &b, &tip1.tip, &handed.snap).expect("landed");
    assert!(landed.created && landed.moved);
    assert_eq!(landed.commits, 1);
    assert!(landed.uncommitted);
    assert_eq!(landed.files, 3, "c.txt, d.txt, made/by-task.txt");
    let show = |rev: &str, path: &str| w.fx.git(&w.src, &["show", &format!("{rev}:{path}")]);
    assert_eq!(show(&b, "c.txt"), "charlie, committed by the task");
    assert_eq!(show(&b, "d.txt"), "delta, left uncommitted");
    assert_eq!(show(&b, "made/by-task.txt"), "a new file");
    assert_eq!(
        show(&b, "a.txt"),
        "alpha, changed by the asking session",
        "on top of the handed-over code"
    );
    // The asking checkout: never checked out, its files and HEAD as they were.
    let mut src_now = probe(&w.fx, &w.src);
    src_now.refs.retain(|r| !r.starts_with(&b));
    assert_eq!(src_now.untouched(), src_before.untouched());
    // The second turn: it commits what it left and more; the branch moves
    // forward from the first result.
    w.fx.git(&folder, &["add", "-A"]);
    w.fx.git(&folder, &["commit", "-q", "-m", "the rest"]);
    put(&folder, "e.txt", "echo, second turn\n");
    w.fx.git(&folder, &["commit", "-q", "-am", "second turn"]);
    let (r, out2) = result(&w, &folder, &handed, Some(&tip1), 12);
    let (meta2, tip2) = made(r);
    assert_eq!(
        meta2.chained,
        vec![tip1.snap.clone()],
        "it descends from the last result"
    );
    let (_, landed) =
        land_task_branch(&git, &w.src, &out2, TASK, &b, &tip2.tip, &handed.snap).expect("again");
    assert!(!landed.created && landed.moved);
    assert!(!landed.uncommitted, "everything is committed now");
    assert_eq!(landed.commits, 3);
    assert!(
        w.fx.ok(&w.src, &["merge-base", "--is-ancestor", &tip1.tip, &b]),
        "a fast-forward: the first result stays in the branch's history"
    );
    assert_eq!(show(&b, "e.txt"), "echo, second turn");
    // A turn that changed nothing: no result, the branch stays.
    assert!(matches!(
        result(&w, &folder, &handed, Some(&tip2), 13).0,
        TaskResult::Unchanged
    ));
    // The third turn commits cleanly on top: the tip is that commit itself.
    put(&folder, "b.txt", "bravo, third turn\n");
    w.fx.git(&folder, &["commit", "-q", "-am", "third turn"]);
    let (r, out3) = result(&w, &folder, &handed, Some(&tip2), 14);
    let (_, tip3) = made(r);
    let head3 = w.fx.git(&folder, &["rev-parse", "HEAD"]);
    // tip2 was a snapshot commit (not in HEAD's history): the snapshot again.
    assert_eq!(tip3.tip, tip3.snap);
    land_task_branch(&git, &w.src, &out3, TASK, &b, &tip3.tip, &handed.snap).expect("third");
    assert!(w
        .fx
        .ok(&w.src, &["merge-base", "--is-ancestor", &head3, &b]));
    // The target's checkout: untouched, but for our refs and the worktree.
    let dst_now = probe(&w.fx, &w.dst);
    assert_eq!(dst_now.checkout(), dst_before.checkout());
    assert!(w.fired().is_empty(), "no hook ran: {:?}", w.fired());
    // Removed: the worktree and the task's refs there; the branch here stays.
    remove_task_tree(&git, &w.dst, &folder, TASK).expect("removed");
    assert!(!folder.exists());
    let listed = w.fx.git(&w.dst, &["worktree", "list", "--porcelain"]);
    assert!(!listed.contains(TASK), "{listed}");
    let refs =
        w.fx.git(&w.dst, &["for-each-ref", "refs/mother-claude/moves/"]);
    assert!(!refs.contains(TASK), "{refs}");
    assert!(drop_task_refs(&git, &w.src, TASK).expect("dropped") > 0);
    let refs =
        w.fx.git(&w.src, &["for-each-ref", "refs/mother-claude/moves/"]);
    assert!(!refs.contains(TASK), "{refs}");
    assert_eq!(show(&b, "e.txt"), "echo, second turn", "the branch stays");
}

#[test]
fn a_session_that_commits_everything_gives_its_own_commit() {
    let w = World::new();
    w.seed_with(&[BASE_FILES]);
    w.clones();
    let git = runner();
    let handed = hand_over(&w, &w.src, &w.dst);
    let folder = w.dir.join("tasks").join(TASK).join("app");
    reset_work_tree(&git, &w.dst, &folder, &handed.snap).expect("worktree");
    put(&folder, "a.txt", "alpha, fixed\n");
    w.fx.git(&folder, &["commit", "-q", "-am", "fix"]);
    let head = w.fx.git(&folder, &["rev-parse", "HEAD"]);
    let (r, out) = result(&w, &folder, &handed, None, 2);
    let (_, tip) = made(r);
    assert_eq!(tip.tip, head, "no synthetic commit on a clean worktree");
    let b = branch();
    let (_, landed) =
        land_task_branch(&git, &w.src, &out, TASK, &b, &tip.tip, &handed.snap).expect("landed");
    assert_eq!(
        (landed.commits, landed.files, landed.uncommitted),
        (1, 1, false)
    );
    assert_eq!(w.fx.git(&w.src, &["rev-parse", &b]), head);
    // The same result again: nothing moves.
    let (_, again) =
        land_task_branch(&git, &w.src, &out, TASK, &b, &tip.tip, &handed.snap).expect("again");
    assert!(!again.moved);
}

#[test]
fn a_branch_someone_works_on_here_is_never_moved() {
    let w = World::new();
    w.seed_with(&[BASE_FILES]);
    w.clones();
    let git = runner();
    let handed = hand_over(&w, &w.src, &w.dst);
    let folder = w.dir.join("tasks").join(TASK).join("app");
    reset_work_tree(&git, &w.dst, &folder, &handed.snap).expect("worktree");
    put(&folder, "a.txt", "alpha, first\n");
    let (r, out1) = result(&w, &folder, &handed, None, 2);
    let (_, tip1) = made(r);
    let b = branch();
    land_task_branch(&git, &w.src, &out1, TASK, &b, &tip1.tip, &handed.snap).expect("landed");
    // Someone checks the branch out here in a worktree: a later result
    // does not move it under them.
    let wt = w.dir.join("review");
    w.fx.git(&w.src, &["worktree", "add", "-q", s(&wt), short(&b)]);
    put(&folder, "b.txt", "bravo, second\n");
    let (r, out2) = result(&w, &folder, &handed, Some(&tip1), 3);
    let (_, tip2) = made(r);
    match land_task_branch(&git, &w.src, &out2, TASK, &b, &tip2.tip, &handed.snap) {
        Err(TaskBranchError::CheckedOut { .. }) => {}
        other => panic!("{other:?}"),
    }
    assert_eq!(w.fx.git(&w.src, &["rev-parse", &b]), tip1.tip);
    // They commit on it and let go of it: still not moved (it would drop
    // their commit), though the result itself is here.
    put(&wt, "mine.txt", "my own work\n");
    w.fx.git(&wt, &["add", "mine.txt"]);
    w.fx.git(&wt, &["commit", "-q", "-m", "mine"]);
    w.fx.git(&w.src, &["worktree", "remove", "--force", s(&wt)]);
    match land_task_branch(&git, &w.src, &out2, TASK, &b, &tip2.tip, &handed.snap) {
        Err(TaskBranchError::Diverged { .. }) => {}
        other => panic!("{other:?}"),
    }
    assert!(w.fx.ok(
        &w.src,
        &["cat-file", "-e", &format!("{}^{{commit}}", tip2.tip)]
    ));
    // A tip that is not the result's: refused.
    let other = w.fx.git(&w.src, &["rev-parse", "HEAD"]);
    assert!(land_task_branch(&git, &w.src, &out2, TASK, &b, &other, &handed.snap).is_err());
    // A branch outside ours: refused before git sees it.
    assert!(land_task_branch(
        &git,
        &w.src,
        &out2,
        TASK,
        "refs/heads/main",
        &tip2.tip,
        &handed.snap
    )
    .is_err());
}

fn short(b: &str) -> &str {
    b.strip_prefix("refs/heads/").unwrap_or(b)
}
