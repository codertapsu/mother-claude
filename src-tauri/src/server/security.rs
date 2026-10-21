//! Per-listener HTTP hardening (design §2.9, §10 B1).
//!
//! [`apply`] wraps every listener's router (called by `server::router_for`).
//! The layers run INSIDE the context middleware, so they read the request's
//! [`ClientContext`] (and the [`AppState`] it stores) from the extensions.
//! From the outside in:
//!
//! 1. **Trace** — a span with method and path only: never the query string
//!    (it may carry a local `?token=`) and never a header.
//! 2. **CORS** — permissive only for `local` requests (the desktop webview's
//!    `tauri://localhost` / dev-server origins); same-origin for everything
//!    else. The bridge port has its own any-origin CORS (`bridge`).
//! 3. **No compression for secrets** — `/api/pair/*` and `/api/me` responses
//!    are never compressed (no length oracle next to a token).
//! 4. **Compression** — gzip / br for the SPA and JSON (not SSE or images).
//! 5. **Security headers** — CSP (`script-src 'self'`, `connect-src 'self'`
//!    plus the exact public/LAN `wss://` origins from settings, never from the
//!    request), `frame-ancestors 'none'`, nosniff, `Referrer-Policy:
//!    no-referrer`, `Cache-Control: no-store` on the API, HSTS when the last
//!    `X-Forwarded-Proto` is https. A handler's own CSP (the media route's
//!    `sandbox`) is kept.
//! 6. **Route policy** — the tunnel listener requires the last
//!    `X-Forwarded-Proto` to be `https`; any request whose effective ingress is
//!    `Tunnel` (a proxied request on 6725) never reaches `/v1` or `/hooks/*`.
//! 7. **Audit** — every state-changing request by a non-desktop principal is
//!    appended to `audit.jsonl` and the device's recent actions; a device that
//!    passed a dangerous gate raises a desktop notice.

use axum::body::Body;
use axum::extract::Request;
use axum::http::header::{self, HeaderName, HeaderValue};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Router;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::trace::TraceLayer;

use super::context::{last_forwarded, ClientContext, Ingress, Principal};
use super::devices::audit::AuditEntry;
use super::devices::RecentAction;
use super::error::ApiError;
use crate::claude::model::NoticeLevel;
use crate::state::AppState;

/// The SPA's Content-Security-Policy, before the extra `wss:` origins.
/// `base-uri 'self'`: the Angular shell has `<base href="/">`, which
/// `'none'` would block; a base on another origin stays forbidden.
pub const CSP_BASE: &str = "default-src 'self'; script-src 'self'; \
    style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; \
    object-src 'none'; base-uri 'self'; frame-ancestors 'none'; form-action 'self'; \
    manifest-src 'self'; worker-src 'self'";

/// The bridge's bundled docs/console (served under `/v1` on the main port):
/// Swagger UI may evaluate code, so it gets its own policy.
const CSP_BRIDGE_PAGES: &str = "default-src 'self'; script-src 'self' 'unsafe-eval'; \
    style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; \
    object-src 'none'; base-uri 'none'; frame-ancestors 'none'";

const PERMISSIONS_POLICY: &str = "camera=(self), microphone=(), geolocation=(), payment=(), usb=()";

/// Apply the hardening layers for `ingress`.
pub fn apply(router: Router, ingress: Ingress) -> Router {
    router
        .layer(axum::middleware::from_fn(audit))
        .layer(axum::middleware::from_fn(
            move |req: Request, next: Next| route_policy(ingress, req, next),
        ))
        .layer(axum::middleware::from_fn(security_headers))
        .layer(CompressionLayer::new())
        .layer(axum::middleware::from_fn(no_compression_for_secrets))
        .layer(cors())
        .layer(
            TraceLayer::new_for_http().make_span_with(|req: &axum::http::Request<Body>| {
                // Path only: a query string can carry a (local) token.
                tracing::info_span!("http", method = %req.method(), path = %req.uri().path())
            }),
        )
}

fn ctx_of(req: &Request) -> ClientContext {
    req.extensions()
        .get::<ClientContext>()
        .cloned()
        .unwrap_or_else(ClientContext::unknown)
}

/// CORS: any origin, but only for requests the context calls `local` (whose
/// Origin is by definition a local one). Everyone else gets no CORS headers:
/// same-origin only.
fn cors() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|_origin, parts| {
            parts
                .extensions
                .get::<ClientContext>()
                .is_some_and(|c| c.local)
        }))
        .allow_methods(Any)
        .allow_headers(Any)
        .expose_headers(Any)
        .max_age(std::time::Duration::from_secs(600))
}

async fn no_compression_for_secrets(mut req: Request, next: Next) -> Response {
    let path = req.uri().path();
    if path.starts_with("/api/pair/") || path == "/api/me" {
        req.headers_mut().remove(header::ACCEPT_ENCODING);
    }
    next.run(req).await
}

/// The CSP for a response: the SPA policy with `connect-src 'self'` plus the
/// exact `wss://` origins of the public URL and the LAN listeners.
pub fn csp_for(state: Option<&AppState>) -> String {
    let mut connect = vec!["'self'".to_string()];
    if let Some(st) = state {
        for o in st.remote.ws_origins() {
            if !connect.contains(&o) {
                connect.push(o);
            }
        }
    }
    format!("{CSP_BASE}; connect-src {}", connect.join(" "))
}

/// The bundled reference or console of an API server: under `/v1`, or under
/// `/servers/<id>/v1` (docs/ACCOUNTS.md §13.5).
fn is_bridge_page(path: &str) -> bool {
    let rest = match path.strip_prefix("/servers/") {
        Some(rest) => match rest.split_once('/') {
            Some((_, tail)) => tail,
            None => return false,
        },
        None => path.trim_start_matches('/'),
    };
    rest.starts_with("v1/docs") || rest.starts_with("v1/example")
}

/// A path that reaches an API server: `/v1…` or `/servers/…`.
fn is_bridge_path(path: &str) -> bool {
    path == "/v1" || path.starts_with("/v1/") || path.starts_with("/servers/")
}

fn set_if_absent(resp: &mut Response, name: HeaderName, value: &str) {
    if resp.headers().contains_key(&name) {
        return;
    }
    if let Ok(v) = HeaderValue::from_str(value) {
        resp.headers_mut().insert(name, v);
    }
}

async fn security_headers(req: Request, next: Next) -> Response {
    let ctx = ctx_of(&req);
    let state = req.extensions().get::<AppState>().cloned();
    let path = req.uri().path().to_string();
    let https = last_forwarded(req.headers(), "x-forwarded-proto")
        .is_some_and(|p| p.eq_ignore_ascii_case("https"));
    let mut resp = next.run(req).await;

    let is_api = path.starts_with("/api/") || path == "/api" || path == "/ws";
    let is_bridge_page = is_bridge_page(&path);
    let csp = if is_bridge_page {
        CSP_BRIDGE_PAGES.to_string()
    } else {
        csp_for(state.as_ref())
    };
    set_if_absent(&mut resp, header::CONTENT_SECURITY_POLICY, &csp);
    set_if_absent(&mut resp, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set_if_absent(&mut resp, header::REFERRER_POLICY, "no-referrer");
    set_if_absent(&mut resp, header::X_FRAME_OPTIONS, "DENY");
    set_if_absent(
        &mut resp,
        HeaderName::from_static("permissions-policy"),
        PERMISSIONS_POLICY,
    );
    if is_api {
        set_if_absent(&mut resp, header::CACHE_CONTROL, "no-store");
    } else if resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.starts_with("text/html"))
    {
        // The SPA shell must never be served stale after an update.
        set_if_absent(&mut resp, header::CACHE_CONTROL, "no-cache");
    }
    if !ctx.local {
        set_if_absent(
            &mut resp,
            HeaderName::from_static("cross-origin-opener-policy"),
            "same-origin",
        );
        set_if_absent(
            &mut resp,
            HeaderName::from_static("cross-origin-resource-policy"),
            "same-origin",
        );
        if https && ctx.proxied {
            set_if_absent(
                &mut resp,
                header::STRICT_TRANSPORT_SECURITY,
                "max-age=31536000",
            );
        }
    }
    resp
}

async fn route_policy(ingress: Ingress, req: Request, next: Next) -> Response {
    let ctx = ctx_of(&req);
    if ingress == Ingress::Tunnel {
        let https = last_forwarded(req.headers(), "x-forwarded-proto")
            .is_some_and(|p| p.eq_ignore_ascii_case("https"));
        if !https {
            return ApiError::new(
                StatusCode::BAD_REQUEST,
                "https_required",
                "The Internet-access port only accepts requests forwarded over HTTPS \
                 (X-Forwarded-Proto: https). Open the https:// URL your tunnel prints, and point \
                 the tunnel at http://127.0.0.1 on this port — e.g. `ngrok http \
                 127.0.0.1:6726`.",
            )
            .into_response();
        }
    }
    if ctx.is_tunneled() {
        let path = req.uri().path();
        if is_bridge_path(path) || path.starts_with("/hooks/") {
            return ApiError::not_found(
                "not_found",
                "This endpoint is not available through a tunnel or proxy.",
            )
            .into_response();
        }
    }
    next.run(req).await
}

/// Methods that change state.
fn mutating(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

async fn audit(req: Request, next: Next) -> Response {
    let ctx = ctx_of(&req);
    let state = req.extensions().get::<AppState>().cloned();
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let resp = next.run(req).await;

    let Some(state) = state else { return resp };
    let audited = matches!(
        ctx.principal,
        Principal::Device { .. } | Principal::LocalScript | Principal::Anonymous
    );
    if !audited || !mutating(&method) {
        return resp;
    }
    // Anonymous callers are recorded only where they can change something
    // (pairing); everything else answered them 401 already.
    if ctx.principal == Principal::Anonymous && !path.starts_with("/api/pair/") {
        return resp;
    }
    let status = resp.status();
    let dangerous = ctx.danger.get().filter(|_| status.is_success());
    let path_short: String = path.chars().take(200).collect();
    let device = ctx
        .device_id()
        .and_then(|id| state.devices.get(id))
        .map(|d| (d.id, d.name));
    state.devices.audit.record(AuditEntry {
        method: Some(method.to_string()),
        path: Some(path_short.clone()),
        status: Some(status.as_u16()),
        dangerous: dangerous.clone(),
        device_name: device.as_ref().map(|d| d.1.clone()),
        ..AuditEntry::from_ctx(&ctx, "http")
    });
    if let Some((id, name)) = device {
        state.devices.note_action(
            &id,
            RecentAction {
                at: chrono::Utc::now().timestamp_millis(),
                action: format!("{method} {path_short}"),
                status: Some(status.as_u16()),
                ip: Some(ctx.client_ip.to_string()),
                ingress: Some(ctx.effective_ingress.as_str().to_string()),
                dangerous: dangerous.is_some(),
            },
        );
        if let Some(what) = dangerous {
            crate::server::ws::publish_desktop_notice(
                &state,
                NoticeLevel::Warning,
                format!(
                    "\"{name}\" performed a dangerous action from {} ({}): {what}",
                    ctx.client_ip,
                    ctx.effective_ingress.as_str()
                ),
            );
        }
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csp_is_strict_and_lists_only_known_origins() {
        let csp = csp_for(None);
        assert!(csp.contains("script-src 'self';"));
        assert!(!csp.contains("script-src 'self' 'unsafe"));
        assert!(csp.contains("style-src 'self' 'unsafe-inline'"));
        assert!(csp.contains("frame-ancestors 'none'"));
        assert!(csp.ends_with("connect-src 'self'"));
        assert!(!csp.contains("ws:"));
        assert!(!csp.contains("wss:"), "no scheme-wide websocket allowance");
    }

    #[test]
    fn every_api_servers_pages_get_the_bridge_policy() {
        for page in [
            "/v1/docs/",
            "/v1/example/app.js",
            "/servers/work/v1/docs/",
            "/servers/work/v1/example/",
        ] {
            assert!(is_bridge_page(page), "{page}");
        }
        for other in [
            "/",
            "/docs/",
            "/servers/work",
            "/servers/v1/docs",
            "/api/v1/docs",
        ] {
            assert!(!is_bridge_page(other), "{other}");
        }
        for bridge in [
            "/v1",
            "/v1/threads",
            "/servers/work/v1/threads",
            "/servers/x",
        ] {
            assert!(is_bridge_path(bridge), "{bridge}");
        }
        assert!(!is_bridge_path("/v10") && !is_bridge_path("/api/servers/x"));
    }

    #[test]
    fn only_mutations_are_audited() {
        assert!(mutating(&Method::POST));
        assert!(mutating(&Method::DELETE));
        assert!(mutating(&Method::PATCH));
        assert!(!mutating(&Method::GET));
        assert!(!mutating(&Method::OPTIONS));
    }
}
