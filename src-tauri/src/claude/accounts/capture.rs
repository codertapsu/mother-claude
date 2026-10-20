//! Getting a token account's token: `claude setup-token` in a pseudo-terminal
//! (docs/ACCOUNTS.md D3, P1.1).
//!
//! `setup-token` is the CLI's sign-in screen (an Ink terminal UI): it opens the
//! browser, waits for the OAuth round trip, then shows "✓ Long-lived
//! authentication token created successfully!", the token (`sk-ant-oat01-…`)
//! and "Store this token securely". It never stores the token itself (CLI
//! 2.1.304: the success branch only displays it) and exits half a second
//! later. When the browser cannot reach the CLI's local callback the screen
//! asks for the code the browser shows ("Paste code here if prompted"), which
//! the desktop can type into the terminal for the user.
//!
//! The screen is read here and never logged or kept: terminal escapes are
//! stripped as the bytes arrive, the sign-in URL is taken from the OSC 8 link
//! (or the visible text), and the first complete token goes straight to the
//! secret store. After five minutes without one the run stops and the UI
//! offers the paste field instead.
//!
//! It runs in the Default account's directory with the inherited token, API
//! key and provider switches removed (the same thing the user would get from
//! `claude setup-token` in a clean terminal); the token belongs to whichever
//! Anthropic account signs in in the browser.

use std::sync::{Arc, Mutex, MutexGuard};

use serde::Serialize;

use super::super::secrets::Secret;

/// How long a run waits for the token.
pub const CAPTURE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5 * 60);
/// Terminal width: wide enough that neither the URL nor the token wraps.
pub const COLS: u16 = 1000;
const ROWS: u16 = 50;
/// Visible text kept for scanning (a spinner redraws for minutes).
const KEEP_TEXT: usize = 64 * 1024;

// ─────────────────────────────── the screen ───────────────────────────────

/// Strips terminal escapes from a byte stream, across chunk boundaries.
/// Non-ASCII bytes become `?`: everything looked for is ASCII.
#[derive(Debug, Default)]
pub struct Stripper {
    state: StripState,
    /// The OSC payload being read (for OSC 8 links).
    osc: Vec<u8>,
    /// CSI parameter bytes.
    csi: Vec<u8>,
    /// Link targets seen in OSC 8 sequences.
    pub links: Vec<String>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum StripState {
    #[default]
    Text,
    Esc,
    /// `ESC (`, `ESC )` …: one more byte.
    EscOne,
    Csi,
    Osc,
    OscEsc,
    /// DCS / PM / APC until `ESC \`.
    Str,
    StrEsc,
}

impl Stripper {
    /// Feed bytes; returns the visible text they add.
    pub fn feed(&mut self, bytes: &[u8]) -> String {
        let mut out = String::new();
        for &b in bytes {
            match self.state {
                StripState::Text => match b {
                    0x1b => self.state = StripState::Esc,
                    b'\n' | b'\r' => out.push('\n'),
                    b'\t' => out.push(' '),
                    0x20..=0x7e => out.push(b as char),
                    0x80..=0xff => out.push('?'),
                    _ => {}
                },
                StripState::Esc => {
                    self.state = match b {
                        b'[' => {
                            self.csi.clear();
                            StripState::Csi
                        }
                        b']' => {
                            self.osc.clear();
                            StripState::Osc
                        }
                        b'P' | b'^' | b'_' | b'X' => StripState::Str,
                        b'(' | b')' | b'*' | b'+' | b'#' | b'%' => StripState::EscOne,
                        _ => StripState::Text,
                    }
                }
                StripState::EscOne => self.state = StripState::Text,
                StripState::Csi => {
                    if (0x40..=0x7e).contains(&b) {
                        // Cursor forward stands in for spaces in some renderers.
                        if b == b'C' {
                            let n = std::str::from_utf8(&self.csi)
                                .ok()
                                .and_then(|s| s.parse::<usize>().ok())
                                .unwrap_or(1)
                                .min(500);
                            out.extend(std::iter::repeat_n(' ', n));
                        } else if matches!(b, b'H' | b'f' | b'E' | b'F' | b'J' | b'K' | b'A' | b'B')
                        {
                            // A cursor jump or erase ends the visible run.
                            out.push('\n');
                        }
                        self.state = StripState::Text;
                    } else if self.csi.len() < 32 {
                        self.csi.push(b);
                    }
                }
                StripState::Osc => match b {
                    0x07 => self.end_osc(),
                    0x1b => self.state = StripState::OscEsc,
                    _ => {
                        if self.osc.len() < 8192 {
                            self.osc.push(b);
                        }
                    }
                },
                StripState::OscEsc => {
                    if b == b'\\' {
                        self.end_osc();
                    } else {
                        self.state = StripState::Osc;
                    }
                }
                StripState::Str => {
                    if b == 0x1b {
                        self.state = StripState::StrEsc;
                    }
                }
                StripState::StrEsc => {
                    self.state = if b == b'\\' {
                        StripState::Text
                    } else {
                        StripState::Str
                    };
                }
            }
        }
        out
    }

    fn end_osc(&mut self) {
        self.state = StripState::Text;
        // `8;params;URI` — an empty URI closes the link.
        if let Some(rest) = self.osc.strip_prefix(b"8;") {
            if let Some(pos) = rest.iter().position(|b| *b == b';') {
                let uri = &rest[pos + 1..];
                if !uri.is_empty() && self.links.len() < 16 {
                    if let Ok(uri) = std::str::from_utf8(uri) {
                        self.links.push(uri.to_string());
                    }
                }
            }
        }
        self.osc.clear();
    }
}

fn is_token_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'-' || c == b'_'
}

/// The first complete `sk-ant-oat<2 digits>-…` in `text`: followed by a
/// character that cannot be part of it, so a token cut by a chunk boundary is
/// never taken.
pub fn find_token(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(pos) = text[from..].find("sk-ant-oat") {
        let start = from + pos;
        let rest = &bytes[start + "sk-ant-oat".len()..];
        let digits_ok = rest.len() > 3
            && rest[0].is_ascii_digit()
            && rest[1].is_ascii_digit()
            && rest[2] == b'-';
        if digits_ok {
            let body = start + "sk-ant-oat".len() + 3;
            let end = bytes[body..]
                .iter()
                .position(|c| !is_token_char(*c))
                .map(|p| body + p);
            if let Some(end) = end {
                if end - body >= 20 {
                    return Some(text[start..end].to_string());
                }
            }
        }
        from = start + 1;
    }
    None
}

/// The sign-in URL: an OSC 8 link target or visible `https://` text naming an
/// OAuth authorize page.
pub fn find_url(links: &[String], text: &str) -> Option<String> {
    let good = |u: &str| {
        u.starts_with("https://")
            && (u.contains("oauth") || u.contains("authorize"))
            && u.len() < 4096
    };
    if let Some(u) = links.iter().find(|u| good(u)) {
        return Some(u.clone());
    }
    let mut from = 0;
    while let Some(pos) = text[from..].find("https://") {
        let start = from + pos;
        let end = text[start..]
            .find(|c: char| c.is_whitespace() || c.is_control())
            .map_or(text.len(), |p| start + p);
        let candidate = &text[start..end];
        if good(candidate) {
            return Some(candidate.to_string());
        }
        from = start + 1;
    }
    None
}

/// What the screen showed, as far as the capture cares.
#[derive(Debug, Default)]
pub struct Screen {
    stripper: Stripper,
    text: String,
    pub url: Option<String>,
    pub token: Option<Secret>,
    pub asks_for_code: bool,
}

impl Screen {
    /// Feed terminal output. Returns true when the token just appeared.
    pub fn feed(&mut self, bytes: &[u8]) -> bool {
        let added = self.stripper.feed(bytes);
        // Rescan only what could hold a match that ends in the new text.
        let rescan_from = self.text.len().saturating_sub(600);
        self.text.push_str(&added);
        let window = &self.text[rescan_from..];
        if self.url.is_none() {
            self.url = find_url(&self.stripper.links, window);
        }
        if !self.asks_for_code {
            let lower = window.to_ascii_lowercase();
            self.asks_for_code = lower.contains("paste code") || lower.contains("paste the code");
        }
        let mut found = false;
        if self.token.is_none() {
            if let Some(t) = find_token(window) {
                self.token = Some(Secret::new(t));
                found = true;
            }
        }
        if self.text.len() > KEEP_TEXT {
            let cut = self.text.len() - KEEP_TEXT / 2;
            let cut = (cut..self.text.len())
                .find(|i| self.text.is_char_boundary(*i))
                .unwrap_or(self.text.len());
            self.text.drain(..cut);
        }
        found
    }

    /// The last visible line, for "it ended without a token" (anything that
    /// looks like a token is masked).
    pub fn last_line(&self) -> Option<String> {
        let line = self
            .text
            .lines()
            .map(str::trim)
            .rfind(|l| l.chars().any(|c| c.is_alphanumeric()))?;
        let masked = match line.find("sk-ant-") {
            Some(p) => format!("{}…", &line[..p]),
            None => line.to_string(),
        };
        Some(masked.chars().take(200).collect())
    }
}

// ─────────────────────────────── a run ────────────────────────────────

/// Where a capture is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CaptureState {
    /// `setup-token` is running; the browser should be open.
    Running,
    /// The token was taken and stored.
    Captured,
    /// It ended without a token.
    Failed,
    /// Five minutes passed.
    TimedOut,
    Cancelled,
}

/// A capture as clients see it (never the token).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureView {
    pub state: CaptureState,
    /// The sign-in page, when the CLI showed one (desktop only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The screen asks for the code the browser shows.
    pub asks_for_code: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub started_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
}

impl CaptureView {
    pub fn running(started_at: i64) -> Self {
        Self {
            state: CaptureState::Running,
            url: None,
            asks_for_code: false,
            error: None,
            started_at,
            finished_at: None,
        }
    }

    /// The same without the URL (for frames every client gets).
    pub fn public(&self) -> Self {
        Self {
            url: None,
            ..self.clone()
        }
    }
}

/// Why a capture could not start or take input.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("this build cannot run `claude setup-token`; paste a token instead")]
    Unavailable,
    #[error("no token capture is running for this account")]
    NotRunning,
    #[error("could not start `claude setup-token`: {0}")]
    Spawn(String),
    #[error("could not type into `claude setup-token`: {0}")]
    Input(String),
}

/// One running (or finished) capture.
pub struct CaptureJob {
    view: Mutex<CaptureView>,
    #[cfg(feature = "token-capture")]
    writer: Mutex<Option<Box<dyn std::io::Write + Send>>>,
    #[cfg(feature = "token-capture")]
    child: Mutex<Option<Box<dyn portable_pty::Child + Send + Sync>>>,
}

impl std::fmt::Debug for CaptureJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaptureJob")
            .field("view", &*lock(&self.view))
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl CaptureJob {
    pub fn view(&self) -> CaptureView {
        lock(&self.view).clone()
    }

    pub fn is_running(&self) -> bool {
        lock(&self.view).state == CaptureState::Running
    }

    fn update(&self, f: impl FnOnce(&mut CaptureView)) -> CaptureView {
        let mut v = lock(&self.view);
        f(&mut v);
        v.clone()
    }

    /// End the run with `state` (only while it runs).
    fn finish(&self, state: CaptureState, error: Option<String>) -> bool {
        let mut v = lock(&self.view);
        if v.state != CaptureState::Running {
            return false;
        }
        v.state = state;
        v.error = error;
        v.finished_at = Some(chrono::Utc::now().timestamp_millis());
        true
    }

    /// Type the code the browser shows into the screen.
    pub fn submit_code(&self, code: &str) -> Result<(), CaptureError> {
        if !self.is_running() {
            return Err(CaptureError::NotRunning);
        }
        #[cfg(feature = "token-capture")]
        {
            let code: String = code.trim().chars().filter(|c| !c.is_control()).collect();
            let mut w = lock(&self.writer);
            let w = w.as_mut().ok_or(CaptureError::NotRunning)?;
            w.write_all(code.as_bytes())
                .and_then(|_| w.write_all(b"\r"))
                .and_then(|_| w.flush())
                .map_err(|e| CaptureError::Input(e.kind().to_string()))?;
            self.update(|v| v.asks_for_code = false);
            Ok(())
        }
        #[cfg(not(feature = "token-capture"))]
        {
            let _ = code;
            Err(CaptureError::Unavailable)
        }
    }

    /// Stop the run (the user closed the dialog).
    pub fn cancel(&self) {
        if self.finish(CaptureState::Cancelled, None) {
            self.kill();
        }
    }

    fn kill(&self) {
        #[cfg(feature = "token-capture")]
        if let Some(child) = lock(&self.child).as_mut() {
            let _ = child.kill();
        }
    }
}

/// What a run reports back as it goes.
pub trait CaptureSink: Send + Sync + 'static {
    /// The view changed (URL seen, code asked for, finished).
    fn changed(&self, view: &CaptureView);
    /// The token appeared. Returns an error message when it could not be
    /// stored (the run then fails with it).
    fn captured(&self, token: Secret) -> Result<(), String>;
}

/// Start `claude setup-token` (`bin`) in a pseudo-terminal with `env`
/// applied. The run reports to `sink` from its own thread and ends on the
/// token, on exit, on [`CaptureJob::cancel`] or after `timeout`.
#[cfg(feature = "token-capture")]
pub fn start(
    bin: &str,
    env: &dyn Fn(&mut dyn super::EnvTarget),
    sink: Arc<dyn CaptureSink>,
    timeout: std::time::Duration,
) -> Result<Arc<CaptureJob>, CaptureError> {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};
    use std::io::Read;

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: ROWS,
            cols: COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| CaptureError::Spawn(e.to_string()))?;
    let mut cmd = CommandBuilder::new(bin);
    cmd.arg("setup-token");
    if let Some(home) = super::super::home::user_home_dir() {
        cmd.cwd(home);
    }
    env(&mut cmd);
    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| CaptureError::Spawn(e.to_string()))?;
    drop(pair.slave);
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| CaptureError::Spawn(e.to_string()))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| CaptureError::Spawn(e.to_string()))?;
    let started_at = chrono::Utc::now().timestamp_millis();
    let job = Arc::new(CaptureJob {
        view: Mutex::new(CaptureView::running(started_at)),
        writer: Mutex::new(Some(writer)),
        child: Mutex::new(Some(child)),
    });
    sink.changed(&job.view());

    // The reader: until EOF (the CLI exited or was killed).
    let reading = job.clone();
    let reader_sink = sink.clone();
    let master = pair.master;
    std::thread::Builder::new()
        .name("setup-token".into())
        .spawn(move || {
            // The master end stays open while we read.
            let _master = master;
            let mut screen = Screen::default();
            let mut buf = [0u8; 8192];
            let mut reported = (None::<String>, false);
            loop {
                let n = match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let found = screen.feed(&buf[..n]);
                if found {
                    if let Some(token) = screen.token.take() {
                        match reader_sink.captured(token) {
                            Ok(()) => {
                                if reading.finish(CaptureState::Captured, None) {
                                    reader_sink.changed(&reading.view());
                                }
                            }
                            Err(e) => {
                                if reading.finish(CaptureState::Failed, Some(e)) {
                                    reader_sink.changed(&reading.view());
                                }
                            }
                        }
                        // It exits on its own half a second later.
                        reading.kill();
                        break;
                    }
                }
                let now = (screen.url.clone(), screen.asks_for_code);
                if now != reported && reading.is_running() {
                    let view = reading.update(|v| {
                        v.url = now.0.clone();
                        v.asks_for_code = now.1;
                    });
                    reader_sink.changed(&view);
                    reported = now;
                }
            }
            let why = screen
                .last_line()
                .map(|l| format!("`claude setup-token` ended without a token: {l}"))
                .unwrap_or_else(|| "`claude setup-token` ended without a token.".to_string());
            if reading.finish(CaptureState::Failed, Some(why)) {
                reader_sink.changed(&reading.view());
            }
            lock(&reading.writer).take();
            if let Some(mut child) = lock(&reading.child).take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        })
        .map_err(|e| CaptureError::Spawn(e.to_string()))?;

    // The deadline.
    let timing = job.clone();
    std::thread::Builder::new()
        .name("setup-token-timeout".into())
        .spawn(move || {
            let deadline = std::time::Instant::now() + timeout;
            while std::time::Instant::now() < deadline {
                if !timing.is_running() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            if timing.finish(
                CaptureState::TimedOut,
                Some("No token after five minutes. Paste one instead.".into()),
            ) {
                timing.kill();
                sink.changed(&timing.view());
            }
        })
        .map_err(|e| CaptureError::Spawn(e.to_string()))?;
    Ok(job)
}

/// Without the `token-capture` feature only pasting works.
#[cfg(not(feature = "token-capture"))]
pub fn start(
    _bin: &str,
    _env: &dyn Fn(&mut dyn super::EnvTarget),
    _sink: Arc<dyn CaptureSink>,
    _timeout: std::time::Duration,
) -> Result<Arc<CaptureJob>, CaptureError> {
    Err(CaptureError::Unavailable)
}

/// Whether this build can run the capture.
pub const AVAILABLE: bool = cfg!(feature = "token-capture");

#[cfg(feature = "token-capture")]
impl super::EnvTarget for portable_pty::CommandBuilder {
    fn set_var(&mut self, key: &str, value: &std::ffi::OsStr) {
        self.env(key, value);
    }
    fn remove_var(&mut self, key: &str) {
        self.env_remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "sk-ant-oat01-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789_-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-AbCdEfGhIjKlMnOp";

    /// What Ink prints, roughly: colours, cursor moves, an OSC 8 link, the
    /// token in the warning colour.
    fn ink_screen(token: &str) -> Vec<u8> {
        let url = "https://claude.ai/oauth/authorize?code=true&client_id=x&state=abc";
        format!(
            "\x1b[?25l\x1b[2K\x1b[1A\x1b[2K\x1b[G\x1b[2mBrowser didn't open? Use the url below to sign in\x1b[22m\n\
             \x1b]8;;{url}\x1b\\\x1b[2m{url}\x1b[22m\x1b]8;;\x1b\\\n\
             \x1b[32m\u{2713} Long-lived authentication token created successfully!\x1b[39m\n\
             Your OAuth token (valid for 1 year):\n\x1b[33m{token}\x1b[39m\n\
             \x1b[2mStore this token securely. You won't be able to see it again.\x1b[22m\n"
        )
        .into_bytes()
    }

    #[test]
    fn the_token_and_url_are_read_through_the_escapes() {
        let mut screen = Screen::default();
        assert!(screen.feed(&ink_screen(TOKEN)));
        assert_eq!(screen.token.as_ref().unwrap().expose(), TOKEN);
        assert_eq!(
            screen.url.as_deref(),
            Some("https://claude.ai/oauth/authorize?code=true&client_id=x&state=abc")
        );
        assert!(!screen.asks_for_code);
    }

    #[test]
    fn a_token_split_across_chunks_is_taken_only_when_complete() {
        let bytes = ink_screen(TOKEN);
        let at = bytes.windows(10).position(|w| w == b"sk-ant-oat").unwrap() + 40;
        let mut screen = Screen::default();
        assert!(!screen.feed(&bytes[..at]), "half a token is not a token");
        assert!(screen.token.is_none());
        assert!(screen.feed(&bytes[at..]));
        assert_eq!(screen.token.unwrap().expose(), TOKEN);
    }

    #[test]
    fn escapes_split_across_chunks_still_strip() {
        let bytes = ink_screen(TOKEN);
        let mut screen = Screen::default();
        let mut found = false;
        for chunk in bytes.chunks(3) {
            found |= screen.feed(chunk);
        }
        assert!(found);
        assert!(screen.url.is_some());
    }

    #[test]
    fn the_code_prompt_is_noticed() {
        let mut screen = Screen::default();
        screen.feed(b"\x1b[1mPaste code here if prompted >\x1b[22m ");
        assert!(screen.asks_for_code);
    }

    #[test]
    fn near_misses_are_not_tokens() {
        for text in [
            "sk-ant-oat01-short ",
            "sk-ant-api03-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789 ",
            "sk-ant-oatXX-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789 ",
            "export CLAUDE_CODE_OAUTH_TOKEN=<token>",
        ] {
            assert_eq!(find_token(text), None, "{text}");
        }
        assert_eq!(find_token(&format!("x {TOKEN}\n")).as_deref(), Some(TOKEN));
    }

    #[test]
    fn the_last_line_never_carries_a_token() {
        let mut screen = Screen::default();
        screen.feed(format!("Error: bad thing {}\n", &TOKEN[..30]).as_bytes());
        let line = screen.last_line().unwrap();
        assert!(!line.contains("sk-ant-"), "{line}");
        assert!(line.starts_with("Error: bad thing"));
    }

    #[test]
    fn a_long_spinner_keeps_the_text_bounded() {
        let mut screen = Screen::default();
        for i in 0..20_000 {
            screen.feed(format!("\x1b[2K\x1b[G\u{280b} Waiting for the browser {i}\n").as_bytes());
        }
        assert!(screen.text.len() <= KEEP_TEXT);
        assert!(screen.feed(&ink_screen(TOKEN)));
    }

    #[test]
    fn views_for_everyone_have_no_url() {
        let mut v = CaptureView::running(1);
        v.url = Some("https://claude.ai/oauth/authorize?x".into());
        assert!(v.public().url.is_none());
        let json = serde_json::to_value(v.public()).unwrap();
        assert_eq!(json["state"], "running");
        assert!(json.get("url").is_none());
    }
}
