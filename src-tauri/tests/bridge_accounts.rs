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
    let home = tempfile::tempdir().unwrap();
    let (base, state) = boot_at(log, home.path()).await;
    (base, state, home)
}

/// A server on `home` (and `<home>/app-data`): a second one on the same
/// directories is the app after a restart.
async fn boot_at(log: &std::path::Path, home: &std::path::Path) -> (String, AppState) {
    let fake = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-bridge-host.js");
    // Process-wide, but this file runs one test: nothing else reads them.
    std::env::set_var("MOTHER_CLAUDE_BRIDGE_HOST_PATH", &fake);
    std::env::set_var("FAKE_BRIDGE_LOG", log);
    let mut auth = Auth::ephemeral();
    auth.config_dir = Some(home.join("app-data"));
    let state = Inner::with_bridge_config(
        ClaudeHome::with_base(home),
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
    (format!("http://{addr}"), state)
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

    // A conversation whose token stopped working marks its account at once;
    // a turn that works clears it (§5.8).
    let turn = |thread: String, text: &'static str| {
        let base = base.clone();
        async move {
            reqwest::Client::new()
                .post(format!("{base}/threads/{thread}/turns"))
                .json(&json!({ "input": text }))
                .send()
                .await
                .unwrap()
                .status()
        }
    };
    let failed = |st: &AppState| st.accounts.runtime.status(&pro.id).auth_failed;
    assert!(turn(id.clone(), "AUTHFAIL please").await.is_success());
    for _ in 0..100 {
        if failed(&st) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(failed(&st), "authentication_failed marks the account");
    assert!(turn(id.clone(), "hello").await.is_success());
    for _ in 0..100 {
        if !failed(&st) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(!failed(&st), "a turn that worked clears it");

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

    // After a restart the token account's conversation keeps its account:
    // `/chat` naming it resumes it with the token, not the Default's sign-in.
    // (A `sessions/` directory: a CLI with pidfiles, so a fresh transcript
    // alone does not count as running.)
    std::fs::create_dir_all(_home.path().join("sessions")).unwrap();
    std::fs::write(
        proj.join(format!("{id}.jsonl")),
        format!("{{\"type\":\"user\",\"cwd\":\"/tmp\",\"sessionId\":\"{id}\"}}\n"),
    )
    .unwrap();
    drop(st);
    let (base, st) = boot_at(&log, _home.path()).await;
    mother_claude_lib::server::monitor::refresh_once(&st).await;
    mother_claude_lib::bridge::control::start(&st, BridgeDefaults::default())
        .await
        .unwrap();
    let before = records(&log).len();
    let r = reqwest::Client::new()
        .post(format!("{base}/chat"))
        .json(&json!({ "thread_id": id, "message": "hello again" }))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
    let resumed = records(&log)
        .into_iter()
        .skip(before)
        .find(|r| r["op"] == "session.create" && r["sessionId"] == id)
        .expect("the conversation was resumed");
    assert_eq!(resumed["resume"], id.as_str());
    assert_eq!(
        resumed["env"]["tokenSha256"],
        Secret::new(TOKEN).sha256_hex()
    );
}

/// A folder's trust is the conversation's account's (each keeps its own): a
/// folder the Default has opened is still untrusted for a directory account
/// that never did, for a caller without the dangerous right. Refused before
/// any conversation starts, so this needs no host (and sets no variable the
/// other test reads).
#[tokio::test]
async fn a_folder_is_trusted_per_account() {
    let home = tempfile::tempdir().unwrap();
    let state = Inner::with_bridge_config(
        ClaudeHome::with_base(home.path()),
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        Auth::ephemeral(),
        BridgeConfig {
            enabled: true,
            port: None,
            token: TokenPolicy::Local,
            api_key: None,
            allowed_origins: Vec::new(),
        },
    );
    mother_claude_lib::bridge::control::start(&state, BridgeDefaults::default())
        .await
        .unwrap();
    let app = mother_claude_lib::server::context::layer(
        axum::Router::new()
            .merge(mother_claude_lib::bridge::router(
                state.clone(),
                TokenPolicy::Local,
            ))
            .with_state(state.clone()),
        state.clone(),
        mother_claude_lib::server::Ingress::BridgeLoopback,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    // The Default opened the folder before (its transcript recorded it).
    let folder = tempfile::tempdir().unwrap();
    let cwd = folder.path().to_string_lossy().into_owned();
    let proj = home
        .path()
        .join("projects")
        .join(mother_claude_lib::claude::encode_cwd(&cwd));
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(
        proj.join("1b0b2a3c-1d2e-4f50-8a9b-0c1d2e3f4a5b.jsonl"),
        json!({"type": "user", "cwd": cwd, "sessionId": "1b0b2a3c-1d2e-4f50-8a9b-0c1d2e3f4a5b"})
            .to_string()
            + "\n",
    )
    .unwrap();
    use mother_claude_lib::bridge::routes::threads::cwd_is_trusted;
    assert!(cwd_is_trusted(&state.home, &cwd), "the Default trusts it");
    let dirs = tempfile::tempdir().unwrap();
    let work = state
        .accounts
        .add_directory("Work", &dirs.path().join("claude-work"), None)
        .unwrap();
    assert!(!cwd_is_trusted(
        &state.accounts.home(&work.id).unwrap(),
        &cwd
    ));

    // Another website on this machine: no dangerous right.
    let r = reqwest::Client::new()
        .post(format!("{base}/threads"))
        .header("origin", "https://example.com")
        .json(&json!({ "cwd": cwd, "account": work.id }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::FORBIDDEN);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"]["code"], "untrusted_cwd", "{v}");
}
