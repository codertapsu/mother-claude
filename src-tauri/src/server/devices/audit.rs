//! Append-only audit log of remote activity (`<app data>/audit.jsonl`).
//!
//! One JSON object per line, file mode 0600, rotated to `audit.jsonl.1` at
//! [`MAX_BYTES`]. Never under `~/.claude`. Every entry records who (device id
//! and name, principal), from where (ingress, effective ingress, client ip)
//! and what (action, method + path without the query string, status). It never
//! records a credential, a pairing code, a poll secret, request bodies or tool
//! input.
//!
//! Without a data dir (tests) entries are kept only in the in-memory ring.

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;

/// Rotate the file past this size.
pub const MAX_BYTES: u64 = 2 * 1024 * 1024;
/// Entries kept in memory for `GET /api/devices` / tests.
const RING: usize = 500;

/// One audited event.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    pub at: i64,
    /// e.g. `http`, `pair.exchange`, `pair.approve`, `device.revoked`.
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingress: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_ingress: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Path only — never the query string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// What a dangerous gate allowed, when one did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dangerous: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

impl AuditEntry {
    /// An empty entry stamped with the current time.
    pub fn now() -> Self {
        Self {
            at: chrono::Utc::now().timestamp_millis(),
            ..Default::default()
        }
    }

    /// Fill who/where from a request context.
    pub fn from_ctx(ctx: &crate::server::context::ClientContext, action: &str) -> Self {
        Self {
            action: action.to_string(),
            principal: Some(ctx.principal.label().to_string()),
            device_id: ctx.device_id().map(str::to_string),
            ingress: Some(ctx.ingress.as_str().to_string()),
            effective_ingress: Some(ctx.effective_ingress.as_str().to_string()),
            client_ip: Some(ctx.client_ip.to_string()),
            ..Self::now()
        }
    }
}

/// The audit log.
#[derive(Debug)]
pub struct AuditLog {
    path: Option<PathBuf>,
    write: Mutex<()>,
    ring: Mutex<VecDeque<AuditEntry>>,
}

impl AuditLog {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            write: Mutex::new(()),
            ring: Mutex::new(VecDeque::new()),
        }
    }

    /// Append one entry (errors are logged, never raised).
    pub fn record(&self, entry: AuditEntry) {
        if let Some(path) = &self.path {
            let _guard = self.write.lock().unwrap_or_else(|p| p.into_inner());
            if let Err(e) = append(path, &entry) {
                tracing::warn!(error = %e, "could not write the audit log");
            }
        }
        let mut ring = self.ring.lock().unwrap_or_else(|p| p.into_inner());
        ring.push_back(entry);
        while ring.len() > RING {
            ring.pop_front();
        }
    }

    /// The most recent entries, newest last.
    pub fn recent(&self, limit: usize) -> Vec<AuditEntry> {
        let ring = self.ring.lock().unwrap_or_else(|p| p.into_inner());
        let skip = ring.len().saturating_sub(limit);
        ring.iter().skip(skip).cloned().collect()
    }
}

fn append(path: &std::path::Path, entry: &AuditEntry) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let rotated = path.with_extension("jsonl.1");
        std::fs::rename(path, &rotated)?;
        crate::persist::restrict(&rotated);
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    let mut line = serde_json::to_vec(entry)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    line.push(b'\n');
    f.write_all(&line)?;
    crate::persist::restrict(path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_private_lines_and_rotates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        let log = AuditLog::new(Some(path.clone()));
        log.record(AuditEntry {
            action: "pair.exchange".into(),
            client_ip: Some("203.0.113.9".into()),
            ..AuditEntry::now()
        });
        log.record(AuditEntry {
            action: "http".into(),
            path: Some("/api/sessions/x/rm".into()),
            status: Some(200),
            ..AuditEntry::now()
        });
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2);
        let first: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(first["action"], "pair.exchange");
        assert_eq!(first["clientIp"], "203.0.113.9");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        assert_eq!(log.recent(1)[0].action, "http");

        // Rotation past the cap.
        std::fs::write(&path, vec![b'x'; (MAX_BYTES + 1) as usize]).unwrap();
        log.record(AuditEntry {
            action: "after".into(),
            ..AuditEntry::now()
        });
        assert!(dir.path().join("audit.jsonl.1").is_file());
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 1);
    }

    #[test]
    fn in_memory_without_a_path() {
        let log = AuditLog::new(None);
        for i in 0..(RING + 10) {
            log.record(AuditEntry {
                action: format!("a{i}"),
                ..AuditEntry::now()
            });
        }
        assert_eq!(log.recent(usize::MAX).len(), RING);
        assert_eq!(log.recent(1)[0].action, format!("a{}", RING + 9));
    }
}
