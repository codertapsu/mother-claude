//! The backup engine on temp directories (docs/ACCOUNTS.md §9): archive and
//! restore byte-identical, a wrong passphrase, a truncated archive, a live
//! session skipped, the incremental rule, the pruning warning.

use super::*;
use crate::claude::home::encode_cwd;

const SID: &str = "e2b6972f-7126-4f6e-a490-5ce75cb6a9f9";
const CWD: &str = "/p/app";

fn session(home: &Path, id: &str) {
    let proj = home.join("projects").join(encode_cwd(CWD));
    std::fs::create_dir_all(proj.join(id).join("subagents")).unwrap();
    std::fs::write(
        proj.join(format!("{id}.jsonl")),
        format!("{{\"type\":\"user\",\"cwd\":\"{CWD}\",\"sessionId\":\"{id}\"}}\n"),
    )
    .unwrap();
    std::fs::write(proj.join(id).join("subagents/agent-a1.jsonl"), "{}\n").unwrap();
    std::fs::create_dir_all(home.join("file-history").join(id)).unwrap();
    std::fs::write(
        home.join("file-history").join(id).join("snap@v1"),
        vec![7u8; 5000],
    )
    .unwrap();
    std::fs::create_dir_all(home.join("session-env").join(id)).unwrap();
}

fn config(home: &Path) {
    std::fs::write(home.join("settings.json"), "{\"model\":\"opus\"}").unwrap();
    std::fs::write(home.join("CLAUDE.md"), "be kind").unwrap();
    std::fs::create_dir_all(home.join("skills/mine")).unwrap();
    std::fs::write(home.join("skills/mine/SKILL.md"), "skill").unwrap();
    // Never backed up.
    std::fs::write(home.join(".credentials.json"), "{\"secret\":1}").unwrap();
    std::fs::create_dir_all(home.join("sessions")).unwrap();
    std::fs::write(home.join("sessions/1.json"), "{}").unwrap();
    std::fs::create_dir_all(home.join("skills/synced/x")).unwrap();
    std::fs::write(home.join("skills/synced/x/SKILL.md"), "synced").unwrap();
    std::fs::write(home.join(".claude.json.lock"), "").unwrap();
}

fn pass() -> Secret {
    Secret::new("correct horse battery staple")
}

struct World {
    _tmp: tempfile::TempDir,
    home: ClaudeHome,
    other: ClaudeHome,
    set: Set,
    undo: PathBuf,
}

fn world() -> World {
    let tmp = tempfile::tempdir().unwrap();
    let home = ClaudeHome::for_account_dir(tmp.path().join("claude"));
    let other = ClaudeHome::for_account_dir(tmp.path().join("claude-elsewhere"));
    std::fs::create_dir_all(home.base()).unwrap();
    std::fs::create_dir_all(other.base()).unwrap();
    let set = Set::new(&tmp.path().join("iCloud/Mother Claude"), "Work");
    let undo = tmp.path().join("undo");
    World {
        _tmp: tmp,
        home,
        other,
        set,
        undo,
    }
}

fn not_live(_: &str) -> bool {
    false
}

fn push_once(w: &World, live: &dyn Fn(&str) -> bool) -> PushReport {
    push(&PushInputs {
        home: &w.home,
        set: &w.set,
        account_name: "Work",
        passphrase: &pass(),
        host: "mbp-a",
        is_live: live,
        progress: &Progress::default(),
    })
    .unwrap()
}

#[test]
fn a_round_trip_is_byte_identical_and_the_second_run_archives_nothing() {
    let w = world();
    session(w.home.base(), SID);
    config(w.home.base());
    let r = push_once(&w, &not_live);
    assert_eq!(r.archived, vec![SID.to_string()]);
    assert!(r.config.is_some());
    assert!(w.set.archive(SID).is_file());
    // Nothing secret or plain in the set but set.json.
    let info = read_info(&w.set).unwrap();
    assert_eq!(info.account, "Work");
    assert_eq!(info.machines[0].host, "mbp-a");
    let all: Vec<u8> = std::fs::read(w.set.archive(SID)).unwrap();
    assert!(
        !String::from_utf8_lossy(&all).contains("sessionId"),
        "encrypted"
    );

    // The incremental rule: nothing changed, nothing archived.
    let again = push_once(&w, &not_live);
    assert!(again.archived.is_empty());
    assert_eq!(again.unchanged, 1);
    assert!(again.config.is_none(), "the bundle is unchanged too");
    // A grown transcript is archived again.
    let t = w
        .home
        .projects_dir()
        .join(encode_cwd(CWD))
        .join(format!("{SID}.jsonl"));
    let mut f = std::fs::File::options().append(true).open(&t).unwrap();
    f.write_all(b"{\"type\":\"assistant\"}\n").unwrap();
    drop(f);
    assert_eq!(push_once(&w, &not_live).archived, vec![SID.to_string()]);

    // Restore into another directory: the same bytes and mtimes.
    let (report, m) = restore(&RestoreInputs {
        set: &w.set,
        passphrase: &pass(),
        target: &w.other,
        ids: &[SID.to_string()],
        trust: false,
        is_live: &not_live,
        undo_root: &w.undo,
        job_id: "20261021T101010-0a0b0c0d",
        progress: &Progress::default(),
        kind: "restore",
    })
    .unwrap();
    assert_eq!(report.restored, vec![SID.to_string()]);
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    let mut src = Vec::new();
    crate::claude::transfer::walk(w.home.base(), &mut src).unwrap();
    for f in src.iter().filter(|f| {
        let rel = f.strip_prefix(w.home.base()).unwrap().to_string_lossy();
        SESSION_DIRS.iter().any(|d| rel.starts_with(d))
    }) {
        let rel = f.strip_prefix(w.home.base()).unwrap();
        let back = w.other.base().join(rel);
        assert_eq!(
            std::fs::read(f).unwrap(),
            std::fs::read(&back).unwrap(),
            "{}",
            rel.display()
        );
        let mt = |p: &Path| std::fs::metadata(p).unwrap().modified().unwrap();
        let (a, b) = (mt(f), mt(&back));
        let secs = |t: SystemTime| t.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
        assert_eq!(secs(a), secs(b), "mtime kept: {}", rel.display());
    }
    assert!(
        w.other.session_env_dir().join(SID).is_dir(),
        "the empty dir too"
    );
    assert_eq!(m.created.len(), 3);
    // The restore's undo is a transfer undo.
    crate::claude::transfer::undo(&w.undo, "20261021T101010-0a0b0c0d").unwrap();
    assert!(crate::claude::transfer::transcripts_of(&w.other, SID).is_empty());
}

#[test]
fn the_configuration_bundle_leaves_out_what_d6_excludes() {
    let w = world();
    config(w.home.base());
    let files: Vec<String> = config_files(&w.home)
        .into_iter()
        .map(|(_, rel)| rel.to_string_lossy().replace('\\', "/"))
        .collect();
    assert_eq!(
        files,
        ["CLAUDE.md", "settings.json", "skills/mine/SKILL.md"]
    );
    push_once(&w, &not_live);
    let out = w._tmp.path().join("review");
    let dest = extract_config(&w.set, &pass(), &out).unwrap();
    assert_eq!(
        std::fs::read_to_string(dest.join("CLAUDE.md")).unwrap(),
        "be kind"
    );
    assert!(!dest.join(".credentials.json").exists());
}

#[test]
fn a_wrong_passphrase_opens_nothing() {
    let w = world();
    session(w.home.base(), SID);
    push_once(&w, &not_live);
    let wrong = Secret::new("not it at all");
    assert!(matches!(
        open_keys(&w.set, &wrong),
        Err(BackupError::WrongPassphrase)
    ));
    let err = push(&PushInputs {
        home: &w.home,
        set: &w.set,
        account_name: "Work",
        passphrase: &wrong,
        host: "mbp-a",
        is_live: &not_live,
        progress: &Progress::default(),
    })
    .unwrap_err();
    assert_eq!(err.code(), "wrong_passphrase");
    // A new passphrase keeps every archive readable.
    rekey(&w.set, &pass(), &wrong).unwrap();
    assert!(open_keys(&w.set, &wrong).is_ok());
    assert!(open_keys(&w.set, &pass()).is_err());
}

#[test]
fn a_live_session_waits_and_a_truncated_archive_is_refused() {
    let w = world();
    session(w.home.base(), SID);
    let live = |id: &str| id == SID;
    let r = push_once(&w, &live);
    assert_eq!(r.skipped_live, vec![SID.to_string()]);
    assert!(!w.set.archive(SID).exists());
    push_once(&w, &not_live);
    // Cut the archive short.
    let bytes = std::fs::read(w.set.archive(SID)).unwrap();
    std::fs::write(w.set.archive(SID), &bytes[..bytes.len() / 2]).unwrap();
    let (report, _) = restore(&RestoreInputs {
        set: &w.set,
        passphrase: &pass(),
        target: &w.other,
        ids: &[
            SID.to_string(),
            "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee".to_string(),
        ],
        trust: false,
        is_live: &not_live,
        undo_root: &w.undo,
        job_id: "20261021T101011-0a0b0c0e",
        progress: &Progress::default(),
        kind: "restore",
    })
    .unwrap();
    assert!(report.restored.is_empty());
    let codes: Vec<&str> = report.skipped.iter().map(|s| s.code.as_str()).collect();
    assert_eq!(codes, ["invalid_backup", "not_in_set"]);
    assert!(crate::claude::transfer::transcripts_of(&w.other, SID).is_empty());
}

#[test]
fn restore_warns_when_claude_code_would_prune_it_again() {
    let now = 1_800_000_000_000;
    let day = 24 * 3600 * 1000;
    assert!(prune_warning(now - 31 * day, 30, now)
        .unwrap()
        .contains("cleanupPeriodDays"));
    assert!(prune_warning(now - 29 * day, 30, now).is_none());
    assert!(prune_warning(now - 31 * day, 60, now).is_none());
    // A "keep forever" setting does not overflow into a warning.
    assert!(prune_warning(0, 999_999_999_999, now).is_none());
    assert!(prune_warning(1, i64::MAX, now).is_none());
    let w = world();
    assert_eq!(cleanup_days(&w.other), DEFAULT_CLEANUP_DAYS);
    std::fs::write(w.other.user_settings(), "{\"cleanupPeriodDays\": 90}").unwrap();
    assert_eq!(cleanup_days(&w.other), 90);
}

#[test]
fn archive_entries_outside_the_session_are_refused() {
    for (path, ok) in [
        (format!("projects/-p-app/{SID}.jsonl"), true),
        (
            format!("projects/-p-app/{SID}/subagents/agent-a.jsonl"),
            true,
        ),
        (format!("file-history/{SID}/x"), true),
        (format!("session-env/{SID}"), true),
        (format!("todos/{SID}-agent-x.json"), true),
        ("settings.json".to_string(), false),
        (format!("projects/-p-app/{SID}/../../settings.json"), false),
        (
            "projects/-p-app/aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee.jsonl".to_string(),
            false,
        ),
        (format!("/etc/{SID}"), false),
        (format!("plugins/{SID}"), false),
    ] {
        assert_eq!(allowed_entry(Path::new(&path), SID), ok, "{path}");
    }
}

/// The archives open with the standard `age` tool and `tar` (when `age` is
/// installed): the promise that a backup needs no Mother Claude to read.
#[test]
fn archives_open_with_the_age_command_line_tool() {
    let Ok(out) = std::process::Command::new("age").arg("--version").output() else {
        eprintln!("`age` is not installed; skipped");
        return;
    };
    if !out.status.success() {
        return;
    }
    let w = world();
    session(w.home.base(), SID);
    push_once(&w, &not_live);
    let keys = open_keys(&w.set, &pass()).unwrap();
    let key = w._tmp.path().join("key.txt");
    std::fs::write(
        &key,
        format!("{}\n", keys.identity.to_string().expose_secret()),
    )
    .unwrap();
    let list = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "age -d -i '{}' '{}' | tar tz",
            key.display(),
            w.set.archive(SID).display()
        ))
        .output()
        .unwrap();
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stderr)
    );
    let names = String::from_utf8_lossy(&list.stdout);
    assert!(names.contains(&format!("{SID}.jsonl")), "{names}");
    assert!(names.contains(".mc/meta.json"));
}

fn pull_once(
    w: &World,
    home: &ClaudeHome,
    host: &str,
    live: &dyn Fn(&str) -> bool,
    job: &str,
) -> (PullReport, Manifest) {
    pull(&PullInputs {
        set: &w.set,
        passphrase: &pass(),
        home,
        host,
        ids: None,
        trust: false,
        is_live: live,
        undo_root: &w.undo,
        job_id: job,
        progress: &Progress::default(),
    })
    .unwrap()
}

fn grow(home: &ClaudeHome, id: &str, line: &str) {
    let t = home
        .projects_dir()
        .join(encode_cwd(CWD))
        .join(format!("{id}.jsonl"));
    let mut f = std::fs::File::options().append(true).open(&t).unwrap();
    f.write_all(format!("{line}\n").as_bytes()).unwrap();
    drop(f);
    // A later mtime, whatever the clock's resolution.
    let later = SystemTime::now() + Duration::from_secs(2 + line.len() as u64);
    std::fs::File::options()
        .write(true)
        .open(&t)
        .unwrap()
        .set_modified(later)
        .unwrap();
}

fn push_from(w: &World, home: &ClaudeHome, host: &str) -> PushReport {
    push(&PushInputs {
        home,
        set: &w.set,
        account_name: "Work",
        passphrase: &pass(),
        host,
        is_live: &not_live,
        progress: &Progress::default(),
    })
    .unwrap()
}

/// §5.11 across two machines (two directories, one set).
#[test]
fn pull_brings_new_and_newer_sessions_and_never_merges_a_conflict() {
    let w = world();
    let (a, b) = (&w.home, &w.other);
    session(a.base(), SID);
    std::fs::write(
        a.history_file(),
        "{\"display\":\"hello\",\"timestamp\":1,\"project\":\"/p/app\"}\n",
    )
    .unwrap();
    push_from(&w, a, "mbp-a");

    // B has nothing: the session comes, and the prompt history with it.
    let keys = open_keys(&w.set, &pass()).unwrap();
    let index = read_index(&w.set, &keys).unwrap();
    let plan = compare(&index, b, "mbp-b", &not_live, now_ms());
    assert_eq!(plan[0].action, PullAction::New);
    let (r, _) = pull_once(&w, b, "mbp-b", &not_live, "20261021T120000-00000001");
    assert_eq!(r.restored, vec![SID.to_string()]);
    assert_eq!(r.history_lines, 1);
    assert_eq!(
        std::fs::read_to_string(b.history_file())
            .unwrap()
            .lines()
            .count(),
        1
    );
    // A second pull has nothing to do.
    let (r, _) = pull_once(&w, b, "mbp-b", &not_live, "20261021T120001-00000002");
    assert!(r.restored.is_empty() && r.updated.is_empty() && r.conflicts.is_empty());
    assert_eq!(r.history_lines, 0, "de-duplicated");

    // A continues and pushes; B, unchanged since, is updated.
    grow(a, SID, "{\"type\":\"assistant\",\"n\":1}");
    push_from(&w, a, "mbp-a");
    let (r, m) = pull_once(&w, b, "mbp-b", &not_live, "20261021T120002-00000003");
    assert_eq!(r.updated, vec![SID.to_string()], "{r:?}");
    let t = |h: &ClaudeHome| {
        std::fs::read_to_string(
            h.projects_dir()
                .join(encode_cwd(CWD))
                .join(format!("{SID}.jsonl")),
        )
        .unwrap()
    };
    assert_eq!(t(b), t(a));
    assert!(!m.moved.is_empty(), "the old copy kept for undo");
    // Undo puts B's older copy back.
    crate::claude::transfer::undo(&w.undo, "20261021T120002-00000003").unwrap();
    assert_eq!(t(b).lines().count(), 1);
    // Pull it again for what follows.
    let (r, _) = pull_once(&w, b, "mbp-b", &not_live, "20261021T120003-00000004");
    assert_eq!(r.updated, vec![SID.to_string()]);

    // Both continue: B keeps its own, the set's lands beside it.
    grow(b, SID, "{\"type\":\"user\",\"on\":\"b\"}");
    grow(a, SID, "{\"type\":\"user\",\"on\":\"a, and more\"}");
    push_from(&w, a, "mbp-a");
    let (r, _) = pull_once(&w, b, "mbp-b", &not_live, "20261021T120004-00000005");
    assert!(r.updated.is_empty(), "{r:?}");
    assert_eq!(r.conflicts.len(), 1);
    let (_, beside) = &r.conflicts[0];
    assert!(beside
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with(&format!("{SID}.conflict-mbp-a-")));
    assert_eq!(std::fs::read_to_string(beside).unwrap(), t(a));
    assert!(t(b).contains("\"on\":\"b\""), "the local copy is untouched");

    // A session running on B is never touched.
    grow(
        a,
        SID,
        "{\"type\":\"user\",\"on\":\"a again, longer still\"}",
    );
    push_from(&w, a, "mbp-a");
    let live = |id: &str| id == SID;
    let index = read_index(&w.set, &keys).unwrap();
    assert_eq!(
        compare(&index, b, "mbp-b", &live, now_ms())[0].action,
        PullAction::Live
    );
}

/// Two machines pushing in turn keep each other's entries.
#[test]
fn pushes_from_two_machines_merge_their_index() {
    let w = world();
    let other_sid = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
    session(w.home.base(), SID);
    session(w.other.base(), other_sid);
    push_from(&w, &w.home, "mbp-a");
    push_from(&w, &w.other, "mbp-b");
    let keys = open_keys(&w.set, &pass()).unwrap();
    let index = read_index(&w.set, &keys).unwrap();
    assert_eq!(index.sessions.len(), 2);
    assert!(index.seen.contains_key("mbp-a") && index.seen.contains_key("mbp-b"));
    let info = read_info(&w.set).unwrap();
    let hosts: Vec<&str> = info.machines.iter().map(|m| m.host.as_str()).collect();
    assert_eq!(hosts, ["mbp-a", "mbp-b"]);
}

/// Every file under `dir`, relative, sorted.
fn tree(dir: &Path) -> Vec<String> {
    let mut all = Vec::new();
    let _ = crate::claude::transfer::walk(dir, &mut all);
    let mut out: Vec<String> = all
        .iter()
        .map(|p| p.strip_prefix(dir).unwrap().display().to_string())
        .collect();
    out.sort();
    out
}

/// The security review's proof of concept: an index (forged by anyone who can
/// write the folder) names a session `../../victim`; a pull or a restore must
/// not follow it out of the target directory.
#[test]
fn a_session_id_from_the_index_never_names_a_path_outside_the_target() {
    let w = world();
    let (keys, _) = open_or_create(&w.set, "Work", &pass()).unwrap();
    // <other>/.mc-restore-<job>/../../victim is <tmp>/victim.
    let victim = w._tmp.path().join("victim");
    std::fs::create_dir_all(victim.join("sub")).unwrap();
    std::fs::write(victim.join("a.txt"), "user file A").unwrap();
    std::fs::write(victim.join("sub/b.txt"), "user file B").unwrap();
    let id = "../../victim";
    let archive = seal(&keys.recipient, b"not even an archive").unwrap();
    std::fs::write(
        w.set.sessions_dir().join(format!("{id}.tar.gz.age")),
        &archive,
    )
    .unwrap();
    let entry = IndexEntry {
        id: id.to_string(),
        cwd: None,
        transcript_bytes: 1,
        newest_mtime: 1,
        host: "evil".into(),
        pushed_at: 1,
        archive_sha256: crate::claude::secrets::sha256_hex(&archive),
        archive_bytes: archive.len() as u64,
        files: 0,
    };
    let mut index = SetIndex::default();
    index.sessions.insert(id.to_string(), entry);
    write_index(&w.set, &keys.recipient, &index).unwrap();

    assert!(read_index(&w.set, &keys).unwrap().sessions.is_empty());
    let (r, _) = pull_once(&w, &w.other, "mbp-b", &not_live, "20261021T130000-00000001");
    assert!(r.restored.is_empty() && r.skipped.is_empty(), "{r:?}");
    let (r, _) = restore(&RestoreInputs {
        set: &w.set,
        passphrase: &pass(),
        target: &w.other,
        ids: &[id.to_string()],
        trust: false,
        is_live: &not_live,
        undo_root: &w.undo,
        job_id: "20261021T130001-00000002",
        progress: &Progress::default(),
        kind: "restore",
    })
    .unwrap();
    assert_eq!(r.skipped[0].code, "invalid_id");
    assert_eq!(tree(&victim), ["a.txt", "sub/b.txt"]);
    assert!(
        tree(w.other.base()).is_empty(),
        "{:?}",
        tree(w.other.base())
    );
}

/// A configuration bundle's name from the index is never a path: a push does
/// not delete what it names and "Restore settings" does not unpack there.
#[test]
fn a_bundle_name_from_the_index_never_names_a_path_outside_config() {
    let w = world();
    config(w.home.base());
    let (keys, _) = open_or_create(&w.set, "Work", &pass()).unwrap();
    let precious = w._tmp.path().join("precious.txt");
    std::fs::write(&precious, "important").unwrap();
    let mut index = SetIndex::default();
    for name in [
        precious.display().to_string(),
        "../...tar.gz.age".to_string(),
        "2026-01-01T00-00-00Z.tar.gz.age/../../x.tar.gz.age".to_string(),
    ] {
        index.config.push(ConfigEntry {
            name,
            created_at: 1,
            host: "x".into(),
            fingerprint: "0".into(),
        });
    }
    for i in 0..4 {
        index.config.push(ConfigEntry {
            name: format!("2026-01-0{}T00-00-00Z.tar.gz.age", i + 1),
            created_at: 1,
            host: "x".into(),
            fingerprint: "0".into(),
        });
    }
    write_index(&w.set, &keys.recipient, &index).unwrap();
    assert_eq!(read_index(&w.set, &keys).unwrap().config.len(), 4);
    push_once(&w, &not_live);
    assert!(precious.exists(), "a push deletes only bundles of its own");

    // Only the bundle this push wrote can be unpacked, and only inside `into`.
    let into = w._tmp.path().join("appdata/restored-config/Work");
    let dest = extract_config(&w.set, &pass(), &into).unwrap();
    assert_eq!(dest.parent(), Some(into.as_path()));
    // A second look replaces the first.
    std::fs::write(dest.join("CLAUDE.md"), "edited").unwrap();
    let again = extract_config(&w.set, &pass(), &into).unwrap();
    assert_eq!(again, dest);
    assert_eq!(
        std::fs::read_to_string(dest.join("CLAUDE.md")).unwrap(),
        "be kind"
    );
}

#[test]
fn only_the_names_a_push_writes_are_bundle_names() {
    for (name, ok) in [
        ("2026-10-21T03-09-00Z.tar.gz.age", true),
        ("2026-10-21T03-09-00Z-0123456789ab.tar.gz.age", true),
        ("2026-10-21T03-09-00Z-0123456789AB.tar.gz.age", false),
        ("2026-10-21T03-09-00Z-0123456789a.tar.gz.age", false),
        ("2026-10-21T03-09-00Z", false),
        ("../2026-10-21T03-09-00Z.tar.gz.age", false),
        ("/etc/passwd", false),
        ("2026-10-21T03:09:00Z.tar.gz.age", false),
        ("2026-10-21T03-09-00Zé.tar.gz.age", false),
        ("", false),
    ] {
        assert_eq!(is_config_name(name), ok, "{name}");
    }
}
