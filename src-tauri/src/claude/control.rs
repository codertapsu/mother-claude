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
}

struct OwnedHandle {
    stdin: AsyncMutex<Option<ChildStdin>>,
    child: AsyncMutex<Child>,
    #[allow(dead_code)]
    cwd: String,
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

    /// Launch a new owned session and return its id.
    pub async fn spawn(&self, state: &AppState, opts: SpawnOptions) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let mut cmd = tokio::process::Command::new(crate::claude::claude_bin());
        cmd.arg("-p")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--input-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--include-partial-messages")
            .arg("--session-id")
            .arg(&id)
            .arg("--permission-mode")
            .arg(opts.permission_mode.as_deref().unwrap_or("default"));
        if let Some(model) = &opts.model {
            cmd.arg("--model").arg(model);
        }
        if !opts.prompt.trim().is_empty() {
            cmd.arg(&opts.prompt);
        }
        cmd.current_dir(&opts.cwd)
            // Hooks fired by this session authenticate back to us with the token.
            .env("MOTHER_CLAUDE_TOKEN", &state.auth.token)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn `{}`", crate::claude::claude_bin()))?;

        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let handle = Arc::new(OwnedHandle {
            stdin: AsyncMutex::new(stdin),
            child: AsyncMutex::new(child),
            cwd: opts.cwd.clone(),
        });

        if let Ok(mut map) = self.handles.lock() {
            map.insert(id.clone(), handle.clone());
        }
        state.owned.write().await.insert(id.clone());
        state.broadcast(ServerEvent::Notice(format!("Spawned owned session {id}")));

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
}
