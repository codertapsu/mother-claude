//! Connected browsers: one connection per paired browser (a newer one
//! replaces the older, which is told `replaced` and closed), requests to the
//! extension correlated by `id` with a deadline, and the replies delivered
//! by the connection loop (`channel`).

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, watch};

/// Frames queued for one connection.
pub const QUEUE: usize = 256;
/// Unauthenticated connections at once (the oldest is closed beyond it).
pub const MAX_PREAUTH: usize = 8;

/// Why the desktop closes a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    /// The user removed this browser (`error` `revoked`).
    Revoked,
    /// A newer connection of the same browser took over (`error` `replaced`).
    Replaced,
    /// Too many unauthenticated connections: the oldest goes.
    Evicted,
}

/// A request that got no reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestError {
    /// The connection closed (or was replaced) first.
    Disconnected,
    /// No reply before the deadline.
    Timeout,
}

struct Waiter {
    lease: Option<String>,
    tx: oneshot::Sender<Value>,
}

/// One authenticated connection.
pub struct Conn {
    pub id: u64,
    pub browser: String,
    /// The extension's origin (`chrome-extension://<id>`).
    pub origin: String,
    pub connected_at: i64,
    tx: mpsc::Sender<String>,
    waiters: Mutex<HashMap<String, Waiter>>,
    last_active: AtomicI64,
    pub capabilities: Mutex<Value>,
    pub permissions: Mutex<Vec<String>>,
    pub extension_version: Mutex<String>,
    close: watch::Sender<Option<CloseReason>>,
}

impl std::fmt::Debug for Conn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Conn")
            .field("id", &self.id)
            .field("browser", &self.browser)
            .finish_non_exhaustive()
    }
}

fn now_ms() -> i64 {
    crate::claude::registry::now_ms()
}

impl Conn {
    /// A connection whose frames go to `tx`; `close` is watched by its loop.
    pub fn new(
        id: u64,
        browser: &str,
        origin: &str,
        tx: mpsc::Sender<String>,
    ) -> (Arc<Self>, watch::Receiver<Option<CloseReason>>) {
        let (close, close_rx) = watch::channel(None);
        let now = now_ms();
        (
            Arc::new(Self {
                id,
                browser: browser.to_string(),
                origin: origin.to_string(),
                connected_at: now,
                tx,
                waiters: Mutex::new(HashMap::new()),
                last_active: AtomicI64::new(now),
                capabilities: Mutex::new(Value::Null),
                permissions: Mutex::new(Vec::new()),
                extension_version: Mutex::new(String::new()),
                close,
            }),
            close_rx,
        )
    }

    fn waiters(&self) -> std::sync::MutexGuard<'_, HashMap<String, Waiter>> {
        self.waiters.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The extension did something (a frame other than `ping`).
    pub fn mark_active(&self) {
        self.last_active.store(now_ms(), Ordering::Relaxed);
    }

    pub fn last_active(&self) -> i64 {
        self.last_active.load(Ordering::Relaxed)
    }

    /// Queue a frame without waiting (`state`, `pong`). `false` when the
    /// queue is full or the connection is gone.
    pub fn push(&self, frame: &Value) -> bool {
        self.tx.try_send(frame.to_string()).is_ok()
    }

    /// Send a request and wait for the frame that answers it (same `id`) —
    /// `result`, `lease.opened`, `ok` or `error` — for at most `deadline`.
    /// `lease`: the lease it acts for (a Stop of that lease fails it).
    pub async fn request(
        &self,
        id: &str,
        lease: Option<&str>,
        frame: &Value,
        deadline: Duration,
    ) -> Result<Value, RequestError> {
        let (tx, rx) = oneshot::channel();
        self.waiters().insert(
            id.to_string(),
            Waiter {
                lease: lease.map(str::to_string),
                tx,
            },
        );
        // However this ends — a reply, the deadline, or the caller giving up
        // (its future dropped) — nobody waits for the id any more.
        let _forget = Forget { conn: self, id };
        if self.tx.send(frame.to_string()).await.is_err() {
            return Err(RequestError::Disconnected);
        }
        match tokio::time::timeout(deadline, rx).await {
            Ok(Ok(reply)) => Ok(reply),
            Ok(Err(_)) => Err(RequestError::Disconnected),
            Err(_) => Err(RequestError::Timeout),
        }
    }

    /// A reply arrived: hand it to its waiter. `false` when nobody waits
    /// (late, unknown, or already answered).
    pub fn deliver(&self, id: &str, frame: Value) -> bool {
        match self.waiters().remove(id) {
            Some(w) => w.tx.send(frame).is_ok(),
            None => false,
        }
    }

    /// Fail every request of `lease` with a synthetic `result` (`code`).
    pub fn fail_lease(&self, lease: &str, code: &str, message: &str) -> usize {
        let ids: Vec<String> = self
            .waiters()
            .iter()
            .filter(|(_, w)| w.lease.as_deref() == Some(lease))
            .map(|(id, _)| id.clone())
            .collect();
        let mut n = 0;
        for id in ids {
            let frame = json!({
                "type": "result",
                "id": id,
                "ok": false,
                "error": { "code": code, "message": message },
            });
            n += usize::from(self.deliver(&id, frame));
        }
        n
    }

    /// Requests waiting for a reply.
    pub fn waiting(&self) -> usize {
        self.waiters().len()
    }

    /// Fail every waiting request (the connection is gone).
    fn drop_waiters(&self) {
        self.waiters().clear();
    }

    /// Ask the connection's loop to close it.
    pub fn close(&self, reason: CloseReason) {
        let _ = self.close.send(Some(reason));
    }
}

/// Removes a request's waiter when its `request` ends, however it ends.
struct Forget<'a> {
    conn: &'a Conn,
    id: &'a str,
}

impl Drop for Forget<'_> {
    fn drop(&mut self) {
        self.conn.waiters().remove(self.id);
    }
}

/// The connected browsers.
#[derive(Default)]
pub struct Hub {
    next: AtomicU64,
    conns: Mutex<HashMap<String, Arc<Conn>>>,
    preauth: Mutex<VecDeque<(u64, watch::Sender<Option<CloseReason>>)>>,
}

impl std::fmt::Debug for Hub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hub")
            .field("connected", &self.connected().len())
            .finish()
    }
}

impl Hub {
    fn conns(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Conn>>> {
        self.conns.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// A fresh number (connections, request ids).
    pub fn next_id(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// A fresh request id with `prefix` (`c` commands, `r` lease requests).
    pub fn request_id(&self, prefix: &str) -> String {
        format!("{prefix}_{}", self.next_id())
    }

    /// Hold a slot for an unauthenticated connection; beyond
    /// [`MAX_PREAUTH`] the oldest is told to close.
    pub fn enter_preauth(&self, id: u64) -> watch::Receiver<Option<CloseReason>> {
        let (tx, rx) = watch::channel(None);
        let mut q = self.preauth.lock().unwrap_or_else(|p| p.into_inner());
        q.push_back((id, tx));
        while q.len() > MAX_PREAUTH {
            if let Some((_, old)) = q.pop_front() {
                let _ = old.send(Some(CloseReason::Evicted));
            }
        }
        rx
    }

    /// The connection authenticated (or ended): release its slot.
    pub fn leave_preauth(&self, id: u64) {
        self.preauth
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|(i, _)| *i != id);
    }

    /// Make `conn` its browser's connection; the one it replaces (if any) is
    /// told to close.
    pub fn attach(&self, conn: Arc<Conn>) -> Option<Arc<Conn>> {
        let old = self.conns().insert(conn.browser.clone(), conn);
        if let Some(old) = &old {
            old.close(CloseReason::Replaced);
            old.drop_waiters();
        }
        old
    }

    /// A connection ended: forget it if it is still its browser's.
    /// `true` when it was.
    pub fn detach(&self, conn: &Conn) -> bool {
        let mut g = self.conns();
        let current = g.get(&conn.browser).is_some_and(|c| c.id == conn.id);
        if current {
            g.remove(&conn.browser);
        }
        drop(g);
        conn.drop_waiters();
        current
    }

    pub fn get(&self, browser: &str) -> Option<Arc<Conn>> {
        self.conns().get(browser).cloned()
    }

    /// Every connection.
    pub fn connected(&self) -> Vec<Arc<Conn>> {
        self.conns().values().cloned().collect()
    }

    /// The connection the extension used most recently.
    pub fn most_recent(&self) -> Option<Arc<Conn>> {
        self.conns()
            .values()
            .max_by_key(|c| (c.last_active(), c.id))
            .cloned()
    }

    /// Close a browser's connection (revoked). `true` when it was connected.
    pub fn close_browser(&self, browser: &str, reason: CloseReason) -> bool {
        let conn = self.conns().remove(browser);
        match conn {
            Some(c) => {
                c.close(reason);
                c.drop_waiters();
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn requests_correlate_time_out_and_fail_by_lease() {
        let (tx, mut rx) = mpsc::channel(8);
        let (conn, _close) = Conn::new(1, "brw_1", "chrome-extension://x", tx);
        // A reply delivered by id.
        let c = conn.clone();
        let task = tokio::spawn(async move {
            c.request(
                "c_1",
                Some("l_1"),
                &json!({"id": "c_1"}),
                Duration::from_secs(5),
            )
            .await
        });
        let sent = rx.recv().await.unwrap();
        assert!(sent.contains("c_1"));
        assert!(conn.deliver("c_1", json!({"type": "result", "id": "c_1", "ok": true})));
        assert_eq!(task.await.unwrap().unwrap()["ok"], true);
        assert!(!conn.deliver("c_1", json!({})), "answered once");

        // No reply: the deadline.
        let r = conn
            .request("c_2", None, &json!({}), Duration::from_millis(20))
            .await;
        assert_eq!(r, Err(RequestError::Timeout));
        assert_eq!(conn.waiting(), 0);

        // A Stop fails the lease's requests with a result.
        let c = conn.clone();
        let task = tokio::spawn(async move {
            c.request("c_3", Some("l_9"), &json!({}), Duration::from_secs(5))
                .await
        });
        let _ = rx.recv().await;
        while conn.waiting() == 0 {
            tokio::task::yield_now().await;
        }
        assert_eq!(conn.fail_lease("l_9", "stopped", "Stopped"), 1);
        let reply = task.await.unwrap().unwrap();
        assert_eq!(reply["error"]["code"], "stopped");

        // A caller that gives up (its future dropped) leaves no waiter.
        let c = conn.clone();
        let task = tokio::spawn(async move {
            c.request("c_4", None, &json!({}), Duration::from_secs(30))
                .await
        });
        while conn.waiting() == 0 {
            tokio::task::yield_now().await;
        }
        task.abort();
        let _ = task.await;
        assert_eq!(conn.waiting(), 0);
    }

    #[tokio::test]
    async fn one_connection_per_browser() {
        let hub = Hub::default();
        let (tx1, _rx1) = mpsc::channel(8);
        let (a, mut a_close) = Conn::new(hub.next_id(), "brw_1", "o", tx1);
        let (tx2, _rx2) = mpsc::channel(8);
        let (b, _b_close) = Conn::new(hub.next_id(), "brw_1", "o", tx2);
        assert!(hub.attach(a.clone()).is_none());
        assert!(hub.attach(b.clone()).is_some());
        a_close.changed().await.unwrap();
        assert_eq!(*a_close.borrow(), Some(CloseReason::Replaced));
        assert!(!hub.detach(&a), "the replaced one is not current");
        assert_eq!(hub.get("brw_1").unwrap().id, b.id);
        assert_eq!(hub.most_recent().unwrap().id, b.id);
        assert!(hub.close_browser("brw_1", CloseReason::Revoked));
        assert!(hub.connected().is_empty());
    }

    #[tokio::test]
    async fn too_many_unauthenticated_connections_close_the_oldest() {
        let hub = Hub::default();
        let mut first = hub.enter_preauth(1);
        for i in 2..=(MAX_PREAUTH as u64) {
            let _ = hub.enter_preauth(i);
        }
        assert!(first.borrow().is_none());
        let _ = hub.enter_preauth(99);
        first.changed().await.unwrap();
        assert_eq!(*first.borrow(), Some(CloseReason::Evicted));
        hub.leave_preauth(99);
    }
}
