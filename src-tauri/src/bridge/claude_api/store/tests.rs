use super::*;
use serde_json::json;

#[test]
fn folder_labels_are_safe_names() {
    assert_eq!(folder_label("Foo Bar/x"), "foo-bar-x");
    assert_eq!(folder_label("..hidden.."), "hidden");
    assert_eq!(folder_label(""), "client");
    assert_eq!(folder_label("con"), "client-con");
    assert_eq!(folder_label("COM1.txt"), "client-com1.txt");
    assert_eq!(folder_label("lpt9"), "client-lpt9");
    assert_eq!(folder_label("comet"), "comet");
    assert_eq!(folder_label(&"a".repeat(100)).len(), 64);
}

/// `X-Client-Id` is the application's own folder path.
#[test]
fn client_paths_are_safe_folder_paths() {
    assert_eq!(
        client_path("xiangqi-solver/Dev 1/2026-11-18"),
        "xiangqi-solver/dev-1/2026-11-18"
    );
    assert_eq!(client_path("Alpha"), "alpha");
    // No way out of the clients' folder, no empty folders.
    assert_eq!(client_path("../../etc/passwd"), "etc/passwd");
    assert_eq!(client_path("/a//b/./c/"), "a/b/c");
    assert_eq!(client_path(" / .. / "), "client");
    // Windows' device names and backslashes stay harmless.
    assert_eq!(client_path("app/con/x"), "app/client-con/x");
    assert_eq!(client_path("a\\b"), "a-b");
    // At most CLIENT_DEPTH folders, each a safe name.
    assert_eq!(client_path("1/2/3/4/5/6/7/8"), "1/2/3/4/5/6");
    assert_eq!(client_path(&"x".repeat(100)).len(), 64);
}

/// Each folder of a client's path is made; `latest` is that folder's own.
#[test]
fn a_client_path_is_folders_and_latest_is_its_own() {
    let root = tempfile::tempdir().unwrap();
    let day = Store::client_dir(root.path(), &client("app/dev-1/2026-11-18")).unwrap();
    assert_eq!(
        day,
        root.path().join("app").join("dev-1").join("2026-11-18")
    );
    assert!(day.join("conversations").is_dir());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for d in [
            root.path().join("app"),
            root.path().join("app/dev-1"),
            day.clone(),
        ] {
            let mode = std::fs::metadata(&d).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "{}", d.display());
        }
    }

    let store = Store::new(None);
    assert_eq!(store.latest(&day, "s1"), None, "nothing yet");
    let mut a = record(A, "s1", &["h1"]);
    a.updated_at = 10;
    a.latest = true;
    let mut b = record(B, "s1", &["h1", "h2", "h3"]);
    b.updated_at = 20;
    b.latest = true;
    store.insert(a, day.clone());
    store.insert(b, day.clone());
    store.release(A);
    store.release(B);
    assert_eq!(
        store.latest(&day, "s1").as_deref(),
        Some(B),
        "the one used last"
    );
    assert_eq!(store.latest(&day, "s2"), None, "other settings: another");
    // Another day's folder has its own.
    let next = Store::client_dir(root.path(), &client("app/dev-1/2026-11-19")).unwrap();
    assert_eq!(store.latest(&next, "s1"), None);
    // One that never answered is not one to go on in.
    let c = "00000000-0000-4000-8000-00000000000c";
    let mut fresh = record(c, "s1", &[]);
    fresh.updated_at = 30;
    fresh.latest = true;
    store.insert(fresh, day.clone());
    store.release(c);
    assert_eq!(store.latest(&day, "s1").as_deref(), Some(B));
    // Nor is a one-off, however new: a request began it without `latest`.
    let d = "00000000-0000-4000-8000-00000000000d";
    let mut one_off = record(d, "s1", &["h1", "h9"]);
    one_off.updated_at = 40;
    store.insert(one_off, day.clone());
    store.release(d);
    assert_eq!(store.latest(&day, "s1").as_deref(), Some(B));
}

#[test]
fn ip_addresses_are_folder_names() {
    use std::net::IpAddr;
    let ip = |s: &str| s.parse::<IpAddr>().unwrap();
    assert_eq!(ip_label(ip("192.168.1.20")), "192.168.1.20");
    assert_eq!(ip_label(ip("::1")), "127.0.0.1");
    assert_eq!(ip_label(ip("::ffff:10.0.0.5")), "10.0.0.5");
    assert_eq!(ip_label(ip("fe80::1")), "ip6-fe80--1");
}

#[test]
fn grouping_is_read_leniently() {
    assert_eq!(GroupBy::parse(None), GroupBy::None);
    assert_eq!(GroupBy::parse(Some("ip")), GroupBy::Ip);
    assert_eq!(GroupBy::parse(Some("key")), GroupBy::Key);
    assert_eq!(GroupBy::parse(Some("user")), GroupBy::User);
    assert_eq!(GroupBy::parse(Some("header")), GroupBy::Header);
    assert_eq!(GroupBy::parse(Some("other")), GroupBy::None);
}

fn client(id: &str) -> Client {
    Client {
        id: id.into(),
        kind: "ip".into(),
        label: id.into(),
    }
}

fn record(id: &str, settings: &str, history: &[&str]) -> Record {
    Record {
        id: id.into(),
        settings: settings.into(),
        history: history.iter().map(|s| s.to_string()).collect(),
        updated_at: 1,
        ..Record::default()
    }
}

const A: &str = "00000000-0000-4000-8000-00000000000a";
const B: &str = "00000000-0000-4000-8000-00000000000b";

#[test]
fn a_history_finds_its_conversation() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(None);
    let dir = Store::client_dir(root.path(), &client("1.2.3.4")).unwrap();
    assert!(dir.join("conversations").is_dir());

    store.insert(record(A, "s1", &["h1", "h2"]), dir.clone());
    // Claimed when inserted.
    assert_eq!(store.claim(&dir, A), Err(Busy::Answering));
    assert_eq!(
        store.lookup(&dir, "s1", &["h1".into(), "h2".into()]),
        Lookup::Nothing
    );
    store.release(A);

    assert_eq!(
        store.lookup(&dir, "s1", &["h1".into(), "h2".into()]),
        Lookup::Claimed(A.into())
    );
    store.release(A);
    // Other settings: a fork of it.
    assert_eq!(
        store.lookup(&dir, "s2", &["h1".into(), "h2".into()]),
        Lookup::Fork(A.into())
    );
    // Another history, or none: nothing.
    assert_eq!(store.lookup(&dir, "s1", &["h1".into()]), Lookup::Nothing);
    assert_eq!(store.lookup(&dir, "s1", &[]), Lookup::Nothing);
    // Ending its turn: wait for it.
    store.claim(&dir, A).unwrap();
    store.with(A, |c| c.settling = true);
    assert_eq!(
        store.lookup(&dir, "s1", &["h1".into(), "h2".into()]),
        Lookup::Settling(A.into())
    );
    assert_eq!(store.claim(&dir, A), Err(Busy::Settling));
    store.release(A);
    assert_eq!(store.state_of(A), Some((false, false)));

    // Another client's folder does not see it.
    let other = Store::client_dir(root.path(), &client("5.6.7.8")).unwrap();
    assert_eq!(store.claim(&other, A), Err(Busy::Missing));
    assert_eq!(
        store.lookup(&other, "s1", &["h1".into(), "h2".into()]),
        Lookup::Nothing
    );
}

#[test]
fn records_are_kept_on_disk_once_answered() {
    let root = tempfile::tempdir().unwrap();
    let dir = Store::client_dir(root.path(), &client("c")).unwrap();
    let store = Store::new(None);
    store.insert(record(B, "s", &[]), dir.clone());
    store.release(B);
    let file = dir.join("conversations").join(format!("{B}.json"));
    assert!(
        !file.exists(),
        "a conversation that never answered is not written"
    );

    store.insert(record(A, "s", &["x", "y"]), dir.clone());
    store.release(A);
    let file = dir.join("conversations").join(format!("{A}.json"));
    assert!(file.is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    // A new process reads it back.
    let again = Store::new(None);
    let records = again.records_in(&dir);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].history, vec!["x".to_string(), "y".to_string()]);
    assert_eq!(
        again.lookup(&dir, "s", &["x".into(), "y".into()]),
        Lookup::Claimed(A.into())
    );
    again.release(A);

    assert!(again.forget(A).is_some());
    assert!(!file.exists());
    assert!(again.forget(A).is_none());
}

#[test]
fn a_client_is_counted() {
    let root = tempfile::tempdir().unwrap();
    let dir = Store::client_dir(root.path(), &client("c")).unwrap();
    Store::note_client(&dir, &client("c"), 10);
    Store::note_client(&dir, &client("c"), 20);
    let rec: ClientRecord =
        serde_json::from_slice(&std::fs::read(dir.join("client.json")).unwrap()).unwrap();
    assert_eq!((rec.first_seen, rec.last_seen, rec.requests), (10, 20, 2));
}

#[test]
fn host_calls_are_matched_by_name_and_input() {
    let store = Store::new(None);
    let call = |id: &str, name: &str, input: Value| HostCall {
        request_id: id.into(),
        name: name.into(),
        input,
    };
    store.note_host_call(A, call("r1", "weather", json!({ "city": "Paris" })));
    store.note_host_call(A, call("r2", "weather", json!({ "city": "Rome" })));
    let got = store
        .take_host_call(A, "weather", &json!({ "city": "Rome" }))
        .unwrap();
    assert_eq!(got.request_id, "r2");
    // No exact input: the first by name.
    let got = store
        .take_host_call(A, "weather", &json!({ "city": "Oslo" }))
        .unwrap();
    assert_eq!(got.request_id, "r1");
    assert!(store.take_host_call(A, "weather", &json!({})).is_none());
    store.note_host_call(A, call("r3", "x", json!({})));
    assert_eq!(store.drain_host_calls(A).len(), 1);
    assert!(store.drain_host_calls(A).is_empty());
}

#[test]
fn idle_and_waiting_conversations_are_found() {
    let root = tempfile::tempdir().unwrap();
    let dir = Store::client_dir(root.path(), &client("c")).unwrap();
    let store = Store::new(None);
    store.insert(record(A, "s", &["x"]), dir.clone());
    store.insert(record(B, "s", &["y"]), dir.clone());
    store.with(A, |c| c.open = true);
    store.with(B, |c| {
        c.open = true;
        c.waiting = vec![ToolCall {
            id: "t".into(),
            name: "n".into(),
            input: json!({}),
        }];
    });
    let later = Instant::now() + std::time::Duration::from_secs(1);
    // Busy: neither.
    assert!(store.idle_open(later).is_empty());
    store.release(A);
    store.release(B);
    assert_eq!(store.idle_open(later), vec![A.to_string()]);
    assert_eq!(store.stale_waiting(later), vec![B.to_string()]);
    assert_eq!(store.state_of(B), Some((false, true)));
    let open = store.open_ones();
    assert_eq!(open.len(), 2);
    assert!(open.iter().any(|(id, active)| id == B && *active));
}
