//! The setup's checks Phase 3a adds (docs/PLAN-MACHINES.md §5.14): git and
//! its version, Git for Windows' long paths, Claude Code (and whether it is
//! older than another computer's), each account's sign-in, and room where
//! Mother Claude keeps its data. A move then finds nothing to discover.
//!
//! Each detector is a pure function of what a probe read (`git --version`,
//! `git config`, `reg query`, the accounts' last `claude auth status`, the
//! data dir's free space), tested against recorded outputs: no test runs
//! git, `claude` or the network for them. Every git call goes through
//! `code::GitRunner` (hooks never run). A check never shows an email: an
//! account is named by its name in Mother Claude.

use std::path::Path;

use super::accounts::{AccountFact, SignIn};
use super::{fix, Check, FixKind};
use crate::machines::code::{CodeError, GitRunner};
use crate::state::AppState;

/// git older than this misses what a move relies on (plan §5.14).
pub const GIT_MIN: &str = "2.30";
/// Less free space than this where Mother Claude keeps its data warns: the
/// default room for copies (plan §5.14 step 6).
pub const DISK_WARN_BYTES: u64 = 5_000_000_000;

// ─── versions ──────────────────────────────────────────────────────────────────

/// The version `git --version` prints: `git version 2.47.1.windows.2` →
/// `2.47.1.windows.2`, `git version 2.39.5 (Apple Git-154)` → `2.39.5`.
pub fn parse_git_version(out: &str) -> Option<String> {
    let line = out.lines().find(|l| !l.trim().is_empty())?.trim();
    let v = line
        .strip_prefix("git version ")?
        .split_whitespace()
        .next()?;
    (!version_numbers(v).is_empty()).then(|| v.to_string())
}

/// The version `claude --version` prints: `2.1.330 (Claude Code)` →
/// `2.1.330` (its first word that is a version).
pub fn parse_claude_version(out: &str) -> Option<String> {
    out.split_whitespace()
        .find(|w| !version_numbers(w).is_empty())
        .map(str::to_string)
}

/// The leading dotted numbers of a version: `2.47.1.windows.2` → 2, 47, 1.
pub fn version_numbers(v: &str) -> Vec<u64> {
    v.trim_start_matches('v')
        .split('.')
        .map_while(|p| p.parse::<u64>().ok())
        .collect()
}

/// `a` is an older version than `b` (missing parts count as 0); `false`
/// when either is not a version.
pub fn older(a: &str, b: &str) -> bool {
    let (x, y) = (version_numbers(a), version_numbers(b));
    if x.is_empty() || y.is_empty() {
        return false;
    }
    let n = x.len().max(y.len());
    let at = |v: &[u64], i: usize| v.get(i).copied().unwrap_or(0);
    for i in 0..n {
        match at(&x, i).cmp(&at(&y, i)) {
            std::cmp::Ordering::Less => return true,
            std::cmp::Ordering::Greater => return false,
            std::cmp::Ordering::Equal => {}
        }
    }
    false
}

/// A version shown short: its dotted numbers (`2.47.1.windows.2` → `2.47.1`).
pub fn short_version(v: &str) -> String {
    let n = version_numbers(v);
    if n.is_empty() {
        v.to_string()
    } else {
        n.iter().map(u64::to_string).collect::<Vec<_>>().join(".")
    }
}

// ─── git ───────────────────────────────────────────────────────────────────────

/// What `git --version` told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitProbe {
    Version(String),
    /// git could not be started: not installed (or not on the PATH).
    Missing,
    /// It ran and said something else, or failed.
    Failed,
}

/// git's version, through `code::GitRunner` (blocking).
pub fn probe_git(git: &GitRunner) -> GitProbe {
    let op = match git.begin() {
        Ok(op) => op,
        Err(_) => return GitProbe::Failed,
    };
    let dir = op.dir().to_path_buf();
    match op.cmd(&dir).arg("--version").output() {
        Ok(out) if out.success => parse_git_version(&String::from_utf8_lossy(&out.stdout))
            .map(GitProbe::Version)
            .unwrap_or(GitProbe::Failed),
        Ok(_) => GitProbe::Failed,
        Err(CodeError::NoGit(_)) => GitProbe::Missing,
        Err(_) => GitProbe::Failed,
    }
}

/// The fix that installs git on a system.
fn install_git(os: &str) -> &'static str {
    match os {
        "macos" => "Install the command line tools",
        "windows" => "Install Git for Windows",
        _ => "Install git",
    }
}

/// git is installed, 2.30 or newer (required: code moves through git).
pub fn git_check(probe: &GitProbe, os: &str) -> Check {
    let c = Check::new(
        "git",
        "git",
        "git",
        "A session's code moves through git, on both computers.",
    );
    match probe {
        GitProbe::Missing => c.failed(
            true,
            "git isn't installed: code can't move without it.",
            fix(FixKind::Manual, install_git(os)),
        ),
        GitProbe::Failed => c.unknown("Could not run git."),
        GitProbe::Version(v) if older(v, GIT_MIN) => c.failed(
            true,
            format!(
                "git {} is older than {GIT_MIN}: update it.",
                short_version(v)
            ),
            fix(FixKind::Manual, "Update git"),
        ),
        GitProbe::Version(v) => c.ok(format!("git {}", short_version(v))),
    }
}

/// A git boolean (`git config --get`): `true`/`yes`/`on`/`1` and their
/// opposites; `None` when unset or unreadable.
pub fn parse_git_bool(out: &str) -> Option<bool> {
    match out.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

/// `reg query <key> /v <name>`'s value for a `REG_DWORD`:
/// `    LongPathsEnabled    REG_DWORD    0x1` → 1.
pub fn parse_reg_dword(out: &str, name: &str) -> Option<u32> {
    out.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        if !it.next()?.eq_ignore_ascii_case(name) || it.next()? != "REG_DWORD" {
            return None;
        }
        let hex = it.next()?;
        u32::from_str_radix(hex.trim_start_matches("0x").trim_start_matches("0X"), 16).ok()
    })
}

/// `core.longpaths` for this user (else the system's), through
/// `code::GitRunner`; `None` when git does not run.
pub fn git_long_paths(git: &GitRunner) -> Option<bool> {
    let op = git.begin().ok()?;
    let dir = op.dir().to_path_buf();
    let read = |scope: &str| -> Result<Option<bool>, CodeError> {
        let out = op
            .cmd(&dir)
            .args(["config", scope, "--get", "core.longpaths"])
            .output()?;
        // Exit 1: not set there.
        Ok(out
            .success
            .then(|| parse_git_bool(&String::from_utf8_lossy(&out.stdout)))
            .flatten())
    };
    let global = read("--global").ok()?;
    let system = read("--system").ok()?;
    Some(global.or(system).unwrap_or(false))
}

/// Windows: long paths for git (`core.longpaths`) and for Windows itself
/// (`LongPathsEnabled`). Recommended: Mother Claude's own git calls always
/// pass `core.longpaths=true` (F8), but the user's and Claude Code's do not.
/// `git`: `None` when git does not run; `windows`: `None` when unreadable.
pub fn long_paths_check(git: Option<bool>, windows: Option<bool>) -> Check {
    let c = Check::new(
        "longPaths",
        "files",
        "Long paths",
        "Windows and git leave out paths longer than 260 characters unless long paths are on.",
    );
    match (git, windows) {
        (None, _) => c.unknown("Checked once git runs."),
        (Some(false), _) => c.failed(
            false,
            "Git for Windows leaves out paths longer than 260 characters: core.longpaths is off for this user.",
            fix(FixKind::Auto, "Turn on for git"),
        ),
        (Some(true), Some(false)) => c.failed(
            false,
            "Windows' own long paths setting is off: some tools can't open paths longer than 260 characters.",
            fix(FixKind::Manual, "Ask an administrator to turn on long paths"),
        ),
        (Some(true), None) => c.ok("Long paths are on for git."),
        (Some(true), Some(true)) => c.ok("Long paths are on, for Windows and for git."),
    }
}

// ─── Claude Code ───────────────────────────────────────────────────────────────

/// Claude Code is installed (required), and not older than another
/// computer's (recommended: a session moved here from there may not
/// resume). `peers`: each paired computer's name and Claude Code version.
pub fn claude_check(mine: Option<&str>, peers: &[(String, String)]) -> Check {
    let c = Check::new(
        "claudeCode",
        "claude",
        "Claude Code",
        "Sessions run in Claude Code; one moved to an older version may not resume.",
    );
    let Some(mine) = mine else {
        return c.failed(
            true,
            "Claude Code isn't installed here, or can't be started: sessions can't run.",
            fix(FixKind::Manual, "Install Claude Code"),
        );
    };
    let newest = peers
        .iter()
        .filter(|(_, v)| older(mine, v))
        .max_by(|(_, a), (_, b)| {
            if older(a, b) {
                std::cmp::Ordering::Less
            } else if older(b, a) {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        });
    match newest {
        Some((name, theirs)) => c.failed(
            false,
            format!(
                "Claude Code {} is older than {name}'s {}: a session moved here from there may not resume.",
                short_version(mine),
                short_version(theirs)
            ),
            fix(FixKind::Auto, "Update Claude Code"),
        ),
        None => c.ok(format!("Claude Code {}", short_version(mine))),
    }
}

// ─── accounts ──────────────────────────────────────────────────────────────────

/// One account's sign-in on this computer (the plan's "B for that
/// account": a failure blocks moving and running that account's sessions,
/// not Machines).
pub fn account_check(a: &AccountFact) -> Check {
    let mut c = Check::new(
        &format!("account:{}", a.id),
        "accounts",
        &a.name,
        "A session moves to the account of the same name, and runs only where it is signed in.",
    );
    c.scope = Some("account".into());
    match a.state {
        SignIn::SignedIn => c.ok("Signed in."),
        SignIn::SignedOut => c.failed(
            true,
            "Not signed in: its sessions can't run or move here until it is.",
            fix(FixKind::Auto, "Sign in…"),
        ),
        SignIn::AuthFailed => c.failed(
            true,
            "A session of it failed to sign in: sign in again.",
            fix(FixKind::Auto, "Sign in…"),
        ),
        SignIn::TokenMissing => c.failed(
            true,
            "Its token isn't on this computer.",
            fix(FixKind::Auto, "Sign in…"),
        ),
        SignIn::CheckFailed => c.unknown("Could not check its sign-in."),
        SignIn::Unknown => c.unknown("Not checked yet."),
    }
}

// ─── room ──────────────────────────────────────────────────────────────────────

/// Bytes free for this user on the volume holding `path`.
#[cfg(windows)]
pub fn free_space(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free: u64 = 0;
    // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the call;
    // the one out-pointer is a local `u64`, the others may be null.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(free)
}

/// Bytes free for this user on the volume holding `path`.
#[cfg(unix)]
pub fn free_space(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: `statvfs` is plain old data; zeroed is a valid value for it.
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a NUL-terminated path, `s` a valid out-pointer.
    let r = unsafe { libc::statvfs(c.as_ptr(), &mut s) };
    #[allow(clippy::unnecessary_cast)]
    let free = (s.f_bavail as u64).saturating_mul(s.f_frsize as u64);
    (r == 0).then_some(free)
}

#[cfg(not(any(windows, unix)))]
pub fn free_space(_path: &Path) -> Option<u64> {
    None
}

/// `12.3 GB` (decimal, as the systems' own disk views show it).
pub fn gigabytes(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 100.0 {
        format!("{gb:.0} GB")
    } else {
        format!("{gb:.1} GB")
    }
}

/// Room where Mother Claude keeps its data (recommended: moves and copies
/// land there first).
pub fn disk_check(free: Option<u64>) -> Check {
    let c = Check::new(
        "diskSpace",
        "files",
        "Free space",
        "Moves and copies land in Mother Claude's data folder first.",
    );
    match free {
        None => c.unknown("Could not read the free space."),
        Some(b) if b < DISK_WARN_BYTES => c.failed(
            false,
            format!(
                "Only {} free where Mother Claude keeps its data.",
                gigabytes(b)
            ),
            fix(FixKind::Manual, "Free some space"),
        ),
        Some(b) => c.ok(format!(
            "{} free where Mother Claude keeps its data.",
            gigabytes(b)
        )),
    }
}

// ─── running them ──────────────────────────────────────────────────────────────

/// git, Git for Windows' long paths, Claude Code. Keeps git's version for
/// the readiness test and the other computers (`Facts`).
pub async fn tools(state: &AppState) -> Vec<Check> {
    let (probe, long) = tokio::task::spawn_blocking(|| {
        let git = GitRunner::new();
        let probe = probe_git(&git);
        let long = if cfg!(windows) && matches!(probe, GitProbe::Version(_)) {
            Some(git_long_paths(&git))
        } else {
            None
        };
        (probe, long)
    })
    .await
    .unwrap_or((GitProbe::Failed, None));
    let version = match &probe {
        GitProbe::Version(v) => Some(v.clone()),
        _ => None,
    };
    super::with_cache(state, |c| c.git = version);
    let os = crate::machines::protocol::this_os();
    let mut out = vec![git_check(&probe, os)];
    if let Some(git) = long {
        let windows = windows_long_paths().await;
        out.push(long_paths_check(git, windows));
    }
    let mine = super::claude_version(state);
    let peers: Vec<(String, String)> = state.machines.store_read(|f| {
        f.peers
            .iter()
            .filter_map(|p| {
                let v = p.versions.as_ref()?.claude.clone()?;
                Some((p.display_name().to_string(), v))
            })
            .collect()
    });
    out.push(claude_check(mine.as_deref(), &peers));
    out
}

/// Windows' `LongPathsEnabled`, as `reg query` reads it.
async fn windows_long_paths() -> Option<bool> {
    let out = super::probe(
        "reg",
        &[
            "query",
            r"HKLM\SYSTEM\CurrentControlSet\Control\FileSystem",
            "/v",
            "LongPathsEnabled",
        ],
    )
    .await?;
    parse_reg_dword(&out, "LongPathsEnabled").map(|v| v != 0)
}

/// Each account's sign-in, checked again when the last check is old.
pub async fn accounts(state: &AppState) -> Vec<Check> {
    super::accounts::refresh_stale(state).await;
    super::accounts::local(state)
        .iter()
        .map(account_check)
        .collect()
}

/// Room in the data dir.
pub async fn disk(state: &AppState) -> Check {
    let free = match state.auth.config_dir.clone() {
        Some(dir) => tokio::task::spawn_blocking(move || free_space(&dir))
            .await
            .ok()
            .flatten(),
        None => None,
    };
    disk_check(free)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machines::setup::{CheckState, Severity};

    #[test]
    fn git_versions_parse_as_recorded() {
        assert_eq!(
            parse_git_version("git version 2.47.1.windows.2\n").as_deref(),
            Some("2.47.1.windows.2")
        );
        assert_eq!(
            parse_git_version("git version 2.39.5 (Apple Git-154)\n").as_deref(),
            Some("2.39.5")
        );
        assert_eq!(parse_git_version("git: command not found"), None);
        assert_eq!(parse_git_version(""), None);
        assert_eq!(short_version("2.47.1.windows.2"), "2.47.1");
        assert_eq!(
            parse_claude_version("2.1.330 (Claude Code)\n").as_deref(),
            Some("2.1.330")
        );
        assert_eq!(parse_claude_version("command not found"), None);
    }

    #[test]
    fn versions_compare_by_their_numbers() {
        assert!(older("2.29.9", "2.30"));
        assert!(!older("2.30.0", "2.30"));
        assert!(!older("2.47.1.windows.2", "2.30"));
        assert!(older("2.1.322", "2.1.330"));
        assert!(!older("2.1.330", "2.1.330"));
        assert!(older("2.1.4", "2.1.30"), "numbers, not text");
        assert!(!older("2.1.30", "2.1.4"));
        assert!(!older("garbage", "2.30"), "not a version");
    }

    #[test]
    fn the_git_check_blocks_without_git_or_with_an_old_one() {
        let missing = git_check(&GitProbe::Missing, "windows");
        assert_eq!(
            (missing.state, missing.severity),
            (CheckState::Blocker, Severity::Required)
        );
        assert_eq!(
            missing.fix.as_ref().unwrap().label,
            "Install Git for Windows"
        );
        assert_eq!(missing.fix.unwrap().kind, FixKind::Manual);
        assert_eq!(
            git_check(&GitProbe::Missing, "macos").fix.unwrap().label,
            "Install the command line tools"
        );
        let old = git_check(&GitProbe::Version("2.25.1".into()), "linux");
        assert_eq!(old.state, CheckState::Blocker);
        assert_eq!(old.detail, "git 2.25.1 is older than 2.30: update it.");
        let ok = git_check(&GitProbe::Version("2.47.1.windows.2".into()), "windows");
        assert_eq!(
            (ok.state, ok.detail.as_str()),
            (CheckState::Ok, "git 2.47.1")
        );
        assert!(ok.fix.is_none());
        assert_eq!(
            git_check(&GitProbe::Failed, "macos").state,
            CheckState::Unknown
        );
    }

    #[test]
    fn long_paths_read_from_git_and_the_registry() {
        assert_eq!(parse_git_bool("true\n"), Some(true));
        assert_eq!(parse_git_bool("Off"), Some(false));
        assert_eq!(parse_git_bool(""), None);
        let reg = "\r\nHKEY_LOCAL_MACHINE\\SYSTEM\\CurrentControlSet\\Control\\FileSystem\r\n    LongPathsEnabled    REG_DWORD    0x1\r\n\r\n";
        assert_eq!(parse_reg_dword(reg, "LongPathsEnabled"), Some(1));
        let off = "    LongPathsEnabled    REG_DWORD    0x0";
        assert_eq!(parse_reg_dword(off, "longpathsenabled"), Some(0));
        assert_eq!(
            parse_reg_dword("ERROR: not found", "LongPathsEnabled"),
            None
        );

        let git_off = long_paths_check(Some(false), Some(true));
        assert_eq!(
            (git_off.state, git_off.severity),
            (CheckState::Warning, Severity::Recommended),
            "advice, never a blocker"
        );
        assert_eq!(git_off.fix.unwrap().kind, FixKind::Auto);
        let windows_off = long_paths_check(Some(true), Some(false));
        assert_eq!(windows_off.state, CheckState::Warning);
        assert_eq!(windows_off.fix.unwrap().kind, FixKind::Manual);
        assert_eq!(
            long_paths_check(Some(true), Some(true)).state,
            CheckState::Ok
        );
        assert_eq!(long_paths_check(Some(true), None).state, CheckState::Ok);
        assert_eq!(long_paths_check(None, None).state, CheckState::Unknown);
    }

    #[test]
    fn claude_code_is_required_and_warns_when_older_than_a_peer() {
        let none = claude_check(None, &[]);
        assert_eq!(none.state, CheckState::Blocker);
        let peers = vec![
            ("DESKTOP-PC".to_string(), "2.1.330".to_string()),
            ("BUILD-BOX".to_string(), "2.1.340".to_string()),
        ];
        let older_here = claude_check(Some("2.1.322"), &peers);
        assert_eq!(
            (older_here.state, older_here.severity),
            (CheckState::Warning, Severity::Recommended)
        );
        assert_eq!(
            older_here.detail,
            "Claude Code 2.1.322 is older than BUILD-BOX's 2.1.340: a session moved here from there may not resume."
        );
        assert_eq!(older_here.fix.unwrap().label, "Update Claude Code");
        let same = claude_check(Some("2.1.340"), &peers);
        assert_eq!(
            (same.state, same.detail.as_str()),
            (CheckState::Ok, "Claude Code 2.1.340")
        );
    }

    fn fact(state: SignIn) -> AccountFact {
        AccountFact {
            id: "acc_0123456789abcdef".into(),
            name: "Work".into(),
            kind: "directory".into(),
            state,
        }
    }

    #[test]
    fn an_account_that_is_not_signed_in_blocks_its_own_sessions_only() {
        let ok = account_check(&fact(SignIn::SignedIn));
        assert_eq!(ok.id, "account:acc_0123456789abcdef");
        assert_eq!((ok.title.as_str(), ok.state), ("Work", CheckState::Ok));
        assert!(!ok.gates());
        for s in [SignIn::SignedOut, SignIn::AuthFailed, SignIn::TokenMissing] {
            let c = account_check(&fact(s));
            assert_eq!(c.state, CheckState::Blocker, "{s:?}");
            assert_eq!(c.scope.as_deref(), Some("account"));
            assert!(!c.gates(), "a scoped blocker keeps Machines on");
            assert_eq!(c.fix.unwrap().label, "Sign in…");
            assert!(!c.detail.contains('@'), "never an email");
        }
        assert_eq!(
            account_check(&fact(SignIn::CheckFailed)).state,
            CheckState::Unknown
        );
        assert_eq!(
            account_check(&fact(SignIn::Unknown)).state,
            CheckState::Unknown
        );
    }

    #[test]
    fn little_free_space_warns() {
        assert_eq!(gigabytes(12_345_000_000), "12.3 GB");
        assert_eq!(gigabytes(512_000_000_000), "512 GB");
        let low = disk_check(Some(1_200_000_000));
        assert_eq!(
            (low.state, low.severity),
            (CheckState::Warning, Severity::Recommended)
        );
        assert_eq!(
            low.detail,
            "Only 1.2 GB free where Mother Claude keeps its data."
        );
        let ok = disk_check(Some(80_000_000_000));
        assert_eq!(ok.state, CheckState::Ok);
        assert_eq!(disk_check(None).state, CheckState::Unknown);
    }

    #[test]
    fn free_space_reads_this_volume() {
        let dir = tempfile::tempdir().unwrap();
        let free = free_space(dir.path()).expect("the temp folder's volume");
        assert!(free > 0);
    }
}
