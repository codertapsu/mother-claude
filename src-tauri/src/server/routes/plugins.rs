//! `/api/plugins…`: Claude Code's plugins (docs/PLUGINS.md,
//! `claude::plugins`). Reading — what is installed and where, the
//! marketplaces, the catalog — is for any signed-in caller. Every change is
//! the desktop's, on this computer (`DesktopOnly` + local): it runs
//! `claude plugin …`, which installs code that runs here, and Claude Code
//! writes its own settings. Each change is audited (the plugin, the scope,
//! never a project's path).

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::Body;
use crate::claude::plugins::{self, Change, Scope};
use crate::server::auth::{Authed, DesktopOnly};
use crate::server::context::ClientContext;
use crate::server::devices::audit::AuditEntry;
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ListQuery {
    /// The Claude account whose plugins (none: the Default).
    pub account: Option<String>,
    /// A project folder to see its own plugins from.
    pub project: Option<String>,
    /// A search of the catalog.
    pub q: Option<String>,
}

fn project_of(state: &AppState, raw: Option<&str>) -> ApiResult<Option<std::path::PathBuf>> {
    raw.map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| plugins::check_project(state, p))
        .transpose()
}

/// `GET /api/plugins?account=&project=` → `{installed, marketplaces,
/// project}`.
pub async fn list(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Value>> {
    let project = project_of(&state, q.project.as_deref())?;
    Ok(Json(
        plugins::overview(&state, q.account.as_deref(), project.as_deref()).await?,
    ))
}

/// `GET /api/plugins/available?q=&account=` → `{plugins, total, catalog}`:
/// the marketplaces' plugins matching the search, most installed first.
pub async fn available(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Value>> {
    let search: String = q.q.unwrap_or_default().chars().take(200).collect();
    Ok(Json(
        plugins::available(&state, q.account.as_deref(), &search).await?,
    ))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChangeBody {
    /// `name@marketplace`.
    pub plugin: String,
    /// `user` (everywhere, the default), `project` or `local`.
    pub scope: Option<Scope>,
    /// The project folder of a `project` or `local` scope.
    pub project: Option<String>,
    pub account: Option<String>,
    /// The SHA-256 of the marketplace-declared command the person saw.
    pub accept_command: Option<String>,
    /// `POST /plugins/enabled`: on or off.
    pub enabled: Option<bool>,
}

fn on_this_computer(ctx: &ClientContext) -> ApiResult<()> {
    if ctx.local {
        Ok(())
    } else {
        Err(ApiError::dangerous_blocked(
            "Plugins are installed and changed on this computer, in Mother Claude's own window.",
        ))
    }
}

fn audit(state: &AppState, ctx: &ClientContext, action: &str, detail: Value) {
    state.devices.audit.record(AuditEntry {
        detail: Some(detail),
        ..AuditEntry::from_ctx(ctx, action)
    });
}

async fn run_change(
    state: &AppState,
    ctx: &ClientContext,
    change: Change,
    body: ChangeBody,
) -> ApiResult<Json<Value>> {
    on_this_computer(ctx)?;
    let scope = body.scope.unwrap_or(Scope::User);
    let project = if scope.needs_project() {
        Some(project_of(state, body.project.as_deref())?.ok_or_else(|| {
            ApiError::unprocessable("invalid_project", "Choose the project this is for.")
        })?)
    } else {
        None
    };
    let action = format!("plugins.{change:?}").to_lowercase();
    let v = plugins::change(
        state,
        change,
        &body.plugin,
        scope,
        project.as_deref(),
        body.account.as_deref(),
        body.accept_command.as_deref(),
    )
    .await?;
    audit(
        state,
        ctx,
        &action,
        json!({
            "plugin": body.plugin,
            "scope": scope.as_str(),
            "account": body.account,
            "acceptedCommand": body.accept_command.is_some(),
        }),
    );
    Ok(Json(v))
}

/// `POST /api/plugins/install {plugin, scope?, project?, account?,
/// acceptCommand?}` (desktop). `409 confirm_command {command, sha256}`
/// when the marketplace runs a command to install it: shown to the person,
/// then sent again with `acceptCommand`.
pub async fn install(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Body(body): Body<ChangeBody>,
) -> ApiResult<Json<Value>> {
    run_change(&state, &ctx, Change::Install, body).await
}

/// `POST /api/plugins/uninstall {plugin, scope?, project?, account?}`
/// (desktop).
pub async fn uninstall(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Body(body): Body<ChangeBody>,
) -> ApiResult<Json<Value>> {
    run_change(&state, &ctx, Change::Uninstall, body).await
}

/// `POST /api/plugins/enabled {plugin, enabled, scope?, project?,
/// account?}` (desktop): on or off where the scope says.
pub async fn set_enabled(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Body(body): Body<ChangeBody>,
) -> ApiResult<Json<Value>> {
    let change = match body.enabled {
        Some(true) => Change::Enable,
        Some(false) => Change::Disable,
        None => {
            return Err(ApiError::unprocessable(
                "invalid_body",
                "Say whether to turn it on or off.",
            ))
        }
    };
    run_change(&state, &ctx, change, body).await
}

/// `POST /api/plugins/update {plugin, scope?, project?, account?,
/// acceptCommand?}` (desktop).
pub async fn update(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Body(body): Body<ChangeBody>,
) -> ApiResult<Json<Value>> {
    run_change(&state, &ctx, Change::Update, body).await
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct MarketplaceBody {
    /// GitHub `owner/repo`, a git URL or a folder on this computer.
    pub source: String,
    pub account: Option<String>,
}

/// `POST /api/plugins/marketplaces {source, account?}` (desktop): adds a
/// marketplace for this person, everywhere.
pub async fn add_marketplace(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Body(body): Body<MarketplaceBody>,
) -> ApiResult<Json<Value>> {
    on_this_computer(&ctx)?;
    let v = plugins::add_marketplace(&state, &body.source, body.account.as_deref()).await?;
    // A folder's path stays out of the audit; a repository's name is kept.
    let source = if std::path::Path::new(body.source.trim()).is_absolute() {
        "a folder".to_string()
    } else {
        body.source.trim().chars().take(300).collect()
    };
    audit(
        &state,
        &ctx,
        "plugins.marketplace.add",
        json!({ "source": source, "account": body.account }),
    );
    Ok(Json(v))
}

/// `DELETE /api/plugins/marketplaces/{name}?account=` (desktop).
pub async fn remove_marketplace(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Value>> {
    on_this_computer(&ctx)?;
    let v = plugins::remove_marketplace(&state, &name, q.account.as_deref()).await?;
    audit(
        &state,
        &ctx,
        "plugins.marketplace.remove",
        json!({ "marketplace": name, "account": q.account }),
    );
    Ok(Json(v))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ReloadAllBody {
    /// Only the sessions of this account (none: every account's).
    pub account: Option<String>,
}

/// `POST /api/plugins/reload {account?}` (desktop) → `{reloaded, held,
/// busy, failed}`: every session Mother Claude runs that is between turns
/// reads its plugins again, as after a change here. One whose next message
/// would then re-read its whole conversation is left as it is (`held`), as
/// is one that works (`busy`): their Info tab reloads them.
pub async fn reload_everywhere(
    DesktopOnly(_ctx): DesktopOnly,
    State(state): State<AppState>,
    Body(body): Body<ReloadAllBody>,
) -> ApiResult<Json<Value>> {
    let want = body
        .account
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty());
    let (mut reloaded, mut held, mut busy, mut failed) = (0u32, 0u32, 0u32, 0u32);
    for (id, account) in state.control.accounts() {
        if want.is_some_and(|a| a != account) {
            continue;
        }
        if state.control.is_idle(&id) != Some(true) {
            busy += 1;
            continue;
        }
        match crate::claude::owned::reload_plugins(&state, &id, false, None).await {
            Ok(v) if v["held"] == true => held += 1,
            Ok(_) => reloaded += 1,
            Err(e) if e.message.contains("busy") => busy += 1,
            Err(e) => {
                tracing::debug!(session = %id, error = %e.message, "reload_plugins");
                failed += 1;
            }
        }
    }
    Ok(Json(json!({
        "reloaded": reloaded,
        "held": held,
        "busy": busy,
        "failed": failed,
    })))
}
