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
//! is what its exit status and `claude auth status` say afterwards.
//!
//! The link either screen shows is the manual one (`code=true`, sent back to
//! Claude's code page; CLI 2.1.314): the browser ends on a page that shows a
//! code and never hands the sign-in back by itself, so every run finishes
//! with that code ("Paste code here if prompted"), which the desktop types
//! into the screen for the user — as one bracketed paste where the CLI takes
//! pastes. A code the CLI turns down comes back with its words; where
//! `setup-token` offers to start over, Enter is pressed for it (the same page
//! after a code cut short, a new link after one it did not accept).
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
    /// The program turned bracketed paste on (`CSI ? 2004 h`): a paste
    /// into it goes between `CSI 200 ~` and `CSI 201 ~`.
    pub bracketed_paste: bool,
    /// Cursor position queries (`CSI 6 n`) not answered yet. Windows' ConPTY
    /// sends one as it starts and passes none of the program's output on
    /// until the terminal answers it: unanswered, a sign-in never shows its
    /// link.
    pub cursor_queries: usize,
}

/// The answer to a cursor position query: the top left corner.
pub const CURSOR_REPORT: &[u8] = b"\x1b[1;1R";

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
                        match b {
                            // Cursor forward stands in for spaces in some renderers.
                            b'C' => {
                                let n = std::str::from_utf8(&self.csi)
                                    .ok()
                                    .and_then(|s| s.parse::<usize>().ok())
                                    .unwrap_or(1)
                                    .min(500);
                                out.extend(std::iter::repeat_n(' ', n));
                            }
                            // Claude Code's renderer writes only the cells that
                            // change and moves along the line over the rest, so
                            // the words of a line drawn on blank cells are placed
                            // one by one (CHA, HPA, HPR): a move ends a word.
                            b'G' | b'`' | b'a' => out.push(' '),
                            // A cursor jump (VPA too) or erase ends the visible run.
                            b'H' | b'f' | b'E' | b'F' | b'J' | b'K' | b'A' | b'B' | b'd' => {
                                out.push('\n')
                            }
                            // Bracketed paste on or off (`?2004`, alone or
                            // among other modes).
                            b'h' | b'l' => {
                                let modes = self.csi.strip_prefix(b"?").unwrap_or_default();
                                if modes.split(|c| *c == b';').any(|m| m == b"2004") {
                                    self.bracketed_paste = b == b'h';
                                }
                            }
                            // Device status report: where is the cursor?
                            b'n' if self.csi == b"6" => {
                                self.cursor_queries = self.cursor_queries.saturating_add(1);
                            }
                            _ => {}
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

/// Text holding a token or a key (`sk-ant-…`), which is never typed into a
/// sign-in.
fn looks_like_token(text: &str) -> bool {
    text.to_ascii_lowercase().contains("sk-ant-")
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
    /// The last sign-in link Mother Claude may open among those looked at.
    pub link: Option<Link>,
    /// A link that looked like a sign-in page was refused: not `https`, or
    /// not one of Anthropic's hosts ([`Link`]).
    pub refused: bool,
}

/// The sign-in link in what the screen just showed: OSC 8 link targets
/// (`links`) and visible `http(s)://` text naming an OAuth authorize page,
/// taken only when Mother Claude may open it ([`Link`]). A link to anywhere
/// else is never taken, so it is never shown or opened; that one was seen is
/// `refused`. Visible text counts once the line goes on past it there, in
/// `text[fresh..]` (what was just added): a URL cut by a chunk boundary is
/// not one yet, and one looked at before is not looked at again. The last
/// one wins: `setup-token` shows a new link when it starts over.
pub fn find_url(links: &[String], text: &str, fresh: usize) -> LinkScan {
    let mut scan = LinkScan::default();
    let mut look = |u: &str| {
        if !(u.contains("oauth") || u.contains("authorize")) {
            return;
        }
        match Link::parse(u) {
            Some(link) => scan.link = Some(link),
            None => scan.refused = true,
        }
    };
    for u in links {
        look(u);
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
        if start + len >= fresh {
            look(&text[start..start + len]);
        }
        // Never a link inside this one.
        from = start + len;
    }
    scan
}

/// How far back each new piece of the screen is scanned from: a whole link
/// ([`crate::claude::browser`] opens none longer) and then some.
const RESCAN: usize = 4200;
/// The longest line passed on (the last one, a refusal).
const LINE_MAX: usize = 200;
/// How the CLI asks for the code the browser shows (CLI 2.1.314): "Paste
/// code here if prompted >", and "… paste the code shown." where it knows
/// the browser cannot hand the sign-in back.
const CODE_PROMPTS: [&str; 2] = ["paste code", "paste the code"];
/// How it turns a code down (CLI 2.1.314): `setup-token` shows "OAuth error:
/// …" above "Press Enter to retry."; `auth login` says "Invalid code. …" and
/// asks again, or "Login failed: …" and exits.
const REFUSALS: [&str; 4] = [
    "oauth error",
    "login failed",
    "invalid code",
    "wasn't accepted",
];
/// How `setup-token` offers to start over after an error.
const RETRY_PROMPT: &str = "enter to retry";

/// What the screen showed, as far as a sign-in cares. A prompt for the code,
/// a refusal and an offer to start over are counted when they appear, so a
/// run takes each in once.
#[derive(Debug, Default)]
pub struct Screen {
    stripper: Stripper,
    text: String,
    /// The latest sign-in link Mother Claude may open: `setup-token` shows a
    /// new one when it starts over after turning a code down.
    pub url: Option<Link>,
    /// A link that looked like the sign-in page was refused.
    pub refused_url: bool,
    pub token: Option<Secret>,
    /// How many times it asked for the code the browser shows.
    pub prompts: usize,
    /// How many times it turned a code down, and its words the last time.
    pub refusals: usize,
    pub refusal: Option<String>,
    /// How many times it offered to start over ("Press Enter to retry").
    pub retries: usize,
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
        let fresh = self.text.len() - rescan_from;
        self.text.push_str(&added);
        let window = &self.text[rescan_from..];
        let scan = find_url(&std::mem::take(&mut self.stripper.links), window, fresh);
        if scan.link.is_some() {
            self.url = scan.link;
        }
        self.refused_url |= scan.refused;
        // Each counted once: when it ends in what was just added.
        let lower = window.to_ascii_lowercase();
        let ended_now = |needle: &str| {
            lower
                .match_indices(needle)
                .filter(|(at, m)| at + m.len() > fresh)
                .count()
        };
        self.prompts += CODE_PROMPTS.iter().map(|p| ended_now(p)).sum::<usize>();
        self.retries += ended_now(RETRY_PROMPT);
        for why in refusals(window, fresh) {
            self.refusals += 1;
            self.refusal = Some(why);
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

    /// It asked for the code the browser shows.
    pub fn asks_for_code(&self) -> bool {
        self.prompts > 0
    }

    /// The CLI takes a paste as one: what is typed into it goes in as a
    /// bracketed paste.
    pub fn bracketed_paste(&self) -> bool {
        self.stripper.bracketed_paste
    }

    /// The cursor position queries the terminal still owes an answer
    /// ([`CURSOR_REPORT`] each), counted off as they are taken.
    pub fn take_cursor_queries(&mut self) -> usize {
        std::mem::take(&mut self.stripper.cursor_queries)
    }

    /// The last visible line, for "it ended without a token" (masked).
    pub fn last_line(&self) -> Option<String> {
        let line = self
            .text
            .lines()
            .map(str::trim)
            .rfind(|l| l.chars().any(|c| c.is_alphanumeric()))?;
        Some(mask(line))
    }
}

/// The lines turning a code down that ended in `text[fresh..]` (a line ends
/// at its break: a message cut by a read is not one yet), in the CLI's words.
fn refusals(text: &str, fresh: usize) -> impl Iterator<Item = String> + '_ {
    let first = text[..fresh].rfind('\n').map_or(0, |p| p + 1);
    text[first..]
        .split_inclusive('\n')
        .filter_map(|l| l.strip_suffix('\n'))
        .filter_map(refusal_words)
}

/// A line's words when it turns a code down: past the prompt on the same
/// line (`auth login` never ends the prompt's line, and nothing the user
/// types shows), without the "OAuth error:" kind of lead (masked).
fn refusal_words(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    if !REFUSALS.iter().any(|r| lower.contains(r)) {
        return None;
    }
    let from = CODE_PROMPTS
        .iter()
        .filter_map(|p| lower.find(p))
        .min()
        .and_then(|p| lower[p..].find('>').map(|gt| p + gt + 1))
        .unwrap_or(0);
    let words = line[from..].trim();
    let lower = words.to_ascii_lowercase();
    let words = ["oauth error:", "login failed:", "error:"]
        .iter()
        .find(|lead| lower.starts_with(*lead))
        .map_or(words, |lead| words[lead.len()..].trim_start());
    (!words.is_empty()).then(|| mask(words))
}

/// A line as every client may see it: anything that looks like a token or a
/// link, or the code typed as the CLI echoes it (`*`s and its last
/// characters), is masked — the link is the desktop's alone.
fn mask(line: &str) -> String {
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
    masked
        .split(' ')
        .map(|w| if w.starts_with("***") { "***" } else { w })
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(LINE_MAX)
        .collect()
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
    /// A code was typed in and Claude Code has not answered yet.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub code_sent: bool,
    /// Why Claude Code turned the last code typed down, in its words (masked
    /// like `error`); there until another code is typed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_error: Option<String>,
    /// Counts the run's changes: of two reads of one run, the one with the
    /// higher `rev` is the later (its state can go back from `code` to
    /// `open`).
    pub rev: u32,
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
    /// The screen's prompts for the code, refusals and offers to start over
    /// taken in so far ([`Screen`] counts them).
    prompts: usize,
    refusals: usize,
    retries: usize,
    /// The CLI turned the code typed down: Enter is pressed once where it
    /// offers to start over.
    retry_owed: bool,
    /// The CLI turned bracketed paste on: a code goes in as one paste.
    bracketed_paste: bool,
    /// The last line the screen showed (masked), for a run that stops.
    last_line: Option<String>,
}

/// What a look at the screen asks of the run.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Saw {
    /// The view changed: clients are told.
    changed: bool,
    /// Press Enter: the CLI turned the code down and offers to start over.
    retry: bool,
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
                    code_sent: false,
                    code_error: None,
                    rev: 0,
                },
                link: None,
                link_refused: false,
                auto_open,
                timed_out: false,
                prompts: 0,
                refusals: 0,
                retries: 0,
                retry_owed: false,
                bracketed_paste: false,
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
            asks_for_code: v.state == SignInState::Code && !v.code_sent,
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
        run.view.code_sent = false;
        run.view.rev += 1;
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
        let mut run = lock(&self.run);
        if run.view.opened != Some(browser) {
            run.view.opened = Some(browser);
            run.view.rev += 1;
        }
    }

    /// The screen changed: a refusal of the code typed (the field comes back
    /// with why), an offer to start over (Enter is pressed once for a
    /// refusal), the link (a new one: it started over), a prompt for the
    /// code, bracketed paste, the last line.
    fn saw(&self, screen: &Screen) -> Saw {
        let mut run = lock(&self.run);
        run.last_line = screen.last_line();
        run.link_refused |= screen.refused_url;
        run.bracketed_paste = screen.bracketed_paste();
        if !run.view.state.is_running() {
            return Saw::default();
        }
        let before = run.view.clone();
        if screen.refusals > run.refusals {
            run.refusals = screen.refusals;
            if run.view.code_sent {
                run.view.code_sent = false;
                run.view.code_error = screen.refusal.clone();
                run.view.state = SignInState::Code;
                run.retry_owed = true;
            }
        }
        let mut retry = false;
        if screen.retries > run.retries {
            run.retries = screen.retries;
            retry = std::mem::take(&mut run.retry_owed);
        }
        if let Some(link) = screen
            .url
            .as_ref()
            .filter(|l| run.link.as_ref() != Some(*l))
        {
            if run.link.is_some() {
                // It started over: a new page to open, no code typed for it.
                run.view.opened = None;
                run.view.code_sent = false;
                run.view.state = SignInState::Open;
            } else if run.view.state == SignInState::Starting {
                run.view.state = SignInState::Open;
            }
            run.link = Some(link.clone());
            run.view.url = Some(link.as_str().to_string());
        }
        if screen.prompts > run.prompts {
            run.prompts = screen.prompts;
            if !run.view.code_sent {
                run.view.state = SignInState::Code;
            }
        }
        let changed = run.view != before;
        if changed {
            run.view.rev += 1;
        }
        Saw { changed, retry }
    }

    /// Type the code the browser shows into the screen. It counts as sent
    /// before it goes in (the CLI may answer at once) until the CLI turns it
    /// down, starts over or the run ends. A token is never typed in: the CLI
    /// would take it for a code (the refusal never repeats it).
    pub fn submit_code(&self, code: &str) -> Result<(), SignInError> {
        let code: String = code.trim().chars().filter(|c| !c.is_control()).collect();
        if code.is_empty() || code.len() > CODE_MAX {
            return Err(SignInError::Invalid(
                "the code is empty or too long".to_string(),
            ));
        }
        if looks_like_token(&code) {
            return Err(SignInError::Invalid(
                "that is a token (sk-ant-…), not the code the browser shows; it was not typed in"
                    .to_string(),
            ));
        }
        if !self.is_running() {
            return Err(SignInError::NotRunning);
        }
        #[cfg(feature = "token-capture")]
        {
            let mut w = lock(&self.writer);
            let w = w.as_mut().ok_or(SignInError::NotRunning)?;
            let keys = {
                let mut run = lock(&self.run);
                if !run.view.state.is_running() {
                    return Err(SignInError::NotRunning);
                }
                run.view.code_sent = true;
                run.view.code_error = None;
                run.retry_owed = false;
                if run.view.state == SignInState::Code {
                    run.view.state = if run.link.is_some() {
                        SignInState::Open
                    } else {
                        SignInState::Starting
                    };
                }
                run.view.rev += 1;
                keys_for(&code, run.bracketed_paste)
            };
            if let Err(e) = w.write_all(&keys).and_then(|_| w.flush()) {
                let mut run = lock(&self.run);
                if run.view.code_sent {
                    run.view.code_sent = false;
                    run.view.rev += 1;
                }
                return Err(SignInError::Input(e.kind().to_string()));
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

    /// The run's account got its token another way (pasted): a token run
    /// still going has what it was for, so it ends done and its CLI stops.
    pub fn got_token(&self) -> bool {
        if self.kind != SignInKind::Token || !self.finish(SignInState::Done, None) {
            return false;
        }
        self.kill();
        true
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

    /// Take in what the screen shows ([`Self::saw`]) and press Enter where
    /// the CLI offers to start over after turning the code down
    /// (`setup-token`: the same page after a code cut short, a new one after
    /// a code it did not accept). The writer is held throughout, so a code
    /// typed meanwhile goes in after that Enter. Returns whether the view
    /// changed.
    #[cfg(feature = "token-capture")]
    fn take_in(&self, screen: &Screen) -> bool {
        let mut w = lock(&self.writer);
        let saw = self.saw(screen);
        if let Some(w) = w.as_mut().filter(|_| saw.retry) {
            let _ = w.write_all(b"\r").and_then(|_| w.flush());
        }
        saw.changed
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
        run.view.code_sent = false;
        run.view.rev += 1;
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

/// The keys a code goes in as: the code and Enter, the code as one paste
/// while the CLI has bracketed paste on. `setup-token`'s input (CLI 2.1.314)
/// took a long code and its Enter arriving in one read for pasted text and
/// never submitted it; a bracketed paste ends where the code does.
#[cfg(feature = "token-capture")]
fn keys_for(code: &str, bracketed_paste: bool) -> Vec<u8> {
    let mut keys = Vec::with_capacity(code.len() + 13);
    if bracketed_paste {
        keys.extend_from_slice(b"\x1b[200~");
    }
    keys.extend_from_slice(code.as_bytes());
    if bracketed_paste {
        keys.extend_from_slice(b"\x1b[201~");
    }
    keys.push(b'\r');
    keys
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
                // Answer the terminal's questions at once: on Windows the
                // program's output waits for this one.
                let queries = screen.take_cursor_queries();
                if queries > 0 {
                    use std::io::Write;
                    if let Some(w) = lock(&reading.writer).as_mut() {
                        for _ in 0..queries {
                            let _ = w.write_all(CURSOR_REPORT);
                        }
                        let _ = w.flush();
                    }
                }
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
                if reading.take_in(&screen) {
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
        assert!(!screen.asks_for_code());
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
        assert!(screen.asks_for_code());
        // The login's own words for it.
        let mut screen = Screen::default();
        screen.feed(
            b"The browser can't return the sign-in to Claude Code automatically. \
              After you sign in, paste the code shown.\r\n",
        );
        assert!(screen.asks_for_code());
    }

    /// The link `claude setup-token` shows (CLI 2.1.314; its client id,
    /// challenge and state made up here).
    const SETUP_URL: &str = "https://claude.com/cai/oauth/authorize?code=true&client_id=9d1c250a-e61b-44d9-88ed-5944d1962f5e&response_type=code&redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback&scope=user%3Ainference&code_challenge=Xq3vYz0123456789abcdefghijklmnopqrstuvwxyzA&code_challenge_method=S256&state=Rk1yZ0123456789abcdefghijklmnopqrstuvwxyzAB";
    /// Its prompt for the code, as drawn: every word placed with CHA.
    const SETUP_PROMPT: &[u8] =
        b"\x1b[2GPaste\x1b[8Gcode\x1b[13Ghere\x1b[18Gif\x1b[21Gprompted\x1b[30G>\r\r\n";

    /// `claude setup-token` in a pseudo-terminal (CLI 2.1.314), as it wrote
    /// it up to the prompt: words on blank cells placed one by one with CHA,
    /// terminal queries, a spinner redrawn in place, and the link as an OSC 8
    /// target (with an `id`, ended by BEL) and as text.
    fn setup_token_screen() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend_from_slice(b"\x1b7\x1b[r\x1b8\x1b[?25h\x1b[?25l\x1b[?2004h\x1b[?2031h\x1b[?1004h");
        s.extend_from_slice(
            b"\x1b[38;5;174mWelcome\x1b[9Gto\x1b[12GClaude\x1b[19GCode\x1b[24G\
              \x1b[38;5;246mv2.1.314\x1b[39m\r\r\n",
        );
        s.extend_from_slice(
            b"\x1b[2G\x1b[1mThis\x1b[7Gwill\x1b[12Gguide\x1b[18Gyou\x1b[22Gthrough\
              \x1b[30Glong-lived\x1b[41G(1-year)\x1b[50Gauth\x1b[55Gtoken\x1b[61Gsetup\
              \x1b[67Gfor\x1b[71Gyour\x1b[76GClaude\x1b[83Gaccount.\x1b[22m\r\r\n\r\r\n",
        );
        s.extend_from_slice(
            "\x1b[2G\x1b[38;5;231m\u{b7}\x1b[4G\x1b[39mOpening\x1b[12Gbrowser\x1b[20Gto\
             \x1b[23Gsign\x1b[28Gin\u{2026}\r\r\n"
                .as_bytes(),
        );
        s.extend_from_slice(b"\x1b[>0q\x1b[?u\x1b]7501;?\x07\x1b[c");
        for spinner in ['\u{2722}', '\u{2733}', '\u{2736}'] {
            s.extend_from_slice(
                format!("\r\x1b[1C\x1b[1A\x1b[38;5;231m{spinner}\x1b[39m\r\r\n").as_bytes(),
            );
        }
        s.extend_from_slice(
            b"\r\x1b[1C\x1b[1A\x1b[38;5;246mBrowser didn't open? Use the url below to sign in \
              (c to copy)\x1b[39m\r\r\n\r\r\n",
        );
        s.extend_from_slice(
            format!(
                "\x1b]8;id=14i6zrz;{SETUP_URL}\x07\x1b[38;5;246m{SETUP_URL}\x1b[39m\
                 \x1b]8;;\x07\r\r\n\r\r\n\r\r\n"
            )
            .as_bytes(),
        );
        s.extend_from_slice(SETUP_PROMPT);
        s
    }

    /// What it shows once it has the token: the token in its own colour,
    /// the words around it placed one by one.
    fn setup_token_success(token: &str) -> Vec<u8> {
        format!(
            "\x1b[2K\x1b[1A\x1b[2K\x1b[G\x1b[2G\x1b[38;5;114m\u{2713}\x1b[4GLong-lived\
             \x1b[15Gauthentication\x1b[30Gtoken\x1b[36Gcreated\x1b[44Gsuccessfully!\x1b[39m\r\r\n\
             \r\r\n\x1b[2GYour\x1b[7GOAuth\x1b[13Gtoken\x1b[19G(valid\x1b[26Gfor\x1b[30G1\
             \x1b[32Gyear):\r\r\n\r\r\n\x1b[2G\x1b[38;5;220m{token}\x1b[39m\r\r\n"
        )
        .into_bytes()
    }

    /// Words placed one by one end there (CHA before: "Pastecodehere…",
    /// and the prompt was never noticed).
    #[test]
    fn words_placed_one_by_one_are_words() {
        let mut stripper = Stripper::default();
        assert_eq!(
            stripper.feed(SETUP_PROMPT),
            " Paste code here if prompted >\n\n\n"
        );
        // HPA and HPR end a word too; VPA ends the line.
        assert_eq!(
            stripper.feed(b"one\x1b[10`two\x1b[3athree\x1b[5dfour"),
            "one two three\nfour"
        );
    }

    /// Windows' ConPTY asks where the cursor is (`CSI 6 n`) before it passes
    /// any output on (verified with `claude auth login` 2.1.317 on Windows
    /// 11): each query is counted, once, even split across reads, and other
    /// status requests and `n`-final sequences are not.
    #[test]
    fn cursor_position_queries_are_counted_for_an_answer() {
        let mut screen = Screen::default();
        screen.feed(b"\x1b[6n");
        assert_eq!(screen.take_cursor_queries(), 1);
        assert_eq!(screen.take_cursor_queries(), 0, "taken once");
        screen.feed(b"\x1b[");
        screen.feed(b"6");
        screen.feed(b"nOpening browser to sign in\r\n\x1b[5n\x1b[?6n\x1b[16n");
        assert_eq!(screen.take_cursor_queries(), 1, "split across reads");
        // The answer: the top left corner.
        assert_eq!(CURSOR_REPORT, b"\x1b[1;1R");
    }

    /// The whole screen of `claude auth login` in Windows' ConPTY once the
    /// query is answered (CLI 2.1.317): the link is read from it.
    #[test]
    fn a_windows_login_screen_shows_its_link() {
        let mut screen = Screen::default();
        screen.feed(
            b"\x1b[6n\x1b[?9001h\x1b[?1004h\x1b[m\x1b]0;C:\\Users\\x\\claude.EXE\x07\x1b[?25h",
        );
        assert_eq!(screen.take_cursor_queries(), 1);
        screen.feed(b"\x1b]0;claude\x07Opening browser to sign in\xe2\x80\xa6\r\n");
        screen.feed(
            format!(
                "If the browser didn't open, visit: \x1b]8;id=1-1;{URL}\x1b\\{URL}\x1b]8;;\x1b\\\r\n\
                 Paste code here if prompted > "
            )
            .as_bytes(),
        );
        assert_eq!(screen.url.as_ref().map(|l| l.as_str()), Some(URL));
        assert!(screen.asks_for_code());
    }

    #[test]
    fn setup_tokens_screen_asks_for_the_code_and_shows_its_link() {
        let bytes = setup_token_screen();
        let mut screen = Screen::default();
        screen.feed(&bytes);
        assert!(screen.asks_for_code());
        assert_eq!(screen.url.as_ref().map(Link::as_str), Some(SETUP_URL));
        assert!(screen.token.is_none() && !screen.refused_url);
        // Whatever the reads cut the escapes into.
        for size in [1, 2, 3, 7, 64] {
            let mut screen = Screen::default();
            for chunk in bytes.chunks(size) {
                screen.feed(chunk);
            }
            assert!(screen.asks_for_code(), "chunks of {size}");
            assert_eq!(
                screen.url.as_ref().map(Link::as_str),
                Some(SETUP_URL),
                "chunks of {size}"
            );
        }
        // The OSC 8 target alone is enough.
        let mut screen = Screen::default();
        screen
            .feed(format!("\x1b]8;id=1u6bt8l;{SETUP_URL}\x07sign in\x1b]8;;\x07\r\r\n").as_bytes());
        assert_eq!(screen.url.as_ref().map(Link::as_str), Some(SETUP_URL));
    }

    /// The token cut by a read is not taken, now that a move ends a word.
    #[test]
    fn setup_tokens_token_cut_across_chunks_is_taken_only_when_complete() {
        let mut bytes = setup_token_screen();
        bytes.extend_from_slice(&setup_token_success(TOKEN));
        let at = bytes.windows(10).position(|w| w == b"sk-ant-oat").unwrap() + 40;
        let mut screen = Screen::default();
        assert!(!screen.feed(&bytes[..at]), "half a token is not a token");
        assert!(screen.token.is_none());
        assert!(screen.feed(&bytes[at..]));
        assert_eq!(screen.token.unwrap().expose(), TOKEN);
        // A byte at a time, too.
        let mut screen = Screen::default();
        let found: Vec<bool> = bytes.chunks(1).map(|c| screen.feed(c)).collect();
        assert_eq!(found.iter().filter(|f| **f).count(), 1);
        assert_eq!(screen.token.unwrap().expose(), TOKEN);
    }

    /// What is typed into a run, as the CLI reads it.
    #[cfg(feature = "token-capture")]
    #[derive(Clone, Default)]
    struct Keys(Arc<Mutex<Vec<u8>>>);

    #[cfg(feature = "token-capture")]
    impl std::io::Write for Keys {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            lock(&self.0).extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A run that reads what is typed into it.
    #[cfg(feature = "token-capture")]
    fn typed_into(kind: SignInKind) -> (SignInJob, Keys) {
        let job = SignInJob::new("acc_000000000006", kind, false);
        let keys = Keys::default();
        *lock(&job.writer) = Some(Box::new(keys.clone()));
        (job, keys)
    }

    #[test]
    fn the_screen_says_when_it_takes_a_paste_as_one() {
        let mut screen = Screen::default();
        assert!(!screen.bracketed_paste());
        // `setup-token` turns it on first thing (`auth login` never does).
        screen.feed(&setup_token_screen());
        assert!(screen.bracketed_paste());
        screen.feed(b"\x1b[?1004;2004l");
        assert!(!screen.bracketed_paste());
        // Another mode, or the ANSI mode of the same number, is not it.
        screen.feed(b"\x1b[?20045h\x1b[2004h\x1b[?25h");
        assert!(!screen.bracketed_paste());
    }

    /// `setup-token` (CLI 2.1.314) takes a long code and its Enter arriving
    /// in one read for pasted text and submits nothing; bracketed, the
    /// Enter after the paste submits it. `auth login` reads a plain line.
    #[cfg(feature = "token-capture")]
    #[test]
    fn a_code_goes_in_as_one_paste_when_the_cli_takes_pastes() {
        let (job, keys) = typed_into(SignInKind::Login);
        job.submit_code(" abc#def\n").unwrap();
        assert_eq!(*lock(&keys.0), b"abc#def\r");

        let (job, keys) = typed_into(SignInKind::Token);
        let mut screen = Screen::default();
        screen.feed(&setup_token_screen());
        job.saw(&screen);
        assert_eq!(job.view().state, SignInState::Code);
        job.submit_code("abc#def").unwrap();
        assert_eq!(*lock(&keys.0), b"\x1b[200~abc#def\x1b[201~\r");
    }

    /// What `setup-token` (CLI 2.1.314) wrote after a code it did not
    /// accept: the code echoed as `*`s and its last characters, the error,
    /// the offer to start over.
    const SETUP_REFUSAL: &[u8] = b"\r\x1b[31C\x1b[1A**********************************************************rfqo_w\r\r\n\
        \x1b[2K\x1b[1A\x1b[2K\x1b[G\x1b[1A\r\x1b[1C\x1b[4A\x1b[38;5;211mOAuth error: That code wasn't \
        accepted. It may be incomplete, expired, already used, or from an earlier sign-in attempt. \
        Sign in again to get a new code.\r\x1b[2B\x1b[39m\x1b[K\r\x1b[1C\x1b[1B\x1b[38;5;153mPress \
        \x1b[1mEnter\x1b[22m to retry. \r\x1b[1B\x1b[39m\x1b[K\r\x1b[1B\x1b[K\r\x1b[1A";
    const NOT_ACCEPTED: &str = "That code wasn't accepted. It may be incomplete, expired, \
        already used, or from an earlier sign-in attempt. Sign in again to get a new code.";

    /// …and once Enter was pressed: it starts over with a new link.
    fn setup_token_again(url: &str) -> Vec<u8> {
        let mut s = format!(
            "\x1b[2K\x1b[1A\x1b[2K\x1b[1A\x1b[2K\x1b[G\x1b[1A\r\x1b[1C\x1b[1A\x1b[38;5;153mRetrying\
             \u{2026}\x1b[39m\x1b[K\r\x1b[1B\x1b[K\r\x1b[2A\x1b[2K\x1b[G\x1b[1A\x1b[K\x1b[2G\
             \x1b[38;5;231m\u{2722}\x1b[4G\x1b[39mOpening\x1b[12Gbrowser\x1b[20Gto\x1b[23Gsign\
             \x1b[28Gin\u{2026}\r\r\n\r\x1b[1C\x1b[1A\x1b[38;5;246mBrowser didn't open? Use the url \
             below to sign in (c to copy)\x1b[39m\r\r\n\r\r\n\x1b]8;id=1u6bt8l;{url}\x07\
             \x1b[38;5;246m{url}\x1b[39m\x1b]8;;\x07\r\r\n\r\r\n\r\r\n"
        )
        .into_bytes();
        s.extend_from_slice(SETUP_PROMPT);
        s
    }

    /// A code Claude Code turns down comes back with its words, and the
    /// field is offered again; `setup-token` starts over (Enter pressed
    /// once) and shows a new link, not opened yet.
    #[cfg(feature = "token-capture")]
    #[test]
    fn a_code_turned_down_says_why_and_setup_token_starts_over() {
        let (job, keys) = typed_into(SignInKind::Token);
        let mut screen = Screen::default();
        screen.feed(&setup_token_screen());
        job.saw(&screen);
        job.note_opened(Browser::ChromePrivate);
        let rev = job.view().rev;
        job.submit_code("abc#wrong").unwrap();
        let v = job.view();
        assert!(v.code_sent && v.code_error.is_none());
        assert_eq!(v.state, SignInState::Open, "it asks for nothing now");
        assert!(v.rev > rev);
        assert!(!job.capture_view().asks_for_code);
        let json = serde_json::to_value(v.public()).unwrap();
        assert_eq!(json["codeSent"], true);

        lock(&keys.0).clear();
        screen.feed(SETUP_REFUSAL);
        assert!(job.take_in(&screen));
        let v = job.view();
        assert_eq!(v.code_error.as_deref(), Some(NOT_ACCEPTED));
        assert!(!v.code_sent);
        assert_eq!(v.state, SignInState::Code);
        assert!(job.capture_view().asks_for_code);
        // The code never shows, not even its end.
        let last = screen.last_line().unwrap();
        assert!(!format!("{last}{:?}", v.error).contains("rfqo_w"), "{last}");
        assert_eq!(*lock(&keys.0), b"\r", "Enter where it offers to start over");
        // Pressed once: the offer looked at again asks for nothing more.
        assert!(!job.take_in(&screen));
        assert_eq!(*lock(&keys.0), b"\r");

        let again = SETUP_URL.replace("state=Rk1y", "state=Zz9q");
        screen.feed(&setup_token_again(&again));
        assert_eq!(
            job.saw(&screen),
            Saw {
                changed: true,
                retry: false
            }
        );
        let v = job.view();
        assert_eq!(v.url.as_deref(), Some(again.as_str()), "the new link");
        assert_eq!(job.open_link().unwrap().as_str(), again);
        assert_eq!(v.opened, None, "not opened yet");
        assert_eq!(v.state, SignInState::Code);
        assert_eq!(v.code_error.as_deref(), Some(NOT_ACCEPTED), "still why");

        // The next code clears it.
        job.submit_code("abc#def").unwrap();
        assert_eq!(job.view().code_error, None);
        assert!(job.view().code_sent);
    }

    /// `auth login` (CLI 2.1.314) asks again after a code cut short (no
    /// `#state`), the same link; a code it did not accept ends it.
    #[cfg(feature = "token-capture")]
    #[test]
    fn a_login_asks_again_after_a_code_cut_short() {
        let (job, _keys) = typed_into(SignInKind::Login);
        let mut screen = Screen::default();
        screen.feed(
            format!(
                "Opening browser to sign in\u{2026}\r\nIf the browser didn't open, visit: \
                 {URL}\r\nPaste code here if prompted > "
            )
            .as_bytes(),
        );
        job.saw(&screen);
        assert_eq!(job.view().state, SignInState::Code);
        job.submit_code("abc").unwrap();
        screen.feed(b"Invalid code. Please make sure the full code was copied.\r\n");
        let saw = job.saw(&screen);
        assert!(saw.changed && !saw.retry);
        let v = job.view();
        assert_eq!(
            v.code_error.as_deref(),
            Some("Invalid code. Please make sure the full code was copied.")
        );
        assert_eq!((v.state, v.code_sent), (SignInState::Code, false));
        assert_eq!(v.url.as_deref(), Some(URL));
        job.submit_code("abc#def").unwrap();
        screen.feed(format!("Login failed: {NOT_ACCEPTED}\r\n").as_bytes());
        assert_eq!(
            job.saw(&screen),
            Saw {
                changed: true,
                retry: false
            }
        );
        assert_eq!(job.view().code_error.as_deref(), Some(NOT_ACCEPTED));
        assert_eq!(
            screen.last_line().unwrap(),
            format!("Login failed: {NOT_ACCEPTED}")
        );
        // When it ends, nothing is waiting for an answer.
        assert!(job.complete(Err("it ended".into())));
        assert!(!job.view().code_sent);
    }

    /// A token pasted where the code goes is never typed in, and the refusal
    /// never repeats it.
    #[cfg(feature = "token-capture")]
    #[test]
    fn a_token_is_never_typed_in_as_a_code() {
        let (job, keys) = typed_into(SignInKind::Token);
        job.saw(&shown(Some(URL), 1));
        let padded = format!("  {TOKEN}\n");
        let api_key = "sk-ant-api03-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789";
        let shouted = TOKEN.to_ascii_uppercase();
        for pasted in [TOKEN, padded.as_str(), api_key, shouted.as_str()] {
            let e = job.submit_code(pasted).unwrap_err();
            assert_eq!(e.code("x"), "invalid_body");
            let said = e.to_string();
            assert!(said.contains("not the code"), "{said}");
            assert!(!said.contains(&pasted.trim()[..14]), "{said}");
        }
        assert!(lock(&keys.0).is_empty(), "nothing typed");
        let v = job.view();
        assert_eq!((v.state, v.code_sent), (SignInState::Code, false));
    }

    /// The account got its token pasted: its token run ends done and stops;
    /// a login, or a run that ended, stays as it is.
    #[test]
    fn a_token_run_ends_when_the_token_arrives_another_way() {
        let job = SignInJob::new("acc_000000000007", SignInKind::Token, false);
        job.saw(&shown(Some(URL), 1));
        assert!(job.got_token());
        let v = job.view();
        assert_eq!(v.state, SignInState::Done);
        assert!(v.finished_at.is_some() && v.error.is_none());
        assert_eq!(job.capture_view().state, CaptureState::Captured);
        assert!(!job.got_token(), "once");
        let login = SignInJob::new("acc_000000000008", SignInKind::Login, false);
        assert!(!login.got_token());
        assert!(login.is_running());
    }

    /// A prompt, a refusal or an offer to start over counts when it appears:
    /// not again when it is scanned again, not before its line ended.
    #[test]
    fn what_the_screen_says_counts_once() {
        let mut screen = Screen::default();
        screen.feed(SETUP_PROMPT);
        assert_eq!(screen.prompts, 1);
        screen.feed("\r\x1b[1C\x1b[1A\u{2722}\r\r\n".as_bytes());
        assert_eq!(screen.prompts, 1, "rescanned, not counted again");
        screen.feed(SETUP_PROMPT);
        assert_eq!(screen.prompts, 2, "drawn again");
        // A refusal cut by a read is not one yet.
        let (head, tail) = SETUP_REFUSAL.split_at(160);
        screen.feed(head);
        assert_eq!((screen.refusals, screen.retries), (0, 0));
        screen.feed(tail);
        assert_eq!((screen.refusals, screen.retries), (1, 1));
        assert_eq!(screen.refusal.as_deref(), Some(NOT_ACCEPTED));
        screen.feed(b"\r\n");
        assert_eq!((screen.refusals, screen.retries), (1, 1));
        // The CLI's other words for it.
        for (line, words) in [
            (
                "OAuth error: Invalid code. Please make sure the full code was copied",
                "Invalid code. Please make sure the full code was copied",
            ),
            (
                "Login failed: That code is not valid",
                "That code is not valid",
            ),
            (
                "Error: invalid code for https://claude.ai/x",
                "invalid code for <link>",
            ),
            // On the prompt's line, where `auth login` writes it.
            (
                "Paste code here if prompted > Invalid code. Please make sure the full code was \
                 copied.",
                "Invalid code. Please make sure the full code was copied.",
            ),
        ] {
            let mut screen = Screen::default();
            screen.feed(format!("{line}\r\n").as_bytes());
            assert_eq!(screen.refusal.as_deref(), Some(words), "{line}");
        }
        let mut screen = Screen::default();
        screen.feed(b"Opening browser to sign in\r\nLogin successful.\r\n");
        assert_eq!(screen.refusals, 0);
    }

    /// `setup-token` shows a new link when it starts over: the latest wins,
    /// and one looked at before never comes back.
    #[test]
    fn a_new_link_replaces_the_one_before() {
        let again = URL.replace("state=abc", "state=def");
        let mut screen = Screen::default();
        screen.feed(format!("visit: {URL}\r\n").as_bytes());
        assert_eq!(screen.url.as_ref().map(Link::as_str), Some(URL));
        screen.feed(format!("\x1b]8;;{again}\x07here\x1b]8;;\x07\r\n").as_bytes());
        assert_eq!(screen.url.as_ref().map(Link::as_str), Some(again.as_str()));
        screen.feed("\x1b[2K\u{2722} working\r\n".as_bytes());
        assert_eq!(
            screen.url.as_ref().map(Link::as_str),
            Some(again.as_str()),
            "the old one, still rescanned, is not taken again"
        );
        // Never a link inside another one.
        let mut screen = Screen::default();
        screen.feed(
            format!("visit: {URL}&redirect_uri=https://claude.ai/oauth/code/callback\r\n")
                .as_bytes(),
        );
        assert_eq!(
            screen.url.as_ref().map(Link::as_str),
            Some(format!("{URL}&redirect_uri=https://claude.ai/oauth/code/callback").as_str())
        );
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
        // Nor the code typed, which `setup-token` echoes as `*`s and its
        // last characters.
        let mut screen = Screen::default();
        screen.feed(b"\r\x1b[31C\x1b[1A**********************rfqo_w\r\r\n");
        assert_eq!(screen.last_line().unwrap(), "***");
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

    /// A screen that showed `url` and asked for the code `prompts` times.
    fn shown(url: Option<&str>, prompts: usize) -> Screen {
        Screen {
            url: url.map(|u| Link::parse(u).unwrap()),
            prompts,
            ..Default::default()
        }
    }

    #[test]
    fn a_runs_states_follow_the_screen() {
        let job = SignInJob::new("acc_000000000001", SignInKind::Login, true);
        assert_eq!(job.view().state, SignInState::Starting);
        assert!(job.take_auto_open().is_none(), "no link yet");
        assert!(matches!(job.open_link(), Err(SignInError::NoLink)));
        let link = Link::parse(URL).unwrap();
        let mut screen = shown(Some(URL), 0);
        assert!(job.saw(&screen).changed);
        assert_eq!(job.view().state, SignInState::Open);
        assert_eq!(job.view().url.as_deref(), Some(URL));
        assert_eq!(job.take_auto_open(), Some(link.clone()));
        assert!(job.take_auto_open().is_none(), "once");
        assert_eq!(job.open_link().unwrap(), link);
        screen.prompts = 1;
        assert!(job.saw(&screen).changed);
        assert_eq!(job.view().state, SignInState::Code);
        assert!(!job.saw(&screen).changed, "nothing new");
        let rev = job.view().rev;
        assert!(job.complete(Ok(())));
        assert_eq!(job.view().state, SignInState::Done);
        assert!(job.view().finished_at.is_some());
        assert!(job.view().rev > rev, "every change counts");
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
        let refused = Screen {
            refused_url: true,
            ..Default::default()
        };
        assert!(!job.saw(&refused).changed, "nothing to show");
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
        job.saw(&shown(Some(URL), 1));
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
        let mut screen = Screen::default();
        screen.feed(b"Select a method\r\n");
        job.saw(&screen);
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
        job.saw(&shown(Some(URL), 0));
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

    /// The real `claude auth login`, in a pseudo-terminal as a sign-in runs
    /// it, with a throwaway config directory: its link must arrive (on
    /// Windows it never did while ConPTY's cursor query went unanswered).
    /// Needs `claude` installed; signs nothing in and opens no browser.
    ///
    /// `cargo test --manifest-path src-tauri/Cargo.toml --lib real_login -- --ignored`
    #[cfg(feature = "token-capture")]
    #[test]
    #[ignore]
    fn a_real_login_shows_its_link() {
        struct Quiet;
        impl SignInSink for Quiet {
            fn changed(&self, _: &Arc<SignInJob>) {}
            fn token(&self, _: &str, _: Secret) -> Result<(), String> {
                Ok(())
            }
            fn exited(&self, _: &Arc<SignInJob>) {}
        }
        let dir = tempfile::tempdir().unwrap();
        let spec = crate::claude::accounts::signin::login_spec(
            &crate::claude::claude_bin(),
            Some(dir.path()),
            false,
            None,
            None,
        );
        let job = start(
            "acc_test",
            SignInKind::Login,
            &spec,
            false,
            Arc::new(Quiet),
            SIGN_IN_TIMEOUT,
            LINK_TIMEOUT,
        )
        .unwrap();
        let started = std::time::Instant::now();
        let link = loop {
            if let Ok(link) = job.open_link() {
                break link;
            }
            assert!(
                job.is_running() && started.elapsed() < Duration::from_secs(20),
                "no link after {:?}: {:?}",
                started.elapsed(),
                job.view()
            );
            std::thread::sleep(Duration::from_millis(100));
        };
        job.cancel();
        assert!(link.as_str().starts_with("https://"), "{}", link.as_str());
        assert!(link.as_str().contains("oauth"), "{}", link.as_str());
    }
}
