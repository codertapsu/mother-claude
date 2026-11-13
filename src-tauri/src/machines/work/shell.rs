//! How a run runs here (docs/PLAN-MACHINES.md Phase 4, decision 3).
//!
//! * **The shell**: macOS `/bin/zsh -lc` (the login shell's `PATH`, on top
//!   of the one the app recovered for `claude` at start); Windows
//!   PowerShell 7 (`pwsh`) when it is installed, else Windows PowerShell,
//!   both `-NoLogo -NoProfile -NonInteractive`, the command handed over
//!   encoded (`-EncodedCommand`, no quoting to get wrong) in a wrapper that
//!   reports a program's own exit code as the run's; elsewhere `bash -lc`,
//!   else `sh -c`. `machine_list` names each computer's shell so Claude
//!   writes the command for it.
//! * **The environment**: Mother Claude's own, scrubbed as an owned
//!   session's child is (`control::SCRUBBED_ENV`, the account's tokens, API
//!   keys and provider switches, `MOTHER_CLAUDE_OWNED`), plus
//!   `MOTHER_CLAUDE_RUN=1` and `GIT_TERMINAL_PROMPT=0` (nothing may wait on
//!   a prompt nobody sees). Its input is empty.
//! * **The tree**: the shell starts in a process group of its own (unix)
//!   or a job object that ends with it (Windows), so a cancel or a timeout
//!   ends everything it started — and so does the end of the command, for
//!   whatever it left running in the background: a run leaves nothing
//!   behind that would hold its work folder's files.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;

/// How long a stopped tree gets to end before it is killed (unix).
#[cfg(unix)]
const GRACE: Duration = Duration::from_secs(3);
/// How long the output may take to drain once the command ended.
const DRAIN: Duration = Duration::from_secs(5);

/// The shells a run may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShellKind {
    Zsh,
    Bash,
    Sh,
    /// PowerShell 7.
    Pwsh,
    /// Windows PowerShell 5.1.
    Powershell,
}

impl ShellKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ShellKind::Zsh => "zsh",
            ShellKind::Bash => "bash",
            ShellKind::Sh => "sh",
            ShellKind::Pwsh => "pwsh",
            ShellKind::Powershell => "powershell",
        }
    }

    /// The name people know it by.
    pub fn label(self) -> &'static str {
        match self {
            ShellKind::Zsh => "zsh",
            ShellKind::Bash => "bash",
            ShellKind::Sh => "sh",
            ShellKind::Pwsh => "PowerShell 7",
            ShellKind::Powershell => "Windows PowerShell",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "zsh" => Some(ShellKind::Zsh),
            "bash" => Some(ShellKind::Bash),
            "sh" => Some(ShellKind::Sh),
            "pwsh" => Some(ShellKind::Pwsh),
            "powershell" => Some(ShellKind::Powershell),
            _ => None,
        }
    }

    pub fn is_powershell(self) -> bool {
        matches!(self, ShellKind::Pwsh | ShellKind::Powershell)
    }
}

/// A shell: which, and its program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shell {
    pub kind: ShellKind,
    pub program: PathBuf,
}

impl Shell {
    /// This computer's (found once).
    pub fn here() -> &'static Shell {
        static HERE: OnceLock<Shell> = OnceLock::new();
        HERE.get_or_init(detect)
    }

    /// The arguments (after the program) that run `command`.
    pub fn args(&self, command: &str) -> Vec<OsString> {
        match self.kind {
            ShellKind::Zsh | ShellKind::Bash => vec!["-lc".into(), command.into()],
            ShellKind::Sh => vec!["-c".into(), command.into()],
            ShellKind::Pwsh | ShellKind::Powershell => vec![
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-EncodedCommand".into(),
                crate::machines::setup::encode_ps(&powershell_script(command)).into(),
            ],
        }
    }
}

/// The script PowerShell runs for `command`: its output in UTF-8, and the
/// exit code of the program it ran last (a cmdlet that failed: 1) as its
/// own — PowerShell would otherwise turn every native code into 0 or 1.
pub fn powershell_script(command: &str) -> String {
    format!(
        "$ErrorActionPreference = 'Continue'\n\
         try {{ [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 }} catch {{}}\n\
         $OutputEncoding = [System.Text.Encoding]::UTF8\n\
         $global:LASTEXITCODE = 0\n\
         {command}\n\
         $mcRunOk = $?\n\
         if ($null -ne $global:LASTEXITCODE -and $global:LASTEXITCODE -ne 0) {{ exit $global:LASTEXITCODE }}\n\
         if (-not $mcRunOk) {{ exit 1 }}\n\
         exit 0\n"
    )
}

#[cfg(not(target_os = "macos"))]
fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|d| !d.as_os_str().is_empty())
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

#[cfg(windows)]
fn detect() -> Shell {
    let pwsh = on_path("pwsh.exe").or_else(|| {
        ["ProgramFiles", "ProgramW6432"]
            .iter()
            .filter_map(std::env::var_os)
            .map(|d| PathBuf::from(d).join(r"PowerShell\7\pwsh.exe"))
            .find(|p| p.is_file())
    });
    if let Some(program) = pwsh {
        return Shell {
            kind: ShellKind::Pwsh,
            program,
        };
    }
    let root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let ps = root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    Shell {
        kind: ShellKind::Powershell,
        program: if ps.is_file() {
            ps
        } else {
            PathBuf::from("powershell.exe")
        },
    }
}

#[cfg(target_os = "macos")]
fn detect() -> Shell {
    let zsh = Path::new("/bin/zsh");
    if zsh.is_file() {
        return Shell {
            kind: ShellKind::Zsh,
            program: zsh.to_path_buf(),
        };
    }
    Shell {
        kind: ShellKind::Sh,
        program: PathBuf::from("/bin/sh"),
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn detect() -> Shell {
    match on_path("bash").or_else(|| Some(PathBuf::from("/bin/bash")).filter(|p| p.is_file())) {
        Some(program) => Shell {
            kind: ShellKind::Bash,
            program,
        },
        None => Shell {
            kind: ShellKind::Sh,
            program: PathBuf::from("/bin/sh"),
        },
    }
}

/// Git's variables that would point a build's git at another repository.
const GIT_ENV: [&str; 6] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
];

/// The run's environment: scrubbed as an owned session's child is, plus
/// `MOTHER_CLAUDE_RUN=1`.
pub fn scrub(cmd: &mut tokio::process::Command) {
    for key in crate::claude::control::SCRUBBED_ENV {
        cmd.env_remove(key);
    }
    for key in crate::claude::accounts::ACCOUNT_OVERRIDE_ENV {
        cmd.env_remove(key);
    }
    for key in GIT_ENV {
        cmd.env_remove(key);
    }
    cmd.env_remove(crate::claude::control::OWNED_ENV);
    cmd.env_remove("MOTHER_CLAUDE_SESSION");
    cmd.env("MOTHER_CLAUDE_RUN", "1");
    cmd.env("GIT_TERMINAL_PROMPT", "0");
}

// ─── the process tree ──────────────────────────────────────────────────────────

/// A job object holding a run's processes; closing it ends them.
#[cfg(windows)]
struct Job(windows_sys::Win32::Foundation::HANDLE);

// SAFETY: a job handle is a kernel handle, usable from any thread.
#[cfg(windows)]
unsafe impl Send for Job {}
// SAFETY: as above; the calls made through it are thread-safe.
#[cfg(windows)]
unsafe impl Sync for Job {}

#[cfg(windows)]
impl Job {
    /// A job that ends its processes when its last handle closes, holding
    /// `child` (and so everything `child` starts from now on).
    fn holding(child: &tokio::process::Child) -> Option<Job> {
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        let process = child.raw_handle()? as HANDLE;
        // SAFETY: plain Win32 calls; `info` lives across the call that
        // reads it, and the handles are closed on every failure.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let set = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if set == 0 || AssignProcessToJobObject(job, process) == 0 {
                CloseHandle(job);
                return None;
            }
            Some(Job(job))
        }
    }

    fn terminate(&self) {
        // SAFETY: our own job handle, open until drop.
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0, 1);
        }
    }
}

#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: our own handle, closed once.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

/// A started shell and everything it starts.
pub struct Tree {
    pub child: tokio::process::Child,
    #[cfg(windows)]
    job: Option<Job>,
    #[cfg(unix)]
    group: Option<i32>,
}

impl Tree {
    /// Starts `cmd` in a process group of its own (unix) or a job object
    /// (Windows, no console window).
    pub fn spawn(mut cmd: tokio::process::Command) -> std::io::Result<Tree> {
        #[cfg(unix)]
        cmd.process_group(0);
        #[cfg(windows)]
        cmd.creation_flags(crate::proc::CREATE_NO_WINDOW);
        cmd.kill_on_drop(true);
        let child = cmd.spawn()?;
        #[cfg(windows)]
        {
            let job = Job::holding(&child);
            if job.is_none() {
                tracing::warn!("a run's process could not be put in a job object: a cancel ends the shell only");
            }
            Ok(Tree { child, job })
        }
        #[cfg(unix)]
        {
            let group = child.id().and_then(|p| i32::try_from(p).ok());
            Ok(Tree { child, group })
        }
        #[cfg(not(any(unix, windows)))]
        Ok(Tree { child })
    }

    /// Ends the whole tree: asked first (SIGTERM), killed after a grace
    /// period (unix); the job ended at once (Windows).
    pub async fn kill(&mut self) {
        #[cfg(unix)]
        if let Some(g) = self.group {
            // SAFETY: plain syscalls on our own process group.
            unsafe {
                libc::killpg(g, libc::SIGTERM);
            }
            let _ = tokio::time::timeout(GRACE, self.child.wait()).await;
            // SAFETY: as above.
            unsafe {
                libc::killpg(g, libc::SIGKILL);
            }
        }
        #[cfg(windows)]
        if let Some(j) = &self.job {
            j.terminate();
        }
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
    }

    /// After the shell ended: whatever it left running in the background
    /// ends too.
    pub async fn sweep(&mut self) {
        #[cfg(unix)]
        if let Some(g) = self.group {
            // SAFETY: plain syscalls on our own process group; signal 0
            // only asks whether anything is left in it.
            let left = unsafe { libc::killpg(g, 0) } == 0;
            if left {
                // SAFETY: as above.
                unsafe {
                    libc::killpg(g, libc::SIGTERM);
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
                // SAFETY: as above.
                unsafe {
                    libc::killpg(g, libc::SIGKILL);
                }
            }
        }
        #[cfg(windows)]
        if let Some(j) = &self.job {
            j.terminate();
        }
    }
}

// ─── running ───────────────────────────────────────────────────────────────────

/// How a command ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    /// By itself, with this exit code (none: a signal ended it).
    Exited(Option<i32>),
    Cancelled,
    TimedOut,
}

/// Where a run's output goes as it arrives.
pub type Sink = Arc<dyn Fn(&[u8]) + Send + Sync>;

async fn until_true(rx: &mut tokio::sync::watch::Receiver<bool>) {
    loop {
        if *rx.borrow_and_update() {
            return;
        }
        if rx.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

fn pump<R>(reader: Option<R>, sink: Sink) -> tokio::task::JoinHandle<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let Some(mut r) = reader else { return };
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match r.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => sink(&buf[..n]),
            }
        }
    })
}

/// Runs `command` in `dir` with `shell`, its output (stdout and stderr as
/// they come) into `sink`, until it ends, `cancel` turns true or `deadline`
/// passes; then its whole tree ends.
pub async fn execute(
    shell: &Shell,
    command: &str,
    dir: &Path,
    sink: Sink,
    mut cancel: tokio::sync::watch::Receiver<bool>,
    deadline: tokio::time::Instant,
) -> std::io::Result<Ending> {
    let mut cmd = crate::proc::command(&shell.program);
    cmd.args(shell.args(command))
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    scrub(&mut cmd);
    let mut tree = Tree::spawn(cmd)?;
    let out = pump(tree.child.stdout.take(), sink.clone());
    let err = pump(tree.child.stderr.take(), sink);
    enum Why {
        Exit(std::io::Result<std::process::ExitStatus>),
        Cancel,
        Timeout,
    }
    let why = {
        let wait = tree.child.wait();
        tokio::pin!(wait);
        tokio::select! {
            status = &mut wait => Why::Exit(status),
            _ = until_true(&mut cancel) => Why::Cancel,
            _ = tokio::time::sleep_until(deadline) => Why::Timeout,
        }
    };
    let ending = match why {
        Why::Exit(Ok(status)) => {
            tree.sweep().await;
            Ending::Exited(status.code())
        }
        Why::Exit(Err(e)) => {
            tree.kill().await;
            return Err(e);
        }
        Why::Cancel => {
            tree.kill().await;
            Ending::Cancelled
        }
        Why::Timeout => {
            tree.kill().await;
            Ending::TimedOut
        }
    };
    // What it wrote last (a process that escaped the tree may hold a pipe
    // open: never waited on for long).
    let _ = tokio::time::timeout(DRAIN, async {
        let _ = out.await;
        let _ = err.await;
    })
    .await;
    Ok(ending)
}

/// The readiness test's run (docs/PLAN-MACHINES.md Phase 4): a fixed
/// `echo` that works in every shell here, in a scratch folder.
pub const ECHO: &str = "echo machine-run-ok";
/// What it prints.
pub const ECHO_SAYS: &str = "machine-run-ok";

/// Runs [`ECHO`]: its exit code, output and duration.
pub async fn echo() -> Result<(Option<i32>, String, u64), String> {
    let dir = std::env::temp_dir().join(format!("mc-run-echo-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let result = echo_in(&dir).await;
    let _ = std::fs::remove_dir_all(&dir);
    result
}

async fn echo_in(dir: &Path) -> Result<(Option<i32>, String, u64), String> {
    let out = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let sink_out = out.clone();
    let sink: Sink = Arc::new(move |b: &[u8]| {
        let mut o = sink_out.lock().unwrap_or_else(|e| e.into_inner());
        if o.len() < 64 * 1024 {
            o.extend_from_slice(b);
        }
    });
    let started = std::time::Instant::now();
    let (_tx, rx) = tokio::sync::watch::channel(false);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let ending = execute(Shell::here(), ECHO, dir, sink, rx, deadline)
        .await
        .map_err(|e| format!("{} could not start: {e}", Shell::here().kind.label()))?;
    let text = String::from_utf8_lossy(&out.lock().unwrap_or_else(|e| e.into_inner())).into_owned();
    let ms = started.elapsed().as_millis() as u64;
    match ending {
        Ending::Exited(code) => Ok((code, text, ms)),
        Ending::TimedOut => Err("It did not finish within a minute.".into()),
        Ending::Cancelled => Err("It was cancelled.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_shell_gets_the_command_its_own_way() {
        let zsh = Shell {
            kind: ShellKind::Zsh,
            program: "/bin/zsh".into(),
        };
        assert_eq!(
            zsh.args("cargo test"),
            vec![OsString::from("-lc"), "cargo test".into()]
        );
        let sh = Shell {
            kind: ShellKind::Sh,
            program: "/bin/sh".into(),
        };
        assert_eq!(sh.args("make"), vec![OsString::from("-c"), "make".into()]);
        let ps = Shell {
            kind: ShellKind::Pwsh,
            program: "pwsh.exe".into(),
        };
        let args = ps.args("cargo test; echo \"done\"");
        assert_eq!(
            &args[..4],
            &[
                OsString::from("-NoLogo"),
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-EncodedCommand".into()
            ]
        );
        // Base64 of UTF-16LE: no quote survives to be misread.
        let encoded = args[4].to_string_lossy().into_owned();
        assert!(encoded
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'='));
        let script = powershell_script("cargo test; echo \"done\"");
        assert!(script.contains("\ncargo test; echo \"done\"\n"));
        assert!(script.contains("exit $global:LASTEXITCODE"));
        assert!(script.ends_with("exit 0\n"));
        for k in [
            ShellKind::Zsh,
            ShellKind::Bash,
            ShellKind::Sh,
            ShellKind::Pwsh,
            ShellKind::Powershell,
        ] {
            assert_eq!(ShellKind::parse(k.as_str()), Some(k));
        }
        assert!(ShellKind::Pwsh.is_powershell() && !ShellKind::Zsh.is_powershell());
    }

    #[test]
    fn this_computer_has_a_shell_of_its_os() {
        let s = Shell::here();
        if cfg!(windows) {
            assert!(s.kind.is_powershell(), "{s:?}");
        } else if cfg!(target_os = "macos") {
            assert_eq!(s.kind, ShellKind::Zsh);
        }
    }

    #[test]
    fn the_environment_loses_tokens_and_gains_the_marker() {
        let mut cmd = crate::proc::command("x");
        cmd.env("CLAUDE_CODE_OAUTH_TOKEN", "secret")
            .env("ANTHROPIC_API_KEY", "secret")
            .env("MOTHER_CLAUDE_TOKEN", "secret")
            .env("GIT_DIR", "/elsewhere");
        scrub(&mut cmd);
        let envs: Vec<(String, Option<String>)> = cmd
            .as_std()
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        let get = |k: &str| envs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        for gone in [
            "CLAUDE_CODE_OAUTH_TOKEN",
            "ANTHROPIC_API_KEY",
            "MOTHER_CLAUDE_TOKEN",
            "GIT_DIR",
            "MOTHER_CLAUDE_OWNED",
        ] {
            assert_eq!(get(gone), Some(None), "{gone} is removed");
        }
        assert_eq!(get("MOTHER_CLAUDE_RUN"), Some(Some("1".into())));
    }

    fn sink() -> (Sink, Arc<std::sync::Mutex<Vec<u8>>>) {
        let out = Arc::new(std::sync::Mutex::new(Vec::new()));
        let o = out.clone();
        (
            Arc::new(move |b: &[u8]| o.lock().unwrap().extend_from_slice(b)),
            out,
        )
    }

    #[tokio::test]
    async fn a_command_runs_with_its_exit_code_and_output() {
        let dir = tempfile::tempdir().unwrap();
        let (s, out) = sink();
        let (_tx, rx) = tokio::sync::watch::channel(false);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        let cmd = if cfg!(windows) {
            "echo first; cmd /c exit 7"
        } else {
            "echo first; exit 7"
        };
        let ending = execute(Shell::here(), cmd, dir.path(), s, rx, deadline)
            .await
            .unwrap();
        assert_eq!(ending, Ending::Exited(Some(7)));
        let text = String::from_utf8_lossy(&out.lock().unwrap()).into_owned();
        assert!(text.contains("first"), "{text:?}");
        let (code, text, _) = echo().await.unwrap();
        assert_eq!(code, Some(0));
        assert!(text.contains(ECHO_SAYS), "{text:?}");
    }

    /// A cancel and a timeout end the shell and what it started: a grand-
    /// child that would write a file a moment later never does.
    #[tokio::test]
    async fn a_cancel_or_a_timeout_ends_the_whole_tree() {
        for timeout in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let marker = dir.path().join("late.txt");
            let cmd = if cfg!(windows) {
                format!(
                    "Start-Process -NoNewWindow -FilePath '{}' -ArgumentList '-NoProfile','-Command','Start-Sleep -Seconds 6; Set-Content -Path ''{}'' -Value late'; Start-Sleep -Seconds 60",
                    Shell::here().program.display(),
                    marker.display()
                )
            } else {
                format!("(sleep 6; echo late > '{}') & sleep 60", marker.display())
            };
            let (s, _) = sink();
            let (tx, rx) = tokio::sync::watch::channel(false);
            let deadline = tokio::time::Instant::now()
                + if timeout {
                    Duration::from_millis(3500)
                } else {
                    Duration::from_secs(120)
                };
            let started = std::time::Instant::now();
            let run = tokio::spawn({
                let dir = dir.path().to_path_buf();
                async move { execute(Shell::here(), &cmd, &dir, s, rx, deadline).await }
            });
            if !timeout {
                tokio::time::sleep(Duration::from_millis(3500)).await;
                tx.send_replace(true);
            }
            let ending = run.await.unwrap().unwrap();
            assert_eq!(
                ending,
                if timeout {
                    Ending::TimedOut
                } else {
                    Ending::Cancelled
                }
            );
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "it ended early"
            );
            // Past the grandchild's 6 s: had it survived, it wrote the file.
            tokio::time::sleep(Duration::from_secs(5)).await;
            assert!(
                !marker.exists(),
                "the grandchild was ended too (timeout: {timeout})"
            );
        }
    }
}
