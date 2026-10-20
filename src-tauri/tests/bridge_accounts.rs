//! Bridge conversations under an account (docs/ACCOUNTS.md P1.7), offline:
//! a fake Node host (`tests/fixtures/fake-bridge-host.js`) records what each
//! conversation was started with. Nothing here runs Claude.

use std::net::SocketAddr;
use std::path::PathBuf;

use mother_claude_lib::bridge::control::BridgeDefaults;
use mother_claude_lib::bridge::{BridgeConfig, TokenPolicy};
use mother_claude_lib::claude::secrets::Secret;
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use serde_json::{json, Value};

const TOKEN: &str = "sk-ant-oat01-BridgeAccountToken0123456789abcdefghij";

async fn boot(log: &std::path::Path) -> (String, AppState, tempfile::TempDir) {
    let fake = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-bridge-host.js");
    // Process-wide, but this file runs one test: nothing else reads them.
    std::env::set_var("MOTHER_CLAUDE_BRIDGE_HOST_PATH", &fake);
    std::env::set_var("FAKE_BRIDGE_LOG", log);
    let home = tempfile::tempdir().unwrap();
    let mut auth = Auth::ephemeral();
    auth.config_dir = Some(home.path().join("app-data"));
    let state = Inner::with_bridge_config(
        ClaudeHome::with_base(home.path()),
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        auth,
        BridgeConfig {
            enabled: true,
            port: None,
            token: TokenPolicy::Local,
            api_key: None,
            allowed_origins: Vec::new(),
        },
    );
    let app = axum::Router::new()
        .merge(mother_claude_lib::bridge::router(
            state.clone(),
            TokenPolicy::Local,
        ))
        .with_state(state.clone());
    let app = mother_claude_lib::server::context::layer(
        app,
        state.clone(),
        mother_claude_lib::server::Ingress::BridgeLoopback,
    );
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
    (format!("http://{addr}"), state, home)
}

fn records(log: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn created(log: &std::path::Path, id: &str) -> Value {
    records(log)
        .into_iter()
        .find(|r| r["op"] == "session.create" && r["sessionId"] == id)
        .unwrap_or_else(|| panic!("no session.create for {id}: {:?}", records(log)))
}

async fn post(base: &str, body: Value) -> (reqwest::StatusCode, Value) {
    let r = reqwest::Client::new()
        .post(format!("{base}/threads"))
        .json(&body)
        .send()
        .await
        .unwrap();
    (r.status(), r.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn conversations_run_under_their_account() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("host.log");
    let (base, st, _home) = boot(&log).await;
    let pro = st
        .accounts
        .add_token("Pro", Some(&Secret::new(TOKEN)), None)
        .unwrap();
    let work = st
        .accounts
        .add_directory("Work", &tmp.path().join("claude-work"), None)
        .unwrap();
    mother_claude_lib::bridge::control::start(
        &st,
        BridgeDefaults {
            account: Some(pro.id.clone()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    // The start-time default: the token account's token, the overrides gone.
    let (s, v) = post(&base, json!({})).await;
    assert_eq!(s, reqwest::StatusCode::CREATED, "{v}");
    let id = v["thread_id"].as_str().unwrap().to_string();
    let r = created(&log, &id);
    assert_eq!(
        r["env"]["tokenSha256"],
        Secret::new(TOKEN).sha256_hex(),
        "{r}"
    );
    assert!(r["env"]["unset"]
        .as_array()
        .unwrap()
        .contains(&json!("ANTHROPIC_API_KEY")));
    assert!(r["env"]["configDir"].is_null());
    assert_eq!(st.bridge.thread(&id).await.unwrap().account, pro.id);
    // The session row names it too.
    let row = st.find_session(&id).await.unwrap();
    assert_eq!(row.account.unwrap().id, pro.id);

    // The request wins over the default: the Default inherits as always.
    let (_, v) = post(&base, json!({"account": "default"})).await;
    let r = created(&log, v["thread_id"].as_str().unwrap());
    assert!(r["env"].is_null(), "{r}");

    // A directory account: its own config directory.
    let (_, v) = post(&base, json!({"account": work.id})).await;
    let r = created(&log, v["thread_id"].as_str().unwrap());
    assert_eq!(
        r["env"]["configDir"].as_str().map(PathBuf::from),
        work.dir.clone(),
        "{r}"
    );
    assert!(r["env"]["tokenSha256"].is_null());

    // Continuing a conversation on another account forks it; one that keeps
    // its own directory needs a copy first.
    let resume = "6b0b2a3c-1d2e-4f50-8a9b-0c1d2e3f4a5b";
    let (s, v) = post(&base, json!({"resume": resume, "account": pro.id})).await;
    assert_eq!(s, reqwest::StatusCode::CREATED, "{v}");
    assert_eq!(v["forked"], true);
    let r = created(&log, v["thread_id"].as_str().unwrap());
    assert_eq!(r["resume"], resume);
    assert_eq!(r["forkSession"], true);
    let (s, v) = post(&base, json!({"resume": resume, "account": work.id})).await;
    assert_eq!(s, reqwest::StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "copy_consent_required");
    // Allowed: the conversation is copied into Work's directory and forked
    // there.
    let proj = _home.path().join("projects/-tmp");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(
        proj.join(format!("{resume}.jsonl")),
        format!("{{\"type\":\"user\",\"cwd\":\"/tmp\",\"sessionId\":\"{resume}\"}}\n"),
    )
    .unwrap();
    st.accounts
        .set_copy_consent("default", &work.id, true)
        .unwrap();
    let (s, v) = post(&base, json!({"resume": resume, "account": work.id})).await;
    assert_eq!(s, reqwest::StatusCode::CREATED, "{v}");
    assert_eq!(v["forked"], true);
    assert!(work
        .dir
        .as_ref()
        .unwrap()
        .join(format!("projects/-tmp/{resume}.jsonl"))
        .is_file());
    let r = created(&log, v["thread_id"].as_str().unwrap());
    assert_eq!(
        r["env"]["configDir"].as_str().map(PathBuf::from),
        work.dir.clone()
    );

    // Refusals.
    let (s, v) = post(&base, json!({"account": "acc_000000000000"})).await;
    assert_eq!(s, reqwest::StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"]["field"], "account");
    let empty = st.accounts.add_token("Empty", None, None).unwrap();
    let (s, v) = post(&base, json!({"account": empty.id})).await;
    assert_eq!(s, reqwest::StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "account_token_missing");

    // The host never saw the token in anything it could log: only the env
    // block of a conversation carries it, and the fake recorded its hash.
    assert!(!std::fs::read_to_string(&log).unwrap().contains(TOKEN));

    // Starting with an account that does not exist is refused.
    mother_claude_lib::bridge::control::stop(&st).await.unwrap();
    let err = mother_claude_lib::bridge::control::start(
        &st,
        BridgeDefaults {
            account: Some("acc_ffffffffffff".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.body()["error"]["field"], "account");
}
