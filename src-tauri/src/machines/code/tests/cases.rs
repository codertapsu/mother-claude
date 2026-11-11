//! The round trips: every kind of change × both transfers × both landings,
//! then the branch rules, parking and chaining.

use super::*;
use crate::machines::code::{
    undo, BranchAction, LandingKind, Outcome, SubmoduleState, Transfer, BUNDLE,
};

fn mix(transfer: Transfer, worktree: bool) {
    let w = World::new();
    w.seed_with(&[BASE_FILES]);
    w.clones();
    w.hooks();
    dirty_mix(&w.fx, &w.src);
    let src_before = probe(&w.fx, &w.src);
    let dst_before = probe(&w.fx, &w.dst);
    let landing = if worktree {
        Landing::Worktree {
            root: w.worktrees(),
        }
    } else {
        Landing::Checkout
    };
    let r = w.round("ses_mix", transfer.clone(), landing);
    assert_eq!(
        probe(&w.fx, &w.src).untouched(),
        src_before.untouched(),
        "the snapshot changed the source"
    );
    r.assert_pass();
    let a = r.applied();
    for ignored in ["debug.log", "build/out.bin"] {
        assert!(!a.path.join(ignored).exists(), "{ignored} travelled");
    }
    assert_eq!(
        std::fs::read_to_string(a.path.join("b.txt")).expect("b.txt"),
        "bravo 3\n"
    );
    if transfer != Transfer::Bundle {
        let left =
            w.fx.git(&w.dir, &["ls-remote", s(&w.origin), "refs/mother-claude/*"]);
        assert!(left.is_empty(), "hidden refs left on the remote: {left}");
    }
    if worktree {
        assert_eq!(a.landing, LandingKind::Worktree);
        assert_eq!(
            probe(&w.fx, &w.dst).checkout(),
            dst_before.checkout(),
            "a worktree landing touched the checkout"
        );
    } else {
        assert_eq!(a.action, Some(BranchAction::Same));
    }
    let undone = undo(&runner(), &w.dst, &r.dir).expect("undo");
    assert!(undone.kept.is_empty(), "kept {:?}", undone.kept);
    assert_eq!(
        probe(&w.fx, &w.dst).restored(),
        dst_before.restored(),
        "undo did not restore the target"
    );
    assert!(w.fired().is_empty(), "hooks ran: {:?}", w.fired());
    // The markers are live: with the repository's own hooks, a commit
    // leaves one.
    let own = s(&w.dst.join(".git").join("hooks")).replace('\\', "/");
    let own = format!("core.hooksPath={own}");
    w.fx.git(
        &w.dst,
        &["-c", &own, "commit", "-q", "--allow-empty", "-m", "markers"],
    );
    assert!(
        w.fired().iter().any(|h| h == "pre-commit"),
        "the marker hooks never fire: {:?}",
        w.fired()
    );
}

#[test]
fn mix_bundle_checkout() {
    mix(Transfer::Bundle, false);
}

#[test]
fn mix_bundle_worktree() {
    mix(Transfer::Bundle, true);
}

#[test]
fn mix_remote_checkout() {
    mix(remote(), false);
}

#[test]
fn mix_remote_worktree() {
    mix(remote(), true);
}

#[test]
fn binary_files_travel_byte_for_byte() {
    let w = World::new();
    w.init_origin();
    put(&w.seed, "img.bin", noise(1, 4096));
    put(&w.seed, "a.txt", "a\n");
    w.commit_seed("binary");
    w.push_seed();
    w.clones();
    put(&w.src, "img.bin", noise(2, 4096));
    put(&w.src, "new.bin", noise(3, 8192));
    let r = w.round("ses_bin", Transfer::Bundle, Landing::Checkout);
    r.assert_pass();
    assert_eq!(
        std::fs::read(w.dst.join("new.bin")).expect("new.bin"),
        noise(3, 8192)
    );
}

#[test]
fn autocrlf_false_to_true() {
    let w = World::new();
    w.seed_with(&[&[("a.txt", "one\ntwo\n"), ("src/b.js", "let x = 1;\n")]]);
    w.clone_to(&w.src, &[]);
    w.clone_to(&w.dst, &[("core.autocrlf", "true")]);
    put(&w.src, "a.txt", "one\ntwo\nthree\n");
    put(&w.src, "c.md", "# c\n");
    put(&w.src, "src/b.js", "let x = 2;\n");
    w.fx.git(&w.src, &["add", "src/b.js"]);
    w.round("ses_crlf1", Transfer::Bundle, Landing::Checkout)
        .assert_pass();
}

#[test]
fn autocrlf_true_to_false() {
    let w = World::new();
    w.seed_with(&[&[("a.txt", "one\ntwo\n"), ("lf-edit.txt", "x\n")]]);
    w.clone_to(&w.src, &[("core.autocrlf", "true")]);
    w.clone_to(&w.dst, &[]);
    put(&w.src, "a.txt", "one\r\ntwo\r\nthree\r\n");
    put(&w.src, "c.md", "# c\r\n");
    put(&w.src, "lf-edit.txt", "x\ny\n");
    let r = w.round(
        "ses_crlf2",
        Transfer::Bundle,
        Landing::Worktree {
            root: w.worktrees(),
        },
    );
    r.assert_pass();
}

#[test]
fn eol_lf_attribute_keeps_lf_everywhere() {
    let w = World::new();
    w.seed_with(&[&[
        (".gitattributes", "* text=auto eol=lf\n"),
        ("a.txt", "one\n"),
    ]]);
    w.clone_to(&w.src, &[]);
    w.clone_to(&w.dst, &[("core.autocrlf", "true")]);
    put(&w.src, "a.txt", "one\ntwo\n");
    put(&w.src, "b.txt", "b\n");
    let r = w.round("ses_eol", Transfer::Bundle, Landing::Checkout);
    r.assert_pass();
    assert!(
        r.report().raw_kinds.is_empty(),
        "{:?}",
        r.report().raw_kinds
    );
}

#[test]
fn a_target_behind_fast_forwards_and_undo_puts_it_back() {
    let w = World::new();
    w.seed_with(&[&[("a.txt", "1\n")]]);
    w.clones();
    put(&w.src, "a.txt", "2\n");
    put(&w.src, "b.txt", "b\n");
    w.fx.git(&w.src, &["add", "-A"]);
    w.fx.git(&w.src, &["commit", "-q", "-m", "c2 on S"]);
    put(&w.src, "a.txt", "3\n");
    put(&w.src, "c.txt", "c\n");
    let before = probe(&w.fx, &w.dst);
    let r = w.round("ses_behind", Transfer::Bundle, Landing::Checkout);
    r.assert_pass();
    assert_eq!(r.applied().action, Some(BranchAction::FastForward));
    undo(&runner(), &w.dst, &r.dir).expect("undo");
    assert_eq!(probe(&w.fx, &w.dst).restored(), before.restored());
}

#[test]
fn a_diverged_target_stops_and_a_worktree_lands() {
    let w = World::new();
    w.seed_with(&[&[("a.txt", "1\n")]]);
    w.clones();
    put(&w.dst, "t.txt", "t\n");
    w.fx.git(&w.dst, &["add", "-A"]);
    w.fx.git(&w.dst, &["commit", "-q", "-m", "only on T"]);
    put(&w.src, "s.txt", "s\n");
    w.fx.git(&w.src, &["add", "-A"]);
    w.fx.git(&w.src, &["commit", "-q", "-m", "only on S"]);
    put(&w.src, "a.txt", "2\n");
    let before = probe(&w.fx, &w.dst);
    let r = w.round("ses_div", Transfer::Bundle, Landing::Checkout);
    match &r.applied {
        Err(ApplyStop::Diverged { branch, .. }) => assert_eq!(branch, "main"),
        other => panic!("want a diverged stop, got {other:?}"),
    }
    assert_eq!(
        r.applied.as_ref().err().and_then(ApplyStop::offer),
        Some(crate::machines::code::Offer::WorktreeOrReset)
    );
    assert_eq!(probe(&w.fx, &w.dst).untouched(), before.untouched());
    // The same move, into a worktree.
    let mut req = ApplyRequest::new(
        "ses_div",
        Landing::Worktree {
            root: w.worktrees(),
        },
    );
    req.machine = "machine-b".into();
    let a = apply(&runner(), &w.dst, &r.dir, &req).expect("worktree landing");
    let report = verify(&runner(), &a.path, &r.dir).expect("verify");
    assert_eq!(report.outcome, Outcome::Pass, "{:?}", report.fails);
    assert_eq!(probe(&w.fx, &w.dst).checkout(), before.checkout());
}

#[test]
fn a_dirty_target_stops_and_a_worktree_lands() {
    let w = World::new();
    w.seed_with(&[&[("a.txt", "1\n"), ("b.txt", "b\n")]]);
    w.clones();
    put(&w.dst, "b.txt", "changed on T\n");
    put(&w.src, "a.txt", "2\n");
    let before = probe(&w.fx, &w.dst);
    let r = w.round("ses_dirty", Transfer::Bundle, Landing::Checkout);
    assert!(
        matches!(r.applied, Err(ApplyStop::Dirty { changed: 1 })),
        "{:?}",
        r.applied
    );
    assert_eq!(probe(&w.fx, &w.dst).untouched(), before.untouched());
    let r = w.move_code(
        &w.src,
        &w.dst,
        "ses_dirty",
        2,
        Transfer::Bundle,
        Landing::Worktree {
            root: w.worktrees(),
        },
        "move-2",
    );
    r.assert_pass();
    assert_eq!(
        std::fs::read_to_string(w.dst.join("b.txt")).expect("b.txt"),
        "changed on T\n"
    );
}

fn unborn(worktree: bool) {
    let w = World::new();
    for r in [&w.src, &w.dst] {
        w.fx.git(&w.dir, &["init", "-q", s(r)]);
    }
    put(&w.src, "a.txt", "staged\n");
    w.fx.git(&w.src, &["add", "a.txt"]);
    put(&w.src, "a.txt", "staged then changed\n");
    put(&w.src, "b.txt", "untracked\n");
    let before = probe(&w.fx, &w.dst);
    let landing = if worktree {
        Landing::Worktree {
            root: w.worktrees(),
        }
    } else {
        Landing::Checkout
    };
    let r = w.round("ses_unborn", Transfer::Bundle, landing);
    assert!(r.meta.base.is_none());
    assert!(r.meta.warning_kinds().contains(&"unborn"));
    r.assert_pass();
    if !worktree {
        assert_eq!(r.applied().action, Some(BranchAction::Unborn));
    }
    undo(&runner(), &w.dst, &r.dir).expect("undo");
    assert_eq!(probe(&w.fx, &w.dst).restored(), before.restored());
}

#[test]
fn an_unborn_branch_lands_in_the_checkout() {
    unborn(false);
}

#[test]
fn an_unborn_branch_lands_in_a_worktree() {
    unborn(true);
}

#[test]
fn a_detached_head_lands_detached() {
    let w = World::new();
    w.seed_with(&[&[("a.txt", "1\n")], &[("a.txt", "2\n")]]);
    w.clones();
    w.fx.git(&w.src, &["switch", "-q", "--detach", "HEAD~1"]);
    put(&w.src, "a.txt", "on detached\n");
    put(&w.src, "x.txt", "x\n");
    let before = probe(&w.fx, &w.dst);
    let r = w.round("ses_detached", Transfer::Bundle, Landing::Checkout);
    r.assert_pass();
    assert_eq!(r.applied().action, Some(BranchAction::Detach));
    undo(&runner(), &w.dst, &r.dir).expect("undo");
    assert_eq!(probe(&w.fx, &w.dst).restored(), before.restored());
}

/// S → T, the session works on T (edits, a commit), then T → S.
fn there_and_back(edit_source: bool) {
    let w = World::new();
    w.seed_with(&[BASE_FILES]);
    w.clones();
    dirty_mix(&w.fx, &w.src);
    let first = w.round("ses_trip", Transfer::Bundle, Landing::Checkout);
    first.assert_pass();
    let parked = probe(&w.fx, &w.src);
    put(&w.dst, "g.txt", "golf, continued on T\n");
    put(&w.dst, "c.txt", "charlie from T\n");
    w.fx.git(&w.dst, &["add", "c.txt"]);
    w.fx.git(&w.dst, &["commit", "-q", "-m", "work done on T"]);
    put(&w.dst, "h.txt", "hotel, new on T\n");
    put(&w.dst, "a.txt", "alpha from T, unstaged\n");
    if edit_source {
        put(&w.src, "g.txt", "golf, edited on S meanwhile\n");
    }
    let back = w.move_code(
        &w.dst,
        &w.src,
        "ses_trip",
        2,
        Transfer::Bundle,
        Landing::Checkout,
        "move-back",
    );
    if edit_source {
        assert!(
            matches!(back.applied, Err(ApplyStop::Dirty { .. })),
            "{:?}",
            back.applied
        );
        assert_eq!(
            std::fs::read_to_string(w.src.join("g.txt")).expect("g.txt"),
            "golf, edited on S meanwhile\n"
        );
        return;
    }
    back.assert_pass();
    let a = back.applied();
    assert_eq!(a.parked.as_deref(), Some("1"));
    assert_eq!(a.action, Some(BranchAction::FastForward));
    // Undo of the way back: the source is parked again.
    undo(&runner(), &w.src, &back.dir).expect("undo");
    assert_eq!(probe(&w.fx, &w.src).restored(), parked.restored());
    let info = crate::machines::code::repo::repo_info(&w.fx.op, &w.src).expect("repo");
    let again = crate::machines::code::repo::parked_state(&w.fx.op, &info, "ses_trip")
        .expect("parked state");
    assert_eq!(again.map(|p| p.epoch).as_deref(), Some("1"));
}

#[test]
fn back_onto_the_parked_source() {
    there_and_back(false);
}

#[test]
fn back_is_refused_when_the_source_was_edited() {
    there_and_back(true);
}

#[test]
fn chaining_makes_the_next_bundle_small() {
    let w = World::new();
    w.seed_with(&[&[("a.txt", "a\n")]]);
    w.clones();
    put(&w.src, "big.bin", noise(7, 300_000));
    put(&w.src, "a.txt", "a2\n");
    let first = w.round("ses_chain", Transfer::Bundle, Landing::Checkout);
    first.assert_pass();
    put(&w.src, "a.txt", "a3\n");
    let git = runner();
    let have = haves(&git, &w.dst, Some("ses_chain")).expect("haves");
    let mut req = SnapshotRequest::new("ses_chain", 2, "machine-a", w.dir.join("move-2"));
    req.haves = have.haves.clone();
    let chained = snapshot(&git, &w.src, &req).expect("chained snapshot");
    let mut req = SnapshotRequest::new("ses_chain", 3, "machine-a", w.dir.join("move-3"));
    req.haves = have.haves;
    req.chain = false;
    let unchained = snapshot(&git, &w.src, &req).expect("unchained snapshot");
    let size = |m: &SnapshotMeta| m.bundle.as_ref().expect("bundle").size;
    assert_eq!(chained.chained.len(), 1);
    assert!(
        size(&chained) * 10 < size(&unchained),
        "chained {} B, unchained {} B",
        size(&chained),
        size(&unchained)
    );
    assert!(w.dir.join("move-2").join(BUNDLE).is_file());
    // T still holds the first move (parked for the session): it lands there.
    let mut areq = ApplyRequest::new("ses_chain", Landing::Checkout);
    areq.machine = "machine-b".into();
    let a = apply(&git, &w.dst, &w.dir.join("move-2"), &areq).expect("second move");
    assert_eq!(a.parked.as_deref(), Some("1"));
    let report = verify(&git, &a.path, &w.dir.join("move-2")).expect("verify");
    assert_eq!(report.outcome, Outcome::Pass, "{:?}", report.fails);
}

#[test]
fn a_missing_submodule_commit_is_a_limit_not_a_failed_fetch() {
    let w = World::new();
    let sub_origin = w.dir.join("sub-origin.git");
    let sub_seed = w.dir.join("sub-seed");
    w.fx.git(&w.dir, &["init", "--bare", "-q", s(&sub_origin)]);
    w.fx.git(&w.dir, &["clone", "-q", s(&sub_origin), s(&sub_seed)]);
    put(&sub_seed, "lib.txt", "v1\n");
    w.fx.git(&sub_seed, &["add", "-A"]);
    w.fx.git(&sub_seed, &["commit", "-q", "-m", "lib v1"]);
    w.fx.git(&sub_seed, &["push", "-q", "origin", "HEAD:refs/heads/main"]);
    w.init_origin();
    put(&w.seed, "a.txt", "a\n");
    w.fx.git(&w.seed, &["submodule", "add", "-q", s(&sub_origin), "lib"]);
    w.commit_seed("with a submodule");
    w.push_seed();
    w.clones();
    for r in [&w.src, &w.dst] {
        w.fx.git(r, &["submodule", "update", "-q", "--init"]);
    }
    let lib = w.src.join("lib");
    w.fx.git(&lib, &["switch", "-q", "-c", "work"]);
    put(&lib, "lib.txt", "v2\n");
    w.fx.git(&lib, &["commit", "-q", "-am", "lib v2, only on S"]);
    put(&w.src, "a.txt", "a2\n");
    let r = w.round("ses_sub", Transfer::Bundle, Landing::Checkout);
    assert!(r.meta.warning_kinds().contains(&"submodules"));
    let a = r.applied();
    assert_eq!(a.submodules.len(), 1);
    assert_eq!(a.submodules[0].state, SubmoduleState::Missing);
    let report = r.report();
    assert_eq!(report.outcome, Outcome::Limit, "{:?}", report.fails);
    assert_eq!(report.limits, vec!["submodule".to_string()]);
    assert_eq!(
        std::fs::read_to_string(w.dst.join("a.txt")).expect("a.txt"),
        "a2\n"
    );
}
