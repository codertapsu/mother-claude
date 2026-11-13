//! Git, run the one way a move may run it (plan §5.10, F3, F4, F8, F9):
//! argv only, the environment scrubbed of everything that would point git
//! elsewhere, `GIT_OPTIONAL_LOCKS=0`, commits never signed, no maintenance,
//! and the repository's hooks replaced by a fresh, private, empty folder per
//! operation — checked empty before each call (Git LFS writes its hooks
//! wherever `core.hooksPath` points, F9) and deleted after.

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use super::types::{io_err, CodeError};

/// Variables that would make git read or write another repository, index
/// or configuration than the one asked.
const SCRUBBED: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_NAMESPACE",
    "GIT_COMMON_DIR",
    "GIT_PREFIX",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
];

/// Settings every call gets.
const PRE: &[&str] = &[
    "-c",
    "commit.gpgSign=false",
    "-c",
    "tag.gpgSign=false",
    "-c",
    "gc.auto=0",
    "-c",
    "maintenance.auto=false",
    "-c",
    "advice.detachedHead=false",
    // An fsmonitor hook is a program the repository names: never run it.
    "-c",
    "core.fsmonitor=false",
];

/// F8: what git prints when it skipped a path it could not read.
const UNREADABLE: &[&str] = &["Filename too long", "could not open directory"];

/// How to run git; [`GitRunner::begin`] makes one operation's runner.
#[derive(Clone, Debug)]
pub struct GitRunner {
    program: PathBuf,
    isolated: bool,
}

impl Default for GitRunner {
    fn default() -> Self {
        Self {
            program: PathBuf::from("git"),
            isolated: false,
        }
    }
}

impl GitRunner {
    /// `git` from `PATH`, with the user's own configuration.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_program(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            isolated: false,
        }
    }

    /// Tests: no global or system configuration, a fixed identity, `main`
    /// as the default branch and local submodule clones allowed.
    #[doc(hidden)]
    pub fn isolated(mut self) -> Self {
        self.isolated = true;
        self
    }

    pub fn is_isolated(&self) -> bool {
        self.isolated
    }

    /// One operation: a fresh private folder for the hooks override and the
    /// index copies, deleted when the [`GitOp`] is dropped.
    pub fn begin(&self) -> Result<GitOp, CodeError> {
        let dir = std::env::temp_dir().join(format!("mc-git-{}", uuid::Uuid::new_v4().simple()));
        private_dir(&dir)?;
        let op = GitOp {
            runner: self.clone(),
            hooks: dir.join("hooks"),
            scratch: dir.join("work"),
            dir,
            intrusions: Mutex::new(Vec::new()),
            seq: AtomicU64::new(0),
        };
        private_dir(&op.hooks)?;
        private_dir(&op.scratch)?;
        if self.isolated {
            std::fs::write(op.dir.join("empty.gitconfig"), b"")
                .map_err(io_err("write an empty git configuration"))?;
            private_dir(&op.dir.join("xdg"))?;
        }
        Ok(op)
    }
}

/// A folder only this user may read (on Windows the temp folder already is).
fn private_dir(p: &Path) -> Result<(), CodeError> {
    #[cfg(unix)]
    let made = {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(p)
    };
    #[cfg(not(unix))]
    let made = std::fs::DirBuilder::new().create(p);
    made.map_err(io_err(format!("create {}", p.display())))
}

/// One operation's git: its own hooks folder and scratch space.
#[derive(Debug)]
pub struct GitOp {
    runner: GitRunner,
    dir: PathBuf,
    hooks: PathBuf,
    scratch: PathBuf,
    intrusions: Mutex<Vec<String>>,
    seq: AtomicU64,
}

impl Drop for GitOp {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir_all(&self.dir) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(dir = %self.dir.display(), error = %e, "a git operation's folder was left behind");
            }
        }
    }
}

/// What a git call printed.
#[derive(Debug)]
pub struct GitOutput {
    pub success: bool,
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl GitOp {
    /// The operation's private folder (removed on drop).
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The folder `core.hooksPath` points at.
    pub fn hooks_dir(&self) -> &Path {
        &self.hooks
    }

    pub fn is_isolated(&self) -> bool {
        self.runner.isolated
    }

    /// Hooks something wrote into the hooks folder, removed before the next
    /// call (F9).
    pub fn intrusions(&self) -> Vec<String> {
        self.intrusions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// A fresh path in the operation's private folder.
    pub fn scratch_path(&self, name: &str) -> PathBuf {
        let n = self.seq.fetch_add(1, Ordering::Relaxed);
        self.scratch.join(format!("{n}-{name}"))
    }

    /// A git call in `cwd`.
    pub fn cmd(&self, cwd: &Path) -> Git<'_> {
        Git {
            op: self,
            cwd: cwd.to_path_buf(),
            args: Vec::new(),
            env: Vec::new(),
            input: None,
            fail_on_unreadable: false,
        }
    }

    /// `git <args>`'s output, trimmed; an error when it fails.
    pub fn git(&self, cwd: &Path, args: &[&str]) -> Result<String, CodeError> {
        self.cmd(cwd).args(args).text()
    }

    /// `rev-parse -q --verify <spec>`: the object, or none. `spec` is one of
    /// ours (a checked ref or id), never an option.
    pub fn rev(&self, cwd: &Path, spec: &str) -> Result<Option<String>, CodeError> {
        if spec.starts_with('-') {
            return Err(CodeError::Invalid {
                what: "revision",
                value: spec.to_string(),
            });
        }
        let out = self
            .cmd(cwd)
            .args(["rev-parse", "-q", "--verify", spec])
            .output()?;
        let text = trim_text(&out.stdout);
        Ok((out.success && !text.is_empty()).then_some(text))
    }

    /// F9: the hooks folder is empty before every call.
    fn ensure_hooks_empty(&self) -> Result<(), CodeError> {
        let entries = match std::fs::read_dir(&self.hooks) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return private_dir(&self.hooks);
            }
            Err(e) => return Err(CodeError::Hooks(e.to_string())),
        };
        let mut found = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| CodeError::Hooks(e.to_string()))?;
            let path = entry.path();
            let removed = if path.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            removed.map_err(|e| CodeError::Hooks(format!("{}: {e}", path.display())))?;
            found.push(entry.file_name().to_string_lossy().into_owned());
        }
        if !found.is_empty() {
            tracing::warn!(hooks = ?found, "hooks were written into Mother Claude's private hooks folder; removed before the next git call");
            let mut list = self.intrusions.lock().unwrap_or_else(|e| e.into_inner());
            for f in found {
                if !list.contains(&f) {
                    list.push(f);
                }
            }
        }
        Ok(())
    }

    fn command(&self, cwd: &Path) -> Command {
        let mut cmd = crate::proc::std_command(&self.runner.program);
        cmd.current_dir(cwd);
        for k in SCRUBBED {
            cmd.env_remove(k);
        }
        for (k, _) in std::env::vars_os() {
            let upper = k.to_string_lossy().to_ascii_uppercase();
            if upper.starts_with("GIT_CONFIG_KEY_") || upper.starts_with("GIT_CONFIG_VALUE_") {
                cmd.env_remove(&k);
            }
        }
        cmd.env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            // F8's warnings are matched in English.
            .env_remove("LC_ALL")
            .env("LC_MESSAGES", "C");
        if self.runner.isolated {
            cmd.env("GIT_CONFIG_GLOBAL", self.dir.join("empty.gitconfig"))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("XDG_CONFIG_HOME", self.dir.join("xdg"))
                .env("GIT_AUTHOR_NAME", "Mother Claude test")
                .env("GIT_AUTHOR_EMAIL", "test@mother-claude.invalid")
                .env("GIT_COMMITTER_NAME", "Mother Claude test")
                .env("GIT_COMMITTER_EMAIL", "test@mother-claude.invalid");
        }
        cmd.args(PRE);
        let hooks = self.hooks.to_string_lossy().replace('\\', "/");
        cmd.arg("-c").arg(format!("core.hooksPath={hooks}"));
        if cfg!(windows) {
            // F8: without it a long path is left out with only a warning.
            cmd.args(["-c", "core.longpaths=true"]);
        }
        if self.runner.isolated {
            cmd.args([
                "-c",
                "init.defaultBranch=main",
                "-c",
                "protocol.file.allow=always",
            ]);
        }
        cmd
    }
}

/// One git call being put together.
pub struct Git<'a> {
    op: &'a GitOp,
    cwd: PathBuf,
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    input: Option<Vec<u8>>,
    fail_on_unreadable: bool,
}

impl Git<'_> {
    pub fn arg(mut self, a: impl AsRef<OsStr>) -> Self {
        self.args.push(a.as_ref().to_os_string());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args
            .extend(args.into_iter().map(|a| a.as_ref().to_os_string()));
        self
    }

    pub fn env(mut self, k: impl AsRef<OsStr>, v: impl AsRef<OsStr>) -> Self {
        self.env
            .push((k.as_ref().to_os_string(), v.as_ref().to_os_string()));
        self
    }

    pub fn input(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.input = Some(bytes.into());
        self
    }

    /// F8: a warning that git skipped a path it could not read fails the
    /// call (a snapshot would silently lose the file).
    pub fn fail_on_unreadable(mut self) -> Self {
        self.fail_on_unreadable = true;
        self
    }

    fn describe(&self) -> String {
        let all: Vec<String> = self
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let s = all.join(" ");
        if s.chars().count() > 200 {
            format!("{}…", s.chars().take(200).collect::<String>())
        } else {
            s
        }
    }

    /// Runs it; never an error for a non-zero exit.
    pub fn output(self) -> Result<GitOutput, CodeError> {
        self.op.ensure_hooks_empty()?;
        let mut cmd = self.op.command(&self.cwd);
        cmd.args(&self.args);
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        cmd.stdin(if self.input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                CodeError::NoGit(e)
            } else {
                io_err(format!("start git {}", self.describe()))(e)
            }
        })?;
        let writer = match (self.input.clone(), child.stdin.take()) {
            (Some(input), Some(mut stdin)) => Some(std::thread::spawn(move || {
                let _ = stdin.write_all(&input);
            })),
            _ => None,
        };
        let out = child
            .wait_with_output()
            .map_err(io_err(format!("run git {}", self.describe())))?;
        if let Some(w) = writer {
            let _ = w.join();
        }
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        if self.fail_on_unreadable {
            if let Some(line) = stderr
                .lines()
                .find(|l| UNREADABLE.iter().any(|m| l.contains(m)))
            {
                return Err(CodeError::Unreadable(line.trim().to_string()));
            }
        }
        Ok(GitOutput {
            success: out.status.success(),
            code: out.status.code().unwrap_or(-1),
            stdout: out.stdout,
            stderr,
        })
    }

    /// Its stdout, untouched; an error when it fails.
    pub fn raw(self) -> Result<Vec<u8>, CodeError> {
        let command = self.describe();
        let out = self.output()?;
        if !out.success {
            let mut stderr = out.stderr.trim().to_string();
            if stderr.chars().count() > 1500 {
                stderr = stderr.chars().take(1500).collect::<String>() + "…";
            }
            return Err(CodeError::Git {
                command,
                code: out.code,
                stderr,
            });
        }
        Ok(out.stdout)
    }

    /// Its stdout as text, the last line break trimmed.
    pub fn text(self) -> Result<String, CodeError> {
        Ok(trim_text(&self.raw()?))
    }

    /// Whether it succeeded.
    pub fn ok(self) -> Result<bool, CodeError> {
        Ok(self.output()?.success)
    }
}

/// Lossy text without the trailing line break.
pub fn trim_text(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    let s = s.strip_suffix('\n').unwrap_or(&s);
    s.strip_suffix('\r').unwrap_or(s).to_string()
}
