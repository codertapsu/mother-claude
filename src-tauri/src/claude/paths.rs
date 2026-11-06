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

use std::borrow::Cow;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// This machine's system as clients name it (`me.hostOs`): `macos`,
/// `windows`, `linux` (any other Unix: its own name).
pub fn host_os() -> &'static str {
    std::env::consts::OS
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
    let drive = |s: &str| s.len() == 2 && s.ends_with(':');
    trim_end_separators(p)
        .split(separators(p))
        .filter(|s| !s.is_empty() && !drive(s))
        .count()
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
}
