//! Machines, Phase 5 (docs/PLAN-MACHINES.md §5.12, §5.13): three computers
//! and more. Each is an app state in this process with its own data dir and
//! iroh endpoint, finding the others through an in-memory address lookup
//! (no relay, no mDNS); everything goes through the real dashboard API and
//! the real QUIC transport.
//!
//! * An introduction pairs two computers only once both desktops accepted
//!   it and confirmed the six digits; declining leaves nothing; a forged
//!   one (from a key nobody paired, naming the computer itself or its
//!   introducer, or a computer that isn't the one introduced) is refused.
//! * *Revoke everywhere* cuts a computer from every paired computer, an
//!   offline one when it is back; the request only ever removes trust.
//! * Versions: what an older computer lacks, and *Update it* asked there.
//! * *Start Mother Claude when you sign in, in the tray*: the desktop's
//!   switch, only recorded in tests.
//!
//! Nothing touches `~/.claude` or `~/.claude.json` (`CLAUDE_CONFIG_DIR` is
//! set once, as `tests/moves.rs` does) or spawns `claude`.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use iroh::address_lookup::MemoryLookup;
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::machines::{self, protocol, transport};
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

async fn machine(lookup: &MemoryLookup, name: &str) -> Machine {
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
    st.machines.store_write(|f| f.name = Some(name.to_string()));
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

    fn id(&self) -> String {
        machines::machine_info(&self.st).id
    }

    fn key(&self) -> String {
        machines::machine_info(&self.st).key
    }

    /// The computers paired here.
    fn peers(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .st
            .machines
            .store_read(|f| f.peers.iter().map(|p| p.id.clone()).collect());
        ids.sort();
        ids
    }

    fn has(&self, other: &Machine) -> bool {
        self.peers().contains(&other.id())
    }

    /// Its audit lines with `action`, newest first.
    fn audited(&self, action: &str) -> Vec<Value> {
        self.st
            .devices
            .audit
            .recent(500)
            .into_iter()
            .rev()
            .filter(|e| e.action == action)
            .map(|e| serde_json::to_value(e).unwrap())
            .collect()
    }
}

fn state_of(v: &Value) -> &str {
    v["pairing"]["state"].as_str().unwrap_or("")
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

/// Pairs `a` (host) and `b` (joiner) by code through the dashboard API.
async fn pair(a: &Machine, b: &Machine) {
    let (s, v) = a.call("POST", "/machines/pair", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let code = v["pairing"]["code"].as_str().expect("a code").to_string();
    let (s, v) = b
        .call("POST", "/machines/join", Some(json!({ "code": code })))
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
            Some(json!({ "grants": grants(false) })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = a
        .call(
            "POST",
            "/machines/confirm",
            Some(json!({ "grants": grants(false) })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    b.wait("the joiner to finish", |v| state_of(v) == "paired")
        .await;
    assert!(a.has(b) && b.has(a));
}

fn key_of(m: &Machine, peer: &str) -> String {
    m.st.machines
        .store_read(|f| f.peer(peer).map(|p| p.key.clone()))
        .expect("paired")
}

/// `from` greets `to` (as the presence loop does every 20 s).
async fn greet(from: &Machine, to: &Machine) {
    machines::greet(from.st.clone(), to.id(), key_of(from, &to.id())).await;
}

/// Calls `f` until it returns `true` (45 s at most).
async fn eventually<F, Fut>(what: &str, mut f: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + Duration::from_secs(45);
    while !f().await {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// The introduction `m` received last.
fn intro_of(v: &Value) -> &Value {
    &v["introductions"][0]
}

/// A introduces `machine` to `with`: both are offered it.
async fn introduce(a: &Machine, machine: &Machine, with: &Machine) -> String {
    let (s, v) = a
        .call(
            "POST",
            "/machines/introduce",
            Some(json!({ "machine": machine.id(), "with": with.id() })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["host"]["state"], "offered", "{v}");
    assert_eq!(v["joiner"]["state"], "offered", "{v}");
    v["id"].as_str().unwrap().to_string()
}

async fn answer(m: &Machine, id: &str, accept: bool) -> Value {
    let (s, v) = m
        .call(
            "POST",
            &format!("/machines/introductions/{id}"),
            Some(json!({ "accept": accept })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
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

// ─── introductions ───────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_introduction_pairs_two_computers_only_once_both_confirm() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup, "MacBook").await;
    let b = machine(&lookup, "DESKTOP-PC").await;
    let c = machine(&lookup, "Laptop").await;
    pair(&a, &b).await;
    pair(&a, &c).await;

    // A knows whom each is paired with (ids only): B is not paired with C.
    greet(&a, &b).await;
    greet(&a, &c).await;
    let va = a.view().await;
    let b_on_a = va["peers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == b.id())
        .unwrap()
        .clone();
    assert!(
        !b_on_a["pairedWith"]
            .as_array()
            .unwrap()
            .contains(&json!(c.id())),
        "{b_on_a}"
    );

    // A introduces C to B: both are told, nothing is paired.
    let id = introduce(&a, &c, &b).await;
    let vb = b
        .wait("B to show the introduction", |v| {
            intro_of(v)["id"] == id.as_str()
        })
        .await;
    assert_eq!(intro_of(&vb)["from"], a.id());
    assert_eq!(intro_of(&vb)["fromName"], "MacBook");
    assert_eq!(intro_of(&vb)["role"], "host");
    assert_eq!(intro_of(&vb)["other"]["id"], c.id());
    assert_eq!(intro_of(&vb)["other"]["name"], "Laptop");
    assert_eq!(intro_of(&vb)["state"], "offered");
    let vc = c.view().await;
    assert_eq!(intro_of(&vc)["role"], "joiner");
    assert_eq!(intro_of(&vc)["other"]["id"], b.id());
    assert!(
        !b.has(&c) && !c.has(&b),
        "an introduction alone pairs nothing"
    );

    // C accepts first: it waits for B to accept too.
    answer(&c, &id, true).await;
    c.wait("C to wait for B", |v| v["pairing"]["waitingAccept"] == true)
        .await;
    assert_eq!(state_of(&c.view().await), "joining");

    // B accepts: the exchange runs, both show the same six digits.
    answer(&b, &id, true).await;
    let vb = b.wait("B's digits", |v| state_of(v) == "confirm").await;
    let vc = c.wait("C's digits", |v| state_of(v) == "confirm").await;
    let digits = vc["pairing"]["digits"].as_str().unwrap();
    assert_eq!(digits.len(), 6);
    assert_eq!(vb["pairing"]["digits"], vc["pairing"]["digits"]);
    assert_eq!(vb["pairing"]["intro"]["by"]["id"], a.id());
    assert_eq!(vb["pairing"]["other"]["id"], c.id());
    assert_eq!(vb["pairing"]["mustType"], false, "same computer: no typing");
    assert_eq!(vb["pairing"]["code"], Value::Null, "no code is shown");

    // One confirmation is not enough.
    let mut b_gives = grants(true);
    b_gives["move"] = json!({"lan": false, "internet": false});
    let (s, v) = b
        .call(
            "POST",
            "/machines/confirm",
            Some(json!({ "grants": b_gives })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(state_of(&v), "waitingOther");
    assert!(!b.has(&c) && !c.has(&b));

    // Both confirmed: each stores the other, with the grants each chose.
    let (s, v) = c
        .call(
            "POST",
            "/machines/confirm",
            Some(json!({ "grants": grants(false) })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    c.wait("C paired", |v| state_of(v) == "paired").await;
    b.wait("B paired", |v| state_of(v) == "paired").await;
    assert!(b.has(&c) && c.has(&b));
    let c_on_b =
        b.st.machines
            .store_read(|f| f.peer(&c.id()).cloned())
            .unwrap();
    assert_eq!(c_on_b.key, c.key(), "the key the introduction named");
    assert!(c_on_b.grants.run.lan && !c_on_b.grants.moves.lan);

    // Audited as introduced by A; the introduction is done.
    let line = &b.audited("machine.paired")[0];
    assert_eq!(line["detail"]["introducedBy"], a.id(), "{line}");
    assert_eq!(b.view().await["introductions"], json!([]));
    assert_eq!(a.audited("machine.introduce").len(), 1);

    // Introducing them again offers nothing: they are paired already.
    let (s, v) = a
        .call(
            "POST",
            "/machines/introduce",
            Some(json!({ "machine": c.id(), "with": b.id() })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["host"]["state"], "already", "{v}");
    assert_eq!(v["joiner"]["state"], "already", "{v}");
    assert_eq!(b.view().await["introductions"], json!([]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn declining_an_introduction_leaves_nothing() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup, "MacBook").await;
    let b = machine(&lookup, "DESKTOP-PC").await;
    let c = machine(&lookup, "Laptop").await;
    pair(&a, &b).await;
    pair(&a, &c).await;

    // Two of the same is no introduction.
    let (s, v) = a
        .call(
            "POST",
            "/machines/introduce",
            Some(json!({ "machine": b.id(), "with": b.id() })),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");

    let id = introduce(&a, &c, &b).await;
    b.wait("B to show it", |v| intro_of(v)["id"] == id.as_str())
        .await;
    // C accepts and waits; B says no: C is told, nothing is stored.
    answer(&c, &id, true).await;
    c.wait("C to wait for B", |v| v["pairing"]["waitingAccept"] == true)
        .await;
    let vb = answer(&b, &id, false).await;
    assert_eq!(intro_of(&vb)["state"], "declined", "{vb}");
    let vc = c
        .wait("C's pairing to end", |v| state_of(v) == "failed")
        .await;
    let code = vc["pairing"]["error"]["code"].as_str().unwrap();
    assert!(code == "intro_closed" || code == "pair_declined", "{vc:#}");
    assert!(!b.has(&c) && !c.has(&b), "declining stores nothing");
    assert_eq!(b.peers(), vec![a.id()]);
    assert_eq!(c.peers(), vec![a.id()]);
    // C's own introduction says so too.
    c.wait("C's introduction declined", |v| {
        intro_of(v)["state"] == "declined"
    })
    .await;
    // Answering it again is refused: it is closed.
    let (s, v) = b
        .call(
            "POST",
            &format!("/machines/introductions/{id}"),
            Some(json!({ "accept": true })),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_forged_introduction_is_refused() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup, "MacBook").await;
    let b = machine(&lookup, "DESKTOP-PC").await;
    let c = machine(&lookup, "Laptop").await;
    let d = machine(&lookup, "Intruder").await;
    pair(&a, &b).await;
    pair(&a, &c).await;
    pair(&d, &b).await;

    // A introduces only computers it holds.
    let (s, v) = a
        .call(
            "POST",
            "/machines/introduce",
            Some(json!({ "machine": d.id(), "with": b.id() })),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    assert_eq!(v["error"]["code"], "machines_unknown");
    // Only its desktop introduces.
    let script = a.st.auth.script_token.clone();
    let (s, v) = a
        .call_as(
            &script,
            "POST",
            "/machines/introduce",
            Some(json!({ "machine": c.id(), "with": b.id() })),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");

    // A key nobody paired never reaches B.
    let endpoint = transport::ensure_started(&b.st).await.unwrap();
    let stranger = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .relay_mode(iroh::RelayMode::Disabled)
        .bind()
        .await
        .unwrap();
    let conn = stranger
        .connect(endpoint.addr(), protocol::ALPN_PEER)
        .await
        .unwrap();
    let offer = json!({
        "id": "in_0123456789abcdef", "role": "host",
        "other": {"id": d.id(), "name": "x", "os": "macos", "key": d.key()}
    });
    let r = transport::request_on(
        &conn,
        "POST",
        "/peer/intro",
        Some(offer),
        Duration::from_secs(5),
    )
    .await;
    assert!(r.is_err(), "closed at the door: {r:?}");
    stranger.close().await;

    // A paired computer can't introduce B to itself, nor itself to B.
    let offer = |other: &Machine| {
        json!({
            "id": "in_0123456789abcdef", "role": "host",
            "other": {"id": other.id(), "name": "x", "os": "macos", "key": other.key()}
        })
    };
    for (other, code) in [(&b, "intro_self"), (&a, "intro_introducer")] {
        let (s, v) = transport::request(
            &a.st,
            &b.id(),
            "POST",
            "/peer/intro",
            Some(offer(other)),
            Duration::from_secs(10),
        )
        .await
        .unwrap();
        assert_eq!(s, 422, "{v}");
        assert_eq!(v["error"]["code"], code);
    }
    let (s, _) = transport::request(
        &a.st,
        &b.id(),
        "POST",
        "/peer/intro",
        Some(
            json!({"id": "nope", "role": "host", "other": {"id": c.id(), "name": "x", "key": "x"}}),
        ),
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert_eq!(s, 400);
    assert_eq!(b.view().await["introductions"], json!([]));

    // A real introduction of C to B: B accepts and waits for C's key only.
    let id = introduce(&a, &c, &b).await;
    b.wait("B to show it", |v| intro_of(v)["id"] == id.as_str())
        .await;
    answer(&b, &id, true).await;
    b.wait("B waiting", |v| state_of(v) == "waiting").await;
    // D, paired with B, comes for it with its own key: refused.
    let join = json!({
        "intro": id,
        "commit": "0".repeat(64),
        "machine": machines::machine_info(&d.st),
    });
    let (s, v) = transport::request(
        &d.st,
        &b.id(),
        "POST",
        "/peer/pair/join",
        Some(join),
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert_eq!(s, 403, "{v}");
    assert_eq!(v["error"]["code"], "intro_wrong_computer");
    // An introduction B never got: gone.
    let join = json!({
        "intro": "in_fedcba9876543210",
        "commit": "0".repeat(64),
        "machine": machines::machine_info(&d.st),
    });
    let (s, v) = transport::request(
        &d.st,
        &b.id(),
        "POST",
        "/peer/pair/join",
        Some(join),
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert_eq!(s, 409, "{v}");
    assert_eq!(v["error"]["code"], "pair_gone");
    assert_eq!(state_of(&b.view().await), "waiting", "still C's to join");
    assert!(!b.has(&c));
}

// ─── revoke everywhere ───────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoke_everywhere_cuts_a_computer_from_all_of_them() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup, "MacBook").await;
    let b = machine(&lookup, "DESKTOP-PC").await;
    let c = machine(&lookup, "Laptop").await;
    let d = machine(&lookup, "Lost laptop").await;
    pair(&a, &b).await;
    pair(&a, &c).await;
    pair(&a, &d).await;
    pair(&b, &d).await;
    pair(&c, &d).await;
    // D reaches B before.
    let (s, _) = transport::request(
        &d.st,
        &b.id(),
        "GET",
        "/peer/setup",
        None,
        Duration::from_secs(20),
    )
    .await
    .unwrap();
    assert_eq!(s, 200);
    let mut ws_b = dashboard(&b).await;

    // Only the desktop does it.
    let script = a.st.auth.script_token.clone();
    let (s, v) = a
        .call_as(
            &script,
            "POST",
            &format!("/machines/{}/revoke-everywhere", d.id()),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");

    let (s, v) = a
        .call(
            "POST",
            &format!("/machines/{}/revoke-everywhere", d.id()),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(!a.has(&d));
    let rev = &v["revocations"][0];
    assert_eq!(rev["machine"], d.id(), "{v:#}");
    assert_eq!(rev["name"], "Lost laptop");
    assert_eq!(rev["pending"], json!([]), "both online took it: {v:#}");
    let done: Vec<&str> = rev["done"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["id"].as_str().unwrap())
        .collect();
    assert!(done.contains(&b.id().as_str()) && done.contains(&c.id().as_str()));

    // D is gone on B and C, its connections cut, its key refused.
    assert!(!b.has(&d) && !c.has(&d));
    assert!(b.has(&a) && c.has(&a), "nothing else changed");
    for m in [&b, &c] {
        let r = transport::request(
            &d.st,
            &m.id(),
            "GET",
            "/peer/setup",
            None,
            Duration::from_secs(5),
        )
        .await;
        assert!(
            r.is_err() || r.as_ref().is_ok_and(|(s, _)| *s == 403),
            "D is refused: {r:?}"
        );
    }
    // Audited and shown on B.
    let line = &b.audited("machine.revoked")[0];
    assert_eq!(line["deviceId"], d.id(), "{line}");
    assert_eq!(line["detail"]["everywhere"], true);
    assert_eq!(line["detail"]["by"], a.id());
    let notice = frame_where(&mut ws_b, "B's notice", |f| {
        f["t"] == "notice"
            && f["text"]
                .as_str()
                .is_some_and(|t| t.contains("on all your computers"))
    })
    .await;
    assert_eq!(
        notice["text"],
        "MacBook revoked Lost laptop on all your computers."
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_offline_computer_takes_the_revocation_when_it_is_back() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup, "MacBook").await;
    let b = machine(&lookup, "DESKTOP-PC").await;
    let d = machine(&lookup, "Lost laptop").await;
    pair(&a, &b).await;
    pair(&a, &d).await;
    pair(&b, &d).await;

    transport::go_offline(&b.st).await;
    let (s, v) = a
        .call(
            "POST",
            &format!("/machines/{}/revoke-everywhere", d.id()),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let rev = &v["revocations"][0];
    assert_eq!(rev["pending"][0]["id"], b.id(), "{v:#}");
    assert!(b.has(&d), "B was away");
    // Kept across a restart of A's store.
    let kept = machines::store::MachinesFile::load(a.st.auth.config_dir.as_deref());
    assert_eq!(kept.revocations[0].pending, vec![b.id()]);

    // B is back: the next hello tells it.
    transport::come_back(&b.st);
    transport::ensure_started(&b.st).await.unwrap();
    eventually("B to take the revocation", || async {
        greet(&a, &b).await;
        !b.has(&d)
    })
    .await;
    a.wait("A to see it done", |v| {
        v["revocations"][0]["pending"] == json!([])
            && v["revocations"][0]["done"][0]["id"] == b.id().as_str()
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_revoke_request_names_a_third_computer_and_adds_nothing() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup, "MacBook").await;
    let b = machine(&lookup, "DESKTOP-PC").await;
    let c = machine(&lookup, "Laptop").await;
    pair(&b, &c).await;
    pair(&a, &c).await;
    let ask = |machine: String, key: String| json!({ "machine": machine, "key": key, "name": "x" });

    // B asks C to revoke B itself, or C: refused, nothing changes.
    for (body, code) in [
        (ask(b.id(), b.key()), "revoke_asker"),
        (ask(b.id(), "e".repeat(64)), "revoke_asker"),
        (ask(c.id(), c.key()), "revoke_self"),
        (ask("f".repeat(32), c.key()), "revoke_self"),
    ] {
        let (s, v) = transport::request(
            &b.st,
            &c.id(),
            "POST",
            "/peer/revoke",
            Some(body),
            Duration::from_secs(10),
        )
        .await
        .unwrap();
        assert_eq!(s, 422, "{v}");
        assert_eq!(v["error"]["code"], code);
    }
    // A computer C doesn't know: nothing to revoke, nothing added.
    let before = c.peers();
    let (s, v) = transport::request(
        &b.st,
        &c.id(),
        "POST",
        "/peer/revoke",
        Some(ask("d".repeat(32), "d".repeat(64))),
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["revoked"], false);
    assert_eq!(c.peers(), before);
    assert!(c.has(&a) && c.has(&b));
    // Malformed: refused.
    let (s, _) = transport::request(
        &b.st,
        &c.id(),
        "POST",
        "/peer/revoke",
        Some(json!({"machine": "../x", "key": "nope"})),
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert_eq!(s, 400);
    // A third computer it knows: revoked, by its key alone too.
    let (s, v) = transport::request(
        &b.st,
        &c.id(),
        "POST",
        "/peer/revoke",
        Some(ask("d".repeat(32), a.key())),
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["revoked"], true);
    assert!(!c.has(&a) && c.has(&b));
}

// ─── versions ────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_older_computer_is_named_with_what_it_lacks_and_asked_to_update() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup, "MacBook").await;
    let b = machine(&lookup, "DESKTOP-PC").await;
    let c = machine(&lookup, "Laptop").await;
    pair(&a, &b).await;
    pair(&a, &c).await;
    greet(&a, &b).await;
    let va = a.view().await;
    assert_eq!(va["self"]["versions"]["protocol"], protocol::PROTOCOL);
    let b_on_a = |v: &Value| {
        v["peers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == b.id())
            .unwrap()
            .clone()
    };
    assert_eq!(b_on_a(&va)["skew"], json!({}), "the same build");
    assert_eq!(
        b_on_a(&va)["versions"]["app"],
        env!("CARGO_PKG_VERSION"),
        "{va:#}"
    );

    // As if B ran an older build: named, with what it lacks.
    a.st.machines.store_write(|f| {
        let p = f.peer_mut(&b.id()).unwrap();
        p.versions.as_mut().unwrap().app = "0.0.1".into();
        p.capabilities = vec!["hello".into(), "setup".into(), "moves".into()];
    });
    let skew = b_on_a(&a.view().await)["skew"].clone();
    assert_eq!(skew["app"], "older", "{skew}");
    let lacks = skew["lacks"].as_array().unwrap();
    for cap in [
        "tasks",
        "teamRoles",
        "sync",
        "introductions",
        "revokeEverywhere",
    ] {
        assert!(lacks.contains(&json!(cap)), "{cap}: {skew}");
    }
    assert!(!lacks.contains(&json!("moves")));
    // …so it can't be introduced yet.
    let (s, v) = a
        .call(
            "POST",
            "/machines/introduce",
            Some(json!({ "machine": c.id(), "with": b.id() })),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "machine_too_old");

    // As if B ran a newer one: this computer's `app` check says to update.
    a.st.machines.store_write(|f| {
        f.peer_mut(&b.id()).unwrap().versions.as_mut().unwrap().app = "99.0.0".into();
    });
    let (s, v) = a.call("GET", "/machines/setup?fresh=true", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let app = v["self"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "app")
        .unwrap()
        .clone();
    assert_eq!(app["state"], "warning", "{app}");
    assert!(app["detail"]
        .as_str()
        .unwrap()
        .contains("older than DESKTOP-PC's 99.0.0"));

    // *Update it* from B: an ask on A that opens the Software updates page
    // there once someone confirms it — nothing installs on its own.
    let (s, v) = b
        .call(
            "POST",
            "/machines/setup/ask",
            Some(json!({ "machine": a.id(), "check": "app" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let ask_id = v["ask"]["id"].as_str().unwrap().to_string();
    let (s, v) = a
        .call(
            "POST",
            &format!("/machines/asks/{ask_id}"),
            Some(json!({ "answer": "confirm" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["next"], "/settings#updates", "{v}");
    assert_eq!(v["ask"]["state"], "done");
}

// ─── starting at sign-in ─────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn starting_at_sign_in_is_the_desktops_switch_and_only_recorded_here() {
    let lookup = MemoryLookup::new();
    let a = machine(&lookup, "MacBook").await;
    let (s, v) = a.call("GET", "/app/autostart", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["enabled"], false, "off by default");
    assert_eq!(v["supported"], true);
    assert_eq!(v["startedInTray"], false);

    // A local script may look, never switch it.
    let script = a.st.auth.script_token.clone();
    let (s, _) = a.call_as(&script, "GET", "/app/autostart", None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = a
        .call_as(
            &script,
            "PUT",
            "/app/autostart",
            Some(json!({ "enabled": true })),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"]["code"], "desktop_only");

    // The setup recommends it; its fix turns it on.
    let check = |v: &Value| {
        v["self"]["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == "autostart")
            .unwrap()
            .clone()
    };
    let (_, v) = a.call("GET", "/machines/setup?fresh=true", None).await;
    assert_eq!(check(&v)["state"], "warning", "{v:#}");
    assert_eq!(check(&v)["severity"], "recommended");
    let (s, v) = a
        .call(
            "POST",
            "/machines/setup/fix",
            Some(json!({ "check": "autostart" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(check(&v)["state"], "ok", "{v:#}");

    // The desktop's switch, audited.
    let (s, v) = a
        .call("PUT", "/app/autostart", Some(json!({ "enabled": false })))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["enabled"], false);
    assert!(!a.st.autostart.status().on);
    let line = &a.audited("app.autostart")[0];
    assert_eq!(line["detail"]["on"], false, "{line}");
}
