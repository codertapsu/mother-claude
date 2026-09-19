//! Integration tests for the Claude HTTP bridge.
//!
//! Everything here runs the real axum router over real HTTP. The tests split in
//! two:
//!
//! * the default set never starts the Node host or calls a model, so it is safe
//!   for CI — it covers auth, the error envelope, static assets, the OpenAPI
//!   document and cursor handling;
//! * the `#[ignore]`d `live_*` tests drive an actual Claude conversation and
//!   cost tokens. Run them deliberately:
//!
//!   ```bash
//!   npm run sidecar:build
//!   cargo test --manifest-path src-tauri/Cargo.toml --test bridge -- --ignored --nocapture
//!   ```

use std::net::SocketAddr;

use mother_claude_lib::bridge::control::BridgeDefaults;
use mother_claude_lib::bridge::BridgeConfig;
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use serde_json::{json, Value};

/// Boot against an isolated Claude home. The offline tests use this: nothing in
/// CI may read — or write to — the developer's real `~/.claude`.
async fn boot() -> (String, String, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    // Leak the tempdir: the server outlives the test body and reads from it.
    let base = dir.keep();
    boot_with_home(ClaudeHome::with_base(&base)).await
}

/// The real Claude home, bridge started, no dedicated port.
async fn boot_live_started() -> (String, String, AppState) {
    let (base, token, state) = boot_stopped(
        ClaudeHome::resolve().expect("a resolvable Claude home"),
        false,
    )
    .await;
    mother_claude_lib::bridge::control::start(&state, BridgeDefaults::default())
        .await
        .expect("the bridge should start");
    (base, token, state)
}

/// Boot against the real Claude home. Live tests need it — the runtime reads
/// the user's own sign-in from there, and an isolated home answers every turn
/// with "Not logged in".
async fn boot_live() -> (String, String, AppState) {
    boot_with_home(ClaudeHome::resolve().expect("a resolvable Claude home")).await
}

/// Boot the bridge the way the dedicated loopback port serves it: no token.
async fn boot_tokenless() -> (String, String, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.keep();
    boot_router(ClaudeHome::with_base(&base), false).await
}

/// Boot the bridge router on an ephemeral loopback port.
async fn boot_with_home(home: ClaudeHome) -> (String, String, AppState) {
    boot_router(home, true).await
}

async fn boot_router(home: ClaudeHome, require_token: bool) -> (String, String, AppState) {
    let (base, token, state) = boot_stopped(home, require_token).await;
    // Most tests exercise the running bridge; `boot_stopped` covers the gate.
    mother_claude_lib::bridge::control::start(&state, BridgeDefaults::default())
        .await
        .expect("the bridge should start");
    (base, token, state)
}

/// Boot the router with the bridge *not* started, which is how the app opens.
async fn boot_stopped(home: ClaudeHome, require_token: bool) -> (String, String, AppState) {
    let state = Inner::with_bridge_config(
        home,
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        Auth::ephemeral(),
        // No dedicated port: these tests drive the router directly, and binding
        // 5612 would collide with a real Mother Claude and with each other.
        BridgeConfig {
            enabled: true,
            port: None,
            require_token,
            api_key: None,
            allowed_origins: Vec::new(),
        },
    );
    let token = state.auth.token.clone();

    let app = axum::Router::new()
        .merge(mother_claude_lib::bridge::router(
            state.clone(),
            require_token,
        ))
        .with_state(state.clone());

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

    (format!("http://{addr}"), token, state)
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

#[tokio::test]
async fn api_routes_require_the_token_but_assets_do_not() {
    let (base, token, _state) = boot().await;
    let http = client();

    // No token -> 401, and not a body the client has to guess at.
    let resp = http
        .get(format!("{base}/capabilities"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // With the token -> the real contract.
    let caps: Value = http
        .get(format!("{base}/capabilities"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(caps["limits"]["max_active_operations"], 16);
    assert_eq!(caps["auth"]["require_token"], true);
    assert!(caps["backends"]["agent_sdk"]["features"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f == "tool_approval"));

    // The token also travels in the query string, which is how EventSource and
    // the phone pairing link authenticate.
    let resp = http
        .get(format!("{base}/capabilities?token={token}"))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());

    // Documentation and the client module are public: they carry no secret and
    // a browser must be able to load them before it has a token.
    for path in [
        "/openapi.json",
        "/docs",
        "/docs/swagger-ui.css",
        "/docs/VENDOR.json",
        "/client.mjs",
        "/example",
        "/example/app.js",
    ] {
        let resp = http.get(format!("{base}{path}")).send().await.unwrap();
        assert!(
            resp.status().is_success(),
            "{path} should be public, got {}",
            resp.status()
        );
    }
}

/// The bridge is mounted twice; the mount that matters most is the one on the
/// main server, because that listener is reachable from the LAN. Nesting it
/// outside the main auth layer (so it can carry its own policy) is exactly the
/// kind of wiring that silently stops authenticating, so prove it here.
#[tokio::test]
async fn the_v1_mount_on_the_main_server_is_reachable_and_authenticated() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base_dir = dir.keep();
    let state = Inner::with_bridge_config(
        ClaudeHome::with_base(&base_dir),
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        Auth::ephemeral(),
        BridgeConfig {
            enabled: true,
            port: None,
            require_token: true,
            api_key: None,
            allowed_origins: Vec::new(),
        },
    );
    let token = state.auth.token.clone();
    mother_claude_lib::bridge::control::start(&state, BridgeDefaults::default())
        .await
        .expect("the bridge should start");

    // The whole application router, not just the bridge.
    let app = mother_claude_lib::server::router(state);
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
    let base = format!("http://{addr}");
    let http = client();

    // Bridge API on /v1: token required.
    let resp = http
        .get(format!("{base}/v1/capabilities"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "the /v1 mount must not be reachable without the token"
    );
    let caps: Value = http
        .get(format!("{base}/v1/capabilities"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(caps["auth"]["require_token"], true);

    // Bridge docs on /v1: public, and the SPA fallback must not swallow them.
    for path in ["/v1/docs", "/v1/openapi.json", "/v1/client.mjs"] {
        let resp = http.get(format!("{base}{path}")).send().await.unwrap();
        assert!(
            resp.status().is_success(),
            "{path} should be public, got {}",
            resp.status()
        );
    }

    // The document served from /v1 names /v1 as its server; a guessed "/" would
    // send Swagger's "Try it out" into the dashboard's SPA fallback.
    let doc: Value = http
        .get(format!("{base}/v1/openapi.json"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(doc["servers"][0]["url"], "/v1");
    assert_eq!(doc["x-bridge-require-token"], true);

    // The existing dashboard API is untouched.
    let resp = http
        .get(format!("{base}/api/sessions"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    let sessions: Value = http
        .get(format!("{base}/api/sessions"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(sessions.is_array());
}

#[tokio::test]
async fn the_openapi_document_describes_this_listener() {
    let (base, _token, _state) = boot().await;
    let doc: Value = client()
        .get(format!("{base}/openapi.json"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(doc["openapi"], "3.1.0");
    assert_eq!(doc["x-bridge-require-token"], true);
    assert_eq!(doc["security"][0]["BridgeBearer"], json!([]));
    assert!(doc["paths"]["/threads"]["post"].is_object());
    assert!(doc["paths"]["/chat"]["post"].is_object());
    assert!(doc["paths"]["/operations/{operation_id}/events"]["get"].is_object());
}

#[tokio::test]
async fn every_failure_uses_one_error_envelope() {
    let (base, token, _state) = boot().await;
    let http = client();

    // Unknown operation -> 404 operation_not_found.
    let resp = http
        .get(format!("{base}/operations/nope"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["code"], "operation_not_found");
    assert_eq!(body["error"]["operation_id"], "nope");

    // Unknown field -> 422 with the serde message, not a bare 400.
    let resp = http
        .post(format!("{base}/threads"))
        .bearer_auth(&token)
        .json(&json!({ "modl": "haiku" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        resp.json::<Value>().await.unwrap()["error"]["code"],
        "unknown_field"
    );

    // Malformed JSON -> 400 invalid_json.
    let resp = http
        .post(format!("{base}/threads"))
        .bearer_auth(&token)
        .header("content-type", "application/json")
        .body("{not json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(
        resp.json::<Value>().await.unwrap()["error"]["code"],
        "invalid_json"
    );

    // Wrong content type -> 415.
    let resp = http
        .post(format!("{base}/threads"))
        .bearer_auth(&token)
        .header("content-type", "text/plain")
        .body("hello")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNSUPPORTED_MEDIA_TYPE);

    // An invalid enum names the field that was wrong.
    let resp = http
        .post(format!("{base}/threads"))
        .bearer_auth(&token)
        .json(&json!({ "effort": "turbo" }))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["code"], "invalid_field");
    assert_eq!(body["error"]["field"], "effort");

    // A missing conversation is a typed 404, not a 500.
    let resp = http
        .post(format!("{base}/chat"))
        .bearer_auth(&token)
        .json(&json!({ "message": "hi", "thread_id": "does-not-exist" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
    assert_eq!(
        resp.json::<Value>().await.unwrap()["error"]["code"],
        "thread_not_found"
    );
}

/// Opening Mother Claude must not publish an API that can drive Claude. The
/// routes that spend tokens stay closed until someone presses Start; the ones
/// that answer "is it running?" stay open, or you could not tell "not started"
/// from "not there".
#[tokio::test]
async fn nothing_drives_claude_until_the_bridge_is_started() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base_dir = dir.keep();
    let (base, _token, state) = boot_stopped(ClaudeHome::with_base(&base_dir), false).await;
    let http = client();

    for (method, path) in [
        ("POST", "/chat"),
        ("POST", "/threads"),
        ("GET", "/threads"),
        ("GET", "/requests"),
        ("GET", "/events"),
        ("POST", "/messages"),
    ] {
        let request = match method {
            "POST" => http.post(format!("{base}{path}")).json(&json!({})),
            _ => http.get(format!("{base}{path}")),
        };
        let resp = request.send().await.unwrap();
        assert_eq!(
            resp.status(),
            reqwest::StatusCode::SERVICE_UNAVAILABLE,
            "{method} {path} should be closed before the bridge is started"
        );
        let body: Value = resp.json().await.unwrap();
        assert_eq!(
            body["error"]["code"], "bridge_not_started",
            "{method} {path}"
        );
    }

    // Discovery and the documentation stay reachable.
    for path in [
        "/health",
        "/capabilities",
        "/auth",
        "/openapi.json",
        "/docs/",
    ] {
        let resp = http.get(format!("{base}{path}")).send().await.unwrap();
        assert!(
            resp.status().is_success(),
            "{path} should answer while stopped, got {}",
            resp.status()
        );
    }

    // Once started, the same routes open up.
    mother_claude_lib::bridge::control::start(&state, BridgeDefaults::default())
        .await
        .unwrap();
    let resp = http.get(format!("{base}/threads")).send().await.unwrap();
    assert!(resp.status().is_success(), "got {}", resp.status());
}

/// The control surface the app screen drives: start with defaults, read them
/// back, stop again.
#[tokio::test]
async fn the_dashboard_can_start_configure_and_stop_the_bridge() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base_dir = dir.keep();
    let state = Inner::with_bridge_config(
        ClaudeHome::with_base(&base_dir),
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        Auth::ephemeral(),
        BridgeConfig {
            enabled: true,
            port: None,
            require_token: false,
            api_key: None,
            allowed_origins: Vec::new(),
        },
    );
    let token = state.auth.token.clone();
    let app = mother_claude_lib::server::router(state.clone());
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
    let base = format!("http://{addr}");
    let http = client();

    // Stopped on open.
    let status: Value = http
        .get(format!("{base}/api/bridge"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["running"], false);
    assert_eq!(status["enabled"], true);

    // A bad configuration is refused on the button that set it, naming the field.
    let resp = http
        .post(format!("{base}/api/bridge/start"))
        .bearer_auth(&token)
        .json(&json!({ "defaults": { "effort": "turbo" } }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        resp.json::<Value>().await.unwrap()["error"]["field"],
        "effort"
    );

    // Nominating a conversation that does not exist fails here, not later.
    let resp = http
        .post(format!("{base}/api/bridge/start"))
        .bearer_auth(&token)
        .json(&json!({ "defaults": { "defaultThread": "no-such-conversation" } }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        resp.json::<Value>().await.unwrap()["error"]["field"],
        "defaultThread"
    );

    // A good one starts, and the defaults come back.
    let cwd = std::env::temp_dir().to_string_lossy().into_owned();
    let status: Value = http
        .post(format!("{base}/api/bridge/start"))
        .bearer_auth(&token)
        .json(&json!({ "defaults": {
            "cwd": cwd,
            "model": "haiku",
            "effort": "low",
            "permissionMode": "acceptEdits",
            // Blank strings from a form mean "unset".
            "thinking": "",
        }}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["running"], true);
    assert_eq!(status["defaults"]["model"], "haiku");
    assert_eq!(status["defaults"]["effort"], "low");
    assert_eq!(status["defaults"]["permissionMode"], "acceptEdits");
    assert!(status["defaults"]["thinking"].is_null());
    assert!(status["startedAt"].is_number());

    // The bridge routes on /v1 are now open.
    let resp = http
        .get(format!("{base}/v1/threads"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());

    // And stop closes them again.
    let status: Value = http
        .post(format!("{base}/api/bridge/stop"))
        .bearer_auth(&token)
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["running"], false);
    let resp = http
        .get(format!("{base}/v1/threads"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
}

/// Naming a conversation that has never existed is a 404 about the id — and it
/// must not spawn a runtime just to say so.
#[tokio::test]
async fn an_unknown_conversation_is_a_404_without_starting_a_runtime() {
    let (base, token, state) = boot().await;
    let resp = client()
        .post(format!("{base}/chat"))
        .bearer_auth(&token)
        .json(&json!({ "message": "hi", "thread_id": "no-such-conversation" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
    assert_eq!(
        resp.json::<Value>().await.unwrap()["error"]["code"],
        "thread_not_found"
    );
    assert!(
        !state.bridge.host.is_running().await,
        "resolving a bogus id must not start the Node host"
    );
}

/// The bridge promises one error shape. That has to hold for the failures axum
/// generates on its own — 401, 404, 405 — not just the ones handlers return.
#[tokio::test]
async fn framework_level_failures_use_the_envelope_too() {
    let (base, token, _state) = boot().await;
    let http = client();

    let resp = http
        .get(format!("{base}/capabilities"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["code"], "unauthorized");
    assert!(body["error"]["message"].as_str().unwrap().contains("token"));

    let resp = http
        .get(format!("{base}/no-such-endpoint"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
    assert_eq!(
        resp.json::<Value>().await.unwrap()["error"]["code"],
        "not_found"
    );

    // DELETE on a GET-only route.
    let resp = http
        .delete(format!("{base}/capabilities"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(
        resp.json::<Value>().await.unwrap()["error"]["code"],
        "method_not_allowed"
    );
}

/// The whole point of the loopback bridge is that a local tool — curl, Postman,
/// a page you are building — can just call it. This is the exact request from
/// the README, with no token and no setup.
#[tokio::test]
async fn the_loopback_bridge_answers_without_a_token_or_an_allowlisted_origin() {
    // `false` is what `bridge::serve` passes for the dedicated port by default.
    let (base, _token, _state) = boot_tokenless().await;
    let http = client();

    let resp = http
        .get(format!("{base}/capabilities"))
        .header("accept", "application/json")
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "an unauthenticated GET /capabilities must succeed, got {}",
        resp.status()
    );
    let caps: Value = resp.json().await.unwrap();
    assert_eq!(caps["auth"]["require_token"], false);

    // From a page on another origin, which is how a web app would reach it.
    let resp = http
        .get(format!("{base}/capabilities"))
        .header("origin", "https://some-other-site.example")
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "a cross-origin browser request must succeed by default, got {}",
        resp.status()
    );

    // And a real POST, since that is what actually drives Claude.
    let resp = http
        .post(format!("{base}/threads"))
        .header("origin", "https://some-other-site.example")
        .json(&json!({ "cwd": "/definitely/not/a/directory" }))
        .send()
        .await
        .unwrap();
    // Refused on its merits (the directory does not exist), not on auth.
    assert_eq!(resp.status(), reqwest::StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(resp.json::<Value>().await.unwrap()["error"]["field"], "cwd");
}

/// Startup reports Claude's own sign-in, so a signed-out account is one clear
/// statement rather than every turn returning "Not logged in".
#[tokio::test]
async fn the_bridge_reports_claude_s_own_sign_in() {
    let (base, _token, _state) = boot_tokenless().await;
    let http = client();

    let health: Value = http
        .get(format!("{base}/health"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    // Cached and not yet refreshed in this harness: present, and a boolean.
    assert!(health["claude_authenticated"].is_boolean());

    let auth: Value = http
        .get(format!("{base}/auth"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(auth["authenticated"].is_boolean());
    assert!(auth["summary"].as_str().is_some());
    assert!(auth["login_hint"].as_str().unwrap().contains("auth login"));
}

/// The pages reference their assets relatively, so the no-slash form has to
/// redirect rather than render a shell whose every asset 404s.
#[tokio::test]
async fn directory_paths_redirect_to_their_trailing_slash_form() {
    let (base, _token, _state) = boot().await;
    let no_redirect = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    for path in ["/docs", "/example"] {
        let resp = no_redirect
            .get(format!("{base}{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            reqwest::StatusCode::PERMANENT_REDIRECT,
            "{path} should redirect"
        );
        assert_eq!(
            resp.headers()["location"],
            format!("{path}/"),
            "{path} redirects to the wrong place"
        );
    }
}

#[tokio::test]
async fn input_is_validated_before_the_runtime_is_touched() {
    let (base, token, _state) = boot().await;
    let http = client();

    // A 2x2 PNG: structurally valid, but the model rejects images this small
    // with an untyped error, so the bridge refuses it with a typed one. This
    // must happen without ever starting the Node host.
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend_from_slice(&13u32.to_be_bytes());
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&2u32.to_be_bytes());
    png.extend_from_slice(&2u32.to_be_bytes());
    png.extend_from_slice(&[8, 6, 0, 0, 0]);
    let data = mother_claude_lib::bridge::inputs::encode_base64(&png);

    let resp = http
        .post(format!("{base}/threads/any/turns"))
        .bearer_auth(&token)
        .json(&json!({
            "input": [{
                "type": "image",
                "source": { "type": "base64", "media_type": "image/png", "data": data },
            }],
        }))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["code"], "image_dimensions");

    // A file_id from a client is never relayed: uploads are workspace-scoped.
    let resp = http
        .post(format!("{base}/threads/any/turns"))
        .bearer_auth(&token)
        .json(&json!({
            "input": [{ "type": "image", "source": { "type": "file", "file_id": "file_x" } }],
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::FORBIDDEN);
    assert_eq!(
        resp.json::<Value>().await.unwrap()["error"]["code"],
        "file_id_not_accepted"
    );

    // An empty input is rejected rather than starting an empty turn.
    let resp = http
        .post(format!("{base}/threads/any/turns"))
        .bearer_auth(&token)
        .json(&json!({ "input": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.json::<Value>().await.unwrap()["error"]["code"],
        "invalid_input"
    );
}

#[tokio::test]
async fn sse_cursors_are_validated_before_the_stream_opens() {
    let (base, token, state) = boot().await;
    let http = client();

    // Put one event on the global log so the cursor bounds are meaningful.
    state
        .bridge
        .ops
        .global
        .append(&json!({ "method": "test", "params": {} }));

    // A cursor past the head is a readable 422, not a stream that dies.
    let resp = http
        .get(format!("{base}/events?after=99"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        resp.json::<Value>().await.unwrap()["error"]["code"],
        "invalid_cursor"
    );

    // So is a query parameter we do not understand.
    let resp = http
        .get(format!("{base}/events?limit=5"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn the_messages_api_explains_itself_when_unconfigured() {
    // The suite must not depend on whether the developer has a key exported.
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        return;
    }
    let (base, token, _state) = boot().await;
    let resp = client()
        .post(format!("{base}/messages"))
        .bearer_auth(&token)
        .json(&json!({ "messages": [{ "role": "user", "content": "hi" }] }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["code"], "messages_api_unconfigured");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("ANTHROPIC_API_KEY"));
}

// --------------------------------------------------------------------------
// Live tests — real conversations, real tokens. Opt in with `-- --ignored`.
// --------------------------------------------------------------------------

/// A whole conversation: create, run a turn, read it back, close.
#[tokio::test]
#[ignore = "starts a real Claude conversation and spends tokens"]
async fn live_conversation_round_trip() {
    let (base, token, _state) = boot_live().await;
    let http = client();
    let cwd = std::env::temp_dir().join("mother-claude-bridge-live");
    std::fs::create_dir_all(&cwd).unwrap();

    let created: Value = http
        .post(format!("{base}/threads"))
        .bearer_auth(&token)
        .json(&json!({
            "cwd": cwd.to_string_lossy(),
            "model": "haiku",
            "setting_sources": [],
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = created["thread_id"]
        .as_str()
        .expect("thread_id")
        .to_string();
    eprintln!("created {thread_id}");

    let result: Value = http
        .post(format!("{base}/threads/{thread_id}/run"))
        .bearer_auth(&token)
        .json(&json!({ "input": "Reply with exactly: BRIDGEOK" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    eprintln!("run -> {result}");
    assert_eq!(result["status"], "completed");
    assert!(result["result"]["result"]
        .as_str()
        .unwrap_or_default()
        .contains("BRIDGEOK"));

    // The turn is replayable from its operation log after the fact.
    let operation_id = result["operation_id"].as_str().unwrap();
    let events = http
        .get(format!("{base}/operations/{operation_id}/events"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(events.contains("turn/started"));
    assert!(events.contains("bridge/completed"));

    let closed: Value = http
        .delete(format!("{base}/threads/{thread_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(closed["status"], "closed");
}

/// `/chat` creates a conversation and answers in one call.
#[tokio::test]
#[ignore = "starts a real Claude conversation and spends tokens"]
async fn live_chat_shortcut_and_context_recall() {
    let (base, token, _state) = boot_live().await;
    let http = client();
    let cwd = std::env::temp_dir().join("mother-claude-bridge-live");
    std::fs::create_dir_all(&cwd).unwrap();

    let first: Value = http
        .post(format!("{base}/chat"))
        .bearer_auth(&token)
        .json(&json!({
            "message": "Remember the word ORCHARD. Reply with just: OK",
            "cwd": cwd.to_string_lossy(),
            "model": "haiku",
            "setting_sources": [],
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    eprintln!("chat 1 -> {first}");
    let thread_id = first["thread_id"].as_str().expect("thread_id").to_string();

    let second: Value = http
        .post(format!("{base}/chat"))
        .bearer_auth(&token)
        .json(&json!({
            "message": "What word did I ask you to remember? One word.",
            "thread_id": thread_id,
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    eprintln!("chat 2 -> {second}");
    assert!(second["response"]
        .as_str()
        .unwrap_or_default()
        .to_uppercase()
        .contains("ORCHARD"));
}

/// An image reaches the model through the bridge's own validation path.
#[tokio::test]
#[ignore = "starts a real Claude conversation and spends tokens"]
async fn live_vision_turn() {
    let (base, token, _state) = boot_live().await;
    let http = client();
    let cwd = std::env::temp_dir().join("mother-claude-bridge-live");
    std::fs::create_dir_all(&cwd).unwrap();

    // A 200x200 solid blue PNG, built here so the test needs no fixture.
    let png = solid_blue_png(200, 200);
    let data = mother_claude_lib::bridge::inputs::encode_base64(&png);

    let reply: Value = http
        .post(format!("{base}/chat"))
        .bearer_auth(&token)
        .json(&json!({
            "input": [
                { "type": "image", "url": format!("data:image/png;base64,{data}") },
                { "type": "text", "text": "What single colour is this image? One word." },
            ],
            "cwd": cwd.to_string_lossy(),
            "model": "haiku",
            "setting_sources": [],
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    eprintln!("vision -> {reply}");
    assert!(reply["response"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase()
        .contains("blue"));
}

/// Run the shipped JavaScript client against a live bridge.
///
/// This is the only check that exercises `client.mjs` itself — the file the
/// example console imports and the one users will `import` from a page. A
/// change that breaks it should fail here, not in someone's browser.
#[tokio::test]
#[ignore = "starts a real Claude conversation and spends tokens"]
async fn live_javascript_client_smoke() {
    let (base, token, _state) = boot_live().await;
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../scripts/bridge-smoke.mjs")
        .canonicalize()
        .expect("scripts/bridge-smoke.mjs");

    let output = tokio::process::Command::new("node")
        .arg(&script)
        .arg(&base)
        .arg(&token)
        .output()
        .await
        .expect("node on PATH");

    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "the JS client smoke test failed");
}

/// The three conversation scenarios, end to end against real Claude.
///
/// 1. No conversation yet → the first message creates one and later messages
///    join it.
/// 2. `createNewChat: true` opens a fresh one, which then becomes the one later
///    messages join.
/// 3. Starting with a nominated conversation sends messages there.
#[tokio::test]
#[ignore = "starts real Claude conversations and spends tokens"]
async fn live_messages_join_one_conversation_until_told_otherwise() {
    let (base, _token, state) = boot_live_started().await;
    let http = client();
    let cwd = std::env::temp_dir().join("mother-claude-bridge-live");
    std::fs::create_dir_all(&cwd).unwrap();

    let say = |body: Value| {
        let http = http.clone();
        let base = base.clone();
        async move {
            let resp = http
                .post(format!("{base}/chat"))
                .json(&body)
                .send()
                .await
                .unwrap();
            let status = resp.status();
            let value: Value = resp.json().await.unwrap();
            assert!(status.is_success(), "chat failed: {value}");
            value
        }
    };

    // --- scenario 1: a bare message opens one conversation and stays in it ---
    let first = say(json!({
        "message": "Remember the word ORCHARD. Reply with just: OK",
        "cwd": cwd.to_string_lossy(),
        "model": "haiku",
        "setting_sources": [],
    }))
    .await;
    let thread_a = first["thread_id"].as_str().unwrap().to_string();

    let second = say(json!({ "message": "What word did I ask you to remember? One word." })).await;
    assert_eq!(
        second["thread_id"].as_str().unwrap(),
        thread_a,
        "a bare second message must stay in the same conversation"
    );
    assert!(
        second["response"]
            .as_str()
            .unwrap_or_default()
            .to_uppercase()
            .contains("ORCHARD"),
        "context did not carry: {}",
        second["response"]
    );

    // --- scenario 2: createNewChat opens a fresh one, and it becomes current -
    let third = say(json!({
        "message": "Reply with just: NEW",
        "createNewChat": true,
    }))
    .await;
    let thread_b = third["thread_id"].as_str().unwrap().to_string();
    assert_ne!(
        thread_b, thread_a,
        "createNewChat must open a new conversation"
    );

    let fourth =
        say(json!({ "message": "What word did I ask you to remember? One word, or say NONE." }))
            .await;
    assert_eq!(
        fourth["thread_id"].as_str().unwrap(),
        thread_b,
        "later bare messages must join the newest conversation"
    );
    assert!(
        !fourth["response"]
            .as_str()
            .unwrap_or_default()
            .to_uppercase()
            .contains("ORCHARD"),
        "the new conversation should not have the old one's context: {}",
        fourth["response"]
    );

    // --- naming a conversation explicitly still wins, without changing it ----
    let explicit = say(json!({
        "message": "What word did I ask you to remember? One word.",
        "thread_id": thread_a,
    }))
    .await;
    assert_eq!(explicit["thread_id"].as_str().unwrap(), thread_a);
    assert!(explicit["response"]
        .as_str()
        .unwrap_or_default()
        .to_uppercase()
        .contains("ORCHARD"));

    let after = say(json!({ "message": "Reply with just: OK" })).await;
    assert_eq!(
        after["thread_id"].as_str().unwrap(),
        thread_b,
        "an explicitly addressed message must not change which conversation is current"
    );

    // --- scenario 3: restart nominating an existing conversation ------------
    mother_claude_lib::bridge::control::start(
        &state,
        BridgeDefaults {
            cwd: Some(cwd.to_string_lossy().into_owned()),
            model: Some("haiku".into()),
            setting_sources: Some(Vec::new()),
            default_thread: Some(thread_a.clone()),
            ..Default::default()
        },
    )
    .await
    .expect("restart with a nominated conversation");

    let nominated =
        say(json!({ "message": "What word did I ask you to remember? One word." })).await;
    assert_eq!(
        nominated["thread_id"].as_str().unwrap(),
        thread_a,
        "messages must join the conversation nominated at start"
    );
    assert!(nominated["response"]
        .as_str()
        .unwrap_or_default()
        .to_uppercase()
        .contains("ORCHARD"));

    eprintln!("conversation A = {thread_a}\nconversation B = {thread_b}");
}

/// The startup sign-in check against the real CLI. Ignored because it shells
/// out to `claude` and reads the developer's actual account.
#[tokio::test]
#[ignore = "runs `claude auth status` against the real account"]
async fn live_startup_reports_the_real_claude_account() {
    let (base, _token, state) = boot_tokenless().await;

    let status = state.refresh_claude_auth().await;
    eprintln!("startup banner would read: Claude: {}", status.summary());
    assert!(
        status.error.is_none(),
        "the auth check itself failed: {:?}",
        status.error
    );
    assert!(
        status.logged_in,
        "expected a signed-in Claude; got {}",
        status.summary()
    );

    let auth: Value = client()
        .get(format!("{base}/auth"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(auth["authenticated"], true);
    assert!(auth["summary"].as_str().unwrap().len() > 1);
    eprintln!("GET /auth -> {}", auth["summary"]);

    // …and health reports it without leaking the account's identity.
    let health: Value = client()
        .get(format!("{base}/health"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health["claude_authenticated"], true);
    assert!(health.get("email").is_none());
}

/// Bind the *real* default port the way the app does and make the exact request
/// from the documentation. Ignored because it takes a fixed port (5612) and so
/// cannot run alongside a live Mother Claude.
#[tokio::test]
#[ignore = "binds the real default bridge port (5612)"]
async fn live_default_port_serves_the_documented_curl() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base_dir = dir.keep();
    let state = Inner::new(
        ClaudeHome::with_base(&base_dir),
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        Auth::ephemeral(),
    );
    let port = state.bridge.config.port.expect("a default bridge port");
    // The bridge does not listen until it is started, which is the point.
    mother_claude_lib::bridge::control::start(
        &state,
        mother_claude_lib::bridge::control::BridgeDefaults::default(),
    )
    .await
    .expect("the bridge should start");

    let base = format!("http://127.0.0.1:{port}");
    let http = client();

    // curl -X GET http://127.0.0.1:5612/capabilities -H 'accept: application/json'
    let resp = http
        .get(format!("{base}/capabilities"))
        .header("accept", "application/json")
        .send()
        .await
        .expect("the bridge port should be listening");
    assert!(resp.status().is_success(), "got {}", resp.status());
    let caps: Value = resp.json().await.unwrap();
    assert_eq!(caps["auth"]["require_token"], false);
    eprintln!("GET {base}/capabilities -> {}", caps["app"]);

    // The same routes are also served under /v1 on this listener.
    let resp = http
        .get(format!("{base}/v1/capabilities"))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());

    // And the reference and console are reachable.
    for path in ["/docs/", "/example/", "/openapi.json", "/client.mjs"] {
        let resp = http.get(format!("{base}{path}")).send().await.unwrap();
        assert!(resp.status().is_success(), "{path} -> {}", resp.status());
    }

    // Finally, through the real `curl` binary — the literal command in the
    // documentation, so a change that breaks it breaks this test.
    let curl = tokio::process::Command::new("curl")
        .args([
            "--fail-with-body",
            "-sS",
            "-X",
            "GET",
            &format!("{base}/capabilities"),
            "-H",
            "accept: application/json",
        ])
        .output()
        .await
        .expect("curl on PATH");
    assert!(
        curl.status.success(),
        "curl failed: {}",
        String::from_utf8_lossy(&curl.stderr)
    );
    let body: Value = serde_json::from_slice(&curl.stdout).expect("curl returned JSON");
    assert_eq!(body["app"], "mother-claude");
    eprintln!(
        "curl -X GET {base}/capabilities -H 'accept: application/json'  ->  {} bytes, \
         require_token={}",
        curl.stdout.len(),
        body["auth"]["require_token"]
    );
}

/// Build a minimal, valid, single-colour PNG without an image dependency.
fn solid_blue_png(width: u32, height: u32) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut table = [0u32; 256];
        for (i, entry) in table.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *entry = c;
        }
        let mut crc = 0xFFFF_FFFFu32;
        for byte in data {
            crc = table[((crc ^ u32::from(*byte)) & 0xFF) as usize] ^ (crc >> 8);
        }
        crc ^ 0xFFFF_FFFF
    }

    fn adler32(data: &[u8]) -> u32 {
        let (mut a, mut b) = (1u32, 0u32);
        for byte in data {
            a = (a + u32::from(*byte)) % 65521;
            b = (b + a) % 65521;
        }
        (b << 16) | a
    }

    fn chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = (payload.len() as u32).to_be_bytes().to_vec();
        let mut body = kind.to_vec();
        body.extend_from_slice(payload);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
        out
    }

    // Raw scanlines: a filter byte of 0, then RGB per pixel. Every row is
    // identical, so build one and repeat it.
    let mut scanline = vec![0u8];
    scanline.extend(
        std::iter::repeat([0x20u8, 0x4E, 0xD8])
            .take(width as usize)
            .flatten(),
    );
    let raw: Vec<u8> = scanline.repeat(height as usize);

    // zlib with stored (uncompressed) deflate blocks — valid, and no dependency.
    let mut z = vec![0x78, 0x01];
    for (i, block) in raw.chunks(65_535).enumerate() {
        let last = u8::from((i + 1) * 65_535 >= raw.len());
        z.push(last);
        z.extend_from_slice(&(block.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut ihdr = width.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit RGB

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend_from_slice(&chunk(b"IHDR", &ihdr));
    png.extend_from_slice(&chunk(b"IDAT", &z));
    png.extend_from_slice(&chunk(b"IEND", &[]));
    png
}
