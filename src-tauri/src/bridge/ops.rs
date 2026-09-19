//! Background operations: the unit of work behind `202 Accepted`.
//!
//! Starting a turn returns immediately with an `operation_id`; the turn itself
//! runs on. Clients then either stream `/operations/{id}/events`, poll
//! `/operations/{id}`, or (via `/run`) let the bridge await completion for
//! them. Each operation owns its own [`EventLog`], so a client that connects
//! late still replays the whole turn from the beginning.
//!
//! Caps mirror the Codex bridge so the two services behave alike under load:
//! [`MAX_ACTIVE`] concurrent, [`MAX_RETAINED`] kept, [`RETENTION`] before a
//! finished operation may be evicted. `MAX_RETAINED` bounds the registry as a
//! whole, and only *finished* operations are ever evicted — so running work
//! counts against the budget and pushes the oldest finished operations out
//! rather than being dropped itself.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::watch;
use uuid::Uuid;

use super::error::{BridgeError, BridgeResult};
use super::events::EventLog;

/// Concurrently unfinished operations before new starts are refused with 409.
pub const MAX_ACTIVE: usize = 16;
/// Total operations the registry holds. Only finished ones are evicted, so in
/// the worst case `MAX_RETAINED - MAX_ACTIVE` finished operations stay readable.
pub const MAX_RETAINED: usize = 128;
/// How long a finished operation stays readable.
pub const RETENTION: Duration = Duration::from_secs(1800);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OperationStatus {
    Running,
    Completed,
    Failed,
    Interrupted,
}

impl OperationStatus {
    pub fn is_terminal(self) -> bool {
        !matches!(self, OperationStatus::Running)
    }
}

#[derive(Debug)]
struct OpState {
    status: OperationStatus,
    result: Option<Value>,
    error: Option<Value>,
    finished_at: Option<Instant>,
}

/// One background operation.
#[derive(Debug)]
pub struct Operation {
    pub id: String,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub log: Arc<EventLog>,
    pub created_at: Instant,
    state: Mutex<OpState>,
    done_tx: watch::Sender<bool>,
}

impl Operation {
    fn new(id: String, thread_id: Option<String>, turn_id: Option<String>) -> Arc<Self> {
        let (done_tx, _rx) = watch::channel(false);
        Arc::new(Self {
            id,
            thread_id,
            turn_id,
            log: Arc::new(EventLog::default()),
            created_at: Instant::now(),
            state: Mutex::new(OpState {
                status: OperationStatus::Running,
                result: None,
                error: None,
                finished_at: None,
            }),
            done_tx,
        })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, OpState> {
        match self.state.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }

    pub fn status(&self) -> OperationStatus {
        self.state().status
    }

    pub fn is_finished(&self) -> bool {
        self.state().status.is_terminal()
    }

    /// Context fields echoed into every snapshot and error body.
    fn context(&self) -> Vec<(&'static str, Value)> {
        let mut out = Vec::new();
        if let Some(t) = &self.thread_id {
            out.push(("thread_id", json!(t)));
        }
        if let Some(t) = &self.turn_id {
            out.push(("turn_id", json!(t)));
        }
        out
    }

    pub fn snapshot(&self) -> Value {
        let state = self.state();
        let mut map = serde_json::Map::new();
        map.insert("operation_id".into(), json!(self.id));
        for (k, v) in self.context() {
            map.insert(k.into(), v);
        }
        map.insert("status".into(), json!(state.status));
        map.insert("result".into(), state.result.clone().unwrap_or(Value::Null));
        map.insert("error".into(), state.error.clone().unwrap_or(Value::Null));
        Value::Object(map)
    }

    /// Append an event to this operation's log (and mirror it to `global`).
    pub fn emit(&self, global: &EventLog, value: &Value) {
        let data: Arc<str> = Arc::from(value.to_string().as_str());
        self.log.append_raw(data.clone());
        global.append_raw(data);
    }

    /// Finish the operation exactly once.
    ///
    /// Returns `true` for the caller that won the race — used so a watchdog
    /// only escalates when it actually was the one that timed the turn out.
    pub fn finish(
        &self,
        global: &EventLog,
        status: OperationStatus,
        result: Option<Value>,
        error: Option<Value>,
    ) -> bool {
        {
            let mut state = self.state();
            if state.status.is_terminal() {
                return false;
            }
            state.status = status;
            state.result = result.clone();
            state.error = error.clone();
            state.finished_at = Some(Instant::now());
        }

        // A terminal frame tells a streaming client the log is finished rather
        // than merely quiet.
        let mut params = serde_json::Map::new();
        params.insert("operation_id".into(), json!(self.id));
        for (k, v) in self.context() {
            params.insert(k.into(), v);
        }
        params.insert("status".into(), json!(status));
        let terminal = match &error {
            Some(err) => {
                params.insert("error".into(), err.clone());
                json!({ "method": "bridge/error", "params": Value::Object(params) })
            }
            None => {
                params.insert("result".into(), result.unwrap_or(Value::Null));
                json!({ "method": "bridge/completed", "params": Value::Object(params) })
            }
        };
        self.emit(global, &terminal);
        self.log.close();
        let _ = self.done_tx.send(true);
        true
    }

    /// Fail with the standard error envelope shape, carrying this operation's
    /// context so a client never has to correlate by hand.
    pub fn fail(&self, global: &EventLog, code: &str, message: impl Into<String>) -> bool {
        let mut err = serde_json::Map::new();
        err.insert("code".into(), json!(code));
        err.insert("message".into(), json!(message.into()));
        for (k, v) in self.context() {
            err.insert(k.into(), v);
        }
        self.finish(
            global,
            OperationStatus::Failed,
            None,
            Some(Value::Object(err)),
        )
    }

    /// Await completion, up to `wait`. Returns `true` if it finished in time.
    pub async fn wait_done(&self, wait: Duration) -> bool {
        let mut rx = self.done_tx.subscribe();
        if *rx.borrow_and_update() || self.is_finished() {
            return true;
        }
        tokio::time::timeout(wait, rx.changed()).await.is_ok() || self.is_finished()
    }

    fn finished_at(&self) -> Option<Instant> {
        self.state().finished_at
    }
}

/// The process-wide operation registry.
#[derive(Debug)]
pub struct OperationRegistry {
    ops: Mutex<HashMap<String, Arc<Operation>>>,
    /// `(thread_id, turn_id)` → operation id, so `/steer` and `/interrupt` can
    /// find the operation a turn belongs to.
    by_turn: Mutex<HashMap<(String, String), String>>,
    /// Every event from every operation, for the global `/events` feed.
    pub global: Arc<EventLog>,
    max_active: usize,
    max_retained: usize,
    retention: Duration,
}

impl Default for OperationRegistry {
    fn default() -> Self {
        Self::new(MAX_ACTIVE, MAX_RETAINED, RETENTION)
    }
}

impl OperationRegistry {
    pub fn new(max_active: usize, max_retained: usize, retention: Duration) -> Self {
        Self {
            ops: Mutex::new(HashMap::new()),
            by_turn: Mutex::new(HashMap::new()),
            global: Arc::new(EventLog::default()),
            max_active,
            max_retained,
            retention,
        }
    }

    fn ops(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Operation>>> {
        match self.ops.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }

    fn by_turn(&self) -> std::sync::MutexGuard<'_, HashMap<(String, String), String>> {
        match self.by_turn.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }

    /// Evict finished operations that have either aged past the retention
    /// window or pushed the registry over its cap, oldest first. Running
    /// operations are never evicted — they are still producing events — so they
    /// consume the budget and squeeze finished ones out ahead of schedule.
    fn prune(&self) {
        let mut ops = self.ops();
        let now = Instant::now();
        let mut finished: Vec<(Instant, String)> = ops
            .values()
            .filter_map(|op| op.finished_at().map(|at| (at, op.id.clone())))
            .collect();
        finished.sort_by_key(|(at, _)| *at);

        let mut total = ops.len();
        let mut evicted: Vec<String> = Vec::new();
        for (at, id) in finished {
            let aged_out = now.duration_since(at) > self.retention;
            let over_cap = total > self.max_retained;
            if !aged_out && !over_cap {
                break;
            }
            ops.remove(&id);
            evicted.push(id);
            total -= 1;
        }
        drop(ops);

        if !evicted.is_empty() {
            let mut index = self.by_turn();
            index.retain(|_, op_id| !evicted.contains(op_id));
        }
    }

    /// Register a new running operation, or 409 if too many are already live.
    pub fn create(
        &self,
        thread_id: Option<String>,
        turn_id: Option<String>,
    ) -> BridgeResult<Arc<Operation>> {
        self.prune();
        {
            let ops = self.ops();
            let active = ops.values().filter(|o| !o.is_finished()).count();
            if active >= self.max_active {
                return Err(BridgeError::busy(format!(
                    "{active} operations are already running (limit {}); wait for one to finish.",
                    self.max_active
                ))
                .with("max_active_operations", self.max_active));
            }
        }
        let op = Operation::new(
            Uuid::new_v4().to_string(),
            thread_id.clone(),
            turn_id.clone(),
        );
        self.ops().insert(op.id.clone(), op.clone());
        if let (Some(t), Some(tu)) = (thread_id, turn_id) {
            self.by_turn().insert((t, tu), op.id.clone());
        }
        Ok(op)
    }

    /// Drop an operation that never actually started, so a failed launch does
    /// not burn one of the active slots until it ages out.
    pub fn discard(&self, id: &str) {
        self.ops().remove(id);
        self.by_turn().retain(|_, op_id| op_id != id);
    }

    pub fn get(&self, id: &str) -> BridgeResult<Arc<Operation>> {
        self.prune();
        self.ops()
            .get(id)
            .cloned()
            .ok_or_else(|| BridgeError::operation_not_found(id))
    }

    pub fn for_turn(&self, thread_id: &str, turn_id: &str) -> Option<Arc<Operation>> {
        let id = self
            .by_turn()
            .get(&(thread_id.to_string(), turn_id.to_string()))
            .cloned()?;
        self.ops().get(&id).cloned()
    }

    /// Every operation belonging to a thread, newest first.
    pub fn for_thread(&self, thread_id: &str) -> Vec<Arc<Operation>> {
        let mut out: Vec<Arc<Operation>> = self
            .ops()
            .values()
            .filter(|o| o.thread_id.as_deref() == Some(thread_id))
            .cloned()
            .collect();
        out.sort_by_key(|o| std::cmp::Reverse(o.created_at));
        out
    }

    pub fn active_count(&self) -> usize {
        self.ops().values().filter(|o| !o.is_finished()).count()
    }

    /// Fail every unfinished operation — used when the runtime dies.
    pub fn fail_all(&self, code: &str, message: &str) {
        let running: Vec<Arc<Operation>> = self
            .ops()
            .values()
            .filter(|o| !o.is_finished())
            .cloned()
            .collect();
        for op in running {
            op.fail(&self.global, code, message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn snapshot_carries_context_and_status() {
        let reg = OperationRegistry::default();
        let op = reg.create(Some("t1".into()), Some("turn1".into())).unwrap();
        let snap = op.snapshot();
        assert_eq!(snap["thread_id"], "t1");
        assert_eq!(snap["turn_id"], "turn1");
        assert_eq!(snap["status"], "running");
        assert!(snap["result"].is_null());
    }

    #[tokio::test]
    async fn finish_is_idempotent_and_only_one_caller_wins() {
        let reg = OperationRegistry::default();
        let op = reg.create(Some("t".into()), Some("u".into())).unwrap();
        assert!(op.finish(
            &reg.global,
            OperationStatus::Completed,
            Some(serde_json::json!({"ok": true})),
            None
        ));
        assert!(!op.fail(&reg.global, "timeout", "too late"));
        assert_eq!(op.status(), OperationStatus::Completed);
        assert!(op.log.is_closed());
    }

    #[tokio::test]
    async fn terminal_frame_lands_on_both_logs() {
        let reg = OperationRegistry::default();
        let op = reg.create(None, None).unwrap();
        op.finish(&reg.global, OperationStatus::Completed, None, None);

        let out = op.log.read(0, Duration::from_millis(0)).await.unwrap();
        let last = out.events.last().unwrap();
        let parsed: Value = serde_json::from_str(&last.data).unwrap();
        assert_eq!(parsed["method"], "bridge/completed");
        assert!(out.closed);

        let global = reg.global.read(0, Duration::from_millis(0)).await.unwrap();
        assert_eq!(global.events.len(), 1);
    }

    #[tokio::test]
    async fn failure_frame_uses_the_error_envelope() {
        let reg = OperationRegistry::default();
        let op = reg.create(Some("t".into()), None).unwrap();
        op.fail(&reg.global, "claude_error", "boom");
        let snap = op.snapshot();
        assert_eq!(snap["status"], "failed");
        assert_eq!(snap["error"]["code"], "claude_error");
        assert_eq!(snap["error"]["thread_id"], "t");
    }

    #[tokio::test]
    async fn active_cap_is_enforced_and_released_on_finish() {
        let reg = OperationRegistry::new(2, MAX_RETAINED, RETENTION);
        let a = reg.create(None, None).unwrap();
        let _b = reg.create(None, None).unwrap();
        let err = reg.create(None, None).unwrap_err();
        assert_eq!(err.code, "busy");
        assert_eq!(err.status, axum::http::StatusCode::CONFLICT);

        a.finish(&reg.global, OperationStatus::Completed, None, None);
        assert!(reg.create(None, None).is_ok());
    }

    #[tokio::test]
    async fn discard_frees_a_slot_immediately() {
        let reg = OperationRegistry::new(1, MAX_RETAINED, RETENTION);
        let a = reg.create(None, None).unwrap();
        reg.discard(&a.id);
        assert!(reg.create(None, None).is_ok());
        assert_eq!(reg.active_count(), 1);
    }

    #[tokio::test]
    async fn finished_operations_evict_oldest_first_over_the_cap() {
        let reg = OperationRegistry::new(MAX_ACTIVE, 2, RETENTION);
        let a = reg.create(None, None).unwrap();
        a.finish(&reg.global, OperationStatus::Completed, None, None);
        let b = reg.create(None, None).unwrap();
        b.finish(&reg.global, OperationStatus::Completed, None, None);

        // Two finished operations sit exactly at the cap, so both survive.
        assert!(reg.get(&a.id).is_ok());
        assert!(reg.get(&b.id).is_ok());

        // A third operation pushes past it: the oldest finished one goes, and
        // the still-running one is never a candidate.
        let c = reg.create(None, None).unwrap();
        assert!(
            reg.get(&a.id).is_err(),
            "the oldest finished op should be evicted"
        );
        assert!(reg.get(&b.id).is_ok());
        assert!(reg.get(&c.id).is_ok());
    }

    #[tokio::test]
    async fn running_operations_are_never_evicted() {
        let reg = OperationRegistry::new(MAX_ACTIVE, 1, RETENTION);
        let running = reg.create(None, None).unwrap();
        for _ in 0..4 {
            let done = reg.create(None, None).unwrap();
            done.finish(&reg.global, OperationStatus::Completed, None, None);
        }
        assert!(reg.get(&running.id).is_ok());
        assert_eq!(reg.active_count(), 1);
    }

    #[tokio::test]
    async fn an_evicted_operation_leaves_no_turn_index_entry() {
        let reg = OperationRegistry::new(MAX_ACTIVE, 1, RETENTION);
        let a = reg.create(Some("t".into()), Some("u".into())).unwrap();
        a.finish(&reg.global, OperationStatus::Completed, None, None);
        let b = reg.create(Some("t".into()), Some("v".into())).unwrap();
        b.finish(&reg.global, OperationStatus::Completed, None, None);
        reg.prune();
        assert!(reg.for_turn("t", "u").is_none());
    }

    #[tokio::test]
    async fn unknown_operation_is_404() {
        let reg = OperationRegistry::default();
        let err = reg.get("nope").unwrap_err();
        assert_eq!(err.code, "operation_not_found");
        assert_eq!(err.status, axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn turn_index_resolves_and_wait_done_returns() {
        let reg = OperationRegistry::default();
        let op = reg.create(Some("t".into()), Some("u".into())).unwrap();
        assert_eq!(reg.for_turn("t", "u").unwrap().id, op.id);
        assert!(!op.wait_done(Duration::from_millis(5)).await);
        op.finish(&reg.global, OperationStatus::Completed, None, None);
        assert!(op.wait_done(Duration::from_millis(5)).await);
    }

    #[tokio::test]
    async fn fail_all_terminates_running_work() {
        let reg = OperationRegistry::default();
        let op = reg.create(None, None).unwrap();
        reg.fail_all("runtime_unavailable", "runtime closed");
        assert_eq!(op.status(), OperationStatus::Failed);
        assert_eq!(op.snapshot()["error"]["code"], "runtime_unavailable");
    }
}
