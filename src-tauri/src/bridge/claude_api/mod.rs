//! The Claude API, answered by Claude Code (docs/CLAUDE-API.md).
//!
//! Every API server serves the Claude API's own endpoints beside the
//! bridge's — `POST /v1/messages`, `POST /v1/messages/count_tokens`, `GET
//! /v1/models` and `GET /v1/models/{id}` — so a client written for
//! `https://api.anthropic.com` works here with only its base URL changed:
//! the same request bodies, responses, streamed events and errors. Claude
//! Code on this computer answers, signed in as the server's account: no API
//! key of Anthropic's is needed, and none a client sends is ever used, kept
//! or passed on (a server set to `messagesBackend: anthropic` sends requests
//! to the Anthropic API with the key Mother Claude was given instead).
//!
//! Beyond the API: conversations a client can name and continue
//! (`x-conversation-id`; `GET`/`DELETE /v1/conversations`), clients told
//! apart and kept in a folder each ([`store`]), and who may call a server —
//! from this computer, the local network or anywhere, with a key or without
//! ([`access`], [`network`]). The dashboard manages clients ([`admin`]).

pub mod access;
pub mod admin;
pub mod convert;
pub mod extras;
pub mod network;
pub mod routes;
pub mod store;
pub mod stream;
pub mod sweep;
pub mod wire;

use std::path::PathBuf;

use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use serde_json::Value;

use crate::bridge::control::BridgeDefaults;
use crate::state::AppState;

/// The header a server that groups clients by it reads (`groupClientsBy:
/// header`).
pub const CLIENT_HEADER: &str = "x-client-id";
/// `messagesBackend`: Claude Code answers (the default).
pub const BACKEND_CLAUDE_CODE: &str = "claude-code";
/// `messagesBackend`: the Anthropic API answers, with Mother Claude's key.
pub const BACKEND_ANTHROPIC: &str = "anthropic";
pub const BACKENDS: [&str; 2] = [BACKEND_CLAUDE_CODE, BACKEND_ANTHROPIC];
/// The folder under the app data dir a server without its own keeps its
/// clients in.
pub const DEFAULT_DIR: &str = "claude-api";

/// Where a server with these defaults keeps its clients' folders.
pub fn root_for(state: &AppState, defaults: &BridgeDefaults) -> Option<PathBuf> {
    defaults
        .api_data_dir
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| state.bridge.api.default_root.clone())
}

/// Whether a request speaks the Claude API: its errors then come in that
/// API's envelope. Its own endpoints do, and any request with an
/// `anthropic-version` header (the API's SDKs send one with every call).
pub fn speaks_claude_api(path: &str, headers: &HeaderMap) -> bool {
    headers.contains_key("anthropic-version")
        || path.ends_with("/messages")
        || path.ends_with("/messages/count_tokens")
}

/// The Claude API's error type for a status.
pub fn error_type(status: u16) -> &'static str {
    match status {
        413 => "request_too_large",
        400 | 405 | 415 | 422 => "invalid_request_error",
        401 => "authentication_error",
        402 => "billing_error",
        403 => "permission_error",
        404 | 410 => "not_found_error",
        409 => "conflict_error",
        429 => "rate_limit_error",
        504 => "timeout_error",
        529 => "overloaded_error",
        _ => "api_error",
    }
}

/// Middleware: for a request that speaks the Claude API, a bridge error
/// (`{"error":{"code","message"}}`) — not started, no key, an unknown path —
/// becomes that API's (`{"type":"error","error":{"type","message"},
/// "request_id"}`), its status as the API would answer.
pub async fn envelope(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let speaks = speaks_claude_api(req.uri().path(), req.headers());
    let response = next.run(req).await;
    if !speaks {
        return response;
    }
    in_envelope(response).await
}

/// [`envelope`] for a listener that serves the Claude API alone (the LAN's,
/// [`lan`]): every error in the API's envelope.
pub async fn envelope_always(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    in_envelope(next.run(req).await).await
}

/// A bridge error response as the Claude API's; anything else as it is.
async fn in_envelope(response: Response) -> Response {
    if response.status().as_u16() < 400 {
        return response;
    }
    let json = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|c| c.starts_with("application/json"));
    if !json || response.headers().contains_key("request-id") {
        return response;
    }
    let (parts, body) = response.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, 256 * 1024).await else {
        return Response::from_parts(parts, axum::body::Body::empty());
    };
    let Some(error) = serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|v| v.get("error").cloned())
        .filter(|e| e.get("code").is_some())
    else {
        return Response::from_parts(parts, axum::body::Body::from(bytes));
    };
    let mut status = parts.status;
    if matches!(status.as_u16(), 415 | 422) {
        status = axum::http::StatusCode::BAD_REQUEST;
    }
    if status.as_u16() == 410 {
        status = axum::http::StatusCode::NOT_FOUND;
    }
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("The request failed.")
        .to_string();
    let mut converted = wire::ApiError::new(status, error_type(parts.status.as_u16()), message);
    if let Some(code) = error.get("code").and_then(Value::as_str) {
        converted = converted.with_code(code);
    }
    let mut converted = converted.respond(&wire::request_id());
    // What the bridge said besides (Retry-After, CORS) stays.
    for (k, v) in parts.headers.iter() {
        if k != axum::http::header::CONTENT_TYPE && k != axum::http::header::CONTENT_LENGTH {
            converted.headers_mut().insert(k.clone(), v.clone());
        }
    }
    converted.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_types_are_the_apis() {
        assert_eq!(error_type(400), "invalid_request_error");
        assert_eq!(error_type(413), "request_too_large");
        assert_eq!(error_type(401), "authentication_error");
        assert_eq!(error_type(403), "permission_error");
        assert_eq!(error_type(404), "not_found_error");
        assert_eq!(error_type(429), "rate_limit_error");
        assert_eq!(error_type(503), "api_error");
        assert_eq!(error_type(529), "overloaded_error");
    }

    #[test]
    fn claude_api_requests_are_recognised() {
        let mut h = HeaderMap::new();
        assert!(speaks_claude_api("/v1/messages", &h));
        assert!(speaks_claude_api("/messages/count_tokens", &h));
        assert!(!speaks_claude_api("/v1/threads", &h));
        h.insert("anthropic-version", "2023-06-01".parse().unwrap());
        assert!(speaks_claude_api("/v1/models", &h));
    }
}
