//! Pairing two machines (docs/PLAN-MACHINES.md §5.4, spikes/machines/UX.md §2).
//!
//! The first machine (the **host**) shows an 8-character code (Crockford
//! base32, 40 bits, one use, five minutes). The code does two things:
//!
//! 1. **Finding the host.** A short-lived *rendezvous* endpoint runs with a
//!    key derived from the code, so the other machine (the **joiner**) can
//!    compute whom to dial from the code alone — on the local network (mDNS,
//!    which also lists it as *Nearby*) and across the internet (n0's DNS with
//!    the public relays). The rendezvous answers only a caller that proves it
//!    holds the code (an HMAC over its own key), and only with the host's
//!    real address. Two different callers proving the code spoil it for both
//!    ("used by two computers"), as for phones.
//! 2. **Binding the real keys.** The joiner then dials the host's *machine
//!    key* and runs the pairing over that authenticated connection:
//!    `join` (an HMAC of the code over both machine keys, and a commitment
//!    to the joiner's nonce) → the host's nonce → `reveal` (the joiner's
//!    nonce). Both compute six digits from the code, both keys and both
//!    nonces. The commitment means neither side — nor anyone in the middle
//!    who learnt the code — can choose its nonce after seeing the other's,
//!    so nobody can grind keys until the two screens agree.
//!
//! The user compares the digits and confirms on both screens, each saying
//! what the other may do on it (its grants). When the joiner came over the
//! internet (not a direct path on the local network), the host must type
//! the digits the joiner shows instead of tapping, as phone pairing does.
//! Only when both confirmed does each store the other.
//!
//! **An introduced pairing** (`intro`, plan §5.12) runs the same exchange
//! without a code: a paired computer told both whom to dial, so the joiner
//! dials the host by the key the introduction names and the host admits
//! that key alone; `join` names the introduction instead of proving a code,
//! and the six digits are computed from the introduction (its introducer
//! and id) in the code's place, with both keys and both nonces.

use std::time::Duration;

use hmac::{Hmac, KeyInit, Mac};
use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointAddr, SecretKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::grants::Grants;
use super::protocol::{MachineInfo, ALPN_PAIR, ALPN_PEER};
use super::store::{PeerRecord, RelayMode};
use super::transport::{self, PathKind};
use crate::server::error::ApiError;
use crate::state::AppState;

/// How long a code works.
pub const CODE_TTL_MS: i64 = 5 * 60_000;
/// How long the two screens have to confirm once a machine joined.
pub const CONFIRM_TTL_MS: i64 = 10 * 60_000;
/// How long the joiner looks for the host.
const FIND_TIMEOUT: Duration = Duration::from_secs(25);
const CODE_LEN: usize = 8;
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// The mDNS note a rendezvous carries, so a machine nearby can list it.
pub const NEARBY_TAG: &str = "mcpair1:";

// ─── the code and its cryptography ───────────────────────────────────────────

/// A new code.
pub fn new_code() -> String {
    crate::server::auth::random_bytes(CODE_LEN)
        .iter()
        .map(|b| ALPHABET[(*b & 31) as usize] as char)
        .collect()
}

/// `K7QM4R2X` → `K7QM-4R2X`.
pub fn display_code(code: &str) -> String {
    if code.len() == CODE_LEN {
        format!("{}-{}", &code[..4], &code[4..])
    } else {
        code.to_string()
    }
}

/// What the user typed → the code, forgiving case, spaces, dashes and the
/// letters people confuse with digits (`O` → `0`, `I`/`L` → `1`).
pub fn normalize_code(input: &str) -> Option<String> {
    let mut out = String::with_capacity(CODE_LEN);
    for c in input.chars() {
        let c = match c.to_ascii_uppercase() {
            ' ' | '-' | '\t' => continue,
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        };
        if !ALPHABET.contains(&(c as u8)) || !c.is_ascii() {
            return None;
        }
        out.push(c);
    }
    (out.len() == CODE_LEN).then_some(out)
}

fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes a key of any size");
    for p in parts {
        mac.update(&(p.len() as u32).to_be_bytes());
        mac.update(p);
    }
    mac.finalize().into_bytes().into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// The rendezvous key a code stands for.
pub fn rendezvous_key(code: &str) -> SecretKey {
    SecretKey::from_bytes(&hmac(
        b"mother-claude/pair-rendezvous/1",
        &[code.as_bytes()],
    ))
}

/// Proof that `finder` holds the code (sent to the rendezvous).
pub fn find_proof(code: &str, finder: &str) -> String {
    hex(&hmac(code.as_bytes(), &[b"find", finder.as_bytes()]))
}

/// Proof that the joiner holds the code, bound to both machine keys.
pub fn join_proof(code: &str, host: &str, joiner: &str) -> String {
    hex(&hmac(
        code.as_bytes(),
        &[b"join", host.as_bytes(), joiner.as_bytes()],
    ))
}

/// The commitment to a nonce.
pub fn commitment(nonce: &[u8; 32]) -> String {
    let mut h = Sha256::new();
    h.update(b"mother-claude/pair-commit/1");
    h.update(nonce);
    hex(&h.finalize())
}

/// The six digits both screens show.
pub fn digits(
    code: &str,
    host: &str,
    joiner: &str,
    host_nonce: &[u8; 32],
    joiner_nonce: &[u8; 32],
) -> String {
    let mac = hmac(
        b"mother-claude/pair-digits/1",
        &[
            code.as_bytes(),
            host.as_bytes(),
            joiner.as_bytes(),
            host_nonce,
            joiner_nonce,
        ],
    );
    let n = u32::from_be_bytes([mac[0], mac[1], mac[2], mac[3]]) % 1_000_000;
    format!("{n:06}")
}

fn nonce() -> [u8; 32] {
    let mut n = [0u8; 32];
    n.copy_from_slice(&crate::server::auth::random_bytes(32));
    n
}

/// Constant-time comparison of two short strings.
pub fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

// ─── the session ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    Host,
    Joiner,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    /// The host is making its code; the joiner is looking for the host.
    Starting,
    /// The host shows its code and waits for a machine to join.
    Waiting,
    /// The joiner found the host and is binding the keys.
    Joining,
    /// Both show the digits; this side has not confirmed.
    Confirm,
    /// This side confirmed; the other has not.
    WaitingOther,
    Paired,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PairError {
    pub code: &'static str,
    pub message: String,
}

impl PairError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// The other machine, once known.
#[derive(Clone, Debug)]
pub struct OtherSide {
    pub info: MachineInfo,
    pub path: PathKind,
}

/// The one pairing this machine runs (as host or joiner).
pub struct Session {
    pub role: Role,
    pub phase: Phase,
    /// The code, normalized.
    pub code: String,
    pub expires_at: i64,
    pub other: Option<OtherSide>,
    pub digits: Option<String>,
    /// The host must type the digits (the joiner came over the internet).
    pub must_type: bool,
    pub confirmed: bool,
    /// What this side lets the other do (when it confirmed).
    pub grants: Option<Grants>,
    pub other_confirmed: bool,
    /// What the other lets this side do (when it confirmed).
    pub other_grants: Option<Grants>,
    pub error: Option<PairError>,
    pub peer_id: Option<String>,
    /// Tells this session apart from a later one (tasks check it).
    pub generation: u64,
    /// Host: the key that proved the code at the rendezvous.
    pub finder: Option<String>,
    /// This side's nonce.
    pub nonce: [u8; 32],
    /// Host: the joiner's commitment; joiner: unused.
    pub commit: Option<String>,
    /// Host: the rendezvous endpoint (closed when the session ends).
    pub rendezvous: Option<Endpoint>,
    /// An introduced pairing: who introduced the other computer, and which
    /// one it is (`None`: a pairing by code).
    pub intro: Option<super::intro::Ticket>,
    /// Joiner of an introduced pairing: the other computer has not accepted
    /// the introduction yet.
    pub waiting_accept: bool,
}

impl Session {
    fn new(role: Role, code: String, generation: u64) -> Self {
        Self {
            role,
            phase: Phase::Starting,
            code,
            expires_at: now() + CODE_TTL_MS,
            other: None,
            digits: None,
            must_type: false,
            confirmed: false,
            grants: None,
            other_confirmed: false,
            other_grants: None,
            error: None,
            peer_id: None,
            generation,
            finder: None,
            nonce: nonce(),
            commit: None,
            rendezvous: None,
            intro: None,
            waiting_accept: false,
        }
    }

    /// What the six digits are computed from besides the keys and nonces:
    /// the code, or for an introduced pairing the introduction.
    fn seed(&self) -> String {
        match &self.intro {
            Some(t) => t.seed(),
            None => self.code.clone(),
        }
    }

    fn fail(&mut self, error: PairError) {
        self.phase = Phase::Failed;
        self.error = Some(error);
        self.close_rendezvous();
    }

    fn close_rendezvous(&mut self) {
        if let Some(ep) = self.rendezvous.take() {
            tokio::spawn(async move { ep.close().await });
        }
    }

    /// Finished: paired, failed, or past its time.
    pub fn is_over(&self) -> bool {
        matches!(self.phase, Phase::Paired | Phase::Failed)
    }

    /// The wire view (`MachinesView.pairing`).
    pub fn view(&self) -> Value {
        let show_code = self.role == Role::Host
            && self.intro.is_none()
            && matches!(self.phase, Phase::Starting | Phase::Waiting);
        json!({
            "role": self.role,
            "state": self.phase,
            "code": show_code.then(|| display_code(&self.code)),
            "expiresAt": (!self.is_over()).then_some(self.expires_at),
            "other": self.other.as_ref().map(|o| json!({
                "id": o.info.id,
                "name": o.info.name,
                "os": o.info.os,
                "path": o.path,
            })),
            // A host that must type the digits never shows them: the person
            // at it reads them off the other computer's screen.
            "digits": (!(self.role == Role::Host && self.must_type)).then_some(self.digits.as_ref()).flatten(),
            "mustType": self.must_type,
            "confirmed": self.confirmed,
            "otherConfirmed": self.other_confirmed,
            "error": self.error,
            "peerId": self.peer_id,
            // An introduced pairing: by whom, and with which computer.
            "intro": self.intro.as_ref().map(super::intro::Ticket::view),
            "waitingAccept": self.waiting_accept,
        })
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn publish(state: &AppState) {
    super::publish(state, "pairing");
}

/// Runs `f` on the session when it is still generation `generation`.
fn with_session<R>(
    state: &AppState,
    generation: u64,
    f: impl FnOnce(&mut Session) -> R,
) -> Option<R> {
    let mut guard = state
        .machines
        .pairing
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    match guard.as_mut() {
        Some(s) if s.generation == generation => Some(f(s)),
        _ => None,
    }
}

fn next_generation(state: &AppState) -> u64 {
    use std::sync::atomic::Ordering;
    state.machines.generation.fetch_add(1, Ordering::Relaxed) + 1
}

/// Ends the current session, if any (cancel, or a new one replacing it).
fn end_current(state: &AppState, reason: Option<PairError>) {
    let mut guard = state
        .machines
        .pairing
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(s) = guard.as_mut() {
        if !s.is_over() {
            if let Some(r) = reason {
                s.fail(r);
            } else {
                s.close_rendezvous();
            }
        }
    }
}

// ─── the host ─────────────────────────────────────────────────────────────────

/// `POST /api/machines/pair`: a new code (replacing any pairing in progress).
pub async fn host_start(state: &AppState) -> Result<(), ApiError> {
    let ident = super::identity(state).map_err(ApiError::internal)?;
    let endpoint = transport::ensure_started(state)
        .await
        .map_err(unavailable)?;
    end_current(
        state,
        Some(PairError::new("pair_replaced", "A new pairing started.")),
    );
    let code = new_code();
    let generation = next_generation(state);
    {
        let mut guard = state
            .machines
            .pairing
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *guard = Some(Session::new(Role::Host, code.clone(), generation));
    }
    publish(state);

    let me = super::machine_info(state);
    let rendezvous = transport::bind_rendezvous(state, rendezvous_key(&code), &me)
        .await
        .map_err(|e| {
            with_session(state, generation, |s| {
                s.fail(PairError::new("pair_unavailable", e.clone()))
            });
            publish(state);
            unavailable(e)
        })?;
    let started = with_session(state, generation, |s| {
        s.rendezvous = Some(rendezvous.clone());
        s.phase = Phase::Waiting;
        s.expires_at = now() + CODE_TTL_MS;
    });
    if started.is_none() {
        // Cancelled while the rendezvous started.
        rendezvous.close().await;
        return Ok(());
    }
    publish(state);
    // Answer finders until the code is used or expires.
    let st = state.clone();
    let key_hex = ident.key_hex();
    tokio::spawn(rendezvous_loop(
        st, generation, rendezvous, endpoint, key_hex,
    ));
    let st = state.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(CODE_TTL_MS as u64)).await;
        let expired = with_session(&st, generation, |s| {
            if s.phase == Phase::Waiting {
                s.fail(PairError::new(
                    "pair_code_expired",
                    "The code expired. Make a new one.",
                ));
                true
            } else {
                false
            }
        });
        if expired == Some(true) {
            publish(&st);
        }
        // A pairing that never finished confirming ends too.
        tokio::time::sleep(Duration::from_millis((CONFIRM_TTL_MS - CODE_TTL_MS) as u64)).await;
        let stale = with_session(&st, generation, |s| {
            if !s.is_over() {
                s.fail(PairError::new(
                    "pair_timed_out",
                    "The pairing was not confirmed in time. Start again.",
                ));
                true
            } else {
                false
            }
        });
        if stale == Some(true) {
            publish(&st);
        }
    });
    Ok(())
}

#[derive(Deserialize)]
struct FindRequest {
    key: String,
    proof: String,
}

async fn rendezvous_loop(
    state: AppState,
    generation: u64,
    rendezvous: Endpoint,
    main: Endpoint,
    host_key: String,
) {
    while let Some(incoming) = rendezvous.accept().await {
        let Ok(conn) = incoming.await else { continue };
        let st = state.clone();
        let main = main.clone();
        let host_key = host_key.clone();
        tokio::spawn(async move {
            let _ = answer_find(&st, generation, conn, &main, &host_key).await;
        });
    }
}

async fn answer_find(
    state: &AppState,
    generation: u64,
    conn: Connection,
    main: &Endpoint,
    host_key: &str,
) -> Result<(), String> {
    let (mut send, mut recv) = conn.accept_bi().await.map_err(|e| e.to_string())?;
    let body = recv.read_to_end(4096).await.map_err(|e| e.to_string())?;
    let req: FindRequest = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    // The finder's key is the one the rendezvous handshake authenticated.
    let finder = conn.remote_id().to_string();
    let answer = with_session(state, generation, |s| -> Result<(), PairError> {
        if s.phase != Phase::Waiting || now() > s.expires_at {
            return Err(PairError::new(
                "pair_code_expired",
                "That code has expired. Make a new one on the other computer.",
            ));
        }
        if req.key != finder || !ct_eq(&req.proof, &find_proof(&s.code, &finder)) {
            return Err(PairError::new(
                "pair_code_wrong",
                "That code doesn't match.",
            ));
        }
        match &s.finder {
            Some(f) if *f != finder => {
                s.fail(PairError::new(
                    "pair_code_contested",
                    "This code was used by two computers, so neither can pair. Make a new code.",
                ));
                Err(PairError::new(
                    "pair_code_contested",
                    "This code was used by two computers, so neither can pair.",
                ))
            }
            _ => {
                s.finder = Some(finder.clone());
                Ok(())
            }
        }
    });
    let reply = match answer {
        None => {
            json!({"ok": false, "error": {"code": "pair_code_expired", "message": "That code has expired."}})
        }
        Some(Err(e)) => {
            publish(state);
            json!({"ok": false, "error": e})
        }
        Some(Ok(())) => json!({
            "ok": true,
            "key": host_key,
            "addr": main.addr(),
            "machine": super::machine_info(state),
        }),
    };
    let bytes = serde_json::to_vec(&reply).map_err(|e| e.to_string())?;
    send.write_all(&bytes).await.map_err(|e| e.to_string())?;
    send.finish().map_err(|e| e.to_string())?;
    let _ = tokio::time::timeout(Duration::from_secs(5), send.stopped()).await;
    Ok(())
}

/// `POST /peer/pair/join` on the host, from the key the rendezvous saw (or
/// the key an introduction named: then `intro` names it and there is no
/// code to prove).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinRequest {
    #[serde(default)]
    proof: String,
    commit: String,
    machine: MachineInfo,
    /// The introduction this joiner comes for (an introduced pairing).
    #[serde(default)]
    intro: Option<String>,
}

pub fn host_join(
    state: &AppState,
    caller_key: &str,
    path: PathKind,
    req: JoinRequest,
) -> Result<Value, ApiError> {
    let host_key = super::identity(state)
        .map_err(ApiError::internal)?
        .key_hex();
    if req.machine.key != caller_key || !super::ids::is_machine_id(&req.machine.id) {
        return Err(bad_request("The other computer sent a malformed request."));
    }
    let mut guard = state
        .machines
        .pairing
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // The session this join is for: a pairing by code, or the one of the
    // introduction it names.
    let Some(s) = guard.as_mut().filter(|s| {
        s.role == Role::Host
            && s.intro.as_ref().map(|t| t.id.as_str()) == req.intro.as_deref()
            && (s.intro.is_none() || !s.is_over())
    }) else {
        drop(guard);
        return Err(match req.intro.as_deref() {
            Some(id) => super::intro::join_refused(state, id, caller_key),
            None => pair_gone(),
        });
    };
    if s.phase != Phase::Waiting || now() > s.expires_at {
        return Err(pair_gone());
    }
    let proven = match &s.intro {
        // The key the introduction named, and the computer it named.
        Some(t) => {
            s.finder.as_deref() == Some(caller_key)
                && t.other.key == caller_key
                && t.other.id == req.machine.id
        }
        None => {
            s.finder.as_deref() == Some(caller_key)
                && ct_eq(&req.proof, &join_proof(&s.code, &host_key, caller_key))
        }
    };
    if !proven {
        return Err(if s.intro.is_some() {
            ApiError::forbidden(
                "intro_wrong_computer",
                "That is not the computer the introduction named.",
            )
        } else {
            ApiError::forbidden("pair_code_wrong", "That code doesn't match.")
        });
    }
    if req.commit.len() != 64 {
        return Err(bad_request("The other computer sent a malformed request."));
    }
    s.commit = Some(req.commit);
    s.other = Some(OtherSide {
        info: req.machine,
        path,
    });
    s.phase = Phase::Joining;
    s.expires_at = now() + CONFIRM_TTL_MS;
    let reply = json!({ "nonce": hex(&s.nonce), "machine": super::machine_info(state) });
    drop(guard);
    publish(state);
    Ok(reply)
}

#[derive(Deserialize)]
pub struct RevealRequest {
    nonce: String,
}

pub fn host_reveal(
    state: &AppState,
    caller_key: &str,
    req: RevealRequest,
) -> Result<Value, ApiError> {
    let host_key = super::identity(state)
        .map_err(ApiError::internal)?
        .key_hex();
    let mut guard = state
        .machines
        .pairing
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let Some(s) = guard
        .as_mut()
        .filter(|s| s.role == Role::Host && s.phase == Phase::Joining)
    else {
        return Err(pair_gone());
    };
    if s.finder.as_deref() != Some(caller_key) {
        return Err(pair_gone());
    }
    let Some(theirs) = unhex32(&req.nonce) else {
        return Err(bad_request("The other computer sent a malformed request."));
    };
    if !s
        .commit
        .as_deref()
        .is_some_and(|c| ct_eq(c, &commitment(&theirs)))
    {
        s.fail(PairError::new(
            "pair_bad_commitment",
            "The other computer's answer did not match its first message. Nothing was paired.",
        ));
        drop(guard);
        publish(state);
        return Err(ApiError::forbidden(
            "pair_bad_commitment",
            "Nothing was paired.",
        ));
    }
    s.digits = Some(digits(&s.seed(), &host_key, caller_key, &s.nonce, &theirs));
    // Over the internet the host types the digits the joiner shows.
    s.must_type = s.other.as_ref().is_some_and(|o| o.path != PathKind::Lan);
    s.phase = Phase::Confirm;
    drop(guard);
    publish(state);
    Ok(json!({ "ok": true }))
}

#[derive(Deserialize)]
pub struct PeerConfirm {
    /// What the joiner lets the host do on it.
    grants: Grants,
}

pub fn host_peer_confirm(
    state: &AppState,
    caller_key: &str,
    req: PeerConfirm,
) -> Result<Value, ApiError> {
    let result = {
        let mut guard = state
            .machines
            .pairing
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let Some(s) = guard.as_mut().filter(|s| s.role == Role::Host) else {
            return Err(pair_gone());
        };
        if s.finder.as_deref() != Some(caller_key) {
            return Err(pair_gone());
        }
        if !matches!(s.phase, Phase::Confirm | Phase::WaitingOther) {
            return Ok(status_reply(s));
        }
        s.other_confirmed = true;
        s.other_grants = Some(req.grants.normalized());
        if s.confirmed {
            finish_host(state, s)
        } else {
            Ok(())
        }
        .map(|_| status_reply(s))
    };
    publish(state);
    result
}

pub fn host_status(state: &AppState, caller_key: &str) -> Result<Value, ApiError> {
    let guard = state
        .machines
        .pairing
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    match guard
        .as_ref()
        .filter(|s| s.role == Role::Host && s.finder.as_deref() == Some(caller_key))
    {
        Some(s) => Ok(status_reply(s)),
        None => Err(pair_gone()),
    }
}

pub fn host_peer_cancel(state: &AppState, caller_key: &str) -> Result<Value, ApiError> {
    let changed = {
        let mut guard = state
            .machines
            .pairing
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        match guard
            .as_mut()
            .filter(|s| s.role == Role::Host && s.finder.as_deref() == Some(caller_key))
        {
            Some(s) if !s.is_over() => {
                let name = s
                    .other
                    .as_ref()
                    .map(|o| o.info.name.clone())
                    .unwrap_or_default();
                s.fail(PairError::new(
                    "pair_cancelled",
                    format!("{name} cancelled."),
                ));
                true
            }
            _ => false,
        }
    };
    if changed {
        publish(state);
    }
    Ok(json!({ "ok": true }))
}

fn status_reply(s: &Session) -> Value {
    json!({
        "state": s.phase,
        // What the joiner may do on the host, once paired.
        "grants": (s.phase == Phase::Paired).then_some(s.grants).flatten(),
        "error": s.error,
    })
}

fn bad_request(message: &str) -> ApiError {
    ApiError::new(
        axum::http::StatusCode::BAD_REQUEST,
        "pair_bad_request",
        message,
    )
}

fn unavailable(message: String) -> ApiError {
    ApiError::new(
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        "machines_transport",
        message,
    )
}

fn pair_gone() -> ApiError {
    ApiError::conflict(
        "pair_gone",
        "This pairing is no longer open on the other computer.",
    )
}

/// Both confirmed: the host stores the joiner.
fn finish_host(state: &AppState, s: &mut Session) -> Result<(), ApiError> {
    let other = s
        .other
        .clone()
        .ok_or_else(|| ApiError::internal("pairing finished without the other machine"))?;
    let grants = s.grants.unwrap_or_default();
    let their = s.other_grants;
    super::add_peer(state, &other.info, grants, their)?;
    s.phase = Phase::Paired;
    s.peer_id = Some(other.info.id.clone());
    s.close_rendezvous();
    paired_audit(state, &other.info, s.intro.as_ref());
    Ok(())
}

/// The audit line of a pairing, naming the introducer of an introduced one;
/// its introduction is done.
fn paired_audit(state: &AppState, info: &MachineInfo, intro: Option<&super::intro::Ticket>) {
    let Some(t) = intro else {
        super::audit(state, "machine.paired", &info.id, &info.name);
        return;
    };
    super::audit_detail(
        state,
        "machine.paired",
        &info.id,
        &info.name,
        json!({ "intro": t.id, "introducedBy": t.from, "introducedByName": t.from_name }),
    );
    super::intro::finished(state, &t.id);
}

/// `POST /api/machines/confirm` (either side).
pub async fn confirm(
    state: &AppState,
    grants: Grants,
    typed: Option<String>,
) -> Result<(), ApiError> {
    let grants = grants.normalized();
    let (role, generation) = {
        let guard = state
            .machines
            .pairing
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let s = guard
            .as_ref()
            .ok_or_else(|| ApiError::conflict("pair_none", "No pairing is in progress."))?;
        if s.phase != Phase::Confirm {
            return Err(ApiError::conflict(
                "pair_not_ready",
                "There is nothing to confirm yet.",
            ));
        }
        (s.role, s.generation)
    };
    match role {
        Role::Host => {
            let outcome = with_session(state, generation, |s| -> Result<(), ApiError> {
                if s.must_type {
                    let ok = typed
                        .as_deref()
                        .map(|t| t.chars().filter(char::is_ascii_digit).collect::<String>())
                        .zip(s.digits.as_deref())
                        .is_some_and(|(t, d)| ct_eq(&t, d));
                    if !ok {
                        let name = s
                            .other
                            .as_ref()
                            .map(|o| o.info.name.clone())
                            .unwrap_or_default();
                        s.fail(PairError::new(
                            "pair_digits_wrong",
                            format!("That's not the number on {name}. Nothing was paired."),
                        ));
                        return Err(ApiError::unprocessable(
                            "pair_digits_wrong",
                            format!("That's not the number on {name}. Nothing was paired."),
                        ));
                    }
                }
                s.confirmed = true;
                s.grants = Some(grants);
                if s.other_confirmed {
                    finish_host(state, s)
                } else {
                    s.phase = Phase::WaitingOther;
                    Ok(())
                }
            })
            .unwrap_or_else(|| {
                Err(ApiError::conflict(
                    "pair_none",
                    "No pairing is in progress.",
                ))
            });
            publish(state);
            outcome
        }
        Role::Joiner => joiner_confirm(state, generation, grants).await,
    }
}

/// `DELETE /api/machines/pair`: cancel on this side (and tell the other).
pub async fn cancel(state: &AppState) {
    let (role, host, generation) = {
        let guard = state
            .machines
            .pairing
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            Some(s) if !s.is_over() => (
                Some(s.role),
                s.other.as_ref().map(|o| o.info.key.clone()),
                s.generation,
            ),
            _ => (None, None, 0),
        }
    };
    if role.is_none() {
        let mut guard = state
            .machines
            .pairing
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *guard = None;
        drop(guard);
        publish(state);
        return;
    }
    with_session(state, generation, |s| {
        s.fail(PairError::new("pair_cancelled", "You cancelled."))
    });
    publish(state);
    if role == Some(Role::Joiner) {
        if let Some(key) = host {
            let _ = transport::request_key(
                state,
                &key,
                "POST",
                "/peer/pair/cancel",
                Some(json!({})),
                Duration::from_secs(5),
            )
            .await;
        }
    }
}

// ─── the joiner ───────────────────────────────────────────────────────────────

/// `POST /api/machines/join`: look for the machine showing `code` and pair.
pub async fn join_start(state: &AppState, code_input: &str) -> Result<(), ApiError> {
    let code = normalize_code(code_input).ok_or_else(|| {
        ApiError::unprocessable(
            "pair_code_malformed",
            "A code has 8 letters and digits, like K7QM-4R2X.",
        )
    })?;
    let ident = super::identity(state).map_err(ApiError::internal)?;
    // A machine cannot pair with itself.
    let own_code = {
        let guard = state
            .machines
            .pairing
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        guard
            .as_ref()
            .is_some_and(|s| s.role == Role::Host && s.code == code && !s.is_over())
    };
    if own_code {
        return Err(ApiError::unprocessable(
            "pair_code_own",
            "That is this computer's own code: type it on the other computer.",
        ));
    }
    transport::ensure_started(state)
        .await
        .map_err(unavailable)?;
    end_current(
        state,
        Some(PairError::new("pair_replaced", "A new pairing started.")),
    );
    let generation = next_generation(state);
    {
        let mut guard = state
            .machines
            .pairing
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut s = Session::new(Role::Joiner, code.clone(), generation);
        s.expires_at = now() + CONFIRM_TTL_MS;
        *guard = Some(s);
    }
    publish(state);
    let st = state.clone();
    tokio::spawn(async move {
        if let Err(e) = join_run(&st, generation, &code, &ident.key_hex()).await {
            with_session(&st, generation, |s| {
                if !s.is_over() {
                    s.fail(e)
                }
            });
            publish(&st);
        }
    });
    Ok(())
}

#[derive(Deserialize)]
struct FindReply {
    ok: bool,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    addr: Option<EndpointAddr>,
    #[serde(default)]
    machine: Option<MachineInfo>,
    #[serde(default)]
    error: Option<WireError>,
}

#[derive(Deserialize)]
struct WireError {
    code: String,
    message: String,
}

fn wire_error(e: WireError) -> PairError {
    let code = match e.code.as_str() {
        "pair_code_expired" => "pair_code_expired",
        "pair_code_contested" => "pair_code_contested",
        "pair_code_wrong" => "pair_code_wrong",
        _ => "pair_failed",
    };
    PairError::new(code, e.message)
}

async fn join_run(
    state: &AppState,
    generation: u64,
    code: &str,
    my_key: &str,
) -> Result<(), PairError> {
    let not_found = || {
        PairError::new(
            "pair_not_found",
            "No computer showing this code was found. Check the code, and that both computers \
             have the internet or are on the same network.",
        )
    };
    // 1. The rendezvous: its key comes from the code.
    let rendezvous_id = rendezvous_key(code).public();
    let mut addr = EndpointAddr::new(rendezvous_id);
    let settings = state.machines.settings();
    if settings.relay.mode == RelayMode::Custom {
        if let Some(url) = settings.relay.url.as_deref().and_then(|u| u.parse().ok()) {
            addr = addr.with_relay_url(url);
        }
    }
    if let Some(seen) = state.machines.nearby_addr(&rendezvous_id.to_string()) {
        addr = seen;
    }
    // The rendezvous publishes its address a moment after the code shows (n0's
    // DNS), and mDNS answers in its own time: keep trying until the window ends.
    let deadline = tokio::time::Instant::now() + FIND_TIMEOUT;
    let conn = loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(left, transport::dial(state, addr.clone(), ALPN_PAIR)).await {
            Ok(Ok(conn)) => break conn,
            Ok(Err(_)) if tokio::time::Instant::now() + Duration::from_secs(1) < deadline => {
                tokio::time::sleep(Duration::from_secs(1)).await;
                if let Some(seen) = state.machines.nearby_addr(&rendezvous_id.to_string()) {
                    addr = seen;
                }
            }
            _ => return Err(not_found()),
        }
    };
    let (mut send, mut recv) = conn.open_bi().await.map_err(|_| not_found())?;
    let req = json!({ "key": my_key, "proof": find_proof(code, my_key) });
    send.write_all(&serde_json::to_vec(&req).unwrap_or_default())
        .await
        .map_err(|_| not_found())?;
    send.finish().map_err(|_| not_found())?;
    let body = tokio::time::timeout(Duration::from_secs(15), recv.read_to_end(64 * 1024))
        .await
        .map_err(|_| not_found())?
        .map_err(|_| not_found())?;
    conn.close(0u32.into(), b"found");
    let reply: FindReply = serde_json::from_slice(&body).map_err(|_| not_found())?;
    if !reply.ok {
        return Err(reply.error.map(wire_error).unwrap_or_else(not_found));
    }
    let (Some(host_key), Some(host_addr), Some(host_info)) = (reply.key, reply.addr, reply.machine)
    else {
        return Err(not_found());
    };
    if host_addr.id.to_string() != host_key || host_info.key != host_key {
        return Err(PairError::new(
            "pair_failed",
            "The other computer answered inconsistently.",
        ));
    }
    if !super::protocol::compatible(host_info.versions.protocol) {
        return Err(PairError::new(
            "pair_incompatible",
            format!(
                "{} runs Mother Claude {}, which pairs differently: update both computers to the same version.",
                host_info.name, host_info.versions.app
            ),
        ));
    }
    // 2. The host's machine key, authenticated by the QUIC handshake.
    let host_conn = transport::dial(state, host_addr, ALPN_PEER)
        .await
        .map_err(|e| {
            PairError::new(
                "pair_unreachable",
                format!("Can't reach {}: {e}", host_info.name),
            )
        })?;
    let path = transport::path_of(&host_conn);
    let my_nonce = with_session(state, generation, |s| {
        s.phase = Phase::Joining;
        s.other = Some(OtherSide {
            info: host_info.clone(),
            path,
        });
        s.nonce
    })
    .ok_or_else(|| PairError::new("pair_cancelled", "You cancelled."))?;
    publish(state);
    let me = super::machine_info(state);
    let join = json!({
        "proof": join_proof(code, &host_key, my_key),
        "commit": commitment(&my_nonce),
        "machine": me,
    });
    let reply = transport::request_on(
        &host_conn,
        "POST",
        "/peer/pair/join",
        Some(join),
        Duration::from_secs(15),
    )
    .await
    .map_err(|e| PairError::new("pair_failed", e))?;
    let reply = expect_ok(reply)?;
    let their_nonce = reply
        .get("nonce")
        .and_then(Value::as_str)
        .and_then(unhex32)
        .ok_or_else(|| {
            PairError::new("pair_failed", "The other computer answered inconsistently.")
        })?;
    let reveal = json!({ "nonce": hex(&my_nonce) });
    expect_ok(
        transport::request_on(
            &host_conn,
            "POST",
            "/peer/pair/reveal",
            Some(reveal),
            Duration::from_secs(15),
        )
        .await
        .map_err(|e| PairError::new("pair_failed", e))?,
    )?;
    let shown = digits(code, &host_key, my_key, &their_nonce, &my_nonce);
    with_session(state, generation, |s| {
        s.digits = Some(shown);
        s.phase = Phase::Confirm;
    })
    .ok_or_else(|| PairError::new("pair_cancelled", "You cancelled."))?;
    publish(state);
    Ok(())
}

fn expect_ok((status, body): (u16, Value)) -> Result<Value, PairError> {
    if (200..300).contains(&status) {
        return Ok(body);
    }
    let err = body.get("error").unwrap_or(&body);
    let code = err
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("pair_failed");
    let message = err
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("The other computer refused the pairing.")
        .to_string();
    let code: &'static str = match code {
        "pair_code_wrong" => "pair_code_wrong",
        "pair_gone" => "pair_gone",
        "pair_bad_commitment" => "pair_bad_commitment",
        _ => "pair_failed",
    };
    Err(PairError::new(code, message))
}

async fn joiner_confirm(state: &AppState, generation: u64, grants: Grants) -> Result<(), ApiError> {
    let host_key = with_session(state, generation, |s| {
        s.confirmed = true;
        s.grants = Some(grants);
        s.phase = Phase::WaitingOther;
        s.other.as_ref().map(|o| o.info.key.clone())
    })
    .flatten()
    .ok_or_else(|| ApiError::conflict("pair_none", "No pairing is in progress."))?;
    publish(state);
    let reply = transport::request_key(
        state,
        &host_key,
        "POST",
        "/peer/pair/confirm",
        Some(json!({ "grants": grants })),
        Duration::from_secs(15),
    )
    .await;
    match reply
        .map_err(|e| PairError::new("pair_unreachable", e))
        .and_then(expect_ok)
    {
        Ok(v) => apply_host_status(state, generation, &v),
        Err(e) => {
            with_session(state, generation, |s| s.fail(e));
            publish(state);
            return Ok(());
        }
    }
    // Wait for the host's confirmation.
    let st = state.clone();
    tokio::spawn(async move {
        let deadline = now() + CONFIRM_TTL_MS;
        loop {
            let waiting =
                with_session(&st, generation, |s| s.phase == Phase::WaitingOther).unwrap_or(false);
            if !waiting || now() > deadline {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            let r = transport::request_key(
                &st,
                &host_key,
                "GET",
                "/peer/pair/status",
                None,
                Duration::from_secs(10),
            )
            .await;
            match r
                .map_err(|e| PairError::new("pair_unreachable", e))
                .and_then(expect_ok)
            {
                Ok(v) => apply_host_status(&st, generation, &v),
                Err(e) if e.code == "pair_gone" => {
                    with_session(&st, generation, |s| {
                        s.fail(PairError::new(
                            "pair_cancelled",
                            "The other computer cancelled.",
                        ))
                    });
                    publish(&st);
                    break;
                }
                Err(_) => continue,
            }
        }
    });
    Ok(())
}

fn apply_host_status(state: &AppState, generation: u64, v: &Value) {
    let phase = v.get("state").and_then(Value::as_str).unwrap_or("");
    match phase {
        "paired" => {
            let host_grants: Option<Grants> = v
                .get("grants")
                .and_then(|g| serde_json::from_value(g.clone()).ok());
            type Finished = (
                MachineInfo,
                Grants,
                Option<Grants>,
                Option<super::intro::Ticket>,
            );
            let finished = with_session(state, generation, |s| -> Option<Finished> {
                if s.phase == Phase::Paired {
                    return None;
                }
                let other = s.other.clone()?;
                s.other_confirmed = true;
                s.other_grants = host_grants;
                s.phase = Phase::Paired;
                s.peer_id = Some(other.info.id.clone());
                Some((
                    other.info,
                    s.grants.unwrap_or_default(),
                    host_grants,
                    s.intro.clone(),
                ))
            })
            .flatten();
            if let Some((info, grants, their, intro)) = finished {
                if let Err(e) = super::add_peer(state, &info, grants, their) {
                    with_session(state, generation, |s| {
                        s.fail(PairError::new("pair_failed", e.to_string()))
                    });
                } else {
                    paired_audit(state, &info, intro.as_ref());
                }
            }
            publish(state);
        }
        "failed" => {
            let err = v
                .get("error")
                .and_then(|e| serde_json::from_value::<WireError>(e.clone()).ok())
                .map(wire_error)
                .unwrap_or_else(|| {
                    PairError::new("pair_failed", "The other computer ended the pairing.")
                });
            with_session(state, generation, |s| s.fail(err));
            publish(state);
        }
        _ => {}
    }
}

// ─── introduced pairings (plan §5.12) ─────────────────────────────────────────

/// This desktop accepted an introduction as its host: it waits for the
/// introduced computer — that key alone — to join (replacing any pairing in
/// progress), until the confirmation window ends.
pub async fn start_intro_host(
    state: &AppState,
    ticket: super::intro::Ticket,
) -> Result<(), ApiError> {
    super::identity(state).map_err(ApiError::internal)?;
    transport::ensure_started(state)
        .await
        .map_err(unavailable)?;
    end_current(
        state,
        Some(PairError::new("pair_replaced", "A new pairing started.")),
    );
    let generation = next_generation(state);
    {
        let mut s = Session::new(Role::Host, String::new(), generation);
        s.phase = Phase::Waiting;
        s.finder = Some(ticket.other.key.clone());
        s.expires_at = now() + CONFIRM_TTL_MS;
        s.intro = Some(ticket);
        *state
            .machines
            .pairing
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(s);
    }
    publish(state);
    let st = state.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(CONFIRM_TTL_MS as u64)).await;
        let stale = with_session(&st, generation, |s| {
            if s.is_over() {
                return false;
            }
            s.fail(PairError::new(
                "pair_timed_out",
                "The pairing was not confirmed in time. Ask for the introduction again.",
            ));
            true
        });
        if stale == Some(true) {
            publish(&st);
        }
    });
    Ok(())
}

/// This desktop accepted an introduction as its joiner: it dials the host
/// by the key the introduction names, and joins once the host accepted too.
pub async fn start_intro_joiner(
    state: &AppState,
    ticket: super::intro::Ticket,
) -> Result<(), ApiError> {
    let ident = super::identity(state).map_err(ApiError::internal)?;
    transport::ensure_started(state)
        .await
        .map_err(unavailable)?;
    end_current(
        state,
        Some(PairError::new("pair_replaced", "A new pairing started.")),
    );
    let generation = next_generation(state);
    {
        let mut s = Session::new(Role::Joiner, String::new(), generation);
        s.phase = Phase::Joining;
        s.expires_at = now() + CONFIRM_TTL_MS;
        s.intro = Some(ticket.clone());
        *state
            .machines
            .pairing
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(s);
    }
    publish(state);
    let st = state.clone();
    tokio::spawn(async move {
        if let Err(e) = intro_join_run(&st, generation, &ticket, &ident.key_hex()).await {
            with_session(&st, generation, |s| {
                if !s.is_over() {
                    s.fail(e)
                }
            });
            publish(&st);
        }
    });
    Ok(())
}

/// How often the joiner tries again while the host has not accepted yet.
const INTRO_RETRY: Duration = Duration::from_secs(2);

/// The joiner's half of an introduced pairing: `join` (naming the
/// introduction, committing to its nonce) until the host accepted, then the
/// reveal and the six digits — as a pairing by code from its step 2.
async fn intro_join_run(
    state: &AppState,
    generation: u64,
    ticket: &super::intro::Ticket,
    my_key: &str,
) -> Result<(), PairError> {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(CONFIRM_TTL_MS as u64);
    let cancelled = || PairError::new("pair_cancelled", "You cancelled.");
    let addr = super::intro::addr_of(state, &ticket.other)
        .map_err(|e| PairError::new("pair_failed", e))?;
    let my_nonce = with_session(state, generation, |s| s.nonce).ok_or_else(cancelled)?;
    let me = super::machine_info(state);
    let join = json!({
        "intro": ticket.id,
        "commit": commitment(&my_nonce),
        "machine": me,
    });
    let mut conn: Option<Connection> = None;
    let (host_conn, reply) = loop {
        if tokio::time::Instant::now() > deadline {
            return Err(PairError::new(
                "pair_timed_out",
                format!("{} did not answer in time.", ticket.other.name),
            ));
        }
        if with_session(state, generation, |s| !s.is_over()) != Some(true) {
            return Err(cancelled());
        }
        let live = match conn.take().filter(|c| c.close_reason().is_none()) {
            Some(c) => c,
            None => match transport::dial(state, addr.clone(), ALPN_PEER).await {
                Ok(c) => c,
                Err(_) => {
                    tokio::time::sleep(INTRO_RETRY).await;
                    continue;
                }
            },
        };
        let answer = transport::request_on(
            &live,
            "POST",
            "/peer/pair/join",
            Some(join.clone()),
            Duration::from_secs(15),
        )
        .await;
        match answer {
            Ok((200, v)) => break (live, v),
            Ok((409, v)) if v.pointer("/error/code") == Some(&json!("intro_waiting")) => {
                let first = with_session(state, generation, |s| {
                    !std::mem::replace(&mut s.waiting_accept, true)
                })
                .ok_or_else(cancelled)?;
                if first {
                    publish(state);
                }
                conn = Some(live);
                tokio::time::sleep(INTRO_RETRY).await;
            }
            Ok((409, v)) if v.pointer("/error/code") == Some(&json!("intro_declined")) => {
                return Err(PairError::new(
                    "pair_declined",
                    format!("{} declined.", ticket.other.name),
                ));
            }
            Ok(other) => {
                return Err(expect_ok(other).err().unwrap_or_else(|| {
                    PairError::new("pair_failed", "The other computer refused the pairing.")
                }))
            }
            Err(_) => {
                tokio::time::sleep(INTRO_RETRY).await;
            }
        }
    };
    // The host must be the computer the introduction named.
    let host_info: MachineInfo = reply
        .get("machine")
        .and_then(|m| serde_json::from_value(m.clone()).ok())
        .filter(|m: &MachineInfo| m.key == ticket.other.key && m.id == ticket.other.id)
        .ok_or_else(|| {
            PairError::new("pair_failed", "The other computer answered inconsistently.")
        })?;
    let their_nonce = reply
        .get("nonce")
        .and_then(Value::as_str)
        .and_then(unhex32)
        .ok_or_else(|| {
            PairError::new("pair_failed", "The other computer answered inconsistently.")
        })?;
    let path = transport::path_of(&host_conn);
    with_session(state, generation, |s| {
        s.waiting_accept = false;
        s.other = Some(OtherSide {
            info: host_info,
            path,
        });
    })
    .ok_or_else(cancelled)?;
    publish(state);
    expect_ok(
        transport::request_on(
            &host_conn,
            "POST",
            "/peer/pair/reveal",
            Some(json!({ "nonce": hex(&my_nonce) })),
            Duration::from_secs(15),
        )
        .await
        .map_err(|e| PairError::new("pair_failed", e))?,
    )?;
    let shown = digits(
        &ticket.seed(),
        &ticket.other.key,
        my_key,
        &their_nonce,
        &my_nonce,
    );
    with_session(state, generation, |s| {
        s.digits = Some(shown);
        s.phase = Phase::Confirm;
    })
    .ok_or_else(cancelled)?;
    publish(state);
    Ok(())
}

/// The introduction `id` closed (declined, withdrawn): its pairing, when
/// one runs and has not finished, ends with `why`.
pub fn end_intro(state: &AppState, id: &str, code: &'static str, why: &str) {
    let ended = {
        let mut guard = state
            .machines
            .pairing
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        match guard.as_mut() {
            Some(s) if !s.is_over() && s.intro.as_ref().is_some_and(|t| t.id == id) => {
                s.fail(PairError::new(code, why));
                true
            }
            _ => false,
        }
    };
    if ended {
        publish(state);
    }
}

/// A finished session's record for the stored peer.
pub fn peer_record(info: &MachineInfo, grants: Grants, their: Option<Grants>) -> PeerRecord {
    PeerRecord {
        id: info.id.clone(),
        name: super::clean_peer_name(&info.name),
        alias: None,
        os: info.os.clone(),
        os_version: info.os_version.clone(),
        key: info.key.clone(),
        grants,
        their_grants: their,
        paired_at: now(),
        last_seen: Some(now()),
        versions: Some(info.versions.clone()),
        capabilities: Vec::new(),
        relay: None,
        addrs: Vec::new(),
        relay_url: None,
        shell: info.shell.clone(),
        paired_with: Vec::new(),
        extra: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_eight_unambiguous_characters_and_forgive_typing() {
        let c = new_code();
        assert_eq!(c.len(), 8);
        assert!(c.bytes().all(|b| ALPHABET.contains(&b)));
        assert_eq!(
            normalize_code(&display_code(&c)).as_deref(),
            Some(c.as_str())
        );
        assert_eq!(normalize_code("k7qm-4r2x").as_deref(), Some("K7QM4R2X"));
        assert_eq!(normalize_code(" K7QM 4R2X ").as_deref(), Some("K7QM4R2X"));
        assert_eq!(normalize_code("o1il-0000").as_deref(), Some("01110000"));
        assert!(normalize_code("K7QM4R2").is_none(), "too short");
        assert!(normalize_code("K7QM4R2XY").is_none(), "too long");
        assert!(
            normalize_code("K7QM-4R2U").is_none(),
            "U is not in the alphabet"
        );
        assert!(normalize_code("K7QM-4R2É").is_none());
    }

    #[test]
    fn the_rendezvous_key_follows_the_code_only() {
        assert_eq!(
            rendezvous_key("K7QM4R2X").public(),
            rendezvous_key("K7QM4R2X").public()
        );
        assert_ne!(
            rendezvous_key("K7QM4R2X").public(),
            rendezvous_key("K7QM4R2Y").public()
        );
    }

    #[test]
    fn proofs_bind_the_code_and_the_keys() {
        let (a, b) = ("aa".repeat(32), "bb".repeat(32));
        assert_eq!(
            join_proof("K7QM4R2X", &a, &b),
            join_proof("K7QM4R2X", &a, &b)
        );
        assert_ne!(
            join_proof("K7QM4R2X", &a, &b),
            join_proof("K7QM4R2X", &b, &a)
        );
        assert_ne!(
            join_proof("K7QM4R2X", &a, &b),
            join_proof("K7QM4R2Y", &a, &b)
        );
        assert_ne!(find_proof("K7QM4R2X", &a), find_proof("K7QM4R2X", &b));
    }

    #[test]
    fn both_sides_compute_the_same_six_digits_and_a_changed_nonce_changes_them() {
        let (a, b) = ("aa".repeat(32), "bb".repeat(32));
        let (na, nb) = ([1u8; 32], [2u8; 32]);
        let d = digits("K7QM4R2X", &a, &b, &na, &nb);
        assert_eq!(d.len(), 6);
        assert!(d.bytes().all(|c| c.is_ascii_digit()));
        assert_eq!(d, digits("K7QM4R2X", &a, &b, &na, &nb));
        assert_ne!(d, digits("K7QM4R2X", &a, &b, &na, &[3u8; 32]));
        assert_ne!(d, digits("K7QM4R2X", &b, &a, &na, &nb));
    }

    #[test]
    fn a_host_that_must_type_never_shows_the_digits() {
        let mut s = Session::new(Role::Host, "K7QM4R2X".into(), 1);
        s.digits = Some("123456".into());
        s.phase = Phase::Confirm;
        assert_eq!(s.view()["digits"], "123456");
        s.must_type = true;
        assert!(s.view()["digits"].is_null());
        assert_eq!(s.view()["mustType"], true);
        let mut j = Session::new(Role::Joiner, "K7QM4R2X".into(), 2);
        j.digits = Some("123456".into());
        j.must_type = true;
        assert_eq!(j.view()["digits"], "123456", "the joiner shows them");
    }

    #[test]
    fn a_commitment_matches_only_its_nonce() {
        let n = [7u8; 32];
        assert!(ct_eq(&commitment(&n), &commitment(&n)));
        assert!(!ct_eq(&commitment(&n), &commitment(&[8u8; 32])));
        assert_eq!(unhex32(&hex(&n)), Some(n));
        assert_eq!(unhex32("zz"), None);
    }
}
