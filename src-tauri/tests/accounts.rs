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
    setup();
    let home = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let st = Inner::new(
        ClaudeHome::with_base(home.path()),
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        Auth::ephemeral(),
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
