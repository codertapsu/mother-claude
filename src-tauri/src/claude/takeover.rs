//! Taking over a session an earlier run of Mother Claude started.
//!
//! Mother Claude drives the sessions it runs through their stdin and stdout
//! (`claude/control.rs`). When it goes away without stopping them — an
//! update that replaces it, a crash — a session busy with a turn or with
//! background work outlives it: its process goes on and writes its
//! transcript, but its pipes went with the old run, so nothing reaches it
//! anymore, and it still holds the conversation (never two writers on one
//! transcript). Such a process is known by what Mother Claude gives every
//! session it runs (`LaunchFlags::mother_claude`: `MOTHER_CLAUDE_OWNED=1` in
//! its environment, its own plugin folder in `--plugin-dir`) while no control
//! handle here has it (`ControlCaps::take_over`).
//!
//! Taking it over stops that process and everything it started (its
//! background shells, its MCP servers), waits until it is gone, then resumes
//! the same session here in place (`--resume <id>`, never a fork) with the
//! model, effort, thinking and permission mode it was launched with. A first
//! message, when one is given, is told what stopped.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use super::live::{self, LaunchThinking};
use super::model::{ImageInput, SessionStatus};
use super::owned::{self, Caller, ContinueRequest};
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// How long the old process gets to be gone after it is told to stop.
const GONE_WAIT: Duration = Duration::from_secs(10);

/// What stopped with a taken-over session's old process, until its first
/// message (taken over with none): that message says so.
static NOTES: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Default::default);

fn notes() -> std::sync::MutexGuard<'static, HashMap<String, String>> {
    NOTES.lock().unwrap_or_else(|p| p.into_inner())
}

/// The note session `id`'s next message carries, once (`owned::send_as`).
pub fn take_note(id: &str) -> Option<String> {
    notes().remove(id)
}

/// What a take-over did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TookOver {
    /// The session, going on here (the same id).
    pub id: String,
    /// What stopped with the old process: its turn, or its background work.
    pub stopped: Option<String>,
}

/// Take session `id` over: stop the process an earlier run of Mother Claude
/// started for it, and resume the same session here. `prompt` (and
/// `images`): its first message, if any.
pub async fn take_over(
    state: &AppState,
    caller: Caller,
    id: &str,
    prompt: Option<String>,
    images: Vec<ImageInput>,
) -> ApiResult<TookOver> {
    super::ids::require_session_id(id)?;
    if owned::has_handle(state, id) || state.bridge.thread(id).await.is_some() {
        return Err(ApiError::conflict(
            "already_owned",
            "Mother Claude runs this session already.",
        ));
    }
    let (Some(rec), Some(flags)) = (state.live.get(id), state.live.launch_flags(id)) else {
        return Err(not_running());
    };
    let Some(pid) = rec.pid else {
        return Err(not_running());
    };
    if !flags.mother_claude {
        return Err(not_ours());
    }
    // Its own mode again: one without approvals needs the dangerous right,
    // asked before anything stops.
    owned::launch_mode(flags.permission_mode.as_deref(), caller)?;
    let summary = state.find_session(id).await;
    let cwd = summary
        .as_ref()
        .map(|s| s.cwd.clone())
        .filter(|c| !c.is_empty())
        .or_else(|| rec.cwd.clone())
        .filter(|c| !c.is_empty())
        .ok_or_else(not_running)?;
    let stopped = summary.as_ref().and_then(|s| match s.status {
        SessionStatus::Working | SessionStatus::Starting | SessionStatus::Waiting => {
            Some("the turn it was working on".to_string())
        }
        SessionStatus::Background => Some(
            s.activity
                .detail
                .clone()
                .filter(|d| !d.trim().is_empty())
                .unwrap_or_else(|| "its background work".to_string()),
        ),
        _ => None,
    });

    // Still the same process, still ours: then it stops, with what it runs.
    if !live::process_facts(pid).mother_claude
        && !live::process_argv(pid)
            .map(|a| live::launch_flags_from_args(&a))
            .is_some_and(|f| f.mother_claude)
    {
        return Err(not_ours());
    }
    let killed = tokio::task::spawn_blocking(move || kill_tree(pid))
        .await
        .unwrap_or(0);
    tracing::info!(session = %id, pid, killed, "taking a session over: its old process stopped");
    let deadline = Instant::now() + GONE_WAIT;
    while live::pid_exists(pid) {
        if Instant::now() > deadline {
            return Err(ApiError::new(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "take_over_failed",
                "The session's old process did not stop in time, so it was not taken over.",
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // The session list without it, so the resume is in place, not a fork.
    for _ in 0..20 {
        crate::server::monitor::tick_once(state).await;
        let gone =
            state.live.get(id).is_none() && !state.find_session(id).await.is_some_and(|s| s.live);
        if gone {
            break;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    if state.live.get(id).is_some() || state.find_session(id).await.is_some_and(|s| s.live) {
        return Err(ApiError::new(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "take_over_failed",
            "The session still shows as running, so it was not taken over: try again.",
        ));
    }

    let prompt = prompt.filter(|p| !p.trim().is_empty());
    let first = prompt
        .as_deref()
        .map(|p| first_message(p, stopped.as_deref()));
    let new_id = owned::continue_as(
        state,
        caller,
        id,
        &cwd,
        ContinueRequest {
            fork: Some(false),
            prompt: first,
            images,
            model: flags.model.clone(),
            effort: flags.effort.clone(),
            permission_mode: flags.permission_mode.clone(),
            thinking: flags.thinking.map(|t| {
                match t {
                    LaunchThinking::Off => "off",
                    LaunchThinking::Adaptive | LaunchThinking::Budget(_) => "on",
                }
                .to_string()
            }),
            ..Default::default()
        },
    )
    .await?;
    // No message yet: the first one says what stopped.
    if prompt.is_none() {
        if let Some(what) = &stopped {
            notes().insert(new_id.clone(), what.clone());
        }
    }
    Ok(TookOver {
        id: new_id,
        stopped,
    })
}

/// The first message after a take-over: what stopped, then the person's.
pub fn first_message(prompt: &str, stopped: Option<&str>) -> String {
    match stopped {
        Some(what) => format!(
            "(Mother Claude took this session over after it restarted: the session's earlier \
             process was stopped, and with it: {what}. Start that again if it is still \
             needed.)\n\n{prompt}"
        ),
        None => prompt.to_string(),
    }
}

/// Stop process `pid` and every process it started (its children's
/// children too). How many were told to stop.
pub fn kill_tree(pid: u32) -> usize {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let Ok(mut sys) = System::new() else {
        tracing::warn!(pid, "processes cannot be read: nothing stopped");
        return 0;
    };
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let root = Pid::from_u32(pid);
    if sys.process(root).is_none() {
        return 0;
    }
    let mut tree = vec![root];
    let mut i = 0;
    while i < tree.len() {
        let parent = tree[i];
        let children: Vec<Pid> = sys
            .processes()
            .iter()
            .filter(|(child, p)| p.parent() == Some(parent) && !tree.contains(child))
            .map(|(child, _)| *child)
            .collect();
        tree.extend(children);
        i += 1;
    }
    // The root first, so it starts nothing new; then what it started.
    tree.iter()
        .filter(|p| sys.process(**p).is_some_and(|proc_| proc_.kill()))
        .count()
}

fn not_running() -> ApiError {
    ApiError::conflict(
        "not_running",
        "This session isn't running: continue it instead.",
    )
}

fn not_ours() -> ApiError {
    ApiError::conflict(
        "not_ours",
        "Mother Claude didn't start this session's process, so it won't stop it: continue it \
         here as a new session instead.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_message_says_what_stopped() {
        assert_eq!(first_message("go on", None), "go on");
        let m = first_message(
            "go on",
            Some("Wait in the background for the 8 new downloads"),
        );
        assert!(m.starts_with("(Mother Claude took this session over after it restarted"));
        assert!(m.contains("and with it: Wait in the background for the 8 new downloads."));
        assert!(m.ends_with("\n\ngo on"));
    }

    #[test]
    fn a_tree_is_stopped_whole() {
        // A child that starts a grandchild, both long-lived.
        #[cfg(windows)]
        let mut child = crate::proc::std_command("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "Start-Process -NoNewWindow powershell -ArgumentList '-NoProfile','-Command','Start-Sleep 60'; Start-Sleep 60",
            ])
            .spawn()
            .unwrap();
        #[cfg(unix)]
        let mut child = crate::proc::std_command("sh")
            .args(["-c", "sleep 60 & sleep 60"])
            .spawn()
            .unwrap();
        let pid = child.id();
        // Its child shows up.
        let start = Instant::now();
        let kids = || {
            use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
            let mut sys = System::new().unwrap();
            sys.refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                ProcessRefreshKind::nothing(),
            );
            sys.processes()
                .iter()
                .filter(|(_, p)| p.parent() == Some(Pid::from_u32(pid)))
                .map(|(c, _)| c.as_u32())
                .collect::<Vec<_>>()
        };
        let mut grandchildren = kids();
        while grandchildren.is_empty() && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(100));
            grandchildren = kids();
        }
        assert!(!grandchildren.is_empty(), "the child started its own");
        assert!(kill_tree(pid) >= 2);
        let _ = child.wait();
        let deadline = Instant::now() + Duration::from_secs(10);
        while grandchildren.iter().any(|g| live::pid_exists(*g)) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            grandchildren.iter().all(|g| !live::pid_exists(*g)),
            "what it started stopped too"
        );
        assert_eq!(kill_tree(pid), 0, "nothing left");
    }
}
