//! Accounts over real HTTP (docs/ACCOUNTS.md P1.2, P1.3, §15.2–15.3): who may
//! list and who may change them, the registry's view (never a token), signing
//! accounts in through a pseudo-terminal (`claude auth login`, `claude
//! setup-token`), the browsers a link opens in, one sign-in at a time — all
//! against the FAKE `claude` (`tests/fixtures/fake-claude.py`), which only
//! ever writes into a test's temp directory. No browser ever opens: the
//! opener only records.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Once;
use std::time::Duration;

use futures_util::StreamExt;
use mother_claude_lib::claude::accounts::AccountKind;
use mother_claude_lib::claude::browser::{self, Browser, Installed, Link, Platform};
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

    /// Add a directory account (its directory in the scratch dir) and wait
    /// for its first sign-in check.
    async fn directory_account(&self, name: &str) -> (String, PathBuf) {
        let dir = self.scratch.path().join(format!("claude-{name}"));
        let (s, a) = self
            .call(
                "POST",
                "/accounts",
                &self.desktop(),
                Some(json!({"name": name, "kind": "directory", "dir": dir})),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{a}");
        let id = a["id"].as_str().unwrap().to_string();
        self.wait_for(&id, "the first check", |a| {
            a["checking"] == false && a.get("attention").is_some()
        })
        .await;
        // The directory as the registry keeps it (symlinks resolved).
        let dir = self.st.accounts.get(&id).unwrap().dir.unwrap();
        (id, dir)
    }

    /// Start `id`'s login (`body`), expecting it to be accepted.
    async fn login(&self, id: &str, body: Value) -> Value {
        let (s, v) = self
            .call(
                "POST",
                &format!("/accounts/{id}/login"),
                &self.desktop(),
                Some(body),
            )
            .await;
        assert_eq!(s, StatusCode::ACCEPTED, "{v}");
        v
    }

    /// The account's sign-in once `ok` holds for it.
    async fn sign_in_when(&self, id: &str, what: &str, ok: impl Fn(&Value) -> bool) -> Value {
        self.wait_for(id, what, |a| ok(&a["signIn"])).await["signIn"].clone()
    }
}

/// Script a directory account's next `claude auth login` (the fake reads it
/// from the account's own temp directory).
fn script_login(dir: &std::path::Path, script: Value) {
    std::fs::write(dir.join(".fake-login.json"), script.to_string()).unwrap();
}

/// The facts the fake put in a sign-in's link (its query).
fn link_facts(url: &Value) -> HashMap<String, String> {
    let url = reqwest::Url::parse(url.as_str().expect("a link")).unwrap();
    url.query_pairs().into_owned().collect()
}

/// The command this machine would run to open `url` in `browser`.
fn open_command(browser: Browser, installed: &Installed, url: &Value) -> browser::Command {
    let root = std::env::var_os("SystemRoot").map(PathBuf::from);
    browser::command(
        Platform::current(),
        browser,
        installed,
        &Link::parse(url.as_str().unwrap()).unwrap(),
        root.as_deref(),
    )
    .unwrap()
}

const TOKEN: &str = "sk-ant-oat01-PastedTokenValue0123456789abcdefghijABCDEFGHIJ";

/// A phone lists accounts; adding, removing, signing in and tokens stay on
/// the desktop (§13.9; what a phone may change is in
/// `a_phone_manages_data_but_never_secrets`).
#[tokio::test]
async fn a_phone_lists_accounts_but_cannot_add_or_sign_them_in() {
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
        ("DELETE", "/accounts/acc_000000000000", json!({})),
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

/// §15.2: a directory account's `claude auth login` runs in a pseudo-terminal
/// with its own `CLAUDE_CONFIG_DIR` and `BROWSER=true`; the link it shows
/// opens in the default browser on its own; the run ends signed in.
#[tokio::test]
async fn a_directory_accounts_login_runs_in_a_terminal_and_opens_the_browser() {
    let f = boot().await;
    let desk = f.desktop();
    let (id, dir) = f.directory_account("Fresh").await;
    let a = f.account(&id).await;
    assert_eq!(a["signedIn"], false);
    assert_eq!(a["attention"], "signedOut");

    let v = f.login(&id, json!({})).await;
    assert_eq!(v["signIn"]["kind"], "login");
    assert_eq!(v["signingIn"], true);
    let a = f
        .wait_for(&id, "signed in", |a| a["signedIn"] == true)
        .await;
    assert_eq!(a["email"], "work@example.test");
    assert!(a.get("attention").is_none());
    let run = f.sign_in_when(&id, "done", |s| s["state"] == "done").await;
    assert_eq!(run["opened"], "default", "{run}");
    assert!(run["finishedAt"].as_i64().is_some());
    assert_eq!(f.account(&id).await["signingIn"], false);
    // The fake signed in only the account's own directory, in a terminal,
    // with BROWSER=true, no token, and nothing to pre-fill the first time.
    assert!(dir.join(".fake-signed-in").exists());
    let facts = link_facts(&run["url"]);
    assert_eq!(facts["fake_config_dir"], dir.to_string_lossy());
    assert_eq!(facts["fake_browser"], "true");
    assert_eq!(facts["fake_tty"], "1");
    assert_eq!(facts["fake_token"], "0");
    assert_eq!(facts["fake_email"], "");
    assert_eq!(facts["fake_console"], "0");
    // Mother Claude opened it, in the default browser, once.
    assert_eq!(
        f.st.opener.recorded(),
        [open_command(
            Browser::Default,
            &Installed::default(),
            &run["url"]
        )]
    );
    // A phone sees the run, never its link.
    let (_, all) = f.call("GET", "/accounts", &f.phone(), None).await;
    let phone_view = all["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == id.as_str())
        .unwrap()
        .clone();
    assert_eq!(phone_view["signIn"]["state"], "done");
    assert!(phone_view["signIn"].get("url").is_none(), "{phone_view}");
    let (s, one) = f
        .call("GET", &format!("/accounts/{id}"), &f.phone(), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{one}");
    assert!(one["signIn"].get("url").is_none());
    assert_eq!(one["live"], 0);
    let (s, one) = f.call("GET", &format!("/accounts/{id}"), &desk, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(one["signIn"]["url"], run["url"]);
    let (s, _) = f
        .call("GET", "/accounts/acc_000000000000", &desk, None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

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
    // A re-sign-in of an account whose email is known asks for that account.
    let run = f.sign_in_when(&id, "done", |s| s["state"] == "done").await;
    assert_eq!(link_facts(&run["url"])["fake_email"], "work@example.test");
}

/// §15.2: the Default's login keeps the environment as the app found it — its
/// `CLAUDE_CONFIG_DIR` is never set by us — and still gets `BROWSER=true`;
/// `--console` and an email the desktop names reach the CLI.
#[tokio::test]
async fn the_defaults_login_never_sets_its_directory() {
    let f = boot().await;
    let v = f
        .login(
            "default",
            json!({"console": true, "email": " me@example.test "}),
        )
        .await;
    assert_eq!(v["signIn"]["kind"], "login");
    let run = f
        .sign_in_when("default", "finished", |s| {
            s["state"] != "starting" && s["state"] != "open"
        })
        .await;
    let facts = link_facts(&run["url"]);
    let inherited = std::env::var("CLAUDE_CONFIG_DIR").unwrap_or_default();
    assert_eq!(
        facts["fake_config_dir"], inherited,
        "left as the app found it"
    );
    assert_eq!(facts["fake_browser"], "true");
    assert_eq!(facts["fake_tty"], "1");
    assert_eq!(facts["fake_console"], "1");
    assert_eq!(facts["fake_email"], "me@example.test");
    assert!(
        run["url"]
            .as_str()
            .unwrap()
            .starts_with("https://platform.claude.com/oauth/authorize?"),
        "{run}"
    );
    if inherited.is_empty() {
        // The fake's Default is signed in as default@example.test.
        assert_eq!(run["state"], "done", "{run}");
    }
    // An email that is not one is refused before anything runs.
    let (s, v) = f
        .call(
            "POST",
            "/accounts/default/login",
            &f.desktop(),
            Some(json!({"email": "--sso"})),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"]["code"], "invalid_body");
}

/// §15.3: one sign-in at a time across all accounts; an account's own running
/// sign-in is handed back; cancelling frees the slot.
#[tokio::test]
async fn one_sign_in_runs_at_a_time() {
    let f = boot().await;
    let desk = f.desktop();
    let (a, a_dir) = f.directory_account("Alpha").await;
    let (b, _) = f.directory_account("Beta").await;
    let (_, t) = f
        .call(
            "POST",
            "/accounts",
            &desk,
            Some(json!({"name": "Tok", "kind": "token"})),
        )
        .await;
    let t = t["id"].as_str().unwrap().to_string();
    script_login(&a_dir, json!({"mode": "hang"}));
    f.login(&a, json!({})).await;
    f.sign_in_when(&a, "open", |s| s["state"] == "open").await;

    for (path, body) in [
        (format!("/accounts/{b}/login"), json!({})),
        ("/accounts/default/login".to_string(), json!({})),
        (format!("/accounts/{t}/token"), json!({})),
    ] {
        let (s, v) = f.call("POST", &path, &desk, Some(body)).await;
        assert_eq!(s, StatusCode::CONFLICT, "{path}: {v}");
        assert_eq!(v["error"]["code"], "sign_in_running", "{path}");
        assert_eq!(v["error"]["account"], a.as_str(), "{path}");
        assert_eq!(v["error"]["accountName"], "Alpha", "{path}");
    }
    assert!(f.account(&b).await.get("signIn").is_none(), "nothing ran");
    // Alpha's own: the running one again.
    let started = f.account(&a).await["signIn"]["startedAt"].clone();
    let v = f.login(&a, json!({})).await;
    assert_eq!(v["signIn"]["startedAt"], started);
    assert_eq!(v["signIn"]["state"], "open");

    // Nothing to cancel on Beta; then Alpha's is cancelled.
    let (s, v) = f
        .call("DELETE", &format!("/accounts/{b}/sign-in"), &desk, None)
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "sign_in_not_running");
    let (s, v) = f
        .call("DELETE", &format!("/accounts/{a}/sign-in"), &desk, None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["signIn"]["state"], "cancelled");
    assert!(f
        .st
        .devices
        .audit
        .recent(50)
        .iter()
        .any(|e| e.action == "account.sign_in.cancel"));

    // Now Beta signs in.
    f.login(&b, json!({})).await;
    f.sign_in_when(&b, "done", |s| s["state"] == "done").await;
    assert_eq!(f.account(&a).await["signIn"]["state"], "cancelled");
}

/// §15.2: the link opens in a browser this machine has, only while the run
/// runs and only once the link is known; a link to another host is never
/// shown or opened; a phone may do none of it.
#[tokio::test]
async fn a_sign_ins_link_opens_only_where_it_may() {
    let f = boot().await;
    let desk = f.desktop();
    let chrome = Installed {
        chrome: Some(PathBuf::from("/Applications/Google Chrome.app")),
    };
    f.st.opener.pretend_installed(chrome.clone());
    let (id, dir) = f.directory_account("Opener").await;

    let (s, v) = f
        .call("GET", &format!("/accounts/{id}/sign-in/open"), &desk, None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        v["browsers"],
        json!(["default", "chrome", "chrome-private"])
    );

    let open_path = format!("/accounts/{id}/sign-in/open");
    let open = |browser: &'static str| {
        f.call(
            "POST",
            &open_path,
            &desk,
            Some(json!({ "browser": browser })),
        )
    };
    // Nothing runs yet.
    let (s, v) = open("default").await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "sign_in_not_running");

    // A screen whose link points elsewhere: never shown, never opened.
    script_login(&dir, json!({"mode": "elsewhere"}));
    f.login(&id, json!({})).await;
    let mut refused = (StatusCode::OK, Value::Null);
    for _ in 0..100 {
        refused = open("default").await;
        if refused.1["error"]["code"] != "sign_in_no_link" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(refused.0, StatusCode::UNPROCESSABLE_ENTITY, "{}", refused.1);
    assert_eq!(refused.1["error"]["code"], "invalid_sign_in_url");
    let run = f.account(&id).await["signIn"].clone();
    assert_eq!(run["state"], "starting", "{run}");
    assert!(run.get("url").is_none());
    assert!(f.st.opener.recorded().is_empty(), "nothing was opened");
    let (s, v) = f
        .call("DELETE", &format!("/accounts/{id}/sign-in"), &desk, None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["signIn"]["state"], "cancelled");

    // A good one: the default browser on its own, then a private window.
    script_login(&dir, json!({"mode": "hang"}));
    f.login(&id, json!({})).await;
    let run = f
        .sign_in_when(&id, "open", |s| {
            s["state"] == "open" && s["opened"] == "default"
        })
        .await;
    let (s, v) = open("chrome-private").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["signIn"]["opened"], "chrome-private");
    assert_eq!(
        f.st.opener.recorded(),
        [
            open_command(Browser::Default, &chrome, &run["url"]),
            open_command(Browser::ChromePrivate, &chrome, &run["url"]),
        ]
    );
    // A browser this machine lacks, or none at all.
    f.st.opener.pretend_installed(Installed::default());
    let (s, v) = open("chrome").await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"]["code"], "browser_unavailable");
    let (s, v) = open("firefox").await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"]["code"], "invalid_body");
    // A phone may not open, type, cancel or even list.
    let phone = f.phone();
    for (method, path, body) in [
        ("GET", format!("/accounts/{id}/sign-in/open"), None),
        (
            "POST",
            format!("/accounts/{id}/sign-in/open"),
            Some(json!({"browser": "default"})),
        ),
        (
            "POST",
            format!("/accounts/{id}/sign-in/code"),
            Some(json!({"code": "x"})),
        ),
        ("DELETE", format!("/accounts/{id}/sign-in"), None),
    ] {
        let (s, v) = f.call(method, &path, &phone, body).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {path}: {v}");
    }
    assert_eq!(f.st.opener.recorded().len(), 2);
    let audit = f.st.devices.audit.recent(50);
    assert!(audit.iter().any(|e| e.action == "account.sign_in.open"));
    let audit_text = serde_json::to_string(&audit).unwrap();
    assert!(!audit_text.contains("oauth"), "no link in the audit");
    f.call("DELETE", &format!("/accounts/{id}/sign-in"), &desk, None)
        .await;
}

/// §15.2: when the browser cannot hand the sign-in back, the screen asks for
/// the code it shows; the desktop types it in.
#[tokio::test]
async fn a_login_takes_the_code_the_browser_shows() {
    let f = boot().await;
    let desk = f.desktop();
    let (id, dir) = f.directory_account("Coded").await;
    script_login(&dir, json!({"mode": "code", "code": "abc#def"}));
    f.login(&id, json!({})).await;
    f.sign_in_when(&id, "asks for the code", |s| s["state"] == "code")
        .await;
    let (s, v) = f
        .call(
            "POST",
            &format!("/accounts/{id}/sign-in/code"),
            &desk,
            Some(json!({"code": " abc#def\n"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_ne!(v["signIn"]["state"], "code", "typed: {v}");
    f.sign_in_when(&id, "done", |s| s["state"] == "done").await;
    assert_eq!(f.account(&id).await["signedIn"], true);
    // A finished run takes no code.
    let (s, v) = f
        .call(
            "POST",
            &format!("/accounts/{id}/sign-in/code"),
            &desk,
            Some(json!({"code": "abc#def"})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "sign_in_not_running");

    // A wrong code ends it, saying why (never the link).
    script_login(&dir, json!({"mode": "code", "code": "right"}));
    f.login(&id, json!({})).await;
    f.sign_in_when(&id, "asks again", |s| s["state"] == "code")
        .await;
    f.call(
        "POST",
        &format!("/accounts/{id}/sign-in/code"),
        &desk,
        Some(json!({"code": "wrong"})),
    )
    .await;
    let run = f
        .sign_in_when(&id, "failed", |s| s["state"] == "failed")
        .await;
    let error = run["error"].as_str().unwrap();
    assert!(error.contains("Invalid code"), "{error}");
    assert!(!error.contains("https://"), "{error}");
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
    assert_eq!(a["signIn"]["kind"], "token");
    assert_eq!(a["signIn"]["state"], "done");
    assert_eq!(a["signingIn"], false, "a login's flag only");
    // The desktop's own read has the sign-in URL; the token is nowhere. It
    // ran in a terminal with BROWSER=true and without the inherited token,
    // and nothing opened on its own (§15.2: the desktop asks first).
    let (_, status) = f
        .call("GET", &format!("/accounts/{id}/token"), &desk, None)
        .await;
    let url = status["capture"]["url"].as_str().unwrap();
    assert!(
        url.starts_with("https://claude.ai/oauth/authorize?code=true&client_id=fake&state=s1&"),
        "{url}"
    );
    assert_eq!(status["signIn"]["url"], url);
    let facts = link_facts(&status["capture"]["url"]);
    assert_eq!(facts["fake_browser"], "true");
    assert_eq!(facts["fake_tty"], "1");
    assert_eq!(facts["fake_token"], "0");
    assert!(
        f.st.opener.recorded().is_empty(),
        "nothing opened by itself"
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
    assert_eq!(v["signIn"]["state"], "cancelled");
    std::env::remove_var("FAKE_SETUP_TOKEN");

    // §15.3: a run stores into the account that started it, and only there.
    // A second account whose browser handed over the first one's token (the
    // fake shows the same one) is refused, and the first keeps its own.
    let (_, other) = f
        .call(
            "POST",
            "/accounts",
            &desk,
            Some(json!({"name": "Second", "kind": "token"})),
        )
        .await;
    let other = other["id"].as_str().unwrap().to_string();
    let (s, v) = f
        .call(
            "POST",
            &format!("/accounts/{other}/token"),
            &desk,
            Some(json!({})),
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    assert_eq!(v["signIn"]["kind"], "token");
    let a = f
        .wait_for(&other, "refused", |a| a["capture"]["state"] == "failed")
        .await;
    assert!(
        a["capture"]["error"]
            .as_str()
            .unwrap()
            .contains("already account \"Captured\""),
        "{a}"
    );
    assert_eq!(a["tokenStored"], false);
    assert!(f.st.accounts.env(&other).is_err(), "nothing stored there");
    match f.st.accounts.env(&id).unwrap() {
        mother_claude_lib::claude::accounts::AccountEnv::Token(t) => assert_eq!(t.expose(), token),
        other => panic!("{other:?}"),
    }
    assert!(f.st.opener.recorded().is_empty());
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
/// run, the job, its undo — and a local script may watch but not move.
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
    let script = f.st.auth.script_token.clone();
    let (s, v) = f
        .call("POST", "/data/move", &script, Some(body(true)))
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(v["error"]["code"], "desktop_only");

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

    // A local script cannot undo; the desktop can.
    let (s, _) = f
        .call(
            "POST",
            &format!("/data/jobs/{job}/undo"),
            &script,
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
/// once), push, the index, a restore with its undo — and a local script only
/// looks.
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
    assert!(v.get("machine").is_none(), "no id is made by looking: {v}");

    let script = f.st.auth.script_token.clone();
    let (s, _) = f
        .call("POST", "/backup/push", &script, Some(json!({})))
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
    assert_eq!(done["account"], json!(work.id), "the job names its set");
    // Now this Mac has an id, and the set lists it under that id.
    let (_, v) = f.call("GET", "/backup", &phone, None).await;
    let me = v["machine"]
        .as_str()
        .expect("this machine's id")
        .to_string();
    let work_set = v["sets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["account"] == json!(work.id))
        .unwrap()
        .clone();
    assert!(
        work_set["machines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"] == json!(me)),
        "{work_set}"
    );
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
    assert_eq!(done["account"], json!(work.id));
    assert_eq!(done["to"], json!(work.id), "restored into its own account");
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

    // A pull brings it back the same way (and a local script may not pull).
    let (s, _) = f
        .call("POST", "/backup/pull", &script, Some(json!({})))
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
    assert_eq!(done["account"], json!(work.id));
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

/// A data job's view once it stopped running.
async fn finished_job(f: &Fixture, token: &str, job: &str) -> Value {
    for _ in 0..400 {
        let (_, j) = f
            .call("GET", &format!("/data/jobs/{job}"), token, None)
            .await;
        if j["state"] != "running" {
            return j;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("job {job} did not finish");
}

type Bus = tokio::sync::broadcast::Receiver<std::sync::Arc<mother_claude_lib::state::ServerEvent>>;

/// The desktop notices sent on the bus since `rx` subscribed.
fn desktop_notices(rx: &mut Bus) -> Vec<String> {
    use mother_claude_lib::state::ServerEvent;
    use tokio::sync::broadcast::error::TryRecvError;
    let mut out = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(ev) => {
                if let ServerEvent::DesktopNotice { text, .. } = &*ev {
                    out.push(text.clone());
                }
            }
            Err(TryRecvError::Lagged(_)) => continue,
            Err(_) => return out,
        }
    }
}

/// §13.9: a paired phone (on the LAN or through a tunnel) moves, copies,
/// undoes, backs up, restores, pulls and changes the account defaults; a
/// folder's trust needs the dangerous right; the passphrase stays the
/// desktop's; the local-script token gets none of it; the desktop is told
/// what the phone did, by the phone's name, and the audit names the device.
#[tokio::test]
async fn a_phone_manages_data_but_never_secrets() {
    let f = boot_inside().await;
    let desk = f.desktop();
    let (device, phone) = f.st.devices.issue(
        "Kitchen phone",
        "ios",
        None,
        Ingress::Lan,
        "192.168.1.61".parse().unwrap(),
    );
    let script = f.st.auth.script_token.clone();
    let cwd = f.scratch.path().join("proj");
    std::fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.to_string_lossy().into_owned();
    let work_dir = f.scratch.path().join("claude-work");
    let work =
        f.st.accounts
            .add_directory("Work", &work_dir, None)
            .unwrap();
    let sid = "99999999-9999-4999-8999-999999999999";
    write_session(&work_dir, &cwd, sid, None);
    // The folder is trusted in Work: `trust` would carry that.
    std::fs::write(
        work_dir.join(".claude.json"),
        json!({"projects": {&cwd: {"hasTrustDialogAccepted": true}}}).to_string(),
    )
    .unwrap();
    let mut bus = f.st.bus.subscribe();

    // The account settings a phone may change.
    let (s, v) = f
        .call(
            "PUT",
            "/accounts/defaults",
            &phone,
            Some(json!({"newSession": work.id, "bridge": "default"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = f
        .call(
            "PATCH",
            &format!("/accounts/{}", work.id),
            &phone,
            Some(json!({"color": "#123456"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = f
        .call(
            "PUT",
            "/accounts/copy-consent",
            &phone,
            Some(json!({"from": "default", "to": work.id, "granted": true})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(f.st.accounts.has_copy_consent("default", &work.id));
    // The passphrase is the desktop's.
    let (s, v) = f
        .call(
            "POST",
            "/backup/passphrase",
            &phone,
            Some(json!({"generate": true})),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"]["code"], "desktop_only");

    // A local script: none of the data routes.
    let copy = |trust: bool, dry: bool| {
        json!({"what": "session", "id": sid, "from": work.id, "to": "default",
               "mode": "copy", "trust": trust, "dryRun": dry})
    };
    let work_path = format!("/accounts/{}", work.id);
    for (method, path, body) in [
        (
            "PUT",
            "/accounts/defaults",
            json!({"newSession": "default", "bridge": "default"}),
        ),
        ("PATCH", work_path.as_str(), json!({"color": "#654321"})),
        ("POST", "/data/move", copy(false, true)),
        ("POST", "/backup/push", json!({})),
        ("PUT", "/backup/settings", json!({"everyHours": 2})),
        ("POST", "/backup/pull", json!({})),
    ] {
        let (s, v) = f.call(method, path, &script, Some(body)).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {path}: {v}");
        assert_eq!(v["error"]["code"], "desktop_only", "{method} {path}");
    }

    // Trust needs the dangerous right, even in a dry run; the rest does not.
    let (s, v) = f
        .call("POST", "/data/move", &phone, Some(copy(true, true)))
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"]["code"], "dangerous_blocked");
    let (s, v) = f
        .call("POST", "/data/move", &phone, Some(copy(false, false)))
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let job = v["job"]["id"].as_str().unwrap().to_string();
    let done = finished_job(&f, &phone, &job).await;
    assert_eq!(done["state"], "done", "{done}");
    let (s, v) = f
        .call(
            "POST",
            &format!("/data/jobs/{job}/undo"),
            &phone,
            Some(json!({})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["state"], "undone");

    // Backups from the phone: settings, push, restore and pull.
    let folder = f.scratch.path().join("iCloud/Mother Claude");
    let (s, v) = f
        .call(
            "PUT",
            "/backup/settings",
            &phone,
            Some(json!({ "folder": folder })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, _) = f
        .call(
            "POST",
            "/backup/passphrase",
            &desk,
            Some(json!({"generate": true})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = f
        .call(
            "POST",
            "/backup/push",
            &phone,
            Some(json!({"account": work.id})),
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let pushed = finished_job(&f, &phone, v["jobs"][0].as_str().unwrap()).await;
    assert_eq!(pushed["state"], "done", "{pushed}");
    for (path, body) in [
        (
            "/backup/restore",
            json!({"account": work.id, "into": "default", "ids": [sid], "trust": true}),
        ),
        ("/backup/pull", json!({"account": work.id, "trust": true})),
    ] {
        let (s, v) = f.call("POST", path, &phone, Some(body)).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{path}: {v}");
        assert_eq!(v["error"]["code"], "dangerous_blocked", "{path}");
    }
    let (s, v) = f
        .call(
            "POST",
            "/backup/restore",
            &phone,
            Some(json!({"account": work.id, "into": "default", "ids": [sid]})),
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let restored = finished_job(&f, &phone, v["job"].as_str().unwrap()).await;
    assert_eq!(restored["result"]["restored"][0], sid, "{restored}");

    // With the grant the trust goes through.
    f.st.devices
        .set_grants(&device.id, Some(true), Some(true))
        .unwrap();
    let (s, v) = f
        .call("POST", "/data/move", &phone, Some(copy(true, true)))
        .await;
    // Past the gate (the restore above put the conversation there already).
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "already_exists");
    let (s, v) = f
        .call(
            "POST",
            "/backup/pull",
            &phone,
            Some(json!({"account": work.id, "trust": true})),
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    finished_job(&f, &phone, v["jobs"][0].as_str().unwrap()).await;

    // The desktop was told, by the phone's name; the audit names the device.
    let notices = desktop_notices(&mut bus);
    for what in [
        "changed the account defaults",
        "undid a data job",
        "started a backup",
        "started to restore 1 conversation(s)",
    ] {
        assert!(
            notices
                .iter()
                .any(|n| n.contains("\"Kitchen phone\"") && n.contains(what)),
            "{what}: {notices:?}"
        );
    }
    let audit = f.st.devices.audit.recent(200);
    for action in ["data.copy", "data.undo", "backup.push", "backup.restore"] {
        assert!(
            audit
                .iter()
                .any(|e| e.action == action && e.device_id.as_deref() == Some(device.id.as_str())),
            "{action}"
        );
    }
}

/// §13.3 rule 8 and §13.4 over HTTP: a storage is added (its folder made
/// when only it is missing), listed after the backup folder, renamed and
/// forgotten (its files stay); one inside an account's directory, the backup
/// folder or another storage is refused; a passphrase change re-keys a set in
/// every storage and refuses while one is unplugged; a set of any storage is
/// listed by `?storage=&set=`; a phone may do all of it but the passphrase;
/// and with only the Default and only the backup folder, `GET /api/backup`
/// is what it was (goal 7).
#[tokio::test]
async fn storages_are_added_listed_and_re_keyed() {
    use mother_claude_lib::claude::backup::{self as b, PushInputs, Set, ThisMachine};
    use mother_claude_lib::claude::secrets::Secret;
    let f = boot_inside().await;
    let desk = f.desktop();
    let phone = f.phone();
    let folder = f.scratch.path().join("iCloud/Mother Claude");
    let (s, _) = f
        .call(
            "PUT",
            "/backup/settings",
            &desk,
            Some(json!({ "folder": folder })),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, v) = f
        .call(
            "POST",
            "/backup/passphrase",
            &desk,
            Some(json!({"generate": true})),
        )
        .await;
    let pass = v["passphrase"].as_str().unwrap().to_string();

    // Goal 7: nothing of storages while none is in use.
    let (_, v) = f.call("GET", "/backup", &phone, None).await;
    assert!(v.get("storage").is_none(), "{v}");
    assert!(v["settings"].get("storages").is_none(), "{v}");

    // Refused: inside an account's directory, the backup folder, nowhere.
    std::fs::create_dir_all(f.scratch.path().join("Archive disk")).unwrap();
    let disk = f.scratch.path().join("Archive disk/Claude");
    for path in [
        f._home.path().join("archive"),
        folder.join("inner"),
        f.scratch.path().join("Unplugged/Claude"),
        std::path::PathBuf::from("relative/path"),
    ] {
        let (s, v) = f
            .call(
                "POST",
                "/backup/storage",
                &phone,
                Some(json!({ "path": path })),
            )
            .await;
        assert_eq!(
            s,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}: {v}",
            path.display()
        );
        assert_eq!(v["error"]["code"], "invalid_storage", "{}", path.display());
    }
    let (s, added) = f
        .call(
            "POST",
            "/backup/storage",
            &phone,
            Some(json!({"path": disk, "label": "Archive disk"})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{added}");
    let id = added["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("stor_"), "{added}");
    assert_eq!(added["primary"], false);
    assert_eq!(added["available"], true);
    assert_eq!(added["kind"], "folder");
    assert!(disk.is_dir(), "its folder is made");
    let (s, v) = f
        .call(
            "POST",
            "/backup/storage",
            &phone,
            Some(json!({"path": disk.join("inner")})),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    let (s, v) = f
        .call(
            "PUT",
            "/backup/settings",
            &desk,
            Some(json!({ "folder": disk.join("x") })),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"]["code"], "invalid_folder");

    // A set in it (Move data makes them; here the engine does).
    let cwd = f.scratch.path().join("proj").to_string_lossy().into_owned();
    let sid = "abababab-abab-4bab-8bab-abababababab";
    write_session(f._home.path(), &cwd, sid, None);
    let set = Set::new(&disk, "Archive");
    b::push(&PushInputs {
        home: &f.st.home,
        set: &set,
        account_name: "Archive",
        passphrases: &[Secret::new(pass.clone())],
        machine: &ThisMachine {
            id: "0".repeat(32),
            name: "test".into(),
        },
        is_live: &|_: &str| false,
        title_of: &|_: &str| None,
        progress: &Default::default(),
    })
    .unwrap();
    let (_, v) = f.call("GET", "/backup", &phone, None).await;
    let storage = v["storage"].as_array().expect("listed now");
    assert_eq!(storage[0]["id"], "backup");
    assert_eq!(storage[0]["primary"], true);
    assert_eq!(storage[1]["id"], json!(id));
    assert_eq!(storage[1]["label"], "Archive disk");
    assert_eq!(storage[1]["sets"][0]["name"], "Archive");
    assert_eq!(storage[1]["sets"][0]["exists"], true);
    assert_eq!(storage[1]["sets"][0]["sessions"], 1);
    assert_eq!(v["settings"]["storages"][0]["id"], json!(id));
    let (s, v) = f
        .call(
            "GET",
            &format!("/backup/index?storage={id}&set=Archive"),
            &phone,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["storage"], json!(id));
    assert_eq!(
        v["account"], "default",
        "compared with the Default's directory"
    );
    assert_eq!(v["sessions"][0]["id"], sid);
    assert_eq!(v["sessions"][0]["action"], "same");
    let (s, v) = f
        .call(
            "GET",
            &format!("/backup/index?storage={id}&set=..%2F..%2Fetc"),
            &phone,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");

    // Renamed; the backup folder's label is fixed.
    let (s, v) = f
        .call(
            "PATCH",
            &format!("/backup/storage/{id}"),
            &phone,
            Some(json!({"label": "Old disk"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["label"], "Old disk");
    let (s, v) = f
        .call(
            "PATCH",
            "/backup/storage/backup",
            &phone,
            Some(json!({"label": "Mine"})),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");

    // A new passphrase re-keys the storage's set too; never while a storage
    // is unplugged.
    let away = f.scratch.path().join("Archive disk/Claude (away)");
    std::fs::rename(&disk, &away).unwrap();
    let (s, v) = f
        .call(
            "POST",
            "/backup/passphrase",
            &desk,
            Some(json!({"generate": true})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "storage_unavailable");
    let (_, v) = f.call("GET", "/backup", &phone, None).await;
    assert_eq!(v["storage"][1]["available"], false);
    std::fs::rename(&away, &disk).unwrap();
    assert!(b::open_keys(&set, &Secret::new(pass.clone())).is_ok());
    let (s, v) = f
        .call(
            "POST",
            "/backup/passphrase",
            &desk,
            Some(json!({"generate": true})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["rekeyed"], json!(["Old disk/Archive"]));
    let new = v["passphrase"].as_str().unwrap();
    assert!(b::open_keys(&set, &Secret::new(new)).is_ok());

    // Forgotten, never deleted.
    let (s, v) = f
        .call("DELETE", "/backup/storage/backup", &phone, None)
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    let (s, v) = f
        .call("DELETE", "/backup/storage/stor_000000000000", &phone, None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    assert_eq!(v["error"]["code"], "storage_not_found");
    let (s, _) = f
        .call("DELETE", &format!("/backup/storage/{id}"), &phone, None)
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(
        disk.join("Archive/identity.age").is_file(),
        "its files stay"
    );
    let (_, v) = f.call("GET", "/backup", &phone, None).await;
    assert!(v.get("storage").is_none(), "{v}");
    assert!(v["settings"].get("storages").is_none(), "{v}");
    let audit = f.st.devices.audit.recent(100);
    for action in [
        "backup.storage.add",
        "backup.storage.rename",
        "backup.storage.remove",
    ] {
        assert!(audit.iter().any(|e| e.action == action), "{action}");
    }
}
