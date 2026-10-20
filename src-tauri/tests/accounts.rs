//! Accounts over real HTTP (docs/ACCOUNTS.md P1.2, P1.3): who may list and
//! who may change them, the registry's view (never a token), signing a
//! directory account in, and `setup-token` captured through a pseudo-terminal
//! — all against the FAKE `claude` (`tests/fixtures/fake-claude.py`), which
//! only ever writes into a test's temp directory.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Once;
use std::time::Duration;

use futures_util::StreamExt;
use mother_claude_lib::claude::accounts::AccountKind;
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::server::{self, Ingress};
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use reqwest::StatusCode;
use serde_json::{json, Value};

static SETUP: Once = Once::new();

fn setup() {
    SETUP.call_once(|| {
        let fake = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-claude.py");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755));
        }
        std::env::set_var("MOTHER_CLAUDE_CLI", &fake);
    });
}

struct Fixture {
    st: AppState,
    base: String,
    _home: tempfile::TempDir,
    scratch: tempfile::TempDir,
}

async fn boot() -> Fixture {
    boot_with(|home| ClaudeHome::with_base(home)).await
}

/// A Default whose `.claude.json` is inside its temp directory (with
/// `with_base` it would be the real `~/.claude.json`): for tests that back
/// the Default up.
async fn boot_inside() -> Fixture {
    boot_with(|home| ClaudeHome::for_account_dir(home.to_path_buf())).await
}

async fn boot_with(make_home: impl FnOnce(&std::path::Path) -> ClaudeHome) -> Fixture {
    setup();
    let home = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    // A data dir of its own (secrets in 0600 files there, undo for moves).
    let mut auth = Auth::ephemeral();
    auth.config_dir = Some(scratch.path().join("app-data"));
    // A bridge that never binds a port of its own (the `/v1` mount only).
    let st = Inner::with_bridge_config(
        make_home(home.path()),
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        auth,
        mother_claude_lib::bridge::BridgeConfig {
            enabled: true,
            port: None,
            token: mother_claude_lib::bridge::TokenPolicy::Required,
            api_key: None,
            allowed_origins: Vec::new(),
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = server::router_for(st.clone(), Ingress::Loopback);
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    Fixture {
        st,
        base: format!("http://{addr}"),
        _home: home,
        scratch,
    }
}

impl Fixture {
    fn desktop(&self) -> String {
        self.st.auth.token.clone()
    }

    fn phone(&self) -> String {
        self.st
            .devices
            .issue(
                "Phone",
                "ios",
                None,
                Ingress::Lan,
                "192.168.1.60".parse().unwrap(),
            )
            .1
    }

    async fn call(
        &self,
        method: &str,
        path: &str,
        token: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let c = reqwest::Client::new();
        let url = format!("{}/api{path}", self.base);
        let req = match method {
            "GET" => c.get(url),
            "POST" => c.post(url),
            "PUT" => c.put(url),
            "PATCH" => c.patch(url),
            "DELETE" => c.delete(url),
            other => panic!("{other}"),
        }
        .bearer_auth(token);
        let req = match body {
            Some(b) => req.json(&b),
            None => req,
        };
        let r = req.send().await.unwrap();
        let status = r.status();
        let text = r.text().await.unwrap();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    async fn account(&self, id: &str) -> Value {
        let (_, v) = self.call("GET", "/accounts", &self.desktop(), None).await;
        v["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == id)
            .cloned()
            .unwrap_or(Value::Null)
    }

    async fn wait_for(&self, id: &str, what: &str, ok: impl Fn(&Value) -> bool) -> Value {
        for _ in 0..200 {
            let a = self.account(id).await;
            if ok(&a) {
                return a;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("{what}: {}", self.account(id).await);
    }
}

const TOKEN: &str = "sk-ant-oat01-PastedTokenValue0123456789abcdefghijABCDEFGHIJ";

#[tokio::test]
async fn a_phone_lists_accounts_but_cannot_change_them() {
    let f = boot().await;
    let phone = f.phone();
    let (s, v) = f.call("GET", "/accounts", &phone, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["accounts"][0]["id"], "default");
    assert_eq!(v["accounts"][0]["isDefault"], true);
    let dir = f.scratch.path().join("claude-work");
    for (method, path, body) in [
        (
            "POST",
            "/accounts",
            json!({"name": "Work", "kind": "directory", "dir": dir}),
        ),
        ("PATCH", "/accounts/default", json!({"name": "Mine"})),
        ("DELETE", "/accounts/acc_000000000000", json!({})),
        (
            "PUT",
            "/accounts/defaults",
            json!({"newSession": "default", "bridge": "default"}),
        ),
        (
            "PUT",
            "/accounts/copy-consent",
            json!({"from": "default", "to": "default", "granted": true}),
        ),
        ("POST", "/accounts/default/login", json!({})),
        ("POST", "/accounts/default/token", json!({})),
        ("GET", "/accounts/default/token", json!({})),
    ] {
        let body = (method != "GET").then_some(body);
        let (s, v) = f.call(method, path, &phone, body).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {path}: {v}");
        assert_eq!(v["error"]["code"], "desktop_only", "{method} {path}");
    }
    assert!(!dir.exists(), "nothing was created");
    assert!(f.st.accounts.is_default_only());
}

#[tokio::test]
async fn the_desktop_manages_accounts_and_tokens_never_show() {
    let f = boot().await;
    let desk = f.desktop();
    let mut ws = {
        let url = format!("{}/ws?token={desk}", f.base.replace("http://", "ws://"));
        tokio_tungstenite::connect_async(url).await.unwrap().0
    };

    let dir = f.scratch.path().join("claude-work");
    let (s, work) = f
        .call(
            "POST",
            "/accounts",
            &desk,
            Some(json!({"name": "Work", "kind": "directory", "dir": dir})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{work}");
    assert_eq!(work["kind"], "directory");
    assert!(work["id"].as_str().unwrap().starts_with("acc_"));
    assert!(dir.is_dir(), "the directory is created");

    let (s, pro) = f
        .call(
            "POST",
            "/accounts",
            &desk,
            Some(json!({"name": "Pro", "kind": "token", "token": TOKEN})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{pro}");
    assert_eq!(pro["tokenStored"], true);
    assert_eq!(pro["signedIn"], true);
    assert!(pro.get("dir").is_none());

    let (_, all) = f.call("GET", "/accounts", &desk, None).await;
    let text = all.to_string();
    assert!(
        !text.contains(TOKEN),
        "the token never leaves the secret store"
    );
    assert_eq!(all["accounts"].as_array().unwrap().len(), 3);
    assert_eq!(all["tokenCapture"], true);

    // The frame went out too (the first one after the hello, sessions and
    // planUsage frames), without the token.
    let mut saw_accounts = false;
    for _ in 0..20 {
        let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_secs(2), ws.next()).await
        else {
            break;
        };
        let text = msg.into_text().unwrap().to_string();
        assert!(!text.contains(TOKEN));
        if text.contains("\"t\":\"accounts\"") {
            saw_accounts = true;
            break;
        }
    }
    assert!(saw_accounts);

    // Mistakes are refused with their own codes.
    let (s, v) = f
        .call(
            "POST",
            "/accounts",
            &desk,
            Some(json!({"name": "work", "kind": "token"})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "account_name_taken");
    let (s, v) = f
        .call(
            "POST",
            "/accounts",
            &desk,
            Some(json!({"name": "Nested", "kind": "directory", "dir": dir.join("x")})),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"]["code"], "account_dir_unavailable");
    let (s, v) = f
        .call(
            "POST",
            "/accounts",
            &desk,
            Some(json!({"name": "Bad", "kind": "token", "token": "nope"})),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    let (s, _) = f
        .call(
            "PATCH",
            "/accounts/acc_000000000000",
            &desk,
            Some(json!({"name": "X"})),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Rename, defaults, consent, remove.
    let id = work["id"].as_str().unwrap();
    let (s, v) = f
        .call(
            "PATCH",
            &format!("/accounts/{id}"),
            &desk,
            Some(json!({"name": "Work (Acme)", "color": "#123456"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["name"], "Work (Acme)");
    let (s, v) = f
        .call(
            "PUT",
            "/accounts/defaults",
            &desk,
            Some(json!({"newSession": pro["id"], "bridge": "default", "byProject": {"/p/gitweaver": id}})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(f.st.accounts.account_for_cwd("/p/gitweaver/src"), id);
    assert_eq!(
        f.st.accounts.account_for_cwd("/elsewhere"),
        pro["id"].as_str().unwrap()
    );
    let (s, v) = f
        .call(
            "PUT",
            "/accounts/copy-consent",
            &desk,
            Some(json!({"from": "default", "to": id, "granted": true})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(f.st.accounts.has_copy_consent("default", id));
    let (s, v) = f.call("DELETE", "/accounts/default", &desk, None).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"]["code"], "default_account_fixed");
    let (s, _) = f
        .call("DELETE", &format!("/accounts/{id}"), &desk, None)
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(dir.is_dir(), "removing never deletes the directory");
    assert_eq!(
        f.st.accounts.account_for_cwd("/p/gitweaver"),
        pro["id"].as_str().unwrap()
    );

    // The audit log names what happened, never the token.
    let audit = f.st.devices.audit.recent(50);
    for action in [
        "account.add",
        "account.update",
        "account.defaults",
        "account.copy_consent",
        "account.remove",
    ] {
        assert!(audit.iter().any(|e| e.action == action), "{action}");
    }
    assert!(!serde_json::to_string(&audit).unwrap().contains(TOKEN));
}

/// The `accounts` frames a client receives within `ms`.
async fn accounts_frames<S>(ws: &mut S, ms: u64) -> usize
where
    S: futures_util::Stream<
            Item = Result<
                tokio_tungstenite::tungstenite::Message,
                tokio_tungstenite::tungstenite::Error,
            >,
        > + Unpin,
{
    let deadline = tokio::time::Instant::now() + Duration::from_millis(ms);
    let mut n = 0;
    while let Ok(Some(Ok(msg))) = tokio::time::timeout_at(deadline, ws.next()).await {
        if msg
            .into_text()
            .is_ok_and(|t| t.contains("\"t\":\"accounts\""))
        {
            n += 1;
        }
    }
    n
}

/// Goal 7: with only the Default no `accounts` frame goes out, not even when
/// one of its sessions fails to authenticate; the one that says the other
/// accounts are gone does.
#[tokio::test]
async fn only_the_default_sends_no_accounts_frames() {
    use mother_claude_lib::claude::accounts::status;
    let f = boot().await;
    let desk = f.desktop();
    let url = format!("{}/ws?token={desk}", f.base.replace("http://", "ws://"));
    let mut ws = tokio_tungstenite::connect_async(url).await.unwrap().0;
    assert_eq!(accounts_frames(&mut ws, 300).await, 0, "none on connect");
    status::note_auth_failed(&f.st, "default").await;
    assert_eq!(
        accounts_frames(&mut ws, 500).await,
        0,
        "none for the Default"
    );

    let dir = f.scratch.path().join("claude-work");
    let (s, work) = f
        .call(
            "POST",
            "/accounts",
            &desk,
            Some(json!({"name": "Work", "kind": "directory", "dir": dir})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{work}");
    assert!(accounts_frames(&mut ws, 1500).await >= 1);
    let (s, _) = f
        .call(
            "DELETE",
            &format!("/accounts/{}", work["id"].as_str().unwrap()),
            &desk,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(
        accounts_frames(&mut ws, 1000).await,
        1,
        "the frame that says it is gone"
    );
    status::note_auth_ok(&f.st, "default").await;
    assert_eq!(accounts_frames(&mut ws, 500).await, 0, "then none again");
}

/// The account the running API starts its conversations under cannot be
/// removed (every new conversation would fail); once the API is stopped it
/// can, and the API's remembered choice goes with it.
#[tokio::test]
async fn the_apis_default_account_is_not_removed_under_it() {
    use mother_claude_lib::bridge::control::{self, BridgeDefaults};
    let f = boot().await;
    let desk = f.desktop();
    let work =
        f.st.accounts
            .add_directory("Work", &f.scratch.path().join("claude-work"), None)
            .unwrap();
    control::start(
        &f.st,
        BridgeDefaults {
            account: Some(work.id.clone()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let path = format!("/accounts/{}", work.id);
    let (s, v) = f.call("DELETE", &path, &desk, None).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "account_used_by_bridge");
    assert!(f.st.accounts.get(&work.id).is_some());

    control::stop(&f.st).await.unwrap();
    let (s, v) = f.call("DELETE", &path, &desk, None).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    assert!(f.st.bridge.defaults().await.account.is_none());
    control::start(&f.st, f.st.bridge.defaults().await)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_fresh_directory_account_signs_in_without_a_terminal() {
    let f = boot().await;
    let desk = f.desktop();
    let dir = f.scratch.path().join("claude-fresh");
    let (_, work) = f
        .call(
            "POST",
            "/accounts",
            &desk,
            Some(json!({"name": "Fresh", "kind": "directory", "dir": dir})),
        )
        .await;
    let id = work["id"].as_str().unwrap().to_string();
    let a = f
        .wait_for(&id, "the first check", |a| {
            a["checking"] == false && a.get("attention").is_some()
        })
        .await;
    assert_eq!(a["signedIn"], false);
    assert_eq!(a["attention"], "signedOut");

    let (s, v) = f
        .call(
            "POST",
            &format!("/accounts/{id}/login"),
            &desk,
            Some(json!({})),
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let a = f
        .wait_for(&id, "signed in", |a| a["signedIn"] == true)
        .await;
    assert_eq!(a["email"], "work@example.test");
    assert!(a.get("attention").is_none());
    // The fake signed in only the account's own directory.
    assert!(dir.join(".fake-signed-in").exists());

    // A token account has no `auth login`.
    let (_, t) = f
        .call(
            "POST",
            "/accounts",
            &desk,
            Some(json!({"name": "T", "kind": "token"})),
        )
        .await;
    let (s, v) = f
        .call(
            "POST",
            &format!("/accounts/{}/login", t["id"].as_str().unwrap()),
            &desk,
            Some(json!({})),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
}

/// An account whose session failed to authenticate signs in again as the
/// same email: `claude auth status` reads as before, so the login process
/// finishing is what clears the mark.
#[tokio::test]
async fn signing_in_again_as_the_same_email_clears_a_failure() {
    use mother_claude_lib::claude::accounts::status;
    let f = boot().await;
    let desk = f.desktop();
    let dir = f.scratch.path().join("claude-again");
    let (_, work) = f
        .call(
            "POST",
            "/accounts",
            &desk,
            Some(json!({"name": "Again", "kind": "directory", "dir": dir})),
        )
        .await;
    let id = work["id"].as_str().unwrap().to_string();
    let path = format!("/accounts/{id}/login");
    let login = || f.call("POST", &path, &desk, Some(json!({})));
    let (s, v) = login().await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    f.wait_for(&id, "signed in", |a| {
        a["signedIn"] == true && a["signingIn"] == false
    })
    .await;

    status::note_auth_failed(&f.st, &id).await;
    let a = f.account(&id).await;
    assert_eq!(a["attention"], "authFailed", "{a}");
    assert_eq!(a["signedIn"], false);

    let (s, v) = login().await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let a = f
        .wait_for(&id, "the failure cleared", |a| a.get("attention").is_none())
        .await;
    assert_eq!(a["signedIn"], true);
    assert_eq!(a["email"], "work@example.test", "the same email");
}

#[tokio::test]
async fn setup_token_is_captured_into_the_secret_store() {
    let f = boot().await;
    let desk = f.desktop();
    let (_, t) = f
        .call(
            "POST",
            "/accounts",
            &desk,
            Some(json!({"name": "Captured", "kind": "token"})),
        )
        .await;
    let id = t["id"].as_str().unwrap().to_string();
    assert_eq!(t["tokenStored"], false);
    assert_eq!(t["attention"], "tokenMissing");
    // Without a token the account cannot run anything.
    assert!(f.st.accounts.env(&id).is_err());

    let (s, v) = f
        .call(
            "POST",
            &format!("/accounts/{id}/token"),
            &desk,
            Some(json!({})),
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let a = f
        .wait_for(&id, "captured", |a| a["capture"]["state"] == "captured")
        .await;
    assert_eq!(a["tokenStored"], true);
    assert!(a.get("attention").is_none(), "{a}");
    // The desktop's own read has the sign-in URL; the token is nowhere.
    let (_, status) = f
        .call("GET", &format!("/accounts/{id}/token"), &desk, None)
        .await;
    assert_eq!(
        status["capture"]["url"],
        "https://claude.ai/oauth/authorize?code=true&client_id=fake&state=s1"
    );
    let token = format!("sk-ant-oat01-{}-fakeTokenTail0123456789", "Ab1_".repeat(20));
    assert!(!status.to_string().contains(&token));
    assert!(!a.to_string().contains(&token));
    // It is the token the screen showed.
    match f.st.accounts.env(&id).unwrap() {
        mother_claude_lib::claude::accounts::AccountEnv::Token(t) => assert_eq!(t.expose(), token),
        other => panic!("{other:?}"),
    }
    let account = f.st.accounts.get(&id).unwrap();
    assert_eq!(account.kind, AccountKind::Token);
    assert_eq!(
        account.token_sha256.as_deref(),
        Some(mother_claude_lib::claude::secrets::sha256_hex(token.as_bytes()).as_str())
    );

    // Pasting replaces it (checked with `auth status` first).
    let (s, v) = f
        .call(
            "POST",
            &format!("/accounts/{id}/token"),
            &desk,
            Some(json!({"token": TOKEN})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    match f.st.accounts.env(&id).unwrap() {
        mother_claude_lib::claude::accounts::AccountEnv::Token(t) => assert_eq!(t.expose(), TOKEN),
        other => panic!("{other:?}"),
    }
    // A code for a capture that is not running is refused.
    let (s, v) = f
        .call(
            "POST",
            &format!("/accounts/{id}/token"),
            &desk,
            Some(json!({"code": "abc#def"})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");

    // When the browser cannot reach the CLI, the screen asks for the code the
    // browser shows; the desktop types it in. (Only this test runs
    // `setup-token`, so switching the fake's mode here races nothing.)
    std::env::set_var("FAKE_SETUP_TOKEN", "code");
    let (s, _) = f
        .call(
            "POST",
            &format!("/accounts/{id}/token"),
            &desk,
            Some(json!({})),
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    f.wait_for(&id, "asks for the code", |a| {
        a["capture"]["asksForCode"] == true
    })
    .await;
    let (s, v) = f
        .call(
            "POST",
            &format!("/accounts/{id}/token"),
            &desk,
            Some(json!({"code": "abc#def"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    f.wait_for(&id, "captured after the code", |a| {
        a["capture"]["state"] == "captured"
    })
    .await;
    match f.st.accounts.env(&id).unwrap() {
        mother_claude_lib::claude::accounts::AccountEnv::Token(t) => assert_eq!(t.expose(), token),
        other => panic!("{other:?}"),
    }

    // A flow that ends without a token says why, and keeps the stored one.
    std::env::set_var("FAKE_SETUP_TOKEN", "fail");
    f.call(
        "POST",
        &format!("/accounts/{id}/token"),
        &desk,
        Some(json!({})),
    )
    .await;
    let a = f
        .wait_for(&id, "failed", |a| a["capture"]["state"] == "failed")
        .await;
    assert!(
        a["capture"]["error"]
            .as_str()
            .unwrap()
            .contains("the sign-in was cancelled"),
        "{a}"
    );
    assert_eq!(a["tokenStored"], true);

    // And one the user gives up on is stopped.
    std::env::set_var("FAKE_SETUP_TOKEN", "hang");
    f.call(
        "POST",
        &format!("/accounts/{id}/token"),
        &desk,
        Some(json!({})),
    )
    .await;
    f.wait_for(&id, "running", |a| a["capture"]["state"] == "running")
        .await;
    let (s, v) = f
        .call("DELETE", &format!("/accounts/{id}/token"), &desk, None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["capture"]["state"], "cancelled");
    std::env::remove_var("FAKE_SETUP_TOKEN");
}

fn transcript(cwd: &str, sid: &str) -> String {
    let user = json!({
        "type": "user", "sessionId": sid, "cwd": cwd, "uuid": "u-1",
        "timestamp": "2026-10-21T09:00:00.000Z",
        "message": {"role": "user", "content": "hello"}
    });
    let assistant = json!({
        "type": "assistant", "sessionId": sid, "cwd": cwd, "uuid": "a-1", "parentUuid": "u-1",
        "timestamp": "2026-10-21T09:00:05.000Z",
        "message": {"id": "msg_1", "role": "assistant", "model": "claude-haiku-4-5",
                    "content": [{"type": "text", "text": "hi"}],
                    "usage": {"input_tokens": 1, "output_tokens": 2}}
    });
    format!("{user}\n{assistant}\n")
}

fn write_session(home: &std::path::Path, cwd: &str, sid: &str, live_pid: Option<u32>) {
    let dir = home
        .join("projects")
        .join(mother_claude_lib::claude::encode_cwd(cwd));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{sid}.jsonl")), transcript(cwd, sid)).unwrap();
    if let Some(pid) = live_pid {
        std::fs::create_dir_all(home.join("sessions")).unwrap();
        std::fs::write(
            home.join("sessions").join(format!("{pid}.json")),
            json!({"pid": pid, "sessionId": sid, "cwd": cwd, "status": "idle",
                   "kind": "interactive", "entrypoint": "cli"})
            .to_string(),
        )
        .unwrap();
    }
}

/// P2.1/P2.2: sessions in every account's directory are listed, each with its
/// account; one in the Default directory that runs with a registered token
/// (its hook reported the hash) is that token account's; an unknown token is
/// "unregistered".
#[tokio::test]
async fn every_accounts_sessions_are_listed_with_their_account() {
    let f = boot().await;
    let default_home = f._home.path().to_path_buf();
    let cwd = f.scratch.path().join("proj");
    std::fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.to_string_lossy().into_owned();
    let (plain, tokened, stranger, worked) = (
        "11111111-1111-4111-8111-111111111111",
        "22222222-2222-4222-8222-222222222222",
        "33333333-3333-4333-8333-333333333333",
        "44444444-4444-4444-8444-444444444444",
    );
    write_session(&default_home, &cwd, plain, None);
    // Two live sessions of this very process (its pid is alive), in the
    // Default's directory, one per pidfile directory below.
    write_session(&default_home, &cwd, tokened, Some(std::process::id()));
    let work_dir = f.scratch.path().join("claude-work");
    let work =
        f.st.accounts
            .add_directory("Work", &work_dir, None)
            .unwrap();
    write_session(&work_dir, &cwd, worked, Some(std::process::id()));
    write_session(&default_home, &cwd, stranger, None);
    let pro =
        f.st.accounts
            .add_token(
                "Pro",
                Some(&mother_claude_lib::claude::secrets::Secret::new(TOKEN)),
                None,
            )
            .unwrap();
    f.st.activity.note_token_sha256(
        tokened,
        &mother_claude_lib::claude::secrets::sha256_hex(TOKEN.as_bytes()),
    );

    mother_claude_lib::server::monitor::refresh_once(&f.st).await;
    let account_of = |id: &str| {
        let st = f.st.clone();
        let id = id.to_string();
        async move {
            st.find_session(&id)
                .await
                .and_then(|s| s.account)
                .map(|a| a.id)
        }
    };
    assert_eq!(account_of(plain).await.as_deref(), Some("default"));
    assert_eq!(account_of(worked).await.as_deref(), Some(work.id.as_str()));
    assert_eq!(account_of(tokened).await.as_deref(), Some(pro.id.as_str()));
    // The live roster knows where each pidfile lives.
    assert_eq!(
        f.st.live.get(worked).unwrap().home_account.as_deref(),
        Some(work.id.as_str())
    );

    // A token no account holds.
    f.st.activity.note_token_sha256(tokened, &"0".repeat(64));
    mother_claude_lib::server::monitor::tick_once(&f.st).await;
    assert_eq!(account_of(tokened).await.as_deref(), Some("unregistered"));

    // The fleet header counts per account.
    let (_, fleet) = f.call("GET", "/agents", &f.desktop(), None).await;
    let per = &fleet["summary"]["accounts"];
    assert!(per.get(&work.id).is_some(), "{fleet}");

    // With only the Default again, no session names an account.
    f.st.accounts.remove(&work.id).unwrap();
    f.st.accounts.remove(&pro.id).unwrap();
    mother_claude_lib::server::monitor::refresh_once(&f.st).await;
    assert!(f.st.sessions().await.iter().all(|s| s.account.is_none()));
}

/// P3.2: moving a session between two directory accounts over HTTP — a dry
/// run, the job, its undo — and a phone may watch but not move.
#[tokio::test]
async fn sessions_move_between_accounts_with_an_undo() {
    let f = boot().await;
    let desk = f.desktop();
    let cwd = f.scratch.path().join("proj");
    std::fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.to_string_lossy().into_owned();
    let sid = "55555555-5555-4555-8555-555555555555";
    write_session(f._home.path(), &cwd, sid, None);
    let work_dir = f.scratch.path().join("claude-work");
    let work =
        f.st.accounts
            .add_directory("Work", &work_dir, None)
            .unwrap();
    let body = |dry: bool| {
        json!({"what": "session", "id": sid, "from": "default", "to": work.id,
               "mode": "move", "dryRun": dry})
    };

    let phone = f.phone();
    let (s, _) = f.call("POST", "/data/move", &phone, Some(body(true))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let (s, v) = f.call("POST", "/data/move", &desk, Some(body(true))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["plan"]["files"], 1);
    assert_eq!(v["plan"]["writesShared"], false);
    assert!(
        work_dir.join("projects").read_dir().is_err(),
        "a dry run writes nothing"
    );

    let (s, v) = f.call("POST", "/data/move", &desk, Some(body(false))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let job = v["job"]["id"].as_str().unwrap().to_string();
    let mut done = Value::Null;
    for _ in 0..200 {
        let (_, j) = f
            .call("GET", &format!("/data/jobs/{job}"), &phone, None)
            .await;
        if j["state"] != "running" {
            done = j;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(done["state"], "done", "{done}");
    assert!(done["undoUntil"].as_i64().is_some());
    assert_eq!(done["from"], "default", "{done}");
    assert_eq!(done["to"], json!(work.id));
    // A job of an earlier run is listed from its manifest, accounts and all.
    let undo_root = f.st.auth.config_dir.clone().unwrap().join("undo");
    let earlier = "20260101T000000-0000abcd";
    std::fs::create_dir_all(undo_root.join(earlier)).unwrap();
    let mut m: Value =
        serde_json::from_slice(&std::fs::read(undo_root.join(&job).join("manifest.json")).unwrap())
            .unwrap();
    m["jobId"] = json!(earlier);
    m["created"] = json!([]);
    m["moved"] = json!([]);
    std::fs::write(undo_root.join(earlier).join("manifest.json"), m.to_string()).unwrap();
    let (s, list) = f.call("GET", "/data/jobs", &phone, None).await;
    assert_eq!(s, StatusCode::OK);
    let jobs = list["jobs"].as_array().unwrap();
    assert_eq!(jobs[0]["id"], json!(job), "newest first: {list}");
    let old = jobs.iter().find(|j| j["id"] == earlier).expect("listed");
    assert_eq!(old["kind"], "move");
    assert_eq!(old["from"], "default");
    assert_eq!(old["to"], json!(work.id));
    std::fs::remove_dir_all(undo_root.join(earlier)).unwrap();
    let moved = work_dir
        .join("projects")
        .join(mother_claude_lib::claude::encode_cwd(&cwd))
        .join(format!("{sid}.jsonl"));
    assert!(moved.is_file());
    // The list shows it under Work now.
    let row = f.st.find_session(sid).await.unwrap();
    assert_eq!(row.account.unwrap().id, work.id);

    // Moving it again is refused: Work has it now.
    let (s, v) = f.call("POST", "/data/move", &desk, Some(body(true))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "already_exists");

    // A phone cannot undo; the desktop can.
    let (s, _) = f
        .call(
            "POST",
            &format!("/data/jobs/{job}/undo"),
            &phone,
            Some(json!({})),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, v) = f
        .call(
            "POST",
            &format!("/data/jobs/{job}/undo"),
            &desk,
            Some(json!({})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["state"], "undone");
    assert!(!moved.exists());
    let row = f.st.find_session(sid).await.unwrap();
    assert_eq!(row.account.unwrap().id, "default");
    let audit = f.st.devices.audit.recent(50);
    assert!(audit.iter().any(|e| e.action == "data.move"));
    assert!(audit.iter().any(|e| e.action == "data.undo"));

    // Same directory (a token account shares the Default's): nothing to do.
    let pro = f.st.accounts.add_token("Pro", None, None).unwrap();
    let (s, v) = f
        .call(
            "POST",
            "/data/move",
            &desk,
            Some(
                json!({"what": "session", "id": sid, "from": "default", "to": pro.id,
                        "mode": "copy", "dryRun": true}),
            ),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"]["code"], "same_directory");
}

/// The account a session was last seen running under counts only while its
/// transcript is still in that account's directory: one moved to Work since
/// is Work's (its chip, and what Continue resumes it with), while a token
/// account's session keeps its account in the Default's directory.
#[tokio::test]
async fn a_moved_session_belongs_to_the_account_that_has_it_now() {
    use mother_claude_lib::claude::encode_cwd;
    use mother_claude_lib::claude::summaries::session_account;
    use mother_claude_lib::server::monitor::refresh_once;
    let f = boot().await;
    let cwd = f.scratch.path().join("proj");
    std::fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.to_string_lossy().into_owned();
    let work_dir = f.scratch.path().join("claude-work");
    let work =
        f.st.accounts
            .add_directory("Work", &work_dir, None)
            .unwrap();
    let pro =
        f.st.accounts
            .add_token(
                "Pro",
                Some(&mother_claude_lib::claude::secrets::Secret::new(TOKEN)),
                None,
            )
            .unwrap();
    let (moved, tokened) = (
        "66666666-6666-4666-8666-666666666666",
        "77777777-7777-4777-8777-777777777777",
    );
    let pidfile = f
        ._home
        .path()
        .join("sessions")
        .join(format!("{}.json", std::process::id()));

    // Both ran in the Default's directory (this process's pid is alive).
    write_session(f._home.path(), &cwd, moved, Some(std::process::id()));
    refresh_once(&f.st).await;
    assert_eq!(session_account(&f.st, moved), "default");
    std::fs::remove_file(&pidfile).unwrap();
    write_session(f._home.path(), &cwd, tokened, Some(std::process::id()));
    f.st.activity.note_token_sha256(
        tokened,
        &mother_claude_lib::claude::secrets::sha256_hex(TOKEN.as_bytes()),
    );
    refresh_once(&f.st).await;
    assert_eq!(session_account(&f.st, tokened), pro.id);
    std::fs::remove_file(&pidfile).unwrap();

    // Both ended; the desktop moved the first one to Work.
    let to = work_dir.join("projects").join(encode_cwd(&cwd));
    std::fs::create_dir_all(&to).unwrap();
    std::fs::rename(
        f._home
            .path()
            .join("projects")
            .join(encode_cwd(&cwd))
            .join(format!("{moved}.jsonl")),
        to.join(format!("{moved}.jsonl")),
    )
    .unwrap();
    refresh_once(&f.st).await;
    assert!(f.st.live.get(moved).is_none() && f.st.live.get(tokened).is_none());
    assert_eq!(
        session_account(&f.st, moved),
        work.id,
        "it follows the move"
    );
    assert_eq!(
        f.st.find_session(moved).await.unwrap().account.unwrap().id,
        work.id
    );
    assert_eq!(
        session_account(&f.st, tokened),
        pro.id,
        "the Default's directory is the token account's too"
    );
}

/// P4.3/P4.2: a backup set over HTTP — folder, generated passphrase (shown
/// once), push, the index, a restore with its undo — and a phone only looks.
#[tokio::test]
async fn an_account_is_backed_up_and_restored_over_http() {
    let f = boot().await;
    let desk = f.desktop();
    let phone = f.phone();
    let cwd = f.scratch.path().join("proj");
    std::fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.to_string_lossy().into_owned();
    // A directory account: its .claude.json is its own (the Default's would
    // be the real ~/.claude.json).
    let work_dir = f.scratch.path().join("claude-work");
    let work =
        f.st.accounts
            .add_directory("Work", &work_dir, None)
            .unwrap();
    let sid = "66666666-6666-4666-8666-666666666666";
    write_session(&work_dir, &cwd, sid, None);
    let folder = f.scratch.path().join("iCloud/Mother Claude");

    let (s, v) = f
        .call(
            "PUT",
            "/backup/settings",
            &desk,
            Some(json!({"folder": folder, "everyHours": 6})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["passphraseSet"], false);
    let (s, v) = f
        .call(
            "POST",
            "/backup/push",
            &desk,
            Some(json!({"account": work.id})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "no passphrase yet: {v}");
    assert_eq!(v["error"]["code"], "no_backup_passphrase");

    let (s, v) = f
        .call(
            "POST",
            "/backup/passphrase",
            &desk,
            Some(json!({"generate": true})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let pass = v["passphrase"].as_str().unwrap().to_string();
    assert_eq!(pass.len(), 39, "eight groups of four: {pass}");
    let (_, v) = f.call("GET", "/backup", &phone, None).await;
    assert_eq!(v["passphraseSet"], true);
    assert!(!v.to_string().contains(&pass), "never shown again");

    let (s, _) = f
        .call("POST", "/backup/push", &phone, Some(json!({})))
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, v) = f
        .call(
            "POST",
            "/backup/push",
            &desk,
            Some(json!({"account": work.id})),
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let job = v["jobs"][0].as_str().unwrap().to_string();
    let wait = |job: String| {
        let f = &f;
        let desk = desk.clone();
        async move {
            for _ in 0..400 {
                let (_, j) = f
                    .call("GET", &format!("/data/jobs/{job}"), &desk, None)
                    .await;
                if j["state"] != "running" {
                    return j;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            panic!("job {job} did not finish");
        }
    };
    let done = wait(job).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["result"]["archived"][0], sid);
    // One archive, named after the session and what it holds.
    let archives: Vec<String> = std::fs::read_dir(folder.join("Work/sessions"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(archives.len(), 1, "{archives:?}");
    assert!(mother_claude_lib::claude::backup::is_archive_name(
        &archives[0],
        sid
    ));

    // The set compared with Work's directory.
    let (s, v) = f
        .call(
            "GET",
            &format!("/backup/index?account={}", work.id),
            &phone,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["sessions"][0]["action"], "same");

    // Gone locally (Claude Code pruned it): the index offers it, a restore
    // brings it back, its undo takes it away again.
    let transcript = work_dir
        .join("projects")
        .join(mother_claude_lib::claude::encode_cwd(&cwd))
        .join(format!("{sid}.jsonl"));
    let original = std::fs::read(&transcript).unwrap();
    std::fs::remove_file(&transcript).unwrap();
    let (_, v) = f
        .call(
            "GET",
            &format!("/backup/index?account={}", work.id),
            &desk,
            None,
        )
        .await;
    assert_eq!(v["sessions"][0]["action"], "new");
    let (s, v) = f
        .call(
            "POST",
            "/backup/restore",
            &desk,
            Some(json!({"account": work.id, "ids": [sid]})),
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let done = wait(v["job"].as_str().unwrap().to_string()).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["result"]["restored"][0], sid);
    assert_eq!(std::fs::read(&transcript).unwrap(), original);
    let (s, v) = f
        .call(
            "POST",
            &format!("/data/jobs/{}/undo", done["id"].as_str().unwrap()),
            &desk,
            Some(json!({})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(!transcript.exists());

    // A pull brings it back the same way (and only the desktop may pull).
    let (s, _) = f
        .call("POST", "/backup/pull", &phone, Some(json!({})))
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, v) = f
        .call(
            "POST",
            "/backup/pull",
            &desk,
            Some(json!({"account": work.id})),
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let done = wait(v["jobs"][0].as_str().unwrap().to_string()).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["result"]["restored"][0], sid, "{done}");
    assert_eq!(std::fs::read(&transcript).unwrap(), original);

    // The configuration bundle unpacks for review, never over the live one.
    std::fs::write(work_dir.join("settings.json"), "{\"model\":\"opus\"}").unwrap();
    let (_, v) = f
        .call(
            "POST",
            "/backup/push",
            &desk,
            Some(json!({"account": work.id})),
        )
        .await;
    wait(v["jobs"][0].as_str().unwrap().to_string()).await;
    let (s, v) = f
        .call(
            "POST",
            "/backup/restore",
            &desk,
            Some(json!({"account": work.id, "config": true})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let review = std::path::PathBuf::from(v["folder"].as_str().unwrap());
    assert_eq!(
        std::fs::read_to_string(review.join("settings.json")).unwrap(),
        "{\"model\":\"opus\"}"
    );

    // Nothing of the passphrase in the audit log.
    let audit = serde_json::to_string(&f.st.devices.audit.recent(100)).unwrap();
    assert!(!audit.contains(&pass));
    assert!(audit.contains("backup.passphrase"));
}

/// A token account uses the Default's directory, so it is backed up in the
/// Default's set (never a second copy of that directory); a set that is
/// being backed up already is never queued again; pulling everything names a
/// set that cannot be read and pulls the others.
#[tokio::test]
async fn sets_are_per_directory_and_a_set_is_never_queued_twice() {
    let f = boot_inside().await;
    let desk = f.desktop();
    let cwd = f.scratch.path().join("proj");
    std::fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.to_string_lossy().into_owned();
    let tok = f.st.accounts.add_token("Tok", None, None).unwrap();
    let work_dir = f.scratch.path().join("claude-work");
    let work =
        f.st.accounts
            .add_directory("Work", &work_dir, None)
            .unwrap();
    write_session(
        f._home.path(),
        &cwd,
        "77777777-7777-4777-8777-777777777777",
        None,
    );
    write_session(
        &work_dir,
        &cwd,
        "88888888-8888-4888-8888-888888888888",
        None,
    );
    let folder = f.scratch.path().join("iCloud/Mother Claude");
    let (s, v) = f
        .call(
            "PUT",
            "/backup/settings",
            &desk,
            Some(json!({ "folder": folder })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = f
        .call(
            "POST",
            "/backup/passphrase",
            &desk,
            Some(json!({"generate": true})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // Hold the data jobs' turn: the pushes queue.
    let held = f.st.jobs.lock.lock().await;
    let (s, v) = f.call("POST", "/backup/push", &desk, Some(json!({}))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let jobs: Vec<String> = v["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j.as_str().unwrap().to_string())
        .collect();
    assert_eq!(jobs.len(), 2, "the Default and Work, not Tok: {v}");
    let (_, again) = f.call("POST", "/backup/push", &desk, Some(json!({}))).await;
    assert_eq!(again["jobs"], v["jobs"], "queued already: the same jobs");
    let (_, one) = f
        .call(
            "POST",
            "/backup/push",
            &desk,
            Some(json!({ "account": tok.id })),
        )
        .await;
    assert!(jobs.contains(&one["jobs"][0].as_str().unwrap().to_string()));
    drop(held);
    for job in &jobs {
        for _ in 0..400 {
            let (_, j) = f
                .call("GET", &format!("/data/jobs/{job}"), &desk, None)
                .await;
            if j["state"] != "running" {
                assert_eq!(j["state"], "done", "{j}");
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    let mut sets: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    sets.sort();
    assert_eq!(sets, ["Default", "Work"]);
    let (_, v) = f.call("GET", "/backup", &desk, None).await;
    let tok_view = v["sets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["account"] == tok.id.as_str())
        .unwrap()
        .clone();
    assert_eq!(tok_view["sharedWith"], "default");
    assert!(tok_view["dir"].as_str().unwrap().ends_with("Default"));
    assert_eq!(tok_view["exists"], true);

    // Pull everything: a set that cannot be read is named, the rest pulled.
    let empty =
        f.st.accounts
            .add_directory("Empty", &f.scratch.path().join("claude-empty"), None)
            .unwrap();
    let (s, v) = f.call("POST", "/backup/pull", &desk, Some(json!({}))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    assert_eq!(v["jobs"].as_array().unwrap().len(), 2, "{v}");
    assert_eq!(v["skipped"][0]["account"], empty.id.as_str(), "{v}");
    assert_eq!(v["skipped"][0]["code"], "invalid_backup");
    assert!(v["skipped"][0]["message"].as_str().is_some());
    let _ = work;
}
