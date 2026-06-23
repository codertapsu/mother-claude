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
        .route("/sessions/{id}", get(http::get_session))
        .route("/sessions/{id}/transcript", get(http::get_transcript))
        .route("/sessions/{id}/diff", get(http::get_diff))
        .route("/sessions/{id}/file-patch", get(http::get_file_patch))
        .route("/sessions/{id}/message", post(http::post_message))
        .route("/sessions/{id}/continue", post(http::post_continue))
        .route(
            "/sessions/{id}/permission-request",
            post(http::post_permission_request),
        )
        .route("/sessions/{id}/permission", post(http::post_permission))
        .route("/sessions/{id}/answer", post(http::post_answer))
        .route("/sessions/{id}/stop", post(http::post_stop))
        .route("/sessions/{id}/respawn", post(http::post_respawn))
        .route("/sessions/{id}/rm", post(http::post_rm))
        .route("/services", get(http::get_services))
        .route("/daemon", get(http::get_daemon))
        .route("/pairing", get(http::get_pairing))
        .route("/hooks/install", post(http::post_install_hooks));

    // Experimental, unsanctioned foreign-session injection (off by default).
    #[cfg(feature = "experimental")]
    let api = api
        .route("/sessions/{id}/pty-attach", post(http::post_pty_attach))
        .route("/sessions/{id}/pty-inject", post(http::post_pty_inject));

    let secured = Router::new()
        .route("/ws", get(ws::ws_handler))
        .route("/hooks/event", post(http::post_hook_event))
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

/// Bind and serve.
///
/// Always serves plain HTTP on `127.0.0.1:<port>` — this loopback endpoint is
/// what the desktop webview, foreign-session hooks, and the local sidecar use,
/// avoiding self-signed-cert friction. When the configured host is non-loopback,
/// it additionally serves **TLS** on each detected LAN IP (same port) for phones.
/// Also spawns the background monitor.
pub async fn serve(state: AppState) -> anyhow::Result<()> {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    tokio::spawn(monitor::run(state.clone()));

    let port = state.config.port;

    // LAN TLS servers (bound to specific IPs so they don't collide with the
    // loopback HTTP bind on the same port).
    if state.config.is_non_loopback() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let cert_dir = state
            .auth
            .config_dir
            .clone()
            .unwrap_or_else(|| std::env::temp_dir().join("mother-claude"))
            .join("certs");
        match tls::ensure_cert(&cert_dir) {
            Ok(bundle) => {
                *state.fingerprint.write().await = Some(bundle.fingerprint.clone());
                match axum_server::tls_rustls::RustlsConfig::from_pem(
                    bundle.cert_pem.into_bytes(),
                    bundle.key_pem.into_bytes(),
                )
                .await
                {
                    Ok(tls_config) => {
                        for ip in tls::local_ips() {
                            let addr = SocketAddr::new(ip, port);
                            let app = router(state.clone());
                            let cfg = tls_config.clone();
                            tokio::spawn(async move {
                                if let Err(e) = axum_server::bind_rustls(addr, cfg)
                                    .serve(app.into_make_service_with_connect_info::<SocketAddr>())
                                    .await
                                {
                                    tracing::error!(%addr, error = %e, "TLS server stopped");
                                }
                            });
                        }
                    }
                    Err(e) => tracing::error!(error = %e, "could not build TLS config"),
                }
            }
            Err(e) => tracing::error!(error = %e, "could not prepare TLS cert"),
        }
    }

    announce(&state).await;

    // Primary: loopback HTTP. Keeps serve() alive for the app's lifetime.
    let loopback = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let listener = tokio::net::TcpListener::bind(loopback)
        .await
        .with_context(|| format!("could not bind {loopback}"))?;
    let app = router(state.clone());
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .context("loopback server error")?;
    Ok(())
}

/// Log the token and URLs to the console at startup.
async fn announce(state: &AppState) {
    let fingerprint = state
        .fingerprint
        .read()
        .await
        .clone()
        .unwrap_or_else(|| "n/a (http)".to_string());
    let port = state.config.port;
    tracing::info!(token = %state.auth.token, "Mother Claude server listening");
    println!("\n  Mother Claude dashboard");
    println!("  Local:  http://127.0.0.1:{port}");
    if state.config.is_non_loopback() {
        for ip in tls::local_ips() {
            println!("  LAN:    https://{ip}:{port}  (scan the QR in Settings)");
        }
        println!("  TLS fingerprint (SHA-256): {fingerprint}");
    }
    println!("  Token:  {}", state.auth.token);
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

    /// End-to-end approval loop: a (simulated) sidecar raises a permission
    /// request that blocks; the dashboard approves it; the request unblocks with
    /// `allow`.
    #[tokio::test]
    async fn permission_request_resolves_via_dashboard() {
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
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .unwrap();
        });

        let base = format!("http://{addr}");
        let client = reqwest::Client::new();

        // Sidecar side: raise a permission request and block for the answer.
        let (c2, b2, t2) = (client.clone(), base.clone(), token.clone());
        let blocked = tokio::spawn(async move {
            c2.post(format!("{b2}/api/sessions/sess1/permission-request"))
                .bearer_auth(&t2)
                .json(&serde_json::json!({ "kind": "permission", "tool": "Bash" }))
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap()
        });

        // Dashboard side: approve (retry until the pending request is registered).
        let mut approved = false;
        for _ in 0..50 {
            let r = client
                .post(format!("{base}/api/sessions/sess1/permission"))
                .bearer_auth(&token)
                .json(&serde_json::json!({ "decision": "allow" }))
                .send()
                .await
                .unwrap();
            if r.status() == reqwest::StatusCode::NO_CONTENT {
                approved = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        }
        assert!(approved, "dashboard approval never landed");

        let decision = blocked.await.unwrap();
        assert_eq!(decision["behavior"], "allow");
    }

    /// Hook events require the token and are fanned out on the bus.
    #[tokio::test]
    async fn hook_event_authenticated_and_broadcast() {
        use crate::state::ServerEvent;

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
        let mut rx = state.bus.subscribe();
        let app = router(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .unwrap();
        });

        let base = format!("http://{addr}");
        let client = reqwest::Client::new();

        // No token -> 401.
        let resp = client
            .post(format!("{base}/hooks/event"))
            .json(&serde_json::json!({ "hook_event_name": "PreToolUse" }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

        // With token -> 200 and a Hook event on the bus.
        let resp = client
            .post(format!("{base}/hooks/event"))
            .bearer_auth(&token)
            .json(&serde_json::json!({ "hook_event_name": "PreToolUse", "tool_name": "Bash" }))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success());

        let event = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("no event")
            .unwrap();
        match event {
            ServerEvent::Hook(v) => assert_eq!(v["tool_name"], "Bash"),
            other => panic!("expected Hook event, got {other:?}"),
        }
    }
}
