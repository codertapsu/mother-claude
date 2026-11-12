//! The `machine_*` tools of Mother Claude's MCP server (docs/MCP.md
//! "Machines", docs/PLAN-MACHINES.md Phase 4): a session runs commands on
//! another of the user's computers at its own code.
//!
//! * `machine_list` — the computers this session may use: name, OS, shell,
//!   online, whether this computer may run there (their `run` grant),
//!   versions.
//! * `machine_run {machine, command, cwd?, timeout?, artifacts?}` — the run;
//!   an approval here unless a standing grant covers it; waits up to 10
//!   minutes.
//! * `machine_wait {id, timeout?}`, `machine_output {id, from?, max?}` /
//!   `{id, artifact}`, `machine_cancel {id}`.
//!
//! A failure is an `isError` result that says why, never a protocol error;
//! nothing here panics.

use std::time::Duration;

use serde_json::{json, Value};

use super::source;
use crate::machines::grants::Grant;
use crate::mcp::{Registry, ToolAnnotations, ToolCtx, ToolDef, ToolError, ToolOutput};

/// The longest `machine_run` may take here: an approval (10 minutes), the
/// code's transfer, the wait (10 minutes).
const RUN_LIMIT: Duration = Duration::from_secs(60 * 60);
const WAIT_LIMIT: Duration = Duration::from_secs(11 * 60);
const OUTPUT_LIMIT: Duration = Duration::from_secs(20 * 60);

fn def(name: &str, title: &str, description: &str, schema: Value, a: ToolAnnotations) -> ToolDef {
    ToolDef {
        name: name.into(),
        title: Some(title.into()),
        description: description.into(),
        input_schema: schema,
        annotations: Some(a),
    }
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn id_of(args: &Value) -> Result<String, ToolError> {
    text(args, "id")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ToolError::invalid("Name the run: {\"id\": \"run_…\"} (machine_run said it).")
        })
}

fn number(args: &Value, key: &str) -> Result<Option<i64>, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_i64()
            .or_else(|| v.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64))
            .map(Some)
            .ok_or_else(|| ToolError::invalid(format!("`{key}` is a whole number."))),
    }
}

pub fn register(r: &mut Registry) {
    r.add(
        def(
            "machine_list",
            "Your other computers",
            "List the user's other computers paired in Mother Claude (Machines) that this \
             session could run commands on: each one's name, OS, shell (write commands for it: \
             zsh on a Mac, PowerShell on Windows), whether it is online, whether this computer \
             may run commands there now (its Run permission for this one), and its versions.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            ToolAnnotations {
                read_only_hint: Some(true),
                idempotent_hint: Some(true),
                open_world_hint: Some(false),
                ..Default::default()
            },
        ),
        |ctx: ToolCtx, _args: Value| async move { Ok(ToolOutput::json(list(&ctx))) },
    );
    r.add_with_limit(
        def(
            "machine_run",
            "Run a command on another computer",
            "Run a command on another of the user's computers, at this session's code — e.g. \
             build and test the Windows version from a Mac. Mother Claude snapshots this \
             project's working tree (uncommitted and untracked files included, ignored ones \
             not), sends it, and runs the command there in a kept work folder: a git worktree \
             of that computer's checkout, so build caches the project ignores (target/, \
             node_modules/) survive between runs; that computer's own checkout is never \
             touched. The user approves each run (or a standing grant covers it). Write the \
             command for that computer's shell (machine_list). Waits up to 10 minutes and \
             returns the exit code, the duration and the output's last 64 KiB — or 'still \
             running' with the run's id: then machine_wait, machine_output, machine_cancel.",
            json!({
                "type": "object",
                "properties": {
                    "machine": {
                        "type": "string",
                        "description": "The computer: its name or id, as machine_list gives them."
                    },
                    "command": {
                        "type": "string",
                        "description": "The command line, written for that computer's shell."
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Where to run it, relative to the project's top (default: the folder this session works in)."
                    },
                    "timeout": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": super::MAX_TIMEOUT_SECS,
                        "description": "Seconds before the run is stopped, with everything it started (default 1800, at most 7200)."
                    },
                    "artifacts": {
                        "type": "string",
                        "description": "A glob of files the run makes, relative to the project's top (e.g. target/release/*.exe): listed with their sizes when it ends; bring one here with machine_output's `artifact`."
                    }
                },
                "required": ["machine", "command"],
                "additionalProperties": false
            }),
            ToolAnnotations {
                read_only_hint: Some(false),
                destructive_hint: Some(true),
                open_world_hint: Some(true),
                ..Default::default()
            },
        ),
        Some(RUN_LIMIT),
        |ctx: ToolCtx, args: Value| async move {
            let ask = source::Ask {
                machine: text(&args, "machine").unwrap_or_default(),
                command: text(&args, "command").unwrap_or_default(),
                cwd: text(&args, "cwd"),
                timeout_secs: match number(&args, "timeout")? {
                    Some(n) if n > 0 => Some(n as u64),
                    Some(_) => return Err(ToolError::invalid("`timeout` is at least 1 second.")),
                    None => None,
                },
                artifacts: text(&args, "artifacts"),
            };
            if ask.machine.trim().is_empty() {
                return Err(ToolError::invalid(
                    "Name the computer: {\"machine\": \"DESKTOP-PC\"} (machine_list names them).",
                ));
            }
            source::run(&ctx.state, &ctx.caller, ask)
                .await
                .map(ToolOutput::text)
        },
    );
    r.add_with_limit(
        def(
            "machine_wait",
            "Wait for a run",
            "Wait for a run machine_run started to end (at most `timeout` seconds, 600 by \
             default and at most), then return what machine_run would have: the exit code, the \
             duration and the output's last 64 KiB — or 'still running'.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The run's id (run_…)." },
                    "timeout": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": super::WAIT_SECS,
                        "description": "Seconds to wait at most (default 600)."
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
            ToolAnnotations {
                read_only_hint: Some(true),
                open_world_hint: Some(false),
                ..Default::default()
            },
        ),
        Some(WAIT_LIMIT),
        |ctx: ToolCtx, args: Value| async move {
            let id = id_of(&args)?;
            let secs = number(&args, "timeout")?.map(|n| n.max(1) as u64);
            source::wait(&ctx.state, &ctx.caller, &id, secs)
                .await
                .map(ToolOutput::text)
        },
    );
    r.add_with_limit(
        def(
            "machine_output",
            "A run's output, or a file it made",
            "Read a run's whole output, which the computer that ran it keeps for a day: a page \
             of at most `max` bytes (64 KiB by default) from byte `from` (0: the start; \
             negative: from the end). Or bring one of the files the run listed (its \
             `artifacts`, 64 MiB at most) into this project, under \
             .mother-claude/artifacts/<run>/ (git ignores it): give `artifact` its path.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The run's id (run_…)." },
                    "from": { "type": "integer", "description": "The byte to start at (0: the start; negative: that many bytes before the end)." },
                    "max": { "type": "integer", "minimum": 1024, "maximum": 262144, "description": "Bytes at most (default 65536)." },
                    "artifact": { "type": "string", "description": "A file the run listed (its path), to bring into this project instead of reading output." }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
            ToolAnnotations {
                read_only_hint: Some(false),
                idempotent_hint: Some(true),
                open_world_hint: Some(false),
                ..Default::default()
            },
        ),
        Some(OUTPUT_LIMIT),
        |ctx: ToolCtx, args: Value| async move {
            let id = id_of(&args)?;
            if let Some(path) = text(&args, "artifact").filter(|p| !p.trim().is_empty()) {
                return source::fetch(&ctx.state, &ctx.caller, &id, &path)
                    .await
                    .map(ToolOutput::text);
            }
            let from = number(&args, "from")?;
            let max = number(&args, "max")?.map(|n| n.max(0) as u64);
            source::output(&ctx.state, &ctx.caller, &id, from, max)
                .await
                .map(ToolOutput::text)
        },
    );
    r.add(
        def(
            "machine_cancel",
            "Stop a run",
            "Stop a run machine_run started: the command and everything it started end on \
             that computer. Returns how it stands.",
            json!({
                "type": "object",
                "properties": { "id": { "type": "string", "description": "The run's id (run_…)." } },
                "required": ["id"],
                "additionalProperties": false
            }),
            ToolAnnotations {
                read_only_hint: Some(false),
                destructive_hint: Some(true),
                idempotent_hint: Some(true),
                open_world_hint: Some(false),
            },
        ),
        |ctx: ToolCtx, args: Value| async move {
            let id = id_of(&args)?;
            source::cancel(&ctx.state, &ctx.caller, &id)
                .await
                .map(ToolOutput::text)
        },
    );
}

/// `machine_list`'s answer.
pub fn list(ctx: &ToolCtx) -> Value {
    let state = &ctx.state;
    let peers = state.machines.store_read(|f| f.peers.clone());
    let machines: Vec<Value> = peers
        .iter()
        .map(|p| {
            let link = state.machines.link(&p.id);
            let shell = super::shell::ShellKind::parse(p.shell.as_deref().unwrap_or_default());
            let why = source::why_not(state, p);
            json!({
                "id": p.id,
                "name": p.display_name(),
                "os": p.os,
                "osVersion": p.os_version,
                "shell": shell.map(|s| s.as_str()),
                "shellLabel": shell.map(|s| s.label()).or(match p.os.as_str() {
                    "windows" => Some("PowerShell"),
                    "macos" => Some("zsh"),
                    _ => None,
                }),
                "online": link.online,
                "path": link.path,
                "theirGrants": p.their_grants.map(|g| json!({ "run": g.get(Grant::Run) })),
                "mayRun": why.is_none(),
                "why": why,
                "versions": p.versions,
            })
        })
        .collect();
    let here = super::shell::Shell::here();
    json!({
        "machinesOn": state.machines.enabled(),
        "thisComputer": {
            "name": crate::machines::this_name(state),
            "os": crate::machines::protocol::this_os(),
            "shell": here.kind.as_str(),
        },
        "machines": machines,
    })
}
