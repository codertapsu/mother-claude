//! The team tools of Mother Claude's MCP server (docs/TEAMS.md "Tools"):
//! `task_create`, `task_list`, `task_get`, `task_claim`, `task_update`,
//! `task_done`, `team_roles`, `team_status`, `team_request_role` and
//! `session_summary`.
//!
//! Who may call them is decided by the calling session
//! ([`crate::mcp::Caller`]): it must be the lead or one of the role
//! sessions of a team, and its working folder (when known) inside that
//! team's project; anything else gets a `denied` result that says why.
//! Writing tasks, sending work back, `team_request_role` and
//! `session_summary` (read-only) are the lead's.

use serde::Deserialize;
use serde_json::{json, Value};

use super::board::{Actor, BoardError, Outcome, TaskDraft, TaskPatch, TaskReport, TaskStatus};
use super::teams::{Binding, Team};
use super::templates::TASK_CLASSES;
use crate::mcp::{Registry, ToolAnnotations, ToolCtx, ToolDef, ToolError, ToolOutput};

fn now_ms() -> i64 {
    crate::claude::registry::now_ms()
}

/// The calling session's team and place in it.
fn member(ctx: &ToolCtx) -> Result<(Team, Binding), ToolError> {
    let Some(session) = ctx.caller.session_id.as_deref() else {
        return Err(ToolError::new(
            "denied",
            "Mother Claude could not tell which Claude Code session is calling. The team tools \
             work in the sessions of a team Mother Claude runs.",
        ));
    };
    let Some(b) = ctx.state.teams.binding(session) else {
        return Err(ToolError::new(
            "denied",
            "This session has no role in a team. Start a team on the Teams page of Mother \
             Claude, or give this session a role there, to use the board.",
        ));
    };
    let team = ctx
        .state
        .teams
        .get(&b.team)
        .ok_or_else(|| ToolError::new("denied", "This session's team is gone."))?;
    if team.stopped {
        return Err(ToolError::new(
            "denied",
            "This session's team is stopped; resume it on the Teams page first.",
        ));
    }
    if let Some(cwd) = ctx.caller.cwd.as_deref().filter(|c| !c.is_empty()) {
        if !crate::claude::paths::is_within_recorded(cwd, &team.project) {
            return Err(ToolError::new(
                "denied",
                format!(
                    "This session works in {cwd}, outside the team's project {}.",
                    team.project
                ),
            ));
        }
    }
    Ok((team, b))
}

fn actor(b: &Binding, ctx: &ToolCtx) -> Actor {
    Actor::Session {
        session: ctx.caller.session_id.clone().unwrap_or_default(),
        role: b.role.clone(),
        lead: b.lead,
    }
}

fn lead_only(b: &Binding, what: &str) -> Result<(), ToolError> {
    if b.lead {
        Ok(())
    } else {
        Err(ToolError::new(
            "denied",
            format!("{what} is the lead's: send the lead a message (SendMessage) instead."),
        ))
    }
}

fn board_error(e: BoardError) -> ToolError {
    ToolError::new(e.code(), e.to_string())
}

/// A task id given as a number or as digits.
fn task_id(v: &Value) -> Result<u64, ToolError> {
    match v.get("id") {
        Some(Value::Number(n)) => n.as_u64(),
        Some(Value::String(s)) => s.trim().trim_start_matches('#').parse().ok(),
        _ => None,
    }
    .filter(|n| *n > 0)
    .ok_or_else(|| ToolError::invalid("`id` is the task's number (see task_list)."))
}

fn parse<T: for<'de> Deserialize<'de>>(args: Value) -> Result<T, ToolError> {
    serde_json::from_value(args).map_err(|e| ToolError::invalid(e.to_string()))
}

/// A `status` argument must name a status (a stored board reads an unknown
/// one as `todo`; a request must never).
pub fn strict_status(args: &Value) -> Result<(), String> {
    match args.get("status") {
        None | Some(Value::Null) => Ok(()),
        Some(Value::String(s)) if TaskStatus::parse(s).is_some() => Ok(()),
        Some(other) => Err(format!(
            "{other} is not a status (todo, claimed, in_progress, review, done, failed, \
             cancelled)."
        )),
    }
}

fn audit(ctx: &ToolCtx, b: &Binding, action: &str, task: u64) {
    ctx.state
        .devices
        .audit
        .record(crate::server::devices::audit::AuditEntry {
            action: format!("team.task.{action}"),
            principal: Some("session".into()),
            detail: Some(json!({
                "team": b.team,
                "role": b.role,
                "session": ctx.caller.session_id,
                "task": task,
            })),
            ..crate::server::devices::audit::AuditEntry::now()
        });
}

// ─────────────────────────────── handlers ───────────────────────────────

async fn task_create(ctx: ToolCtx, args: Value) -> Result<ToolOutput, ToolError> {
    let (team, b) = member(&ctx)?;
    lead_only(&b, "Writing tasks")?;
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        title: String,
        #[serde(default)]
        goal: String,
        #[serde(default)]
        acceptance: Vec<String>,
        role: String,
        #[serde(default)]
        class: Option<String>,
        #[serde(default)]
        priority: Option<u8>,
        #[serde(default)]
        files: Vec<String>,
        #[serde(default)]
        constraints: Vec<String>,
    }
    let a: Args = parse(args)?;
    let mut draft = TaskDraft {
        title: a.title,
        goal: a.goal,
        acceptance: a.acceptance,
        role: Some(a.role),
        class: a.class,
        priority: a.priority,
        files: a.files,
        constraints: a.constraints,
        model: None,
    };
    super::board::classify_draft(&ctx.state, &mut draft).await;
    let who = actor(&b, &ctx);
    let roles = team.role_ids();
    let change = ctx
        .state
        .teams
        .boards
        .mutate(&team.project, |board| {
            board.create(draft, &who, &roles, now_ms())
        })
        .map_err(board_error)?;
    audit(&ctx, &b, "create", change.task.id);
    let task = change.task.clone();
    super::delegation::on_change(&ctx.state, &team.id, change).await;
    Ok(ToolOutput::json(json!({ "task": task })))
}

async fn task_list(ctx: ToolCtx, args: Value) -> Result<ToolOutput, ToolError> {
    let (team, b) = member(&ctx)?;
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Args {
        status: Vec<String>,
        role: Option<String>,
        mine: bool,
    }
    let a: Args = parse(if args.is_null() { json!({}) } else { args })?;
    let mut statuses = Vec::new();
    for s in &a.status {
        statuses.push(TaskStatus::parse(s).ok_or_else(|| {
            ToolError::invalid(format!(
                "{s:?} is not a status (todo, claimed, in_progress, review, done, failed, \
                 cancelled)."
            ))
        })?);
    }
    let me = ctx.caller.session_id.clone().unwrap_or_default();
    let mut rows: Vec<_> = ctx.state.teams.boards.read(&team.project, |board| {
        board
            .tasks
            .iter()
            .filter(|t| statuses.is_empty() || statuses.contains(&t.status))
            .filter(|t| a.role.as_ref().is_none_or(|r| t.role.as_ref() == Some(r)))
            .filter(|t| !a.mine || t.claimed_by.as_deref() == Some(me.as_str()))
            .map(|t| t.row())
            .collect()
    });
    rows.sort_by_key(|r| (r.status.ended(), r.priority, r.id));
    let total = rows.len();
    rows.truncate(200);
    Ok(ToolOutput::json(json!({
        "you": { "role": b.role, "lead": b.lead },
        "tasks": rows,
        "total": total,
    })))
}

async fn task_get(ctx: ToolCtx, args: Value) -> Result<ToolOutput, ToolError> {
    let (team, _) = member(&ctx)?;
    let id = task_id(&args)?;
    let task = ctx
        .state
        .teams
        .boards
        .read(&team.project, |board| board.get(id).cloned())
        .ok_or_else(|| board_error(BoardError::NotFound(id)))?;
    Ok(ToolOutput::json(json!({ "task": task })))
}

async fn task_claim(ctx: ToolCtx, args: Value) -> Result<ToolOutput, ToolError> {
    let (team, b) = member(&ctx)?;
    let id = task_id(&args)?;
    let who = actor(&b, &ctx);
    let change = ctx
        .state
        .teams
        .boards
        .mutate(&team.project, |board| board.claim(id, &who, now_ms()))
        .map_err(board_error)?;
    audit(&ctx, &b, "claim", id);
    let task = change.task.clone();
    if !change.kinds.is_empty() {
        super::delegation::on_change(&ctx.state, &team.id, change).await;
    }
    Ok(ToolOutput::json(json!({ "task": task })))
}

async fn task_update(ctx: ToolCtx, args: Value) -> Result<ToolOutput, ToolError> {
    let (team, b) = member(&ctx)?;
    let id = task_id(&args)?;
    strict_status(&args).map_err(ToolError::invalid)?;
    let mut patch: TaskPatch = {
        let mut a = args.clone();
        if let Some(o) = a.as_object_mut() {
            o.remove("id");
        }
        parse(a)?
    };
    // A session never picks a model: that is the user's (or the router's).
    patch.model = None;
    let who = actor(&b, &ctx);
    let roles = team.role_ids();
    let change = ctx
        .state
        .teams
        .boards
        .mutate(&team.project, |board| {
            board.update(id, patch, &who, &roles, now_ms())
        })
        .map_err(board_error)?;
    audit(&ctx, &b, "update", id);
    let task = change.task.clone();
    if !change.kinds.is_empty() {
        super::delegation::on_change(&ctx.state, &team.id, change).await;
    }
    Ok(ToolOutput::json(json!({ "task": task })))
}

async fn task_done(ctx: ToolCtx, args: Value) -> Result<ToolOutput, ToolError> {
    let (team, b) = member(&ctx)?;
    let id = task_id(&args)?;
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        summary: String,
        #[serde(default)]
        outcome: Outcome,
        #[serde(default)]
        branch: Option<String>,
        #[serde(default)]
        changed_files: Vec<String>,
        #[serde(default)]
        gate_passed: Option<bool>,
    }
    let a: Args = parse({
        let mut a = args.clone();
        if let Some(o) = a.as_object_mut() {
            o.remove("id");
        }
        a
    })?;
    let who = actor(&b, &ctx);
    let review = team.template.reviews();
    let report = TaskReport {
        summary: a.summary,
        branch: a.branch,
        changed_files: a.changed_files,
        gate_passed: a.gate_passed,
    };
    let change = ctx
        .state
        .teams
        .boards
        .mutate(&team.project, |board| {
            board.finish(id, report, a.outcome, review, &who, now_ms())
        })
        .map_err(board_error)?;
    audit(&ctx, &b, "done", id);
    let task = change.task.clone();
    super::delegation::on_change(&ctx.state, &team.id, change).await;
    let next = if task.status == TaskStatus::Review {
        "It waits for the lead's review. Wait for Mother Claude's next message; do not start \
         other work on your own."
    } else {
        "Wait for Mother Claude's next message."
    };
    Ok(ToolOutput::json(json!({ "task": task, "next": next })))
}

async fn team_roles(ctx: ToolCtx, _args: Value) -> Result<ToolOutput, ToolError> {
    let (team, b) = member(&ctx)?;
    let view = super::teams::view(&ctx.state, &team).await;
    let roles: Vec<Value> = view
        .roles
        .iter()
        .map(|r| {
            json!({
                "role": r.id,
                "title": r.title,
                "lead": r.lead,
                "sessionName": r.session_name,
                "state": r.state,
                "description": r.description,
                "paths": r.paths,
                "readOnly": r.read_only,
                "worktree": r.worktree,
                "classes": r.classes,
            })
        })
        .collect();
    Ok(ToolOutput::json(json!({
        "you": { "role": b.role, "lead": b.lead, "sessionName": team.session_name(&b.role) },
        "project": team.project,
        "roles": roles,
        "integration": team.template.integration,
    })))
}

async fn team_status(ctx: ToolCtx, _args: Value) -> Result<ToolOutput, ToolError> {
    let (team, _) = member(&ctx)?;
    let view = super::teams::view(&ctx.state, &team).await;
    let roles: Vec<Value> = view
        .roles
        .iter()
        .map(|r| {
            json!({
                "role": r.id,
                "state": r.state,
                "sessionName": r.session_name,
                "model": r.model,
                "effort": r.effort,
                "currentTask": r.current_task,
                "queuedTasks": r.queued_tasks,
                "waitReasons": r.wait_reasons,
            })
        })
        .collect();
    Ok(ToolOutput::json(json!({
        "board": view.counts,
        "roles": roles,
    })))
}

async fn team_request_role(ctx: ToolCtx, args: Value) -> Result<ToolOutput, ToolError> {
    let (team, b) = member(&ctx)?;
    lead_only(&b, "Starting a role's session")?;
    #[derive(Deserialize)]
    struct Args {
        role: String,
    }
    let a: Args = parse(args)?;
    let Some(role) = team.role(&a.role).cloned() else {
        return Err(ToolError::invalid(format!(
            "No role {:?} in this team (roles: {}).",
            a.role,
            team.role_ids().join(", ")
        )));
    };
    let slot = team.slot(&role.id);
    if super::teams::liveness(&ctx.state, &slot).await.0 {
        return Ok(ToolOutput::json(json!({
            "role": role.id,
            "state": "running",
            "sessionName": team.session_name(&role.id),
        })));
    }
    ctx.state.teams.update(&team.id, |t| {
        t.roles.entry(role.id.clone()).or_default().wanted = true;
    });
    let running = super::launcher::running_workers(&ctx.state).await;
    let next = ctx.state.teams.boards.read(&team.project, |board| {
        board.next_for(&role.id).map(|t| t.id)
    });
    let out = match super::launcher::start_role(&ctx.state, &team.id, &role.id, next, &running)
        .await
    {
        Ok(_) => {
            json!({ "role": role.id, "state": "started", "sessionName": team.session_name(&role.id) })
        }
        Err(super::launcher::StartError::Waiting(reasons)) => json!({
            "role": role.id,
            "state": "waiting",
            "reasons": reasons,
            "note": "Mother Claude starts it as soon as capacity allows.",
        }),
        Err(e) => return Err(ToolError::new("unavailable", e.to_string())),
    };
    Ok(ToolOutput::json(out))
}

async fn session_summary(ctx: ToolCtx, args: Value) -> Result<ToolOutput, ToolError> {
    let (team, b) = member(&ctx)?;
    lead_only(&b, "Reading another session")?;
    #[derive(Deserialize)]
    struct Args {
        role: String,
    }
    let a: Args = parse(args)?;
    if team.role(&a.role).is_none() {
        return Err(ToolError::invalid(format!(
            "No role {:?} in this team (roles: {}).",
            a.role,
            team.role_ids().join(", ")
        )));
    }
    let slot = team.slot(&a.role);
    let Some(sid) = slot.session_id.clone() else {
        return Ok(ToolOutput::json(json!({
            "role": a.role,
            "state": "notStarted",
        })));
    };
    let summary = ctx.state.find_session(&sid).await;
    let (alive, busy) = super::teams::liveness(&ctx.state, &slot).await;
    let held = ctx
        .state
        .teams
        .boards
        .read(&team.project, |board| board.held_by(&sid).map(|t| t.row()));
    let out = match summary {
        Some(s) => json!({
            "role": a.role,
            "sessionName": team.session_name(&a.role),
            "running": alive,
            "inTurn": busy,
            "status": s.status,
            "activity": s.activity.label,
            "model": s.model.map(|m| m.id),
            "lastPrompt": s.last_prompt,
            "lastMessage": s.last_message,
            "messages": s.message_count,
            "costUsd": s.cost_usd,
            "todos": s.todos,
            "currentTask": held,
        }),
        None => json!({
            "role": a.role,
            "sessionName": team.session_name(&a.role),
            "running": alive,
            "inTurn": busy,
            "currentTask": held,
        }),
    };
    Ok(ToolOutput::json(out))
}

// ─────────────────────────────── definitions ───────────────────────────────

fn read_only() -> Option<ToolAnnotations> {
    Some(ToolAnnotations {
        read_only_hint: Some(true),
        idempotent_hint: Some(true),
        open_world_hint: Some(false),
        ..Default::default()
    })
}

fn writes() -> Option<ToolAnnotations> {
    Some(ToolAnnotations {
        read_only_hint: Some(false),
        destructive_hint: Some(false),
        open_world_hint: Some(false),
        ..Default::default()
    })
}

fn id_property() -> Value {
    json!({ "type": ["integer", "string"], "description": "The task's number (task_list shows it)." })
}

/// The team tools, in the order `tools/list` shows them.
pub fn definitions() -> Vec<ToolDef> {
    let classes = json!(TASK_CLASSES);
    vec![
        ToolDef {
            name: "task_create".into(),
            title: Some("Write a task packet".into()),
            description: "Lead only. Write a task packet on the team's board for ONE role. \
                Mother Claude starts (or wakes) that role's session as capacity allows and \
                hands it the packet, already claimed. The worker sees only the packet and the \
                project, so make it self-contained: a one-line title, the goal (what and why, \
                and what done looks like), acceptance criteria it can check, the files it \
                should touch (inside its role's paths, see team_roles), constraints, a priority \
                (0 highest … 3, default 2) and a class. One task per role and change; split \
                work that spans roles. Returns the task with its number."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "One line: what to do." },
                    "goal": { "type": "string", "description": "What and why, and what done looks like." },
                    "acceptance": { "type": "array", "items": { "type": "string" }, "description": "Checks the result must pass." },
                    "role": { "type": "string", "description": "The role that does it (a role id from team_roles)." },
                    "class": { "type": "string", "enum": classes, "description": "What kind of work it is (picks the model)." },
                    "priority": { "type": "integer", "minimum": 0, "maximum": 3 },
                    "files": { "type": "array", "items": { "type": "string" }, "description": "Files to read or change, relative to the project." },
                    "constraints": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["title", "goal", "role"],
                "additionalProperties": false
            }),
            annotations: writes(),
        },
        ToolDef {
            name: "task_list".into(),
            title: Some("List the board's tasks".into()),
            description: "The tasks on the team's board, open ones first (by priority), with \
                their status (todo, claimed, in_progress, review, done, failed, cancelled), \
                role, priority and who holds them. Filter by status, role, or mine: true for \
                the tasks this session holds. Use task_get for a task's whole packet and \
                history."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "status": { "type": "array", "items": { "type": "string", "enum": ["todo", "claimed", "in_progress", "review", "done", "failed", "cancelled"] } },
                    "role": { "type": "string" },
                    "mine": { "type": "boolean" }
                },
                "additionalProperties": false
            }),
            annotations: read_only(),
        },
        ToolDef {
            name: "task_get".into(),
            title: Some("Read a task".into()),
            description: "One task's whole packet (goal, acceptance criteria, files, \
                constraints), its progress notes, result and history."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": { "id": id_property() },
                "required": ["id"],
                "additionalProperties": false
            }),
            annotations: read_only(),
        },
        ToolDef {
            name: "task_claim".into(),
            title: Some("Claim a task".into()),
            description: "Take a todo task of your own role. Rarely needed: a task Mother \
                Claude hands you is already claimed for you. Claiming the task you hold \
                changes nothing."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": { "id": id_property() },
                "required": ["id"],
                "additionalProperties": false
            }),
            annotations: writes(),
        },
        ToolDef {
            name: "task_update".into(),
            title: Some("Update a task".into()),
            description: "Report on the task you hold: a progress note (note), status \
                in_progress, blocked with your question (blocked: \"…\"; an empty string \
                clears it — also SendMessage the lead), or give it back (status todo). The \
                lead also reprioritises (priority), reassigns (role), edits the packet (title, \
                goal, acceptance, files, constraints, class), accepts a task in review (status \
                done), sends one back (status todo with a note; escalate: true for a stronger \
                model or more effort) and cancels (status cancelled). Finish your own task \
                with task_done, not here."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": id_property(),
                    "status": { "type": "string", "enum": ["todo", "in_progress", "done", "failed", "cancelled"] },
                    "note": { "type": "string" },
                    "blocked": { "type": "string" },
                    "priority": { "type": "integer", "minimum": 0, "maximum": 3 },
                    "role": { "type": "string" },
                    "title": { "type": "string" },
                    "goal": { "type": "string" },
                    "acceptance": { "type": "array", "items": { "type": "string" } },
                    "files": { "type": "array", "items": { "type": "string" } },
                    "constraints": { "type": "array", "items": { "type": "string" } },
                    "class": { "type": "string", "enum": classes },
                    "escalate": { "type": "boolean" }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
            annotations: writes(),
        },
        ToolDef {
            name: "task_done".into(),
            title: Some("Finish a task".into()),
            description: "Finish the task you hold. Say what you did in summary (the lead \
                reads it before your diff), the branch it is on (a worktree role's \
                worktree-<role>) and the files you changed; gatePassed when you ran the \
                integration gate. outcome \"failed\" when you could not do it — say why. A \
                worker's finished task goes to the lead's review when the team reviews. Then \
                wait for Mother Claude's next message."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": id_property(),
                    "summary": { "type": "string" },
                    "outcome": { "type": "string", "enum": ["done", "failed"] },
                    "branch": { "type": "string" },
                    "changedFiles": { "type": "array", "items": { "type": "string" } },
                    "gatePassed": { "type": "boolean" }
                },
                "required": ["id", "summary"],
                "additionalProperties": false
            }),
            annotations: writes(),
        },
        ToolDef {
            name: "team_roles".into(),
            title: Some("Who is on the team".into()),
            description: "The team's roles: each one's title, session name (the address for \
                SendMessage), state (notStarted, waiting, starting, running, idle, stopped), \
                what it is for, the files it may change, whether it is read-only or works in \
                a worktree, and which role you are."
                .into(),
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            annotations: read_only(),
        },
        ToolDef {
            name: "team_status".into(),
            title: Some("The team now".into()),
            description: "The board's counts per status, and per role: its state, model, the \
                task it holds, how many wait for it, and why it waits when capacity holds it \
                back."
                .into(),
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            annotations: read_only(),
        },
        ToolDef {
            name: "team_request_role".into(),
            title: Some("Start a role's session".into()),
            description: "Lead only. Ask Mother Claude to start a role's session now, for \
                example to talk to it with SendMessage before you write its task. It starts \
                when capacity allows; the answer says whether it started or waits, and why. A \
                role with a task on the board starts by itself."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": { "role": { "type": "string" } },
                "required": ["role"],
                "additionalProperties": false
            }),
            annotations: writes(),
        },
        ToolDef {
            name: "session_summary".into(),
            title: Some("What a role's session is doing".into()),
            description: "Lead only, read-only. What a role's session is doing now: running \
                or not, its activity, the task it holds, its last prompt and last message, \
                and its cost so far. It does not disturb the session; to talk to it use \
                SendMessage."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": { "role": { "type": "string" } },
                "required": ["role"],
                "additionalProperties": false
            }),
            annotations: read_only(),
        },
    ]
}

/// How a session's model names a tool of Mother Claude's MCP server: the
/// plugin `mother-claude`, its server `mc` (docs/MCP.md).
pub use crate::mcp::plugin::TOOL_PREFIX;

/// The team tools by their full names, for a role session's
/// `--allowedTools`: Mother Claude decides who may do what on every call
/// ([`member`]), so a call needs no permission card. Exact names only,
/// never the server's name, which would allow every other tool of the
/// server too (the browser's among them).
pub fn allowed_tools() -> Vec<String> {
    definitions()
        .iter()
        .map(|d| format!("{TOOL_PREFIX}{}", d.name))
        .collect()
}

/// Register the team tools (`mcp::registry`, the "teams" line).
pub fn register(r: &mut Registry) {
    for def in definitions() {
        match def.name.as_str() {
            "task_create" => r.add(def, task_create),
            "task_list" => r.add(def, task_list),
            "task_get" => r.add(def, task_get),
            "task_claim" => r.add(def, task_claim),
            "task_update" => r.add(def, task_update),
            "task_done" => r.add(def, task_done),
            "team_roles" => r.add(def, team_roles),
            "team_status" => r.add(def, team_status),
            "team_request_role" => r.add(def, team_request_role),
            "session_summary" => r.add(def, session_summary),
            other => tracing::error!(tool = %other, "a team tool without a handler"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_is_defined_once_with_an_object_schema() {
        let defs = definitions();
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "task_create",
                "task_list",
                "task_get",
                "task_claim",
                "task_update",
                "task_done",
                "team_roles",
                "team_status",
                "team_request_role",
                "session_summary"
            ]
        );
        for d in &defs {
            assert_eq!(d.input_schema["type"], "object", "{}", d.name);
            assert!(d.name.len() <= 64);
            assert!(d.description.len() > 60, "{}: say how to use it", d.name);
        }
        let mut r = Registry::default();
        register(&mut r);
        assert_eq!(r.len(), 10);
    }

    #[test]
    fn a_role_session_may_call_exactly_the_team_tools() {
        let allowed = allowed_tools();
        assert_eq!(allowed.len(), definitions().len());
        assert!(allowed.contains(&"mcp__plugin_mother-claude_mc__task_list".to_string()));
        assert!(allowed.contains(&"mcp__plugin_mother-claude_mc__task_done".to_string()));
        for name in &allowed {
            let tool = name.strip_prefix(TOOL_PREFIX).unwrap_or_default();
            assert!(!tool.is_empty(), "{name}: never the whole server");
            assert!(
                tool.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{name}: an exact name, no pattern"
            );
            assert!(!tool.starts_with("browser_") && !tool.starts_with("mc_"));
        }
    }

    #[test]
    fn task_ids_read_as_numbers_or_digits() {
        assert_eq!(task_id(&json!({ "id": 12 })).unwrap(), 12);
        assert_eq!(task_id(&json!({ "id": "#7" })).unwrap(), 7);
        assert!(task_id(&json!({ "id": 0 })).is_err());
        assert!(task_id(&json!({ "id": "x" })).is_err());
        assert!(task_id(&json!({})).is_err());
    }
}
