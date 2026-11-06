//! What sessions did in the browser: a ring of recent actions for the
//! Browser page (per session, newest first) and one audit line per action in
//! `audit.jsonl`. Neither ever holds a typed value, a page's content or a
//! URL's query string: the session, the tool, the action, the origin (and
//! path), the outcome and how long it took.

use std::collections::VecDeque;
use std::sync::Mutex;

use serde::Serialize;
use serde_json::json;

use crate::server::devices::audit::AuditEntry;
use crate::state::AppState;

/// Entries kept in memory.
const RING: usize = 500;

/// One browser action, as the page lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub at: i64,
    pub session: String,
    /// `browser_click`, or a lifecycle step (`lease.open`, `stop`, …).
    pub tool: String,
    /// The protocol action (`click`), when there was one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// `scheme://host[:port]/path`, never the query or fragment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    /// `ok`, `denied`, `refused`, `stopped`, `timeout`, `error`, ….
    pub outcome: String,
    /// The error code, when it failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// It waited for an approval.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub approved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser: Option<String>,
}

/// The ring.
#[derive(Debug, Default)]
pub struct ActivityLog {
    ring: Mutex<VecDeque<Entry>>,
}

impl ActivityLog {
    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<Entry>> {
        self.ring.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn push(&self, entry: Entry) {
        let mut g = self.lock();
        g.push_back(entry);
        while g.len() > RING {
            g.pop_front();
        }
    }

    /// The newest `limit` entries (of one session, when given), newest first.
    pub fn recent(&self, session: Option<&str>, limit: usize) -> Vec<Entry> {
        self.lock()
            .iter()
            .rev()
            .filter(|e| session.is_none_or(|s| e.session == s))
            .take(limit)
            .cloned()
            .collect()
    }
}

/// Record an action: the ring, an audit line, and (debounced) a `browser`
/// frame so open Browser pages refresh their activity.
pub fn record(state: &AppState, entry: Entry) {
    state.devices.audit.record(AuditEntry {
        action: "browser.action".into(),
        principal: Some("mcp".into()),
        detail: Some(json!({
            "session": entry.session,
            "tool": entry.tool,
            "action": entry.action,
            "origin": entry.origin,
            "outcome": entry.outcome,
            "code": entry.code,
            "approved": entry.approved,
            "browser": entry.browser,
        })),
        ..AuditEntry::now()
    });
    state.extension.activity.push(entry);
    super::notify_soon(state, "activity");
}

/// An audit line for a change a person made (a site mode, a grant, a
/// pairing, a revocation, a Stop): who, from where, and what — no values.
pub fn audit_change(
    state: &AppState,
    ctx: Option<&crate::server::context::ClientContext>,
    action: &str,
    detail: serde_json::Value,
) {
    let base = match ctx {
        Some(ctx) => AuditEntry::from_ctx(ctx, action),
        None => AuditEntry {
            action: action.to_string(),
            ..AuditEntry::now()
        },
    };
    let device_name = ctx
        .and_then(|c| c.device_id())
        .and_then(|id| state.devices.get(id))
        .map(|d| d.name);
    state.devices.audit.record(AuditEntry {
        detail: Some(detail),
        device_name,
        ..base
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(session: &str, tool: &str) -> Entry {
        Entry {
            at: 1,
            session: session.into(),
            tool: tool.into(),
            action: None,
            origin: None,
            page: None,
            outcome: "ok".into(),
            code: None,
            approved: false,
            ms: None,
            browser: None,
        }
    }

    #[test]
    fn newest_first_per_session_and_bounded() {
        let log = ActivityLog::default();
        for i in 0..(RING + 5) {
            log.push(entry(if i % 2 == 0 { "a" } else { "b" }, &format!("t{i}")));
        }
        let all = log.recent(None, usize::MAX);
        assert_eq!(all.len(), RING);
        assert_eq!(all[0].tool, format!("t{}", RING + 4));
        let a = log.recent(Some("a"), 3);
        assert_eq!(a.len(), 3);
        assert!(a.iter().all(|e| e.session == "a"));
    }
}
