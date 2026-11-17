//! Who talks to whom: the calling session ([`Me`]), the session it names
//! ([`Target`], here or on a paired computer) and the list
//! (`session_list`).
//!
//! A session is looked for here and in what the paired computers that show
//! theirs here last said (`machines::federation::snapshot`); naming a
//! computer asks that computer itself, so a session it does not show here
//! is found by its id. One conversation on two computers (a copy, a moved
//! session) is the one its lease names, else the one running; a write never
//! guesses beyond that.

use std::time::Duration;

use serde_json::{json, Value};

use super::format::iso;
use crate::claude::ids;
use crate::claude::model::{ControlChannel, SessionSummary};
use crate::machines::grants::Grant;
use crate::mcp::{Caller, ToolError};
use crate::state::AppState;

/// A plain request's time.
pub(crate) const CALL: Duration = Duration::from_secs(30);

/// The calling session.
#[derive(Clone, Debug, Default)]
pub(crate) struct Me {
    pub session: String,
    /// Its title, else its name.
    pub name: String,
    /// Its project folder (`pending::rules::project_of` its folder): what a
    /// permission for a project compares.
    pub project: Option<String>,
    pub project_name: Option<String>,
    /// This computer's name.
    pub computer: String,
}

fn unknown_caller() -> ToolError {
    ToolError::new(
        "unknown_session",
        "Mother Claude could not tell which Claude Code session is calling, so it cannot say \
         who this comes from. If this session started moments ago, try again.",
    )
}

/// The calling session, or why it is unknown.
pub(crate) async fn me(state: &AppState, caller: &Caller) -> Result<Me, ToolError> {
    me_if_known(state, caller).await.ok_or_else(unknown_caller)
}

/// The calling session, when Mother Claude could tell it.
pub(crate) async fn me_if_known(state: &AppState, caller: &Caller) -> Option<Me> {
    let session = caller.session_id.clone()?;
    let summary = state.find_session(&session).await;
    let cwd = caller.cwd.clone().filter(|c| !c.is_empty()).or_else(|| {
        summary
            .as_ref()
            .map(|s| s.cwd.clone())
            .filter(|c| !c.is_empty())
    });
    let project = cwd.as_deref().map(crate::pending::rules::project_of);
    let project_name = project
        .as_deref()
        .map(crate::machines::setup::folders::name_of)
        .filter(|n| !n.is_empty());
    let name = summary
        .as_ref()
        .map(name_of)
        .unwrap_or_else(|| short_name(&session));
    Some(Me {
        session,
        name,
        project,
        project_name,
        computer: crate::machines::this_name(state),
    })
}

pub(crate) fn short_name(id: &str) -> String {
    format!("Session {}", id.chars().take(8).collect::<String>())
}

fn given(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// A session's title, else its name, else its short id.
pub(crate) fn name_of(s: &SessionSummary) -> String {
    given(s.title.as_deref())
        .or_else(|| given(Some(&s.name)))
        .unwrap_or_else(|| short_name(&s.id))
}

fn name_in(v: &Value, id: &str) -> String {
    given(v.get("title").and_then(Value::as_str))
        .or_else(|| given(v.get("name").and_then(Value::as_str)))
        .unwrap_or_else(|| short_name(id))
}

// ─── where ─────────────────────────────────────────────────────────────────────

/// This computer, or a paired one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Place {
    #[default]
    Here,
    /// A paired computer: its id and the name it shows here.
    Peer { id: String, name: String },
}

impl Place {
    pub(crate) fn is_here(&self) -> bool {
        matches!(self, Place::Here)
    }

    /// The paired computer's id.
    pub(crate) fn machine(&self) -> Option<&str> {
        match self {
            Place::Peer { id, .. } => Some(id),
            Place::Here => None,
        }
    }

    pub(crate) fn name(&self, state: &AppState) -> String {
        match self {
            Place::Peer { name, .. } => name.clone(),
            Place::Here => crate::machines::this_name(state),
        }
    }
}

/// The computer a `computer` argument names: `here` (or this computer's
/// name) or a paired one, by name or id.
pub(crate) fn place_of(state: &AppState, computer: &str) -> Result<Place, ToolError> {
    let c = computer.trim();
    let lower = c.to_lowercase();
    if matches!(
        lower.as_str(),
        "" | "here" | "this" | "local" | "localhost" | "this computer"
    ) {
        return Ok(Place::Here);
    }
    match crate::machines::work::source::find_machine(state, c) {
        Ok(p) => Ok(Place::Peer {
            name: p.display_name().to_string(),
            id: p.id,
        }),
        // This computer by its own name or id (what a reply names).
        Err(_)
            if lower == crate::machines::this_name(state).to_lowercase()
                || crate::machines::self_id(state).is_some_and(|id| id == c) =>
        {
            Ok(Place::Here)
        }
        Err(e) => Err(e),
    }
}

/// Why this computer may not ask `machine` for `grant` now, if it may not.
pub(crate) fn why_not(state: &AppState, machine: &str, grant: Grant) -> Option<String> {
    let name = crate::machines::peer_name(state, machine);
    if !state.machines.enabled() {
        return Some(
            "Computers is off on this computer: turn it on in Computers first, or work with the \
             sessions here."
                .into(),
        );
    }
    let Some(peer) = state.machines.store_read(|f| f.peer(machine).cloned()) else {
        return Some(format!("{name} is no longer paired with this computer."));
    };
    let link = state.machines.link(machine);
    if !link.online {
        return Some(format!("{name} is offline."));
    }
    let lan = link.path == Some(crate::machines::transport::PathKind::Lan);
    if peer.their_grants.is_some_and(|g| !g.allows(grant, lan)) {
        // The permissions as Computers names them.
        let what = match grant {
            Grant::View => "See",
            _ => "Drive",
        };
        return Some(format!(
            "{name} has not given this computer the {what} permission{}: on {name}, open \
             Computers, press Change on {this}'s card and tick {what}.",
            if lan { "" } else { " over the internet" },
            this = crate::machines::this_name(state),
        ));
    }
    None
}

/// A JSON request to a paired computer's `/api` routes, as this computer
/// (`X-MC-Acting: machine`, `dangerous` after a dangerous approval here).
pub(crate) async fn peer_call(
    state: &AppState,
    machine: &str,
    method: &str,
    path: &str,
    body: Option<&Value>,
    dangerous: bool,
    timeout: Duration,
) -> Result<(u16, Value), ToolError> {
    let acting =
        crate::machines::acting::machine_header(&crate::machines::this_name(state), dangerous);
    crate::machines::moves::call(state, machine, method, path, body, &acting, timeout)
        .await
        .map_err(|e| {
            ToolError::new(
                "unreachable",
                format!(
                    "{} can't be reached right now: {e}",
                    crate::machines::peer_name(state, machine)
                ),
            )
        })
}

/// A paired computer's refusal, in its own words.
pub(crate) fn refused(state: &AppState, machine: &str, status: u16, v: &Value) -> ToolError {
    let e = crate::machines::moves::refusal(status, v, &crate::machines::peer_name(state, machine));
    ToolError::new(e.code, e.message)
}

// ─── whom ──────────────────────────────────────────────────────────────────────

/// How a message reaches a session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Reach {
    /// Mother Claude runs it.
    Owned,
    /// A conversation of the HTTP API.
    Api,
    /// The Mother Claude plugin's hooks (at its next step).
    Hooks,
    /// Typed into its tmux pane.
    Tmux,
    /// Typed into a background job's terminal.
    Terminal,
    /// It ended: a message continues it.
    Continue,
    /// Nothing reaches it (another app runs it, without the plugin).
    #[default]
    None,
}

impl Reach {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Reach::Owned => "owned",
            Reach::Api => "api",
            Reach::Hooks => "hooks",
            Reach::Tmux => "tmux",
            Reach::Terminal => "terminal",
            Reach::Continue => "continue",
            Reach::None => "none",
        }
    }
}

/// What `reach` means, once per list.
const REACH_KEY: &str = "owned: Mother Claude runs it; api: an HTTP API conversation; hooks: \
                         through Mother Claude's plugin, at its next step; tmux or terminal: \
                         typed into it; continue: it has ended, a message continues it; none: \
                         another app runs it and Mother Claude can't reach it";

/// A session, here or on a paired computer.
#[derive(Clone, Debug, Default)]
pub(crate) struct Target {
    pub place: Place,
    /// The computer's name.
    pub computer: String,
    pub id: String,
    pub name: String,
    pub project_name: String,
    pub cwd: String,
    pub live: bool,
    /// `working`, `waiting`, `idle`, … (`SessionStatus`).
    pub status: String,
    pub waiting_for: Option<String>,
    pub permission_mode: Option<String>,
    pub reach: Reach,
    pub last_activity_at: Option<i64>,
    pub surface: String,
    /// The computer its lease says it lives on now, for a copy here.
    pub lives_on: Option<String>,
}

impl Target {
    /// Its turn runs.
    pub(crate) fn busy(&self) -> bool {
        matches!(self.status.as_str(), "working" | "starting")
    }
}

fn word<T: serde::Serialize>(v: T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// A session here.
pub(crate) async fn from_summary(state: &AppState, s: &SessionSummary) -> Target {
    let reach = if state.bridge.thread(&s.id).await.is_some() {
        Reach::Api
    } else if crate::claude::owned::has_handle(state, &s.id) {
        Reach::Owned
    } else if !s.live {
        Reach::Continue
    } else {
        match s.control.channel {
            ControlChannel::Hooks => Reach::Hooks,
            ControlChannel::Tmux => Reach::Tmux,
            ControlChannel::Pty => Reach::Terminal,
            ControlChannel::Owned | ControlChannel::None => Reach::None,
        }
    };
    Target {
        place: Place::Here,
        computer: crate::machines::this_name(state),
        id: s.id.clone(),
        name: name_of(s),
        project_name: s.project_name.clone(),
        cwd: s.cwd.clone(),
        live: s.live,
        status: word(s.status),
        waiting_for: s.waiting_for.clone(),
        permission_mode: s.permission_mode.clone(),
        reach,
        last_activity_at: s.last_activity_at,
        surface: word(s.surface),
        lives_on: s.lives_on.as_ref().map(|l| l.machine.clone()),
    }
}

/// A session a paired computer described (its summary's JSON).
pub(crate) fn from_value(v: &Value, machine: &str, computer: &str) -> Option<Target> {
    let id = v.get("id")?.as_str()?.to_string();
    if !ids::is_session_id(&id) {
        return None;
    }
    let s = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|s| !s.is_empty())
    };
    let live = v.get("live").and_then(Value::as_bool).unwrap_or(false);
    let reach = if !live {
        Reach::Continue
    } else {
        match v.pointer("/control/channel").and_then(Value::as_str) {
            Some("owned") => Reach::Owned,
            Some("hooks") => Reach::Hooks,
            Some("tmux") => Reach::Tmux,
            Some("pty") => Reach::Terminal,
            _ => Reach::None,
        }
    };
    Some(Target {
        place: Place::Peer {
            id: machine.to_string(),
            name: computer.to_string(),
        },
        computer: computer.to_string(),
        name: name_in(v, &id),
        project_name: s("projectName").unwrap_or_default(),
        cwd: s("cwd").unwrap_or_default(),
        live,
        status: s("status").unwrap_or_else(|| "idle".into()),
        waiting_for: s("waitingFor"),
        permission_mode: s("permissionMode"),
        reach,
        last_activity_at: v.get("lastActivityAt").and_then(Value::as_i64),
        surface: s("surface").unwrap_or_default(),
        lives_on: None,
        id,
    })
}

/// A row of `session_list`.
pub(crate) fn row(t: &Target) -> Value {
    super::approve::strip_nulls(json!({
        "id": t.id,
        "name": t.name,
        "project": Some(&t.project_name).filter(|p| !p.is_empty()),
        "folder": Some(&t.cwd).filter(|c| !c.is_empty()),
        "computer": t.computer,
        "here": t.place.is_here(),
        "live": t.live,
        "status": t.status,
        "waitingFor": t.waiting_for,
        "lastActive": iso(t.last_activity_at),
        "reach": t.reach.as_str(),
        "permissionMode": t.permission_mode,
        "surface": Some(&t.surface).filter(|s| !s.is_empty() && *s != "unknown"),
    }))
}

/// `wanted` names this session: its id, an id's first characters (6 or
/// more), or its exact title or name (case aside).
pub(crate) fn names(wanted: &str, id: &str, title: Option<&str>, name: &str) -> bool {
    let w = wanted.trim();
    let lw = w.to_lowercase();
    if lw == id {
        return true;
    }
    if lw.len() >= 6
        && lw.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
        && id.starts_with(&lw)
    {
        return true;
    }
    [title.unwrap_or_default(), name]
        .iter()
        .any(|n| !n.trim().is_empty() && n.trim().to_lowercase() == lw)
}

async fn local_matches(state: &AppState, wanted: &str) -> Vec<Target> {
    let lw = wanted.trim().to_lowercase();
    let found: Vec<SessionSummary> = if ids::is_session_id(&lw) {
        state.find_session(&lw).await.into_iter().collect()
    } else {
        state
            .sessions()
            .await
            .into_iter()
            .filter(|s| names(wanted, &s.id, s.title.as_deref(), &s.name))
            .collect()
    };
    let mut out = Vec::with_capacity(found.len());
    for s in &found {
        out.push(from_summary(state, s).await);
    }
    out
}

/// What the paired computers that show their sessions here said, matching.
fn snapshot_matches(state: &AppState, wanted: &str, only: Option<&str>) -> Vec<Target> {
    let mut out = Vec::new();
    for m in crate::machines::federation::snapshot(state) {
        let Some(machine) = m.get("machine").and_then(Value::as_str) else {
            continue;
        };
        if only.is_some_and(|o| o != machine) {
            continue;
        }
        let computer = m
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| crate::machines::peer_name(state, machine));
        for s in m
            .get("sessions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let id = s.get("id").and_then(Value::as_str).unwrap_or_default();
            let title = s.get("title").and_then(Value::as_str);
            let name = s.get("name").and_then(Value::as_str).unwrap_or_default();
            if names(wanted, id, title, name) {
                out.extend(from_value(s, machine, &computer));
            }
        }
    }
    out
}

/// One session on `machine`, asked of it (found even when it does not
/// show that session here).
async fn fetch(state: &AppState, machine: &str, id: &str) -> Result<Option<Target>, ToolError> {
    let (status, v) = peer_call(
        state,
        machine,
        "GET",
        &format!("/api/sessions/{id}"),
        None,
        false,
        CALL,
    )
    .await?;
    match status {
        200..=299 => Ok(from_value(
            &v,
            machine,
            &crate::machines::peer_name(state, machine),
        )),
        404 => Ok(None),
        s => Err(refused(state, machine, s, &v)),
    }
}

/// Sessions on `machine` that `wanted` names: what it shows here, else
/// asked of it.
async fn peer_matches(
    state: &AppState,
    machine: &str,
    wanted: &str,
) -> Result<Vec<Target>, ToolError> {
    let lw = wanted.trim().to_lowercase();
    if ids::is_session_id(&lw) {
        return Ok(fetch(state, machine, &lw).await?.into_iter().collect());
    }
    let shown = snapshot_matches(state, wanted, Some(machine));
    if !shown.is_empty() {
        return Ok(shown);
    }
    let q: String = url::form_urlencoded::byte_serialize(wanted.trim().as_bytes()).collect();
    let (status, v) = peer_call(
        state,
        machine,
        "GET",
        &format!("/api/sessions?scope=all&limit=50&q={q}"),
        None,
        false,
        CALL,
    )
    .await?;
    if !(200..300).contains(&status) {
        return Err(refused(state, machine, status, &v));
    }
    let computer = crate::machines::peer_name(state, machine);
    Ok(v.get("sessions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|s| {
            names(
                wanted,
                s.get("id").and_then(Value::as_str).unwrap_or_default(),
                s.get("title").and_then(Value::as_str),
                s.get("name").and_then(Value::as_str).unwrap_or_default(),
            )
        })
        .filter_map(|s| from_value(s, machine, &computer))
        .collect())
}

fn self_error() -> ToolError {
    ToolError::new(
        "self",
        "That is this session itself: a session can't read, message or ask itself.",
    )
}

/// The session `wanted` names, here or on a paired computer. `strict` (a
/// write): one conversation on several computers must be told apart by
/// its lease or by running on exactly one of them, else `computer` must
/// say which; a read takes the most recent.
pub(crate) async fn resolve(
    state: &AppState,
    me: &Me,
    wanted: &str,
    computer: Option<&str>,
    strict: bool,
) -> Result<Target, ToolError> {
    let place = computer.map(|c| place_of(state, c)).transpose()?;
    let mut found: Vec<Target> = Vec::new();
    if place.as_ref().is_none_or(Place::is_here) {
        found.extend(local_matches(state, wanted).await);
    }
    match &place {
        Some(Place::Peer { id, .. }) => {
            if let Some(why) = why_not(state, id, Grant::View) {
                return Err(ToolError::new("not_allowed", why));
            }
            found.extend(peer_matches(state, id, wanted).await?);
        }
        None if state.machines.enabled() => {
            found.extend(snapshot_matches(state, wanted, None));
            // A copy here of a session that lives on another computer now:
            // that computer's, asked of it when it does not show it here.
            let away: Vec<(String, String)> = found
                .iter()
                .filter_map(|t| t.lives_on.clone().map(|m| (m, t.id.clone())))
                .filter(|(m, id)| {
                    !found
                        .iter()
                        .any(|t| t.place.machine() == Some(m.as_str()) && &t.id == id)
                })
                .collect();
            for (m, id) in away {
                if why_not(state, &m, Grant::View).is_none() {
                    if let Ok(Some(t)) = fetch(state, &m, &id).await {
                        found.push(t);
                    }
                }
            }
        }
        _ => {}
    }
    choose(me, wanted, found, strict, place.is_some())
}

/// One of `found`, or why not.
pub(crate) fn choose(
    me: &Me,
    wanted: &str,
    mut found: Vec<Target>,
    strict: bool,
    placed: bool,
) -> Result<Target, ToolError> {
    let had_self = found.iter().any(|t| t.id == me.session);
    found.retain(|t| t.id != me.session);
    // The same conversation heard twice from one computer: once.
    let mut seen: Vec<(String, Option<String>)> = Vec::new();
    found.retain(|t| {
        let key = (t.id.clone(), t.place.machine().map(str::to_string));
        if seen.contains(&key) {
            false
        } else {
            seen.push(key);
            true
        }
    });
    if found.is_empty() {
        return Err(if had_self {
            self_error()
        } else {
            ToolError::new(
                "not_found",
                format!(
                    "No session {wanted:?} {}. session_list names the sessions it can reach.",
                    if placed {
                        "on that computer"
                    } else {
                        "here or on the paired computers that show theirs here"
                    }
                ),
            )
        });
    }
    if found.len() == 1 {
        return Ok(found.remove(0));
    }
    let first = found[0].id.clone();
    if found.iter().any(|t| t.id != first) {
        let some: Vec<String> = found
            .iter()
            .take(6)
            .map(|t| format!("{} “{}” on {}", t.id, t.name, t.computer))
            .collect();
        return Err(ToolError::new(
            "ambiguous",
            format!(
                "More than one session matches {wanted:?}: {}. Name it by its id.",
                some.join("; ")
            ),
        ));
    }
    // One conversation on several computers: where its lease says it lives…
    if let Some(m) = found.iter().find_map(|t| t.lives_on.clone()) {
        if let Some(i) = found
            .iter()
            .position(|t| t.place.machine() == Some(m.as_str()))
        {
            return Ok(found.remove(i));
        }
    }
    // …else where it runs, when that is one place…
    let live: Vec<usize> = found
        .iter()
        .enumerate()
        .filter(|(_, t)| t.live)
        .map(|(i, _)| i)
        .collect();
    if live.len() == 1 {
        return Ok(found.remove(live[0]));
    }
    // …else, for a read, the most recent.
    if !strict {
        found.sort_by_key(|t| std::cmp::Reverse(t.last_activity_at.unwrap_or(0)));
        return Ok(found.remove(0));
    }
    let places: Vec<String> = found.iter().map(|t| t.computer.clone()).collect();
    Err(ToolError::new(
        "ambiguous",
        format!(
            "The session {first} is on {}{}: say which with computer.",
            places.join(" and "),
            if live.len() > 1 {
                ", running on each"
            } else {
                ""
            }
        ),
    ))
}

// ─── the list ──────────────────────────────────────────────────────────────────

pub(crate) struct ListArgs {
    pub computer: Option<String>,
    pub query: Option<String>,
    pub live_only: bool,
    pub limit: usize,
    pub projects: bool,
}

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// A paired computer's session matches `q` (lower case) the way the list's
/// search does, near enough: its title, name, project, folder, branch,
/// last prompt or id.
fn value_matches(s: &Value, q: &str) -> bool {
    [
        "title",
        "name",
        "projectName",
        "cwd",
        "gitBranch",
        "lastPrompt",
        "id",
    ]
    .iter()
    .filter_map(|k| s.get(*k).and_then(Value::as_str))
    .any(|v| v.to_lowercase().contains(q))
}

/// Running, waiting on someone, or active in the last 24 hours.
fn value_active(s: &Value, now: i64) -> bool {
    s.get("live").and_then(Value::as_bool).unwrap_or(false)
        || s.get("pending")
            .and_then(Value::as_array)
            .is_some_and(|p| !p.is_empty())
        || s.get("lastActivityAt")
            .or_else(|| s.get("startedAt"))
            .and_then(Value::as_i64)
            .is_some_and(|t| now - t < DAY_MS)
}

/// `session_list`.
pub(crate) async fn list(
    state: &AppState,
    me: Option<&Me>,
    a: ListArgs,
) -> Result<Value, ToolError> {
    let place = a
        .computer
        .as_deref()
        .map(|c| place_of(state, c))
        .transpose()?;
    let now = crate::claude::registry::now_ms();
    let q = a.query.as_deref().map(str::to_lowercase);
    let mine = me.map(|m| m.session.as_str());
    let mut rows: Vec<Target> = Vec::new();
    let mut computers: Vec<Value> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    if place.as_ref().is_none_or(Place::is_here) {
        for s in state.sessions().await {
            if Some(s.id.as_str()) == mine || (a.live_only && !s.live) {
                continue;
            }
            let keep = match &q {
                Some(q) => s.matches_search(q),
                None => s.in_active_scope(now),
            };
            if keep {
                rows.push(from_summary(state, &s).await);
            }
        }
        computers.push(json!({
            "name": crate::machines::this_name(state),
            "here": true,
            "online": true,
        }));
    }
    let machines_on = state.machines.enabled();
    if !machines_on && !place.as_ref().is_some_and(Place::is_here) {
        notes.push(
            "Computers is off on this computer: only its own sessions are listed.".to_string(),
        );
    }
    if machines_on && !place.as_ref().is_some_and(Place::is_here) {
        for m in crate::machines::federation::snapshot(state) {
            let Some(machine) = m.get("machine").and_then(Value::as_str) else {
                continue;
            };
            if place
                .as_ref()
                .and_then(Place::machine)
                .is_some_and(|only| only != machine)
            {
                continue;
            }
            let computer = m
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| crate::machines::peer_name(state, machine));
            computers.push(json!({
                "name": computer,
                "here": false,
                "online": m.get("online").and_then(Value::as_bool).unwrap_or(false),
            }));
            for s in m
                .get("sessions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(t) = from_value(s, machine, &computer) else {
                    continue;
                };
                if Some(t.id.as_str()) == mine || (a.live_only && !t.live) {
                    continue;
                }
                let keep = match &q {
                    Some(q) => value_matches(s, q),
                    None => value_active(s, now),
                };
                if keep {
                    rows.push(t);
                }
            }
        }
    }
    rows.sort_by(|x, y| {
        y.live
            .cmp(&x.live)
            .then(y.last_activity_at.cmp(&x.last_activity_at))
    });
    let total = rows.len();
    rows.truncate(a.limit);
    if total > rows.len() {
        notes.push(format!(
            "{} more: narrow it with query or computer, or raise limit.",
            total - rows.len()
        ));
    }
    let mut out = json!({
        "you": me.map(|m| json!({ "id": m.session, "name": m.name, "computer": m.computer })),
        "sessions": rows.iter().map(row).collect::<Vec<_>>(),
        "computers": computers,
        "reach": REACH_KEY,
    });
    if a.projects {
        out["projects"] = Value::Array(projects(state, place.as_ref()).await);
    }
    if !notes.is_empty() {
        out["notes"] = json!(notes);
    }
    Ok(super::approve::strip_nulls(out))
}

/// The project folders a session can start in, here and on the paired
/// computers that let this one see theirs.
async fn projects(state: &AppState, place: Option<&Place>) -> Vec<Value> {
    const EACH: usize = 40;
    let mut out = Vec::new();
    if place.is_none_or(Place::is_here) {
        let here = crate::machines::this_name(state);
        for p in crate::server::routes::data::project_list(state)
            .await
            .into_iter()
            .take(EACH)
        {
            out.push(super::approve::strip_nulls(json!({
                "name": p.name,
                "folder": p.path,
                "computer": here,
                "here": true,
                "lastActive": iso(p.last_activity_at),
            })));
        }
    }
    if !state.machines.enabled() || place.is_some_and(Place::is_here) {
        return out;
    }
    let peers: Vec<(String, String)> = match place {
        Some(Place::Peer { id, name }) => vec![(id.clone(), name.clone())],
        _ => state.machines.store_read(|f| {
            f.peers
                .iter()
                .map(|p| (p.id.clone(), p.display_name().to_string()))
                .collect()
        }),
    };
    for (id, name) in peers {
        if why_not(state, &id, Grant::View).is_some() {
            continue;
        }
        let Ok((200..=299, v)) = peer_call(
            state,
            &id,
            "GET",
            "/api/projects",
            None,
            false,
            Duration::from_secs(15),
        )
        .await
        else {
            continue;
        };
        for p in v
            .get("projects")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(EACH)
        {
            let Some(folder) = p.get("path").and_then(Value::as_str) else {
                continue;
            };
            out.push(super::approve::strip_nulls(json!({
                "name": p.get("name").and_then(Value::as_str).map(str::to_string)
                    .unwrap_or_else(|| crate::machines::setup::folders::name_of(folder)),
                "folder": folder,
                "computer": name,
                "here": false,
                "lastActive": iso(p.get("lastActivityAt").and_then(Value::as_i64)),
            })));
        }
    }
    out
}
