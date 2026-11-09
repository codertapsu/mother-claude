//! The Mother Claude browser extension's desktop side (docs/BROWSER.md, the
//! wire contract in docs/BROWSER-PROTOCOL.md, plan §5 and §10).
//!
//! * [`channel`] — `/ws/browser`, the extension's WebSocket: loopback
//!   listener only, machine-local requests only (no proxy or tunnel header,
//!   no raw TCP tunnel pointing at the port), `Origin:
//!   chrome-extension://<id>` only; `pair` → `paired`, `hello` → `welcome`;
//!   pings; 60 s of silence drops it; 8 MiB frames.
//! * [`store`] / [`pairing`] — paired browsers (`brw_…`, an `mcx_` token
//!   kept as its SHA-256, the pinned extension origin) and the 8-character
//!   pairing codes the desktop creates.
//! * [`hub`] — one connection per paired browser; requests to the extension
//!   with deadlines.
//! * [`leases`] / [`page`] — one lease (a tab group) per calling session;
//!   Stop; what the desktop knows about each tab.
//! * [`sites`] — site modes (`off`/`read`/`act`), the sensitive list,
//!   settings, standing grants.
//! * [`danger`] — which actions need a person's approval; [`actions`] raises
//!   them through `crate::pending` (the desktop, a phone holding the
//!   dangerous grant, or the side panel answers; the first answer wins).
//! * [`redact`] — what the extension reads is redacted again before it
//!   reaches a model.
//! * [`mcp`] — the `browser_*` tools; [`routes`] — `/api/browser/*`;
//!   [`activity`] — the activity log and audit lines.
//!
//! AppState field: `state.extension` ([`ExtensionState::load`] from the app
//! data dir; `None` keeps everything in memory, for tests). Nothing here
//! ever writes under a config directory.

pub mod actions;
pub mod activity;
pub mod channel;
pub mod connect;
pub mod danger;
pub mod hub;
pub mod leases;
pub mod mcp;
pub mod page;
pub mod pairing;
pub mod redact;
pub mod routes;
pub mod sites;
pub mod store;
pub mod upload;

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use crate::state::AppState;

/// The protocol version this side speaks (`welcome.protocol`).
pub const PROTOCOL: u32 = 1;

/// A browser approval waiting for an answer (for the side panel's `state`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    /// The pending item's id.
    pub id: String,
    pub browser: String,
    pub lease: String,
    pub session: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub created_at: i64,
}

/// Everything the browser feature keeps.
pub struct ExtensionState {
    pub browsers: store::BrowserStore,
    pub settings: sites::SettingsStore,
    pub codes: pairing::PairingCodes,
    pub hub: hub::Hub,
    pub leases: leases::Leases,
    pub activity: activity::ActivityLog,
    approvals: Mutex<HashMap<String, Approval>>,
    lease_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    notify_scheduled: AtomicBool,
}

impl std::fmt::Debug for ExtensionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtensionState")
            .field("browsers", &self.browsers)
            .field("hub", &self.hub)
            .finish_non_exhaustive()
    }
}

impl ExtensionState {
    /// Load the paired browsers and the settings from the app data dir.
    pub fn load(data_dir: Option<&Path>) -> Self {
        Self {
            browsers: store::BrowserStore::load(data_dir),
            settings: sites::SettingsStore::load(data_dir),
            codes: pairing::PairingCodes::default(),
            hub: hub::Hub::default(),
            leases: leases::Leases::default(),
            activity: activity::ActivityLog::default(),
            approvals: Mutex::new(HashMap::new()),
            lease_locks: Mutex::new(HashMap::new()),
            notify_scheduled: AtomicBool::new(false),
        }
    }

    fn approvals(&self) -> std::sync::MutexGuard<'_, HashMap<String, Approval>> {
        self.approvals.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// An approval is waiting.
    pub fn track_approval(&self, a: Approval) {
        self.approvals().insert(a.id.clone(), a);
    }

    /// It was answered or withdrawn.
    pub fn untrack_approval(&self, id: &str) -> Option<Approval> {
        self.approvals().remove(id)
    }

    pub fn approval(&self, id: &str) -> Option<Approval> {
        self.approvals().get(id).cloned()
    }

    /// Approvals waiting in one browser (or all), oldest first.
    pub fn approvals_for(&self, browser: Option<&str>) -> Vec<Approval> {
        let mut out: Vec<Approval> = self
            .approvals()
            .values()
            .filter(|a| browser.is_none_or(|b| a.browser == b))
            .cloned()
            .collect();
        out.sort_by_key(|a| a.created_at);
        out
    }

    /// The lock that serialises opening a session's lease.
    pub fn lease_lock(&self, session: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut g = self.lease_locks.lock().unwrap_or_else(|p| p.into_inner());
        g.retain(|_, l| Arc::strong_count(l) > 1);
        g.entry(session.to_string()).or_default().clone()
    }
}

/// The `settings` object the extension gets (`welcome`, `state`).
pub fn settings_frame(s: &sites::Settings) -> Value {
    json!({
        "screenshots": s.screenshots,
        "defaultMode": s.default_mode.as_str(),
        "sensitiveDomains": sites::sensitive_domains(),
    })
}

/// The `state` frame for one browser.
pub fn state_frame(state: &AppState, browser: &str) -> Value {
    let ext = &state.extension;
    let settings = ext.settings.get();
    let leases: Vec<Value> = ext
        .leases
        .in_browser(browser)
        .into_iter()
        .map(|l| {
            json!({
                "lease": l.id,
                "session": { "id": l.session, "title": l.title },
                "color": l.color,
                "groupId": l.group_id,
            })
        })
        .collect();
    let pending: Vec<Value> = ext
        .approvals_for(Some(browser))
        .into_iter()
        .map(|a| {
            json!({
                "id": a.id,
                "lease": a.lease,
                "session": a.session,
                "summary": a.summary,
                "detail": a.detail,
                "dangerous": true,
                "createdAt": a.created_at,
            })
        })
        .collect();
    json!({
        "type": "state",
        "siteModes": settings.site_modes_frame(),
        "settings": settings_frame(&settings),
        "leases": leases,
        "pending": pending,
    })
}

/// Send a fresh `state` to one browser.
pub fn push_state(state: &AppState, browser: &str) {
    if let Some(conn) = state.extension.hub.get(browser) {
        if !conn.push(&state_frame(state, browser)) {
            tracing::debug!(browser, "browser queue full; state frame dropped");
        }
    }
}

/// Send a fresh `state` to every connected browser.
pub fn broadcast_state(state: &AppState) {
    for conn in state.extension.hub.connected() {
        let _ = conn.push(&state_frame(state, &conn.browser));
    }
}

/// Tell dashboards (desktop and phones) that `what` changed: they refetch
/// `/api/browser`.
pub fn notify(state: &AppState, what: &str) {
    crate::server::ws::publish_browser(state, what);
}

/// [`notify`] at most every 750 ms (activity changes with every action).
pub fn notify_soon(state: &AppState, what: &'static str) {
    if state
        .extension
        .notify_scheduled
        .swap(true, Ordering::AcqRel)
    {
        return;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        state
            .extension
            .notify_scheduled
            .store(false, Ordering::Release);
        notify(state, what);
        return;
    };
    let st = state.clone();
    handle.spawn(async move {
        tokio::time::sleep(Duration::from_millis(750)).await;
        st.extension
            .notify_scheduled
            .store(false, Ordering::Release);
        notify(&st, what);
    });
}

/// Start the background sweep (expired Stops, old revoked browsers).
pub fn start(state: &AppState) {
    let st = state.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(15));
        loop {
            tick.tick().await;
            sweep(&st);
        }
    });
}

/// One sweep: forget expired Stops and old revoked browsers. It closes no
/// lease: a session stays connected to its browser, idle, busy elsewhere or
/// ended (a session continued later keeps its tabs), until the user
/// disconnects it (Stop, closing its group, removing the browser), closes
/// the browser (`freshBrowser` on its next `hello`) or quits the app.
pub fn sweep(state: &AppState) {
    let ext = &state.extension;
    ext.leases.sweep();
    ext.browsers.sweep();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> AppState {
        crate::state::Inner::new(
            crate::claude::ClaudeHome::with_base(std::env::temp_dir()),
            crate::state::ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            crate::server::auth::Auth::ephemeral(),
        )
    }

    #[test]
    fn the_state_frame_carries_modes_settings_leases_and_approvals() {
        let st = state();
        st.extension
            .settings
            .update(|s| {
                s.sites
                    .insert("https://github.com".into(), sites::SiteMode::Act);
                Ok(())
            })
            .unwrap();
        st.extension.leases.insert(leases::Lease {
            id: "l_1".into(),
            session: "s1".into(),
            title: "Frontend · app".into(),
            browser: "brw_1".into(),
            color: "blue".into(),
            group_id: Some(7),
            opened_at: 1,
            last_used: 1,
            active_tab: None,
        });
        st.extension.track_approval(Approval {
            id: "p1".into(),
            browser: "brw_1".into(),
            lease: "l_1".into(),
            session: "s1".into(),
            summary: "Submit the form on github.com".into(),
            detail: None,
            created_at: 5,
        });
        st.extension.track_approval(Approval {
            id: "p2".into(),
            browser: "brw_2".into(),
            lease: "l_2".into(),
            session: "s2".into(),
            summary: "x".into(),
            detail: None,
            created_at: 6,
        });
        let f = state_frame(&st, "brw_1");
        assert_eq!(f["type"], "state");
        assert_eq!(f["siteModes"]["*"], "off");
        assert_eq!(f["siteModes"]["https://github.com"], "act");
        assert_eq!(f["settings"]["screenshots"], true);
        assert_eq!(f["settings"]["defaultMode"], "off");
        assert!(f["settings"]["sensitiveDomains"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d == "paypal.com"));
        assert_eq!(f["leases"][0]["lease"], "l_1");
        assert_eq!(f["leases"][0]["session"]["title"], "Frontend · app");
        assert_eq!(f["leases"][0]["groupId"], 7);
        assert_eq!(
            f["pending"].as_array().unwrap().len(),
            1,
            "its own browser's"
        );
        assert_eq!(f["pending"][0]["dangerous"], true);
        assert!(st.extension.untrack_approval("p1").is_some());
        assert!(st.extension.approval("p1").is_none());
    }

    /// A lease lasts however long its session goes without a browser
    /// command: idle, busy with something else or ended, it stays connected
    /// until the user disconnects it, closes the browser or quits the app
    /// (the sweep closed one after ten idle minutes, or once its session
    /// ended).
    #[tokio::test]
    async fn the_sweep_keeps_a_lease_however_long_it_goes_unused() {
        let st = state();
        let now = crate::claude::registry::now_ms();
        let day = 24 * 60 * 60 * 1000;
        for (session, last_used) in [("quiet", now - day), ("busy", now)] {
            st.extension.leases.insert(leases::Lease {
                id: format!("l_{session}"),
                session: session.into(),
                title: "t".into(),
                browser: "brw_1".into(),
                color: "blue".into(),
                group_id: None,
                opened_at: now - day,
                last_used,
                active_tab: None,
            });
        }
        sweep(&st);
        assert!(st.extension.leases.get("quiet").is_some());
        assert!(st.extension.leases.get("busy").is_some());
        assert!(st.extension.activity.recent(Some("quiet"), 5).is_empty());
    }
}
