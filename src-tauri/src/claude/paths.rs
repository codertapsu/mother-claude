//! Paths as any machine writes them. A transcript's `cwd`, a backup's
//! recorded folder or a folder another machine names may come from a Mac or
//! from Windows, whatever this machine is, so `Path::is_absolute` (this
//! machine's rules) cannot judge them: on Windows `/Users/me/app` has no drive
//! and is not absolute, on a Mac `D:\dev\app` is one relative file name.
//! Paths the user gives for this machine keep this machine's rules.

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
}
