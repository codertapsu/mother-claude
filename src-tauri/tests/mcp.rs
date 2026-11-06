//! Mother Claude's MCP server over real HTTP (docs/MCP.md): who may call
//! `/mcp` and from where, the JSON-RPC surface, the calling session's
//! identity, and the dashboard's panel. Nothing here touches `~/.claude` or
//! spawns `claude`, except the `#[ignore]`d live test at the end (Spike B),
//! which runs the real CLI and spends tokens.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::mcp::{Registry, ToolCtx, ToolDef, ToolOutput};
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::server::{self, Ingress};
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use reqwest::StatusCode;
use serde_json::{json, Value};

const SID: &str = "0f1e2d3c-4b5a-4968-8776-655443322110";
const OWNED: &str = "1a2b3c4d-5e6f-4a0b-8c1d-2e3f4a5b6c7d";

fn state_in(home: &Path) -> AppState {
    Inner::new(
        ClaudeHome::with_base(home),
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        Auth::ephemeral(),
    )
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
    base: String,
    st: AppState,
    home: tempfile::TempDir,
}

async fn boot() -> Fixture {
    let home = tempfile::tempdir().unwrap();
    let st = state_in(home.path());
    let base = serve(server::router(st.clone())).await;
    Fixture { base, st, home }
}

fn rpc(id: Value, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

impl Fixture {
    fn token(&self) -> String {
        self.st.mcp.token().to_string()
    }

    fn post(&self, body: &Value) -> reqwest::RequestBuilder {
        reqwest::Client::new()
            .post(format!("{}/mcp", self.base))
            .bearer_auth(self.token())
            .json(body)
    }

    /// As a client after `initialize` (Claude Code sends the negotiated
    /// version on every later request; without it the server assumes
    /// 2025-03-26, which has no `structuredContent`).
    fn post_current(&self, body: &Value) -> reqwest::RequestBuilder {
        self.post(body).header("mcp-protocol-version", "2025-06-18")
    }

    async fn call(&self, body: &Value) -> Value {
        let r = self.post_current(body).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()["content-type"], "application/json");
        r.json().await.unwrap()
    }
}

#[tokio::test]
async fn only_the_mcp_token_from_this_machine_gets_in() {
    let f = boot().await;
    let c = reqwest::Client::new();
    let ping = rpc(json!(1), "ping", json!({}));
    let url = format!("{}/mcp", f.base);

    // No token, a wrong one, a forged one with the right prefix.
    let r = c.post(&url).json(&ping).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    assert!(
        r.headers().get("www-authenticate").is_none(),
        "no OAuth challenge"
    );
    let script = f.st.auth.script_token.clone();
    for wrong in ["nope", "mcm_forged", script.as_str()] {
        let r = c
            .post(&url)
            .bearer_auth(wrong)
            .json(&ping)
            .send()
            .await
            .unwrap();
        assert!(
            r.status() == StatusCode::UNAUTHORIZED || r.status() == StatusCode::FORBIDDEN,
            "{wrong}: {}",
            r.status()
        );
    }
    // Other principals are not MCP clients.
    for (token, what) in [
        (f.st.auth.token.clone(), "desktop"),
        (f.st.auth.hook_token.clone(), "hook"),
    ] {
        let r = c
            .post(&url)
            .bearer_auth(&token)
            .json(&ping)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN, "{what}");
        let v: Value = r.json().await.unwrap();
        assert_eq!(v["error"]["code"], "wrong_credential", "{what}");
    }
    // The token works.
    let v = f.call(&ping).await;
    assert_eq!(v, json!({"jsonrpc": "2.0", "id": 1, "result": {}}));

    // A web page (any Origin that is not this machine's), a tunnel, a
    // rebinding host name.
    for origin in ["https://evil.example", "null", "tauri://localhost"] {
        let r = f.post(&ping).header("origin", origin).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN, "{origin}");
    }
    let r = f
        .post(&ping)
        .header("origin", "http://localhost:6274")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK, "a loopback origin is fine");
    let r = f
        .post(&ping)
        .header("x-forwarded-for", "203.0.113.9")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND, "through a tunnel");
    let r = f
        .post(&ping)
        .header("host", "evil.example:6725")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND, "DNS rebinding");

    // The MCP token reaches nothing else.
    for path in ["/api/sessions", "/api/me"] {
        let r = c
            .get(format!("{}{path}", f.base))
            .bearer_auth(f.token())
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN, "{path}");
    }
    let r = c
        .post(format!("{}/hooks/event", f.base))
        .bearer_auth(f.token())
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = c
        .get(format!("{}/v1/threads", f.base))
        .bearer_auth(f.token())
        .send()
        .await
        .unwrap();
    assert!(r.status().is_client_error() || r.status().is_server_error());
}

#[tokio::test]
async fn other_listeners_never_serve_mcp() {
    let home = tempfile::tempdir().unwrap();
    let st = state_in(home.path());
    let ping = rpc(json!(1), "ping", json!({}));
    for ingress in [Ingress::Lan, Ingress::Tunnel] {
        let base = serve(server::router_for(st.clone(), ingress)).await;
        let r = reqwest::Client::new()
            .post(format!("{base}/mcp"))
            .bearer_auth(st.mcp.token())
            .header("x-forwarded-proto", "https")
            .json(&ping)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::NOT_FOUND, "{ingress:?}");
    }
}

#[tokio::test]
async fn initialize_negotiates_and_opens_a_session() {
    let f = boot().await;
    let r = f
        .post(&rpc(
            json!(0),
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "claude-code", "version": "2.1.319"}
            }),
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let session = r.headers()["mcp-session-id"].to_str().unwrap().to_string();
    assert_eq!(session.len(), 32);
    assert_eq!(r.headers()["cache-control"], "no-store");
    let v: Value = r.json().await.unwrap();
    let res = &v["result"];
    assert_eq!(res["protocolVersion"], "2025-06-18");
    assert_eq!(
        res["capabilities"],
        json!({"tools": {"listChanged": false}})
    );
    assert_eq!(res["serverInfo"]["name"], "mother-claude");
    assert_eq!(res["serverInfo"]["title"], "Mother Claude");

    // What Claude Code 2.1.319 asks for, an old client, an unknown version.
    for (asked, answered) in [
        ("2025-11-25", "2025-11-25"),
        ("2024-11-05", "2024-11-05"),
        ("2026-07-28", "2025-11-25"),
    ] {
        let v = f
            .call(&rpc(
                json!(1),
                "initialize",
                json!({"protocolVersion": asked, "capabilities": {}}),
            ))
            .await;
        assert_eq!(v["result"]["protocolVersion"], answered, "{asked}");
    }

    // The CLI probes `server/discover` first; -32601 makes it initialize.
    let v = f
        .call(&json!({
            "jsonrpc": "2.0", "id": "server-discover-probe-1", "method": "server/discover",
            "params": {"_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28"}}
        }))
        .await;
    assert_eq!(v["id"], "server-discover-probe-1");
    assert_eq!(v["error"]["code"], -32601);

    // The session id is accepted later, and so is an unknown one (an app
    // restart forgets them all); neither is required.
    for sid in [session.as_str(), "deadbeefdeadbeefdeadbeefdeadbeef"] {
        let r = f
            .post(&rpc(json!(2), "ping", json!({})))
            .header("mcp-session-id", sid)
            .header("mcp-protocol-version", "2026-07-28")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK, "{sid}");
    }
    let r = reqwest::Client::new()
        .delete(format!("{}/mcp", f.base))
        .bearer_auth(f.token())
        .header("mcp-session-id", &session)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    // No event stream.
    let r = reqwest::Client::new()
        .get(format!("{}/mcp", f.base))
        .bearer_auth(f.token())
        .header("accept", "text/event-stream")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn notifications_batches_and_errors() {
    let f = boot().await;
    // A notification: 202, no body.
    let r = f
        .post(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::ACCEPTED);
    assert!(r.bytes().await.unwrap().is_empty());
    // A client's response to us: 202 too.
    let r = f
        .post(&json!({"jsonrpc": "2.0", "id": 9, "result": {}}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::ACCEPTED);

    // A batch: answers for the requests only, in order.
    let v = f
        .call(&json!([
            rpc(json!(1), "ping", json!({})),
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            rpc(json!("two"), "tools/list", json!({})),
            {"jsonrpc": "2.0", "id": 3},
        ]))
        .await;
    let a = v.as_array().unwrap();
    assert_eq!(a.len(), 3);
    assert_eq!(a[0]["id"], 1);
    assert_eq!(a[1]["id"], "two");
    assert!(a[1]["result"]["tools"].is_array());
    assert_eq!(a[2]["error"]["code"], -32600);
    // A batch of notifications only: 202.
    let r = f
        .post(&json!([{"jsonrpc": "2.0", "method": "notifications/initialized"}]))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::ACCEPTED);

    // Protocol errors.
    let v = f.call(&rpc(json!(4), "resources/list", json!({}))).await;
    assert_eq!(v["error"]["code"], -32601);
    let v = f
        .call(&rpc(
            json!(5),
            "tools/call",
            json!({"name": "no_such_tool"}),
        ))
        .await;
    assert_eq!(v["error"]["code"], -32602);
    let v = f
        .call(&rpc(
            json!(6),
            "tools/call",
            json!({"name": "mc_ping", "arguments": [1]}),
        ))
        .await;
    assert_eq!(v["error"]["code"], -32602);
    let v = f
        .call(&rpc(json!(7), "tools/list", json!({"cursor": "x"})))
        .await;
    assert_eq!(v["error"]["code"], -32602);
    let v = f
        .call(&json!({"jsonrpc": "1.0", "id": 8, "method": "ping"}))
        .await;
    assert_eq!(v["error"]["code"], -32600);
    assert_eq!(v["id"], 8);
    let r = reqwest::Client::new()
        .post(format!("{}/mcp", f.base))
        .bearer_auth(f.token())
        .header("content-type", "application/json")
        .body("{not json")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"]["code"], -32700);
    assert!(v["id"].is_null());
    // Too large.
    let big = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\",\"params\":{{\"x\":\"{}\"}}}}",
        "a".repeat(17 * 1024 * 1024)
    );
    let r = reqwest::Client::new()
        .post(format!("{}/mcp", f.base))
        .bearer_auth(f.token())
        .header("content-type", "application/json")
        .body(big)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn tools_are_listed_and_called() {
    let f = boot().await;
    let v = f.call(&rpc(json!(1), "tools/list", json!({}))).await;
    let tools = v["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"mc_ping"), "{names:?}");
    let ping = tools.iter().find(|t| t["name"] == "mc_ping").unwrap();
    assert_eq!(ping["inputSchema"]["type"], "object");
    assert_eq!(ping["annotations"]["readOnlyHint"], true);

    let v = f
        .call(&rpc(
            json!(2),
            "tools/call",
            json!({"name": "mc_ping", "arguments": {}}),
        ))
        .await;
    let res = &v["result"];
    assert!(res.get("isError").is_none());
    assert_eq!(res["structuredContent"]["app"], "Mother Claude");
    assert!(res["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Mother Claude"));

    // An older client gets no structuredContent and no titles.
    let r = f
        .post(&rpc(json!(3), "tools/call", json!({"name": "mc_ping"})))
        .header("mcp-protocol-version", "2025-03-26")
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert!(v["result"].get("structuredContent").is_none());
    let r = f
        .post(&rpc(json!(4), "tools/list", json!({})))
        .header("mcp-protocol-version", "2024-11-05")
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    let t = &v["result"]["tools"][0];
    assert!(t.get("title").is_none() && t.get("annotations").is_none());
}

#[tokio::test]
async fn the_caller_is_known_by_its_pid_or_its_owned_session() {
    let f = boot().await;
    let me = std::process::id();
    std::fs::create_dir_all(f.home.path().join("sessions")).unwrap();
    std::fs::write(
        f.home.path().join(format!("sessions/{me}.json")),
        json!({"pid": me, "sessionId": SID, "cwd": "/tmp/mcp-demo", "kind": "interactive"})
            .to_string(),
    )
    .unwrap();
    let r = f
        .post_current(&rpc(json!(1), "tools/call", json!({"name": "mc_ping"})))
        .header("x-mc-pid", me.to_string())
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["result"]["structuredContent"]["session"], SID);
    assert_eq!(v["result"]["structuredContent"]["owned"], false);

    // An owned session names itself; a session nobody owns cannot.
    let call = |sid: &str| {
        f.post_current(&rpc(json!(3), "tools/call", json!({"name": "mc_ping"})))
            .header("x-mc-session", sid)
            .send()
    };
    let v: Value = call(OWNED).await.unwrap().json().await.unwrap();
    assert!(v["result"]["structuredContent"]["session"].is_null());
    f.st.control.note_owned(OWNED, true);
    let v: Value = call(OWNED).await.unwrap().json().await.unwrap();
    assert_eq!(v["result"]["structuredContent"]["session"], OWNED);
    assert_eq!(v["result"]["structuredContent"]["owned"], true);

    // Each call is remembered with who made it, newest first.
    let calls = f.st.mcp.recent_calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].session.as_deref(), Some(OWNED));
    assert!(calls[0].owned);
    assert_eq!(calls[1].session, None);
    assert_eq!(calls[2].session.as_deref(), Some(SID));
    assert!(calls.iter().all(|c| c.tool == "mc_ping" && c.ok));
}

/// A request naming another running process — as a `curl` in one session's
/// Bash could, the token being readable — is nobody: the connection is not
/// that process's. Needs `lsof` (the check cannot run without it).
#[cfg(unix)]
#[tokio::test]
async fn a_request_naming_another_process_is_nobody() {
    let lsof = ["/usr/sbin/lsof", "/usr/bin/lsof", "/sbin/lsof"]
        .iter()
        .any(|p| Path::new(p).is_file());
    if !lsof {
        return;
    }
    let f = boot().await;
    let mut other = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let pid = other.id();
    std::fs::create_dir_all(f.home.path().join("sessions")).unwrap();
    std::fs::write(
        f.home.path().join(format!("sessions/{pid}.json")),
        json!({"pid": pid, "sessionId": SID, "cwd": "/tmp/elsewhere"}).to_string(),
    )
    .unwrap();
    let r = f
        .post_current(&rpc(json!(1), "tools/call", json!({"name": "mc_ping"})))
        .header("x-mc-pid", pid.to_string())
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert!(v["result"]["structuredContent"]["session"].is_null(), "{v}");
    assert_eq!(f.st.mcp.recent_calls()[0].session, None);
    let _ = other.kill();
    let _ = other.wait();
}

#[tokio::test]
async fn mc_session_describes_the_calling_session() {
    let f = boot().await;
    let v = f.call(&rpc(json!(1), "tools/list", json!({}))).await;
    let names: Vec<&str> = v["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"mc_session"), "{names:?}");
    // Nobody it can tell: a result saying so, not a protocol error.
    let v = f
        .call(&rpc(json!(2), "tools/call", json!({"name": "mc_session"})))
        .await;
    assert_eq!(v["result"]["isError"], true);
    assert!(v["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .ends_with("(unknown_session)"));

    let me = std::process::id();
    std::fs::create_dir_all(f.home.path().join("sessions")).unwrap();
    std::fs::write(
        f.home.path().join(format!("sessions/{me}.json")),
        json!({"pid": me, "sessionId": SID, "cwd": "/tmp/mcp-demo", "kind": "interactive"})
            .to_string(),
    )
    .unwrap();
    let r = f
        .post_current(&rpc(json!(3), "tools/call", json!({"name": "mc_session"})))
        .header("x-mc-pid", me.to_string())
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    let s = &v["result"]["structuredContent"];
    assert_eq!(s["session"], SID, "{v}");
    assert_eq!(s["cwd"], "/tmp/mcp-demo");
    assert_eq!(s["pid"], me);
    assert_eq!(s["owned"], false);
}

#[tokio::test]
async fn the_panel_lists_calls_and_never_the_token() {
    let f = boot().await;
    let me = std::process::id();
    std::fs::create_dir_all(f.home.path().join("sessions")).unwrap();
    std::fs::write(
        f.home.path().join(format!("sessions/{me}.json")),
        json!({"pid": me, "sessionId": SID, "cwd": "/tmp/mcp-demo"}).to_string(),
    )
    .unwrap();
    for id in 1..=3 {
        let r = f
            .post_current(&rpc(
                json!(id),
                "tools/call",
                json!({"name": "mc_ping", "arguments": {"secret": "do not show"}}),
            ))
            .header("x-mc-pid", me.to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
    }
    let p: Value = reqwest::Client::new()
        .get(format!("{}/api/mcp", f.base))
        .bearer_auth(&f.st.auth.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(p["enabled"], true);
    // Every area's tools (core, browser, teams): however many there are.
    assert_eq!(p["toolCount"], mother_claude_lib::mcp::registry().len());
    assert_eq!(p["toolPrefix"], "mcp__plugin_mother-claude_mc__");
    assert_eq!(p["url"], "http://127.0.0.1:0/mcp");
    assert_eq!(p["callsTotal"], 3);
    let calls = p["recentCalls"].as_array().unwrap();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0]["session"], SID);
    assert_eq!(calls[0]["owned"], false);
    assert_eq!(calls[0]["tool"], "mc_ping");
    assert_eq!(calls[0]["ok"], true);
    assert!(calls.iter().all(|c| c.get("arguments").is_none()));
    assert!(
        !p.to_string().contains("do not show"),
        "never the arguments"
    );
    assert!(
        !p.to_string().contains(&f.token()),
        "the token is never served"
    );
    assert!(p["plugin"]["expectedVersion"]
        .as_str()
        .unwrap()
        .ends_with("+mcp.1"));
    // A poll leaves the plugin out (it may run the CLI), and so does every
    // answer to someone other than the desktop.
    for (query, token) in [
        ("?plugin=false", f.st.auth.token.clone()),
        ("", f.st.auth.script_token.clone()),
    ] {
        let p: Value = reqwest::Client::new()
            .get(format!("{}/api/mcp{query}", f.base))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(p.get("plugin").is_none(), "{query:?}: {p}");
        assert_eq!(p["recentCalls"].as_array().unwrap().len(), 3);
    }
    // A session's own credential does not read the panel.
    let r = reqwest::Client::new()
        .get(format!("{}/api/mcp", f.base))
        .bearer_auth(f.token())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_setting_turns_it_off_for_everyone_and_only_the_desktop_flips_it() {
    let f = boot().await;
    let c = reqwest::Client::new();
    let put = |token: &str, on: bool| {
        c.put(format!("{}/api/mcp/settings", f.base))
            .bearer_auth(token)
            .json(&json!({"enabled": on}))
            .send()
    };
    let r = put(&f.st.auth.script_token, false).await.unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN, "a local script cannot");
    let r = put(&f.token(), false).await.unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN, "nor can a session");
    let r = put(&f.st.auth.token, false).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["enabled"], false);
    assert_eq!(v["ownedSessions"], false);

    let r = f
        .post(&rpc(json!(1), "ping", json!({})))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"]["code"], "mcp_disabled");

    let r = put(&f.st.auth.token, true).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let r = f
        .post(&rpc(json!(1), "ping", json!({})))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    // Unknown fields are refused, not ignored.
    let r = c
        .put(format!("{}/api/mcp/settings", f.base))
        .bearer_auth(&f.st.auth.token)
        .json(&json!({"enabled": true, "open": "wide"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

fn def(name: &str) -> ToolDef {
    ToolDef {
        name: name.into(),
        title: None,
        description: "test".into(),
        input_schema: json!({"type": "object"}),
        annotations: None,
    }
}

#[tokio::test]
async fn a_refusal_is_a_result_and_only_changes_are_audited() {
    use mother_claude_lib::mcp::{ToolAnnotations, ToolError};
    let f = boot().await;
    let mut r = Registry::default();
    r.add(def("refuses"), |_c: ToolCtx, _a: Value| async move {
        Err(ToolError::new("denied", "Not on this site"))
    });
    r.add(
        ToolDef {
            annotations: Some(ToolAnnotations {
                read_only_hint: Some(true),
                ..Default::default()
            }),
            ..def("looks")
        },
        |_c: ToolCtx, _a: Value| async move { Ok(ToolOutput::text("seen")) },
    );
    f.st.mcp.replace_registry(r);
    let audited = || {
        f.st.devices
            .audit
            .recent(usize::MAX)
            .into_iter()
            .filter(|e| e.action == "mcp.call")
            .collect::<Vec<_>>()
    };

    let v = f
        .call(&rpc(
            json!(1),
            "tools/call",
            json!({"name": "refuses", "arguments": {"url": "https://bank.example"}}),
        ))
        .await;
    assert!(v.get("error").is_none(), "not a protocol error: {v}");
    assert_eq!(v["result"]["isError"], true);
    assert_eq!(
        v["result"]["content"][0]["text"],
        "Not on this site (denied)"
    );
    let lines = audited();
    assert_eq!(lines.len(), 1);
    let detail = lines[0].detail.clone().unwrap();
    assert_eq!(detail["tool"], "refuses");
    assert_eq!(detail["ok"], false);
    assert_eq!(lines[0].principal.as_deref(), Some("mcp"));
    let line = serde_json::to_string(&lines[0]).unwrap();
    assert!(!line.contains("bank.example"), "never the arguments");
    assert!(!line.contains(&f.token()), "never the token");

    // A read-only tool is not audited (the panel still lists it).
    f.call(&rpc(json!(2), "tools/call", json!({"name": "looks"})))
        .await;
    assert_eq!(audited().len(), 1);
    assert_eq!(f.st.mcp.recent_calls()[0].tool, "looks");
}

#[tokio::test]
async fn a_call_is_cancelled_by_its_client() {
    let f = boot().await;
    let mut r = Registry::default();
    r.add(def("wait"), |_c: ToolCtx, _a: Value| async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        Ok(ToolOutput::text("late"))
    });
    f.st.mcp.replace_registry(r);
    let pending = tokio::spawn(
        f.post(&rpc(json!(7), "tools/call", json!({"name": "wait"})))
            .header("mcp-session-id", "client-a")
            .send(),
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    // Another client's cancel of the same id changes nothing.
    let r = f
        .post(
            &json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
                      "params": {"requestId": 7}}),
        )
        .header("mcp-session-id", "client-b")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::ACCEPTED);
    assert!(!pending.is_finished());
    let r = f
        .post(
            &json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
                      "params": {"requestId": 7, "reason": "user pressed Esc"}}),
        )
        .header("mcp-session-id", "client-a")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::ACCEPTED);
    let resp = tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .expect("answered once cancelled")
        .unwrap()
        .unwrap();
    let v: Value = resp.json().await.unwrap();
    assert_eq!(v["result"]["isError"], true);
    assert!(v["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .ends_with("(cancelled)"));
    let calls = f.st.mcp.recent_calls();
    assert_eq!(calls[0].tool, "wait");
    assert!(!calls[0].ok);
}

/// Spike B on the real CLI (docs/MCP.md "Spike B"): a throwaway server on an
/// ephemeral port, the plugin written into a temp data dir and loaded with
/// `--plugin-dir`, and `claude -p` (haiku, $0.25 cap) asked to call
/// `mc_ping`. Checks that the tool is listed and called under its plugin
/// name and that the server knew the session from `X-MC-Pid`.
///
/// ```sh
/// cargo test --manifest-path src-tauri/Cargo.toml --test mcp -- --ignored --nocapture
/// ```
#[tokio::test]
#[ignore = "runs the real claude CLI and spends tokens (haiku, $0.25 cap)"]
async fn live_claude_code_calls_mc_ping_through_the_plugin() {
    use mother_claude_lib::claude::hooks::PluginPaths;
    use mother_claude_lib::mcp::plugin;

    let home = tempfile::tempdir().unwrap();
    let data = tempfile::Builder::new()
        .prefix("mc mcp spike ")
        .tempdir()
        .unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let st = state_in(home.path());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    assert!(![6725, 6726, 5612].contains(&port));
    let app = server::router(st.clone());
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    // The real plugin folder (hooks, scripts, `.mcp.json`) in a data dir with
    // a space in its path, as on macOS.
    let paths = PluginPaths::new(data.path());
    plugin::write_all(&paths, port, st.mcp.token()).unwrap();

    let session = uuid::Uuid::new_v4().to_string();
    let tool = format!("{}mc_ping", plugin::TOOL_PREFIX);
    let mut cmd = tokio::process::Command::new(
        std::env::var("MOTHER_CLAUDE_CLI").unwrap_or_else(|_| "claude".into()),
    );
    // The prompt first: `--allowedTools` is variadic and would take it.
    cmd.current_dir(cwd.path())
        .args([
            "-p",
            "Call the mc_ping tool once and print exactly what it returns.",
        ])
        .args(["--model", "haiku", "--max-budget-usd", "0.25"])
        .args(["--output-format", "stream-json", "--verbose"])
        .args(["--setting-sources", "project,local"])
        .args(["--session-id", &session])
        .arg("--plugin-dir")
        .arg(&paths.plugin.root)
        .args(["--allowedTools", &tool])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    for k in [
        "CLAUDECODE",
        "CLAUDE_CODE_ENTRYPOINT",
        "CLAUDE_CODE_SESSION_ID",
        "CLAUDE_CODE_MESSAGING_SOCKET",
        "CLAUDE_CODE_MESSAGING_TOKEN",
        "CLAUDE_CODE_CHILD_SESSION",
        "CLAUDE_CODE_SSE_PORT",
        "CLAUDE_CODE_SESSION_ATTENDED",
        "CLAUDE_CODE_EXECPATH",
        "CLAUDE_PID",
        "CLAUDE_AGENT_SDK_VERSION",
        "MOTHER_CLAUDE_TOKEN",
        "MOTHER_CLAUDE_SESSION",
    ] {
        cmd.env_remove(k);
    }
    // The user's installed plugin (if its hooks load at all) stays quiet.
    cmd.env("MOTHER_CLAUDE_OWNED", "1");
    let child = cmd.spawn().expect("the claude CLI");
    let pid = child.id().unwrap();
    // The CLI writes its real pidfile under the real config dir; the server
    // here reads a synthetic one, so put the same record there.
    std::fs::create_dir_all(home.path().join("sessions")).unwrap();
    std::fs::write(
        home.path().join(format!("sessions/{pid}.json")),
        json!({"pid": pid, "sessionId": session, "cwd": cwd.path(), "kind": "interactive",
               "entrypoint": "sdk-cli"})
        .to_string(),
    )
    .unwrap();
    let out = tokio::time::timeout(Duration::from_secs(240), child.wait_with_output())
        .await
        .expect("claude finished")
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut listed = Vec::new();
    let mut used = Vec::new();
    let mut result = String::new();
    for line in stdout.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match v["type"].as_str() {
            Some("system") if v["subtype"] == "init" => {
                for t in v["tools"].as_array().into_iter().flatten() {
                    if let Some(t) = t.as_str().filter(|t| t.contains("mother-claude")) {
                        listed.push(t.to_string());
                    }
                }
                println!("mcp_servers: {}", v["mcp_servers"]);
            }
            Some("assistant") => {
                for b in v["message"]["content"].as_array().into_iter().flatten() {
                    if b["type"] == "tool_use" {
                        used.push(b["name"].as_str().unwrap_or_default().to_string());
                    }
                }
            }
            Some("result") => {
                result = v["result"].as_str().unwrap_or_default().to_string();
                println!("cost: {}", v["total_cost_usd"]);
            }
            _ => {}
        }
    }
    println!("listed: {listed:?}\nused: {used:?}\nresult: {result}");
    println!("stderr: {}", String::from_utf8_lossy(&out.stderr));
    let calls = st.mcp.recent_calls();
    println!("server saw: {calls:?}");
    assert!(listed.contains(&tool), "the model was offered {tool}");
    assert!(used.contains(&tool), "the model called {tool}");
    let call = calls
        .iter()
        .find(|c| c.tool == "mc_ping")
        .expect("the server ran mc_ping");
    assert!(call.ok);
    assert_eq!(
        call.session.as_deref(),
        Some(session.as_str()),
        "known by X-MC-Pid"
    );
    assert!(result.contains(&session), "the answer names the session");
    // The CLI holds its MCP connection itself: the binding check passed.
    println!("binding: {:?}", st.mcp.last_binding(pid));
    #[cfg(target_os = "macos")]
    assert_eq!(
        st.mcp.last_binding(pid),
        Some(mother_claude_lib::server::hooks::Binding::Verified)
    );
}

/// The owned half of Spike B: a session Mother Claude launches itself (Path
/// B, the real CLI) loads the plugin inline (`--plugin-dir`) and names
/// itself with `X-MC-Session` (from `MOTHER_CLAUDE_SESSION`). The CLI's own
/// pidfile is in the real config dir, which this server does not read, so
/// only the header can tell the session here.
#[tokio::test]
#[ignore = "runs the real claude CLI as an owned session and spends tokens (haiku, $0.25 cap)"]
async fn live_owned_session_gets_the_tools_inline() {
    use mother_claude_lib::claude::SpawnOptions;
    use mother_claude_lib::mcp::plugin;

    let home = tempfile::tempdir().unwrap();
    let data = tempfile::Builder::new()
        .prefix("mc mcp owned ")
        .tempdir()
        .unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut auth = Auth::ephemeral();
    auth.config_dir = Some(data.path().to_path_buf());
    let st = Inner::new(
        ClaudeHome::with_base(home.path()),
        ServerConfig {
            host: "127.0.0.1".into(),
            port,
        },
        auth,
    );
    let app = server::router(st.clone());
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    plugin::apply(&st);
    assert!(
        plugin::owned_plugin_dir().is_some(),
        "the inline plugin is ready"
    );

    let tool = format!("{}mc_session", plugin::TOOL_PREFIX);
    let cwd_s = std::fs::canonicalize(cwd.path())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let id = st
        .control
        .spawn(
            &st,
            SpawnOptions {
                cwd: cwd_s,
                prompt: "Call the mc_session tool once and print exactly what it returns.".into(),
                model: Some("haiku".into()),
                permission_mode: Some("default".into()),
                setting_sources: Some("project,local".into()),
                extra_args: vec![
                    "--allowedTools".into(),
                    tool,
                    "--max-budget-usd".into(),
                    "0.25".into(),
                ],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let mut seen = None;
    for _ in 0..1500 {
        seen = st
            .mcp
            .recent_calls()
            .into_iter()
            .find(|c| c.tool == "mc_session");
        if seen.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = st.control.kill(&id).await;
    println!("server saw: {:?}", st.mcp.recent_calls());
    let call = seen.expect("the owned session called mc_session");
    assert!(call.ok, "{call:?}");
    assert_eq!(call.session.as_deref(), Some(id.as_str()));
    assert!(call.owned);
}
