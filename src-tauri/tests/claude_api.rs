//! The Claude API on an API server (docs/CLAUDE-API.md), offline: a fake Node
//! host (`tests/fixtures/fake-bridge-host.js`) answers bare conversations with
//! the API's own stream events. Requests go over HTTP exactly as a client of
//! the Claude API sends them; nothing runs Claude or binds 5612.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use mother_claude_lib::bridge::control::{self, BridgeDefaults};
use mother_claude_lib::bridge::{BridgeConfig, TokenPolicy};
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use reqwest::header::HeaderMap;
use reqwest::StatusCode;
use serde_json::{json, Value};

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

/// What the host was sent, `op` for `id`.
fn host_ops(op: &str, id: &str) -> Vec<Value> {
    std::fs::read_to_string(host_log())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|r| r["op"] == op && r["sessionId"] == id)
        .collect()
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
    /// The dashboard (main app port).
    app: String,
    _home: tempfile::TempDir,
    scratch: tempfile::TempDir,
}

async fn boot() -> Fixture {
    host_log();
    let home = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let mut auth = Auth::ephemeral();
    auth.config_dir = Some(scratch.path().join("app-data"));
    let st = Inner::with_bridge_config(
        ClaudeHome::with_base(home.path()),
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
    Fixture {
        st,
        app,
        _home: home,
        scratch,
    }
}

impl Fixture {
    async fn start(&self, defaults: BridgeDefaults) -> String {
        control::start(&self.st, defaults).await.unwrap();
        let port = self.st.bridge.main().bound_port().unwrap();
        format!("http://127.0.0.1:{port}")
    }

    fn desktop(&self) -> String {
        self.st.auth.token.clone()
    }
}

/// A Claude API call as an SDK makes it: `anthropic-version`, an
/// `x-api-key` (any; Mother Claude never uses it), more `headers`.
async fn messages(
    api: &str,
    body: Value,
    headers: &[(&str, &str)],
) -> (StatusCode, HeaderMap, Value) {
    let mut r = reqwest::Client::new()
        .post(format!("{api}/v1/messages"))
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json");
    if !headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("x-api-key"))
    {
        r = r.header("x-api-key", "not-a-real-anthropic-key");
    }
    for (k, v) in headers {
        r = r.header(*k, *v);
    }
    let resp = r.json(&body).send().await.unwrap();
    let status = resp.status();
    let h = resp.headers().clone();
    let text = resp.text().await.unwrap_or_default();
    (
        status,
        h,
        serde_json::from_str(&text).unwrap_or(Value::Null),
    )
}

fn conversation(h: &HeaderMap) -> String {
    h.get("x-conversation-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

fn text_of(message: &Value) -> String {
    message["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|b| b["text"].as_str())
        .collect()
}

async fn dashboard(
    f: &Fixture,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let c = reqwest::Client::new();
    let url = format!("{}{path}", f.app);
    let mut r = match method {
        "GET" => c.get(url),
        "DELETE" => c.delete(url),
        _ => c.post(url),
    }
    .bearer_auth(f.desktop());
    if let Some(b) = body {
        r = r.json(&b);
    }
    let resp = r.send().await.unwrap();
    let s = resp.status();
    let text = resp.text().await.unwrap_or_default();
    (s, serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// A whole exchange as a client of the Claude API has it: an answer, the
/// history sent back continuing the same conversation, a stream, a
/// conversation named with only its next message, and an edited history
/// branching off.
#[tokio::test]
async fn the_messages_api_answers_and_conversations_go_on() {
    let f = boot().await;
    let api = f.start(BridgeDefaults::default()).await;

    let first = json!({ "role": "user", "content": "Hello" });
    let (s, h, a1) = messages(
        &api,
        json!({ "model": "claude-sonnet-5-5", "max_tokens": 256, "messages": [first.clone()] }),
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{a1}");
    assert_eq!(a1["type"], "message");
    assert_eq!(a1["role"], "assistant");
    assert_eq!(a1["model"], "claude-sonnet-5-5");
    assert_eq!(text_of(&a1), "Echo (turn 1): Hello");
    assert_eq!(a1["stop_reason"], "end_turn");
    assert_eq!(a1["usage"]["input_tokens"], 10);
    assert_eq!(a1["usage"]["output_tokens"], 5);
    assert!(a1["id"].as_str().unwrap().starts_with("msg_"));
    assert!(h["request-id"].to_str().unwrap().starts_with("req_mc"));
    let id = conversation(&h);
    assert_eq!(id.len(), 36);
    // The model alone: the client's system prompt (none), no Claude Code
    // tools, max_tokens as the output cap.
    let created = &host_ops("session.create", &id)[0];
    assert_eq!(created["bare"], true);
    assert_eq!(created["systemPrompt"], "");
    assert_eq!(created["maxOutputTokens"], 256);
    // `thinking` left out: what the API does for the model (adaptive on the
    // Claude 5 models).
    assert_eq!(created["thinkingConfig"], json!({ "type": "adaptive" }));

    // The history sent back (as an SDK re-serialises it) goes on in it.
    let mut echoed = a1.clone();
    echoed["content"][0]["citations"] = Value::Null;
    let second = json!({ "role": "user", "content": [{ "type": "text", "text": "Again" }] });
    let (s, h, a2) = messages(
        &api,
        json!({
            "model": "claude-sonnet-5-5", "max_tokens": 256,
            "messages": [first.clone(), { "role": "assistant", "content": echoed["content"] }, second.clone()],
        }),
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{a2}");
    assert_eq!(conversation(&h), id);
    assert_eq!(text_of(&a2), "Echo (turn 2): Again");
    assert_eq!(host_ops("session.create", &id).len(), 1, "one session");

    // Streamed: the API's events, in order.
    let resp = reqwest::Client::new()
        .post(format!("{api}/v1/messages"))
        .header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": "claude-sonnet-5-5", "max_tokens": 256, "stream": true,
            "messages": [
                first.clone(), { "role": "assistant", "content": a1["content"] },
                second.clone(), { "role": "assistant", "content": a2["content"] },
                { "role": "user", "content": "Third" },
            ],
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(conversation(resp.headers()), id);
    assert!(resp.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/event-stream"));
    let body = resp.text().await.unwrap();
    let names: Vec<&str> = body
        .lines()
        .filter_map(|l| l.strip_prefix("event: "))
        .collect();
    assert_eq!(names.first(), Some(&"message_start"));
    assert_eq!(names.last(), Some(&"message_stop"));
    assert!(names.contains(&"content_block_delta"));
    let streamed: String = body
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .filter_map(|d| serde_json::from_str::<Value>(d).ok())
        .filter_map(|v| v["delta"]["text"].as_str().map(str::to_string))
        .collect();
    assert_eq!(streamed, "Echo (turn 3): Third");

    // Named, with only its next message: the conversation keeps the history.
    let (s, h, a4) = messages(
        &api,
        json!({ "model": "claude-sonnet-5-5", "max_tokens": 256,
                "messages": [{ "role": "user", "content": "Fourth" }] }),
        &[("x-conversation-id", id.as_str())],
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{a4}");
    assert_eq!(conversation(&h), id);
    assert_eq!(text_of(&a4), "Echo (turn 4): Fourth");

    // An edited history is a branch of its own, begun with that history.
    let (s, h, b1) = messages(
        &api,
        json!({
            "model": "claude-sonnet-5-5", "max_tokens": 256,
            "messages": [
                { "role": "user", "content": "Hello, edited" },
                { "role": "assistant", "content": "Hi" },
                { "role": "user", "content": "Branch" },
            ],
        }),
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b1}");
    assert_ne!(conversation(&h), id);
    assert_eq!(text_of(&b1), "Echo (turn 1, seeded): Branch");

    // Other settings for the same history: a fork, the transcript carried
    // over (here none was written, so the history is written in).
    let (s, h, _) = messages(
        &api,
        json!({
            "model": "claude-sonnet-5-5", "max_tokens": 512, "system": "Be brief.",
            "messages": [first.clone(), { "role": "assistant", "content": a1["content"] },
                         { "role": "user", "content": "Other settings" }],
        }),
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let forked = conversation(&h);
    assert_ne!(forked, id);
    assert_eq!(
        host_ops("session.create", &forked)[0]["systemPrompt"],
        "Be brief."
    );

    // Client-facing: the conversations, newest first; one with its messages.
    let list: Value = reqwest::Client::new()
        .get(format!("{api}/v1/conversations"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ids: Vec<&str> = list["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 3, "{list}");
    assert!(ids.contains(&id.as_str()));
    let one: Value = reqwest::Client::new()
        .get(format!("{api}/v1/conversations/{id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(one["type"], "conversation");
    assert_eq!(one["message_count"], 8);
    assert_eq!(one["messages"][7]["role"], "assistant");
}

/// The API's own errors: in its envelope, with its types and statuses.
#[tokio::test]
async fn errors_come_as_the_api_sends_them() {
    let f = boot().await;
    // Not started: the API's envelope, not the bridge's.
    control::start(&f.st, BridgeDefaults::default())
        .await
        .unwrap();
    let api = format!(
        "http://127.0.0.1:{}",
        f.st.bridge.main().bound_port().unwrap()
    );
    control::stop(&f.st).await.unwrap();
    let app_api = format!("{}/v1", f.app);
    let resp = reqwest::Client::new()
        .post(format!("{app_api}/messages"))
        .bearer_auth(f.desktop())
        .json(&json!({ "model": "m", "max_tokens": 1, "messages": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let v: Value = resp.json().await.unwrap();
    assert_eq!(v["type"], "error");
    assert_eq!(v["error"]["type"], "api_error");
    assert!(v["request_id"].as_str().unwrap().starts_with("req_"));

    let api = {
        let _ = api;
        f.start(BridgeDefaults::default()).await
    };
    let (s, _, v) = messages(
        &api,
        json!({ "max_tokens": 1, "messages": [{ "role": "user", "content": "x" }] }),
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(
        v["error"],
        json!({ "type": "invalid_request_error", "message": "model: Field required" })
    );

    // A prefill cannot be answered by Claude Code.
    let (s, _, v) = messages(
        &api,
        json!({ "model": "m", "max_tokens": 5, "messages": [
            { "role": "user", "content": "x" }, { "role": "assistant", "content": "{" },
        ]}),
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");

    // Not JSON.
    let resp = reqwest::Client::new()
        .post(format!("{api}/v1/messages"))
        .header("content-type", "application/json")
        .body("{nope")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let v: Value = resp.json().await.unwrap();
    assert_eq!(v["error"]["type"], "invalid_request_error");

    // A conversation named that is not there.
    let (s, _, v) = messages(
        &api,
        json!({ "model": "m", "max_tokens": 5, "messages": [{ "role": "user", "content": "x" }] }),
        &[("x-conversation-id", "00000000-0000-4000-8000-000000000000")],
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(v["error"]["type"], "not_found_error");

    // max_tokens 0 warms a cache: nothing generated, no conversation.
    let (s, h, v) = messages(
        &api,
        json!({ "model": "m", "max_tokens": 0, "messages": [{ "role": "user", "content": "x" }] }),
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["content"], json!([]));
    assert!(h.get("x-conversation-id").is_none());

    // Sign-in trouble is the API's authentication error.
    let (s, _, v) = messages(
        &api,
        json!({ "model": "m", "max_tokens": 5, "messages": [{ "role": "user", "content": "AUTHFAIL" }] }),
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "{v}");
    assert_eq!(v["error"]["type"], "authentication_error");

    // Fields Claude Code cannot honour are named, not refused.
    let (s, h, _) = messages(
        &api,
        json!({ "model": "m", "max_tokens": 5, "temperature": 0.3,
                "messages": [{ "role": "user", "content": "x" }] }),
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["x-mother-claude-ignored"], "temperature");
}

/// A client's tool: the model's call ends the answer with `tool_use`; the
/// next request's `tool_result` goes to the turn that waits for it.
#[tokio::test]
async fn client_tools_go_to_the_client_and_back() {
    let f = boot().await;
    let api = f.start(BridgeDefaults::default()).await;
    let tools = json!([{
        "name": "get_weather",
        "description": "The weather in a city",
        "input_schema": { "type": "object", "properties": { "city": { "type": "string" } } },
    }]);
    let ask = json!({ "role": "user", "content": "TOOL: what is the weather?" });
    let (s, h, a1) = messages(
        &api,
        json!({ "model": "m", "max_tokens": 100, "tools": tools, "messages": [ask.clone()] }),
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{a1}");
    assert_eq!(a1["stop_reason"], "tool_use");
    let call = &a1["content"][0];
    assert_eq!(call["type"], "tool_use");
    assert_eq!(call["name"], "get_weather", "the client's name, no prefix");
    assert_eq!(call["input"], json!({ "city": "Paris" }));
    let id = conversation(&h);
    assert_eq!(
        host_ops("session.create", &id)[0]["clientTools"],
        json!(["get_weather"])
    );

    // Without the result: the API's own complaint.
    let (s, _, v) = messages(
        &api,
        json!({ "model": "m", "max_tokens": 100, "tools": tools, "messages": [
            ask.clone(), { "role": "assistant", "content": a1["content"] },
            { "role": "user", "content": "never mind" },
        ]}),
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("tool_result"));

    let (s, h, a2) = messages(
        &api,
        json!({ "model": "m", "max_tokens": 100, "tools": tools, "messages": [
            ask, { "role": "assistant", "content": a1["content"] },
            { "role": "user", "content": [{
                "type": "tool_result", "tool_use_id": call["id"], "content": "sunny, 21°C",
            }]},
        ]}),
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{a2}");
    assert_eq!(conversation(&h), id);
    assert_eq!(text_of(&a2), "It is known in Paris: sunny, 21°C");
    assert_eq!(a2["stop_reason"], "end_turn");
}

/// `output_config.format`: Claude Code answers through its StructuredOutput
/// tool; the client gets the API's answer, a text block of the JSON
/// (`end_turn`) — whole or streamed, words before it left out, an output
/// the schema refused tried again within the same message.
#[tokio::test]
async fn structured_outputs_answer_as_the_api_does() {
    let f = boot().await;
    let api = f.start(BridgeDefaults::default()).await;
    let format = json!({ "format": { "type": "json_schema", "schema": {
        "type": "object", "properties": { "reply": { "type": "string" } },
        "required": ["reply"], "additionalProperties": false,
    }}});
    let (s, h, v) = messages(
        &api,
        json!({ "model": "claude-opus-5-5", "max_tokens": 1000, "output_config": format,
                "messages": [{ "role": "user", "content": "PREAMBLE Hello" }] }),
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["stop_reason"], "end_turn", "{v}");
    let content = v["content"].as_array().unwrap();
    assert_eq!(content.len(), 1, "{v}");
    assert_eq!(content[0]["type"], "text");
    let answer: Value = serde_json::from_str(content[0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(answer, json!({ "reply": "Echo (turn 1): PREAMBLE Hello" }));
    let created = &host_ops("session.create", &conversation(&h))[0];
    assert_eq!(created["outputFormat"]["type"], "json_schema");

    // Streamed, with a first output the schema refused: one message.
    let resp = reqwest::Client::new()
        .post(format!("{api}/v1/messages"))
        .header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": "claude-opus-5-5", "max_tokens": 1000, "stream": true, "output_config": format,
            "messages": [{ "role": "user", "content": "SORETRY Bonjour" }],
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.text().await.unwrap();
    let names: Vec<&str> = body
        .lines()
        .filter_map(|l| l.strip_prefix("event: "))
        .filter(|n| *n != "ping")
        .collect();
    assert_eq!(
        names.iter().filter(|n| **n == "message_start").count(),
        1,
        "{body}"
    );
    assert_eq!(names.last(), Some(&"message_stop"), "{body}");
    let data: Vec<Value> = body
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .filter_map(|d| serde_json::from_str(d).ok())
        .collect();
    assert!(
        !data
            .iter()
            .any(|d| d["content_block"]["type"] == "tool_use"),
        "{body}"
    );
    let text: String = data
        .iter()
        .filter_map(|d| d.pointer("/delta/text").and_then(Value::as_str))
        .collect();
    let answer: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(answer, json!({ "reply": "Echo (turn 1): SORETRY Bonjour" }));
    assert!(data
        .iter()
        .any(|d| d["type"] == "message_delta" && d["delta"]["stop_reason"] == "end_turn"));
}

/// A stop sequence cuts the answer where it begins, and the turn that went
/// on is interrupted.
#[tokio::test]
async fn stop_sequences_cut_the_answer() {
    let f = boot().await;
    let api = f.start(BridgeDefaults::default()).await;
    let (s, h, v) = messages(
        &api,
        json!({ "model": "m", "max_tokens": 100, "stop_sequences": ["STOP"],
                "messages": [{ "role": "user", "content": "STOPTEST" }] }),
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(text_of(&v), "alpha ");
    assert_eq!(v["stop_reason"], "stop_sequence");
    assert_eq!(v["stop_sequence"], "STOP");
    let id = conversation(&h);
    for _ in 0..100 {
        if !host_ops("session.interrupt", &id).is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the turn was not interrupted");
}

/// The key a server is started with: every request carries it, as the
/// Claude API takes one (`x-api-key` or a bearer token), from this computer
/// too; kept in the secret store (never in a file), so it holds after a
/// relaunch; blank at the next start, no authentication.
#[tokio::test]
async fn a_server_takes_the_key_it_is_started_with() {
    let f = boot().await;
    let (s, v) = dashboard(
        &f,
        "POST",
        "/api/bridge/start",
        Some(json!({ "defaults": { "groupClientsBy": "key" }, "key": "service-key-0123" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["keyHint"], "…0123");
    let api = format!(
        "http://127.0.0.1:{}",
        f.st.bridge.main().bound_port().unwrap()
    );
    let body =
        json!({ "model": "m", "max_tokens": 5, "messages": [{ "role": "user", "content": "hi" }] });
    let (s, _, v) = messages(&api, body.clone(), &[("x-api-key", "")]).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "{v}");
    assert_eq!(v["error"]["type"], "authentication_error");
    assert_eq!(v["error"]["code"], "api_key_required");
    let (s, _, v) = messages(&api, body.clone(), &[("x-api-key", "wrong-key")]).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(v["error"]["code"], "invalid_api_key");
    let (s, _, _) = messages(&api, body.clone(), &[("x-api-key", "service-key-0123")]).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _, _) = messages(
        &api,
        body.clone(),
        &[
            ("x-api-key", ""),
            ("authorization", "Bearer service-key-0123"),
        ],
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    // The bridge's own routes too.
    let resp = reqwest::get(format!("{api}/threads")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    // The key is the client: its folder by the key's hash.
    let folders: Vec<String> =
        std::fs::read_dir(f.scratch.path().join("app-data").join("claude-api"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
    assert!(folders.iter().any(|n| n.starts_with("key-")), "{folders:?}");

    // Never in a file of ours; its hint is.
    let data = f.scratch.path().join("app-data");
    let servers = std::fs::read_to_string(data.join("api-servers.json")).unwrap();
    assert!(!servers.contains("service-key-0123"));
    assert!(servers.contains("…0123"));

    // A relaunch on the same data: the key holds.
    let next = relaunch(&f);
    control::start(&next, BridgeDefaults::default())
        .await
        .unwrap();
    let next_api = format!(
        "http://127.0.0.1:{}",
        next.bridge.main().bound_port().unwrap()
    );
    let (s, _, _) = messages(&next_api, body.clone(), &[("x-api-key", "")]).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _, _) = messages(
        &next_api,
        body.clone(),
        &[("x-api-key", "service-key-0123")],
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    control::stop(&next).await.unwrap();

    // Blank: no authentication. (A test server restarts on a new port.)
    let (s, v) = dashboard(&f, "POST", "/api/bridge/start", Some(json!({ "key": "" }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v.get("keyHint").is_none());
    let api = format!(
        "http://127.0.0.1:{}",
        f.st.bridge.main().bound_port().unwrap()
    );
    let (s, _, _) = messages(&api, body, &[("x-api-key", "")]).await;
    assert_eq!(s, StatusCode::OK);
    // A key with a space is no key.
    let (s, v) = dashboard(
        &f,
        "POST",
        "/api/bridge/start",
        Some(json!({ "key": "two words" })),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"]["field"], "key");
}

/// The app again on the same data (a relaunch).
fn relaunch(f: &Fixture) -> AppState {
    let mut auth = Auth::ephemeral();
    auth.config_dir = Some(f.scratch.path().join("app-data"));
    Inner::with_bridge_config(
        ClaudeHome::with_base(f._home.path()),
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
    )
}

/// Clients told apart by a header, each in its folder under the folder
/// chosen at the start; the dashboard lists them and deletes a
/// conversation, several, or all of a client's data.
#[tokio::test]
async fn clients_are_kept_apart_and_can_be_deleted() {
    let f = boot().await;
    let root = f.scratch.path().join("claude-api");
    let api = f
        .start(BridgeDefaults {
            api_data_dir: Some(root.to_string_lossy().into_owned()),
            group_clients_by: Some("header".into()),
            ..Default::default()
        })
        .await;
    let ask = |text: &str| json!({ "model": "m", "max_tokens": 5, "messages": [{ "role": "user", "content": text }] });
    let (_, h, _) = messages(&api, ask("one"), &[("x-client-id", "Alpha")]).await;
    let alpha1 = conversation(&h);
    let (_, h, _) = messages(&api, ask("two"), &[("x-client-id", "Alpha")]).await;
    let alpha2 = conversation(&h);
    let (_, h, _) = messages(&api, ask("three"), &[("x-client-id", "Beta")]).await;
    let beta = conversation(&h);
    let alpha_dir = root.join("alpha");
    assert!(alpha_dir.join("client.json").is_file());
    assert!(alpha_dir
        .join("conversations")
        .join(format!("{alpha1}.json"))
        .is_file());

    // A client sees its own conversations only.
    let list = |client: &'static str| {
        let api = api.clone();
        async move {
            let v: Value = reqwest::Client::new()
                .get(format!("{api}/v1/conversations"))
                .header("x-client-id", client)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            v["data"].as_array().unwrap().len()
        }
    };
    assert_eq!(list("Alpha").await, 2);
    assert_eq!(list("Beta").await, 1);
    let resp = reqwest::Client::new()
        .get(format!("{api}/v1/conversations/{beta}"))
        .header("x-client-id", "Alpha")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let (s, v) = dashboard(&f, "GET", "/api/bridge/clients", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let ours = v["roots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["path"] == root.to_string_lossy().as_ref())
        .expect("the chosen folder is listed");
    let clients: Vec<&str> = ours["clients"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(clients.len(), 2);
    assert!(clients.contains(&"alpha") && clients.contains(&"beta"));
    let q = format!("?root={}", urlencode(&root.to_string_lossy()));
    let (s, v) = dashboard(&f, "GET", &format!("/api/bridge/clients/alpha{q}"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["conversations"].as_array().unwrap().len(), 2);
    let (s, v) = dashboard(
        &f,
        "GET",
        &format!("/api/bridge/api-conversations/{alpha1}"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["messageList"].as_array().unwrap().len(), 2);

    // One, then the client's whole data.
    let (s, v) = dashboard(
        &f,
        "POST",
        "/api/bridge/api-conversations/delete",
        Some(json!({ "ids": [alpha1] })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["deleted"], json!([alpha1]));
    assert!(!alpha_dir
        .join("conversations")
        .join(format!("{alpha1}.json"))
        .exists());
    assert_eq!(list("Alpha").await, 1);
    let (s, v) = dashboard(&f, "DELETE", &format!("/api/bridge/clients/alpha{q}"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["deleted"], json!([alpha2]));
    assert!(!alpha_dir.exists());
    assert_eq!(list("Beta").await, 1);

    // A client forgets its own.
    let resp = reqwest::Client::new()
        .delete(format!("{api}/v1/conversations/{beta}"))
        .header("x-client-id", "Beta")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(list("Beta").await, 0);
}

/// An application chooses its own folders with `X-Client-Id`
/// (`app/device/day`), and `x-conversation-id: latest` keeps one
/// conversation going in each: the day's for a device (another model goes on
/// in it), another for other settings, a new one on the next day. The
/// dashboard lists such clients by their path, and deleting one leaves no
/// empty folders behind.
#[tokio::test]
async fn an_application_chooses_its_folders_and_goes_on_in_the_latest() {
    let f = boot().await;
    let root = f.scratch.path().join("mother-claude-api");
    let api = f
        .start(BridgeDefaults {
            api_data_dir: Some(root.to_string_lossy().into_owned()),
            group_clients_by: Some("header".into()),
            ..Default::default()
        })
        .await;
    let ask = |model: &str, text: &str| json!({ "model": model, "max_tokens": 50, "messages": [{ "role": "user", "content": text }] });
    let ask_longer = |text: &str| json!({ "model": "m", "max_tokens": 900, "messages": [{ "role": "user", "content": text }] });
    let today = "xiangqi-solver/Device-1/2026-11-18";
    let latest = |client: &'static str| [("x-client-id", client), ("x-conversation-id", "latest")];

    // The first request of the day: none yet, a new conversation.
    let (s, h, a) = messages(&api, ask("m", "board one"), &latest(today)).await;
    assert_eq!(s, StatusCode::OK, "{a}");
    let first = conversation(&h);
    let day = root
        .join("xiangqi-solver")
        .join("device-1")
        .join("2026-11-18");
    assert!(day.join("client.json").is_file());
    assert!(day
        .join("conversations")
        .join(format!("{first}.json"))
        .is_file());
    // Its conversations run in that folder.
    let created = &host_ops("session.create", &first)[0];
    assert_eq!(
        created["cwd"].as_str().map(PathBuf::from),
        Some(day.clone())
    );

    // The next one goes on in it, with only its own message.
    let (s, h, a) = messages(&api, ask("m", "board two"), &latest(today)).await;
    assert_eq!(s, StatusCode::OK, "{a}");
    assert_eq!(conversation(&h), first);
    assert_eq!(text_of(&a), "Echo (turn 2): board two");

    // A one-off in the same folder (sent without x-conversation-id, as a
    // client does after a 409) is a conversation of its own, and `latest`
    // leaves it be.
    let (s, h, a) = messages(
        &api,
        ask("m", "a second opinion"),
        &[("x-client-id", today)],
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{a}");
    let one_off = conversation(&h);
    assert_ne!(one_off, first);
    let (_, h, a) = messages(&api, ask("m", "board two and a half"), &latest(today)).await;
    assert_eq!(conversation(&h), first, "not the one-off");
    assert_eq!(text_of(&a), "Echo (turn 3): board two and a half");

    // Another model (a re-read on a stronger one) goes on in the same
    // conversation: the model is switched for that turn.
    let (_, h, a) = messages(&api, ask("strong", "board two again"), &latest(today)).await;
    assert_eq!(conversation(&h), first);
    assert_eq!(text_of(&a), "Echo (turn 4): board two again");
    assert_eq!(a["model"], "strong");
    assert_eq!(host_ops("session.setModel", &first).len(), 1);

    // Other settings (here max_tokens): one of their own, and `latest`
    // keeps each.
    let (_, h, _) = messages(&api, ask_longer("board three"), &latest(today)).await;
    let longer = conversation(&h);
    assert_ne!(longer, first);
    let (_, h, a) = messages(&api, ask("m", "board four"), &latest(today)).await;
    assert_eq!(conversation(&h), first);
    assert_eq!(text_of(&a), "Echo (turn 5): board four");
    let (_, h, _) = messages(&api, ask_longer("board four again"), &latest(today)).await;
    assert_eq!(conversation(&h), longer);

    // The next day: another folder, a new conversation.
    let tomorrow = "xiangqi-solver/Device-1/2026-11-19";
    let (_, h, a) = messages(&api, ask("m", "board five"), &latest(tomorrow)).await;
    assert_ne!(conversation(&h), first);
    assert_eq!(text_of(&a), "Echo (turn 1): board five");

    // `latest` is no other word: anything else that is not an id is refused.
    let (s, _, e) = messages(
        &api,
        ask("m", "x"),
        &[("x-client-id", today), ("x-conversation-id", "newest")],
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{e}");

    // The dashboard lists the days by their path, and reads one.
    let (s, v) = dashboard(&f, "GET", "/api/bridge/clients", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let ours = v["roots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["path"] == root.to_string_lossy().as_ref())
        .expect("the chosen folder is listed")
        .clone();
    let mut ids: Vec<&str> = ours["clients"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    ids.sort();
    assert_eq!(
        ids,
        [
            "xiangqi-solver/device-1/2026-11-18",
            "xiangqi-solver/device-1/2026-11-19",
        ]
    );
    let q = format!("?root={}", urlencode(&root.to_string_lossy()));
    let id = urlencode("xiangqi-solver/device-1/2026-11-18");
    let (s, v) = dashboard(&f, "GET", &format!("/api/bridge/clients/{id}{q}"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["conversations"].as_array().unwrap().len(), 3);
    assert_eq!(v["client"]["label"], today);

    // Deleting both days leaves the clients' folder as it was.
    for day in ["2026-11-18", "2026-11-19"] {
        let id = urlencode(&format!("xiangqi-solver/device-1/{day}"));
        let (s, v) = dashboard(&f, "DELETE", &format!("/api/bridge/clients/{id}{q}"), None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    assert!(
        !root.join("xiangqi-solver").exists(),
        "no empty folders left"
    );
    assert!(root.is_dir());
}

/// `GET /v1/models` in the API's shape for a client of the API (the
/// bridge's own shape otherwise), and an estimated `count_tokens`.
#[tokio::test]
async fn models_and_token_counts() {
    let f = boot().await;
    let api = f.start(BridgeDefaults::default()).await;
    let v: Value = reqwest::Client::new()
        .get(format!("{api}/v1/models"))
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(v["has_more"].is_boolean(), "{v}");
    for m in v["data"].as_array().unwrap() {
        assert_eq!(m["type"], "model");
        assert!(m["id"].is_string() && m["display_name"].is_string());
    }
    let bridge: Value = reqwest::get(format!("{api}/v1/models"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(bridge.get("has_more").is_none());
    assert!(bridge["source"].is_string());

    let resp = reqwest::Client::new()
        .post(format!("{api}/v1/messages/count_tokens"))
        .header("anthropic-version", "2023-06-01")
        .json(&json!({ "model": "m", "messages": [{ "role": "user", "content": "Hello there, Claude" }] }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()["x-mother-claude-estimate"], "true");
    let v: Value = resp.json().await.unwrap();
    assert!(v["input_tokens"].as_u64().unwrap() > 0);
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[allow(dead_code)]
fn _unused(_: &Path) {}

/// "Start when Mother Claude opens": the desktop turns it on (a script may
/// only turn it off); the main server's defaults are saved with it, and the
/// next launch on the same data starts it with them — its key too.
#[tokio::test]
async fn a_server_can_start_when_mother_claude_opens() {
    let f = boot().await;
    let data = f.scratch.path().join("app-data");

    let resp = reqwest::Client::new()
        .post(format!("{}/api/bridge/autostart", f.app))
        .bearer_auth(&f.st.auth.script_token)
        .json(&json!({ "on": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let (s, v) = dashboard(
        &f,
        "POST",
        "/api/bridge/autostart",
        Some(json!({ "on": true })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["autostart"], true);
    let (s, v) = dashboard(
        &f,
        "POST",
        "/api/bridge/start",
        Some(json!({ "defaults": { "groupClientsBy": "ip" }, "key": "launch-key-42" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let file: Value =
        serde_json::from_str(&std::fs::read_to_string(data.join("api-servers.json")).unwrap())
            .unwrap();
    assert_eq!(file["main"]["autostart"], true);
    assert_eq!(file["main"]["defaults"]["groupClientsBy"], "ip");
    assert_eq!(file["main"]["keyHint"], "…y-42");

    // The next launch.
    let next = relaunch(&f);
    assert!(!next.bridge.main().is_running());
    mother_claude_lib::bridge::autostart_saved(&next).await;
    assert!(next.bridge.main().is_running());
    let d = next.bridge.main().defaults();
    assert_eq!(d.group_clients_by.as_deref(), Some("ip"));
    let api = format!(
        "http://127.0.0.1:{}",
        next.bridge.main().bound_port().unwrap()
    );
    let ask =
        json!({ "model": "m", "max_tokens": 5, "messages": [{ "role": "user", "content": "hi" }] });
    let (s, _, _) = messages(&api, ask.clone(), &[("x-api-key", "")]).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _, _) = messages(&api, ask, &[("x-api-key", "launch-key-42")]).await;
    assert_eq!(s, StatusCode::OK);
    control::stop(&next).await.unwrap();

    // Off: nothing of the main server is written but its key's hint.
    let resp = reqwest::Client::new()
        .post(format!("{}/api/bridge/autostart", f.app))
        .bearer_auth(&f.st.auth.script_token)
        .json(&json!({ "on": false }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let file: Value =
        serde_json::from_str(&std::fs::read_to_string(data.join("api-servers.json")).unwrap())
            .unwrap();
    assert!(file["main"].get("autostart").is_none(), "{file}");
    assert!(file["main"].get("defaults").is_none(), "{file}");
    assert_eq!(file["main"]["keyHint"], "…y-42");
}

/// A free loopback port (bound, then let go).
async fn free_port() -> u16 {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    l.local_addr().unwrap().port()
}

/// Beyond this computer (`access`): the desktop opens a server to the local
/// network or anywhere. Its listeners there serve the server's routes over
/// plain HTTP, read no Mother Claude token, and without a key answer the
/// Claude API alone; with the key, everything. Through a tunnel (a proxied
/// request to the loopback port) the same holds once it is reachable from
/// anywhere. The main server's port can be chosen.
#[tokio::test]
async fn a_server_can_be_reached_beyond_this_computer() {
    let f = boot().await;

    // Only the desktop opens it.
    let resp = reqwest::Client::new()
        .post(format!("{}/api/bridge/start", f.app))
        .bearer_auth(&f.st.auth.script_token)
        .json(&json!({ "defaults": { "access": "all" } }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let port = free_port().await;
    let (s, v) = dashboard(
        &f,
        "POST",
        "/api/bridge/start",
        Some(json!({ "defaults": { "access": "all" }, "port": port })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["networkUrls"].is_array(), "{v}");
    assert_eq!(v["defaults"]["access"], "all");
    assert_eq!(v["port"], port, "the chosen port");
    let api = format!("http://127.0.0.1:{port}");
    let file: Value = serde_json::from_str(
        &std::fs::read_to_string(f.scratch.path().join("app-data").join("api-servers.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(file["main"]["port"], port);
    // A device keeps it open on a restart.
    let resp = reqwest::Client::new()
        .post(format!("{}/api/bridge/start", f.app))
        .bearer_auth(&f.st.auth.script_token)
        .json(&json!({ "defaults": { "access": "all" } }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // A listener beyond this computer, served here on a loopback port: the
    // ingress is what counts.
    let net_port = free_port().await;
    let net = mother_claude_lib::bridge::claude_api::network::start(
        f.st.clone(),
        "main".into(),
        net_port,
        |_| vec!["127.0.0.1".parse().unwrap()],
    );
    let base = format!("http://127.0.0.1:{net_port}");
    let mut up = false;
    for _ in 0..100 {
        if net.urls() == vec![base.clone()] {
            up = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(up, "the listener did not come up");

    let ask =
        json!({ "model": "m", "max_tokens": 5, "messages": [{ "role": "user", "content": "hi" }] });
    // No key: the Claude API answers…
    let resp = reqwest::Client::new()
        .post(format!("{base}/v1/messages"))
        .json(&ask)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let resp = reqwest::get(format!("{base}/v1/models")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    // …the rest does not, and a Mother Claude token opens nothing there.
    for path in ["/v1/threads", "/threads", "/v1/requests"] {
        let resp = reqwest::Client::new()
            .get(format!("{base}{path}"))
            .bearer_auth(f.desktop())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{path}");
    }
    // A client's conversations: only with the key.
    let resp = reqwest::get(format!("{base}/v1/conversations"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Through a tunnel: a proxied request to the loopback port.
    let proxied = |path: &str| {
        reqwest::Client::new()
            .post(format!("{api}{path}"))
            .header("x-forwarded-for", "203.0.113.9")
            .json(&ask)
    };
    assert_eq!(
        proxied("/v1/messages").send().await.unwrap().status(),
        StatusCode::OK
    );
    let resp = proxied("/threads").send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // With a key: everything, from anywhere — with it only.
    let (s, _) = dashboard(
        &f,
        "POST",
        "/api/bridge/start",
        Some(json!({ "defaults": { "access": "all" }, "key": "net-key-9876" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let resp = reqwest::Client::new()
        .post(format!("{base}/v1/messages"))
        .json(&ask)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let resp = reqwest::Client::new()
        .get(format!("{base}/v1/threads"))
        .header("x-api-key", "net-key-9876")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let resp = reqwest::Client::new()
        .get(format!("{base}/v1/conversations"))
        .header("x-api-key", "net-key-9876")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    // Only the desktop takes the key off a server that is open.
    let resp = reqwest::Client::new()
        .post(format!("{}/api/bridge/start", f.app))
        .bearer_auth(&f.st.auth.script_token)
        .json(&json!({ "defaults": { "access": "all" }, "key": "" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Kept to this computer: a tunnel gets nothing without a key.
    let (s, _) = dashboard(&f, "POST", "/api/bridge/start", Some(json!({ "key": "" }))).await;
    assert_eq!(s, StatusCode::OK);
    let resp = proxied("/v1/messages").send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let v: Value = resp.json().await.unwrap();
    assert_eq!(v["error"]["code"], "tunneled_request_requires_token");

    // Stopped: the port closes.
    net.stop();
    let mut closed = false;
    for _ in 0..100 {
        if reqwest::get(format!("{base}/v1/models")).await.is_err() {
            closed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(closed, "the listener did not close");
    assert!(net.urls().is_empty());

    // The chosen port holds after a relaunch.
    let next = relaunch(&f);
    assert_eq!(next.bridge.main().port(), Some(port));
}
