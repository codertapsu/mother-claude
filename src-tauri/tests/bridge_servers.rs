//! API servers (docs/ACCOUNTS.md §13.5), offline: several at once, each with
//! its own account, port, defaults, conversation, token policy and started
//! gate; `accountOnly` on every route kind. A fake Node host
//! (`tests/fixtures/fake-bridge-host.js`) stands in for the runtime. Nothing
//! here binds 5612 or runs Claude: the main server and every added one listen
//! on ports the system picks.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use mother_claude_lib::bridge::control::{self, BridgeDefaults};
use mother_claude_lib::bridge::{BridgeConfig, TokenPolicy};
use mother_claude_lib::claude::secrets::Secret;
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use reqwest::StatusCode;
use serde_json::{json, Value};

const TOKEN: &str = "sk-ant-oat01-ServersTestToken0123456789abcdefghijk";

/// The fake host and its log, for every test here: the variables are
/// process-wide, so they are set once, the same for all.
fn host_log() -> PathBuf {
    static LOG: OnceLock<PathBuf> = OnceLock::new();
    LOG.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap().keep();
        let log = dir.join("host.log");
        let fake =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-bridge-host.js");
        std::env::set_var("MOTHER_CLAUDE_BRIDGE_HOST_PATH", &fake);
        std::env::set_var("FAKE_BRIDGE_LOG", &log);
        log
    })
    .clone()
}

/// The `session.create` the host was sent for `id`.
fn created(id: &str) -> Value {
    std::fs::read_to_string(host_log())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find(|r| r["op"] == "session.create" && r["sessionId"] == id)
        .unwrap_or_else(|| panic!("no session.create for {id}"))
}

async fn serve(app: axum::Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    format!("http://{addr}")
}

struct Fixture {
    st: AppState,
    /// The main app port (the dashboard, loopback).
    app: String,
    home: tempfile::TempDir,
    scratch: tempfile::TempDir,
}

/// The app on `home` with its data in `data`; the main API server gets a port
/// the system picks, and starts tokenless for this machine.
async fn boot_in(home: &Path, data: &Path) -> (AppState, String) {
    host_log();
    let mut auth = Auth::ephemeral();
    auth.config_dir = Some(data.to_path_buf());
    let st = Inner::with_bridge_config(
        ClaudeHome::with_base(home),
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        auth,
        BridgeConfig {
            enabled: true,
            port: Some(0),
            token: TokenPolicy::Local,
            api_key: None,
            allowed_origins: Vec::new(),
        },
    );
    let app = serve(mother_claude_lib::server::router(st.clone())).await;
    (st, app)
}

async fn boot() -> Fixture {
    let home = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let (st, app) = boot_in(home.path(), &scratch.path().join("app-data")).await;
    Fixture {
        st,
        app,
        home,
        scratch,
    }
}

impl Fixture {
    /// Where the API server `id` listens.
    fn api(&self, id: &str) -> String {
        let port = self
            .st
            .bridge
            .server(id)
            .unwrap()
            .bound_port()
            .expect("the server is running");
        format!("http://127.0.0.1:{port}")
    }

    fn work_dir(&self) -> PathBuf {
        self.scratch.path().join("claude-work")
    }
}

async fn call(
    method: &str,
    url: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let c = reqwest::Client::new();
    let mut r = match method {
        "GET" => c.get(url),
        "DELETE" => c.delete(url),
        "PATCH" => c.patch(url),
        _ => c.post(url),
    };
    if let Some(t) = token {
        r = r.bearer_auth(t);
    }
    if let Some(b) = body {
        r = r.json(&b);
    }
    let resp = r.send().await.unwrap();
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

async fn get(url: &str) -> (StatusCode, Value) {
    call("GET", url, None, None).await
}

async fn post(url: &str, body: Value) -> (StatusCode, Value) {
    call("POST", url, None, Some(body)).await
}

/// A new conversation on `api`, its id.
async fn create(api: &str, body: Value) -> String {
    let (s, v) = post(&format!("{api}/threads"), body).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v["thread_id"].as_str().unwrap().to_string()
}

/// A turn on `thread` through `api`, its operation id.
async fn turn(api: &str, thread: &str, text: &str) -> String {
    let (s, v) = post(
        &format!("{api}/threads/{thread}/turns"),
        json!({ "input": text }),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    v["operation_id"].as_str().unwrap().to_string()
}

/// The request id a conversation's turn raised, once the host sent it.
async fn request_of(api: &str, thread: &str) -> String {
    for _ in 0..200 {
        let (_, v) = get(&format!("{api}/requests")).await;
        if let Some(r) = v["data"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|r| r["thread_id"] == thread)
        {
            return r["request_id"].as_str().unwrap().to_string();
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no request for {thread}");
}

/// The `thread_id`s of the events `/events` replays on `api`.
async fn event_threads(api: &str) -> Vec<String> {
    let mut resp = reqwest::Client::new()
        .get(format!("{api}/events?after=0"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let mut text = String::new();
    while let Ok(Ok(Some(chunk))) =
        tokio::time::timeout(Duration::from_millis(400), resp.chunk()).await
    {
        text.push_str(&String::from_utf8_lossy(&chunk));
    }
    text.lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .filter_map(|d| serde_json::from_str::<Value>(d).ok())
        .map(|v| v["params"]["thread_id"].as_str().unwrap_or("").to_string())
        .collect()
}

/// Two servers at once: each its own port, account, defaults, conversation
/// unaddressed messages join, token policy and started gate.
#[tokio::test]
async fn two_api_servers_run_at_once_each_with_its_own_account_and_rules() {
    let f = boot().await;
    let work =
        f.st.accounts
            .add_directory("Work", &f.work_dir(), None)
            .unwrap();
    control::start(
        &f.st,
        BridgeDefaults {
            model: Some("haiku".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let main = f.api("main");
    let server = control::add_server(
        &f.st,
        "Work",
        Some(0),
        BridgeDefaults {
            model: Some("sonnet".into()),
            account: Some(work.id.clone()),
            account_only: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(server.id, "work");
    control::start_server(&f.st, "work", None, TokenPolicy::Required)
        .await
        .unwrap();
    let work_api = f.api("work");
    assert_ne!(main, work_api);
    let script = f.st.auth.script_token.clone();

    // Each port has its own token policy: the main one is tokenless from
    // here, Work's wants a token.
    assert_eq!(get(&format!("{main}/threads")).await.0, StatusCode::OK);
    let (s, v) = get(&format!("{work_api}/threads")).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "{v}");
    assert_eq!(v["error"]["code"], "unauthorized");
    let (s, caps) = call(
        "GET",
        &format!("{work_api}/capabilities"),
        Some(&script),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(caps["auth"]["token_policy"], "required");
    assert_eq!(
        get(&format!("{main}/capabilities")).await.1["auth"]["token_policy"],
        "local"
    );

    // A conversation on each: its server's defaults and account.
    let a = create(&main, json!({})).await;
    let (s, v) = call(
        "POST",
        &format!("{work_api}/threads"),
        Some(&script),
        Some(json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let b = v["thread_id"].as_str().unwrap().to_string();
    let (ra, rb) = (created(&a), created(&b));
    assert_eq!(ra["model"], "haiku");
    assert!(ra["env"].is_null(), "the Default inherits: {ra}");
    assert_eq!(rb["model"], "sonnet");
    assert_eq!(
        rb["env"]["configDir"].as_str().map(PathBuf::from),
        work.dir.clone(),
        "{rb}"
    );
    let (ta, tb) = (
        f.st.bridge.thread(&a).await.unwrap(),
        f.st.bridge.thread(&b).await.unwrap(),
    );
    assert_eq!(
        (ta.server.as_str(), ta.account.as_str()),
        ("main", "default")
    );
    assert_eq!(
        (tb.server.as_str(), tb.account.as_str()),
        ("work", work.id.as_str())
    );

    // Each server's unaddressed messages join its own conversation.
    let chat = |api: String, token: Option<String>| async move {
        let (s, v) = call(
            "POST",
            &format!("{api}/chat"),
            token.as_deref(),
            Some(json!({ "message": "hello" })),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v["thread_id"].as_str().unwrap().to_string()
    };
    let c = chat(main.clone(), None).await;
    assert_eq!(chat(main.clone(), None).await, c);
    let d = chat(work_api.clone(), Some(script.clone())).await;
    assert_eq!(chat(work_api.clone(), Some(script.clone())).await, d);
    assert_ne!(c, d);
    assert_eq!(
        f.st.bridge.main().active_thread().as_deref(),
        Some(c.as_str())
    );
    assert_eq!(
        f.st.bridge
            .server("work")
            .unwrap()
            .active_thread()
            .as_deref(),
        Some(d.as_str())
    );

    // A conversation names its server in the bridge's own answers, the main
    // one's none (as before).
    let (_, listed) = get(&format!("{main}/threads?live_only=true")).await;
    let live = listed["data"].as_array().unwrap();
    let of = |id: &str| live.iter().find(|t| t["thread_id"] == id).cloned().unwrap();
    assert!(of(&a).get("server").is_none(), "{}", of(&a));
    assert_eq!(of(&b)["server"], "work");

    // The started gate is the server's: the main one stopped, Work runs on.
    control::stop(&f.st).await.unwrap();
    let desk = f.st.auth.token.clone();
    let (s, v) = call("GET", &format!("{}/v1/threads", f.app), Some(&desk), None).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{v}");
    assert_eq!(v["error"]["code"], "bridge_not_started");
    let (s, _) = call("GET", &format!("{work_api}/threads"), Some(&script), None).await;
    assert_eq!(s, StatusCode::OK);
    control::stop_server(&f.st, "work").await.unwrap();
    assert!(!f.st.bridge.server("work").unwrap().is_running());
}

/// `accountOnly`: a server that runs as one account shows nothing of any
/// other — not a conversation, an operation, an event or a runtime request —
/// and refuses a request that names another account. Here the account is a
/// token account, which shares the Default's directory: the hardest case.
#[tokio::test]
async fn an_account_only_server_shows_nothing_of_another_account() {
    let f = boot().await;
    let pro =
        f.st.accounts
            .add_token("Pro", Some(&Secret::new(TOKEN)), None)
            .unwrap();
    control::start(&f.st, BridgeDefaults::default())
        .await
        .unwrap();
    let main = f.api("main");
    control::add_server(
        &f.st,
        "Pro only",
        Some(0),
        BridgeDefaults {
            account: Some(pro.id.clone()),
            account_only: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    control::start_server(&f.st, "pro-only", None, TokenPolicy::Local)
        .await
        .unwrap();
    let pro_api = f.api("pro-only");

    let theirs = create(&main, json!({})).await;
    let mine = create(&pro_api, json!({})).await;
    assert_eq!(f.st.bridge.thread(&mine).await.unwrap().account, pro.id);
    assert_eq!(
        created(&mine)["env"]["tokenSha256"],
        Secret::new(TOKEN).sha256_hex()
    );

    // Naming another account is refused; its own is fine.
    for (path, body) in [
        ("/threads", json!({ "account": "default" })),
        ("/chat", json!({ "message": "hi", "account": "default" })),
    ] {
        let (s, v) = post(&format!("{pro_api}{path}"), body).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{path}: {v}");
        assert_eq!(v["error"]["code"], "account_not_served");
    }
    create(&pro_api, json!({ "account": pro.id })).await;

    // Another account's conversation does not exist here, on every route
    // that names one.
    let routes: Vec<(&str, String, Value)> = vec![
        ("GET", format!("/threads/{theirs}"), Value::Null),
        ("GET", format!("/threads/{theirs}/messages"), Value::Null),
        ("GET", format!("/threads/{theirs}/context"), Value::Null),
        (
            "POST",
            format!("/threads/{theirs}/name"),
            json!({ "name": "x" }),
        ),
        ("POST", format!("/threads/{theirs}/fork"), json!({})),
        (
            "POST",
            format!("/threads/{theirs}/model"),
            json!({ "model": null }),
        ),
        (
            "POST",
            format!("/threads/{theirs}/permission-mode"),
            json!({ "permission_mode": "plan" }),
        ),
        (
            "POST",
            format!("/threads/{theirs}/turns"),
            json!({ "input": "hi" }),
        ),
        (
            "POST",
            format!("/threads/{theirs}/run"),
            json!({ "input": "hi" }),
        ),
        (
            "POST",
            format!("/threads/{theirs}/turns/t1/steer"),
            json!({ "input": "hi" }),
        ),
        (
            "POST",
            format!("/threads/{theirs}/turns/t1/interrupt"),
            json!({}),
        ),
        ("DELETE", format!("/threads/{theirs}"), Value::Null),
        (
            "POST",
            "/chat".to_string(),
            json!({ "message": "hi", "thread_id": theirs }),
        ),
        ("POST", "/threads".to_string(), json!({ "resume": theirs })),
    ];
    for (method, path, body) in &routes {
        let body = (!body.is_null()).then(|| body.clone());
        let (s, v) = call(method, &format!("{pro_api}{path}"), None, body.clone()).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{method} {path}: {v}");
        assert_eq!(v["error"]["code"], "thread_not_found", "{method} {path}");
        // The same conversation through the main server is there.
        if *method == "GET" {
            assert_eq!(
                call(method, &format!("{main}{path}"), None, body).await.0,
                StatusCode::OK
            );
        }
    }
    let (s, _) = get(&format!("{pro_api}/threads/{mine}")).await;
    assert_eq!(s, StatusCode::OK);

    // The listing: only its account's, live and stored (the Default's
    // directory is shared with the token account).
    let ids = |v: &Value, key: &str, field: &str| -> Vec<String> {
        v[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t[field].as_str().unwrap().to_string())
            .collect()
    };
    let (_, here) = get(&format!("{pro_api}/threads")).await;
    assert!(ids(&here, "data", "thread_id").contains(&mine));
    assert!(!ids(&here, "data", "thread_id").contains(&theirs));
    assert!(ids(&here, "stored", "sessionId").contains(&mine));
    assert!(
        !ids(&here, "stored", "sessionId").contains(&theirs),
        "{here}"
    );
    let (_, there) = get(&format!("{main}/threads")).await;
    assert!(ids(&there, "data", "thread_id").contains(&theirs));
    assert!(ids(&there, "data", "thread_id").contains(&mine));

    // Operations of another account's turns.
    let op = turn(&main, &theirs, "hello").await;
    for path in [
        format!("/operations/{op}"),
        format!("/operations/{op}/events"),
    ] {
        let (s, v) = get(&format!("{pro_api}{path}")).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{path}: {v}");
        assert_eq!(v["error"]["code"], "operation_not_found");
    }
    assert_eq!(
        get(&format!("{main}/operations/{op}")).await.0,
        StatusCode::OK
    );
    let own = turn(&pro_api, &mine, "hello").await;
    assert_eq!(
        get(&format!("{pro_api}/operations/{own}")).await.0,
        StatusCode::OK
    );

    // The event stream carries its account's conversations only.
    let seen = event_threads(&pro_api).await;
    assert!(seen.contains(&mine), "{seen:?}");
    assert!(seen.iter().all(|t| *t == mine), "{seen:?}");
    assert!(event_threads(&main).await.contains(&theirs));

    // Runtime requests of another account's conversation: not listed, not
    // answerable here; the main server answers it.
    turn(&main, &theirs, "ASK first").await;
    let rid = request_of(&main, &theirs).await;
    let (_, listed) = get(&format!("{pro_api}/requests")).await;
    assert!(
        listed["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["request_id"] != rid),
        "{listed}"
    );
    let (s, v) = post(
        &format!("{pro_api}/requests/{rid}/respond"),
        json!({ "decision": "allow" }),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    assert_eq!(v["error"]["code"], "request_not_found");
    turn(&pro_api, &mine, "ASK too").await;
    let own_rid = request_of(&pro_api, &mine).await;
    let (s, v) = post(
        &format!("{pro_api}/requests/{own_rid}/respond"),
        json!({ "decision": "allow" }),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, _) = post(
        &format!("{main}/requests/{rid}/respond"),
        json!({ "decision": "allow" }),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // Its sign-in is its account's; the main server's is the Default's.
    let (_, auth) = get(&format!("{pro_api}/auth")).await;
    assert_eq!(auth["account"]["id"], pro.id.as_str(), "{auth}");
    assert!(auth["login_hint"]
        .as_str()
        .unwrap()
        .contains("token account"));
    assert!(get(&format!("{main}/auth"))
        .await
        .1
        .get("account")
        .is_none());

    // It says which server it is and that it runs as one account only.
    let (_, caps) = get(&format!("{pro_api}/capabilities")).await;
    assert_eq!(caps["server"]["id"], "pro-only");
    assert_eq!(caps["server"]["accountOnly"], true);
    assert_eq!(caps["server"]["account"]["id"], pro.id.as_str());
    let (_, meta) = get(&format!("{main}/metadata")).await;
    assert_eq!(meta["server"]["id"], "main");
    assert_eq!(meta["server"]["accountOnly"], false);

    // An unaddressed message starts (and then joins) a conversation of its
    // own account, never the main server's.
    let (s, v) = post(&format!("{pro_api}/chat"), json!({ "message": "hi" })).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let joined = v["thread_id"].as_str().unwrap().to_string();
    assert_eq!(f.st.bridge.thread(&joined).await.unwrap().account, pro.id);
}

/// The saved servers come back after a restart, as they were saved; "open to
/// tunnels" never does.
#[tokio::test]
async fn api_servers_are_saved_and_come_back() {
    let home = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let data = scratch.path().join("app-data");
    let (st, _) = boot_in(home.path(), &data).await;
    let work = st
        .accounts
        .add_directory("Work", &scratch.path().join("claude-work"), None)
        .unwrap();
    let defaults = BridgeDefaults {
        model: Some("opus".into()),
        effort: Some("high".into()),
        account: Some(work.id.clone()),
        account_only: true,
        ..Default::default()
    };
    control::add_server(&st, "Work", Some(0), defaults.clone())
        .await
        .unwrap();
    control::add_server(&st, "Work", Some(0), BridgeDefaults::default())
        .await
        .expect_err("a name is taken once");
    control::add_server(&st, "Other", Some(0), BridgeDefaults::default())
        .await
        .unwrap();
    control::start_server(&st, "work", None, TokenPolicy::Never)
        .await
        .unwrap();
    assert_eq!(
        st.bridge.server("work").unwrap().token_policy(),
        TokenPolicy::Never
    );
    let file = data.join("api-servers.json");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(!text.contains("never") && !text.contains("token"), "{text}");
    control::stop_server(&st, "work").await.unwrap();
    drop(st);

    let (st, _) = boot_in(home.path(), &data).await;
    let ids: Vec<String> = st
        .bridge
        .servers
        .all()
        .iter()
        .map(|s| s.id.clone())
        .collect();
    assert_eq!(ids, ["main", "work", "other"]);
    let again = st.bridge.server("work").unwrap();
    assert_eq!(again.name(), "Work");
    assert_eq!(again.saved(), defaults);
    assert_eq!(again.defaults(), defaults);
    assert!(!again.is_running(), "nothing listens until it is started");
    assert_eq!(
        again.token_policy(),
        TokenPolicy::Local,
        "closed to tunnels again"
    );
    // The main server's defaults are never saved.
    assert!(!text.contains("\"main\""), "{text}");
}

/// `/servers/<id>` on the main API port and on the main app port: the target
/// server's rules — its token policy, its account, whether it is started —
/// and on the main app port a token always, never through a tunnel.
#[tokio::test]
async fn the_servers_prefix_gets_the_target_servers_rules() {
    use tower::ServiceExt as _;
    let f = boot().await;
    let work =
        f.st.accounts
            .add_directory("Work", &f.work_dir(), None)
            .unwrap();
    control::start(&f.st, BridgeDefaults::default())
        .await
        .unwrap();
    let main = f.api("main");
    control::add_server(
        &f.st,
        "Work",
        Some(0),
        BridgeDefaults {
            account: Some(work.id.clone()),
            account_only: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    control::start_server(&f.st, "work", None, TokenPolicy::Required)
        .await
        .unwrap();
    let script = f.st.auth.script_token.clone();
    let desk = f.st.auth.token.clone();
    let via = format!("{main}/servers/work");

    // Its token policy: Work wants a token, the main port it came through
    // does not.
    assert_eq!(get(&format!("{main}/capabilities")).await.0, StatusCode::OK);
    for base in [via.clone(), format!("{via}/v1")] {
        let (s, v) = get(&format!("{base}/capabilities")).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{base}: {v}");
        let (s, caps) = call("GET", &format!("{base}/capabilities"), Some(&script), None).await;
        assert_eq!(s, StatusCode::OK, "{base}");
        assert_eq!(caps["server"]["id"], "work");
        assert_eq!(caps["auth"]["token_policy"], "required");
    }

    // Its account: one only, and the conversations it makes are its.
    let (s, v) = call(
        "POST",
        &format!("{via}/threads"),
        Some(&script),
        Some(json!({ "account": "default" })),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"]["code"], "account_not_served");
    let (s, v) = call(
        "POST",
        &format!("{via}/v1/threads"),
        Some(&script),
        Some(json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let made =
        f.st.bridge
            .thread(v["thread_id"].as_str().unwrap())
            .await
            .unwrap();
    assert_eq!(
        (made.server.as_str(), made.account.as_str()),
        ("work", work.id.as_str())
    );

    // The document names the base it came through; the pages redirect within it.
    for (base, server) in [
        (via.clone(), "/servers/work"),
        (format!("{via}/v1"), "/servers/work/v1"),
    ] {
        let (_, doc) = get(&format!("{base}/openapi.json")).await;
        assert_eq!(doc["servers"][0]["url"], server);
        assert_eq!(doc["x-bridge-require-token"], true);
    }
    let no_redirect = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    for (url, to) in [
        (format!("{via}/docs"), "/servers/work/docs/"),
        (format!("{via}/v1/example"), "/servers/work/v1/example/"),
        (format!("{main}/v1/docs"), "/v1/docs/"),
    ] {
        let resp = no_redirect.get(&url).send().await.unwrap();
        assert_eq!(resp.status(), StatusCode::PERMANENT_REDIRECT, "{url}");
        assert_eq!(resp.headers()["location"], to, "{url}");
    }

    // A server that is not there.
    let (s, v) = get(&format!("{main}/servers/nope/threads")).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    assert_eq!(v["error"]["code"], "server_not_found");

    // Whether it is started: stopped, its routes are gated, discovery is not.
    control::stop_server(&f.st, "work").await.unwrap();
    let (s, v) = call("GET", &format!("{via}/threads"), Some(&script), None).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{v}");
    assert_eq!(v["error"]["code"], "bridge_not_started");
    assert!(v["error"]["message"].as_str().unwrap().contains("\"Work\""));
    let (s, _) = call("GET", &format!("{via}/health"), Some(&script), None).await;
    assert_eq!(s, StatusCode::OK);
    // Started again tokenless: so it is through the prefix too.
    control::start_server(&f.st, "work", None, TokenPolicy::Local)
        .await
        .unwrap();
    assert_eq!(get(&format!("{via}/threads")).await.0, StatusCode::OK);

    // The main app port: `/servers/<id>/v1`, and a token always — even for a
    // server that wants none on its own port.
    let app_via = format!("{}/servers/work/v1", f.app);
    let (s, _) = get(&format!("{app_via}/capabilities")).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, caps) = call("GET", &format!("{app_via}/capabilities"), Some(&desk), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(caps["auth"]["require_token"], true);
    assert_eq!(caps["server"]["id"], "work");
    // Never through a tunnel or proxy to it.
    let resp = reqwest::Client::new()
        .get(format!("{app_via}/capabilities"))
        .bearer_auth(&desk)
        .header("x-forwarded-for", "203.0.113.9")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    // The tunnel listener serves no API server at all.
    let tunnel = serve(mother_claude_lib::server::router_for(
        f.st.clone(),
        mother_claude_lib::server::Ingress::Tunnel,
    ))
    .await;
    let resp = reqwest::Client::new()
        .get(format!("{tunnel}/servers/work/v1/capabilities"))
        .header("x-forwarded-proto", "https")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // The LAN mount: a paired phone's token, from a private address.
    let lan = mother_claude_lib::server::router_for(
        f.st.clone(),
        mother_claude_lib::server::Ingress::Lan,
    );
    let (_, phone) = f.st.devices.issue(
        "Phone",
        "ios",
        None,
        mother_claude_lib::server::Ingress::Lan,
        "192.168.1.9".parse().unwrap(),
    );
    let ask = |token: Option<String>| {
        let mut req = axum::http::Request::get("/servers/work/v1/capabilities")
            .header("host", "192.168.1.5:6725");
        if let Some(t) = token {
            req = req.header("authorization", format!("Bearer {t}"));
        }
        let mut req = req.body(axum::body::Body::empty()).unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(SocketAddr::from((
                [192, 168, 1, 9],
                50000,
            ))));
        lan.clone().oneshot(req)
    };
    assert_eq!(ask(None).await.unwrap().status(), StatusCode::UNAUTHORIZED);
    assert_eq!(ask(Some(phone)).await.unwrap().status(), StatusCode::OK);
}

/// Tunnel detection tracks every server's port: a tunnel to one turns off
/// only that port's tokenless mode, and a request through `/servers/<id>` is
/// judged by the port it arrived on.
#[tokio::test]
async fn a_tunnel_to_one_servers_port_closes_only_that_port() {
    use mother_claude_lib::server::remote::{target_of, DetectedTunnel};
    let f = boot().await;
    control::start(&f.st, BridgeDefaults::default())
        .await
        .unwrap();
    control::add_server(&f.st, "Work", Some(0), BridgeDefaults::default())
        .await
        .unwrap();
    control::start_server(&f.st, "work", None, TokenPolicy::Local)
        .await
        .unwrap();
    let (main, work_api) = (f.api("main"), f.api("work"));
    let main_port = f.st.bridge.main().bound_port().unwrap();
    let work_port = f.st.bridge.server("work").unwrap().bound_port().unwrap();
    let desk = f.st.auth.token.clone();

    // Detection names the server a tunnel points at (none: the main one).
    assert_eq!(
        target_of(&f.st, Some(work_port)),
        ("bridge".into(), Some("work".into()))
    );
    assert_eq!(target_of(&f.st, Some(main_port)), ("bridge".into(), None));
    assert_eq!(target_of(&f.st, Some(1)), ("other".into(), None));
    let tunnel = |port: u16, server: Option<&str>| DetectedTunnel {
        provider: "ngrok".into(),
        url: format!("https://p{port}.ngrok.app"),
        upstream_port: Some(port),
        kind: "tcp".into(),
        target: "bridge".into(),
        server: server.map(str::to_string),
        ..Default::default()
    };
    let code = |v: &Value| v["error"]["code"].as_str().unwrap_or("").to_string();

    // A tunnel to Work's port: its tokenless mode is off; the main port's is not.
    f.st.remote.set_detected(
        vec![tunnel(work_port, Some("work"))],
        f.st.bridge.server_ports(),
    );
    let (s, v) = get(&format!("{work_api}/threads")).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "{v}");
    assert_eq!(code(&v), "bridge_exposed_requires_token");
    assert_eq!(get(&format!("{main}/threads")).await.0, StatusCode::OK);
    // …and Work through the main port, which no tunnel points at, keeps its
    // tokenless mode for this machine.
    assert_eq!(
        get(&format!("{main}/servers/work/threads")).await.0,
        StatusCode::OK
    );

    // The API screen and the Remote page name it.
    let (_, status) = call("GET", &format!("{}/api/bridge", f.app), Some(&desk), None).await;
    assert_eq!(status["exposedByTunnel"], false);
    assert_eq!(status["servers"][1]["id"], "work");
    assert_eq!(status["servers"][1]["exposedByTunnel"], true);
    assert_eq!(
        status["servers"][1]["tunnelUrl"],
        format!("https://p{work_port}.ngrok.app")
    );
    let (_, remote) = call("GET", &format!("{}/api/remote", f.app), Some(&desk), None).await;
    assert_eq!(remote["bridge"]["exposed"], false);
    assert_eq!(remote["bridge"]["servers"][1]["id"], "work");
    assert_eq!(remote["bridge"]["servers"][1]["exposed"], true);
    assert_eq!(remote["internet"]["detected"][0]["server"], "work");
    let warnings = remote["internet"]["warnings"].to_string();
    assert!(warnings.contains("the API server"), "{warnings}");
    assert!(warnings.contains("Work"), "{warnings}");

    // A tunnel to the main port instead: the main port, and everything
    // through it, needs a token; Work's own port does not.
    f.st.remote
        .set_detected(vec![tunnel(main_port, None)], f.st.bridge.server_ports());
    for url in [
        format!("{main}/threads"),
        format!("{main}/servers/work/threads"),
    ] {
        let (s, v) = get(&url).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{url}: {v}");
        assert_eq!(code(&v), "bridge_exposed_requires_token");
    }
    assert_eq!(get(&format!("{work_api}/threads")).await.0, StatusCode::OK);
    let (s, _) = call(
        "GET",
        &format!("{main}/servers/work/threads"),
        Some(&desk),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "a token still works");
}

/// The dashboard routes: add, read, change, start, stop and remove servers,
/// with their refusals; the account removal rule.
#[tokio::test]
async fn the_dashboard_manages_api_servers() {
    let f = boot().await;
    let desk = f.st.auth.token.clone();
    let script = f.st.auth.script_token.clone();
    let work =
        f.st.accounts
            .add_directory("Work", &f.work_dir(), None)
            .unwrap();
    let api = |path: &str| format!("{}/api/bridge{path}", f.app);

    // Add one: not started, a port of its own from 5613 up.
    let (s, v) = call(
        "POST",
        &api("/servers"),
        Some(&script),
        Some(json!({ "name": "Work", "defaults": { "account": work.id, "accountOnly": true } })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["id"], "work");
    assert_eq!(v["name"], "Work");
    assert_eq!(v["running"], false);
    assert_eq!(v["prefix"], "/servers/work");
    assert_eq!(v["defaults"]["accountOnly"], true);
    assert!(v["port"].as_u64().unwrap() >= 5613, "{v}");
    assert!(v.get("accountMissing").is_none());
    let port = v["port"].as_u64().unwrap();

    // Refusals, each naming what is wrong.
    for (body, status, code) in [
        (
            json!({ "name": "work" }),
            StatusCode::CONFLICT,
            "server_name_taken",
        ),
        (
            json!({ "name": "Main" }),
            StatusCode::CONFLICT,
            "server_name_taken",
        ),
        (
            json!({ "name": " " }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_field",
        ),
        (
            json!({ "name": "Low", "port": 80 }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_field",
        ),
        (
            json!({ "name": "Zero", "port": 0 }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_field",
        ),
        (
            json!({ "name": "Twin", "port": port }),
            StatusCode::CONFLICT,
            "port_unavailable",
        ),
        (
            json!({ "name": "Nobody", "defaults": { "account": "acc_000000000000" } }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_field",
        ),
        (
            json!({ "name": "Only", "defaults": { "accountOnly": true } }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_field",
        ),
        (
            json!({ "name": "Typo", "prot": 6000 }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "unknown_field",
        ),
        (
            json!({ "name": "Wild", "defaults": { "permissionMode": "bypassPermissions" } }),
            StatusCode::FORBIDDEN,
            "desktop_only",
        ),
    ] {
        let (s, v) = call("POST", &api("/servers"), Some(&script), Some(body.clone())).await;
        assert_eq!(s, status, "{body}: {v}");
        assert_eq!(v["error"]["code"], code, "{body}: {v}");
    }
    let (_, v) = call(
        "POST",
        &api("/servers"),
        Some(&script),
        Some(json!({ "name": "Twin", "port": port })),
    )
    .await;
    assert_eq!(v["error"]["server"], "work");

    // `servers` in the status, main first.
    let (_, status) = call("GET", &api(""), Some(&desk), None).await;
    let servers = status["servers"].as_array().unwrap();
    assert_eq!(servers.len(), 2);
    assert_eq!(servers[0]["id"], "main");
    assert_eq!(servers[0]["name"], "Main");
    assert_eq!(servers[0]["prefix"], "");
    assert_eq!(servers[1]["id"], "work");

    // Change it: a new name keeps the id.
    let (s, v) = call(
        "PATCH",
        &api("/servers/work"),
        Some(&script),
        Some(json!({ "name": "Work API" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        (v["id"].as_str(), v["name"].as_str()),
        (Some("work"), Some("Work API"))
    );
    for (path, status, code) in [
        ("/servers/main", StatusCode::CONFLICT, "main_server_fixed"),
        ("/servers/nope", StatusCode::NOT_FOUND, "server_not_found"),
    ] {
        let (s, v) = call(
            "PATCH",
            &api(path),
            Some(&script),
            Some(json!({ "name": "X" })),
        )
        .await;
        assert_eq!(
            (s, v["error"]["code"].as_str()),
            (status, Some(code)),
            "{path}"
        );
    }
    // A port the system picks, so it can start here (tests only).
    control::update_server(&f.st, "work", None, Some(0), None)
        .await
        .unwrap();

    // Start it: opening to tunnels is the desktop's alone.
    let (s, v) = call(
        "POST",
        &api("/servers/work/start"),
        Some(&script),
        Some(json!({ "openToTunnels": true })),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"]["code"], "desktop_only");
    let (s, v) = call(
        "POST",
        &api("/servers/work/start"),
        Some(&desk),
        Some(json!({ "openToTunnels": true })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["running"], true);
    assert_eq!(v["tokenPolicy"], "never");
    assert!(v["url"].as_str().unwrap().starts_with("http://127.0.0.1:"));
    // Started without defaults: the saved ones, closed to tunnels.
    let (_, v) = call(
        "POST",
        &api("/servers/work/start"),
        Some(&script),
        Some(json!({})),
    )
    .await;
    assert_eq!(v["tokenPolicy"], "local");
    assert_eq!(v["defaults"]["account"], work.id.as_str());

    // The account a running server uses cannot be removed.
    let account = format!("{}/api/accounts/{}", f.app, work.id);
    let (s, v) = call("DELETE", &account, Some(&desk), None).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "account_used_by_bridge");
    assert_eq!(v["error"]["server"], "work");
    assert_eq!(v["error"]["serverName"], "Work API");
    // Stopped, it can; the server keeps it, says so and does not start.
    let (s, v) = call("POST", &api("/servers/work/stop"), Some(&script), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["running"], false);
    let (s, _) = call("DELETE", &account, Some(&desk), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, status) = call("GET", &api(""), Some(&desk), None).await;
    assert_eq!(status["servers"][1]["accountMissing"], true);
    assert_eq!(
        status["servers"][1]["defaults"]["account"],
        work.id.as_str()
    );
    let (s, v) = call(
        "POST",
        &api("/servers/work/start"),
        Some(&script),
        Some(json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "account_not_found");
    // Another account chosen: it starts, and keeps that one.
    let (s, v) = call(
        "POST",
        &api("/servers/work/start"),
        Some(&script),
        Some(json!({ "defaults": { "account": "default", "accountOnly": true } })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v.get("accountMissing").is_none());
    assert_eq!(
        f.st.bridge
            .server("work")
            .unwrap()
            .saved()
            .account
            .as_deref(),
        Some("default")
    );

    // The conversation unaddressed messages join, per server.
    let thread = create(&f.api("work"), json!({})).await;
    let (s, v) = call(
        "POST",
        &api("/servers/work/active-thread"),
        Some(&script),
        Some(json!({ "threadId": thread })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["activeThread"], thread.as_str());
    assert!(f.st.bridge.main().active_thread().is_none());
    // Its conversations are listed under it, not under the main server.
    assert_eq!(v["threads"][0]["threadId"], thread.as_str());
    let (_, status) = call("GET", &api(""), Some(&desk), None).await;
    assert!(status["threads"].as_array().unwrap().is_empty(), "{status}");
    // The main server answers on these routes too, as a server like the others.
    let (s, v) = call("POST", &api("/servers/main/stop"), Some(&script), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        (v["id"].as_str(), v["running"].as_bool()),
        (Some("main"), Some(false))
    );
    let (s, v) = call(
        "POST",
        &api("/servers/work/active-thread"),
        Some(&script),
        Some(json!({ "threadId": "0a1b2c3d-0000-4000-8000-0000000000ff" })),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");

    // A running server changed is restarted with the change.
    let (s, v) = call(
        "PATCH",
        &api("/servers/work"),
        Some(&script),
        Some(json!({ "defaults": { "account": "default", "model": "haiku" } })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["running"], true);
    assert_eq!(v["defaults"]["model"], "haiku");
    let after = create(&f.api("work"), json!({})).await;
    assert_eq!(created(&after)["model"], "haiku");

    // Removed: stopped first, its conversations go on as the main server's;
    // with only the main server left the status reads as before again.
    let (s, _) = call("DELETE", &api("/servers/work"), Some(&script), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(f.st.bridge.server("work").is_none());
    assert_eq!(f.st.bridge.thread(&thread).await.unwrap().server, "main");
    let (_, status) = call("GET", &api(""), Some(&desk), None).await;
    assert!(status.get("servers").is_none(), "{status}");
    let (s, _) = call("DELETE", &api("/servers/main"), Some(&script), None).await;
    assert_eq!(s, StatusCode::CONFLICT);
}

/// Goal 7 (§13.3 rule 9): with only the Default account and only the main
/// server, every answer is what it was before API servers.
#[tokio::test]
async fn with_only_the_main_server_everything_reads_as_before() {
    let f = boot().await;
    let desk = f.st.auth.token.clone();
    control::start(&f.st, BridgeDefaults::default())
        .await
        .unwrap();
    let main = f.api("main");
    create(&main, json!({})).await;

    let (_, status) = call("GET", &format!("{}/api/bridge", f.app), Some(&desk), None).await;
    let mut keys: Vec<&str> = status
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    let mut before = vec![
        "activeOperations",
        "activeThread",
        "claudeAccount",
        "claudeAuthenticated",
        "configuredTokenPolicy",
        "consoleUrl",
        "defaults",
        "docsUrl",
        "enabled",
        "exposedByTunnel",
        "hostBuilt",
        "hostRunning",
        "messagesApi",
        "port",
        "requireToken",
        "running",
        "startedAt",
        "threads",
        "tokenPolicy",
        "tunnelUrl",
        "url",
    ];
    before.sort_unstable();
    assert_eq!(keys, before);
    let mut defaults: Vec<&str> = status["defaults"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    defaults.sort_unstable();
    assert_eq!(
        defaults,
        [
            "cwd",
            "defaultThread",
            "effort",
            "model",
            "permissionMode",
            "settingSources",
            "systemPromptAppend",
            "thinking"
        ]
    );
    assert!(status["threads"][0].get("server").is_none());

    // The bridge's own answers name no server either.
    let (_, caps) = get(&format!("{main}/capabilities")).await;
    assert!(caps.get("server").is_none(), "{caps}");
    let (_, meta) = get(&format!("{main}/metadata")).await;
    assert!(meta.get("server").is_none(), "{meta}");
    let (_, threads) = get(&format!("{main}/threads")).await;
    assert!(threads["data"][0].get("server").is_none(), "{threads}");
    let (_, remote) = call("GET", &format!("{}/api/remote", f.app), Some(&desk), None).await;
    let mut bridge: Vec<&str> = remote["bridge"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    bridge.sort_unstable();
    assert_eq!(bridge, ["exposed", "port", "running"]);
    assert!(!f.scratch.path().join("app-data/api-servers.json").exists());
    assert!(f.home.path().is_dir());
}
