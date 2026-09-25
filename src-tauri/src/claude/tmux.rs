//! Driving foreign sessions that run inside tmux (design §5.4, with the §10
//! safety rules).
//!
//! Owner: **B4 (foreign control)**.
//!
//! * **Pane lookup.** The pidfile `tmux` field is `session:@window.%pane`
//!   (validated with the CLI's own pattern). The socket is not recorded, so
//!   the default server is tried first, then every socket in
//!   `${TMUX_TMPDIR:-/tmp}/tmux-<uid>/`, with `list-panes -a -F '#{pane_pid}
//!   #{pane_id}'` matched against the claude pid (or, failing that, the
//!   recorded pane id whose pane pid is an ancestor of the claude pid).
//! * **Typing text** ([`send_text_as`]): refused unless the pidfile shows
//!   `idle`, or `busy` with no `waitingFor` (409 `session_waiting`); C0/C1,
//!   ESC and DEL are stripped; single lines go through `send-keys -t <pane>
//!   -l -- <text>`, multi-line text through `load-buffer` + `paste-buffer -p`
//!   (bracketed paste); after 400 ms the pidfile is re-read and, if a dialog
//!   opened meanwhile, `C-u` clears the input instead of pressing Enter and
//!   the request fails. For callers without the dangerous right a leading `!`
//!   (shell mode) is refused, only an allowlist of slash commands passes, and
//!   sessions in bypassPermissions are refused outright. Max 16 KiB.
//! * **Keys** ([`send_key`]): the allowlist in [`ALLOWED_KEYS`].
//! * **Screen** ([`capture`]): `capture-pane -p -J -S -<lines>` (plain text).
//!
//! Every tmux call is an argv (no shell) with a short timeout. The argv
//! builders and parsers are pure and unit-tested; tmux itself is not needed
//! for the tests.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use super::model::{SessionStatus, SessionSummary};
use crate::server::context::ClientContext;
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Keys `POST /sessions/{id}/keys` accepts.
pub const ALLOWED_KEYS: [&str; 14] = [
    "Escape", "Enter", "Up", "Down", "Tab", "1", "2", "3", "4", "5", "6", "7", "8", "9",
];

/// Slash commands a caller without the dangerous right may type. Everything
/// else (`/permissions`, `/add-dir`, `/hooks`, `/mcp`, `/plugin`, `/login`,
/// `/config`, `/sandbox`, …) changes standing state.
pub const SAFE_SLASH_COMMANDS: [&str; 11] = [
    "/compact", "/clear", "/context", "/usage", "/cost", "/status", "/todos", "/model", "/effort",
    "/rename", "/btw",
];

/// Largest text typed in one request.
pub const TEXT_MAX_BYTES: usize = 16 * 1024;

/// Pause between typing and pressing Enter (long text + Enter in one call is
/// taken as a paste and Enter becomes a newline).
const ENTER_DELAY: Duration = Duration::from_millis(400);

/// Timeout of one tmux invocation.
const TMUX_TIMEOUT: Duration = Duration::from_secs(3);

// ───────────────────────────── pure helpers ─────────────────────────────

/// A parsed pidfile `tmux` target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub session: String,
    /// `@N`
    pub window: String,
    /// `%N`
    pub pane: String,
}

/// Parse `session:@window.%pane` the way the CLI validates it
/// (`^[A-Za-z0-9_.-]{1,64}:@?\d{1,6}\.%?\d{1,6}$`). A bare pane id (`%12`)
/// is accepted too.
pub fn parse_target(target: &str) -> Option<Target> {
    let t = target.trim();
    if let Some(n) = t.strip_prefix('%') {
        return is_digits(n, 6).then(|| Target {
            session: String::new(),
            window: String::new(),
            pane: t.to_string(),
        });
    }
    let (session, rest) = t.split_once(':')?;
    if session.is_empty()
        || session.len() > 64
        || !session
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        return None;
    }
    let (window, pane) = rest.split_once('.')?;
    let w = window.strip_prefix('@').unwrap_or(window);
    let p = pane.strip_prefix('%').unwrap_or(pane);
    if !is_digits(w, 6) || !is_digits(p, 6) {
        return None;
    }
    Some(Target {
        session: session.to_string(),
        window: format!("@{w}"),
        pane: format!("%{p}"),
    })
}

fn is_digits(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && s.bytes().all(|b| b.is_ascii_digit())
}

/// Parse `list-panes -F '#{pane_pid} #{pane_id}'` output.
pub fn parse_list_panes(out: &str) -> Vec<(u32, String)> {
    out.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let pid = it.next()?.parse::<u32>().ok()?;
            let pane = it.next()?;
            parse_target(pane).map(|t| (pid, t.pane))
        })
        .collect()
}

fn base(socket: Option<&Path>) -> Vec<OsString> {
    match socket {
        Some(s) => vec!["-S".into(), s.as_os_str().to_os_string()],
        None => Vec::new(),
    }
}

fn with(socket: Option<&Path>, rest: &[&str]) -> Vec<OsString> {
    let mut v = base(socket);
    v.extend(rest.iter().map(OsString::from));
    v
}

/// `list-panes -a -F '#{pane_pid} #{pane_id}'`.
pub fn list_panes_args(socket: Option<&Path>) -> Vec<OsString> {
    with(
        socket,
        &["list-panes", "-a", "-F", "#{pane_pid} #{pane_id}"],
    )
}

/// `send-keys -t <pane> -l -- <text>`: literal text, never key names.
pub fn send_literal_args(socket: Option<&Path>, pane: &str, text: &str) -> Vec<OsString> {
    with(socket, &["send-keys", "-t", pane, "-l", "--", text])
}

/// `send-keys -t <pane> <key>` for one allowlisted key name.
pub fn send_key_args(socket: Option<&Path>, pane: &str, key: &str) -> Vec<OsString> {
    with(socket, &["send-keys", "-t", pane, key])
}

/// `load-buffer -b <buffer> -` (text on stdin).
pub fn load_buffer_args(socket: Option<&Path>, buffer: &str) -> Vec<OsString> {
    with(socket, &["load-buffer", "-b", buffer, "-"])
}

/// `paste-buffer -p -d -b <buffer> -t <pane>` (bracketed paste, then delete
/// the buffer).
pub fn paste_buffer_args(socket: Option<&Path>, buffer: &str, pane: &str) -> Vec<OsString> {
    with(
        socket,
        &["paste-buffer", "-p", "-d", "-b", buffer, "-t", pane],
    )
}

/// `capture-pane -p -J -t <pane> -S -<lines>` (plain text, joined lines).
pub fn capture_args(socket: Option<&Path>, pane: &str, lines: u32) -> Vec<OsString> {
    let start = format!("-{lines}");
    with(
        socket,
        &["capture-pane", "-p", "-J", "-t", pane, "-S", &start],
    )
}

/// Make text safe to type: CRLF/CR → LF, tabs → spaces, strip every other C0
/// control, ESC, DEL and C1 control. Trailing whitespace is trimmed. Errors
/// when nothing is left or the text is too long.
pub fn sanitize_text(text: &str) -> ApiResult<String> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let cleaned: String = normalized
        .chars()
        .filter_map(|c| match c {
            '\n' => Some('\n'),
            '\t' => Some(' '),
            c if (c as u32) < 0x20 || c == '\u{7f}' => None,
            c if ('\u{80}'..='\u{9f}').contains(&c) => None,
            c => Some(c),
        })
        .collect();
    let cleaned = cleaned.trim_end().to_string();
    if cleaned.trim().is_empty() {
        return Err(ApiError::unprocessable("empty_message", "Nothing to type."));
    }
    if cleaned.len() > TEXT_MAX_BYTES {
        return Err(ApiError::unprocessable(
            "message_too_long",
            format!(
                "Text typed into a terminal is limited to {} KiB.",
                TEXT_MAX_BYTES / 1024
            ),
        ));
    }
    Ok(cleaned)
}

/// The rules for callers without the dangerous right: no leading `!` (shell
/// mode, runs without a permission check) and only [`SAFE_SLASH_COMMANDS`].
pub fn check_restricted_text(text: &str) -> ApiResult<()> {
    let t = text.trim_start();
    if t.starts_with('!') {
        return Err(ApiError::dangerous_blocked(
            "Text starting with `!` runs a shell command in the session without a permission \
             check; only this computer or a device allowed dangerous actions may send it.",
        ));
    }
    if t.starts_with('/') {
        let cmd = t.split_whitespace().next().unwrap_or(t);
        if !SAFE_SLASH_COMMANDS.contains(&cmd) {
            return Err(ApiError::dangerous_blocked(format!(
                "{cmd} changes the session's standing configuration; only this computer or a \
                 device allowed dangerous actions may send it. Allowed here: {}.",
                SAFE_SLASH_COMMANDS.join(" ")
            )));
        }
    }
    Ok(())
}

/// Whether the session may be typed into now: `idle`, or `busy` with no
/// `waitingFor` (a dialog or picker would take the keystrokes).
pub fn may_type(status: Option<&str>, waiting_for: Option<&str>) -> bool {
    match status {
        Some("idle") => true,
        Some("busy") => waiting_for.is_none_or(|w| w.trim().is_empty()),
        _ => false,
    }
}

/// Strip what a terminal screen should never carry to a client (escapes and
/// other controls; `capture-pane` without `-e` has none, but be sure) and
/// drop trailing blank lines.
pub fn clean_screen(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .filter(|&c| c == '\n' || c == '\t' || !(c.is_control() || c == '\u{7f}'))
        .collect();
    let mut lines: Vec<&str> = cleaned.lines().map(str::trim_end).collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

// ─────────────────────────────── tmux state ───────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct Resolved {
    socket: Option<PathBuf>,
    pane: String,
    pid: Option<u32>,
}

/// Resolved tmux sockets / panes per session.
#[derive(Debug, Default)]
pub struct TmuxState {
    resolved: Mutex<HashMap<String, Resolved>>,
    bin: OnceLock<Option<PathBuf>>,
}

impl TmuxState {
    pub fn new() -> Self {
        Self::default()
    }

    fn cached(&self, id: &str, pid: Option<u32>) -> Option<Resolved> {
        let m = self.resolved.lock().ok()?;
        m.get(id).filter(|r| r.pid == pid).cloned()
    }

    fn remember(&self, id: &str, r: Resolved) {
        if let Ok(mut m) = self.resolved.lock() {
            m.insert(id.to_string(), r);
        }
    }

    fn forget(&self, id: &str) {
        if let Ok(mut m) = self.resolved.lock() {
            m.remove(id);
        }
    }

    /// The tmux binary: `$MOTHER_CLAUDE_TMUX`, `PATH`, then the usual
    /// Homebrew/system locations (a GUI app's PATH is short).
    fn binary(&self) -> Option<PathBuf> {
        self.bin
            .get_or_init(|| {
                if let Ok(p) = std::env::var("MOTHER_CLAUDE_TMUX") {
                    let p = PathBuf::from(p);
                    if p.is_file() {
                        return Some(p);
                    }
                }
                let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
                    .map(|p| std::env::split_paths(&p).collect())
                    .unwrap_or_default();
                for extra in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
                    dirs.push(PathBuf::from(extra));
                }
                dirs.into_iter()
                    .map(|d| d.join("tmux"))
                    .find(|p| p.is_file())
            })
            .clone()
    }
}

async fn run_tmux(bin: &Path, args: &[OsString], stdin: Option<&[u8]>) -> ApiResult<String> {
    use tokio::io::AsyncWriteExt;
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args(args)
        .stdin(if stdin.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| ApiError::conflict("tmux_unavailable", format!("Could not run tmux: {e}")))?;
    if let (Some(data), Some(mut pipe)) = (stdin, child.stdin.take()) {
        if let Err(e) = pipe.write_all(data).await {
            tracing::debug!(error = %e, "tmux stdin write failed");
        }
        drop(pipe);
    }
    let out = tokio::time::timeout(TMUX_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| ApiError::conflict("tmux_timeout", "tmux did not answer in time."))?
        .map_err(|e| ApiError::conflict("tmux_failed", format!("tmux failed: {e}")))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(ApiError::conflict(
            "tmux_failed",
            format!("tmux refused the command: {}", err.trim()),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Candidate sockets besides the default server.
fn socket_candidates() -> Vec<PathBuf> {
    let tmp = std::env::var_os("TMUX_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    let dir = tmp.join(format!("tmux-{uid}"));
    let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.file_name().is_none_or(|n| n != "default"))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// Parent pid of `pid` (`ps -o ppid=`), for the pane-pid ancestor match.
async fn parent_of(pid: u32) -> Option<u32> {
    let out = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::process::Command::new("ps")
            .args(["-o", "ppid=", "-p", &pid.to_string()])
            .env("LC_ALL", "C")
            .stdin(std::process::Stdio::null())
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

async fn is_ancestor(ancestor: u32, mut pid: u32) -> bool {
    for _ in 0..8 {
        match parent_of(pid).await {
            Some(p) if p == ancestor => return true,
            Some(p) if p > 1 => pid = p,
            _ => return false,
        }
    }
    false
}

/// The tmux target of a session: the pidfile record's, else the summary's.
fn target_for(state: &AppState, session: &SessionSummary) -> Option<(Target, Option<u32>)> {
    let live = state.live.get(&session.id);
    let raw = live
        .as_ref()
        .and_then(|l| l.tmux.clone())
        .or_else(|| session.tmux_pane.clone())?;
    let pid = live.as_ref().and_then(|l| l.pid).or(session.pid);
    parse_target(&raw).map(|t| (t, pid))
}

async fn resolve(state: &AppState, session: &SessionSummary) -> ApiResult<(PathBuf, Resolved)> {
    let bin = state.tmux.binary().ok_or_else(|| {
        ApiError::conflict(
            "tmux_unavailable",
            "tmux was not found on this computer (set MOTHER_CLAUDE_TMUX to its path).",
        )
    })?;
    let (target, pid) = target_for(state, session).ok_or_else(|| {
        ApiError::conflict(
            "no_tmux_pane",
            "This session is not running inside tmux (no pane recorded).",
        )
    })?;
    if let Some(r) = state.tmux.cached(&session.id, pid) {
        return Ok((bin, r));
    }
    let mut sockets: Vec<Option<PathBuf>> = vec![None];
    sockets.extend(socket_candidates().into_iter().map(Some));
    let mut fallback: Option<Resolved> = None;
    for socket in sockets {
        let Ok(out) = run_tmux(&bin, &list_panes_args(socket.as_deref()), None).await else {
            continue;
        };
        let panes = parse_list_panes(&out);
        if let Some(pid) = pid {
            if let Some((_, pane)) = panes.iter().find(|(p, _)| *p == pid) {
                let r = Resolved {
                    socket: socket.clone(),
                    pane: pane.clone(),
                    pid: Some(pid),
                };
                state.tmux.remember(&session.id, r.clone());
                return Ok((bin, r));
            }
            if let Some((pane_pid, pane)) = panes.iter().find(|(_, p)| *p == target.pane) {
                if fallback.is_none() && is_ancestor(*pane_pid, pid).await {
                    fallback = Some(Resolved {
                        socket: socket.clone(),
                        pane: pane.clone(),
                        pid: Some(pid),
                    });
                }
            }
        }
    }
    match fallback {
        Some(r) => {
            state.tmux.remember(&session.id, r.clone());
            Ok((bin, r))
        }
        None => Err(ApiError::conflict(
            "no_tmux_pane",
            "The session's tmux pane could not be found (is its tmux server still running?).",
        )),
    }
}

/// The pane (`%12`) a session runs in, if found.
pub async fn pane_for(state: &AppState, session: &SessionSummary) -> Option<String> {
    resolve(state, session).await.ok().map(|(_, r)| r.pane)
}

/// The pidfile status right now (`status`, `waitingFor`), falling back to
/// the summary when the live roster has no record.
fn current_status(state: &AppState, session: &SessionSummary) -> (Option<String>, Option<String>) {
    if let Some(l) = state.live.get(&session.id) {
        if l.status.is_some() {
            return (l.status, l.waiting_for);
        }
    }
    let status = match session.status {
        SessionStatus::Waiting => "waiting",
        SessionStatus::Working | SessionStatus::Starting => "busy",
        SessionStatus::Idle => "idle",
        _ => "ended",
    };
    let waiting = session.waiting_for.clone().or_else(|| {
        session
            .pending
            .iter()
            .any(|p| p.answerable)
            .then(|| "permission prompt".to_string())
    });
    (Some(status.to_string()), waiting)
}

fn waiting_error(waiting_for: Option<&str>) -> ApiError {
    ApiError::conflict(
        "session_waiting",
        match waiting_for {
            Some(w) if !w.trim().is_empty() => format!(
                "The session is showing a dialog ({w}); typing now would answer it. Answer it \
                 first (or open the terminal view)."
            ),
            _ => "The session is not ready for typed input right now.".to_string(),
        },
    )
}

async fn type_now(
    state: &AppState,
    session: &SessionSummary,
    bin: &Path,
    r: &Resolved,
    text: &str,
) -> ApiResult<()> {
    let socket = r.socket.as_deref();
    if text.contains('\n') {
        let buffer = format!("mc-{}", uuid::Uuid::new_v4().simple());
        run_tmux(
            bin,
            &load_buffer_args(socket, &buffer),
            Some(text.as_bytes()),
        )
        .await?;
        run_tmux(bin, &paste_buffer_args(socket, &buffer, &r.pane), None).await?;
    } else {
        run_tmux(bin, &send_literal_args(socket, &r.pane, text), None).await?;
    }
    tokio::time::sleep(ENTER_DELAY).await;
    // A dialog may have opened while we typed: never let our Enter answer it.
    let (status, waiting_for) = current_status(state, session);
    if status.as_deref() == Some("waiting") || !may_type(status.as_deref(), waiting_for.as_deref())
    {
        let _ = run_tmux(bin, &send_key_args(socket, &r.pane, "C-u"), None).await;
        return Err(waiting_error(waiting_for.as_deref()));
    }
    run_tmux(bin, &send_key_args(socket, &r.pane, "Enter"), None).await?;
    Ok(())
}

/// Type `text` into the session's pane and press Enter, applying the rules
/// for `ctx` (see the module docs).
pub async fn send_text_as(
    state: &AppState,
    ctx: &ClientContext,
    session: &SessionSummary,
    text: &str,
) -> ApiResult<()> {
    send_text_inner(state, ctx.may_dangerous(), session, text).await
}

/// Type `text` with the restricted rules (a caller without the dangerous
/// right). Used for the fixed slash commands the dispatchers send.
pub async fn send_text(state: &AppState, session: &SessionSummary, text: &str) -> ApiResult<()> {
    send_text_inner(state, false, session, text).await
}

async fn send_text_inner(
    state: &AppState,
    privileged: bool,
    session: &SessionSummary,
    text: &str,
) -> ApiResult<()> {
    let text = sanitize_text(text)?;
    if !privileged {
        check_restricted_text(&text)?;
        if crate::server::hooks::effective_bypass(state, session).await {
            return Err(ApiError::dangerous_blocked(
                "This session runs without permission prompts (bypassPermissions); typing into \
                 it is restricted to this computer or a device allowed dangerous actions.",
            ));
        }
    }
    let (status, waiting_for) = current_status(state, session);
    if !may_type(status.as_deref(), waiting_for.as_deref()) {
        return Err(waiting_error(waiting_for.as_deref()));
    }
    let (bin, r) = resolve(state, session).await?;
    match type_now(state, session, &bin, &r, &text).await {
        Err(e) if e.code == "tmux_failed" => {
            state.tmux.forget(&session.id);
            Err(e)
        }
        other => other,
    }
}

/// Press one allowlisted key in the session's pane.
pub async fn send_key(state: &AppState, session: &SessionSummary, key: &str) -> ApiResult<()> {
    if !ALLOWED_KEYS.contains(&key) {
        return Err(ApiError::unprocessable(
            "invalid_key",
            format!("Key must be one of {ALLOWED_KEYS:?}."),
        ));
    }
    let (bin, r) = resolve(state, session).await?;
    let res = run_tmux(
        &bin,
        &send_key_args(r.socket.as_deref(), &r.pane, key),
        None,
    )
    .await;
    if res.is_err() {
        state.tmux.forget(&session.id);
    }
    res.map(|_| ())
}

/// The last `lines` lines of the pane as plain text.
pub async fn capture(state: &AppState, session: &SessionSummary, lines: u32) -> ApiResult<String> {
    let lines = lines.clamp(1, 2000);
    let (bin, r) = resolve(state, session).await?;
    let out = run_tmux(
        &bin,
        &capture_args(r.socket.as_deref(), &r.pane, lines),
        None,
    )
    .await;
    if out.is_err() {
        state.tmux.forget(&session.id);
    }
    Ok(clean_screen(&out?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(v: Vec<OsString>) -> Vec<String> {
        v.into_iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn targets_parse_like_the_cli() {
        let t = parse_target("mcprobe:@0.%0").unwrap();
        assert_eq!(t.session, "mcprobe");
        assert_eq!(t.window, "@0");
        assert_eq!(t.pane, "%0");
        assert_eq!(parse_target("main:1.2").unwrap().pane, "%2");
        assert_eq!(parse_target("%12").unwrap().pane, "%12");
        for bad in [
            "",
            "main",
            "main:@1",
            "a b:@1.%2",
            "x;rm:@1.%2",
            "main:@1.%2x",
            "main:@1234567.%2",
            "%",
            "%1a",
            &format!("{}:@1.%2", "s".repeat(65)),
        ] {
            assert!(parse_target(bad).is_none(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn list_panes_output_parses() {
        let out = "74764 %0\n  123 %7 \ngarbage\n9 notapane\n42 %3\n";
        assert_eq!(
            parse_list_panes(out),
            vec![(74764, "%0".into()), (123, "%7".into()), (42, "%3".into())]
        );
    }

    #[test]
    fn argv_is_built_without_a_shell() {
        let sock = PathBuf::from("/tmp/tmux-501/my sock");
        assert_eq!(
            strs(send_literal_args(Some(&sock), "%3", "-rf; echo $(x)")),
            vec![
                "-S",
                "/tmp/tmux-501/my sock",
                "send-keys",
                "-t",
                "%3",
                "-l",
                "--",
                "-rf; echo $(x)"
            ]
        );
        assert_eq!(
            strs(send_key_args(None, "%3", "Enter")),
            vec!["send-keys", "-t", "%3", "Enter"]
        );
        assert_eq!(
            strs(load_buffer_args(None, "mc-1")),
            vec!["load-buffer", "-b", "mc-1", "-"]
        );
        assert_eq!(
            strs(paste_buffer_args(None, "mc-1", "%3")),
            vec!["paste-buffer", "-p", "-d", "-b", "mc-1", "-t", "%3"]
        );
        assert_eq!(
            strs(capture_args(None, "%3", 200)),
            vec!["capture-pane", "-p", "-J", "-t", "%3", "-S", "-200"]
        );
        assert_eq!(
            strs(list_panes_args(None)),
            vec!["list-panes", "-a", "-F", "#{pane_pid} #{pane_id}"]
        );
    }

    #[test]
    fn text_is_sanitized() {
        assert_eq!(sanitize_text("hello\r\nworld\r").unwrap(), "hello\nworld");
        assert_eq!(sanitize_text("a\x1b[2Jb\x07c\td").unwrap(), "a[2Jbc d");
        assert_eq!(sanitize_text("x\u{9b}31my\u{7f}").unwrap(), "x31my");
        assert_eq!(sanitize_text(" \n\x1b ").unwrap_err().code, "empty_message");
        assert_eq!(
            sanitize_text(&"a".repeat(TEXT_MAX_BYTES + 1))
                .unwrap_err()
                .code,
            "message_too_long"
        );
        assert!(sanitize_text(&"a".repeat(TEXT_MAX_BYTES)).is_ok());
    }

    #[test]
    fn restricted_callers_get_safe_input_only() {
        assert!(check_restricted_text("please run the tests").is_ok());
        assert!(check_restricted_text("/compact keep the plan").is_ok());
        assert!(check_restricted_text("/model opus").is_ok());
        assert!(check_restricted_text("see !this").is_ok());
        for bad in [
            "!rm -rf ~",
            "  ! ls",
            "/permissions",
            "/add-dir /",
            "/hooks",
            "/mcp",
            "/plugin install x",
            "/login",
            "/config",
            "/sandbox",
            "/compactx",
        ] {
            let e = check_restricted_text(bad).unwrap_err();
            assert_eq!(e.code, "dangerous_blocked", "{bad:?}");
        }
    }

    #[test]
    fn typing_needs_an_idle_or_undialogued_session() {
        assert!(may_type(Some("idle"), None));
        assert!(may_type(Some("idle"), Some("permission prompt")));
        assert!(may_type(Some("busy"), None));
        assert!(may_type(Some("busy"), Some("")));
        assert!(!may_type(Some("busy"), Some("dialog open")));
        assert!(!may_type(Some("waiting"), None));
        assert!(!may_type(Some("waiting"), Some("permission prompt")));
        assert!(!may_type(None, None));
        assert!(!may_type(Some("ended"), None));
    }

    #[test]
    fn screens_are_plain_text() {
        assert_eq!(
            clean_screen("a\x1b[31mred\x07\r\nline  \n\n\n"),
            "a[31mred\nline"
        );
    }

    #[test]
    fn key_allowlist() {
        for k in ["Escape", "Enter", "Up", "Down", "Tab", "1", "9"] {
            assert!(ALLOWED_KEYS.contains(&k));
        }
        for k in ["C-c", "0", "q", "BSpace", "C-u"] {
            assert!(!ALLOWED_KEYS.contains(&k));
        }
    }
}
