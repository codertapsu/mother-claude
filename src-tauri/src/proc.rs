//! Every program Mother Claude starts is made here.
//!
//! On Windows, Mother Claude is a GUI program (`windows_subsystem =
//! "windows"` in release builds) with no console of its own, so each
//! console program it starts — `claude` (or its `.cmd` shim), `git`,
//! `node`, PowerShell, `cmd` — would open a console window of its own,
//! which flashes up for as long as the program runs: when the Computers
//! page checks this computer, when the usage refreshes, when a session
//! continues. These commands carry `CREATE_NO_WINDOW` there: the program
//! runs with a console nobody sees, and the programs it starts in turn
//! share it. GUI programs (Explorer, a browser) ignore the flag. Elsewhere
//! they are plain `Command::new`.
//!
//! `clippy.toml` forbids `Command::new` everywhere else, so a new call site
//! cannot bring the windows back. A caller that sets its own creation flags
//! must keep [`CREATE_NO_WINDOW`] among them (`creation_flags` replaces).

use std::ffi::OsStr;

/// `CREATE_NO_WINDOW`: a console program runs without a console window.
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A [`tokio::process::Command`] for `program` that opens no window.
#[allow(clippy::disallowed_methods)] // the one place it is made
pub fn command(program: impl AsRef<OsStr>) -> tokio::process::Command {
    #[allow(unused_mut)]
    let mut cmd = tokio::process::Command::new(program);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// A [`std::process::Command`] for `program` that opens no window.
#[allow(clippy::disallowed_methods)] // the one place it is made
pub fn std_command(program: impl AsRef<OsStr>) -> std::process::Command {
    #[allow(unused_mut)]
    let mut cmd = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

#[cfg(test)]
mod tests {
    /// Both kinds run their program and capture what it prints — the flag
    /// changes only whether a window shows.
    #[tokio::test]
    async fn commands_run_their_program() {
        let (program, args): (&str, &[&str]) = if cfg!(windows) {
            ("cmd", &["/d", "/c", "echo mc-proc"])
        } else {
            ("sh", &["-c", "echo mc-proc"])
        };
        let out = super::command(program).args(args).output().await.unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains("mc-proc"));
        let out = super::std_command(program).args(args).output().unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains("mc-proc"));
    }
}
