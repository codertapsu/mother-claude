//! Rate limits keyed by the tunnel-aware client address (design §2.9, §10 B1).
//!
//! * **Failed authentication**: 20 failures a minute per client, then a
//!   block with an exponential `Retry-After` (60 s, 120 s, … capped at an
//!   hour). Only attempts with a missing or invalid credential count; a
//!   request carrying a valid credential, or a machine-local one, is never
//!   rejected (enforced by the callers in `context` / `auth`).
//! * **Pairing**: 10 failed exchanges or pending polls a minute per client,
//!   with the same exponential block. There is deliberately no global bucket —
//!   a stranger who finds the public URL must not be able to block pairing for
//!   everyone.
//! * **Pre-auth WebSockets** ([`AuthLimiter::preauth`]): a browser cannot send
//!   a credential with the upgrade, so every socket is anonymous until its
//!   first frame. Each [`RateScope`] has its own pool (machine-local, direct
//!   LAN peers, forwarded/tunnel clients), so internet traffic can never use
//!   up the desktop's or the LAN's; at most 4 per client inside the LAN and
//!   tunnel pools. A full pool never refuses a newcomer: the OLDEST waiting
//!   socket of that client (or of the pool) is closed instead, so a stranger
//!   holding slots cannot lock paired devices out.
//!
//! Keys are [`RateKey`]s: an address plus the scope it was learned in, so an
//! address taken from a forwarding header can never share a bucket with a
//! real local peer, and forwarded IPv6 clients are counted per /64.
//!
//! AppState field: `state.limits`.

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv6Addr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::Notify;

/// Failed credentials allowed per window before a client is blocked.
pub const MAX_FAILURES_PER_MINUTE: usize = 20;
/// Failed pairing exchanges / polls allowed per window.
pub const MAX_PAIR_FAILURES_PER_MINUTE: usize = 10;
/// Not-yet-authenticated WebSockets per client (LAN and tunnel pools).
pub const MAX_PREAUTH_PER_IP: usize = 4;
/// Not-yet-authenticated WebSockets per pool (LAN, tunnel).
pub const MAX_PREAUTH_TOTAL: usize = 64;
/// Not-yet-authenticated WebSockets from this machine (one shared address).
pub const MAX_PREAUTH_LOCAL: usize = 32;

const WINDOW: Duration = Duration::from_secs(60);
/// The first block; each further block within [`STRIKE_MEMORY`] doubles it.
const BASE_BLOCK: Duration = Duration::from_secs(60);
const MAX_BLOCK: Duration = Duration::from_secs(3600);
/// A client that stays clean this long starts again from the base block.
const STRIKE_MEMORY: Duration = Duration::from_secs(3600);
/// Track at most this many distinct clients: idle ones are dropped first, then
/// the least recently seen ones that are not blocked. Active blocks are never
/// dropped to make room (a flood of fresh addresses must not lift them).
const MAX_TRACKED: usize = 10_000;
/// Hard ceiling: past it (every tracked client is blocked), new clients are
/// simply not tracked, so memory stays bounded.
const MAX_TRACKED_HARD: usize = 2 * MAX_TRACKED;

/// Where a client address was learned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RateScope {
    /// A machine-local caller (loopback listener, loopback peer, loopback
    /// `Host`, no proxy headers).
    Local,
    /// A direct non-loopback TCP peer (the LAN listeners).
    Direct,
    /// Taken from a forwarding header, or any other loopback peer that is not
    /// machine-local (a tunnel, a proxy, the tunnel listener).
    Forwarded,
}

/// What the limiters count against.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RateKey {
    pub scope: RateScope,
    /// The address; a forwarded IPv6 address is reduced to its /64.
    pub net: IpAddr,
}

impl RateKey {
    pub fn new(scope: RateScope, ip: IpAddr) -> Self {
        let ip = ip.to_canonical();
        let net = match (scope, ip) {
            // One internet host usually owns a whole /64.
            (RateScope::Forwarded, IpAddr::V6(v6)) => {
                let s = v6.segments();
                IpAddr::V6(Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
            }
            _ => ip,
        };
        Self { scope, net }
    }

    /// A direct peer (tests, callers without a context).
    pub fn direct(ip: IpAddr) -> Self {
        Self::new(RateScope::Direct, ip)
    }
}

#[derive(Debug)]
struct Entry {
    failures: VecDeque<Instant>,
    strikes: u32,
    blocked_until: Option<Instant>,
    last_block: Option<Instant>,
    last_seen: Instant,
}

impl Entry {
    fn new(now: Instant) -> Self {
        Self {
            failures: VecDeque::new(),
            strikes: 0,
            blocked_until: None,
            last_block: None,
            last_seen: now,
        }
    }
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

/// Per-client failure counters with exponential blocks.
#[derive(Debug)]
struct Buckets {
    max: usize,
    map: Mutex<HashMap<RateKey, Entry>>,
}

impl Buckets {
    fn new(max: usize) -> Self {
        Self {
            max,
            map: Mutex::new(HashMap::new()),
        }
    }

    fn blocked(&self, key: RateKey) -> Option<u64> {
        let now = Instant::now();
        let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
        let e = map.get_mut(&key)?;
        e.prune(now);
        let until = e.blocked_until?;
        let wait = until.saturating_duration_since(now);
        Some(wait.as_secs_f64().ceil().max(1.0) as u64)
    }

    /// Make room for one more client: drop idle entries, then the least
    /// recently seen entries that are not blocked. Never an active block.
    fn make_room(map: &mut HashMap<RateKey, Entry>, now: Instant) {
        map.retain(|_, e| {
            e.prune(now);
            !e.idle()
        });
        if map.len() < MAX_TRACKED {
            return;
        }
        let mut unblocked: Vec<(Instant, RateKey)> = map
            .iter()
            .filter(|(_, e)| e.blocked_until.is_none())
            .map(|(k, e)| (e.last_seen, *k))
            .collect();
        unblocked.sort_by_key(|(seen, _)| *seen);
        // Free a tenth at once so a flood does not sort on every request.
        let excess = map.len() + 1 - MAX_TRACKED + MAX_TRACKED / 10;
        for (_, k) in unblocked.into_iter().take(excess) {
            map.remove(&k);
        }
    }

    fn record(&self, key: RateKey) {
        let now = Instant::now();
        let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
        if map.len() >= MAX_TRACKED && !map.contains_key(&key) {
            Self::make_room(&mut map, now);
            if map.len() >= MAX_TRACKED_HARD {
                return;
            }
        }
        let e = map.entry(key).or_insert_with(|| Entry::new(now));
        e.prune(now);
        e.last_seen = now;
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

    fn clear(&self, key: RateKey) {
        let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
        map.remove(&key);
    }
}

#[derive(Debug)]
struct PreAuthEntry {
    id: u64,
    key: RateKey,
    evict: Arc<Notify>,
}

/// Waiting sockets, oldest first.
#[derive(Debug, Default)]
struct PreAuthCounts {
    next_id: u64,
    entries: VecDeque<PreAuthEntry>,
}

impl PreAuthCounts {
    /// Close the oldest waiting socket matching `pred`.
    fn evict_oldest(&mut self, pred: impl Fn(&PreAuthEntry) -> bool) {
        if let Some(pos) = self.entries.iter().position(pred) {
            if let Some(e) = self.entries.remove(pos) {
                e.evict.notify_one();
            }
        }
    }
}

fn preauth_caps(scope: RateScope) -> (usize, usize) {
    match scope {
        // Every local caller shares one address: the pool is the cap.
        RateScope::Local => (MAX_PREAUTH_LOCAL, MAX_PREAUTH_LOCAL),
        RateScope::Direct | RateScope::Forwarded => (MAX_PREAUTH_PER_IP, MAX_PREAUTH_TOTAL),
    }
}

/// A slot for one not-yet-authenticated WebSocket; released on drop (hold it
/// until the first-frame auth has completed or failed). When a newer socket
/// needs the slot, [`PreAuthSlot::evicted`] completes and the holder must
/// close.
#[derive(Debug)]
pub struct PreAuthSlot {
    counts: Arc<Mutex<PreAuthCounts>>,
    id: u64,
    evict: Arc<Notify>,
}

impl PreAuthSlot {
    /// Completes once this slot was taken over by a newer socket.
    pub async fn evicted(&self) {
        self.evict.notified().await;
    }
}

impl Drop for PreAuthSlot {
    fn drop(&mut self) {
        let mut c = self.counts.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(pos) = c.entries.iter().position(|e| e.id == self.id) {
            c.entries.remove(pos);
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

    /// `Some(seconds to wait)` while `key` is blocked for failed credentials.
    pub fn blocked(&self, key: RateKey) -> Option<u64> {
        self.auth.blocked(key)
    }

    /// Count one failed credential from `key`.
    pub fn record_failure(&self, key: RateKey) {
        self.auth.record(key);
    }

    /// `Some(seconds to wait)` while `key` is blocked from pairing.
    pub fn pair_blocked(&self, key: RateKey) -> Option<u64> {
        self.pair.blocked(key)
    }

    /// Count one failed pairing exchange or pending poll from `key`.
    pub fn record_pair_failure(&self, key: RateKey) {
        self.pair.record(key);
    }

    /// Forget `key`'s pairing failures (after a successful pairing).
    pub fn clear_pair_failures(&self, key: RateKey) {
        self.pair.clear(key);
    }

    /// Reserve a slot for an unauthenticated WebSocket from `key`. Never
    /// refuses: when `key` already holds its share, or its scope's pool is
    /// full, the oldest waiting socket of that client (else of that pool) is
    /// told to close ([`PreAuthSlot::evicted`]).
    pub fn preauth(&self, key: RateKey) -> PreAuthSlot {
        let (per_key, per_pool) = preauth_caps(key.scope);
        let mut c = self.preauth.lock().unwrap_or_else(|p| p.into_inner());
        if c.entries.iter().filter(|e| e.key == key).count() >= per_key {
            c.evict_oldest(|e| e.key == key);
        }
        if c.entries
            .iter()
            .filter(|e| e.key.scope == key.scope)
            .count()
            >= per_pool
        {
            c.evict_oldest(|e| e.key.scope == key.scope);
        }
        c.next_id += 1;
        let id = c.next_id;
        let evict = Arc::new(Notify::new());
        c.entries.push_back(PreAuthEntry {
            id,
            key,
            evict: evict.clone(),
        });
        PreAuthSlot {
            counts: self.preauth.clone(),
            id,
            evict,
        }
    }

    /// Unauthenticated WebSockets currently held (tests, diagnostics).
    pub fn preauth_count(&self) -> usize {
        self.preauth.lock().map(|c| c.entries.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(ip: &str) -> RateKey {
        RateKey::direct(ip.parse().unwrap())
    }

    #[test]
    fn blocks_after_budget() {
        let l = AuthLimiter::new();
        let ip = key("10.0.0.1");
        for _ in 0..MAX_FAILURES_PER_MINUTE - 1 {
            l.record_failure(ip);
        }
        assert!(l.blocked(ip).is_none());
        l.record_failure(ip);
        let wait = l.blocked(ip).unwrap();
        assert!((1..=60).contains(&wait));
        assert!(l.blocked(key("10.0.0.2")).is_none());
        // The same address learned from a forwarding header is another client.
        let forwarded = RateKey::new(RateScope::Forwarded, "10.0.0.1".parse().unwrap());
        assert!(l.blocked(forwarded).is_none());
    }

    #[test]
    fn forwarded_ipv6_clients_are_counted_per_64() {
        let a = RateKey::new(RateScope::Forwarded, "2001:db8:1:2::1".parse().unwrap());
        let b = RateKey::new(
            RateScope::Forwarded,
            "2001:db8:1:2:ffff::9".parse().unwrap(),
        );
        let c = RateKey::new(RateScope::Forwarded, "2001:db8:1:3::1".parse().unwrap());
        assert_eq!(a, b);
        assert_ne!(a, c);
        // LAN peers keep their own address (a home network shares one /64).
        let d = RateKey::direct("fd00::1".parse().unwrap());
        let e = RateKey::direct("fd00::2".parse().unwrap());
        assert_ne!(d, e);
        // IPv4-mapped addresses are canonicalised.
        assert_eq!(
            RateKey::direct("::ffff:10.0.0.1".parse().unwrap()),
            key("10.0.0.1")
        );
    }

    #[test]
    fn repeated_blocks_back_off_exponentially() {
        let b = Buckets::new(3);
        let ip = key("10.0.0.9");
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
    fn a_flood_of_new_addresses_never_lifts_an_active_block() {
        let b = Buckets::new(2);
        let victim = key("198.51.100.8");
        b.record(victim);
        b.record(victim);
        assert!(b.blocked(victim).is_some());
        // Fill the table with one failure each from many other addresses.
        for i in 0..(MAX_TRACKED as u32 + 500) {
            let ip = IpAddr::from((0x0a00_0000u32 + i).to_be_bytes());
            b.record(RateKey::direct(ip));
        }
        assert!(b.blocked(victim).is_some(), "the block survived");
        assert!(b.map.lock().unwrap().len() <= MAX_TRACKED);
    }

    #[test]
    fn pairing_has_its_own_smaller_budget() {
        let l = AuthLimiter::new();
        let ip = key("203.0.113.5");
        for _ in 0..MAX_PAIR_FAILURES_PER_MINUTE {
            l.record_pair_failure(ip);
        }
        assert!(l.pair_blocked(ip).is_some());
        assert!(l.blocked(ip).is_none(), "separate from failed auth");
        l.clear_pair_failures(ip);
        assert!(l.pair_blocked(ip).is_none());
    }

    fn is_evicted(slot: &PreAuthSlot) -> bool {
        use std::future::Future;
        let fut = slot.evicted();
        let mut fut = std::pin::pin!(fut);
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        fut.as_mut().poll(&mut cx).is_ready()
    }

    #[test]
    fn preauth_slots_evict_the_oldest_of_a_client_and_are_released_on_drop() {
        let l = AuthLimiter::new();
        let ip = key("192.168.1.9");
        let slots: Vec<_> = (0..MAX_PREAUTH_PER_IP).map(|_| l.preauth(ip)).collect();
        assert_eq!(l.preauth_count(), MAX_PREAUTH_PER_IP);
        assert!(slots.iter().all(|s| !is_evicted(s)));
        // A fifth socket from the same client closes that client's oldest.
        let fifth = l.preauth(ip);
        assert!(is_evicted(&slots[0]));
        assert!(slots[1..].iter().all(|s| !is_evicted(s)));
        assert!(!is_evicted(&fifth));
        assert_eq!(l.preauth_count(), MAX_PREAUTH_PER_IP);
        // Another client is unaffected.
        let other = l.preauth(key("192.168.1.10"));
        assert!(!is_evicted(&other));
        drop((slots, fifth, other));
        assert_eq!(l.preauth_count(), 0);
    }

    #[test]
    fn preauth_pools_are_separate_per_scope() {
        let l = AuthLimiter::new();
        // An internet flood fills the tunnel pool from many addresses…
        let flood: Vec<_> = (0..MAX_PREAUTH_TOTAL * 2)
            .map(|i| {
                let ip = IpAddr::from([203, 0, (i / 200) as u8, (i % 200) as u8]);
                l.preauth(RateKey::new(RateScope::Forwarded, ip))
            })
            .collect();
        assert_eq!(l.preauth_count(), MAX_PREAUTH_TOTAL);
        // …and neither the desktop nor a LAN phone is touched by it.
        let desktop = l.preauth(RateKey::new(RateScope::Local, "127.0.0.1".parse().unwrap()));
        let phone = l.preauth(key("192.168.1.20"));
        assert!(!is_evicted(&desktop) && !is_evicted(&phone));
        // The newest tunnel socket evicts the oldest tunnel socket only.
        let newest = l.preauth(RateKey::new(
            RateScope::Forwarded,
            "198.51.100.1".parse().unwrap(),
        ));
        assert!(!is_evicted(&newest));
        assert!(!is_evicted(&desktop) && !is_evicted(&phone));
        assert!(flood.iter().filter(|s| !is_evicted(s)).count() == MAX_PREAUTH_TOTAL - 1);
        // A forwarded 127.0.0.1 is not the desktop's bucket.
        let spoofed: Vec<_> = (0..MAX_PREAUTH_LOCAL)
            .map(|_| {
                l.preauth(RateKey::new(
                    RateScope::Forwarded,
                    "127.0.0.1".parse().unwrap(),
                ))
            })
            .collect();
        assert!(!is_evicted(&desktop));
        drop(spoofed);
    }
}
