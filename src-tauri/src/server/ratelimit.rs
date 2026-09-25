//! Failed-authentication rate limiting (design §2.9: 20 failures/min per
//! client ip → 429 with Retry-After).
//!
//! Owner: **B1 (security/remote)**. The foundation ships a working sliding
//! window used by `auth::authenticate` and WebSocket first-frame auth. B1 may
//! extend it (pairing-exchange limits: 10 failures/min per ip, 30 per 10 min
//! globally — add new methods rather than changing these two).
//! AppState field: `state.limits`.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Failures allowed per window before a client is blocked.
pub const MAX_FAILURES_PER_MINUTE: usize = 20;
const WINDOW: Duration = Duration::from_secs(60);
/// Stop tracking this many distinct ips (oldest are evicted) — a flood of
/// spoofed addresses must not grow memory without bound.
const MAX_TRACKED: usize = 10_000;

/// Sliding-window counter of failed credentials per client ip.
#[derive(Debug, Default)]
pub struct AuthLimiter {
    failures: Mutex<HashMap<IpAddr, VecDeque<Instant>>>,
}

impl AuthLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    fn prune(q: &mut VecDeque<Instant>, now: Instant) {
        while q.front().is_some_and(|t| now.duration_since(*t) >= WINDOW) {
            q.pop_front();
        }
    }

    /// `Some(seconds to wait)` while `ip` is over budget.
    pub fn blocked(&self, ip: IpAddr) -> Option<u64> {
        let now = Instant::now();
        let mut map = self.failures.lock().unwrap_or_else(|p| p.into_inner());
        let q = map.get_mut(&ip)?;
        Self::prune(q, now);
        if q.len() < MAX_FAILURES_PER_MINUTE {
            return None;
        }
        let oldest = *q.front()?;
        let wait = WINDOW.saturating_sub(now.duration_since(oldest));
        Some(wait.as_secs().max(1))
    }

    /// Count one failed credential from `ip`.
    pub fn record_failure(&self, ip: IpAddr) {
        let now = Instant::now();
        let mut map = self.failures.lock().unwrap_or_else(|p| p.into_inner());
        if map.len() >= MAX_TRACKED && !map.contains_key(&ip) {
            map.retain(|_, q| {
                Self::prune(q, now);
                !q.is_empty()
            });
            if map.len() >= MAX_TRACKED {
                map.clear();
            }
        }
        let q = map.entry(ip).or_default();
        Self::prune(q, now);
        q.push_back(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_after_budget() {
        let l = AuthLimiter::new();
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        for _ in 0..MAX_FAILURES_PER_MINUTE - 1 {
            l.record_failure(ip);
        }
        assert!(l.blocked(ip).is_none());
        l.record_failure(ip);
        let wait = l.blocked(ip).unwrap();
        assert!((1..=60).contains(&wait));
        assert!(l.blocked("10.0.0.2".parse().unwrap()).is_none());
    }
}
