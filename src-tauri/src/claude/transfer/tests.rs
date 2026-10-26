//! The transfer engine on temp directories (docs/ACCOUNTS.md §9): copy, move,
//! every refusal, undo, the trust opt-in.

use super::*;

const SID: &str = "e2b6972f-7126-4f6e-a490-5ce75cb6a9f9";
const CWD: &str = "/p/app";

/// A session's whole file set in `home`.
fn session(home: &Path, id: &str, cwd: &str) {
    let proj = home.join("projects").join(encode_cwd(cwd));
    std::fs::create_dir_all(proj.join(id).join("subagents")).unwrap();
    std::fs::write(
        proj.join(format!("{id}.jsonl")),
        format!("{{\"type\":\"user\",\"cwd\":\"{cwd}\",\"sessionId\":\"{id}\"}}\n{{\"type\":\"assistant\"}}\n"),
    )
    .unwrap();
    std::fs::write(proj.join(id).join("subagents/agent-a1.jsonl"), "{}\n").unwrap();
    std::fs::create_dir_all(home.join("file-history").join(id)).unwrap();
    std::fs::write(
        home.join("file-history").join(id).join("snap@v1"),
        "old file",
    )
    .unwrap();
    // An empty session-env directory, as the CLI leaves them.
    std::fs::create_dir_all(home.join("session-env").join(id)).unwrap();
    std::fs::create_dir_all(home.join("todos")).unwrap();
    std::fs::write(
        home.join("todos").join(format!("{id}-agent-{id}.json")),
        "[]",
    )
    .unwrap();
}

struct World {
    _tmp: tempfile::TempDir,
    a: ClaudeHome,
    b: ClaudeHome,
    undo: PathBuf,
}

fn world() -> World {
    let tmp = tempfile::tempdir().unwrap();
    let a = ClaudeHome::for_account_dir(tmp.path().join("a"));
    let b = ClaudeHome::for_account_dir(tmp.path().join("b"));
    std::fs::create_dir_all(a.base()).unwrap();
    std::fs::create_dir_all(b.base()).unwrap();
    let undo = tmp.path().join("undo");
    World {
        _tmp: tmp,
        a,
        b,
        undo,
    }
}

fn not_live(_: &str) -> bool {
    false
}

fn req(what: What, mode: Mode, trust: bool) -> Request {
    Request {
        what,
        mode,
        trust,
        snapshot: false,
        refresh: false,
    }
}

fn sess() -> What {
    What::Session { id: SID.into() }
}

#[test]
fn a_copy_lands_byte_for_byte_and_touches_nothing_shared() {
    let w = world();
    session(w.a.base(), SID, CWD);
    // A project entry in the source: without the opt-in it is not copied.
    std::fs::write(
        w.a.claude_json(),
        serde_json::json!({"projects": {CWD: {"hasTrustDialogAccepted": true, "allowedTools": ["Bash"]}}})
            .to_string(),
    )
    .unwrap();
    let ctx = Context {
        from: &w.a,
        to: &w.b,
        is_live: &not_live,
    };
    let p = plan(&ctx, &req(sess(), Mode::Copy, false)).unwrap();
    assert_eq!(p.files, 4, "transcript, subagent, file history, todo");
    assert!(!p.writes_shared);
    assert!(p.adds_project_entry.is_none());
    let progress = Progress::default();
    let m = run(
        &p,
        &w.undo,
        "20261021T101010-0a0b0c0d",
        &progress,
        &not_live,
    )
    .unwrap();
    assert_eq!(m.created.len(), 4);
    assert_eq!(progress.files_done.load(Ordering::Relaxed), 4);
    for c in &m.created {
        let rel = c.path.strip_prefix(w.b.base()).unwrap();
        let src = w.a.base().join(rel);
        assert_eq!(
            std::fs::read(&src).unwrap(),
            std::fs::read(&c.path).unwrap()
        );
        let mt = |p: &Path| std::fs::metadata(p).unwrap().modified().unwrap();
        assert_eq!(mt(&src), mt(&c.path), "the source's mtime is kept");
    }
    assert!(
        w.b.session_env_dir().join(SID).is_dir(),
        "the empty env dir too"
    );
    assert!(
        !w.b.claude_json().exists(),
        "no .claude.json without the opt-in"
    );
    assert!(
        transcripts_of(&w.a, SID).len() == 1,
        "copy keeps the source"
    );
    // A second copy is refused: the target has it now.
    let again = plan(&ctx, &req(sess(), Mode::Copy, false)).unwrap_err();
    assert_eq!(again.code(), "already_exists");
    // Also when the source has it no more (it moved there): the target
    // having it is what the refusal says.
    std::fs::remove_file(transcripts_of(&w.a, SID).remove(0)).unwrap();
    let again = plan(&ctx, &req(sess(), Mode::Move, false)).unwrap_err();
    assert_eq!(again.code(), "already_exists");
}

#[test]
fn every_precondition_has_its_refusal() {
    let w = world();
    session(w.a.base(), SID, CWD);
    let live = |id: &str| id == SID;
    let same = Context {
        from: &w.a,
        to: &w.a,
        is_live: &not_live,
    };
    assert_eq!(
        plan(&same, &req(sess(), Mode::Copy, false))
            .unwrap_err()
            .code(),
        "same_directory"
    );
    let ctx_live = Context {
        from: &w.a,
        to: &w.b,
        is_live: &live,
    };
    assert_eq!(
        plan(&ctx_live, &req(sess(), Mode::Copy, false))
            .unwrap_err()
            .code(),
        "session_live"
    );
    let ctx = Context {
        from: &w.a,
        to: &w.b,
        is_live: &not_live,
    };
    let missing = What::Session {
        id: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee".into(),
    };
    assert_eq!(
        plan(&ctx, &req(missing, Mode::Copy, false))
            .unwrap_err()
            .code(),
        "not_found"
    );
    let bad = What::Session { id: "../x".into() };
    assert_eq!(
        plan(&ctx, &req(bad, Mode::Copy, false)).unwrap_err().code(),
        "invalid_transfer",
        "no transcript of a non-id is ever looked for"
    );
    // The trust opt-in writes the target's .claude.json: refused while a
    // live process uses it (our own pid is alive), and while it is locked.
    std::fs::write(
        w.a.claude_json(),
        serde_json::json!({"projects": {CWD: {"hasTrustDialogAccepted": true}}}).to_string(),
    )
    .unwrap();
    std::fs::create_dir_all(w.b.sessions_dir()).unwrap();
    let pidfile =
        w.b.sessions_dir()
            .join(format!("{}.json", std::process::id()));
    std::fs::write(&pidfile, "{}").unwrap();
    assert_eq!(
        plan(&ctx, &req(sess(), Mode::Copy, true))
            .unwrap_err()
            .code(),
        "directory_in_use"
    );
    // …while a plain copy (new files only) still may.
    assert!(plan(&ctx, &req(sess(), Mode::Copy, false)).is_ok());
    std::fs::remove_file(&pidfile).unwrap();
    std::fs::write(w.b.claude_json().with_extension("json.lock"), "").unwrap();
    assert_eq!(
        plan(&ctx, &req(sess(), Mode::Copy, true))
            .unwrap_err()
            .code(),
        "config_locked"
    );
    // A relative project is not a project.
    assert_eq!(
        plan(
            &ctx,
            &req(
                What::Project {
                    cwd: "p/app".into()
                },
                Mode::Copy,
                false
            )
        )
        .unwrap_err()
        .code(),
        "invalid_transfer"
    );
}

#[test]
fn a_move_can_be_undone_and_undo_refuses_lost_work() {
    let w = world();
    session(w.a.base(), SID, CWD);
    let before: Vec<(PathBuf, Vec<u8>)> = {
        let mut files = Vec::new();
        walk(w.a.base(), &mut files).unwrap();
        files
            .into_iter()
            .map(|f| {
                let bytes = std::fs::read(&f).unwrap();
                (f, bytes)
            })
            .collect()
    };
    let ctx = Context {
        from: &w.a,
        to: &w.b,
        is_live: &not_live,
    };
    let p = plan(&ctx, &req(sess(), Mode::Move, false)).unwrap();
    let id = new_job_id();
    assert!(is_job_id(&id));
    run(&p, &w.undo, &id, &Progress::default(), &not_live).unwrap();
    assert!(transcripts_of(&w.a, SID).is_empty(), "moved away");
    assert!(!w.a.session_env_dir().join(SID).exists());
    assert_eq!(transcripts_of(&w.b, SID).len(), 1);

    let m = undo(&w.undo, &id).unwrap();
    assert!(m.undone_at.is_some());
    for (f, bytes) in &before {
        assert_eq!(&std::fs::read(f).unwrap(), bytes, "{}", f.display());
    }
    assert!(
        w.a.session_env_dir().join(SID).is_dir(),
        "the empty dir is back"
    );
    assert!(transcripts_of(&w.b, SID).is_empty(), "the copy is gone");
    assert!(
        !w.b.projects_dir().join(encode_cwd(CWD)).exists(),
        "and the folders it made"
    );
    assert_eq!(undo(&w.undo, &id).unwrap_err().code(), "undo_conflict");

    // A copy that was continued in the target is not taken back.
    let id = new_job_id();
    let p = plan(&ctx, &req(sess(), Mode::Copy, false)).unwrap();
    run(&p, &w.undo, &id, &Progress::default(), &not_live).unwrap();
    let copied = transcripts_of(&w.b, SID).remove(0);
    std::fs::write(&copied, "{\"type\":\"user\"}\nmore work\n").unwrap();
    assert_eq!(undo(&w.undo, &id).unwrap_err().code(), "undo_conflict");
    assert_eq!(
        view_from_manifest(&manifest(&w.undo, &id).unwrap()).state,
        JobState::Done
    );
}

#[test]
fn a_project_moves_with_its_history_and_trust_only_when_asked() {
    let w = world();
    session(w.a.base(), SID, CWD);
    let other = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
    session(w.a.base(), other, CWD);
    // A look-alike folder's session lands in the same encoded directory.
    let lookalike = "bbbbbbbb-cccc-4ddd-8eee-ffffffffffff";
    session(w.a.base(), lookalike, "/p.app");
    std::fs::write(
        w.a.claude_json(),
        serde_json::json!({"projects": {CWD: {"hasTrustDialogAccepted": true, "allowedTools": ["Bash"]},
                                         "/elsewhere": {"x": 1}},
                           "userID": "keep-me"})
        .to_string(),
    )
    .unwrap();
    let line = |t: i64, d: &str, p: &str| {
        serde_json::json!({"display": d, "pastedContents": {}, "timestamp": t, "project": p})
            .to_string()
    };
    std::fs::write(
        w.a.history_file(),
        [
            line(1, "first", CWD),
            line(2, "elsewhere", "/elsewhere"),
            line(3, "second", CWD),
        ]
        .join("\n")
            + "\n",
    )
    .unwrap();
    // The target already has one of the lines: it is not duplicated.
    std::fs::write(w.b.history_file(), line(1, "first", CWD) + "\n").unwrap();
    let ctx = Context {
        from: &w.a,
        to: &w.b,
        is_live: &not_live,
    };
    let project = What::Project { cwd: CWD.into() };
    let p = plan(&ctx, &req(project.clone(), Mode::Move, true)).unwrap();
    let ids: Vec<&str> = p.sessions.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, [other, SID], "the look-alike's session stays");
    assert_eq!(p.adds_project_entry.as_deref(), Some(CWD));
    assert_eq!(p.removes_project_entry.as_deref(), Some(CWD));
    assert_eq!(p.history_lines, 1);
    let id = new_job_id();
    run(&p, &w.undo, &id, &Progress::default(), &not_live).unwrap();
    let read = |h: &ClaudeHome| -> Value {
        serde_json::from_str(&std::fs::read_to_string(h.claude_json()).unwrap()).unwrap()
    };
    assert_eq!(read(&w.b)["projects"][CWD]["hasTrustDialogAccepted"], true);
    assert!(read(&w.a)["projects"].get(CWD).is_none());
    assert_eq!(read(&w.a)["userID"], "keep-me", "the rest is untouched");
    assert_eq!(read(&w.a)["projects"]["/elsewhere"]["x"], 1);
    let hist = |h: &ClaudeHome| std::fs::read_to_string(h.history_file()).unwrap();
    assert_eq!(hist(&w.b).lines().count(), 2);
    assert_eq!(
        hist(&w.a).lines().count(),
        1,
        "only the other project's line"
    );
    assert_eq!(transcripts_of(&w.a, lookalike).len(), 1);

    undo(&w.undo, &id).unwrap();
    assert_eq!(read(&w.a)["projects"][CWD]["hasTrustDialogAccepted"], true);
    assert!(read(&w.b)["projects"].get(CWD).is_none());
    assert_eq!(hist(&w.b).lines().count(), 1);
    assert_eq!(hist(&w.a).lines().count(), 3);

    // Without the opt-in a project copy brings no entry, only history.
    let p = plan(&ctx, &req(project, Mode::Copy, false)).unwrap();
    assert!(p.adds_project_entry.is_none());
    assert!(p.writes_shared, "history lines");
}

#[test]
fn a_failed_copy_leaves_nothing_behind() {
    let w = world();
    session(w.a.base(), SID, CWD);
    let ctx = Context {
        from: &w.a,
        to: &w.b,
        is_live: &not_live,
    };
    let mut p = plan(&ctx, &req(sess(), Mode::Move, false)).unwrap();
    // A source file disappears between the plan and the run.
    let gone = p.sessions[0].files.last().unwrap().from.clone();
    std::fs::remove_file(&gone).unwrap();
    p.sessions[0].files.last_mut().unwrap().from = gone;
    let id = new_job_id();
    assert_eq!(
        run(&p, &w.undo, &id, &Progress::default(), &not_live)
            .unwrap_err()
            .code(),
        "transfer_io"
    );
    assert!(transcripts_of(&w.b, SID).is_empty());
    assert!(!w.b.projects_dir().join(encode_cwd(CWD)).exists());
    assert_eq!(
        transcripts_of(&w.a, SID).len(),
        1,
        "a move keeps its source"
    );
    assert!(!w.undo.join(&id).exists());
}

#[test]
fn old_undo_directories_are_pruned() {
    let tmp = tempfile::tempdir().unwrap();
    let old = tmp.path().join("20200101T000000-00000000");
    let fresh = tmp.path().join(new_job_id());
    let other = tmp.path().join("not-ours");
    for d in [&old, &fresh, &other] {
        std::fs::create_dir_all(d).unwrap();
    }
    let long_ago = SystemTime::now() - Duration::from_secs(30 * 24 * 3600);
    std::fs::File::open(&old)
        .unwrap()
        .set_modified(long_ago)
        .unwrap();
    prune_undo(tmp.path());
    assert!(!old.exists());
    assert!(fresh.exists());
    assert!(other.exists(), "only our job directories");
}

#[test]
fn a_snapshot_of_a_live_session_leaves_out_its_unfinished_line() {
    let w = world();
    session(w.a.base(), SID, CWD);
    let transcript = transcripts_of(&w.a, SID).remove(0);
    // The CLI is in the middle of a line.
    let mut f = std::fs::File::options()
        .append(true)
        .open(&transcript)
        .unwrap();
    f.write_all(b"{\"type\":\"assist").unwrap();
    let live = |_: &str| true;
    let ctx = Context {
        from: &w.a,
        to: &w.b,
        is_live: &live,
    };
    let mut r = req(sess(), Mode::Copy, false);
    assert_eq!(plan(&ctx, &r).unwrap_err().code(), "session_live");
    r.snapshot = true;
    let p = plan(&ctx, &r).unwrap();
    run(&p, &w.undo, &new_job_id(), &Progress::default(), &not_live).unwrap();
    let copied = std::fs::read_to_string(transcripts_of(&w.b, SID).remove(0)).unwrap();
    assert!(copied.ends_with('\n'));
    assert_eq!(copied.lines().count(), 2, "the two complete lines");
    // Never a move, never a project.
    r.mode = Mode::Move;
    assert_eq!(plan(&ctx, &r).unwrap_err().code(), "invalid_transfer");
}

/// Every file under a directory with its bytes (a `.claude.json` as its
/// JSON: undo writes it back pretty-printed).
fn snapshot_of(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    walk(dir, &mut files).unwrap();
    files.sort();
    files
        .into_iter()
        .map(|f| {
            let mut bytes = std::fs::read(&f).unwrap();
            if f.file_name().is_some_and(|n| n == ".claude.json") {
                let v: Value = serde_json::from_slice(&bytes).unwrap();
                bytes = v.to_string().into_bytes();
            }
            (f, bytes)
        })
        .collect()
}

/// Two sessions of one project in `a`, with an entry and history lines.
fn project_world() -> (World, What) {
    let w = world();
    session(w.a.base(), SID, CWD);
    session(w.a.base(), "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee", CWD);
    std::fs::write(
        w.a.claude_json(),
        serde_json::json!({"projects": {CWD: {"hasTrustDialogAccepted": true}}}).to_string(),
    )
    .unwrap();
    let line = serde_json::json!({"display": "hi", "timestamp": 1, "project": CWD}).to_string();
    std::fs::write(w.a.history_file(), line + "\n").unwrap();
    (w, What::Project { cwd: CWD.into() })
}

#[test]
fn a_move_that_fails_halfway_puts_every_moved_file_back() {
    let (w, project) = project_world();
    let before = snapshot_of(w.a.base());
    let ctx = Context {
        from: &w.a,
        to: &w.b,
        is_live: &not_live,
    };
    let p = plan(&ctx, &req(project, Mode::Move, true)).unwrap();
    let id = new_job_id();
    // The third file fails to move (a full disk, a vanished file…).
    fail_at("stash", 2);
    let err = run(&p, &w.undo, &id, &Progress::default(), &not_live).unwrap_err();
    clear_failures();
    assert_eq!(err.code(), "transfer_io");
    assert_eq!(snapshot_of(w.a.base()), before, "the source is as it was");
    assert!(w.a.session_env_dir().join(SID).is_dir());
    assert!(transcripts_of(&w.b, SID).is_empty(), "the copies are gone");
    assert!(
        !w.b.claude_json().exists() || {
            let v: Value =
                serde_json::from_str(&std::fs::read_to_string(w.b.claude_json()).unwrap()).unwrap();
            v["projects"].get(CWD).is_none()
        }
    );
    assert!(!w.undo.join(&id).exists(), "nothing was left to undo");
}

#[test]
fn a_failed_move_that_cannot_be_put_back_keeps_its_undo_and_undo_finishes_it() {
    let (w, project) = project_world();
    let before = snapshot_of(w.a.base());
    let ctx = Context {
        from: &w.a,
        to: &w.b,
        is_live: &not_live,
    };
    let p = plan(&ctx, &req(project, Mode::Move, true)).unwrap();
    let id = new_job_id();
    fail_at("stash", 2);
    // …and putting the first file back fails as well.
    fail_at("put_back", 0);
    let err = run(&p, &w.undo, &id, &Progress::default(), &not_live).unwrap_err();
    clear_failures();
    assert_eq!(err.code(), "transfer_incomplete");
    assert!(
        w.undo.join(&id).join("manifest.json").exists(),
        "the undo directory and its manifest stay"
    );
    let m = manifest(&w.undo, &id).unwrap();
    assert_eq!(m.moved.len(), 2, "what had moved is recorded");
    undo(&w.undo, &id).unwrap();
    assert_eq!(
        snapshot_of(w.a.base()),
        before,
        "undo finished putting it back"
    );
    assert!(transcripts_of(&w.b, SID).is_empty());
}

#[test]
fn an_undo_that_fails_can_be_retried() {
    let (w, project) = project_world();
    let before = snapshot_of(w.a.base());
    let ctx = Context {
        from: &w.a,
        to: &w.b,
        is_live: &not_live,
    };
    let p = plan(&ctx, &req(project, Mode::Move, true)).unwrap();
    let id = new_job_id();
    run(&p, &w.undo, &id, &Progress::default(), &not_live).unwrap();
    // The third file fails to come back: two are back, the copies all stay.
    fail_at("put_back", 2);
    assert_eq!(undo(&w.undo, &id).unwrap_err().code(), "transfer_io");
    clear_failures();
    assert_eq!(transcripts_of(&w.b, SID).len(), 1, "nothing deleted yet");
    undo(&w.undo, &id).unwrap();
    assert_eq!(snapshot_of(w.a.base()), before);
    assert!(transcripts_of(&w.b, SID).is_empty());
    let hist = std::fs::read_to_string(w.a.history_file()).unwrap();
    assert_eq!(hist.lines().count(), 1, "the history line came back once");
}

#[test]
fn undo_refuses_while_a_session_runs_or_the_directory_is_in_use() {
    let (w, project) = project_world();
    let ctx = Context {
        from: &w.a,
        to: &w.b,
        is_live: &not_live,
    };
    let p = plan(&ctx, &req(project, Mode::Move, true)).unwrap();
    let id = new_job_id();
    run(&p, &w.undo, &id, &Progress::default(), &not_live).unwrap();
    let live = |s: &str| s == SID;
    assert_eq!(
        undo_with(&w.undo, &id, &live).unwrap_err().code(),
        "session_live"
    );
    // A Claude Code process in the target (this test's own pid stands in).
    std::fs::create_dir_all(w.b.sessions_dir()).unwrap();
    let pidfile =
        w.b.sessions_dir()
            .join(format!("{}.json", std::process::id()));
    std::fs::write(&pidfile, "{}").unwrap();
    assert_eq!(undo(&w.undo, &id).unwrap_err().code(), "directory_in_use");
    std::fs::remove_file(&pidfile).unwrap();
    undo(&w.undo, &id).unwrap();
}

#[test]
fn a_session_that_changes_or_starts_after_the_plan_is_not_moved() {
    let w = world();
    session(w.a.base(), SID, CWD);
    let ctx = Context {
        from: &w.a,
        to: &w.b,
        is_live: &not_live,
    };
    let p = plan(&ctx, &req(sess(), Mode::Move, false)).unwrap();
    // It started meanwhile (the job waited behind a backup).
    let live = |s: &str| s == SID;
    assert_eq!(recheck(&p, &live).unwrap_err().code(), "session_live");
    assert_eq!(
        run(&p, &w.undo, &new_job_id(), &Progress::default(), &live)
            .unwrap_err()
            .code(),
        "session_live"
    );
    assert_eq!(transcripts_of(&w.a, SID).len(), 1);
    assert!(transcripts_of(&w.b, SID).is_empty());
    // It wrote a file the plan does not know.
    std::fs::write(
        w.a.file_history_dir().join(SID).join("snap@v2"),
        "newer file",
    )
    .unwrap();
    let before = snapshot_of(w.a.base());
    assert_eq!(
        run(&p, &w.undo, &new_job_id(), &Progress::default(), &not_live)
            .unwrap_err()
            .code(),
        "session_changed"
    );
    assert_eq!(
        snapshot_of(w.a.base()),
        before,
        "nothing moved, nothing lost"
    );
    assert!(transcripts_of(&w.b, SID).is_empty());
    // The target got the session meanwhile.
    let p = plan(&ctx, &req(sess(), Mode::Copy, false)).unwrap();
    session(w.b.base(), SID, CWD);
    assert_eq!(recheck(&p, &not_live).unwrap_err().code(), "already_exists");
}

#[test]
fn a_project_move_without_the_opt_in_leaves_the_entry_where_it_is() {
    let (w, project) = project_world();
    let ctx = Context {
        from: &w.a,
        to: &w.b,
        is_live: &not_live,
    };
    let p = plan(&ctx, &req(project, Mode::Move, false)).unwrap();
    assert!(p.adds_project_entry.is_none());
    assert!(p.removes_project_entry.is_none());
    run(&p, &w.undo, &new_job_id(), &Progress::default(), &not_live).unwrap();
    let v: Value =
        serde_json::from_str(&std::fs::read_to_string(w.a.claude_json()).unwrap()).unwrap();
    assert_eq!(
        v["projects"][CWD]["hasTrustDialogAccepted"], true,
        "its MCP servers and tools are not thrown away"
    );
    assert!(!w.b.claude_json().exists());
    // The history still moves with the project.
    assert_eq!(
        std::fs::read_to_string(w.b.history_file())
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert_eq!(std::fs::read_to_string(w.a.history_file()).unwrap(), "");
}

#[cfg(unix)]
#[test]
fn a_symlink_in_a_sessions_place_is_never_followed() {
    let w = world();
    session(w.a.base(), SID, CWD);
    let outside = w._tmp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret"), "not a session's").unwrap();
    let fh = w.a.file_history_dir().join(SID);
    std::fs::remove_dir_all(&fh).unwrap();
    std::os::unix::fs::symlink(&outside, &fh).unwrap();
    let ctx = Context {
        from: &w.a,
        to: &w.b,
        is_live: &not_live,
    };
    let p = plan(&ctx, &req(sess(), Mode::Copy, false)).unwrap();
    assert!(
        p.sessions[0].files.iter().all(|f| !f.from.starts_with(&fh)),
        "{:?}",
        p.sessions[0].files
    );
}

#[test]
fn history_edits_keep_every_byte_and_never_write_over_an_unreadable_file() {
    let tmp = tempfile::tempdir().unwrap();
    let h = tmp.path().join("history.jsonl");
    // A torn multibyte character (the CLI killed mid-append) in one line.
    let mut bytes = b"{\"display\":\"a\",\"project\":\"/p\"}\n".to_vec();
    bytes.extend_from_slice(b"{\"display\":\"\xe2\x82\"}\n");
    bytes.extend_from_slice(b"{\"display\":\"b\",\"project\":\"/p\"}\n");
    std::fs::write(&h, &bytes).unwrap();
    assert_eq!(
        history_lines(&h, "/p").len(),
        2,
        "the bad line is skipped, not the rest"
    );
    append_lines(&h, &["{\"display\":\"c\"}".to_string()]).unwrap();
    let after = std::fs::read(&h).unwrap();
    assert!(after.starts_with(&bytes), "every byte that was there stays");
    let removed =
        remove_lines(&h, &["{\"display\":\"a\",\"project\":\"/p\"}".to_string()]).unwrap();
    assert_eq!(removed.len(), 1);
    assert!(std::fs::read(&h)
        .unwrap()
        .windows(3)
        .any(|w| w == b"\xe2\x82\""));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&h, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&h).is_err() {
            assert!(append_lines(&h, &["x".to_string()]).is_err());
            assert!(remove_lines(&h, &["x".to_string()]).is_err());
        }
        std::fs::set_permissions(&h, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    // A retried undo adds nothing twice.
    let lines = vec![
        "{\"display\":\"c\"}".to_string(),
        "{\"display\":\"d\"}".to_string(),
    ];
    append_missing_lines(&h, &lines).unwrap();
    append_missing_lines(&h, &lines).unwrap();
    let text = String::from_utf8_lossy(&std::fs::read(&h).unwrap()).into_owned();
    assert_eq!(text.matches("{\"display\":\"c\"}").count(), 1);
    assert_eq!(text.matches("{\"display\":\"d\"}").count(), 1);
}

#[test]
fn trimming_a_snapshot_reads_only_its_tail() {
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join("t.jsonl");
    // Longer than one block, the partial line straddling a block boundary.
    let mut body = String::new();
    while body.len() < 200 * 1024 {
        body.push_str("{\"type\":\"assistant\",\"text\":\"lorem ipsum\"}\n");
    }
    let partial = "x".repeat(100 * 1024);
    std::fs::write(&f, format!("{body}{partial}")).unwrap();
    trim_partial_line(&f).unwrap();
    assert_eq!(std::fs::read_to_string(&f).unwrap(), body);
    trim_partial_line(&f).unwrap();
    assert_eq!(
        std::fs::read_to_string(&f).unwrap(),
        body,
        "complete: untouched"
    );
    std::fs::write(&f, "no newline at all").unwrap();
    trim_partial_line(&f).unwrap();
    assert_eq!(std::fs::read(&f).unwrap(), b"");
}

#[cfg(unix)]
#[test]
fn a_rewritten_file_without_an_original_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join(".claude.json");
    rewrite(&f, b"{}").unwrap();
    assert_eq!(
        std::fs::metadata(&f).unwrap().permissions().mode() & 0o777,
        0o600
    );
    std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
    rewrite(&f, b"{\"a\":1}").unwrap();
    assert_eq!(
        std::fs::metadata(&f).unwrap().permissions().mode() & 0o777,
        0o644,
        "an existing file keeps its mode"
    );
}

#[test]
fn a_second_continue_on_brings_its_earlier_copy_up_to_date() {
    let w = world();
    session(w.a.base(), SID, CWD);
    let live = |_: &str| true;
    let ctx = Context {
        from: &w.a,
        to: &w.b,
        is_live: &live,
    };
    let mut r = req(sess(), Mode::Copy, false);
    r.snapshot = true;
    r.refresh = true;
    let p = plan(&ctx, &r).unwrap();
    run(&p, &w.undo, &new_job_id(), &Progress::default(), &live).unwrap();
    let src = transcripts_of(&w.a, SID).remove(0);
    let copy = transcripts_of(&w.b, SID).remove(0);
    let first = std::fs::read(&copy).unwrap();
    // Nothing new (an unfinished line does not count): fork it as it is.
    let mut f = std::fs::File::options().append(true).open(&src).unwrap();
    f.write_all(b"{\"type\":\"assist").unwrap();
    assert_eq!(plan(&ctx, &r).unwrap_err().code(), "already_exists");
    // The original went on (its limit reset) and gained a turn.
    f.write_all(b"ant\"}\n{\"type\":\"user\",\"n\":3}\n")
        .unwrap();
    drop(f);
    let p = plan(&ctx, &r).unwrap();
    let id = new_job_id();
    let m = run(&p, &w.undo, &id, &Progress::default(), &live).unwrap();
    assert_eq!(std::fs::read(&copy).unwrap(), std::fs::read(&src).unwrap());
    assert_eq!(m.moved.len(), 1, "only the transcript was behind");
    assert_eq!(m.created.len(), 1);
    // Undo puts the earlier copy back.
    undo(&w.undo, &id).unwrap();
    assert_eq!(std::fs::read(&copy).unwrap(), first);
    // A copy that was continued in the target is forked as it stands.
    let mut f = std::fs::File::options().append(true).open(&copy).unwrap();
    f.write_all(b"{\"type\":\"user\",\"in\":\"b\"}\n").unwrap();
    assert_eq!(plan(&ctx, &r).unwrap_err().code(), "already_exists");
    // Only a snapshot refreshes.
    r.snapshot = false;
    assert_eq!(plan(&ctx, &r).unwrap_err().code(), "invalid_transfer");
}

fn items(sessions: &[&str], projects: &[&str], mode: Mode, trust: bool) -> Items {
    Items {
        sessions: sessions.iter().map(|s| s.to_string()).collect(),
        projects: projects.iter().map(|s| s.to_string()).collect(),
        mode,
        trust,
        customs: Vec::new(),
        replace_existing: false,
    }
}

fn mine(_: &str) -> Result<(), (String, String)> {
    Ok(())
}

const CWD2: &str = "/p/lib";
const S2: &str = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
const S3: &str = "cccccccc-dddd-4eee-8fff-000000000000";

/// §13.3 rule 2: the `items` body skips what cannot go, each with its code,
/// and the rest goes ahead; the old body refuses with the same code.
#[test]
fn bulk_skips_what_cannot_go_where_the_old_body_refuses() {
    let w = world();
    session(w.a.base(), SID, CWD);
    session(w.a.base(), S2, CWD);
    session(w.a.base(), S3, CWD2);
    // The target has S3 already.
    session(w.b.base(), S3, CWD2);
    let live = |id: &str| id == SID;
    let ctx = Context {
        from: &w.a,
        to: &w.b,
        is_live: &live,
    };
    let missing = "dddddddd-eeee-4fff-8000-111111111111";
    let (p, skipped) = plan_items(
        &ctx,
        &items(&[SID, S2, S3, missing, "../x"], &[], Mode::Copy, false),
        &mine,
    )
    .unwrap();
    let ids: Vec<&str> = p.sessions.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, [S2]);
    let codes: Vec<(&str, &str)> = skipped
        .iter()
        .map(|s| (s.id.as_deref().unwrap(), s.code.as_str()))
        .collect();
    assert_eq!(
        codes,
        [
            (SID, "session_live"),
            (S3, "already_exists"),
            (missing, "not_found"),
            ("../x", "invalid_transfer"),
        ]
    );
    // The old body refuses the first that cannot go.
    assert_eq!(
        plan(&ctx, &req(sess(), Mode::Copy, false))
            .unwrap_err()
            .code(),
        "session_live"
    );
    // A project with nothing here is skipped by its folder.
    let (_, skipped) =
        plan_items(&ctx, &items(&[], &["/p/none"], Mode::Copy, false), &mine).unwrap();
    assert_eq!(skipped[0].cwd.as_deref(), Some("/p/none"));
    assert_eq!(skipped[0].code, "not_found");
    // A conversation the source account does not own is left out.
    let theirs = |id: &str| {
        if id == S2 {
            Err((
                "not_found".to_string(),
                "it is Pro's conversation".to_string(),
            ))
        } else {
            Ok(())
        }
    };
    let (p, skipped) = plan_items(&ctx, &items(&[S2], &[], Mode::Copy, false), &theirs).unwrap();
    assert!(p.sessions.is_empty() && p.is_empty());
    assert_eq!(skipped[0].message, "it is Pro's conversation");
    // Same directory: nothing to move.
    let same = Context {
        from: &w.a,
        to: &w.a,
        is_live: &not_live,
    };
    assert_eq!(
        plan_items(&same, &items(&[S2], &[], Mode::Copy, false), &mine)
            .unwrap_err()
            .code(),
        "same_directory"
    );
}

/// Two projects in one job: their conversations, prompt history and (trust)
/// entries go together, and one undo puts all of it back.
#[test]
fn several_projects_move_in_one_job_and_one_undo() {
    let w = world();
    session(w.a.base(), SID, CWD);
    session(w.a.base(), S2, CWD);
    session(w.a.base(), S3, CWD2);
    std::fs::write(
        w.a.claude_json(),
        serde_json::json!({"projects": {
            CWD: {"hasTrustDialogAccepted": true},
            CWD2: {"hasTrustDialogAccepted": true, "allowedTools": ["Bash"]},
            "/elsewhere": {"x": 1}}})
        .to_string(),
    )
    .unwrap();
    let line = |t: i64, d: &str, p: &str| {
        serde_json::json!({"display": d, "timestamp": t, "project": p}).to_string()
    };
    std::fs::write(
        w.a.history_file(),
        [
            line(1, "app one", CWD),
            line(2, "lib one", CWD2),
            line(3, "elsewhere", "/elsewhere"),
            line(4, "app two", CWD),
        ]
        .join("\n")
            + "\n",
    )
    .unwrap();
    // History lines come back at the end: compared as a set.
    let lines = |h: &ClaudeHome| {
        let mut l: Vec<String> = std::fs::read_to_string(h.history_file())
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect();
        l.sort();
        l
    };
    let without_history = |dir: &Path| {
        snapshot_of(dir)
            .into_iter()
            .filter(|(f, _)| !f.ends_with("history.jsonl"))
            .collect::<Vec<_>>()
    };
    let before_a = without_history(w.a.base());
    let lines_a = lines(&w.a);
    let ctx = Context {
        from: &w.a,
        to: &w.b,
        is_live: &not_live,
    };
    let (p, skipped) =
        plan_items(&ctx, &items(&[], &[CWD, CWD2], Mode::Move, true), &mine).unwrap();
    assert!(skipped.is_empty(), "{skipped:?}");
    assert_eq!(p.sessions.len(), 3);
    assert_eq!(p.projects, [CWD, CWD2]);
    assert_eq!(p.added_entries(), [CWD, CWD2]);
    assert_eq!(p.removed_entries(), [CWD, CWD2]);
    assert_eq!(p.history_lines, 3);
    assert!(p.writes_shared && p.warnings.is_empty());
    let id = new_job_id();
    run(&p, &w.undo, &id, &Progress::default(), &not_live).unwrap();
    let read = |h: &ClaudeHome| -> Value {
        serde_json::from_str(&std::fs::read_to_string(h.claude_json()).unwrap()).unwrap()
    };
    assert_eq!(read(&w.b)["projects"][CWD2]["allowedTools"][0], "Bash");
    assert!(read(&w.b)["projects"].get(CWD).is_some());
    assert!(read(&w.a)["projects"].get(CWD).is_none());
    assert!(read(&w.a)["projects"].get(CWD2).is_none());
    assert_eq!(read(&w.a)["projects"]["/elsewhere"]["x"], 1);
    let hist = |h: &ClaudeHome| std::fs::read_to_string(h.history_file()).unwrap();
    assert_eq!(hist(&w.b).lines().count(), 3);
    assert_eq!(
        hist(&w.a).lines().count(),
        1,
        "only the other folder's line"
    );
    for id in [SID, S2, S3] {
        assert!(transcripts_of(&w.a, id).is_empty());
        assert_eq!(transcripts_of(&w.b, id).len(), 1);
    }
    let m = manifest(&w.undo, &id).unwrap();
    assert_eq!(m.added_entries.len(), 2);
    assert_eq!(m.removed_entries.len(), 2);

    undo(&w.undo, &id).unwrap();
    assert_eq!(
        without_history(w.a.base()),
        before_a,
        "the source as it was"
    );
    assert_eq!(lines(&w.a), lines_a);
    for id in [SID, S2, S3] {
        assert!(transcripts_of(&w.b, id).is_empty());
    }
    assert!(read(&w.b)["projects"].as_object().unwrap().is_empty());
    assert_eq!(hist(&w.b), "");
}

/// A project of which a conversation cannot go is copied as far as it can
/// be, and in a move its prompt history and entry stay in the source.
#[test]
fn a_project_that_cannot_go_whole_keeps_its_history_where_it_is() {
    let w = world();
    session(w.a.base(), SID, CWD);
    session(w.a.base(), S2, CWD);
    std::fs::write(
        w.a.claude_json(),
        serde_json::json!({"projects": {CWD: {"hasTrustDialogAccepted": true}}}).to_string(),
    )
    .unwrap();
    let line = serde_json::json!({"display": "hi", "timestamp": 1, "project": CWD}).to_string();
    std::fs::write(w.a.history_file(), line.clone() + "\n").unwrap();
    let live = |id: &str| id == SID;
    let ctx = Context {
        from: &w.a,
        to: &w.b,
        is_live: &live,
    };
    let (p, skipped) = plan_items(&ctx, &items(&[], &[CWD], Mode::Move, true), &mine).unwrap();
    assert_eq!(skipped[0].code, "session_live");
    assert_eq!(p.sessions.len(), 1);
    assert!(p.removed_entries().is_empty(), "the entry stays");
    assert_eq!(p.history_lines, 1, "copied all the same");
    assert_eq!(p.warnings.len(), 1, "{:?}", p.warnings);
    run(&p, &w.undo, &new_job_id(), &Progress::default(), &live).unwrap();
    assert_eq!(
        std::fs::read_to_string(w.a.history_file()).unwrap(),
        line.clone() + "\n",
        "the source keeps its history"
    );
    assert_eq!(
        std::fs::read_to_string(w.b.history_file()).unwrap(),
        line + "\n"
    );
    let v: Value =
        serde_json::from_str(&std::fs::read_to_string(w.a.claude_json()).unwrap()).unwrap();
    assert!(v["projects"].get(CWD).is_some());
    assert_eq!(transcripts_of(&w.a, SID).len(), 1, "the running one stays");
    assert_eq!(transcripts_of(&w.b, S2).len(), 1);
}

use super::custom::{Custom, CustomAction};

const ALL_CUSTOMS: [Custom; 9] = [
    Custom::Memory,
    Custom::Settings,
    Custom::Skills,
    Custom::Agents,
    Custom::Commands,
    Custom::OutputStyles,
    Custom::Keybindings,
    Custom::McpServers,
    Custom::History,
];

fn customs(list: &[Custom], replace: bool) -> Items {
    Items {
        sessions: Vec::new(),
        projects: Vec::new(),
        mode: Mode::Copy,
        trust: false,
        customs: list.to_vec(),
        replace_existing: replace,
    }
}

/// The source's customizations: every category, a skill linked from a
/// repository among them.
fn customized(w: &World) -> PathBuf {
    let a = w.a.base();
    std::fs::write(a.join("CLAUDE.md"), "be brief").unwrap();
    std::fs::write(
        a.join("settings.json"),
        serde_json::json!({"model": "opus", "env": {"ANTHROPIC_API_KEY": "sk-secret-in-settings"},
                           "statusLine": {"type": "command", "command": "x"}})
        .to_string(),
    )
    .unwrap();
    std::fs::create_dir_all(a.join("skills/review/scripts")).unwrap();
    std::fs::write(a.join("skills/review/SKILL.md"), "review skill").unwrap();
    std::fs::write(a.join("skills/review/scripts/run.sh"), "echo hi").unwrap();
    std::fs::create_dir_all(a.join("skills/synced/x")).unwrap();
    std::fs::write(a.join("skills/synced/x/SKILL.md"), "synced").unwrap();
    let repo = w._tmp.path().join("repo/skills/linted");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("SKILL.md"), "from a repository").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("../../repo/skills/linted", a.join("skills/linted")).unwrap();
    std::fs::create_dir_all(a.join("agents")).unwrap();
    std::fs::write(a.join("agents/planner.md"), "plan").unwrap();
    std::fs::create_dir_all(a.join("commands/git")).unwrap();
    std::fs::write(a.join("commands/git/ship.md"), "ship it").unwrap();
    std::fs::create_dir_all(a.join("output-styles")).unwrap();
    std::fs::write(a.join("output-styles/terse.md"), "terse").unwrap();
    std::fs::write(a.join("keybindings.json"), "[]").unwrap();
    std::fs::write(
        w.a.claude_json(),
        serde_json::json!({"userID": "a", "mcpServers": {
            "github": {"command": "gh-mcp", "env": {"GITHUB_TOKEN": "ghp-secret-token"}},
            "shared": {"command": "a-version"}}})
        .to_string(),
    )
    .unwrap();
    let line = |t: i64, d: &str| {
        serde_json::json!({"display": d, "timestamp": t, "project": "/p"}).to_string()
    };
    std::fs::write(
        w.a.history_file(),
        [line(1, "one"), line(2, "two")].join("\n") + "\n",
    )
    .unwrap();
    repo
}

fn mcp_of(h: &ClaudeHome) -> Value {
    let v: Value =
        serde_json::from_str(&std::fs::read_to_string(h.claude_json()).unwrap()).unwrap();
    v["mcpServers"].clone()
}

/// §13.3 rule 7: every category adds what the target lacks and leaves what
/// it has; MCP servers merge by name; the history by `(timestamp,
/// display)`; a symlinked skill is linked again, never followed; the plan
/// names MCP servers only by name and warns about settings that choose the
/// sign-in or run commands; one undo takes all of it back.
#[cfg(unix)]
#[test]
fn customizations_add_what_the_target_lacks_and_undo_takes_them_back() {
    let w = world();
    let repo = customized(&w);
    let b = w.b.base();
    std::fs::write(b.join("CLAUDE.md"), "the target's own").unwrap();
    std::fs::write(
        w.b.claude_json(),
        serde_json::json!({"userID": "b", "mcpServers": {"shared": {"command": "b-version"}}})
            .to_string(),
    )
    .unwrap();
    std::fs::write(
        w.b.history_file(),
        serde_json::json!({"display": "one", "timestamp": 1, "project": "/p"}).to_string() + "\n",
    )
    .unwrap();
    let before = snapshot_of(b);
    let ctx = Context {
        from: &w.a,
        to: &w.b,
        is_live: &not_live,
    };
    let (p, skipped) = plan_items(&ctx, &customs(&ALL_CUSTOMS, false), &mine).unwrap();
    assert!(skipped.is_empty(), "{skipped:?}");
    let cp = p.customs.clone().unwrap();
    let actions: Vec<(String, &str, CustomAction)> = cp
        .items
        .iter()
        .map(|i| (i.item.key().to_string(), i.name.as_str(), i.action))
        .collect();
    assert_eq!(
        actions,
        [
            ("memory".to_string(), "CLAUDE.md", CustomAction::Skip),
            ("settings".into(), "settings.json", CustomAction::Add),
            ("skills".into(), "linted", CustomAction::Add),
            ("skills".into(), "review", CustomAction::Add),
            ("agents".into(), "planner.md", CustomAction::Add),
            ("commands".into(), "git", CustomAction::Add),
            ("outputStyles".into(), "terse.md", CustomAction::Add),
            ("keybindings".into(), "keybindings.json", CustomAction::Add),
            ("mcpServers".into(), "github", CustomAction::Add),
            ("mcpServers".into(), "shared", CustomAction::Skip),
            ("history".into(), "history.jsonl", CustomAction::Merge),
        ]
    );
    assert_eq!(p.history_lines, 1, "the line the target lacks");
    let served = serde_json::to_string(&cp.items).unwrap() + &p.warnings.concat();
    assert!(!served.contains("ghp-secret-token") && !served.contains("sk-secret-in-settings"));
    assert!(
        p.warnings
            .iter()
            .any(|w| w.contains("env ANTHROPIC_API_KEY")),
        "{:?}",
        p.warnings
    );
    assert!(p.warnings.iter().any(|w| w.contains("statusLine")));
    assert!(p.writes_shared, "the MCP servers and the history");

    let id = new_job_id();
    run(&p, &w.undo, &id, &Progress::default(), &not_live).unwrap();
    assert_eq!(
        std::fs::read_to_string(b.join("CLAUDE.md")).unwrap(),
        "the target's own"
    );
    assert_eq!(
        std::fs::read_to_string(b.join("skills/review/scripts/run.sh")).unwrap(),
        "echo hi"
    );
    assert!(
        !b.join("skills/synced").exists(),
        "synced skills are runtime state"
    );
    let link = b.join("skills/linted");
    assert!(std::fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        std::fs::read_link(&link).unwrap(),
        repo.parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("skills/linted"),
        "the same absolute target"
    );
    assert_eq!(
        std::fs::read_to_string(b.join("commands/git/ship.md")).unwrap(),
        "ship it"
    );
    assert_eq!(mcp_of(&w.b)["github"]["command"], "gh-mcp");
    assert_eq!(
        mcp_of(&w.b)["shared"]["command"],
        "b-version",
        "never over its own"
    );
    assert_eq!(
        std::fs::read_to_string(w.b.history_file())
            .unwrap()
            .lines()
            .count(),
        2
    );

    undo(&w.undo, &id).unwrap();
    assert_eq!(snapshot_of(b), before, "the target as it was");
    assert!(
        std::fs::symlink_metadata(&link).is_err(),
        "the link is gone"
    );
    assert!(
        repo.join("SKILL.md").exists(),
        "and never what it pointed to"
    );
}

/// With `replaceExisting`, what the target has is replaced, the old copy kept
/// in the undo directory; the undo puts it back — a replaced skill folder
/// whole, with the file the new one did not have.
#[test]
fn replaced_customizations_come_back_with_the_undo() {
    let w = world();
    customized(&w);
    let b = w.b.base();
    std::fs::write(b.join("CLAUDE.md"), "the target's own").unwrap();
    std::fs::create_dir_all(b.join("skills/review")).unwrap();
    std::fs::write(b.join("skills/review/SKILL.md"), "old review").unwrap();
    std::fs::write(b.join("skills/review/notes.md"), "only here").unwrap();
    std::fs::write(
        w.b.claude_json(),
        serde_json::json!({"mcpServers": {"shared": {"command": "b-version"}}}).to_string(),
    )
    .unwrap();
    let before = snapshot_of(b);
    let ctx = Context {
        from: &w.a,
        to: &w.b,
        is_live: &not_live,
    };
    let list = [Custom::Memory, Custom::Skills, Custom::McpServers];
    let (p, _) = plan_items(&ctx, &customs(&list, true), &mine).unwrap();
    let replaced: Vec<&str> = p
        .customs
        .as_ref()
        .unwrap()
        .items
        .iter()
        .filter(|i| i.action == CustomAction::Replace)
        .map(|i| i.name.as_str())
        .collect();
    assert_eq!(replaced, ["CLAUDE.md", "review", "shared"]);
    let id = new_job_id();
    run(&p, &w.undo, &id, &Progress::default(), &not_live).unwrap();
    assert_eq!(
        std::fs::read_to_string(b.join("CLAUDE.md")).unwrap(),
        "be brief"
    );
    assert_eq!(
        std::fs::read_to_string(b.join("skills/review/SKILL.md")).unwrap(),
        "review skill"
    );
    assert!(!b.join("skills/review/notes.md").exists(), "replaced whole");
    assert_eq!(mcp_of(&w.b)["shared"]["command"], "a-version");
    // A file of the target changed since: the undo refuses.
    std::fs::write(b.join("CLAUDE.md"), "edited after").unwrap();
    assert_eq!(undo(&w.undo, &id).unwrap_err().code(), "undo_conflict");
    std::fs::write(b.join("CLAUDE.md"), "be brief").unwrap();
    undo(&w.undo, &id).unwrap();
    assert_eq!(snapshot_of(b), before, "every replaced file back");
}

/// Adding new files runs while the target has live sessions; replacing, the
/// MCP servers and the history wait (D4). Customizations are never moved,
/// and a category the source lacks is skipped by name.
#[test]
fn customizations_follow_the_in_use_rule_and_are_only_copied() {
    let w = world();
    std::fs::create_dir_all(w.a.base().join("agents")).unwrap();
    std::fs::write(w.a.base().join("agents/planner.md"), "plan").unwrap();
    std::fs::write(w.a.base().join("CLAUDE.md"), "be brief").unwrap();
    std::fs::write(w.b.base().join("CLAUDE.md"), "own").unwrap();
    std::fs::create_dir_all(w.b.sessions_dir()).unwrap();
    std::fs::write(
        w.b.sessions_dir()
            .join(format!("{}.json", std::process::id())),
        "{}",
    )
    .unwrap();
    let ctx = Context {
        from: &w.a,
        to: &w.b,
        is_live: &not_live,
    };
    let (p, skipped) = plan_items(
        &ctx,
        &customs(&[Custom::Agents, Custom::Keybindings], false),
        &mine,
    )
    .unwrap();
    assert!(!p.writes_shared, "new files only");
    assert_eq!(skipped[0].item.as_deref(), Some("keybindings"));
    assert_eq!(skipped[0].code, "not_found");
    run(&p, &w.undo, &new_job_id(), &Progress::default(), &not_live).unwrap();
    assert!(w.b.base().join("agents/planner.md").exists());
    assert_eq!(
        plan_items(&ctx, &customs(&[Custom::Memory], true), &mine)
            .unwrap_err()
            .code(),
        "directory_in_use"
    );
    let mut moving = customs(&[Custom::Agents], false);
    moving.mode = Mode::Move;
    assert_eq!(
        plan_items(&ctx, &moving, &mine).unwrap_err().code(),
        "invalid_transfer"
    );
}

// ─────────────────────────── deleting (§14.3) ───────────────────────────

/// Delete `ids` from `a`: the job id and what it did.
fn delete_from_a(
    w: &World,
    ids: &[&str],
    keep_undo: bool,
    is_live: &dyn Fn(&str) -> bool,
    may_go: &dyn Fn(&str) -> Result<(), Skipped>,
) -> Result<(String, delete::TakeReport), TransferError> {
    let job = new_job_id();
    let ids: Vec<String> = ids.iter().map(|s| s.to_string()).collect();
    let r = delete::take(&delete::TakeInputs {
        home: &w.a,
        ids: &ids,
        undo_root: &w.undo,
        job_id: &job,
        account: "default",
        keep_undo,
        is_live,
        may_go,
        attributed: &|id| (id == SID).then(|| "acc_0123456789ab".to_string()),
        progress: &Progress::default(),
    })?;
    Ok((job, r))
}

fn any_may_go(_: &str) -> Result<(), Skipped> {
    Ok(())
}

fn codes(skipped: &[Skipped]) -> Vec<(String, String)> {
    skipped
        .iter()
        .map(|s| (s.id.clone().unwrap_or_default(), s.code.clone()))
        .collect()
}

/// A delete takes every file of the conversation (§1.4) into the undo
/// directory and nothing else: another conversation of the folder, the
/// folder itself, `.claude.json` and `history.jsonl` stay as they are; its
/// undo puts every file back byte for byte.
#[test]
fn a_delete_takes_a_conversations_files_and_nothing_shared() {
    let w = world();
    session(w.a.base(), SID, CWD);
    session(w.a.base(), S2, CWD);
    std::fs::write(
        w.a.claude_json(),
        serde_json::json!({"projects": {CWD: {"hasTrustDialogAccepted": true}}}).to_string(),
    )
    .unwrap();
    std::fs::write(
        w.a.history_file(),
        format!("{{\"display\":\"hi\",\"project\":\"{CWD}\",\"timestamp\":1}}\n"),
    )
    .unwrap();
    let shared = || {
        (
            std::fs::read(w.a.claude_json()).unwrap(),
            std::fs::read(w.a.history_file()).unwrap(),
        )
    };
    let (config, history) = shared();
    let before = snapshot_of(w.a.base());
    let (job, r) = delete_from_a(&w, &[SID], true, &not_live, &any_may_go).unwrap();
    assert_eq!(r.deleted, [SID]);
    assert!(r.skipped.is_empty());
    let proj = w.a.transcript_dir(CWD);
    assert!(transcripts_of(&w.a, SID).is_empty());
    assert!(!proj.join(SID).exists(), "its own folder");
    assert!(!w.a.file_history_dir().join(SID).exists());
    assert!(!w.a.session_env_dir().join(SID).exists());
    assert!(!w
        .a
        .todos_dir()
        .join(format!("{SID}-agent-{SID}.json"))
        .exists());
    assert_eq!(transcripts_of(&w.a, S2).len(), 1, "the other one stays");
    assert!(w.a.file_history_dir().join(S2).is_dir());
    assert!(proj.is_dir(), "a folder that still holds one stays");
    assert_eq!(shared(), (config, history), "no shared file is touched");
    let m = manifest(&w.undo, &job).unwrap();
    assert_eq!(m.kind.as_deref(), Some(delete::KIND));
    assert_eq!(m.sessions, [SID]);
    assert_eq!(
        m.token_accounts,
        [(SID.to_string(), "acc_0123456789ab".to_string())],
        "its account, for the undo"
    );
    assert_eq!(m.moved.len(), 4, "transcript, subagent, file history, todo");
    let v = view_from_manifest(&m);
    assert_eq!(v.kind, "delete");
    assert_eq!(v.account.as_deref(), Some("default"));
    assert_eq!((v.from, v.to), (None, None), "a delete has no target");
    assert_eq!(v.files_total, 4);
    assert!(v.undo_until.is_some());

    undo(&w.undo, &job).unwrap();
    assert_eq!(snapshot_of(w.a.base()), before, "every file back");
    assert!(
        w.a.session_env_dir().join(SID).is_dir(),
        "the empty env dir too"
    );
}

/// A project folder its conversations left empty goes too (a `.DS_Store`
/// with it) and comes back with Undo; one that holds anything else — the
/// project's memory, say — stays.
#[test]
fn a_project_folder_left_empty_goes_and_comes_back() {
    let w = world();
    session(w.a.base(), SID, CWD);
    let proj = w.a.transcript_dir(CWD);
    std::fs::write(proj.join(".DS_Store"), "finder").unwrap();
    session(w.a.base(), S2, "/p/other");
    let other = w.a.transcript_dir("/p/other");
    std::fs::create_dir_all(other.join("memory")).unwrap();
    std::fs::write(other.join("memory/MEMORY.md"), "remember").unwrap();
    let before = snapshot_of(w.a.base());
    let (job, r) = delete_from_a(&w, &[SID, S2], true, &not_live, &any_may_go).unwrap();
    assert_eq!(r.deleted, [SID, S2]);
    assert!(!proj.exists(), "left empty, it goes");
    assert!(
        other.join("memory/MEMORY.md").is_file(),
        "a folder with anything else stays"
    );
    assert!(transcripts_of(&w.a, S2).is_empty());
    undo(&w.undo, &job).unwrap();
    assert_eq!(snapshot_of(w.a.base()), before, "the folder is back");
}

/// `undo: false`: every file was taken out first, then the undo directory
/// goes and the job has no Undo.
#[test]
fn without_undo_nothing_is_kept() {
    let w = world();
    session(w.a.base(), SID, CWD);
    let (job, r) = delete_from_a(&w, &[SID], false, &not_live, &any_may_go).unwrap();
    assert_eq!(r.deleted, [SID]);
    assert!(r.manifest.is_none());
    assert!(transcripts_of(&w.a, SID).is_empty());
    assert!(!w.undo.join(&job).exists(), "the undo directory is gone");
    assert_eq!(undo(&w.undo, &job).unwrap_err().code(), "not_found");
}

/// What runs is skipped, never taken: a living pidfile of the directory
/// (read right before), what the app knows to run (an owned session, a
/// bridge conversation), and one the request may no longer take.
#[test]
fn what_runs_or_may_not_go_is_skipped_and_kept() {
    let w = world();
    session(w.a.base(), SID, CWD);
    session(w.a.base(), S2, CWD);
    session(w.a.base(), S3, "/p/third");
    std::fs::create_dir_all(w.a.sessions_dir()).unwrap();
    std::fs::write(
        w.a.sessions_dir()
            .join(format!("{}.json", std::process::id())),
        serde_json::json!({"pid": std::process::id(), "sessionId": SID}).to_string(),
    )
    .unwrap();
    let live = |id: &str| id == S2;
    let may_go = |id: &str| {
        if id == S3 {
            Err(Skipped::session(id, "session_changed", "given away"))
        } else {
            Ok(())
        }
    };
    let before = snapshot_of(w.a.base());
    let (job, r) = delete_from_a(&w, &[SID, S2, S3], true, &live, &may_go).unwrap();
    assert!(r.deleted.is_empty());
    let want = |id: &str, code: &str| (id.to_string(), code.to_string());
    assert_eq!(
        codes(&r.skipped),
        [
            want(SID, "session_live"),
            want(S2, "session_live"),
            want(S3, "session_changed")
        ]
    );
    assert_eq!(snapshot_of(w.a.base()), before, "nothing moved");
    assert!(!w.undo.join(&job).exists(), "nothing to undo");
}

/// A conversation that cannot go whole is put back and skipped; the rest
/// goes.
#[test]
fn a_conversation_that_cannot_go_whole_is_put_back() {
    let w = world();
    session(w.a.base(), SID, CWD);
    session(w.a.base(), S2, CWD);
    let files_of = |id: &str| {
        snapshot_of(w.a.base())
            .into_iter()
            .filter(|(p, _)| p.to_string_lossy().contains(id))
            .collect::<Vec<_>>()
    };
    let sid_before = files_of(SID);
    // The first conversation's second file fails to move.
    fail_at("stash", 1);
    let (job, r) = delete_from_a(&w, &[SID, S2], true, &not_live, &any_may_go).unwrap();
    clear_failures();
    assert_eq!(r.deleted, [S2]);
    assert_eq!(r.skipped[0].id.as_deref(), Some(SID));
    assert_eq!(r.skipped[0].code, "transfer_io");
    assert_eq!(files_of(SID), sid_before, "put back whole");
    assert_eq!(manifest(&w.undo, &job).unwrap().sessions, [S2]);
}

/// When even putting it back fails, the undo directory stays (an `undo:
/// false` one too), the job says so, and Undo finishes it.
#[test]
fn a_conversation_that_cannot_be_put_back_keeps_the_undo() {
    let w = world();
    session(w.a.base(), SID, CWD);
    let before = snapshot_of(w.a.base());
    fail_at("stash", 1);
    fail_at("put_back", 0);
    let err = delete_from_a(&w, &[SID], false, &not_live, &any_may_go).unwrap_err();
    clear_failures();
    assert_eq!(err.code(), "transfer_incomplete");
    let job = std::fs::read_dir(&w.undo)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| is_job_id(n))
        .expect("the undo directory stays");
    assert!(w.undo.join(&job).join("manifest.json").exists());
    undo(&w.undo, &job).unwrap();
    assert_eq!(snapshot_of(w.a.base()), before);
}

/// Undo never puts a file over one that is there now (`undo_conflict`), and
/// then changes nothing.
#[test]
fn undo_of_a_delete_refuses_where_something_new_is() {
    let w = world();
    session(w.a.base(), SID, CWD);
    let (job, _) = delete_from_a(&w, &[SID], true, &not_live, &any_may_go).unwrap();
    let t = w.a.transcript_path(CWD, SID);
    std::fs::create_dir_all(t.parent().unwrap()).unwrap();
    std::fs::write(&t, "a new transcript by that name").unwrap();
    assert_eq!(undo(&w.undo, &job).unwrap_err().code(), "undo_conflict");
    assert_eq!(
        std::fs::read_to_string(&t).unwrap(),
        "a new transcript by that name"
    );
    assert!(
        !w.a.file_history_dir().join(SID).exists(),
        "nothing else came back either"
    );
}

/// A symbolic link in a conversation's place is never followed: what it
/// points to stays.
#[cfg(unix)]
#[test]
fn a_delete_never_follows_a_symlink() {
    let w = world();
    session(w.a.base(), SID, CWD);
    let outside = w.a.base().parent().unwrap().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("keep.txt"), "mine").unwrap();
    let fh = w.a.file_history_dir().join(SID);
    std::fs::remove_dir_all(&fh).unwrap();
    std::os::unix::fs::symlink(&outside, &fh).unwrap();
    let (_, r) = delete_from_a(&w, &[SID], true, &not_live, &any_may_go).unwrap();
    assert_eq!(r.deleted, [SID]);
    assert_eq!(
        std::fs::read_to_string(outside.join("keep.txt")).unwrap(),
        "mine"
    );
}
