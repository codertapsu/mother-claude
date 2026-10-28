//! Sign-ins in a pseudo-terminal (docs/ACCOUNTS.md D3, §15.2): `claude auth
//! login` for the Default and directory accounts, `claude setup-token` for
//! token accounts. [`super::signin`] decides what runs; this is the terminal
//! side — the screen, the run, what clients see of it.
//!
//! Both run in a pseudo-terminal, so the CLI does not think it is headless
//! (without a terminal `claude auth login` never opens a browser and prints
//! its link to nobody), with `BROWSER=true`, so its own opener runs the `true`
//! no-op and opens nothing: the link is read from the screen — the OSC 8 link
//! target or the visible URL, and only a [`Link`] (`https`, Anthropic's
//! hosts) — and Mother Claude opens it ([`crate::claude::browser`]).
//!
//! `setup-token` is the CLI's sign-in screen (an Ink terminal UI): after the
//! OAuth round trip it shows "✓ Long-lived authentication token created
//! successfully!", the token (`sk-ant-oat01-…`) and "Store this token
//! securely". It never stores the token itself (CLI 2.1.304: the success
//! branch only displays it) and exits half a second later. `auth login`
//! stores its sign-in in the account's own place and exits; whether it worked
//! is what its exit status and `claude auth status` say afterwards. When the
//! browser cannot reach the CLI's local callback, either screen asks for the
//! code the browser shows ("Paste code here if prompted"), which the desktop
//! can type into the terminal for the user.
//!
//! The screen is read here and never logged or kept: terminal escapes are
//! stripped as the bytes arrive, and the first complete token goes straight
//! to the secret store. A run that shows no link within a minute, or does not
//! finish within five, stops; a token account then offers the paste field.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::Serialize;

use super::super::secrets::Secret;
use crate::claude::browser::{Browser, Link};

/// How long a run may take in all.
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// How long a run may take to show its link.
pub const LINK_TIMEOUT: Duration = Duration::from_secs(60);
/// Terminal width: wide enough that neither the URL nor the token wraps.
pub const COLS: u16 = 1000;
const ROWS: u16 = 50;
/// Visible text kept for scanning (a spinner redraws for minutes).
const KEEP_TEXT: usize = 64 * 1024;
/// Longest code typed into the screen.
const CODE_MAX: usize = 1024;

// ─────────────────────────────── the screen ───────────────────────────────

/// Strips terminal escapes from a byte stream, across chunk boundaries.
/// Non-ASCII bytes become spaces: everything looked for is ASCII, and a frame
/// drawn right after a URL must not run into it.
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
                    0x80..=0xff => out.push(' '),
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

/// What a look for the sign-in link found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LinkScan {
    /// The first sign-in link Mother Claude may open.
    pub link: Option<Link>,
    /// A link that looked like a sign-in page was refused: not `https`, or
    /// not one of Anthropic's hosts ([`Link`]).
    pub refused: bool,
}

/// The sign-in link: an OSC 8 link target or visible `http(s)://` text
/// naming an OAuth authorize page, taken only when Mother Claude may open it
/// ([`Link`]). A link to anywhere else is never taken, so it is never shown
/// or opened; that one was seen is `refused`. Visible text counts only once
/// the line goes on past it: a URL cut by a chunk boundary is not one yet.
pub fn find_url(links: &[String], text: &str) -> LinkScan {
    let mut refused = false;
    let mut good = |u: &str| {
        if !(u.contains("oauth") || u.contains("authorize")) {
            return None;
        }
        let link = Link::parse(u);
        refused |= link.is_none();
        link
    };
    for u in links {
        if let Some(link) = good(u) {
            return LinkScan {
                link: Some(link),
                refused,
            };
        }
    }
    let mut from = 0;
    while let Some(pos) = text[from..].find("http") {
        let start = from + pos;
        from = start + 1;
        let rest = &text[start + 4..];
        if !(rest.starts_with("://") || rest.starts_with("s://")) {
            continue;
        }
        let Some(len) = text[start..].find(|c: char| c.is_whitespace() || c.is_control()) else {
            break;
        };
        if let Some(link) = good(&text[start..start + len]) {
            return LinkScan {
                link: Some(link),
                refused,
            };
        }
    }
    LinkScan {
        link: None,
        refused,
    }
}

/// How far back each new piece of the screen is scanned from: a whole link
/// ([`crate::claude::browser`] opens none longer) and then some.
const RESCAN: usize = 4200;

/// What the screen showed, as far as a sign-in cares.
#[derive(Debug, Default)]
pub struct Screen {
    stripper: Stripper,
    text: String,
    pub url: Option<Link>,
    /// A link that looked like the sign-in page was refused.
    pub refused_url: bool,
    pub token: Option<Secret>,
    pub asks_for_code: bool,
}

impl Screen {
    /// Feed terminal output. Returns true when the token just appeared.
    pub fn feed(&mut self, bytes: &[u8]) -> bool {
        let added = self.stripper.feed(bytes);
        // Rescan only what could hold a match that ends in the new text.
        let rescan_from = self.text.len().saturating_sub(RESCAN);
        let rescan_from = (rescan_from..=self.text.len())
            .find(|i| self.text.is_char_boundary(*i))
            .unwrap_or(self.text.len());
        self.text.push_str(&added);
        let window = &self.text[rescan_from..];
        if self.url.is_none() {
            let scan = find_url(&self.stripper.links, window);
            self.url = scan.link;
            self.refused_url |= scan.refused;
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

    /// The last visible line, for "it ended without a token": anything that
    /// looks like a token or a link is masked (the link is the desktop's
    /// alone, and an error goes to every client).
    pub fn last_line(&self) -> Option<String> {
        let line = self
            .text
            .lines()
            .map(str::trim)
            .rfind(|l| l.chars().any(|c| c.is_alphanumeric()))?;
        let mut masked = match line.find("sk-ant-") {
            Some(p) => format!("{}…", &line[..p]),
            None => line.to_string(),
        };
        while let Some(p) = masked.find("https://").or_else(|| masked.find("http://")) {
            let end = masked[p..]
                .find(char::is_whitespace)
                .map_or(masked.len(), |e| p + e);
            masked.replace_range(p..end, "<link>");
        }
        Some(masked.chars().take(200).collect())
    }
}

// ─────────────────────────────── a run ────────────────────────────────

/// Which sign-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SignInKind {
    /// `claude auth login`: the Default or a directory account.
    Login,
    /// `claude setup-token`: a token account.
    Token,
}

impl SignInKind {
    /// The command, for messages.
    pub fn command(self) -> &'static str {
        match self {
            Self::Login => "claude auth login",
            Self::Token => "claude setup-token",
        }
    }
}

/// Where a sign-in is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SignInState {
    /// The CLI runs; its link is not known yet.
    Starting,
    /// The link is known: the browser is (or can be) open on it.
    Open,
    /// The CLI's screen asks for the code the browser shows (it may still
    /// finish on its own when the browser reaches it).
    Code,
    /// Signed in: the login checked out, or the token is stored.
    Done,
    /// It ended without a sign-in (`error` says why).
    Failed,
    Cancelled,
}

impl SignInState {
    pub fn is_running(self) -> bool {
        matches!(self, Self::Starting | Self::Open | Self::Code)
    }
}

/// A sign-in as clients see it (never a token; the link only in the
/// desktop's own reads).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignInView {
    pub kind: SignInKind,
    pub state: SignInState,
    /// The sign-in page the CLI showed (desktop only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The browser the link was last opened in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opened: Option<Browser>,
    pub started_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl SignInView {
    /// The same without the link (for frames every client gets).
    pub fn public(&self) -> Self {
        Self {
            url: None,
            ..self.clone()
        }
    }
}

/// Where a `setup-token` capture is: [`SignInState`] as the token routes and
/// the account view's `capture` have always said it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CaptureState {
    /// `setup-token` is running.
    Running,
    /// The token was taken and stored.
    Captured,
    /// It ended without a token.
    Failed,
    /// Five minutes passed.
    TimedOut,
    Cancelled,
}

/// A token account's sign-in as the token routes show it (never the token).
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
    /// The same without the URL (for frames every client gets).
    pub fn public(&self) -> Self {
        Self {
            url: None,
            ..self.clone()
        }
    }
}

/// Why a sign-in could not start or take input. Never carries a secret.
#[derive(Debug, thiserror::Error)]
pub enum SignInError {
    #[error("this build cannot run sign-ins in a terminal; run `claude auth login` or `claude setup-token` yourself (a token can be pasted)")]
    Unavailable,
    #[error("no sign-in is running for this account")]
    NotRunning,
    #[error("the sign-in has not shown its link yet")]
    NoLink,
    #[error("the sign-in link Claude Code showed is not an https link to claude.ai, claude.com or anthropic.com; Mother Claude does not open it")]
    LinkRefused,
    #[error("a sign-in for {name:?} is running; finish or cancel it first")]
    Running { account: String, name: String },
    #[error("could not start `{0}`: {1}")]
    Spawn(&'static str, String),
    #[error("could not type into the sign-in: {0}")]
    Input(String),
    #[error("{0}")]
    Invalid(String),
}

impl SignInError {
    /// A stable code for API errors (`fallback` for a failure to start, which
    /// each route has always named its own way).
    pub fn code(&self, fallback: &'static str) -> &'static str {
        match self {
            Self::Unavailable => "sign_in_unavailable",
            Self::NotRunning => "sign_in_not_running",
            Self::NoLink => "sign_in_no_link",
            Self::LinkRefused => "invalid_sign_in_url",
            Self::Running { .. } => "sign_in_running",
            Self::Invalid(_) => "invalid_body",
            Self::Spawn(..) | Self::Input(_) => fallback,
        }
    }
}

/// What to run: the program, its arguments, its working directory and the
/// changes to the inherited environment, in order (`None`: removed).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct RunSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, Option<OsString>)>,
}

impl std::fmt::Debug for RunSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The variables' names only: a value is never printed.
        let env: Vec<String> = self
            .env
            .iter()
            .map(|(k, v)| format!("{}{k}", if v.is_some() { "+" } else { "-" }))
            .collect();
        f.debug_struct("RunSpec")
            .field("program", &self.program)
            .field("args", &self.args)
            .field("cwd", &self.cwd)
            .field("env", &env)
            .finish()
    }
}

impl RunSpec {
    /// The last change to `key`: `Some(Some(v))` set, `Some(None)` removed,
    /// `None` inherited as it is.
    pub fn env_change(&self, key: &str) -> Option<Option<&std::ffi::OsStr>> {
        self.env
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_deref())
    }
}

impl super::EnvTarget for RunSpec {
    fn set_var(&mut self, key: &str, value: &std::ffi::OsStr) {
        self.env.push((key.to_string(), Some(value.to_os_string())));
    }
    fn remove_var(&mut self, key: &str) {
        self.env.push((key.to_string(), None));
    }
}

/// A run's view and what only the run itself needs.
#[derive(Debug)]
struct RunState {
    view: SignInView,
    link: Option<Link>,
    /// The screen showed a sign-in link Mother Claude may not open.
    link_refused: bool,
    /// The default browser is still to be opened once the link is known.
    auto_open: bool,
    timed_out: bool,
    /// A code was typed since the screen asked for one (its prompt stays on
    /// the screen, asking for nothing more).
    code_typed: bool,
    /// The last line the screen showed (masked), for a run that stops.
    last_line: Option<String>,
}

/// One running (or finished) sign-in, bound to its account from the start.
pub struct SignInJob {
    /// The account it signs in: the only one it can store into.
    pub account: String,
    pub kind: SignInKind,
    run: Mutex<RunState>,
    #[cfg(feature = "token-capture")]
    writer: Mutex<Option<Box<dyn std::io::Write + Send>>>,
    #[cfg(feature = "token-capture")]
    child: Mutex<Option<Box<dyn portable_pty::Child + Send + Sync>>>,
}

impl std::fmt::Debug for SignInJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignInJob")
            .field("account", &self.account)
            .field("kind", &self.kind)
            .field("state", &lock(&self.run).view.state)
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl SignInJob {
    fn new(account: &str, kind: SignInKind, auto_open: bool) -> Self {
        Self {
            account: account.to_string(),
            kind,
            run: Mutex::new(RunState {
                view: SignInView {
                    kind,
                    state: SignInState::Starting,
                    url: None,
                    opened: None,
                    started_at: now_ms(),
                    finished_at: None,
                    error: None,
                },
                link: None,
                link_refused: false,
                auto_open,
                timed_out: false,
                code_typed: false,
                last_line: None,
            }),
            #[cfg(feature = "token-capture")]
            writer: Mutex::new(None),
            #[cfg(feature = "token-capture")]
            child: Mutex::new(None),
        }
    }

    pub fn view(&self) -> SignInView {
        lock(&self.run).view.clone()
    }

    /// The view as the token routes show it.
    pub fn capture_view(&self) -> CaptureView {
        let run = lock(&self.run);
        let v = &run.view;
        CaptureView {
            state: match v.state {
                SignInState::Starting | SignInState::Open | SignInState::Code => {
                    CaptureState::Running
                }
                SignInState::Done => CaptureState::Captured,
                SignInState::Failed if run.timed_out => CaptureState::TimedOut,
                SignInState::Failed => CaptureState::Failed,
                SignInState::Cancelled => CaptureState::Cancelled,
            },
            url: v.url.clone(),
            asks_for_code: v.state == SignInState::Code,
            error: v.error.clone(),
            started_at: v.started_at,
            finished_at: v.finished_at,
        }
    }

    /// The link to open, while the run runs: the one the CLI showed, if
    /// Mother Claude may open it.
    pub fn open_link(&self) -> Result<Link, SignInError> {
        let run = lock(&self.run);
        if !run.view.state.is_running() {
            return Err(SignInError::NotRunning);
        }
        match (&run.link, run.link_refused) {
            (Some(link), _) => Ok(link.clone()),
            (None, true) => Err(SignInError::LinkRefused),
            (None, false) => Err(SignInError::NoLink),
        }
    }

    pub fn is_running(&self) -> bool {
        lock(&self.run).view.state.is_running()
    }

    /// End the run with `state` (only while it runs).
    fn finish(&self, state: SignInState, error: Option<String>) -> bool {
        let mut run = lock(&self.run);
        if !run.view.state.is_running() {
            return false;
        }
        run.view.state = state;
        run.view.error = error;
        run.view.finished_at = Some(now_ms());
        true
    }

    /// The verdict on a run whose CLI is done (a login checked with `claude
    /// auth status`, a token stored): `Done`, or `Failed` with why.
    pub fn complete(&self, verdict: Result<(), String>) -> bool {
        match verdict {
            Ok(()) => self.finish(SignInState::Done, None),
            Err(e) => self.finish(SignInState::Failed, Some(e)),
        }
    }

    /// The link is known and the run may open the default browser by itself
    /// (a login, once): the link to open.
    pub fn take_auto_open(&self) -> Option<Link> {
        let mut run = lock(&self.run);
        if !run.auto_open || !run.view.state.is_running() {
            return None;
        }
        let link = run.link.clone()?;
        run.auto_open = false;
        Some(link)
    }

    /// The link was opened in `browser`.
    pub fn note_opened(&self, browser: Browser) {
        lock(&self.run).view.opened = Some(browser);
    }

    /// The screen changed: the link (or one refused), the code prompt, the
    /// last line. Returns whether the view changed.
    fn saw(
        &self,
        link: Option<&Link>,
        refused: bool,
        asks_for_code: bool,
        last_line: Option<String>,
    ) -> bool {
        let mut run = lock(&self.run);
        run.last_line = last_line;
        run.link_refused |= refused;
        if !run.view.state.is_running() {
            return false;
        }
        let before = (run.view.state, run.view.url.clone());
        if run.link.is_none() {
            if let Some(link) = link {
                run.link = Some(link.clone());
                run.view.url = Some(link.as_str().to_string());
                if run.view.state == SignInState::Starting {
                    run.view.state = SignInState::Open;
                }
            }
        }
        if asks_for_code && run.view.state != SignInState::Code && !run.code_typed {
            run.view.state = SignInState::Code;
        }
        before != (run.view.state, run.view.url.clone())
    }

    /// Type the code the browser shows into the screen.
    pub fn submit_code(&self, code: &str) -> Result<(), SignInError> {
        let code: String = code.trim().chars().filter(|c| !c.is_control()).collect();
        if code.is_empty() || code.len() > CODE_MAX {
            return Err(SignInError::Invalid(
                "the code is empty or too long".to_string(),
            ));
        }
        if !self.is_running() {
            return Err(SignInError::NotRunning);
        }
        #[cfg(feature = "token-capture")]
        {
            let mut w = lock(&self.writer);
            let w = w.as_mut().ok_or(SignInError::NotRunning)?;
            w.write_all(code.as_bytes())
                .and_then(|_| w.write_all(b"\r"))
                .and_then(|_| w.flush())
                .map_err(|e| SignInError::Input(e.kind().to_string()))?;
            let mut run = lock(&self.run);
            run.code_typed = true;
            if run.view.state == SignInState::Code {
                run.view.state = if run.link.is_some() {
                    SignInState::Open
                } else {
                    SignInState::Starting
                };
            }
            Ok(())
        }
        #[cfg(not(feature = "token-capture"))]
        {
            let _ = code;
            Err(SignInError::Unavailable)
        }
    }

    /// Stop the run (the user closed the panel, or another run is wanted).
    pub fn cancel(&self) -> bool {
        let stopped = self.finish(SignInState::Cancelled, None);
        if stopped {
            self.kill();
        }
        stopped
    }

    /// Stop the CLI and reap it. The child leaves the job here: nothing
    /// signals its pid again, which may since belong to another process.
    fn kill(&self) {
        #[cfg(feature = "token-capture")]
        if let Some(mut child) = lock(&self.child).take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// Store the token the screen showed while the run runs, so a cancel
    /// cannot slip in between; the run ends with the store's verdict.
    /// Returns false (nothing stored) when the run had already ended.
    pub fn store_token(&self, store: impl FnOnce() -> Result<(), String>) -> bool {
        let mut run = lock(&self.run);
        if !run.view.state.is_running() {
            return false;
        }
        let verdict = store();
        run.view.state = if verdict.is_ok() {
            SignInState::Done
        } else {
            SignInState::Failed
        };
        run.view.error = verdict.err();
        run.view.finished_at = Some(now_ms());
        true
    }

    /// The deadlines: no link after `link_timeout`, no end after `timeout`.
    /// Returns whether it stopped the run.
    fn check_deadlines(
        &self,
        elapsed: Duration,
        timeout: Duration,
        link_timeout: Duration,
    ) -> bool {
        let (timed_out, why) = {
            let run = lock(&self.run);
            if !run.view.state.is_running() {
                return false;
            }
            let shown = run
                .last_line
                .as_deref()
                .map(|l| format!(" It showed: {l}"))
                .unwrap_or_default();
            if elapsed >= timeout {
                (
                    true,
                    match self.kind {
                        SignInKind::Token => {
                            "No token after five minutes. Paste one instead.".to_string()
                        }
                        SignInKind::Login => "No sign-in after five minutes.".to_string(),
                    },
                )
            } else if elapsed >= link_timeout && run.link.is_none() {
                let why = if run.link_refused {
                    format!(
                        "`{}` showed a sign-in link that is not an https link to claude.ai, \
                         claude.com or anthropic.com; Mother Claude does not open it.",
                        self.kind.command()
                    )
                } else {
                    format!(
                        "`{}` showed no sign-in link Mother Claude may open.{shown}",
                        self.kind.command()
                    )
                };
                (false, why)
            } else {
                return false;
            }
        };
        if self.finish(SignInState::Failed, Some(why)) {
            lock(&self.run).timed_out = timed_out;
            self.kill();
            return true;
        }
        false
    }
}

/// What a run reports back as it goes (from its own threads).
pub trait SignInSink: Send + Sync + 'static {
    /// The view changed (link seen, code asked for, finished).
    fn changed(&self, job: &Arc<SignInJob>);
    /// The token appeared (a token run): store it into `account`, the run's
    /// own. Returns an error message when it could not be stored (the run
    /// then fails with it). Called with the run held: it must not touch the
    /// job.
    fn token(&self, account: &str, token: Secret) -> Result<(), String>;
    /// A login's CLI exited well: check it and [`SignInJob::complete`] it.
    fn exited(&self, job: &Arc<SignInJob>);
}

/// Start `spec` in a pseudo-terminal for `account`. The run reports to
/// `sink` from its own threads and ends on the token (a token run), on the
/// CLI's exit, on [`SignInJob::cancel`], or at a deadline. `auto_open`: the
/// run may open the default browser on its own once the link is known.
#[cfg(feature = "token-capture")]
pub fn start(
    account: &str,
    kind: SignInKind,
    spec: &RunSpec,
    auto_open: bool,
    sink: Arc<dyn SignInSink>,
    timeout: Duration,
    link_timeout: Duration,
) -> Result<Arc<SignInJob>, SignInError> {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};
    use std::io::Read;

    let spawn_err = |e: String| SignInError::Spawn(kind.command(), e);
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: ROWS,
            cols: COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| spawn_err(e.to_string()))?;
    let mut cmd = CommandBuilder::new(&spec.program);
    cmd.args(&spec.args);
    if let Some(cwd) = &spec.cwd {
        cmd.cwd(cwd);
    }
    for (key, value) in &spec.env {
        match value {
            Some(v) => cmd.env(key, v),
            None => cmd.env_remove(key),
        }
    }
    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| spawn_err(e.to_string()))?;
    drop(pair.slave);
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| spawn_err(e.to_string()))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| spawn_err(e.to_string()))?;
    let job = Arc::new(SignInJob::new(account, kind, auto_open));
    *lock(&job.writer) = Some(writer);
    *lock(&job.child) = Some(child);
    sink.changed(&job);

    // The reader: until EOF (the CLI exited or was killed).
    let reading = job.clone();
    let reader_sink = sink.clone();
    let master = pair.master;
    std::thread::Builder::new()
        .name("sign-in".into())
        .spawn(move || {
            // The master end stays open while we read.
            let _master = master;
            let mut screen = Screen::default();
            let mut buf = [0u8; 8192];
            loop {
                let n = match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let found = screen.feed(&buf[..n]);
                if found && reading.kind == SignInKind::Token {
                    if let Some(token) = screen.token.take() {
                        let account = reading.account.clone();
                        if reading.store_token(|| reader_sink.token(&account, token)) {
                            reader_sink.changed(&reading);
                        }
                        // It exits on its own half a second later.
                        reading.kill();
                        break;
                    }
                }
                if reading.saw(
                    screen.url.as_ref(),
                    screen.refused_url,
                    screen.asks_for_code,
                    screen.last_line(),
                ) {
                    reader_sink.changed(&reading);
                }
            }
            lock(&reading.writer).take();
            let exited_well = wait_exit(&reading, Duration::from_secs(5));
            if !reading.is_running() {
                return;
            }
            if reading.kind == SignInKind::Login && exited_well {
                // Whether it signed in is for `claude auth status` to say.
                reader_sink.exited(&reading);
                return;
            }
            let what = match reading.kind {
                SignInKind::Token => "ended without a token",
                SignInKind::Login => "ended without a sign-in",
            };
            let why = screen
                .last_line()
                .map(|l| format!("`{}` {what}: {l}", reading.kind.command()))
                .unwrap_or_else(|| format!("`{}` {what}.", reading.kind.command()));
            if reading.finish(SignInState::Failed, Some(why)) {
                reader_sink.changed(&reading);
            }
        })
        .map_err(|e| spawn_err(e.to_string()))?;

    // The deadlines.
    let timing = job.clone();
    std::thread::Builder::new()
        .name("sign-in-timeout".into())
        .spawn(move || {
            let started = std::time::Instant::now();
            while timing.is_running() {
                std::thread::sleep(Duration::from_millis(250));
                if timing.check_deadlines(started.elapsed(), timeout, link_timeout) {
                    sink.changed(&timing);
                    return;
                }
            }
        })
        .map_err(|e| spawn_err(e.to_string()))?;
    Ok(job)
}

/// After the screen closed: whether the CLI exited well, given `grace` to
/// exit (then it is killed). A reaped child leaves the job at once, so
/// nothing signals its pid again.
#[cfg(feature = "token-capture")]
fn wait_exit(job: &SignInJob, grace: Duration) -> bool {
    let deadline = std::time::Instant::now() + grace;
    loop {
        let mut guard = lock(&job.child);
        let Some(child) = guard.as_mut() else {
            return false;
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                guard.take();
                return status.success();
            }
            Ok(None) if std::time::Instant::now() < deadline => {
                drop(guard);
                std::thread::sleep(Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                guard.take();
                return false;
            }
        }
    }
}

/// Without the `token-capture` feature no sign-in runs here (a token can
/// still be pasted).
#[cfg(not(feature = "token-capture"))]
pub fn start(
    _account: &str,
    _kind: SignInKind,
    _spec: &RunSpec,
    _auto_open: bool,
    _sink: Arc<dyn SignInSink>,
    _timeout: Duration,
    _link_timeout: Duration,
) -> Result<Arc<SignInJob>, SignInError> {
    Err(SignInError::Unavailable)
}

/// Whether this build can run sign-ins in a terminal.
pub const AVAILABLE: bool = cfg!(feature = "token-capture");

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "sk-ant-oat01-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789_-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-AbCdEfGhIjKlMnOp";
    const URL: &str = "https://claude.ai/oauth/authorize?code=true&client_id=x&state=abc";

    /// What Ink prints, roughly: colours, cursor moves, an OSC 8 link, the
    /// token in the warning colour.
    fn ink_screen(token: &str) -> Vec<u8> {
        format!(
            "\x1b[?25l\x1b[2K\x1b[1A\x1b[2K\x1b[G\x1b[2mBrowser didn't open? Use the url below to sign in\x1b[22m\n\
             \x1b]8;;{URL}\x1b\\\x1b[2m{URL}\x1b[22m\x1b]8;;\x1b\\\n\
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
        assert_eq!(screen.url.as_ref().map(Link::as_str), Some(URL));
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
        // The login's own words for it.
        let mut screen = Screen::default();
        screen.feed(
            b"The browser can't return the sign-in to Claude Code automatically. \
              After you sign in, paste the code shown.\r\n",
        );
        assert!(screen.asks_for_code);
    }

    /// `claude auth login` in a terminal (CLI 2.1.312): plain lines, the link
    /// as text, a frame drawn right after it.
    #[test]
    fn a_logins_link_is_read_from_plain_text() {
        let mut screen = Screen::default();
        screen.feed(
            format!("Opening browser to sign in\u{2026}\r\nIf the browser didn't open, visit: {URL}\u{2502}\r\n")
                .as_bytes(),
        );
        assert_eq!(screen.url.as_ref().map(Link::as_str), Some(URL));
        assert!(screen.token.is_none());
    }

    #[test]
    fn a_link_to_anywhere_else_is_never_taken() {
        for screen_text in [
            "Sign in at: https://evil.example/oauth/authorize?state=x\r\n",
            "Sign in at: http://claude.ai/oauth/authorize?state=x\r\n",
            "Sign in at: https://claude.ai.evil.example/oauth/authorize\r\n",
            "\x1b]8;;https://evil.example/oauth/authorize\x1b\\click\x1b]8;;\x1b\\\r\n",
        ] {
            let mut screen = Screen::default();
            screen.feed(screen_text.as_bytes());
            assert!(screen.url.is_none(), "{screen_text:?}");
            assert!(screen.refused_url, "{screen_text:?}");
        }
        // Other links are neither taken nor refused.
        let mut screen = Screen::default();
        screen.feed(b"Docs: https://evil.example/docs and http://localhost:5454/callback\r\n");
        assert!(screen.url.is_none() && !screen.refused_url);
        // The good one after a bad one is taken.
        let mut screen = Screen::default();
        screen.feed(format!("https://evil.example/oauth/authorize then {URL}\r\n").as_bytes());
        assert_eq!(screen.url.as_ref().map(Link::as_str), Some(URL));
    }

    /// A URL cut by a chunk boundary is not the link yet: half an OAuth URL
    /// parses fine and would send the browser nowhere.
    #[test]
    fn a_link_split_across_chunks_is_taken_only_when_complete() {
        let line = format!("If the browser didn't open, visit: {URL}\r\n");
        let at = line.find("/oauth/auth").unwrap() + 8;
        let mut screen = Screen::default();
        screen.feed(&line.as_bytes()[..at]);
        assert!(screen.url.is_none(), "half a link is not a link");
        assert!(!screen.refused_url);
        screen.feed(&line.as_bytes()[at..]);
        assert_eq!(screen.url.as_ref().map(Link::as_str), Some(URL));
        // A long one too (the rescan reaches back far enough).
        let long = format!("{URL}&pad={}", "a".repeat(3000));
        let line = format!("visit: {long}\r\n");
        let mut screen = Screen::default();
        for chunk in line.as_bytes().chunks(500) {
            screen.feed(chunk);
        }
        assert_eq!(screen.url.as_ref().map(Link::as_str), Some(long.as_str()));
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
    fn the_last_line_never_carries_a_token_or_a_link() {
        let mut screen = Screen::default();
        screen.feed(format!("Error: bad thing {}\n", &TOKEN[..30]).as_bytes());
        let line = screen.last_line().unwrap();
        assert!(!line.contains("sk-ant-"), "{line}");
        assert!(line.starts_with("Error: bad thing"));
        let mut screen = Screen::default();
        screen.feed(format!("visit: {URL} or http://x.example now\n").as_bytes());
        assert_eq!(
            screen.last_line().unwrap(),
            "visit: <link> or <link> now",
            "the link is the desktop's alone"
        );
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
    fn a_runs_states_follow_the_screen() {
        let job = SignInJob::new("acc_000000000001", SignInKind::Login, true);
        assert_eq!(job.view().state, SignInState::Starting);
        assert!(job.take_auto_open().is_none(), "no link yet");
        assert!(matches!(job.open_link(), Err(SignInError::NoLink)));
        let link = Link::parse(URL).unwrap();
        assert!(job.saw(Some(&link), false, false, None));
        assert_eq!(job.view().state, SignInState::Open);
        assert_eq!(job.view().url.as_deref(), Some(URL));
        assert_eq!(job.take_auto_open(), Some(link.clone()));
        assert!(job.take_auto_open().is_none(), "once");
        assert_eq!(job.open_link().unwrap(), link);
        assert!(job.saw(Some(&link), false, true, None));
        assert_eq!(job.view().state, SignInState::Code);
        assert!(!job.saw(Some(&link), false, true, None), "nothing new");
        assert!(job.complete(Ok(())));
        assert_eq!(job.view().state, SignInState::Done);
        assert!(job.view().finished_at.is_some());
        assert!(!job.cancel(), "a finished run stays as it ended");
        assert!(
            matches!(job.open_link(), Err(SignInError::NotRunning)),
            "nothing to open any more"
        );
        assert!(matches!(
            job.submit_code("abc"),
            Err(SignInError::NotRunning)
        ));
    }

    #[test]
    fn a_refused_link_is_never_opened_and_says_so() {
        let job = SignInJob::new("acc_000000000005", SignInKind::Login, true);
        assert!(!job.saw(None, true, false, None), "nothing to show");
        assert!(job.take_auto_open().is_none());
        assert!(matches!(job.open_link(), Err(SignInError::LinkRefused)));
        assert_eq!(SignInError::LinkRefused.code("x"), "invalid_sign_in_url");
        assert!(job.check_deadlines(LINK_TIMEOUT, SIGN_IN_TIMEOUT, LINK_TIMEOUT));
        let e = job.view().error.unwrap();
        assert!(e.contains("not an https link to claude.ai"), "{e}");
    }

    #[test]
    fn a_token_run_never_opens_on_its_own_and_reads_as_a_capture() {
        let job = SignInJob::new("acc_000000000002", SignInKind::Token, false);
        job.saw(Some(&Link::parse(URL).unwrap()), false, true, None);
        assert!(job.take_auto_open().is_none());
        let c = job.capture_view();
        assert_eq!(c.state, CaptureState::Running);
        assert!(c.asks_for_code);
        assert_eq!(c.url.as_deref(), Some(URL));
        assert!(job.check_deadlines(SIGN_IN_TIMEOUT, SIGN_IN_TIMEOUT, LINK_TIMEOUT));
        assert_eq!(job.capture_view().state, CaptureState::TimedOut);
        assert_eq!(job.view().state, SignInState::Failed);
        let json = serde_json::to_value(job.capture_view().public()).unwrap();
        assert_eq!(json["state"], "timedOut");
        assert!(json.get("url").is_none());
    }

    #[test]
    fn a_run_without_a_link_stops_after_a_minute() {
        let job = SignInJob::new("acc_000000000003", SignInKind::Login, true);
        job.saw(None, false, false, Some("Select a method".into()));
        assert!(!job.check_deadlines(Duration::from_secs(59), SIGN_IN_TIMEOUT, LINK_TIMEOUT));
        assert!(job.check_deadlines(LINK_TIMEOUT, SIGN_IN_TIMEOUT, LINK_TIMEOUT));
        let v = job.view();
        assert_eq!(v.state, SignInState::Failed);
        let e = v.error.unwrap();
        assert!(
            e.contains("showed no sign-in link") && e.contains("Select a method"),
            "{e}"
        );
    }

    #[test]
    fn views_for_everyone_have_no_url() {
        let job = SignInJob::new("acc_000000000004", SignInKind::Login, false);
        job.saw(Some(&Link::parse(URL).unwrap()), false, false, None);
        job.note_opened(Browser::ChromePrivate);
        let json = serde_json::to_value(job.view().public()).unwrap();
        assert_eq!(json["state"], "open");
        assert_eq!(json["kind"], "login");
        assert_eq!(json["opened"], "chrome-private");
        assert!(json.get("url").is_none());
        assert_eq!(
            serde_json::to_value(job.view()).unwrap()["url"],
            URL,
            "the desktop's own read has it"
        );
    }

    #[test]
    fn a_run_spec_never_prints_a_value() {
        let mut spec = RunSpec {
            program: "claude".into(),
            ..Default::default()
        };
        super::super::EnvTarget::set_var(
            &mut spec,
            "SECRET_ISH",
            std::ffi::OsStr::new("value-123"),
        );
        super::super::EnvTarget::remove_var(&mut spec, "GONE");
        let printed = format!("{spec:?}");
        assert!(!printed.contains("value-123"), "{printed}");
        assert!(printed.contains("+SECRET_ISH") && printed.contains("-GONE"));
        assert_eq!(
            spec.env_change("SECRET_ISH"),
            Some(Some(std::ffi::OsStr::new("value-123")))
        );
        assert_eq!(spec.env_change("GONE"), Some(None));
        assert_eq!(spec.env_change("OTHER"), None);
    }
}
