//! Conversations of the Claude API, and the clients they belong to.
//!
//! **Clients.** An API server can tell its clients apart (`groupBy`): by
//! their IP address, the API key they send, `metadata.user_id`, or an
//! `X-Client-Id` header — or not at all (`default`). Each client has a folder
//! under the server's data folder (`dataDir`; else `<app data>/claude-api`):
//! its conversations run there (so Claude Code keeps their transcripts per
//! client too), and Mother Claude keeps there what it knows of each
//! (`client.json`, `conversations/<id>.json`, 0600, written atomically).
//!
//! **Conversations.** One conversation is one Claude Code session (its id),
//! remembered by the fingerprints of the history a client sent and the
//! answer it got: a request whose history is a conversation's continues it.
//! A client may also name one (`x-conversation-id`, `conversation_id`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::stream::ToolCall;
use crate::bridge::ops::Operation;

/// How a server tells its clients apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupBy {
    None,
    Ip,
    Key,
    User,
    Header,
}

impl GroupBy {
    pub fn parse(s: Option<&str>) -> Self {
        match s.unwrap_or("none") {
            "ip" => Self::Ip,
            "key" => Self::Key,
            "user" => Self::User,
            "header" => Self::Header,
            _ => Self::None,
        }
    }
}

pub const GROUP_BY: [&str; 5] = ["none", "ip", "key", "user", "header"];

/// A client as a folder name: letters, digits, `.`, `-`, `_` (64 at most),
/// valid on every system.
pub fn folder_label(raw: &str) -> String {
    let mut out: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .take(64)
        .collect();
    while out.starts_with('.') {
        out.remove(0);
    }
    while out.ends_with('.') {
        out.pop();
    }
    if out.is_empty() {
        out = "client".into();
    }
    // Windows keeps these names for devices.
    let upper = out.to_ascii_uppercase();
    let base = upper.split('.').next().unwrap_or("");
    if ["CON", "PRN", "AUX", "NUL"].contains(&base)
        || ((base.starts_with("COM") || base.starts_with("LPT")) && base.len() == 4)
    {
        out.insert_str(0, "client-");
    }
    out
}

/// An IP address as a folder name: IPv4 as written, IPv6 `ip6-…`.
pub fn ip_label(ip: std::net::IpAddr) -> String {
    match ip {
        std::net::IpAddr::V4(v4) => v4.to_string(),
        std::net::IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None if v6.is_loopback() => "127.0.0.1".into(),
            None => format!("ip6-{}", v6.to_string().replace(':', "-")),
        },
    }
}

/// Who a request is from, as the server groups clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Client {
    /// Its folder's name.
    pub id: String,
    /// `default`, `ip`, `key`, `user`, `header`.
    pub kind: String,
    /// How the dashboard names it.
    pub label: String,
}

/// What the dashboard and the client see of a conversation.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
}

impl Usage {
    pub fn add(&mut self, u: &Value) {
        let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
        self.input_tokens += n("input_tokens");
        self.output_tokens += n("output_tokens");
        self.cache_read_input_tokens += n("cache_read_input_tokens");
        self.cache_creation_input_tokens += n("cache_creation_input_tokens");
    }
}

/// A conversation as Mother Claude keeps it (`conversations/<id>.json`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Record {
    pub id: String,
    pub client: String,
    pub client_kind: String,
    pub server: String,
    pub account: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub model: String,
    /// The fingerprint of what the conversation's process was started with
    /// (system prompt, tools, thinking, output format, effort, max_tokens).
    pub settings: String,
    pub title: Option<String>,
    /// One fingerprint per message of the history, the last answer included.
    pub history: Vec<String>,
    /// Begun with a history this server never saw (written into its first
    /// message).
    pub seeded: bool,
    /// The conversation it was forked from (other settings, same history).
    pub forked_from: Option<String>,
    /// The model that last answered, as the API named it.
    pub model_used: Option<String>,
    pub requests: u64,
    pub usage: Usage,
    /// The exchange as the client sees it (the bytes of images and documents
    /// left out).
    pub messages: Vec<Value>,
}

/// A conversation now: its record and what runs.
#[derive(Debug)]
pub struct Conv {
    pub record: Record,
    /// The client's folder: where it runs.
    pub dir: PathBuf,
    /// Answering a request now.
    pub busy: bool,
    /// Its answer is with the client and recorded; the turn is still ending.
    /// A request that continues it waits for that, rather than start another.
    pub settling: bool,
    /// Its Claude Code session is open in the host.
    pub open: bool,
    /// A turn waiting on the client's tool results, and how far its events
    /// were read.
    pub op: Option<Arc<Operation>>,
    pub cursor: u64,
    /// The tool calls the client was handed and owes results for.
    pub waiting: Vec<ToolCall>,
    pub last_used: Instant,
}

/// A client tool call the host is waiting on.
#[derive(Debug, Clone)]
pub struct HostCall {
    pub request_id: String,
    pub name: String,
    pub input: Value,
}

/// Every conversation, loaded per client from its folder on first use.
#[derive(Debug, Default)]
pub struct Store {
    /// `<app data>/claude-api`, for servers without a data folder of their
    /// own.
    pub default_root: Option<PathBuf>,
    convs: Mutex<HashMap<String, Conv>>,
    /// Clients whose folder was read (`root/client`).
    loaded: Mutex<std::collections::HashSet<PathBuf>>,
    /// Client tool calls the host raised, by conversation.
    calls: Mutex<HashMap<String, Vec<HostCall>>>,
    pub calls_changed: tokio::sync::Notify,
    /// A conversation was freed (a request waiting on one that was settling
    /// looks again).
    pub released: tokio::sync::Notify,
    /// The sweeper (idle processes closed, tool calls nobody answers given
    /// up) runs.
    pub sweeping: std::sync::atomic::AtomicBool,
}

/// What a request's history finds among a client's conversations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    /// One with this history and these settings, now claimed (busy).
    Claimed(String),
    /// One that ends with this history, still ending its last turn: wait
    /// for it ([`Store::released`]) and look again.
    Settling(String),
    /// One with this history but other settings (system prompt, tools, …):
    /// a new conversation forks it.
    Fork(String),
    /// None.
    Nothing,
}

/// Why a conversation could not be claimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Busy {
    /// Answering another request.
    Answering,
    /// Its last turn is ending: free in a moment.
    Settling,
    /// No such conversation for this client.
    Missing,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Writes a file atomically, readable by the owner only.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4().simple()));
    std::fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// A client's own record (`client.json`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ClientRecord {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub first_seen: i64,
    pub last_seen: i64,
    pub requests: u64,
}

impl Store {
    pub fn new(default_root: Option<PathBuf>) -> Self {
        Self {
            default_root,
            ..Self::default()
        }
    }

    /// Reads a client's conversations from its folder, once.
    fn load_client(&self, client_dir: &Path) {
        if !lock(&self.loaded).insert(client_dir.to_path_buf()) {
            return;
        }
        let Ok(entries) = std::fs::read_dir(client_dir.join("conversations")) else {
            return;
        };
        let mut convs = lock(&self.convs);
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let Some(record) = std::fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Record>(&b).ok())
                .filter(|r| crate::claude::ids::is_uuid(&r.id))
            else {
                continue;
            };
            convs.entry(record.id.clone()).or_insert_with(|| Conv {
                record,
                dir: client_dir.to_path_buf(),
                busy: false,
                settling: false,
                open: false,
                op: None,
                cursor: 0,
                waiting: Vec::new(),
                last_used: Instant::now(),
            });
        }
    }

    /// The client's folder, made when missing (0700).
    pub fn client_dir(root: &Path, client: &Client) -> std::io::Result<PathBuf> {
        let dir = root.join(&client.id);
        std::fs::create_dir_all(dir.join("conversations"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
        Ok(dir)
    }

    /// What `history` finds among this client's conversations (read from
    /// its folder on first use). An empty history finds nothing: a request
    /// with one message begins a conversation.
    pub fn lookup(&self, client_dir: &Path, settings: &str, history: &[String]) -> Lookup {
        if history.is_empty() {
            return Lookup::Nothing;
        }
        self.load_client(client_dir);
        let mut convs = lock(&self.convs);
        let same: Vec<&Conv> = convs
            .values()
            .filter(|c| c.dir == client_dir && c.record.history == history)
            .collect();
        let free = |c: &&Conv| !c.busy;
        let newest = |list: Vec<&Conv>| {
            list.into_iter()
                .max_by_key(|c| c.record.updated_at)
                .map(|c| c.record.id.clone())
        };
        let exact = newest(
            same.iter()
                .copied()
                .filter(|c| c.record.settings == settings)
                .filter(free)
                .collect(),
        );
        if let Some(id) = exact {
            if let Some(c) = convs.get_mut(&id) {
                c.busy = true;
                c.last_used = Instant::now();
            }
            return Lookup::Claimed(id);
        }
        if let Some(id) = newest(same.iter().copied().filter(|c| c.settling).collect()) {
            return Lookup::Settling(id);
        }
        match newest(same.into_iter().filter(free).collect()) {
            Some(id) => Lookup::Fork(id),
            None => Lookup::Nothing,
        }
    }

    /// A conversation by id, when it is this client's: claimed (busy).
    pub fn claim(&self, client_dir: &Path, id: &str) -> Result<(), Busy> {
        self.load_client(client_dir);
        let mut convs = lock(&self.convs);
        match convs.get_mut(id).filter(|c| c.dir == client_dir) {
            Some(c) if c.settling => Err(Busy::Settling),
            Some(c) if c.busy => Err(Busy::Answering),
            Some(c) => {
                c.busy = true;
                c.last_used = Instant::now();
                Ok(())
            }
            None => Err(Busy::Missing),
        }
    }

    /// A new conversation, claimed.
    pub fn insert(&self, record: Record, dir: PathBuf) {
        lock(&self.convs).insert(
            record.id.clone(),
            Conv {
                record,
                dir,
                busy: true,
                settling: false,
                open: false,
                op: None,
                cursor: 0,
                waiting: Vec::new(),
                last_used: Instant::now(),
            },
        );
    }

    /// Reads or changes a conversation.
    pub fn with<R>(&self, id: &str, f: impl FnOnce(&mut Conv) -> R) -> Option<R> {
        lock(&self.convs).get_mut(id).map(f)
    }

    /// The request is answered: free for the next, written to its folder.
    pub fn release(&self, id: &str) {
        let saved = self.with(id, |c| {
            c.busy = false;
            c.settling = false;
            c.last_used = Instant::now();
            (c.dir.clone(), c.record.clone())
        });
        if let Some((dir, record)) = saved {
            // A conversation that never answered is not kept.
            if !record.history.is_empty() {
                Self::save(&dir, &record);
            }
        }
        self.released.notify_waiters();
    }

    fn save(dir: &Path, record: &Record) {
        let path = dir
            .join("conversations")
            .join(format!("{}.json", record.id));
        match serde_json::to_vec_pretty(record) {
            Ok(bytes) => {
                if let Err(e) = write_private(&path, &bytes) {
                    tracing::warn!(error = %e, id = %record.id, "could not save an API conversation");
                }
            }
            Err(e) => tracing::warn!(error = %e, "could not serialise an API conversation"),
        }
    }

    /// Counts a request of the client (`client.json`).
    pub fn note_client(dir: &Path, client: &Client, now: i64) {
        let path = dir.join("client.json");
        let mut rec: ClientRecord = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        if rec.first_seen == 0 {
            rec.first_seen = now;
        }
        rec.id = client.id.clone();
        rec.kind = client.kind.clone();
        rec.label = client.label.clone();
        rec.last_seen = now;
        rec.requests += 1;
        if let Ok(bytes) = serde_json::to_vec_pretty(&rec) {
            let _ = write_private(&path, &bytes);
        }
    }

    /// Forgets a conversation (its record file removed); its open session
    /// is the caller's to close. Whether it was there.
    pub fn forget(&self, id: &str) -> Option<Conv> {
        lock(&self.calls).remove(id);
        let conv = lock(&self.convs).remove(id)?;
        let _ = std::fs::remove_file(conv.dir.join("conversations").join(format!("{id}.json")));
        self.released.notify_waiters();
        Some(conv)
    }

    /// The conversations whose session is open, least recently used first.
    pub fn open_ones(&self) -> Vec<(String, bool)> {
        let convs = lock(&self.convs);
        let mut list: Vec<&Conv> = convs.values().filter(|c| c.open).collect();
        list.sort_by_key(|c| c.last_used);
        list.into_iter()
            .map(|c| (c.record.id.clone(), c.busy || !c.waiting.is_empty()))
            .collect()
    }

    /// Every conversation under `root` this process knows (loaded clients).
    pub fn ids_under(&self, root: &Path) -> Vec<String> {
        lock(&self.convs)
            .values()
            .filter(|c| c.dir.starts_with(root))
            .map(|c| c.record.id.clone())
            .collect()
    }

    /// Reads a client's folder again on next use (after its files changed).
    pub fn unload(&self, client_dir: &Path) {
        lock(&self.loaded).remove(client_dir);
    }

    /// The conversations of a client folder (read from disk when needed),
    /// newest first.
    pub fn records_in(&self, client_dir: &Path) -> Vec<Record> {
        self.load_client(client_dir);
        let mut out: Vec<Record> = lock(&self.convs)
            .values()
            .filter(|c| c.dir == client_dir)
            .map(|c| c.record.clone())
            .collect();
        out.sort_by_key(|r| std::cmp::Reverse(r.updated_at));
        out
    }

    /// Whether a conversation is answering a request now, or waits on the
    /// client's tool results.
    pub fn state_of(&self, id: &str) -> Option<(bool, bool)> {
        self.with(id, |c| (c.busy, !c.waiting.is_empty()))
    }

    /// Conversations whose session is open and that have been idle (not
    /// busy, not waiting on tools) since before `idle_since`.
    pub fn idle_open(&self, idle_since: Instant) -> Vec<String> {
        lock(&self.convs)
            .values()
            .filter(|c| c.open && !c.busy && c.waiting.is_empty() && c.last_used < idle_since)
            .map(|c| c.record.id.clone())
            .collect()
    }

    /// Conversations that have waited on the client's tool results since
    /// before `since`.
    pub fn stale_waiting(&self, since: Instant) -> Vec<String> {
        lock(&self.convs)
            .values()
            .filter(|c| !c.busy && !c.waiting.is_empty() && c.last_used < since)
            .map(|c| c.record.id.clone())
            .collect()
    }

    // ── client tool calls the host raised ──

    pub fn note_host_call(&self, conversation: &str, call: HostCall) {
        lock(&self.calls)
            .entry(conversation.to_string())
            .or_default()
            .push(call);
        self.calls_changed.notify_waiters();
    }

    /// Takes the host's call for a tool call handed to the client: the first
    /// with its name and input.
    pub fn take_host_call(
        &self,
        conversation: &str,
        name: &str,
        input: &Value,
    ) -> Option<HostCall> {
        let mut calls = lock(&self.calls);
        let list = calls.get_mut(conversation)?;
        let at = list
            .iter()
            .position(|c| c.name == name && &c.input == input)
            .or_else(|| list.iter().position(|c| c.name == name))?;
        Some(list.remove(at))
    }

    /// Every call the host waits on for a conversation (to cancel them).
    pub fn drain_host_calls(&self, conversation: &str) -> Vec<HostCall> {
        lock(&self.calls).remove(conversation).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests;
