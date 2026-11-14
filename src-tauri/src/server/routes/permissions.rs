//! Permissions' routes (`pending::rules`, docs/PERMISSIONS.md).
//!
//! | Route | Who | Does |
//! |---|---|---|
//! | `GET /api/permissions` | Authed | `{rules: […], paused}` |
//! | `PATCH /api/permissions` `{paused}` | ManageData (resuming: the dangerous right) | pauses or resumes them all; answers the list |
//! | `PATCH /api/permissions/{id}` `{patterns?, scope?}` | ManageData (a change that allows more: the dangerous right) | the rule as it is now |
//! | `DELETE /api/permissions/{id}` | ManageData | removes one; answers the list |
//!
//! Rules are made by answering a card with `remember` (`pending::respond`).
//! Pausing and removing only ever take away, so any paired device that may
//! manage data does them; resuming and a change that lets a rule allow more
//! are a standing grant, so they need the dangerous right, as saving one
//! for a project or the computer does. Each change is audited (never a
//! pattern's text) and told to every client as `{t:"permissions"}`.

use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::claude::model::RuleScope;
use crate::pending::rules::{self, RuleError};
use crate::server::auth::{note_device_action, Authed, ManageData};
use crate::server::context::ClientContext;
use crate::server::devices::audit::AuditEntry;
use crate::server::error::{ApiError, ApiResult};
use crate::server::routes::Body;
use crate::server::ws::ServerEvent;
use crate::state::AppState;

fn list_answer(state: &AppState) -> Value {
    let (list, paused) = state.rules.list();
    let rules: Vec<Value> = list.iter().map(|r| rules::view(state, r)).collect();
    json!({ "rules": rules, "paused": paused })
}

fn unknown() -> ApiError {
    ApiError::not_found("permission_unknown", "No such permission.")
}

fn changed(state: &AppState, ctx: &ClientContext, action: &str, detail: Value, what: &str) {
    state.devices.audit.record(AuditEntry {
        detail: Some(detail),
        ..AuditEntry::from_ctx(ctx, action)
    });
    note_device_action(state, ctx, what);
    state.broadcast(ServerEvent::Permissions);
}

/// The dangerous right, for a change that lets Claude do more without
/// asking.
fn require_dangerous(ctx: &ClientContext, what: &str) -> ApiResult<()> {
    if !ctx.may_dangerous() {
        return Err(ApiError::dangerous_blocked(format!(
            "{what} lets Claude do more without asking: it needs this computer, or a device you \
             allowed dangerous actions."
        )));
    }
    ctx.note_dangerous(what);
    Ok(())
}

/// `GET /api/permissions`.
pub async fn list(Authed(_): Authed, State(state): State<AppState>) -> Json<Value> {
    Json(list_answer(&state))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PausedBody {
    paused: bool,
}

/// `PATCH /api/permissions` `{paused}`.
pub async fn set_paused(
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Body(body): Body<PausedBody>,
) -> ApiResult<Json<Value>> {
    if body.paused == state.rules.paused() {
        return Ok(Json(list_answer(&state)));
    }
    if !body.paused {
        require_dangerous(&ctx, "Turning the permissions back on")?;
    }
    state.rules.set_paused(body.paused);
    let (action, what) = if body.paused {
        ("permission.pause", "paused every permission")
    } else {
        ("permission.resume", "turned the permissions back on")
    };
    changed(&state, &ctx, action, json!({}), what);
    Ok(Json(list_answer(&state)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateBody {
    #[serde(default)]
    patterns: Option<Vec<String>>,
    #[serde(default)]
    scope: Option<RuleScope>,
}

/// `PATCH /api/permissions/{id}` `{patterns?, scope?}`.
pub async fn update(
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<UpdateBody>,
) -> ApiResult<Json<Value>> {
    if !rules::is_rule_id(&id) {
        return Err(unknown());
    }
    let before = state.rules.get(&id).ok_or_else(unknown)?;
    // Judged first: a change that allows more needs the right.
    let (_, widened) =
        rules::changed(&before, body.patterns.as_deref(), body.scope).map_err(rule_error)?;
    if widened {
        require_dangerous(&ctx, "Widening a permission")?;
    }
    let (rule, _) = state
        .rules
        .update(&id, body.patterns.as_deref(), body.scope)
        .map_err(rule_error)?;
    changed(
        &state,
        &ctx,
        "permission.update",
        rules::audit_detail(&rule),
        "changed a permission",
    );
    Ok(Json(rules::view(&state, &rule)))
}

/// `DELETE /api/permissions/{id}`.
pub async fn remove(
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    if !rules::is_rule_id(&id) {
        return Err(unknown());
    }
    let rule = state.rules.remove(&id).ok_or_else(unknown)?;
    changed(
        &state,
        &ctx,
        "permission.remove",
        rules::audit_detail(&rule),
        "removed a permission",
    );
    Ok(Json(list_answer(&state)))
}

fn rule_error(e: RuleError) -> ApiError {
    match e {
        RuleError::NotFound => unknown(),
        RuleError::Invalid(m) => ApiError::unprocessable("invalid_pattern", m),
    }
}
