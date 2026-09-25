//! Rate limits keyed by the tunnel-aware client ip (design §2.9, §10 B1).
//!
//! * **Failed authentication**: 20 failures a minute per client ip, then a
//!   block with an exponential `Retry-After` (60 s, 120 s, … capped at an
//!   hour). Only attempts with a missing or invalid credential count; a
//!   request carrying a valid credential, or a machine-local one, is never
//!   rejected (enforced by the callers in `context` / `auth`).
//! * **Pairing**: 10 failed exchanges or pending polls a minute per client ip,
//!   with the same exponential block. There is deliberately no global bucket —
//!   a stranger who finds the public URL must not be able to block pairing for
//!   everyone.
//! * **Pre-auth WebSockets**: at most 4 not-yet-authenticated sockets per
//!   client ip and 64 overall ([`AuthLimiter::try_preauth`]).
//!
//! AppState field: `state.limits`.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Failed credentials allowed per window before a client is blocked.
pub const MAX_FAILURES_PER_MINUTE: usize = 20;
/// Failed pairing exchanges / polls allowed per window.
pub const MAX_PAIR_FAILURES_PER_MINUTE: usize = 10;
/// Not-yet-authenticated WebSockets per client ip.
pub const MAX_PREAUTH_PER_IP: usize = 4;
/// Not-yet-authenticated WebSockets overall.
pub const MAX_PREAUTH_TOTAL: usize = 64;

const WINDOW: Duration = Duration::from_secs(60);
/// The first block; each further block within [`STRIKE_MEMORY`] doubles it.
const BASE_BLOCK: Duration = Duration::from_secs(60);
const MAX_BLOCK: Duration = Duration::from_secs(3600);
/// A client that stays clean this long starts again from the base block.
const STRIKE_MEMORY: Duration = Duration::from_secs(3600);
/// Stop tracking this many distinct ips (idle ones are evicted first) — a
/// flood of spoofed addresses must not grow memory without bound.
const MAX_TRACKED: usize = 10_000;

#[derive(Debug, Default)]
struct Entry {
    failures: VecDeque<Instant>,
    strikes: u32,
    blocked_until: Option<Instant>,
    last_block: Option<Instant>,
}

impl Entry {
    fn prune(&mut self, now: Instant) {
        while self
            .failures
            .front()
            .is_some_and(|t| now.duration_since(*t) >= WINDOW)
        {
            self.failures.pop_front();
        }
        if self.blocked_until.is_some_and(|u| u <= now) {
            self.blocked_until = None;
        }
        if self
            .last_block
            .is_some_and(|b| now.duration_since(b) >= STRIKE_MEMORY)
        {
            self.strikes = 0;
            self.last_block = None;
        }
    }

    fn idle(&self) -> bool {
        self.failures.is_empty() && self.blocked_until.is_none() && self.strikes == 0
    }
}

/// Per-ip failure counters with exponential blocks.
#[derive(Debug)]
struct Buckets {
    max: usize,
    map: Mutex<HashMap<IpAddr, Entry>>,
}

impl Buckets {
    fn new(max: usize) -> Self {
        Self {
            max,
            map: Mutex::new(HashMap::new()),
        }
    }

    fn blocked(&self, ip: IpAddr) -> Option<u64> {
        let now = Instant::now();
        let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
        let e = map.get_mut(&ip)?;
        e.prune(now);
        let until = e.blocked_until?;
        let wait = until.saturating_duration_since(now);
        Some(wait.as_secs_f64().ceil().max(1.0) as u64)
    }

    fn record(&self, ip: IpAddr) {
        let now = Instant::now();
        let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
        if map.len() >= MAX_TRACKED && !map.contains_key(&ip) {
            map.retain(|_, e| {
                e.prune(now);
                !e.idle()
            });
            if map.len() >= MAX_TRACKED {
                map.clear();
            }
        }
        let e = map.entry(ip).or_default();
        e.prune(now);
        e.failures.push_back(now);
        if e.failures.len() >= self.max {
            e.strikes = e.strikes.saturating_add(1);
            let factor = 1u32 << (e.strikes - 1).min(10);
            let block = (BASE_BLOCK * factor).min(MAX_BLOCK);
            e.blocked_until = Some(now + block);
            e.last_block = Some(now);
            e.failures.clear();
        }
    }

    fn clear(&self, ip: IpAddr) {
        let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
        map.remove(&ip);
    }
}

#[derive(Debug, Default)]
struct PreAuthCounts {
    per_ip: HashMap<IpAddr, usize>,
    total: usize,
}

/// A slot for one not-yet-authenticated WebSocket; released on drop (hold it
/// until the first-frame auth has completed or failed).
#[derive(Debug)]
pub struct PreAuthSlot {
    counts: Arc<Mutex<PreAuthCounts>>,
    ip: IpAddr,
}

impl Drop for PreAuthSlot {
    fn drop(&mut self) {
        let mut c = self.counts.lock().unwrap_or_else(|p| p.into_inner());
        c.total = c.total.saturating_sub(1);
        if let Some(n) = c.per_ip.get_mut(&self.ip) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                c.per_ip.remove(&self.ip);
            }
        }
    }
}

/// Failed-auth, pairing and pre-auth WebSocket limits.
#[derive(Debug)]
pub struct AuthLimiter {
    auth: Buckets,
    pair: Buckets,
    preauth: Arc<Mutex<PreAuthCounts>>,
}

impl Default for AuthLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl AuthLimiter {
    pub fn new() -> Self {
        Self {
            auth: Buckets::new(MAX_FAILURES_PER_MINUTE),
            pair: Buckets::new(MAX_PAIR_FAILURES_PER_MINUTE),
            preauth: Arc::new(Mutex::new(PreAuthCounts::default())),
        }
    }

    /// `Some(seconds to wait)` while `ip` is blocked for failed credentials.
    pub fn blocked(&self, ip: IpAddr) -> Option<u64> {
        self.auth.blocked(ip)
    }

    /// Count one failed credential from `ip`.
    pub fn record_failure(&self, ip: IpAddr) {
        self.auth.record(ip);
    }

    /// `Some(seconds to wait)` while `ip` is blocked from pairing.
    pub fn pair_blocked(&self, ip: IpAddr) -> Option<u64> {
        self.pair.blocked(ip)
    }

    /// Count one failed pairing exchange or pending poll from `ip`.
    pub fn record_pair_failure(&self, ip: IpAddr) {
        self.pair.record(ip);
    }

    /// Forget `ip`'s pairing failures (after a successful pairing).
    pub fn clear_pair_failures(&self, ip: IpAddr) {
        self.pair.clear(ip);
    }

    /// Reserve a slot for an unauthenticated WebSocket from `ip`, or `None`
    /// when `ip` already holds [`MAX_PREAUTH_PER_IP`] or the server
    /// [`MAX_PREAUTH_TOTAL`].
    pub fn try_preauth(&self, ip: IpAddr) -> Option<PreAuthSlot> {
        let mut c = self.preauth.lock().unwrap_or_else(|p| p.into_inner());
        let mine = c.per_ip.get(&ip).copied().unwrap_or(0);
        if mine >= MAX_PREAUTH_PER_IP || c.total >= MAX_PREAUTH_TOTAL {
            return None;
        }
        c.total += 1;
        c.per_ip.insert(ip, mine + 1);
        Some(PreAuthSlot {
            counts: self.preauth.clone(),
            ip,
        })
    }

    /// Unauthenticated WebSockets currently held (tests, diagnostics).
    pub fn preauth_count(&self) -> usize {
        self.preauth.lock().map(|c| c.total).unwrap_or(0)
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

    #[test]
    fn repeated_blocks_back_off_exponentially() {
        let b = Buckets::new(3);
        let ip: IpAddr = "10.0.0.9".parse().unwrap();
        let mut waits = Vec::new();
        for _ in 0..4 {
            for _ in 0..3 {
                b.record(ip);
            }
            waits.push(b.blocked(ip).unwrap());
            // Pretend the block expired so the next round can strike again.
            let mut map = b.map.lock().unwrap();
            map.get_mut(&ip).unwrap().blocked_until = Some(Instant::now());
        }
        assert!((59..=60).contains(&waits[0]), "{waits:?}");
        assert!((119..=120).contains(&waits[1]), "{waits:?}");
        assert!((239..=240).contains(&waits[2]), "{waits:?}");
        assert!((479..=480).contains(&waits[3]), "{waits:?}");
    }

    #[test]
    fn pairing_has_its_own_smaller_budget() {
        let l = AuthLimiter::new();
        let ip: IpAddr = "203.0.113.5".parse().unwrap();
        for _ in 0..MAX_PAIR_FAILURES_PER_MINUTE {
            l.record_pair_failure(ip);
        }
        assert!(l.pair_blocked(ip).is_some());
        assert!(l.blocked(ip).is_none(), "separate from failed auth");
        l.clear_pair_failures(ip);
        assert!(l.pair_blocked(ip).is_none());
    }

    #[test]
    fn preauth_slots_are_capped_per_ip_and_released_on_drop() {
        let l = AuthLimiter::new();
        let ip: IpAddr = "192.168.1.9".parse().unwrap();
        let slots: Vec<_> = (0..MAX_PREAUTH_PER_IP)
            .map(|_| l.try_preauth(ip).unwrap())
            .collect();
        assert!(l.try_preauth(ip).is_none());
        assert!(l.try_preauth("192.168.1.10".parse().unwrap()).is_some());
        assert_eq!(l.preauth_count(), MAX_PREAUTH_PER_IP);
        drop(slots);
        assert_eq!(l.preauth_count(), 0);
        assert!(l.try_preauth(ip).is_some());

        // The global cap.
        let mut held = Vec::new();
        for i in 0..MAX_PREAUTH_TOTAL {
            let ip = IpAddr::from([10, 1, (i / 200) as u8, (i % 200) as u8]);
            held.push(l.try_preauth(ip).unwrap());
        }
        assert!(l.try_preauth("10.9.9.9".parse().unwrap()).is_none());
    }
}
