//! Machines: one Mother Claude across several of the user's computers
//! (docs/PLAN-MACHINES.md).
//!
//! Phase 1 — pairing and setup: this machine's identity (`identity`), the
//! paired machines and settings (`store`, `machines.json`), the peer
//! transport (`transport`, iroh over QUIC), pairing (`pairing`), the setup's
//! checks and fixes (`setup`), keeping awake while used (`awake`), and the
//! routes (`routes`: `/api/machines/*` for this machine's dashboard,
//! `/peer/*` for the other machines). Phase 3a adds the roots (`roots`: one
//! folder per machine, replicated, mapping a project between machines).
//!
//! Phase 3a — moving a session with its code: which machine each session
//! lives on (`lease`), the move itself (`moves`, both sides) and its routes
//! (`move_routes`), over the git half in `code`.
//!
//! Invariants (plan §11): a peer is a principal (`Principal::Peer`), built
//! only from the transport's authenticated key, and reaches the peer routes
//! only; nothing on a machine changes because another asked until that
//! machine's desktop confirms it; the transport runs only while Machines is
//! in use; while Machines is off on a machine, the others may only pair and
//! run the setup with it ([`serving`], [`using`]); one writer per session
//! across machines (the lease); a config directory is written only by a
//! move's landing, as a pull writes it.

pub mod acting;
pub mod awake;
pub mod code;
pub mod federation;
pub mod grants;
pub mod identity;
pub mod ids;
pub mod lease;
pub mod move_routes;
pub mod moves;
pub mod pairing;
pub mod protocol;
pub mod roots;
pub mod routes;
pub mod setup;
pub mod store;
pub mod transport;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, RwLock, Weak};
use std::time::{Duration, Instant};

use iroh::address_lookup::MemoryLookup;
use iroh::endpoint::Connection;
use iroh::EndpointAddr;
use serde::Serialize;
use serde_json::{json, Value};

use self::grants::{Grant, Grants};
use self::identity::Identity;
use self::protocol::{Hello, MachineInfo, Versions};
use self::store::{MachinesFile, PeerRecord, Settings};
use self::transport::PathKind;
use crate::server::devices::audit::AuditEntry;
use crate::server::error::ApiError;
use crate::server::ws::ServerEvent;
use crate::state::{AppState, Inner};

/// A pairing rendezvous seen on the local network (the join dialog's
/// *Nearby* list).
#[derive(Clone, Debug)]
pub struct Nearby {
    pub id: String,
    pub name: String,
    pub os: String,
    pub addr: EndpointAddr,
    pub seen: Instant,
}

/// How long a nearby rendezvous stays listed after it was last heard.
const NEARBY_TTL: Duration = Duration::from_secs(45);

/// What this machine knows of a paired machine's presence.
#[derive(Clone, Debug, Default)]
pub struct Link {
    pub online: bool,
    /// The last answered `hello` (ms).
    pub last_ok: Option<i64>,
    pub path: Option<PathKind>,
    pub rtt_ms: Option<f64>,
    pub error: Option<String>,
}

/// A paired machine is offline once it has not answered for this long.
const OFFLINE_AFTER_MS: i64 = 60_000;
/// How often each paired machine is greeted.
const HELLO_EVERY: Duration = Duration::from_secs(20);

/// Another machine's suggestion for the shared settings (plan §5.3): applied
/// only when this machine's desktop accepts it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsOffer {
    pub from: String,
    pub from_name: String,
    pub relay: store::RelaySetting,
    /// Its code options, when it sent them (an older build does not).
    pub code: Option<store::CodeSettings>,
    pub at: i64,
}

/// Everything Machines keeps at run time.
pub struct MachinesState {
    data_dir: Option<PathBuf>,
    file: RwLock<MachinesFile>,
    save: Mutex<()>,
    /// The last time `lastSeen` changes were saved.
    seen_saved: Mutex<Option<Instant>>,
    ident: OnceLock<Identity>,
    pub(crate) net: tokio::sync::Mutex<Option<transport::Net>>,
    pub(crate) pairing: Mutex<Option<pairing::Session>>,
    pub(crate) generation: AtomicU64,
    conns: Mutex<HashMap<String, Connection>>,
    links: Mutex<HashMap<String, Link>>,
    nearby: Mutex<HashMap<String, Nearby>>,
    pub(crate) setup: Mutex<setup::Cache>,
    offer: Mutex<Option<SettingsOffer>>,
    pub(crate) awake: awake::KeepAwake,
    peer_app: OnceLock<axum::Router>,
    /// The dashboard's own `/api` router, serving allow-listed peer calls.
    api_app: OnceLock<axum::Router>,
    /// Other machines' sessions, as their links report them.
    pub(crate) federation: federation::Federation,
    /// The last exchange of roots with each machine.
    pub(crate) roots_exchanges: roots::Exchanges,
    test_lookup: Mutex<Option<MemoryLookup>>,
    /// Fixes run for real (UAC, System Settings) only once the app turns
    /// this on; tests only record them.
    pub(crate) fixes_live: AtomicBool,
    pub(crate) fixes_asked: Mutex<Vec<String>>,
    /// Tests: this machine is "switched off" (`transport::go_offline`):
    /// nothing starts its endpoint until `transport::come_back`.
    pub(crate) held_down: AtomicBool,
    /// Which machine each session lives on (`machines/leases.json`).
    pub(crate) leases: lease::Leases,
    /// Moves of sessions to and from this machine under way, and their test
    /// hooks.
    pub(crate) moves: moves::Moves,
}

impl std::fmt::Debug for MachinesState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MachinesState").finish_non_exhaustive()
    }
}

impl MachinesState {
    pub fn load(data_dir: Option<&Path>) -> Self {
        Self {
            data_dir: data_dir.map(Path::to_path_buf),
            file: RwLock::new(MachinesFile::load(data_dir)),
            save: Mutex::new(()),
            seen_saved: Mutex::new(None),
            ident: OnceLock::new(),
            net: tokio::sync::Mutex::new(None),
            pairing: Mutex::new(None),
            generation: AtomicU64::new(0),
            conns: Mutex::new(HashMap::new()),
            links: Mutex::new(HashMap::new()),
            nearby: Mutex::new(HashMap::new()),
            setup: Mutex::new(setup::Cache::load(data_dir)),
            offer: Mutex::new(None),
            awake: awake::KeepAwake::default(),
            peer_app: OnceLock::new(),
            api_app: OnceLock::new(),
            federation: federation::Federation::default(),
            roots_exchanges: roots::Exchanges::default(),
            test_lookup: Mutex::new(None),
            fixes_live: AtomicBool::new(false),
            fixes_asked: Mutex::new(Vec::new()),
            held_down: AtomicBool::new(false),
            leases: lease::Leases::load(data_dir),
            moves: moves::Moves::default(),
        }
    }

    /// Reads the store.
    pub fn store_read<R>(&self, f: impl FnOnce(&MachinesFile) -> R) -> R {
        f(&self.file.read().unwrap_or_else(|e| e.into_inner()))
    }

    /// Changes the store and saves it.
    pub fn store_write<R>(&self, f: impl FnOnce(&mut MachinesFile) -> R) -> R {
        let _g = self.save.lock().unwrap_or_else(|e| e.into_inner());
        let (out, snapshot) = {
            let mut file = self.file.write().unwrap_or_else(|e| e.into_inner());
            let out = f(&mut file);
            (out, file.clone())
        };
        if let Some(dir) = &self.data_dir {
            if let Err(e) = snapshot.save(dir) {
                tracing::warn!(error = %e, "machines.json could not be saved");
            }
        }
        out
    }

    pub fn settings(&self) -> Settings {
        self.store_read(|f| f.settings.clone())
    }

    pub fn enabled(&self) -> bool {
        self.store_read(|f| f.enabled)
    }

    /// Whether `machine` holds `grant` for a request on the local network
    /// (`lan`) or not.
    pub fn allows(&self, machine: &str, grant: Grant, lan: bool) -> bool {
        self.store_read(|f| f.peer(machine).is_some_and(|p| p.grants.allows(grant, lan)))
    }

    /// The key pairing with this machine right now (it may reach
    /// `/peer/pair/*`).
    pub fn pairing_admits(&self, key: &str) -> bool {
        let guard = self.pairing.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().is_some_and(|s| {
            s.role == pairing::Role::Host && !s.is_over() && s.finder.as_deref() == Some(key)
        })
    }

    pub(crate) fn remember_connection(&self, key: &str, conn: &Connection) {
        let mut conns = self.conns.lock().unwrap_or_else(|e| e.into_inner());
        conns.retain(|_, c| c.close_reason().is_none());
        conns.insert(key.to_string(), conn.clone());
    }

    pub(crate) fn live_connection(&self, key: &str) -> Option<Connection> {
        let conns = self.conns.lock().unwrap_or_else(|e| e.into_inner());
        conns
            .get(key)
            .filter(|c| c.close_reason().is_none())
            .cloned()
    }

    pub(crate) fn drop_connection(&self, key: &str) {
        let removed = self
            .conns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key);
        if let Some(c) = removed {
            c.close(2u32.into(), b"closed");
        }
    }

    pub(crate) fn forget_connections(&self) {
        let all: Vec<Connection> = self
            .conns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
            .map(|(_, c)| c)
            .collect();
        for c in all {
            c.close(0u32.into(), b"stopping");
        }
    }

    /// Whether any paired machine is connected now.
    pub fn any_connected(&self) -> bool {
        let conns = self.conns.lock().unwrap_or_else(|e| e.into_inner());
        let keys: Vec<String> = conns
            .iter()
            .filter(|(_, c)| c.close_reason().is_none())
            .map(|(k, _)| k.clone())
            .collect();
        drop(conns);
        self.store_read(|f| keys.iter().any(|k| f.peer_by_key(k).is_some()))
    }

    pub(crate) fn peer_app(&self, state: &AppState) -> axum::Router {
        self.peer_app
            .get_or_init(|| routes::peer_app(state.clone()))
            .clone()
    }

    pub(crate) fn api_app(&self, state: &AppState) -> axum::Router {
        self.api_app
            .get_or_init(|| crate::server::routes::api_router().with_state(state.clone()))
            .clone()
    }

    pub(crate) fn note_nearby(&self, n: Nearby) {
        self.nearby
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(n.id.clone(), n);
    }

    pub(crate) fn forget_nearby(&self, id: &str) {
        self.nearby
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }

    pub(crate) fn nearby_addr(&self, id: &str) -> Option<EndpointAddr> {
        let nearby = self.nearby.lock().unwrap_or_else(|e| e.into_inner());
        nearby
            .get(id)
            .filter(|n| n.seen.elapsed() < NEARBY_TTL)
            .map(|n| n.addr.clone())
    }

    /// The pairing rendezvous heard on the local network lately.
    pub fn nearby(&self) -> Vec<Nearby> {
        let mut nearby = self.nearby.lock().unwrap_or_else(|e| e.into_inner());
        nearby.retain(|_, n| n.seen.elapsed() < NEARBY_TTL);
        let mut out: Vec<Nearby> = nearby.values().cloned().collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub(crate) fn link(&self, machine: &str) -> Link {
        self.links
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(machine)
            .cloned()
            .unwrap_or_default()
    }

    fn set_link(&self, machine: &str, f: impl FnOnce(&mut Link)) {
        let mut links = self.links.lock().unwrap_or_else(|e| e.into_inner());
        f(links.entry(machine.to_string()).or_default());
    }

    pub fn offer(&self) -> Option<SettingsOffer> {
        self.offer.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub(crate) fn set_offer(&self, offer: Option<SettingsOffer>) {
        *self.offer.lock().unwrap_or_else(|e| e.into_inner()) = offer;
    }

    pub(crate) fn test_lookup(&self) -> Option<MemoryLookup> {
        self.test_lookup
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Tests: find the other machines of this process through `lookup`
    /// instead of mDNS and n0's DNS (and use no relay).
    #[doc(hidden)]
    pub fn use_test_lookup(&self, lookup: MemoryLookup) {
        *self.test_lookup.lock().unwrap_or_else(|e| e.into_inner()) = Some(lookup);
        self.store_write(|f| {
            f.settings.relay = store::RelaySetting {
                mode: store::RelayMode::None,
                url: None,
            }
        });
    }

    /// The app turns this on; until then a fix only records what it would do.
    pub fn enable_fixes(&self) {
        self.fixes_live.store(true, Ordering::Relaxed);
    }

    /// The fixes asked for while they only record (tests).
    #[doc(hidden)]
    pub fn recorded_fixes(&self) -> Vec<String> {
        self.fixes_asked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// The app turns this on: a move resumes its session with `claude`;
    /// until then (tests) the resume is only recorded.
    pub fn enable_resumes(&self) {
        self.moves.enable_resumes();
    }

    /// The resumes moves asked for while they only record (tests): what
    /// each would have launched.
    #[doc(hidden)]
    pub fn recorded_resumes(&self) -> Vec<Value> {
        self.moves.recorded_resumes()
    }

    /// Tests: moves run git without this machine's configuration.
    #[doc(hidden)]
    pub fn use_isolated_git(&self) {
        self.moves.use_git(code::GitRunner::new().isolated());
    }

    /// Tests: a move fails at the named step (`freeze`, `package`, `send`,
    /// `land` — this machine's half — `commit`, `resume`), once.
    #[doc(hidden)]
    pub fn fail_move_at(&self, step: Option<&str>) {
        self.moves.fail_at(step);
    }
}

// ─── shared helpers ────────────────────────────────────────────────────────────

/// This machine's identity (made on first use).
pub fn identity(state: &AppState) -> Result<Identity, String> {
    if let Some(i) = state.machines.ident.get() {
        return Ok(i.clone());
    }
    let ident = identity::load(state)?;
    Ok(state.machines.ident.get_or_init(|| ident).clone())
}

/// This machine's id, without making a machine key: the identity's when it
/// is loaded, else the per-install id the backups keep. `None` without a
/// data dir that can hold one.
pub fn self_id(state: &AppState) -> Option<String> {
    if let Some(i) = state.machines.ident.get() {
        return Some(i.id.clone());
    }
    crate::claude::backup::ops::this_machine(state)
        .ok()
        .map(|m| m.id)
        .filter(|id| ids::is_machine_id(id))
}

/// The name this machine shows.
pub fn this_name(state: &AppState) -> String {
    state
        .machines
        .store_read(|f| f.name.clone())
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(crate::claude::backup::host_name)
}

/// Who this machine is, as it tells another.
pub fn machine_info(state: &AppState) -> MachineInfo {
    let (id, key) = match identity(state) {
        Ok(i) => (i.id.clone(), i.key_hex()),
        Err(_) => (String::new(), String::new()),
    };
    MachineInfo {
        id,
        name: this_name(state),
        os: protocol::this_os().to_string(),
        os_version: identity::os_version(),
        versions: Versions::this_build(setup::claude_version(state)),
        key,
    }
}

/// Machines changed: dashboards refetch (`{t:"machines", what}`).
pub fn publish(state: &AppState, what: &str) {
    state.broadcast(ServerEvent::Machines {
        what: what.to_string(),
    });
}

/// An audit line for a Machines action.
pub fn audit(state: &AppState, action: &str, machine: &str, name: &str) {
    state.devices.audit.record(AuditEntry {
        action: action.to_string(),
        device_id: Some(machine.to_string()),
        device_name: Some(name.to_string()),
        ..AuditEntry::now()
    });
}

/// Stores a newly paired machine (replacing an earlier pairing of the same
/// machine or key).
pub fn add_peer(
    state: &AppState,
    info: &MachineInfo,
    grants: Grants,
    their: Option<Grants>,
) -> Result<(), ApiError> {
    if !ids::is_machine_id(&info.id) || !ids::is_key(&info.key) {
        return Err(ApiError::unprocessable(
            "pair_bad_request",
            "The other computer sent a malformed id.",
        ));
    }
    let own = identity(state).map_err(ApiError::internal)?;
    if info.id == own.id || info.key == own.key_hex() {
        return Err(ApiError::unprocessable(
            "pair_self",
            "A computer cannot pair with itself.",
        ));
    }
    let record = pairing::peer_record(info, grants.normalized(), their);
    state.machines.store_write(|f| {
        f.peers.retain(|p| p.id != record.id && p.key != record.key);
        f.peers.push(record);
    });
    state.machines.set_link(&info.id, |l| {
        l.online = true;
        l.last_ok = Some(now());
    });
    publish(state, "peers");
    Ok(())
}

/// Forgets a paired machine and cuts its connections.
pub fn revoke(state: &AppState, machine: &str) -> bool {
    let removed = state.machines.store_write(|f| {
        let before = f.peers.len();
        let key = f
            .peer(machine)
            .map(|p| (p.key.clone(), p.display_name().to_string()));
        f.peers.retain(|p| p.id != machine);
        (before != f.peers.len()).then_some(key).flatten()
    });
    let Some((key, name)) = removed else {
        return false;
    };
    state.machines.drop_connection(&key);
    state
        .machines
        .links
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(machine);
    federation::forget(state, machine);
    audit(state, "machine.revoked", machine, &name);
    publish(state, "peers");
    true
}

/// A name another machine gave itself, made safe to show: no control
/// characters, at most 64 of them, never empty.
pub fn clean_peer_name(name: &str) -> String {
    let cleaned: String = name.chars().filter(|c| !c.is_control()).take(64).collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "Computer".to_string()
    } else {
        cleaned.to_string()
    }
}

/// A paired machine's name as the user sees it (their alias, else its own
/// name), for audit lines, notices and who answered an approval.
pub fn peer_name(state: &AppState, machine: &str) -> String {
    state
        .machines
        .store_read(|f| f.peer(machine).map(|p| p.display_name().to_string()))
        .unwrap_or_else(|| machine.chars().take(8).collect())
}

pub(crate) fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ─── on and off ──────────────────────────────────────────────────────────────────

/// Turns Machines on or off on this computer (`POST /api/machines/enable`
/// checks the setup's blockers first). On, the links to the other computers'
/// sessions start at once; off, they close, and so do the other computers'
/// links to this one's: while off, another computer may only pair and run
/// the setup with this one ([`serving`]), and this one uses none ([`using`]).
pub fn set_enabled(state: &AppState, on: bool) {
    let was = state.machines.store_write(|f| {
        let was = f.enabled;
        f.enabled = on;
        if on {
            f.setup_step = None;
        }
        was
    });
    if was != on {
        if on {
            federation::wake(state);
        } else {
            federation::stop(state);
            state.hub.close_peers(
                crate::server::ws::CLOSE_FORBIDDEN,
                "Machines is off on this computer",
            );
        }
    }
    publish(state, "enabled");
}

/// What another computer may use of this one: while Machines is off here,
/// only what pairing and the setup need (hello, pairing, the setup's checks,
/// asks and test, settings offers, roots, leases). Its session stream
/// (`/ws`), the session routes (`/api/*`) and moves (`/peer/moves/*`) answer
/// `403 machines_off`.
pub(crate) fn serving(state: &AppState) -> Result<(), ApiError> {
    if state.machines.enabled() {
        return Ok(());
    }
    Err(ApiError::forbidden(
        "machines_off",
        format!("Machines is off on {}.", this_name(state)),
    ))
}

/// Whether this computer may use another one: while Machines is off here,
/// the proxy and starting a move answer `409 machines_off`.
pub(crate) fn using(state: &AppState) -> Result<(), ApiError> {
    if state.machines.enabled() {
        return Ok(());
    }
    Err(ApiError::conflict(
        "machines_off",
        "Machines is off on this computer: turn it on on the Machines page first.",
    ))
}

// ─── presence ──────────────────────────────────────────────────────────────────

/// Greets every paired machine every 20 s while the transport runs: who is
/// online, by which path, and what each now says about itself.
pub(crate) async fn presence_loop(state: Weak<Inner>) {
    let mut tick = tokio::time::interval(HELLO_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let Some(state) = state.upgrade() else { break };
        let peers: Vec<(String, String)> = state.machines.store_read(|f| {
            f.peers
                .iter()
                .map(|p| (p.id.clone(), p.key.clone()))
                .collect()
        });
        let greetings = peers
            .into_iter()
            .map(|(id, key)| greet(state.clone(), id, key));
        futures_util::future::join_all(greetings).await;
        awake::update(&state);
    }
}

/// The hello this machine sends `peer` (and answers its hello with): its
/// leases only to a machine it lets see its sessions.
pub(crate) fn hello_for(state: &AppState, peer: &PeerRecord) -> Hello {
    let sees = peer.grants.view.lan || peer.grants.view.internet;
    Hello {
        machine: machine_info(state),
        relay: state.machines.settings().relay,
        grants_you_have: peer.grants,
        capabilities: protocol::CAPABILITIES
            .iter()
            .map(|s| s.to_string())
            .collect(),
        roots_digest: Some(roots::digest(state)),
        leases: if sees {
            state.machines.leases.wire()
        } else {
            Vec::new()
        },
    }
}

/// One greeting: updates the peer's record and link.
pub async fn greet(state: AppState, machine: String, key: String) {
    let Some(peer) = state.machines.store_read(|f| f.peer(&machine).cloned()) else {
        return;
    };
    let body = serde_json::to_value(hello_for(&state, &peer)).unwrap_or(Value::Null);
    let answer = transport::request_key(
        &state,
        &key,
        "POST",
        "/peer/hello",
        Some(body),
        Duration::from_secs(10),
    )
    .await;
    let was_online = state.machines.link(&machine).online;
    match answer {
        Ok((200, v)) => match serde_json::from_value::<Hello>(v) {
            Ok(hello) if hello.machine.key == key && hello.machine.id == machine => {
                let conn = state.machines.live_connection(&key);
                learn(&state, &hello, conn.as_ref());
                let path = conn.as_ref().map(transport::path_of);
                state.machines.set_link(&machine, |l| {
                    l.online = true;
                    l.last_ok = Some(now());
                    l.path = path;
                    l.rtt_ms = conn.as_ref().and_then(transport::rtt_of);
                    l.error = None;
                });
                roots::after_hello(&state, &machine, &hello);
                // Its leases: which machine each session lives on.
                let lan = path == Some(PathKind::Lan);
                let changed = lease::accept(&state, &machine, lan, &hello.leases).await;
                moves::on_leases_changed(&state, &changed).await;
                moves::follow_up(&state, &machine).await;
            }
            _ => mark_unanswered(&state, &machine, "it answered something unexpected"),
        },
        Ok((status, _)) => mark_unanswered(&state, &machine, &format!("it answered {status}")),
        Err(e) => mark_unanswered(&state, &machine, &e),
    }
    if state.machines.link(&machine).online != was_online {
        publish(&state, "peers");
    }
}

fn mark_unanswered(state: &AppState, machine: &str, error: &str) {
    state.machines.set_link(machine, |l| {
        l.error = Some(error.to_string());
        l.online = l.last_ok.is_some_and(|t| now() - t < OFFLINE_AFTER_MS);
        if !l.online {
            l.path = None;
            l.rtt_ms = None;
        }
    });
}

/// What a hello (either way) teaches about the other machine.
pub fn learn(state: &AppState, hello: &Hello, conn: Option<&Connection>) {
    let addrs: Option<Vec<String>> = conn.map(|c| {
        transport::direct_addrs(c)
            .iter()
            .map(|a| a.to_string())
            .collect()
    });
    let m = &hello.machine;
    let save_seen = {
        let mut saved = state
            .machines
            .seen_saved
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let due = saved.is_none_or(|t| t.elapsed() > Duration::from_secs(300));
        if due {
            *saved = Some(Instant::now());
        }
        due
    };
    let changed = state.machines.store_read(|f| {
        f.peer(&m.id).is_some_and(|p| {
            p.name != clean_peer_name(&m.name)
                || p.os != m.os
                || p.os_version != m.os_version
                || p.versions.as_ref() != Some(&m.versions)
                || p.capabilities != hello.capabilities
                || p.relay.as_ref() != Some(&hello.relay)
                || p.their_grants != Some(hello.grants_you_have)
                || addrs
                    .as_ref()
                    .is_some_and(|a| !a.is_empty() && *a != p.addrs)
        })
    });
    if !changed && !save_seen {
        return;
    }
    state.machines.store_write(|f| {
        if let Some(p) = f.peer_mut(&m.id) {
            p.name = clean_peer_name(&m.name);
            p.os = m.os.clone();
            p.os_version = m.os_version.clone();
            p.versions = Some(m.versions.clone());
            p.capabilities = hello.capabilities.clone();
            p.relay = Some(hello.relay.clone());
            p.their_grants = Some(hello.grants_you_have);
            p.last_seen = Some(now());
            if let Some(a) = addrs.filter(|a| !a.is_empty()) {
                p.addrs = a;
            }
        }
    });
    if changed {
        publish(state, "peers");
    }
}

// ─── starting ──────────────────────────────────────────────────────────────────

/// Starts the transport when a machine is paired (in the background).
pub fn start(state: &AppState) {
    let paired = state.machines.store_read(|f| !f.peers.is_empty());
    let st = state.clone();
    tokio::spawn(async move {
        if paired {
            if let Err(e) = transport::ensure_started(&st).await {
                tracing::warn!(error = %e, "Machines transport did not start");
            }
        }
        // A move a restart interrupted: failed and unfrozen before it
        // landed, finished after (decision 10) — on its own, so a computer
        // that is away does not hold up the checks.
        let rec = st.clone();
        tokio::spawn(async move { moves::recover(&rec).await });
        setup::health_loop(st).await;
    });
}

// ─── the view ──────────────────────────────────────────────────────────────────

/// `GET /api/machines`.
pub async fn view(state: &AppState) -> Value {
    let me = machine_info(state);
    let file = state.machines.store_read(|f| f.clone());
    let peers: Vec<Value> = file.peers.iter().map(|p| peer_view(state, p)).collect();
    let pairing = state
        .machines
        .pairing
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(pairing::Session::view);
    let endpoint = transport::current(state).await;
    let relay = endpoint.as_ref().and_then(|ep| {
        use iroh::Watcher;
        let statuses = ep.home_relay_status().get();
        statuses
            .into_iter()
            .next()
            .map(|s| json!({ "url": s.url().to_string(), "connected": s.is_connected() }))
    });
    json!({
        "enabled": file.enabled,
        "self": {
            "id": me.id,
            "name": me.name,
            "os": me.os,
            "osVersion": me.os_version,
            "versions": me.versions,
            "key": ids::fingerprint(&me.key),
        },
        "peers": peers,
        "pairing": pairing,
        "settings": settings_view(&file.settings),
        "offer": state.machines.offer(),
        "transport": { "running": endpoint.is_some(), "relay": relay },
        "health": setup::health_summary(state),
        "setupStep": file.setup_step,
        // What other computers asked this one to do (`setup::asks`).
        "asks": setup::asks::view(state),
    })
}

fn peer_view(state: &AppState, p: &PeerRecord) -> Value {
    let link = state.machines.link(&p.id);
    let problem = p.versions.as_ref().and_then(|v| {
        if protocol::compatible(v.protocol) {
            None
        } else if v.protocol < protocol::PROTOCOL {
            Some("olderApp")
        } else {
            Some("newerApp")
        }
    });
    json!({
        "id": p.id,
        "name": p.display_name(),
        "os": p.os,
        "osVersion": p.os_version,
        "versions": p.versions,
        "key": ids::fingerprint(&p.key),
        "pairedAt": p.paired_at,
        "lastSeen": link.last_ok.or(p.last_seen),
        "online": link.online,
        "path": link.path,
        "rttMs": link.rtt_ms,
        "grants": p.grants,
        "theirGrants": p.their_grants,
        "capabilities": p.capabilities,
        "relay": p.relay,
        "problem": problem,
        "sessions": state.machines.federation.counts(&p.id).map(|(total, running)| json!({ "total": total, "running": running })),
    })
}

pub fn settings_view(s: &Settings) -> Value {
    json!({ "relay": s.relay, "keepAwake": s.keep_awake, "code": s.code })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_state_is_off_with_no_peers() {
        let m = MachinesState::load(None);
        assert!(!m.enabled());
        assert!(m.store_read(|f| f.peers.is_empty()));
        assert!(!m.any_connected());
        assert!(m.nearby().is_empty());
        assert!(!m.pairing_admits(&"a".repeat(64)));
    }
}
