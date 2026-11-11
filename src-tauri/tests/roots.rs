//! Roots (docs/PLAN-MACHINES.md §5.12): two app states in one process, each
//! with its own data dir and iroh endpoint, paired through the real
//! dashboard API (as `tests/machines.rs` does). A root set on one computer
//! reaches the other and maps a path both ways; an edit and a removal
//! follow; a computer without the `move` grant cannot change the other's
//! roots; only the desktop or a paired device manages them; the folder
//! pairs the backups keep become roots once. Nothing touches `~/.claude` or
//! spawns `claude`.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use iroh::address_lookup::MemoryLookup;
use mother_claude_lib::claude::{owned, ClaudeHome};
use mother_claude_lib::machines::{self, roots, transport};
use mother_claude_lib::server::{self, auth::Auth};
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tempfile::TempDir;

struct Machine {
    st: AppState,
    url: String,
    dir: TempDir,
}

/// The process-wide part, once: with `CLAUDE_CONFIG_DIR` set, a test home's
/// `.claude.json` is inside it, never the user's own `~/.claude.json`.
fn isolate() {
    static ONCE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap().keep();
        std::env::set_var("CLAUDE_CONFIG_DIR", dir.join("unused-config"));
        dir
    });
}

async fn machine(lookup: &MemoryLookup) -> Machine {
    isolate();
    let dir = tempfile::tempdir().unwrap();
    let mut auth = Auth::ephemeral();
    auth.config_dir = Some(dir.path().join("data"));
    std::fs::create_dir_all(dir.path().join("data")).unwrap();
    let st = Inner::new(
        ClaudeHome::with_base(dir.path().join("home")),
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        auth,
    );
    st.machines.use_test_lookup(lookup.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = server::router(st.clone());
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    Machine {
        st,
        url: format!("http://{addr}"),
        dir,
    }
}

impl Machine {
    async fn call_as(
        &self,
        token: &str,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let client = reqwest::Client::new();
        let url = format!("{}/api{path}", self.url);
        let req = match method {
            "GET" => client.get(url),
            "POST" => client.post(url),
            "PUT" => client.put(url),
            "PATCH" => client.patch(url),
            "DELETE" => client.delete(url),
            m => panic!("method {m}"),
        };
        let req = req.bearer_auth(token);
        let req = match body {
            Some(b) => req.json(&b),
            None => req,
        };
        let resp = req.send().await.unwrap();
        let status = resp.status();
        let v = resp.json::<Value>().await.unwrap_or(Value::Null);
        (status, v)
    }

    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let token = self.st.auth.token.clone();
        self.call_as(&token, method, path, body).await
    }

    async fn view(&self) -> Value {
        let (s, v) = self.call("GET", "/machines", None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v
    }

    /// Polls the view until `pred` holds (45 s at most).
    async fn wait(&self, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            let v = self.view().await;
            if pred(&v) {
                return v;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}: {v:#}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// `GET /api/roots` as the desktop.
    async fn roots(&self) -> Value {
        let (s, v) = self.call("GET", "/roots", None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v
    }

    /// Polls `GET /api/roots` until `pred` holds (30 s at most).
    async fn roots_where(&self, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let v = self.roots().await;
            if pred(&v) {
                return v;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}: {v:#}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn put_roots(&self, body: Value) -> (StatusCode, Value) {
        self.call("PUT", "/roots", Some(body)).await
    }

    fn id(&self) -> String {
        machines::machine_info(&self.st).id
    }
}

fn state_of(v: &Value) -> &str {
    v["pairing"]["state"].as_str().unwrap_or("")
}

/// Pairs `a` (host) and `b` (joiner) through the dashboard API; `a_gives` is
/// what `b` may do on `a`, `b_gives` what `a` may do on `b`.
async fn pair(a: &Machine, b: &Machine, a_gives: Value, b_gives: Value) {
    let (s, v) = a.call("POST", "/machines/pair", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let code = v["pairing"]["code"].as_str().expect("a code").to_string();
    let (s, v) = b
        .call(
            "POST",
            "/machines/join",
            Some(json!({ "code": code, "name": "Joiner" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    b.wait("the joiner to show the digits", |v| {
        state_of(v) == "confirm"
    })
    .await;
    a.wait("the host to show the digits", |v| state_of(v) == "confirm")
        .await;
    let (s, v) = b
        .call(
            "POST",
            "/machines/confirm",
            Some(json!({ "grants": b_gives })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = a
        .call(
            "POST",
            "/machines/confirm",
            Some(json!({ "grants": a_gives })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(state_of(&v), "paired", "{v}");
    b.wait("the joiner to finish", |v| state_of(v) == "paired")
        .await;
}

/// See, drive and move on the local network (where these two meet); the
/// move grant only when `moves`.
fn grants(moves: bool) -> Value {
    json!({
        "view": {"lan": true, "internet": true},
        "control": {"lan": true, "internet": true},
        "move": {"lan": moves, "internet": false},
        "run": {"lan": false, "internet": false},
        "dangerous": {"lan": false, "internet": false}
    })
}

/// A folder in this system's form (both test machines run here), under a
/// name no real folder has.
fn folder(tag: &str, parts: &[&str]) -> String {
    if cfg!(windows) {
        format!(r"D:\mc-roots-{tag}\{}", parts.join("\\"))
    } else {
        format!("/mc-roots-{tag}/{}", parts.join("/"))
    }
}

/// `base` and `parts` below it.
fn child(base: &str, parts: &[&str]) -> String {
    let sep = if cfg!(windows) { "\\" } else { "/" };
    format!("{base}{sep}{}", parts.join(sep))
}

fn tag() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

fn names(v: &Value) -> Vec<String> {
    v["roots"]
        .as_array()
        .map(|l| {
            l.iter()
                .filter_map(|r| r["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Calls `f` until it returns `true` (30 s at most).
async fn eventually<F, Fut>(what: &str, mut f: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + Duration::from_secs(30);
    while !f().await {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_root_set_on_one_computer_reaches_the_other_and_maps_both_ways() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let b = machine(&lookup).await;
    pair(&a, &b, grants(true), grants(true)).await;
    let (ida, idb) = (a.id(), b.id());
    // A's folder is a real one (so a session from B continues in it); B's
    // is only named.
    let on_a = a.dir.path().join("projects").to_string_lossy().into_owned();
    std::fs::create_dir_all(child(&on_a, &["app", "src"])).unwrap();
    let on_b = folder(&tag(), &["work", "projects"]);

    let (s, v) = a
        .put_roots(
            json!({ "upsert": [{ "name": "projects", "paths": { &ida: on_a, &idb: on_b } }] }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["machine"], ida.as_str());
    assert_eq!(v["machines"][0]["id"], ida.as_str(), "this computer first");
    assert_eq!(v["machines"][0]["here"], true);
    assert_eq!(v["machines"][1]["id"], idb.as_str());
    assert_eq!(v["machines"][1]["paired"], true);
    let id = v["roots"][0]["id"].as_str().expect("an id").to_string();
    assert_eq!(v["roots"][0]["by"], ida.as_str());

    // B gets it, as A wrote it.
    let vb = b
        .roots_where("A's root on B", |v| names(v) == ["projects"])
        .await;
    assert_eq!(vb["roots"][0]["id"], id.as_str());
    assert_eq!(vb["roots"][0]["paths"][&ida], on_a.as_str());
    assert_eq!(vb["roots"][0]["paths"][&idb], on_b.as_str());

    // It maps a path both ways, on either computer.
    let app_a = child(&on_a, &["app", "src"]);
    let app_b = child(&on_b, &["app", "src"]);
    assert_eq!(
        roots::map_here(&b.st, &ida, &app_a).as_deref(),
        Some(app_b.as_str())
    );
    assert_eq!(
        roots::map_here(&a.st, &idb, &app_b).as_deref(),
        Some(app_a.as_str())
    );
    assert_eq!(
        roots::map_between(&b.st, &idb, &ida, &app_b).as_deref(),
        Some(app_a.as_str())
    );
    assert_eq!(
        roots::map_here(&b.st, &ida, &folder(&tag(), &["elsewhere"])),
        None,
        "a path under no root"
    );
    // A session from B continues in A's folder, its machine known or not.
    let mapped = owned::folder_here_from(&a.st, Some(&idb), &app_b, None)
        .await
        .unwrap();
    assert_eq!(mapped, app_a);
    let mapped = owned::folder_here_from(&a.st, None, &app_b, None)
        .await
        .unwrap();
    assert_eq!(mapped, app_a);

    // B renames it: the last writer wins on both.
    let (s, v) = b
        .put_roots(json!({ "upsert": [{ "id": id, "name": "code", "paths": { &ida: on_a, &idb: on_b } }] }))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    a.roots_where("B's rename on A", |v| names(v) == ["code"])
        .await;

    // A removes it: gone on both, and it stays gone.
    let (s, v) = a.put_roots(json!({ "remove": [id] })).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(names(&v).is_empty(), "{v}");
    b.roots_where("the removal on B", |v| names(v).is_empty())
        .await;
    assert_eq!(roots::map_here(&b.st, &ida, &app_a), None);
    let e = owned::folder_here_from(&a.st, None, &app_b, None)
        .await
        .unwrap_err();
    assert_eq!(e.code, "folder_elsewhere", "{}", e.message);
    let (s, v) = b
        .put_roots(json!({ "upsert": [{ "id": id, "name": "code", "paths": { &idb: on_b } }] }))
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "root_removed");
    // Its tombstone is still on both, so no copy brings it back.
    for m in [&a, &b] {
        let all = roots::all(&m.st);
        assert!(all.iter().any(|r| r.id == id && !r.live()), "{all:?}");
    }
    assert_eq!(roots::digest(&a.st), roots::digest(&b.st));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_computer_without_the_move_grant_cannot_change_the_others_roots() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let b = machine(&lookup).await;
    // A lets B move (and so change A's roots); B does not let A.
    pair(&a, &b, grants(true), grants(false)).await;
    let (ida, idb) = (a.id(), b.id());
    let t = tag();

    // A's root stays on A: B refuses it.
    let (s, v) = a
        .put_roots(json!({ "upsert": [{ "name": "from-a", "paths": {
            &ida: folder(&t, &["a"]), &idb: folder(&t, &["b"])
        } }] }))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = transport::request(
        &a.st,
        &idb,
        "POST",
        roots::PEER_PATH,
        Some(json!({ "roots": roots::all(&a.st) })),
        Duration::from_secs(20),
    )
    .await
    .unwrap();
    assert_eq!(s, 403, "{v}");
    assert_eq!(v["error"]["code"], "grant_missing");
    assert_eq!(v["error"]["grant"], "move");
    // It may still read them (view).
    let (s, v) = transport::request(
        &a.st,
        &idb,
        "GET",
        roots::PEER_PATH,
        None,
        Duration::from_secs(20),
    )
    .await
    .unwrap();
    assert_eq!(s, 200, "{v}");
    assert!(v["digest"].is_string());
    // Never through the API: `/api/roots` is not another computer's.
    for (method, path) in [("GET", "/api/roots"), ("PUT", "/api/roots")] {
        let (s, _) = transport::request(
            &a.st,
            &idb,
            method,
            path,
            Some(json!({})),
            Duration::from_secs(20),
        )
        .await
        .unwrap();
        assert_eq!(s, 404, "{method} {path}");
    }
    // (Through the proxy, with Machines on: still not one of its routes.)
    machines::set_enabled(&a.st, true);
    machines::set_enabled(&b.st, true);
    let (s, _) = a
        .call("GET", &format!("/machines/{idb}/proxy/roots"), None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(names(&b.roots().await).is_empty());

    // B's root reaches A (A gave B the grant), but A's answer does not
    // change B's.
    let (s, v) = b
        .put_roots(json!({ "upsert": [{ "name": "from-b", "paths": {
            &idb: folder(&t, &["b2"]), &ida: folder(&t, &["a2"])
        } }] }))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    a.roots_where("B's root on A", |v| names(v) == ["from-a", "from-b"])
        .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(names(&b.roots().await), ["from-b"]);

    // B gives A the grant: the hellos that follow bring A's root to B.
    let (s, v) = b
        .call(
            "PATCH",
            &format!("/machines/{ida}"),
            Some(json!({ "grants": grants(true) })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    b.roots_where("A's root on B, once granted", |v| {
        names(v) == ["from-a", "from-b"]
    })
    .await;
    eventually("the same roots on both", || async {
        roots::digest(&a.st) == roots::digest(&b.st)
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn only_the_desktop_or_a_device_manages_roots_and_a_change_is_checked() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let ida = a.id();
    let t = tag();

    // A local script may neither read nor change them; a paired phone may.
    let script = a.st.auth.script_token.clone();
    for (method, body) in [("GET", None), ("PUT", Some(json!({})))] {
        let (s, v) = a.call_as(&script, method, "/roots", body).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method}: {v}");
        assert_eq!(v["error"]["code"], "desktop_only");
    }
    let phone =
        a.st.devices
            .issue(
                "Phone",
                "ios",
                None,
                server::Ingress::Lan,
                "192.168.1.20".parse().unwrap(),
            )
            .1;
    let (s, v) = a
        .call_as(
            &phone,
            "PUT",
            "/roots",
            Some(json!({ "upsert": [{ "name": "phone", "paths": { &ida: folder(&t, &["p"]) } }] })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = a.call_as(&phone, "GET", "/roots", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(names(&v), ["phone"]);
    let id = v["roots"][0]["id"].as_str().unwrap().to_string();

    // Checked whole: nothing of a refused change is kept.
    let relative = if cfg!(windows) { r"dev\app" } else { "dev/app" };
    let long = "n".repeat(41);
    for (body, status, code) in [
        (
            json!({ "upsert": [{ "name": "x", "paths": { &ida: relative } }] }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "roots_bad_path",
        ),
        (
            json!({ "upsert": [{ "name": long, "paths": { &ida: folder(&t, &["x"]) } }] }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "roots_bad_name",
        ),
        (
            json!({ "upsert": [{ "name": "x", "paths": { "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee": "/x" } }] }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "roots_unknown_machine",
        ),
        (
            json!({ "upsert": [
                { "name": "fine", "paths": { &ida: folder(&t, &["fine"]) } },
                { "name": "PHONE", "paths": { &ida: folder(&t, &["dup"]) } }
            ] }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "roots_duplicate_name",
        ),
        (
            json!({ "remove": ["rt_0000000000000000"] }),
            StatusCode::NOT_FOUND,
            "root_not_found",
        ),
        (
            json!({ "upsert": [{ "name": "x", "paths": {} }], "extra": 1 }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_body",
        ),
    ] {
        let (s, v) = a.put_roots(body.clone()).await;
        assert_eq!(s, status, "{body}: {v}");
        assert_eq!(v["error"]["code"], code, "{body}: {v}");
    }
    let many: Vec<Value> = (0..=roots::MAX_ROOTS)
        .map(|i| {
            let name = format!("r{i}");
            json!({ "name": name, "paths": { &ida: folder(&t, &[name.as_str()]) } })
        })
        .collect();
    let (s, v) = a.put_roots(json!({ "upsert": many })).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"]["code"], "roots_too_many");
    assert_eq!(names(&a.roots().await), ["phone"], "nothing of it kept");

    // The phone's change was audited; removing it is too.
    let (s, _) = a.put_roots(json!({ "remove": [id] })).await;
    assert_eq!(s, StatusCode::OK);
    assert!(names(&a.roots().await).is_empty());
}

/// What the backup sets last said of folder pairs (`backup-pairs.json`), as
/// a push or pull leaves it: `pairs` in the Default account's set.
fn pairs_seen(
    m: &Machine,
    set_dir: &std::path::Path,
    list: &[mother_claude_lib::claude::backup::pairs::FolderPair],
) {
    use mother_claude_lib::claude::backup::pairs;
    let mut cache = pairs::Cache::default();
    cache.sets.insert(
        set_dir.to_string_lossy().into_owned(),
        pairs::SetPairs {
            pairs: list.to_vec(),
            machines: Default::default(),
            at: chrono::Utc::now().timestamp_millis(),
        },
    );
    std::fs::write(
        m.dir.path().join("data").join(pairs::CACHE_FILE),
        serde_json::to_vec(&cache).unwrap(),
    )
    .unwrap();
}

/// The folder pairs the backups keep become roots, each once: a root the
/// user removes is not imported again (its pair still maps, as the backups
/// still have it), and a pair removed in the backups takes its root along.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_backups_folder_pairs_become_roots_once() {
    use mother_claude_lib::claude::backup::{pairs, Set};
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let ida = a.id();
    let other = "ab".repeat(16);
    let t = tag();
    let nas = a.dir.path().join("NAS");
    std::fs::create_dir_all(&nas).unwrap();
    let (s, v) = a
        .call("PUT", "/backup/settings", Some(json!({ "folder": nas })))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let backups = a.st.accounts.backup_settings().folder.expect("a folder");
    let set_dir = Set::new(&backups, "Default").dir;
    // Reading the backup settings again reads the sets' pairs again.
    let reread = || async {
        let (s, v) = a.call("PUT", "/backup/settings", Some(json!({}))).await;
        assert_eq!(s, StatusCode::OK, "{v}");
    };
    let pair = |id: &str, name: &str, at: i64| pairs::FolderPair {
        id: id.to_string(),
        folders: [
            (ida.clone(), folder(&t, &[name])),
            (other.clone(), format!("/zz-{t}/{name}")),
        ]
        .into_iter()
        .collect(),
        created_at: at,
        by: other.clone(),
        removed: None,
        extra: Default::default(),
    };
    let dev = pair("fp_0123456789abcdef", "dev", 10);
    pairs_seen(&a, &set_dir, std::slice::from_ref(&dev));
    reread().await;
    let v = a
        .roots_where("the pair as a root", |v| names(v) == ["dev"])
        .await;
    let root = &v["roots"][0];
    assert_eq!(root["id"], "rt_0123456789abcdef");
    assert_eq!(root["pair"], "fp_0123456789abcdef");
    assert_eq!(root["paths"][&other], format!("/zz-{t}/dev"));

    // Removed by the user: not imported again, and the pair still maps.
    let (s, v) = a
        .put_roots(json!({ "remove": ["rt_0123456789abcdef"] }))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    reread().await;
    assert!(names(&a.roots().await).is_empty());
    assert_eq!(
        roots::map_here(&a.st, &other, &format!("/zz-{t}/dev/app")).as_deref(),
        Some(child(&folder(&t, &["dev"]), &["app"]).as_str()),
        "the folder pair still maps it"
    );

    // A pair removed in the backups takes its root along.
    let work = pair("fp_1111111111111111", "work", 20);
    pairs_seen(&a, &set_dir, &[dev.clone(), work.clone()]);
    reread().await;
    a.roots_where("the second pair as a root", |v| names(v) == ["work"])
        .await;
    let gone = pairs::FolderPair {
        removed: Some(pairs::Removed {
            at: 30,
            by: other.clone(),
        }),
        ..work
    };
    pairs_seen(&a, &set_dir, &[dev, gone]);
    reread().await;
    assert!(names(&a.roots().await).is_empty());
    let all = roots::all(&a.st);
    assert_eq!(all.len(), 2, "both kept as tombstones: {all:?}");
    assert!(all.iter().all(|r| !r.live()));
    let imported = a.st.machines.store_read(|f| f.roots_imported.clone());
    assert_eq!(imported, ["fp_0123456789abcdef", "fp_1111111111111111"]);
    // This process's other tests see no pair of this one.
    mother_claude_lib::claude::paths::set_local_pairs(
        &a.dir.path().join("data").to_string_lossy(),
        Vec::new(),
    );
}
