//! The direct Messages API routes.
//!
//! These are a faithful pass-through to `api.anthropic.com`, with three
//! additions that make them pleasant to call from a browser or a shell:
//!
//! * the same `localImage` / `localDocument` input sugar the conversation
//!   endpoints accept, expanded and validated before anything leaves the machine;
//! * `betas` as a normal JSON field instead of a header;
//! * sensible `model` and `max_tokens` defaults.
//!
//! Everything else — tools, `allowed_callers` for programmatic tool calling,
//! server tools, `output_config`, `transformations`, `container` — is forwarded
//! untouched, so the platform documentation is the reference for this surface
//! and nothing here goes stale when the API grows.

use std::collections::HashMap;

use axum::body::Body as AxumBody;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use serde_json::Value;

use super::{ok, validate_id, Body};
use crate::bridge::error::{BridgeError, BridgeResult};
use crate::bridge::inputs;
use crate::bridge::messages::apply_defaults;
use crate::state::AppState;

/// Largest file accepted for upload. The Files API allows 500 MB, but the
/// bridge's own body limit is the binding constraint and saying so is kinder
/// than a truncated stream.
const MAX_UPLOAD_BYTES: usize = crate::bridge::MAX_BODY_BYTES;

/// Pull `betas` out of the request body and render it as a header value.
fn take_betas(body: &mut Value) -> BridgeResult<Option<String>> {
    let Some(obj) = body.as_object_mut() else {
        return Err(BridgeError::invalid_request(
            "The request body must be a JSON object.",
        ));
    };
    let Some(value) = obj.remove("betas") else {
        return Ok(None);
    };
    match value {
        Value::Null => Ok(None),
        Value::String(s) => Ok(Some(s)),
        Value::Array(items) => {
            let mut parts = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Value::String(s) => parts.push(s),
                    other => {
                        return Err(BridgeError::invalid_field(
                            "betas",
                            format!("`betas` entries must be strings, got {other}."),
                        ))
                    }
                }
            }
            Ok(Some(parts.join(",")))
        }
        other => Err(BridgeError::invalid_field(
            "betas",
            format!("`betas` must be a string or an array of strings, got {other}."),
        )),
    }
}

/// Expand and validate the input sugar inside `messages[].content`.
///
/// Only top-level content arrays are rewritten. Blocks the bridge does not
/// recognize pass through untouched, so a new upstream block type works here
/// the day it ships.
fn expand_message_content(body: &mut Value) -> BridgeResult<()> {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    for (index, message) in messages.iter_mut().enumerate() {
        let Some(content) = message.get_mut("content") else {
            continue;
        };
        if !content.is_array() {
            continue;
        }
        if content.as_array().is_some_and(Vec::is_empty) {
            continue;
        }
        let normalized = inputs::normalize(content).map_err(|e| {
            // Re-anchor the field path onto the message the caller sent.
            let field = e
                .extra
                .get("field")
                .and_then(Value::as_str)
                .map(|f| f.replacen("input", &format!("messages[{index}].content"), 1))
                .unwrap_or_else(|| format!("messages[{index}].content"));
            BridgeError::new(e.status, e.code, e.message).with("field", field)
        })?;
        *content = normalized.content();
    }
    Ok(())
}

/// `POST /messages` — one Messages API call.
pub async fn create(
    State(state): State<AppState>,
    Body(mut body): Body<Value>,
) -> BridgeResult<Response> {
    let client = &state.bridge.messages;
    if !client.is_configured() {
        // Surface the configuration error before doing any work on the body.
        client.create_message(&Value::Null, None).await?;
    }

    let betas = take_betas(&mut body)?;
    expand_message_content(&mut body)?;
    apply_defaults(&mut body);

    if body.get("stream").and_then(Value::as_bool).unwrap_or(false) {
        let upstream = client.stream_message(&body, betas.as_deref()).await?;
        let content_type = upstream
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("text/event-stream")
            .to_string();
        return Ok((
            [
                (header::CONTENT_TYPE, content_type),
                (header::CACHE_CONTROL, "no-store".to_string()),
                ("x-accel-buffering".parse().unwrap(), "no".to_string()),
            ],
            AxumBody::from_stream(upstream.bytes_stream()),
        )
            .into_response());
    }

    Ok(ok(client.create_message(&body, betas.as_deref()).await?))
}

/// `POST /files` — upload one file to the Files API.
pub async fn upload_file(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> BridgeResult<Response> {
    let mut filename: Option<String> = None;
    let mut mime: Option<String> = None;
    let mut bytes: Option<Vec<u8>> = None;
    let mut expires_in_seconds: Option<u64> = None;

    while let Some(field) = multipart.next_field().await.map_err(multipart_error)? {
        match field.name().unwrap_or_default() {
            "file" => {
                filename = field.file_name().map(str::to_string);
                mime = field.content_type().map(str::to_string);
                let data = field.bytes().await.map_err(multipart_error)?;
                if data.len() > MAX_UPLOAD_BYTES {
                    return Err(BridgeError::too_large(
                        "request_size",
                        format!("The upload exceeds {MAX_UPLOAD_BYTES} bytes."),
                    ));
                }
                bytes = Some(data.to_vec());
            }
            "filename" => {
                filename = field.text().await.ok().filter(|s| !s.trim().is_empty());
            }
            "expires_in_seconds" => {
                let raw = field.text().await.unwrap_or_default();
                expires_in_seconds = Some(raw.trim().parse().map_err(|_| {
                    BridgeError::invalid_field(
                        "expires_in_seconds",
                        "`expires_in_seconds` must be a whole number.",
                    )
                })?);
            }
            other => {
                return Err(BridgeError::unprocessable(
                    "unknown_field",
                    format!("Unexpected multipart field {other:?}."),
                ))
            }
        }
    }

    let bytes =
        bytes.ok_or_else(|| BridgeError::invalid_multipart("The upload needs a `file` part."))?;
    if bytes.is_empty() {
        return Err(BridgeError::invalid_field(
            "file",
            "The uploaded file is empty.",
        ));
    }
    let filename = filename.unwrap_or_else(|| "upload.bin".to_string());
    // The Files API picks the content-block type from the media type, so a
    // browser that sends application/octet-stream would make an image unusable.
    let mime = mime
        .filter(|m| m != "application/octet-stream")
        .or_else(|| inputs::sniff_image(&bytes).map(str::to_string))
        .unwrap_or_else(|| guess_mime(&filename).to_string());

    Ok(ok(state
        .bridge
        .messages
        .upload_file(&filename, &mime, bytes, expires_in_seconds)
        .await?))
}

/// An upload that blows the body limit is too large, not malformed — saying
/// `400 invalid_multipart` sends the caller to look for a framing bug that is
/// not there.
fn multipart_error(e: axum::extract::multipart::MultipartError) -> BridgeError {
    if e.status() == axum::http::StatusCode::PAYLOAD_TOO_LARGE {
        return BridgeError::too_large(
            "request_size",
            format!(
                "The upload exceeds the {} MiB limit.",
                MAX_UPLOAD_BYTES / (1024 * 1024)
            ),
        );
    }
    BridgeError::invalid_multipart(format!("Malformed multipart body: {e}"))
}

fn guess_mime(filename: &str) -> &'static str {
    match filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => "application/pdf",
        "txt" | "md" | "csv" => "text/plain",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

/// `GET /files` and `GET /files/{id}` — listing and metadata.
pub async fn list_files(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> BridgeResult<Response> {
    let query: Vec<(String, String)> = params.into_iter().filter(|(k, _)| k != "token").collect();
    Ok(ok(state.bridge.messages.get_json("files", &query).await?))
}

/// `GET /files/{id}/content` — download a file the API generated.
pub async fn file_content(
    State(state): State<AppState>,
    Path(file_id): Path<String>,
) -> BridgeResult<Response> {
    validate_id("file_id", &file_id)?;
    let (bytes, mime) = state
        .bridge
        .messages
        .get_bytes(&format!("files/{file_id}/content"))
        .await?;
    Ok(([(header::CONTENT_TYPE, mime)], bytes).into_response())
}

/// `DELETE /files/{id}`.
pub async fn delete_file(
    State(state): State<AppState>,
    Path(file_id): Path<String>,
) -> BridgeResult<Response> {
    validate_id("file_id", &file_id)?;
    Ok(ok(state
        .bridge
        .messages
        .delete(&format!("files/{file_id}"))
        .await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn betas_accept_a_string_or_a_list_and_leave_the_body_clean() {
        let mut body = json!({ "betas": "a", "model": "m" });
        assert_eq!(take_betas(&mut body).unwrap().as_deref(), Some("a"));
        assert!(body.get("betas").is_none());

        let mut body = json!({ "betas": ["a", "b"] });
        assert_eq!(take_betas(&mut body).unwrap().as_deref(), Some("a,b"));

        let mut body = json!({});
        assert!(take_betas(&mut body).unwrap().is_none());

        let mut body = json!({ "betas": [1] });
        assert_eq!(take_betas(&mut body).unwrap_err().code, "invalid_field");

        let mut body = json!([]);
        assert_eq!(take_betas(&mut body).unwrap_err().code, "invalid_request");
    }

    #[test]
    fn content_sugar_is_expanded_and_string_content_is_left_alone() {
        let png = {
            let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
            b.extend_from_slice(&13u32.to_be_bytes());
            b.extend_from_slice(b"IHDR");
            b.extend_from_slice(&200u32.to_be_bytes());
            b.extend_from_slice(&200u32.to_be_bytes());
            b.extend_from_slice(&[8, 6, 0, 0, 0]);
            inputs::encode_base64(&b)
        };
        let mut body = json!({
            "messages": [
                { "role": "user", "content": "plain string" },
                { "role": "user", "content": [
                    { "type": "image", "url": format!("data:image/png;base64,{png}") },
                    { "type": "text", "text": "what is this" },
                ]},
            ]
        });
        expand_message_content(&mut body).unwrap();
        assert_eq!(body["messages"][0]["content"], "plain string");
        assert_eq!(
            body["messages"][1]["content"][0]["source"]["type"],
            "base64"
        );
        assert_eq!(body["messages"][1]["content"][1]["type"], "text");
    }

    #[test]
    fn expansion_errors_point_at_the_offending_message() {
        let mut body = json!({
            "messages": [
                { "role": "user", "content": [{ "type": "image", "url": "ftp://nope" }] },
            ]
        });
        let err = expand_message_content(&mut body).unwrap_err();
        assert_eq!(err.body()["error"]["field"], "messages[0].content[0].url");
    }

    #[test]
    fn expansion_tolerates_bodies_without_messages() {
        let mut body = json!({ "model": "m" });
        assert!(expand_message_content(&mut body).is_ok());
        let mut body = json!({ "messages": [{ "role": "user", "content": [] }] });
        assert!(expand_message_content(&mut body).is_ok());
    }

    #[test]
    fn mime_guessing_covers_the_documented_file_types() {
        assert_eq!(guess_mime("a.pdf"), "application/pdf");
        assert_eq!(guess_mime("notes.TXT"), "text/plain");
        assert_eq!(guess_mime("shot.PNG"), "image/png");
        assert_eq!(guess_mime("x.unknown"), "application/octet-stream");
        assert_eq!(guess_mime("noextension"), "application/octet-stream");
    }
}
