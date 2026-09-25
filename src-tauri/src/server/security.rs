//! Per-ingress HTTP hardening layers (design §2.9).
//!
//! Owner: **B1 (security/remote)**. The foundation applies CORS (permissive,
//! as before) and a `TraceLayer` whose span records method + path only (never
//! the query string, which may carry a local `?token=`). What B1 adds here:
//!
//! * security headers on Lan/Tunnel responses (and the SPA everywhere): CSP,
//!   `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`,
//!   `Cache-Control: no-store` on `/api`, HSTS when the last
//!   `X-Forwarded-Proto` is https;
//! * CORS permissive only for local requests (`ClientContext.local`),
//!   same-origin otherwise;
//! * compression (gzip, br — the tower-http features are already enabled);
//! * the Tunnel ingress `X-Forwarded-Proto == https` requirement.
//!
//! [`apply`] is called by `server::router_for` for every listener. Layers
//! added here run INSIDE the context middleware, so they can read the
//! request's `ClientContext` from its extensions.

use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use super::context::Ingress;

/// Apply the hardening layers for `ingress`.
pub fn apply(router: Router, _ingress: Ingress) -> Router {
    router
        .layer(CorsLayer::permissive())
        .layer(
            TraceLayer::new_for_http().make_span_with(|req: &axum::http::Request<_>| {
                // Path only: a query string can carry a (local) token.
                tracing::info_span!("http", method = %req.method(), path = %req.uri().path())
            }),
        )
}
