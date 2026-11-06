//! Pairing codes for browsers.
//!
//! The desktop creates a code (`POST /api/browser/pair`); the user types it
//! into the extension, which sends it in a `pair` frame. A code is 8
//! characters of Crockford base32 (40 bits of OS randomness, shown
//! `ABCD-EFGH`), lives 5 minutes and works once; at most 3 are alive (a 4th
//! evicts the oldest). Every extension connects from this machine, so failed
//! attempts share one budget: 5 a minute, then pairing is refused for a
//! minute (doubling on each further block, up to 15 minutes). A code, used
//! or not, is never logged.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::server::auth;

/// Characters in a code.
pub const CODE_LEN: usize = 8;
/// A code's lifetime.
pub const CODE_TTL_MS: i64 = 5 * 60 * 1000;
/// Codes alive at once.
pub const MAX_CODES: usize = 3;
/// Failed attempts a minute before pairing is blocked.
pub const MAX_FAILURES_PER_MINUTE: usize = 5;
/// How long an expired or used code is still recognised (to say
/// `expired_code` rather than `bad_code`).
const REMEMBER_MS: i64 = 10 * 60 * 1000;
const WINDOW: Duration = Duration::from_secs(60);
const BASE_BLOCK: Duration = Duration::from_secs(60);
const MAX_BLOCK: Duration = Duration::from_secs(15 * 60);

/// Crockford base32 (no I, L, O, U).
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A new code, as the desktop shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewCode {
    pub code: String,
    /// `ABCD-EFGH`.
    pub display: String,
    pub expires_at: i64,
    pub ttl_sec: i64,
}

/// Why a code was refused (the `error` frame's `code`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairError {
    /// Not a code we issued.
    BadCode,
    /// It was, but expired or already used.
    ExpiredCode,
    /// Too many failures: retry after this many seconds.
    RateLimited(u64),
}

impl PairError {
    pub fn code(self) -> &'static str {
        match self {
            PairError::BadCode => "bad_code",
            PairError::ExpiredCode => "expired_code",
            PairError::RateLimited(_) => "rate_limited",
        }
    }

    pub fn message(self) -> String {
        match self {
            PairError::BadCode => "That pairing code is not right. Check the code Mother Claude \
                                  shows (Browser › Pair a browser) and try again."
                .into(),
            PairError::ExpiredCode => "That pairing code expired or was already used. Create a \
                                       new one in Mother Claude (Browser › Pair a browser)."
                .into(),
            PairError::RateLimited(secs) => {
                format!("Too many wrong pairing codes. Try again in {secs} seconds.")
            }
        }
    }
}

#[derive(Debug)]
struct Code {
    code: String,
    expires_at: i64,
    used: bool,
}

#[derive(Debug, Default)]
struct Inner {
    codes: Vec<Code>,
    failures: VecDeque<Instant>,
    blocked_until: Option<Instant>,
    /// Blocks in a row (each doubles the next).
    strikes: u32,
}

/// Live codes and the failure budget (memory only: a restart forgets them).
#[derive(Debug, Default)]
pub struct PairingCodes {
    inner: Mutex<Inner>,
}

fn now_ms() -> i64 {
    crate::claude::registry::now_ms()
}

/// `n` random Crockford characters.
fn random_code() -> String {
    auth::random_bytes(CODE_LEN)
        .into_iter()
        .map(|b| CROCKFORD[(b & 31) as usize] as char)
        .collect()
}

/// What a person typed: case-insensitive, spaces and dashes ignored, O→0,
/// I/L→1. `None` when it cannot be a code.
pub fn normalize(raw: &str) -> Option<String> {
    let mut out = String::with_capacity(CODE_LEN);
    for c in raw.chars().take(64) {
        let c = match c.to_ascii_uppercase() {
            ' ' | '-' | '_' | '\t' => continue,
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        };
        if !c.is_ascii() || !CROCKFORD.contains(&(c as u8)) {
            return None;
        }
        out.push(c);
    }
    (out.len() == CODE_LEN).then_some(out)
}

/// `ABCD-EFGH`.
pub fn display(code: &str) -> String {
    let (a, b) = code.split_at(code.len().min(4));
    format!("{a}-{b}")
}

impl PairingCodes {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// A new code (the oldest unused one goes beyond [`MAX_CODES`]).
    pub fn create(&self) -> NewCode {
        let now = now_ms();
        let code = random_code();
        let mut g = self.lock();
        g.codes.retain(|c| now < c.expires_at + REMEMBER_MS);
        loop {
            let live: Vec<usize> = g
                .codes
                .iter()
                .enumerate()
                .filter(|(_, c)| !c.used && now < c.expires_at)
                .map(|(i, _)| i)
                .collect();
            if live.len() < MAX_CODES {
                break;
            }
            g.codes[live[0]].used = true;
        }
        g.codes.push(Code {
            code: code.clone(),
            expires_at: now + CODE_TTL_MS,
            used: false,
        });
        NewCode {
            display: display(&code),
            code,
            expires_at: now + CODE_TTL_MS,
            ttl_sec: CODE_TTL_MS / 1000,
        }
    }

    /// Redeem a code: `Ok` exactly once per live code.
    pub fn redeem(&self, raw: &str) -> Result<(), PairError> {
        let mut g = self.lock();
        let now = Instant::now();
        if let Some(until) = g.blocked_until {
            if now < until {
                return Err(PairError::RateLimited(
                    until.duration_since(now).as_secs().max(1),
                ));
            }
            g.blocked_until = None;
        }
        let at = now_ms();
        let found = normalize(raw).and_then(|code| {
            g.codes
                .iter()
                .position(|c| c.code.len() == code.len() && auth::token_matches(&c.code, &code))
        });
        let outcome = match found {
            Some(i) if !g.codes[i].used && at < g.codes[i].expires_at => {
                g.codes[i].used = true;
                Ok(())
            }
            Some(_) => Err(PairError::ExpiredCode),
            None => Err(PairError::BadCode),
        };
        if outcome.is_err() {
            g.failures.push_back(now);
            while g
                .failures
                .front()
                .is_some_and(|t| now.duration_since(*t) > WINDOW)
            {
                g.failures.pop_front();
            }
            if g.failures.len() >= MAX_FAILURES_PER_MINUTE {
                let block = BASE_BLOCK
                    .saturating_mul(1u32 << g.strikes.min(4))
                    .min(MAX_BLOCK);
                g.strikes = g.strikes.saturating_add(1);
                g.blocked_until = Some(now + block);
                g.failures.clear();
            }
        } else {
            g.failures.clear();
            g.strikes = 0;
        }
        outcome
    }

    /// Codes still usable (for the page).
    pub fn live_count(&self) -> usize {
        let now = now_ms();
        self.lock()
            .codes
            .iter()
            .filter(|c| !c.used && now < c.expires_at)
            .count()
    }

    /// Expire a code early (tests).
    #[doc(hidden)]
    pub fn expire_all_for_test(&self) {
        for c in &mut self.lock().codes {
            c.expires_at = now_ms() - 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_short_typed_loosely_and_single_use() {
        let codes = PairingCodes::default();
        let c = codes.create();
        assert_eq!(c.code.len(), CODE_LEN);
        assert!(c.code.bytes().all(|b| CROCKFORD.contains(&b)));
        assert_eq!(c.display.len(), CODE_LEN + 1);
        assert_eq!(&c.display[4..5], "-");
        assert_eq!(c.ttl_sec, 300);
        // Typed in lower case with the dash: fine, once.
        assert_eq!(codes.redeem(&c.display.to_lowercase()), Ok(()));
        assert_eq!(codes.redeem(&c.code), Err(PairError::ExpiredCode));
        assert_eq!(codes.redeem("ZZZZ-ZZZZ"), Err(PairError::BadCode));
        assert_eq!(codes.redeem("nope"), Err(PairError::BadCode));
        assert_eq!(normalize("abcd efgh").as_deref(), Some("ABCDEFGH"));
        assert_eq!(normalize("oiLo-0000").as_deref(), Some("01100000"));
        assert_eq!(normalize("ABCD-EFG"), None);
        assert_eq!(normalize("ABCD-EFGU"), None, "U is not Crockford");
    }

    #[test]
    fn expired_codes_say_so_and_old_codes_are_evicted() {
        let codes = PairingCodes::default();
        let first = codes.create();
        let _ = codes.create();
        let _ = codes.create();
        assert_eq!(codes.live_count(), 3);
        let _ = codes.create();
        assert_eq!(codes.live_count(), 3, "the oldest went");
        assert_eq!(codes.redeem(&first.code), Err(PairError::ExpiredCode));
        let late = codes.create();
        codes.expire_all_for_test();
        assert_eq!(codes.redeem(&late.code), Err(PairError::ExpiredCode));
    }

    #[test]
    fn failures_are_rate_limited() {
        let codes = PairingCodes::default();
        let good = codes.create();
        for _ in 0..MAX_FAILURES_PER_MINUTE {
            assert!(matches!(
                codes.redeem("0000-0000"),
                Err(PairError::BadCode | PairError::ExpiredCode)
            ));
        }
        // Blocked now, even with the right code.
        match codes.redeem(&good.code) {
            Err(PairError::RateLimited(secs)) => assert!((1..=60).contains(&secs)),
            other => panic!("{other:?}"),
        }
        assert_eq!(PairError::RateLimited(5).code(), "rate_limited");
        assert!(PairError::RateLimited(5).message().contains("5 seconds"));
    }
}
