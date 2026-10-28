//! Owned-session control facade (design §5.1, §10 B3).
//!
//! Every route that drives an owned session goes through these functions; the
//! machinery (Path B stream-json control protocol, Path A sidecar) lives in
//! `claude/control.rs`. This layer adds the policy:
//!
//! * **Launch rules.** Path B always gets an explicit `--permission-mode`
//!   (omitted ⇒ `default`; a mode is never inherited on resume or fork).
//!   Bypass-class modes and `--allow-dangerously-skip-permissions` need a
//!   caller allowed dangerous actions; if a session still comes up in
//!   bypassPermissions for a caller without that right, it is stopped.
//! * **Trusted working directories.** Headless runs show no trust dialog, so
//!   an untrusted repo's hooks and `.mcp.json` would run at spawn. Callers
//!   other than the desktop may only spawn/continue in a directory the user
//!   already trusted (`~/.claude.json` `hasTrustDialogAccepted`, inherited
//!   from a parent, or one with transcripts under `projects/`).
//! * **Takeover = fork when live elsewhere.** Continuing a session that
//!   another process holds forks it under a new id (`--resume <id>
//!   --fork-session --session-id <new>`); an ended one resumes in place.
//! * **Images:** ≤ 10 per message, ≤ 5 MB decoded each, png/jpeg/gif/webp
//!   verified by magic bytes.
//! * **Background sessions** are driven by their SHORT job id only.
//! * **Session settings** change through control requests only —
//!   `apply_flag_settings {effortLevel, ultracode}` (a session-only flag
//!   layer; ultracode rides along with an effort change) and
//!   `set_max_thinking_tokens` — never a slash command (`/effort` persists to
//!   the user's settings). What the CLI then applies is read back with
//!   `get_settings` and overlaid on the summary ([`overlay_settings`]).
//! * CLI `error` replies become `422 cli_error` with the CLI's message.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::control::{
    self, ControlError, Driver, OwnedLive, OwnedThinking, SpawnOptions, REQUEST_TIMEOUT,
    THINKING_ON_BUDGET,
};
use super::model::{
    AgentInfo, BackgroundTask, CommandInfo, ControlChannel, Delivery, EffortSource, ImageInput,
    MessageReceipt, ModelInfo, QueuedMessage, SessionSummary, ThinkingInfo, ThinkingMode,
    ThinkingSource, UltracodeSource, UltracodeState,
};
use crate::server::context::ClientContext;
use crate::server::error::{ApiError, ApiResult};
use crate::server::ws;
use crate::state::AppState;

/// Effort levels the CLI accepts.
pub const EFFORT_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// Permission modes a spawn/continue/mode change may request.
pub const PERMISSION_MODES: [&str; 6] = [
    "default",
    "acceptEdits",
    "plan",
    "auto",
    "bypassPermissions",
    "dontAsk",
];

/// Modes any caller may launch or switch to (the rest are dangerous).
pub const SAFE_MODES: [&str; 4] = ["default", "acceptEdits", "plan", "auto"];

/// Image limits for messages and spawns.
pub const MAX_IMAGES: usize = 10;
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
pub const IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/// Longest message text accepted for an owned session.
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;

/// `POST /api/sessions` body.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SpawnRequest {
    pub cwd: String,
    pub prompt: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<String>,
    /// Legacy `"on"`/`"off"` thinking override.
    pub thinking: Option<String>,
    pub images: Vec<ImageInput>,
    /// The account to run under (default: the project's, else the default
    /// for new sessions; docs/ACCOUNTS.md §5.4).
    pub account: Option<String>,
}

/// `POST /api/sessions/{id}/continue` body.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ContinueRequest {
    pub fork: Option<bool>,
    pub prompt: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<String>,
    pub thinking: Option<String>,
    /// Images on the first message of the continued session.
    pub images: Vec<ImageInput>,
    /// Continue under this account ("continue on B"): another account than
    /// the session's always forks (docs/ACCOUNTS.md §5.4).
    pub account: Option<String>,
}

/// Who is asking, for the launch policy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Caller {
    /// May use dangerous actions (L): bypass modes, the bypass flag.
    pub may_dangerous: bool,
    /// The desktop itself (D): free working directories.
    pub desktop: bool,
}

impl Caller {
    pub fn of(ctx: &ClientContext) -> Self {
        Self {
            may_dangerous: ctx.may_dangerous(),
            desktop: ctx.is_desktop(),
        }
    }
}

/// Whether a permission mode removes the human from the loop (bypass-class ⇒
/// the caller needs `ClientContext::may_dangerous`).
pub fn is_dangerous_mode(mode: Option<&str>) -> bool {
    mode.is_some_and(|m| crate::pending::UNGATED_MODES.contains(&m))
}

/// Reject unknown effort / thinking / permission-mode values.
pub fn validate_launch(
    effort: Option<&str>,
    thinking: Option<&str>,
    permission_mode: Option<&str>,
) -> ApiResult<()> {
    if let Some(e) = effort {
        if !EFFORT_LEVELS.contains(&e) {
            return Err(ApiError::unprocessable(
                "invalid_effort",
                format!("Unknown effort level {e:?} (expected one of {EFFORT_LEVELS:?})."),
            ));
        }
    }
    if let Some(t) = thinking {
        if t != "on" && t != "off" {
            return Err(ApiError::unprocessable(
                "invalid_thinking",
                format!("Unknown thinking value {t:?} (expected \"on\" or \"off\")."),
            ));
        }
    }
    if let Some(m) = permission_mode {
        if !PERMISSION_MODES.contains(&m) {
            return Err(ApiError::unprocessable(
                "invalid_permission_mode",
                format!("Unknown permission mode {m:?} (expected one of {PERMISSION_MODES:?})."),
            ));
        }
    }
    Ok(())
}

/// A model name that is safe as a CLI argument: `opus`, `claude-opus-5-5`,
/// `claude-sonnet-5[1m]`, `us.anthropic.claude-…:0`, ….
pub fn validate_model(model: &str) -> ApiResult<String> {
    let m = model.trim();
    let ok = !m.is_empty()
        && m.len() <= 128
        && !m.starts_with('-')
        && m.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/' | '[' | ']' | '@')
        });
    if ok {
        Ok(m.to_string())
    } else {
        Err(ApiError::unprocessable("invalid_model", "Name a model."))
    }
}

/// A canonical lowercase UUID (session ids, message ids).
pub fn is_uuid(id: &str) -> bool {
    super::ids::is_uuid(id)
}

/// A task / agent / tool-use id: `^[A-Za-z0-9_-]{1,80}$`.
pub fn is_simple_id(id: &str) -> bool {
    super::ids::is_simple_id(id)
}

/// Validate images: count, type, base64, size, magic bytes. A `data:` URL
/// prefix is tolerated and stripped.
pub fn validate_images(images: &[ImageInput]) -> ApiResult<Vec<ImageInput>> {
    if images.len() > MAX_IMAGES {
        return Err(ApiError::unprocessable(
            "too_many_images",
            format!("At most {MAX_IMAGES} images per message."),
        ));
    }
    images
        .iter()
        .enumerate()
        .map(|(i, img)| {
            let media_type = img.media_type.trim().to_ascii_lowercase();
            if !IMAGE_TYPES.contains(&media_type.as_str()) {
                return Err(ApiError::unprocessable(
                    "unsupported_image_type",
                    format!("Image {i}: {media_type:?} is not png, jpeg, gif or webp."),
                ));
            }
            let data = img
                .data
                .split_once(";base64,")
                .map(|(_, d)| d)
                .unwrap_or(&img.data);
            let data: String = data.chars().filter(|c| !c.is_ascii_whitespace()).collect();
            if data.len() > MAX_IMAGE_BYTES / 3 * 4 + 8 {
                return Err(ApiError::unprocessable(
                    "image_too_large",
                    format!("Image {i} is larger than 5 MB."),
                ));
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&data)
                .map_err(|_| {
                    ApiError::unprocessable("invalid_image", format!("Image {i} is not base64."))
                })?;
            if bytes.len() > MAX_IMAGE_BYTES {
                return Err(ApiError::unprocessable(
                    "image_too_large",
                    format!("Image {i} is larger than 5 MB."),
                ));
            }
            if sniff_image(&bytes) != Some(media_type.as_str()) {
                return Err(ApiError::unprocessable(
                    "invalid_image",
                    format!("Image {i} is not a valid {media_type}."),
                ));
            }
            Ok(ImageInput { media_type, data })
        })
        .collect()
}

/// The image type the magic bytes say.
pub fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// Owned by Mother Claude in any way — a Path A/B control handle, or an HTTP
/// bridge conversation (the hook short-circuit uses this). Sync.
pub fn is_owned(state: &AppState, id: &str) -> bool {
    state.control.is_owned(id)
}

/// Mother Claude holds a live control handle (Path A or B) for `id`.
pub fn has_handle(state: &AppState, id: &str) -> bool {
    state.control.controls(id)
}

/// What the control protocol reported about an owned session (model, mode,
/// activity, queue, tasks, …). The data layer overlays this on summaries.
pub fn live_state(state: &AppState, id: &str) -> Option<OwnedLive> {
    state.control.state(id)
}

/// Queued (not yet answered) messages of an owned session.
pub fn queued(state: &AppState, id: &str) -> Vec<QueuedMessage> {
    live_state(state, id).map(|l| l.queued).unwrap_or_default()
}

/// Subagents and background tasks an owned session reported.
pub fn tasks(state: &AppState, id: &str) -> (Vec<AgentInfo>, Vec<BackgroundTask>) {
    live_state(state, id)
        .map(|l| (l.agents_of(id), l.background_tasks()))
        .unwrap_or_default()
}

/// Apply what the control protocol knows to a summary built from disk (the
/// data layer calls this so a refresh never overwrites live owned state).
pub fn overlay(state: &AppState, summary: &mut SessionSummary) {
    let Some(live) = live_state(state, &summary.id) else {
        return;
    };
    let always_thinking = super::settings::always_thinking_enabled(
        &super::summaries::session_home(state, &summary.id),
    );
    overlay_settings(summary, &live, always_thinking);
    if let Some(mode) = live.permission_mode {
        summary.permission_mode = Some(mode);
    }
    if let Some(model) = live.model {
        if summary.model.as_ref().map(|m| m.id.as_str()) != Some(model.as_str()) {
            summary.model = Some(ModelInfo {
                id: model.clone(),
                label: model,
                context_window: None,
            });
        }
    }
    summary.queued = summary.queued.max(live.queued.len() as u32);
    if live.last_error.is_some() && summary.last_error.is_none() {
        summary.last_error = live.last_error;
    }
    if !summary.pending.iter().any(|p| p.answerable) && live.started {
        summary.activity = live.activity;
    }
}

/// The model an owned session runs: what `get_settings` resolved (unless a
/// model change is not confirmed yet), else the stream's report or what we
/// switched to, else the summary's.
fn current_model<'a>(summary: &'a SessionSummary, live: &'a OwnedLive) -> Option<&'a str> {
    live.applied_model()
        .and_then(|a| a.model.as_deref())
        .or(live.model.as_deref())
        .or(live.applied.as_ref().and_then(|a| a.model.as_deref()))
        .or(summary.model.as_ref().map(|m| m.id.as_str()))
}

/// Effort, ultracode, thinking, fast mode and the matching switches of an
/// owned session, from what its control protocol reported (design2 §1):
///
/// * effort: `get_settings.applied.effort` — a null there means the model
///   has no effort levels (effort absent, source `applied`); until the CLI
///   confirms a change, what we requested; else the transcript's (already
///   set);
/// * ultracode: `applied.ultracode` ⇒ on, requested but not available ⇒
///   unavailable, else off; a switch not confirmed yet shows as `requested`;
/// * thinking: what we set (spawn option or switch) — adaptive on
///   adaptive-capable models, else on with the budget — or the CLI default;
/// * fast: `fast_mode_state == "on"`;
/// * the ultracode switch whenever it has something to do: the CLI can run
///   it, or it is requested (so it can be withdrawn).
pub fn overlay_settings(
    summary: &mut SessionSummary,
    live: &OwnedLive,
    always_thinking: Option<bool>,
) {
    if let Some(a) = live.applied_effort() {
        summary.effort = a.effort.clone();
        summary.effort_source = Some(EffortSource::Applied);
    } else if let Some(e) = &live.effort {
        summary.effort = Some(e.clone());
        summary.effort_source = Some(EffortSource::Requested);
    } else if live.applied.is_some() {
        // A model change the CLI has not confirmed: its effort is unknown.
        summary.effort = None;
        summary.effort_source = None;
    }
    let ultracode = if let Some(a) = live.applied_ultracode() {
        Some((
            if a.ultracode {
                UltracodeState::On
            } else if a.ultracode_requested {
                UltracodeState::Unavailable
            } else {
                UltracodeState::Off
            },
            UltracodeSource::Applied,
        ))
    } else if let Some(on) = live.ultracode_request {
        let state = if on {
            UltracodeState::On
        } else {
            UltracodeState::Off
        };
        Some((state, UltracodeSource::Requested))
    } else {
        // A model change the CLI has not confirmed: the request stands, but
        // whether the new model can run it is not known yet.
        live.applied.as_ref().map(|a| {
            if a.ultracode_requested {
                (UltracodeState::On, UltracodeSource::Requested)
            } else {
                (UltracodeState::Off, UltracodeSource::Applied)
            }
        })
    };
    if let Some((state, source)) = ultracode {
        summary.ultracode = Some(state);
        summary.ultracode_source = Some(source);
        summary.ultracode_at = None;
        summary.ultracode_unknown = None;
        summary.ultracode_before = None;
        summary.ultracode_before_at = None;
    }
    let adaptive = current_model(summary, live).and_then(|m| {
        control::catalog_adaptive_thinking(&live.models, m)
            .or_else(|| super::pricing::supports_adaptive_thinking(m))
    });
    summary.thinking = match live.thinking {
        Some(OwnedThinking::Off) => Some(ThinkingInfo {
            mode: ThinkingMode::Off,
            source: ThinkingSource::Control,
            budget: None,
        }),
        Some(OwnedThinking::On(budget)) => Some(match adaptive {
            Some(true) => ThinkingInfo {
                mode: ThinkingMode::Adaptive,
                source: ThinkingSource::Control,
                budget: None,
            },
            _ => ThinkingInfo {
                mode: ThinkingMode::On,
                source: ThinkingSource::Control,
                budget: Some(budget),
            },
        }),
        None => super::summaries::default_thinking(always_thinking, adaptive),
    };
    if live.fast_mode_state.as_deref() == Some("on") {
        summary.fast = Some(true);
    }
    if summary.live && summary.control.channel == ControlChannel::Owned {
        let (ultracode, thinking) = super::caps::owned_switches(
            live.driver == Driver::Sidecar,
            live.ultracode_switchable(),
        );
        summary.control.set_ultracode = ultracode;
        summary.control.set_thinking = thinking;
    }
}

/// Publish an owned session's settings overlay on its summary now (after a
/// `get_settings` answer or a settings change).
pub async fn publish_settings(state: &AppState, id: &str) {
    let Some(live) = live_state(state, id) else {
        return;
    };
    let always_thinking =
        super::settings::always_thinking_enabled(&super::summaries::session_home(state, id));
    ws::patch_session(state, id, move |s| {
        overlay_settings(s, &live, always_thinking)
    })
    .await;
}

/// The effective permission mode of any session, for the "no unapproved
/// execution for callers without dangerous rights" gates: the owned
/// session's live mode, a bridge conversation's mode, else the data layer's
/// effective mode (hook / owned notes, transcript records, pidfile argv),
/// else the summary's.
pub async fn effective_permission_mode(state: &AppState, id: &str) -> Option<String> {
    if let Some(mode) = live_state(state, id).and_then(|l| l.permission_mode) {
        return Some(mode);
    }
    if let Some(t) = state.bridge.thread(id).await {
        return Some(t.permission_mode);
    }
    if let Some(mode) = super::summaries::effective_permission_mode(state, id) {
        return Some(mode);
    }
    state.find_session(id).await.and_then(|s| s.permission_mode)
}

/// Whether messages into session `id` run tools with nobody approving them.
/// Fails closed: any source saying bypassPermissions is enough — the owned
/// session's live mode, a bridge conversation's record, and every report
/// [`crate::server::hooks::reported_bypass`] weighs (hook state, the newest
/// hook / owned / transcript note, the transcript's latest record, the
/// summary). Bypass is sticky across sources on purpose: a bridge record
/// alone went stale when an approval switched the conversation to bypass, and
/// one forged hook report must never lower the gate.
pub async fn runs_unapproved(state: &AppState, id: &str) -> bool {
    const BYPASS: &str = "bypassPermissions";
    let live = live_state(state, id).and_then(|l| l.permission_mode);
    let bridge = state.bridge.thread(id).await.map(|t| t.permission_mode);
    if [live, bridge].iter().flatten().any(|m| m == BYPASS) {
        return true;
    }
    let summary_mode = state.find_session(id).await.and_then(|s| s.permission_mode);
    crate::server::hooks::reported_bypass(state, id, summary_mode.as_deref()).unwrap_or(false)
}

/// The permission mode a launch runs in: always explicit; callers without
/// dangerous rights get `default|acceptEdits|plan|auto` only.
pub(crate) fn launch_mode(requested: Option<&str>, caller: Caller) -> ApiResult<String> {
    let mode = requested.unwrap_or("default").to_string();
    if !SAFE_MODES.contains(&mode.as_str()) && !caller.may_dangerous {
        return Err(ApiError::dangerous_blocked(
            "Starting a session without permission prompts is only possible from this \
             computer or a device allowed dangerous actions.",
        ));
    }
    Ok(mode)
}

/// "Continue on" a directory account: copy the conversation into its
/// directory first.
pub(crate) async fn copy_for_continue(
    state: &AppState,
    from: &str,
    to: &str,
    id: &str,
) -> ApiResult<()> {
    use crate::claude::transfer::ops::{self, ContinueCopyError};
    match ops::copy_for_continue(state, from, to, id).await {
        Ok(_) => Ok(()),
        Err(ContinueCopyError::ConsentRequired) => Err(ApiError::conflict(
            "copy_consent_required",
            "That account keeps its own directory: the conversation has to be copied there \
             first. Allow copies between these two accounts (on the desktop), then try again.",
        )
        .with("from", from.to_string())
        .with("to", to.to_string())),
        Err(ContinueCopyError::Transfer(e)) => {
            Err(crate::server::routes::transfers::transfer_error(e))
        }
    }
}

/// Map an account refusal onto the API envelope.
pub fn account_error(e: crate::claude::accounts::AccountError) -> ApiError {
    use crate::claude::accounts::AccountError as E;
    match &e {
        E::NotFound(_) => ApiError::unprocessable("unknown_account", e.to_string()),
        E::TokenMissing => ApiError::conflict(e.code(), e.to_string()),
        _ => ApiError::unprocessable(e.code(), e.to_string()),
    }
}

/// The account a launch in `cwd` runs under: the one asked for, else the
/// project's, else the default for new sessions — with its environment.
fn launch_account(
    state: &AppState,
    asked: Option<&str>,
    cwd: &str,
) -> ApiResult<(String, crate::claude::accounts::AccountEnv)> {
    let id = match asked.map(str::trim).filter(|a| !a.is_empty()) {
        Some(a) => a.to_string(),
        None => state.accounts.account_for_cwd(cwd),
    };
    let env = state.accounts.env(&id).map_err(account_error)?;
    Ok((id, env))
}

/// A spawn/continue working directory: absolute and existing; for callers
/// other than the desktop, one the user already trusted. Returned as given
/// (not canonicalized): `--resume` finds a transcript by the cwd it was
/// recorded under, which may go through a symlink.
/// Trust is read from the config directory of the account it runs under.
pub(crate) fn launch_cwd(
    home: &crate::claude::ClaudeHome,
    raw: &str,
    caller: Caller,
) -> ApiResult<String> {
    let raw = raw.trim();
    let raw = match raw.trim_end_matches('/') {
        "" => raw,
        trimmed => trimmed,
    };
    if raw.is_empty() {
        return Err(ApiError::unprocessable("invalid_cwd", "`cwd` is required."));
    }
    let path = Path::new(raw);
    if !path.is_absolute() {
        return Err(ApiError::unprocessable(
            "invalid_cwd",
            "`cwd` must be an absolute path.",
        ));
    }
    let canonical = std::fs::canonicalize(path)
        .ok()
        .filter(|p| p.is_dir())
        .ok_or_else(|| {
            ApiError::unprocessable("invalid_cwd", format!("{raw} is not a directory."))
        })?;
    if !caller.desktop && !is_trusted_cwd(home, path, &canonical) {
        return Err(ApiError::forbidden(
            "untrusted_cwd",
            "Claude Code has not been trusted in this folder yet. Open it once on the Mac \
             (or start the session from the desktop) first.",
        ));
    }
    Ok(raw.to_string())
}

/// Whether the user already trusted `path` (as given, or canonical) in the
/// config directory of `home`.
fn is_trusted_cwd(home: &crate::claude::ClaudeHome, path: &Path, canonical: &Path) -> bool {
    let claude_json = home.claude_json();
    let projects = home.projects_dir();
    [path, canonical]
        .iter()
        .any(|p| trusted_in(&claude_json, &projects, p))
}

/// `~/.claude.json` `projects[<dir or an ancestor>].hasTrustDialogAccepted`,
/// or transcripts recorded for exactly this directory.
pub fn trusted_in(claude_json: &Path, projects_dir: &Path, dir: &Path) -> bool {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Global {
        projects: std::collections::HashMap<String, Project>,
    }
    #[derive(Deserialize, Default)]
    #[serde(default, rename_all = "camelCase")]
    struct Project {
        has_trust_dialog_accepted: Option<bool>,
    }
    let global: Global = std::fs::File::open(claude_json)
        .ok()
        .and_then(|f| serde_json::from_reader(std::io::BufReader::new(f)).ok())
        .unwrap_or_default();
    let mut cur: Option<&Path> = Some(dir);
    while let Some(d) = cur {
        let key = d.to_string_lossy();
        if global
            .projects
            .get(key.as_ref())
            .and_then(|p| p.has_trust_dialog_accepted)
            == Some(true)
        {
            return true;
        }
        cur = d.parent();
    }
    transcripts_recorded_in(projects_dir, dir)
}

/// Transcripts scanned per directory by [`transcripts_recorded_in`].
const TRUST_SCAN_FILES: usize = 200;
/// Lines read per transcript while looking for its `cwd`.
const TRUST_SCAN_LINES: usize = 200;

/// Whether Claude Code recorded a session in EXACTLY `dir`: a transcript
/// under `projects/<encoded dir>` whose records name `cwd == dir`. The
/// encoded name alone is not enough — `/x/app.v2`, `/x/app_v2` and
/// `/x/app v2` all encode like `/x/app-v2`, and trusting a freshly cloned
/// look-alike would run its project hooks and `.mcp.json` unasked.
pub fn transcripts_recorded_in(projects_dir: &Path, dir: &Path) -> bool {
    use std::io::BufRead as _;
    let wanted = dir.to_string_lossy();
    let wanted = match wanted.trim_end_matches('/') {
        "" => "/",
        w => w,
    };
    let recorded = projects_dir.join(crate::claude::encode_cwd(wanted));
    let Ok(entries) = std::fs::read_dir(recorded) else {
        return false;
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .take(TRUST_SCAN_FILES)
        .any(|e| {
            let Ok(file) = std::fs::File::open(e.path()) else {
                return false;
            };
            // The first record that names a cwd decides for this file.
            std::io::BufReader::new(file)
                .lines()
                .take(TRUST_SCAN_LINES)
                .map_while(Result::ok)
                .find_map(|line| {
                    let v: serde_json::Value = serde_json::from_str(&line).ok()?;
                    v.get("cwd")?.as_str().map(|c| {
                        let c = c.trim_end_matches('/');
                        (if c.is_empty() { "/" } else { c }) == wanted
                    })
                })
                .unwrap_or(false)
        })
}

pub(crate) fn spawn_error(e: anyhow::Error) -> ApiError {
    let msg = format!("{e:#}");
    if msg.contains("already controlled") {
        ApiError::conflict("already_owned", msg)
    } else {
        ApiError::unprocessable("spawn_failed", msg)
    }
}

/// Map a control failure onto the API error envelope (CLI refusals are 422
/// with the CLI's own message and `errorCode`).
pub fn control_error(e: ControlError) -> ApiError {
    match e {
        ControlError::Cli(cli) => {
            let mut err = ApiError::unprocessable("cli_error", cli.message);
            if let Some(code) = cli.code {
                err = err.with("errorCode", code);
            }
            err
        }
        ControlError::Timeout(_) => ApiError::new(
            axum::http::StatusCode::GATEWAY_TIMEOUT,
            "cli_timeout",
            e.to_string(),
        ),
        ControlError::NotSupported(msg) => ApiError::conflict("not_supported", msg),
        ControlError::Switching => switching_account(),
        ControlError::NotOwned(_) | ControlError::Closed | ControlError::Io(_) => {
            ApiError::conflict("owned_session_unavailable", e.to_string())
        }
    }
}

/// `409 switching_account`: the session is going on under another account
/// right now (docs/ACCOUNTS.md §15.1) — stopped, maybe moving, about to be
/// resumed — so nothing reaches it until that is done.
pub fn switching_account() -> ApiError {
    ApiError::conflict(
        "switching_account",
        "This session is switching to another account. Try again in a moment.",
    )
}

/// Spawn a new owned session as a caller without dangerous rights and with
/// desktop-level cwd freedom. Routes use [`spawn_as`].
pub async fn spawn(state: &AppState, req: SpawnRequest) -> ApiResult<String> {
    spawn_as(
        state,
        Caller {
            may_dangerous: false,
            desktop: true,
        },
        req,
    )
    .await
}

/// Spawn a new owned session on behalf of `caller`; returns its id.
pub async fn spawn_as(state: &AppState, caller: Caller, req: SpawnRequest) -> ApiResult<String> {
    validate_launch(
        req.effort.as_deref(),
        req.thinking.as_deref(),
        req.permission_mode.as_deref(),
    )?;
    let mode = launch_mode(req.permission_mode.as_deref(), caller)?;
    let (account, account_env) = launch_account(state, req.account.as_deref(), req.cwd.trim())?;
    let home = state
        .accounts
        .home(&account)
        .unwrap_or_else(|| state.home.clone());
    let cwd = launch_cwd(&home, &req.cwd, caller)?;
    let model = req.model.as_deref().map(validate_model).transpose()?;
    let images = validate_images(&req.images)?;
    let prompt = req.prompt.unwrap_or_default();
    if prompt.len() > MAX_TEXT_BYTES {
        return Err(ApiError::unprocessable(
            "message_too_long",
            "The prompt is too long.",
        ));
    }
    let opts = SpawnOptions {
        cwd,
        prompt,
        model,
        effort: req.effort,
        thinking: req.thinking,
        permission_mode: Some(mode),
        images,
        allow_bypass: caller.may_dangerous,
        caller_may_dangerous: caller.may_dangerous,
        account: Some(account),
        account_env,
        ..Default::default()
    };
    state.control.spawn(state, opts).await.map_err(spawn_error)
}

/// Continue session `id` as an owned session; returns the (possibly new) id.
/// Uses the conservative caller of [`spawn`]; routes use [`continue_as`].
pub async fn continue_session(
    state: &AppState,
    id: &str,
    cwd: &str,
    req: ContinueRequest,
) -> ApiResult<String> {
    continue_as(
        state,
        Caller {
            may_dangerous: false,
            desktop: true,
        },
        id,
        cwd,
        req,
    )
    .await
}

/// Whether another process holds session `id` right now.
pub(crate) async fn live_elsewhere(state: &AppState, id: &str) -> bool {
    if state.live.get(id).is_some() {
        return true;
    }
    state.find_session(id).await.is_some_and(|s| s.live)
}

/// Continue session `id` (running in `cwd`) on behalf of `caller`: fork when
/// asked or when it is live in another process, else resume in place.
pub async fn continue_as(
    state: &AppState,
    caller: Caller,
    id: &str,
    cwd: &str,
    req: ContinueRequest,
) -> ApiResult<String> {
    if !is_uuid(id) {
        return Err(ApiError::unprocessable(
            "invalid_session_id",
            "Only Claude Code sessions (UUID ids) can be continued.",
        ));
    }
    // Between its stop and its resume under another account nothing holds
    // it, and continuing it then would make two writers.
    if state.control.switches.is_underway(id) {
        return Err(switching_account());
    }
    // The session's own account; one that ran with a token no account holds
    // is the Default's for every decision below (its chip says otherwise).
    let shown = crate::claude::summaries::session_account(state, id);
    let source = crate::claude::summaries::acting_account(state, id);
    // "Continue on B" forks, so it may start from a conversation Mother Claude
    // drives right now (the usual case: it stopped at A's limit); the original
    // keeps running as it was. Continuing on the same account is a message.
    let other_account = req
        .account
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .is_some_and(|a| a != source);
    if !other_account && (has_handle(state, id) || state.bridge.thread(id).await.is_some()) {
        return Err(ApiError::conflict(
            "already_owned",
            "This session is already driven by Mother Claude.",
        ));
    }
    validate_launch(
        req.effort.as_deref(),
        req.thinking.as_deref(),
        req.permission_mode.as_deref(),
    )?;
    // Never inherit a mode on resume or fork: always explicit.
    let mode = launch_mode(req.permission_mode.as_deref(), caller)?;
    // The session's own account unless another one is asked for ("continue
    // on B"), which always forks: the original stays as it was. One that ran
    // with a token no account holds (or under a removed account) continues
    // under the project's account (or the default) unless one is named.
    let fallback = if shown == source && state.accounts.is_usable(&source) {
        source.clone()
    } else {
        state.accounts.account_for_cwd(cwd)
    };
    let target = req
        .account
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .unwrap_or(&fallback)
        .to_string();
    let (account, account_env) = launch_account(state, Some(&target), cwd)?;
    let source_home = state.accounts.home(&source);
    let target_home = state
        .accounts
        .home(&account)
        .unwrap_or_else(|| state.home.clone());
    let same_dir = source_home
        .as_ref()
        .is_none_or(|h| h.base() == target_home.base());
    // The cwd is checked before anything is copied.
    let cwd = launch_cwd(&target_home, cwd, caller)?;
    if !same_dir {
        // A directory account keeps its own data: the conversation is copied
        // there first (once the desktop allowed copies between the two),
        // then forked under it (docs/ACCOUNTS.md P3.3).
        copy_for_continue(state, &source, &account, id).await?;
    }
    let model = req.model.as_deref().map(validate_model).transpose()?;
    let images = validate_images(&req.images)?;
    let fork = req.fork.unwrap_or(false) || account != source || live_elsewhere(state, id).await;
    let opts = SpawnOptions {
        cwd,
        prompt: req.prompt.unwrap_or_default(),
        model,
        effort: req.effort,
        thinking: req.thinking,
        permission_mode: Some(mode),
        resume: Some(id.to_string()),
        fork,
        session_id: fork.then(|| uuid::Uuid::new_v4().to_string()),
        images,
        allow_bypass: caller.may_dangerous,
        caller_may_dangerous: caller.may_dangerous,
        account: Some(account),
        account_env,
        ..Default::default()
    };
    state.control.spawn(state, opts).await.map_err(spawn_error)
}

/// [`send`] for a caller: a message a caller without the dangerous right
/// queues behind a running turn is remembered, so it can be taken back
/// before the session switches to a mode without approvals ([`set_mode`]).
pub async fn send_as(
    state: &AppState,
    may_dangerous: bool,
    id: &str,
    text: &str,
    images: &[ImageInput],
) -> ApiResult<MessageReceipt> {
    let receipt = send(state, id, text, images).await?;
    if !may_dangerous && receipt.delivery == Delivery::Queued {
        let mid = receipt.message_id.clone();
        state.control.update(id, move |l| {
            l.restricted_queued
                .retain(|q| l.queued.iter().any(|m| &m.id == q));
            l.restricted_queued.push(mid);
        });
    }
    Ok(receipt)
}

/// Send a user message (text + images) to an owned session. Returns the
/// delivery and the message id (cancel a queued one with [`cancel_queued`]).
pub async fn send(
    state: &AppState,
    id: &str,
    text: &str,
    images: &[ImageInput],
) -> ApiResult<MessageReceipt> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(ApiError::unprocessable(
            "message_too_long",
            "The message is too long.",
        ));
    }
    let images = validate_images(images)?;
    let (message_id, delivery) = state
        .control
        .send_user(id, text, &images)
        .await
        .map_err(control_error)?;
    let queued = queued(state, id).len() as u32;
    ws::patch_session(state, id, |s| s.queued = queued).await;
    Ok(MessageReceipt {
        delivery,
        message_id,
    })
}

/// Send a user message to an owned session (delivery only).
pub async fn send_message(
    state: &AppState,
    id: &str,
    text: &str,
    images: &[ImageInput],
) -> ApiResult<Delivery> {
    send(state, id, text, images).await.map(|r| r.delivery)
}

async fn request(state: &AppState, id: &str, request: Value) -> ApiResult<Value> {
    state
        .control
        .request(id, request, REQUEST_TIMEOUT)
        .await
        .map_err(control_error)
}

/// Interrupt the current turn (`interrupt`). Messages still queued behind it
/// keep their place (the CLI answers them next).
pub async fn interrupt(state: &AppState, id: &str) -> ApiResult<()> {
    request(state, id, json!({ "subtype": "interrupt" })).await?;
    Ok(())
}

/// Switch model live (`set_model`).
pub async fn set_model(state: &AppState, id: &str, model: &str) -> ApiResult<()> {
    let model = validate_model(model)?;
    request(state, id, json!({ "subtype": "set_model", "model": model })).await?;
    state.control.update(id, |l| {
        l.model = Some(model.clone());
        l.note_change(control::SettingChange::Model);
    });
    ws::patch_session(state, id, |s| {
        s.model = Some(ModelInfo {
            id: model.clone(),
            label: model.clone(),
            context_window: None,
        })
    })
    .await;
    // The applied effort and ultracode availability follow the model: not
    // known until the CLI answers again.
    publish_settings(state, id).await;
    state.control.refresh_settings(state, id).await;
    Ok(())
}

/// Switch permission mode live (`set_permission_mode`). The dangerous-mode
/// gate is the route's; a session launched without bypass available refuses
/// bypassPermissions here with a clear message.
pub async fn set_mode(state: &AppState, id: &str, mode: &str) -> ApiResult<()> {
    validate_launch(None, None, Some(mode))?;
    if mode == "bypassPermissions" {
        let available = live_state(state, id).is_some_and(|l| l.bypass_available);
        if !available {
            return Err(ApiError::unprocessable(
                "bypass_unavailable",
                "This session was started without bypass available; restart it from the Mac \
                 to use bypassPermissions.",
            ));
        }
    }
    if is_dangerous_mode(Some(mode)) {
        withdraw_restricted_queued(state, id).await;
    }
    request(
        state,
        id,
        json!({ "subtype": "set_permission_mode", "mode": mode }),
    )
    .await?;
    state
        .control
        .update(id, |l| l.permission_mode = Some(mode.to_string()));
    let mode = mode.to_string();
    ws::patch_session(state, id, |s| s.permission_mode = Some(mode)).await;
    Ok(())
}

/// Take back the queued messages a caller without the dangerous right sent:
/// they were allowed because the session asked for approvals, which it is
/// about to stop doing.
async fn withdraw_restricted_queued(state: &AppState, id: &str) {
    let Some(live) = live_state(state, id) else {
        return;
    };
    let ids: Vec<String> = live
        .restricted_queued
        .iter()
        .filter(|q| live.queued.iter().any(|m| &m.id == *q))
        .cloned()
        .collect();
    let mut withdrawn = 0;
    for mid in &ids {
        match cancel_queued(state, id, mid).await {
            Ok(()) => withdrawn += 1,
            Err(e) => {
                tracing::debug!(session = %id, error = %e.message, "queued message not withdrawn")
            }
        }
    }
    state.control.update(id, |l| l.restricted_queued.clear());
    if withdrawn > 0 {
        ws::publish_notice(
            state,
            super::model::NoticeLevel::Warning,
            format!(
                "{withdrawn} queued message(s) from a device without the right to dangerous \
                 actions were withdrawn before the session stopped asking for approvals."
            ),
            Some(id),
        );
    }
}

/// Switch effort live (`apply_flag_settings {effortLevel}`, a session-only
/// flag layer — never `/effort`, which persists to the user's settings).
/// Ultracode rides along while it is on or requested
/// ([`OwnedLive::ultracode_wanted`]): an effort change without the key
/// turns it off.
pub async fn set_effort(state: &AppState, id: &str, effort: &str) -> ApiResult<()> {
    validate_launch(Some(effort), None, None)?;
    let mut settings = json!({ "effortLevel": effort });
    if live_state(state, id).is_some_and(|l| l.ultracode_wanted()) {
        settings["ultracode"] = json!(true);
    }
    request(
        state,
        id,
        json!({ "subtype": "apply_flag_settings", "settings": settings }),
    )
    .await?;
    state.control.update(id, |l| {
        l.effort = Some(effort.to_string());
        l.note_change(control::SettingChange::Effort);
    });
    // Shown as requested until `get_settings` says what the model actually
    // runs at (`max` is `high` without it).
    publish_settings(state, id).await;
    state.control.refresh_settings(state, id).await;
    Ok(())
}

/// Switch ultracode live (`apply_flag_settings {ultracode}`, session only),
/// then re-read what the CLI applies. The key goes alone: it leaves the
/// session's effort as it is (CLI 2.1.284), while an `effortLevel` — even
/// the reduced one `get_settings` reports, `high` for a requested `max` —
/// would set the session to that level. A model or workspace that cannot
/// run it leaves it requested but `unavailable`.
pub async fn set_ultracode(state: &AppState, id: &str, enabled: bool) -> ApiResult<()> {
    if live_state(state, id).is_none() {
        return Err(control_error(ControlError::NotOwned(id.to_string())));
    }
    request(
        state,
        id,
        json!({ "subtype": "apply_flag_settings", "settings": ultracode_settings(enabled) }),
    )
    .await?;
    state.control.update(id, |l| {
        l.ultracode_request = Some(enabled);
        l.note_change(control::SettingChange::Ultracode);
    });
    publish_settings(state, id).await;
    state.control.refresh_settings(state, id).await;
    Ok(())
}

/// The flag settings an ultracode switch sends: the `ultracode` key only.
fn ultracode_settings(enabled: bool) -> Value {
    json!({ "ultracode": enabled })
}

/// Switch thinking live (`set_max_thinking_tokens`: 0 = off, else on; the
/// budget is ignored by adaptive-capable models). Nothing reports thinking
/// back, so the new state is remembered here.
pub async fn set_thinking(state: &AppState, id: &str, enabled: bool) -> ApiResult<()> {
    let budget = if enabled { THINKING_ON_BUDGET } else { 0 };
    request(
        state,
        id,
        json!({ "subtype": "set_max_thinking_tokens", "max_thinking_tokens": budget }),
    )
    .await?;
    state.control.update(id, |l| {
        l.thinking = Some(if enabled {
            OwnedThinking::On(THINKING_ON_BUDGET)
        } else {
            OwnedThinking::Off
        })
    });
    publish_settings(state, id).await;
    Ok(())
}

/// Rename the session (`rename_session {title}`).
pub async fn rename(state: &AppState, id: &str, name: &str) -> ApiResult<()> {
    let title: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect::<String>()
        .trim()
        .to_string();
    if title.is_empty() {
        return Err(ApiError::unprocessable(
            "invalid_name",
            "Name must not be empty.",
        ));
    }
    request(
        state,
        id,
        json!({ "subtype": "rename_session", "title": title }),
    )
    .await?;
    ws::patch_session(state, id, |s| {
        s.name = title.clone();
        s.title = Some(title.clone());
    })
    .await;
    Ok(())
}

/// Stop one background task / subagent of an owned session (`stop_task`).
pub async fn stop_task(state: &AppState, id: &str, task_id: &str) -> ApiResult<()> {
    if !is_simple_id(task_id) {
        return Err(ApiError::unprocessable("invalid_task_id", "Not a task id."));
    }
    request(
        state,
        id,
        json!({ "subtype": "stop_task", "task_id": task_id }),
    )
    .await?;
    Ok(())
}

/// Take back a queued message (`cancel_async_message {message_uuid}`).
pub async fn cancel_queued(state: &AppState, id: &str, message_id: &str) -> ApiResult<()> {
    if !is_uuid(message_id) {
        return Err(ApiError::not_found("not_queued", "No such queued message."));
    }
    if !queued(state, id).iter().any(|q| q.id == message_id) {
        return Err(ApiError::conflict(
            "already_delivered",
            "That message is no longer queued: Claude already has it.",
        ));
    }
    let resp = request(
        state,
        id,
        json!({ "subtype": "cancel_async_message", "message_uuid": message_id }),
    )
    .await?;
    if resp.get("cancelled").and_then(Value::as_bool) == Some(false) {
        return Err(ApiError::conflict(
            "already_delivered",
            "That message is no longer queued: Claude already has it.",
        ));
    }
    state
        .control
        .update(id, |l| l.queued.retain(|q| q.id != message_id));
    let n = queued(state, id).len() as u32;
    ws::patch_session(state, id, |s| s.queued = n).await;
    Ok(())
}

/// The CLI's context usage breakdown (`get_context_usage`).
pub async fn context_usage(state: &AppState, id: &str) -> ApiResult<Value> {
    request(state, id, json!({ "subtype": "get_context_usage" })).await
}

/// The end of a background shell or Monitor task's output
/// (`get_task_output`: `{output, total_bytes, truncated}`, the last 8 KiB,
/// read-only, no model turn). A CLI that predates the request answers an
/// error.
pub async fn task_output(state: &AppState, id: &str, task_id: &str) -> ApiResult<Value> {
    request(
        state,
        id,
        json!({ "subtype": "get_task_output", "task_id": task_id }),
    )
    .await
}

/// MCP server status (`mcp_status`) → `{servers:[{name,status,scope?}]}`.
pub async fn mcp_status(state: &AppState, id: &str) -> ApiResult<Value> {
    let resp = request(state, id, json!({ "subtype": "mcp_status" })).await?;
    let list = resp
        .get("mcpServers")
        .or_else(|| resp.get("servers"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    // Never echo server config (env / headers can hold secrets).
    let servers: Vec<Value> = list
        .iter()
        .map(|s| {
            json!({
                "name": s.get("name").cloned().unwrap_or(Value::Null),
                "status": s.get("status").cloned().unwrap_or(Value::Null),
                "scope": s.get("scope").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    Ok(json!({ "servers": servers }))
}

/// Slash commands the owned session offers (from `initialize`), else `None`
/// (the data route serves the static common list).
pub async fn commands(state: &AppState, id: &str) -> Option<Vec<CommandInfo>> {
    live_state(state, id)
        .filter(|l| l.driver == Driver::Stream && !l.commands.is_empty())
        .map(|l| l.commands)
}

/// Kill an owned session's process (SIGTERM, then SIGKILL).
pub async fn kill(state: &AppState, id: &str) -> ApiResult<()> {
    state
        .control
        .kill(id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))
}

// ──────────────────────────── background sessions ─────────────────────────────

/// The SHORT job id `claude stop|respawn|rm|logs` understand for session `id`:
/// the summary's `jobId`, the live pidfile's `jobId`, else a
/// `jobs/<short>/state.json` whose `sessionId` is `id`. `None` for anything
/// that is not a background job.
pub async fn short_job_id(state: &AppState, id: &str) -> Option<String> {
    let candidates = [
        state.find_session(id).await.and_then(|s| s.job_id),
        state.live.get(id).and_then(|l| l.job_id),
    ];
    if let Some(short) = candidates
        .into_iter()
        .flatten()
        .find(|s| control::is_short_job_id(s))
    {
        return Some(short);
    }
    if !is_uuid(id) {
        return None;
    }
    // Every account's `jobs/` (docs/ACCOUNTS.md P2.5).
    let jobs: Vec<PathBuf> = state
        .accounts
        .homes()
        .into_iter()
        .map(|(_, h)| h.jobs_dir())
        .collect();
    let id = id.to_string();
    tokio::task::spawn_blocking(move || jobs.iter().find_map(|dir| job_for_session(dir, &id)))
        .await
        .ok()
        .flatten()
}

/// Scan `jobs/*/state.json` for the job whose `sessionId` is `session_id`.
pub fn job_for_session(jobs_dir: &Path, session_id: &str) -> Option<String> {
    #[derive(Deserialize, Default)]
    #[serde(default, rename_all = "camelCase")]
    struct JobState {
        session_id: Option<String>,
        daemon_short: Option<String>,
    }
    let entries = std::fs::read_dir(jobs_dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !control::is_short_job_id(&name) {
            continue;
        }
        let path: PathBuf = entry.path().join("state.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(state) = serde_json::from_str::<JobState>(&text) else {
            tracing::debug!(path = %path.display(), "unreadable job state");
            continue;
        };
        if state.session_id.as_deref() == Some(session_id) {
            return Some(
                state
                    .daemon_short
                    .filter(|s| control::is_short_job_id(s))
                    .unwrap_or(name),
            );
        }
    }
    None
}

/// `422 not_a_background_session`.
pub fn not_background() -> ApiError {
    ApiError::unprocessable(
        "not_a_background_session",
        "Only background sessions (claude --bg) can be stopped, respawned or removed this \
         way. Interactive VS Code or terminal sessions have no job id.",
    )
}

/// Run `claude stop|respawn|rm <short>` for session `id`.
pub async fn lifecycle(state: &AppState, action: &str, id: &str) -> ApiResult<String> {
    let short = short_job_id(state, id).await.ok_or_else(not_background)?;
    let env = job_env(state, id)?;
    control::run_lifecycle_for(action, &short, &env)
        .await
        .map_err(|e| ApiError::unprocessable("lifecycle_failed", format!("{e:#}")))
}

/// The current screen of a background session (`claude logs <short>`),
/// rendered to plain text lines.
pub async fn background_screen(state: &AppState, id: &str) -> ApiResult<control::vt::Screen> {
    let short = short_job_id(state, id).await.ok_or_else(not_background)?;
    let env = job_env(state, id)?;
    control::logs_screen_for(&short, &env)
        .await
        .map_err(|e| ApiError::unprocessable("terminal_failed", format!("{e:#}")))
}

/// The environment a background job's lifecycle commands run with: the
/// account whose directory holds the job (docs/ACCOUNTS.md D5, P2.5). A job
/// of a token account the CLI found in the Default's directory needs no
/// token to be stopped or read, so only the directory matters.
fn job_env(state: &AppState, id: &str) -> ApiResult<crate::claude::accounts::AccountEnv> {
    use crate::claude::accounts::{AccountEnv, AccountKind};
    let home_account = state
        .live
        .job(id)
        .and_then(|j| j.account)
        .or_else(|| state.live.get(id).and_then(|l| l.home_account))
        .unwrap_or_else(|| crate::claude::accounts::DEFAULT_ID.to_string());
    match state.accounts.get(&home_account) {
        Some(a) if !a.is_default() && a.kind == AccountKind::Directory => {
            state.accounts.env(&home_account).map_err(account_error)
        }
        _ => Ok(AccountEnv::Inherit),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_validation() {
        assert!(validate_launch(Some("high"), Some("on"), Some("plan")).is_ok());
        assert_eq!(
            validate_launch(Some("turbo"), None, None).unwrap_err().code,
            "invalid_effort"
        );
        assert_eq!(
            validate_launch(None, Some("maybe"), None).unwrap_err().code,
            "invalid_thinking"
        );
        assert_eq!(
            validate_launch(None, None, Some("yolo")).unwrap_err().code,
            "invalid_permission_mode"
        );
        assert!(is_dangerous_mode(Some("bypassPermissions")));
        assert!(!is_dangerous_mode(Some("plan")));
        assert!(!is_dangerous_mode(None));
    }

    #[test]
    fn launch_modes_are_explicit_and_gated() {
        let phone = Caller::default();
        let mac = Caller {
            may_dangerous: true,
            desktop: true,
        };
        assert_eq!(launch_mode(None, phone).unwrap(), "default");
        assert_eq!(launch_mode(Some("auto"), phone).unwrap(), "auto");
        assert_eq!(
            launch_mode(Some("bypassPermissions"), phone)
                .unwrap_err()
                .code,
            "dangerous_blocked"
        );
        assert_eq!(
            launch_mode(Some("dontAsk"), phone).unwrap_err().code,
            "dangerous_blocked"
        );
        assert_eq!(
            launch_mode(Some("bypassPermissions"), mac).unwrap(),
            "bypassPermissions"
        );
    }

    #[test]
    fn ids_and_models() {
        assert!(is_uuid("631b387d-a7e5-4e12-ab38-522cb612803b"));
        assert!(!is_uuid("631B387D-A7E5-4E12-AB38-522CB612803B"));
        assert!(!is_uuid("../../etc/passwd"));
        assert!(!is_uuid("631b387d"));
        assert!(is_simple_id("bldjmw3om"));
        assert!(is_simple_id("toolu_01ABC-x"));
        assert!(!is_simple_id("../x"));
        assert!(!is_simple_id(""));
        assert!(validate_model("claude-opus-5-5").is_ok());
        assert!(validate_model("claude-sonnet-5[1m]").is_ok());
        assert!(validate_model("us.anthropic.claude-haiku-4-5:0").is_ok());
        assert!(validate_model("--dangerously-skip-permissions").is_err());
        assert!(validate_model("opus; rm -rf").is_err());
        assert!(validate_model("").is_err());
    }

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn images_are_verified() {
        let png = [b"\x89PNG\r\n\x1a\n".as_slice(), &[0u8; 16]].concat();
        let ok = validate_images(&[ImageInput {
            media_type: "image/PNG".into(),
            data: format!("data:image/png;base64,{}", b64(&png)),
        }])
        .unwrap();
        assert_eq!(ok[0].media_type, "image/png");
        assert_eq!(ok[0].data, b64(&png));

        // Declared type must match the bytes.
        let err = validate_images(&[ImageInput {
            media_type: "image/jpeg".into(),
            data: b64(&png),
        }])
        .unwrap_err();
        assert_eq!(err.code, "invalid_image");
        // Unsupported types, bad base64, too many, too large.
        assert_eq!(
            validate_images(&[ImageInput {
                media_type: "image/svg+xml".into(),
                data: b64(b"<svg/>"),
            }])
            .unwrap_err()
            .code,
            "unsupported_image_type"
        );
        assert_eq!(
            validate_images(&[ImageInput {
                media_type: "image/gif".into(),
                data: "!!!".into(),
            }])
            .unwrap_err()
            .code,
            "invalid_image"
        );
        let many = vec![
            ImageInput {
                media_type: "image/png".into(),
                data: b64(&png),
            };
            11
        ];
        assert_eq!(validate_images(&many).unwrap_err().code, "too_many_images");
        let big = [b"GIF89a".as_slice(), &vec![0u8; MAX_IMAGE_BYTES]].concat();
        assert_eq!(
            validate_images(&[ImageInput {
                media_type: "image/gif".into(),
                data: b64(&big),
            }])
            .unwrap_err()
            .code,
            "image_too_large"
        );
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff_image(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
    }

    #[test]
    fn trust_from_claude_json_and_transcripts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let claude_json = root.join(".claude.json");
        let projects = root.join("projects");
        std::fs::write(
            &claude_json,
            r#"{"numStartups":3,"projects":{"/work/trusted":{"hasTrustDialogAccepted":true},
               "/work/no":{"hasTrustDialogAccepted":false}}}"#,
        )
        .unwrap();
        assert!(trusted_in(
            &claude_json,
            &projects,
            Path::new("/work/trusted")
        ));
        // Trust is inherited from a parent directory.
        assert!(trusted_in(
            &claude_json,
            &projects,
            Path::new("/work/trusted/sub/dir")
        ));
        assert!(!trusted_in(&claude_json, &projects, Path::new("/work/no")));
        assert!(!trusted_in(
            &claude_json,
            &projects,
            Path::new("/elsewhere")
        ));
        // A directory with transcripts recorded in it counts as trusted.
        let enc = crate::claude::encode_cwd("/elsewhere");
        std::fs::create_dir_all(projects.join(&enc)).unwrap();
        assert!(!trusted_in(
            &claude_json,
            &projects,
            Path::new("/elsewhere")
        ));
        std::fs::write(projects.join(&enc).join("x.jsonl"), "{}").unwrap();
        assert!(
            !trusted_in(&claude_json, &projects, Path::new("/elsewhere")),
            "a transcript that names no cwd proves nothing"
        );
        std::fs::write(
            projects.join(&enc).join("x.jsonl"),
            "{\"type\":\"summary\"}\n{\"type\":\"user\",\"cwd\":\"/elsewhere\"}\n",
        )
        .unwrap();
        assert!(trusted_in(&claude_json, &projects, Path::new("/elsewhere")));
        assert!(trusted_in(
            &claude_json,
            &projects,
            Path::new("/elsewhere/")
        ));
        // A look-alike folder that encodes the same way is not trusted.
        let enc = crate::claude::encode_cwd("/x/app-v2");
        std::fs::create_dir_all(projects.join(&enc)).unwrap();
        std::fs::write(
            projects.join(&enc).join("s.jsonl"),
            "{\"type\":\"user\",\"cwd\":\"/x/app-v2\"}\n",
        )
        .unwrap();
        assert!(trusted_in(&claude_json, &projects, Path::new("/x/app-v2")));
        for twin in ["/x/app.v2", "/x/app_v2", "/x/app v2"] {
            assert_eq!(crate::claude::encode_cwd(twin), enc);
            assert!(
                !trusted_in(&claude_json, &projects, Path::new(twin)),
                "{twin}"
            );
        }
        // Missing or corrupt config is just "not trusted".
        std::fs::write(&claude_json, "{not json").unwrap();
        assert!(!trusted_in(
            &claude_json,
            &projects,
            Path::new("/work/trusted")
        ));
    }

    #[test]
    fn job_state_scan_uses_short_ids_only() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = dir.path();
        std::fs::create_dir_all(jobs.join("631b387d")).unwrap();
        std::fs::write(
            jobs.join("631b387d/state.json"),
            r#"{"state":"done","sessionId":"631b387d-a7e5-4e12-ab38-522cb612803b","daemonShort":"631b387d"}"#,
        )
        .unwrap();
        std::fs::create_dir_all(jobs.join("not-a-job")).unwrap();
        std::fs::write(
            jobs.join("not-a-job/state.json"),
            r#"{"sessionId":"aaaaaaaa-a7e5-4e12-ab38-522cb612803b"}"#,
        )
        .unwrap();
        assert_eq!(
            job_for_session(jobs, "631b387d-a7e5-4e12-ab38-522cb612803b").as_deref(),
            Some("631b387d")
        );
        assert!(job_for_session(jobs, "aaaaaaaa-a7e5-4e12-ab38-522cb612803b").is_none());
        assert!(job_for_session(&jobs.join("missing"), "x").is_none());
    }

    #[test]
    fn control_errors_map_to_the_envelope() {
        let e = control_error(ControlError::Cli(control::CliError {
            message: "auto mode needs a newer model".into(),
            code: Some("auto_mode_model".into()),
        }));
        assert_eq!(e.status, axum::http::StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(e.code, "cli_error");
        assert_eq!(e.body()["error"]["errorCode"], "auto_mode_model");
        assert_eq!(
            control_error(ControlError::Timeout(REQUEST_TIMEOUT)).status,
            axum::http::StatusCode::GATEWAY_TIMEOUT
        );
        assert_eq!(
            control_error(ControlError::Closed).code,
            "owned_session_unavailable"
        );
    }

    fn applied(
        model: &str,
        effort: Option<&str>,
        requested: bool,
        available: bool,
    ) -> control::AppliedSettings {
        control::AppliedSettings {
            model: Some(model.into()),
            effort: effort.map(str::to_string),
            ultracode: requested && available,
            ultracode_requested: requested,
            ultracode_available: available,
        }
    }

    /// A live owned row as the data layer builds it (the transcript's
    /// effort already on it).
    fn owned_row() -> SessionSummary {
        SessionSummary {
            live: true,
            owned: true,
            effort: Some("xhigh".into()),
            effort_source: Some(EffortSource::Observed),
            control: super::super::caps::compute(&super::super::caps::CapsInput {
                owned: true,
                live: true,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn an_owned_session_on_a_model_without_effort_says_so() {
        let live = OwnedLive {
            applied: Some(applied("claude-haiku-4-5", None, false, false)),
            ..Default::default()
        };
        let mut s = owned_row();
        overlay_settings(&mut s, &live, None);
        assert_eq!(s.effort, None, "the transcript's level is gone");
        assert_eq!(s.effort_source, Some(EffortSource::Applied));
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["effortSource"], "applied");
        assert!(v.get("effort").is_none());
        assert_eq!(s.ultracode, Some(UltracodeState::Off));
        assert!(!s.control.set_ultracode && s.control.set_thinking);
    }

    #[test]
    fn what_was_asked_for_shows_until_the_cli_confirms_it() {
        let mut live = OwnedLive {
            applied: Some(applied("claude-opus-5-5", Some("high"), false, true)),
            effort: Some("high".into()),
            ..Default::default()
        };
        let mut s = owned_row();
        overlay_settings(&mut s, &live, None);
        assert_eq!(
            (s.effort.as_deref(), s.effort_source),
            (Some("high"), Some(EffortSource::Applied))
        );
        // `max` accepted, but the follow-up `get_settings` failed: the old
        // answer no longer speaks for the effort (every rebuild says so).
        live.effort = Some("max".into());
        live.note_change(control::SettingChange::Effort);
        for _ in 0..2 {
            overlay_settings(&mut s, &live, None);
            assert_eq!(
                (s.effort.as_deref(), s.effort_source),
                (Some("max"), Some(EffortSource::Requested))
            );
            assert_eq!(s.ultracode, Some(UltracodeState::Off));
            assert_eq!(s.ultracode_source, Some(UltracodeSource::Applied));
        }
        // An ultracode switch the CLI has not confirmed: on, as requested,
        // and still switchable.
        live.ultracode_request = Some(true);
        live.note_change(control::SettingChange::Ultracode);
        overlay_settings(&mut s, &live, None);
        assert_eq!(s.ultracode, Some(UltracodeState::On));
        assert_eq!(s.ultracode_source, Some(UltracodeSource::Requested));
        assert!(s.control.set_ultracode);
        // The answer to a request sent after both confirms them (max runs
        // as max here).
        let as_of = live.sync.changes;
        live.store_applied(applied("claude-opus-5-5", Some("max"), true, true), as_of);
        overlay_settings(&mut s, &live, None);
        assert_eq!(s.effort_source, Some(EffortSource::Applied));
        assert_eq!(s.ultracode_source, Some(UltracodeSource::Applied));
        assert_eq!(s.ultracode, Some(UltracodeState::On));

        // A model change not confirmed yet: the requested effort, ultracode
        // requested (whether it can run is unknown), thinking labelled for
        // the new model.
        live.model = Some("claude-haiku-4-5".into());
        live.thinking = Some(OwnedThinking::On(THINKING_ON_BUDGET));
        live.note_change(control::SettingChange::Model);
        overlay_settings(&mut s, &live, None);
        assert_eq!(s.effort_source, Some(EffortSource::Requested));
        assert_eq!(s.ultracode, Some(UltracodeState::On));
        assert_eq!(s.ultracode_source, Some(UltracodeSource::Requested));
        assert_eq!(
            s.thinking.map(|t| (t.mode, t.budget)),
            Some((ThinkingMode::On, Some(THINKING_ON_BUDGET))),
            "haiku has no adaptive thinking"
        );
        // Nothing requested: the new model's effort is unknown until then.
        live.effort = None;
        overlay_settings(&mut s, &live, None);
        assert_eq!((s.effort, s.effort_source), (None, None));
    }

    #[test]
    fn ultracode_switches_send_the_key_alone() {
        assert_eq!(ultracode_settings(true), json!({ "ultracode": true }));
        assert_eq!(ultracode_settings(false), json!({ "ultracode": false }));
    }
}
