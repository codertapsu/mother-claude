//! Accounts (docs/ACCOUNTS.md §4.4): everyone signed in may list them and
//! pick one for a session (D9); adding, changing, removing and signing in
//! accounts is for the desktop only.
//!
//! * `GET /api/accounts` (Authed) → `AccountsView`; the desktop also sees a
//!   running `setup-token`'s sign-in URL.
//! * `POST /api/accounts {name, kind: "directory", dir}` or
//!   `{name, kind: "token", token?}` (DesktopOnly) → `201 AccountView`. A
//!   token is checked with `claude auth status` first and goes straight to the
//!   secret store; without one the account waits for `setup-token`.
//! * `PATCH /api/accounts/{id} {name?, color?}`, `DELETE /api/accounts/{id}`
//!   (DesktopOnly). Removing never touches the account's directory or its
//!   Anthropic sign-in.
//! * `PUT /api/accounts/defaults {newSession, bridge, byProject}`,
//!   `PUT /api/accounts/copy-consent {from, to, granted}` (DesktopOnly).
//! * `POST /api/accounts/{id}/refresh` (Authed): re-check the sign-in.
//! * `POST /api/accounts/{id}/login {console?}` (DesktopOnly): `claude auth
//!   login` for the account's directory; watched until it lands.
//! * `POST /api/accounts/{id}/hooks/install|uninstall` (DesktopOnly): the
//!   hooks plugin in a directory account's own directory.
//! * `GET|POST|DELETE /api/accounts/{id}/token` (DesktopOnly): a token
//!   account's `setup-token` capture — `POST {}` starts it, `POST {code}`
//!   types the browser's code into it, `POST {token}` stores a pasted token,
//!   `DELETE` stops it.
//!
//! Every change is audited (never with a token) and sent as an `accounts`
//! frame.

use std::path::PathBuf;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::Body;
use crate::claude::accounts::view::{self, AccountView, AccountsView};
use crate::claude::accounts::{
    status, AccountDefaults, AccountEnv, AccountError, AccountKind, AccountPatch,
};
use crate::claude::secrets::Secret;
use crate::server::auth::{Authed, DesktopOnly};
use crate::server::context::ClientContext;
use crate::server::devices::audit::AuditEntry;
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

fn account_error(e: AccountError) -> ApiError {
    match &e {
        AccountError::NotFound(_) => ApiError::not_found(e.code(), e.to_string()),
        AccountError::NameTaken(_) | AccountError::TokenMissing => {
            ApiError::conflict(e.code(), e.to_string())
        }
        AccountError::Secret(_) | AccountError::Save(_) => ApiError::internal(e.to_string()),
        _ => ApiError::unprocessable(e.code(), e.to_string()),
    }
}

fn audit(state: &AppState, ctx: &ClientContext, action: &str, detail: serde_json::Value) {
    state.devices.audit.record(AuditEntry {
        detail: Some(detail),
        ..AuditEntry::from_ctx(ctx, action)
    });
}

fn one(state: &AppState, id: &str) -> ApiResult<AccountView> {
    view::build(state, true, None)
        .accounts
        .into_iter()
        .find(|a| a.id == id)
        .ok_or_else(|| account_error(AccountError::NotFound(id.to_string())))
}

fn known(state: &AppState, id: &str) -> ApiResult<()> {
    state
        .accounts
        .get(id)
        .map(|_| ())
        .ok_or_else(|| account_error(AccountError::NotFound(id.to_string())))
}

/// `GET /api/accounts`.
pub async fn list(Authed(ctx): Authed, State(state): State<AppState>) -> Json<AccountsView> {
    Json(view::view(&state, ctx.is_desktop()).await)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AddBody {
    pub name: String,
    pub kind: AccountKind,
    #[serde(default)]
    pub dir: Option<PathBuf>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
}

/// `POST /api/accounts`.
pub async fn add(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Body(body): Body<AddBody>,
) -> ApiResult<Response> {
    let account = match body.kind {
        AccountKind::Directory => {
            if body.token.is_some() {
                return Err(ApiError::unprocessable(
                    "invalid_account",
                    "A directory account signs in with `claude auth login`, not a token.",
                ));
            }
            let dir = body.dir.as_deref().ok_or_else(|| {
                ApiError::unprocessable("invalid_account", "A directory account needs `dir`.")
            })?;
            state
                .accounts
                .add_directory(&body.name, dir, body.color.as_deref())
                .map_err(account_error)?
        }
        AccountKind::Token => {
            if body.dir.is_some() {
                return Err(ApiError::unprocessable(
                    "invalid_account",
                    "A token account uses the Default account's directory; `dir` is not taken.",
                ));
            }
            let token = body
                .token
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(Secret::new);
            let mut checked = None;
            if let Some(t) = &token {
                let status = crate::claude::auth::status_for(&AccountEnv::Token(t.clone())).await;
                if !status.logged_in {
                    return Err(ApiError::unprocessable(
                        "token_rejected",
                        status
                            .error
                            .unwrap_or_else(|| "Claude Code does not accept this token.".into()),
                    ));
                }
                checked = Some(status);
            }
            let account = state
                .accounts
                .add_token(&body.name, token.as_ref(), body.color.as_deref())
                .map_err(account_error)?;
            if let Some(status) = checked {
                state.accounts.runtime.set_auth(&account.id, status);
            }
            account
        }
    };
    audit(
        &state,
        &ctx,
        "account.add",
        json!({ "account": account.id, "name": account.name, "kind": account.kind, "dir": account.dir }),
    );
    // A directory's sign-in, in the background (a fresh one is signed out).
    if account.kind == AccountKind::Directory {
        let st = state.clone();
        let id = account.id.clone();
        tokio::spawn(async move {
            let _ = status::refresh(&st, &id).await;
        });
    }
    view::publish(&state).await;
    Ok((StatusCode::CREATED, Json(one(&state, &account.id)?)).into_response())
}

/// `PATCH /api/accounts/{id}`.
pub async fn update(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<AccountPatch>,
) -> ApiResult<Json<AccountView>> {
    let before = state.accounts.get(&id).map(|a| a.name);
    let account = state.accounts.update(&id, &body).map_err(account_error)?;
    // A backup set is named after its account: it follows a rename.
    if let Some(old) = before.filter(|n| *n != account.name) {
        crate::claude::backup::ops::on_renamed(&state, &old, &account.name);
    }
    audit(
        &state,
        &ctx,
        "account.update",
        json!({ "account": account.id, "name": account.name, "color": account.color }),
    );
    view::publish(&state).await;
    Ok(Json(one(&state, &id)?))
}

/// `DELETE /api/accounts/{id}`.
pub async fn remove(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let live = state
        .control
        .accounts()
        .into_iter()
        .filter(|(_, a)| *a == id)
        .count();
    if live > 0 {
        return Err(ApiError::conflict(
            "account_in_use",
            format!("{live} session(s) Mother Claude started still run under this account."),
        ));
    }
    let account = state.accounts.remove(&id).map_err(account_error)?;
    state.index.forget_checkpoint(&id);
    audit(
        &state,
        &ctx,
        "account.remove",
        json!({ "account": account.id, "name": account.name, "kind": account.kind }),
    );
    view::publish(&state).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `PUT /api/accounts/defaults`.
pub async fn put_defaults(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Body(body): Body<AccountDefaults>,
) -> ApiResult<Json<AccountDefaults>> {
    let defaults = state.accounts.set_defaults(body).map_err(account_error)?;
    audit(
        &state,
        &ctx,
        "account.defaults",
        json!({ "newSession": defaults.new_session, "bridge": defaults.bridge,
                "projects": defaults.by_project.len() }),
    );
    view::publish(&state).await;
    Ok(Json(defaults))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsentBody {
    pub from: String,
    pub to: String,
    pub granted: bool,
}

/// `PUT /api/accounts/copy-consent`.
pub async fn put_copy_consent(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Body(body): Body<ConsentBody>,
) -> ApiResult<Json<AccountsView>> {
    state
        .accounts
        .set_copy_consent(&body.from, &body.to, body.granted)
        .map_err(account_error)?;
    audit(
        &state,
        &ctx,
        "account.copy_consent",
        json!({ "from": body.from, "to": body.to, "granted": body.granted }),
    );
    view::publish(&state).await;
    Ok(Json(view::view(&state, true).await))
}

/// `POST /api/accounts/{id}/refresh`.
pub async fn refresh(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<AccountView>> {
    status::refresh(&state, &id).await.map_err(account_error)?;
    Ok(Json(one(&state, &id)?))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoginBody {
    #[serde(default)]
    pub console: bool,
}

/// `POST /api/accounts/{id}/login`.
pub async fn login(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<LoginBody>,
) -> ApiResult<Response> {
    known(&state, &id)?;
    status::start_login(&state, &id, body.console)
        .await
        .map_err(|m| ApiError::unprocessable("login_failed", m))?;
    ctx.note_dangerous("account sign-in");
    audit(&state, &ctx, "account.login", json!({ "account": id }));
    Ok((StatusCode::ACCEPTED, Json(one(&state, &id)?)).into_response())
}

/// `GET /api/accounts/{id}/token`.
pub async fn token_status(
    DesktopOnly(_ctx): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    known(&state, &id)?;
    let capture = state.accounts.runtime.capture(&id).map(|j| j.view());
    Ok(Json(json!({
        "available": crate::claude::accounts::capture::AVAILABLE,
        "capture": capture,
        "tokenStored": state.accounts.has_token(&id),
    })))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TokenBody {
    /// A token to store (pasted).
    #[serde(default)]
    pub token: Option<String>,
    /// The code the browser shows, for a running capture.
    #[serde(default)]
    pub code: Option<String>,
}

/// `POST /api/accounts/{id}/token`.
pub async fn token(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<TokenBody>,
) -> ApiResult<Response> {
    known(&state, &id)?;
    let pasted = body
        .token
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());
    let code = body
        .code
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    match (pasted, code) {
        (Some(_), Some(_)) => Err(ApiError::unprocessable(
            "invalid_body",
            "Send either `token` or `code`, not both.",
        )),
        (Some(t), None) => {
            status::paste_token(&state, &id, Secret::new(t))
                .await
                .map_err(|m| ApiError::unprocessable("token_rejected", m))?;
            audit(
                &state,
                &ctx,
                "account.token",
                json!({ "account": id, "how": "paste" }),
            );
            Ok(Json(one(&state, &id)?).into_response())
        }
        (None, Some(c)) => {
            let view = status::submit_code(&state, &id, c)
                .map_err(|m| ApiError::conflict("capture_not_running", m))?;
            Ok(Json(json!({ "capture": view })).into_response())
        }
        (None, None) => {
            let view = status::start_capture(&state, &id)
                .map_err(|m| ApiError::unprocessable("capture_failed", m))?;
            audit(
                &state,
                &ctx,
                "account.token",
                json!({ "account": id, "how": "setup-token" }),
            );
            view::publish(&state).await;
            Ok((StatusCode::ACCEPTED, Json(json!({ "capture": view }))).into_response())
        }
    }
}

/// `DELETE /api/accounts/{id}/token`.
pub async fn cancel_token(
    DesktopOnly(_ctx): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    known(&state, &id)?;
    let view = status::cancel_capture(&state, &id);
    view::publish(&state).await;
    Ok(Json(json!({ "capture": view })))
}

/// `POST /api/accounts/{id}/hooks/install`.
pub async fn hooks_install(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<AccountView>> {
    let answer = crate::claude::hooks::install_for(&state, &id).await?;
    audit(
        &state,
        &ctx,
        "account.hooks.install",
        json!({ "account": id, "state": answer }),
    );
    view::publish(&state).await;
    Ok(Json(one(&state, &id)?))
}

/// `POST /api/accounts/{id}/hooks/uninstall`.
pub async fn hooks_uninstall(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<AccountView>> {
    let answer = crate::claude::hooks::uninstall_for(&state, &id).await?;
    audit(
        &state,
        &ctx,
        "account.hooks.uninstall",
        json!({ "account": id, "state": answer }),
    );
    view::publish(&state).await;
    Ok(Json(one(&state, &id)?))
}
