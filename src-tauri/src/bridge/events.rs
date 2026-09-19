//! Bounded replay event logs — the backing store for every SSE stream.
//!
//! The dashboard's `tokio::sync::broadcast` bus is lossy by design (a lagging
//! WebSocket client simply drops frames). An HTTP client that reconnects mid
//! turn needs the opposite: a durable-enough window it can resume from with
//! `Last-Event-ID`. So each operation owns a log, and one global log carries
//! everything, both capped at [`MAX_EVENTS`] / [`MAX_BYTES`].
//!
//! The cursor contract is exactly the Codex bridge's, because clients depend on
//! being able to tell "you asked for something I threw away" (410) apart from
//! "you asked for something that hasn't happened" (422):
//!
//! * `after` older than the oldest retained event → [`BridgeError::events_expired`] (410)
//! * `after` greater than the newest sequence → `invalid_cursor` (422)
//! * otherwise → every retained event with `seq > after`, waiting up to
//!   `wait` for at least one to arrive.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::watch;

use super::error::{BridgeError, BridgeResult};

/// Retained events per log.
pub const MAX_EVENTS: usize = 2048;
/// Retained bytes per log.
pub const MAX_BYTES: usize = 8 * 1024 * 1024;

/// One retained event. `data` is the already-serialized JSON payload so a
/// replay never re-serializes and every subscriber shares one allocation.
#[derive(Debug, Clone)]
pub struct LogEvent {
    pub seq: u64,
    pub data: Arc<str>,
}

#[derive(Debug)]
struct Inner {
    events: VecDeque<LogEvent>,
    bytes: usize,
    sequence: u64,
    closed: bool,
}

/// A bounded, replayable, single-writer-many-reader event log.
#[derive(Debug)]
pub struct EventLog {
    inner: Mutex<Inner>,
    /// Bumped on every append/close so readers wake without polling.
    tx: watch::Sender<u64>,
    max_events: usize,
    max_bytes: usize,
}

impl Default for EventLog {
    fn default() -> Self {
        Self::new(MAX_EVENTS, MAX_BYTES)
    }
}

/// What a read returned: the events after the cursor, and whether the log is
/// finished (no further events will ever arrive).
#[derive(Debug)]
pub struct ReadOut {
    pub events: Vec<LogEvent>,
    pub closed: bool,
}

impl EventLog {
    pub fn new(max_events: usize, max_bytes: usize) -> Self {
        let (tx, _rx) = watch::channel(0);
        Self {
            inner: Mutex::new(Inner {
                events: VecDeque::new(),
                bytes: 0,
                sequence: 0,
                closed: false,
            }),
            tx,
            max_events,
            max_bytes,
        }
    }

    /// Append a JSON value. Returns the assigned sequence number, or `None` if
    /// the log is already closed.
    pub fn append(&self, value: &Value) -> Option<u64> {
        let data: Arc<str> = Arc::from(value.to_string().as_str());
        self.append_raw(data)
    }

    pub fn append_raw(&self, data: Arc<str>) -> Option<u64> {
        let seq = {
            let mut inner = match self.inner.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            if inner.closed {
                return None;
            }
            inner.sequence += 1;
            let seq = inner.sequence;
            inner.bytes += data.len();
            inner.events.push_back(LogEvent { seq, data });
            // Always keep at least one event, even one larger than max_bytes,
            // so an oversized frame is still readable rather than vanishing.
            while inner.events.len() > 1
                && (inner.events.len() > self.max_events || inner.bytes > self.max_bytes)
            {
                if let Some(old) = inner.events.pop_front() {
                    inner.bytes -= old.data.len();
                }
            }
            seq
        };
        let _ = self.tx.send(seq);
        Some(seq)
    }

    /// Mark the log finished and wake every reader.
    pub fn close(&self) {
        {
            let mut inner = match self.inner.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            if inner.closed {
                return;
            }
            inner.closed = true;
        }
        // Bump the watch value so every blocked reader re-checks and observes
        // the closed flag. `send_modify` notifies even with no receivers.
        self.tx.send_modify(|v| *v = v.wrapping_add(1));
    }

    pub fn is_closed(&self) -> bool {
        self.lock().closed
    }

    pub fn sequence(&self) -> u64 {
        self.lock().sequence
    }

    /// The cursor a brand-new subscriber should start from so it receives the
    /// whole retained window instead of a 410.
    pub fn oldest_cursor(&self) -> u64 {
        let inner = self.lock();
        match inner.events.front() {
            Some(e) => e.seq - 1,
            None => inner.sequence,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }

    /// Validate a cursor against the retained window without reading.
    ///
    /// "Ahead of the head" is checked first so an absurd cursor is a 422 about
    /// the cursor rather than a 410 about eviction — and so the arithmetic
    /// below cannot overflow on `u64::MAX`.
    fn check_cursor(inner: &Inner, after: u64) -> BridgeResult<()> {
        if after > inner.sequence {
            return Err(BridgeError::invalid_cursor(format!(
                "Cursor {after} is ahead of the newest event {}.",
                inner.sequence
            ))
            .with("newest_event_id", inner.sequence));
        }
        if let Some(front) = inner.events.front() {
            if after < front.seq.saturating_sub(1) {
                return Err(BridgeError::events_expired()
                    .with("oldest_event_id", front.seq)
                    .with("requested_after", after));
            }
        }
        Ok(())
    }

    /// Read everything after `after`, waiting up to `wait` for the first event.
    ///
    /// Returns immediately when events are already available or the log is
    /// closed; returns an empty batch on timeout so the caller can emit an SSE
    /// heartbeat and come back.
    pub async fn read(&self, after: u64, wait: Duration) -> BridgeResult<ReadOut> {
        // Subscribe *before* the first look so an append racing this read wakes
        // us instead of being missed.
        let mut rx = self.tx.subscribe();
        loop {
            {
                let inner = self.lock();
                Self::check_cursor(&inner, after)?;
                let events: Vec<LogEvent> = inner
                    .events
                    .iter()
                    .filter(|e| e.seq > after)
                    .cloned()
                    .collect();
                if !events.is_empty() || inner.closed {
                    return Ok(ReadOut {
                        events,
                        closed: inner.closed,
                    });
                }
            }
            if tokio::time::timeout(wait, rx.changed()).await.is_err() {
                let inner = self.lock();
                Self::check_cursor(&inner, after)?;
                return Ok(ReadOut {
                    events: Vec::new(),
                    closed: inner.closed,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: Duration = Duration::from_millis(0);

    #[tokio::test]
    async fn sequences_start_at_one_and_replay_is_stable() {
        let log = EventLog::default();
        log.append(&json!({"n": 1}));
        log.append(&json!({"n": 2}));

        let first = log.read(0, NOW).await.unwrap();
        assert_eq!(first.events.len(), 2);
        assert_eq!(first.events[0].seq, 1);
        assert!(!first.closed);

        // Reading again is non-destructive and byte-identical.
        let again = log.read(0, NOW).await.unwrap();
        assert_eq!(
            again.events.iter().map(|e| &*e.data).collect::<Vec<_>>(),
            first.events.iter().map(|e| &*e.data).collect::<Vec<_>>()
        );

        // A cursor skips what it has already seen.
        let after_one = log.read(1, NOW).await.unwrap();
        assert_eq!(after_one.events.len(), 1);
        assert_eq!(after_one.events[0].seq, 2);
    }

    #[tokio::test]
    async fn eviction_expires_old_cursors_but_keeps_one_event() {
        let log = EventLog::new(2, MAX_BYTES);
        for n in 1..=3 {
            log.append(&json!({ "n": n }));
        }
        // Oldest retained is seq 2, so a fresh subscriber starts at 1.
        assert_eq!(log.oldest_cursor(), 1);
        let err = log.read(0, NOW).await.unwrap_err();
        assert_eq!(err.code, "events_expired");
        assert_eq!(err.status, axum::http::StatusCode::GONE);

        let ok = log.read(log.oldest_cursor(), NOW).await.unwrap();
        assert_eq!(ok.events.len(), 2);
    }

    #[tokio::test]
    async fn a_single_oversized_event_is_still_retained() {
        let log = EventLog::new(MAX_EVENTS, 8);
        log.append(&json!({ "big": "x".repeat(64) }));
        let out = log.read(0, NOW).await.unwrap();
        assert_eq!(out.events.len(), 1);
    }

    #[tokio::test]
    async fn probe_u64_max_cursor() {
        let log = EventLog::default();
        log.append(&json!({"n": 1}));
        let r = log.read(u64::MAX, NOW).await;
        match r {
            Ok(_) => println!("PROBE: Ok"),
            Err(e) => println!("PROBE: err code={} status={}", e.code, e.status),
        }
    }

    #[tokio::test]
    async fn cursor_beyond_head_is_422() {
        let log = EventLog::default();
        log.append(&json!({"n": 1}));
        for absurd in [9, u64::MAX] {
            let err = log.read(absurd, NOW).await.unwrap_err();
            assert_eq!(err.code, "invalid_cursor", "cursor {absurd}");
            assert_eq!(err.status, axum::http::StatusCode::UNPROCESSABLE_ENTITY);
        }
    }

    #[tokio::test]
    async fn close_wakes_readers_and_reports_closed() {
        let log = Arc::new(EventLog::default());
        let bg = log.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            bg.append(&serde_json::json!({"n": 1}));
            bg.close();
        });
        let out = log.read(0, Duration::from_secs(5)).await.unwrap();
        assert_eq!(out.events.len(), 1);
        // The append and the close race; either way a follow-up read observes
        // the closed log rather than hanging.
        let tail = log.read(1, Duration::from_secs(5)).await.unwrap();
        assert!(tail.closed);
        assert!(log.append(&serde_json::json!({"n": 2})).is_none());
    }

    #[tokio::test]
    async fn read_waits_for_a_late_event() {
        let log = Arc::new(EventLog::default());
        let bg = log.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            bg.append(&serde_json::json!({"late": true}));
        });
        let out = log.read(0, Duration::from_secs(5)).await.unwrap();
        assert_eq!(out.events.len(), 1);
    }

    #[tokio::test]
    async fn read_times_out_empty_rather_than_erroring() {
        let log = EventLog::default();
        let out = log.read(0, Duration::from_millis(10)).await.unwrap();
        assert!(out.events.is_empty());
        assert!(!out.closed);
    }
}
