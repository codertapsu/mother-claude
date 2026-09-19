//! The direct Messages API backend.
//!
//! The Agent SDK gives you Claude Code — a coding agent with tools, a working
//! directory and an approval loop. It does not give you the model-level
//! features of `POST /v1/messages`: the Files API, programmatic tool calling,
//! server tools, structured outputs on arbitrary schemas, or the
//! `transformations` control that keeps vision coordinates in the original
//! pixel space. Those need `api.anthropic.com` directly, so the bridge offers
//! both and lets the caller pick per request.
//!
//! This path needs an API key (`ANTHROPIC_API_KEY`). Claude Code's own
//! subscription OAuth credentials are not usable here, so when no key is
//! configured every route in this module answers `503` with instructions rather
//! than failing somewhere less obvious.

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde_json::{json, Value};

use super::error::{BridgeError, BridgeResult};

/// The API version header every request carries.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Model used when a caller does not name one.
pub const DEFAULT_MODEL: &str = "claude-opus-5";
/// `max_tokens` default for a non-streaming request — low enough to stay under
/// HTTP timeouts, high enough not to truncate ordinary answers.
pub const DEFAULT_MAX_TOKENS: u64 = 16_000;
/// `max_tokens` default when streaming, where timeouts are not a concern.
pub const DEFAULT_MAX_TOKENS_STREAMING: u64 = 64_000;

/// A thin, faithful client for `api.anthropic.com`.
#[derive(Debug, Clone)]
pub struct MessagesClient {
    api_key: Option<String>,
    base_url: String,
    http: reqwest::Client,
}

impl MessagesClient {
    pub fn new(api_key: Option<String>) -> Self {
        let base_url = std::env::var("ANTHROPIC_BASE_URL")
            .ok()
            .map(|v| v.trim().trim_end_matches('/').to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "https://api.anthropic.com".to_string());
        let http = reqwest::Client::builder()
            // Long, because a high-effort non-streaming turn legitimately takes
            // minutes. Streaming callers are not affected by this at all.
            .timeout(Duration::from_secs(1800))
            .build()
            .unwrap_or_default();
        Self {
            api_key,
            base_url,
            http,
        }
    }

    pub fn is_configured(&self) -> bool {
        self.api_key.is_some()
    }

    fn key(&self) -> BridgeResult<&str> {
        self.api_key.as_deref().ok_or_else(|| {
            BridgeError::new(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "messages_api_unconfigured",
                "The direct Messages API path needs an API key. Set ANTHROPIC_API_KEY \
                 before starting Mother Claude. Conversation endpoints (/threads, /chat) \
                 work without one — they use your Claude Code sign-in.",
            )
        })
    }

    fn headers(&self, betas: Option<&str>) -> BridgeResult<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-api-key",
            HeaderValue::from_str(self.key()?).map_err(|_| {
                BridgeError::internal("The configured API key is not a valid header.")
            })?,
        );
        headers.insert(
            "anthropic-version",
            HeaderValue::from_static(ANTHROPIC_VERSION),
        );
        if let Some(betas) = betas.filter(|b| !b.trim().is_empty()) {
            headers.insert(
                "anthropic-beta",
                HeaderValue::from_str(betas).map_err(|_| {
                    BridgeError::invalid_field("betas", "`betas` must be header-safe ASCII.")
                })?,
            );
        }
        Ok(headers)
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}/v1/{}", self.base_url, path.trim_start_matches('/'))
    }

    /// `POST /v1/messages`, non-streaming.
    pub async fn create_message(&self, body: &Value, betas: Option<&str>) -> BridgeResult<Value> {
        let response = self
            .http
            .post(self.url("messages"))
            .headers(self.headers(betas)?)
            .header(CONTENT_TYPE, "application/json")
            .json(body)
            .send()
            .await
            .map_err(transport_error)?;
        read_json(response).await
    }

    /// `POST /v1/messages` with `stream: true`, returning the raw upstream
    /// response so the caller can proxy its SSE body verbatim.
    pub async fn stream_message(
        &self,
        body: &Value,
        betas: Option<&str>,
    ) -> BridgeResult<reqwest::Response> {
        let response = self
            .http
            .post(self.url("messages"))
            .headers(self.headers(betas)?)
            .header(CONTENT_TYPE, "application/json")
            .json(body)
            .send()
            .await
            .map_err(transport_error)?;
        if !response.status().is_success() {
            return Err(upstream_error(response.status(), read_body(response).await));
        }
        Ok(response)
    }

    /// `POST /v1/messages/count_tokens`.
    pub async fn count_tokens(&self, body: &Value, betas: Option<&str>) -> BridgeResult<Value> {
        let response = self
            .http
            .post(self.url("messages/count_tokens"))
            .headers(self.headers(betas)?)
            .header(CONTENT_TYPE, "application/json")
            .json(body)
            .send()
            .await
            .map_err(transport_error)?;
        read_json(response).await
    }

    /// `POST /v1/files` — multipart upload.
    pub async fn upload_file(
        &self,
        filename: &str,
        mime: &str,
        bytes: Vec<u8>,
        expires_in_seconds: Option<u64>,
    ) -> BridgeResult<Value> {
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(filename.to_string())
            .mime_str(mime)
            .map_err(|_| {
                BridgeError::unprocessable(
                    "content_type",
                    format!("{mime:?} is not a valid media type."),
                )
            })?;
        let mut form = reqwest::multipart::Form::new().part("file", part);
        if let Some(expiry) = expires_in_seconds {
            form = form.text("expires_in_seconds", expiry.to_string());
        }
        let response = self
            .http
            .post(self.url("files"))
            .headers(self.headers(None)?)
            .multipart(form)
            .send()
            .await
            .map_err(transport_error)?;
        read_json(response).await
    }

    /// `GET /v1/files` (and, with a path suffix, one file's metadata).
    pub async fn get_json(&self, path: &str, query: &[(String, String)]) -> BridgeResult<Value> {
        let response = self
            .http
            .get(with_query(&self.url(path), query))
            .headers(self.headers(None)?)
            .send()
            .await
            .map_err(transport_error)?;
        read_json(response).await
    }

    /// `GET /v1/files/{id}/content` — raw bytes plus their media type.
    pub async fn get_bytes(&self, path: &str) -> BridgeResult<(Vec<u8>, String)> {
        let response = self
            .http
            .get(self.url(path))
            .headers(self.headers(None)?)
            .send()
            .await
            .map_err(transport_error)?;
        let status = response.status();
        let mime = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        if !status.is_success() {
            return Err(upstream_error(status, read_body(response).await));
        }
        let bytes = response.bytes().await.map_err(transport_error)?.to_vec();
        Ok((bytes, mime))
    }

    /// `DELETE /v1/files/{id}`.
    pub async fn delete(&self, path: &str) -> BridgeResult<Value> {
        let response = self
            .http
            .delete(self.url(path))
            .headers(self.headers(None)?)
            .send()
            .await
            .map_err(transport_error)?;
        read_json(response).await
    }
}

/// Append `?a=b&c=d`, percent-encoding anything that is not unreserved.
fn with_query(url: &str, params: &[(String, String)]) -> String {
    if params.is_empty() {
        return url.to_string();
    }
    let encoded: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
        .collect();
    format!("{url}?{}", encoded.join("&"))
}

fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn transport_error(e: reqwest::Error) -> BridgeError {
    if e.is_timeout() {
        BridgeError::timeout(format!("The Anthropic API did not answer in time: {e}"))
    } else {
        BridgeError::claude_error(format!("Could not reach the Anthropic API: {e}"))
    }
}

async fn read_body(response: reqwest::Response) -> Value {
    match response.text().await {
        Ok(text) => serde_json::from_str(&text).unwrap_or(Value::String(text)),
        Err(e) => Value::String(e.to_string()),
    }
}

async fn read_json(response: reqwest::Response) -> BridgeResult<Value> {
    let status = response.status();
    let body = read_body(response).await;
    if status.is_success() {
        Ok(body)
    } else {
        Err(upstream_error(status, body))
    }
}

/// Preserve the upstream status and error text rather than flattening every
/// API failure into one generic 502 — an `invalid_request_error` about a bad
/// schema is far more useful than "the runtime failed".
fn upstream_error(status: reqwest::StatusCode, body: Value) -> BridgeError {
    let message = body
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("The Anthropic API rejected the request.")
        .to_string();
    let kind = body
        .get("error")
        .and_then(|e| e.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("anthropic_error")
        .to_string();
    let status = axum::http::StatusCode::from_u16(status.as_u16())
        .unwrap_or(axum::http::StatusCode::BAD_GATEWAY);
    BridgeError::new(status, "anthropic_error", message)
        .with("anthropic_type", kind)
        .with("upstream", body)
}

/// Fill in the defaults a caller may reasonably omit, without overriding
/// anything they set.
pub fn apply_defaults(body: &mut Value) {
    let streaming = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    if let Some(obj) = body.as_object_mut() {
        obj.entry("model").or_insert_with(|| json!(DEFAULT_MODEL));
        obj.entry("max_tokens").or_insert_with(|| {
            json!(if streaming {
                DEFAULT_MAX_TOKENS_STREAMING
            } else {
                DEFAULT_MAX_TOKENS
            })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unconfigured_client_explains_itself() {
        let client = MessagesClient::new(None);
        assert!(!client.is_configured());
        let err = client.key().unwrap_err();
        assert_eq!(err.code, "messages_api_unconfigured");
        assert_eq!(err.status, axum::http::StatusCode::SERVICE_UNAVAILABLE);
        assert!(err.message.contains("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn headers_carry_the_key_version_and_betas() {
        let client = MessagesClient::new(Some("sk-test".into()));
        let headers = client.headers(Some("files-api-2025-04-14")).unwrap();
        assert_eq!(headers["x-api-key"], "sk-test");
        assert_eq!(headers["anthropic-version"], ANTHROPIC_VERSION);
        assert_eq!(headers["anthropic-beta"], "files-api-2025-04-14");

        // Blank betas are omitted rather than sent empty.
        let headers = client.headers(Some("   ")).unwrap();
        assert!(!headers.contains_key("anthropic-beta"));
    }

    #[test]
    fn urls_are_built_under_v1_regardless_of_leading_slash() {
        let client = MessagesClient::new(Some("k".into()));
        assert_eq!(
            client.url("messages"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(client.url("/files"), "https://api.anthropic.com/v1/files");
    }

    #[test]
    fn defaults_fill_gaps_without_overriding() {
        let mut body = json!({ "messages": [] });
        apply_defaults(&mut body);
        assert_eq!(body["model"], DEFAULT_MODEL);
        assert_eq!(body["max_tokens"], DEFAULT_MAX_TOKENS);

        let mut body = json!({ "messages": [], "stream": true });
        apply_defaults(&mut body);
        assert_eq!(body["max_tokens"], DEFAULT_MAX_TOKENS_STREAMING);

        let mut body = json!({ "model": "claude-haiku-4-5", "max_tokens": 10 });
        apply_defaults(&mut body);
        assert_eq!(body["model"], "claude-haiku-4-5");
        assert_eq!(body["max_tokens"], 10);
    }

    #[test]
    fn query_strings_are_encoded_not_concatenated() {
        assert_eq!(with_query("http://x/files", &[]), "http://x/files");
        assert_eq!(
            with_query(
                "http://x/files",
                &[
                    ("limit".into(), "20".into()),
                    ("page".into(), "a b&c".into())
                ],
            ),
            "http://x/files?limit=20&page=a%20b%26c"
        );
        assert_eq!(percent_encode("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn upstream_errors_keep_their_status_and_detail() {
        let err = upstream_error(
            reqwest::StatusCode::BAD_REQUEST,
            json!({ "error": { "type": "invalid_request_error", "message": "bad tools" } }),
        );
        assert_eq!(err.status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "anthropic_error");
        assert_eq!(err.message, "bad tools");
        assert_eq!(
            err.body()["error"]["anthropic_type"],
            "invalid_request_error"
        );
        assert!(err.body()["error"]["upstream"].is_object());

        // A non-JSON upstream body still produces a usable envelope.
        let err = upstream_error(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            Value::String("<html>oops".into()),
        );
        assert_eq!(err.status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        assert!(err.message.contains("rejected"));
    }
}
