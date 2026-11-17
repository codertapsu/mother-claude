//! Who may call an API server (docs/CLAUDE-API.md "Access").
//!
//! * **Reachable from** (`access` in its defaults): this computer (the
//!   loopback port, as always), the local network too (its port on the
//!   private addresses) or anywhere (its port on every address of this
//!   computer, public ones included, and through tunnels such as ngrok).
//!   Beyond this computer the listeners are plain HTTP ([`super::network`]).
//! * **Key**: optional, given at the start (`key` on the start routes). With
//!   one, every request carries it — `x-api-key: <key>` or `Authorization:
//!   Bearer <key>`, what the Claude API's SDKs send — from this computer too;
//!   Mother Claude's own tokens (the desktop, paired devices) still work
//!   where they always did. It is kept in the secret store (Keychain,
//!   Credential Manager), never in a file; `api-servers.json` keeps its last
//!   four characters. A start whose key cannot be read back fails, never
//!   starts open.
//! * **Without a key**, from beyond this computer the Claude API answers
//!   (its conversations have no tools); the rest of the API — conversations
//!   that run Claude Code with its tools on this computer — needs the key,
//!   or the desktop's *Open to tunnels* (`TokenPolicy::Never`).

use axum::http::HeaderMap;

use crate::bridge::error::{BridgeError, BridgeResult};

/// Where an API server can be reached from, from the narrowest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Access {
    /// This computer only: the loopback port.
    Local,
    /// The local network too: the port on the private addresses.
    Lan,
    /// Anywhere: the port on every address, and through tunnels.
    All,
}

/// `access` as written.
pub const ACCESS: [&str; 3] = ["local", "lan", "all"];

impl Access {
    pub fn parse(s: Option<&str>) -> Self {
        match s {
            Some("lan") => Self::Lan,
            Some("all") => Self::All,
            _ => Self::Local,
        }
    }

    /// Beyond this computer.
    pub fn is_open(self) -> bool {
        self != Self::Local
    }
}

/// The longest key taken.
pub const KEY_MAX: usize = 256;

/// A key is 1–256 visible ASCII characters (it travels in a header).
pub fn check_key(key: &str) -> BridgeResult<()> {
    if key.is_empty() || key.len() > KEY_MAX || !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(BridgeError::invalid_field(
            "key",
            format!(
                "A key is 1–{KEY_MAX} visible characters: letters, digits and symbols, no spaces."
            ),
        ));
    }
    Ok(())
}

/// The key a request presents: `x-api-key`, else `Authorization: Bearer`.
pub fn presented(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        return Some(v.to_string());
    }
    let auth = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .trim();
    (auth.len() > 7 && auth[..7].eq_ignore_ascii_case("bearer "))
        .then(|| auth[7..].trim().to_string())
        .filter(|t| !t.is_empty())
}

/// A path of the Claude API (at `/` or `/v1`): what a server reachable
/// beyond this computer answers without a key.
pub fn is_api_path(path: &str) -> bool {
    let p = path.strip_prefix("/v1").unwrap_or(path);
    let p = p.trim_end_matches('/');
    let one_more = |prefix: &str| {
        p.strip_prefix(prefix)
            .is_some_and(|rest| !rest.is_empty() && !rest.contains('/'))
    };
    matches!(
        p,
        "/messages" | "/messages/count_tokens" | "/models" | "/conversations"
    ) || one_more("/models/")
        || one_more("/conversations/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_is_read_leniently() {
        assert_eq!(Access::parse(None), Access::Local);
        assert_eq!(Access::parse(Some("lan")), Access::Lan);
        assert_eq!(Access::parse(Some("all")), Access::All);
        assert_eq!(Access::parse(Some("public")), Access::Local);
        assert!(!Access::Local.is_open() && Access::Lan.is_open() && Access::All.is_open());
    }

    #[test]
    fn keys_are_visible_characters() {
        assert!(check_key("sk-my-service-key_2026!").is_ok());
        assert!(check_key("").is_err());
        assert!(check_key("has space").is_err());
        assert!(check_key("tab\tkey").is_err());
        assert!(check_key("é").is_err());
        assert!(check_key(&"x".repeat(KEY_MAX + 1)).is_err());
    }

    #[test]
    fn a_key_is_read_from_either_header() {
        let mut h = HeaderMap::new();
        h.insert("x-api-key", "one".parse().unwrap());
        h.insert("authorization", "Bearer two".parse().unwrap());
        assert_eq!(presented(&h).as_deref(), Some("one"));
        h.remove("x-api-key");
        assert_eq!(presented(&h).as_deref(), Some("two"));
        h.insert("authorization", "Basic abc".parse().unwrap());
        assert_eq!(presented(&h), None);
    }

    #[test]
    fn the_claude_apis_paths_are_known() {
        for p in [
            "/messages",
            "/v1/messages",
            "/v1/messages/count_tokens",
            "/models",
            "/v1/models/claude-sonnet-5-5",
            "/v1/conversations",
            "/conversations/00000000-0000-4000-8000-000000000000",
        ] {
            assert!(is_api_path(p), "{p}");
        }
        for p in [
            "/threads",
            "/v1/chat",
            "/v1/files",
            "/v1/models/a/b",
            "/v1/messagesx",
            "/requests",
            "/health",
        ] {
            assert!(!is_api_path(p), "{p}");
        }
    }
}
