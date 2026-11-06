//! `/api/browser/*`: the Browser page (desktop and phone, one code path).
//!
//! | route | guard |
//! |---|---|
//! | `GET /api/browser` — browsers (with connection state), settings, site modes, the sensitive list, grants, leases, Stops, waiting approvals | `Authed` |
//! | `GET /api/browser/activity?session=&limit=` — recent actions (no values) | `Authed` |
//! | `POST /api/browser/pair` — a pairing code | dangerous right (the desktop) |
//! | `DELETE /api/browser/browsers/{id}` — revoke | dangerous right |
//! | `PUT /api/browser/sites {origin, mode}` — set (`null`: remove) a site mode | `ManageData`; `act` on a sensitive site, on `*` or on a wildcard needs the dangerous right |
//! | `PUT /api/browser/settings` — screenshots, default mode, the per-minute limit, the pinned browser, upload folders | `ManageData`; default `act` and new upload folders need the dangerous right |
//! | `POST /api/browser/grants {action, origin}` — a standing grant | dangerous right |
//! | `DELETE /api/browser/grants/{id}` | `ManageData` |
//! | `POST /api/browser/sessions/{id}/stop` — stop a session's browser use | `Authed` |
//! | `POST /api/browser/sessions/{id}/resume` — undo a Stop | `ManageData` |
//!
//! Every change is an audit line; a paired device's changes are named to
//! the desktop.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::sites::{self, GrantAction, SiteMode, DEFAULT_KEY};
use crate::server::auth::{note_device_action, require_dangerous, Authed, ManageData};
use crate::server::context::ClientContext;
use crate::server::error::{ApiError, ApiResult};
use crate::server::routes::Body;
use crate::state::AppState;

/// The whole page.
pub fn overview(state: &AppState) -> Value {
    let ext = &state.extension;
    let settings = ext.settings.get();
    let browsers: Vec<Value> = ext
        .browsers
        .list()
        .into_iter()
        .map(|b| {
            let conn = ext.hub.get(&b.id);
            let mut v = json!(b);
            v["connected"] = json!(conn.is_some());
            if let Some(c) = conn {
                v["connectedAt"] = json!(c.connected_at);
                v["lastActiveAt"] = json!(c.last_active());
                v["capabilities"] = c.capabilities.lock().map(|g| g.clone()).unwrap_or_default();
                v["permissions"] =
                    json!(c.permissions.lock().map(|g| g.clone()).unwrap_or_default());
            }
            v["pinned"] = json!(settings.pinned_browser.as_deref() == Some(b.id.as_str()));
            v
        })
        .collect();
    let mut site_list: Vec<Value> = vec![json!({
        "origin": DEFAULT_KEY,
        "mode": settings.default_mode,
    })];
    site_list.extend(settings.sites.iter().map(|(origin, mode)| {
        let host = sites::host_of(origin);
        let sensitive = (!origin.contains("://*."))
            .then(|| sites::sensitive_category(host))
            .flatten();
        json!({ "origin": origin, "mode": mode, "sensitive": sensitive })
    }));
    let sensitive: Vec<Value> = sites::SENSITIVE
        .iter()
        .map(|(domain, category)| json!({ "domain": domain, "category": category }))
        .collect();
    let leases: Vec<Value> = ext
        .leases
        .all()
        .into_iter()
        .map(|l| {
            let connected = ext.hub.get(&l.browser).is_some();
            let mut v = json!(l);
            v["connected"] = json!(connected);
            v
        })
        .collect();
    json!({
        "browsers": browsers,
        "settings": {
            "defaultMode": settings.default_mode,
            "screenshots": settings.screenshots,
            "actionsPerMinute": settings.actions_per_minute,
            "pinnedBrowser": settings.pinned_browser,
            "uploadFolders": settings.upload_folders,
        },
        "sites": site_list,
        "sensitive": sensitive,
        "grants": settings.grants,
        "leases": leases,
        "stops": ext.leases.stops(),
        "approvals": ext.approvals_for(None),
        "pairing": { "liveCodes": ext.codes.live_count() },
        "endpoint": format!("ws://127.0.0.1:{}/ws/browser", state.config.port),
    })
}

/// `GET /api/browser`.
pub async fn get_overview(Authed(_ctx): Authed, State(state): State<AppState>) -> Json<Value> {
    Json(overview(&state))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityQuery {
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// `GET /api/browser/activity`.
pub async fn get_activity(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Query(q): Query<ActivityQuery>,
) -> ApiResult<Json<Value>> {
    if let Some(s) = q.session.as_deref() {
        crate::claude::ids::require_session_id(s)?;
    }
    let limit = q.limit.unwrap_or(100).clamp(1, 500);
    Ok(Json(json!({
        "entries": state.extension.activity.recent(q.session.as_deref(), limit),
    })))
}

/// `POST /api/browser/pair` → a code for the extension.
pub async fn post_pair(
    Authed(ctx): Authed,
    State(state): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    require_dangerous(&ctx, "Pairing a browser")?;
    let code = state.extension.codes.create();
    super::activity::audit_change(&state, Some(&ctx), "browser.pair.code", json!({}));
    note_device_action(&state, &ctx, "created a browser pairing code");
    Ok((StatusCode::CREATED, Json(json!(code))))
}

/// `DELETE /api/browser/browsers/{id}`.
pub async fn delete_browser(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require_dangerous(&ctx, "Removing a browser")?;
    let ext = &state.extension;
    let view = ext
        .browsers
        .revoke(&id)
        .ok_or_else(|| ApiError::not_found("browser_not_found", "No such paired browser."))?;
    // Its connection closes with `revoked`; its leases end (their approvals
    // are withdrawn).
    ext.hub.close_browser(&id, super::hub::CloseReason::Revoked);
    for lease in ext.leases.in_browser(&id) {
        super::actions::close_lease(&state, &lease, "browser removed").await;
    }
    if ext.settings.get().pinned_browser.as_deref() == Some(id.as_str()) {
        let _ = ext.settings.update(|s| {
            s.pinned_browser = None;
            Ok(())
        });
    }
    super::activity::audit_change(
        &state,
        Some(&ctx),
        "browser.revoked",
        json!({ "browser": id, "name": view.name }),
    );
    note_device_action(
        &state,
        &ctx,
        &format!("removed the browser \"{}\"", view.name),
    );
    super::notify(&state, "browsers");
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SiteBody {
    pub origin: String,
    /// `off` | `read` | `act`; `null` removes the site's own mode.
    #[serde(default)]
    pub mode: Option<String>,
}

/// Raising a site to `act` that needs the dangerous right: what it is.
fn act_needs_right(key: &str) -> Option<String> {
    if key == DEFAULT_KEY {
        return Some("Letting sessions act on every site".into());
    }
    if key.contains("://*.") {
        return Some(format!("Letting sessions act on every {key} site"));
    }
    sites::sensitive_category(sites::host_of(key))
        .map(|c| format!("Letting sessions act on {key}, a sensitive site ({c}),"))
}

/// `PUT /api/browser/sites`.
pub async fn put_site(
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Body(body): Body<SiteBody>,
) -> ApiResult<Json<Value>> {
    let key = sites::normalize_key(&body.origin)
        .map_err(|m| ApiError::unprocessable("invalid_site", m).with("field", "origin"))?;
    let mode = match body.mode.as_deref() {
        None => None,
        Some(m) => Some(SiteMode::parse(m).ok_or_else(|| {
            ApiError::unprocessable("invalid_mode", "The mode is one of off, read, act.")
                .with("field", "mode")
        })?),
    };
    if mode == Some(SiteMode::Act) {
        if let Some(what) = act_needs_right(&key) {
            require_dangerous(&ctx, &what)?;
        }
    }
    apply_site(&state, &ctx, &key, mode)?;
    Ok(Json(overview(&state)))
}

fn apply_site(
    state: &AppState,
    ctx: &ClientContext,
    key: &str,
    mode: Option<SiteMode>,
) -> ApiResult<()> {
    state
        .extension
        .settings
        .update(|s| {
            match (key, mode) {
                (DEFAULT_KEY, m) => s.default_mode = m.unwrap_or(SiteMode::Off),
                (k, Some(m)) => {
                    s.sites.insert(k.to_string(), m);
                }
                (k, None) => {
                    s.sites.remove(k);
                }
            }
            Ok(())
        })
        .map_err(ApiError::internal)?;
    let label = mode.map_or("the default", SiteMode::as_str);
    super::activity::audit_change(
        state,
        Some(ctx),
        "browser.site_mode",
        json!({ "origin": key, "mode": mode.map(SiteMode::as_str) }),
    );
    note_device_action(
        state,
        ctx,
        &format!("set the browser site mode of {key} to {label}"),
    );
    super::broadcast_state(state);
    super::notify(state, "sites");
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsBody {
    #[serde(default)]
    pub default_mode: Option<String>,
    #[serde(default)]
    pub screenshots: Option<bool>,
    #[serde(default)]
    pub actions_per_minute: Option<u32>,
    /// `""` unpins.
    #[serde(default)]
    pub pinned_browser: Option<String>,
    #[serde(default)]
    pub upload_folders: Option<Vec<String>>,
}

/// `PUT /api/browser/settings`.
pub async fn put_settings(
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Body(body): Body<SettingsBody>,
) -> ApiResult<Json<Value>> {
    let ext = &state.extension;
    let current = ext.settings.get();
    let default_mode = match body.default_mode.as_deref() {
        None => None,
        Some(m) => Some(SiteMode::parse(m).ok_or_else(|| {
            ApiError::unprocessable("invalid_mode", "The mode is one of off, read, act.")
                .with("field", "defaultMode")
        })?),
    };
    if default_mode == Some(SiteMode::Act) && current.default_mode != SiteMode::Act {
        require_dangerous(&ctx, "Letting sessions act on every site")?;
    }
    if let Some(n) = body.actions_per_minute {
        if !(1..=600).contains(&n) {
            return Err(ApiError::unprocessable(
                "invalid_limit",
                "The limit is between 1 and 600 actions a minute.",
            )
            .with("field", "actionsPerMinute"));
        }
    }
    let pinned = match body.pinned_browser.as_deref() {
        None => None,
        Some("") => Some(None),
        Some(id) => {
            if ext.browsers.get(id).is_none_or(|b| b.revoked) {
                return Err(ApiError::unprocessable(
                    "browser_not_found",
                    "No such paired browser.",
                )
                .with("field", "pinnedBrowser"));
            }
            Some(Some(id.to_string()))
        }
    };
    let folders = match &body.upload_folders {
        None => None,
        Some(list) => {
            let mut out = Vec::new();
            for raw in list.iter().take(20) {
                let p = std::path::Path::new(raw.trim());
                if !p.is_absolute() {
                    return Err(ApiError::unprocessable(
                        "invalid_folder",
                        format!("{raw} is not a full path."),
                    )
                    .with("field", "uploadFolders"));
                }
                let resolved = dunce::canonicalize(p).map_err(|_| {
                    ApiError::unprocessable("invalid_folder", format!("{raw} does not exist."))
                        .with("field", "uploadFolders")
                })?;
                if !resolved.is_dir() {
                    return Err(ApiError::unprocessable(
                        "invalid_folder",
                        format!("{raw} is not a folder."),
                    )
                    .with("field", "uploadFolders"));
                }
                let text = resolved.to_string_lossy().into_owned();
                if !out.contains(&text) {
                    out.push(text);
                }
            }
            if out.iter().any(|f| !current.upload_folders.contains(f)) {
                require_dangerous(&ctx, "Letting sessions upload files from a new folder")?;
            }
            Some(out)
        }
    };
    ext.settings
        .update(|s| {
            if let Some(m) = default_mode {
                s.default_mode = m;
            }
            if let Some(on) = body.screenshots {
                s.screenshots = on;
            }
            if let Some(n) = body.actions_per_minute {
                s.actions_per_minute = n;
            }
            if let Some(p) = pinned.clone() {
                s.pinned_browser = p;
            }
            if let Some(f) = folders.clone() {
                s.upload_folders = f;
            }
            Ok(())
        })
        .map_err(ApiError::internal)?;
    super::activity::audit_change(
        &state,
        Some(&ctx),
        "browser.settings",
        json!({
            "defaultMode": default_mode.map(SiteMode::as_str),
            "screenshots": body.screenshots,
            "actionsPerMinute": body.actions_per_minute,
            "pinnedBrowser": pinned,
            "uploadFolders": folders.as_ref().map(Vec::len),
        }),
    );
    note_device_action(&state, &ctx, "changed the browser settings");
    super::broadcast_state(&state);
    super::notify(&state, "settings");
    Ok(Json(overview(&state)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GrantBody {
    pub action: String,
    pub origin: String,
}

/// `POST /api/browser/grants`.
pub async fn post_grant(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Body(body): Body<GrantBody>,
) -> ApiResult<impl IntoResponse> {
    let action = GrantAction::parse(&body.action).ok_or_else(|| {
        ApiError::unprocessable(
            "invalid_action",
            "The action is one of eval, upload, navigate, act, secretForm, burst.",
        )
        .with("field", "action")
    })?;
    let origin = sites::origin_of(&body.origin).ok_or_else(|| {
        ApiError::unprocessable("invalid_site", "Give an origin like https://github.com.")
            .with("field", "origin")
    })?;
    require_dangerous(&ctx, "A standing browser grant")?;
    let by = match ctx.device_id().and_then(|id| state.devices.get(id)) {
        Some(d) => d.name,
        None => crate::claude::paths::machine_noun().to_string(),
    };
    let id = super::actions::add_grant(&state, action, &origin, &by, Some(&ctx))
        .map_err(ApiError::internal)?;
    note_device_action(
        &state,
        &ctx,
        &format!("allowed sessions to {} on {origin}", action.label()),
    );
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

/// `DELETE /api/browser/grants/{id}`.
pub async fn delete_grant(
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let removed = state
        .extension
        .settings
        .update(|s| {
            let before = s.grants.len();
            s.grants.retain(|g| g.id != id);
            Ok(before != s.grants.len())
        })
        .map_err(ApiError::internal)?;
    if !removed {
        return Err(ApiError::not_found("grant_not_found", "No such grant."));
    }
    super::activity::audit_change(
        &state,
        Some(&ctx),
        "browser.grant.revoke",
        json!({ "grant": id }),
    );
    note_device_action(&state, &ctx, "removed a standing browser grant");
    super::notify(&state, "grants");
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/browser/sessions/{id}/stop`.
pub async fn post_stop(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    crate::claude::ids::require_session_id(&id)?;
    let by = if ctx.is_desktop() {
        "desktop"
    } else {
        "device"
    };
    let stop = super::actions::stop_session(&state, &id, by)
        .await
        .ok_or_else(|| {
            ApiError::conflict("not_browsing", "This session is not using the browser.")
        })?;
    super::activity::audit_change(
        &state,
        Some(&ctx),
        "browser.stop",
        json!({ "session": id, "by": by }),
    );
    Ok(Json(json!(stop)))
}

/// `POST /api/browser/sessions/{id}/resume`.
pub async fn post_resume(
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    crate::claude::ids::require_session_id(&id)?;
    if !state.extension.leases.resume(&id) {
        return Err(ApiError::conflict(
            "not_stopped",
            "This session's browser use is not stopped.",
        ));
    }
    super::activity::audit_change(
        &state,
        Some(&ctx),
        "browser.resume",
        json!({ "session": id }),
    );
    note_device_action(&state, &ctx, "resumed a session's browser use");
    super::notify(&state, "leases");
    Ok(StatusCode::NO_CONTENT)
}
