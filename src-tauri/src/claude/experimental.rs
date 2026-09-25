//! EXPERIMENTAL, UNSANCTIONED foreign-session control (Stage 3).
//!
//! Compiled only with `--features experimental`. These paths use undocumented,
//! unstable internals that break across Claude Code versions. They are OFF by
//! default and gated in the UI behind an explicit "uses unstable, unsupported
//! internals" confirmation. **Do not** rely on them.
//!
//! - PTY-drive `claude attach <id>`: Ink discards piped `\n`, so a real PTY is
//!   required to inject keystrokes into a foreign live session.
//! - CCR v1 transport (reverse-engineered `--sdk-url` / `--sdk-server`,
//!   `CLAUDE_CODE_USE_CCR_V2`): reportedly unauthenticated — **unverified**; not
//!   implemented here beyond a status probe. We never speak the cc-daemon
//!   control socket (authenticated, rotating key, breaks on auto-update).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};

use crate::state::AppState;

struct PtyHandle {
    writer: Mutex<Box<dyn Write + Send>>,
    _child: Box<dyn portable_pty::Child + Send + Sync>,
    /// Which attach created it (the EOF reaper only removes its own handle).
    generation: u64,
}

/// Registry of PTY-attached foreign sessions.
#[derive(Default)]
pub struct PtyRegistry {
    sessions: Arc<Mutex<HashMap<String, PtyHandle>>>,
    generations: AtomicU64,
}

impl PtyRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_attached(&self, id: &str) -> bool {
        self.sessions
            .lock()
            .map(|m| m.contains_key(id))
            .unwrap_or(false)
    }

    /// Attach to a background session in a PTY running `claude attach
    /// <short>`. `id` must be the SHORT background job id: `claude attach`
    /// rejects session UUIDs (and interactive VS Code / terminal sessions have
    /// no job at all), exiting at once while a handle stayed registered.
    pub fn attach(&self, state: &AppState, id: &str, cwd: &str) -> Result<()> {
        if !crate::claude::control::is_short_job_id(id) {
            return Err(anyhow!(
                "PTY attach works only for background sessions (claude --bg) with a job id"
            ));
        }
        if self.is_attached(id) {
            return Ok(());
        }
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: 40,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("openpty")?;

        let mut cmd = CommandBuilder::new(crate::claude::claude_bin());
        cmd.arg("attach");
        cmd.arg(id);
        if !cwd.is_empty() {
            cmd.cwd(cwd);
        }
        let child = pair
            .slave
            .spawn_command(cmd)
            .context("spawn claude attach")?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().context("clone pty reader")?;
        let writer = pair.master.take_writer().context("take pty writer")?;

        // Drain the PTY so the child never blocks on a full buffer. The raw
        // terminal output is not broadcast (it can contain anything the
        // session prints); B4 may keep a bounded tail for the terminal view.
        // On EOF (the attach exited: job gone, detached, refused) the handle
        // is reaped, so `is_attached` never reports a dead PTY.
        let _ = state;
        let sid = id.to_string();
        let generation = self.generations.fetch_add(1, Ordering::Relaxed);
        let sessions = self.sessions.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => tracing::trace!(session = %sid, bytes = n, "pty output"),
                }
            }
            if let Ok(mut map) = sessions.lock() {
                if map.get(&sid).is_some_and(|h| h.generation == generation) {
                    map.remove(&sid);
                    tracing::info!(session = %sid, "PTY attach ended; handle reaped");
                }
            }
        });

        if let Ok(mut map) = self.sessions.lock() {
            map.insert(
                id.to_string(),
                PtyHandle {
                    writer: Mutex::new(writer),
                    _child: child,
                    generation,
                },
            );
        }
        tracing::warn!(session = %id, "EXPERIMENTAL: attached foreign session via PTY");
        Ok(())
    }

    /// Inject text + Enter (`\r`) into an attached foreign session.
    pub fn inject(&self, id: &str, text: &str) -> Result<()> {
        let map = self
            .sessions
            .lock()
            .map_err(|_| anyhow!("pty registry poisoned"))?;
        let handle = map
            .get(id)
            .ok_or_else(|| anyhow!("session {id} is not PTY-attached; attach first"))?;
        let mut writer = handle
            .writer
            .lock()
            .map_err(|_| anyhow!("pty writer poisoned"))?;
        writer.write_all(text.as_bytes())?;
        writer.write_all(b"\r")?;
        writer.flush()?;
        Ok(())
    }
}

/// CCR v1 transport status. Unverified and intentionally not implemented.
pub fn ccr_v1_status() -> &'static str {
    "CCR v1 transport is unverified and disabled; verify the 'no authentication' \
     claim independently before relying on it (see KNOWN_ISSUES.md)"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_starts_empty() {
        let r = PtyRegistry::new();
        assert!(!r.is_attached("x"));
    }

    #[test]
    fn attach_requires_a_short_job_id() {
        let dir = tempfile::tempdir().unwrap();
        let st = crate::state::Inner::new(
            crate::claude::ClaudeHome::with_base(dir.path()),
            crate::state::ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            crate::server::auth::Auth::ephemeral(),
        );
        let r = PtyRegistry::new();
        let err = r
            .attach(&st, "631b387d-a7e5-4e12-ab38-522cb612803b", "")
            .unwrap_err();
        assert!(err.to_string().contains("background"));
        assert!(!r.is_attached("631b387d-a7e5-4e12-ab38-522cb612803b"));
    }

    #[test]
    fn ccr_is_disabled() {
        assert!(ccr_v1_status().contains("unverified"));
    }
}
