//! The embedded axum HTTP/WebSocket server.
//!
//! Every piece of dashboard data flows through here so the desktop webview and
//! phone browsers share exactly one path. A background monitor keeps the session
//! snapshot fresh (polling `claude agents --json` + filesystem watch) and streams
//! live transcript deltas over the broadcast bus.

pub mod http;
pub mod monitor;
pub mod ws;

use std::path::PathBuf;

use anyhow::Context;
use axum::routing::get;
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
pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(http::health))
        .route("/sessions", get(http::list_sessions))
        .route("/sessions/:id", get(http::get_session))
        .route("/sessions/:id/transcript", get(http::get_transcript))
        .route("/sessions/:id/diff", get(http::get_diff))
        .route("/sessions/:id/file-patch", get(http::get_file_patch))
        .route("/services", get(http::get_services))
        .route("/daemon", get(http::get_daemon));

    let mut app = Router::new()
        .route("/ws", get(ws::ws_handler))
        .nest("/api", api);

    app = match resolve_web_dir() {
        Some(dir) => {
            let index = dir.join("index.html");
            app.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(index)))
        }
        None => app.fallback(http::no_frontend),
    };

    app.layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Bind and serve (plain HTTP). TLS is layered on in the auth commit. Also spawns
/// the background monitor.
pub async fn serve(state: AppState) -> anyhow::Result<()> {
    let addr = state.config.bind_addr().with_context(|| {
        format!(
            "invalid bind address {}:{}",
            state.config.host, state.config.port
        )
    })?;

    tokio::spawn(monitor::run(state.clone()));

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("could not bind {addr}"))?;
    tracing::info!(%addr, "Mother Claude server listening (http)");

    let app = router(state);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .context("server error")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::ClaudeHome;
    use crate::state::{Inner, ServerConfig};

    /// Boot the router on an ephemeral port (no monitor, no `claude` calls) and
    /// hit the REST API over real HTTP.
    #[tokio::test]
    async fn serves_health_and_sessions_over_http() {
        let dir = tempfile::tempdir().unwrap();
        let state = Inner::new(
            ClaudeHome::with_base(dir.path()),
            ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
        );
        let app = router(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let base = format!("http://{addr}");
        let client = reqwest::Client::new();

        let health: serde_json::Value = client
            .get(format!("{base}/api/health"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(health["status"], "ok");

        let sessions: serde_json::Value = client
            .get(format!("{base}/api/sessions"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(sessions.is_array());
        assert_eq!(sessions.as_array().unwrap().len(), 0);

        // Unknown session -> 404.
        let resp = client
            .get(format!("{base}/api/sessions/does-not-exist"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
    }
}
