//! Authentication, device pairing, and the dangerous-action gate.
//!
//! A random API token is required on every `/api/*`, `/ws`, and `/hooks/*`
//! request (bearer header, `?token=` query — needed for browser WebSockets — or
//! `mc_token` cookie). Dangerous actions default to local-desktop only.

use std::net::SocketAddr;
use std::path::PathBuf;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use directories::ProjectDirs;
use serde::Serialize;
use uuid::Uuid;

use crate::state::AppState;

/// Auth/session configuration carried in [`crate::state::Inner`].
#[derive(Debug, Clone)]
pub struct Auth {
    pub token: String,
    /// Allow remote (non-loopback) clients to approve dangerous actions
    /// (`bypassPermissions`) and trigger irreversible lifecycle ops. Default off.
    pub allow_remote_dangerous: bool,
    /// Persisted-state directory (certs/token). `None` for ephemeral/test auth.
    pub config_dir: Option<PathBuf>,
}

impl Auth {
    /// Random, in-memory only (used in tests — no disk writes).
    pub fn ephemeral() -> Self {
        Self {
            token: new_token(),
            allow_remote_dangerous: false,
            config_dir: None,
        }
    }

    /// Load the persisted token from the app config dir, generating + persisting
    /// one on first run.
    pub fn load_or_create() -> Self {
        let config_dir = app_config_dir();
        let token = config_dir
            .as_ref()
            .and_then(|dir| {
                let path = dir.join("token");
                std::fs::read_to_string(&path)
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .or_else(|| {
                        let t = new_token();
                        let _ = std::fs::create_dir_all(dir);
                        if std::fs::write(&path, &t).is_ok() {
                            restrict(&path);
                        }
                        Some(t)
                    })
            })
            .unwrap_or_else(new_token);

        Self {
            token,
            allow_remote_dangerous: std::env::var("MOTHER_CLAUDE_ALLOW_REMOTE_DANGEROUS")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            config_dir,
        }
    }
}

fn new_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

/// `~/Library/Application Support/dev.motherclaude…` (or the platform equivalent).
pub fn app_config_dir() -> Option<PathBuf> {
    ProjectDirs::from("dev", "motherclaude", "Mother Claude").map(|d| d.data_dir().to_path_buf())
}

#[cfg(unix)]
fn restrict(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict(_path: &std::path::Path) {}

/// Whether the peer is the local machine (desktop). Dangerous actions are
/// allowed only for loopback peers unless `allow_remote_dangerous` is set.
pub fn is_loopback(addr: &SocketAddr) -> bool {
    addr.ip().is_loopback()
}

/// The dangerous-action gate: block a dangerous approval when it comes from a
/// non-loopback peer and remote-dangerous is not explicitly enabled.
pub fn dangerous_blocked(dangerous: bool, loopback: bool, allow_remote: bool) -> bool {
    dangerous && !loopback && !allow_remote
}

/// Extract the presented token from header / query / cookie.
fn presented_token(req: &Request<Body>) -> Option<String> {
    if let Some(value) = req.headers().get(axum::http::header::AUTHORIZATION) {
        if let Ok(s) = value.to_str() {
            if let Some(rest) = s.strip_prefix("Bearer ") {
                return Some(rest.trim().to_string());
            }
        }
    }
    if let Some(query) = req.uri().query() {
        for pair in query.split('&') {
            if let Some(v) = pair.strip_prefix("token=") {
                return Some(urldecode(v));
            }
        }
    }
    if let Some(cookie) = req.headers().get(axum::http::header::COOKIE) {
        if let Ok(s) = cookie.to_str() {
            for part in s.split(';') {
                if let Some(v) = part.trim().strip_prefix("mc_token=") {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

fn urldecode(s: &str) -> String {
    // Minimal %XX decoding sufficient for tokens (hex chars only anyway).
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Constant-time-ish token comparison.
fn token_matches(expected: &str, presented: &str) -> bool {
    let a = expected.as_bytes();
    let b = presented.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Auth middleware applied to `/api`, `/ws`, and `/hooks`.
pub async fn require_token(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    match presented_token(&req) {
        Some(tok) if token_matches(&state.auth.token, &tok) => Ok(next.run(req).await),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

/// Device-pairing payload (QR + verification info).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pairing {
    pub url: String,
    pub token: String,
    pub svg: String,
    pub fingerprint: String,
    pub addresses: Vec<String>,
    pub port: u16,
    pub tls: bool,
}

/// Build the pairing info: the URL a phone scans, an inline SVG QR code, the TLS
/// fingerprint, and the reachable addresses.
pub fn build_pairing(state: &AppState, fingerprint: &str) -> Pairing {
    let tls = state.config.is_non_loopback();
    let scheme = if tls { "https" } else { "http" };
    let port = state.config.port;
    let ips: Vec<String> = super::tls::local_ips()
        .into_iter()
        .map(|ip| ip.to_string())
        .collect();
    let host = ips
        .first()
        .cloned()
        .unwrap_or_else(|| "localhost".to_string());
    let url = format!("{scheme}://{host}:{port}/#/pair?token={}", state.auth.token);
    let svg = qr_svg(&url);
    Pairing {
        url,
        token: state.auth.token.clone(),
        svg,
        fingerprint: fingerprint.to_string(),
        addresses: ips,
        port,
        tls,
    }
}

/// Render `data` as an inline SVG QR code (no raster image dependency).
pub fn qr_svg(data: &str) -> String {
    match qrcode::QrCode::new(data.as_bytes()) {
        Ok(code) => code
            .render::<qrcode::render::svg::Color>()
            .min_dimensions(220, 220)
            .quiet_zone(true)
            .build(),
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_compare() {
        assert!(token_matches("abc123", "abc123"));
        assert!(!token_matches("abc123", "abc124"));
        assert!(!token_matches("abc", "abcd"));
    }

    #[test]
    fn ephemeral_tokens_are_long_and_unique() {
        let a = Auth::ephemeral();
        let b = Auth::ephemeral();
        assert_eq!(a.token.len(), 64);
        assert_ne!(a.token, b.token);
        assert!(a.config_dir.is_none());
    }

    #[test]
    fn qr_svg_is_svg() {
        let svg = qr_svg("https://example.test/#/pair?token=xyz");
        assert!(svg.contains("<svg"));
    }

    #[test]
    fn urldecode_basic() {
        assert_eq!(urldecode("ab%20cd"), "ab cd");
        assert_eq!(urldecode("plain"), "plain");
    }

    #[test]
    fn loopback_detection() {
        assert!(is_loopback(&"127.0.0.1:1".parse().unwrap()));
        assert!(!is_loopback(&"192.168.1.5:1".parse().unwrap()));
    }

    #[test]
    fn dangerous_gate() {
        // Safe actions are never blocked.
        assert!(!dangerous_blocked(false, false, false));
        // Dangerous + loopback (desktop) -> allowed.
        assert!(!dangerous_blocked(true, true, false));
        // Dangerous + remote + not allowed -> blocked.
        assert!(dangerous_blocked(true, false, false));
        // Dangerous + remote + explicitly allowed -> permitted.
        assert!(!dangerous_blocked(true, false, true));
    }
}
