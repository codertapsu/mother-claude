//! Mother Claude's MCP server over real HTTP (docs/MCP.md): who may call
//! `/mcp` and from where, the JSON-RPC surface and the calling session's
//! identity. Nothing here touches `~/.claude` or spawns `claude`.

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
