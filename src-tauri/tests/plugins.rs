//! Claude Code's plugins managed from Mother Claude (docs/PLUGINS.md):
//! reading what is installed, the marketplaces and the catalog, and the
//! desktop's changes — each one `claude plugin …` run by the FAKE `claude`
//! (`tests/fixtures/fake-claude.py`, selected with `MOTHER_CLAUDE_CLI`),
//! whose plugin state is one file in the temp directory
//! (`FAKE_PLUGINS_STATE`). Nothing touches `~/.claude`: the real CLI never
//! runs.
#![cfg(unix)]
// Tests start their programs directly; the app's own code goes through
// `proc`, which keeps console windows from flashing up on Windows.
#![allow(clippy::disallowed_methods)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::OnceLock;

use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::server::{self, auth::Auth};
use mother_claude_lib::state::{AppState, Inner, ServerConfig};
use reqwest::StatusCode;
use serde_json::{json, Value};

/// The fake, and the one file its plugin state lives in.
fn setup() -> PathBuf {
    static ONCE: OnceLock<PathBuf> = OnceLock::new();
    ONCE.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap().keep();
        let fake = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-claude.py");
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755));
        }
        std::env::set_var("MOTHER_CLAUDE_CLI", &fake);
        std::env::set_var("CLAUDE_CONFIG_DIR", dir.join("config"));
        let state = dir.join("plugins.json");
        std::env::set_var("FAKE_PLUGINS_STATE", &state);
        state
    })
    .clone()
}

struct App {
    st: AppState,
    base: String,
    _home: tempfile::TempDir,
}

async fn app() -> App {
    setup();
    let home = tempfile::tempdir().unwrap();
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
    let router = server::router(st.clone());
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    App {
        st,
        base: format!("http://{addr}"),
        _home: home,
    }
}

impl App {
    async fn call_as(
        &self,
        token: &str,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let c = reqwest::Client::new();
        let url = format!("{}/api{path}", self.base);
        let req = match method {
            "GET" => c.get(url),
            "POST" => c.post(url),
            "DELETE" => c.delete(url),
            m => panic!("{m}"),
        }
        .bearer_auth(token);
        let req = match body {
            Some(b) => req.json(&b),
            None => req,
        };
        let r = req.send().await.unwrap();
        let s = r.status();
        (s, r.json().await.unwrap_or(Value::Null))
    }

    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let token = self.st.auth.token.clone();
        self.call_as(&token, method, path, body).await
    }
}

fn calls() -> Vec<Value> {
    let state: Value = serde_json::from_str(&std::fs::read_to_string(setup()).unwrap_or_default())
        .unwrap_or_default();
    state["calls"].as_array().cloned().unwrap_or_default()
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

// One test: the fake's plugin state is one file for the whole binary.
#[tokio::test]
async fn plugins_are_managed_through_claude_code_from_the_desktop_only() {
    let a = app().await;

    // Nothing installed yet; the official marketplace is known.
    let (s, v) = a.call("GET", "/plugins", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["installed"], json!([]));
    assert_eq!(v["marketplaces"][0]["name"], "claude-plugins-official");
    assert_eq!(
        v["marketplaces"][0]["location"],
        "anthropics/claude-plugins-official"
    );

    // The catalog, searched: by name first.
    let (s, v) = a.call("GET", "/plugins/available?q=figma", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["plugins"][0]["id"], "figma@claude-plugins-official");
    assert_eq!(v["total"], 1);
    assert_eq!(v["catalog"], 3);

    // Installed everywhere, through Claude Code; its server named without
    // the header that carries a token.
    let (s, v) = a
        .call(
            "POST",
            "/plugins/install",
            Some(json!({ "plugin": "figma@claude-plugins-official" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let last = calls().last().cloned().unwrap();
    assert_eq!(
        last["argv"],
        json!([
            "plugin",
            "install",
            "figma@claude-plugins-official",
            "--scope",
            "user",
            "--json"
        ])
    );
    let (_, v) = a.call("GET", "/plugins", None).await;
    let text = v.to_string();
    assert!(!text.contains("t0ken"), "{text}");
    assert_eq!(v["installed"][0]["scope"], "user");
    assert_eq!(
        v["installed"][0]["mcpServers"],
        json!([{ "name": "srv", "kind": "http", "host": "mcp.example.com" }])
    );

    // A marketplace that installs by running a command: shown first, then
    // run only once the person accepted that exact command.
    let (s, v) = a
        .call(
            "POST",
            "/plugins/install",
            Some(json!({ "plugin": "needs-command@claude-plugins-official" })),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "confirm_command");
    assert_eq!(
        v["error"]["command"],
        "npx needs-command-installer --global"
    );
    let sha = v["error"]["sha256"].as_str().unwrap().to_string();
    let (s, v) = a
        .call(
            "POST",
            "/plugins/install",
            Some(
                json!({ "plugin": "needs-command@claude-plugins-official", "acceptCommand": sha }),
            ),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // For one project, only for me: run in that project's folder.
    let project = tempfile::tempdir().unwrap();
    let dir = dunce::canonicalize(project.path()).unwrap();
    let dir_s = dir.to_string_lossy().into_owned();
    let (s, v) = a
        .call(
            "POST",
            "/plugins/install",
            Some(json!({ "plugin": "postman@claude-plugins-official", "scope": "local", "project": dir_s })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(calls().last().unwrap()["cwd"], dir_s.as_str());
    let (_, everywhere) = a.call("GET", "/plugins", None).await;
    assert!(!everywhere.to_string().contains("postman"));
    let (_, there) = a
        .call("GET", &format!("/plugins?project={}", enc(&dir_s)), None)
        .await;
    let postman = there["installed"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "postman")
        .cloned()
        .expect("the project's plugin");
    assert_eq!(postman["scope"], "local");
    assert_eq!(postman["projectEnabled"], true);
    let (s, v) = a
        .call(
            "POST",
            "/plugins/install",
            Some(json!({ "plugin": "postman@claude-plugins-official", "scope": "project" })),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a project scope needs one: {v}"
    );

    // Off, then removed.
    let (s, v) = a
        .call(
            "POST",
            "/plugins/enabled",
            Some(json!({ "plugin": "figma@claude-plugins-official", "enabled": false })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, v) = a.call("GET", "/plugins", None).await;
    let figma = v["installed"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "figma")
        .cloned()
        .unwrap();
    assert_eq!(figma["enabled"], false);
    let (s, v) = a
        .call(
            "POST",
            "/plugins/uninstall",
            Some(json!({ "plugin": "figma@claude-plugins-official" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = a
        .call(
            "POST",
            "/plugins/uninstall",
            Some(json!({ "plugin": "figma@claude-plugins-official" })),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not installed"),
        "{v}"
    );

    // Mother Claude's own plugin has its own place; nothing reads as an
    // option.
    let (s, v) = a
        .call(
            "POST",
            "/plugins/uninstall",
            Some(json!({ "plugin": "mother-claude@mother-claude-local" })),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(v["error"]["code"], "plugin_is_ours");
    let before = calls().len();
    let (s, _) = a
        .call("POST", "/plugins/install", Some(json!({ "plugin": "-y" })))
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, _) = a
        .call(
            "POST",
            "/plugins/marketplaces",
            Some(json!({ "source": "--scope=project" })),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(calls().len(), before, "refused before Claude Code ran");

    // A marketplace of one's own, then gone.
    let (s, v) = a
        .call(
            "POST",
            "/plugins/marketplaces",
            Some(json!({ "source": "acme/team-plugins" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, v) = a.call("GET", "/plugins", None).await;
    assert!(v["marketplaces"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["name"] == "team-plugins"));
    let (s, v) = a
        .call("DELETE", "/plugins/marketplaces/team-plugins", None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, _) = a
        .call("DELETE", "/plugins/marketplaces/mother-claude-local", None)
        .await;
    assert_eq!(s, StatusCode::CONFLICT);

    // A local script reads, never changes.
    let script = a.st.auth.script_token.clone();
    let (s, _) = a.call_as(&script, "GET", "/plugins", None).await;
    assert_eq!(s, StatusCode::OK);
    let before = calls().len();
    let (s, _) = a
        .call_as(
            &script,
            "POST",
            "/plugins/install",
            Some(json!({ "plugin": "figma@claude-plugins-official" })),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(calls().len(), before);

    // Nothing ran under another directory than the one the app runs with.
    for c in calls() {
        assert_eq!(
            c["configDir"],
            std::env::var("CLAUDE_CONFIG_DIR").unwrap().as_str(),
            "{c}"
        );
    }

    // Every session it runs reads its plugins again (none run here).
    let (s, v) = a.call("POST", "/plugins/reload", Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        v,
        json!({ "reloaded": 0, "held": 0, "busy": 0, "failed": 0 })
    );
}
