//! Paths as any machine writes them. A transcript's `cwd`, a backup's
//! recorded folder or a folder another machine names may come from a Mac or
//! from Windows, whatever this machine is, so `Path::is_absolute` (this
//! machine's rules) cannot judge them: on Windows `/Users/me/app` has no drive
//! and is not absolute, on a Mac `D:\dev\app` is one relative file name.
//! Paths the user gives for this machine keep this machine's rules.
//!
//! So two kinds of path meet here:
//!
//! * **Recorded** ones — a conversation's folder, a project named by it, a
//!   `.claude.json` or `history.jsonl` key, a folder in a backup's index, a
//!   per-project default — are judged by their own form
//!   ([`is_absolute_anywhere`]) and compared by [`same_path`] and
//!   [`is_within_recorded`]: a Windows path without regard to case or to
//!   which separator, as Windows compares them (Claude Code itself keys
//!   `.claude.json` with `/` there and records `\` in transcripts), a Mac's
//!   or Linux's exactly, as before.
//! * **This machine's**, typed by the user for it (the backup folder, a
//!   storage, a directory account's directory): `~` expanded the way Claude
//!   Code expands it here ([`expand_home`]), [`canonicalize`]d without
//!   Windows' `\\?\` prefix, compared by [`same_dir`] and [`is_within`] —
//!   without regard to case on Windows, whose file systems compare names so.
//!
//! And the names Mother Claude gives folders from names people chose (a
//! backup set's, after its account) are [`portable_name`]s: the same on
//! every system, so a set made on one is found on the other.
//!
//! Folder pairs (docs/BACKUP.md "Folder pairs") map a project between
//! machines: [`rebase`] moves a recorded path from one machine's folder to
//! another's, [`project_dir_name`] is the `projects/<name>` folder Claude Code
//! keeps a cwd's sessions in, [`config_key`] the key it files a folder under
//! in `.claude.json`, and [`view_cwd`] the folder Mother Claude shows (and
//! continues) a session in on this machine.

use std::borrow::Cow;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, RwLock};

use serde::{Deserialize, Serialize};

/// This machine's system as clients name it (`me.hostOs`): `macos`,
/// `windows`, `linux` (any other Unix: its own name).
pub fn host_os() -> &'static str {
    std::env::consts::OS
}

/// This machine in a sentence, as people call one of its system: `Mac`,
/// `PC`, else `machine` ("this PC's copies").
pub fn machine_noun() -> &'static str {
    match host_os() {
        "macos" => "Mac",
        "windows" => "PC",
        _ => "machine",
    }
}

/// Absolute on macOS or Linux (`/…`) or on Windows ([`is_windows_absolute`]).
pub fn is_absolute_anywhere(p: &str) -> bool {
    p.starts_with('/') || is_windows_absolute(p)
}

/// Absolute the Windows way: a drive (`C:\…`, `C:/…`) or a share
/// (`\\server\share\…`, `\\?\C:\…`). `C:` alone and `C:dev` are relative to a
/// drive's current folder, so not.
pub fn is_windows_absolute(p: &str) -> bool {
    let b = p.as_bytes();
    let drive =
        b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'\\' | b'/');
    let share = b.len() > 2 && b.starts_with(br"\\") && b[2] != b'\\';
    drive || share
}

// ─────────────────────────────── names ───────────────────────────────

/// The name a folder (or file) named after `name` gets, the same on every
/// system: `name` itself, unless Windows could not keep it. Windows drops a
/// trailing dot or space, and takes a device's name — `CON`, `PRN`, `AUX`,
/// `NUL`, `COM0`–`COM9`, `LPT0`–`LPT9` (and `¹²³`), in any case, also before
/// an extension (`nul.txt`) — for the device; such a name gets a `_`:
/// `Acme Inc.` → `Acme Inc._`, `Con` → `Con_`, `aux.work` → `aux_.work`.
/// `name` holds no character Windows refuses (`< > : " / \ | ? *` or a
/// control character): account and set names cannot.
pub fn portable_name(name: &str) -> Cow<'_, str> {
    let base_end = name.find('.').unwrap_or(name.len());
    let device = is_device_name(name[..base_end].trim_end_matches(' '));
    let trailing = name.ends_with(['.', ' ']);
    if !device && !trailing {
        return Cow::Borrowed(name);
    }
    let mut out = String::with_capacity(name.len() + 2);
    out.push_str(&name[..base_end]);
    if device {
        out.push('_');
    }
    out.push_str(&name[base_end..]);
    if out.ends_with(['.', ' ']) {
        out.push('_');
    }
    Cow::Owned(out)
}

/// One of Windows' reserved device names (without its extension).
fn is_device_name(base: &str) -> bool {
    let upper = base.to_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|p| {
            upper.strip_prefix(p).is_some_and(|n| {
                matches!(
                    n,
                    "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        })
}

// ─────────────────────────────── recorded paths ───────────────────────────────

/// The separators of a recorded path: `\` as well as `/` in a Windows one
/// (on a Mac `\` is part of a name).
fn separators(p: &str) -> &'static [char] {
    if is_windows_absolute(p) {
        &['/', '\\']
    } else {
        &['/']
    }
}

/// A recorded path without trailing separators; a root keeps its own (`/`,
/// `C:\`).
pub fn trim_end_separators(p: &str) -> &str {
    let t = p.trim_end_matches(separators(p));
    if t.len() == 2 && is_windows_absolute(p) {
        // `C:\` (`C:` alone would be the drive's current folder).
        return &p[..3];
    }
    if t.is_empty() && p.starts_with('/') {
        return &p[..1];
    }
    t
}

/// The last component of a recorded path: `app` of `/Users/me/app` and of
/// `D:\dev\app`; `None` for a root, `.` or `..`.
pub fn last_component(p: &str) -> Option<&str> {
    let t = trim_end_separators(p);
    let name = t.rsplit(separators(p)).next().unwrap_or(t);
    let drive = name.len() == 2 && name.ends_with(':');
    (!name.is_empty() && name != "." && name != ".." && !drive).then_some(name)
}

/// Whether a recorded path names `.` or `..` anywhere.
pub fn has_dot_segments(p: &str) -> bool {
    p.split(separators(p)).any(|s| s == "." || s == "..")
}

/// What a recorded path compares by: a Windows one with `\` only, in lower
/// case; any without a trailing separator.
pub fn path_key(p: &str) -> String {
    let t = trim_end_separators(p);
    if is_windows_absolute(t) {
        t.chars()
            .map(|c| if c == '/' { '\\' } else { c })
            .flat_map(char::to_lowercase)
            .collect()
    } else {
        t.to_string()
    }
}

/// Two recorded paths name one folder: a Windows path without regard to case
/// or to which separator, a trailing separator never counts, and a Mac's or
/// Linux's path exactly.
pub fn same_path(a: &str, b: &str) -> bool {
    a == b || path_key(a) == path_key(b)
}

/// `path` is `dir` or inside it — both recorded, compared as [`same_path`]
/// does, by whole components.
pub fn is_within_recorded(path: &str, dir: &str) -> bool {
    let (p, d) = (path_key(path), path_key(dir));
    let sep = if is_windows_absolute(&d) { '\\' } else { '/' };
    match p.strip_prefix(d.as_str()) {
        Some("") => true,
        Some(rest) => d.ends_with(sep) || rest.starts_with(sep),
        None => false,
    }
}

/// How many components a recorded path has below its root (`/a/b` and
/// `C:\a\b`: 2).
pub fn depth(p: &str) -> usize {
    components(p).count()
}

/// The components of a recorded path below its root, the ones [`depth`]
/// counts (a share's server and name among them).
fn components(p: &str) -> impl Iterator<Item = &str> {
    let drive = |s: &str| s.len() == 2 && s.ends_with(':');
    trim_end_separators(p)
        .split(separators(p))
        .filter(move |s| !s.is_empty() && !drive(s))
}

/// A recorded Windows path written the way Windows writes it, with `\`
/// (Claude Code keys `.claude.json` with `/` there); any other as it is.
pub fn windows_separators(p: &str) -> Cow<'_, str> {
    if is_windows_absolute(p) && p.contains('/') {
        Cow::Owned(p.replace('/', "\\"))
    } else {
        Cow::Borrowed(p)
    }
}

/// The one of `keys` that names the folder `wanted`: itself, else one that is
/// [`same_path`] (on Windows `.claude.json` keys a folder `C:/Users/me/app`
/// while its transcripts record `C:\Users\me\app`).
pub fn find_path<'a>(keys: impl IntoIterator<Item = &'a str>, wanted: &str) -> Option<&'a str> {
    let mut similar = None;
    for k in keys {
        if k == wanted {
            return Some(k);
        }
        if similar.is_none() && same_path(k, wanted) {
            similar = Some(k);
        }
    }
    similar
}

// ─────────────────────────────── this machine's ───────────────────────────────

/// `~` or a path in it — `~/…`, and on Windows `~\…`, as Claude Code expands
/// them there — in `home`; any other path as it is. `None`: a `~` with no home
/// to put there.
pub fn expand_home(p: &Path, home: Option<&Path>) -> Option<PathBuf> {
    match p.strip_prefix("~") {
        Ok(rest) if rest.as_os_str().is_empty() => home.map(Path::to_path_buf),
        Ok(rest) => home.map(|h| h.join(rest)),
        Err(_) => Some(p.to_path_buf()),
    }
}

/// Why `raw` is not a folder of this machine, in its own terms.
pub fn not_a_full_path(raw: &str) -> String {
    let start = if cfg!(windows) {
        r"a drive like C:\ or a share like \\server\share"
    } else {
        "/"
    };
    format!("{raw} is not a full path on this computer: start it with {start} (or ~ for your home folder)")
}

/// `std::fs::canonicalize` without Windows' `\\?\` verbatim prefix wherever a
/// plain path names the same folder (`\\?\D:\NAS` → `D:\NAS`): a verbatim path
/// compares unequal to the plain one a user typed or a configuration names,
/// and reads as nothing anyone typed. A folder only a verbatim path can name —
/// a share or a mapped network drive, which resolve to `\\?\UNC\…`, or a path
/// longer than Windows' 260 characters — keeps the form it was asked by
/// (lexically clean). Elsewhere exactly `std::fs::canonicalize`.
pub fn canonicalize(p: &Path) -> std::io::Result<PathBuf> {
    let c = dunce::canonicalize(p)?;
    if is_verbatim(&c) && p.is_absolute() {
        return Ok(p.components().collect());
    }
    Ok(c)
}

/// A path of this machine without Windows' `\\?\` prefix where a plain path
/// names the same file, as [`canonicalize`] answers it (no I/O): what an
/// older build stored on Windows reads as typed. Elsewhere the path itself.
pub fn plain(p: &Path) -> PathBuf {
    dunce::simplified(p).to_path_buf()
}

fn is_verbatim(p: &Path) -> bool {
    p.as_os_str()
        .to_str()
        .is_some_and(|s| s.starts_with(r"\\?\"))
}

/// [`canonicalize`], or the path as given where that fails (it is not there).
pub fn canonical_or_given(p: &Path) -> PathBuf {
    canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Two folders of this machine are one: by their canonical forms where they
/// exist, without regard to case on Windows.
pub fn same_dir(a: &Path, b: &Path) -> bool {
    same_here(&canonical_or_given(a), &canonical_or_given(b))
}

/// Two paths of this machine are one as given (already normalized): by whole
/// components, without regard to case on Windows.
pub fn same_here(a: &Path, b: &Path) -> bool {
    a.components().count() == b.components().count() && is_within(a, b)
}

/// `path` is `dir` or inside it, both this machine's and normalized (no
/// `..`): by whole components, without regard to case on Windows.
pub fn is_within(path: &Path, dir: &Path) -> bool {
    within(path, dir, cfg!(windows))
}

fn within(path: &Path, dir: &Path, fold_case: bool) -> bool {
    if !fold_case {
        return path.starts_with(dir);
    }
    let mut p = path.components();
    dir.components().all(|d| {
        p.next()
            .is_some_and(|c| same_name(c.as_os_str(), d.as_os_str()))
    })
}

fn same_name(a: &OsStr, b: &OsStr) -> bool {
    a == b || a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

// ─────────────────────────── project folders and folder pairs ───────────────────────────

/// How a machine writes an absolute path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PathForm {
    /// macOS, Linux: `/…`, compared exactly.
    Posix,
    /// Windows: `D:\…` or `\\server\share\…`, `\` or `/`, compared without
    /// regard to case ([`same_path`]).
    Windows,
}

impl PathForm {
    /// This machine's.
    pub fn here() -> Self {
        Self::of_os(host_os())
    }

    /// A machine's, by its system as [`host_os`] names it (`windows`,
    /// `macos`, `linux`, …).
    pub fn of_os(os: &str) -> Self {
        if os.eq_ignore_ascii_case("windows") {
            Self::Windows
        } else {
            Self::Posix
        }
    }

    /// The form an absolute path is written in (`None`: not absolute
    /// anywhere).
    pub fn of_path(p: &str) -> Option<Self> {
        if is_windows_absolute(p) {
            Some(Self::Windows)
        } else if p.starts_with('/') {
            Some(Self::Posix)
        } else {
            None
        }
    }

    fn sep(self) -> char {
        match self {
            Self::Posix => '/',
            Self::Windows => '\\',
        }
    }
}

/// The `projects/<name>` folder Claude Code keeps the sessions of `cwd` in,
/// exactly as the CLI makes it (2.1.318, `U0`): every UTF-16 code unit that
/// is not an ASCII letter or digit becomes `-` (so a character beyond the
/// Basic Multilingual Plane becomes two), and a name longer than 200 is cut
/// to 200 and given `-` and the base-36 absolute value of the path's Java
/// string hash. `/Users/m/app` → `-Users-m-app`, `D:\dev\app` → `D--dev-app`.
pub fn project_dir_name(cwd: &str) -> String {
    const MAX: usize = 200;
    let units: Vec<u16> = cwd.encode_utf16().collect();
    let name: String = units
        .iter()
        .map(|&u| match u8::try_from(u) {
            Ok(b) if b.is_ascii_alphanumeric() => char::from(b),
            _ => '-',
        })
        .collect();
    if name.len() <= MAX {
        return name;
    }
    let hash = units.iter().fold(0i32, |h, &u| {
        h.wrapping_shl(5).wrapping_sub(h).wrapping_add(i32::from(u))
    });
    format!(
        "{}-{}",
        &name[..MAX],
        base36(i64::from(hash).unsigned_abs())
    )
}

fn base36(mut n: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".into();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// Whether `cwd`'s project folder is the one named `name`: a Windows path's
/// whatever its letter case, as NTFS finds it.
pub fn names_project_dir(cwd: &str, name: &str) -> bool {
    let mine = project_dir_name(cwd);
    match PathForm::of_path(cwd) {
        Some(PathForm::Windows) => mine.eq_ignore_ascii_case(name),
        _ => mine == name,
    }
}

/// The key Claude Code files a folder under in `.claude.json` `projects` on
/// a machine of `form`: on Windows with `/` where Windows writes `\`
/// (`D:/dev/app`; [`windows_separators`] goes the other way, for showing a
/// path), elsewhere the folder as it is. [`find_path`] finds either spelling.
pub fn config_key(folder: &str, form: PathForm) -> String {
    match form {
        PathForm::Windows => folder.replace('\\', "/"),
        PathForm::Posix => folder.to_string(),
    }
}

/// A recorded path without Windows' verbatim prefix, which names the same
/// folder plainly (`\\?\D:\dev` → `D:\dev`, `\\?\UNC\nas\share` →
/// `\\nas\share`).
fn without_verbatim(p: &str) -> Cow<'_, str> {
    if let Some(unc) = p.strip_prefix(r"\\?\UNC\") {
        return Cow::Owned(format!(r"\\{unc}"));
    }
    match p.strip_prefix(r"\\?\") {
        Some(rest) if is_windows_absolute(rest) => Cow::Borrowed(rest),
        _ => Cow::Borrowed(p),
    }
}

/// A folder of a folder pair, absolute in `form` and written plainly: that
/// form's separators, single ones and none at the end but a root's, a drive
/// letter upper-cased, Windows' verbatim prefix left out — never `.` or
/// `..`. `Err`: why not.
pub fn clean_folder(raw: &str, form: PathForm) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("name a folder".into());
    }
    if raw.contains('\0') {
        return Err("a folder has no NUL character".into());
    }
    let p = without_verbatim(raw);
    let Some(got) = PathForm::of_path(&p) else {
        return Err(format!("{raw} is not a full path"));
    };
    if has_dot_segments(&p) {
        return Err(format!(
            "{raw} names . or .., which a folder pair does not follow"
        ));
    }
    if got != form {
        return Err(match form {
            PathForm::Windows => {
                format!("{raw} is not a Windows path (one like D:\\dev or \\\\server\\share)")
            }
            PathForm::Posix => {
                format!("{raw} is not a macOS or Linux path (one like /Users/me/dev)")
            }
        });
    }
    let names: Vec<&str> = components(&p).collect();
    Ok(match form {
        PathForm::Posix => format!("/{}", names.join("/")),
        PathForm::Windows if p.as_bytes().get(1) == Some(&b':') => {
            format!("{}:\\{}", p[..1].to_ascii_uppercase(), names.join("\\"))
        }
        PathForm::Windows => format!(r"\\{}", names.join("\\")),
    })
}

/// `path` re-rooted from the folder `from` to the folder `to`: when `path`
/// is `from` or under it ([`is_within_recorded`]: by whole components — so
/// `/a/dev2` is not under `/a/dev` — a Windows path whatever its case and
/// separators), `to` and the rest of `path` in `to`'s form. `None`
/// otherwise, and when any of them names `.` or `..`.
pub fn rebase(path: &str, from: &str, to: &str) -> Option<String> {
    let (path, from, to) = (
        without_verbatim(path),
        without_verbatim(from),
        without_verbatim(to),
    );
    if [&path, &from, &to]
        .iter()
        .any(|p| !is_absolute_anywhere(p) || has_dot_segments(p))
        || !is_within_recorded(&path, &from)
    {
        return None;
    }
    let rest: Vec<&str> = components(&path).skip(depth(&from)).collect();
    let form = PathForm::of_path(&to)?;
    let mut out = match form {
        PathForm::Windows => windows_separators(trim_end_separators(&to)).into_owned(),
        PathForm::Posix => trim_end_separators(&to).to_string(),
    };
    if !rest.is_empty() {
        if !out.ends_with(form.sep()) {
            out.push(form.sep());
        }
        out.push_str(&rest.join(&form.sep().to_string()));
    }
    Some(out)
}

/// One folder pair as this machine uses it: its own folder and the other
/// machine's (in that machine's form).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalPair {
    pub here: String,
    pub there: String,
}

/// `path` (another machine's) mapped to this machine's folder by the pair
/// whose other folder is the longest one it is under.
pub fn map_here(path: &str, pairs: &[LocalPair]) -> Option<String> {
    pairs
        .iter()
        .filter_map(|p| rebase(path, &p.there, &p.here).map(|m| (depth(&p.there), m)))
        .max_by_key(|(d, _)| *d)
        .map(|(_, m)| m)
}

/// The folder a session's transcript stands for on this machine: of the cwds
/// it records first and last, the one usable here whose project folder name
/// is the transcript's folder (`enc`); else one a pair maps to a folder with
/// that name (a session pulled into a paired folder); else one that names
/// it in another machine's form (pulled before a pair, or without one),
/// mapped when a pair maps it; else the first, mapped when it is another
/// machine's and a pair maps it. With no pair this is the first rule that
/// matched before pairs: the recorded cwd naming the folder, else the first.
pub fn view_cwd(
    first: Option<&str>,
    last: Option<&str>,
    enc: Option<&str>,
    pairs: &[LocalPair],
) -> Option<String> {
    let here = PathForm::here();
    let recorded = [first, last];
    let names = |c: &str| enc.is_some_and(|e| names_project_dir(c, e));
    let usable = |c: &str| PathForm::of_path(c) == Some(here);
    let flat = || recorded.into_iter().flatten();
    if let Some(c) = flat().find(|c| usable(c) && names(c)) {
        return Some(c.to_string());
    }
    if !pairs.is_empty() {
        if let Some(m) = flat().find_map(|c| map_here(c, pairs).filter(|m| names(m))) {
            return Some(m);
        }
    }
    // Continued here after it came from another machine: the folder it last
    // ran in here. Its transcript stays in the other machine's project folder
    // (`--resume` finds it by its id), so no cwd here names that folder.
    if flat().any(|c| !usable(c)) {
        if let Some(c) = [last, first].into_iter().flatten().find(|c| usable(c)) {
            return Some(c.to_string());
        }
    }
    if let Some(c) = flat().find(|c| names(c)) {
        return Some(map_here(c, pairs).unwrap_or_else(|| c.to_string()));
    }
    let c = first.or(last)?;
    Some(if usable(c) {
        c.to_string()
    } else {
        map_here(c, pairs).unwrap_or_else(|| c.to_string())
    })
}

/// The components of another machine's `path` below its owner's home folder
/// (`/Users/<u>/…`, `/home/<u>/…`, `C:\Users\<u>\…`), else below its root
/// or drive: `development`, `projects`, `app` of both
/// `/Users/marcus/development/projects/app` and `D:\development\projects\app`.
pub fn below_home_or_root(path: &str) -> Vec<&str> {
    let parts: Vec<&str> = components(path).collect();
    let skip = match parts.as_slice() {
        [first, _user, ..] if first.eq_ignore_ascii_case("Users") || *first == "home" => 2,
        _ => 0,
    };
    parts.into_iter().skip(skip).collect()
}

/// Folders on this machine that may be the folder `recorded` names on
/// another machine (or one gone from here), the likeliest first, existing
/// ones only: the same path below this machine's home folder or (on Windows)
/// a drive's root, then `known` folders here with the same last component —
/// the folders of sessions and the trusted ones. At most six.
pub fn folder_candidates(recorded: &str, home: Option<&Path>, known: &[String]) -> Vec<String> {
    let here = PathForm::here();
    let mut out: Vec<String> = Vec::new();
    let mut push = |p: String| {
        if PathForm::of_path(&p) == Some(here)
            && !same_path(&p, recorded)
            && !out.iter().any(|o| same_path(o, &p))
            && Path::new(&p).is_dir()
        {
            out.push(p);
        }
    };
    let rest = below_home_or_root(recorded);
    if !rest.is_empty() {
        let tail = rest.join(&here.sep().to_string());
        if let Some(h) = home {
            push(h.join(&tail).to_string_lossy().into_owned());
        }
        if here == PathForm::Windows {
            for drive in local_drives() {
                push(format!("{drive}:\\{tail}"));
            }
        }
    }
    if let Some(name) = last_component(recorded) {
        for k in known {
            if last_component(k).is_some_and(|n| n.eq_ignore_ascii_case(name)) {
                push(trim_end_separators(k).to_string());
            }
        }
    }
    out.truncate(6);
    out
}

/// The drives of this PC a folder is looked for on (`C` to `Z`): its own
/// disks, never a network drive, which can keep a look waiting while its
/// server is away.
#[cfg(windows)]
fn local_drives() -> Vec<char> {
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    use windows_sys::Win32::System::WindowsProgramming::{
        DRIVE_FIXED, DRIVE_RAMDISK, DRIVE_REMOVABLE,
    };
    // SAFETY: no arguments; the answer is a bit per drive letter.
    let mask = unsafe { GetLogicalDrives() };
    (b'C'..=b'Z')
        .filter(|d| mask & (1 << (d - b'A')) != 0)
        .filter(|d| {
            let root: Vec<u16> = format!("{}:\\", *d as char)
                .encode_utf16()
                .chain(Some(0))
                .collect();
            // SAFETY: `root` is a NUL-terminated wide string that outlives the call.
            let kind = unsafe { GetDriveTypeW(root.as_ptr()) };
            matches!(kind, DRIVE_FIXED | DRIVE_REMOVABLE | DRIVE_RAMDISK)
        })
        .map(char::from)
        .collect()
}

#[cfg(not(windows))]
fn local_drives() -> Vec<char> {
    Vec::new()
}

/// This machine's folder pairs, as the backup sets last said
/// (`backup::pairs`): what the session list shows and continues sessions
/// in. Empty until a set is read, or loaded from its cache at launch. Kept
/// per owner (the app's data directory: one in the app, one per fixture in a
/// test process), all of them in force at once.
#[derive(Default)]
struct PairsTable {
    by_owner: std::collections::BTreeMap<String, Vec<LocalPair>>,
    all: Arc<Vec<LocalPair>>,
}

static LOCAL_PAIRS: LazyLock<RwLock<PairsTable>> = LazyLock::new(Default::default);

pub fn local_pairs() -> Arc<Vec<LocalPair>> {
    LOCAL_PAIRS
        .read()
        .map(|t| t.all.clone())
        .unwrap_or_else(|e| e.into_inner().all.clone())
}

/// The pairs `owner` (an app data directory) knows, in place of its earlier
/// ones.
pub fn set_local_pairs(owner: &str, pairs: Vec<LocalPair>) {
    let mut t = LOCAL_PAIRS.write().unwrap_or_else(|e| e.into_inner());
    if pairs.is_empty() {
        t.by_owner.remove(owner);
    } else {
        t.by_owner.insert(owner.to_string(), pairs);
    }
    t.all = Arc::new(t.by_owner.values().flatten().cloned().collect());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_on_either_system() {
        for p in [
            "/",
            "/Users/marcus/dev/app",
            r"D:\NAS\MotherClaude",
            "d:/dev/app",
            r"C:\",
            r"\\nas\share\Mother Claude",
            r"\\?\C:\Users\m",
        ] {
            assert!(is_absolute_anywhere(p), "{p}");
        }
        for p in [
            "", "~", "~/dev", r"dev\app", "C:", "C:dev", r"\\", r"\\\x", "1:/x",
        ] {
            assert!(!is_absolute_anywhere(p), "{p}");
        }
        assert!(is_windows_absolute(r"D:\dev") && !is_windows_absolute("/Users/m"));
    }

    #[test]
    fn a_name_windows_cannot_keep_gets_the_same_portable_name_everywhere() {
        // Every name Windows keeps is its own folder name, as before.
        for name in [
            "Work",
            "Default",
            "Marcus (Pro)",
            "Archive 2026",
            "Work ✨",
            "Console",
            "Nullable",
            "COM10",
            "LPT",
            "auxiliary.x",
            "a.b.c",
        ] {
            assert!(
                matches!(portable_name(name), Cow::Borrowed(n) if n == name),
                "{name}"
            );
        }
        for (name, folder) in [
            ("Acme Inc.", "Acme Inc._"),
            ("Work...", "Work..._"),
            ("Con", "Con_"),
            ("con", "con_"),
            ("NUL", "NUL_"),
            ("prn.txt", "prn_.txt"),
            ("aux.work.", "aux_.work._"),
            ("COM1", "COM1_"),
            ("lpt0", "lpt0_"),
            ("COM¹", "COM¹_"),
            ("CON .x", "CON _.x"),
            ("x ", "x _"),
        ] {
            assert_eq!(portable_name(name), folder, "{name}");
            // A portable name stays as it is.
            assert_eq!(portable_name(folder), folder, "{folder}");
        }
    }

    #[test]
    fn a_recorded_path_is_taken_apart_by_its_own_separators() {
        assert_eq!(trim_end_separators("/a/b/"), "/a/b");
        assert_eq!(trim_end_separators("/"), "/");
        assert_eq!(trim_end_separators("//"), "/");
        assert_eq!(trim_end_separators(r"D:\dev\app\"), r"D:\dev\app");
        assert_eq!(trim_end_separators("D:/dev/"), "D:/dev");
        assert_eq!(trim_end_separators(r"C:\"), r"C:\");
        assert_eq!(trim_end_separators("C:/"), "C:/");
        assert_eq!(trim_end_separators(r"\\nas\share\"), r"\\nas\share");
        // On a Mac a backslash is part of a name.
        assert_eq!(trim_end_separators(r"/a/b\"), r"/a/b\");
        assert_eq!(last_component("/Users/me/app/"), Some("app"));
        assert_eq!(last_component(r"D:\dev\app"), Some("app"));
        assert_eq!(last_component("D:/dev/app"), Some("app"));
        assert_eq!(
            last_component(r"\\nas\share\Mother Claude"),
            Some("Mother Claude")
        );
        assert_eq!(last_component(r"/x/a\b"), Some(r"a\b"));
        assert_eq!(last_component("/"), None);
        assert_eq!(last_component(r"C:\"), None);
        assert_eq!(last_component("/a/.."), None);
        assert!(has_dot_segments(r"D:\a\..\b") && has_dot_segments("/a/./b"));
        assert!(!has_dot_segments(r"/a/..b") && !has_dot_segments(r"/a\..\b"));
        assert_eq!(depth("/a/b"), 2);
        assert_eq!(depth(r"C:\a\b\"), 2);
        assert_eq!(depth("/"), 0);
    }

    #[test]
    fn a_windows_path_compares_as_windows_does_and_a_mac_one_exactly() {
        assert!(same_path(r"C:\Users\me\app", "c:/users/ME/app/"));
        assert!(same_path("/Users/me/app", "/Users/me/app/"));
        assert!(!same_path("/Users/me/app", "/users/me/app"));
        assert!(!same_path("/Users/me/app", r"C:\Users\me\app"));
        assert!(!same_path(r"C:\dev\app", r"C:\dev\app2"));
        assert!(is_within_recorded(r"C:\Dev\app\src", "c:/dev/app"));
        assert!(is_within_recorded(r"C:\dev\app", r"C:\dev\app\"));
        assert!(!is_within_recorded(r"C:\dev\app2", r"C:\dev\app"));
        assert!(is_within_recorded(r"C:\dev", r"C:\"));
        assert!(is_within_recorded("/p/acme/api", "/p/acme/"));
        assert!(!is_within_recorded("/p/acme2", "/p/acme"));
        assert!(is_within_recorded("/anything", "/"));
        assert!(!is_within_recorded("/P/acme/api", "/p/acme"));
        assert!(!is_within_recorded("/p/acme", r"C:\p\acme"));
        let keys = ["C:/Users/me/web", "C:/Users/me/app", r"C:\Users\me\app"];
        assert_eq!(
            find_path(keys, r"C:\Users\me\app"),
            Some(r"C:\Users\me\app")
        );
        assert_eq!(
            find_path(keys[..2].iter().copied(), r"c:\users\me\app"),
            Some("C:/Users/me/app")
        );
        assert_eq!(find_path(["/Users/me/app"], "/users/me/app"), None);
        assert_eq!(windows_separators("C:/Users/me/app"), r"C:\Users\me\app");
        assert_eq!(windows_separators("/Users/me/a/b"), "/Users/me/a/b");
    }

    #[test]
    fn home_is_expanded_by_this_machines_rules() {
        let home = Path::new("/Users/me");
        let exp = |p: &str| expand_home(Path::new(p), Some(home));
        assert_eq!(exp("~"), Some(PathBuf::from("/Users/me")));
        assert_eq!(exp("~/Backups"), Some(PathBuf::from("/Users/me/Backups")));
        assert_eq!(exp("/Volumes/NAS"), Some(PathBuf::from("/Volumes/NAS")));
        assert_eq!(exp("~user/x"), Some(PathBuf::from("~user/x")));
        assert_eq!(expand_home(Path::new("~/x"), None), None);
        // `~\` is a home folder where `\` separates (Windows), a name elsewhere.
        if cfg!(windows) {
            assert_eq!(exp(r"~\Backups"), Some(home.join("Backups")));
        } else {
            assert_eq!(exp(r"~\Backups"), Some(PathBuf::from(r"~\Backups")));
        }
        assert!(not_a_full_path("NAS").starts_with("NAS is not a full path on this computer"));
    }

    #[test]
    fn this_machines_folders_compare_by_component_and_on_windows_in_any_case() {
        let (a, b) = (Path::new("/Users/Me/X/y"), Path::new("/users/me/x"));
        assert!(!within(a, b, false));
        assert!(within(a, b, true));
        assert!(!within(Path::new("/users/me/xy"), b, true));
        assert!(!within(Path::new("/users"), b, true));
        assert!(is_within(Path::new("/a/b"), Path::new("/a")));
        assert!(!is_within(Path::new("/a/bc"), Path::new("/a/b")));
        assert!(same_here(Path::new("/a/b"), Path::new("/a/b/")));
        assert!(!same_here(Path::new("/a/b/c"), Path::new("/a/b")));
    }

    #[test]
    fn canonical_forms_are_plain_and_a_missing_folder_stays_as_given() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("NAS");
        std::fs::create_dir(&dir).unwrap();
        let c = canonicalize(&dir).unwrap();
        assert!(c.is_absolute() && !is_verbatim(&c), "{}", c.display());
        if !cfg!(windows) {
            // Elsewhere exactly what std answers (on Windows: without `\\?\`).
            assert_eq!(c, std::fs::canonicalize(&dir).unwrap());
        }
        let missing = tmp.path().join("missing");
        assert_eq!(canonical_or_given(&missing), missing);
        assert!(same_dir(&dir, &dir.join("..").join("NAS")));
        assert!(!same_dir(&dir, &missing));
    }

    /// The CLI's own function run on these (Node, from the 2.1.318 binary).
    #[test]
    fn project_folders_are_named_as_the_cli_names_them() {
        assert_eq!(
            project_dir_name("/Users/marcus/dev/app"),
            "-Users-marcus-dev-app"
        );
        assert_eq!(project_dir_name(r"D:\dev\app"), "D--dev-app");
        assert_eq!(project_dir_name(r"\\nas\share\x"), "--nas-share-x");
        // One `-` per UTF-16 unit: two for a character beyond the BMP.
        assert_eq!(project_dir_name("/Users/m/café/😀"), "-Users-m-caf----");
        let long = format!("/Users/marcus/{}app", "a-very-long-folder-name/".repeat(9));
        assert_eq!(long.len(), 233);
        let name = project_dir_name(&long);
        assert_eq!(name.len(), 207);
        assert!(name.ends_with("-folder-y382k8"), "{name}");
        let win = format!(r"C:\Users\Marcus\{}app", r"projects\".repeat(25));
        assert!(project_dir_name(&win).ends_with("-proj-hhsmm7"));
        // The app's older name for it agrees.
        assert_eq!(crate::claude::encode_cwd(r"D:\dev\app"), "D--dev-app");
        // NTFS finds `D--dev-app` for a `d:\dev\app` cwd too.
        assert!(names_project_dir(r"d:\dev\app", "D--dev-app"));
        assert!(!names_project_dir("/users/m/app", "-Users-m-app"));
    }

    #[test]
    fn folders_are_cleaned_by_the_form_of_their_machine() {
        let w = PathForm::Windows;
        let m = PathForm::Posix;
        assert_eq!(clean_folder(r" d:\dev\ ", w).unwrap(), r"D:\dev");
        assert_eq!(
            clean_folder("D:/dev//projects", w).unwrap(),
            r"D:\dev\projects"
        );
        assert_eq!(clean_folder(r"\\?\D:\dev", w).unwrap(), r"D:\dev");
        assert_eq!(
            clean_folder(r"\\?\UNC\nas\share\dev", w).unwrap(),
            r"\\nas\share\dev"
        );
        assert_eq!(clean_folder(r"\\nas\share\", w).unwrap(), r"\\nas\share");
        assert_eq!(clean_folder(r"D:\", w).unwrap(), r"D:\");
        assert_eq!(
            clean_folder("/Users/marcus//dev/", m).unwrap(),
            "/Users/marcus/dev"
        );
        assert_eq!(clean_folder("/", m).unwrap(), "/");
        for (raw, form) in [
            ("/Users/marcus/dev", w),
            (r"D:\dev", m),
            ("dev", m),
            ("~/dev", m),
            (r"D:\dev\..\x", w),
            ("/a/./b", m),
            ("", m),
        ] {
            assert!(clean_folder(raw, form).is_err(), "{raw}");
        }
    }

    #[test]
    fn rebasing_keeps_folder_boundaries_and_the_target_form() {
        let mac = "/Users/marcus/dev";
        let win = r"D:\dev";
        assert_eq!(
            rebase("/Users/marcus/dev/app/web", mac, win).as_deref(),
            Some(r"D:\dev\app\web")
        );
        assert_eq!(rebase(mac, mac, win).as_deref(), Some(r"D:\dev"));
        assert_eq!(
            rebase("/Users/marcus/dev2/app", mac, win),
            None,
            "not a boundary"
        );
        assert_eq!(
            rebase("/Users/Marcus/dev/app", mac, win),
            None,
            "macOS paths keep case"
        );
        // Windows: either separator, any case, a drive letter in any case.
        assert_eq!(
            rebase(r"d:\DEV\app", win, mac).as_deref(),
            Some("/Users/marcus/dev/app")
        );
        assert_eq!(
            rebase("D:/dev/app/src", win, mac).as_deref(),
            Some("/Users/marcus/dev/app/src")
        );
        assert_eq!(rebase(r"D:\devx\app", win, mac), None);
        assert_eq!(rebase(r"E:\dev\app", win, mac), None, "another drive");
        // A Windows path is never under a macOS folder.
        assert_eq!(rebase(r"D:\dev\app", mac, win), None);
        // Shares, verbatim paths, and roots on either side.
        assert_eq!(
            rebase(r"\\NAS\Share\dev\app", r"\\nas\share\dev", "/Volumes/dev").as_deref(),
            Some("/Volumes/dev/app")
        );
        assert_eq!(
            rebase(r"\\?\D:\dev\app", win, mac).as_deref(),
            Some("/Users/marcus/dev/app")
        );
        assert_eq!(rebase("/x/y", "/", r"D:\").as_deref(), Some(r"D:\x\y"));
        assert_eq!(rebase(r"D:\x", r"D:\", "/").as_deref(), Some("/x"));
        assert_eq!(
            rebase("/a/b", "/a", "D:/dev").as_deref(),
            Some(r"D:\dev\b"),
            "the target's own separators"
        );
        // Nothing that climbs out.
        assert_eq!(rebase("/Users/marcus/dev/../etc", mac, win), None);
    }

    #[test]
    fn the_longest_pair_wins() {
        let pairs = [
            LocalPair {
                here: r"D:\dev".into(),
                there: "/Users/marcus/dev".into(),
            },
            LocalPair {
                here: r"E:\work".into(),
                there: "/Users/marcus/dev/work".into(),
            },
        ];
        assert_eq!(
            map_here("/Users/marcus/dev/work/api", &pairs).as_deref(),
            Some(r"E:\work\api")
        );
        assert_eq!(
            map_here("/Users/marcus/dev/site", &pairs).as_deref(),
            Some(r"D:\dev\site")
        );
        assert_eq!(map_here("/Users/marcus/other", &pairs), None);
    }

    #[test]
    fn config_keys_are_what_claude_code_files_a_folder_under() {
        assert_eq!(config_key(r"D:\dev\app", PathForm::Windows), "D:/dev/app");
        assert_eq!(config_key("/Users/m/app", PathForm::Posix), "/Users/m/app");
        // Found again by either spelling.
        let key = config_key(r"D:\dev\app", PathForm::Windows);
        assert_eq!(find_path([key.as_str()], r"d:\dev\app"), Some("D:/dev/app"));
        assert_eq!(windows_separators(&key), r"D:\dev\app");
    }

    /// The view on a Mac (POSIX here; the same rules run on Windows the
    /// other way round, in the backup engine's tests).
    #[cfg(not(windows))]
    #[test]
    fn the_view_shows_the_folder_the_transcript_stands_for_here() {
        let pairs = [LocalPair {
            here: "/Users/marcus/dev".into(),
            there: r"D:\dev".into(),
        }];
        let mac_enc = project_dir_name("/Users/marcus/dev/app");
        // Started here: as recorded.
        assert_eq!(
            view_cwd(Some("/Users/marcus/dev/app"), None, Some(&mac_enc), &pairs).as_deref(),
            Some("/Users/marcus/dev/app")
        );
        // Started on Windows, pulled into the paired folder here.
        assert_eq!(
            view_cwd(
                Some(r"D:\dev\app"),
                Some(r"D:\dev\app"),
                Some(&mac_enc),
                &pairs
            )
            .as_deref(),
            Some("/Users/marcus/dev/app")
        );
        // Started here, continued on Windows, pulled back: the folder here.
        assert_eq!(
            view_cwd(
                Some("/Users/marcus/dev/app"),
                Some(r"D:\dev\app"),
                Some(&mac_enc),
                &pairs
            )
            .as_deref(),
            Some("/Users/marcus/dev/app")
        );
        // Pulled before the pair: in Windows' folder name, shown (and
        // continued) in the paired folder here.
        let win_enc = project_dir_name(r"D:\dev\app");
        assert_eq!(
            view_cwd(Some(r"D:\dev\app"), None, Some(&win_enc), &pairs).as_deref(),
            Some("/Users/marcus/dev/app")
        );
        // No pair: as before pairs.
        assert_eq!(
            view_cwd(Some(r"D:\dev\app"), None, Some(&win_enc), &[]).as_deref(),
            Some(r"D:\dev\app")
        );
        assert_eq!(
            view_cwd(Some("/a"), Some("/b"), Some("-b"), &[]).as_deref(),
            Some("/b")
        );
        assert_eq!(
            view_cwd(Some("/a"), Some("/b"), Some("-c"), &[]).as_deref(),
            Some("/a")
        );
        assert_eq!(view_cwd(None, None, Some("-c"), &pairs), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn a_session_from_another_machine_continued_here_shows_where_it_ran_here() {
        // Started on a PC (its transcript stays in `D--dev-app`, where
        // `--resume` finds it by its id), then continued on this Mac.
        let win_enc = project_dir_name(r"D:\dev\app");
        assert_eq!(
            view_cwd(
                Some(r"D:\dev\app"),
                Some("/Users/marcus/dev/app"),
                Some(&win_enc),
                &[]
            )
            .as_deref(),
            Some("/Users/marcus/dev/app")
        );
        // Not continued here yet: the PC's folder, as before.
        assert_eq!(
            view_cwd(Some(r"D:\dev\app"), None, Some(&win_enc), &[]).as_deref(),
            Some(r"D:\dev\app")
        );
        // A session of this machine that moved between its folders keeps
        // the one naming its project folder.
        let enc = project_dir_name("/Users/marcus/dev/app");
        assert_eq!(
            view_cwd(
                Some("/Users/marcus/dev/app"),
                Some("/Users/marcus/dev/app/sub"),
                Some(&enc),
                &[]
            )
            .as_deref(),
            Some("/Users/marcus/dev/app")
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn folders_here_are_found_for_another_machines_folder() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let same = home.join("development/projects/app");
        let other = home.join("elsewhere/app");
        std::fs::create_dir_all(&same).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let known = vec![
            other.to_string_lossy().into_owned(),
            home.join("gone/app").to_string_lossy().into_owned(),
            home.join("elsewhere/other").to_string_lossy().into_owned(),
        ];
        let same_s = same.to_string_lossy().into_owned();
        let other_s = other.to_string_lossy().into_owned();
        // A PC's folder below a drive, and a Mac's below another user's home:
        // the same path below this home first, then known folders by name.
        for recorded in [
            r"D:\development\projects\app",
            "/Users/alice/development/projects/app",
        ] {
            assert_eq!(
                folder_candidates(recorded, Some(&home), &known),
                vec![same_s.clone(), other_s.clone()],
                "{recorded}"
            );
        }
        assert_eq!(
            below_home_or_root(r"C:\Users\me\dev\app"),
            vec!["dev", "app"]
        );
        assert_eq!(below_home_or_root("/home/me/dev/app"), vec!["dev", "app"]);
        assert_eq!(below_home_or_root(r"D:\dev\app"), vec!["dev", "app"]);
        // Nothing that does not exist here, nor the recorded folder itself.
        assert!(folder_candidates("/nowhere/at/all", Some(&home), &[]).is_empty());
        assert!(!folder_candidates(&same_s, Some(&home), &known).contains(&same_s));
    }
}
