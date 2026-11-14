//! The Teams page's HTTP routes (`/api/team-templates*`, `/api/teams*`).
//!
//! Who may do what: anyone authenticated reads, starts and stops teams and
//! roles and works the board (creating, assigning, reprioritising and
//! cancelling tasks — a phone can follow and steer a team); writing
//! templates is the desktop's or a paired device's (`ManageData`, never the
//! local-script token); starting a team with a role that runs without
//! permission prompts needs the dangerous right, and so does giving a role
//! to a foreign session that runs without them. Every change is audited
//! (`team.*`) and a device's is told to the desktop.

use std::collections::BTreeMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::board::{Actor, BoardError, TaskDraft, TaskPatch};
use super::teams::{self, RoleOverride, Starter, Team};
use super::templates::{self, Template, TemplateError};
use crate::claude::owned::{self, Caller, EFFORT_LEVELS, PERMISSION_MODES};
use crate::server::auth::{note_device_action, require_dangerous, Authed, ManageData};
use crate::server::context::ClientContext;
use crate::server::devices::audit::AuditEntry;
use crate::server::error::{ApiError, ApiResult};
use crate::server::routes::Body;
use crate::state::AppState;

/// Templates carry charters: larger bodies than the default limit.
pub const TEMPLATE_BODY_MAX: usize = 1024 * 1024;
/// The longest brief for a team's lead.
const BRIEF_MAX: usize = 20_000;

/// The teams' routes, merged into the protected `/api` router (every one
/// passes `auth::require_api_principal`, then its own guard).
pub fn router() -> axum::Router<AppState> {
    use axum::extract::DefaultBodyLimit;
    use axum::routing::{get, post};
    axum::Router::new()
        .route(
            "/team-templates",
            get(list_templates)
                .post(create_template)
                .layer(DefaultBodyLimit::max(TEMPLATE_BODY_MAX)),
        )
        .route(
            "/team-templates/validate",
            post(validate_template).layer(DefaultBodyLimit::max(TEMPLATE_BODY_MAX)),
        )
        .route(
            "/team-templates/{id}",
            axum::routing::put(put_template)
                .delete(delete_template)
                .layer(DefaultBodyLimit::max(TEMPLATE_BODY_MAX)),
        )
        .route("/team-templates/{id}/duplicate", post(duplicate_template))
        .route(
            "/teams",
            get(list_teams)
                .post(start_team)
                .layer(DefaultBodyLimit::max(TEMPLATE_BODY_MAX)),
        )
        .route("/teams/events", get(observed))
        .route("/teams/{id}", get(get_team).delete(delete_team))
        .route("/teams/{id}/stop", post(stop_team))
        .route("/teams/{id}/resume", post(resume_team))
        .route("/teams/{id}/roles/{role}/start", post(start_role))
        .route("/teams/{id}/roles/{role}/stop", post(stop_role))
        .route(
            "/teams/{id}/roles/{role}/take",
            post(take_role).delete(release_role),
        )
        .route(
            "/teams/{id}/tasks",
            get(list_tasks)
                .post(create_task)
                .layer(DefaultBodyLimit::max(TEMPLATE_BODY_MAX)),
        )
        .route(
            "/teams/{id}/tasks/{task}",
            get(get_task)
                .patch(patch_task)
                .layer(DefaultBodyLimit::max(TEMPLATE_BODY_MAX)),
        )
}

fn now_ms() -> i64 {
    crate::claude::registry::now_ms()
}

fn template_error(e: TemplateError) -> ApiError {
    match &e {
        TemplateError::NotFound(_) => ApiError::not_found(e.code(), e.to_string()),
        TemplateError::Exists(_) | TemplateError::Builtin(_) => {
            ApiError::conflict(e.code(), e.to_string())
        }
        TemplateError::Invalid(_) => ApiError::unprocessable(e.code(), e.to_string()),
        TemplateError::Io(_) => ApiError::internal(e.to_string()),
    }
}

fn board_error(e: BoardError) -> ApiError {
    match &e {
        BoardError::NotFound(_) => ApiError::not_found("task_not_found", e.to_string()),
        BoardError::Denied(_) => ApiError::forbidden("denied", e.to_string()),
        BoardError::Invalid(_) => ApiError::unprocessable("invalid_task", e.to_string()),
        BoardError::Conflict(_) => ApiError::conflict("task_conflict", e.to_string()),
    }
}

/// How the history and the audit name the caller.
fn who(state: &AppState, ctx: &ClientContext) -> String {
    if let Some(id) = ctx.device_id() {
        return state
            .devices
            .get(id)
            .map(|d| d.name)
            .unwrap_or_else(|| "a paired device".into());
    }
    if ctx.is_desktop() {
        "desktop".into()
    } else {
        ctx.principal.label().to_string()
    }
}

fn audit(state: &AppState, ctx: &ClientContext, action: &str, detail: Value) {
    state.devices.audit.record(AuditEntry {
        detail: Some(detail),
        ..AuditEntry::from_ctx(ctx, action)
    });
}

fn team_of(state: &AppState, id: &str) -> ApiResult<Team> {
    if !teams::is_team_id(id) {
        return Err(ApiError::not_found("team_not_found", "No such team."));
    }
    state
        .teams
        .get(id)
        .ok_or_else(|| ApiError::not_found("team_not_found", "No such team."))
}

fn template_from(body: Value) -> ApiResult<Template> {
    let v = body.get("template").cloned().unwrap_or(body);
    if !v.is_object() {
        return Err(ApiError::unprocessable(
            "invalid_template",
            "A template is a JSON object.",
        ));
    }
    serde_json::from_value(v)
        .map_err(|e| ApiError::unprocessable("invalid_template", format!("Not a template: {e}")))
}

// ─────────────────────────────── templates ───────────────────────────────

/// What a template's roles name of the paired computers: a warning for each
/// computer no pairing matches (a start refuses them).
fn machine_warnings(state: &AppState, t: &Template) -> Vec<templates::Issue> {
    templates::machine_warnings(t, &|spec| {
        crate::machines::work::roles::spec_warning(state, spec)
    })
}

/// `GET /api/team-templates` → `{templates: [{…template, builtin, report}]}`.
pub async fn list_templates(Authed(_ctx): Authed, State(state): State<AppState>) -> Json<Value> {
    let mut all = state.teams.templates.all();
    for e in &mut all {
        let extra = machine_warnings(&state, &e.template);
        e.report.warnings.extend(extra);
    }
    Json(json!({ "templates": all }))
}

/// `POST /api/team-templates/validate` (a template) → `{ok, errors,
/// warnings}`: what saving it would say, without saving.
pub async fn validate_template(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Body(body): Body<Value>,
) -> Json<Value> {
    match template_from(body) {
        Ok(t) => {
            let mut r = templates::validate(&t);
            r.warnings.extend(machine_warnings(&state, &t));
            Json(json!({ "ok": r.ok(), "errors": r.errors, "warnings": r.warnings }))
        }
        Err(e) => Json(json!({
            "ok": false,
            "errors": [{ "path": "", "message": e.message }],
            "warnings": [],
        })),
    }
}

/// `POST /api/team-templates` (a template) → `201` the template.
pub async fn create_template(
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Body(body): Body<Value>,
) -> ApiResult<Response> {
    let t = template_from(body)?;
    let saved = state
        .teams
        .templates
        .save(t, true)
        .map_err(template_error)?;
    audit(
        &state,
        &ctx,
        "team.template.create",
        json!({ "template": saved.id }),
    );
    note_device_action(
        &state,
        &ctx,
        &format!("added the team template {:?}", saved.name),
    );
    super::delegation::publish_templates(&state);
    Ok((StatusCode::CREATED, Json(saved)).into_response())
}

/// `PUT /api/team-templates/{id}` (a template) → the template.
pub async fn put_template(
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<Value>,
) -> ApiResult<Json<Template>> {
    let t = template_from(body)?;
    if t.id != id {
        return Err(ApiError::unprocessable(
            "template_id_mismatch",
            "The template's id is the one in the address (duplicate it to change the id).",
        ));
    }
    let saved = state
        .teams
        .templates
        .save(t, false)
        .map_err(template_error)?;
    audit(
        &state,
        &ctx,
        "team.template.update",
        json!({ "template": saved.id }),
    );
    note_device_action(
        &state,
        &ctx,
        &format!("changed the team template {:?}", saved.name),
    );
    super::delegation::publish_templates(&state);
    Ok(Json(saved))
}

/// `DELETE /api/team-templates/{id}` → `204`.
pub async fn delete_template(
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    state.teams.templates.remove(&id).map_err(template_error)?;
    audit(
        &state,
        &ctx,
        "team.template.delete",
        json!({ "template": id }),
    );
    note_device_action(&state, &ctx, &format!("removed the team template {id:?}"));
    super::delegation::publish_templates(&state);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct DuplicateBody {
    pub name: Option<String>,
}

/// `POST /api/team-templates/{id}/duplicate {name?}` → `201` the copy.
pub async fn duplicate_template(
    ManageData(ctx): ManageData,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<DuplicateBody>,
) -> ApiResult<Response> {
    let copy = state
        .teams
        .templates
        .duplicate(&id, body.name.as_deref())
        .map_err(template_error)?;
    audit(
        &state,
        &ctx,
        "team.template.create",
        json!({ "template": copy.id, "from": id }),
    );
    note_device_action(
        &state,
        &ctx,
        &format!("duplicated the team template {id:?}"),
    );
    super::delegation::publish_templates(&state);
    Ok((StatusCode::CREATED, Json(copy)).into_response())
}

// ─────────────────────────────── teams ───────────────────────────────

/// `GET /api/teams` → `{teams: [TeamView]}`.
pub async fn list_teams(Authed(_ctx): Authed, State(state): State<AppState>) -> Json<Value> {
    let mut views = Vec::new();
    for t in state.teams.all() {
        views.push(teams::view(&state, &t).await);
    }
    views.sort_by_key(|v| std::cmp::Reverse(v.created_at));
    Json(json!({ "teams": views }))
}

/// `GET /api/teams/events` → `{events: [...]}`: what interactive sessions'
/// agent teams reported through the hooks (observe only).
pub async fn observed(Authed(_ctx): Authed, State(state): State<AppState>) -> Json<Value> {
    let mut events = state.teams.observed();
    events.reverse();
    Json(json!({ "events": events }))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StartBody {
    pub project: String,
    pub template: String,
    pub account: Option<String>,
    pub overrides: BTreeMap<String, RoleOverride>,
    pub brief: Option<String>,
}

fn check_override(role: &str, o: &RoleOverride) -> ApiResult<()> {
    let bad = |what: &str| {
        ApiError::unprocessable(
            "invalid_override",
            format!("The {role} role's {what} is not one Claude Code takes."),
        )
    };
    if let Some(m) = &o.model {
        owned::validate_model(m).map_err(|_| bad("model"))?;
    }
    if o.effort
        .as_deref()
        .is_some_and(|e| !EFFORT_LEVELS.contains(&e))
    {
        return Err(bad("effort"));
    }
    if o.permission_mode
        .as_deref()
        .is_some_and(|m| !PERMISSION_MODES.contains(&m))
    {
        return Err(bad("permission mode"));
    }
    if o.budget_usd
        .is_some_and(|b| !(b > 0.0 && b <= templates::BUDGET_MAX_USD))
    {
        return Err(bad("budget"));
    }
    if o.machine.as_deref().is_some_and(|m| {
        m.chars().count() > templates::MACHINE_MAX || m.chars().any(char::is_control)
    }) {
        return Err(bad("computer"));
    }
    Ok(())
}

/// `POST /api/teams {project, template, account?, overrides?, brief?}` →
/// `201` the team: a template applied to a project folder and an account.
pub async fn start_team(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Body(body): Body<StartBody>,
) -> ApiResult<Response> {
    let (mut template, _) = state
        .teams
        .templates
        .get(body.template.trim())
        .ok_or_else(|| ApiError::not_found("template_not_found", "No such template."))?;
    for (role, o) in &body.overrides {
        if template.role(role).is_none() {
            return Err(ApiError::unprocessable(
                "invalid_override",
                format!("The template has no role {role:?}."),
            ));
        }
        check_override(role, o)?;
    }
    template.roles = template
        .roles
        .iter()
        .map(|r| {
            body.overrides
                .get(&r.id)
                .map(|o| o.apply(r))
                .unwrap_or_else(|| r.clone())
        })
        .collect();
    let report = templates::validate(&template);
    if let Some(e) = report.first_error() {
        return Err(ApiError::unprocessable("invalid_template", e));
    }
    if template.needs_dangerous() {
        require_dangerous(
            &ctx,
            "Starting a team with a role that runs without permission prompts",
        )?;
    }
    let account = match body
        .account
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty())
    {
        Some(a) => a.to_string(),
        None => state.accounts.account_for_cwd(body.project.trim()),
    };
    state.accounts.env(&account).map_err(owned::account_error)?;
    let home = state
        .accounts
        .home(&account)
        .unwrap_or_else(|| state.home.clone());
    let caller = Caller::of(&ctx);
    let project = owned::launch_cwd(&home, &body.project, caller)?;
    if let Some(existing) = state.teams.for_project(&project) {
        return Err(ApiError::conflict(
            "team_exists",
            "This project has a team already: resume it, or remove it first.",
        )
        .with("team", existing.id));
    }
    if template.roles.iter().any(|r| r.worktree)
        && !std::path::Path::new(&project).join(".git").exists()
    {
        return Err(ApiError::unprocessable(
            "not_a_git_repository",
            "A role of this template works in a git worktree, and this folder is not a git \
             repository (its top folder).",
        ));
    }
    // A role on another computer: a paired computer must match it (online
    // and its permissions are its start's business), and it works at the
    // project's code, which travels as git.
    for role in template.roles.iter().filter(|r| !r.lead) {
        let Some(spec) = role.machine_spec() else {
            continue;
        };
        if let Some(why) = crate::machines::work::roles::spec_warning(&state, spec) {
            return Err(ApiError::unprocessable(
                "machine_unknown",
                format!("The {} role: {why}", role.title),
            )
            .with("role", role.id.clone()));
        }
        if !std::path::Path::new(&project).join(".git").exists() {
            return Err(ApiError::unprocessable(
                "not_a_git_repository",
                format!(
                    "The {} role runs on another computer, at the project's code: this folder \
                     must be a git repository (its top folder).",
                    role.title
                ),
            ));
        }
    }
    let brief = body
        .brief
        .as_deref()
        .map(|b| super::board::clean(b, BRIEF_MAX))
        .filter(|b| !b.is_empty());
    let label = state.teams.new_label(&project);
    let team = Team {
        id: teams::new_team_id(),
        project: project.clone(),
        label,
        template_id: template.id.clone(),
        template,
        account,
        overrides: body.overrides,
        created_at: now_ms(),
        stopped: false,
        starter: Starter {
            desktop: ctx.is_desktop(),
            may_dangerous: ctx.may_dangerous(),
            label: who(&state, &ctx),
        },
        roles: BTreeMap::new(),
        brief,
        extra: BTreeMap::new(),
    };
    let id = team.id.clone();
    state.teams.insert(team.clone());
    audit(
        &state,
        &ctx,
        "team.start",
        json!({ "team": id, "project": project, "template": team.template_id }),
    );
    note_device_action(
        &state,
        &ctx,
        &format!("started a {} team on {}", team.template.name, team.label),
    );
    super::delegation::publish_team(&state, &id).await;
    state.teams.nudge();
    let view = teams::view(&state, &team).await;
    Ok((StatusCode::CREATED, Json(view)).into_response())
}

/// `GET /api/teams/{id}` → the team.
pub async fn get_team(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<teams::TeamView>> {
    let team = team_of(&state, &id)?;
    Ok(Json(teams::view(&state, &team).await))
}

/// `POST /api/teams/{id}/stop`: every owned role session stops (between
/// turns or not), foreign ones give their roles back; nothing starts until
/// it is resumed.
pub async fn stop_team(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<teams::TeamView>> {
    let team = team_of(&state, &id)?;
    // Stopped first: the loop starts nothing for it from here.
    state.teams.update(&id, |t| t.stopped = true);
    audit(&state, &ctx, "team.stop", json!({ "team": id }));
    note_device_action(&state, &ctx, &format!("stopped the team on {}", team.label));
    // Every role, even when the caller goes away meanwhile.
    let (st, team_id, roles) = (state.clone(), id.clone(), team.template.roles.clone());
    super::launcher::to_the_end(async move {
        for role in &roles {
            super::launcher::stop_role(&st, &team_id, &role.id, "the team was stopped").await;
        }
        for role in &roles {
            st.teams.with_runtime(&team_id, &role.id, |r| {
                r.waiting = None;
                r.pending_notices.clear();
            });
        }
        super::delegation::publish_team(&st, &team_id).await;
    })
    .await;
    let team = team_of(&state, &id)?;
    Ok(Json(teams::view(&state, &team).await))
}

/// `POST /api/teams/{id}/resume`: a stopped team runs again (its lead and
/// the roles with work start as capacity allows).
pub async fn resume_team(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<teams::TeamView>> {
    let team = team_of(&state, &id)?;
    if team.template.needs_dangerous() {
        require_dangerous(
            &ctx,
            "Resuming a team with a role that runs without permission prompts",
        )?;
    }
    state.teams.update(&id, |t| t.stopped = false);
    audit(&state, &ctx, "team.resume", json!({ "team": id }));
    note_device_action(&state, &ctx, &format!("resumed the team on {}", team.label));
    state.teams.nudge();
    super::delegation::publish_team(&state, &id).await;
    let team = team_of(&state, &id)?;
    Ok(Json(teams::view(&state, &team).await))
}

/// `DELETE /api/teams/{id}` → `204`: forget a stopped team (its board stays
/// with the project).
pub async fn delete_team(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let team = team_of(&state, &id)?;
    if !team.stopped {
        return Err(ApiError::conflict(
            "team_running",
            "Stop the team before removing it.",
        ));
    }
    // Its roles on other computers: their worktrees and records go there
    // (when that computer can be reached), the bindings with them.
    let remote: Vec<String> = team
        .roles
        .iter()
        .filter(|(_, s)| s.remote.is_some())
        .map(|(r, _)| r.clone())
        .collect();
    if !remote.is_empty() {
        let (st, gone) = (state.clone(), team.clone());
        tokio::spawn(async move {
            for role in remote {
                crate::machines::work::roles::lead::remove(&st, &gone, &role).await;
            }
        });
    }
    state.teams.remove(&id);
    audit(&state, &ctx, "team.remove", json!({ "team": id }));
    note_device_action(&state, &ctx, &format!("removed the team on {}", team.label));
    super::delegation::publish_removed(&state, &id);
    Ok(StatusCode::NO_CONTENT)
}

fn role_of(team: &Team, role: &str) -> ApiResult<()> {
    team.role(role).map(|_| ()).ok_or_else(|| {
        ApiError::not_found("role_not_found", format!("No role {role:?} in this team."))
    })
}

/// `POST /api/teams/{id}/roles/{role}/start` → `{state, sessionId?,
/// reasons?}`: start (or resume) the role's session now, capacity allowing.
pub async fn start_role(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path((id, role)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let team = team_of(&state, &id)?;
    role_of(&team, &role)?;
    if team.stopped {
        return Err(ApiError::conflict("team_stopped", "Resume the team first."));
    }
    state.teams.update(&id, |t| {
        t.roles.entry(role.clone()).or_default().wanted = true;
    });
    audit(
        &state,
        &ctx,
        "team.role.start",
        json!({ "team": id, "role": role }),
    );
    let running = super::launcher::running_workers(&state).await;
    let next = state
        .teams
        .boards
        .read(&team.project, |b| b.next_for(&role).map(|t| t.id));
    let out = match super::launcher::start_role(&state, &id, &role, next, &running).await {
        Ok(sid) => json!({ "state": "started", "sessionId": sid }),
        Err(super::launcher::StartError::Waiting(reasons)) => {
            json!({ "state": "waiting", "reasons": reasons })
        }
        Err(super::launcher::StartError::Refused(m)) => {
            return Err(ApiError::conflict("role_not_started", m))
        }
        Err(super::launcher::StartError::Failed(m)) => {
            return Err(ApiError::unprocessable("role_not_started", m))
        }
    };
    Ok(Json(out))
}

/// `POST /api/teams/{id}/roles/{role}/stop`: stop the role's owned session
/// (a foreign one gives the role back instead).
pub async fn stop_role(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path((id, role)): Path<(String, String)>,
) -> ApiResult<Json<teams::TeamView>> {
    let team = team_of(&state, &id)?;
    role_of(&team, &role)?;
    super::launcher::stop_role(&state, &id, &role, "stopped from the Teams page").await;
    audit(
        &state,
        &ctx,
        "team.role.stop",
        json!({ "team": id, "role": role }),
    );
    let team = team_of(&state, &id)?;
    Ok(Json(teams::view(&state, &team).await))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TakeBody {
    pub session_id: String,
}

/// `POST /api/teams/{id}/roles/{role}/take {sessionId}`: a foreign live
/// session takes the role. It is never started or stopped by Mother
/// Claude; its packet and the board's notices reach it through the hooks
/// relay, and the guard holds it to the role's paths.
pub async fn take_role(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path((id, role)): Path<(String, String)>,
    Body(body): Body<TakeBody>,
) -> ApiResult<Json<teams::TeamView>> {
    let team = team_of(&state, &id)?;
    role_of(&team, &role)?;
    let sid = body.session_id.trim().to_string();
    if !owned::is_uuid(&sid) {
        return Err(ApiError::unprocessable(
            "invalid_session_id",
            "Name a session.",
        ));
    }
    let s = state
        .find_session(&sid)
        .await
        .ok_or_else(|| ApiError::no_such_session(&sid))?;
    if !s.live {
        return Err(ApiError::conflict(
            "session_not_live",
            "Only a running session can take a role.",
        ));
    }
    if s.owned || state.is_owned(&sid).await {
        return Err(ApiError::conflict(
            "session_owned",
            "Sessions Mother Claude runs are started for a role by the team itself.",
        ));
    }
    if !crate::claude::paths::is_within_recorded(&s.cwd, &team.project) {
        return Err(ApiError::conflict(
            "other_project",
            "That session works in another folder than the team's project.",
        ));
    }
    if !crate::server::hooks::is_active(&state, &sid) {
        return Err(ApiError::conflict(
            "hooks_inactive",
            "Sessions started elsewhere is not on in that session (turn it on on the desktop, \
             on the Sessions started elsewhere page, then run /reload-plugins there): Mother \
             Claude could neither guard its edits nor hand it tasks.",
        ));
    }
    if crate::server::hooks::effective_bypass(&state, &s).await {
        require_dangerous(
            &ctx,
            "Giving a role to a session without permission prompts",
        )?;
    }
    if let Some(b) = state.teams.binding(&sid) {
        return Err(ApiError::conflict(
            "session_has_role",
            format!("That session holds the {} role already.", b.role),
        ));
    }
    let slot = team.slot(&role);
    if teams::liveness(&state, &slot).await.0 {
        return Err(ApiError::conflict(
            "role_taken",
            "The role's session runs: stop it first.",
        ));
    }
    state.teams.bind(&id, &role, &sid, true, now_ms());
    audit(
        &state,
        &ctx,
        "team.role.take",
        json!({ "team": id, "role": role, "session": sid }),
    );
    note_device_action(
        &state,
        &ctx,
        &format!("gave the {role} role of {} to a session", team.label),
    );
    // Its charter (it has none of its own), then its next task.
    let team = team_of(&state, &id)?;
    if let Some(r) = team.role(&role) {
        let intro = format!(
            "[Mother Claude · role] You now hold this role in a team Mother Claude runs.\n\n{}",
            super::launcher::charter(&team, r)
        );
        super::delegation::deliver(&state, &team, &role, &intro).await;
    }
    state.teams.nudge();
    super::delegation::publish_team(&state, &id).await;
    Ok(Json(teams::view(&state, &team).await))
}

/// `DELETE /api/teams/{id}/roles/{role}/take`: a foreign session gives the
/// role back (its tasks return to the board).
pub async fn release_role(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path((id, role)): Path<(String, String)>,
) -> ApiResult<Json<teams::TeamView>> {
    let team = team_of(&state, &id)?;
    role_of(&team, &role)?;
    if !team.slot(&role).foreign {
        return Err(ApiError::conflict(
            "not_foreign",
            "The role's session is one Mother Claude runs: stop it instead.",
        ));
    }
    super::launcher::release_role(&state, &id, &role).await;
    audit(
        &state,
        &ctx,
        "team.role.release",
        json!({ "team": id, "role": role }),
    );
    let team = team_of(&state, &id)?;
    Ok(Json(teams::view(&state, &team).await))
}

// ─────────────────────────────── the board ───────────────────────────────

/// `GET /api/teams/{id}/tasks` → `{tasks: [Task]}` (newest first).
pub async fn list_tasks(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let team = team_of(&state, &id)?;
    let mut tasks = state.teams.boards.read(&team.project, |b| b.tasks.clone());
    tasks.sort_by_key(|t| std::cmp::Reverse(t.id));
    Ok(Json(json!({ "tasks": tasks })))
}

/// `GET /api/teams/{id}/tasks/{task}` → the task.
pub async fn get_task(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path((id, task)): Path<(String, u64)>,
) -> ApiResult<Json<super::board::Task>> {
    let team = team_of(&state, &id)?;
    state
        .teams
        .boards
        .read(&team.project, |b| b.get(task).cloned())
        .map(Json)
        .ok_or_else(|| board_error(BoardError::NotFound(task)))
}

/// `POST /api/teams/{id}/tasks` (a draft) → `201` the task.
pub async fn create_task(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(mut draft): Body<TaskDraft>,
) -> ApiResult<Response> {
    let team = team_of(&state, &id)?;
    if let Some(m) = &draft.model {
        owned::validate_model(&m.model)?;
    }
    super::board::classify_draft(&state, &mut draft).await;
    let actor = Actor::User(who(&state, &ctx));
    let roles = team.role_ids();
    let change = state
        .teams
        .boards
        .mutate(&team.project, |b| b.create(draft, &actor, &roles, now_ms()))
        .map_err(board_error)?;
    let task = change.task.clone();
    audit(
        &state,
        &ctx,
        "team.task.create",
        json!({ "team": id, "task": task.id }),
    );
    note_device_action(
        &state,
        &ctx,
        &format!("added task {} to the team on {}", task.id, team.label),
    );
    super::delegation::on_change(&state, &id, change).await;
    Ok((StatusCode::CREATED, Json(task)).into_response())
}

/// `PATCH /api/teams/{id}/tasks/{task}` (a patch) → the task: reprioritise,
/// reassign, edit, cancel, accept or send back.
pub async fn patch_task(
    Authed(ctx): Authed,
    State(state): State<AppState>,
    Path((id, task)): Path<(String, u64)>,
    Body(body): Body<Value>,
) -> ApiResult<Json<super::board::Task>> {
    let team = team_of(&state, &id)?;
    super::mcp::strict_status(&body).map_err(|m| ApiError::unprocessable("invalid_task", m))?;
    let patch: TaskPatch = serde_json::from_value(body)
        .map_err(|e| ApiError::unprocessable("invalid_task", e.to_string()))?;
    if let Some(m) = &patch.model {
        owned::validate_model(&m.model)?;
    }
    let actor = Actor::User(who(&state, &ctx));
    let roles = team.role_ids();
    let change = state
        .teams
        .boards
        .mutate(&team.project, |b| {
            b.update(task, patch, &actor, &roles, now_ms())
        })
        .map_err(board_error)?;
    let out = change.task.clone();
    audit(
        &state,
        &ctx,
        "team.task.update",
        json!({ "team": id, "task": task, "status": out.status.as_str() }),
    );
    note_device_action(
        &state,
        &ctx,
        &format!("changed task {task} of the team on {}", team.label),
    );
    if !change.kinds.is_empty() {
        super::delegation::on_change(&state, &id, change).await;
    }
    Ok(Json(out))
}
