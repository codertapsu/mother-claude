//! The rest of the Claude API's surface — `POST /v1/messages/count_tokens`
//! and the models list in its shape — and Mother Claude's own additions for
//! a client: `GET /v1/conversations`, `GET` and `DELETE
//! /v1/conversations/{id}`.

use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};

use super::store::Record;
use super::wire::{self, ApiError, MessagesRequest};
use crate::bridge::MountPolicy;
use crate::server::context::ClientContext;
use crate::state::AppState;

fn ok(request_id: &str, body: Value, extra: &[(&'static str, String)]) -> Response {
    let mut resp = (StatusCode::OK, axum::Json(body)).into_response();
    let h = resp.headers_mut();
    if let Ok(v) = HeaderValue::from_str(request_id) {
        h.insert("request-id", v);
    }
    for (k, v) in extra {
        if let Ok(v) = HeaderValue::from_str(v) {
            h.insert(HeaderName::from_static(k), v);
        }
    }
    resp
}

/// A time as the API writes them (RFC 3339, UTC).
pub fn rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// ───────────────────────────── count_tokens ─────────────────────────────

/// Input tokens of one block, roughly: four bytes of text a token; an image
/// by its size (`⌈w/28⌉×⌈h/28⌉`, as the vision docs count).
fn block_tokens(b: &Value) -> u64 {
    let text_len = |v: Option<&Value>| match v {
        Some(Value::String(s)) => s.len(),
        Some(other) => other.to_string().len(),
        None => 0,
    };
    let bytes = match b.get("type").and_then(Value::as_str).unwrap_or("") {
        "text" => text_len(b.get("text")),
        "thinking" => text_len(b.get("thinking")),
        "image" => {
            let dims = b
                .pointer("/source/data")
                .and_then(Value::as_str)
                .and_then(crate::bridge::inputs::decode_base64)
                .and_then(|bytes| crate::bridge::inputs::image_dimensions(&bytes));
            return dims.map_or(1_600, |d| d.estimated_tokens());
        }
        "document" => {
            // A page of a PDF is about 1,500–3,000 tokens; its bytes are the
            // only measure here.
            let data = b
                .pointer("/source/data")
                .and_then(Value::as_str)
                .map_or(0, str::len);
            return (data as u64 / 4).clamp(100, 200_000);
        }
        "tool_use" => text_len(b.get("input")) + text_len(b.get("name")),
        "tool_result" => match b.get("content") {
            Some(Value::Array(items)) => return items.iter().map(block_tokens).sum::<u64>() + 4,
            other => text_len(other),
        },
        _ => b.to_string().len(),
    };
    (bytes as u64).div_ceil(4)
}

/// The request's input tokens, estimated.
pub fn estimate(req: &MessagesRequest) -> u64 {
    let mut n = (req.system.len() as u64).div_ceil(4);
    for t in &req.tools {
        let schema = t.input_schema.to_string();
        let len = t.name.len() + t.description.as_deref().map_or(0, str::len) + schema.len();
        n += (len as u64).div_ceil(4) + 8;
    }
    for m in &req.messages {
        n += 4 + m.content.iter().map(block_tokens).sum::<u64>();
    }
    n.max(1)
}

/// `POST /v1/messages/count_tokens`. Claude Code has no way to count, so
/// the answer is an estimate (`x-mother-claude-estimate: true`) — unless the
/// server answers with the Anthropic API, which counts exactly.
pub async fn count_tokens(
    State(state): State<AppState>,
    mount: MountPolicy,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let rid = wire::request_id();
    let mut body: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return ApiError::invalid(format!("There was an issue with your request body: {e}"))
                .respond(&rid)
        }
    };
    let backend = headers
        .get(super::routes::BACKEND_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_ascii_lowercase())
        .or_else(|| mount.defaults.messages_backend.clone());
    if backend.as_deref() == Some(super::BACKEND_ANTHROPIC) {
        let betas = headers
            .get("anthropic-beta")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        return match state
            .bridge
            .messages
            .count_tokens(&body, betas.as_deref())
            .await
        {
            Ok(v) => ok(&rid, v, &[]),
            Err(e) => ApiError::from(e).respond(&rid),
        };
    }
    if body.get("model").is_none_or(Value::is_null) {
        if let (Some(m), Some(obj)) = (&mount.defaults.model, body.as_object_mut()) {
            obj.insert("model".into(), json!(m));
        }
    }
    match MessagesRequest::parse(&body, true) {
        Ok(req) => ok(
            &rid,
            json!({ "input_tokens": estimate(&req) }),
            &[("x-mother-claude-estimate", "true".into())],
        ),
        Err(e) => e.respond(&rid),
    }
}

// ───────────────────────────────── models ─────────────────────────────────

/// The models a session or the settings list (`value`, `displayName`, …;
/// the Agent SDK's `ModelInfo`) as the Claude API lists them: `id` the model
/// the API knows (`resolvedModel`, else the name Claude Code takes), once
/// each. `created_at` is not known here: the Unix epoch stands in.
pub fn api_models(list: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for m in list {
        let Some(value) = m.get("value").and_then(Value::as_str) else {
            continue;
        };
        let id = m
            .get("resolvedModel")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(value);
        if out.iter().any(|o| o["id"] == id) {
            continue;
        }
        let mut entry = json!({
            "type": "model",
            "id": id,
            "display_name": m.get("displayName").and_then(Value::as_str).unwrap_or(id),
            "created_at": rfc3339(0),
        });
        // What Claude Code says of it, under its own names.
        if let (Some(e), Some(src)) = (entry.as_object_mut(), m.as_object()) {
            for (k, v) in src {
                if !matches!(k.as_str(), "type" | "id" | "display_name" | "created_at") {
                    e.insert(k.clone(), v.clone());
                }
            }
        }
        out.push(entry);
    }
    out
}

/// A list of models in the API's page shape: `limit` (1–1000, default 20),
/// `after_id`, `before_id`.
pub fn api_models_page(models: Vec<Value>, query: &HashMap<String, String>) -> Value {
    let limit = query
        .get("limit")
        .and_then(|l| l.parse::<usize>().ok())
        .unwrap_or(20)
        .clamp(1, 1000);
    let at = |id: &str| models.iter().position(|m| m["id"] == id);
    let (start, end) = match (query.get("after_id"), query.get("before_id")) {
        (Some(after), _) => {
            let s = at(after).map_or(models.len(), |i| i + 1);
            (s, (s + limit).min(models.len()))
        }
        (None, Some(before)) => {
            let e = at(before).unwrap_or(0);
            (e.saturating_sub(limit), e)
        }
        (None, None) => (0, limit.min(models.len())),
    };
    let page: Vec<Value> = models[start..end].to_vec();
    let has_more = match query.get("before_id") {
        Some(_) if query.get("after_id").is_none() => start > 0,
        _ => end < models.len(),
    };
    json!({
        "data": page,
        "first_id": page.first().map(|m| m["id"].clone()),
        "last_id": page.last().map(|m| m["id"].clone()),
        "has_more": has_more,
    })
}

/// `GET /v1/models/{model_id}`.
pub async fn model(
    State(state): State<AppState>,
    mount: MountPolicy,
    Path(model_id): Path<String>,
) -> Response {
    let rid = wire::request_id();
    let list = crate::bridge::routes::meta::model_list(&state, &mount).await;
    let models = api_models(&list);
    match models
        .into_iter()
        .find(|m| m["id"] == model_id.as_str() || m["value"] == model_id.as_str())
    {
        Some(m) => ok(&rid, m, &[]),
        None => ApiError::not_found(format!("model: {model_id}")).respond(&rid),
    }
}

// ───────────────────────────── conversations ─────────────────────────────

/// From beyond this computer, a server without a key does not serve its
/// clients' conversations (docs/CLAUDE-API.md "Access"): with no key,
/// nothing would tell one caller from another who sends the same
/// `X-Client-Id`. With one — or for this computer, or a Mother Claude
/// token — they are served.
fn needs_key(ctx: &ClientContext, mount: &MountPolicy, rid: &str) -> Option<Response> {
    let trusted = ctx.on_machine || ctx.principal.is_user() || mount.server.has_key();
    (!trusted).then(|| {
        ApiError::permission(
            "Reading and deleting conversations from beyond this computer needs the server's \
             key: give it one when you start it in Mother Claude (HTTP API › Access).",
        )
        .respond(rid)
    })
}

/// The client the request is from, and its folder (when it has one).
fn caller_dir(
    state: &AppState,
    mount: &MountPolicy,
    ctx: &ClientContext,
    headers: &HeaderMap,
    user_id: Option<&str>,
) -> Option<std::path::PathBuf> {
    let client = super::routes::client_of(mount, ctx, headers, user_id);
    super::routes::root_of(state, mount).map(|root| root.join(client.id))
}

/// A conversation as a client sees it (`messages` when asked).
pub fn conversation_view(state: &AppState, r: &Record, messages: bool) -> Value {
    let status = match state.bridge.api.state_of(&r.id) {
        Some((true, _)) => "answering",
        Some((false, true)) => "waiting_for_tool_results",
        _ => "idle",
    };
    let mut v = json!({
        "id": r.id,
        "type": "conversation",
        "title": r.title,
        "model": r.model_used.clone().unwrap_or_else(|| r.model.clone()),
        "created_at": rfc3339(r.created_at),
        "updated_at": rfc3339(r.updated_at),
        "message_count": r.messages.len(),
        "requests": r.requests,
        "usage": {
            "input_tokens": r.usage.input_tokens,
            "output_tokens": r.usage.output_tokens,
            "cache_read_input_tokens": r.usage.cache_read_input_tokens,
            "cache_creation_input_tokens": r.usage.cache_creation_input_tokens,
        },
        "status": status,
        "forked_from": r.forked_from,
    });
    if messages {
        v["messages"] = json!(r.messages);
    }
    v
}

/// `GET /v1/conversations` — the caller's conversations, newest first
/// (`limit`, `after_id`; the models list's page shape).
pub async fn list_conversations(
    State(state): State<AppState>,
    ctx: ClientContext,
    mount: MountPolicy,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let rid = wire::request_id();
    if let Some(refused) = needs_key(&ctx, &mount, &rid) {
        return refused;
    }
    let user = query.get("user_id").map(String::as_str);
    let Some(dir) = caller_dir(&state, &mount, &ctx, &headers, user) else {
        return ok(
            &rid,
            json!({ "data": [], "has_more": false, "first_id": null, "last_id": null }),
            &[],
        );
    };
    let views: Vec<Value> = state
        .bridge
        .api
        .records_in(&dir)
        .iter()
        .filter(|r| !r.history.is_empty())
        .map(|r| conversation_view(&state, r, false))
        .collect();
    ok(&rid, api_models_page(views, &query), &[])
}

/// `GET /v1/conversations/{id}` — one of the caller's, with its messages.
pub async fn get_conversation(
    State(state): State<AppState>,
    ctx: ClientContext,
    mount: MountPolicy,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let rid = wire::request_id();
    if let Some(refused) = needs_key(&ctx, &mount, &rid) {
        return refused;
    }
    let user = query.get("user_id").map(String::as_str);
    let found = caller_dir(&state, &mount, &ctx, &headers, user).and_then(|dir| {
        state
            .bridge
            .api
            .records_in(&dir)
            .into_iter()
            .find(|r| r.id == id && !r.history.is_empty())
    });
    match found {
        Some(r) => ok(&rid, conversation_view(&state, &r, true), &[]),
        None => ApiError::not_found(format!("conversation: {id}")).respond(&rid),
    }
}

/// `DELETE /v1/conversations/{id}` — forgets one of the caller's: its
/// record goes and its process closes. Its Claude Code transcript stays on
/// this computer until it is deleted in Mother Claude.
pub async fn delete_conversation(
    State(state): State<AppState>,
    ctx: ClientContext,
    mount: MountPolicy,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let rid = wire::request_id();
    if let Some(refused) = needs_key(&ctx, &mount, &rid) {
        return refused;
    }
    let user = query.get("user_id").map(String::as_str);
    let Some(dir) = caller_dir(&state, &mount, &ctx, &headers, user) else {
        return ApiError::not_found(format!("conversation: {id}")).respond(&rid);
    };
    let mine = state.bridge.api.records_in(&dir).iter().any(|r| r.id == id);
    if !mine {
        return ApiError::not_found(format!("conversation: {id}")).respond(&rid);
    }
    if state.bridge.api.state_of(&id).is_some_and(|(busy, _)| busy) {
        return ApiError::conflict("This conversation is answering a request.").respond(&rid);
    }
    super::admin::forget_conversation(&state, &id).await;
    ok(
        &rid,
        json!({ "id": id, "type": "conversation_deleted" }),
        &[],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_get_the_apis_ids_once() {
        let list = vec![
            json!({ "value": "default", "resolvedModel": "claude-opus-5-5", "displayName": "Default" }),
            json!({ "value": "opus", "resolvedModel": "claude-opus-5-5", "displayName": "Opus" }),
            json!({ "value": "haiku", "displayName": "Haiku", "supportsEffort": false }),
        ];
        let models = api_models(&list);
        let ids: Vec<&str> = models.iter().map(|m| m["id"].as_str().unwrap()).collect();
        assert_eq!(ids, vec!["claude-opus-5-5", "haiku"]);
        assert_eq!(models[0]["type"], "model");
        assert_eq!(models[0]["display_name"], "Default");
        assert_eq!(models[0]["created_at"], "1970-01-01T00:00:00Z");
        assert_eq!(models[1]["supportsEffort"], false);
    }

    #[test]
    fn pages_follow_after_and_before() {
        let models: Vec<Value> = (0..5).map(|i| json!({ "id": format!("m{i}") })).collect();
        let q = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<HashMap<_, _>>()
        };
        let p = api_models_page(models.clone(), &q(&[("limit", "2")]));
        assert_eq!(p["first_id"], "m0");
        assert_eq!(p["last_id"], "m1");
        assert_eq!(p["has_more"], true);
        let p = api_models_page(models.clone(), &q(&[("limit", "2"), ("after_id", "m3")]));
        assert_eq!(p["data"].as_array().unwrap().len(), 1);
        assert_eq!(p["has_more"], false);
        let p = api_models_page(models, &q(&[("limit", "2"), ("before_id", "m3")]));
        assert_eq!(p["first_id"], "m1");
        assert_eq!(p["last_id"], "m2");
        assert_eq!(p["has_more"], true);
    }

    #[test]
    fn an_estimate_counts_text_tools_and_images() {
        let body = json!({
            "model": "m",
            "system": "a".repeat(400),
            "messages": [{ "role": "user", "content": "b".repeat(800) }],
            "tools": [{ "name": "t", "input_schema": { "type": "object" } }],
        });
        let req = MessagesRequest::parse(&body, true).unwrap();
        let n = estimate(&req);
        assert!((300..400).contains(&n), "{n}");
    }
}
