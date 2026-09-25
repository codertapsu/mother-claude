//! Build the v2 [`SessionSummary`] list (design §4.1, amended §10 B2) from
//! the live roster, the transcript index, the agent tracker, the activity
//! hints, the pending store and the control capabilities.
//!
//! * The session set is the union of live pidfiles, indexed transcripts,
//!   owned children / bridge threads and background jobs.
//! * `pending` carries [`PendingPreview`]s: answerable items come from the
//!   pending store (the one source of truth); a waiting foreign session's
//!   on-disk prompt is added as `source: observed`, `answerable: false` (full
//!   items are kept in `state.activity` for `GET /inbox` / the detail).
//! * `control` comes from `claude::caps::compute(&caps::input_for(…))`.
//! * Cost/usage cover the main transcript plus every subagent / workflow
//!   agent file: `costUsd` = last `cost-state.totalCostUSD` + priced usage
//!   written after it (main by position, subagents by timestamp), else the
//!   priced sum; `<synthetic>` records never count.
//! * [`effective_permission_mode`] is the helper B1/B3/B4 call for bypass
//!   gating.
//!
//! Everything here is synchronous and in-memory (no disk reads besides the
//! cheap caches of the index/agents), so the monitor can rebuild every
//! summary on each refresh; `ws::publish_sessions` diffs by content hash.

use std::collections::{HashMap, HashSet};

use super::activity::{self, PendingLike};
use super::caps::{self, CapsInput, SessionFacts};
use super::index::{Aggregates, FileIndex, PendingTool, Totals};
use super::live::LiveSession;
use super::model::{
    Activity, ActivityKind, AgentCounts, BackgroundTask, BackgroundTaskKind, ContextFill,
    FileChanged, LastError, ModelInfo, PendingItem, PendingKind, PendingPreview, PendingSource,
    PrLink, SessionDetail, SessionStatus, SessionSummary, Surface, TodoStatus, TodoSummary, Usage,
};
use super::pricing;
use crate::state::AppState;

/// A pidfile tmux target (`session:@win.%pane`), as the CLI's own surface
/// classifier recognises it.
pub fn is_tmux_target(t: &str) -> bool {
    let Some((sess, rest)) = t.split_once(':') else {
        return false;
    };
    let Some((win, pane)) = rest.split_once('.') else {
        return false;
    };
    let digits = |s: &str| (1..=6).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_digit());
    (1..=64).contains(&sess.len())
        && sess
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-'))
        && digits(win.strip_prefix('@').unwrap_or(win))
        && digits(pane.strip_prefix('%').unwrap_or(pane))
}

/// Where a session runs.
pub fn surface_for(
    owned: bool,
    pid: Option<&LiveSession>,
    entrypoint: Option<&str>,
    job: bool,
) -> Surface {
    if owned {
        return Surface::MotherClaude;
    }
    if let Some(p) = pid {
        if p.tmux.as_deref().is_some_and(is_tmux_target) {
            return Surface::Tmux;
        }
        if p.is_background() {
            return Surface::Background;
        }
    }
    if job {
        return Surface::Background;
    }
    match entrypoint.map(str::to_ascii_lowercase).as_deref() {
        Some("cli") => Surface::Terminal,
        Some(e) if e.contains("vscode") => Surface::Vscode,
        Some(e) if e.contains("jetbrains") || e.contains("intellij") => Surface::Jetbrains,
        Some(e) if e.contains("desktop") => Surface::Desktop,
        Some(e) if e.starts_with("sdk") => Surface::Sdk,
        Some(e) if e.contains("terminal") => Surface::Terminal,
        _ => Surface::Unknown,
    }
}

/// The activity implied by a status and pending previews (used by
/// `ws::sync_pending` and `Inner::unmark_owned` for immediate updates; the
/// refresh derives the full activity with [`activity::derive`]).
pub fn activity_for<P: PendingLike>(status: SessionStatus, pending: &[P]) -> Activity {
    match status {
        SessionStatus::Working => Activity::new(ActivityKind::Thinking, "Working"),
        SessionStatus::Waiting => match pending.iter().find(|p| p.answerable()) {
            Some(p) => activity::waiting_activity(p),
            None => Activity::new(ActivityKind::Waiting, "Waiting for you"),
        },
        SessionStatus::Idle => Activity::new(ActivityKind::Idle, "Idle"),
        SessionStatus::Starting => Activity::new(ActivityKind::Starting, "Starting"),
        SessionStatus::Ended | SessionStatus::Stopped => {
            Activity::new(ActivityKind::Ended, "Ended")
        }
        SessionStatus::Failed => Activity::new(ActivityKind::Ended, "Failed"),
    }
}

/// The session's effective permission mode: the latest of the hook payload /
/// owned stream note, and the transcript (`permission-mode` record or the
/// latest human prompt's `permissionMode`); when none is known, the live
/// process's argv (`--permission-mode X`, `--dangerously-skip-permissions`).
pub fn effective_permission_mode(state: &AppState, id: &str) -> Option<String> {
    let note = state.activity.permission_mode_note(id);
    let transcript = state
        .index
        .with_main(id, |f| f.agg.permission_mode.clone())
        .flatten();
    match (note, transcript) {
        (Some(a), Some(b)) => Some(if a.1 >= b.1 { a.0 } else { b.0 }),
        (Some(a), None) => Some(a.0),
        (None, Some(b)) => Some(b.0),
        (None, None) => state.live.argv_permission_mode(id),
    }
}

/// Whether `cwd` is a directory the user already trusted: accepted in the
/// CLI's trust dialog (`~/.claude.json`), or holding transcripts. B3 uses it
/// for the `untrusted_cwd` rule.
pub fn is_trusted_cwd(state: &AppState, cwd: &str) -> bool {
    let cwd = cwd.trim_end_matches('/');
    if cwd.is_empty() {
        return false;
    }
    if super::settings::trusted_projects(&state.home)
        .iter()
        .any(|p| p.trim_end_matches('/') == cwd)
    {
        return true;
    }
    state.index.session_ids().iter().any(|id| {
        state
            .index
            .with_main(id, |f| {
                f.agg.cwd.as_deref().map(|c| c.trim_end_matches('/') == cwd)
            })
            .flatten()
            .unwrap_or(false)
    })
}

/// Usage and cost of a session (main + subagent files).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CostView {
    pub usage: Usage,
    pub cost: Option<f64>,
    pub estimated: bool,
    pub final_: bool,
    pub partial: bool,
}

/// Compute a session's usage and cost from the index.
pub fn cost_view(state: &AppState, id: &str, live: bool) -> CostView {
    struct MainView {
        total: Totals,
        after: Totals,
        cost_state: Option<(f64, Option<i64>)>,
        complete: bool,
    }
    let main = state.index.with_main(id, |f: &FileIndex| MainView {
        total: f.agg.usage.total(),
        after: f.agg.usage.after_cost_state(),
        cost_state: f.agg.cost_state.as_ref().map(|c| (c.total, c.ts_before)),
        complete: f.is_complete(),
    });
    let files = state.index.session_files(id);
    let mut total = main.as_ref().map(|m| m.total.clone()).unwrap_or_default();
    let mut partial = main.as_ref().is_none_or(|m| !m.complete);
    let cutoff = main
        .as_ref()
        .and_then(|m| m.cost_state)
        .map(|(_, ts)| ts.unwrap_or(0));
    let mut sub_cost_total = 0.0;
    let mut sub_cost_after = 0.0;
    let mut sub_after_any = false;
    for a in files.iter().flat_map(|f| f.subagents.values()) {
        let v = state.index.with_file(&a.path, |f| {
            let t = f.agg.usage.total();
            let after = cutoff.map(|c| f.agg.usage.cost_after(c)).unwrap_or(0.0);
            (t, after, f.is_complete())
        });
        match v {
            Some((t, after, complete)) => {
                sub_cost_total += t.cost;
                sub_cost_after += after;
                sub_after_any |= after > 0.0;
                total.add(&Totals { cost: 0.0, ..t });
                partial |= !complete;
            }
            None => partial = true,
        }
    }
    let usage = Usage {
        input: total.input,
        output: total.output,
        cache_read: total.cache_read,
        cache_write: total.cache_write,
        thinking: (total.thinking > 0).then_some(total.thinking),
    };
    let estimated = total.unpriced > 0 || total.nonstandard > 0;
    let (cost, final_) = match main.as_ref().and_then(|m| m.cost_state.map(|c| (m, c))) {
        Some((m, (cs_total, _))) => {
            let c = cs_total + m.after.cost + sub_cost_after;
            let final_ = !live && m.after.messages == 0 && !sub_after_any;
            (Some(c), final_)
        }
        None => {
            let c = main.as_ref().map(|m| m.total.cost).unwrap_or(0.0) + sub_cost_total;
            let any_priced = total.priced > 0;
            (
                (any_priced || total.messages == 0 && main.is_some()).then_some(c),
                false,
            )
        }
    };
    CostView {
        usage,
        cost: cost.filter(|_| main.is_some()),
        estimated,
        final_,
        partial,
    }
}

fn project_name(cwd: &str) -> String {
    std::path::Path::new(cwd)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(cwd)
        .to_string()
}

/// Observed (unanswerable) pending item of a waiting foreign session, from
/// the transcript's last pending tool use.
pub fn observed_item(id: &str, tool: &PendingTool, waiting_for: Option<&str>) -> PendingItem {
    let input = tool.input.clone();
    let (kind, title, questions, plan) = match tool.name.as_str() {
        "AskUserQuestion" => (
            PendingKind::Question,
            "Claude has a question".to_string(),
            input.as_ref().map(crate::pending::questions_from_input),
            None,
        ),
        "ExitPlanMode" => (
            PendingKind::Plan,
            "Plan ready for review".to_string(),
            None,
            input
                .as_ref()
                .and_then(|i| i.get("plan"))
                .and_then(|p| p.as_str())
                .map(str::to_string),
        ),
        name => (
            PendingKind::Permission,
            format!("Allow {name}?"),
            None,
            None,
        ),
    };
    PendingItem {
        id: format!("observed:{}", tool.id),
        session_id: id.to_string(),
        kind,
        source: PendingSource::Observed,
        tool_name: Some(tool.name.clone()),
        tool_use_id: Some(tool.id.clone()),
        title,
        detail: Some(tool.summary.clone())
            .filter(|s| !s.is_empty())
            .or_else(|| waiting_for.map(str::to_string)),
        input,
        questions,
        plan,
        answerable: false,
        created_at: tool.ts.unwrap_or_default(),
        ..Default::default()
    }
}

/// Inputs gathered asynchronously by the monitor before a (sync) build.
#[derive(Debug, Clone, Default)]
pub struct BuildCtx {
    pub now: i64,
    /// `state.owned` (spawned/continued ids + bridge threads).
    pub owned_ids: HashSet<String>,
    /// HTTP bridge conversations.
    pub bridge_ids: HashSet<String>,
    /// cwd/start of owned children and bridge threads.
    pub owned_meta: HashMap<String, (String, i64)>,
    /// The previous snapshot (owned sessions nobody else knows keep their row).
    pub previous: HashMap<String, SessionSummary>,
}

/// Every session id the data layer knows.
pub fn session_ids(state: &AppState, ctx: &BuildCtx) -> Vec<String> {
    let mut ids: HashSet<String> = state.index.session_ids().into_iter().collect();
    ids.extend(state.live.all().into_iter().filter_map(|s| s.session_id));
    ids.extend(state.live.jobs().into_iter().filter_map(|j| j.session_id));
    ids.extend(ctx.owned_ids.iter().cloned());
    ids.extend(ctx.bridge_ids.iter().cloned());
    ids.extend(ctx.owned_meta.keys().cloned());
    let mut v: Vec<String> = ids.into_iter().collect();
    v.sort();
    v
}

/// Build every summary.
pub fn build_all(state: &AppState, ctx: &BuildCtx) -> Vec<SessionSummary> {
    session_ids(state, ctx)
        .iter()
        .filter_map(|id| build_one(state, id, ctx))
        .collect()
}

struct MainFacts {
    agg: Aggregates,
    complete: bool,
    mtime: i64,
    messages: u64,
}

/// Build one session's summary (`None` when nothing is known about it).
pub fn build_one(state: &AppState, id: &str, ctx: &BuildCtx) -> Option<SessionSummary> {
    let pid = state.live.get(id);
    let job = state.live.job(id);
    let files = state.index.session_files(id);
    // A light copy of the aggregates (the big lists stay in the index).
    let main: Option<MainFacts> = state.index.with_main(id, |f| MainFacts {
        agg: f.agg.light_clone(),
        complete: f.is_complete(),
        mtime: f.mtime,
        messages: f.agg.usage.total().messages,
    });
    let bridge_thread = ctx.bridge_ids.contains(id);
    let owned_handle = super::owned::is_owned(state, id);
    let owned = ctx.owned_ids.contains(id) || owned_handle || bridge_thread;
    let previous = ctx.previous.get(id);
    if pid.is_none() && job.is_none() && files.is_none() && !owned {
        return None;
    }
    let legacy_live = !state.live.has_sessions_dir()
        && files
            .as_ref()
            .is_some_and(|f| ctx.now - f.main_mtime < 60_000);
    let live = pid.is_some() || owned_handle || bridge_thread || legacy_live;
    let agg = main.as_ref().map(|m| &m.agg);

    let cwd = pid
        .as_ref()
        .and_then(|p| p.cwd.clone())
        .or_else(|| agg.and_then(|a| a.cwd.clone()))
        .or_else(|| ctx.owned_meta.get(id).map(|(c, _)| c.clone()))
        .or_else(|| job.as_ref().and_then(|j| j.cwd.clone()))
        .or_else(|| previous.map(|p| p.cwd.clone()).filter(|c| !c.is_empty()))
        .or_else(|| {
            files
                .as_ref()
                .and_then(|f| f.project_dir.as_ref())
                .and_then(|d| d.file_name())
                .and_then(|n| n.to_str())
                .map(super::home::best_effort_decode)
        })
        .unwrap_or_default();
    let project = project_name(&cwd);
    let entrypoint = pid
        .as_ref()
        .and_then(|p| p.entrypoint.clone())
        .or_else(|| agg.and_then(|a| a.entrypoint.clone()));
    let surface = surface_for(owned, pid.as_ref(), entrypoint.as_deref(), job.is_some());
    let title = agg.and_then(|a| a.custom_title.clone().or(a.ai_title.clone()));
    let user_named = pid
        .as_ref()
        .filter(|p| p.name_source.as_deref() == Some("user"))
        .and_then(|p| p.name.clone());
    let name = agg
        .and_then(|a| a.custom_title.clone())
        .or(user_named)
        .or_else(|| agg.and_then(|a| a.ai_title.clone()))
        .or_else(|| agg.and_then(|a| a.agent_name.clone()))
        .or_else(|| job.as_ref().and_then(|j| j.name.clone()))
        .or_else(|| previous.map(|p| p.name.clone()).filter(|n| !n.is_empty()))
        .unwrap_or_else(|| project.clone());

    // Pending: store items first, then an observed prompt of a waiting
    // foreign session.
    let store = state.pending.list(id);
    let mut observed: Vec<PendingItem> = Vec::new();
    let waiting = pid
        .as_ref()
        .is_some_and(|p| p.status.as_deref() == Some("waiting"));
    let process_started = pid.as_ref().and_then(|p| p.started_at);
    let pending_tools: &[PendingTool] = agg.map(|a| a.pending_tools.as_slice()).unwrap_or(&[]);
    if live && !owned && store.is_empty() {
        let current: Vec<&PendingTool> = pending_tools
            .iter()
            .filter(|t| process_started.is_none_or(|s| t.ts.is_none_or(|ts| ts + 5_000 >= s)))
            .filter(|t| t.name != "TodoWrite")
            .collect();
        let candidate = if waiting {
            current.last().copied()
        } else if !state.live.has_sessions_dir() {
            current
                .iter()
                .rev()
                .find(|t| matches!(t.name.as_str(), "AskUserQuestion" | "ExitPlanMode"))
                .copied()
        } else {
            None
        };
        if let Some(t) = candidate {
            observed.push(observed_item(
                id,
                t,
                pid.as_ref().and_then(|p| p.waiting_for.as_deref()),
            ));
        }
    }
    state.activity.set_observed(id, observed.clone());
    let previews: Vec<PendingPreview> = store
        .iter()
        .chain(observed.iter())
        .map(PendingPreview::from)
        .collect();

    // Agents.
    let counts = state.agents.counts(id).unwrap_or_else(|| AgentCounts {
        total: files
            .as_ref()
            .map(|f| f.subagents.len() as u32)
            .unwrap_or(0),
        ..Default::default()
    });

    // Activity + status.
    let owned_hint = state.activity.owned_hint(id);
    let hook_hint = state.activity.hook_hint(id);
    let (status, act) = activity::derive(&activity::Inputs {
        live,
        owned,
        pidfile: pid.as_ref(),
        pending: &previews,
        owned_hint,
        hook_hint,
        mark: agg.map(|a| &a.last_mark),
        pending_tools,
        agents_running: counts.running,
        background_running: counts.background_running,
        job: job.as_ref(),
        has_transcript: main.is_some(),
        process_started,
    });

    // Model + context.
    let model_id = agg
        .and_then(|a| a.display_model().map(str::to_string))
        .filter(|m| !pricing::is_synthetic(m));
    let model = model_id.as_ref().map(|m| ModelInfo {
        id: m.clone(),
        label: pricing::model_label(m),
        context_window: Some(pricing::context_window(m)),
    });
    let context = agg.and_then(|a| a.context.as_ref()).map(|c| {
        let window = model_id
            .as_deref()
            .map(pricing::context_window)
            .unwrap_or_else(|| pricing::context_window(&c.model));
        ContextFill {
            used: c.used,
            window,
            pct: if window > 0 {
                ((c.used as f64 / window as f64) * 1000.0).round() / 10.0
            } else {
                0.0
            },
        }
    });
    let todos = agg.and_then(|a| a.todos.as_ref()).map(|t| TodoSummary {
        total: t.len() as u32,
        done: t
            .iter()
            .filter(|x| x.status == TodoStatus::Completed)
            .count() as u32,
        in_progress: t
            .iter()
            .filter(|x| x.status == TodoStatus::InProgress)
            .count() as u32,
        active_form: t
            .iter()
            .find(|x| x.status == TodoStatus::InProgress)
            .and_then(|x| x.active_form.clone().or(Some(x.content.clone()))),
    });
    let cost = cost_view(state, id, live);
    let started_at = pid
        .as_ref()
        .and_then(|p| p.started_at)
        .or_else(|| ctx.owned_meta.get(id).map(|(_, t)| *t))
        .or_else(|| agg.and_then(|a| a.first_ts))
        .or(job.as_ref().and_then(|j| j.created_at))
        .or(previous.and_then(|p| p.started_at));
    let last_activity_at = [
        agg.and_then(|a| a.last_ts),
        pid.as_ref()
            .and_then(|p| p.status_updated_at.or(p.updated_at)),
        main.as_ref().map(|m| m.mtime).filter(|m| *m > 0),
        files.as_ref().map(|f| f.main_mtime).filter(|m| *m > 0),
        job.as_ref().and_then(|j| j.updated_at),
        started_at,
    ]
    .into_iter()
    .flatten()
    .max();
    let queued_cli = agg
        .map(|a| a.queue.iter().filter(|q| !q.notification).count() as u32)
        .unwrap_or(0);
    let queued = queued_cli + crate::server::hooks::queued(state, id).len() as u32;

    // Capabilities.
    let job_id = state.live.short_job_id(id);
    let tmux = pid.as_ref().and_then(|p| p.tmux.clone());
    let facts = SessionFacts {
        id: id.to_string(),
        owned: owned_handle || ctx.owned_ids.contains(id) || bridge_thread,
        bridge_thread,
        live,
        kind: pid
            .as_ref()
            .and_then(|p| p.kind.clone())
            .or_else(|| job.as_ref().map(|_| "background".to_string())),
        job_id: job_id.clone(),
        tmux: tmux.clone(),
    };
    let permission_mode = effective_permission_mode(state, id);
    let mut input: CapsInput = caps::input_with(
        state,
        &facts,
        &caps::ExtraFacts {
            pid: pid.as_ref().and_then(|p| p.pid),
            permission_mode: permission_mode.clone(),
            transcript_missing: main.is_none(),
        },
    );
    if !owned_handle && !bridge_thread {
        // `owned` in the caps means "a live control handle".
        input.owned = false;
    }
    let control = caps::compute(&input);

    let last_error = agg.and_then(|a| a.last_error.as_ref()).map(|e| LastError {
        kind: e.kind.clone(),
        text: e.text.clone(),
        at: e.at,
    });
    let mut summary = SessionSummary {
        id: id.to_string(),
        name,
        title,
        project_name: project,
        cwd,
        surface,
        kind: facts.kind.clone(),
        entrypoint,
        owned,
        live,
        status,
        waiting_for: pid
            .as_ref()
            .filter(|_| status == SessionStatus::Waiting)
            .and_then(|p| p.waiting_for.clone()),
        activity: act,
        model,
        effort: agg.and_then(|a| a.effort.clone()),
        permission_mode,
        git_branch: agg.and_then(|a| a.git_branch.clone()),
        pr_link: agg.and_then(|a| a.pr_link.as_ref()).map(|p| PrLink {
            number: p.number,
            url: p.url.clone(),
            repo: p.repo.clone(),
        }),
        started_at,
        last_activity_at,
        pid: pid.as_ref().and_then(|p| p.pid),
        job_id,
        version: pid
            .as_ref()
            .and_then(|p| p.version.clone())
            .or_else(|| agg.and_then(|a| a.version.clone())),
        tmux_pane: tmux,
        usage: cost.usage,
        cost_usd: cost.cost,
        cost_final: cost.final_.then_some(true),
        cost_estimated: cost.estimated.then_some(true),
        usage_partial: (cost.partial && main.is_some()
            || main.as_ref().is_some_and(|m| !m.complete))
        .then_some(true),
        context,
        message_count: main
            .as_ref()
            .map(|m| m.agg.prompts + m.messages)
            .unwrap_or(0),
        last_prompt: agg.and_then(|a| a.last_prompt.clone()),
        last_message: agg.and_then(|a| a.last_text.clone()),
        todos,
        agents: counts,
        queued,
        pending: previews,
        last_error,
        remote_control: pid
            .as_ref()
            .map(|p| p.bridge_session_id.is_some())
            .filter(|on| *on),
        control,
        cli_name: pid.as_ref().and_then(|p| p.name.clone()),
    };
    if owned_handle {
        // What the control protocol reported (model, mode, effort, queue,
        // activity) wins over what the files say.
        super::owned::overlay(state, &mut summary);
    }
    Some(summary)
}

/// A just-spawned owned session that no data source knows yet.
pub fn synthetic_owned(id: &str, cwd: &str, started_at: i64) -> SessionSummary {
    let project_name = project_name(cwd);
    let control = caps::compute(&CapsInput {
        owned: true,
        live: true,
        ..Default::default()
    });
    SessionSummary {
        id: id.to_string(),
        name: project_name.clone(),
        project_name,
        cwd: cwd.to_string(),
        surface: Surface::MotherClaude,
        owned: true,
        live: true,
        status: SessionStatus::Starting,
        activity: Activity::new(ActivityKind::Starting, "Starting"),
        started_at: Some(started_at),
        last_activity_at: Some(started_at),
        control,
        ..Default::default()
    }
}

/// Every pending item of a session in full: store items, then observed ones.
pub fn pending_detail(state: &AppState, id: &str) -> Vec<PendingItem> {
    let mut items = state.pending.list(id);
    items.extend(state.activity.observed(id));
    items
}

fn task_kind(kind: &str) -> BackgroundTaskKind {
    match kind {
        "agent" => BackgroundTaskKind::Agent,
        "workflow" => BackgroundTaskKind::Workflow,
        "monitor" => BackgroundTaskKind::Monitor,
        _ => BackgroundTaskKind::Bash,
    }
}

/// `GET /api/sessions/{id}` detail: agents/workflows (computed on demand),
/// background tasks, todos and files changed from the index aggregates, the
/// relay queue, and the full pending items.
pub async fn detail(state: &AppState, summary: SessionSummary) -> SessionDetail {
    let id = summary.id.clone();
    if !state.agents.has_session(&id) {
        let st = state.clone();
        let sid = id.clone();
        let live = summary.live;
        let owned = summary.owned;
        let started = summary.started_at;
        let _ = tokio::task::spawn_blocking(move || {
            st.agents.recompute(
                &st.index,
                &sid,
                &super::agents::AgentCtx {
                    live,
                    process_started: started,
                    owned,
                    now: chrono::Utc::now().timestamp_millis(),
                },
            )
        })
        .await;
    }
    let (agents_detail, workflows) = state.agents.for_session(&id);
    let cwd = summary.cwd.clone();
    let live = summary.live;
    let started = summary.started_at;
    let (tasks, todos_detail, files_changed) = state
        .index
        .with_main(&id, |f| {
            let tasks: Vec<BackgroundTask> = f
                .agg
                .tasks
                .iter()
                .rev()
                .take(100)
                .map(|t| {
                    let stale_running = t.status == "running"
                        && (!live
                            || started
                                .is_some_and(|s| t.started_at.is_some_and(|ts| ts + 5_000 < s)));
                    BackgroundTask {
                        id: t.id.clone(),
                        kind: task_kind(&t.kind),
                        label: t.label.clone(),
                        status: if stale_running {
                            "stopped".into()
                        } else {
                            t.status.clone()
                        },
                        started_at: t.started_at,
                        ended_at: t.ended_at,
                        output_available: t
                            .output_file
                            .as_deref()
                            .is_some_and(|p| std::path::Path::new(p).is_file()),
                    }
                })
                .collect();
            let files: Vec<FileChanged> = f
                .agg
                .files_changed
                .iter()
                .map(|(p, (a, r))| FileChanged {
                    path: p
                        .strip_prefix(&cwd)
                        .map(|r| r.trim_start_matches('/').to_string())
                        .filter(|r| !r.is_empty())
                        .unwrap_or_else(|| p.clone()),
                    added: (*a > 0 || *r > 0).then_some(*a),
                    removed: (*a > 0 || *r > 0).then_some(*r),
                })
                .collect();
            (tasks, f.agg.todos.clone().unwrap_or_default(), files)
        })
        .unwrap_or_default();
    let mut queued_messages = crate::server::hooks::queued(state, &id);
    let (mut agents_detail, mut tasks) = (agents_detail, tasks);
    if summary.owned {
        // An owned session's control protocol reports its queue, subagents
        // and background tasks live; merge them over what the files show.
        for q in super::owned::queued(state, &id) {
            if !queued_messages.iter().any(|m| m.id == q.id) {
                queued_messages.push(q);
            }
        }
        let (owned_agents, owned_tasks) = super::owned::tasks(state, &id);
        for a in owned_agents {
            // Same rule as the owned `agents` frame: the disk view wins, the
            // control protocol adds what the files don't show yet.
            let dup = agents_detail.iter().any(|b| {
                b.id == a.id || (a.tool_use_id.is_some() && b.tool_use_id == a.tool_use_id)
            });
            if !dup {
                agents_detail.push(a);
            }
        }
        for t in owned_tasks {
            match tasks.iter_mut().find(|x| x.id == t.id) {
                Some(x) => *x = t,
                None => tasks.push(t),
            }
        }
    }
    SessionDetail {
        pending_detail: pending_detail(state, &id),
        summary,
        agents_detail,
        workflows,
        tasks,
        todos_detail,
        queued_messages,
        files_changed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::model::ControlChannel;

    #[test]
    fn tmux_targets() {
        assert!(is_tmux_target("mcprobe:@0.%0"));
        assert!(is_tmux_target("main:1.3"));
        assert!(!is_tmux_target("main"));
        assert!(!is_tmux_target("a b:@1.%2"));
        assert!(!is_tmux_target("x:@1234567.%1"));
    }

    #[test]
    fn surfaces() {
        let tmux = LiveSession {
            tmux: Some("s:@1.%2".into()),
            ..Default::default()
        };
        assert_eq!(
            surface_for(false, Some(&tmux), Some("cli"), false),
            Surface::Tmux
        );
        assert_eq!(
            surface_for(true, Some(&tmux), None, false),
            Surface::MotherClaude
        );
        let bg = LiveSession {
            kind: Some("bg".into()),
            ..Default::default()
        };
        assert_eq!(
            surface_for(false, Some(&bg), Some("cli"), false),
            Surface::Background
        );
        assert_eq!(
            surface_for(false, None, Some("claude-vscode"), false),
            Surface::Vscode
        );
        assert_eq!(
            surface_for(false, None, Some("sdk-cli"), false),
            Surface::Sdk
        );
        assert_eq!(
            surface_for(false, None, Some("cli"), false),
            Surface::Terminal
        );
        assert_eq!(surface_for(false, None, None, true), Surface::Background);
        assert_eq!(surface_for(false, None, None, false), Surface::Unknown);
    }

    #[test]
    fn observed_items() {
        let q = observed_item(
            "s",
            &PendingTool {
                id: "t1".into(),
                name: "AskUserQuestion".into(),
                summary: "DB?".into(),
                ts: Some(5),
                input: Some(
                    serde_json::json!({"questions":[{"question":"DB?","options":[{"label":"pg"}],"multiSelect":false}]}),
                ),
            },
            Some("input needed"),
        );
        assert_eq!(q.kind, PendingKind::Question);
        assert!(!q.answerable);
        assert_eq!(q.source, PendingSource::Observed);
        assert_eq!(q.questions.as_ref().unwrap().len(), 1);
        let preview = PendingPreview::from(&q);
        assert_eq!(preview.quick.unwrap().options, vec!["pg".to_string()]);
        let p = observed_item(
            "s",
            &PendingTool {
                id: "t2".into(),
                name: "Bash".into(),
                summary: "rm -rf build".into(),
                ts: Some(5),
                input: None,
            },
            Some("permission prompt"),
        );
        assert_eq!(p.title, "Allow Bash?");
        assert_eq!(p.detail.as_deref(), Some("rm -rf build"));
    }

    #[test]
    fn synthetic_owned_rows() {
        let syn = synthetic_owned("n1", "/x/y", 5);
        assert_eq!(syn.project_name, "y");
        assert_eq!(syn.status, SessionStatus::Starting);
        assert!(syn.owned && syn.live);
        assert_eq!(syn.control.channel, ControlChannel::Owned);
    }

    #[test]
    fn activity_for_previews_and_items() {
        let items = vec![PendingItem {
            kind: PendingKind::Plan,
            answerable: true,
            ..Default::default()
        }];
        assert_eq!(
            activity_for(SessionStatus::Waiting, &items).label,
            "Waiting for plan approval"
        );
        let previews: Vec<PendingPreview> = items.iter().map(PendingPreview::from).collect();
        assert_eq!(
            activity_for(SessionStatus::Waiting, &previews).label,
            "Waiting for plan approval"
        );
    }
}
