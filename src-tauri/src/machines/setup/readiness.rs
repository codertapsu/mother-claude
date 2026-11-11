//! The readiness test's steps 3–6 (docs/PLAN-MACHINES.md §5.14; UX §3.7's
//! columns), after the connection and the 20 MiB transfer, for each pair of
//! computers:
//!
//! * `tools` — git (2.30 or newer) and Claude Code on both, and whether the
//!   other's Claude Code is older;
//! * `code` — a git round trip on scratch repositories, both ways
//!   (`codetest`);
//! * `accounts` — each account of this computer has one of the same name
//!   there, signed in;
//! * `folders` — every root that names both computers has its folder
//!   there, and each project both computers have maps from one to the
//!   other.
//!
//! Each line is `ok`, `warning` (works, but a part waits — an account, a
//! project), `failed` or `skipped`, with one sentence. Nothing touches a
//! project or a config directory. The plan's last step, a session-shaped
//! transfer placed with the pull's code, comes with moving sessions: it
//! reuses the move's own packing and landing.

use super::accounts::{self, AccountFact, Column};
use super::checks::{older, short_version, GIT_MIN};
use super::folders::{self, Origins, Repo};
use super::{codetest, Facts, TestLine};
use crate::machines::roots::{self, Root};
use crate::machines::transport;
use crate::state::AppState;

/// The lines this module adds, in order.
pub const LINES: [&str; 4] = ["tools", "code", "accounts", "folders"];

fn line(id: &str, state: &str, detail: String) -> TestLine {
    TestLine {
        id: id.into(),
        state: state.into(),
        detail,
        ms: None,
        mibps: None,
    }
}

/// Every line, skipped: the other computer could not be reached.
pub fn skipped() -> Vec<TestLine> {
    LINES
        .iter()
        .map(|id| line(id, "skipped", "Skipped: not connected.".into()))
        .collect()
}

/// Steps 3–6 with `machine` (`name`).
pub async fn lines(state: &AppState, machine: &str, name: &str) -> Vec<TestLine> {
    let tools = tools_line(state, machine, name).await;
    let code = codetest::round_trip(state, machine, name).await;
    let accounts = accounts_line(state, machine, name).await;
    let folders = folders_line(state, machine, name).await;
    vec![tools, code, accounts, folders]
}

// ─── tools ─────────────────────────────────────────────────────────────────────

/// What git and Claude Code on both computers mean for moving sessions.
pub fn tools_verdict(name: &str, mine: &Facts, theirs: Option<&Facts>) -> (&'static str, String) {
    let Some(theirs) = theirs else {
        return (
            "skipped",
            format!("Update Mother Claude on {name} to compare its tools."),
        );
    };
    let place = |here: bool| {
        if here {
            "here".to_string()
        } else {
            format!("on {name}")
        }
    };
    for (here, f) in [(true, mine), (false, theirs)] {
        match &f.git {
            None => return ("failed", format!("git isn't installed {}.", place(here))),
            Some(v) if older(v, GIT_MIN) => {
                return (
                    "failed",
                    format!(
                        "git {} {} is older than {GIT_MIN}.",
                        short_version(v),
                        place(here)
                    ),
                )
            }
            Some(_) => {}
        }
        if f.claude.is_none() {
            return (
                "failed",
                format!("Claude Code isn't installed {}.", place(here)),
            );
        }
    }
    let (Some(a), Some(b)) = (mine.claude.as_deref(), theirs.claude.as_deref()) else {
        return ("failed", "Claude Code's version is not known.".into());
    };
    if older(b, a) {
        return (
            "warning",
            format!(
                "Claude Code on {name} ({}) is older than here ({}).",
                short_version(b),
                short_version(a)
            ),
        );
    }
    if older(a, b) {
        return (
            "warning",
            format!(
                "Claude Code here ({}) is older than on {name} ({}).",
                short_version(a),
                short_version(b)
            ),
        );
    }
    (
        "ok",
        format!("git and Claude Code {} on both.", short_version(a)),
    )
}

async fn tools_line(state: &AppState, machine: &str, name: &str) -> TestLine {
    let mine = super::facts(state);
    let answer = transport::request(
        state,
        machine,
        "GET",
        "/peer/setup",
        None,
        std::time::Duration::from_secs(25),
    )
    .await;
    match answer {
        Ok((200, v)) => {
            let theirs: Option<Facts> = v
                .get("facts")
                .and_then(|f| serde_json::from_value(f.clone()).ok());
            let (s, d) = tools_verdict(name, &mine, theirs.as_ref());
            line("tools", s, d)
        }
        Ok((status, _)) => line("tools", "failed", format!("{name} answered {status}.")),
        Err(e) => line("tools", "failed", format!("Can't ask {name}: {e}")),
    }
}

// ─── accounts ──────────────────────────────────────────────────────────────────

/// Whether each account of this computer has one of the same name on the
/// other, signed in (decision 3).
pub fn accounts_verdict(
    name: &str,
    mine: &[AccountFact],
    theirs: &[AccountFact],
) -> (&'static str, String) {
    let rows = accounts::table(&[
        Column {
            machine: "here".into(),
            accounts: Some(mine.to_vec()),
        },
        Column {
            machine: "there".into(),
            accounts: Some(theirs.to_vec()),
        },
    ]);
    let mut problems = Vec::new();
    let mut good = 0;
    for r in rows.iter().filter(|r| r.cells["here"].state != "missing") {
        match r.cells["there"].state.as_str() {
            "signedIn" => good += 1,
            "missing" => problems.push(format!("{}: not on {name}", r.name)),
            "unknown" | "checkFailed" => {
                problems.push(format!("{}: not checked on {name}", r.name))
            }
            _ => problems.push(format!("{}: not signed in on {name}", r.name)),
        }
    }
    if problems.is_empty() {
        let n = good;
        (
            "ok",
            if n == 1 {
                format!("1 account matches, signed in on {name}.")
            } else {
                format!("{n} accounts match, signed in on {name}.")
            },
        )
    } else {
        ("warning", problems.join(" · "))
    }
}

async fn accounts_line(state: &AppState, machine: &str, name: &str) -> TestLine {
    let mine = accounts::local(state);
    match accounts::fetch(state, machine).await {
        Ok(theirs) => {
            let (s, d) = accounts_verdict(name, &mine, &theirs);
            line("accounts", s, d)
        }
        Err(e) if e == "older" => line(
            "accounts",
            "skipped",
            format!("Update Mother Claude on {name} to compare its accounts."),
        ),
        Err(e) => line(
            "accounts",
            "failed",
            format!("Can't read {name}'s accounts: {e}"),
        ),
    }
}

// ─── folders ───────────────────────────────────────────────────────────────────

/// Whether every root naming both computers has its folder on the other,
/// and each project both have maps from this one to the other.
pub fn folders_verdict(
    name: &str,
    here: &str,
    there: &str,
    roots_now: &[Root],
    mine: &[Repo],
    theirs: &Origins,
    to_there: &dyn Fn(&str) -> Option<String>,
) -> (&'static str, String) {
    let both: Vec<&Root> = roots_now
        .iter()
        .filter(|r| r.live() && r.path_of(here).is_some() && r.path_of(there).is_some())
        .collect();
    let mut problems = Vec::new();
    for r in &both {
        match theirs.roots.iter().find(|x| x.id == r.id) {
            Some(x) if x.exists => {}
            Some(_) => problems.push(format!("{}: its folder is missing on {name}", r.name)),
            None => problems.push(format!("{}: not known on {name} yet", r.name)),
        }
    }
    let plan = folders::plan(here, there, roots_now, mine, &theirs.repos, to_there);
    let unmapped: usize = plan.proposals.iter().map(|p| p.repos.len()).sum();
    let total = plan.matched.len() + plan.mismatched.len() + unmapped;
    for m in &plan.mismatched {
        problems.push(format!("{}: mapped elsewhere on {name}", m.name));
    }
    if unmapped > 0 {
        problems.push(if unmapped == 1 {
            "1 project on both is not mapped yet (step 5)".to_string()
        } else {
            format!("{unmapped} projects on both are not mapped yet (step 5)")
        });
    }
    if both.is_empty() && total == 0 {
        return (
            "warning",
            format!("No folder maps a project to {name} yet: accept or add one in step 5."),
        );
    }
    if problems.is_empty() {
        let roots_part = if both.len() == 1 {
            format!("1 folder resolves on {name}")
        } else {
            format!("{} folders resolve on {name}", both.len())
        };
        (
            "ok",
            format!(
                "{roots_part} · {} of {total} projects map.",
                plan.matched.len()
            ),
        )
    } else {
        ("warning", problems.join(" · "))
    }
}

async fn folders_line(state: &AppState, machine: &str, name: &str) -> TestLine {
    let here = crate::machines::machine_info(state).id;
    let mine = folders::local_repos(state, false).await;
    match folders::fetch(state, machine, true).await {
        Ok(theirs) => {
            let roots_now = roots::roots(state);
            let to_there = |p: &str| roots::map_between(state, &here, machine, p);
            let (s, d) =
                folders_verdict(name, &here, machine, &roots_now, &mine, &theirs, &to_there);
            line("folders", s, d)
        }
        Err(e) if e == "older" => line(
            "folders",
            "skipped",
            format!("Update Mother Claude on {name} to compare its folders."),
        ),
        Err(e) => line(
            "folders",
            "failed",
            format!("Can't read {name}'s folders: {e}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machines::setup::accounts::SignIn;
    use crate::machines::setup::folders::RootHere;
    use crate::machines::store::CodeSettings;
    use std::collections::BTreeMap;

    fn facts(git: Option<&str>, claude: Option<&str>) -> Facts {
        Facts {
            os: "macos".into(),
            git: git.map(str::to_string),
            claude: claude.map(str::to_string),
            code: Some(CodeSettings::default()),
        }
    }

    #[test]
    fn tools_need_git_and_claude_code_on_both() {
        let good = facts(Some("2.47.1"), Some("2.1.330"));
        assert_eq!(tools_verdict("PC", &good, Some(&good)).0, "ok");
        assert_eq!(
            tools_verdict("PC", &good, Some(&facts(None, Some("2.1.330")))),
            ("failed", "git isn't installed on PC.".to_string())
        );
        assert_eq!(
            tools_verdict("PC", &facts(Some("2.20.0"), Some("2.1.330")), Some(&good)),
            ("failed", "git 2.20.0 here is older than 2.30.".to_string())
        );
        assert_eq!(
            tools_verdict("PC", &good, Some(&facts(Some("2.47.1"), None))).0,
            "failed"
        );
        let (s, d) = tools_verdict("PC", &good, Some(&facts(Some("2.47.1"), Some("2.1.322"))));
        assert_eq!(s, "warning");
        assert_eq!(
            d,
            "Claude Code on PC (2.1.322) is older than here (2.1.330)."
        );
        assert_eq!(tools_verdict("PC", &good, None).0, "skipped");
    }

    fn acc(name: &str, kind: &str, state: SignIn) -> AccountFact {
        AccountFact {
            id: String::new(),
            name: name.into(),
            kind: kind.into(),
            state,
        }
    }

    #[test]
    fn every_account_here_needs_one_signed_in_there() {
        let mine = vec![
            acc("Default", "default", SignIn::SignedIn),
            acc("Work", "directory", SignIn::SignedIn),
        ];
        let all = vec![
            acc("Default", "default", SignIn::SignedIn),
            acc("work", "directory", SignIn::SignedIn),
            acc("Extra", "directory", SignIn::SignedOut),
        ];
        assert_eq!(
            accounts_verdict("PC", &mine, &all),
            ("ok", "2 accounts match, signed in on PC.".to_string()),
            "an account only there does not count"
        );
        let some = vec![acc("Default", "default", SignIn::SignedOut)];
        assert_eq!(
            accounts_verdict("PC", &mine, &some),
            (
                "warning",
                "Default: not signed in on PC · Work: not on PC".to_string()
            )
        );
    }

    const MAC: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const PC: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn root() -> Root {
        Root {
            id: "rt_1111111111111111".into(),
            name: "projects".into(),
            paths: [
                (MAC.to_string(), "/Users/m/dev".to_string()),
                (PC.to_string(), r"D:\dev".to_string()),
            ]
            .into_iter()
            .collect(),
            updated_at: 1,
            by: MAC.into(),
            pair: None,
            removed: None,
            extra: BTreeMap::new(),
        }
    }

    fn repo(path: &str, origin: &str) -> Repo {
        Repo {
            path: path.into(),
            origin: Some(origin.into()),
        }
    }

    #[test]
    fn folders_resolve_there_and_projects_map() {
        let roots_now = vec![root()];
        let map = |p: &str| roots::map_in(&roots_now, MAC, PC, p);
        let mine = vec![
            repo("/Users/m/dev/app", "https://github.com/you/app"),
            repo("/Users/m/dev/lib", "https://github.com/you/lib"),
        ];
        let mut theirs = Origins {
            repos: vec![
                repo(r"D:\dev\app", "https://github.com/you/app.git"),
                repo(r"D:\dev\lib", "git@github.com:you/lib.git"),
            ],
            roots: vec![RootHere {
                id: "rt_1111111111111111".into(),
                exists: true,
            }],
        };
        assert_eq!(
            folders_verdict("PC", MAC, PC, &roots_now, &mine, &theirs, &map),
            (
                "ok",
                "1 folder resolves on PC · 2 of 2 projects map.".to_string()
            )
        );
        theirs.roots[0].exists = false;
        theirs.repos[1].path = r"E:\lib".into();
        let (s, d) = folders_verdict("PC", MAC, PC, &roots_now, &mine, &theirs, &map);
        assert_eq!(s, "warning");
        assert_eq!(
            d,
            "projects: its folder is missing on PC · lib: mapped elsewhere on PC"
        );
        let none: Vec<Root> = Vec::new();
        let nothing = |_: &str| None;
        let (s, _) = folders_verdict("PC", MAC, PC, &none, &[], &Origins::default(), &nothing);
        assert_eq!(s, "warning", "nothing maps yet");
    }
}
