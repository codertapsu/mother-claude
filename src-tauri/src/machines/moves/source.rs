//! The source's half of a move (docs/PLAN-MACHINES.md §5.8): the preflight,
//! then — in the background, one record walking its states — freezing,
//! packaging, sending, asking the target to land it, and committing the
//! lease. Before *landed* any failure (or a cancel) leaves both machines as
//! they were: the target drops what it got and the session starts again
//! here when it ran. *Take over* and asking the target to undo live here too.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::ignored;
use super::resume::{self, Spec};
use super::wire::{self, AccountAsk, CodeAsk};
use super::{
    call, file_path, injected, load, move_folder, peer_ref, refusal, save, this_machine, update,
    AccountPart, Active, Carried, Check, CodePart, FileEntry, MoveFailure, MoveRecord, MoveState,
    Role, CHUNK, LAND_TIMEOUT, PREFLIGHT_TIMEOUT, UNDO_KEEP_MS,
};
use crate::claude::backup::session_pack;
use crate::claude::control::{CloseRefused, Driver};
use crate::claude::home::ClaudeHome;
use crate::claude::owned;
use crate::machines::chunks::{self, PutError};
use crate::machines::code::{self, BranchRelation, CheckoutState, GitRunner};
use crate::machines::lease::{self, Lease};
use crate::machines::{self as m, protocol};
use crate::server::context::ClientContext;
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// How long a stopped process may take to end (as an account switch's).
const STOP_WAIT: Duration = Duration::from_secs(20);
/// How long an interrupted turn may take to end before the process stops.
const INTERRUPT_WAIT: Duration = Duration::from_secs(10);

/// `POST /api/sessions/{id}/move` and `…/move/preflight`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MoveBody {
    /// The computer to move it to (a machine id; this one's for a take over).
    pub to: String,
    /// `bundle` (the default: machine to machine) | `remote` (hidden refs on
    /// your remote) | `none` (only the conversation).
    pub code: Option<String>,
    /// `checkout` (the default: the project folder there, when clean) |
    /// `worktree` (always a separate one).
    pub apply: Option<String>,
    /// The project folder's branch there has commits this computer's lacks:
    /// replace it with this computer's (plan §5.10, *Replace the branch
    /// there*), its tip kept under the landing's undo ref for *Undo move*.
    /// Only the tip the preflight saw is replaced.
    pub replace_branch: Option<bool>,
    /// The folder there, when the user chose one.
    pub folder: Option<String>,
    /// Keep driving it from here (on by default; the screens' choice).
    pub keep_driving: Option<bool>,
    /// A copy goes on there under a new id; this one stays (a session
    /// another process runs moves only so).
    pub as_copy: Option<bool>,
    /// Ask Claude to run the project's recipe there.
    pub recipe: Option<bool>,
    /// Carry the folder's trust there (the dangerous right).
    pub trust: Option<bool>,
    /// Carry the files git ignores that the project lists: the dangerous
    /// right on both computers. Left unsaid, they travel when whoever moves
    /// it holds that right here, and stay otherwise; `false` leaves them.
    pub carry_ignored: Option<bool>,
    /// The session's home is offline: it lives here from now on.
    pub take_over: Option<bool>,
    /// A take over lands the copy this computer keeps of it (docs/
    /// PLAN-MACHINES.md §5.9): on by default when there is one; `false`
    /// goes on from what this computer's config directory has.
    pub from_copy: Option<bool>,
    /// `idle` (after this turn, the default) | `now` (interrupting it).
    pub when: Option<String>,
    /// The remote of a `remote` move (default `origin`).
    pub remote: Option<String>,
}

/// What a move was asked to do, checked.
#[derive(Clone, Debug)]
struct Plan {
    to: String,
    /// `bundle` | `remote` | `none`.
    code: String,
    /// `checkout` | `worktree`.
    apply: String,
    /// Replace the branch there when it went on there (`checkout` only).
    replace_branch: bool,
    remote: String,
    folder: Option<String>,
    keep_driving: bool,
    as_copy: bool,
    recipe: bool,
    trust: bool,
    /// `None`: as the caller's right says (`check`).
    carry_ignored: Option<bool>,
    when: String,
}

/// The choices in force for the project in `cwd` (D10: the setup's choice
/// for all machines, else the project's own), for what a move left unsaid.
fn fill_defaults(plan: &mut Plan, opts: &crate::machines::setup::options::Effective) {
    use crate::machines::store::{CodeApply, CodeTransfer};
    if plan.code.is_empty() {
        plan.code = match opts.code.transfer {
            CodeTransfer::Remote => "remote",
            CodeTransfer::Bundle => "bundle",
        }
        .into();
    }
    if plan.apply.is_empty() {
        plan.apply = match opts.code.apply {
            CodeApply::Worktree => "worktree",
            CodeApply::Checkout => "checkout",
        }
        .into();
    }
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::unprocessable("invalid_move", message)
}

fn parse(state: &AppState, body: &MoveBody) -> ApiResult<Plan> {
    let to = body.to.trim().to_string();
    if !m::ids::is_machine_id(&to) {
        return Err(invalid("Name the computer to move it to."));
    }
    if !state.machines.store_read(|f| f.peer(&to).is_some()) {
        return Err(ApiError::not_found(
            "machines_unknown",
            "Not a paired computer.",
        ));
    }
    // Left unsaid: the project's choice, filled once its folder is known.
    let code = match body.code.as_deref().map(str::trim) {
        None | Some("") => String::new(),
        Some("bundle" | "direct") => "bundle".into(),
        Some("remote") => "remote".into(),
        Some("none") => "none".into(),
        Some(other) => {
            return Err(invalid(format!(
                "Unknown `code` {other:?} (expected \"bundle\", \"remote\" or \"none\")."
            )))
        }
    };
    let apply = match body.apply.as_deref().map(str::trim) {
        None | Some("") => String::new(),
        Some(a @ ("checkout" | "worktree")) => a.to_string(),
        Some(other) => {
            return Err(invalid(format!(
                "Unknown `apply` {other:?} (expected \"checkout\" or \"worktree\")."
            )))
        }
    };
    let remote = body
        .remote
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .unwrap_or("origin")
        .to_string();
    if !code::valid_remote_name(&remote) {
        return Err(invalid("That is not a remote's name."));
    }
    let when = crate::claude::switch::When::parse(body.when.as_deref())?;
    Ok(Plan {
        to,
        code,
        apply,
        replace_branch: body.replace_branch.unwrap_or(false),
        remote,
        folder: body.folder.clone().filter(|f| !f.trim().is_empty()),
        keep_driving: body.keep_driving.unwrap_or(true),
        as_copy: body.as_copy.unwrap_or(false),
        recipe: body.recipe.unwrap_or(false),
        trust: body.trust.unwrap_or(false),
        carry_ignored: body.carry_ignored,
        when: match when {
            crate::claude::switch::When::Idle => "idle".into(),
            crate::claude::switch::When::Now => "now".into(),
        },
    })
}

/// The session here.
#[derive(Clone, Debug)]
struct Local {
    title: Option<String>,
    account: String,
    account_name: String,
    is_default: bool,
    home: ClaudeHome,
    facts: session_pack::Facts,
    owned: bool,
    bridge: bool,
    /// Another process runs it (a terminal, an editor).
    foreign: bool,
    surface: String,
    /// Where it works here.
    cwd: String,
    settings: Carried,
}

async fn local_facts(state: &AppState, session: &str) -> ApiResult<Local> {
    let summary = state.find_session(session).await;
    let account = crate::claude::summaries::acting_account(state, session);
    let home = state
        .accounts
        .home(&account)
        .unwrap_or_else(|| state.home.clone());
    let pairs = crate::claude::paths::local_pairs();
    let (h, id) = (home.clone(), session.to_string());
    let facts = tokio::task::spawn_blocking(move || session_pack::facts(&h, &id, &pairs))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .map_err(|e| match e {
            crate::claude::backup::BackupError::Transfer(
                crate::claude::transfer::TransferError::NotFound(_),
            ) => ApiError::not_found(
                "session_not_found",
                "Nothing of this session is on disk yet: it moves once it has a conversation.",
            ),
            e => ApiError::unprocessable(e.code(), e.to_string()),
        })?;
    let owned = owned::has_handle(state, session);
    let bridge = state.bridge.thread(session).await.is_some();
    let foreign = !owned && !bridge && crate::claude::transfer::ops::is_live(state, session);
    // Where it works here: where it runs; else where a move placed it here
    // (its transcript still names the other computer's folder until it
    // goes on here); else what its transcript says.
    let cwd = state
        .control
        .cwd_of(session)
        .or_else(|| landed_here(state, session))
        .or_else(|| facts.folder.clone())
        .or_else(|| facts.cwd.clone())
        .or_else(|| summary.as_ref().map(|s| s.cwd.clone()))
        .unwrap_or_default();
    let settings = match owned::live_state(state, session) {
        Some(live) => resume::carried(&live),
        None => summary.as_ref().map(resume::carried_of).unwrap_or_default(),
    };
    let acc = state.accounts.get(&account);
    Ok(Local {
        title: summary
            .as_ref()
            .map(|s| s.title.clone().unwrap_or_else(|| s.name.clone()))
            .filter(|t| !t.trim().is_empty()),
        account_name: acc.as_ref().map(|a| a.name.clone()).unwrap_or_default(),
        is_default: account == crate::claude::accounts::DEFAULT_ID,
        account,
        home,
        facts,
        owned,
        bridge,
        foreign,
        surface: summary
            .as_ref()
            .and_then(|s| s.entrypoint.clone())
            .unwrap_or_else(|| "another app".into()),
        cwd,
        settings,
    })
}

/// The code here.
#[derive(Clone, Debug)]
struct CodeHere {
    state: CheckoutState,
    /// The session's folder under the repository's top, `/`-separated.
    rel: String,
    /// The project's `.mother-claude/recipe.json`.
    recipe: Option<Value>,
}

async fn code_here(git: &GitRunner, cwd: &str) -> Option<Result<CodeHere, code::CodeError>> {
    if cwd.is_empty() || !Path::new(cwd).is_dir() {
        return None;
    }
    let (git, cwd) = (git.clone(), PathBuf::from(cwd));
    tokio::task::spawn_blocking(move || {
        let st = code::checkout_state(&git, &cwd)?;
        let rel = rel_of(&st.top, &cwd);
        let recipe = std::fs::metadata(st.top.join(".mother-claude").join("recipe.json"))
            .ok()
            .filter(|m| m.len() <= 64 * 1024)
            .and_then(|_| {
                crate::persist::read_json::<Value>(
                    &st.top.join(".mother-claude").join("recipe.json"),
                )
            });
        Ok(CodeHere {
            state: st,
            rel,
            recipe,
        })
    })
    .await
    .ok()
}

/// `cwd` under `top`, `/`-separated (`""`: the top itself).
fn rel_of(top: &Path, cwd: &Path) -> String {
    let canon = |p: &Path| dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(cwd)
        .strip_prefix(canon(top))
        .map(|r| {
            r.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_default()
}

/// The move sheet's checks (`POST /api/sessions/{id}/move/preflight`).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preflight {
    /// No blocker: the move may start.
    pub ok: bool,
    pub blockers: Vec<Check>,
    pub warnings: Vec<Check>,
    /// The session here: `{id, title, owned, live, foreign, account: {id,
    /// name}, cwd, folder, transcriptBytes, bytes, files, subagents,
    /// settings}`.
    pub session: Value,
    /// The computer it goes to: `{id, name, os, online}`.
    pub to: Value,
    /// The code: `{mode, apply, repo, top, branch, head, changed, untracked,
    /// origin, relation, ignored, ignoredBytes, carryIgnored,
    /// replaceBranch}` (`relation` of the target's branch to this base:
    /// `create`, `same`, `fastForward`, `diverged`; `ignored` the files git
    /// ignores that the project lists, as found here — they travel when
    /// `carryIgnored`; `replaceBranch` when a diverged branch there is
    /// replaced by this computer's).
    pub code: Value,
    /// What the other computer said (its account, folder, checkout, …).
    pub target: Option<wire::Answer>,
    /// The project's recipe for the target's system.
    pub recipe: Vec<String>,
    pub as_copy: bool,
}

/// Everything a start needs from its preflight.
struct Checked {
    pre: Preflight,
    plan: Plan,
    local: Local,
    code: Option<CodeHere>,
    answer: Option<wire::Answer>,
    hint: Option<String>,
    home_branch: Option<String>,
    /// The files git ignores that travel (none when they stay here).
    ignored: Vec<ignored::Found>,
    /// The tip of the branch there this move replaces (*Replace the branch
    /// there*, the branch having gone on there).
    replaces: Option<String>,
    /// What the target was asked.
    ask: wire::Ask,
}

/// `POST /api/sessions/{id}/move/preflight`: nothing is written anywhere.
pub async fn preflight(
    state: &AppState,
    ctx: &ClientContext,
    session: &str,
    body: &MoveBody,
) -> ApiResult<Preflight> {
    if body.take_over.unwrap_or(false) {
        return Err(invalid(
            "A take over has no preflight: it only moves the lease here.",
        ));
    }
    Ok(check(state, ctx, session, body).await?.pre)
}

async fn check(
    state: &AppState,
    ctx: &ClientContext,
    session: &str,
    body: &MoveBody,
) -> ApiResult<Checked> {
    if !owned::is_uuid(session) {
        return Err(ApiError::unprocessable(
            "invalid_session_id",
            "Only Claude Code sessions (UUID ids) move to another computer.",
        ));
    }
    let mut plan = parse(state, body)?;
    // It moves from where it lives.
    lease::require_here(state, session)?;
    let to = peer_ref(state, &plan.to);
    let local = local_facts(state, session).await?;
    let opts = crate::machines::setup::options::effective(state, &local.cwd);
    fill_defaults(&mut plan, &opts);
    let git = state.machines.moves.git();
    let mut blockers: Vec<Check> = Vec::new();
    let mut warnings: Vec<Check> = Vec::new();

    // The session here.
    if local.bridge {
        blockers.push(Check::new(
            "session_bridge",
            "A conversation of the API can't move to another computer yet.",
        ));
    }
    if local.foreign && !plan.as_copy {
        blockers.push(
            Check::new(
                "session_foreign",
                format!(
                    "It runs in {} — Mother Claude can't stop it. A copy can go on on {} as a new \
                     session while this one goes on here, or end it there first.",
                    local.surface, to.name
                ),
            )
            .offer("as-copy"),
        );
    }
    let dangerous_mode = owned::is_dangerous_mode(local.settings.permission_mode.as_deref());
    if dangerous_mode && !ctx.may_dangerous() {
        blockers.push(Check::new(
            "dangerous_mode",
            "It runs without permission prompts: moving it needs the dangerous right here.",
        ));
    }
    if plan.trust && !ctx.may_dangerous() {
        blockers.push(Check::new(
            "trust_needs_dangerous",
            "Carrying a folder's trust needs the dangerous permission (set on the desktop).",
        ));
    }

    // The code here.
    let mut code_mode = plan.code.clone();
    let mut code = None;
    if code_mode != "none" {
        match code_here(&git, &local.cwd).await {
            Some(Ok(c)) => {
                if let Some(op) = &c.state.busy {
                    blockers.push(Check::new(
                        "source_busy",
                        format!(
                            "A git operation is under way in {} ({op}): finish it first, or move \
                             only the conversation.",
                            c.state.top.display()
                        ),
                    ));
                }
                if code_mode == "remote" && !c.state.remotes.contains(&plan.remote) {
                    blockers.push(Check::new(
                        "remote_missing",
                        format!(
                            "This repository has no remote named {}: move the code machine to \
                             machine instead.",
                            plan.remote
                        ),
                    ));
                } else if code_mode == "remote" {
                    // It must answer without a prompt (the setup's check,
                    // made again before this move).
                    let r = crate::machines::setup::folders::remote_named_answers_async(
                        git.clone(),
                        c.state.top.clone(),
                        plan.remote.clone(),
                    )
                    .await;
                    if r.state != "ok" {
                        blockers.push(
                            Check::new(
                                "remote_unreachable",
                                format!(
                                    "The remote {} doesn't answer from this computer without a \
                                     prompt.",
                                    plan.remote
                                ),
                            )
                            .fix(r.detail),
                        );
                    }
                }
                code = Some(c);
            }
            Some(Err(code::CodeError::NotARepository(_))) | None => {
                warnings.push(Check::new(
                    "not_a_repo",
                    format!(
                        "{} is not in a git repository: only the conversation goes.",
                        if local.cwd.is_empty() {
                            "Its folder"
                        } else {
                            &local.cwd
                        }
                    ),
                ));
                code_mode = "none".into();
            }
            Some(Err(e)) => {
                blockers.push(Check::new("code_unreadable", e.to_string()));
            }
        }
    }

    // The files git ignores that the project lists to carry: found here
    // (only plain files git ignores, never through a link), shown either
    // way; carrying them is a dangerous act, here and there.
    let listed: Vec<String> = opts
        .recipe
        .as_ref()
        .map(|r| r.carry_ignored.clone())
        .unwrap_or_default();
    let carry = plan.carry_ignored.unwrap_or_else(|| ctx.may_dangerous());
    let mut found: Vec<ignored::Found> = Vec::new();
    if let Some(c) = code
        .as_ref()
        .filter(|_| code_mode != "none" && !listed.is_empty())
    {
        let (g, top, rel, l) = (git.clone(), c.state.top.clone(), c.rel.clone(), listed);
        let scan = tokio::task::spawn_blocking(move || ignored::scan(&g, &top, &rel, &l))
            .await
            .unwrap_or_default();
        found = scan.found;
        if carry {
            blockers.extend(scan.problems);
        }
    }
    let carrying = carry && !found.is_empty();
    if carrying && !ctx.may_dangerous() {
        blockers.push(Check::new(
            "ignored_needs_dangerous",
            "Carrying files git ignores needs the dangerous permission (set on the desktop).",
        ));
    }

    // Where it goes home to, when it came from there (a worktree there, or a
    // folder no pair maps).
    let came = came_from(state, session, &plan.to);
    let hint = came.as_ref().map(|c| c.0.clone());
    let in_worktree = super::worktrees_root(state)
        .is_some_and(|root| crate::claude::paths::is_within(Path::new(&local.cwd), &root));
    // Only from a worktree it lived in here.
    let home_branch = came.and_then(|c| c.1).filter(|_| in_worktree);

    // Ask the other computer. The recipe set for the project in Mother
    // Claude, else the one the repository commits.
    let recipe: Vec<String> = opts
        .recipe
        .as_ref()
        .map(|r| super::note::clean_recipe(r.after_move.for_os(&to.os)))
        .filter(|r| !r.is_empty())
        .or_else(|| {
            code.as_ref()
                .and_then(|c| c.recipe.as_ref())
                .map(|r| super::note::recipe_for(r, &to.os))
        })
        .unwrap_or_default();
    let ask = wire::Ask {
        session: session.to_string(),
        as_copy: plan.as_copy,
        account: AccountAsk {
            name: local.account_name.clone(),
            is_default: local.is_default,
        },
        os: protocol::this_os().to_string(),
        cwd: local.cwd.clone(),
        folder: plan.folder.clone(),
        hint: hint.clone(),
        code: code
            .as_ref()
            .filter(|_| code_mode != "none")
            .map(|c| CodeAsk {
                mode: code_mode.clone(),
                apply: plan.apply.clone(),
                remote: (code_mode == "remote").then(|| plan.remote.clone()),
                rel: c.rel.clone(),
                branch: home_branch.clone().or_else(|| c.state.branch.clone()),
                origin: c.state.origin.clone(),
                ignored: if carrying {
                    found.iter().map(|f| f.path.clone()).collect()
                } else {
                    Vec::new()
                },
            }),
        trust: plan.trust,
        permission_mode: local.settings.permission_mode.clone(),
        bytes: local.facts.bytes,
        claude: m::setup::claude_version(state),
        epoch: state
            .machines
            .leases
            .get(session)
            .map(|l| l.epoch)
            .unwrap_or(0),
    };
    let acting = m::acting::header_for(state, ctx);
    let answer = match ask_target(state, &plan.to, &ask, &acting).await {
        Ok(a) => {
            blockers.extend(a.blockers.iter().cloned());
            warnings.extend(a.warnings.iter().cloned());
            Some(a)
        }
        Err(b) => {
            blockers.push(b);
            None
        }
    };
    if carrying && answer.is_some() {
        let takes = state.machines.store_read(|f| {
            f.peer(&plan.to)
                .is_some_and(|p| p.capabilities.iter().any(|c| c == protocol::IGNORED_FILES))
        });
        if !takes {
            blockers.push(
                Check::new(
                    "ignored_unsupported",
                    format!(
                        "{}'s Mother Claude can't take files git ignores yet.",
                        to.name
                    ),
                )
                .fix(format!("Update Mother Claude on {}.", to.name)),
            );
        }
    }

    // The branch there against this base.
    let mut relation = None;
    let mut replaces = None;
    if let (Some(c), Some(a)) = (&code, &answer) {
        if let Some(there) = a.code.as_ref().filter(|t| t.repo) {
            let base = c.state.head.clone();
            let tip = there.branch_tip.clone();
            let (g, top) = (git.clone(), c.state.top.clone());
            let seen = tip.clone();
            let r = tokio::task::spawn_blocking(move || {
                code::branch_relation(&g, &top, base.as_deref(), tip.as_deref())
            })
            .await
            .ok()
            .and_then(Result::ok);
            if r == Some(BranchRelation::Diverged) && plan.apply == "checkout" {
                let branch = c
                    .state
                    .branch
                    .as_deref()
                    .map(|b| b.trim_start_matches("refs/heads/"))
                    .unwrap_or("Its branch");
                // Replacing it lands only where the other computer can, and
                // only the tip seen here (the landing stops if it moved).
                let can_replace = state.machines.store_read(|f| {
                    f.peer(&plan.to).is_some_and(|p| {
                        p.capabilities.iter().any(|c| c == protocol::REPLACE_BRANCH)
                    })
                });
                match seen.filter(|t| code::is_oid(t)) {
                    Some(tip) if plan.replace_branch && can_replace => {
                        warnings.push(Check::new(
                            "branch_replaced",
                            format!(
                                "{branch} on {} is replaced by this computer's: its own commits \
                                 there are kept for Undo move.",
                                to.name
                            ),
                        ));
                        replaces = Some(tip);
                    }
                    _ if plan.replace_branch => blockers.push(
                        Check::new(
                            "replace_unsupported",
                            format!("{}'s Mother Claude can't replace a branch yet.", to.name),
                        )
                        .fix(format!("Update Mother Claude on {}.", to.name))
                        .offer("worktree"),
                    ),
                    _ => blockers.push(
                        Check::new(
                            "branch_diverged",
                            format!("{branch} on {} has commits this computer doesn't.", to.name),
                        )
                        .fix(if can_replace {
                            "Use a separate folder (worktree) there, or replace the branch there \
                             with this computer's (its commits are kept for Undo move)."
                        } else {
                            "Use a separate folder (worktree) there."
                        })
                        .offer(if can_replace {
                            "worktree-or-reset"
                        } else {
                            "worktree"
                        })
                        .detail(json!({ "branch": branch, "tip": there.branch_tip })),
                    ),
                }
            }
            if let (Some(mine), Some(theirs)) = (&c.state.origin, &there.origin) {
                if !same_origin(mine, theirs) {
                    blockers.push(
                        Check::new(
                            "origin_differs",
                            format!(
                                "The folder on {} is another repository (its origin is {theirs}).",
                                to.name
                            ),
                        )
                        .fix("Choose the project's folder there.")
                        .offer("choose-folder"),
                    );
                }
            }
            relation = r;
        }
    }
    if let (Some(mine), Some(theirs)) = (
        m::setup::claude_version(state),
        answer.as_ref().and_then(|a| a.claude.clone()),
    ) {
        if older(&theirs, &mine) {
            warnings.push(Check::new(
                "claude_older",
                format!(
                    "Claude Code on {} is older ({theirs}, here {mine}).",
                    to.name
                ),
            ));
        }
    }

    let online = state.machines.link(&plan.to).online;
    let pre = Preflight {
        ok: blockers.is_empty(),
        blockers,
        warnings,
        session: json!({
            "id": session,
            "title": local.title,
            "owned": local.owned,
            "live": local.owned || local.foreign || local.bridge,
            "foreign": local.foreign,
            "account": { "id": local.account, "name": local.account_name },
            "cwd": local.cwd,
            "folder": local.facts.folder,
            "transcriptBytes": local.facts.transcript_bytes,
            "bytes": local.facts.bytes,
            "files": local.facts.files,
            "subagents": local.facts.subagents,
            "settings": local.settings,
        }),
        to: json!({ "id": to.id, "name": to.name, "os": to.os, "online": online }),
        code: match &code {
            Some(c) if code_mode != "none" => json!({
                "mode": code_mode,
                "apply": plan.apply,
                "repo": true,
                "top": c.state.top,
                "branch": c.state.branch,
                "head": c.state.head,
                "changed": c.state.changed,
                "untracked": c.state.untracked,
                "origin": c.state.origin,
                "relation": relation,
                "ignored": found.iter().map(|f| f.listed.as_str()).collect::<Vec<_>>(),
                "ignoredBytes": found.iter().map(|f| f.size).sum::<u64>(),
                "carryIgnored": carrying,
                "replaceBranch": replaces.is_some(),
            }),
            _ => json!({ "mode": "none", "repo": code.is_some() }),
        },
        target: answer.clone(),
        recipe,
        as_copy: plan.as_copy,
    };
    plan.code = code_mode;
    Ok(Checked {
        pre,
        plan,
        local,
        code,
        answer,
        hint,
        home_branch,
        ignored: if carrying { found } else { Vec::new() },
        replaces,
        ask,
    })
}

/// Two remotes' URLs name the same repository (`.git` and a trailing `/`
/// aside, the host's case aside).
fn same_origin(a: &str, b: &str) -> bool {
    let norm = |u: &str| {
        u.trim()
            .trim_end_matches('/')
            .trim_end_matches(".git")
            .to_ascii_lowercase()
    };
    norm(a) == norm(b)
}

/// Whether version `a` is older than `b` (`2.1.322` < `2.1.330`).
fn older(a: &str, b: &str) -> bool {
    let nums = |v: &str| -> Vec<u64> {
        v.split(|c: char| !c.is_ascii_digit())
            .filter(|s| !s.is_empty())
            .take(4)
            .filter_map(|s| s.parse().ok())
            .collect()
    };
    nums(a) < nums(b)
}

/// The folder the last move that brought this session here placed it in,
/// while it is a folder here.
fn landed_here(state: &AppState, session: &str) -> Option<String> {
    super::of_session(state, session)
        .into_iter()
        .find(|r| {
            r.role == Role::In
                && !r.as_copy
                && matches!(r.state, MoveState::Committed | MoveState::Resumed)
        })
        .and_then(|r| r.target_cwd)
        .filter(|c| Path::new(c).is_dir())
}

/// Where a session that came here from `to` goes back to there, and the
/// branch it came on: the incoming move that brought it.
fn came_from(state: &AppState, session: &str, to: &str) -> Option<(String, Option<String>)> {
    super::of_session(state, session)
        .into_iter()
        .find(|r| r.role == Role::In && r.from.id == to && !r.as_copy)
        .filter(|r| !r.source_cwd.is_empty())
        .map(|r| (r.source_cwd, r.code.and_then(|c| c.branch)))
}

/// Asks the target's preflight.
async fn ask_target(
    state: &AppState,
    to: &str,
    ask: &wire::Ask,
    acting: &str,
) -> Result<wire::Answer, Check> {
    let name = m::peer_name(state, to);
    if !speaks_moves(state, to).await {
        return Err(Check::new(
            "moves_unsupported",
            format!("{name}'s Mother Claude can't take sessions yet."),
        )
        .fix(format!("Update Mother Claude on {name}.")));
    }
    let body = serde_json::to_value(ask).unwrap_or(Value::Null);
    match call(
        state,
        to,
        "POST",
        "/peer/moves/preflight",
        Some(&body),
        acting,
        PREFLIGHT_TIMEOUT,
    )
    .await
    {
        Err(e) => Err(Check::new(
            "target_offline",
            format!("{name} can't be reached right now: {e}"),
        )
        .fix(format!(
            "Check that Mother Claude runs on {name} and that both computers are online."
        ))),
        Ok((200, v)) => serde_json::from_value(v)
            .map_err(|e| Check::new("target_answer", format!("{name} answered oddly: {e}"))),
        Ok((404, _)) => Err(Check::new(
            "moves_unsupported",
            format!("{name}'s Mother Claude can't take sessions yet."),
        )
        .fix(format!("Update Mother Claude on {name}."))),
        Ok((status, v)) => {
            let e = refusal(status, &v, &name);
            let mut c = Check::new(&e.code, e.message.clone());
            if e.code == "grant_missing" {
                c.message = format!(
                    "{name} has not given this computer the Move permission{}.",
                    if state.machines.link(to).path == Some(m::transport::PathKind::Lan) {
                        " on the local network"
                    } else {
                        " over the internet"
                    }
                );
                c = c.fix(format!("Change it on {name}'s Machines page."));
            }
            Err(c)
        }
    }
}

/// Whether the other machine takes sessions (its hello listed `moves`); an
/// older one, or one never greeted, is greeted once first.
async fn speaks_moves(state: &AppState, to: &str) -> bool {
    let has = |st: &AppState| {
        st.machines.store_read(|f| {
            f.peer(to)
                .is_some_and(|p| p.capabilities.iter().any(|c| c == protocol::MOVES))
        })
    };
    if has(state) {
        return true;
    }
    let Some(key) = state
        .machines
        .store_read(|f| f.peer(to).map(|p| p.key.clone()))
    else {
        return false;
    };
    m::greet(state.clone(), to.to_string(), key).await;
    has(state)
}

// ─── starting ──────────────────────────────────────────────────────────────────

/// `POST /api/sessions/{id}/move`: checks again, then runs in the background
/// (`202` with the record); a take over is done at once (`200`).
pub async fn start(
    state: &AppState,
    ctx: &ClientContext,
    session: &str,
    body: &MoveBody,
) -> ApiResult<(StatusCode, Value)> {
    if body.take_over.unwrap_or(false) {
        return take_over(state, ctx, session, body).await;
    }
    if super::moves_dir(state).is_none() {
        return Err(ApiError::internal("This build has no data directory."));
    }
    let checked = check(state, ctx, session, body).await?;
    if !checked.pre.ok {
        let first = checked
            .pre
            .blockers
            .first()
            .map(|b| b.message.clone())
            .unwrap_or_default();
        return Err(
            ApiError::conflict("move_blocked", format!("It can't move yet: {first}")).with(
                "preflight",
                serde_json::to_value(&checked.pre).unwrap_or(Value::Null),
            ),
        );
    }
    if checked.plan.trust {
        crate::server::auth::require_dangerous(
            ctx,
            "Carrying a folder's trust to another computer",
        )?;
    }
    if owned::is_dangerous_mode(checked.local.settings.permission_mode.as_deref()) {
        crate::server::auth::require_dangerous(
            ctx,
            "Moving a session that runs without permission prompts",
        )?;
    }
    if !checked.ignored.is_empty() {
        crate::server::auth::require_dangerous(
            ctx,
            "Carrying files git ignores to another computer",
        )?;
    }
    let id = super::new_move_id();
    let acting = m::acting::header_for(state, ctx);
    let Some(active) = state
        .machines
        .moves
        .begin(&id, session, &checked.plan.to, acting)
    else {
        return Err(ApiError::conflict(
            "move_underway",
            "This session is moving already.",
        ));
    };
    let me = this_machine(state);
    let to = peer_ref(state, &checked.plan.to);
    let epoch = state
        .machines
        .leases
        .get(session)
        .map(|l| l.epoch)
        .unwrap_or(0)
        + 1;
    let now = m::now();
    let mut rec = MoveRecord {
        id: id.clone(),
        role: Role::Out,
        session: session.to_string(),
        target_session: None,
        title: checked.local.title.clone(),
        from: me,
        to,
        state: MoveState::Preflight,
        as_copy: checked.plan.as_copy,
        keep_driving: checked.plan.keep_driving,
        when: checked.plan.when.clone(),
        account: AccountPart {
            name: checked.local.account_name.clone(),
            is_default: checked.local.is_default,
            source: checked.local.account.clone(),
            target: checked
                .answer
                .as_ref()
                .and_then(|a| a.account.as_ref())
                .map(|a| a.id.clone()),
        },
        epoch: if checked.plan.as_copy { 0 } else { epoch },
        source_cwd: checked.local.cwd.clone(),
        target_cwd: checked.answer.as_ref().and_then(|a| a.cwd.clone()),
        target_folder: checked
            .answer
            .as_ref()
            .and_then(|a| a.folder.as_ref())
            .map(|f| f.path.clone()),
        code: checked
            .code
            .as_ref()
            .filter(|_| checked.plan.code != "none")
            .map(|c| CodePart {
                mode: checked.plan.code.clone(),
                apply: checked.plan.apply.clone(),
                remote: (checked.plan.code == "remote").then(|| checked.plan.remote.clone()),
                rel: c.rel.clone(),
                branch: checked
                    .home_branch
                    .clone()
                    .or_else(|| c.state.branch.clone()),
                base: c.state.head.clone(),
                changed: c.state.changed,
                untracked: c.state.untracked,
                ignored: checked.ignored.iter().map(|f| f.path.clone()).collect(),
                replaces: checked.replaces.clone(),
                ..CodePart::default()
            }),
        files: Vec::new(),
        bytes_total: 0,
        bytes_sent: 0,
        conversation_bytes: checked.local.facts.bytes,
        settings: checked.local.settings.clone(),
        was_running: false,
        held: Vec::new(),
        recipe: if checked.plan.recipe {
            checked.pre.recipe.clone()
        } else {
            Vec::new()
        },
        trust: checked.plan.trust,
        from_copy: false,
        steps: Vec::new(),
        error: None,
        warnings: checked.pre.warnings.clone(),
        landing: None,
        resume_error: None,
        by: super::who(state, ctx),
        dangerous: ctx.may_dangerous(),
        created_at: now,
        updated_at: now,
        finished_at: None,
        undo_until: None,
        worktree_removed_at: None,
    };
    rec.reach(MoveState::Preflight);
    save(state, &rec);
    super::audit(
        state,
        "machine.move.start",
        &rec,
        json!({ "code": checked.plan.code, "apply": checked.plan.apply }),
    );
    crate::server::auth::note_device_action(
        state,
        ctx,
        &format!(
            "started moving {} to {}",
            rec.title.as_deref().unwrap_or("a session"),
            rec.to.name
        ),
    );
    let view = rec.view();
    let prep = Prepared {
        home: checked.local.home.clone(),
        hint: checked.hint.clone(),
        haves: checked
            .answer
            .as_ref()
            .and_then(|a| a.code.as_ref())
            .map(|c| c.haves.clone())
            .unwrap_or_default(),
        folder: checked.plan.folder.clone(),
        top: checked.code.as_ref().map(|c| c.state.top.clone()),
        ignored: checked.ignored.clone(),
        ask: checked.ask.clone(),
        remote: checked.plan.remote.clone(),
    };
    let st = state.clone();
    tokio::spawn(async move {
        run(&st, &id, active, prep).await;
    });
    Ok((StatusCode::ACCEPTED, view))
}

/// What the background run carries from the preflight.
struct Prepared {
    home: ClaudeHome,
    hint: Option<String>,
    haves: Vec<String>,
    folder: Option<String>,
    /// The repository's top here.
    top: Option<PathBuf>,
    /// The files git ignores that travel.
    ignored: Vec<ignored::Found>,
    /// What the preflight asked the target (asked again, through the
    /// remote, before a bundle over the limit goes that way instead).
    ask: wire::Ask,
    /// The remote the code would go through (`origin` unless named).
    remote: String,
}

/// Why a move stopped before it landed.
#[derive(Debug, Clone)]
struct Fail {
    code: String,
    message: String,
    offer: Option<String>,
}

impl Fail {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            offer: None,
        }
    }
}

impl From<ApiError> for Fail {
    fn from(e: ApiError) -> Self {
        Self {
            offer: e
                .extra
                .get("offer")
                .and_then(Value::as_str)
                .map(str::to_string),
            code: e.code,
            message: e.message,
        }
    }
}

fn cancelled(active: &Active) -> Result<(), Fail> {
    if active.cancel.load(Ordering::SeqCst) {
        Err(Fail::new("cancelled", "It was cancelled."))
    } else {
        Ok(())
    }
}

fn reach(state: &AppState, id: &str, s: MoveState) {
    update(state, id, |r| r.reach(s));
}

async fn run(state: &AppState, id: &str, active: Arc<Active>, prep: Prepared) {
    let session = load(state, id).map(|r| r.session).unwrap_or_default();
    match drive(state, id, &active, &prep).await {
        Ok(()) => {}
        Err(fail) => {
            failed(state, id, fail, &active.acting).await;
        }
    }
    state.machines.moves.end(id, &session);
}

async fn drive(state: &AppState, id: &str, active: &Active, prep: &Prepared) -> Result<(), Fail> {
    let rec =
        load(state, id).ok_or_else(|| Fail::new("move_lost", "The move's record is gone."))?;
    reach(state, id, MoveState::Freezing);
    cancelled(active)?;
    injected(state, "freeze")?;
    if !rec.as_copy {
        freeze(state, id, active).await?;
    }
    cancelled(active)?;
    reach(state, id, MoveState::Packaging);
    injected(state, "package")?;
    package(state, id, prep, &active.acting).await?;
    cancelled(active)?;
    reach(state, id, MoveState::Sending);
    send(state, id, active, prep).await?;
    cancelled(active)?;
    reach(state, id, MoveState::Landing);
    let landed = land(state, id, &active.acting).await?;
    // Landed: from here on nothing is undone on its own.
    let rec = update(state, id, |r| {
        r.target_cwd = Some(landed.target_cwd.clone());
        r.target_folder = landed.target_folder.clone().or(r.target_folder.take());
        r.target_session = Some(landed.target_session.clone());
        if landed.account.is_some() {
            r.account.target = landed.account.clone();
        }
        r.landing = Some(landed.landing.clone());
        if let (Some(c), Some(applied)) = (r.code.as_mut(), landed.code.clone()) {
            c.landed = Some(applied);
        }
        r.warnings.extend(landed.warnings.iter().cloned());
        r.reach(MoveState::Landed);
    })
    .ok_or_else(|| Fail::new("move_lost", "The move's record is gone."))?;
    super::audit(
        state,
        "machine.move.landed",
        &rec,
        json!({ "action": landed.landing.action }),
    );
    // Sent and placed: the package here is no longer needed (the target
    // keeps what undo needs; the code's refs stay in the repository).
    if let Some(dir) = move_folder(state, id) {
        let _ = std::fs::remove_dir_all(dir);
    }
    if rec.as_copy {
        copied(state, id, &landed, &active.acting).await;
        return Ok(());
    }
    finish(state, id, &active.acting).await;
    Ok(())
}

/// Freezes an owned session as an account switch does (decision 5): after
/// this turn, or now (what was queued behind it taken back and carried),
/// then stopped and reaped, its approvals withdrawn. A session nobody runs
/// needs nothing.
async fn freeze(state: &AppState, id: &str, active: &Active) -> Result<(), Fail> {
    let rec =
        load(state, id).ok_or_else(|| Fail::new("move_lost", "The move's record is gone."))?;
    let session = rec.session.clone();
    if !owned::has_handle(state, &session) {
        if crate::claude::transfer::ops::is_live(state, &session) {
            return Err(Fail::new(
                "session_live",
                "Another process started running it meanwhile, so it did not move.",
            ));
        }
        return Ok(());
    }
    let idle_only = rec.when != "now";
    let closing = loop {
        match state.control.close_for_switch(&session, idle_only) {
            Ok(c) => break Some(c),
            Err(CloseRefused::Gone) => break None,
            Err(CloseRefused::Busy) => {
                cancelled(active)?;
                let _ =
                    tokio::time::timeout(Duration::from_millis(200), active.wake.notified()).await;
            }
        }
    };
    let Some(closing) = closing else {
        return Ok(());
    };
    let mut held = Vec::new();
    if !idle_only && closing.driver == Driver::Stream {
        for q in &closing.live.queued {
            let Some(content) = closing.queued.get(&q.id) else {
                continue;
            };
            if owned::cancel_queued(state, &session, &q.id).await.is_ok() {
                held.push(content.text.clone());
            }
        }
        if state.control.is_idle(&session) == Some(false) {
            if let Err(e) = owned::interrupt(state, &session).await {
                tracing::debug!(session = %session, error = %e.message, "interrupt before a move");
            }
            let deadline = Instant::now() + INTERRUPT_WAIT;
            while state.control.is_idle(&session) == Some(false) && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
    let settings = resume::carried(&closing.live);
    if !state.control.stop_and_wait(&session, STOP_WAIT).await {
        state.control.reopen(&session);
        return Err(Fail::new(
            "stop_failed",
            "The session's process did not stop in time, so it did not move: a second process on \
             the same conversation would write over it.",
        ));
    }
    crate::pending::withdraw_session(state, &session, "moving to another computer").await;
    crate::server::monitor::tick_once(state).await;
    let cwd = closing.cwd.clone();
    update(state, id, |r| {
        r.was_running = true;
        r.settings = settings;
        r.held = held;
        r.source_cwd = cwd;
    });
    Ok(())
}

/// The list of the session's files a move that lands from the target's copy
/// sends (in the move's folder).
const COPY_LIST: &str = "copy-files.json";

/// The conversation packed (`session.tar.gz`): a turn that writes meanwhile
/// is waited out a little.
async fn pack_conversation(
    dir: &Path,
    rec: &MoveRecord,
    prep: &Prepared,
) -> Result<FileEntry, Fail> {
    let pairs = crate::claude::paths::local_pairs();
    let mut packed = None;
    for attempt in 0..3 {
        let (home, session, out, pairs) = (
            prep.home.clone(),
            rec.session.clone(),
            dir.join(session_pack::FILE),
            pairs.clone(),
        );
        let r =
            tokio::task::spawn_blocking(move || session_pack::pack(&home, &session, &out, &pairs))
                .await
                .map_err(|e| Fail::new("package_failed", e.to_string()))?;
        match r {
            Ok(p) => {
                packed = Some(p);
                break;
            }
            Err(crate::claude::backup::BackupError::Changed(_)) if attempt < 2 => {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => return Err(Fail::new(e.code(), e.to_string())),
        }
    }
    let packed = packed
        .ok_or_else(|| Fail::new("session_changed", "It kept changing while it was packed."))?;
    Ok(FileEntry {
        name: session_pack::FILE.to_string(),
        size: packed.size,
        sha256: packed.sha256.clone(),
        sent: 0,
    })
}

/// Whether a move's code may go through the remote instead of a bundle
/// (plan §5.10): the repository here has the remote and it answers here
/// without a prompt, and the target — asked its preflight again, through
/// the remote — has it too and it answers there. The remote's name.
async fn remote_answers(
    state: &AppState,
    rec: &MoveRecord,
    prep: &Prepared,
    acting: &str,
) -> Option<String> {
    let remote = prep.remote.clone();
    let top = prep.top.clone()?;
    let here = crate::machines::setup::folders::remote_named_answers_async(
        state.machines.moves.git(),
        top,
        remote.clone(),
    )
    .await;
    if here.state != "ok" {
        return None;
    }
    let mut ask = prep.ask.clone();
    let c = ask.code.as_mut()?;
    c.mode = "remote".into();
    c.remote = Some(remote.clone());
    let answer = ask_target(state, &rec.to.id, &ask, acting).await.ok()?;
    let there = answer
        .code
        .as_ref()
        .is_some_and(|t| t.repo && t.remotes.contains(&remote))
        && !answer
            .blockers
            .iter()
            .any(|b| b.code.starts_with("remote_"));
    there.then_some(remote)
}

/// The conversation (`session.tar.gz`, or the list of its files when the
/// target lands from its copy) and the code (a snapshot: its `meta.json`, a
/// bundle, Git LFS objects) in the move's folder.
async fn package(state: &AppState, id: &str, prep: &Prepared, acting: &str) -> Result<(), Fail> {
    let rec =
        load(state, id).ok_or_else(|| Fail::new("move_lost", "The move's record is gone."))?;
    let dir = move_folder(state, id)
        .ok_or_else(|| Fail::new("no_data_dir", "This build has no data directory."))?;
    std::fs::create_dir_all(&dir).map_err(|e| Fail::new("package_failed", e.to_string()))?;
    crate::claude::transfer::private_dir(&dir);
    let mut files = Vec::new();
    // The conversation: when the target keeps a copy of it current (§5.9),
    // the copy is brought up to date and only the list of every file with
    // its SHA-256 travels — the target lands from its copy once it checked
    // it against that list. A copy of a session another process runs forks
    // there from the package instead.
    let listed = if rec.as_copy {
        None
    } else {
        crate::machines::copies::source::for_move(state, &rec.session, &rec.to.id).await
    };
    if let Some(list) = listed {
        let spec: Vec<wire::FileSpec> = list
            .into_iter()
            .map(|(name, size, sha256)| wire::FileSpec { name, size, sha256 })
            .collect();
        crate::persist::write_json_private(&dir.join(COPY_LIST), &spec)
            .map_err(|e| Fail::new("package_failed", e.to_string()))?;
        update(state, id, |r| r.from_copy = true);
    } else {
        let _ = std::fs::remove_file(dir.join(COPY_LIST));
        update(state, id, |r| r.from_copy = false);
        files.push(pack_conversation(&dir, &rec, prep).await?);
    }
    // The code.
    let mut code = rec.code.clone();
    if let Some(c) = code.as_mut() {
        let git = state.machines.moves.git();
        let mut req = code::SnapshotRequest::new(
            &rec.session,
            // Unique per snapshot: a move tried again never meets its own refs.
            (m::now().max(0) as u64).max(1),
            &m::this_name(state),
            dir.join("code"),
        );
        req.transfer = if c.mode == "remote" {
            code::Transfer::Remote {
                remote: c.remote.clone().unwrap_or_else(|| "origin".into()),
                push_branch: false,
            }
        } else {
            code::Transfer::Bundle
        };
        req.haves = prep.haves.clone();
        req.branch = c.branch.clone().filter(|b| code::valid_branch(b));
        let cwd = PathBuf::from(&rec.source_cwd);
        let snapshot = |req: code::SnapshotRequest| {
            let (git, cwd) = (git.clone(), cwd.clone());
            async move {
                tokio::task::spawn_blocking(move || code::snapshot(&git, &cwd, &req))
                    .await
                    .map_err(|e| Fail::new("code_snapshot", e.to_string()))?
                    .map_err(|e| {
                        Fail::new(
                            "code_snapshot",
                            format!("The code could not be packed: {e}"),
                        )
                    })
            }
        };
        let fallback = req.clone();
        let mut meta = snapshot(req).await?;
        // Machine to machine, a bundle over the limit goes through the
        // remote instead when both computers reach it without a prompt
        // (plan §5.10) — and the record says so.
        let limit = state.machines.moves.bundle_limit();
        if let Some(size) = meta.bundle.as_ref().map(|b| b.size).filter(|s| *s > limit) {
            if let Some(remote) = remote_answers(state, &rec, prep, acting).await {
                let _ = std::fs::remove_file(dir.join("code").join(code::BUNDLE));
                let mut again = fallback;
                again.epoch = again.epoch.saturating_add(1);
                again.transfer = code::Transfer::Remote {
                    remote: remote.clone(),
                    push_branch: false,
                };
                again.haves = Vec::new();
                let first = meta.epoch.clone();
                meta = snapshot(again).await?;
                // The bundle's snapshot never travels: its refs go.
                let (g, top, session) =
                    (state.machines.moves.git(), cwd.clone(), rec.session.clone());
                let _ = tokio::task::spawn_blocking(move || {
                    code::drop_snapshot(&g, &top, &session, &first)
                })
                .await;
                c.mode = "remote".into();
                c.remote = Some(remote.clone());
                let note = Check::new(
                    "code_through_remote",
                    format!(
                        "The code went through your remote {remote}: machine to machine it would \
                         have been {} (over {}).",
                        crate::machines::copies::target::human(size),
                        crate::machines::copies::target::human(limit)
                    ),
                );
                update(state, id, |r| r.warnings.push(note.clone()));
                super::audit(
                    state,
                    "machine.move.through_remote",
                    &rec,
                    json!({ "bundleBytes": size, "remote": remote }),
                );
            }
        }
        c.branch = meta.branch.clone();
        c.base = meta.base.clone();
        c.snapshot = Some(meta.snap.clone());
        c.index_tree = Some(meta.idx_tree.clone());
        c.warnings = meta.warning_kinds().iter().map(|k| k.to_string()).collect();
        let mut names = vec![code::META.to_string()];
        if meta.bundle.is_some() {
            names.push(code::BUNDLE.to_string());
        }
        if let Some(l) = &meta.lfs {
            names.extend(l.carried.iter().map(|o| format!("lfs.{o}")));
        }
        for name in names {
            let path = file_path(&dir, &name)
                .ok_or_else(|| Fail::new("package_failed", format!("{name} has no place")))?;
            let (size, sha256) = crate::claude::transfer::sha256_file(&path)
                .map_err(|e| Fail::new("package_failed", format!("{name}: {e}")))?;
            files.push(FileEntry {
                name,
                size,
                sha256,
                sent: 0,
            });
        }
        // The files git ignores that travel, read again as they are now.
        c.ignored.clear();
        if let (Some(top), false) = (&prep.top, prep.ignored.is_empty()) {
            let (top, list, out) = (top.clone(), prep.ignored.clone(), dir.join(ignored::FILE));
            let packed = tokio::task::spawn_blocking(move || ignored::pack(&top, &list, &out))
                .await
                .map_err(|e| Fail::new("ignored_changed", e.to_string()))?
                .map_err(|e| {
                    Fail::new(
                        "ignored_changed",
                        format!("The files git ignores could not be packed: {e}"),
                    )
                })?;
            c.ignored = packed.files.iter().map(|f| f.path.clone()).collect();
            if !packed.files.is_empty() {
                files.push(FileEntry {
                    name: ignored::FILE.to_string(),
                    size: packed.size,
                    sha256: packed.sha256,
                    sent: 0,
                });
            }
        }
    }
    let total: u64 = files.iter().map(|f| f.size).sum();
    update(state, id, |r| {
        r.files = files;
        r.bytes_total = total;
        r.code = code;
    });
    Ok(())
}

/// Sends the package: the target learns what comes (`POST /peer/moves/{id}`)
/// and each file goes in chunks from what it has, so a broken transfer goes
/// on where it stopped.
async fn send(state: &AppState, id: &str, active: &Active, prep: &Prepared) -> Result<(), Fail> {
    let rec =
        load(state, id).ok_or_else(|| Fail::new("move_lost", "The move's record is gone."))?;
    let dir = move_folder(state, id)
        .ok_or_else(|| Fail::new("no_data_dir", "This build has no data directory."))?;
    let manifest = wire::Manifest {
        session: rec.session.clone(),
        epoch: rec.epoch,
        as_copy: rec.as_copy,
        title: rec.title.clone(),
        account: AccountAsk {
            name: rec.account.name.clone(),
            is_default: rec.account.is_default,
        },
        os: protocol::this_os().to_string(),
        cwd: rec.source_cwd.clone(),
        folder: prep.folder.clone(),
        hint: prep.hint.clone(),
        files: rec
            .files
            .iter()
            .map(|f| wire::FileSpec {
                name: f.name.clone(),
                size: f.size,
                sha256: f.sha256.clone(),
            })
            .collect(),
        code: rec.code.as_ref().map(|c| wire::CodeSpec {
            mode: c.mode.clone(),
            apply: c.apply.clone(),
            remote: c.remote.clone(),
            rel: c.rel.clone(),
            branch: c.branch.clone(),
            base: c.base.clone(),
            snapshot: c.snapshot.clone(),
            index_tree: c.index_tree.clone(),
            changed: c.changed,
            untracked: c.untracked,
            warnings: c.warnings.clone(),
            ignored: c.ignored.clone(),
            replaces: c.replaces.clone(),
        }),
        settings: rec.settings.clone(),
        held: rec.held.clone(),
        recipe: rec.recipe.clone(),
        keep_driving: rec.keep_driving,
        trust: rec.trust,
        when: rec.when.clone(),
        by: rec.by.clone(),
        transcript_bytes: 0,
        transcript_sha256: String::new(),
        from_copy: if rec.from_copy {
            Some(wire::FromCopy {
                files: crate::persist::read_json(&dir.join(COPY_LIST)).ok_or_else(|| {
                    Fail::new("package_failed", "The list of the session's files is gone.")
                })?,
            })
        } else {
            None
        },
    };
    let body = serde_json::to_value(&manifest).unwrap_or(Value::Null);
    let to = rec.to.id.clone();
    let (status, v) = call(
        state,
        &to,
        "POST",
        &format!("/peer/moves/{id}"),
        Some(&body),
        &active.acting,
        Duration::from_secs(60),
    )
    .await
    .map_err(|e| {
        Fail::new(
            "target_offline",
            format!("{} can't be reached: {e}", rec.to.name),
        )
    })?;
    if status != 200 {
        return Err(refusal(status, &v, &rec.to.name).into());
    }
    let have = |name: &str| -> u64 {
        v.get("files")
            .and_then(Value::as_array)
            .and_then(|l| l.iter().find(|f| f["name"] == name))
            .and_then(|f| f["have"].as_u64())
            .unwrap_or(0)
    };
    let mut first_chunk = true;
    for f in &rec.files {
        let path = file_path(&dir, &f.name)
            .ok_or_else(|| Fail::new("package_failed", format!("{} has no place", f.name)))?;
        let mut offset = have(&f.name).min(f.size);
        while offset < f.size {
            cancelled(active)?;
            let n = ((f.size - offset) as usize).min(CHUNK);
            let p = path.clone();
            let bytes = tokio::task::spawn_blocking(move || chunks::read_chunk(&p, offset, n))
                .await
                .map_err(|e| Fail::new("send_failed", e.to_string()))?
                .map_err(|e| Fail::new("send_failed", format!("{}: {e}", f.name)))?;
            offset = put_chunk(state, &rec, &f.name, offset, bytes, &active.acting).await?;
            if first_chunk {
                first_chunk = false;
                injected(state, "send")?;
            }
            let name = f.name.clone();
            update(state, id, |r| {
                if let Some(e) = r.files.iter_mut().find(|e| e.name == name) {
                    e.sent = offset;
                }
                r.bytes_sent = r.files.iter().map(|e| e.sent).sum();
            });
        }
        if f.size == 0 || offset >= f.size {
            let name = f.name.clone();
            let size = f.size;
            update(state, id, |r| {
                if let Some(e) = r.files.iter_mut().find(|e| e.name == name) {
                    e.sent = size;
                }
                r.bytes_sent = r.files.iter().map(|e| e.sent).sum();
            });
        }
    }
    Ok(())
}

/// One chunk (tried again a few times); returns what the target has after it.
async fn put_chunk(
    state: &AppState,
    rec: &MoveRecord,
    name: &str,
    offset: u64,
    bytes: Vec<u8>,
    acting: &str,
) -> Result<u64, Fail> {
    let path = format!("/peer/moves/{}/files/{name}?offset={offset}", rec.id);
    match chunks::put(state, &rec.to.id, &path, bytes, acting).await {
        Ok(size) => Ok(size),
        Err(PutError::Refused(status, v)) => Err(refusal(status, &v, &rec.to.name).into()),
        Err(PutError::Odd) => Err(Fail::new(
            "send_failed",
            format!("{} answered oddly", rec.to.name),
        )),
        Err(PutError::Offline(last)) => Err(Fail::new(
            "target_offline",
            format!("{} went offline while it was sent: {last}", rec.to.name),
        )),
    }
}

/// Asks the target to land it.
async fn land(state: &AppState, id: &str, acting: &str) -> Result<wire::Landed, Fail> {
    let rec =
        load(state, id).ok_or_else(|| Fail::new("move_lost", "The move's record is gone."))?;
    let (status, v) = call(
        state,
        &rec.to.id,
        "POST",
        &format!("/peer/moves/{id}/land"),
        Some(&json!({})),
        acting,
        LAND_TIMEOUT,
    )
    .await
    .map_err(|e| {
        Fail::new(
            "target_offline",
            format!("{} did not say it placed it: {e}", rec.to.name),
        )
    })?;
    if status != 200 {
        return Err(refusal(status, &v, &rec.to.name).into());
    }
    serde_json::from_value(v).map_err(|e| {
        Fail::new(
            "target_answer",
            format!("{} answered oddly: {e}", rec.to.name),
        )
    })
}

/// A copy landed (and was forked there): the session's own lease says it
/// lives here, so the target never resumes the original as a second writer.
async fn copied(state: &AppState, id: &str, landed: &wire::Landed, acting: &str) {
    let Some(rec) = load(state, id) else { return };
    let me = m::self_id(state).unwrap_or_default();
    let lease = state
        .machines
        .leases
        .get(&rec.session)
        .filter(|l| l.epoch > 0);
    if lease.is_none() {
        state.machines.leases.set(
            &rec.session,
            Lease {
                home: me.clone(),
                epoch: 1,
                since: m::now(),
                move_id: None,
                note: None,
            },
            &me,
        );
    }
    if let Some(w) = state.machines.leases.wire_of(&rec.session) {
        let body = json!({ "leases": [w] });
        let _ = call(
            state,
            &rec.to.id,
            "POST",
            "/peer/leases",
            Some(&body),
            acting,
            Duration::from_secs(20),
        )
        .await;
    }
    let ok = landed.resumed;
    let error = landed.resume_error.clone();
    let rec = update(state, id, |r| {
        r.resume_error = error.clone();
        if ok {
            r.reach(MoveState::Resumed);
        }
        r.undo_until = None;
    });
    if let Some(rec) = rec {
        super::audit(state, "machine.move.copied", &rec, json!({ "resumed": ok }));
        notice_done(state, &rec);
    }
}

/// Commits a landed move — the lease names the target from now on — and
/// tells the target, which resumes it. A target that cannot be told now
/// learns the lease at the next hello and resumes then.
pub(super) async fn finish(state: &AppState, id: &str, acting: &str) {
    let Some(rec) = load(state, id) else { return };
    let Some(me) = m::self_id(state) else { return };
    if rec.state == MoveState::Landed {
        state.machines.leases.set(
            &rec.session,
            Lease {
                home: rec.to.id.clone(),
                epoch: rec.epoch,
                since: m::now(),
                move_id: Some(rec.id.clone()),
                note: None,
            },
            &me,
        );
        state.machines.moves.release(&rec.session);
        lease::changed_here(state, std::slice::from_ref(&rec.session)).await;
        let rec = update(state, id, |r| {
            r.reach(MoveState::Committed);
            r.undo_until = Some(m::now() + UNDO_KEEP_MS);
        });
        if let Some(rec) = &rec {
            super::audit(
                state,
                "machine.move.committed",
                rec,
                json!({ "epoch": rec.epoch }),
            );
        }
    }
    let Some(rec) = load(state, id) else { return };
    if rec.state != MoveState::Committed {
        return;
    }
    if injected(state, "commit").is_err() {
        update(state, id, |r| {
            r.resume_error = Some(format!(
                "{} has not taken it over yet: it will once it hears from this computer.",
                rec.to.name
            ))
        });
        return;
    }
    let Some(w) = state.machines.leases.wire_of(&rec.session) else {
        return;
    };
    let body = json!({ "leases": [w] });
    let answer = call(
        state,
        &rec.to.id,
        "POST",
        "/peer/leases",
        Some(&body),
        acting,
        Duration::from_secs(120),
    )
    .await;
    let outcome: Option<wire::Resumed> = match &answer {
        Ok((200, v)) => v
            .get("resumed")
            .and_then(Value::as_array)
            .and_then(|l| l.iter().find(|r| r["move"] == id))
            .and_then(|r| serde_json::from_value(r.clone()).ok()),
        _ => None,
    };
    let rec = update(state, id, |r| match &outcome {
        Some(o) if o.ok => {
            r.resume_error = None;
            r.reach(MoveState::Resumed);
        }
        Some(o) => {
            r.resume_error = Some(
                o.error
                    .clone()
                    .unwrap_or_else(|| "It did not start there.".into()),
            )
        }
        None => {
            r.resume_error = Some(match &answer {
                Err(e) => format!(
                    "{} has not taken it over yet ({e}): it will once it hears from this \
                     computer.",
                    r.to.name
                ),
                Ok((status, _)) => format!("{} answered {status}.", r.to.name),
            })
        }
    });
    if let Some(rec) = rec {
        notice_done(state, &rec);
    }
}

fn notice_done(state: &AppState, rec: &MoveRecord) {
    let name = rec.title.clone().unwrap_or_else(|| "The session".into());
    let (level, text) = match (&rec.state, &rec.resume_error) {
        (MoveState::Resumed, _) => (
            crate::claude::model::NoticeLevel::Success,
            if rec.as_copy {
                format!("A copy of {name} now runs on {}.", rec.to.name)
            } else {
                format!("{name} now runs on {}.", rec.to.name)
            },
        ),
        (_, Some(e)) => (
            crate::claude::model::NoticeLevel::Warning,
            format!("{name} is on {} but didn't start there: {e}", rec.to.name),
        ),
        _ => return,
    };
    crate::server::ws::publish_notice(state, level, text, Some(&rec.session));
}

/// A move that stopped before it landed: the target drops what it got, the
/// session starts again here when it ran, and the record says why.
async fn failed(state: &AppState, id: &str, fail: Fail, acting: &str) {
    let Some(rec) = load(state, id) else { return };
    if rec.state.is_final() {
        return;
    }
    if !rec.state.before_landed() {
        // Landed already: only the lease is left to move.
        finish(state, id, acting).await;
        return;
    }
    if matches!(rec.state, MoveState::Sending | MoveState::Landing) {
        let _ = call(
            state,
            &rec.to.id,
            "DELETE",
            &format!("/peer/moves/{id}"),
            None,
            acting,
            Duration::from_secs(60),
        )
        .await;
    }
    state.machines.moves.release(&rec.session);
    let mut resume_error = None;
    if rec.was_running && !rec.as_copy {
        if let Err(e) = resume::start(
            state,
            &Spec {
                session: &rec.session,
                fork_as: None,
                cwd: &rec.source_cwd,
                account: &rec.account.source,
                settings: &rec.settings,
                may_dangerous: true,
                held: &rec.held,
            },
        )
        .await
        {
            resume_error = Some(resume::words(&e));
        }
    }
    let cancelled = fail.code == "cancelled";
    let step = rec.state;
    let rec = update(state, id, |r| {
        r.error = Some(MoveFailure {
            code: fail.code.clone(),
            message: fail.message.clone(),
            step,
            offer: fail.offer.clone(),
        });
        r.resume_error = resume_error.clone();
        r.held.clear();
        r.reach(if cancelled {
            MoveState::Cancelled
        } else {
            MoveState::Failed
        });
    });
    if let Some(rec) = rec {
        super::audit(
            state,
            if cancelled {
                "machine.move.cancelled"
            } else {
                "machine.move.failed"
            },
            &rec,
            json!({ "code": fail.code, "step": step }),
        );
        let name = rec.title.clone().unwrap_or_else(|| "The session".into());
        crate::server::ws::publish_notice(
            state,
            if cancelled {
                crate::claude::model::NoticeLevel::Info
            } else {
                crate::claude::model::NoticeLevel::Error
            },
            if cancelled {
                format!("{name} stays on this computer: the move was cancelled.")
            } else {
                format!(
                    "Couldn't move {name}: {} It goes on here, as it was.",
                    fail.message
                )
            },
            Some(&rec.session),
        );
    }
    if let Some(dir) = move_folder(state, id) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// After a restart: a move this machine sent that had not landed.
pub(super) async fn fail_after_restart(state: &AppState, rec: &MoveRecord) {
    failed(
        state,
        &rec.id,
        Fail::new("interrupted", "Mother Claude stopped during the move."),
        "",
    )
    .await;
}

// ─── undo, coming home, take over ──────────────────────────────────────────────

/// `POST /api/moves/{id}/remove-worktree` for a move this machine sent: the
/// target removes the worktree the code landed in there (it checks that the
/// session left it or ended, and that its work is in a snapshot), and this
/// record says so too.
pub(super) async fn remove_worktree_there(
    state: &AppState,
    ctx: &ClientContext,
    rec: &MoveRecord,
) -> ApiResult<MoveRecord> {
    let acting = m::acting::header_for(state, ctx);
    let (status, v) = call(
        state,
        &rec.to.id,
        "POST",
        &format!("/peer/moves/{}/remove-worktree", rec.id),
        Some(&json!({})),
        &acting,
        LAND_TIMEOUT,
    )
    .await
    .map_err(|e| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "machine_unreachable",
            format!("{} can't be reached right now: {e}", rec.to.name),
        )
    })?;
    if status != 200 {
        return Err(refusal(status, &v, &rec.to.name));
    }
    let at = v
        .pointer("/move/worktreeRemovedAt")
        .and_then(Value::as_i64)
        .unwrap_or_else(m::now);
    update(state, &rec.id, |r| {
        r.worktree_removed_at = Some(at);
        r.undo_until = None;
    })
    .ok_or_else(super::not_found)
}

/// `POST /api/moves/{id}/undo` for a move this machine sent: the target
/// undoes it and gives the lease back.
pub(super) async fn undo_there(
    state: &AppState,
    ctx: &ClientContext,
    rec: &MoveRecord,
) -> ApiResult<MoveRecord> {
    if rec.as_copy {
        return Err(ApiError::conflict(
            "move_not_undoable",
            "A copy is not undone: stop or delete it on the other computer.",
        ));
    }
    if !matches!(rec.state, MoveState::Committed | MoveState::Resumed) {
        return Err(ApiError::conflict(
            "move_not_undoable",
            "Only a move that finished can be undone.",
        ));
    }
    if rec.undo_until.is_some_and(|t| m::now() > t) {
        return Err(ApiError::conflict(
            "move_not_undoable",
            "The undo of this move has expired.",
        ));
    }
    let acting = m::acting::header_for(state, ctx);
    let (status, v) = call(
        state,
        &rec.to.id,
        "POST",
        &format!("/peer/moves/{}/undo", rec.id),
        Some(&json!({})),
        &acting,
        LAND_TIMEOUT,
    )
    .await
    .map_err(|e| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "machine_unreachable",
            format!("{} can't be reached right now: {e}", rec.to.name),
        )
    })?;
    if status != 200 {
        return Err(refusal(status, &v, &rec.to.name));
    }
    // The lease it gave back: home here, under this very move.
    if let Some(w) = v
        .get("lease")
        .and_then(|l| serde_json::from_value::<lease::WireLease>(l.clone()).ok())
    {
        let me = m::self_id(state).unwrap_or_default();
        if w.session == rec.session && w.home == me && w.move_id.as_deref() == Some(&rec.id) {
            let known = |x: &str| x == me;
            let changed = state.machines.leases.merge(&[w], &known, &me);
            lease::changed_here(state, &changed).await;
        }
    }
    let now = load(state, &rec.id).ok_or_else(super::not_found)?;
    came_home(state, &now).await;
    load(state, &rec.id).ok_or_else(super::not_found)
}

/// The target undid a move this machine sent (its lease came back under the
/// move's id): the session lives here again, and starts again here when it
/// ran when it was frozen.
pub(super) async fn came_home(state: &AppState, rec: &MoveRecord) {
    let mut proceed = false;
    let Some(rec) = update(state, &rec.id, |r| {
        if matches!(r.state, MoveState::Committed | MoveState::Resumed) {
            proceed = true;
            r.reach(MoveState::Undone);
            r.undo_until = None;
        }
    }) else {
        return;
    };
    if !proceed {
        return;
    }
    lease::changed_here(state, std::slice::from_ref(&rec.session)).await;
    if rec.was_running {
        if let Err(e) = resume::start(
            state,
            &Spec {
                session: &rec.session,
                fork_as: None,
                cwd: &rec.source_cwd,
                account: &rec.account.source,
                settings: &rec.settings,
                may_dangerous: true,
                held: &[],
            },
        )
        .await
        {
            update(state, &rec.id, |r| r.resume_error = Some(resume::words(&e)));
        }
    }
    super::audit(state, "machine.move.undone", &rec, Value::Null);
    let name = rec.title.clone().unwrap_or_else(|| "The session".into());
    crate::server::ws::publish_notice(
        state,
        crate::claude::model::NoticeLevel::Success,
        format!("{name} lives on this computer again: the move was undone."),
        Some(&rec.session),
    );
}

/// `POST /api/sessions/{id}/move {to: <this machine>, takeOver: true}`: the
/// session's home is offline, and it lives here from now on (epoch + 1)
/// from this machine's own copy (decision 11). When the old home comes back
/// it learns the lease; what it did meanwhile becomes a conflict copy when
/// the session next moves there.
async fn take_over(
    state: &AppState,
    ctx: &ClientContext,
    session: &str,
    body: &MoveBody,
) -> ApiResult<(StatusCode, Value)> {
    if !owned::is_uuid(session) {
        return Err(ApiError::unprocessable(
            "invalid_session_id",
            "Only Claude Code sessions (UUID ids) move to another computer.",
        ));
    }
    let me = m::self_id(state).ok_or_else(|| ApiError::internal("This computer has no id."))?;
    if body.to.trim() != me {
        return Err(invalid(
            "A take over brings it to this computer: name this computer.",
        ));
    }
    let Some(old) = lease::elsewhere(state, session) else {
        return Err(ApiError::conflict(
            "not_elsewhere",
            "It lives on this computer already.",
        ));
    };
    // The copy its home kept current here (§5.9), unless told otherwise.
    let copy = crate::machines::copies::target::held_of(state, session)
        .filter(|h| h.machine == old.home)
        .filter(|_| body.from_copy != Some(false));
    let home = crate::claude::summaries::session_home(state, session);
    if copy.is_none() && crate::claude::transfer::transcripts_of(&home, session).is_empty() {
        return Err(ApiError::conflict(
            "nothing_here",
            "This computer has nothing of this conversation: it can't go on here.",
        ));
    }
    let name = m::peer_name(state, &old.home);
    // Only while the home is away: online, it is brought back instead.
    let online = matches!(
        call(
            state,
            &old.home,
            "GET",
            "/peer/leases",
            None,
            "",
            Duration::from_secs(5)
        )
        .await,
        Ok((200, _))
    );
    if online {
        return Err(ApiError::conflict(
            "home_online",
            format!("{name} is online: bring it back here instead."),
        )
        .with("machine", old.home.clone()));
    }
    // From the copy: it lands in this computer's config directory first (as
    // a pull lands a session, undo kept); nothing changes when it can't.
    let landed = match &copy {
        Some(_) => Some(
            crate::machines::copies::target::land_for_take_over(
                state,
                &old.home,
                session,
                body.folder.as_deref(),
            )
            .await?,
        ),
        None => None,
    };
    let epoch = old.epoch + 1;
    state.machines.leases.set(
        session,
        Lease {
            home: me.clone(),
            epoch,
            since: m::now(),
            move_id: None,
            note: None,
        },
        &me,
    );
    lease::changed_here(state, &[session.to_string()]).await;
    state
        .devices
        .audit
        .record(crate::server::devices::audit::AuditEntry {
            action: "machine.lease.takeover".into(),
            device_id: Some(old.home.clone()),
            device_name: Some(name.clone()),
            detail: Some(json!({
                "session": session,
                "epoch": epoch,
                "by": super::who(state, ctx),
                "fromCopy": landed.is_some(),
            })),
            ..crate::server::devices::audit::AuditEntry::now()
        });
    crate::server::auth::note_device_action(
        state,
        ctx,
        &format!("took over a session from {name}"),
    );
    // From the copy, it goes on here at once as an owned session, with the
    // account, folder and settings it had there.
    let (mut resumed, mut resume_error) = (false, None);
    if let Some(l) = &landed {
        match resume::start(
            state,
            &Spec {
                session,
                fork_as: None,
                cwd: &l.cwd,
                account: &l.account,
                settings: &l.settings,
                may_dangerous: ctx.may_dangerous(),
                held: &[],
            },
        )
        .await
        {
            Ok(_) => resumed = true,
            Err(e) => resume_error = Some(resume::words(&e)),
        }
    }
    let warning = format!(
        "It goes on here from this computer's copy. If {name} went on with it while it was \
         away, both versions are kept side by side when it next moves there."
    );
    Ok((
        StatusCode::OK,
        json!({
            "takenOver": true,
            "session": session,
            "home": me,
            "from": old.home,
            "epoch": epoch,
            "warning": warning,
            "fromCopy": landed.is_some(),
            "copiedAt": landed.as_ref().and_then(|l| l.copied_at),
            "complete": landed.as_ref().map(|l| l.complete),
            "cwd": landed.as_ref().map(|l| l.cwd.clone()),
            "landing": landed.as_ref().map(|l| l.landing.clone()),
            "warnings": landed.as_ref().map(|l| l.warnings.clone()).unwrap_or_default(),
            "resumed": resumed,
            "resumeError": resume_error,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_and_origins_compare_as_people_mean_them() {
        assert!(older("2.1.322", "2.1.330"));
        assert!(!older("2.1.330", "2.1.322"));
        assert!(!older("2.1.330", "2.1.330"));
        assert!(older("2.1.9 (Claude Code)", "2.1.10"));
        assert!(same_origin(
            "https://github.com/me/app.git",
            "https://GitHub.com/me/app/"
        ));
        assert!(!same_origin(
            "git@github.com:me/app.git",
            "git@github.com:me/other.git"
        ));
    }
}
