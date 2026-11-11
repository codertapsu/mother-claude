//! Accounts across computers (docs/PLAN-MACHINES.md §5.14 step 5, Phase 3a
//! decision 3): a session moves to the account of the same name there
//! (case-insensitive), else the Default to the Default. Each computer tells
//! the others its accounts' names, kinds and sign-in states
//! (`GET /peer/accounts`, the `view` grant) — never a token, an email or a
//! folder. *Sign in on DESKTOP-PC…* and *Add it on DESKTOP-PC…* are asked
//! actions (`asks`): shown there, done only when someone there confirms.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::claude::accounts::signin::{self, LoginOptions};
use crate::claude::accounts::{status, Account, AccountKind};
use crate::machines::transport;
use crate::server::error::ApiError;
use crate::state::AppState;

/// An account's sign-in older than this is checked again by the setup.
pub const FRESH_MS: i64 = 10 * 60 * 1000;
/// Where another computer's accounts are read.
pub const PEER_PATH: &str = "/peer/accounts";

/// An account's sign-in on one computer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SignIn {
    SignedIn,
    SignedOut,
    /// A session of it failed to authenticate since its last good check.
    AuthFailed,
    /// A token account whose token is not on that computer.
    TokenMissing,
    /// `claude auth status` could not tell.
    CheckFailed,
    /// Not checked yet.
    #[serde(other)]
    Unknown,
}

/// One account as another computer may see it: its name, its kind
/// (`default`, `directory`, `token`) and its sign-in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountFact {
    /// This computer's own id for it: never sent to another.
    #[serde(skip)]
    pub id: String,
    pub name: String,
    pub kind: String,
    pub state: SignIn,
}

fn kind_of(a: &Account) -> &'static str {
    if a.is_default() {
        "default"
    } else if a.kind == AccountKind::Token {
        "token"
    } else {
        "directory"
    }
}

/// This computer's accounts, with their last known sign-ins.
pub fn local(state: &AppState) -> Vec<AccountFact> {
    let reg = state.accounts.snapshot();
    reg.accounts
        .iter()
        .map(|a| {
            let st = state.accounts.runtime.status(&a.id);
            let sign_in = if a.kind == AccountKind::Token
                && !a.is_default()
                && !state.accounts.has_token(&a.id)
            {
                SignIn::TokenMissing
            } else if st.auth_failed {
                SignIn::AuthFailed
            } else {
                match &st.auth {
                    Some(x) if x.error.is_some() => SignIn::CheckFailed,
                    Some(x) if x.logged_in => SignIn::SignedIn,
                    Some(_) => SignIn::SignedOut,
                    None => SignIn::Unknown,
                }
            };
            AccountFact {
                id: a.id.clone(),
                name: a.name.clone(),
                kind: kind_of(a).to_string(),
                state: sign_in,
            }
        })
        .collect()
}

/// Asks `claude auth status` again (with each account's environment) for
/// every account whose last check is older than [`FRESH_MS`] — in parallel.
pub async fn refresh_stale(state: &AppState) {
    let now = crate::machines::now();
    let stale: Vec<String> = state
        .accounts
        .accounts()
        .into_iter()
        .filter(|a| {
            let st = state.accounts.runtime.status(&a.id);
            !st.checking && st.checked_at.is_none_or(|t| now - t > FRESH_MS)
        })
        .map(|a| a.id)
        .collect();
    let checks = stale.iter().map(|id| status::refresh(state, id));
    for (id, r) in stale
        .iter()
        .zip(futures_util::future::join_all(checks).await)
    {
        if let Err(e) = r {
            tracing::debug!(account = %id, error = %e, "the setup could not check an account");
        }
    }
}

/// The account here that a session of `name` (another computer's) moves to:
/// the same name without regard to case, else the Default for the Default.
pub fn matching(state: &AppState, name: &str, default: bool) -> Option<Account> {
    let accounts = state.accounts.accounts();
    let lower = name.to_lowercase();
    accounts
        .iter()
        .find(|a| a.name.to_lowercase() == lower)
        .or_else(|| {
            default
                .then(|| accounts.iter().find(|a| a.is_default()))
                .flatten()
        })
        .cloned()
}

/// `GET /peer/accounts`: this computer's accounts as another may see them.
pub fn peer_answer(state: &AppState) -> Value {
    json!({ "accounts": local(state) })
}

/// Starts an account's own sign-in on this computer — `claude auth login`
/// for the Default or a directory account, `claude setup-token` for a token
/// account — whose browser opens here (docs/ACCOUNTS.md §15.2). Only from
/// this computer's desktop (a fix, or a confirmed ask); recorded instead
/// until the app turned fixes on.
pub async fn sign_in(state: &AppState, id: &str) -> Result<(), ApiError> {
    let account = state
        .accounts
        .get(id)
        .ok_or_else(|| ApiError::not_found("account_not_found", "No such account."))?;
    super::note(state, &format!("sign in {}", account.name));
    if !super::fixes_live(state) {
        return Ok(());
    }
    let started = if account.kind == AccountKind::Token && !account.is_default() {
        signin::start_token(state, id).await.map(|_| ())
    } else {
        signin::start_login(
            state,
            id,
            LoginOptions {
                console: false,
                email: None,
            },
        )
        .await
        .map(|_| ())
    };
    started.map_err(|e| ApiError::unprocessable("sign_in_failed", e.to_string()))
}

// ─── the table ─────────────────────────────────────────────────────────────────

/// One computer's column: its accounts, or why they are not known.
#[derive(Clone, Debug)]
pub struct Column {
    pub machine: String,
    pub accounts: Option<Vec<AccountFact>>,
}

/// One account on one computer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cell {
    /// Its sign-in there, or `missing` (no account of that name there),
    /// `unknown` (that computer's accounts are not known now).
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Its name there when it differs (the Default under another name).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// One account, matched across computers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub name: String,
    pub default: bool,
    /// Machine id → its cell.
    pub cells: BTreeMap<String, Cell>,
}

fn sign_in_word(s: SignIn) -> &'static str {
    match s {
        SignIn::SignedIn => "signedIn",
        SignIn::SignedOut => "signedOut",
        SignIn::AuthFailed => "authFailed",
        SignIn::TokenMissing => "tokenMissing",
        SignIn::CheckFailed => "checkFailed",
        SignIn::Unknown => "unknown",
    }
}

/// The accounts of every column matched by name (decision 3): one row per
/// account, the Default first, then this computer's (the first column's) in
/// its order, then the others' by name.
pub fn table(cols: &[Column]) -> Vec<Row> {
    let key = |a: &AccountFact| {
        if a.kind == "default" {
            "\u{0}default".to_string()
        } else {
            a.name.to_lowercase()
        }
    };
    let mut order: Vec<(String, String, bool)> = Vec::new();
    for col in cols {
        let Some(list) = &col.accounts else { continue };
        for a in list {
            let k = key(a);
            if !order.iter().any(|(o, _, _)| *o == k) {
                order.push((k, a.name.clone(), a.kind == "default"));
            }
        }
    }
    // The Default first; the first column's next as they come; then the
    // rest by name.
    let first: Vec<String> = cols
        .first()
        .and_then(|c| c.accounts.as_ref())
        .map(|l| l.iter().map(key).collect())
        .unwrap_or_default();
    order.sort_by(|(ka, na, da), (kb, nb, db)| {
        let pos = |k: &String| first.iter().position(|f| f == k).unwrap_or(usize::MAX);
        db.cmp(da)
            .then_with(|| pos(ka).cmp(&pos(kb)))
            .then_with(|| na.to_lowercase().cmp(&nb.to_lowercase()))
    });
    order
        .into_iter()
        .map(|(k, name, default)| {
            let cells = cols
                .iter()
                .map(|col| {
                    let cell = match &col.accounts {
                        None => Cell {
                            state: "unknown".into(),
                            kind: None,
                            name: None,
                        },
                        Some(list) => {
                            // The Default for the Default; any other by its
                            // name, without regard to case.
                            let found = if default {
                                list.iter().find(|a| a.kind == "default")
                            } else {
                                list.iter().find(|a| a.name.to_lowercase() == k)
                            };
                            match found {
                                Some(a) => Cell {
                                    state: sign_in_word(a.state).into(),
                                    kind: Some(a.kind.clone()),
                                    name: (a.name != name).then(|| a.name.clone()),
                                },
                                None => Cell {
                                    state: "missing".into(),
                                    kind: None,
                                    name: None,
                                },
                            }
                        }
                    };
                    (col.machine.clone(), cell)
                })
                .collect();
            Row {
                name,
                default,
                cells,
            }
        })
        .collect()
}

/// Another computer's accounts (`GET /peer/accounts`): `Err` with why not.
pub async fn fetch(state: &AppState, machine: &str) -> Result<Vec<AccountFact>, String> {
    let (status, v) = transport::request(
        state,
        machine,
        "GET",
        PEER_PATH,
        None,
        Duration::from_secs(20),
    )
    .await?;
    match status {
        200 => serde_json::from_value(v["accounts"].clone())
            .map_err(|_| "it answered something unexpected".to_string()),
        404 => Err("older".into()),
        403 => Err("refused".into()),
        s => Err(format!("it answered {s}")),
    }
}

/// `GET /api/machines/accounts`: every computer's accounts matched by name,
/// with what this computer asked of the others and how it went.
pub async fn view(state: &AppState) -> Value {
    let here = crate::machines::machine_info(state);
    let peers: Vec<(String, String)> = state.machines.store_read(|f| {
        f.peers
            .iter()
            .map(|p| (p.id.clone(), p.display_name().to_string()))
            .collect()
    });
    let fetched =
        futures_util::future::join_all(peers.iter().map(|(id, _)| fetch(state, id))).await;
    let mut cols = vec![Column {
        machine: here.id.clone(),
        accounts: Some(local(state)),
    }];
    let mut machines = vec![json!({
        "id": here.id, "name": here.name, "here": true, "online": true, "error": null,
    })];
    for ((id, name), r) in peers.iter().zip(fetched) {
        let online = state.machines.link(id).online;
        let error = r.as_ref().err().cloned();
        machines.push(json!({
            "id": id, "name": name, "here": false, "online": online,
            "error": error.map(|e| match e.as_str() {
                "older" => format!("Update Mother Claude on {name} to see its accounts."),
                "refused" => format!("{name} does not let this computer see it."),
                _ => format!("Can't reach {name} now: {e}"),
            }),
        }));
        cols.push(Column {
            machine: id.clone(),
            accounts: r.ok(),
        });
    }
    let asked = super::with_cache(state, |c| {
        c.peers
            .iter()
            .map(|(id, pc)| (id.clone(), pc.asks.clone()))
            .collect::<BTreeMap<_, _>>()
    });
    let rows: Vec<Value> = table(&cols)
        .into_iter()
        .map(|row| {
            let lower = row.name.to_lowercase();
            let asks: BTreeMap<String, Value> = asked
                .iter()
                .filter_map(|(machine, list)| {
                    let a = list.iter().rev().find(|a| {
                        matches!(
                            a.kind,
                            super::asks::AskKind::SignIn | super::asks::AskKind::AddAccount
                        ) && a.account.as_deref().map(str::to_lowercase).as_deref()
                            == Some(lower.as_str())
                    })?;
                    Some((
                        machine.clone(),
                        serde_json::to_value(a).unwrap_or(Value::Null),
                    ))
                })
                .collect();
            json!({ "name": row.name, "default": row.default, "cells": row.cells, "asks": asks })
        })
        .collect();
    json!({ "machine": here.id, "machines": machines, "accounts": rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(name: &str, kind: &str, state: SignIn) -> AccountFact {
        AccountFact {
            id: String::new(),
            name: name.into(),
            kind: kind.into(),
            state,
        }
    }

    #[test]
    fn accounts_match_by_name_without_regard_to_case_and_the_default_by_kind() {
        let mac = Column {
            machine: "m".into(),
            accounts: Some(vec![
                a("Default", "default", SignIn::SignedIn),
                a("Work", "directory", SignIn::SignedIn),
                a("Client X", "token", SignIn::SignedIn),
            ]),
        };
        let pc = Column {
            machine: "w".into(),
            accounts: Some(vec![
                a("Home", "default", SignIn::SignedIn),
                a("work", "directory", SignIn::SignedOut),
                a("Side", "directory", SignIn::TokenMissing),
            ]),
        };
        let off = Column {
            machine: "b".into(),
            accounts: None,
        };
        let rows = table(&[mac, pc, off]);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Default", "Work", "Client X", "Side"]);
        let default = &rows[0];
        assert!(default.default);
        assert_eq!(default.cells["w"].state, "signedIn");
        assert_eq!(
            default.cells["w"].name.as_deref(),
            Some("Home"),
            "the Default there, under its own name"
        );
        assert_eq!(rows[1].cells["w"].state, "signedOut", "case-insensitive");
        assert_eq!(rows[1].cells["w"].name.as_deref(), Some("work"));
        assert_eq!(rows[2].cells["m"].state, "signedIn");
        assert_eq!(rows[2].cells["w"].state, "missing");
        assert_eq!(rows[3].cells["m"].state, "missing");
        assert_eq!(rows[3].cells["w"].state, "tokenMissing");
        assert!(
            rows.iter().all(|r| r.cells["b"].state == "unknown"),
            "a computer that could not be asked"
        );
    }

    #[test]
    fn what_another_computer_sees_has_no_id_token_email_or_folder() {
        let f = a("Work", "directory", SignIn::SignedIn);
        let v = serde_json::to_value(AccountFact {
            id: "acc_0123456789abcdef".into(),
            ..f
        })
        .unwrap();
        assert_eq!(
            v,
            json!({ "name": "Work", "kind": "directory", "state": "signedIn" })
        );
        let back: AccountFact =
            serde_json::from_value(json!({ "name": "W", "kind": "token", "state": "later" }))
                .unwrap();
        assert_eq!(back.state, SignIn::Unknown, "a state a newer build adds");
    }
}
