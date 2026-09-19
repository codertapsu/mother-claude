//! HTTP handlers for the bridge, plus the extractors and helpers they share.

pub mod messages;
pub mod meta;
pub mod requests;
pub mod threads;
pub mod turns;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{FromRequest, Request};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use super::error::{BridgeError, BridgeResult};
use super::events::EventLog;

/// How long a stream waits before emitting a keep-alive.
const HEARTBEAT: Duration = Duration::from_secs(15);

/// Longest identifier accepted in a path segment, matching the Codex bridge.
const MAX_ID_LEN: usize = 200;

/// A JSON request body, with the bridge's error envelope on every failure.
///
/// `axum::Json`'s own rejections are plain text, which would make malformed
/// input the one case where a client has to parse something other than
/// `{"error":{…}}`.
pub struct Body<T>(pub T);

impl<T, S> FromRequest<S> for Body<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = BridgeError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let content_type = req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let bytes = axum::body::Bytes::from_request(req, state)
            .await
            .map_err(|e| match e.status() {
                StatusCode::PAYLOAD_TOO_LARGE => BridgeError::too_large(
                    "request_size",
                    format!(
                        "The request body exceeds the {} MiB limit.",
                        super::MAX_BODY_BYTES / (1024 * 1024)
                    ),
                ),
                _ => BridgeError::invalid_json(format!("Could not read the request body: {e}")),
            })?;

        // An absent body means "no options", which every optional-bodied
        // endpoint (interrupt, close, …) should accept.
        if bytes.is_empty() {
            return serde_json::from_slice(b"{}")
                .map(Body)
                .map_err(|_| BridgeError::invalid_request("This endpoint requires a JSON body."));
        }

        if !content_type.is_empty()
            && !content_type
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("application/json")
        {
            return Err(BridgeError::content_type(format!(
                "Expected application/json, got {content_type:?}."
            )));
        }

        serde_json::from_slice(&bytes).map(Body).map_err(|e| {
            let message = e.to_string();
            if message.starts_with("unknown field") {
                BridgeError::unprocessable("unknown_field", message)
            } else if e.classify() == serde_json::error::Category::Data {
                BridgeError::unprocessable("invalid_field", message)
            } else {
                BridgeError::invalid_json(message)
            }
        })
    }
}

/// Reject identifiers that could not possibly be real, before they reach the
/// runtime. Path segments are already percent-decoded by axum.
pub fn validate_id(kind: &str, id: &str) -> BridgeResult<()> {
    if id.trim().is_empty() {
        return Err(BridgeError::invalid_id(format!(
            "{kind} must not be blank."
        )));
    }
    if id.len() > MAX_ID_LEN {
        return Err(BridgeError::invalid_id(format!(
            "{kind} must be at most {MAX_ID_LEN} characters."
        )));
    }
    if id.contains('/') || id.contains('\\') {
        return Err(BridgeError::invalid_id(format!(
            "{kind} must not contain a path separator."
        )));
    }
    Ok(())
}

/// A `201`/`202` JSON response.
pub fn created(body: Value) -> Response {
    (StatusCode::CREATED, axum::Json(body)).into_response()
}

pub fn accepted(body: Value, operation_id: &str) -> Response {
    (
        StatusCode::ACCEPTED,
        [("x-operation-id", operation_id.to_string())],
        axum::Json(body),
    )
        .into_response()
}

pub fn ok(body: Value) -> Response {
    axum::Json(body).into_response()
}

/// Resolve an SSE cursor from `?after=`, then `Last-Event-ID`, then a default.
pub fn resolve_cursor(
    params: &HashMap<String, String>,
    headers: &HeaderMap,
    default: u64,
) -> BridgeResult<u64> {
    for (key, value) in params {
        if key != "after" && key != "token" {
            return Err(BridgeError::invalid_cursor(format!(
                "Unsupported query parameter {key:?}; only `after` is accepted."
            )));
        }
        let _ = value;
    }
    if let Some(after) = params.get("after") {
        return after.trim().parse::<u64>().map_err(|_| {
            BridgeError::invalid_cursor(format!("`after` must be a whole number, got {after:?}."))
        });
    }
    if let Some(value) = headers.get("last-event-id").and_then(|v| v.to_str().ok()) {
        if !value.trim().is_empty() {
            return value.trim().parse::<u64>().map_err(|_| {
                BridgeError::invalid_cursor(format!(
                    "Last-Event-ID must be a whole number, got {value:?}."
                ))
            });
        }
    }
    Ok(default)
}

/// Stream an event log as Server-Sent Events.
///
/// The cursor is validated *before* the response starts, so an expired one is a
/// normal JSON `410` the client can read rather than a stream that dies with no
/// explanation. Once streaming, an expiry can still happen (the writer outran a
/// slow reader); that is delivered as a terminal `bridge/error` frame.
pub async fn sse(log: Arc<EventLog>, cursor: u64) -> BridgeResult<Response> {
    // Validate now; discard the batch and let the stream replay it.
    log.read(cursor, Duration::ZERO).await?;

    let stream = futures_util::stream::unfold(Some(cursor), move |state| {
        let log = log.clone();
        async move {
            let cursor = state?;
            match log.read(cursor, HEARTBEAT).await {
                Ok(out) => {
                    let next = out.events.last().map(|e| e.seq).unwrap_or(cursor);
                    let frames: Vec<Result<Event, std::convert::Infallible>> = out
                        .events
                        .iter()
                        .map(|e| Ok(Event::default().id(e.seq.to_string()).data(&*e.data)))
                        .collect();
                    // A closed log with nothing left to send ends the stream.
                    let carry = if out.closed && next == cursor {
                        None
                    } else {
                        Some(next)
                    };
                    Some((frames, carry))
                }
                Err(err) => {
                    // Same shape as the terminal frame an operation emits, so a
                    // client reads `params.error` in both cases instead of
                    // rendering "failed" with no reason.
                    let frame = Event::default().data(
                        json!({
                            "method": "bridge/error",
                            "params": { "status": "failed", "error": err.body()["error"] },
                        })
                        .to_string(),
                    );
                    Some((vec![Ok(frame)], None))
                }
            }
        }
    })
    .flat_map(futures_util::stream::iter);

    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(HEARTBEAT).text("heartbeat"))
        .into_response())
}

/// Read a required string field with a typed error.
pub fn require_str<'a>(value: &'a Value, field: &str) -> BridgeResult<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| BridgeError::invalid_field(field, format!("`{field}` is required.")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn identifiers_are_bounded_and_slash_free() {
        assert!(validate_id("thread_id", "abc").is_ok());
        assert_eq!(
            validate_id("thread_id", "  ").unwrap_err().code,
            "invalid_id"
        );
        assert_eq!(
            validate_id("thread_id", &"x".repeat(MAX_ID_LEN + 1))
                .unwrap_err()
                .code,
            "invalid_id"
        );
        assert_eq!(
            validate_id("thread_id", "a/b").unwrap_err().code,
            "invalid_id"
        );
    }

    #[test]
    fn cursor_precedence_is_after_then_header_then_default() {
        let mut headers = HeaderMap::new();
        headers.insert("last-event-id", "7".parse().unwrap());

        assert_eq!(
            resolve_cursor(&params(&[("after", "3")]), &headers, 99).unwrap(),
            3
        );
        assert_eq!(resolve_cursor(&params(&[]), &headers, 99).unwrap(), 7);
        assert_eq!(
            resolve_cursor(&params(&[]), &HeaderMap::new(), 99).unwrap(),
            99
        );
        // The auth token may ride along in the query string for EventSource.
        assert_eq!(
            resolve_cursor(&params(&[("token", "abc")]), &HeaderMap::new(), 5).unwrap(),
            5
        );
    }

    #[test]
    fn bad_cursors_and_stray_query_params_are_422() {
        let empty = HeaderMap::new();
        assert_eq!(
            resolve_cursor(&params(&[("after", "soon")]), &empty, 0)
                .unwrap_err()
                .code,
            "invalid_cursor"
        );
        assert_eq!(
            resolve_cursor(&params(&[("limit", "5")]), &empty, 0)
                .unwrap_err()
                .code,
            "invalid_cursor"
        );

        let mut headers = HeaderMap::new();
        headers.insert("last-event-id", "abc".parse().unwrap());
        assert_eq!(
            resolve_cursor(&params(&[]), &headers, 0).unwrap_err().code,
            "invalid_cursor"
        );
    }

    #[test]
    fn required_fields_report_their_own_name() {
        let body = json!({ "message": "hi", "blank": "  " });
        assert_eq!(require_str(&body, "message").unwrap(), "hi");
        let err = require_str(&body, "blank").unwrap_err();
        assert_eq!(err.code, "invalid_field");
        assert_eq!(err.body()["error"]["field"], "blank");
    }

    #[tokio::test]
    async fn sse_rejects_an_expired_cursor_before_streaming() {
        let log = Arc::new(EventLog::new(1, super::super::events::MAX_BYTES));
        log.append(&json!({ "n": 1 }));
        log.append(&json!({ "n": 2 }));
        let err = sse(log, 0).await.unwrap_err();
        assert_eq!(err.code, "events_expired");
    }
}
