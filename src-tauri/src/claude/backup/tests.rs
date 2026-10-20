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

/// The archive the set's index names for `id` now.
fn archive_of(w: &World, id: &str) -> PathBuf {
    let keys = open_keys(&w.set, &pass()).unwrap();
    let index = read_index(&w.set, &keys).unwrap();
    index
        .sessions
        .get(id)
        .and_then(|e| w.set.session_archive(e))
        .unwrap_or_else(|| w.set.sessions_dir().join(format!("{id}.none.tar.gz.age")))
}

/// A machine named `name`, with an id of its own (two machines may share a
/// name: see `two_machines_with_one_name_stay_apart`).
fn machine(name: &str) -> ThisMachine {
    named_machine(name, name)
}

fn named_machine(name: &str, seed: &str) -> ThisMachine {
    ThisMachine {
        id: crate::claude::secrets::sha256_hex(seed.as_bytes())[..32].to_string(),
        name: name.to_string(),
    }
}

fn push_once(w: &World, live: &dyn Fn(&str) -> bool) -> PushReport {
    push(&PushInputs {
        home: &w.home,
        set: &w.set,
        account_name: "Work",
        passphrase: &pass(),
        machine: &machine("mbp-a"),
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
    assert!(archive_of(&w, SID).is_file());
    // Nothing secret or plain in the set but set.json.
    let info = read_info(&w.set).unwrap();
    assert_eq!(info.account, "Work");
    assert_eq!(info.machines[0].host, "mbp-a");
    let all: Vec<u8> = std::fs::read(archive_of(&w, SID)).unwrap();
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
    let dest = extract_config(&w.set, &pass(), &out, None).unwrap();
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
        machine: &machine("mbp-a"),
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
    assert!(!archive_of(&w, SID).exists());
    push_once(&w, &not_live);
    // Cut the archive short.
    let bytes = std::fs::read(archive_of(&w, SID)).unwrap();
    std::fs::write(archive_of(&w, SID), &bytes[..bytes.len() / 2]).unwrap();
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
            archive_of(&w, SID).display()
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
        machine: &machine(host),
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
        machine: &machine(host),
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
    let plan = compare(&index, b, &machine("mbp-b").id, &not_live, now_ms());
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
        compare(&index, b, &machine("mbp-b").id, &live, now_ms())[0].action,
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
    assert!(
        index.seen.contains_key(&machine("mbp-a").id)
            && index.seen.contains_key(&machine("mbp-b").id)
    );
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
    let name = format!("{id}.0123456789ab.tar.gz.age");
    std::fs::write(w.set.sessions_dir().join(&name), &archive).unwrap();
    let entry = IndexEntry {
        id: id.to_string(),
        cwd: None,
        transcript_bytes: 1,
        newest_mtime: 1,
        host: "evil".into(),
        machine: String::new(),
        pushed_at: 1,
        archive: name,
        archive_sha256: crate::claude::secrets::sha256_hex(&archive),
        archive_bytes: archive.len() as u64,
        files: 0,
    };
    let mut index = SetIndex::default();
    index.sessions.insert(id.to_string(), entry);
    write_index(&w.set, &keys, &index).unwrap();

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
    let bundle = |name: String| ConfigEntry {
        name,
        created_at: 1,
        host: "x".into(),
        machine: "f".repeat(32),
        fingerprint: "0".into(),
        archive_sha256: String::new(),
        archive_bytes: 0,
    };
    let mut index = SetIndex::default();
    for name in [
        precious.display().to_string(),
        "../...tar.gz.age".to_string(),
        "2026-01-01T00-00-00Z-0123456789ab.tar.gz.age/../../x.tar.gz.age".to_string(),
    ] {
        index.config.push(bundle(name));
    }
    for i in 0..4 {
        index.config.push(bundle(format!(
            "2026-01-0{}T00-00-00Z-0123456789ab.tar.gz.age",
            i + 1
        )));
    }
    write_index(&w.set, &keys, &index).unwrap();
    assert_eq!(read_index(&w.set, &keys).unwrap().config.len(), 4);
    push_once(&w, &not_live);
    assert!(precious.exists(), "a push deletes only bundles of its own");

    // Only the bundle this push wrote can be unpacked, and only inside `into`.
    let into = w._tmp.path().join("appdata/restored-config/Work");
    let dest = extract_config(&w.set, &pass(), &into, None).unwrap();
    assert_eq!(dest.parent(), Some(into.as_path()));
    // A second look replaces the first.
    std::fs::write(dest.join("CLAUDE.md"), "edited").unwrap();
    let again = extract_config(&w.set, &pass(), &into, None).unwrap();
    assert_eq!(again, dest);
    assert_eq!(
        std::fs::read_to_string(dest.join("CLAUDE.md")).unwrap(),
        "be kind"
    );
}

#[test]
fn only_the_names_a_push_writes_are_bundle_names() {
    for (name, ok) in [
        ("2026-10-21T03-09-00Z.tar.gz.age", false),
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

/// What anyone who can write the folder can do: encrypt to the public key in
/// set.json. Such an index (no HMAC, or a wrong one) is refused everywhere,
/// and a push never writes over it.
#[test]
fn an_index_not_written_with_the_sets_key_is_refused() {
    let w = world();
    session(w.home.base(), SID);
    push_once(&w, &not_live);
    let keys = open_keys(&w.set, &pass()).unwrap();
    let recipient: age::x25519::Recipient = read_info(&w.set).unwrap().recipient.parse().unwrap();
    let forged = serde_json::json!({ "sessions": {}, "config": [], "seen": {} });

    // No HMAC line at all.
    let plain = serde_json::to_vec(&forged).unwrap();
    std::fs::write(w.set.index_path(), seal(&recipient, &plain).unwrap()).unwrap();
    let err = read_index(&w.set, &keys).unwrap_err();
    assert_eq!(err.code(), "backup_unauthenticated");
    // A plausible line with a made-up HMAC.
    let mut plain = format!("{INDEX_MAGIC} {}\n", "ab".repeat(32)).into_bytes();
    plain.extend_from_slice(&serde_json::to_vec(&forged).unwrap());
    let forged_bytes = seal(&recipient, &plain).unwrap();
    std::fs::write(w.set.index_path(), &forged_bytes).unwrap();
    assert_eq!(
        read_index(&w.set, &keys).unwrap_err().code(),
        "backup_unauthenticated"
    );
    // Encrypted to another key entirely.
    let other = age::x25519::Identity::generate().to_public();
    std::fs::write(w.set.index_path(), seal(&other, &plain).unwrap()).unwrap();
    assert_eq!(
        read_index(&w.set, &keys).unwrap_err().code(),
        "backup_unauthenticated"
    );

    std::fs::write(w.set.index_path(), &forged_bytes).unwrap();
    let err = pull(&PullInputs {
        set: &w.set,
        passphrase: &pass(),
        home: &w.other,
        machine: &machine("mbp-b"),
        ids: None,
        trust: false,
        is_live: &not_live,
        undo_root: &w.undo,
        job_id: "20261021T140000-00000001",
        progress: &Progress::default(),
    })
    .unwrap_err();
    assert_eq!(err.code(), "backup_unauthenticated");
    let err = push(&PushInputs {
        home: &w.home,
        set: &w.set,
        account_name: "Work",
        passphrase: &pass(),
        machine: &machine("mbp-a"),
        is_live: &not_live,
        progress: &Progress::default(),
    })
    .unwrap_err();
    assert_eq!(err.code(), "backup_unauthenticated");
    assert_eq!(
        std::fs::read(w.set.index_path()).unwrap(),
        forged_bytes,
        "a push never writes over an index it cannot trust"
    );

    // The genuine index, rewritten with the key, opens again.
    write_index(&w.set, &keys, &SetIndex::default()).unwrap();
    assert!(read_index(&w.set, &keys).is_ok());
}

/// set.json is display only: archives are encrypted to the key derived from
/// identity.age, and a set.json naming another key stops a push.
#[test]
fn set_json_never_chooses_the_key_archives_are_encrypted_to() {
    let w = world();
    session(w.home.base(), SID);
    push_once(&w, &not_live);
    let mut info = read_info(&w.set).unwrap();
    let attacker = age::x25519::Identity::generate();
    info.recipient = attacker.to_public().to_string();
    write_info(&w.set, &info).unwrap();
    grow(&w.home, SID, "{\"type\":\"assistant\"}");
    let err = push(&PushInputs {
        home: &w.home,
        set: &w.set,
        account_name: "Work",
        passphrase: &pass(),
        machine: &machine("mbp-a"),
        is_live: &not_live,
        progress: &Progress::default(),
    })
    .unwrap_err();
    assert_eq!(err.code(), "backup_unauthenticated");

    // A lost set.json is rebuilt from the key.
    std::fs::remove_file(w.set.info_path()).unwrap();
    let r = push_once(&w, &not_live);
    assert_eq!(r.archived, vec![SID.to_string()]);
    let keys = open_keys(&w.set, &pass()).unwrap();
    assert_eq!(
        read_info(&w.set).unwrap().recipient,
        keys.recipient.to_string()
    );
    // The archive opens with the set's key, not the attacker's.
    let bytes = std::fs::read(archive_of(&w, SID)).unwrap();
    assert!(open_sealed(&keys.identity, &bytes).is_ok());
    assert!(open_sealed(&attacker, &bytes).is_err());
}

/// A set whose key is gone (or still in the cloud) is never replaced by a
/// new one: its archives would be unreadable from then on.
#[test]
fn a_set_without_its_key_is_never_given_a_new_one() {
    let w = world();
    session(w.home.base(), SID);
    push_once(&w, &not_live);
    let index_before = std::fs::read(w.set.index_path()).unwrap();
    let identity = std::fs::read(w.set.identity_path()).unwrap();

    // Evicted by iCloud: only a placeholder is left.
    std::fs::remove_file(w.set.identity_path()).unwrap();
    std::fs::write(placeholder(&w.set.identity_path()), "bplist").unwrap();
    for e in [
        push(&PushInputs {
            home: &w.home,
            set: &w.set,
            account_name: "Work",
            passphrase: &pass(),
            machine: &machine("mbp-a"),
            is_live: &not_live,
            progress: &Progress::default(),
        })
        .unwrap_err(),
        open_keys(&w.set, &pass()).err().unwrap(),
    ] {
        assert_eq!(e.code(), "set_key_missing");
    }
    // Gone for good, archives still there: the same.
    std::fs::remove_file(placeholder(&w.set.identity_path())).unwrap();
    assert!(w.set.key_missing());
    assert_eq!(
        open_or_create(&w.set, "Work", &pass())
            .err()
            .unwrap()
            .code(),
        "set_key_missing"
    );
    assert!(!w.set.identity_path().exists());
    assert_eq!(std::fs::read(w.set.index_path()).unwrap(), index_before);

    // Back from the cloud: everything opens as before.
    std::fs::write(w.set.identity_path(), identity).unwrap();
    assert_eq!(push_once(&w, &not_live).unchanged, 1);

    // An empty folder is not a set: a new one is made there.
    let fresh = Set::new(&w._tmp.path().join("iCloud/Mother Claude"), "Fresh");
    std::fs::create_dir_all(fresh.dir.join("sessions")).unwrap();
    std::fs::write(fresh.dir.join(".DS_Store"), "").unwrap();
    assert!(!fresh.key_missing());
    assert!(open_or_create(&fresh, "Fresh", &pass()).is_ok());
}

/// A bundle that is not the one the index names is never decrypted.
#[test]
fn a_changed_configuration_bundle_is_refused() {
    let w = world();
    config(w.home.base());
    let r = push_once(&w, &not_live);
    let name = r.config.unwrap();
    let path = w.set.config_bundle(&name).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.truncate(bytes.len() - 10);
    std::fs::write(&path, bytes).unwrap();
    let into = w._tmp.path().join("review");
    let err = extract_config(&w.set, &pass(), &into, None).unwrap_err();
    assert_eq!(err.code(), "invalid_backup", "{err}");
}

/// Two Macs with one host name (set up from one another, or left on the
/// default) keep their own `seen`: a change on both is a conflict, never an
/// update that replaces one of them.
#[test]
fn two_machines_with_one_name_stay_apart() {
    let w = world();
    let (a, b) = (&w.home, &w.other);
    let ma = named_machine("MacBook-Pro", "first");
    let mb = named_machine("MacBook-Pro", "second");
    assert_ne!(ma.id, mb.id);
    let push_as = |home: &ClaudeHome, m: &ThisMachine| {
        push(&PushInputs {
            home,
            set: &w.set,
            account_name: "Work",
            passphrase: &pass(),
            machine: m,
            is_live: &not_live,
            progress: &Progress::default(),
        })
        .unwrap()
    };
    let pull_as = |home: &ClaudeHome, m: &ThisMachine, job: &str| {
        pull(&PullInputs {
            set: &w.set,
            passphrase: &pass(),
            home,
            machine: m,
            ids: None,
            trust: false,
            is_live: &not_live,
            undo_root: &w.undo,
            job_id: job,
            progress: &Progress::default(),
        })
        .unwrap()
    };
    session(a.base(), SID);
    push_as(a, &ma);
    let (r, _) = pull_as(b, &mb, "20261021T150000-00000001");
    assert_eq!(r.restored, vec![SID.to_string()]);
    grow(b, SID, "{\"type\":\"user\",\"on\":\"b\"}");
    grow(a, SID, "{\"type\":\"user\",\"on\":\"a, which is longer\"}");
    push_as(a, &ma);
    let keys = open_keys(&w.set, &pass()).unwrap();
    let index = read_index(&w.set, &keys).unwrap();
    assert_eq!(
        compare(&index, b, &mb.id, &not_live, now_ms())[0].action,
        PullAction::Conflict
    );
    let (r, _) = pull_as(b, &mb, "20261021T150001-00000002");
    assert!(r.updated.is_empty(), "{r:?}");
    assert_eq!(r.conflicts.len(), 1);
    let info = read_info(&w.set).unwrap();
    assert_eq!(info.machines.len(), 1, "only A pushed");
    assert_eq!(info.machines[0].id.as_deref(), Some(ma.id.as_str()));
}

/// The machine id is made once and kept; a copy of the app's data on other
/// hardware (Migration Assistant, a restored backup) gets an id of its own.
#[test]
fn the_machine_id_is_kept_and_a_copy_on_other_hardware_gets_its_own() {
    let tmp = tempfile::tempdir().unwrap();
    let hw = Some("aa".repeat(32));
    let first = ops::machine_id_in(tmp.path(), hw.clone()).unwrap();
    assert!(is_machine_id(&first));
    assert_eq!(ops::machine_id_in(tmp.path(), hw.clone()).unwrap(), first);
    // No hardware id on this platform: the stored one stays.
    assert_eq!(ops::machine_id_in(tmp.path(), None).unwrap(), first);
    let moved = ops::machine_id_in(tmp.path(), Some("bb".repeat(32))).unwrap();
    assert_ne!(moved, first);
    assert_eq!(
        ops::machine_id_in(tmp.path(), Some("bb".repeat(32))).unwrap(),
        moved
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(tmp.path().join("backup-machine.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    // The hardware's id itself is never stored.
    #[cfg(target_os = "macos")]
    if let Some(h) = ops::hardware_fingerprint() {
        assert_eq!(h.len(), 64);
    }
}

/// Archives are never rewritten: a second push writes a new file, the old one
/// stays a day (another machine may be reading an index that names it), then
/// goes; a file nothing named (a push that crashed) goes a day after it was
/// written.
#[test]
fn archives_are_never_rewritten_and_unnamed_ones_go_a_day_later() {
    let w = world();
    session(w.home.base(), SID);
    push_once(&w, &not_live);
    let first = archive_of(&w, SID);
    let first_bytes = std::fs::read(&first).unwrap();
    grow(&w.home, SID, "{\"type\":\"assistant\"}");
    assert_eq!(push_once(&w, &not_live).archived, vec![SID.to_string()]);
    let second = archive_of(&w, SID);
    assert_ne!(first, second);
    assert_eq!(std::fs::read(&first).unwrap(), first_bytes, "untouched");
    let keys = open_keys(&w.set, &pass()).unwrap();
    let index = read_index(&w.set, &keys).unwrap();
    let name = |p: &Path| p.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(index.superseded.len(), 1);
    assert_eq!(
        index.superseded[0].path,
        format!("sessions/{}", name(&first))
    );

    // A crash between an archive and the index: a file nothing names.
    let orphan = w
        .set
        .sessions_dir()
        .join(format!("{SID}.0123456789ab.tar.gz.age"));
    std::fs::write(&orphan, b"half").unwrap();
    let temp = w.set.sessions_dir().join(".archive.mc-0a0b.tmp");
    std::fs::write(&temp, b"half").unwrap();
    // Not someone else's file, whatever its age.
    let stranger = w.set.sessions_dir().join("notes.txt");
    std::fs::write(&stranger, b"mine").unwrap();
    let now = now_ms();
    assert_eq!(collect_garbage(&w.set, &index, now), 0, "all within a day");
    let later = now + GRACE_MS + 60_000;
    let set_old = |p: &Path| {
        std::fs::File::options()
            .write(true)
            .open(p)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(2 * 24 * 3600))
            .unwrap();
    };
    set_old(&orphan);
    set_old(&temp);
    set_old(&stranger);
    assert_eq!(collect_garbage(&w.set, &index, later), 3);
    assert!(!first.exists() && !orphan.exists() && !temp.exists());
    assert!(second.exists() && stranger.exists());
    // The next index written after the day forgets what is gone.
    let merged = write_index_merged(
        &w.set,
        &keys,
        &Changes::default(),
        &machine("mbp-a").id,
        later,
    )
    .unwrap();
    assert!(
        merged.index.superseded.is_empty(),
        "{:?}",
        merged.index.superseded
    );
}

/// Two machines push the same session at once: the one whose push was made
/// from an entry the set no longer has loses, its archive is removed and it
/// reports a conflict; the other machine's copy stands.
#[test]
fn a_push_made_from_an_old_entry_never_replaces_a_newer_one() {
    let w = world();
    let (a, b) = (&w.home, &w.other);
    session(a.base(), SID);
    push_from(&w, a, "mbp-a");
    let keys = open_keys(&w.set, &pass()).unwrap();
    let before = read_index(&w.set, &keys).unwrap();
    let base = before.sessions[SID].archive.clone();
    // B pulls, continues and pushes while A is in the middle of a push made
    // from the index it read before.
    pull_once(&w, b, "mbp-b", &not_live, "20261021T160000-00000001");
    grow(b, SID, "{\"type\":\"user\",\"on\":\"b\"}");
    push_from(&w, b, "mbp-b");
    let theirs = read_index(&w.set, &keys).unwrap().sessions[SID].clone();
    assert_ne!(theirs.archive, base);

    grow(a, SID, "{\"type\":\"user\",\"on\":\"a\"}");
    let ma = machine("mbp-a");
    let inp = PushInputs {
        home: a,
        set: &w.set,
        account_name: "Work",
        passphrase: &pass(),
        machine: &ma,
        is_live: &not_live,
        progress: &Progress::default(),
    };
    let now = session_now(a, SID).unwrap();
    let mine = archive_session(&inp, &keys, SID, &now).unwrap();
    let mine_path = w.set.session_archive(&mine).unwrap();
    assert!(mine_path.exists());
    let mut changes = Changes::default();
    changes
        .sessions
        .insert(SID.to_string(), (mine, Some(base.clone())));
    let merged = write_index_merged(&w.set, &keys, &changes, &ma.id, now_ms()).unwrap();
    assert_eq!(merged.lost, vec![SID.to_string()]);
    assert_eq!(merged.index.sessions[SID], theirs, "B's push stands");
    assert!(
        !mine_path.exists(),
        "A's archive is named by nothing: removed"
    );
    assert!(w.set.session_archive(&theirs).unwrap().exists());
}

/// A file of the configuration that cannot be read is left out of the
/// bundle with a warning; the sessions are indexed all the same.
#[cfg(unix)]
#[test]
fn an_unreadable_configuration_file_never_costs_the_index() {
    use std::os::unix::fs::PermissionsExt;
    let w = world();
    session(w.home.base(), SID);
    config(w.home.base());
    let locked = w.home.base().join("settings.json");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::File::open(&locked).is_ok() {
        // Running as root: nothing is unreadable.
        return;
    }
    let r = push_once(&w, &not_live);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(r.archived, vec![SID.to_string()]);
    assert!(r.config.is_some());
    assert!(
        r.warnings.iter().any(|w| w.contains("settings.json")),
        "{:?}",
        r.warnings
    );
    assert!(archive_of(&w, SID).is_file());
    let (report, _) = restore(&RestoreInputs {
        set: &w.set,
        passphrase: &pass(),
        target: &w.other,
        ids: &[SID.to_string()],
        trust: false,
        is_live: &not_live,
        undo_root: &w.undo,
        job_id: "20261021T170000-00000001",
        progress: &Progress::default(),
        kind: "restore",
    })
    .unwrap();
    assert_eq!(report.restored, vec![SID.to_string()]);
}

fn transcript(home: &ClaudeHome, id: &str) -> PathBuf {
    home.projects_dir()
        .join(encode_cwd(CWD))
        .join(format!("{id}.jsonl"))
}

/// Sub-second times, as APFS and NTFS keep them (a tar keeps whole seconds).
fn set_mtime_ms(path: &Path, ms: u64) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::UNIX_EPOCH + Duration::from_millis(ms))
        .unwrap();
}

/// The correctness review's second case: a pulled copy has whole-second
/// times, the pushed one had milliseconds. It compares the same, and two
/// machines pushing in turn archive nothing.
#[test]
fn a_pulled_session_compares_the_same_and_nothing_goes_back_and_forth() {
    let w = world();
    let (a, b) = (&w.home, &w.other);
    session(a.base(), SID);
    set_mtime_ms(&transcript(a, SID), 1_760_000_100_500);
    set_mtime_ms(
        &a.projects_dir()
            .join(encode_cwd(CWD))
            .join(SID)
            .join("subagents/agent-a1.jsonl"),
        1_760_000_000_250,
    );
    push_from(&w, a, "mbp-a");
    pull_once(&w, b, "mbp-b", &not_live, "20261021T180000-00000001");
    let keys = open_keys(&w.set, &pass()).unwrap();
    let index = read_index(&w.set, &keys).unwrap();
    assert_eq!(
        compare(&index, b, &machine("mbp-b").id, &not_live, now_ms())[0].action,
        PullAction::Same
    );
    for _ in 0..3 {
        assert!(push_from(&w, b, "mbp-b").archived.is_empty());
        assert!(push_from(&w, a, "mbp-a").archived.is_empty());
    }
}

/// The correctness review's first case: A pushes, B pulls, continues and
/// pushes; A, idle and not pulled since, must not put its older copy back
/// over B's. A pull brings B's copy to A.
#[test]
fn a_push_never_replaces_a_copy_another_machine_pushed_since() {
    let w = world();
    let (a, b) = (&w.home, &w.other);
    session(a.base(), SID);
    push_from(&w, a, "mbp-a");
    pull_once(&w, b, "mbp-b", &not_live, "20261021T181000-00000001");
    grow(b, SID, "{\"type\":\"user\",\"on\":\"b\"}");
    assert_eq!(push_from(&w, b, "mbp-b").archived, vec![SID.to_string()]);
    let keys = open_keys(&w.set, &pass()).unwrap();
    let theirs = read_index(&w.set, &keys).unwrap().sessions[SID].clone();

    let r = push_from(&w, a, "mbp-a");
    assert!(r.archived.is_empty(), "{r:?}");
    assert_eq!(r.newer_in_set, vec![SID.to_string()]);
    assert_eq!(read_index(&w.set, &keys).unwrap().sessions[SID], theirs);
    let (r, _) = pull_once(&w, a, "mbp-a", &not_live, "20261021T181001-00000002");
    assert_eq!(r.updated, vec![SID.to_string()], "{r:?}");
    assert_eq!(
        std::fs::read(transcript(a, SID)).unwrap(),
        std::fs::read(transcript(b, SID)).unwrap()
    );
}

/// Changed on both machines: neither push replaces the other's copy, a pull
/// puts the other copy beside, and once one side continues, the other side
/// gets that copy beside its own again rather than in its place.
#[test]
fn a_session_changed_on_both_machines_is_never_lost_by_either() {
    let w = world();
    let (a, b) = (&w.home, &w.other);
    session(a.base(), SID);
    push_from(&w, a, "mbp-a");
    pull_once(&w, b, "mbp-b", &not_live, "20261021T182000-00000001");
    grow(a, SID, "{\"type\":\"user\",\"on\":\"a\"}");
    grow(b, SID, "{\"type\":\"user\",\"on\":\"b, a longer line\"}");
    assert_eq!(push_from(&w, b, "mbp-b").archived, vec![SID.to_string()]);
    let r = push_from(&w, a, "mbp-a");
    assert_eq!(r.conflicts, vec![SID.to_string()], "{r:?}");
    let mine_a = std::fs::read(transcript(a, SID)).unwrap();

    // A pulls: B's copy beside A's, A's untouched; then nothing more to do.
    let (r, _) = pull_once(&w, a, "mbp-a", &not_live, "20261021T182001-00000002");
    assert_eq!(r.conflicts.len(), 1, "{r:?}");
    assert_eq!(std::fs::read(transcript(a, SID)).unwrap(), mine_a);
    let (r, _) = pull_once(&w, a, "mbp-a", &not_live, "20261021T182002-00000003");
    assert!(r.conflicts.is_empty() && r.updated.is_empty(), "{r:?}");
    assert!(push_from(&w, a, "mbp-a").archived.is_empty());

    // A continues: its copy goes to the set; B, unchanged since its push,
    // does not get it in place of its own (it did not come from B's).
    grow(a, SID, "{\"type\":\"assistant\",\"on\":\"a again\"}");
    assert_eq!(push_from(&w, a, "mbp-a").archived, vec![SID.to_string()]);
    let mine_b = std::fs::read(transcript(b, SID)).unwrap();
    let (r, _) = pull_once(&w, b, "mbp-b", &not_live, "20261021T182003-00000004");
    assert!(r.updated.is_empty(), "{r:?}");
    assert_eq!(r.conflicts.len(), 1);
    assert_eq!(std::fs::read(transcript(b, SID)).unwrap(), mine_b);
    assert_eq!(
        std::fs::read(&r.conflicts[0].1).unwrap(),
        std::fs::read(transcript(a, SID)).unwrap()
    );
}

/// The correctness review's eighth case: an update that fails half-way puts
/// the local copy back and leaves nothing listed for undo.
#[test]
fn an_update_that_fails_half_way_puts_the_local_copy_back() {
    let w = world();
    let (a, b) = (&w.home, &w.other);
    session(a.base(), SID);
    push_from(&w, a, "mbp-a");
    pull_once(&w, b, "mbp-b", &not_live, "20261021T183000-00000001");
    grow(a, SID, "{\"type\":\"assistant\",\"n\":2}");
    push_from(&w, a, "mbp-a");
    let before = tree(b.base());
    let bytes = std::fs::read(transcript(b, SID)).unwrap();
    // The transcript and the subagent file move out, then file-history
    // cannot (a file where its folder in the undo would go).
    let job = "20261021T183001-00000002";
    std::fs::create_dir_all(w.undo.join(job).join("moved")).unwrap();
    std::fs::write(w.undo.join(job).join("moved/file-history"), "in the way").unwrap();
    let (r, m) = pull_once(&w, b, "mbp-b", &not_live, job);
    assert!(r.updated.is_empty(), "{r:?}");
    assert_eq!(r.skipped.len(), 1, "{r:?}");
    assert!(
        r.skipped[0].reason.contains("moved/file-history"),
        "it failed after the first files moved: {r:?}"
    );
    assert_eq!(tree(b.base()), before);
    assert_eq!(std::fs::read(transcript(b, SID)).unwrap(), bytes);
    assert!(m.moved.is_empty() && m.created.is_empty(), "{m:?}");
    // The next pull (nothing in the way) updates it.
    let (r, _) = pull_once(&w, b, "mbp-b", &not_live, "20261021T183002-00000003");
    assert_eq!(r.updated, vec![SID.to_string()]);
}

/// A session restored (not pulled) and continued here: the set's copy is
/// where it came from, so a push sends it; one that did not come from it is
/// a conflict.
#[test]
fn a_restored_and_continued_session_is_pushed_when_it_came_from_the_set() {
    let w = world();
    let (a, b) = (&w.home, &w.other);
    session(a.base(), SID);
    push_from(&w, a, "mbp-a");
    restore(&RestoreInputs {
        set: &w.set,
        passphrase: &pass(),
        target: b,
        ids: &[SID.to_string()],
        trust: false,
        is_live: &not_live,
        undo_root: &w.undo,
        job_id: "20261021T184000-00000001",
        progress: &Progress::default(),
        kind: "restore",
    })
    .unwrap();
    grow(b, SID, "{\"type\":\"user\",\"on\":\"b\"}");
    assert_eq!(push_from(&w, b, "mbp-b").archived, vec![SID.to_string()]);

    // A session with the same id that did not come from the set's copy.
    let other_sid = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
    session(a.base(), other_sid);
    push_from(&w, a, "mbp-a");
    session(b.base(), other_sid);
    std::fs::write(
        transcript(b, other_sid),
        "{\"type\":\"user\",\"made\":\"elsewhere, and longer than A's\"}\n",
    )
    .unwrap();
    let r = push_from(&w, b, "mbp-b");
    assert_eq!(r.conflicts, vec![other_sid.to_string()], "{r:?}");
}
