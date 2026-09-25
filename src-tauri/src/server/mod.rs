//! The embedded axum HTTP/WebSocket server.
//!
//! Every piece of dashboard data flows through here so the desktop webview and
//! phone browsers share exactly one path. Each listener serves
//! [`router_for`] with its own [`Ingress`], so the context middleware knows
//! which door a request came through.
//!
//! Module owners (see `interfaces.md`): `context`, `auth`, `error`, `ws`,
//! `routes/mod.rs` — foundation; `devices`, `pairing`, `remote`, `ratelimit`,
//! `security`, `listeners`, `tls` — B1; `monitor` — B2; `hooks` — B4; `push` —
//! B5.

pub mod auth;
pub mod context;
pub mod devices;
pub mod error;
pub mod hooks;
pub mod listeners;
pub mod monitor;
pub mod pairing;
pub mod push;
pub mod ratelimit;
pub mod remote;
pub mod routes;
pub mod security;
pub mod tls;
pub mod ws;

use std::path::PathBuf;

use anyhow::Context;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use tower_http::services::{ServeDir, ServeFile};

pub use context::Ingress;

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

/// Served when the Angular build is not present on disk.
pub async fn no_frontend() -> impl IntoResponse {
    (
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        [("content-type", "text/html; charset=utf-8")],
        "<h1>Mother Claude</h1><p>Frontend not built. Run <code>npm run build</code> \
         or set <code>MOTHER_CLAUDE_WEB_DIR</code>. The API is available under <code>/api</code>.</p>",
    )
}

/// Build the application router for one listener.
///
/// * `/api/*` — the dashboard API (guards per route, see `routes`);
/// * `/ws` — WebSocket v2 (authenticates itself);
/// * `/hooks/event`, `/hooks/wait` — the hook plugin (Hook principal; not on
///   the tunnel ingress);
/// * `/v1` — the Claude HTTP bridge, token always required here (not on the
///   tunnel ingress);
/// * everything else — the SPA (no auth, so a phone can load the app and
///   then pair).
pub fn router_for(state: AppState, ingress: Ingress) -> Router {
    let mut app: Router<AppState> = Router::new()
        .nest("/api", routes::api_router())
        .route("/ws", get(ws::ws_handler));
    if ingress != Ingress::Tunnel {
        app = app
            .route(
                "/hooks/event",
                post(hooks::post_event)
                    .layer(axum::extract::DefaultBodyLimit::max(routes::IMAGE_BODY_MAX)),
            )
            .route(
                "/hooks/wait",
                get(hooks::wait)
                    .post(hooks::wait)
                    .layer(axum::extract::DefaultBodyLimit::max(routes::IMAGE_BODY_MAX)),
            )
            // Mounted with its own auth layer — always token-required here,
            // since this listener may be reachable from the LAN. Its tokenless
            // mode exists only on the dedicated loopback port.
            .nest("/v1", crate::bridge::router(state.clone(), true));
    }
    let app = match resolve_web_dir() {
        Some(dir) => {
            let index = dir.join("index.html");
            app.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(index)))
        }
        None => app.fallback(no_frontend),
    };
    let app = security::apply(app.with_state(state.clone()), ingress);
    context::layer(app, state, ingress)
}

/// The loopback router (desktop, local scripts, hooks, sidecar). Exposed for
/// integration tests.
pub fn router(state: AppState) -> Router {
    router_for(state, Ingress::Loopback)
}

/// Bind and serve.
///
/// Always serves plain HTTP on `127.0.0.1:<port>` — this loopback endpoint is
/// what the desktop webview, the hook plugin and the local sidecar use. When
/// the configured host is non-loopback it additionally serves TLS on the LAN
/// addresses (same port) for phones (`listeners::start_lan`). Also starts the
/// background subsystems (each owner's `start` function).
pub async fn serve(state: AppState) -> anyhow::Result<()> {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    // Establish Claude's own sign-in before anything can depend on it.
    let auth = state.refresh_claude_auth().await;
    if !auth.logged_in {
        tracing::warn!(status = %auth.summary(), "Claude is not signed in");
    }

    tokio::spawn(monitor::run(state.clone()));
    tokio::spawn(crate::bridge::autostart(state.clone()));
    hooks::start(&state);
    push::start(&state);

    listeners::start_lan(&state).await;

    announce(&state).await;

    // Primary: loopback HTTP. Keeps serve() alive for the app's lifetime.
    let port = state.config.port;
    let loopback = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let listener = tokio::net::TcpListener::bind(loopback)
        .await
        .with_context(|| format!("could not bind {loopback}"))?;
    remote::start(&state);
    let app = router_for(state.clone(), Ingress::Loopback);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .context("loopback server error")?;
    Ok(())
}

/// Log the URLs to the console at startup. Tokens are shown only as a
/// fingerprint; the desktop webview gets the real one through `server_info`.
async fn announce(state: &AppState) {
    let fingerprint = state
        .fingerprint
        .read()
        .await
        .clone()
        .unwrap_or_else(|| "n/a (http)".to_string());
    let port = state.config.port;
    tracing::info!(
        token = %auth::fingerprint(&state.auth.token),
        "Mother Claude server listening"
    );
    println!("\n  Mother Claude dashboard");
    println!("  Local:  http://127.0.0.1:{port}");
    if state.config.is_non_loopback() {
        for ip in tls::local_ips() {
            println!("  LAN:    https://{ip}:{port}  (pair from Remote in the app)");
        }
        println!("  TLS fingerprint (SHA-256): {fingerprint}");
    }
    println!(
        "  Token:  {} (the desktop app holds it)",
        auth::fingerprint(&state.auth.token)
    );
    let claude_auth = state.claude_auth.read().await.clone();
    println!("  Claude: {}", claude_auth.summary());
    if !claude_auth.logged_in {
        println!("          until then every turn will answer \"Not logged in\"");
    }
    if state.bridge.config.enabled {
        let running = state.bridge.is_running().await;
        println!(
            "  API:    http://127.0.0.1:{port}/v1  ({})",
            if running {
                "running".to_string()
            } else {
                "not started — start it on the API screen".to_string()
            }
        );
    }
    println!();
}
