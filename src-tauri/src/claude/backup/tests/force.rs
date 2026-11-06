//! Force backup, force pull and force restore (docs/BACKUP.md "Force"): one
//! machine's copy replaces the other's, whatever the usual rules say, with
//! what it replaced kept for a week and the bookkeeping left so that the
//! next normal push or pull on either machine behaves.

use super::*;

fn push_as(w: &World, home: &ClaudeHome, host: &str, force: bool) -> PushReport {
    push_with(
        &PushInputs {
            home,
            set: &w.set,
            account_name: "Work",
            title_of: &no_title,
            passphrases: &[pass()],
            machine: &machine(host),
            is_live: &not_live,
            progress: &Progress::default(),
        },
        force,
    )
    .unwrap()
}

fn pull_as(
    w: &World,
    home: &ClaudeHome,
    host: &str,
    job: &str,
    force: bool,
    live: &dyn Fn(&str) -> bool,
) -> (PullReport, Manifest) {
    pull(&PullInputs {
        set: &w.set,
        passphrases: &[pass()],
        home,
        machine: &machine(host),
        ids: None,
        trust: false,
        is_live: live,
        undo_root: &w.undo,
        job_id: job,
        progress: &Progress::default(),
        force,
    })
    .unwrap()
}

fn bytes(home: &ClaudeHome, id: &str) -> Vec<u8> {
    std::fs::read(transcript(home, id)).unwrap()
}

/// A (the Mac) pushes; B pulls, continues and pushes, so the set's copy is
/// B's and newer; A, which did not pull it, force-pushes its older copy.
/// It replaces B's (kept seven days in the set), a session only B pushed
/// stays, and B's next normal pull finds its copy not where the set's came
/// from: the set's copy lands beside B's own, B's untouched. B's force pull
/// then makes B's copy A's, and its undo puts B's back.
#[test]
fn a_force_push_replaces_a_newer_copy_and_the_other_machine_pulls_by_the_rules() {
    let w = world();
    let (a, b) = (&w.home, &w.other);
    session(a.base(), SID);
    push_as(&w, a, "mbp-a", false);
    pull_as(&w, b, "win-b", "20261106T100000-00000001", false, &not_live);
    grow(b, SID, "{\"type\":\"user\",\"on\":\"b, continued\"}");
    session(b.base(), OTHER);
    assert_eq!(push_as(&w, b, "win-b", false).archived.len(), 2);
    let theirs = index_of(&w).sessions[SID].clone();
    let a_copy = bytes(a, SID);

    // A normal push leaves B's copy: it is newer in the set.
    let r = push_as(&w, a, "mbp-a", false);
    assert_eq!(r.newer_in_set, [SID], "{r:?}");
    assert!(r.archived.is_empty());

    // A force push replaces it.
    let before = now_ms();
    let r = push_as(&w, a, "mbp-a", true);
    assert!(r.force);
    assert_eq!(r.archived, [SID], "{r:?}");
    assert_eq!(r.replaced, [SID]);
    let index = index_of(&w);
    let ours = index.sessions[SID].clone();
    assert_eq!(ours.machine, machine("mbp-a").id);
    assert_eq!(ours.transcript_bytes, a_copy.len() as u64);
    assert!(
        index.sessions.contains_key(OTHER),
        "a session only in the set stays"
    );
    // B's archive is kept a week, not the usual day.
    let kept = index
        .superseded
        .iter()
        .find(|s| s.path == format!("sessions/{}", theirs.archive))
        .expect("B's archive is kept");
    let until = kept.until.expect("until a week from now");
    assert!(until >= before + FORCE_KEEP_MS && until <= now_ms() + FORCE_KEEP_MS);
    assert_eq!(kept.at, until - GRACE_MS, "an older build keeps it as long");
    let b_archive = w.set.session_archive(&theirs).unwrap();
    // Three days on (A's first archive, superseded by B's push, goes then).
    collect_garbage(&w.set, &index, now_ms() + 3 * GRACE_MS);
    assert!(b_archive.exists(), "recoverable for a week");
    collect_garbage(&w.set, &index, until + 1);
    assert!(!b_archive.exists(), "then it goes");
    assert!(w.set.session_archive(&ours).unwrap().exists());
    // A's own bookkeeping: its next normal push has nothing to do.
    let r = push_as(&w, a, "mbp-a", false);
    assert!(r.archived.is_empty() && r.newer_in_set.is_empty() && r.conflicts.is_empty());

    // B, unchanged since its push, pulls normally: the set's copy (A's, an
    // older one) is not where B's came from, so it lands beside B's.
    let b_copy = bytes(b, SID);
    let keys = open_keys(&w.set, &pass()).unwrap();
    let ix = read_index(&w.set, &keys).unwrap();
    let plan = compare(&ix, b, &machine("win-b").id, &not_live, now_ms());
    let item = plan.iter().find(|p| p.id == SID).unwrap();
    assert_eq!(item.action, PullAction::Update);
    let (r, _) = pull_as(&w, b, "win-b", "20261106T100001-00000002", false, &not_live);
    assert!(r.updated.is_empty() && r.replaced.is_empty(), "{r:?}");
    assert_eq!(r.conflicts.len(), 1, "{r:?}");
    assert_eq!(std::fs::read(&r.conflicts[0].1).unwrap(), a_copy);
    assert_eq!(bytes(b, SID), b_copy, "B's copy untouched");
    // Neither side pushes over the other until one continues.
    let r = push_as(&w, b, "win-b", false);
    assert!(r.archived.is_empty() && r.conflicts.is_empty(), "{r:?}");
    assert_eq!(index_of(&w).sessions[SID], ours);

    // B's force pull: A's copy in B's place, B's kept for undo.
    let job = "20261106T100002-00000003";
    let (r, m) = pull_as(&w, b, "win-b", job, true, &not_live);
    assert!(r.force);
    assert_eq!(r.replaced, [SID], "{r:?}");
    assert!(r.updated.is_empty() && r.conflicts.is_empty());
    assert_eq!(bytes(b, SID), a_copy);
    assert!(!m.moved.is_empty(), "B's copy kept for undo");
    assert_eq!(
        transfer::transcripts_of(b, OTHER).len(),
        1,
        "a session only B has stays"
    );
    // The next normal push and pull on B: the same copy, nothing to do.
    let r = push_as(&w, b, "win-b", false);
    assert!(r.archived.is_empty() && r.newer_in_set.is_empty() && r.conflicts.is_empty());
    let (r, _) = pull_as(&w, b, "win-b", "20261106T100003-00000004", false, &not_live);
    assert!(r.updated.is_empty() && r.replaced.is_empty() && r.conflicts.is_empty());

    // The force pull's undo puts B's own copy back.
    transfer::undo(&w.undo, job).unwrap();
    assert_eq!(bytes(b, SID), b_copy);
}

/// After an undone force pull, the copy that is back compares as it did
/// before the pull: B's push does not put it over A's in the set.
#[test]
fn an_undone_force_pull_leaves_the_set_as_the_usual_rules_say() {
    let w = world();
    let (a, b) = (&w.home, &w.other);
    session(a.base(), SID);
    push_as(&w, a, "mbp-a", false);
    pull_as(&w, b, "win-b", "20261106T110000-00000001", false, &not_live);
    // Both continue; A pushes first.
    grow(a, SID, "{\"type\":\"user\",\"on\":\"a\"}");
    grow(b, SID, "{\"type\":\"user\",\"on\":\"b, a longer line\"}");
    push_as(&w, a, "mbp-a", false);
    let set_copy = index_of(&w).sessions[SID].clone();
    let b_copy = bytes(b, SID);
    let job = "20261106T110001-00000002";
    let (r, _) = pull_as(&w, b, "win-b", job, true, &not_live);
    assert_eq!(r.replaced, [SID], "a conflict, replaced: {r:?}");
    assert_eq!(bytes(b, SID), bytes(a, SID));
    transfer::undo(&w.undo, job).unwrap();
    assert_eq!(bytes(b, SID), b_copy);
    // B's copy changed on both sides still: a conflict, never pushed over A's.
    let r = push_as(&w, b, "win-b", false);
    assert_eq!(r.conflicts, [SID], "{r:?}");
    assert_eq!(index_of(&w).sessions[SID], set_copy);
}

/// A session that runs is never touched by any force: a force push skips
/// it, a force pull says it was not replaced, a force restore skips it.
#[test]
fn a_running_session_is_never_touched_by_a_force() {
    let w = world();
    let (a, b) = (&w.home, &w.other);
    session(a.base(), SID);
    push_as(&w, a, "mbp-a", false);
    pull_as(&w, b, "win-b", "20261106T120000-00000001", false, &not_live);
    grow(b, SID, "{\"type\":\"user\",\"on\":\"b\"}");
    push_as(&w, b, "win-b", false);
    let set_copy = index_of(&w).sessions[SID].clone();
    let live = |id: &str| id == SID;

    let r = push_with(
        &PushInputs {
            home: a,
            set: &w.set,
            account_name: "Work",
            title_of: &no_title,
            passphrases: &[pass()],
            machine: &machine("mbp-a"),
            is_live: &live,
            progress: &Progress::default(),
        },
        true,
    )
    .unwrap();
    assert_eq!(r.skipped_live, [SID]);
    assert!(r.archived.is_empty() && r.replaced.is_empty());
    assert_eq!(index_of(&w).sessions[SID], set_copy);

    let a_copy = bytes(a, SID);
    let (r, m) = pull_as(&w, a, "mbp-a", "20261106T120001-00000002", true, &live);
    assert!(r.replaced.is_empty() && r.updated.is_empty(), "{r:?}");
    assert_eq!(r.skipped.len(), 1, "{r:?}");
    assert_eq!(r.skipped[0].code, "session_live");
    assert_eq!(bytes(a, SID), a_copy, "untouched");
    assert!(m.moved.is_empty() && m.created.is_empty());

    let (r, _) = restore_with(
        &RestoreInputs {
            set: &w.set,
            passphrases: &[pass()],
            target: a,
            ids: &[SID.to_string()],
            trust: false,
            is_live: &live,
            undo_root: &w.undo,
            job_id: "20261106T120002-00000003",
            progress: &Progress::default(),
            kind: "restore",
        },
        true,
    )
    .unwrap();
    assert_eq!(r.skipped[0].code, "session_live");
    assert!(r.replaced.is_empty());
    assert_eq!(bytes(a, SID), a_copy);
}

/// A force restore replaces a copy the account has (an ordinary one skips
/// it), keeps that copy for undo, and still brings new ones as before.
#[test]
fn a_force_restore_replaces_the_copy_here_and_its_undo_puts_it_back() {
    let w = world();
    session(w.home.base(), SID);
    session(w.home.base(), OTHER);
    push_once(&w, &not_live);
    let target = &w.other;
    session(target.base(), SID);
    std::fs::write(
        transcript(target, SID),
        "{\"type\":\"user\",\"made\":\"elsewhere, and longer than the set's copy\"}\n",
    )
    .unwrap();
    let mine = bytes(target, SID);
    let inputs = |job: &'static str, ids: &'static [&'static str]| {
        (job, ids.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    };
    let (job, ids) = inputs("20261106T130000-00000001", &[SID, OTHER]);
    let run = |job: &str, ids: &[String], force: bool| {
        restore_with(
            &RestoreInputs {
                set: &w.set,
                passphrases: &[pass()],
                target,
                ids,
                trust: false,
                is_live: &not_live,
                undo_root: &w.undo,
                job_id: job,
                progress: &Progress::default(),
                kind: "restore",
            },
            force,
        )
        .unwrap()
    };
    let (r, _) = run(job, &ids, false);
    assert_eq!(r.restored, [OTHER]);
    assert_eq!(r.skipped[0].code, "already_exists");
    assert_eq!(bytes(target, SID), mine);
    crate::claude::transfer::undo(&w.undo, job).unwrap();

    let (job, ids) = inputs("20261106T130001-00000002", &[SID, OTHER]);
    let (r, m) = run(job, &ids, true);
    assert!(r.force);
    assert_eq!(r.restored, [OTHER]);
    assert_eq!(r.replaced, [SID]);
    assert_eq!(bytes(target, SID), bytes(&w.home, SID));
    assert!(m
        .moved
        .iter()
        .any(|mv| mv.original == transcript(target, SID)));
    crate::claude::transfer::undo(&w.undo, job).unwrap();
    assert_eq!(
        bytes(target, SID),
        mine,
        "the undo puts the account's copy back"
    );
    assert!(transfer::transcripts_of(target, OTHER).is_empty());
}

/// A force push replaces only what differs: a session the set has as it is
/// here stays as it is (no new archive, nothing superseded).
#[test]
fn a_force_push_leaves_a_copy_the_set_has_as_it_is() {
    let w = world();
    session(w.home.base(), SID);
    push_as(&w, &w.home, "mbp-a", false);
    let before = index_of(&w);
    let r = push_as(&w, &w.home, "mbp-a", true);
    assert!(r.archived.is_empty() && r.replaced.is_empty(), "{r:?}");
    assert_eq!(r.unchanged, 1);
    let after = index_of(&w);
    assert_eq!(after.sessions, before.sessions);
    assert!(after.superseded.is_empty());
}
