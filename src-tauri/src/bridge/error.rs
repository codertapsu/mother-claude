//! The bridge's single error envelope.
//!
//! Every failure is serialized as `{"error":{"code","message",…context}}` — the
//! same shape (and, where the concept exists on both sides, the same `code`
//! strings and HTTP statuses) as the Codex HTTP bridge, so a client written
//! against that service can be repointed here without rewriting its error
//! handling.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Map, Value};

/// A bridge failure: an HTTP status, a stable machine-readable `code`, a human
/// message, and optional context fields flattened alongside them (`thread_id`,
/// `request_id`, `field`, …).
#[derive(Debug, thiserror::Error)]
#[error("{code}: {message}")]
pub struct BridgeError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub extra: Map<String, Value>,
}

pub type BridgeResult<T> = Result<T, BridgeError>;

impl BridgeError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            extra: Map::new(),
        }
    }

    /// Attach one context field (chainable). Values that are `Null` are dropped
    /// so callers can pass `Option`s without emitting `"thread_id": null`.
    #[must_use]
    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        let value = value.into();
        if !value.is_null() {
            self.extra.insert(key.to_string(), value);
        }
        self
    }

    pub fn body(&self) -> Value {
        let mut error = Map::new();
        error.insert("code".into(), json!(self.code));
        error.insert("message".into(), json!(self.message));
        for (k, v) in &self.extra {
            error.insert(k.clone(), v.clone());
        }
        json!({ "error": Value::Object(error) })
    }

    // --- 4xx -------------------------------------------------------------

    pub fn invalid_json(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_json", message)
    }

    pub fn invalid_multipart(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_multipart", message)
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }

    /// Claude itself is not signed in (distinct from a missing bridge token).
    pub fn login_required(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "claude_login_required", message)
    }

    pub fn forbidden(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, code, message)
    }

    pub fn not_found(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, code, message)
    }

    pub fn thread_not_found(id: &str) -> Self {
        Self::not_found("thread_not_found", format!("No such conversation: {id}"))
            .with("thread_id", id)
    }

    pub fn operation_not_found(id: &str) -> Self {
        Self::not_found(
            "operation_not_found",
            "Operation expired or belongs to a previous server process.",
        )
        .with("operation_id", id)
    }

    pub fn request_not_found(id: &str) -> Self {
        Self::not_found("request_not_found", "No such pending request.").with("request_id", id)
    }

    pub fn conflict(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, code, message)
    }

    pub fn busy(message: impl Into<String>) -> Self {
        Self::conflict("busy", message)
    }

    /// The per-operation event log no longer retains the requested cursor.
    pub fn events_expired() -> Self {
        Self::new(
            StatusCode::GONE,
            "events_expired",
            "The requested events are no longer retained; read the operation result instead.",
        )
    }

    pub fn too_large(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::PAYLOAD_TOO_LARGE, code, message)
    }

    pub fn content_type(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNSUPPORTED_MEDIA_TYPE, "content_type", message)
    }

    pub fn unprocessable(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, code, message)
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::unprocessable("invalid_request", message)
    }

    pub fn invalid_field(field: &str, message: impl Into<String>) -> Self {
        Self::unprocessable("invalid_field", message).with("field", field)
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::unprocessable("invalid_input", message)
    }

    pub fn invalid_cursor(message: impl Into<String>) -> Self {
        Self::unprocessable("invalid_cursor", message)
    }

    pub fn invalid_id(message: impl Into<String>) -> Self {
        Self::unprocessable("invalid_id", message)
    }

    // --- 5xx -------------------------------------------------------------

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
    }

    /// The Claude runtime (Agent SDK host, or the Messages API) failed.
    pub fn claude_error(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, "claude_error", message)
    }

    pub fn runtime_unavailable(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "runtime_unavailable",
            message,
        )
    }

    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new(StatusCode::GATEWAY_TIMEOUT, "timeout", message)
    }
}

impl IntoResponse for BridgeError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body())).into_response()
    }
}

impl From<anyhow::Error> for BridgeError {
    fn from(e: anyhow::Error) -> Self {
        BridgeError::claude_error(e.to_string())
    }
}

impl From<serde_json::Error> for BridgeError {
    fn from(e: serde_json::Error) -> Self {
        BridgeError::invalid_json(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_shape_matches_codex() {
        let e = BridgeError::thread_not_found("t-1");
        let body = e.body();
        assert_eq!(body["error"]["code"], "thread_not_found");
        assert_eq!(body["error"]["thread_id"], "t-1");
        assert!(body["error"]["message"].is_string());
        assert_eq!(e.status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn null_context_is_dropped() {
        let e = BridgeError::invalid_request("nope").with("turn_id", Value::Null);
        assert!(e.body()["error"].get("turn_id").is_none());
    }

    #[test]
    fn statuses_mirror_the_codex_table() {
        assert_eq!(BridgeError::busy("x").status, StatusCode::CONFLICT);
        assert_eq!(BridgeError::events_expired().status, StatusCode::GONE);
        assert_eq!(
            BridgeError::claude_error("x").status,
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(
            BridgeError::timeout("x").status,
            StatusCode::GATEWAY_TIMEOUT
        );
        assert_eq!(
            BridgeError::runtime_unavailable("x").status,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
