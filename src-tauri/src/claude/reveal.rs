//! Showing a conversation's transcript or a project's folder in the file
//! manager (docs/ACCOUNTS.md §14.1): selected in Finder (`open -R`), in File
//! Explorer (`explorer.exe /select,"…"`), or the folder opened with
//! `xdg-open`. The command is built by a pure function per platform, so it is
//! tested without spawning anything; [`Revealer`] runs it — or, in tests,
//! only records it, so no test ever opens a window. The path always comes
//! from the index, never from a request.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

/// The file manager to drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    Windows,
    Linux,
}

impl Platform {
    /// The one this build runs on (anything but macOS and Windows is driven
    /// like Linux).
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

/// What to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A file, selected in its folder.
    File(PathBuf),
    /// A folder, opened.
    Folder(PathBuf),
}

/// A command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub program: &'static str,
    pub args: Vec<OsString>,
    /// Appended to the command line exactly as it is, after `args` (Windows
    /// only: Rust would quote `/select,<path with spaces>` as one argument,
    /// which Explorer does not understand, and open Documents instead).
    pub raw: Option<OsString>,
}

/// The command that shows `target` on `platform`.
pub fn command(platform: Platform, target: &Target) -> Command {
    let os = |p: &Path| p.as_os_str().to_os_string();
    match (platform, target) {
        (Platform::MacOs, Target::File(p)) => Command {
            program: "/usr/bin/open",
            args: vec!["-R".into(), os(p)],
            raw: None,
        },
        (Platform::MacOs, Target::Folder(p)) => Command {
            program: "/usr/bin/open",
            args: vec![os(p)],
            raw: None,
        },
        (Platform::Windows, Target::File(p)) => {
            let mut raw = OsString::from("/select,\"");
            raw.push(p.as_os_str());
            raw.push("\"");
            Command {
                program: "explorer.exe",
                args: Vec::new(),
                raw: Some(raw),
            }
        }
        (Platform::Windows, Target::Folder(p)) => Command {
            program: "explorer.exe",
            args: vec![os(p)],
            raw: None,
        },
        // No file manager selects a file through `xdg-open`: its folder.
        (Platform::Linux, Target::File(p)) => Command {
            program: "xdg-open",
            args: vec![os(p.parent().unwrap_or(p))],
            raw: None,
        },
        (Platform::Linux, Target::Folder(p)) => Command {
            program: "xdg-open",
            args: vec![os(p)],
            raw: None,
        },
    }
}

/// Run a command without waiting for it (it is reaped in the background;
/// Explorer's exit code means nothing, and `open` returns at once).
fn spawn(cmd: &Command) -> std::io::Result<()> {
    let mut c = std::process::Command::new(cmd.program);
    c.args(&cmd.args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    if let Some(raw) = &cmd.raw {
        use std::os::windows::process::CommandExt;
        c.raw_arg(raw);
    }
    let mut child = c.spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Shows things in the file manager. Lives in the app state; a test switches
/// it to [`Revealer::record_only`] (a unit test's starts that way).
#[derive(Debug)]
pub struct Revealer {
    /// `Some`: record instead of running.
    recorded: Mutex<Option<Vec<Command>>>,
}

impl Default for Revealer {
    fn default() -> Self {
        Self {
            recorded: Mutex::new(cfg!(test).then(Vec::new)),
        }
    }
}

impl Revealer {
    fn lock(&self) -> MutexGuard<'_, Option<Vec<Command>>> {
        self.recorded.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// From now on, record each command instead of running it: nothing
    /// opens (tests).
    pub fn record_only(&self) {
        self.lock().get_or_insert_with(Vec::new);
    }

    /// The commands recorded so far.
    pub fn recorded(&self) -> Vec<Command> {
        self.lock().clone().unwrap_or_default()
    }

    /// Show `target` in the file manager.
    pub fn reveal(&self, target: &Target) -> std::io::Result<()> {
        let cmd = command(Platform::current(), target);
        if let Some(recorded) = self.lock().as_mut() {
            recorded.push(cmd);
            return Ok(());
        }
        spawn(&cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(c: &Command) -> Vec<String> {
        c.args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn finder_selects_the_file_and_opens_a_folder() {
        let file = PathBuf::from("/Users/me/.claude/projects/-Users-me/e5ecf333.jsonl");
        let c = command(Platform::MacOs, &Target::File(file.clone()));
        assert_eq!(c.program, "/usr/bin/open");
        assert_eq!(args(&c), ["-R", file.to_str().unwrap()]);
        assert_eq!(c.raw, None);
        let dir = PathBuf::from("/Users/me/My Projects/.claude/projects/-a");
        let c = command(Platform::MacOs, &Target::Folder(dir.clone()));
        assert_eq!(args(&c), [dir.to_str().unwrap()]);
    }

    #[test]
    fn explorer_gets_select_raw_so_a_path_with_spaces_works() {
        let file = PathBuf::from(r"C:\Users\Me\My Projects\.claude\projects\C--a\e5.jsonl");
        let c = command(Platform::Windows, &Target::File(file));
        assert_eq!(c.program, "explorer.exe");
        assert!(c.args.is_empty(), "nothing Rust would quote");
        assert_eq!(
            c.raw.unwrap().to_string_lossy(),
            r#"/select,"C:\Users\Me\My Projects\.claude\projects\C--a\e5.jsonl""#
        );
        let dir = PathBuf::from(r"C:\Users\Me\.claude\projects\C--a");
        let c = command(Platform::Windows, &Target::Folder(dir));
        assert_eq!(args(&c), [r"C:\Users\Me\.claude\projects\C--a"]);
        assert_eq!(c.raw, None);
    }

    #[test]
    fn xdg_open_opens_the_files_folder() {
        let file = PathBuf::from("/home/me/.claude/projects/-home-me/e5.jsonl");
        let c = command(Platform::Linux, &Target::File(file));
        assert_eq!(c.program, "xdg-open");
        assert_eq!(args(&c), ["/home/me/.claude/projects/-home-me"]);
        let c = command(
            Platform::Linux,
            &Target::Folder(PathBuf::from("/home/me/.claude/projects/-home-me")),
        );
        assert_eq!(args(&c), ["/home/me/.claude/projects/-home-me"]);
    }

    #[test]
    fn a_unit_test_only_ever_records() {
        let r = Revealer::default();
        r.reveal(&Target::Folder(PathBuf::from("/nowhere")))
            .unwrap();
        assert_eq!(r.recorded().len(), 1);
        assert_eq!(
            r.recorded()[0],
            command(
                Platform::current(),
                &Target::Folder(PathBuf::from("/nowhere"))
            )
        );
    }
}
