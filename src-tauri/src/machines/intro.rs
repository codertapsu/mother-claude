//! Introductions (docs/PLAN-MACHINES.md §5.12, Phase 5): pairing a third
//! computer with the ones you have, without a code.
//!
//! You pair C with A while A is paired with B; A offers *Also pair C with
//! B?*. When you say yes, A **introduces** them over its own trusted links:
//! it tells B who C is and C who B is — their machine ids, names, keys
//! (their public endpoint ids) and last addresses, never a secret
//! (`POST /peer/intro`, A's `view` grant on each). That alone pairs
//! nothing:
//!
//! * B and C each show the introduction (a desktop notice, the Machines
//!   page) and each desktop accepts or declines it (`POST
//!   /api/machines/introductions/{id}`);
//! * once both accepted, they run the normal pairing exchange directly
//!   (`pairing`): C dials B **by the key A named** — QUIC proves it — and
//!   the commit-then-reveal gives the six digits, computed from both keys,
//!   both nonces and the introduction; both screens show them, B types
//!   them when C came over the internet, and each desktop confirms and
//!   chooses what the other may do on it;
//! * only then does each store the other.
//!
//! Nothing is trusted transitively: A vouches for nothing, it only says
//! whom to dial. A key an introduction names may reach this computer's
//! pairing routes (`/peer/pair/*`) while the introduction is open, as the
//! key pairing by code may — nothing else — and only that key: a computer
//! dialling with another key is refused at the door. An introduction from a
//! computer that isn't paired never arrives (the door), and one naming
//! this computer, or its introducer, is refused.

use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Mutex;
use std::time::Duration;

use iroh::{EndpointAddr, PublicKey, RelayUrl};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::protocol::{ALPN_PEER, INTRODUCTIONS};
use super::store::{PeerRecord, RelayMode};
use super::{ids, transport, versions};
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Where an introducer offers an introduction.
pub const OFFER_PATH: &str = "/peer/intro";
/// Where it takes one back (the other side could not be reached).
pub const WITHDRAW_PATH: &str = "/peer/intro/withdraw";
/// Where one introduced computer tells the other it declined (a pairing
/// route: the two are not paired).
pub const DECLINE_PATH: &str = "/peer/pair/intro/decline";
/// How long an introduction may wait for its answers (ms).
pub const OFFER_TTL_MS: i64 = 10 * 60_000;
/// A closed one (declined, withdrawn) stays listed this long (ms).
const KEEP_CLOSED_MS: i64 = 10 * 60_000;
/// Introductions kept at most, and open from one introducer at most.
const MAX_KEPT: usize = 16;
const MAX_FROM_ONE: usize = 4;
/// Addresses an introduction may carry.
const MAX_ADDRS: usize = 8;
/// How long the introducer waits for each of the two.
const SEND_TIMEOUT: Duration = Duration::from_secs(15);

/// The computer an introduction names: public ids and addresses only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntroPeer {
    /// Its machine id.
    pub id: String,
    /// The name it gave itself.
    pub name: String,
    #[serde(default)]
    pub os: String,
    /// Its machine key (its endpoint id, hex): whom to dial, and the only
    /// key admitted for this introduction.
    pub key: String,
    /// Its last direct addresses, as the introducer saw them.
    #[serde(default)]
    pub addrs: Vec<String>,
    #[serde(default)]
    pub relay_url: Option<String>,
}

impl IntroPeer {
    /// From the introducer's record of a paired computer.
    pub fn of(p: &PeerRecord) -> Self {
        Self {
            id: p.id.clone(),
            name: p.name.clone(),
            os: p.os.clone(),
            key: p.key.clone(),
            addrs: p.addrs.iter().take(MAX_ADDRS).cloned().collect(),
            relay_url: p.relay_url.clone(),
        }
    }

    /// Checked and cleaned as received: well-formed ids, a name safe to
    /// show, addresses that parse, an `https` relay.
    fn checked(mut self) -> Option<Self> {
        if !ids::is_machine_id(&self.id) || !ids::is_key(&self.key) {
            return None;
        }
        self.name = super::clean_peer_name(&self.name);
        self.os = match self.os.as_str() {
            "macos" | "windows" | "linux" => self.os,
            _ => "other".into(),
        };
        self.addrs = self
            .addrs
            .into_iter()
            .filter(|a| a.parse::<SocketAddr>().is_ok())
            .take(MAX_ADDRS)
            .collect();
        self.relay_url = self
            .relay_url
            .filter(|u| u.starts_with("https://") && RelayUrl::from_str(u).is_ok());
        Some(self)
    }
}

/// Which side of the pairing a computer takes: the host waits for the
/// joiner, who dials it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IntroRole {
    Host,
    Joiner,
}

/// `POST /peer/intro`: what the introducer tells each of the two.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    pub id: String,
    pub role: IntroRole,
    pub other: IntroPeer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum IntroState {
    /// Waiting for this desktop's answer.
    Offered,
    /// Accepted here: the pairing runs, or ran (then it can be tried
    /// again).
    Accepted,
    /// Declined here, or by the other computer.
    Declined,
    /// The introducer took it back.
    Withdrawn,
    /// Nobody answered in time (the view's word for an open one past its
    /// time).
    Expired,
}

/// An introduction this computer received.
#[derive(Clone, Debug)]
pub struct Received {
    pub id: String,
    /// The introducer (a paired computer) and its name here.
    pub from: String,
    pub from_name: String,
    pub role: IntroRole,
    pub other: IntroPeer,
    pub at: i64,
    pub expires_at: i64,
    pub state: IntroState,
    /// Why it closed, in a sentence.
    pub detail: Option<String>,
    /// When it closed.
    pub closed_at: Option<i64>,
}

impl Received {
    fn open(&self) -> bool {
        matches!(self.state, IntroState::Offered | IntroState::Accepted)
            && super::now() < self.expires_at
    }

    /// What its pairing session carries.
    pub fn ticket(&self) -> Ticket {
        Ticket {
            id: self.id.clone(),
            from: self.from.clone(),
            from_name: self.from_name.clone(),
            other: self.other.clone(),
        }
    }
}

/// What a pairing session knows of the introduction it runs.
#[derive(Clone, Debug)]
pub struct Ticket {
    pub id: String,
    pub from: String,
    pub from_name: String,
    pub other: IntroPeer,
}

impl Ticket {
    /// What both computers' six digits are computed from, where a code
    /// pairing uses its code: the introducer and the introduction (public:
    /// the digits' strength is the commit-then-reveal over both keys).
    pub fn seed(&self) -> String {
        format!("intro:{}:{}", self.from, self.id)
    }

    /// The wire view (`pairing.intro`).
    pub fn view(&self) -> Value {
        json!({
            "id": self.id,
            "by": { "id": self.from, "name": self.from_name },
            "other": { "id": self.other.id, "name": self.other.name, "os": self.other.os },
        })
    }
}

/// The introductions this computer received (in memory: they last
/// minutes).
#[derive(Default)]
pub struct Intros(Mutex<Vec<Received>>);

impl Intros {
    fn with<R>(&self, f: impl FnOnce(&mut Vec<Received>) -> R) -> R {
        let mut list = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let now = super::now();
        list.retain(|r| r.open() || r.closed_at.unwrap_or(r.expires_at) + KEEP_CLOSED_MS > now);
        f(&mut list)
    }

    /// An open introduction names `key`: it may reach the pairing routes.
    pub fn admits(&self, key: &str) -> bool {
        self.with(|l| l.iter().any(|r| r.open() && r.other.key == key))
    }

    /// One received introduction.
    pub fn get(&self, id: &str) -> Option<Received> {
        self.with(|l| l.iter().find(|r| r.id == id).cloned())
    }
}

/// `in_` and 16 hex characters.
pub fn new_intro_id() -> String {
    let hex: String = crate::server::auth::random_bytes(8)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("in_{hex}")
}

pub fn is_intro_id(s: &str) -> bool {
    s.strip_prefix("in_")
        .is_some_and(|h| h.len() == 16 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

// ─── the introducer ──────────────────────────────────────────────────────────

/// `POST /api/machines/introduce {machine, with}`: introduces `machine` to
/// `with` (both paired here). `with` waits for `machine` to dial it. Each
/// is told who the other is; when one can't be reached the other's offer is
/// taken back. Answers how each took it: `offered`, `already` (they are
/// paired already), `failed` or `withdrawn`.
pub async fn introduce(state: &AppState, machine: &str, with: &str) -> ApiResult<Value> {
    if machine == with {
        return Err(ApiError::unprocessable(
            "intro_same",
            "Choose two different computers.",
        ));
    }
    let find = |id: &str| -> ApiResult<PeerRecord> {
        if !ids::is_machine_id(id) {
            return Err(ApiError::bad_request("Malformed request."));
        }
        state
            .machines
            .store_read(|f| f.peer(id).cloned())
            .ok_or_else(|| ApiError::not_found("machines_unknown", "Not a paired computer."))
    };
    let joiner = find(machine)?;
    let host = find(with)?;
    for p in [&joiner, &host] {
        if !versions::may_have(p, INTRODUCTIONS) {
            return Err(ApiError::conflict(
                "machine_too_old",
                format!(
                    "Update Mother Claude on {} first: it can't be introduced yet.",
                    p.display_name()
                ),
            )
            .with("machine", p.id.clone()));
        }
    }
    let id = new_intro_id();
    let offers = [
        (
            host.id.clone(),
            Offer {
                id: id.clone(),
                role: IntroRole::Host,
                other: IntroPeer::of(&joiner),
            },
        ),
        (
            joiner.id.clone(),
            Offer {
                id: id.clone(),
                role: IntroRole::Joiner,
                other: IntroPeer::of(&host),
            },
        ),
    ];
    let sends = offers.iter().map(|(to, offer)| async move {
        let body = serde_json::to_value(offer).unwrap_or(Value::Null);
        let r = transport::request(state, to, "POST", OFFER_PATH, Some(body), SEND_TIMEOUT).await;
        match r {
            Ok((200, v)) if v.get("already").and_then(Value::as_bool) == Some(true) => {
                ("already", None)
            }
            Ok((200, _)) => ("offered", None),
            Ok((_, v)) => (
                "failed",
                Some(
                    v.pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("It refused.")
                        .to_string(),
                ),
            ),
            Err(e) => ("failed", Some(e)),
        }
    });
    let mut results: Vec<(&str, Option<String>)> = futures_util::future::join_all(sends).await;
    // One side not told, or paired with the other already: the other's
    // offer goes back, so nobody waits for a pairing that can't happen.
    let stuck = results
        .iter()
        .any(|(s, _)| *s == "failed" || *s == "already");
    if stuck {
        for (i, (to, _)) in offers.iter().enumerate() {
            if results[i].0 == "offered" {
                let _ = transport::request(
                    state,
                    to,
                    "POST",
                    WITHDRAW_PATH,
                    Some(json!({ "id": id })),
                    SEND_TIMEOUT,
                )
                .await;
                results[i] = ("withdrawn", None);
            }
        }
    }
    state
        .devices
        .audit
        .record(crate::server::devices::audit::AuditEntry {
            action: "machine.introduce".into(),
            device_id: Some(joiner.id.clone()),
            device_name: Some(joiner.display_name().to_string()),
            detail: Some(json!({
                "intro": id, "with": host.id, "withName": host.display_name(),
                "joiner": results[1].0, "host": results[0].0,
            })),
            ..crate::server::devices::audit::AuditEntry::now()
        });
    let side = |p: &PeerRecord, (how, detail): &(&str, Option<String>)| json!({ "machine": p.id, "name": p.display_name(), "state": how, "detail": detail });
    Ok(json!({
        "id": id,
        "host": side(&host, &results[0]),
        "joiner": side(&joiner, &results[1]),
    }))
}

// ─── the two introduced ──────────────────────────────────────────────────────

/// `POST /peer/intro` from the paired computer `from` (its `view` grant
/// here): kept as an introduction for this desktop to answer, with a
/// notice. Nothing is paired.
pub fn receive(state: &AppState, from: &str, offer: Offer) -> ApiResult<Value> {
    if !is_intro_id(&offer.id) {
        return Err(ApiError::bad_request("Malformed request."));
    }
    let other = offer
        .other
        .clone()
        .checked()
        .ok_or_else(|| ApiError::bad_request("Malformed request."))?;
    let own = super::identity(state).map_err(ApiError::internal)?;
    if other.id == own.id || other.key == own.key_hex() {
        return Err(ApiError::unprocessable(
            "intro_self",
            "A computer can't be introduced to itself.",
        ));
    }
    let from_key = state
        .machines
        .store_read(|f| f.peer(from).map(|p| p.key.clone()));
    if other.id == from || from_key.as_deref() == Some(other.key.as_str()) {
        return Err(ApiError::unprocessable(
            "intro_introducer",
            "A computer introduces two others, never itself.",
        ));
    }
    if state
        .machines
        .store_read(|f| f.peer_by_key(&other.key).is_some())
    {
        return Ok(json!({ "already": true }));
    }
    let from_name = super::peer_name(state, from);
    let now = super::now();
    let received = Received {
        id: offer.id.clone(),
        from: from.to_string(),
        from_name: from_name.clone(),
        role: offer.role,
        other: other.clone(),
        at: now,
        expires_at: now + OFFER_TTL_MS,
        state: IntroState::Offered,
        detail: None,
        closed_at: None,
    };
    state.machines.intros.with(|l| {
        if l.iter().any(|r| r.id == received.id && r.from != from) {
            return Err(ApiError::conflict(
                "intro_taken",
                "Another computer used that introduction.",
            ));
        }
        l.retain(|r| r.id != received.id);
        if l.iter().filter(|r| r.from == from && r.open()).count() >= MAX_FROM_ONE {
            return Err(ApiError::conflict(
                "intro_too_many",
                "That computer has introduced too many computers that aren't answered yet.",
            ));
        }
        l.push(received);
        while l.len() > MAX_KEPT {
            match l.iter().position(|r| !r.open()) {
                Some(i) => l.remove(i),
                None => l.remove(0),
            };
        }
        Ok(())
    })?;
    crate::server::ws::publish_desktop_notice(
        state,
        crate::claude::model::NoticeLevel::Warning,
        format!(
            "{from_name} introduces {}: pair this computer with it? (Machines)",
            other.name
        ),
    );
    state
        .devices
        .audit
        .record(crate::server::devices::audit::AuditEntry {
            action: "machine.introduced".into(),
            device_id: Some(from.to_string()),
            device_name: Some(from_name),
            detail: Some(json!({ "intro": offer.id, "other": other.id, "otherName": other.name })),
            ..crate::server::devices::audit::AuditEntry::now()
        });
    super::publish(state, "introductions");
    Ok(json!({ "offered": true }))
}

/// `POST /peer/intro/withdraw` from the introducer: the introduction goes,
/// and so does a pairing it started that is not finished.
pub fn withdraw(state: &AppState, from: &str, id: &str) -> ApiResult<Value> {
    let name = super::peer_name(state, from);
    let found = close(
        state,
        id,
        |r| r.from == from,
        IntroState::Withdrawn,
        format!("{name} took the introduction back."),
    );
    Ok(json!({ "withdrawn": found }))
}

/// `POST /peer/pair/intro/decline` from the other introduced computer (its
/// key, while the introduction is open here).
pub fn declined_by(state: &AppState, key: &str, id: &str) -> ApiResult<Value> {
    let name = state
        .machines
        .intros
        .get(id)
        .filter(|r| r.other.key == key)
        .map(|r| r.other.name);
    let Some(name) = name else {
        return Err(ApiError::not_found(
            "intro_unknown",
            "No such introduction here.",
        ));
    };
    close(
        state,
        id,
        |r| r.other.key == key,
        IntroState::Declined,
        format!("{name} declined."),
    );
    Ok(json!({ "ok": true }))
}

/// Closes introduction `id` when `who` holds, ending its unfinished pairing
/// with `why`. Whether it did.
fn close(
    state: &AppState,
    id: &str,
    who: impl Fn(&Received) -> bool,
    to: IntroState,
    why: String,
) -> bool {
    let found = state.machines.intros.with(|l| {
        let r = l.iter_mut().find(|r| r.id == id && who(r))?;
        if !r.open() {
            return Some(false);
        }
        r.state = to;
        r.detail = Some(why.clone());
        r.closed_at = Some(super::now());
        Some(true)
    });
    if found == Some(true) {
        super::pairing::end_intro(state, id, "intro_closed", &why);
        super::publish(state, "introductions");
    }
    found.is_some()
}

/// `POST /api/machines/introductions/{id} {accept}` (the desktop): accept
/// starts this side of the pairing — the host waits for the other, the
/// joiner dials it — or tries it again; decline closes it and tells the
/// other computer.
pub async fn answer(state: &AppState, id: &str, accept: bool) -> ApiResult<()> {
    let r = state
        .machines
        .intros
        .get(id)
        .filter(Received::open)
        .ok_or_else(|| {
            ApiError::not_found(
                "intro_unknown",
                "That introduction is no longer open: ask the computer that introduced them again.",
            )
        })?;
    if !accept {
        close(
            state,
            id,
            |_| true,
            IntroState::Declined,
            "You declined.".into(),
        );
        audit(state, &r, "machine.intro.declined");
        let st = state.clone();
        tokio::spawn(async move { tell_declined(&st, &r).await });
        return Ok(());
    }
    state.machines.intros.with(|l| {
        if let Some(x) = l.iter_mut().find(|x| x.id == id) {
            x.state = IntroState::Accepted;
        }
    });
    audit(state, &r, "machine.intro.accepted");
    match r.role {
        IntroRole::Host => super::pairing::start_intro_host(state, r.ticket()).await?,
        IntroRole::Joiner => super::pairing::start_intro_joiner(state, r.ticket()).await?,
    }
    super::publish(state, "introductions");
    Ok(())
}

fn audit(state: &AppState, r: &Received, action: &str) {
    state
        .devices
        .audit
        .record(crate::server::devices::audit::AuditEntry {
            action: action.into(),
            device_id: Some(r.other.id.clone()),
            device_name: Some(r.other.name.clone()),
            detail: Some(json!({ "intro": r.id, "by": r.from, "byName": r.from_name })),
            ..crate::server::devices::audit::AuditEntry::now()
        });
}

/// Tells the other introduced computer this one declined (best effort).
async fn tell_declined(state: &AppState, r: &Received) {
    let Ok(addr) = addr_of(state, &r.other) else {
        return;
    };
    let conn =
        match tokio::time::timeout(SEND_TIMEOUT, transport::dial(state, addr, ALPN_PEER)).await {
            Ok(Ok(c)) => c,
            _ => return,
        };
    let _ = transport::request_on(
        &conn,
        "POST",
        DECLINE_PATH,
        Some(json!({ "id": r.id })),
        SEND_TIMEOUT,
    )
    .await;
}

/// An introduced pairing finished: the introduction is done.
pub fn finished(state: &AppState, id: &str) {
    state.machines.intros.with(|l| l.retain(|r| r.id != id));
    super::publish(state, "introductions");
}

/// The answer to a joiner that came for introduction `id` while no
/// pairing for it waits here: wait (this desktop has not accepted yet),
/// declined, or gone.
pub fn join_refused(state: &AppState, id: &str, key: &str) -> ApiError {
    match state.machines.intros.get(id).filter(|r| r.other.key == key) {
        Some(r) if r.open() && r.state == IntroState::Offered => ApiError::conflict(
            "intro_waiting",
            format!(
                "{} has not accepted the introduction yet.",
                super::this_name(state)
            ),
        ),
        Some(r) if r.state == IntroState::Declined => ApiError::conflict(
            "intro_declined",
            format!("{} declined.", super::this_name(state)),
        ),
        _ => ApiError::conflict(
            "pair_gone",
            "This pairing is no longer open on the other computer.",
        ),
    }
}

/// The introducer is no longer paired here: its introductions go.
pub fn forget_from(state: &AppState, from: &str) {
    let ids: Vec<String> = state.machines.intros.with(|l| {
        let gone: Vec<String> = l
            .iter()
            .filter(|r| r.from == from && r.open())
            .map(|r| r.id.clone())
            .collect();
        l.retain(|r| r.from != from);
        gone
    });
    for id in ids {
        super::pairing::end_intro(
            state,
            &id,
            "intro_closed",
            "The computer that introduced it is no longer paired.",
        );
    }
    super::publish(state, "introductions");
}

/// Where to dial an introduced computer: its key, the addresses the
/// introducer saw, and the relay (yours when you run one).
pub fn addr_of(state: &AppState, other: &IntroPeer) -> Result<EndpointAddr, String> {
    let public =
        PublicKey::from_str(&other.key).map_err(|_| "a malformed machine key".to_string())?;
    let mut addr = EndpointAddr::new(public);
    for a in other
        .addrs
        .iter()
        .filter_map(|a| a.parse::<SocketAddr>().ok())
    {
        addr = addr.with_ip_addr(a);
    }
    let settings = state.machines.settings();
    let relay = if settings.relay.mode == RelayMode::Custom {
        settings.relay.url
    } else {
        other.relay_url.clone()
    };
    if let Some(url) = relay.as_deref().and_then(|u| RelayUrl::from_str(u).ok()) {
        addr = addr.with_relay_url(url);
    }
    Ok(addr)
}

/// `GET /api/machines` `introductions`: the ones received, open first.
pub fn view(state: &AppState) -> Vec<Value> {
    let live = state
        .machines
        .pairing
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .filter(|s| !s.is_over())
        .and_then(|s| s.intro.as_ref().map(|t| t.id.clone()));
    let mut list = state.machines.intros.with(|l| l.clone());
    list.sort_by(|a, b| b.open().cmp(&a.open()).then(b.at.cmp(&a.at)));
    list.iter()
        .map(|r| {
            let shown = match r.state {
                IntroState::Offered | IntroState::Accepted if !r.open() => IntroState::Expired,
                s => s,
            };
            let detail = match shown {
                IntroState::Expired => Some("It expired.".to_string()),
                _ => r.detail.clone(),
            };
            json!({
                "id": r.id,
                "from": r.from,
                "fromName": r.from_name,
                "role": r.role,
                "other": {
                    "id": r.other.id,
                    "name": r.other.name,
                    "os": r.other.os,
                    "key": ids::fingerprint(&r.other.key),
                },
                "at": r.at,
                "expiresAt": r.expires_at,
                "state": shown,
                "detail": detail,
                "pairing": live.as_deref() == Some(r.id.as_str()),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn other() -> IntroPeer {
        IntroPeer {
            id: "c".repeat(32),
            name: "Laptop\u{0007}".into(),
            os: "beos".into(),
            key: "d".repeat(64),
            addrs: vec!["192.168.1.9:4433".into(), "not an address".into()],
            relay_url: Some("http://relay.example.com".into()),
        }
    }

    #[test]
    fn an_introduction_carries_public_ids_and_addresses_only() {
        let p = other().checked().unwrap();
        assert_eq!(p.name, "Laptop", "a name safe to show");
        assert_eq!(p.os, "other");
        assert_eq!(p.addrs, ["192.168.1.9:4433"]);
        assert_eq!(p.relay_url, None, "an https relay only");
        let wire = serde_json::to_value(Offer {
            id: new_intro_id(),
            role: IntroRole::Joiner,
            other: p,
        })
        .unwrap();
        let keys: Vec<&String> = wire["other"].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["addrs", "id", "key", "name", "os", "relayUrl"]);
        let mut bad = other();
        bad.key = "D".repeat(64);
        assert!(bad.checked().is_none(), "a key as the transport names it");
    }

    #[test]
    fn introduction_ids_are_checked() {
        let id = new_intro_id();
        assert!(is_intro_id(&id), "{id}");
        assert!(!is_intro_id("in_0123"));
        assert!(!is_intro_id("ask_0123456789abcdef"));
        assert!(!is_intro_id("in_0123456789ABCDEF"));
    }

    #[test]
    fn the_seed_binds_the_introducer_and_the_introduction() {
        let t = Ticket {
            id: "in_0123456789abcdef".into(),
            from: "a".repeat(32),
            from_name: "MacBook".into(),
            other: other(),
        };
        assert_eq!(
            t.seed(),
            format!("intro:{}:in_0123456789abcdef", "a".repeat(32))
        );
        assert_eq!(t.view()["by"]["name"], "MacBook");
        assert!(t.view()["other"].get("key").is_none());
    }
}
