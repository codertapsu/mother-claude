//! Switching the account of a conversation of the API (docs/ACCOUNTS.md
//! §15.1, P9.1), offline: a fake Node host (`tests/fixtures/fake-bridge-host.js`)
//! records what each conversation was opened with. Between turns the
//! conversation is closed in the host, moved when the directory changes and
//! opened again under the same id with the other account's environment and
//! its options; during a turn, and for an `accountOnly` server's
//! conversation, it is refused.
//!
//! One test: the host is chosen through process-wide variables.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use mother_claude_lib::bridge::control::BridgeDefaults;
use mother_claude_lib::bridge::{BridgeConfig, ThreadRecord, TokenPolicy};
use mother_claude_lib::claude::secrets::Secret;
use mother_claude_lib::claude::switch::{self, Outcome, SwitchBody};
use mother_claude_lib::claude::{encode_cwd, ClaudeHome};
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::server::context::{ClientContext, Ingress, Principal};
use mother_claude_lib::state::{Inner, ServerConfig};
use reqwest::StatusCode;
use serde_json::{json, Value};

const TOKEN: &str = "sk-ant-oat01-BridgeSwitchToken0123456789abcdefghijk";

fn records(log: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

async fn call(method: &str, url: &str, body: Option<Value>) -> (StatusCode, Value) {
    let c = reqwest::Client::new();
    let mut r = match method {
        "GET" => c.get(url),
        "POST" => c.post(url),
        other => panic!("{other}"),
    };
    if let Some(b) = body {
        r = r.json(&b);
    }
    let resp = r.send().await.unwrap();
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

fn desktop() -> ClientContext {
    let mut h = axum::http::HeaderMap::new();
    h.insert("host", "127.0.0.1:6725".parse().unwrap());
    let mut c = ClientContext::new(Ingress::Loopback, "127.0.0.1:9".parse().unwrap(), &h, false);
    c.principal = Principal::Desktop;
    c
}

fn body(to: &str) -> SwitchBody {
    SwitchBody {
        account: to.to_string(),
        when: None,
    }
}

#[tokio::test]
async fn an_api_conversation_switches_between_turns() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("host.log");
    let fake = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-bridge-host.js");
    std::env::set_var("MOTHER_CLAUDE_BRIDGE_HOST_PATH", &fake);
    std::env::set_var("FAKE_BRIDGE_LOG", &log);
    let home = tmp.path().join("claude-home");
    // A CLI with pidfiles: a fresh transcript alone is not a running session.
    std::fs::create_dir_all(home.join("sessions")).unwrap();
    let mut auth = Auth::ephemeral();
    auth.config_dir = Some(tmp.path().join("app-data"));
    let st = Inner::with_bridge_config(
        ClaudeHome::for_account_dir(&home),
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
            st.clone(),
            TokenPolicy::Local,
        ))
        .with_state(st.clone());
    let app = mother_claude_lib::server::context::layer(
        app,
        st.clone(),
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
    let pro = st
        .accounts
        .add_token("Pro", Some(&Secret::new(TOKEN)), None)
        .unwrap();
    let work = st
        .accounts
        .add_directory("Work", &tmp.path().join("claude-work"), None)
        .unwrap();
    let work_dir = work.dir.clone().unwrap();
    mother_claude_lib::bridge::control::start(&st, BridgeDefaults::default())
        .await
        .unwrap();
    let cwd_dir = tempfile::tempdir().unwrap();
    let cwd = cwd_dir.path().to_string_lossy().into_owned();
    let transcript = |dir: &std::path::Path, id: &str| {
        dir.join("projects")
            .join(encode_cwd(&cwd))
            .join(format!("{id}.jsonl"))
    };

    // A conversation of the API with a turn behind it: `/chat` makes it the
    // one unaddressed messages join.
    let (s, v) = call(
        "POST",
        &format!("{base}/chat"),
        Some(json!({ "message": "hello", "cwd": cwd, "model": "opus",
                     "system_prompt_append": "Be brief." })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let id = v["thread_id"].as_str().unwrap().to_string();
    // The SDK wrote its transcript into the Default's directory.
    let path = transcript(&home, &id);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        format!("{}\n", json!({"type": "user", "cwd": cwd, "sessionId": id})),
    )
    .unwrap();
    assert_eq!(
        st.bridge.main().active_thread().as_deref(),
        Some(id.as_str())
    );

    // To the token account (the same directory): closed, opened again under
    // the same id with the token, its options kept.
    let before = records(&log).len();
    let out = switch::switch(&st, &desktop(), &id, &body(&pro.id))
        .await
        .unwrap();
    assert_eq!(
        out,
        Outcome::Switched {
            account: pro.id.clone(),
            moved: false,
            job: None
        }
    );
    let new: Vec<Value> = records(&log).into_iter().skip(before).collect();
    let close = new
        .iter()
        .position(|r| r["op"] == "session.close" && r["sessionId"] == json!(id))
        .expect("closed first");
    let create = new
        .iter()
        .position(|r| r["op"] == "session.create" && r["sessionId"] == json!(id))
        .expect("opened again");
    assert!(close < create);
    let opened = &new[create];
    assert_eq!(opened["resume"], json!(id), "{opened}");
    assert_eq!(opened["forkSession"], false);
    assert_eq!(opened["model"], "opus");
    assert_eq!(opened["permissionMode"], "default");
    assert_eq!(opened["systemPromptAppend"], "Be brief.");
    assert_eq!(
        opened["env"]["tokenSha256"],
        Secret::new(TOKEN).sha256_hex()
    );
    assert_eq!(st.bridge.thread(&id).await.unwrap().account, pro.id);
    assert_eq!(
        st.find_session(&id).await.unwrap().account.unwrap().id,
        pro.id
    );
    assert!(st.find_session(&id).await.unwrap().switching_to.is_none());
    assert_eq!(
        st.bridge.main().active_thread().as_deref(),
        Some(id.as_str()),
        "unaddressed messages still join it"
    );
    // It goes on: a message without a thread joins it, under the token.
    let (s, v) = call(
        "POST",
        &format!("{base}/chat"),
        Some(json!({ "message": "again" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["thread_id"], json!(id));

    // During a turn: refused, nothing closed.
    let (s, v) = call(
        "POST",
        &format!("{base}/threads/{id}/turns"),
        Some(json!({ "input": "ASK first" })),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let rid = {
        let mut found = None;
        for _ in 0..200 {
            let (_, v) = call("GET", &format!("{base}/requests"), None).await;
            found = v["data"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|r| r["thread_id"] == json!(id))
                .and_then(|r| r["request_id"].as_str().map(str::to_string));
            if found.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        found.expect("the turn asked")
    };
    let before = records(&log).len();
    let err = switch::switch(&st, &desktop(), &id, &body("default"))
        .await
        .unwrap_err();
    assert_eq!(err.code, "turn_running");
    assert!(
        !records(&log)
            .into_iter()
            .skip(before)
            .any(|r| r["op"] == "session.close"),
        "nothing was closed"
    );
    assert!(st.control.switches.target(&id).is_none());
    let (s, v) = call(
        "POST",
        &format!("{base}/requests/{rid}/respond"),
        Some(json!({ "decision": "allow" })),
    )
    .await;
    assert!(s.is_success(), "{v}");
    for _ in 0..200 {
        if st
            .bridge
            .ops
            .for_thread(&id)
            .iter()
            .all(|op| op.is_finished())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Into a directory account: with the pair's consent only; then moved
    // there, and opened with its directory.
    let err = switch::switch(&st, &desktop(), &id, &body(&work.id))
        .await
        .unwrap_err();
    assert_eq!(err.code, "copy_consent_required");
    st.accounts
        .set_copy_consent(&pro.id, &work.id, true)
        .unwrap();
    let before = records(&log).len();
    let out = switch::switch(&st, &desktop(), &id, &body(&work.id))
        .await
        .unwrap();
    let Outcome::Switched {
        moved: true,
        job: Some(job),
        ..
    } = out
    else {
        panic!("{out:?}")
    };
    assert!(transcript(&work_dir, &id).is_file());
    assert!(!transcript(&home, &id).exists());
    assert_eq!(st.jobs.get(&job).unwrap().kind, "move");
    let opened = records(&log)
        .into_iter()
        .skip(before)
        .find(|r| r["op"] == "session.create" && r["sessionId"] == json!(id))
        .expect("opened again");
    assert_eq!(
        opened["env"]["configDir"].as_str().map(PathBuf::from),
        Some(work_dir.clone())
    );
    assert!(opened["env"]["tokenSha256"].is_null());
    assert_eq!(st.bridge.thread(&id).await.unwrap().account, work.id);

    // A conversation an `accountOnly` server made stays its account's.
    let solo = mother_claude_lib::bridge::control::add_server(
        &st,
        "Solo",
        Some(0),
        BridgeDefaults {
            account: Some(pro.id.clone()),
            account_only: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let theirs = "3e3e9c5e-3a52-4d4c-9d59-5f0b9a1e2c3d";
    st.bridge
        .insert_thread(ThreadRecord {
            thread_id: theirs.into(),
            cwd: cwd.clone(),
            created_at: 1,
            model: None,
            effort: None,
            permission_mode: "default".into(),
            title: None,
            persisted: false,
            active_turn: None,
            account: pro.id.clone(),
            server: solo.id.clone(),
        })
        .await;
    let err = switch::switch(&st, &desktop(), theirs, &body("default"))
        .await
        .unwrap_err();
    assert_eq!(err.code, "account_not_served");
    assert_eq!(err.body()["error"]["server"], json!(solo.id));
    assert_eq!(st.bridge.thread(theirs).await.unwrap().account, pro.id);
    assert!(!std::fs::read_to_string(&log).unwrap().contains(TOKEN));
}
