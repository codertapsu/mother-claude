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
    let m = run(&p, &w.undo, "20261021T101010-0a0b0c0d", &progress).unwrap();
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
    run(&p, &w.undo, &id, &Progress::default()).unwrap();
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
    run(&p, &w.undo, &id, &Progress::default()).unwrap();
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
    run(&p, &w.undo, &id, &Progress::default()).unwrap();
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
        run(&p, &w.undo, &id, &Progress::default())
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
    run(&p, &w.undo, &new_job_id(), &Progress::default()).unwrap();
    let copied = std::fs::read_to_string(transcripts_of(&w.b, SID).remove(0)).unwrap();
    assert!(copied.ends_with('\n'));
    assert_eq!(copied.lines().count(), 2, "the two complete lines");
    // Never a move, never a project.
    r.mode = Mode::Move;
    assert_eq!(plan(&ctx, &r).unwrap_err().code(), "invalid_transfer");
}
