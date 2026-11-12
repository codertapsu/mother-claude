//! Asked actions (docs/PLAN-MACHINES.md §5.14 step 3 and 5, §7, §10): what
//! another computer asks this one to do — a check's fix, *Sign in…*, *Add
//! the account*, *Clone it* — shown here as a notice and on the Machines
//! page, and done **only when someone at this computer confirms it**
//! (`POST /api/machines/asks/{id}`, the desktop's). Nothing runs because
//! another computer asked.
//!
//! * The asking computer sends `POST /peer/setup/ask` (the `view` grant);
//!   this computer checks every part of the request against its own state
//!   (the check must be one of its own, the account one of its own, the
//!   clone's folder inside one of its roots, the remote a git URL without
//!   credentials) before keeping it.
//! * The asks are kept in `machines/setup.json` with their outcome; the
//!   asking computer reads its own through `/peer/setup` and is told to
//!   read again (`POST /peer/setup/changed`) when one is answered.
//! * A fix or a sign-in that would open a settings page, an administrator
//!   prompt or a browser runs only once the app turned fixes on
//!   (`MachinesState::enable_fixes`); until then (tests) it is recorded. A
//!   clone runs `git clone` through `code::GitRunner`, never prompting.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::folders;
use crate::machines::roots;
use crate::machines::transport;
use crate::server::context::ClientContext;
use crate::server::devices::audit::AuditEntry;
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Asks kept at most (the oldest answered ones go first).
const MAX_KEPT: usize = 100;
/// Unanswered asks one computer may have here at once.
const MAX_OPEN_FROM_ONE: usize = 20;
/// An answered ask is kept this long (ms).
const KEEP_ANSWERED_MS: i64 = 7 * 24 * 3600 * 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AskKind {
    /// One of this computer's checks' fix.
    Fix,
    /// Sign an account in here (its existing sign-in).
    SignIn,
    /// Add an account of that name here.
    AddAccount,
    /// `git clone <origin>` into a folder of one of this computer's roots.
    Clone,
    /// Keep a project in sync with the asking computer (code sync,
    /// plan §5.15): on only once confirmed here.
    Sync,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AskState {
    /// Waiting for someone at this computer.
    Asked,
    /// Confirmed and under way (a clone).
    Running,
    Done,
    /// *Not now*, at this computer.
    Declined,
    Failed,
}

/// One asked action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ask {
    /// `ask_` and 16 hex characters.
    pub id: String,
    /// The computer that asked (its id) and its name then.
    pub from: String,
    #[serde(default)]
    pub from_name: String,
    pub kind: AskKind,
    /// The check whose fix is asked (`fix`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<String>,
    /// What it is about, for the card: the check's title, or the
    /// repository's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The fix's label (`fix`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    /// The account's name (`signIn`, `addAccount`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// The remote, without credentials (`clone`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// Where the clone goes on this computer (`clone`); the project here
    /// (`sync`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// The code sync's id (`sync`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync: Option<String>,
    /// The project's folder on the asking computer (`sync`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub there: Option<String>,
    pub at: i64,
    pub state: AskState,
    /// How it went, in a sentence (never git's own output).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_at: Option<i64>,
}

impl Ask {
    pub fn open(&self) -> bool {
        matches!(self.state, AskState::Asked | AskState::Running)
    }

    /// Two asks for the same thing from one computer.
    fn same_thing(&self, other: &Ask) -> bool {
        let lower = |s: &Option<String>| s.as_deref().map(str::to_lowercase);
        self.from == other.from
            && self.kind == other.kind
            && self.check == other.check
            && lower(&self.account) == lower(&other.account)
            && self.origin == other.origin
            && self.folder == other.folder
            && self.there == other.there
    }
}

/// `POST /peer/setup/ask` (and `POST /api/machines/setup/ask` without
/// `machine`): `check` is a check's id or the action `signIn`,
/// `addAccount`, `clone`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AskRequest {
    pub check: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// The root the clone lands in, and its path in it (`/`-separated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rel: Option<String>,
    /// A code sync's request (`check: "sync"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync: Option<crate::machines::sync::on::SyncAsk>,
}

pub fn new_ask_id() -> String {
    let hex: String = crate::server::auth::random_bytes(8)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("ask_{hex}")
}

pub fn is_ask_id(s: &str) -> bool {
    s.strip_prefix("ask_")
        .is_some_and(|h| h.len() == 16 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

/// A path inside a root, as another computer names it: `/`- or
/// `\`-separated names, none empty, `.` or `..`, none a Windows cannot
/// hold, at most 260 characters. Its names, or `None`.
pub fn rel_parts(rel: &str) -> Option<Vec<String>> {
    if rel.is_empty() || rel.chars().count() > 260 || rel.chars().any(char::is_control) {
        return None;
    }
    let parts: Vec<String> = rel.split(['/', '\\']).map(str::to_string).collect();
    let ok = parts.len() <= 16
        && parts.iter().all(|p| {
            !p.is_empty()
                && p != "."
                && p != ".."
                && !p.ends_with(['.', ' '])
                && !p.contains(['<', '>', ':', '"', '|', '?', '*'])
        });
    ok.then_some(parts)
}

/// The folder `rel` names inside root `root` on this computer: `Err` with
/// why not (no such root here, a bad path).
pub fn folder_in_root(state: &AppState, root: &str, rel: &str) -> Result<PathBuf, String> {
    let parts = rel_parts(rel).ok_or("That is not a folder inside a root.")?;
    let here = roots::here_id(state).ok_or("This computer's id could not be read.")?;
    let base = roots::roots(state)
        .into_iter()
        .find(|r| r.id == root)
        .and_then(|r| r.path_of(&here).map(str::to_string))
        .ok_or("No such folder is shared with this computer.")?;
    let mut p = PathBuf::from(base);
    for part in parts {
        p.push(part);
    }
    Ok(p)
}

/// A folder a clone may go to: not there, or empty.
pub fn free_folder(p: &Path) -> bool {
    match std::fs::read_dir(p) {
        Ok(mut entries) => entries.next().is_none(),
        Err(e) => e.kind() == std::io::ErrorKind::NotFound && !p.exists(),
    }
}

fn bad(code: &str, message: impl Into<String>) -> ApiError {
    ApiError::unprocessable(code, message)
}

/// Another computer's request, checked against this computer's own state
/// and kept as an ask (a repeated one replaces the open one before it). A
/// notice tells whoever is at this computer; nothing runs.
pub async fn receive(state: &AppState, from: &str, req: AskRequest) -> ApiResult<Ask> {
    if req.check.is_empty() || req.check.len() > 80 {
        return Err(ApiError::bad_request("Malformed request."));
    }
    let from_name = crate::machines::peer_name(state, from);
    let mut ask = Ask {
        id: new_ask_id(),
        from: from.to_string(),
        from_name: from_name.clone(),
        kind: AskKind::Fix,
        check: None,
        title: None,
        fix: None,
        account: None,
        origin: None,
        folder: None,
        sync: None,
        there: None,
        at: crate::machines::now(),
        state: AskState::Asked,
        detail: None,
        answered_at: None,
    };
    let notice = match req.check.as_str() {
        "signIn" => {
            let name = req.account.as_deref().map(str::trim).unwrap_or_default();
            let account =
                super::accounts::matching(state, name, name.eq_ignore_ascii_case("default"))
                    .ok_or_else(|| {
                        ApiError::not_found(
                            "machines_unknown_account",
                            "No account of that name here.",
                        )
                    })?;
            ask.kind = AskKind::SignIn;
            ask.account = Some(account.name.clone());
            format!(
                "{from_name} asks you to sign in to {} on this computer (Machines).",
                account.name
            )
        }
        "addAccount" => {
            let name = crate::claude::accounts::clean_name(req.account.as_deref().unwrap_or(""))
                .map_err(|e| bad("machines_bad_account", e.to_string()))?;
            if super::accounts::matching(state, &name, false).is_some() {
                return Err(ApiError::conflict(
                    "machines_account_here",
                    "This computer has an account of that name already.",
                ));
            }
            ask.kind = AskKind::AddAccount;
            ask.account = Some(name.clone());
            format!("{from_name} asks you to add the account {name} on this computer (Machines).")
        }
        "sync" => {
            let asked = req
                .sync
                .as_ref()
                .ok_or_else(|| ApiError::bad_request("Malformed request."))?;
            let c = crate::machines::sync::on::check_ask(state, from, asked).await?;
            ask.kind = AskKind::Sync;
            ask.title = Some(c.title.clone());
            ask.folder = Some(c.folder.clone());
            ask.there = Some(c.there.clone());
            ask.sync = Some(c.id.clone());
            ask.origin = c.origin.clone();
            format!(
                "{from_name} asks to keep {} in sync with it: changes made on either computer go to the other (Machines).",
                c.title
            )
        }
        "clone" => {
            let origin = req
                .origin
                .as_deref()
                .and_then(folders::sanitize_origin)
                .filter(|o| Some(o.as_str()) == req.origin.as_deref())
                .ok_or_else(|| {
                    bad(
                        "machines_bad_origin",
                        "That is not a git remote Mother Claude can clone (https, ssh or git, without credentials).",
                    )
                })?;
            let folder = folder_in_root(
                state,
                req.root.as_deref().unwrap_or(""),
                req.rel.as_deref().unwrap_or(""),
            )
            .map_err(|e| bad("machines_bad_folder", e))?;
            if !free_folder(&folder) {
                return Err(ApiError::conflict(
                    "machines_folder_taken",
                    format!("{} is there already.", folder.display()),
                ));
            }
            ask.kind = AskKind::Clone;
            ask.title = req.rel.as_deref().and_then(|r| rel_parts(r)?.pop());
            ask.origin = Some(origin.clone());
            ask.folder = Some(folder.to_string_lossy().into_owned());
            format!(
                "{from_name} asks you to clone {origin} into {} (Machines).",
                folder.display()
            )
        }
        check => {
            let found = super::view(state, false).await["self"]["checks"]
                .as_array()
                .and_then(|cs| cs.iter().find(|c| c["id"] == check).cloned());
            let Some(found) = found else {
                return Err(ApiError::not_found(
                    "machines_unknown_check",
                    "No such check here.",
                ));
            };
            let title = found["title"].as_str().unwrap_or("A check").to_string();
            let fix = found["fix"]["label"]
                .as_str()
                .unwrap_or("Open Machines › Set up")
                .to_string();
            ask.check = Some(check.to_string());
            ask.title = Some(title.clone());
            ask.fix = Some(fix.clone());
            format!("{from_name} asks you to fix \"{title}\" on this computer: {fix} (Machines).")
        }
    };
    let kept = super::with_cache(state, |c| {
        let now = ask.at;
        c.asks.retain(|a| {
            !(a.open() && a.same_thing(&ask))
                && (a.open() || a.answered_at.is_none_or(|t| now - t < KEEP_ANSWERED_MS))
        });
        if c.asks.iter().filter(|a| a.from == from && a.open()).count() >= MAX_OPEN_FROM_ONE {
            return Err(ApiError::conflict(
                "machines_too_many_asks",
                "That computer asked too many things that are not answered yet.",
            ));
        }
        c.asks.push(ask.clone());
        while c.asks.len() > MAX_KEPT {
            match c.asks.iter().position(|a| !a.open()) {
                Some(i) => c.asks.remove(i),
                None => c.asks.remove(0),
            };
        }
        c.save();
        Ok(())
    });
    kept?;
    crate::server::ws::publish_desktop_notice(
        state,
        crate::claude::model::NoticeLevel::Warning,
        notice,
    );
    state.devices.audit.record(AuditEntry {
        action: "machine.ask".into(),
        device_id: Some(from.to_string()),
        device_name: Some(from_name),
        detail: Some(json!({ "kind": ask.kind, "check": ask.check, "account": ask.account })),
        ..AuditEntry::now()
    });
    crate::machines::publish(state, "asks");
    Ok(ask)
}

/// The asks `machine` made of this computer (what `/peer/setup` tells it).
pub fn made_by(state: &AppState, machine: &str) -> Vec<Ask> {
    super::with_cache(state, |c| {
        c.asks
            .iter()
            .filter(|a| a.from == machine)
            .cloned()
            .collect()
    })
}

/// What the Machines page shows: open asks first, then the last answered
/// ones of the past day.
pub fn view(state: &AppState) -> Vec<Ask> {
    let now = crate::machines::now();
    let mut list: Vec<Ask> = super::with_cache(state, |c| {
        c.asks
            .iter()
            .filter(|a| a.open() || a.answered_at.is_some_and(|t| now - t < 24 * 3600 * 1000))
            .cloned()
            .collect()
    });
    list.sort_by(|a, b| b.open().cmp(&a.open()).then(b.at.cmp(&a.at)));
    list
}

fn set_state(state: &AppState, id: &str, s: AskState, detail: Option<String>) -> Option<Ask> {
    let ask = super::with_cache(state, |c| {
        let a = c.asks.iter_mut().find(|a| a.id == id)?;
        a.state = s;
        a.detail = detail;
        if !a.open() {
            a.answered_at = Some(crate::machines::now());
        }
        let out = a.clone();
        c.save();
        Some(out)
    });
    crate::machines::publish(state, "asks");
    ask
}

/// The asking computer turned off a code sync it asked for before anyone
/// here answered: its open ask is closed, saying so. Whether there was one.
pub fn withdraw_sync(state: &AppState, from: &str, sync: &str) -> bool {
    let open = super::with_cache(state, |c| {
        c.asks
            .iter()
            .find(|a| {
                a.open()
                    && a.from == from
                    && a.kind == AskKind::Sync
                    && a.sync.as_deref() == Some(sync)
            })
            .map(|a| a.id.clone())
    });
    let Some(id) = open else {
        return false;
    };
    let name = crate::machines::peer_name(state, from);
    set_state(
        state,
        &id,
        AskState::Failed,
        Some(format!("{name} turned it off before it was confirmed.")),
    );
    true
}

/// Tells the asking computer to read its asks again (best effort, in the
/// background).
fn tell(state: &AppState, machine: &str) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let st = state.clone();
    let machine = machine.to_string();
    handle.spawn(async move {
        let r = transport::request(
            &st,
            &machine,
            "POST",
            "/peer/setup/changed",
            None,
            Duration::from_secs(15),
        )
        .await;
        if let Err(e) = r {
            tracing::debug!(machine, error = %e, "the asking computer was not told");
        }
    });
}

/// `POST /api/machines/asks/{id} {answer}`: someone at this computer
/// confirms (`confirm`) or declines (`decline`) an ask. Confirming runs it:
/// the check's fix, the account's sign-in, opening Accounts to add the
/// account, or the clone. Answers the ask and where the dashboard goes
/// next (`/accounts` for an account).
pub async fn answer(
    state: &AppState,
    ctx: &ClientContext,
    id: &str,
    confirm: bool,
) -> ApiResult<Value> {
    if !is_ask_id(id) {
        return Err(ApiError::not_found(
            "machines_unknown_ask",
            "No such request.",
        ));
    }
    let ask = super::with_cache(state, |c| c.asks.iter().find(|a| a.id == id).cloned())
        .ok_or_else(|| ApiError::not_found("machines_unknown_ask", "No such request."))?;
    if !ask.open() || ask.state == AskState::Running {
        return Err(ApiError::conflict(
            "machines_ask_answered",
            "That request was answered already.",
        ));
    }
    if !confirm {
        let a = set_state(state, id, AskState::Declined, Some("Not now.".into()));
        audit(state, ctx, &ask, "machine.ask.declined");
        tell(state, &ask.from);
        if ask.kind == AskKind::Sync {
            crate::machines::sync::on::declined(state, &ask);
        }
        return Ok(json!({ "ask": a, "next": null }));
    }
    let mut next: Option<&str> = None;
    let outcome: Result<String, String> = match ask.kind {
        AskKind::Fix => {
            let check = ask.check.clone().unwrap_or_default();
            if check.starts_with("account:") {
                ctx.note_dangerous("account sign-in");
                next = Some("/accounts");
            }
            if check == "app" {
                // *Update Mother Claude*: the update shows on this
                // computer's Software updates page; nothing installs on its
                // own.
                next = Some(super::APP_UPDATE_PAGE);
                Ok("Software updates opened here.".to_string())
            } else {
                super::fix_check(state, &check)
                    .await
                    .map(|()| "Fixed.".to_string())
                    .map_err(|e| e.message)
            }
        }
        AskKind::SignIn => {
            let account = ask.account.clone().unwrap_or_default();
            ctx.note_dangerous("account sign-in");
            next = Some("/accounts");
            match super::accounts::matching(
                state,
                &account,
                account.eq_ignore_ascii_case("default"),
            ) {
                Some(a) => super::accounts::sign_in(state, &a.id)
                    .await
                    .map(|()| {
                        "The sign-in started: finish it in the browser on this computer."
                            .to_string()
                    })
                    .map_err(|e| e.message),
                None => Err("That account is not on this computer any more.".into()),
            }
        }
        AskKind::AddAccount => {
            next = Some("/accounts");
            Ok("Accounts opened to add it.".into())
        }
        AskKind::Sync => {
            set_state(state, id, AskState::Running, Some("Turning it on…".into()));
            crate::machines::sync::on::accept(state, &ask).await
        }
        AskKind::Clone => {
            let (Some(origin), Some(folder)) = (ask.origin.clone(), ask.folder.clone()) else {
                return Err(ApiError::internal("The request lost its remote or folder."));
            };
            set_state(state, id, AskState::Running, Some("Cloning…".into()));
            tell(state, &ask.from);
            super::note(state, &format!("clone {origin} into {folder}"));
            // Runs on its own and records its own end: a page closed
            // meanwhile does not leave it running forever.
            let (st, cx, asked) = (state.clone(), ctx.clone(), ask.clone());
            let job = tokio::spawn(async move {
                let o = origin.clone();
                let f = PathBuf::from(&folder);
                let git = crate::machines::code::GitRunner::new();
                let cloned = tokio::task::spawn_blocking(move || folders::clone_into(&git, &o, &f))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
                    .map(|()| format!("Cloned into {folder}."));
                finish(&st, &cx, &asked, cloned)
            });
            let a = job.await.map_err(|e| ApiError::internal(e.to_string()))?;
            return Ok(json!({ "ask": a, "next": null }));
        }
    };
    let a = finish(state, ctx, &ask, outcome);
    Ok(json!({ "ask": a, "next": next }))
}

/// An ask's end: kept, audited, and the asking computer told.
fn finish(
    state: &AppState,
    ctx: &ClientContext,
    ask: &Ask,
    outcome: Result<String, String>,
) -> Option<Ask> {
    let (s, detail, what) = match outcome {
        Ok(d) => (AskState::Done, d, "machine.ask.done"),
        Err(d) => (AskState::Failed, d, "machine.ask.failed"),
    };
    let a = set_state(state, &ask.id, s, Some(detail));
    audit(state, ctx, ask, what);
    tell(state, &ask.from);
    a
}

fn audit(state: &AppState, ctx: &ClientContext, ask: &Ask, what: &str) {
    state.devices.audit.record(AuditEntry {
        detail: Some(json!({
            "ask": ask.id, "kind": ask.kind, "from": ask.from,
            "check": ask.check, "account": ask.account,
        })),
        ..AuditEntry::from_ctx(ctx, what)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ask_ids_and_folders_inside_a_root_are_checked() {
        let id = new_ask_id();
        assert!(is_ask_id(&id), "{id}");
        assert!(!is_ask_id("ask_../../x"));
        assert!(!is_ask_id("ask_0123456789ABCDEF"));
        assert_eq!(rel_parts("app").unwrap(), ["app"]);
        assert_eq!(rel_parts("org/app").unwrap(), ["org", "app"]);
        assert_eq!(rel_parts(r"org\app").unwrap(), ["org", "app"]);
        for bad in [
            "", "../x", "a/../b", "/abs", "a//b", "C:\\x", "a/./b", "x.", "a\u{0}b", "a|b",
        ] {
            assert!(rel_parts(bad).is_none(), "{bad:?}");
        }
        assert!(rel_parts(&"a/".repeat(20)).is_none(), "too deep");
    }

    #[test]
    fn a_folder_is_free_when_absent_or_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(free_folder(&dir.path().join("new")));
        assert!(free_folder(dir.path()));
        std::fs::write(dir.path().join("f"), "x").unwrap();
        assert!(!free_folder(dir.path()));
        assert!(!free_folder(&dir.path().join("f")), "a file");
    }

    #[test]
    fn a_repeated_ask_is_the_same_thing() {
        let a = Ask {
            id: new_ask_id(),
            from: "m".into(),
            from_name: "Mac".into(),
            kind: AskKind::SignIn,
            check: None,
            title: None,
            fix: None,
            account: Some("Work".into()),
            origin: None,
            folder: None,
            sync: None,
            there: None,
            at: 1,
            state: AskState::Asked,
            detail: None,
            answered_at: None,
        };
        let b = Ask {
            id: new_ask_id(),
            account: Some("work".into()),
            ..a.clone()
        };
        assert!(a.same_thing(&b));
        let c = Ask {
            kind: AskKind::AddAccount,
            ..b.clone()
        };
        assert!(!a.same_thing(&c));
        let d = Ask {
            from: "other".into(),
            ..b
        };
        assert!(!a.same_thing(&d));
    }
}
