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
//! * Effort, ultracode, thinking and fast mode come from [`derive_settings`]
//!   (transcript, main-thread hook reports, the process argv, the user's
//!   `alwaysThinkingEnabled`), each with its source; an owned session's
//!   control protocol then wins (`owned::overlay_settings`).
//!
//! Everything here is synchronous and in-memory (no disk reads besides the
//! cheap caches of the index/agents), so the monitor can rebuild every
//! summary on each refresh; `ws::publish_sessions` diffs by content hash.

use std::collections::{HashMap, HashSet};

use super::activity::{self, PendingLike};
use super::caps::{self, CapsInput, SessionFacts};
use super::index::{notes_ultracode, Aggregates, FileIndex, PendingTool, Totals, UltraUnknown};
use super::live::{LaunchFlags, LaunchThinking, LiveSession};
use super::model::{
    Activity, ActivityKind, AgentCounts, BackgroundTask, BackgroundTaskKind, ContextFill,
    EffortSource, FileChanged, LastError, ModelInfo, PendingItem, PendingKind, PendingPreview,
    PendingSource, PrLink, SessionDetail, SessionStatus, SessionSummary, Surface, ThinkingInfo,
    ThinkingMode, ThinkingSource, TodoStatus, TodoSummary, UltracodeSource, UltracodeState,
    UltracodeUnknown, Usage,
};
use super::pricing;
use crate::state::AppState;

/// Records written this long before a pidfile's `startedAt` still belong to
/// the process (the same slack the pending-tool and queue filters use).
const START_SLACK_MS: i64 = 5_000;

/// Everything the effort / ultracode / thinking / fast derivation of a
/// session looks at (design2 §1). Owned sessions get these from their
/// control protocol afterwards ([`super::owned::overlay_settings`]).
#[derive(Debug, Clone, Default)]
pub struct SettingsFacts<'a> {
    pub live: bool,
    /// Start of the live process (pidfile `startedAt`), when known.
    pub process_started: Option<i64>,
    pub agg: Option<&'a Aggregates>,
    /// The main transcript exists but is not read in full yet (not indexed,
    /// or only its tail).
    pub transcript_partial: bool,
    /// `(level, epoch ms)` of the last main-thread hook report.
    pub hook_effort: Option<(String, i64)>,
    /// What the live process's argv says.
    pub launch: Option<LaunchFlags>,
    /// The user's `alwaysThinkingEnabled`.
    pub always_thinking: Option<bool>,
    /// The model the session runs.
    pub model: Option<&'a str>,
}

/// A session's derived settings, each with its source.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SettingsView {
    pub effort: Option<String>,
    pub effort_source: Option<EffortSource>,
    pub ultracode: Option<UltracodeState>,
    pub ultracode_source: Option<UltracodeSource>,
    pub ultracode_at: Option<i64>,
    pub ultracode_unknown: Option<UltracodeUnknown>,
    pub ultracode_before: Option<UltracodeState>,
    pub ultracode_before_at: Option<i64>,
    pub thinking: Option<ThinkingInfo>,
    pub last_thinking_at: Option<i64>,
    pub fast: Option<bool>,
}

/// Whether something recorded at `ts` belongs to the process that started at
/// `started` (unknown start: yes; unknown time: no).
fn of_current_process(ts: Option<i64>, started: Option<i64>) -> bool {
    match (ts, started) {
        (_, None) => true,
        (Some(ts), Some(s)) => ts + START_SLACK_MS >= s,
        (None, Some(_)) => false,
    }
}

/// Thinking when nobody set it: off when the user's `alwaysThinkingEnabled`
/// is false, adaptive on adaptive-capable models, else unknown.
pub fn default_thinking(
    always_thinking: Option<bool>,
    adaptive: Option<bool>,
) -> Option<ThinkingInfo> {
    let mode = match (always_thinking, adaptive) {
        (Some(false), _) => ThinkingMode::Off,
        (_, Some(true)) => ThinkingMode::Adaptive,
        _ => return None,
    };
    Some(ThinkingInfo {
        mode,
        source: ThinkingSource::Default,
        budget: None,
    })
}

/// Effort, ultracode, thinking and fast mode of a session from the files,
/// hooks and argv (design2 §1):
///
/// * effort — the newer of the last main-thread hook report and the last
///   main-chain transcript record (`observed`; for a live session a record
///   older than the process yields to its `--effort`), else `--effort`
///   (`launch`). A newer record sent without effort (a model that has none,
///   e.g. after `/model haiku`) means no effort — neither an older hook
///   report nor `--effort` stands in for it;
/// * ultracode — the state as of the last message sent (`transcript`, dated
///   to that message): the marker Claude Code wrote after it, or without
///   one what its markers said until then ([`super::index::UltraReplay`]).
///   A live session takes only a message of its own process: a restart
///   turns ultracode off (VS Code doesn't switch it back on), so an earlier
///   value is only what it was before (`restarted`). Else `--effort
///   ultracode` / inline `--settings` (`launch`); else unknown, with why;
/// * thinking (live only) — `--thinking` / `--max-thinking-tokens`
///   (`launch`), else the default ([`default_thinking`]);
/// * the last thinking block and fast mode as evidence, for every session.
pub fn derive_settings(f: &SettingsFacts) -> SettingsView {
    let mut v = SettingsView::default();
    let launch = f.launch.as_ref().filter(|_| f.live);
    let launch_effort = launch.and_then(|l| l.effort.clone());

    // The last main-chain request's effort with its time; `(None, ts)` when
    // that request carried none.
    let transcript = f
        .agg
        .filter(|a| a.effort.is_some() || a.effort_ts.is_some())
        .map(|a| (a.effort.clone(), a.effort_ts))
        .filter(|(_, ts)| {
            !f.live || launch_effort.is_none() || of_current_process(*ts, f.process_started)
        });
    let hook = f.hook_effort.clone().filter(|_| f.live);
    // A hook report wins only when it is newer than the record (or the
    // record has no time).
    let observed: Option<Option<String>> = match (transcript, hook) {
        (Some((e, ts)), Some((h, at))) => Some(if ts.is_none_or(|ts| at > ts) {
            Some(h)
        } else {
            e
        }),
        (Some((e, _)), None) => Some(e),
        (None, Some((h, _))) => Some(Some(h)),
        (None, None) => None,
    };
    match observed {
        Some(Some(e)) => {
            v.effort = Some(e);
            v.effort_source = Some(EffortSource::Observed);
        }
        // The latest request ran without effort: the model has none.
        Some(None) => {}
        None => {
            if let Some(e) = launch_effort {
                v.effort = Some(e);
                v.effort_source = Some(EffortSource::Launch);
            }
        }
    }

    let state = |on: bool| {
        if on {
            UltracodeState::On
        } else {
            UltracodeState::Off
        }
    };
    let latest = f.agg.and_then(|a| a.ultra.state.as_ref());
    let unknown = match latest {
        Some(st) if f.live && !of_current_process(st.at, f.process_started) => {
            v.ultracode_before = st.on.map(state);
            v.ultracode_before_at = st.at.filter(|_| st.on.is_some());
            Some(UltracodeUnknown::Restarted)
        }
        Some(st) => match st.on {
            Some(on) => {
                v.ultracode = Some(state(on));
                v.ultracode_source = Some(UltracodeSource::Transcript);
                v.ultracode_at = st.at;
                None
            }
            None => Some(match st.unknown {
                Some(UltraUnknown::Compacted) => UltracodeUnknown::Compacted,
                Some(UltraUnknown::Unsupported) => UltracodeUnknown::Unsupported,
                Some(UltraUnknown::Reading) | None => UltracodeUnknown::Reading,
            }),
        },
        None if f.transcript_partial => Some(UltracodeUnknown::Reading),
        // A transcript of a version that notes nothing (its prompts may not
        // even be told apart), else no message yet.
        None if f
            .agg
            .and_then(|a| a.version.as_deref())
            .is_some_and(|v| !notes_ultracode(v)) =>
        {
            Some(UltracodeUnknown::Unsupported)
        }
        None => Some(UltracodeUnknown::NoPrompt),
    };
    match launch
        .and_then(|l| l.ultracode)
        .filter(|_| unknown.is_some())
    {
        Some(on) => {
            v.ultracode = Some(state(on));
            v.ultracode_source = Some(UltracodeSource::Launch);
            v.ultracode_before = None;
            v.ultracode_before_at = None;
        }
        None => v.ultracode_unknown = unknown,
    }

    if f.live {
        let adaptive = f.model.and_then(pricing::supports_adaptive_thinking);
        let at_launch = |mode: ThinkingMode, budget: Option<u32>| ThinkingInfo {
            mode,
            source: ThinkingSource::Launch,
            budget,
        };
        v.thinking = match launch.and_then(|l| l.thinking) {
            Some(LaunchThinking::Off) => Some(at_launch(ThinkingMode::Off, None)),
            Some(LaunchThinking::Adaptive) => Some(at_launch(ThinkingMode::Adaptive, None)),
            Some(LaunchThinking::Budget(n)) => Some(match adaptive {
                Some(true) => at_launch(ThinkingMode::Adaptive, None),
                _ => at_launch(ThinkingMode::On, Some(n)),
            }),
            None => default_thinking(f.always_thinking, adaptive),
        };
    }
    v.last_thinking_at = f.agg.and_then(|a| a.last_thinking_ts);
    v.fast = f.agg.filter(|a| a.fast).map(|_| true);
    v
}

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

/// Priced cost of a session's messages (main + subagent files) written at or
/// after `since` (within the last day, e.g. local midnight). The `cost-state`
/// baseline cannot be split by day, so this is the priced estimate. `None`
/// when the main transcript is not indexed.
pub fn cost_since(state: &AppState, id: &str, since: i64) -> Option<f64> {
    let main = state
        .index
        .with_main(id, |f| f.agg.usage.cost_since(since))?;
    let subs: f64 = state
        .index
        .session_files(id)
        .iter()
        .flat_map(|f| f.subagents.values())
        .filter_map(|a| {
            state
                .index
                .with_file(&a.path, |f| f.agg.usage.cost_since(since))
        })
        .sum();
    Some(((main + subs) * 1e6).round() / 1e6)
}

/// Local midnight of the day containing `now_ms` (epoch ms).
pub fn local_midnight_ms(now_ms: i64) -> i64 {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(now_ms)
        .single()
        .and_then(|now| {
            now.date_naive()
                .and_hms_opt(0, 0, 0)
                .and_then(|d| d.and_local_timezone(chrono::Local).earliest())
        })
        .map(|d| d.timestamp_millis())
        .unwrap_or(now_ms - 24 * 3600 * 1000)
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

/// Whether a tool call delegates to a subagent (its result arrives only when
/// the subagent is done).
fn is_delegation(t: &PendingTool) -> bool {
    matches!(t.name.as_str(), "Agent" | "Task")
}

/// The pending tool a foreground subagent is blocked on, with its agent id:
/// the newest pending tool of the subagent the `call` launched or of one of
/// its nested agents. When the tracker has not mapped that subagent yet, any
/// subagent file but the known background and workflow agents (whose running
/// tools are not what the session waits on).
fn subagent_waiting_tool(
    state: &AppState,
    id: &str,
    files: Option<&super::index::SessionFiles>,
    call: &PendingTool,
    process_started: Option<i64>,
) -> Option<(PendingTool, String)> {
    let files = files?;
    let agents = state.agents.for_session(id).0;
    let allowed: HashSet<&str> = match agents
        .iter()
        .find(|a| a.tool_use_id.as_deref() == Some(call.id.as_str()))
    {
        Some(root) => {
            let mut tree: HashSet<&str> = HashSet::from([root.id.as_str()]);
            loop {
                let before = tree.len();
                for a in &agents {
                    if a.parent_id.as_deref().is_some_and(|p| tree.contains(p)) {
                        tree.insert(a.id.as_str());
                    }
                }
                if tree.len() == before {
                    break tree;
                }
            }
        }
        None => {
            let elsewhere: HashSet<&str> = agents
                .iter()
                .filter(|a| a.background || a.workflow_run_id.is_some())
                .map(|a| a.id.as_str())
                .collect();
            files
                .subagents
                .keys()
                .map(String::as_str)
                .filter(|a| !elsewhere.contains(a))
                .collect()
        }
    };
    files
        .subagents
        .values()
        .filter(|a| allowed.contains(a.agent_id.as_str()))
        .filter_map(|a| {
            state
                .index
                .with_file(&a.path, |f| {
                    f.agg
                        .pending_tools
                        .iter()
                        .filter(|t| t.name != "TodoWrite" && !is_delegation(t))
                        .filter(|t| {
                            process_started.is_none_or(|s| t.ts.is_none_or(|ts| ts + 5_000 >= s))
                        })
                        .max_by_key(|t| t.ts)
                        .cloned()
                })
                .flatten()
                .map(|t| (t, a.agent_id.clone()))
        })
        .max_by_key(|(t, _)| t.ts)
}

/// What a waiting foreign session is waiting on: the newest current pending
/// tool, or, when that is a foreground subagent call, the tool its subagent
/// is blocked on (`Err(agent call)` when that cannot be found).
fn waiting_tool<'a>(
    state: &AppState,
    id: &str,
    files: Option<&super::index::SessionFiles>,
    current: &[&'a PendingTool],
    process_started: Option<i64>,
) -> Option<Result<(PendingTool, Option<String>), &'a PendingTool>> {
    let last = *current.last()?;
    if !is_delegation(last) {
        return Some(Ok((last.clone(), None)));
    }
    Some(
        match subagent_waiting_tool(state, id, files, last, process_started) {
            Some((t, agent)) => Ok((t, Some(agent))),
            None => Err(last),
        },
    )
}

/// The observed item for a foreground subagent that waits on something we
/// could not find: no tool name (it is not the `Agent` call that waits).
fn observed_subagent_item(id: &str, call: &PendingTool, waiting_for: Option<&str>) -> PendingItem {
    PendingItem {
        id: format!("observed:{}", call.id),
        session_id: id.to_string(),
        kind: PendingKind::Permission,
        source: PendingSource::Observed,
        title: "A subagent is waiting for approval".to_string(),
        detail: Some(call.summary.clone())
            .filter(|s| !s.is_empty())
            .or_else(|| waiting_for.map(str::to_string)),
        answerable: false,
        created_at: call.ts.unwrap_or_default(),
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
    /// Every account's chip by id; empty while only the Default exists (no
    /// session then names an account).
    pub accounts: std::collections::BTreeMap<String, super::model::AccountRef>,
}

/// The account a session runs (or ran) under: an owned session's own, a
/// bridge conversation's, else the Default's.
pub fn session_account(state: &AppState, id: &str) -> String {
    state
        .control
        .account_of(id)
        .unwrap_or_else(|| super::accounts::DEFAULT_ID.to_string())
}

/// The chip for a session, from [`BuildCtx::accounts`].
fn account_chip(state: &AppState, id: &str, ctx: &BuildCtx) -> Option<super::model::AccountRef> {
    if ctx.accounts.is_empty() {
        return None;
    }
    let account = session_account(state, id);
    ctx.accounts
        .get(&account)
        .or_else(|| ctx.accounts.get(super::accounts::DEFAULT_ID))
        .cloned()
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

    let project_enc = files
        .as_ref()
        .and_then(|f| f.project_dir.as_ref())
        .and_then(|d| d.file_name())
        .and_then(|n| n.to_str());
    let cwd = pid
        .as_ref()
        .and_then(|p| p.cwd.clone())
        .or_else(|| {
            agg.and_then(|a| a.project_cwd(project_enc))
                .map(str::to_string)
        })
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
    let current: Vec<&PendingTool> = pending_tools
        .iter()
        .filter(|t| process_started.is_none_or(|s| t.ts.is_none_or(|ts| ts + 5_000 >= s)))
        .filter(|t| t.name != "TodoWrite")
        .collect();
    // What a waiting session waits on (a foreground subagent's own tool
    // rather than the `Agent` call that delegated to it).
    let waiting_on = if live && waiting {
        waiting_tool(state, id, files.as_ref(), &current, process_started)
    } else {
        None
    };
    if live && !owned && store.is_empty() {
        let waiting_for = pid.as_ref().and_then(|p| p.waiting_for.as_deref());
        if waiting {
            match &waiting_on {
                Some(Ok((t, agent))) => {
                    let mut item = observed_item(id, t, waiting_for);
                    item.agent_id = agent.clone();
                    observed.push(item);
                }
                Some(Err(call)) => observed.push(observed_subagent_item(id, call, waiting_for)),
                None => {}
            }
        } else if !state.live.has_sessions_dir() {
            if let Some(t) = current
                .iter()
                .rev()
                .find(|t| matches!(t.name.as_str(), "AskUserQuestion" | "ExitPlanMode"))
            {
                observed.push(observed_item(id, t, waiting_for));
            }
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
        waiting_on: waiting_on.as_ref().map(|w| match w {
            Ok((t, _)) => Some(t),
            Err(_) => None,
        }),
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
    let midnight = local_midnight_ms(ctx.now);
    let cost_today = last_activity_at
        .filter(|t| *t >= midnight)
        .and_then(|_| cost_since(state, id, midnight))
        .filter(|c| *c > 0.0);
    // Nothing is queued in a process that is gone, nor carried over from an
    // earlier process of the same session.
    let queued_cli = agg
        .filter(|_| live)
        .map(|a| {
            a.queue
                .iter()
                .filter(|q| !q.notification)
                .filter(|q| process_started.is_none_or(|s| q.ts.is_none_or(|ts| ts + 5_000 >= s)))
                .count() as u32
        })
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
    let settings = derive_settings(&SettingsFacts {
        live,
        process_started: process_started.or_else(|| ctx.owned_meta.get(id).map(|(_, t)| *t)),
        agg,
        transcript_partial: files.as_ref().is_some_and(|f| f.main.is_some())
            && main.as_ref().is_none_or(|m| !m.complete),
        hook_effort: live.then(|| state.activity.effort_note(id)).flatten(),
        launch: live.then(|| state.live.launch_flags(id)).flatten(),
        always_thinking: live
            .then(|| super::settings::always_thinking_enabled(&state.home))
            .flatten(),
        model: model_id.as_deref(),
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
        effort: settings.effort,
        effort_source: settings.effort_source,
        ultracode: settings.ultracode,
        ultracode_source: settings.ultracode_source,
        ultracode_at: settings.ultracode_at,
        ultracode_unknown: settings.ultracode_unknown,
        ultracode_before: settings.ultracode_before,
        ultracode_before_at: settings.ultracode_before_at,
        thinking: settings.thinking,
        last_thinking_at: settings.last_thinking_at,
        fast: settings.fast,
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
        cost_today,
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
        account: account_chip(state, id, ctx),
    };
    if owned_handle {
        // What the control protocol reported (model, mode, effort,
        // ultracode, thinking, queue, activity) wins over what the files say.
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
    // Off the async workers: the index's per-file lock can be held by the
    // monitor's catch-up scan (up to 64 MB), and the output files are
    // stat'ed after it is released.
    let st = state.clone();
    let sid = id.clone();
    let (tasks, todos_detail, files_changed) = tokio::task::spawn_blocking(move || {
        let (tasks, todos, files) = st
            .index
            .with_main(&sid, |f| {
                let tasks: Vec<(BackgroundTask, Option<String>)> = f
                    .agg
                    .tasks
                    .iter()
                    .rev()
                    .take(100)
                    .map(|t| {
                        let stale_running = t.status == "running"
                            && (!live
                                || started.is_some_and(|s| {
                                    t.started_at.is_some_and(|ts| ts + 5_000 < s)
                                }));
                        let task = BackgroundTask {
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
                            output_available: false,
                        };
                        (task, t.output_file.clone())
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
        let tasks: Vec<BackgroundTask> = tasks
            .into_iter()
            .map(|(mut task, output)| {
                task.output_available = output
                    .as_deref()
                    .is_some_and(|p| std::path::Path::new(p).is_file());
                task
            })
            .collect();
        (tasks, todos, files)
    })
    .await
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
    fn a_waiting_foreground_subagent_is_observed_by_its_own_tool() {
        let tmp = tempfile::tempdir().unwrap();
        let home = crate::claude::ClaudeHome::with_base(tmp.path());
        let sid = "e2b6972f-7126-4f6e-a490-5ce75cb6a9f9";
        let me = std::process::id();
        let now = chrono::Utc::now().timestamp_millis();
        let iso = |ms: i64| {
            chrono::DateTime::from_timestamp_millis(ms)
                .unwrap()
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        };
        std::fs::create_dir_all(tmp.path().join("sessions")).unwrap();
        std::fs::write(
            tmp.path().join(format!("sessions/{me}.json")),
            format!(
                r#"{{"pid":{me},"sessionId":"{sid}","cwd":"/p","startedAt":{},"status":"waiting","waitingFor":"permission prompt"}}"#,
                now - 60_000
            ),
        )
        .unwrap();
        let proj = tmp.path().join("projects/-p");
        std::fs::create_dir_all(proj.join(sid).join("subagents")).unwrap();
        // Main: a foreground Agent call without a result.
        std::fs::write(
            proj.join(format!("{sid}.jsonl")),
            format!(
                r#"{{"type":"assistant","timestamp":"{}","message":{{"id":"m1","model":"claude-haiku-4-5","stop_reason":"tool_use","content":[{{"type":"tool_use","id":"tu_agent","name":"Agent","input":{{"description":"Explore the repo"}}}}],"usage":{{"input_tokens":1,"output_tokens":1}}}}}}"#,
                iso(now - 30_000)
            ) + "\n",
        )
        .unwrap();
        let sub = proj.join(sid).join("subagents/agent-aexp.jsonl");
        let sub_line = |tool: &str| {
            format!(
                r#"{{"type":"assistant","isSidechain":true,"timestamp":"{}","message":{{"id":"s1","model":"claude-haiku-4-5","content":[{tool}],"usage":{{"input_tokens":1,"output_tokens":1}}}}}}"#,
                iso(now - 10_000)
            ) + "\n"
        };
        // The subagent is blocked on a Bash call.
        std::fs::write(
            &sub,
            sub_line(r#"{"type":"tool_use","id":"stu1","name":"Bash","input":{"command":"rm -rf build","description":"Clean build"}}"#),
        )
        .unwrap();
        std::fs::write(
            sub.with_file_name("agent-aexp.meta.json"),
            r#"{"agentType":"Explore","toolUseId":"tu_agent","requestShape":"foreground"}"#,
        )
        .unwrap();
        // A background agent busy with a newer (running, not waiting) tool.
        std::fs::write(
            sub.with_file_name("agent-abg.jsonl"),
            sub_line(r#"{"type":"tool_use","id":"btu1","name":"Grep","input":{"pattern":"x"}}"#)
                .replace(&iso(now - 10_000), &iso(now - 1_000)),
        )
        .unwrap();
        std::fs::write(
            sub.with_file_name("agent-abg.meta.json"),
            r#"{"agentType":"general-purpose","toolUseId":"tu_bg","requestShape":"background"}"#,
        )
        .unwrap();
        // A fresh state over the home, fully indexed.
        let load = || {
            let state = crate::state::Inner::new(
                home.clone(),
                crate::state::ServerConfig {
                    host: "127.0.0.1".into(),
                    port: 0,
                },
                crate::server::auth::Auth::ephemeral(),
            );
            state.live.refresh(&home);
            state.index.discover(&home);
            state.index.queue_all();
            state.index.drain_jobs_blocking();
            state.agents.recompute(
                &state.index,
                sid,
                &crate::claude::agents::AgentCtx {
                    live: true,
                    now,
                    ..Default::default()
                },
            );
            state
        };
        let ctx = BuildCtx {
            now,
            ..Default::default()
        };
        let state = load();
        let s = build_one(&state, sid, &ctx).unwrap();
        assert_eq!(s.status, SessionStatus::Waiting);
        let p = &s.pending[0];
        assert_eq!(p.tool_name.as_deref(), Some("Bash"));
        assert_eq!(p.agent_id.as_deref(), Some("aexp"));
        assert_eq!(s.activity.tool.as_deref(), Some("Bash"));
        assert_eq!(s.activity.detail.as_deref(), Some("Clean build"));
        let item = &state.activity.observed(sid)[0];
        assert_eq!(item.title, "Allow Bash?");
        assert_eq!(item.agent_id.as_deref(), Some("aexp"));

        // Nothing pending in the subagent: a generic item, never "Allow Agent?".
        std::fs::write(&sub, sub_line(r#"{"type":"text","text":"thinking"}"#)).unwrap();
        let state = load();
        let s = build_one(&state, sid, &ctx).unwrap();
        let item = &state.activity.observed(sid)[0];
        assert_eq!(item.title, "A subagent is waiting for approval");
        assert_eq!(item.tool_name, None);
        assert_eq!(item.detail.as_deref(), Some("Explore the repo"));
        assert_eq!(s.activity.tool, None);
    }

    /// Aggregates whose last regular prompt (at `at`) says `on`.
    fn ultra_as_of(on: Option<bool>, at: i64) -> crate::claude::index::UltraReplay {
        use crate::claude::index::{UltraReplay, UltraSeen, UltraState};
        UltraReplay {
            seen: match on {
                Some(true) => UltraSeen::Enter,
                Some(false) => UltraSeen::Exit,
                None => UltraSeen::Compacted,
            },
            state: Some(UltraState {
                on,
                at: Some(at),
                unknown: on.is_none().then_some(UltraUnknown::Compacted),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn settings_derivation_precedence() {
        let agg = Aggregates {
            effort: Some("xhigh".into()),
            effort_ts: Some(1_000),
            ultra: ultra_as_of(Some(true), 1_500),
            last_thinking_ts: Some(1_200),
            fast: true,
            ..Default::default()
        };
        // Ended: the transcript only; no thinking state, just the evidence.
        let v = derive_settings(&SettingsFacts {
            agg: Some(&agg),
            launch: Some(LaunchFlags {
                effort: Some("low".into()),
                ..Default::default()
            }),
            hook_effort: Some(("max".into(), 9_999)),
            model: Some("claude-opus-5-5"),
            ..Default::default()
        });
        assert_eq!(v.effort.as_deref(), Some("xhigh"));
        assert_eq!(v.effort_source, Some(EffortSource::Observed));
        assert_eq!(v.ultracode, Some(UltracodeState::On));
        assert_eq!(v.ultracode_source, Some(UltracodeSource::Transcript));
        assert_eq!(v.ultracode_at, Some(1_500));
        assert_eq!(v.thinking, None);
        assert_eq!(v.last_thinking_at, Some(1_200));
        assert_eq!(v.fast, Some(true));

        // Live: the newer of the hook report and the transcript.
        let live = SettingsFacts {
            live: true,
            process_started: Some(500),
            agg: Some(&agg),
            model: Some("claude-opus-5-5"),
            ..Default::default()
        };
        let v = derive_settings(&SettingsFacts {
            hook_effort: Some(("max".into(), 2_000)),
            ..live.clone()
        });
        assert_eq!(v.effort.as_deref(), Some("max"));
        assert_eq!(v.effort_source, Some(EffortSource::Observed));
        let v = derive_settings(&SettingsFacts {
            hook_effort: Some(("max".into(), 900)),
            ..live.clone()
        });
        assert_eq!(v.effort.as_deref(), Some("xhigh"));
        // `/model haiku`: the newest request went without effort, so an
        // older hook report of `max` (Haiku's hooks carry none) and the
        // launch flag no longer stand — the model has no effort.
        let haiku = Aggregates {
            effort: None,
            effort_ts: Some(2_000),
            ..agg.clone()
        };
        let v = derive_settings(&SettingsFacts {
            agg: Some(&haiku),
            hook_effort: Some(("max".into(), 1_500)),
            launch: Some(LaunchFlags {
                effort: Some("low".into()),
                ..Default::default()
            }),
            ..live.clone()
        });
        assert_eq!((v.effort, v.effort_source), (None, None));
        // Ended, the same.
        let v = derive_settings(&SettingsFacts {
            agg: Some(&haiku),
            ..Default::default()
        });
        assert_eq!((v.effort, v.effort_source), (None, None));
        // Back on a model with effort, a newer hook report shows it.
        let v = derive_settings(&SettingsFacts {
            agg: Some(&haiku),
            hook_effort: Some(("max".into(), 2_500)),
            ..live.clone()
        });
        assert_eq!(v.effort.as_deref(), Some("max"));
        assert_eq!(v.effort_source, Some(EffortSource::Observed));
        // Default thinking: adaptive on adaptive-capable models …
        assert_eq!(
            v.thinking,
            Some(ThinkingInfo {
                mode: ThinkingMode::Adaptive,
                source: ThinkingSource::Default,
                budget: None
            })
        );
        // … off when the user turned it off, unknown elsewhere.
        let v = derive_settings(&SettingsFacts {
            always_thinking: Some(false),
            ..live.clone()
        });
        assert_eq!(v.thinking.map(|t| t.mode), Some(ThinkingMode::Off));
        let v = derive_settings(&SettingsFacts {
            model: Some("claude-haiku-4-5"),
            ..live.clone()
        });
        assert_eq!(v.thinking, None);
        let v = derive_settings(&SettingsFacts {
            model: None,
            ..live.clone()
        });
        assert_eq!(v.thinking, None);

        // Launch flags: thinking at launch; a budget is adaptive on
        // adaptive-capable models, on + budget elsewhere.
        let flags = |thinking| LaunchFlags {
            thinking: Some(thinking),
            ..Default::default()
        };
        let v = derive_settings(&SettingsFacts {
            launch: Some(flags(LaunchThinking::Budget(31_999))),
            always_thinking: Some(false),
            ..live.clone()
        });
        assert_eq!(
            v.thinking,
            Some(ThinkingInfo {
                mode: ThinkingMode::Adaptive,
                source: ThinkingSource::Launch,
                budget: None
            })
        );
        let v = derive_settings(&SettingsFacts {
            launch: Some(flags(LaunchThinking::Budget(8_000))),
            model: Some("claude-haiku-4-5"),
            ..live.clone()
        });
        assert_eq!(
            v.thinking,
            Some(ThinkingInfo {
                mode: ThinkingMode::On,
                source: ThinkingSource::Launch,
                budget: Some(8_000)
            })
        );
        let v = derive_settings(&SettingsFacts {
            launch: Some(flags(LaunchThinking::Off)),
            ..live.clone()
        });
        assert_eq!(v.thinking.map(|t| t.mode), Some(ThinkingMode::Off));

        // A resumed session: markers and effort from before this process
        // yield to what this process was launched with.
        let resumed = SettingsFacts {
            process_started: Some(10_000),
            launch: Some(LaunchFlags {
                effort: Some("low".into()),
                ultracode: Some(false),
                ..Default::default()
            }),
            ..live.clone()
        };
        let v = derive_settings(&resumed);
        assert_eq!(v.effort.as_deref(), Some("low"));
        assert_eq!(v.effort_source, Some(EffortSource::Launch));
        assert_eq!(v.ultracode, Some(UltracodeState::Off));
        assert_eq!(v.ultracode_source, Some(UltracodeSource::Launch));
        assert_eq!(v.ultracode_at, None);
        assert_eq!(
            (v.ultracode_unknown, v.ultracode_before),
            (None, None),
            "known at launch"
        );
        // Without launch flags the value from before the restart says
        // nothing now (a restart turns ultracode off; VS Code doesn't turn
        // it back on): unknown until the next message, with what it was.
        // The old effort is still the last one observed.
        let v = derive_settings(&SettingsFacts {
            launch: None,
            ..resumed.clone()
        });
        assert_eq!(v.ultracode, None);
        assert_eq!(v.ultracode_source, None);
        assert_eq!(v.ultracode_unknown, Some(UltracodeUnknown::Restarted));
        assert_eq!(v.ultracode_before, Some(UltracodeState::On));
        assert_eq!(v.ultracode_before_at, Some(1_500));
        assert_eq!(v.effort.as_deref(), Some("xhigh"));
        assert_eq!(v.effort_source, Some(EffortSource::Observed));
        // A message of this process decides it again — here one that wrote
        // no marker after the `enter` of the history: still on.
        let confirmed = Aggregates {
            ultra: ultra_as_of(Some(true), 12_000),
            ..agg.clone()
        };
        let v = derive_settings(&SettingsFacts {
            launch: None,
            agg: Some(&confirmed),
            ..resumed.clone()
        });
        assert_eq!(v.ultracode, Some(UltracodeState::On));
        assert_eq!(v.ultracode_source, Some(UltracodeSource::Transcript));
        assert_eq!(v.ultracode_at, Some(12_000));
        assert_eq!((v.ultracode_unknown, v.ultracode_before), (None, None));
        // …and it wins over the launch flags.
        let v = derive_settings(&SettingsFacts {
            agg: Some(&confirmed),
            ..resumed.clone()
        });
        assert_eq!(v.ultracode_source, Some(UltracodeSource::Transcript));
        // Within the start slack a message still counts.
        let v = derive_settings(&SettingsFacts {
            process_started: Some(6_000),
            ..resumed
        });
        assert_eq!(v.ultracode_source, Some(UltracodeSource::Transcript));
    }

    #[test]
    fn unknown_ultracode_says_why() {
        let derive = |agg: Option<&Aggregates>, live: bool, partial: bool| {
            derive_settings(&SettingsFacts {
                live,
                process_started: Some(500),
                agg,
                transcript_partial: partial,
                ..Default::default()
            })
        };
        // No message yet (a new VS Code conversation has no transcript).
        let v = derive(None, true, false);
        assert_eq!(v.ultracode, None);
        assert_eq!(v.ultracode_unknown, Some(UltracodeUnknown::NoPrompt));
        let empty = Aggregates::default();
        assert_eq!(
            derive(Some(&empty), false, false).ultracode_unknown,
            Some(UltracodeUnknown::NoPrompt)
        );
        // A transcript of a version that notes nothing.
        let old = Aggregates {
            version: Some("2.1.200".into()),
            prompts: 3,
            ..Default::default()
        };
        assert_eq!(
            derive(Some(&old), false, false).ultracode_unknown,
            Some(UltracodeUnknown::Unsupported)
        );
        // Not read yet.
        assert_eq!(
            derive(None, true, true).ultracode_unknown,
            Some(UltracodeUnknown::Reading)
        );
        assert_eq!(
            derive(Some(&empty), true, true).ultracode_unknown,
            Some(UltracodeUnknown::Reading)
        );
        // A manual `/compact` hid the marker.
        let compacted = Aggregates {
            ultra: ultra_as_of(None, 1_000),
            ..Default::default()
        };
        let v = derive(Some(&compacted), true, false);
        assert_eq!(v.ultracode, None);
        assert_eq!(v.ultracode_unknown, Some(UltracodeUnknown::Compacted));
        // An exit (or a message confirming off) is off, not unknown.
        let off = Aggregates {
            ultra: ultra_as_of(Some(false), 1_000),
            ..Default::default()
        };
        let v = derive(Some(&off), true, false);
        assert_eq!(v.ultracode, Some(UltracodeState::Off));
        assert_eq!(v.ultracode_at, Some(1_000));
        assert_eq!(v.ultracode_unknown, None);
        // An ended session keeps its last value, however old.
        let v = derive_settings(&SettingsFacts {
            agg: Some(&off),
            process_started: Some(99_000),
            ..Default::default()
        });
        assert_eq!(v.ultracode, Some(UltracodeState::Off));
    }

    #[test]
    fn a_live_session_shows_this_process_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let home = crate::claude::ClaudeHome::with_base(tmp.path());
        let sid = "5f0b9a1e-2c3d-4d4c-9d59-2d2e9c5e3a52";
        let me = std::process::id();
        let now = chrono::Utc::now().timestamp_millis();
        let iso = |ms: i64| {
            chrono::DateTime::from_timestamp_millis(ms)
                .unwrap()
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        };
        let proj = tmp.path().join("projects/-p");
        std::fs::create_dir_all(&proj).unwrap();
        // A message sent from VS Code, and the reply that follows its
        // attachments.
        let prompt = |ms: i64| {
            format!(
                r#"{{"type":"user","timestamp":"{}","version":"2.1.284","origin":{{"kind":"human"}},"promptSource":"sdk","message":{{"role":"user","content":"go on"}}}}"#,
                iso(ms)
            )
        };
        let reply = |ms: i64| {
            format!(
                r#"{{"type":"assistant","timestamp":"{}","effort":"xhigh","message":{{"id":"r{ms}","model":"claude-opus-5-5","content":[{{"type":"text","text":"ok"}}],"usage":{{"input_tokens":1,"output_tokens":1,"speed":"fast"}}}}}}"#,
                iso(ms)
            )
        };
        let exit = |ms: i64| {
            format!(
                r#"{{"type":"attachment","timestamp":"{}","attachment":{{"type":"ultra_effort_exit"}}}}"#,
                iso(ms)
            )
        };
        let transcript = |marker_ms: i64, after: &[String]| {
            [
                format!(
                    r#"{{"type":"assistant","timestamp":"{}","effort":"xhigh","message":{{"id":"m1","model":"claude-opus-5-5","content":[{{"type":"thinking","thinking":""}},{{"type":"text","text":"hi"}}],"usage":{{"input_tokens":1,"output_tokens":1,"speed":"fast"}}}}}}"#,
                    iso(now - 120_000)
                ),
                format!(
                    r#"{{"type":"attachment","timestamp":"{}","attachment":{{"type":"ultra_effort_enter","reminderType":"sparse"}}}}"#,
                    iso(marker_ms)
                ),
            ]
            .iter()
            .chain(after)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
                + "\n"
        };
        let load = |started: i64, session: &str, text: Option<String>| {
            if let Some(text) = text {
                std::fs::write(proj.join(format!("{session}.jsonl")), text).unwrap();
            }
            std::fs::create_dir_all(tmp.path().join("sessions")).unwrap();
            std::fs::write(
                tmp.path().join(format!("sessions/{me}.json")),
                format!(
                    r#"{{"pid":{me},"sessionId":"{session}","cwd":"/p","startedAt":{started},"status":"idle"}}"#
                ),
            )
            .unwrap();
            let state = crate::state::Inner::new(
                home.clone(),
                crate::state::ServerConfig {
                    host: "127.0.0.1".into(),
                    port: 0,
                },
                crate::server::auth::Auth::ephemeral(),
            );
            state.live.refresh(&home);
            state.index.discover(&home);
            state.index.queue_all();
            state.index.drain_jobs_blocking();
            state
        };
        let ctx = BuildCtx {
            now,
            ..Default::default()
        };
        // The marker was written by this process.
        let state = load(now - 60_000, sid, Some(transcript(now - 30_000, &[])));
        let s = build_one(&state, sid, &ctx).unwrap();
        assert!(s.live);
        assert_eq!(s.effort.as_deref(), Some("xhigh"));
        assert_eq!(s.effort_source, Some(EffortSource::Observed));
        assert_eq!(s.ultracode, Some(UltracodeState::On));
        assert_eq!(s.ultracode_source, Some(UltracodeSource::Transcript));
        assert_eq!(s.ultracode_at, Some(now - 30_000));
        assert_eq!(s.ultracode_unknown, None);
        assert_eq!(s.fast, Some(true));
        assert_eq!(s.last_thinking_at, Some(now - 120_000));
        assert_eq!(
            s.thinking.map(|t| (t.mode, t.source)),
            Some((ThinkingMode::Adaptive, ThinkingSource::Default))
        );
        // A hook report newer than the transcript wins.
        state.activity.note_effort(sid, "max");
        let s = build_one(&state, sid, &ctx).unwrap();
        assert_eq!(s.effort.as_deref(), Some("max"));
        // Resumed by a newer process, no message since: what it was before
        // the restart, not what it is.
        let state = load(now - 10_000, sid, Some(transcript(now - 30_000, &[])));
        let s = build_one(&state, sid, &ctx).unwrap();
        assert_eq!(s.ultracode, None);
        assert_eq!(s.ultracode_at, None);
        assert_eq!(s.ultracode_unknown, Some(UltracodeUnknown::Restarted));
        assert_eq!(s.ultracode_before, Some(UltracodeState::On));
        assert_eq!(s.ultracode_before_at, Some(now - 30_000));
        // A message after the restart that wrote no marker: switched back on
        // (Claude Code sees the `enter` of its history and notes nothing).
        let after = [prompt(now - 5_000), reply(now - 4_000)];
        let state = load(now - 10_000, sid, Some(transcript(now - 30_000, &after)));
        let s = build_one(&state, sid, &ctx).unwrap();
        assert_eq!(s.ultracode, Some(UltracodeState::On));
        assert_eq!(s.ultracode_source, Some(UltracodeSource::Transcript));
        assert_eq!(s.ultracode_at, Some(now - 5_000));
        assert_eq!(
            (s.ultracode_unknown, s.ultracode_before),
            (None, None),
            "known again"
        );
        // One that got an `exit`: left off after the restart.
        let after = [prompt(now - 5_000), exit(now - 5_000), reply(now - 4_000)];
        let state = load(now - 10_000, sid, Some(transcript(now - 30_000, &after)));
        let s = build_one(&state, sid, &ctx).unwrap();
        assert_eq!(s.ultracode, Some(UltracodeState::Off));
        assert_eq!(s.ultracode_at, Some(now - 5_000));
        // The contract names.
        let v = serde_json::to_value(build_one(
            &load(now - 60_000, sid, Some(transcript(now - 30_000, &[]))),
            sid,
            &ctx,
        ))
        .unwrap();
        assert_eq!(v["effortSource"], "observed");
        assert_eq!(v["ultracode"], "on");
        assert_eq!(v["ultracodeSource"], "transcript");
        assert_eq!(v["ultracodeAt"], now - 30_000);
        assert!(v.get("ultracodeUnknown").is_none());
        assert_eq!(v["thinking"]["mode"], "adaptive");
        assert_eq!(v["thinking"]["source"], "default");
        assert!(v["thinking"].get("budget").is_none());
        assert_eq!(v["lastThinkingAt"], now - 120_000);
        assert_eq!(v["fast"], true);
        assert_eq!(v["control"]["setUltracode"], false);
        assert_eq!(v["control"]["setThinking"], false);
        // A new conversation: no transcript until the first message.
        let fresh = "7c2d0b3e-4f5a-4b6c-8d7e-9f0a1b2c3d4e";
        let state = load(now - 10_000, fresh, None);
        let s = build_one(&state, fresh, &ctx).unwrap();
        assert!(s.live);
        assert_eq!(s.ultracode, None);
        assert_eq!(s.ultracode_unknown, Some(UltracodeUnknown::NoPrompt));
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["ultracodeUnknown"], "no-prompt");
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
