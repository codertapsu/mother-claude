//! Two-way control of **owned** sessions — sessions Mother Claude launches and
//! therefore can drive live.
//!
//! Path B (this module): a headless `claude -p --output-format stream-json
//! --input-format stream-json` subprocess. We pick the `--session-id`, so the
//! session id is known up front and registered as owned; Claude also writes the
//! normal transcript file, which the monitor already tails for the live view.
//! User messages and (later) permission decisions are written to the child's
//! stdin as NDJSON.
//!
//! Path A (the Agent SDK sidecar with `canUseTool` / `ask_user`) is added in the
//! remote-approval commit and is preferred for rich permission gating.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

use crate::state::{AppState, ServerEvent};

/// Options for launching an owned session.
#[derive(Debug, Clone)]
pub struct SpawnOptions {
    pub cwd: String,
    pub prompt: String,
    pub model: Option<String>,
    pub permission_mode: Option<String>,
    /// Resume (fork) an existing conversation by id — used to "continue" a
    /// foreign session (e.g. a VS Code one) as a fully-controllable owned session.
    pub resume: Option<String>,
}

struct OwnedHandle {
    stdin: AsyncMutex<Option<ChildStdin>>,
    child: AsyncMutex<Child>,
    cwd: String,
    started_at: i64,
}

/// Lightweight metadata for a live owned session, so the registry can list it
/// even before its transcript file exists.
#[derive(Debug, Clone)]
pub struct OwnedSessionMeta {
    pub id: String,
    pub cwd: String,
    pub started_at: i64,
}

type HandleMap = Arc<Mutex<HashMap<String, Arc<OwnedHandle>>>>;

/// Registry of live owned-session subprocess handles.
#[derive(Default)]
pub struct ControlRegistry {
    handles: HandleMap,
}

impl ControlRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// True if we hold a live control handle for this id.
    pub fn controls(&self, id: &str) -> bool {
        self.handles
            .lock()
            .map(|h| h.contains_key(id))
            .unwrap_or(false)
    }

    /// Metadata for every live owned session (for registry inclusion).
    pub fn live(&self) -> Vec<OwnedSessionMeta> {
        self.handles
            .lock()
            .map(|m| {
                m.iter()
                    .map(|(id, h)| OwnedSessionMeta {
                        id: id.clone(),
                        cwd: h.cwd.clone(),
                        started_at: h.started_at,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Launch a new owned session and return its id.
    ///
    /// Path A (the default when the Node sidecar is built/bundled): drive the
    /// session via the Agent SDK bridge, which gates tools with `canUseTool` and
    /// surfaces questions via `ask_user` to the dashboard. If the sidecar is
    /// absent, can't start (e.g. `node` missing), or is disabled with
    /// `MOTHER_CLAUDE_SIDECAR=0`, fall back to Path B: a headless `claude -p`
    /// stream-json subprocess.
    pub async fn spawn(&self, state: &AppState, opts: SpawnOptions) -> Result<String> {
        // Continuing a session resumes it *in place* (same id), so the existing
        // dashboard row becomes owned and drivable and the same transcript keeps
        // growing. Fresh sessions get a brand-new id.
        let id = opts
            .resume
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let permission_mode = opts
            .permission_mode
            .as_deref()
            .unwrap_or("default")
            .to_string();
        let token = state.auth.token.clone();

        // Resuming an existing conversation uses the headless path, whose
        // --resume semantics are well-defined.
        let entry = if opts.resume.is_some() {
            None
        } else {
            sidecar_entry()
        };
        let mut child = match entry {
            Some(entry) => {
                tracing::info!(session = %id, "spawning owned session via sidecar (Path A)");
                let url = server_url(state);
                match sidecar_command(&entry, &id, &opts, &permission_mode, &token, &url).spawn() {
                    Ok(child) => child,
                    Err(e) => {
                        tracing::warn!(error = %e, "sidecar failed to start; falling back to headless (Path B)");
                        headless_command(&id, &opts, &permission_mode, &token)
                            .spawn()
                            .context("failed to spawn claude (Path B fallback)")?
                    }
                }
            }
            None => headless_command(&id, &opts, &permission_mode, &token)
                .spawn()
                .with_context(|| format!("failed to spawn `{}`", crate::claude::claude_bin()))?,
        };

        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let handle = Arc::new(OwnedHandle {
            stdin: AsyncMutex::new(stdin),
            child: AsyncMutex::new(child),
            cwd: opts.cwd.clone(),
            started_at: crate::claude::registry::now_ms(),
        });

        if let Ok(mut map) = self.handles.lock() {
            map.insert(id.clone(), handle.clone());
        }
        // Register ownership + surface the session in the dashboard immediately.
        state.mark_owned(&id, &opts.cwd, handle.started_at).await;
        let verb = if opts.resume.is_some() {
            "Continuing"
        } else {
            "Spawned"
        };
        state.broadcast(ServerEvent::Notice(format!("{verb} owned session {id}")));

        // Drain stdout/stderr so the child never blocks; the transcript file tail
        // handles the live view, so here we mostly watch for completion.
        if let Some(stdout) = stdout {
            let st = state.clone();
            let sid = id.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                        if value.get("type").and_then(|t| t.as_str()) == Some("result") {
                            st.broadcast(ServerEvent::Notice(format!(
                                "Session {sid} turn complete"
                            )));
                        }
                    }
                }
            });
        }
        if let Some(stderr) = stderr {
            let sid = id.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!(session = %sid, "claude stderr: {line}");
                }
            });
        }

        // Reap the child when it exits and drop its control handle.
        {
            let registry_handle = handle.clone();
            let st = state.clone();
            let sid = id.clone();
            let map = self.handles.clone();
            tokio::spawn(async move {
                let status = registry_handle.child.lock().await.wait().await;
                tracing::info!(session = %sid, ?status, "owned session exited");
                if let Ok(mut m) = map.lock() {
                    m.remove(&sid);
                }
                st.broadcast(ServerEvent::Notice(format!("Session {sid} exited")));
            });
        }

        // The initial prompt is passed as argv above; follow-ups go through
        // send_message.
        Ok(id)
    }

    /// Send a user message to an owned session's stdin (stream-json input).
    pub async fn send_message(&self, id: &str, text: &str) -> Result<()> {
        let handle = self
            .get(id)
            .ok_or_else(|| anyhow!("session {id} is not an owned, controllable session"))?;
        let line = user_message_json(text);
        let mut guard = handle.stdin.lock().await;
        let stdin = guard
            .as_mut()
            .ok_or_else(|| anyhow!("session {id} stdin is closed"))?;
        stdin.write_all(line.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        Ok(())
    }

    /// Write a raw NDJSON control line to an owned session's stdin (used by the
    /// permission/answer bridge).
    pub async fn send_raw(&self, id: &str, line: &str) -> Result<()> {
        let handle = self
            .get(id)
            .ok_or_else(|| anyhow!("session {id} is not controllable"))?;
        let mut guard = handle.stdin.lock().await;
        let stdin = guard
            .as_mut()
            .ok_or_else(|| anyhow!("session {id} stdin is closed"))?;
        stdin.write_all(line.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        Ok(())
    }

    /// Terminate an owned session's subprocess (best effort).
    pub async fn kill(&self, id: &str) -> Result<()> {
        if let Some(handle) = self.get(id) {
            let _ = handle.child.lock().await.start_kill();
        }
        Ok(())
    }

    fn get(&self, id: &str) -> Option<Arc<OwnedHandle>> {
        self.handles.lock().ok().and_then(|m| m.get(id).cloned())
    }
}

/// Whether foreign-session injection (PTY-driving `claude attach`) is available.
/// Requires the `experimental` capability to be compiled in (on by default) and
/// is enabled at runtime unless `MOTHER_CLAUDE_FOREIGN_INJECTION` is `0`/off.
pub fn foreign_injection_enabled() -> bool {
    if !cfg!(feature = "experimental") {
        return false;
    }
    match std::env::var("MOTHER_CLAUDE_FOREIGN_INJECTION") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            !(v == "0" || v == "false" || v == "off" || v == "no")
        }
        Err(_) => true,
    }
}

/// Resolve the built sidecar entry point. Path A is used by default whenever the
/// sidecar is present; set `MOTHER_CLAUDE_SIDECAR=0` to force the headless path.
/// `MOTHER_CLAUDE_SIDECAR_PATH` (set at startup for packaged builds) wins.
fn sidecar_entry() -> Option<PathBuf> {
    if let Ok(v) = std::env::var("MOTHER_CLAUDE_SIDECAR") {
        let v = v.trim().to_ascii_lowercase();
        if v == "0" || v == "false" || v == "off" || v == "no" {
            return None;
        }
    }
    if let Ok(custom) = std::env::var("MOTHER_CLAUDE_SIDECAR_PATH") {
        let p = PathBuf::from(custom);
        if p.is_file() {
            return Some(p);
        }
    }
    let candidates = [
        std::env::current_dir()
            .ok()
            .map(|d| d.join("sidecar/dist/agent-bridge.js")),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sidecar/dist/agent-bridge.js")),
    ];
    candidates.into_iter().flatten().find(|p| p.is_file())
}

/// Loopback URL the sidecar uses to reach this server.
fn server_url(state: &AppState) -> String {
    let scheme = if state.config.is_non_loopback() {
        "https"
    } else {
        "http"
    };
    format!("{scheme}://127.0.0.1:{}", state.config.port)
}

/// Shared stdio/env configuration for both spawn paths.
fn configure_common(cmd: &mut tokio::process::Command, cwd: &str, token: &str) {
    cmd.current_dir(cwd)
        // Hooks fired by this session authenticate back to us with the token.
        .env("MOTHER_CLAUDE_TOKEN", token)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
}

/// Path B: headless `claude -p` stream-json subprocess.
fn headless_command(
    id: &str,
    opts: &SpawnOptions,
    permission_mode: &str,
    token: &str,
) -> tokio::process::Command {
    let mut c = tokio::process::Command::new(crate::claude::claude_bin());
    c.arg("-p")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--input-format")
        .arg("stream-json")
        .arg("--verbose")
        .arg("--include-partial-messages")
        .arg("--permission-mode")
        .arg(permission_mode);
    match &opts.resume {
        // Continue the existing conversation in place (same id, same transcript).
        Some(resume) => {
            c.arg("--resume").arg(resume);
        }
        // Fresh session: pin our chosen id.
        None => {
            c.arg("--session-id").arg(id);
        }
    }
    if let Some(model) = &opts.model {
        c.arg("--model").arg(model);
    }
    if !opts.prompt.trim().is_empty() {
        c.arg(&opts.prompt);
    }
    configure_common(&mut c, &opts.cwd, token);
    c
}

/// Path A: the Node Agent SDK sidecar (canUseTool + ask_user).
fn sidecar_command(
    entry: &Path,
    id: &str,
    opts: &SpawnOptions,
    permission_mode: &str,
    token: &str,
    server_url: &str,
) -> tokio::process::Command {
    let mut c = tokio::process::Command::new("node");
    c.arg(entry)
        .env("MC_SESSION_ID", id)
        .env("MC_CWD", &opts.cwd)
        .env("MC_PROMPT", &opts.prompt)
        .env("MC_PERMISSION_MODE", permission_mode)
        .env("MOTHER_CLAUDE_URL", server_url)
        // Self-signed loopback TLS: trust it for the local sidecar.
        .env("NODE_TLS_REJECT_UNAUTHORIZED", "0");
    if let Some(model) = &opts.model {
        c.env("MC_MODEL", model);
    }
    configure_common(&mut c, &opts.cwd, token);
    c
}

/// Run a documented lifecycle subcommand (`stop` / `respawn` / `rm`) against any
/// session — owned or foreign. Returns combined stdout on success.
pub async fn run_lifecycle(action: &str, id: &str) -> Result<String> {
    let out = tokio::process::Command::new(crate::claude::claude_bin())
        .arg(action)
        .arg(id)
        .output()
        .await
        .with_context(|| format!("failed to run `claude {action} {id}`"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if out.status.success() {
        Ok(stdout)
    } else {
        Err(anyhow!("`claude {action} {id}` failed: {stderr}"))
    }
}

/// Serialize a user message line for `--input-format stream-json`.
pub fn user_message_json(text: &str) -> String {
    json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": [{ "type": "text", "text": text }]
        }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_message_is_stream_json_shape() {
        let line = user_message_json("hello world");
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["type"], "user");
        assert_eq!(v["message"]["role"], "user");
        assert_eq!(v["message"]["content"][0]["type"], "text");
        assert_eq!(v["message"]["content"][0]["text"], "hello world");
    }

    #[test]
    fn registry_starts_empty() {
        let r = ControlRegistry::new();
        assert!(!r.controls("anything"));
    }

    #[tokio::test]
    async fn run_lifecycle_invokes_cli() {
        // Stand in for `claude` with `echo` so we exercise the shell-out path
        // without touching real sessions.
        std::env::set_var("MOTHER_CLAUDE_CLI", "echo");
        let out = run_lifecycle("stop", "abc-123").await.unwrap();
        std::env::remove_var("MOTHER_CLAUDE_CLI");
        assert!(out.contains("stop"));
        assert!(out.contains("abc-123"));
    }
}
