//! Leases: which Claude Code session acts in which browser tabs.
//!
//! One lease per calling session, in one browser, opened on the session's
//! first browser tool call (`lease.open`: the extension makes a tab group
//! titled after the session). It ends when the session ends, after
//! [`IDLE_MS`] without a command, when its browser goes away, or when the
//! user presses Stop (`lease.revoked`): then the session's commands are
//! refused for [`STOP_MS`] unless the user resumes it, and every other
//! session acting in that browser within the same minute needs an approval.
//!
//! Also here: what the desktop knows about each lease tab
//! ([`super::page::TabInfo`]) and the per-minute action counters.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;

use serde::Serialize;

use super::page::{TabInfo, ACTIVE};

/// A lease with no command for this long is closed.
pub const IDLE_MS: i64 = 10 * 60 * 1000;
/// How long a Stop refuses the stopped session's commands.
pub const STOP_MS: i64 = 60 * 1000;
/// Window of the per-minute action counter.
const BURST_WINDOW_MS: i64 = 60 * 1000;

/// Chrome's tab group colours, in the order leases take them.
pub const COLORS: [&str; 9] = [
    "blue", "green", "purple", "orange", "cyan", "pink", "yellow", "red", "grey",
];

/// One session's right to act in a browser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lease {
    pub id: String,
    pub session: String,
    pub title: String,
    pub browser: String,
    pub color: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<i64>,
    pub opened_at: i64,
    pub last_used: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_tab: Option<i64>,
    /// The sweep saw its session live (only then does "not live any more"
    /// mean it ended).
    #[serde(skip)]
    pub seen_live: bool,
}

/// A session the user stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stop {
    pub session: String,
    pub browser: String,
    pub at: i64,
    pub until: i64,
    /// `user` (the side panel) or `desktop` (this app).
    pub by: String,
    /// The lease it closed, if the session had one (the side panel's
    /// `lease.resumed` names it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lease: Option<String>,
}

#[derive(Debug, Default)]
struct Inner {
    leases: HashMap<String, Lease>,
    tabs: HashMap<String, HashMap<i64, TabInfo>>,
    stops: HashMap<String, Stop>,
    /// Browser → when its last Stop happened.
    browser_stops: HashMap<String, i64>,
    /// Sessions the user resumed after a Stop (exempt from the after-Stop
    /// approvals of that minute).
    resumed: HashSet<String>,
    /// (session, origin) → when its counted actions happened.
    burst: HashMap<(String, String), VecDeque<i64>>,
    next_color: usize,
}

/// The lease table.
#[derive(Debug, Default)]
pub struct Leases {
    inner: Mutex<Inner>,
}

fn now_ms() -> i64 {
    crate::claude::registry::now_ms()
}

impl Leases {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn get(&self, session: &str) -> Option<Lease> {
        self.lock().leases.get(session).cloned()
    }

    /// The lease with this id.
    pub fn by_id(&self, lease: &str) -> Option<Lease> {
        self.lock().leases.values().find(|l| l.id == lease).cloned()
    }

    pub fn all(&self) -> Vec<Lease> {
        let mut out: Vec<Lease> = self.lock().leases.values().cloned().collect();
        out.sort_by_key(|l| std::cmp::Reverse(l.last_used));
        out
    }

    /// The leases in one browser.
    pub fn in_browser(&self, browser: &str) -> Vec<Lease> {
        let mut out: Vec<Lease> = self
            .lock()
            .leases
            .values()
            .filter(|l| l.browser == browser)
            .cloned()
            .collect();
        out.sort_by_key(|l| l.opened_at);
        out
    }

    /// A colour no other lease of `browser` shows, if there is one.
    pub fn pick_color(&self, browser: &str) -> &'static str {
        let mut g = self.lock();
        let used: HashSet<String> = g
            .leases
            .values()
            .filter(|l| l.browser == browser)
            .map(|l| l.color.clone())
            .collect();
        for i in 0..COLORS.len() {
            let c = COLORS[(g.next_color + i) % COLORS.len()];
            if !used.contains(c) {
                g.next_color = (g.next_color + i + 1) % COLORS.len();
                return c;
            }
        }
        let c = COLORS[g.next_color % COLORS.len()];
        g.next_color = (g.next_color + 1) % COLORS.len();
        c
    }

    pub fn insert(&self, lease: Lease) {
        let mut g = self.lock();
        g.tabs.remove(&lease.session);
        g.leases.insert(lease.session.clone(), lease);
    }

    /// Forget a session's lease (and its tabs). Returns it.
    pub fn remove(&self, session: &str) -> Option<Lease> {
        let mut g = self.lock();
        g.tabs.remove(session);
        g.leases.remove(session)
    }

    /// Forget every lease of a browser. Returns them.
    pub fn remove_browser(&self, browser: &str) -> Vec<Lease> {
        let mut g = self.lock();
        let sessions: Vec<String> = g
            .leases
            .values()
            .filter(|l| l.browser == browser)
            .map(|l| l.session.clone())
            .collect();
        sessions
            .iter()
            .filter_map(|s| {
                g.tabs.remove(s);
                g.leases.remove(s)
            })
            .collect()
    }

    /// A command ran: the lease is not idle.
    pub fn touch(&self, session: &str) {
        if let Some(l) = self.lock().leases.get_mut(session) {
            l.last_used = now_ms();
        }
    }

    /// The sweep saw the session live.
    pub fn mark_seen_live(&self, session: &str) {
        if let Some(l) = self.lock().leases.get_mut(session) {
            l.seen_live = true;
        }
    }

    /// Leases idle for [`IDLE_MS`].
    pub fn idle(&self, now: i64) -> Vec<Lease> {
        self.lock()
            .leases
            .values()
            .filter(|l| now - l.last_used >= IDLE_MS)
            .cloned()
            .collect()
    }

    // ── tabs ──

    /// The key of a tab: the one named, else the lease's active tab.
    pub fn tab_key(&self, session: &str, tab: Option<i64>) -> i64 {
        tab.or_else(|| self.lock().leases.get(session).and_then(|l| l.active_tab))
            .unwrap_or(ACTIVE)
    }

    /// A copy of what is known about a tab.
    pub fn tab(&self, session: &str, key: i64) -> Option<TabInfo> {
        self.lock()
            .tabs
            .get(session)
            .and_then(|t| t.get(&key))
            .cloned()
    }

    /// Change what is known about a tab.
    pub fn with_tab<R>(&self, session: &str, key: i64, f: impl FnOnce(&mut TabInfo) -> R) -> R {
        let mut g = self.lock();
        let tabs = g.tabs.entry(session.to_string()).or_default();
        f(tabs.entry(key).or_default())
    }

    /// The lease's active tab is `tab` now; what was learned while its id was
    /// unknown moves to it.
    pub fn set_active(&self, session: &str, tab: i64) {
        let mut g = self.lock();
        let Some(lease) = g.leases.get_mut(session) else {
            return;
        };
        lease.active_tab = Some(tab);
        let tabs = g.tabs.entry(session.to_string()).or_default();
        if let Some(unknown) = tabs.remove(&ACTIVE) {
            tabs.entry(tab).or_insert(unknown);
        }
    }

    /// A tab closed: forget it.
    pub fn close_tab(&self, session: &str, tab: i64) {
        let mut g = self.lock();
        if let Some(tabs) = g.tabs.get_mut(session) {
            tabs.remove(&tab);
        }
        if let Some(l) = g.leases.get_mut(session) {
            if l.active_tab == Some(tab) {
                l.active_tab = None;
            }
        }
    }

    /// The session whose lease has this tab (events name a tab, maybe a
    /// lease).
    pub fn session_of_tab(&self, browser: &str, lease: Option<&str>, tab: i64) -> Option<String> {
        let g = self.lock();
        if let Some(id) = lease {
            return g
                .leases
                .values()
                .find(|l| l.id == id && l.browser == browser)
                .map(|l| l.session.clone());
        }
        g.leases
            .values()
            .filter(|l| l.browser == browser)
            .find(|l| {
                l.active_tab == Some(tab)
                    || g.tabs.get(&l.session).is_some_and(|t| t.contains_key(&tab))
            })
            .map(|l| l.session.clone())
    }

    // ── Stop ──

    /// The user stopped this session in `browser` (closing `lease`, if any).
    pub fn stop(&self, session: &str, browser: &str, lease: Option<&str>, by: &str) -> Stop {
        let now = now_ms();
        let stop = Stop {
            session: session.to_string(),
            browser: browser.to_string(),
            at: now,
            until: now + STOP_MS,
            by: by.to_string(),
            lease: lease.map(str::to_string),
        };
        let mut g = self.lock();
        g.stops.insert(session.to_string(), stop.clone());
        g.browser_stops.insert(browser.to_string(), now);
        g.resumed.remove(session);
        stop
    }

    /// The session's Stop while it still refuses commands.
    pub fn stopped(&self, session: &str) -> Option<Stop> {
        let now = now_ms();
        self.lock()
            .stops
            .get(session)
            .filter(|s| now < s.until)
            .cloned()
    }

    /// Every Stop still in force.
    pub fn stops(&self) -> Vec<Stop> {
        let now = now_ms();
        let mut out: Vec<Stop> = self
            .lock()
            .stops
            .values()
            .filter(|s| now < s.until)
            .cloned()
            .collect();
        out.sort_by_key(|s| s.at);
        out
    }

    /// The user resumed a stopped session. `false` when it was not stopped.
    pub fn resume(&self, session: &str) -> bool {
        let mut g = self.lock();
        let was = g.stops.remove(session).is_some();
        if was {
            g.resumed.insert(session.to_string());
        }
        was
    }

    /// Resume the session whose live Stop in `browser` closed `lease` (the
    /// side panel's Resume): that session, or `None` when no such Stop holds.
    pub fn resume_lease(&self, browser: &str, lease: &str) -> Option<String> {
        let now = now_ms();
        let session = self
            .lock()
            .stops
            .values()
            .find(|s| s.browser == browser && s.lease.as_deref() == Some(lease) && now < s.until)
            .map(|s| s.session.clone())?;
        self.resume(&session).then_some(session)
    }

    /// Another session was stopped in `browser` less than a minute ago (and
    /// this one was not resumed since).
    pub fn after_stop(&self, browser: &str, session: &str) -> bool {
        let now = now_ms();
        let g = self.lock();
        g.browser_stops
            .get(browser)
            .is_some_and(|at| now - at < STOP_MS)
            && !g.resumed.contains(session)
            && !g
                .stops
                .get(session)
                .is_some_and(|s| s.browser == browser && now < s.until)
    }

    // ── per-minute counters ──

    /// This session's counted actions on `origin` in the last minute.
    pub fn recent_actions(&self, session: &str, origin: &str) -> usize {
        let now = now_ms();
        let mut g = self.lock();
        let key = (session.to_string(), origin.to_string());
        let Some(q) = g.burst.get_mut(&key) else {
            return 0;
        };
        while q.front().is_some_and(|t| now - t >= BURST_WINDOW_MS) {
            q.pop_front();
        }
        q.len()
    }

    /// Count one action.
    pub fn count_action(&self, session: &str, origin: &str) {
        let mut g = self.lock();
        let q = g
            .burst
            .entry((session.to_string(), origin.to_string()))
            .or_default();
        q.push_back(now_ms());
        while q.len() > 1000 {
            q.pop_front();
        }
    }

    /// An approved burst starts a fresh minute.
    pub fn reset_actions(&self, session: &str, origin: &str) {
        self.lock()
            .burst
            .remove(&(session.to_string(), origin.to_string()));
    }

    /// Drop expired Stops and empty counters.
    pub fn sweep(&self) {
        let now = now_ms();
        let mut g = self.lock();
        g.stops.retain(|_, s| now < s.until);
        g.browser_stops.retain(|_, at| now - *at < STOP_MS);
        // A resume matters only during the minute after a Stop.
        if g.browser_stops.is_empty() {
            g.resumed.clear();
        }
        g.burst
            .retain(|_, q| q.back().is_some_and(|t| now - t < BURST_WINDOW_MS));
    }

    /// Pretend a Stop happened `ago_ms` earlier (tests).
    #[doc(hidden)]
    pub fn age_stops_for_test(&self, ago_ms: i64) {
        let mut g = self.lock();
        for s in g.stops.values_mut() {
            s.at -= ago_ms;
            s.until -= ago_ms;
        }
        for at in g.browser_stops.values_mut() {
            *at -= ago_ms;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lease(session: &str, browser: &str, color: &str) -> Lease {
        Lease {
            id: format!("l_{session}"),
            session: session.into(),
            title: "t".into(),
            browser: browser.into(),
            color: color.into(),
            group_id: None,
            opened_at: now_ms(),
            last_used: now_ms(),
            active_tab: None,
            seen_live: false,
        }
    }

    #[test]
    fn leases_colors_and_tabs() {
        let l = Leases::default();
        let c1 = l.pick_color("b1");
        l.insert(lease("s1", "b1", c1));
        let c2 = l.pick_color("b1");
        assert_ne!(c1, c2, "a free colour");
        l.insert(lease("s2", "b1", c2));
        assert_eq!(l.in_browser("b1").len(), 2);
        assert_eq!(l.by_id("l_s2").unwrap().session, "s2");

        // Learned while the active tab was unknown, then moved to it.
        let key = l.tab_key("s1", None);
        assert_eq!(key, ACTIVE);
        l.with_tab("s1", key, |t| {
            t.navigated(Some("https://x.test/".into()), None)
        });
        l.set_active("s1", 42);
        assert_eq!(l.tab_key("s1", None), 42);
        assert_eq!(
            l.tab("s1", 42).unwrap().url.as_deref(),
            Some("https://x.test/")
        );
        assert_eq!(l.session_of_tab("b1", None, 42).as_deref(), Some("s1"));
        assert_eq!(
            l.session_of_tab("b1", Some("l_s2"), 7).as_deref(),
            Some("s2")
        );
        assert_eq!(l.session_of_tab("b2", Some("l_s2"), 7), None);
        l.close_tab("s1", 42);
        assert!(l.tab("s1", 42).is_none());
        assert_eq!(l.get("s1").unwrap().active_tab, None);

        assert_eq!(l.remove_browser("b1").len(), 2);
        assert!(l.all().is_empty());
    }

    #[test]
    fn idle_leases_are_found() {
        let l = Leases::default();
        let mut old = lease("s1", "b1", "blue");
        old.last_used -= IDLE_MS + 1;
        l.insert(old);
        l.insert(lease("s2", "b1", "red"));
        let idle = l.idle(now_ms());
        assert_eq!(idle.len(), 1);
        assert_eq!(idle[0].session, "s1");
        l.touch("s1");
        assert!(l.idle(now_ms()).is_empty());
    }

    #[test]
    fn stops_refuse_for_a_minute_and_flag_other_sessions() {
        let l = Leases::default();
        let stop = l.stop("s1", "b1", Some("l_1"), "user");
        assert_eq!(stop.until - stop.at, STOP_MS);
        assert!(l.stopped("s1").is_some());
        assert!(l.stopped("s2").is_none());
        // Another session in that browser needs approval; elsewhere it does not.
        assert!(l.after_stop("b1", "s2"));
        assert!(!l.after_stop("b2", "s2"));
        assert!(
            !l.after_stop("b1", "s1"),
            "the stopped one is refused instead"
        );
        // Resumed: it acts again, without the after-Stop approval.
        assert!(l.resume("s1"));
        assert!(!l.resume("s1"));
        assert!(l.stopped("s1").is_none());
        assert!(!l.after_stop("b1", "s1"));
        // A minute later nothing is left.
        l.stop("s3", "b1", None, "user");
        l.age_stops_for_test(STOP_MS + 1);
        assert!(l.stopped("s3").is_none());
        assert!(!l.after_stop("b1", "s2"));
        l.sweep();
        assert!(l.stops().is_empty());
    }

    #[test]
    fn actions_are_counted_per_session_and_site() {
        let l = Leases::default();
        for _ in 0..3 {
            l.count_action("s1", "https://x.test");
        }
        l.count_action("s2", "https://x.test");
        assert_eq!(l.recent_actions("s1", "https://x.test"), 3);
        assert_eq!(l.recent_actions("s1", "https://y.test"), 0);
        assert_eq!(l.recent_actions("s2", "https://x.test"), 1);
        l.reset_actions("s1", "https://x.test");
        assert_eq!(l.recent_actions("s1", "https://x.test"), 0);
    }
}
