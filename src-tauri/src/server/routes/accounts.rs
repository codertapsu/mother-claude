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
//! * `PATCH /api/accounts/{id} {name?, color?}` (the desktop or a paired
//!   device, docs/ACCOUNTS.md §13.9), `DELETE /api/accounts/{id}`
//!   (DesktopOnly). Removing never touches the account's directory or its
//!   Anthropic sign-in.
//! * `PUT /api/accounts/defaults {newSession, bridge, byProject}`,
//!   `PUT /api/accounts/copy-consent {from, to, granted}` (the desktop or a
//!   paired device).
//! * `GET /api/accounts/{id}` (Authed) → `AccountView` (the desktop also
//!   sees a running sign-in's link).
//! * `POST /api/accounts/{id}/refresh` (Authed): re-check the sign-in.
//! * `POST /api/accounts/{id}/login {console?, email?}` (DesktopOnly):
//!   `claude auth login` for the Default or a directory account, in a
//!   pseudo-terminal; the default browser opens on its own once its link is
//!   known (docs/ACCOUNTS.md §15.2).
//! * `POST /api/accounts/{id}/hooks/install|uninstall` (DesktopOnly): the
//!   hooks plugin in a directory account's own directory.
//! * `GET|POST|DELETE /api/accounts/{id}/token` (DesktopOnly): a token
//!   account's `setup-token` run — `POST {}` starts it, `POST {code}`
//!   types the browser's code into it, `POST {token}` stores a pasted token,
//!   `DELETE` stops it.
//! * `GET|POST /api/accounts/{id}/sign-in/open` (DesktopOnly): the browsers
//!   this machine has; open the running sign-in's link in one. `POST
//!   /api/accounts/{id}/sign-in/code {code}`, `DELETE
//!   /api/accounts/{id}/sign-in`: either kind's code and cancel.
//! * `POST /api/accounts/{id}/identity/accept` (DesktopOnly): keep who the
//!   account signed in as now (its `identityWarning` goes).
//!
//! One sign-in runs at a time across all accounts (`409 sign_in_running`,
//! naming the account). Every change is audited (never with a token, a link
//! or a code) and sent as an `accounts` frame.

use std::path::PathBuf;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::Body;
use crate::claude::accounts::capture::SignInError;
use crate::claude::accounts::signin::{self, Failure, LoginOptions, SignInJobView};
use crate::claude::accounts::view::{self, AccountView, AccountsView};
use crate::claude::accounts::{
    status, AccountDefaults, AccountEnv, AccountError, AccountKind, AccountPatch,
};
use crate::claude::browser::{Browser, OpenError};
use crate::claude::secrets::Secret;
use crate::server::auth::{self, Authed, DesktopOnly, ManageData};
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

/// [`one`] as `ctx` may see it (a running capture's sign-in URL is the
/// desktop's alone).
fn one_for(state: &AppState, ctx: &ClientContext, id: &str) -> ApiResult<AccountView> {
    view::build(state, ctx.is_desktop(), None)
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

/// A sign-in failure as an API error. `fallback`: the code a failure to
/// start has always had on this route (`login_failed`, `capture_failed`).
fn sign_in_error(e: Failure, fallback: &'static str) -> ApiError {
    let message = e.to_string();
    match e {
        Failure::Account(e @ (AccountError::NotFound(_) | AccountError::TokenMissing)) => {
            account_error(e)
        }
        Failure::Account(_) | Failure::TokenAccount => ApiError::unprocessable(fallback, message),
        Failure::SignIn(SignInError::Running { account, name }) => {
            ApiError::conflict("sign_in_running", message)
                .with("account", account)
                .with("accountName", name)
        }
        Failure::SignIn(e @ (SignInError::NotRunning | SignInError::NoLink)) => {
            ApiError::conflict(e.code(fallback), message)
        }
        Failure::SignIn(e) => ApiError::unprocessable(e.code(fallback), message),
        Failure::Browser(e @ OpenError::Unavailable(_)) => {
            ApiError::unprocessable(e.code(), message)
        }
        Failure::Browser(e @ OpenError::Spawn(_)) => {
            ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.code(), message)
        }
    }
}

/// A run's answer on the token routes: the capture they have always had,
/// and the sign-in.
fn run_body(run: Option<SignInJobView>) -> serde_json::Value {
    let (capture, sign_in) = run.map_or((None, None), |r| (r.capture, Some(r.sign_in)));
    json!({ "capture": capture, "signIn": sign_in })
}

/// `GET /api/accounts`.
pub async fn list(Authed(ctx): Authed, State(state): State<AppState>) -> Json<AccountsView> {
    Json(view::view(&state, ctx.is_desktop()).await)
}

/// `GET /api/accounts/{id}`.
pub async fn get_one(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<AccountView>> {
    let live = view::live_counts(&state).await;
    view::build(&state, ctx.is_desktop(), Some(&live))
        .accounts
        .into_iter()
        .find(|a| a.id == id)
        .map(Json)
        .ok_or_else(|| account_error(AccountError::NotFound(id)))
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
    ManageData(ctx): ManageData,
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
    auth::note_device_action(&state, &ctx, &format!("changed account {:?}", account.name));
    view::publish(&state).await;
    Ok(Json(one_for(&state, &ctx, &id)?))
}

/// `DELETE /api/accounts/{id}`.
pub async fn remove(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    // No running API server may lose the account it runs its conversations
    // under (docs/ACCOUNTS.md §13.5); a stopped one keeps it, says so, and
    // cannot start until another is chosen. Held until the account is gone,
    // so no server starts with it in between.
    let _servers = state.bridge.servers.admin.lock().await;
    for server in state.bridge.servers.all() {
        if server.defaults().account.as_deref() != Some(id.as_str()) || !server.is_running() {
            continue;
        }
        let err = if server.is_main() {
            ApiError::conflict(
                "account_used_by_bridge",
                "The API runs with this account as its default for new conversations. Stop \
                 the API, or start it again with another account, first.",
            )
        } else {
            ApiError::conflict(
                "account_used_by_bridge",
                format!(
                    "The API server \"{}\" runs with this account. Stop it, or start it again \
                     with another account, first.",
                    server.name()
                ),
            )
            .with("server", server.id.clone())
            .with("serverName", server.name())
        };
        return Err(err);
    }
    let main = state.bridge.main();
    let bridge_default =
        main.defaults().account == Some(id.clone()) || main.saved().account == Some(id.clone());
    let live = state
        .control
        .accounts()
        .into_iter()
        .filter(|(_, a)| *a == id)
        .count()
        + state
            .bridge
            .threads()
            .await
            .iter()
            .filter(|t| t.account == id)
            .count();
    if live > 0 {
        return Err(ApiError::conflict(
            "account_in_use",
            format!("{live} session(s) Mother Claude started still run under this account."),
        ));
    }
    let account = state.accounts.remove(&id).map_err(account_error)?;
    if bridge_default {
        // The stopped main API's remembered choice: the next start takes the
        // project's account, else the accounts' bridge default (and so no
        // longer only that one).
        let forget = |d: &mut crate::bridge::control::BridgeDefaults| {
            if d.account.as_deref() == Some(id.as_str()) {
                d.account = None;
                d.account_only = false;
            }
        };
        main.update_defaults(forget);
        main.update_saved(forget);
    }
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
    ManageData(ctx): ManageData,
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
    auth::note_device_action(&state, &ctx, "changed the account defaults");
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
    ManageData(ctx): ManageData,
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
    auth::note_device_action(
        &state,
        &ctx,
        "changed the copy consent between two accounts",
    );
    view::publish(&state).await;
    Ok(Json(view::view(&state, ctx.is_desktop()).await))
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
    /// The email the sign-in page asks for (`--email`); without it, the one
    /// the account is signed in as, if known.
    #[serde(default)]
    pub email: Option<String>,
}

/// `POST /api/accounts/{id}/login`.
pub async fn login(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<LoginBody>,
) -> ApiResult<Response> {
    known(&state, &id)?;
    let options = LoginOptions {
        console: body.console,
        email: body.email.filter(|e| !e.trim().is_empty()),
    };
    signin::start_login(&state, &id, options)
        .await
        .map_err(|e| sign_in_error(e, "login_failed"))?;
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
    let job = state.accounts.runtime.sign_in(&id);
    let capture = job
        .as_ref()
        .filter(|j| j.kind == crate::claude::accounts::capture::SignInKind::Token)
        .map(|j| j.capture_view());
    Ok(Json(json!({
        "available": crate::claude::accounts::capture::AVAILABLE,
        "capture": capture,
        "signIn": job.map(|j| j.view()),
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
            let run = signin::submit_code(&state, &id, c).map_err(|e| match e {
                Failure::SignIn(SignInError::NotRunning) => {
                    ApiError::conflict("capture_not_running", e.to_string())
                }
                other => sign_in_error(other, "capture_failed"),
            })?;
            Ok(Json(run_body(Some(run))).into_response())
        }
        (None, None) => {
            let run = signin::start_token(&state, &id)
                .await
                .map_err(|e| sign_in_error(e, "capture_failed"))?;
            audit(
                &state,
                &ctx,
                "account.token",
                json!({ "account": id, "how": "setup-token" }),
            );
            Ok((StatusCode::ACCEPTED, Json(run_body(Some(run)))).into_response())
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
    let run = signin::cancel(&state, &id);
    view::publish(&state).await;
    Ok(Json(run_body(run)))
}

/// `GET /api/accounts/{id}/sign-in/open`: the browsers a link can be opened
/// in on this machine.
pub async fn sign_in_browsers(
    DesktopOnly(_ctx): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    known(&state, &id)?;
    Ok(Json(json!({ "browsers": signin::browsers(&state) })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenBody {
    pub browser: Browser,
}

/// `POST /api/accounts/{id}/sign-in/open {browser}`.
pub async fn sign_in_open(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<OpenBody>,
) -> ApiResult<Json<serde_json::Value>> {
    known(&state, &id)?;
    let view =
        signin::open(&state, &id, body.browser).map_err(|e| sign_in_error(e, "browser_failed"))?;
    audit(
        &state,
        &ctx,
        "account.sign_in.open",
        json!({ "account": id, "browser": body.browser }),
    );
    view::publish(&state).await;
    Ok(Json(json!({ "signIn": view })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeBody {
    pub code: String,
}

/// `POST /api/accounts/{id}/sign-in/code {code}`.
pub async fn sign_in_code(
    DesktopOnly(_ctx): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<CodeBody>,
) -> ApiResult<Json<serde_json::Value>> {
    known(&state, &id)?;
    let run = signin::submit_code(&state, &id, &body.code)
        .map_err(|e| sign_in_error(e, "sign_in_failed"))?;
    view::publish(&state).await;
    Ok(Json(json!({ "signIn": run.sign_in })))
}

/// `DELETE /api/accounts/{id}/sign-in`.
pub async fn sign_in_cancel(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    known(&state, &id)?;
    let view =
        signin::cancel_running(&state, &id).map_err(|e| sign_in_error(e, "sign_in_failed"))?;
    audit(
        &state,
        &ctx,
        "account.sign_in.cancel",
        json!({ "account": id }),
    );
    view::publish(&state).await;
    Ok(Json(json!({ "signIn": view })))
}

/// `POST /api/accounts/{id}/identity/accept`: keep who the account signed in
/// as now, so its identity warning goes (docs/ACCOUNTS.md §15.3). Without a
/// warning there is nothing to keep, and the account comes back as it is.
pub async fn accept_identity(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<AccountView>> {
    known(&state, &id)?;
    if let Some(warning) = state.accounts.identity_warnings().remove(&id) {
        state.accounts.accept_identity(&id).map_err(account_error)?;
        audit(
            &state,
            &ctx,
            "account.identity.accept",
            json!({ "account": id, "kind": warning.kind, "other": warning.account }),
        );
        view::publish(&state).await;
    }
    Ok(Json(one(&state, &id)?))
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
