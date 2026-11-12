//! The runner's own promises, long paths (F8), Git LFS (F9) and records
//! that do not hold up.

use super::*;
use crate::machines::code::{CodeError, Outcome, Transfer, META};

#[test]
fn every_operation_gets_a_fresh_private_hooks_folder_kept_empty() {
    let git = runner();
    let op = git.begin().expect("begin");
    let other = git.begin().expect("begin");
    assert_ne!(op.hooks_dir(), other.hooks_dir());
    let dir = op.dir().to_path_buf();
    // Something (Git LFS does) writes a hook into the overriding folder:
    // it is gone before the next call can run it.
    std::fs::write(op.hooks_dir().join("post-checkout"), "#!/bin/sh\nexit 1\n").expect("hook");
    let path = op
        .git(&dir, &["config", "--get", "core.hooksPath"])
        .expect("config");
    assert_eq!(path, s(op.hooks_dir()).replace('\\', "/"));
    assert_eq!(std::fs::read_dir(op.hooks_dir()).expect("hooks").count(), 0);
    assert_eq!(op.intrusions(), vec!["post-checkout".to_string()]);
    drop(op);
    assert!(!dir.exists(), "the operation's folder outlived it");
}

#[test]
fn a_long_path_arrives() {
    // F8: 255 characters below the checkout (well past MAX_PATH with the
    // temp folder), core.longpaths unset in the repositories' own config.
    let w = World::new();
    w.seed_with(&[&[("a.txt", "a\n")]]);
    w.clones();
    let rel = format!("deep/{}file.txt", "abcdefghij/".repeat(22));
    put(&w.src, &rel, "deep\n");
    let own =
        w.fx.op
            .cmd(&w.dst)
            .args(["config", "--local", "--get", "core.longpaths"])
            .ok()
            .expect("git");
    assert!(!own, "the target's own config must not set core.longpaths");
    let r = w.round("ses_long", Transfer::Bundle, Landing::Checkout);
    assert!(r.meta.warning_kinds().contains(&"long-path"));
    assert!(
        r.meta.manifest.iter().any(|e| e.path == rel),
        "the snapshot lost the long path"
    );
    r.assert_pass();
    assert_eq!(
        std::fs::read_to_string(repo::at(&w.dst, &rel)).expect("the long path arrived"),
        "deep\n"
    );
}

#[test]
fn git_lfs_objects_travel_in_the_move_folder() {
    let w = World::new();
    if !w.fx.ok(&w.dir, &["lfs", "version"]) {
        eprintln!("skipped: git-lfs is not installed");
        return;
    }
    let lfs: &[(&str, &str)] = &[
        ("filter.lfs.clean", "git-lfs clean -- %f"),
        ("filter.lfs.smudge", "git-lfs smudge -- %f"),
        ("filter.lfs.process", "git-lfs filter-process"),
        ("filter.lfs.required", "true"),
    ];
    w.init_origin();
    for (k, v) in lfs {
        w.fx.git(&w.seed, &["config", k, v]);
    }
    put(&w.seed, "a.txt", "a\n");
    w.fx.git(&w.seed, &["lfs", "track", "*.bin"]);
    put(&w.seed, "big.bin", noise(11, 100_000));
    w.commit_seed("lfs");
    w.push_seed();
    w.fx.git(&w.seed, &["lfs", "push", "--all", "origin"]);
    w.clone_to(&w.src, lfs);
    w.clone_to(&w.dst, lfs);
    put(&w.src, "big.bin", noise(12, 100_000));
    put(&w.src, "new.bin", noise(13, 50_000));
    let r = w.round("ses_lfs", Transfer::Bundle, Landing::Checkout);
    assert!(r.meta.warning_kinds().contains(&"lfs"));
    let carried = r.meta.lfs.as_ref().expect("lfs").carried.len();
    assert_eq!(carried, 2, "{:?}", r.meta.lfs);
    assert_eq!(r.applied().lfs_installed, 2);
    r.assert_pass();
    assert_eq!(
        std::fs::read(w.dst.join("new.bin")).expect("new.bin"),
        noise(13, 50_000)
    );
}

#[test]
fn a_snapshot_waits_for_a_merge_in_progress() {
    let w = World::new();
    w.seed_with(&[&[("a.txt", "a\n")]]);
    w.clones();
    let head = w.fx.git(&w.src, &["rev-parse", "HEAD"]);
    std::fs::write(w.src.join(".git").join("MERGE_HEAD"), format!("{head}\n")).expect("MERGE_HEAD");
    let req = SnapshotRequest::new("ses_busy", 1, "machine-a", w.dir.join("move"));
    match snapshot(&runner(), &w.src, &req) {
        Err(CodeError::InProgress(what)) => assert_eq!(what, "MERGE_HEAD"),
        other => panic!("want InProgress, got {other:?}"),
    }
}

#[test]
fn a_record_that_points_outside_the_checkout_is_refused() {
    let w = World::new();
    w.seed_with(&[&[("a.txt", "a\n")]]);
    w.clones();
    put(&w.src, "new.txt", "n\n");
    let git = runner();
    let dir = w.dir.join("move");
    let req = SnapshotRequest::new("ses_evil", 1, "machine-a", &dir);
    snapshot(&git, &w.src, &req).expect("snapshot");
    let meta_path = dir.join(META);
    let mut meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&meta_path).expect("meta")).expect("json");
    meta["manifest"][0]["path"] = "../../outside.txt".into();
    std::fs::write(&meta_path, serde_json::to_vec(&meta).expect("json")).expect("write");
    let before = probe(&w.fx, &w.dst);
    let result = apply(
        &git,
        &w.dst,
        &dir,
        &ApplyRequest::new("ses_evil", Landing::Checkout),
    );
    assert!(
        matches!(result, Err(ApplyStop::Failed(CodeError::BadMove(_)))),
        "{result:?}"
    );
    assert_eq!(probe(&w.fx, &w.dst).untouched(), before.untouched());
    // A record for another session is refused too.
    std::fs::write(
        &meta_path,
        serde_json::to_vec(&{
            let mut m = meta.clone();
            m["manifest"][0]["path"] = "a.txt".into();
            m
        })
        .expect("json"),
    )
    .expect("write");
    let other = apply(
        &git,
        &w.dst,
        &dir,
        &ApplyRequest::new("ses_other", Landing::Checkout),
    );
    assert!(
        matches!(other, Err(ApplyStop::Failed(CodeError::WrongSession))),
        "{other:?}"
    );
}

#[test]
fn a_file_the_target_ignores_is_never_overwritten() {
    // T looks clean, but it ignores dist/ and has its own dist/app.js and a
    // file `out` where S has a folder; `read-tree -u` would overwrite both.
    let w = World::new();
    w.seed_with(&[&[("a.txt", "1\n")]]);
    w.clones();
    put(&w.src, "a.txt", "2\n");
    w.fx.git(&w.src, &["commit", "-q", "-am", "c2 on S"]);
    put(&w.src, "dist/app.js", "from S\n");
    put(&w.src, "out/report.txt", "from S\n");
    put(&w.dst.join(".git").join("info"), "exclude", "dist/\nout\n");
    put(&w.dst, "dist/app.js", "T's own build\n");
    put(&w.dst, "out", "T's own file\n");
    let before = probe(&w.fx, &w.dst);
    let r = w.round("ses_ignored", Transfer::Bundle, Landing::Checkout);
    match &r.applied {
        Err(stop @ ApplyStop::WouldOverwrite { paths }) => {
            assert_eq!(paths, &["dist/app.js", "out/report.txt"]);
            assert_eq!(stop.offer(), Some(crate::machines::code::Offer::Worktree));
        }
        other => panic!("want WouldOverwrite, got {other:?}"),
    }
    assert_eq!(
        std::fs::read_to_string(w.dst.join("dist").join("app.js")).expect("app.js"),
        "T's own build\n"
    );
    assert_eq!(probe(&w.fx, &w.dst).untouched(), before.untouched());
    // A worktree holds the move without touching them.
    let mut req = ApplyRequest::new(
        "ses_ignored",
        Landing::Worktree {
            root: w.worktrees(),
        },
    );
    req.machine = "machine-b".into();
    let a = apply(&runner(), &w.dst, &r.dir, &req).expect("worktree landing");
    for rel in ["dist/app.js", "out/report.txt"] {
        let got = std::fs::read_to_string(repo::at(&a.path, rel)).expect("arrived");
        assert_eq!(got, "from S\n", "{rel}");
    }
    assert_eq!(
        std::fs::read_to_string(w.dst.join("out")).expect("out"),
        "T's own file\n"
    );
}

#[test]
fn a_file_only_the_target_ignores_is_a_limit_of_the_proof_not_a_failure() {
    // S carries untracked files T's own rules ignore (its info/exclude; the
    // .gitignore is the snapshot's): they land byte for byte, but `git add
    // -A` and `git status` on T don't see them.
    let w = World::new();
    w.seed_with(&[&[("a.txt", "1\n")]]);
    w.clones();
    put(&w.src, "a.txt", "2\n");
    put(&w.src, "notes/todo.md", "from S\n");
    put(&w.src, "scratch.tmp", "S's scratch\n");
    put(
        &w.dst.join(".git").join("info"),
        "exclude",
        "notes/\n*.tmp\n",
    );
    let r = w.round("ses_ignored_there", Transfer::Bundle, Landing::Checkout);
    let a = r.applied();
    for (rel, want) in [
        ("notes/todo.md", "from S\n"),
        ("scratch.tmp", "S's scratch\n"),
    ] {
        let got = std::fs::read_to_string(repo::at(&a.path, rel)).expect("arrived");
        assert_eq!(got, want, "{rel}");
    }
    let report = r.report();
    assert_eq!(
        report.outcome,
        Outcome::Limit,
        "fails {:?}; limits {:?}",
        report.fails,
        report.limits
    );
    assert_eq!(report.limits, vec!["ignored-there".to_string()]);
    assert!(report.fails.is_empty(), "{:?}", report.fails);
    let wt = report
        .checks
        .iter()
        .find(|c| c.check == "working tree = the snapshot tree")
        .expect("the working tree's check");
    assert!(
        wt.details
            .contains(&"ignored-there notes/todo.md".to_string()),
        "{:?}",
        wt.details
    );
    // Ignored or not, a file that came out different, or went missing, is
    // still a failure.
    put(&w.dst, "notes/todo.md", "changed on T\n");
    std::fs::remove_file(w.dst.join("scratch.tmp")).expect("rm");
    let again = verify(&runner(), &w.dst, &r.dir).expect("verify");
    assert_eq!(again.outcome, Outcome::Fail);
    for rel in ["notes/todo.md", "scratch.tmp"] {
        assert!(
            again.fails.iter().any(|f| f.contains(rel)),
            "{rel}: {:?}",
            again.fails
        );
    }
    // An untracked file T does not ignore, gone from T: a failure too.
    let w = World::new();
    w.seed_with(&[&[("a.txt", "1\n")]]);
    w.clones();
    put(&w.src, "notes/todo.md", "from S\n");
    let r = w.round("ses_not_ignored", Transfer::Bundle, Landing::Checkout);
    r.assert_pass();
    std::fs::remove_file(w.dst.join("notes").join("todo.md")).expect("rm");
    let gone = verify(&runner(), &w.dst, &r.dir).expect("verify");
    assert_eq!(gone.outcome, Outcome::Fail, "{:?}", gone.limits);
    assert!(gone.limits.is_empty(), "{:?}", gone.limits);
}

#[test]
fn verify_reports_a_changed_file_as_a_failure() {
    let w = World::new();
    w.seed_with(&[&[("a.txt", "a\n")]]);
    w.clones();
    put(&w.src, "a.txt", "a2\n");
    let r = w.round("ses_verify", Transfer::Bundle, Landing::Checkout);
    r.assert_pass();
    put(&w.dst, "a.txt", "tampered\n");
    let report = verify(&runner(), &w.dst, &r.dir).expect("verify");
    assert_eq!(report.outcome, Outcome::Fail);
    assert!(
        report.fails.iter().any(|f| f.contains("a.txt")),
        "{:?}",
        report.fails
    );
    // Undo refuses to throw that edit away.
    match crate::machines::code::undo(&runner(), &w.dst, &r.dir) {
        Err(CodeError::ChangedSinceApply(_)) => {}
        other => panic!("want ChangedSinceApply, got {other:?}"),
    }
}
