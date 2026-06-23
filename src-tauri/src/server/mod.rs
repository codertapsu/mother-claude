//! The embedded axum HTTP/WebSocket server.
//!
//! Every piece of dashboard data flows through here so the desktop webview and
//! phone browsers share exactly one path. A background monitor keeps the session
//! snapshot fresh (polling `claude agents --json` + filesystem watch) and streams
//! live transcript deltas over the broadcast bus.

pub mod auth;
pub mod http;
pub mod monitor;
pub mod tls;
pub mod ws;

use std::path::PathBuf;

use anyhow::Context;
use axum::routing::{get, post};
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use crate::state::AppState;

/// Resolve the built Angular SPA directory, or `None` if it isn't present.
/// Honors `MOTHER_CLAUDE_WEB_DIR`; otherwise probes the usual locations.
pub fn resolve_web_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("MOTHER_CLAUDE_WEB_DIR") {
        let p = PathBuf::from(dir);
        if p.join("index.html").is_file() {
            return Some(p);
        }
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("dist/mother-claude/browser"));
        candidates.push(cwd.join("../dist/mother-claude/browser"));
    }
    candidates
        .push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dist/mother-claude/browser"));
    candidates
        .into_iter()
        .find(|p| p.join("index.html").is_file())
}

/// Build the axum router (without binding). Exposed for integration tests.
///
/// `/api/*`, `/ws`, and `/hooks/*` require the API token; the static SPA is
/// served without auth so a phone can load the app and then authenticate.
pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(http::health))
        .route("/sessions", get(http::list_sessions).post(http::post_spawn))
        .route("/sessions/:id", get(http::get_session))
        .route("/sessions/:id/transcript", get(http::get_transcript))
        .route("/sessions/:id/diff", get(http::get_diff))
        .route("/sessions/:id/file-patch", get(http::get_file_patch))
        .route("/sessions/:id/message", post(http::post_message))
        .route("/services", get(http::get_services))
        .route("/daemon", get(http::get_daemon))
        .route("/pairing", get(http::get_pairing));

    let secured = Router::new()
        .route("/ws", get(ws::ws_handler))
        .nest("/api", api)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_token,
        ));

    let app = match resolve_web_dir() {
        Some(dir) => {
            let index = dir.join("index.html");
            secured.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(index)))
        }
        None => secured.fallback(http::no_frontend),
    };

    app.layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Bind and serve. TLS (self-signed) is used for any non-loopback bind; plain
/// HTTP for loopback. Also spawns the background monitor.
pub async fn serve(state: AppState) -> anyhow::Result<()> {
    let addr = state.config.bind_addr().with_context(|| {
        format!(
            "invalid bind address {}:{}",
            state.config.host, state.config.port
        )
    })?;

    tokio::spawn(monitor::run(state.clone()));

    let make = router(state.clone()).into_make_service_with_connect_info::<std::net::SocketAddr>();

    if state.config.is_non_loopback() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let cert_dir = state
            .auth
            .config_dir
            .clone()
            .unwrap_or_else(|| std::env::temp_dir().join("mother-claude"))
            .join("certs");
        let bundle = tls::ensure_cert(&cert_dir).context("ensure TLS cert")?;
        *state.fingerprint.write().await = Some(bundle.fingerprint.clone());

        let tls_config = axum_server::tls_rustls::RustlsConfig::from_pem(
            bundle.cert_pem.into_bytes(),
            bundle.key_pem.into_bytes(),
        )
        .await
        .context("build rustls config")?;

        announce(&state, true, &bundle.fingerprint);
        axum_server::bind_rustls(addr, tls_config)
            .serve(make)
            .await
            .context("tls server error")?;
    } else {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .with_context(|| format!("could not bind {addr}"))?;
        announce(&state, false, "n/a (http)");
        axum::serve(listener, make).await.context("server error")?;
    }
    Ok(())
}

/// Log the token, URL, and fingerprint to the console at startup.
fn announce(state: &AppState, tls: bool, fingerprint: &str) {
    let pairing = auth::build_pairing(state, fingerprint);
    let scheme = if tls { "https" } else { "http" };
    tracing::info!(
        url = %pairing.url,
        token = %state.auth.token,
        fingerprint = %fingerprint,
        "Mother Claude server listening ({scheme})"
    );
    println!("\n  Mother Claude dashboard");
    println!("  URL:   {}", pairing.url);
    println!("  Token: {}", state.auth.token);
    if tls {
        println!("  TLS fingerprint (SHA-256): {fingerprint}");
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::ClaudeHome;
    use crate::state::{Inner, ServerConfig};

    /// Boot the router on an ephemeral port (no monitor, no `claude` calls) and
    /// hit the REST API over real HTTP, exercising token auth.
    #[tokio::test]
    async fn serves_api_with_token_auth() {
        let dir = tempfile::tempdir().unwrap();
        let state = Inner::new(
            ClaudeHome::with_base(dir.path()),
            ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            auth::Auth::ephemeral(),
        );
        let token = state.auth.token.clone();
        let app = router(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let base = format!("http://{addr}");
        let client = reqwest::Client::new();

        // Missing token -> 401.
        let resp = client
            .get(format!("{base}/api/sessions"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

        // With bearer token -> ok.
        let health: serde_json::Value = client
            .get(format!("{base}/api/health"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(health["status"], "ok");

        // Token via query string (the WebSocket path) also works.
        let sessions: serde_json::Value = client
            .get(format!("{base}/api/sessions?token={token}"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(sessions.is_array());
        assert_eq!(sessions.as_array().unwrap().len(), 0);

        // Pairing payload includes a QR SVG.
        let pairing: serde_json::Value = client
            .get(format!("{base}/api/pairing"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(pairing["svg"].as_str().unwrap().contains("<svg"));

        // Unknown session -> 404 (authorized).
        let resp = client
            .get(format!("{base}/api/sessions/does-not-exist"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
    }
}
