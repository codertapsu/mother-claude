//! Machines, Phase 1 (docs/PLAN-MACHINES.md): two app states in one process,
//! each with its own data dir and iroh endpoint, finding each other through
//! an in-memory address lookup (no relay, no mDNS). Everything goes through
//! the real dashboard API and the real QUIC transport: pairing with a code
//! and six digits, the hello, grants, revoking, and the door refusing a key
//! nobody paired. Nothing touches `~/.claude` or `~/.claude.json`
//! (`CLAUDE_CONFIG_DIR` is set) or spawns `claude`.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use iroh::address_lookup::MemoryLookup;
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::machines::{self, protocol, transport};
use mother_claude_lib::pending;
use mother_claude_lib::server::{self, auth::Auth};
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tempfile::TempDir;

struct Machine {
    st: AppState,
    url: String,
    _dir: TempDir,
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
        _dir: dir,
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

    /// Polls the view until `pred` holds (45 s at most: a wrong code is
    /// looked for during the whole 25 s window).
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

    fn id(&self) -> String {
        machines::machine_info(&self.st).id
    }
}

fn state_of(v: &Value) -> &str {
    v["pairing"]["state"].as_str().unwrap_or("")
}

/// Pairs `a` (host) and `b` (joiner) through the dashboard API; `b`'s grants
/// to `a` are `b_gives`, `a`'s to `b` are `a_gives`.
async fn pair(a: &Machine, b: &Machine, a_gives: Value, b_gives: Value) {
    let (s, v) = a.call("POST", "/machines/pair", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let code = v["pairing"]["code"].as_str().expect("a code").to_string();
    assert_eq!(code.len(), 9, "shown as XXXX-XXXX: {code}");
    assert_eq!(state_of(&v), "waiting");

    let (s, v) = b
        .call(
            "POST",
            "/machines/join",
            Some(json!({ "code": code.to_lowercase(), "name": "Joiner" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["pairing"]["role"], "joiner");

    let vb = b
        .wait("the joiner to show the digits", |v| {
            state_of(v) == "confirm"
        })
        .await;
    let va = a
        .wait("the host to show the digits", |v| state_of(v) == "confirm")
        .await;
    let digits = vb["pairing"]["digits"].as_str().unwrap();
    assert_eq!(digits.len(), 6);
    assert_eq!(
        va["pairing"]["digits"], vb["pairing"]["digits"],
        "both screens agree"
    );
    assert_eq!(va["pairing"]["mustType"], false, "same computer: no typing");
    assert_eq!(va["pairing"]["other"]["name"], "Joiner");
    assert_eq!(
        va["pairing"]["code"],
        Value::Null,
        "the code is gone once used"
    );

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

fn grants(run: bool) -> Value {
    json!({
        "view": {"lan": true, "internet": true},
        "control": {"lan": true, "internet": true},
        "move": {"lan": true, "internet": false},
        "run": {"lan": run, "internet": false},
        "dangerous": {"lan": false, "internet": false}
    })
}

/// Machines turned on, as *Turn on Machines* does once the setup passed (the
/// setup's own checks are not what these tests are about).
fn turn_on(ms: &[&Machine]) {
    for m in ms {
        machines::set_enabled(&m.st, true);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_computers_pair_greet_and_revoke() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let b = machine(&lookup).await;
    pair(&a, &b, grants(true), grants(false)).await;

    let va = a.view().await;
    let vb = b.view().await;
    let peer_on_a = &va["peers"][0];
    let peer_on_b = &vb["peers"][0];
    assert_eq!(va["peers"].as_array().unwrap().len(), 1);
    assert_eq!(peer_on_a["id"], b.id());
    assert_eq!(peer_on_b["id"], a.id());
    assert_eq!(peer_on_b["name"], machines::this_name(&a.st));
    assert_eq!(peer_on_a["name"], "Joiner", "the joiner's chosen name");
    // Each decided what the other may do on it.
    assert_eq!(peer_on_a["grants"]["run"]["lan"], true);
    assert_eq!(peer_on_b["grants"]["run"]["lan"], false);
    assert_eq!(peer_on_a["grants"]["move"]["internet"], false);
    // And learnt what it may do on the other.
    assert_eq!(peer_on_b["theirGrants"]["run"]["lan"], true);

    // The hello: online, by which path, with what the other says now.
    machines::greet(b.st.clone(), a.id(), key_of(&b, &a.id())).await;
    let vb = b
        .wait("the host online", |v| v["peers"][0]["online"] == true)
        .await;
    assert_eq!(vb["peers"][0]["path"], "lan");
    assert_eq!(vb["peers"][0]["versions"]["protocol"], protocol::PROTOCOL);
    machines::greet(a.st.clone(), b.id(), key_of(&a, &b.id())).await;
    let va = a
        .wait("the joiner's grants learnt", |v| {
            v["peers"][0]["theirGrants"]["run"]["lan"] == false
        })
        .await;
    assert_eq!(va["peers"][0]["online"], true);

    // The readiness test, both lines.
    let (s, v) = a
        .call("POST", "/machines/test", Some(json!({ "machine": b.id() })))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let tests = &v["results"][0]["tests"];
    assert_eq!(tests[0]["id"], "connect");
    assert_eq!(tests[0]["state"], "ok", "{v}");
    assert_eq!(tests[1]["id"], "transfer");
    assert_eq!(tests[1]["state"], "ok", "{v}");
    assert!(tests[1]["mibps"].as_f64().unwrap() > 0.0);

    // Revoking cuts it off: its key no longer gets through the door.
    let (s, v) = a
        .call("DELETE", &format!("/machines/{}", b.id()), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["peers"].as_array().unwrap().len(), 0);
    let r = transport::request(
        &b.st,
        &a.id(),
        "GET",
        "/peer/setup",
        None,
        Duration::from_secs(5),
    )
    .await;
    assert!(
        r.is_err() || r.as_ref().is_ok_and(|(s, _)| *s == 403),
        "a revoked machine is refused: {r:?}"
    );
}

fn key_of(m: &Machine, peer: &str) -> String {
    m.st.machines
        .store_read(|f| f.peer(peer).map(|p| p.key.clone()))
        .expect("paired")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_peer_reaches_only_its_routes_and_only_with_its_grants() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let b = machine(&lookup).await;
    pair(&a, &b, grants(false), grants(false)).await;
    turn_on(&[&a, &b]);

    // B may see A's checks…
    let (s, v) = transport::request(
        &b.st,
        &a.id(),
        "GET",
        "/peer/setup",
        None,
        Duration::from_secs(20),
    )
    .await
    .unwrap();
    assert_eq!(s, 200, "{v}");
    assert!(v["checks"].as_array().is_some_and(|c| !c.is_empty()));
    // …and the allow-listed part of its API (Phase 2), but never the rest
    // of it, the hooks or the MCP server.
    let (s, v) = transport::request(
        &b.st,
        &a.id(),
        "GET",
        "/api/sessions",
        None,
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert_eq!(s, 200, "{v}");
    for path in [
        "/api/machines",
        "/api/accounts",
        "/api/backup",
        "/api/me",
        "/hooks/event",
        "/mcp",
    ] {
        let (s, _) = transport::request(&b.st, &a.id(), "GET", path, None, Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(s, 404, "{path} is not mounted for another computer");
    }
    // Changing anything here stays closed to it.
    let (s, _) = transport::request(
        &b.st,
        &a.id(),
        "POST",
        "/api/machines/pair",
        Some(json!({})),
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert_eq!(s, 404);

    // Without "view", B is refused A's checks.
    let mut none = grants(false);
    none["view"] = json!({"lan": false, "internet": false});
    let (s, v) = a
        .call(
            "PATCH",
            &format!("/machines/{}", b.id()),
            Some(json!({ "grants": none })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        v["peers"][0]["grants"]["control"]["lan"], false,
        "driving needs seeing"
    );
    let (s, v) = transport::request(
        &b.st,
        &a.id(),
        "GET",
        "/peer/setup",
        None,
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert_eq!(s, 403, "{v}");
    assert_eq!(v["error"]["code"], "grant_missing");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_key_nobody_paired_is_refused_at_the_door() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let endpoint = transport::ensure_started(&a.st).await.unwrap();
    let stranger = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .relay_mode(iroh::RelayMode::Disabled)
        .bind()
        .await
        .unwrap();
    let conn = stranger
        .connect(endpoint.addr(), protocol::ALPN_PEER)
        .await
        .unwrap();
    let r = transport::request_on(
        &conn,
        "POST",
        "/peer/hello",
        Some(json!({})),
        Duration::from_secs(5),
    )
    .await;
    assert!(
        r.is_err(),
        "the connection is closed before any request: {r:?}"
    );
    stranger.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_wrong_code_finds_nobody_and_the_host_keeps_waiting() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let b = machine(&lookup).await;
    let (_, v) = a.call("POST", "/machines/pair", None).await;
    let code = v["pairing"]["code"].as_str().unwrap().to_string();
    // A different, well-formed code.
    let rest: String = code.chars().skip(1).collect();
    let wrong = if code.starts_with('0') {
        format!("1{rest}")
    } else {
        format!("0{rest}")
    };
    let (s, v) = b
        .call("POST", "/machines/join", Some(json!({ "code": wrong })))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let v = b
        .wait("the join to fail", |v| state_of(v) == "failed")
        .await;
    assert_eq!(v["pairing"]["error"]["code"], "pair_not_found", "{v}");
    assert_eq!(state_of(&a.view().await), "waiting");

    // A malformed code is refused at once.
    let (s, v) = b
        .call("POST", "/machines/join", Some(json!({ "code": "nope" })))
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"]["code"], "pair_code_malformed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn only_the_desktop_pairs_and_changes_machines() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let script = a.st.auth.script_token.clone();
    let (s, _) = a.call_as(&script, "GET", "/machines", None).await;
    assert_eq!(s, StatusCode::OK, "a local script may read");
    for (method, path) in [
        ("POST", "/machines/pair"),
        ("POST", "/machines/join"),
        ("POST", "/machines/enable"),
        ("PUT", "/machines/settings"),
        ("POST", "/machines/setup/fix"),
    ] {
        let (s, v) = a.call_as(&script, method, path, Some(json!({}))).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {path}: {v}");
        assert_eq!(v["error"]["code"], "desktop_only");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn machines_stays_off_while_a_required_check_fails() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    // Nothing paired: the pairing check blocks.
    let (s, v) = a
        .call("POST", "/machines/enable", Some(json!({ "enabled": true })))
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "machines_blocked");
    assert!(v["error"]["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|b| b["id"] == "pairing"));
    assert_eq!(a.view().await["enabled"], false);
    // Turning it off always works.
    let (s, _) = a
        .call(
            "POST",
            "/machines/enable",
            Some(json!({ "enabled": false })),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_relay_setting_is_validated_and_offered() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let (s, v) = a
        .call(
            "PUT",
            "/machines/settings",
            Some(json!({ "relay": { "mode": "custom", "url": "http://example.com" } })),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"]["code"], "machines_bad_relay");
    let (s, v) = a
        .call(
            "PUT",
            "/machines/settings",
            Some(json!({ "keepAwake": false })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["keepAwake"], false);
    // The fix turns it back on (fixes only record in tests, this one is ours).
    let (s, v) = a
        .call(
            "POST",
            "/machines/setup/fix",
            Some(json!({ "check": "keepAwake" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(a.view().await["settings"]["keepAwake"], true);
    assert!(a.st.machines.recorded_fixes().is_empty());
}

// ─── Phase 2: seeing and driving another computer's sessions ───────────────────

const SID: &str = "fed00000-1111-4222-8333-444455556666";

/// A finished session on `m`'s disk, and its monitor running, so `m`'s
/// session list has it.
fn session_on(m: &Machine) {
    session_saying(m, "hello from the other computer");
}

/// [`session_on`], its first prompt `prompt`.
fn session_saying(m: &Machine, prompt: &str) {
    let home = m._dir.path().join("home");
    let dir = home.join("projects").join("-tmp-fed");
    std::fs::create_dir_all(&dir).unwrap();
    let t0 = chrono::Utc::now().timestamp_millis() - 30_000;
    let iso = |ms: i64| {
        chrono::DateTime::from_timestamp_millis(ms)
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    };
    let lines = [
        json!({"type":"user","uuid":"f-u1","timestamp":iso(t0),"cwd":"/tmp/fed","permissionMode":"default","origin":{"kind":"human"},"message":{"role":"user","content":prompt}}),
        json!({"type":"assistant","uuid":"f-a1","timestamp":iso(t0 + 1000),"message":{"id":"msg_f1","model":"claude-haiku-4-5","content":[{"type":"text","text":"Hi."}],"usage":{"input_tokens":5,"output_tokens":1}}}),
    ];
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    std::fs::write(dir.join(format!("{SID}.jsonl")), text).unwrap();
    tokio::spawn(server::monitor::run(m.st.clone()));
}

async fn proxied(
    a: &Machine,
    token: &str,
    method: &str,
    b: &Machine,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    a.call_as(
        token,
        method,
        &format!("/machines/{}/proxy{path}", b.id()),
        body,
    )
    .await
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
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// `m`'s dashboard, signed in as its desktop.
async fn dashboard(m: &Machine) -> Ws {
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::Message;

    let ws_url = format!("{}/ws", m.url.replace("http://", "ws://"));
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url).await.unwrap();
    ws.send(Message::Text(
        json!({"t": "auth", "token": m.st.auth.token})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    ws
}

/// Reads `ws` until a frame satisfies `pred` (45 s at most).
async fn frame_where(ws: &mut Ws, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message;

    let deadline = Instant::now() + Duration::from_secs(45);
    while Instant::now() < deadline {
        let Ok(Some(Ok(Message::Text(text)))) =
            tokio::time::timeout(Duration::from_secs(5), ws.next()).await
        else {
            continue;
        };
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if pred(&v) {
            return v;
        }
    }
    panic!("timed out waiting for {what}");
}

/// A `remote` frame from `from` listing session [`SID`].
fn lists_sid(v: &Value, from: &str) -> bool {
    v["t"] == "remote"
        && v["machine"] == from
        && v["frame"]["t"] == "sessions"
        && [&v["frame"]["sessions"], &v["frame"]["upsert"]]
            .iter()
            .any(|l| {
                l.as_array()
                    .is_some_and(|l| l.iter().any(|s| s["id"] == SID))
            })
}

/// A permission prompt in `m`'s session [`SID`], raised as a channel raises
/// one; keep the guard while it should stay pending.
async fn prompt_on(
    m: &Machine,
) -> (
    pending::PendingGuard,
    tokio::sync::oneshot::Receiver<pending::Decision>,
    String,
) {
    let (guard, rx) = pending::raise(
        &m.st,
        pending::PendingItem {
            session_id: SID.into(),
            tool_name: Some("Bash".into()),
            title: "Run ls".into(),
            input: Some(json!({ "command": "ls" })),
            ..Default::default()
        },
    )
    .await;
    let id = guard.id().to_string();
    (guard, rx, id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_proxy_reads_and_drives_another_computers_sessions() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let b = machine(&lookup).await;
    pair(&a, &b, grants(false), grants(false)).await;
    turn_on(&[&a, &b]);
    session_on(&b);
    let desktop = a.st.auth.token.clone();

    // B's sessions, read through A.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let (s, v) = proxied(&a, &desktop, "GET", &b, "/sessions", None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let found = v["sessions"]
            .as_array()
            .is_some_and(|l| l.iter().any(|x| x["id"] == SID));
        if found {
            break;
        }
        assert!(Instant::now() < deadline, "B's session never showed: {v}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let (s, v) = proxied(
        &a,
        &desktop,
        "GET",
        &b,
        &format!("/sessions/{SID}/transcript"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // Not allow-listed: B's accounts, backups, devices, machines.
    for path in ["/accounts", "/backup", "/devices", "/machines", "/remote"] {
        let (s, _) = proxied(&a, &desktop, "GET", &b, path, None).await;
        assert_eq!(
            s,
            StatusCode::NOT_FOUND,
            "{path} is closed to another computer"
        );
    }

    // Driving reaches the handler (the session is not running here: not a
    // permission error).
    let (s, v) = proxied(
        &a,
        &desktop,
        "POST",
        &b,
        &format!("/sessions/{SID}/interrupt"),
        Some(json!({})),
    )
    .await;
    assert!(
        s != StatusCode::FORBIDDEN && s != StatusCode::NOT_FOUND
            || v["error"]["code"] == "session_not_found",
        "{s} {v}"
    );

    // Without "control" from B, A may only look.
    let mut look = grants(false);
    look["control"] = json!({"lan": false, "internet": false});
    let (s, v) = b
        .call(
            "PATCH",
            &format!("/machines/{}", a.id()),
            Some(json!({ "grants": look })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = proxied(
        &a,
        &desktop,
        "POST",
        &b,
        &format!("/sessions/{SID}/interrupt"),
        Some(json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"]["code"], "grant_missing");
    let (s, _) = proxied(&a, &desktop, "GET", &b, &format!("/sessions/{SID}"), None).await;
    assert_eq!(s, StatusCode::OK, "seeing still works");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dangerous_action_needs_the_grant_and_the_acting_persons_right() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let b = machine(&lookup).await;
    pair(&a, &b, grants(false), grants(false)).await;
    turn_on(&[&a, &b]);
    let desktop = a.st.auth.token.clone();
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
    let keys = format!("/sessions/{SID}/keys");
    let body = Some(json!({ "keys": "ls" }));

    // B has not given A the dangerous grant: nobody on A gets it.
    let (s, v) = proxied(&a, &desktop, "POST", &b, &keys, body.clone()).await;
    assert_eq!(v["error"]["code"], "dangerous_blocked", "{s} {v}");

    let mut risky = grants(false);
    risky["dangerous"] = json!({"lan": true, "internet": false});
    let (s, v) = b
        .call(
            "PATCH",
            &format!("/machines/{}", a.id()),
            Some(json!({ "grants": risky })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // The phone has no dangerous right on A: still refused on B.
    let (s, v) = proxied(&a, &phone, "POST", &b, &keys, body.clone()).await;
    assert_eq!(v["error"]["code"], "dangerous_blocked", "{s} {v}");
    // The desktop has it, and B granted it: past the gate (the session does
    // not exist there, which is the next answer).
    let (s, v) = proxied(&a, &desktop, "POST", &b, &keys, body).await;
    assert_ne!(v["error"]["code"], "dangerous_blocked", "{s} {v}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn another_computers_sessions_arrive_live_and_go_grey_when_it_stops() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let b = machine(&lookup).await;
    pair(&a, &b, grants(false), grants(false)).await;
    turn_on(&[&a, &b]);
    session_on(&b);

    let mut ws = dashboard(&a).await;
    let from = b.id();
    frame_where(
        &mut ws,
        "B's session on A's dashboard, as a remote frame",
        |v| lists_sid(v, &from),
    )
    .await;

    // A dashboard opened later gets them at once, and that B is online.
    let mut late = dashboard(&a).await;
    frame_where(&mut late, "B's sessions on connect", |v| {
        lists_sid(v, &from)
    })
    .await;
    frame_where(&mut late, "B online on connect", |v| {
        v["t"] == "remote" && v["machine"] == from && v["frame"]["t"] == "online"
    })
    .await;

    // B stops: A's dashboard is told it is offline (since then, not an
    // earlier state).
    let stopped = chrono::Utc::now().timestamp_millis();
    transport::go_offline(&b.st).await;
    frame_where(&mut ws, "A to be told B is offline", |v| {
        v["t"] == "remote"
            && v["machine"] == from
            && v["frame"]["t"] == "offline"
            && v["frame"]["since"].as_i64().is_some_and(|s| s >= stopped)
    })
    .await;
}

/// `GET /api/machines/sessions` on `m` with `token`, conditional on `etag`:
/// the status, the ETag and the body.
async fn remote_sessions(
    m: &Machine,
    token: &str,
    etag: Option<&str>,
) -> (StatusCode, Option<String>, Value) {
    let mut req = reqwest::Client::new()
        .get(format!("{}/api/machines/sessions", m.url))
        .bearer_auth(token);
    if let Some(e) = etag {
        req = req.header("If-None-Match", e);
    }
    let resp = req.send().await.unwrap();
    let status = resp.status();
    let etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let v = resp.json::<Value>().await.unwrap_or(Value::Null);
    (status, etag, v)
}

/// A dashboard on the polling fallback (a phone whose socket cannot connect)
/// reads the other computers' sessions from the federation's cache, as the
/// `remote` frames carry them (plan Phase 2 "Left for later", UX §6.1).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_polling_dashboard_reads_another_computers_sessions() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let b = machine(&lookup).await;
    pair(&a, &b, grants(false), grants(false)).await;
    turn_on(&[&a, &b]);
    session_on(&b);
    let from = b.id();
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

    // A paired phone (any authenticated caller) reads B's sessions through
    // A, each stamped with B.
    let deadline = Instant::now() + Duration::from_secs(45);
    let (etag, v) = loop {
        let (s, etag, v) = remote_sessions(&a, &phone, None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let listed = v["machines"].as_array().and_then(|l| {
            l.iter().find(|m| {
                m["machine"] == from
                    && m["sessions"]
                        .as_array()
                        .is_some_and(|s| s.iter().any(|x| x["id"] == SID))
            })
        });
        if listed.is_some() {
            break (etag.expect("an ETag"), v);
        }
        assert!(Instant::now() < deadline, "B's session never showed: {v}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert_eq!(v["enabled"], true);
    let m = &v["machines"][0];
    assert_eq!(m["name"], "Joiner", "named as the user knows it: {m}");
    assert_eq!(m["online"], true);
    assert_eq!(m["since"], Value::Null);
    assert_eq!(m["loaded"], true);
    let s = m["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == SID)
        .unwrap();
    assert_eq!(s["machine"], from, "stamped with its computer: {s}");

    // Nothing changed: a conditional GET is a 304, until something does.
    let (s, _, _) = remote_sessions(&a, &phone, Some(&etag)).await;
    if s != StatusCode::NOT_MODIFIED {
        // A diff arrived between the two reads (the monitor's next pass):
        // the next one settles.
        let (_, again, _) = remote_sessions(&a, &phone, None).await;
        let (s, _, _) = remote_sessions(&a, &phone, again.as_deref()).await;
        assert_eq!(s, StatusCode::NOT_MODIFIED);
    }

    // B stops: it reads offline since then, its sessions kept as last heard.
    let stopped = chrono::Utc::now().timestamp_millis();
    transport::go_offline(&b.st).await;
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let (_, _, v) = remote_sessions(&a, &phone, None).await;
        let m = &v["machines"][0];
        if m["online"] == false {
            assert!(
                m["since"].as_i64().is_some_and(|s| s >= stopped),
                "offline since then: {m}"
            );
            assert!(
                m["sessions"]
                    .as_array()
                    .is_some_and(|s| s.iter().any(|x| x["id"] == SID)),
                "greyed, not gone: {m}"
            );
            break;
        }
        assert!(Instant::now() < deadline, "B never read offline: {v}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // Machines off here: the lists are this computer's alone.
    machines::set_enabled(&a.st, false);
    let (s, _, v) = remote_sessions(&a, &phone, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v, json!({ "enabled": false, "machines": [] }));

    // Never another computer's to read: not allow-listed for a peer (so
    // not through its proxy either).
    assert_eq!(
        machines::routes::peer_route_grant(&axum::http::Method::GET, "/machines/sessions"),
        None
    );
}

/// Another computer's runs and tasks changes reach this computer's
/// dashboards through the federation (a page showing one of its sessions
/// refetches its background tasks); nothing else of its Machines does.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn another_computers_runs_and_tasks_changes_reach_this_dashboard() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let b = machine(&lookup).await;
    pair(&a, &b, grants(false), grants(false)).await;
    turn_on(&[&a, &b]);
    let mut ws = dashboard(&a).await;
    let from = b.id();
    frame_where(&mut ws, "B online on A", |v| {
        v["t"] == "remote" && v["machine"] == from && v["frame"]["t"] == "online"
    })
    .await;
    machines::publish(&b.st, "peers");
    machines::publish(&b.st, "setup");
    machines::publish(&b.st, "tasks");
    let v = frame_where(&mut ws, "B's tasks changed, on A", |v| {
        v["t"] == "remote" && v["machine"] == from && v["frame"]["t"] == "machines"
    })
    .await;
    assert_eq!(
        v["frame"]["what"], "tasks",
        "nothing else of its Machines: {v}"
    );
    machines::publish(&b.st, "runs");
    let v = frame_where(&mut ws, "B's runs changed, on A", |v| {
        v["t"] == "remote" && v["machine"] == from && v["frame"]["t"] == "machines"
    })
    .await;
    assert_eq!(v["frame"]["what"], "runs");
}

/// Bulk transfers yield to the API (plan §5.3): a chunk going up and a file
/// coming down run at the lower QUIC priority on the side that sends them;
/// plain requests and the sessions' stream keep the default.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_streams_yield_to_the_api() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let b = machine(&lookup).await;
    pair(&a, &b, grants(false), grants(false)).await;
    turn_on(&[&a, &b]);
    session_on(&b);
    // The sessions' stream and plain requests: never lowered.
    let mut ws = dashboard(&a).await;
    let from = b.id();
    frame_where(&mut ws, "B's sessions on A", |v| lists_sid(v, &from)).await;
    let (s, v) = transport::request(
        &a.st,
        &b.id(),
        "GET",
        "/peer/roots",
        None,
        transport::REQUEST_TIMEOUT,
    )
    .await
    .unwrap();
    assert_eq!(s, 200, "{v}");
    let (a0, b0) = (a.st.machines.bulk_streams(), b.st.machines.bulk_streams());
    assert_eq!((a0, b0), (0, 0), "nothing bulk yet");

    // A file coming down (the readiness test's transfer): B's answer yields.
    let (n, _) = transport::bulk_from(&a.st, &b.id(), 1).await.unwrap();
    assert_eq!(n, 1 << 20);
    assert_eq!(b.st.machines.bulk_streams(), 1);
    assert_eq!(a.st.machines.bulk_streams(), 0, "its request was no bulk");

    // A chunk going up: A's request yields (whatever B answers).
    let _ = machines::chunks::put(
        &a.st,
        &b.id(),
        "/peer/moves/mv_0123456789abcdef/files/move.bundle?offset=0",
        vec![1, 2, 3],
        "",
    )
    .await;
    assert_eq!(a.st.machines.bulk_streams(), 1);
    assert_eq!(b.st.machines.bulk_streams(), 1, "its refusal is JSON");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_approval_is_answered_from_either_computer_and_the_first_answer_wins() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let b = machine(&lookup).await;
    let (s, v) = a
        .call("PATCH", "/machines/self", Some(json!({ "name": "Studio" })))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    pair(&a, &b, grants(false), grants(false)).await;
    turn_on(&[&a, &b]);
    session_on(&b);
    let desktop = a.st.auth.token.clone();
    let respond = format!("/sessions/{SID}/respond");

    // A's dashboard is linked to B (B's sessions reach it) before B asks.
    let mut ws = dashboard(&a).await;
    let from = b.id();
    frame_where(&mut ws, "A's link to B", |v| lists_sid(v, &from)).await;

    // B's session asks; A's dashboard shows the card…
    let (_held, rx, id) = prompt_on(&b).await;
    frame_where(&mut ws, "B's prompt on A's dashboard", |v| {
        v["t"] == "remote"
            && v["machine"] == from
            && v["frame"]["t"] == "pending"
            && v["frame"]["item"]["id"] == id.as_str()
    })
    .await;

    // …and answers it through the proxy: B's session gets the answer, and
    // B names the computer that answered, not itself.
    let (s, v) = proxied(
        &a,
        &desktop,
        "POST",
        &b,
        &respond,
        Some(json!({ "requestId": id, "decision": "allow" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["resolution"]["text"], "Allowed by Studio", "{v}");
    let d = pending::wait(rx, Duration::from_secs(10))
        .await
        .expect("B's session got A's answer");
    assert!(d.is_allow());
    // B's own desktop is too late.
    let (s, v) = b
        .call(
            "POST",
            &respond,
            Some(json!({ "requestId": id, "decision": "deny" })),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "already_decided");

    // The other way round: B answers first, A's answer is refused.
    let (_held, rx, id) = prompt_on(&b).await;
    let (s, v) = b
        .call(
            "POST",
            &respond,
            Some(json!({ "requestId": id, "decision": "deny" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let d = pending::wait(rx, Duration::from_secs(10))
        .await
        .expect("B's session got B's answer");
    assert!(!d.is_allow());
    let (s, v) = proxied(
        &a,
        &desktop,
        "POST",
        &b,
        &respond,
        Some(json!({ "requestId": id, "decision": "allow" })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "already_decided");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_same_session_id_on_two_computers_stays_two_sessions() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let b = machine(&lookup).await;
    pair(&a, &b, grants(false), grants(false)).await;
    turn_on(&[&a, &b]);
    // One id on both disks (as a backup pull leaves it), each its own.
    session_saying(&a, "said on A");
    session_saying(&b, "said on B");
    let desktop = a.st.auth.token.clone();
    let transcript = format!("/sessions/{SID}/transcript");

    // Here, A's own; through the proxy, B's.
    eventually("A's own copy", || async {
        let (s, v) = a.call("GET", &transcript, None).await;
        s == StatusCode::OK && v.to_string().contains("said on A")
    })
    .await;
    eventually("B's copy through A", || async {
        let (s, v) = proxied(&a, &desktop, "GET", &b, &transcript, None).await;
        s == StatusCode::OK && v.to_string().contains("said on B")
    })
    .await;
    let (_, mine) = a.call("GET", &transcript, None).await;
    assert!(!mine.to_string().contains("said on B"), "{mine}");
    let (_, theirs) = proxied(&a, &desktop, "GET", &b, &transcript, None).await;
    assert!(!theirs.to_string().contains("said on A"), "{theirs}");

    // A's own list has one session with that id; B's arrives beside it,
    // tagged with B.
    let (_, list) = a.call("GET", "/sessions", None).await;
    let here = list["sessions"]
        .as_array()
        .map(|l| l.iter().filter(|s| s["id"] == SID).count());
    assert_eq!(here, Some(1), "{list}");
    let mut ws = dashboard(&a).await;
    let from = b.id();
    frame_where(&mut ws, "B's copy, tagged with B", |v| lists_sid(v, &from)).await;

    // B's prompt stays B's: A's own copy shows none and cannot answer it.
    let (_held, _rx, id) = prompt_on(&b).await;
    let detail = format!("/sessions/{SID}");
    let (_, theirs) = proxied(&a, &desktop, "GET", &b, &detail, None).await;
    let prompts = |v: &Value| v["pendingDetail"].as_array().map(Vec::len);
    assert_eq!(prompts(&theirs), Some(1), "{theirs}");
    let (_, mine) = a.call("GET", &detail, None).await;
    assert_eq!(prompts(&mine), Some(0), "{mine}");
    let (s, v) = a
        .call(
            "POST",
            &format!("/sessions/{SID}/respond"),
            Some(json!({ "requestId": id, "decision": "allow" })),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    assert_eq!(v["error"]["code"], "request_not_found");
}

/// A WebSocket to `to`'s `/ws` over `from`'s link, as the federation opens
/// one: `Err` with the answer when it is refused.
async fn open_peer_ws(from: &Machine, to: &Machine) -> Result<(), String> {
    let conn = transport::connection_to(&from.st, &key_of(from, &to.id())).await?;
    let (send, recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
    tokio_tungstenite::client_async("ws://peer/ws", tokio::io::join(recv, send))
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn while_machines_is_off_another_computer_only_pairs_and_runs_the_setup() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup).await;
    let b = machine(&lookup).await;
    pair(&a, &b, grants(false), grants(false)).await;
    session_on(&b);
    let desktop = a.st.auth.token.clone();
    assert_eq!(a.view().await["enabled"], false, "off until turned on");

    // Off on both. What pairing and the setup need still answers: the
    // hello, the checks, the roots, the leases, the readiness test.
    machines::greet(a.st.clone(), b.id(), key_of(&a, &b.id())).await;
    a.wait("B online by its hello", |v| v["peers"][0]["online"] == true)
        .await;
    for path in ["/peer/setup", "/peer/roots", "/peer/leases"] {
        let (s, v) = transport::request(&a.st, &b.id(), "GET", path, None, Duration::from_secs(20))
            .await
            .unwrap();
        assert_eq!(s, 200, "{path} while off: {v}");
    }
    let (s, v) = a
        .call("POST", "/machines/test", Some(json!({ "machine": b.id() })))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["results"][0]["tests"][1]["state"], "ok", "{v}");

    // Nothing that uses it: A's proxy refuses here (409), B refuses its
    // session routes, moves and session stream (403).
    let (s, v) = proxied(&a, &desktop, "GET", &b, "/sessions", None).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "machines_off");
    for (method, path, body) in [
        ("GET", "/api/sessions", None),
        ("POST", "/peer/moves/preflight", Some(json!({}))),
        ("GET", "/peer/moves/mv_0123456789abcdef", None),
    ] {
        let (s, v) =
            transport::request(&a.st, &b.id(), method, path, body, Duration::from_secs(20))
                .await
                .unwrap();
        assert_eq!(s, 403, "{method} {path}: {v}");
        assert_eq!(v["error"]["code"], "machines_off", "{path}");
        assert_eq!(
            v["error"]["message"],
            format!("Machines is off on {}.", machines::this_name(&b.st))
        );
    }
    let e = open_peer_ws(&a, &b)
        .await
        .expect_err("B's sessions refused");
    assert!(e.contains("403"), "{e}");

    // On only on B: B serves, but A neither proxies nor links to it.
    turn_on(&[&b]);
    open_peer_ws(&a, &b)
        .await
        .expect("B serves its sessions now");
    let (s, v) = proxied(&a, &desktop, "GET", &b, "/sessions", None).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert_eq!(
        a.view().await["peers"][0]["sessions"],
        Value::Null,
        "A dials no link while off"
    );

    // On on both: A sees and drives B's sessions, live.
    turn_on(&[&a]);
    a.wait("B's session listed on A", |v| {
        v["peers"][0]["sessions"]["total"] == 1
    })
    .await;
    let (s, v) = proxied(&a, &desktop, "GET", &b, "/sessions", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // B turns it off again: its socket to A closes and A is refused.
    let mut ws = dashboard(&a).await;
    let from = b.id();
    frame_where(&mut ws, "B online on A's dashboard", |v| {
        v["t"] == "remote" && v["machine"] == from && v["frame"]["t"] == "online"
    })
    .await;
    let (s, v) = b
        .call(
            "POST",
            "/machines/enable",
            Some(json!({ "enabled": false })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    frame_where(&mut ws, "B's link closed", |v| {
        v["t"] == "remote" && v["machine"] == from && v["frame"]["t"] == "offline"
    })
    .await;
    let (s, v) = proxied(&a, &desktop, "GET", &b, "/sessions", None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"]["code"], "machines_off");
}
