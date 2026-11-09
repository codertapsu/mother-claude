//! Folder pairs (docs/BACKUP.md "Folder pairs"): a MacBook and a Windows
//! desktop share one set; `/Users/marcus/dev` on the Mac is `D:\dev` on
//! Windows. Two homes and two machine ids stand for the two machines, the
//! Windows side's cwds written as Windows writes them (this runs on a Mac:
//! everything here is decided on strings, never on this machine's paths).

use super::*;
use crate::claude::paths::{project_dir_name, view_cwd, LocalPair};

const MAC_DEV: &str = "/Users/marcus/dev";
const WIN_DEV: &str = r"D:\dev";
const MAC_APP: &str = "/Users/marcus/dev/app";
const WIN_APP: &str = r"D:\dev\app";

fn mac() -> ThisMachine {
    named_machine("marcus-mbp", "the mac")
}

fn win() -> ThisMachine {
    named_machine("DESKTOP-PC", "the windows desktop")
}

fn transcript_in(home: &ClaudeHome, id: &str, cwd: &str) -> PathBuf {
    home.projects_dir()
        .join(project_dir_name(cwd))
        .join(format!("{id}.jsonl"))
}

/// A session started in `cwd`, as Claude Code keeps it there.
fn session_in(home: &ClaudeHome, id: &str, cwd: &str) {
    let t = transcript_in(home, id, cwd);
    std::fs::create_dir_all(t.parent().unwrap().join(id).join("subagents")).unwrap();
    let rec = serde_json::json!({"type": "user", "cwd": cwd, "sessionId": id});
    std::fs::write(&t, format!("{rec}\n")).unwrap();
    std::fs::write(
        t.parent()
            .unwrap()
            .join(id)
            .join("subagents/agent-a1.jsonl"),
        "{}\n",
    )
    .unwrap();
}

/// Continue a session (wherever its transcript is) in `cwd`.
fn continue_in(home: &ClaudeHome, id: &str, cwd: &str, text: &str) {
    let t = transfer::transcripts_of(home, id).remove(0);
    let rec = serde_json::json!({"type": "user", "cwd": cwd, "sessionId": id, "text": text});
    let mut f = std::fs::File::options().append(true).open(&t).unwrap();
    f.write_all(format!("{rec}\n").as_bytes()).unwrap();
    drop(f);
    let later = SystemTime::now() + Duration::from_secs(2 + text.len() as u64);
    std::fs::File::options()
        .write(true)
        .open(&t)
        .unwrap()
        .set_modified(later)
        .unwrap();
}

fn push_as(w: &World, home: &ClaudeHome, m: &ThisMachine) -> PushReport {
    push(&PushInputs {
        home,
        set: &w.set,
        account_name: "Work",
        title_of: &no_title,
        passphrases: &[pass()],
        machine: m,
        is_live: &not_live,
        progress: &Progress::default(),
    })
    .unwrap()
}

fn pull_as(w: &World, home: &ClaudeHome, m: &ThisMachine, job: &str, trust: bool) -> PullReport {
    pull(&PullInputs {
        set: &w.set,
        passphrases: &[pass()],
        home,
        machine: m,
        ids: None,
        trust,
        is_live: &not_live,
        undo_root: &w.undo,
        job_id: job,
        progress: &Progress::default(),
        force: false,
    })
    .unwrap()
    .0
}

fn new_pair(
    id: &str,
    by: &ThisMachine,
    mine: &str,
    other: &ThisMachine,
    theirs: &str,
) -> pairs::FolderPair {
    pairs::FolderPair {
        id: id.to_string(),
        folders: [
            (by.id.clone(), mine.to_string()),
            (other.id.clone(), theirs.to_string()),
        ]
        .into_iter()
        .collect(),
        created_at: now_ms(),
        by: by.id.clone(),
        removed: None,
        extra: BTreeMap::new(),
    }
}

/// What `ops::add_pair` writes, on the engine: the pair merged into the set.
fn add_pair(w: &World, by: &ThisMachine, pair: pairs::FolderPair) {
    let keys = open_keys(&w.set, &pass()).unwrap();
    let changes = Changes {
        pairs: vec![pair],
        machine_info: Some(pairs::MachineInfo::here(&by.name, now_ms())),
        ..Changes::default()
    };
    write_index_merged(&w.set, &keys, &changes, &by.id, now_ms()).unwrap();
}

fn dir_of(home: &ClaudeHome, id: &str) -> String {
    let t = transfer::transcripts_of(home, id);
    assert_eq!(t.len(), 1, "one transcript: {t:?}");
    t[0].parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

fn view_on(home: &ClaudeHome, id: &str, local: &[LocalPair]) -> Option<String> {
    let t = transfer::transcripts_of(home, id).remove(0);
    let first = transfer::recorded_cwd(&t);
    let last = last_recorded_cwd(&t);
    let enc = t
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    view_cwd(first.as_deref(), last.as_deref(), Some(&enc), local)
}

/// The user's round trip: the Mac backs up, Windows pulls before a pair
/// exists (the session keeps the Mac's path, and the report says so), the
/// pair is added, the next pull moves it into `D:\dev\app`'s folder (where
/// Claude Code on Windows lists it), Windows continues it and backs up, the
/// Mac pulls it back into its own project folder, continues, and Windows
/// gets it in its folder again — the bytes the same on both, and nothing
/// going back and forth.
#[test]
fn a_session_goes_mac_to_windows_and_back_through_a_folder_pair() {
    let w = world();
    let (m, p) = (&w.home, &w.other);
    let (mac, win) = (mac(), win());
    session_in(m, SID, MAC_APP);
    assert_eq!(push_as(&w, m, &mac).archived, [SID]);
    let entry = index_of(&w).sessions[SID].clone();
    assert_eq!(
        entry.folder.as_deref(),
        Some(MAC_APP),
        "where it lives on the Mac"
    );
    assert_eq!(index_of(&w).machines[&mac.id].os, std::env::consts::OS);

    // Windows pulls with no pair: the Mac's path, and the report names it.
    let r = pull_as(&w, p, &win, "20261106T140000-00000001", false);
    assert_eq!(r.restored, [SID]);
    assert!(r.mapped.is_empty());
    assert_eq!(r.foreign.len(), 1, "{r:?}");
    assert_eq!(r.foreign[0].folder, MAC_APP);
    assert_eq!(r.foreign[0].machine, mac.id);
    assert_eq!(r.foreign[0].host, "marcus-mbp");
    assert_eq!(dir_of(p, SID), project_dir_name(MAC_APP));

    // The pair, made on Windows; the next pull moves the session there.
    add_pair(
        &w,
        &win,
        new_pair("fp_0123456789abcdef", &win, WIN_DEV, &mac, MAC_DEV),
    );
    let job = "20261106T140001-00000002";
    let r = pull_as(&w, p, &win, job, false);
    assert_eq!(r.relocated, [SID], "{r:?}");
    assert_eq!(r.mapped, [(SID.to_string(), WIN_APP.to_string())]);
    assert_eq!(dir_of(p, SID), "D--dev-app");
    assert!(!p
        .projects_dir()
        .join(project_dir_name(MAC_APP))
        .join(SID)
        .exists());
    assert_eq!(
        std::fs::read(transcript_in(p, SID, WIN_APP)).unwrap(),
        std::fs::read(transcript_in(m, SID, MAC_APP)).unwrap(),
        "the transcript is never rewritten"
    );
    // Windows shows (and continues) it in its own folder.
    let win_pairs = pairs::local_pairs(&index_of(&w).folder_pairs, &win.id);
    assert_eq!(view_on(p, SID, &win_pairs).as_deref(), Some(WIN_APP));
    // The move's undo puts it back where it was; the next pull moves it again.
    transfer::undo(&w.undo, job).unwrap();
    assert_eq!(dir_of(p, SID), project_dir_name(MAC_APP));
    assert!(!p.projects_dir().join("D--dev-app").join(SID).exists());
    let r = pull_as(&w, p, &win, "20261106T140001-0000000a", false);
    assert_eq!(r.relocated, [SID], "{r:?}");
    assert_eq!(dir_of(p, SID), "D--dev-app");
    // Nothing more to move, and nothing to push.
    let r = pull_as(&w, p, &win, "20261106T140002-00000003", false);
    assert!(r.relocated.is_empty() && r.updated.is_empty(), "{r:?}");
    assert!(push_as(&w, p, &win).archived.is_empty());

    // Windows continues it in D:\dev\app and backs up: the set says where it
    // lives there, not what its first line says.
    continue_in(p, SID, WIN_APP, "on windows");
    assert_eq!(push_as(&w, p, &win).archived, [SID]);
    let entry = index_of(&w).sessions[SID].clone();
    assert_eq!(entry.cwd.as_deref(), Some(MAC_APP));
    assert_eq!(entry.folder.as_deref(), Some(WIN_APP));
    assert_eq!(entry.machine, win.id);

    // The Mac pulls it back into its own project folder.
    let r = pull_as(&w, m, &mac, "20261106T140003-00000004", false);
    assert_eq!(r.updated, [SID], "{r:?}");
    assert_eq!(r.mapped, [(SID.to_string(), MAC_APP.to_string())]);
    assert_eq!(dir_of(m, SID), project_dir_name(MAC_APP));
    assert_eq!(
        std::fs::read(transcript_in(m, SID, MAC_APP)).unwrap(),
        std::fs::read(transcript_in(p, SID, WIN_APP)).unwrap()
    );
    let mac_pairs = pairs::local_pairs(&index_of(&w).folder_pairs, &mac.id);
    assert_eq!(view_on(m, SID, &mac_pairs).as_deref(), Some(MAC_APP));

    // The Mac continues and backs up; Windows gets it in its folder again.
    continue_in(m, SID, MAC_APP, "on the mac again");
    let r = push_as(&w, m, &mac);
    assert_eq!(r.archived, [SID]);
    assert_eq!(index_of(&w).sessions[SID].folder.as_deref(), Some(MAC_APP));
    let r = pull_as(&w, p, &win, "20261106T140004-00000005", false);
    assert_eq!(r.updated, [SID], "{r:?}");
    assert_eq!(dir_of(p, SID), "D--dev-app");
    assert_eq!(view_on(p, SID, &win_pairs).as_deref(), Some(WIN_APP));
    for _ in 0..2 {
        assert!(push_as(&w, p, &win).archived.is_empty());
        assert!(push_as(&w, m, &mac).archived.is_empty());
    }
}

/// A session started on Windows comes to the Mac in the Mac's project
/// folder with a pair, and in Windows' folder name without one (named in
/// the report); a restore and a copy out of the set map it the same way.
#[test]
fn a_windows_session_lands_in_the_paired_folder_or_keeps_its_path() {
    let w = world();
    let (m, p) = (&w.home, &w.other);
    let (mac, win) = (mac(), win());
    let site_win = r"D:\dev\site";
    let site_mac = "/Users/marcus/dev/site";
    session_in(p, OTHER, site_win);
    push_as(&w, p, &win);
    assert_eq!(
        index_of(&w).sessions[OTHER].folder.as_deref(),
        Some(site_win)
    );

    // No pair: Windows' folder name, and the report says which project.
    let r = pull_as(&w, m, &mac, "20261106T150000-00000001", false);
    assert_eq!(r.restored, [OTHER]);
    assert_eq!(r.foreign[0].folder, site_win);
    assert_eq!(dir_of(m, OTHER), "D--dev-site");
    transfer::undo(&w.undo, "20261106T150000-00000001").unwrap();
    assert!(transfer::transcripts_of(m, OTHER).is_empty());

    // With the pair (made on the Mac this time).
    add_pair(
        &w,
        &mac,
        new_pair("fp_1111111111111111", &mac, MAC_DEV, &win, WIN_DEV),
    );
    let r = pull_as(&w, m, &mac, "20261106T150001-00000002", false);
    assert_eq!(r.restored, [OTHER]);
    assert!(r.foreign.is_empty(), "{r:?}");
    assert_eq!(r.mapped, [(OTHER.to_string(), site_mac.to_string())]);
    assert_eq!(dir_of(m, OTHER), project_dir_name(site_mac));
    let mac_pairs = pairs::local_pairs(&index_of(&w).folder_pairs, &mac.id);
    assert_eq!(view_on(m, OTHER, &mac_pairs).as_deref(), Some(site_mac));

    // A restore into another directory, and Move data's copy out of the set.
    let other_home = ClaudeHome::for_account_dir(w._tmp.path().join("claude-third"));
    std::fs::create_dir_all(other_home.base()).unwrap();
    let (r, _) = restore(&RestoreInputs {
        set: &w.set,
        passphrases: &[pass()],
        target: &other_home,
        ids: &[OTHER.to_string()],
        trust: false,
        is_live: &not_live,
        undo_root: &w.undo,
        job_id: "20261106T150002-00000003",
        progress: &Progress::default(),
        kind: "restore",
        machine: Some(&mac.id),
    })
    .unwrap();
    assert_eq!(r.mapped, [(OTHER.to_string(), site_mac.to_string())]);
    assert_eq!(dir_of(&other_home, OTHER), project_dir_name(site_mac));
    transfer::undo(&w.undo, "20261106T150002-00000003").unwrap();
    let (r, _) = move_out_of_set(
        &RestoreInputs {
            set: &w.set,
            passphrases: &[pass()],
            target: &other_home,
            ids: &[OTHER.to_string()],
            trust: false,
            is_live: &not_live,
            undo_root: &w.undo,
            job_id: "20261106T150003-00000004",
            progress: &Progress::default(),
            kind: "copy",
            machine: Some(&mac.id),
        },
        false,
        now_ms(),
        transfer::StorageRef {
            storage: "backup".into(),
            set: "Work".into(),
            dir: w.set.dir.clone(),
            into: false,
        },
        None,
        false,
    )
    .unwrap();
    assert_eq!(r.restored, [OTHER]);
    assert_eq!(dir_of(&other_home, OTHER), project_dir_name(site_mac));
    // The dry run says the same.
    let fresh = ClaudeHome::for_account_dir(w._tmp.path().join("claude-fourth"));
    std::fs::create_dir_all(fresh.base()).unwrap();
    let ((entries, _, _), mapped) = plan_restore_here(
        &w.set,
        &[pass()],
        &fresh,
        Some(&[OTHER.to_string()]),
        &[],
        &not_live,
        Some(&mac.id),
        false,
    )
    .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(mapped.get(OTHER).map(String::as_str), Some(site_mac));
}

/// The trust a restore carries goes to the folder here, under the key
/// Claude Code files it under there; the prompt history's lines name the
/// project here too.
#[test]
fn trust_and_prompt_history_follow_the_pair() {
    let w = world();
    let (m, p) = (&w.home, &w.other);
    let (mac, win) = (mac(), win());
    session_in(m, SID, MAC_APP);
    std::fs::write(
        m.claude_json(),
        serde_json::json!({"projects": {MAC_APP: {"hasTrustDialogAccepted": true, "allowedTools": []}}})
            .to_string(),
    )
    .unwrap();
    std::fs::write(
        m.history_file(),
        format!(
            "{}\n{}\n",
            serde_json::json!({"display": "fix it", "timestamp": 1, "project": MAC_APP, "sessionId": SID}),
            serde_json::json!({"display": "elsewhere", "timestamp": 2, "project": "/Users/marcus/other"}),
        ),
    )
    .unwrap();
    push_as(&w, m, &mac);
    add_pair(
        &w,
        &win,
        new_pair("fp_2222222222222222", &win, WIN_DEV, &mac, MAC_DEV),
    );
    let job = "20261106T160000-00000001";
    let r = pull_as(&w, p, &win, job, true);
    assert_eq!(r.restored, [SID]);
    assert_eq!(r.history_lines, 2);
    let config: Value =
        serde_json::from_str(&std::fs::read_to_string(p.claude_json()).unwrap()).unwrap();
    assert_eq!(
        config["projects"]["D:/dev/app"]["hasTrustDialogAccepted"], true,
        "{config}"
    );
    assert!(config["projects"].get(MAC_APP).is_none());
    let history = std::fs::read_to_string(p.history_file()).unwrap();
    let lines: Vec<Value> = history
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines[0]["project"], WIN_APP);
    assert_eq!(lines[0]["display"], "fix it");
    assert_eq!(lines[0]["sessionId"], SID);
    assert_eq!(
        lines[1]["project"], "/Users/marcus/other",
        "no pair: as it was"
    );
    // A second pull adds no line twice.
    let r = pull_as(&w, p, &win, "20261106T160001-00000002", true);
    assert_eq!(r.history_lines, 0);
    // The undo takes the trust and the lines back out.
    transfer::undo(&w.undo, "20261106T160001-00000002").unwrap();
    transfer::undo(&w.undo, job).unwrap();
    let config: Value =
        serde_json::from_str(&std::fs::read_to_string(p.claude_json()).unwrap()).unwrap();
    assert!(config["projects"].get("D:/dev/app").is_none(), "{config}");
}

/// The trust a restore carries is keyed the way Claude Code here keys the
/// folder it went into: one no pair maps as this machine's CLI keys a
/// folder, whoever recorded it. It never goes over an entry the target has
/// under another spelling, which the undo then leaves alone.
#[test]
fn trust_is_keyed_as_claude_code_here_keys_it_never_over_another_spelling() {
    use crate::claude::paths::{config_key, PathForm};
    let w = world();
    let (m, p) = (&w.home, &w.other);
    let (mac, win) = (mac(), win());
    let read = |h: &ClaudeHome| -> Value {
        serde_json::from_str(&std::fs::read_to_string(h.claude_json()).unwrap()).unwrap()
    };
    // Windows' session, trusted there under Windows' own key.
    session_in(p, SID, WIN_APP);
    std::fs::write(
        p.claude_json(),
        serde_json::json!({"projects": {"D:/dev/app": {"hasTrustDialogAccepted": true}}})
            .to_string(),
    )
    .unwrap();
    push_as(&w, p, &win);
    // No pair: the Mac keeps Windows' folder, keyed the way this machine's
    // CLI would key it (as recorded on a Mac, with `/` on Windows).
    let r = pull_as(&w, m, &mac, "20261106T170000-00000001", true);
    assert_eq!(r.restored, [SID]);
    let key = config_key(WIN_APP, PathForm::here());
    assert_eq!(
        read(m)["projects"][&key]["hasTrustDialogAccepted"],
        true,
        "{}",
        read(m)
    );
    assert_eq!(read(m)["projects"].as_object().unwrap().len(), 1);

    // The Mac's session, trusted there; Windows has the folder it maps to
    // under another spelling already (another case, `/`): nothing is added
    // and nothing it has is changed — nor taken out by the undo.
    session_in(m, OTHER, MAC_APP);
    std::fs::write(
        m.claude_json(),
        serde_json::json!({"projects": {MAC_APP: {"hasTrustDialogAccepted": true}}}).to_string(),
    )
    .unwrap();
    push_as(&w, m, &mac);
    add_pair(
        &w,
        &win,
        new_pair("fp_3333333333333333", &win, WIN_DEV, &mac, MAC_DEV),
    );
    let theirs =
        serde_json::json!({"projects": {"d:/DEV/App": {"hasTrustDialogAccepted": false, "x": 1}}});
    std::fs::write(p.claude_json(), theirs.to_string()).unwrap();
    let job = "20261106T170001-00000002";
    let r = pull_as(&w, p, &win, job, true);
    assert_eq!(r.restored, [OTHER]);
    assert_eq!(r.mapped, [(OTHER.to_string(), WIN_APP.to_string())]);
    assert_eq!(read(p), theirs);
    transfer::undo(&w.undo, job).unwrap();
    assert_eq!(read(p), theirs);
}

/// Two machines edit the pairs at once, each writing only its own change
/// to the index as it is then: both pairs stay, a removal sticks, and each
/// machine's line (its OS) is in the index; an index without pairs reads as
/// before and keeps what a newer build wrote.
#[test]
fn concurrent_pair_edits_merge_and_removals_stick() {
    let w = world();
    let (mac, win) = (mac(), win());
    session(w.home.base(), SID);
    push_as(&w, &w.home, &mac);
    let p0 = new_pair(
        "fp_0000000000000000",
        &mac,
        "/Users/marcus/old",
        &win,
        r"D:\old",
    );
    add_pair(&w, &mac, p0.clone());
    // Both read the index with p0; the Mac adds p1, Windows removes p0 and
    // adds p2, each merged into what the set has when it writes.
    let p1 = new_pair("fp_1111111111111111", &mac, MAC_DEV, &win, WIN_DEV);
    let p2 = new_pair(
        "fp_2222222222222222",
        &win,
        r"E:\work",
        &mac,
        "/Users/marcus/work",
    );
    let mut gone = p0.clone();
    gone.removed = Some(pairs::Removed {
        at: now_ms(),
        by: win.id.clone(),
    });
    add_pair(&w, &mac, p1);
    let keys = open_keys(&w.set, &pass()).unwrap();
    let changes = Changes {
        pairs: vec![gone, p2],
        machine_info: Some(pairs::MachineInfo::here(&win.name, now_ms())),
        ..Changes::default()
    };
    write_index_merged(&w.set, &keys, &changes, &win.id, now_ms()).unwrap();
    let index = index_of(&w);
    let live: Vec<&str> = index
        .folder_pairs
        .iter()
        .filter(|p| p.live())
        .map(|p| p.id.as_str())
        .collect();
    assert_eq!(live, ["fp_1111111111111111", "fp_2222222222222222"]);
    // A machine that still has p0 adds it again: it stays removed.
    add_pair(&w, &mac, p0);
    assert!(!index_of(&w)
        .folder_pairs
        .iter()
        .any(|p| p.id == "fp_0000000000000000" && p.live()));
    // Each machine's own line.
    let index = index_of(&w);
    assert_eq!(index.machines[&mac.id].name, "marcus-mbp");
    assert_eq!(index.machines[&win.id].name, "DESKTOP-PC");
    let known = pairs::known_machines(&index);
    assert_eq!(known[&win.id].os.as_deref(), Some(std::env::consts::OS));
    // A pair naming a machine that is not an id, or a relative folder, is
    // never read.
    let mut bad = index.clone();
    bad.folder_pairs.push(new_pair(
        "fp_3333333333333333",
        &mac,
        "relative",
        &win,
        WIN_DEV,
    ));
    write_index(&w.set, &keys, &bad).unwrap();
    assert!(!index_of(&w)
        .folder_pairs
        .iter()
        .any(|p| p.id == "fp_3333333333333333"));
}

/// A push from a machine with a session it continued in a folder a pair
/// maps records that folder; a session whose transcript names no folder of
/// its project keeps its first cwd.
#[test]
fn a_push_records_the_folder_as_its_machine_sees_it() {
    let w = world();
    let (mac, win) = (mac(), win());
    // Started on Windows, its transcript in the Mac's folder name (pulled
    // there before a pair): Windows records the Mac's folder.
    session_in(&w.other, SID, MAC_APP);
    continue_in(&w.other, SID, WIN_APP, "continued before a pair");
    push_as(&w, &w.other, &win);
    assert_eq!(index_of(&w).sessions[SID].folder.as_deref(), Some(MAC_APP));
    // The last cwd is read from the end only: a huge first line is skipped.
    let t = transcript_in(&w.home, OTHER, "/p/huge");
    std::fs::create_dir_all(t.parent().unwrap()).unwrap();
    let big = "x".repeat(2 * TAIL_BYTES as usize);
    std::fs::write(
        &t,
        format!(
            "{}\n{}\n",
            serde_json::json!({"type": "user", "cwd": "/p/huge", "text": big}),
            serde_json::json!({"type": "user", "cwd": "/p/elsewhere"}),
        ),
    )
    .unwrap();
    assert_eq!(last_recorded_cwd(&t).as_deref(), Some("/p/elsewhere"));
    push_as(&w, &w.home, &mac);
    assert_eq!(
        index_of(&w).sessions[OTHER].folder.as_deref(),
        Some("/p/huge")
    );
}

/// Move data names a project of a set by the folder its sessions record: a
/// Windows one is found in any case, with either separator and a trailing
/// one, a Mac's exactly (a trailing `/` aside) — into an account and into
/// another set alike.
#[test]
fn a_project_of_a_set_is_found_by_its_folder_written_either_way() {
    let w = world();
    let (m, p) = (&w.home, &w.other);
    session_in(p, SID, WIN_APP);
    push_as(&w, p, &win());
    session_in(m, OTHER, MAC_APP);
    push_as(&w, m, &mac());
    let target = ClaudeHome::for_account_dir(w.undo.with_file_name("fresh"));
    let to = Set::new(&w.undo.with_file_name("storage"), "Copy");
    for (project, want) in [
        (r"d:/DEV/app\", Some(SID)),
        (WIN_APP, Some(SID)),
        ("/Users/marcus/dev/app/", Some(OTHER)),
        ("/Users/marcus/DEV/app", None),
    ] {
        let want: Vec<&str> = want.into_iter().collect();
        let projects = [project.to_string()];
        let (entries, _, empty) = plan_restore(
            &w.set,
            &[pass()],
            &target,
            Some(&[][..]),
            &projects,
            &not_live,
            false,
        )
        .unwrap();
        let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, want, "{project}");
        assert_eq!(empty.is_empty(), !want.is_empty(), "{project}");
        let (entries, _, empty) =
            plan_between(&w.set, &to, &[pass()], Some(&[][..]), &projects).unwrap();
        let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, want, "{project}");
        assert_eq!(empty.is_empty(), !want.is_empty(), "{project}");
    }
}
