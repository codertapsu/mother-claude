//! Claude's own sign-in — read through the CLI, like everything else here.
//!
//! This is *not* the Mother Claude API token. It is the Anthropic account the
//! `claude` runtime uses, and without it every turn comes back as the string
//! "Not logged in · Please run /login" rather than as a typed failure. Checking
//! it once at startup turns a confusing per-request symptom into one clear
//! statement the moment the app opens.
//!
//! `claude auth status` emits JSON by default; every field is optional here
//! because this is a research-preview surface that has already changed shape
//! once.

use std::process::Stdio;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The result of asking the CLI who it is signed in as.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    /// Whether Claude can talk to Anthropic at all.
    #[serde(default)]
    pub logged_in: bool,
    /// `claude.ai`, `console`, `apiKey`, … as reported by the CLI.
    #[serde(default)]
    pub auth_method: Option<String>,
    #[serde(default)]
    pub api_provider: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub org_name: Option<String>,
    #[serde(default)]
    pub subscription_type: Option<String>,
    #[serde(default)]
    pub config_directory: Option<String>,
    /// Why the check itself failed, when it did. `logged_in` is false in that
    /// case, but "we could not ask" is a different problem from "not signed in"
    /// and the difference matters when someone is debugging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl AuthStatus {
    fn failed(message: impl Into<String>) -> Self {
        Self {
            error: Some(message.into()),
            ..Default::default()
        }
    }

    /// A one-line summary for the console and the dashboard.
    pub fn summary(&self) -> String {
        if let Some(error) = &self.error {
            return format!("could not check Claude sign-in: {error}");
        }
        if !self.logged_in {
            return "Claude is NOT signed in — run `claude auth login`".to_string();
        }
        let who = self.email.as_deref().unwrap_or("signed in");
        match (&self.subscription_type, &self.auth_method) {
            (Some(plan), _) => format!("{who} ({plan})"),
            (None, Some(method)) => format!("{who} ({method})"),
            _ => who.to_string(),
        }
    }
}

/// Ask the CLI for the current sign-in state.
///
/// Never fails: a missing binary, a timeout or an unparsable answer all come
/// back as a status carrying `error`, because an auth probe must not be able to
/// take the server down with it.
pub async fn status() -> AuthStatus {
    status_with(&crate::claude::claude_bin()).await
}

/// The check itself, against a named binary.
///
/// Split out so tests can point it at something that does not exist without
/// mutating `MOTHER_CLAUDE_CLI`, which is process-global and shared with every
/// other test running in parallel.
async fn status_with(binary: &str) -> AuthStatus {
    let run = tokio::process::Command::new(binary)
        .arg("auth")
        .arg("status")
        .arg("--json")
        .stdin(Stdio::null())
        .output();

    let output = match tokio::time::timeout(std::time::Duration::from_secs(20), run).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return AuthStatus::failed(format!("could not run `{binary} auth status`: {e}"))
        }
        Err(_) => return AuthStatus::failed("`claude auth status` timed out"),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(json) = stdout
        .find('{')
        .and_then(|start| serde_json::from_str::<Value>(stdout[start..].trim()).ok())
    else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return AuthStatus::failed(if stderr.is_empty() {
            "`claude auth status` returned no JSON".to_string()
        } else {
            stderr
        });
    };
    parse(&json)
}

/// Parse one `claude auth status --json` payload. Split out so the tolerance is
/// testable without a CLI.
pub fn parse(value: &Value) -> AuthStatus {
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
    };
    AuthStatus {
        logged_in: value
            .get("loggedIn")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        auth_method: text("authMethod"),
        api_provider: text("apiProvider"),
        email: text("email"),
        org_name: text("orgName"),
        subscription_type: text("subscriptionType"),
        config_directory: text("configDirectory"),
        error: None,
    }
}

/// Start the interactive sign-in flow.
///
/// `claude auth login` opens a browser and completes out of band, so this
/// spawns it and returns — there is nothing useful to wait for here, and
/// blocking an HTTP handler on a human finishing an OAuth round trip would only
/// produce a timeout. Poll [`status`] to see when it lands.
pub async fn begin_login(console: bool) -> anyhow::Result<()> {
    let mut command = tokio::process::Command::new(crate::claude::claude_bin());
    command.arg("auth").arg("login");
    if console {
        command.arg("--console");
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("could not start `claude auth login`: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_a_signed_in_account() {
        let status = parse(&json!({
            "loggedIn": true,
            "authMethod": "claude.ai",
            "apiProvider": "firstParty",
            "email": "someone@example.test",
            "orgName": "Example Org",
            "subscriptionType": "max",
            "configDirectory": "/Users/someone/.claude",
        }));
        assert!(status.logged_in);
        assert_eq!(status.email.as_deref(), Some("someone@example.test"));
        assert_eq!(status.subscription_type.as_deref(), Some("max"));
        assert_eq!(status.summary(), "someone@example.test (max)");
    }

    #[test]
    fn a_signed_out_account_says_what_to_do() {
        let status = parse(&json!({ "loggedIn": false }));
        assert!(!status.logged_in);
        assert!(status.summary().contains("claude auth login"));
    }

    #[test]
    fn unknown_and_missing_fields_are_tolerated() {
        // Every field absent, plus one this version has never heard of.
        let status = parse(&json!({ "somethingNew": 42 }));
        assert!(!status.logged_in);
        assert!(status.email.is_none());
        assert!(status.error.is_none());

        // Blank strings are treated as absent rather than shown as empty.
        let status = parse(&json!({ "loggedIn": true, "email": "  " }));
        assert!(status.email.is_none());
        assert_eq!(status.summary(), "signed in");
    }

    #[test]
    fn a_failed_check_is_distinguishable_from_being_signed_out() {
        let status = AuthStatus::failed("no such binary");
        assert!(!status.logged_in);
        assert_eq!(status.error.as_deref(), Some("no such binary"));
        assert!(status.summary().contains("could not check"));
    }

    #[tokio::test]
    async fn a_missing_cli_does_not_panic_or_hang() {
        let status = status_with("definitely-not-a-real-binary-xyz").await;
        assert!(!status.logged_in);
        assert!(status.error.is_some());
        assert!(status.summary().contains("could not check"));
    }

    #[tokio::test]
    async fn a_cli_that_says_nothing_useful_is_an_error_not_a_signed_out_account() {
        // `true` exits 0 and prints nothing — no JSON to read.
        let status = status_with("true").await;
        assert!(!status.logged_in);
        assert!(
            status.error.is_some(),
            "silence should not read as signed out"
        );
    }
}
