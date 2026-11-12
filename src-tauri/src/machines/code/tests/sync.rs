//! Code sync's git (plan §5.15): a landing parked at the last synced state,
//! the reset rule for a rewritten history, and the losing side of a
//! conflict — everything kept under one ref, the checkout put back, or
//! nothing done when something could not come back.

use super::*;
use crate::machines::code::{
    changes, keep_and_put_back, limits_only, prune_snapshots, read_state, root_commits, undo,
    BranchAction, CodeError, PutBack, PutBackError, Reset, TreeState,
};

const SYNC: &str = "sy_0123456789abcdef";

fn state(p: &Path) -> TreeState {
    read_state(&runner(), p).expect("state").state
}

/// A sync step from `from` to `to`, `to` parked at `last`.
fn step(
    w: &World,
    from: &Path,
    to: &Path,
    last: &TreeState,
    reset: Reset,
    name: &str,
) -> Result<Applied, ApplyStop> {
    static EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let epoch = EPOCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let git = runner();
    let dir = w.dir.join(name);
    let mut req = SnapshotRequest::new(SYNC, epoch, "machine-a", &dir);
    req.sync = true;
    req.haves = haves(&git, to, Some(SYNC)).expect("haves").haves;
    snapshot(&git, from, &req).expect("snapshot");
    let mut a = ApplyRequest::new(SYNC, Landing::Checkout);
    a.machine = "machine-b".into();
    a.parked = Some(last.clone());
    a.reset = reset;
    apply(&git, to, &dir, &a)
}

fn kept_tree(w: &World, kept: &str, which: &str) -> String {
    w.fx.git(&w.dst, &["rev-parse", &format!("{kept}{which}^{{tree}}")])
}

fn ls(w: &World, tree: &str) -> Vec<String> {
    w.fx.git(&w.dst, &["ls-tree", "-r", "--name-only", tree])
        .lines()
        .map(String::from)
        .collect()
}

#[test]
fn a_landing_parked_at_the_synced_state_goes_and_one_that_moved_stops() {
    let w = World::new();
    w.seed_with(&[BASE_FILES]);
    w.clones();
    let last = state(&w.dst);
    dirty_mix(&w.fx, &w.src);
    let a = step(&w, &w.src, &w.dst, &last, Reset::Never, "s1").expect("lands");
    assert_eq!(a.action, Some(BranchAction::Same));
    assert_eq!(
        state(&w.dst),
        state(&w.src),
        "the target holds the source's state"
    );
    let report = verify(&runner(), &w.dst, &w.dir.join("s1")).expect("verify");
    assert_eq!(
        report.outcome,
        super::super::Outcome::Pass,
        "{:?}",
        report.fails
    );

    // The target was parked at the step it took: the next one lands too.
    let synced = state(&w.dst);
    put(&w.src, "g.txt", "golf, again\n");
    step(&w, &w.src, &w.dst, &synced, Reset::Never, "s2").expect("the next step lands");
    assert_eq!(
        std::fs::read_to_string(w.dst.join("g.txt")).expect("g.txt"),
        "golf, again\n"
    );

    // An edit on the target meanwhile: it stops, and nothing changes there.
    let synced = state(&w.dst);
    put(&w.dst, "c.txt", "edited on the target\n");
    put(&w.src, "h.txt", "hotel\n");
    let before = probe(&w.fx, &w.dst);
    match step(&w, &w.src, &w.dst, &synced, Reset::Never, "s3") {
        Err(ApplyStop::NotParked { what }) => assert_eq!(what, "files changed"),
        other => panic!("want a stop, got {other:?}"),
    }
    assert_eq!(probe(&w.fx, &w.dst).untouched(), before.untouched());

    // Clean but moved (a commit there): not parked either.
    let w2 = World::new();
    w2.seed_with(&[BASE_FILES]);
    w2.clones();
    let last = state(&w2.dst);
    put(&w2.dst, "a.txt", "committed there\n");
    w2.fx.git(&w2.dst, &["commit", "-qam", "there"]);
    put(&w2.src, "b.txt", "here\n");
    match step(&w2, &w2.src, &w2.dst, &last, Reset::Never, "s1") {
        Err(ApplyStop::NotParked { what }) => assert_eq!(what, "HEAD moved"),
        other => panic!("want a stop, got {other:?}"),
    }
}

#[test]
fn a_rewritten_history_resets_only_the_synced_tip_and_undo_puts_it_back() {
    let w = World::new();
    w.seed_with(&[&[("a.txt", "1\n")], &[("a.txt", "2\n")]]);
    w.clones();
    let last = state(&w.dst);
    // The source rewrites the commit both had (an amend).
    put(&w.src, "a.txt", "2, amended\n");
    w.fx.git(&w.src, &["commit", "-q", "--amend", "-am", "amended"]);
    put(&w.src, "b.txt", "new\n");
    let before = probe(&w.fx, &w.dst);
    let other = Reset::IfTip("0".repeat(40));
    match step(&w, &w.src, &w.dst, &last, other, "s1") {
        Err(ApplyStop::Diverged { branch, .. }) => assert_eq!(branch, "main"),
        e => panic!("want a diverged stop, got {e:?}"),
    }
    assert_eq!(probe(&w.fx, &w.dst).untouched(), before.untouched());
    let tip = last.head.clone().expect("a head");
    let a = step(&w, &w.src, &w.dst, &last, Reset::IfTip(tip.clone()), "s2").expect("lands");
    assert_eq!(a.action, Some(BranchAction::Reset));
    assert_eq!(state(&w.dst), state(&w.src));
    // The old tip is kept (as HEAD and as the branch's), and undo puts it
    // back.
    let undo_ref = a.undo_ref.expect("an undo ref");
    assert_eq!(
        w.fx.git(&w.dst, &["rev-parse", &format!("{undo_ref}/head")]),
        tip
    );
    assert_eq!(
        w.fx.git(&w.dst, &["rev-parse", &format!("{undo_ref}/branch")]),
        tip
    );
    undo(&runner(), &w.dst, &w.dir.join("s2")).expect("undo");
    assert_eq!(probe(&w.fx, &w.dst).restored(), before.restored());
}

#[test]
fn the_losing_side_is_kept_whole_and_put_back() {
    let w = World::new();
    w.seed_with(&[BASE_FILES]);
    w.clones();
    // The synced state: a step that left the target dirty (parked).
    let last0 = state(&w.dst);
    put(&w.src, "a.txt", "alpha, synced\n");
    put(&w.src, "synced-new.txt", "synced, untracked\n");
    step(&w, &w.src, &w.dst, &last0, Reset::Never, "s1").expect("lands");
    let synced = state(&w.dst);
    let synced_files = probe(&w.fx, &w.dst);

    // The target goes on its own way: a commit, then staged, unstaged and
    // untracked changes; an ignored file stays as it is.
    put(&w.dst, "b.txt", "bravo, committed there\n");
    w.fx.git(&w.dst, &["add", "b.txt"]);
    w.fx.git(&w.dst, &["commit", "-q", "-m", "only there"]);
    let own_commit = w.fx.git(&w.dst, &["rev-parse", "HEAD"]);
    put(&w.dst, "c.txt", "charlie, staged\n");
    w.fx.git(&w.dst, &["add", "c.txt"]);
    put(&w.dst, "d.txt", "delta, unstaged\n");
    put(&w.dst, "notes/mine.md", "untracked there\n");
    put(&w.dst, "debug.log", "ignored there\n");
    let own = state(&w.dst);
    let seen = changes(&runner(), &w.dst, Some(&synced), &own).expect("changes");
    assert!(
        seen.head_moved && !seen.branch_switched && seen.files >= 4,
        "{seen:?}"
    );

    let kept = keep_and_put_back(
        &runner(),
        &w.dst,
        &PutBack {
            id: SYNC,
            machine: "machine-b",
            to: Some(&synced),
        },
    )
    .expect("put back");
    // The checkout holds the synced state again, files and all.
    assert_eq!(kept.state, synced);
    assert_eq!(state(&w.dst), synced);
    assert_eq!(probe(&w.fx, &w.dst).restored(), synced_files.restored());
    assert!(!w.dst.join("notes/mine.md").exists());
    assert_eq!(
        std::fs::read_to_string(w.dst.join("debug.log")).expect("the ignored file stays"),
        "ignored there\n"
    );
    // Everything it held is under one ref.
    let r = kept.kept_ref.clone().expect("a kept ref");
    assert!(
        r.starts_with(&format!("refs/mother-claude/sync/{SYNC}/kept-")),
        "{r}"
    );
    assert_eq!(kept_tree(&w, &r, ""), own.wt_tree);
    assert_eq!(kept_tree(&w, &r, "^2"), own.idx_tree);
    assert_eq!(
        w.fx.git(&w.dst, &["rev-parse", &format!("{r}^1")]),
        own_commit
    );
    assert!(
        ls(&w, &r).contains(&"notes/mine.md".to_string()),
        "untracked kept"
    );
    assert!(
        !ls(&w, &r).contains(&"debug.log".to_string()),
        "ignored never"
    );
    assert_eq!(
        w.fx.git(&w.dst, &["show", &format!("{r}:d.txt")]),
        "delta, unstaged"
    );
    assert!(kept.commits_kept && kept.files >= 4, "{kept:?}");

    // Nothing to keep: the checkout holds the state already.
    let again = keep_and_put_back(
        &runner(),
        &w.dst,
        &PutBack {
            id: SYNC,
            machine: "machine-b",
            to: Some(&synced),
        },
    )
    .expect("nothing to do");
    assert!(again.kept_ref.is_none());
}

#[test]
fn a_branch_switch_is_put_back_and_the_branchs_own_commits_kept() {
    let w = World::new();
    w.seed_with(&[BASE_FILES]);
    w.clones();
    let synced = state(&w.dst);
    // A commit on main, then another branch with a commit of its own.
    put(&w.dst, "a.txt", "on main there\n");
    w.fx.git(&w.dst, &["commit", "-qam", "main there"]);
    let main_tip = w.fx.git(&w.dst, &["rev-parse", "HEAD"]);
    w.fx.git(&w.dst, &["switch", "-q", "-c", "feature"]);
    put(&w.dst, "e.txt", "on feature\n");
    w.fx.git(&w.dst, &["commit", "-qam", "feature there"]);
    let feature_tip = w.fx.git(&w.dst, &["rev-parse", "HEAD"]);
    let kept = keep_and_put_back(
        &runner(),
        &w.dst,
        &PutBack {
            id: SYNC,
            machine: "machine-b",
            to: Some(&synced),
        },
    )
    .expect("put back");
    assert_eq!(state(&w.dst), synced);
    assert_eq!(
        w.fx.git(&w.dst, &["symbolic-ref", "HEAD"]),
        "refs/heads/main"
    );
    // main's own commit is the kept commit's third parent; feature stays.
    let r = kept.kept_ref.expect("kept");
    assert_eq!(
        w.fx.git(&w.dst, &["rev-parse", &format!("{r}^1")]),
        feature_tip
    );
    assert_eq!(
        w.fx.git(&w.dst, &["rev-parse", &format!("{r}^3")]),
        main_tip
    );
    assert_eq!(w.fx.git(&w.dst, &["rev-parse", "feature"]), feature_tip);
}

#[test]
fn a_put_back_that_could_lose_something_changes_nothing() {
    let w = World::new();
    w.seed_with(&[BASE_FILES]);
    w.clones();
    // The synced state has a file where the target now keeps an ignored one:
    // `x.log` was tracked when synced, then removed and ignored there.
    let last0 = state(&w.dst);
    put(&w.src, "kept.log", "tracked when synced\n");
    w.fx.git(&w.src, &["add", "-f", "kept.log"]);
    step(&w, &w.src, &w.dst, &last0, Reset::Never, "s1").expect("lands");
    let synced = state(&w.dst);
    w.fx.git(&w.dst, &["rm", "-q", "--cached", "kept.log"]);
    put(&w.dst, "kept.log", "the target's own, ignored now\n");
    put(&w.dst, "a.txt", "changed there\n");
    let before = probe(&w.fx, &w.dst);
    let refs_before =
        w.fx.git(&w.dst, &["for-each-ref", "refs/mother-claude/sync"]);
    let req = PutBack {
        id: SYNC,
        machine: "machine-b",
        to: Some(&synced),
    };
    match keep_and_put_back(&runner(), &w.dst, &req) {
        Err(PutBackError::WouldOverwrite { paths }) => assert_eq!(paths, ["kept.log"]),
        other => panic!("want a refusal, got {other:?}"),
    }
    assert_eq!(probe(&w.fx, &w.dst).untouched(), before.untouched());
    assert_eq!(
        w.fx.git(&w.dst, &["for-each-ref", "refs/mother-claude/sync"]),
        refs_before,
        "nothing kept either"
    );

    // A merge under way: refused.
    std::fs::remove_file(w.dst.join("kept.log")).expect("rm");
    let head = w.fx.git(&w.dst, &["rev-parse", "HEAD"]);
    std::fs::write(w.dst.join(".git").join("MERGE_HEAD"), format!("{head}\n")).expect("merge");
    assert!(matches!(
        keep_and_put_back(&runner(), &w.dst, &req),
        Err(PutBackError::Busy(_))
    ));
    assert!(matches!(
        read_state(&runner(), &w.dst),
        Err(CodeError::InProgress(_))
    ));
    std::fs::remove_file(w.dst.join(".git").join("MERGE_HEAD")).expect("rm");

    // The synced branch is checked out in another worktree: refused.
    w.fx.git(&w.dst, &["switch", "-q", "-c", "elsewhere"]);
    let wt = w.dir.join("wt");
    w.fx.git(&w.dst, &["worktree", "add", "-q", s(&wt), "main"]);
    assert!(matches!(
        keep_and_put_back(&runner(), &w.dst, &req),
        Err(PutBackError::CheckedOutElsewhere { .. })
    ));
    // Something another machine sent that is not an id: refused first.
    let bad = TreeState {
        head: Some("--upload-pack=x".into()),
        ..synced.clone()
    };
    assert!(keep_and_put_back(
        &runner(),
        &w.dst,
        &PutBack {
            id: SYNC,
            machine: "m",
            to: Some(&bad),
        }
    )
    .is_err());
    assert!(keep_and_put_back(
        &runner(),
        &w.dst,
        &PutBack {
            id: "../x",
            machine: "m",
            to: Some(&synced),
        }
    )
    .is_err());
}

#[test]
fn a_first_conflict_puts_the_loser_back_to_its_head() {
    let w = World::new();
    w.seed_with(&[BASE_FILES]);
    w.clones();
    put(&w.dst, "a.txt", "never synced\n");
    put(&w.dst, "own.txt", "untracked\n");
    let kept = keep_and_put_back(
        &runner(),
        &w.dst,
        &PutBack {
            id: SYNC,
            machine: "machine-b",
            to: None,
        },
    )
    .expect("put back");
    let now = read_state(&runner(), &w.dst).expect("state");
    assert_eq!(now.changed, 0, "clean at its HEAD");
    assert!(kept.kept_ref.is_some() && !kept.commits_kept);
}

#[test]
fn two_states_that_differ_only_by_a_limit_are_one() {
    let w = World::new();
    w.seed_with(&[&[("run.sh", "echo hi\n"), ("a.txt", "a\n")]]);
    w.clones();
    let a = state(&w.dst);
    // The same content with the exec bit (Windows cannot hold it).
    w.fx.git(&w.dst, &["update-index", "--chmod=+x", "run.sh"]);
    let b = state(&w.dst);
    assert_ne!(a, b);
    assert!(limits_only(&runner(), &w.dst, &a, &b).expect("limits"));
    put(&w.dst, "a.txt", "content\n");
    let c = state(&w.dst);
    assert!(!limits_only(&runner(), &w.dst, &a, &c).expect("content"));
    assert_eq!(
        root_commits(&runner(), &w.dst).expect("roots"),
        root_commits(&runner(), &w.src).expect("roots")
    );
}

#[test]
fn old_steps_are_pruned_and_the_newest_kept() {
    let w = World::new();
    w.seed_with(&[&[("a.txt", "a\n")]]);
    w.clones();
    let git = runner();
    for epoch in 1..=5u64 {
        put(&w.src, "a.txt", format!("a{epoch}\n"));
        let mut req =
            SnapshotRequest::new(SYNC, epoch, "machine-a", w.dir.join(format!("p{epoch}")));
        req.sync = true;
        snapshot(&git, &w.src, &req).expect("snapshot");
    }
    let refs = |p: &Path| {
        w.fx.git(
            p,
            &[
                "for-each-ref",
                "--format=%(refname)",
                "refs/mother-claude/moves",
            ],
        )
        .lines()
        .count()
    };
    assert_eq!(refs(&w.src), 10);
    assert_eq!(prune_snapshots(&git, &w.src, SYNC, 2).expect("prune"), 3);
    let left = w.fx.git(
        &w.src,
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/mother-claude/moves",
        ],
    );
    assert!(
        left.contains(&format!("{SYNC}/5/")) && left.contains(&format!("{SYNC}/4/")),
        "{left}"
    );
    assert_eq!(refs(&w.src), 4);
    assert_eq!(prune_snapshots(&git, &w.src, SYNC, 2).expect("prune"), 0);
}
