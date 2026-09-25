//! The dashboard API's one error shape: `{"error":{"code","message"}}`.
//!
//! Every `/api/*` and `/hooks/*` failure goes through [`ApiError`] so the SPA
//! (and scripts) parse exactly one envelope. The bridge keeps its own,
//! Codex-compatible `BridgeError` with the same outer shape.

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Map, Value};

/// A failed API call.
#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: String,
    pub message: String,
    /// Extra fields merged into the `error` object (e.g. `retryAfter`).
    pub extra: Map<String, Value>,
    /// Seconds for a `Retry-After` header (429).
    pub retry_after: Option<u64>,
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn new(status: StatusCode, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
            extra: Map::new(),
            retry_after: None,
        }
    }

    /// Attach an extra field to the error object.
    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.extra.insert(key.to_string(), value.into());
        self
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    /// 401: no (valid) credential.
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }

    /// 403 with a specific code (`desktop_only`, `dangerous_blocked`, …).
    pub fn forbidden(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, code, message)
    }

    /// The standard dangerous-action refusal.
    pub fn dangerous_blocked(message: impl Into<String>) -> Self {
        Self::forbidden("dangerous_blocked", message)
    }

    pub fn not_found(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, code, message)
    }

    pub fn no_such_session(id: &str) -> Self {
        Self::not_found("session_not_found", format!("No session {id}."))
    }

    pub fn conflict(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, code, message)
    }

    /// 422: well-formed but refused on its merits (bad field value, CLI error).
    pub fn unprocessable(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, code, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", message)
    }

    /// 501 for routes registered by the foundation whose owner has not filled
    /// them in yet. `what` names the feature, e.g. "pairing codes (B1)".
    pub fn not_implemented(what: impl Into<String>) -> Self {
        Self::new(
            StatusCode::NOT_IMPLEMENTED,
            "not_implemented",
            format!("{} is not implemented yet.", what.into()),
        )
    }

    /// 429 with `Retry-After`.
    pub fn too_many_requests(retry_after_secs: u64, message: impl Into<String>) -> Self {
        let mut e = Self::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited", message);
        e.retry_after = Some(retry_after_secs.max(1));
        e.with("retryAfter", retry_after_secs.max(1))
    }

    /// The JSON body.
    pub fn body(&self) -> Value {
        let mut error = Map::new();
        error.insert("code".into(), Value::String(self.code.clone()));
        error.insert("message".into(), Value::String(self.message.clone()));
        for (k, v) in &self.extra {
            error.insert(k.clone(), v.clone());
        }
        json!({ "error": Value::Object(error) })
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({}): {}", self.code, self.status, self.message)
    }
}

impl std::error::Error for ApiError {}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut resp = (self.status, axum::Json(self.body())).into_response();
        if let Some(secs) = self.retry_after {
            if let Ok(v) = HeaderValue::from_str(&secs.to_string()) {
                resp.headers_mut().insert(header::RETRY_AFTER, v);
            }
        }
        resp
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError::internal(e.to_string())
    }
}

/// The bridge's envelope has the same shape; keep its status, code and context.
impl From<crate::bridge::error::BridgeError> for ApiError {
    fn from(e: crate::bridge::error::BridgeError) -> Self {
        let mut out = ApiError::new(e.status, e.code, e.message);
        out.extra = e.extra;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_shape() {
        let e = ApiError::conflict("already_decided", "too late").with("requestId", "r1");
        let b = e.body();
        assert_eq!(b["error"]["code"], "already_decided");
        assert_eq!(b["error"]["message"], "too late");
        assert_eq!(b["error"]["requestId"], "r1");
    }

    #[test]
    fn rate_limit_sets_retry_after() {
        let resp = ApiError::too_many_requests(30, "slow down").into_response();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(resp.headers()[header::RETRY_AFTER], "30");
    }
}
