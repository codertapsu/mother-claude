//! The target's half of a move (docs/PLAN-MACHINES.md §5.8): answering a
//! preflight (nothing written), receiving the package, landing it — the code
//! first (a stop by design changes nothing), then the files git ignores the
//! project carries (`ignored`: this computer's `dangerous` grant for the
//! source), then the conversation with the pull's machinery, all kept for
//! undo — resuming once the lease names this machine, undoing on request,
//! and dropping what a failed move left.
//!
//! Everything a source sends is checked before git or the file system sees
//! it: ids, file names (a fixed list), sizes, hashes, paths, settings.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::folder::{self, BadChoice};
use super::ignored;
use super::note::{self, Note};
use super::resume::{self, Spec};
use super::wire::{self, AccountAsk, AccountThere, CodeThere, FolderThere, SessionThere};
use super::{
    file_path, injected, load, move_folder, peer_ref, save, this_machine, update, worktrees_root,
    AccountPart, Check, CodePart, FileEntry, LandingPart, MoveFailure, MoveRecord, MoveState, Role,
    UNDO_KEEP_MS,
};
use crate::claude::backup::session_pack::{self, LandInputs, Landed};
use crate::claude::home::ClaudeHome;
use crate::claude::owned;
use crate::claude::paths;
use crate::machines::chunks::{self, AppendError, Refusal};
use crate::machines::code::{self, ApplyRequest, Landing};
use crate::machines::lease::{self, Lease};
use crate::machines::{self as m, protocol};
use crate::server::context::ClientContext;
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// At most this many files in a package (a conversation, its code, Git LFS
/// objects).
const MAX_FILES: usize = 10_000;
/// The largest file a package may hold.
const MAX_FILE: u64 = 1 << 40;

fn bad(message: impl Into<String>) -> ApiError {
    ApiError::unprocessable("invalid_move", message)
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A folder under a repository's top as a source names it: `/`-separated
/// plain names, nothing that leaves it.
fn safe_rel(rel: &str) -> bool {
    rel.is_empty()
        || (rel.len() <= 1024
            && rel
                .split('/')
                .all(|c| !c.is_empty() && c != "." && c != ".." && !c.contains(['\\', '\0', ':'])))
}

fn join_rel(base: &Path, rel: &str) -> PathBuf {
    let mut out = base.to_path_buf();
    for c in rel.split('/').filter(|c| !c.is_empty()) {
        out.push(c);
    }
    out
}

fn check_settings(s: &super::Carried) -> ApiResult<()> {
    if let Some(m) = &s.model {
        owned::validate_model(m)?;
    }
    owned::validate_launch(
        s.effort.as_deref(),
        s.thinking.as_deref(),
        s.permission_mode.as_deref(),
    )
}

fn check_code_ask(c: &wire::CodeAsk) -> ApiResult<()> {
    if !matches!(c.mode.as_str(), "bundle" | "remote")
        || !matches!(c.apply.as_str(), "checkout" | "worktree")
        || !safe_rel(&c.rel)
        || c.branch.as_deref().is_some_and(|b| !code::valid_branch(b))
        || c.remote
            .as_deref()
            .is_some_and(|r| !code::valid_remote_name(r))
        || !ignored::valid_list(&c.ignored)
    {
        return Err(bad("The move's code is not one this computer can take."));
    }
    Ok(())
}

fn check_ask(ask: &wire::Ask) -> ApiResult<()> {
    if !owned::is_uuid(&ask.session) {
        return Err(bad("Not a session id."));
    }
    if ask.account.name.chars().count() > 100
        || ask.cwd.len() > 4096
        || !paths::is_absolute_anywhere(&ask.cwd)
        || ask.folder.as_deref().is_some_and(|f| f.len() > 4096)
        || ask.hint.as_deref().is_some_and(|f| f.len() > 4096)
    {
        return Err(bad(
            "The move names a folder or an account this computer can't take.",
        ));
    }
    if let Some(c) = &ask.code {
        check_code_ask(c)?;
    }
    if let Some(mode) = &ask.permission_mode {
        owned::validate_launch(None, None, Some(mode))?;
    }
    Ok(())
}

fn check_manifest(mf: &wire::Manifest) -> ApiResult<()> {
    check_ask(&wire::Ask {
        session: mf.session.clone(),
        account: mf.account.clone(),
        cwd: mf.cwd.clone(),
        folder: mf.folder.clone(),
        hint: mf.hint.clone(),
        ..wire::Ask::default()
    })?;
    if mf.files.is_empty()
        || mf.files.len() > MAX_FILES
        || !mf.files.iter().any(|f| f.name == session_pack::FILE)
    {
        return Err(bad("The move's package is not complete."));
    }
    let mut names = std::collections::HashSet::new();
    for f in &mf.files {
        if file_path(Path::new("x"), &f.name).is_none()
            || !names.insert(f.name.as_str())
            || f.size > MAX_FILE
            || !is_sha256(&f.sha256)
        {
            return Err(bad(format!(
                "The package's file {:?} is not one this computer takes.",
                f.name
            )));
        }
    }
    // The files git ignores come in their own file, with their list.
    let carried = mf.files.iter().any(|f| f.name == ignored::FILE);
    let listed = mf.code.as_ref().is_some_and(|c| !c.ignored.is_empty());
    if carried != listed {
        return Err(bad("The move's files git ignores came without their list."));
    }
    if let Some(c) = &mf.code {
        check_code_ask(&wire::CodeAsk {
            mode: c.mode.clone(),
            apply: c.apply.clone(),
            remote: c.remote.clone(),
            rel: c.rel.clone(),
            branch: c.branch.clone(),
            origin: None,
            ignored: c.ignored.clone(),
        })?;
        for oid in [&c.base, &c.snapshot, &c.index_tree].into_iter().flatten() {
            if !code::is_oid(oid) {
                return Err(bad(
                    "The move's code names a commit this computer can't take.",
                ));
            }
        }
        if !mf.files.iter().any(|f| f.name == code::META) {
            return Err(bad("The move's code came without its record."));
        }
    }
    check_settings(&mf.settings)?;
    if mf.held.len() > 20 || mf.held.iter().any(|h| h.len() > owned::MAX_TEXT_BYTES) {
        return Err(bad("Too many messages came with the move."));
    }
    if mf.recipe.len() > 10
        || mf
            .recipe
            .iter()
            .any(|r| r.len() > 300 || r.contains(['\n', '\r']))
    {
        return Err(bad("The project's recipe is too long."));
    }
    if mf.title.as_deref().is_some_and(|t| t.len() > 1000) || mf.by.len() > 200 {
        return Err(bad("The move's record is too long."));
    }
    if !matches!(mf.os.as_str(), "macos" | "windows" | "linux" | "other") {
        return Err(bad("The move names an unknown system."));
    }
    Ok(())
}

// ─── accounts, folders, checkouts ──────────────────────────────────────────────

/// The account a move runs under here (decision 3): the same name
/// (case-insensitive), else the Default for the source's Default.
fn resolve_account(state: &AppState, ask: &AccountAsk) -> Result<(String, String), Check> {
    let me = m::this_name(state);
    let all = state.accounts.accounts();
    let wanted = ask.name.trim();
    if let Some(a) = all
        .iter()
        .find(|a| !wanted.is_empty() && a.name.trim().eq_ignore_ascii_case(wanted))
    {
        return Ok((a.id.clone(), a.name.clone()));
    }
    if ask.is_default {
        if let Some(a) = all.iter().find(|a| a.is_default()) {
            return Ok((a.id.clone(), a.name.clone()));
        }
    }
    Err(Check::new(
        "account_missing",
        format!("{me} has no account named {wanted}."),
    )
    .fix(format!(
        "Add the account {wanted} on {me} and sign in, then try again."
    )))
}

/// Whether the account can run a session here: usable, its token here, not
/// known to be signed out (`Ok(None)`: never checked here).
fn account_ready(state: &AppState, id: &str, name: &str) -> Result<Option<bool>, Check> {
    use crate::claude::accounts::view::Attention;
    let me = m::this_name(state);
    if !state.accounts.is_usable(id) {
        return Err(Check::new(
            "account_missing",
            format!("{me} has no usable account named {name}."),
        ));
    }
    if let Err(e) = state.accounts.env(id) {
        return Err(Check::new(
            "account_token_missing",
            format!("{name} on {me} can't run sessions: {e}"),
        )
        .fix(format!("Sign {name} in on {me}.")));
    }
    let view = crate::claude::accounts::view::build(state, false, None);
    let Some(a) = view.accounts.into_iter().find(|a| a.id == id) else {
        return Ok(None);
    };
    match a.attention {
        Some(Attention::SignedOut | Attention::AuthFailed | Attention::TokenMissing) => {
            Err(Check::new(
                "account_signed_out",
                format!("{name} isn't signed in on {me}."),
            )
            .fix(format!(
                "Sign {name} in on {me} (Accounts there), then try again."
            )))
        }
        Some(Attention::CheckFailed) => Ok(None),
        None if a.signed_in => Ok(Some(true)),
        None => Ok(None),
    }
}

/// The checks a landing makes again, as the preflight made them: the
/// account, the folder, its trust, the checkout, the session here.
struct Resolved {
    answer: wire::Answer,
    account: Option<(String, String)>,
    folder: Option<String>,
    cwd: Option<String>,
}

async fn resolve(
    state: &AppState,
    ctx: &ClientContext,
    from: &str,
    ask: &wire::Ask,
    source_epoch: Option<u64>,
) -> Resolved {
    let me = this_machine(state);
    let mut a = wire::Answer {
        machine: me.clone(),
        claude: m::setup::claude_version(state),
        ..wire::Answer::default()
    };
    // The account.
    let account = match resolve_account(state, &ask.account) {
        Ok((id, name)) => match account_ready(state, &id, &name) {
            Ok(signed) => {
                if signed.is_none() {
                    a.warnings.push(Check::new(
                        "account_unchecked",
                        format!("Couldn't check that {name} is signed in on {}.", me.name),
                    ));
                }
                a.account = Some(AccountThere {
                    id: id.clone(),
                    name: name.clone(),
                    signed_in: signed,
                });
                Some((id, name))
            }
            Err(c) => {
                a.blockers.push(c);
                None
            }
        },
        Err(c) => {
            a.blockers.push(c);
            None
        }
    };
    let home = account
        .as_ref()
        .and_then(|(id, _)| state.accounts.home(id))
        .unwrap_or_else(|| state.home.clone());
    // The session here.
    a.session = SessionThere {
        here: !crate::claude::transfer::transcripts_of(&home, &ask.session).is_empty(),
        live: crate::claude::transfer::ops::is_live(state, &ask.session),
    };
    if a.session.live {
        a.blockers.push(
            Check::new(
                "session_live_there",
                format!("{} runs this session right now.", me.name),
            )
            .fix(format!("Stop it on {} first.", me.name)),
        );
    }
    // A newer lease here than the source's: it lives elsewhere already.
    if let (Some(l), Some(epoch)) = (state.machines.leases.get(&ask.session), source_epoch) {
        if l.epoch > epoch && l.home != from && !ask.as_copy {
            a.blockers.push(Check::new(
                "lease_newer",
                format!(
                    "{} knows a newer home for this session ({}).",
                    me.name,
                    m::peer_name(state, &l.home)
                ),
            ));
        }
    }
    if owned::is_dangerous_mode(ask.permission_mode.as_deref()) && !ctx.may_dangerous() {
        a.blockers.push(Check::new(
            "dangerous_mode",
            format!(
                "It runs without permission prompts: going on on {} needs its Dangerous \
                 permission for this computer.",
                me.name
            ),
        ));
    }
    // The files git ignores the project carries: they usually hold
    // secrets, so this computer's `dangerous` grant for the source too.
    if ask.code.as_ref().is_some_and(|c| !c.ignored.is_empty()) && !ctx.may_dangerous() {
        a.blockers.push(Check::new(
            "ignored_needs_dangerous",
            format!(
                "Carrying files git ignores needs {}'s Dangerous permission for this computer.",
                me.name
            ),
        ));
    }
    // The folder.
    let found = folder::folder_there(
        state,
        from,
        &ask.cwd,
        ask.folder.as_deref(),
        ask.hint.as_deref(),
    );
    let folder = match found {
        Err(BadChoice(why)) => {
            a.blockers.push(Check::new("invalid_folder", why));
            None
        }
        Ok(f) => match f.path {
            Some(p) => Some((p, f.how)),
            None => {
                a.blockers.push(
                    Check::new(
                        "folder_elsewhere",
                        format!(
                            "{} has no folder for {}: choose its folder there.",
                            me.name, ask.cwd
                        ),
                    )
                    .offer("choose-folder")
                    .detail(json!({ "candidates": f.candidates })),
                );
                a.candidates = f.candidates;
                None
            }
        },
    };
    let mut cwd = None;
    if let Some((path, how)) = &folder {
        let trusted = owned::trusted_in(&home.claude_json(), &home.projects_dir(), Path::new(path));
        a.folder = Some(FolderThere {
            path: path.clone(),
            how: how.to_string(),
            trusted,
        });
        if !trusted && !ask.trust {
            a.blockers.push(
                Check::new(
                    "folder_untrusted",
                    format!("{} doesn't trust {path} yet.", me.name),
                )
                .fix(format!(
                    "Open it once with Claude Code on {}, or carry the folder's trust (needs the \
                     dangerous permission).",
                    me.name
                )),
            );
        }
        if ask.trust && !ctx.may_dangerous() {
            a.blockers.push(Check::new(
                "trust_needs_dangerous",
                format!(
                    "Carrying the folder's trust needs {}'s Dangerous permission for this \
                     computer.",
                    me.name
                ),
            ));
        }
        cwd = Some(path.clone());
        if let Some(c) = &ask.code {
            let git = state.machines.moves.git();
            let (p, session, branch) = (PathBuf::from(path), ask.session.clone(), c.branch.clone());
            let checked = tokio::task::spawn_blocking(move || {
                code::target_check(&git, &p, &session, branch.as_deref())
            })
            .await;
            match checked {
                Ok(Ok(t)) => {
                    let top = t.state.top.display().to_string();
                    if c.apply == "checkout" {
                        if let Some(op) = &t.state.busy {
                            a.blockers.push(
                                Check::new(
                                    "checkout_busy",
                                    format!("A git operation is under way in {top} ({op})."),
                                )
                                .fix("Use a separate folder (worktree) there.")
                                .offer("worktree"),
                            );
                        } else if !t.clean {
                            a.blockers.push(
                                Check::new(
                                    "checkout_dirty",
                                    format!(
                                        "{top} has changes of its own ({} {}). The code can't \
                                         land in the project folder.",
                                        t.state.changed,
                                        if t.state.changed == 1 {
                                            "file"
                                        } else {
                                            "files"
                                        }
                                    ),
                                )
                                .fix("Use a separate folder (worktree) there.")
                                .offer("worktree"),
                            );
                        }
                        if let Some(w) = &t.elsewhere {
                            a.blockers.push(
                                Check::new(
                                    "checked_out_elsewhere",
                                    format!(
                                        "Its branch is checked out in another worktree there ({}).",
                                        w.display()
                                    ),
                                )
                                .offer("worktree"),
                            );
                        }
                    } else {
                        let wt = worktrees_root(state).map(|r| r.join(&ask.session));
                        if let Some(w) = wt
                            .filter(|w| std::fs::read_dir(w).is_ok_and(|mut d| d.next().is_some()))
                        {
                            a.blockers.push(Check::new(
                                "worktree_exists",
                                format!(
                                    "A worktree of this session is still on {} ({}): remove it \
                                     there first.",
                                    me.name,
                                    w.display()
                                ),
                            ));
                        }
                    }
                    if c.mode == "remote" {
                        let remote = c.remote.clone().unwrap_or_else(|| "origin".into());
                        if !t.state.remotes.contains(&remote) {
                            a.blockers.push(Check::new(
                                "remote_missing_there",
                                format!("{top} on {} has no remote named {remote}.", me.name),
                            ));
                        }
                    }
                    cwd = Some(if c.apply == "worktree" {
                        worktrees_root(state)
                            .map(|r| join_rel(&r.join(&ask.session), &c.rel))
                            .unwrap_or_default()
                            .display()
                            .to_string()
                    } else {
                        path.clone()
                    });
                    a.code = Some(CodeThere {
                        repo: true,
                        clean: t.clean,
                        parked: t.parked,
                        changed: t.state.changed,
                        untracked: t.state.untracked,
                        busy: t.state.busy.clone(),
                        branch: t.state.branch.clone(),
                        branch_tip: t.branch_tip.clone(),
                        elsewhere: t.elsewhere.as_ref().map(|w| w.display().to_string()),
                        origin: t.state.origin.clone(),
                        remotes: t.state.remotes.clone(),
                        haves: t.haves.clone(),
                        shallow: t.shallow,
                    });
                }
                Ok(Err(code::CodeError::NotARepository(_))) => {
                    a.code = Some(CodeThere::default());
                    a.blockers.push(
                        Check::new(
                            "repo_missing",
                            format!("{path} on {} is not a git repository.", me.name),
                        )
                        .fix("Clone the project there first, or move only the conversation."),
                    );
                }
                Ok(Err(e)) => a
                    .blockers
                    .push(Check::new("code_unreadable", e.to_string())),
                Err(e) => a
                    .blockers
                    .push(Check::new("code_unreadable", e.to_string())),
            }
        }
        // Room for it.
        if let Some(free) = super::free_bytes(Path::new(path)) {
            a.free_bytes = Some(free);
            let need = ask.bytes.saturating_mul(2).saturating_add(64 << 20);
            if free < need {
                a.blockers.push(Check::new(
                    "disk_full",
                    format!("{} has too little free space for it.", me.name),
                ));
            }
        }
    }
    a.cwd = cwd.clone();
    Resolved {
        answer: a,
        account,
        folder: folder.map(|(p, _)| p),
        cwd,
    }
}

/// `POST /peer/moves/preflight`: this machine's half of the checks (nothing
/// is written). Code through the remote: the remote must answer here
/// without a prompt (the setup's check, made again before the move).
pub async fn peer_preflight(
    state: &AppState,
    ctx: &ClientContext,
    from: &str,
    ask: wire::Ask,
) -> ApiResult<wire::Answer> {
    check_ask(&ask)?;
    let epoch = ask.epoch;
    let mut answer = resolve(state, ctx, from, &ask, Some(epoch)).await.answer;
    let remote_here = ask
        .code
        .as_ref()
        .filter(|c| c.mode == "remote")
        .map(|c| c.remote.clone().unwrap_or_else(|| "origin".into()))
        .filter(|r| {
            answer
                .code
                .as_ref()
                .is_some_and(|t| t.repo && t.remotes.contains(r))
        });
    if let (Some(remote), Some(folder)) = (remote_here, answer.folder.clone()) {
        let r = crate::machines::setup::folders::remote_named_answers_async(
            state.machines.moves.git(),
            PathBuf::from(&folder.path),
            remote.clone(),
        )
        .await;
        if r.state != "ok" {
            let me = m::this_name(state);
            answer.blockers.push(
                Check::new(
                    "remote_unreachable_there",
                    format!("The remote {remote} doesn't answer on {me} without a prompt."),
                )
                .fix(format!("On {me}: {}", r.detail)),
            );
        }
    }
    Ok(answer)
}

fn ask_of(mf: &wire::Manifest) -> wire::Ask {
    wire::Ask {
        session: mf.session.clone(),
        as_copy: mf.as_copy,
        account: mf.account.clone(),
        os: mf.os.clone(),
        cwd: mf.cwd.clone(),
        folder: mf.folder.clone(),
        hint: mf.hint.clone(),
        code: mf.code.as_ref().map(|c| wire::CodeAsk {
            mode: c.mode.clone(),
            apply: c.apply.clone(),
            remote: c.remote.clone(),
            rel: c.rel.clone(),
            branch: c.branch.clone(),
            origin: None,
            ignored: c.ignored.clone(),
        }),
        trust: mf.trust,
        permission_mode: mf.settings.permission_mode.clone(),
        bytes: mf.files.iter().map(|f| f.size).sum(),
        claude: None,
        // The lease the move raises is one above the source's.
        epoch: mf.epoch.saturating_sub(1),
    }
}

// ─── receiving ─────────────────────────────────────────────────────────────────

fn manifest_path(dir: &Path) -> PathBuf {
    dir.join("manifest.json")
}

fn load_manifest(state: &AppState, id: &str) -> Option<wire::Manifest> {
    crate::persist::read_json(&manifest_path(&move_folder(state, id)?))
}

/// How much of a file this machine has: all of it once checked, else what
/// arrived.
fn have(dir: &Path, name: &str) -> u64 {
    file_path(dir, name).map_or(0, |p| chunks::have(&p))
}

/// An incoming move of `from`'s, as this machine has it.
fn incoming(state: &AppState, id: &str, from: &str) -> ApiResult<MoveRecord> {
    load(state, id)
        .filter(|r| r.role == Role::In && r.from.id == from)
        .ok_or_else(super::not_found)
}

/// `POST /peer/moves/{id}`: a move is coming (again, after a broken one:
/// what arrived stays). Answers what this machine has of each file.
pub async fn peer_create(
    state: &AppState,
    ctx: &ClientContext,
    from: &str,
    id: &str,
    mf: wire::Manifest,
) -> ApiResult<Value> {
    if !super::is_move_id(id) {
        return Err(bad("Not a move id."));
    }
    check_manifest(&mf)?;
    let dir = move_folder(state, id).ok_or_else(|| ApiError::internal("No data directory."))?;
    match load(state, id) {
        Some(rec) => {
            if rec.role != Role::In || rec.from.id != from || rec.session != mf.session {
                return Err(ApiError::conflict(
                    "move_conflict",
                    "Another move has this id here.",
                ));
            }
            if rec.state.is_final() {
                return Err(ApiError::conflict(
                    "move_finished",
                    "This move has ended here.",
                ));
            }
        }
        None => {
            std::fs::create_dir_all(&dir).map_err(|e| ApiError::internal(e.to_string()))?;
            crate::claude::transfer::private_dir(&dir);
            crate::persist::write_json_private(&manifest_path(&dir), &mf)
                .map_err(|e| ApiError::internal(e.to_string()))?;
            let now = m::now();
            let mut rec = MoveRecord {
                id: id.to_string(),
                role: Role::In,
                session: mf.session.clone(),
                target_session: None,
                title: mf.title.clone(),
                from: peer_ref(state, from),
                to: this_machine(state),
                state: MoveState::Sending,
                as_copy: mf.as_copy,
                keep_driving: mf.keep_driving,
                when: mf.when.clone(),
                account: AccountPart {
                    name: mf.account.name.clone(),
                    is_default: mf.account.is_default,
                    source: String::new(),
                    target: None,
                },
                epoch: mf.epoch,
                source_cwd: mf.cwd.clone(),
                target_cwd: None,
                target_folder: None,
                code: mf.code.as_ref().map(|c| CodePart {
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
                    landed: None,
                }),
                files: mf
                    .files
                    .iter()
                    .map(|f| FileEntry {
                        name: f.name.clone(),
                        size: f.size,
                        sha256: f.sha256.clone(),
                        sent: 0,
                    })
                    .collect(),
                bytes_total: mf.files.iter().map(|f| f.size).sum(),
                bytes_sent: 0,
                conversation_bytes: mf
                    .files
                    .iter()
                    .find(|f| f.name == session_pack::FILE)
                    .map(|f| f.size)
                    .unwrap_or(0),
                settings: mf.settings.clone(),
                was_running: false,
                held: mf.held.clone(),
                recipe: mf.recipe.clone(),
                trust: mf.trust,
                steps: Vec::new(),
                error: None,
                warnings: Vec::new(),
                landing: None,
                resume_error: None,
                by: format!("{} ({})", m::peer_name(state, from), mf.by),
                dangerous: ctx.may_dangerous(),
                created_at: now,
                updated_at: now,
                finished_at: None,
                undo_until: None,
            };
            rec.reach(MoveState::Sending);
            save(state, &rec);
            super::audit(state, "machine.move.incoming", &rec, Value::Null);
        }
    }
    let files: Vec<Value> = mf
        .files
        .iter()
        .map(|f| json!({ "name": f.name, "have": have(&dir, &f.name).min(f.size) }))
        .collect();
    Ok(json!({ "files": files }))
}

/// `GET /peer/moves/{id}`: the move as this machine has it.
pub async fn peer_state(state: &AppState, from: &str, id: &str) -> ApiResult<Value> {
    Ok(incoming(state, id, from)?.view())
}

/// `GET /peer/moves/{id}/files/{name}`: what this machine has of a file.
pub async fn peer_file_size(
    state: &AppState,
    from: &str,
    id: &str,
    name: &str,
) -> ApiResult<Value> {
    incoming(state, id, from)?;
    let mf = load_manifest(state, id).ok_or_else(super::not_found)?;
    let f = mf
        .files
        .iter()
        .find(|f| f.name == name)
        .ok_or_else(|| ApiError::not_found("not_found", "No such file in this move."))?;
    let dir = move_folder(state, id).ok_or_else(super::not_found)?;
    Ok(json!({ "name": name, "size": have(&dir, name).min(f.size), "total": f.size }))
}

/// `PUT /peer/moves/{id}/files/{name}?offset=N`: the next chunk of a file,
/// appended when `offset` is what this machine has (else `409` with what it
/// has, so the source goes on from there).
pub async fn peer_put_chunk(
    state: &AppState,
    from: &str,
    id: &str,
    name: &str,
    offset: u64,
    bytes: axum::body::Bytes,
) -> ApiResult<Value> {
    let rec = incoming(state, id, from)?;
    if rec.state != MoveState::Sending {
        return Err(ApiError::conflict(
            "move_not_receiving",
            "This move takes no more files.",
        ));
    }
    if bytes.len() > super::CHUNK {
        return Err(bad("A chunk is at most 8 MiB."));
    }
    let mf = load_manifest(state, id).ok_or_else(super::not_found)?;
    let f = mf
        .files
        .iter()
        .find(|f| f.name == name)
        .cloned()
        .ok_or_else(|| ApiError::not_found("not_found", "No such file in this move."))?;
    let dir = move_folder(state, id).ok_or_else(super::not_found)?;
    let path = file_path(&dir, name).ok_or_else(|| bad("No such file."))?;
    let _one = state.machines.moves.chunks.lock().await;
    let total = f.size;
    let written = tokio::task::spawn_blocking(move || chunks::append(&path, offset, &bytes, total))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .map_err(|e| match e {
            AppendError::Refused(Refusal::Offset(size)) => ApiError::conflict(
                "offset_mismatch",
                "This computer has another part of the file.",
            )
            .with("size", size),
            AppendError::Refused(Refusal::TooLong) => bad("The file is longer than the move said."),
            AppendError::Refused(Refusal::TooLarge) => bad("A chunk is at most 8 MiB."),
            AppendError::Io(e) => {
                ApiError::internal(format!("The file could not be written here: {e}"))
            }
        })?;
    let total: u64 = mf
        .files
        .iter()
        .map(|f| have(&dir, &f.name).min(f.size))
        .sum();
    let fname = name.to_string();
    update(state, id, |r| {
        if let Some(e) = r.files.iter_mut().find(|e| e.name == fname) {
            e.sent = written;
        }
        r.bytes_sent = total;
    });
    Ok(json!({ "name": name, "size": written }))
}

/// Every file whole and as the manifest says (each hash checked), in its
/// place.
fn verify_files(dir: &Path, mf: &wire::Manifest) -> Result<(), String> {
    for f in &mf.files {
        let path = file_path(dir, &f.name).ok_or_else(|| format!("{} has no place", f.name))?;
        chunks::finish(&path, f.size, &f.sha256)?;
    }
    Ok(())
}

// ─── landing ───────────────────────────────────────────────────────────────────

fn landed_answer(rec: &MoveRecord) -> wire::Landed {
    wire::Landed {
        landed: true,
        target_cwd: rec.target_cwd.clone().unwrap_or_default(),
        target_folder: rec.target_folder.clone(),
        target_session: rec
            .target_session
            .clone()
            .unwrap_or_else(|| rec.session.clone()),
        account: rec.account.target.clone(),
        landing: rec.landing.clone().unwrap_or_default(),
        code: rec.code.as_ref().and_then(|c| c.landed.clone()),
        warnings: rec.warnings.clone(),
        resumed: rec.state == MoveState::Resumed,
        resume_error: rec.resume_error.clone(),
    }
}

/// Fails an incoming move here (nothing of it stays but its record).
fn fail_here(state: &AppState, id: &str, code: &str, message: &str) {
    let step = load(state, id)
        .map(|r| r.state)
        .unwrap_or(MoveState::Landing);
    if let Some(rec) = update(state, id, |r| {
        r.error = Some(MoveFailure {
            code: code.to_string(),
            message: message.to_string(),
            step,
            offer: None,
        });
        r.held.clear();
        r.reach(MoveState::Failed);
    }) {
        super::audit(state, "machine.move.failed", &rec, json!({ "code": code }));
    }
}

/// `POST /peer/moves/{id}/land`: every hash checked; the code applied (its
/// checkout, or a worktree); the conversation placed with the pull's
/// machinery; the move note kept for the session's next message. Nothing of
/// it stays when a step fails. Answering again for a move that landed is
/// the same answer (a retry sends nothing twice).
pub async fn peer_land(
    state: &AppState,
    ctx: &ClientContext,
    from: &str,
    id: &str,
) -> ApiResult<wire::Landed> {
    let _one = state.machines.moves.landing.lock().await;
    let rec = incoming(state, id, from)?;
    match rec.state {
        MoveState::Landed | MoveState::Committed | MoveState::Resumed => {
            return Ok(landed_answer(&rec))
        }
        MoveState::Sending | MoveState::Landing => {}
        _ => {
            return Err(ApiError::conflict(
                "move_finished",
                "This move has ended here.",
            ))
        }
    }
    let mf = load_manifest(state, id).ok_or_else(super::not_found)?;
    let dir = move_folder(state, id).ok_or_else(super::not_found)?;
    let (d, m2) = (dir.clone(), mf.clone());
    let verified = tokio::task::spawn_blocking(move || verify_files(&d, &m2))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if let Err(why) = verified {
        return Err(ApiError::conflict(
            "move_incomplete",
            format!("The move did not arrive whole: {why}."),
        ));
    }
    update(state, id, |r| r.reach(MoveState::Landing));
    let result = land_checked(state, ctx, from, id, &mf, &dir).await;
    match result {
        Ok(answer) => Ok(answer),
        Err(e) => {
            fail_here(state, id, &e.code, &e.message);
            Err(e)
        }
    }
}

async fn land_checked(
    state: &AppState,
    ctx: &ClientContext,
    from: &str,
    id: &str,
    mf: &wire::Manifest,
    dir: &Path,
) -> ApiResult<wire::Landed> {
    let ask = ask_of(mf);
    let epoch = ask.epoch;
    let resolved = resolve(state, ctx, from, &ask, Some(epoch)).await;
    if let Some(b) = resolved.answer.blockers.first() {
        return Err(ApiError::conflict("move_blocked", b.message.clone())
            .with("blocker", serde_json::to_value(b).unwrap_or(Value::Null)));
    }
    let (Some((account, _)), Some(folder), Some(mut cwd)) = (
        resolved.account.clone(),
        resolved.folder.clone(),
        resolved.cwd.clone(),
    ) else {
        return Err(ApiError::conflict(
            "move_blocked",
            "The move's account or folder was not found here.",
        ));
    };
    let home: ClaudeHome = state
        .accounts
        .home(&account)
        .unwrap_or_else(|| state.home.clone());
    let undo_root = crate::claude::transfer::ops::undo_root(state)
        .ok_or_else(|| ApiError::internal("No data directory."))?;
    // One data job at a time, as a pull.
    let _job = tokio::time::timeout(std::time::Duration::from_secs(60), state.jobs.lock.lock())
        .await
        .map_err(|_| {
            ApiError::conflict(
                "data_job_running",
                "A backup, restore or transfer is running here; try again when it has finished.",
            )
        })?;
    if crate::claude::transfer::ops::is_live(state, &mf.session) {
        return Err(ApiError::conflict(
            "session_live_there",
            "This computer runs the session right now.",
        ));
    }
    // Where it lands, before anything changes (a restart in the middle puts
    // back what it finds from here).
    update(state, id, |r| r.target_folder = Some(folder.clone()));
    // The code first: a stop by design changes nothing here.
    let git = state.machines.moves.git();
    let code_dir = dir.join("code");
    let mut applied = None;
    let mut worktree = false;
    if let Some(c) = &mf.code {
        worktree = c.apply == "worktree";
        let landing = if worktree {
            Landing::Worktree {
                root: worktrees_root(state)
                    .ok_or_else(|| ApiError::internal("No data directory."))?,
            }
        } else {
            Landing::Checkout
        };
        let mut req = ApplyRequest::new(&mf.session, landing);
        req.machine = m::this_name(state);
        req.remote = c.remote.clone();
        // This computer's choice for the project (D10).
        req.keep_remote_refs = crate::machines::setup::options::effective(state, &folder)
            .code
            .keep_remote_refs;
        let (g, repo, cd) = (git.clone(), PathBuf::from(&folder), code_dir.clone());
        let r = tokio::task::spawn_blocking(move || code::apply(&g, &repo, &cd, &req))
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        match r {
            Ok(a) => {
                if worktree {
                    cwd = join_rel(&a.path, &c.rel).display().to_string();
                }
                applied = Some(a);
            }
            Err(stop) if stop.is_stop() => {
                let mut e = ApiError::conflict(
                    "move_code_stopped",
                    format!("The code can't land here: {stop}"),
                );
                if let Some(offer) = stop.offer() {
                    e = e.with("offer", serde_json::to_value(offer).unwrap_or(Value::Null));
                }
                return Err(e);
            }
            Err(stop) => {
                return Err(ApiError::internal(format!(
                    "The code could not land here: {stop}"
                )))
            }
        }
    }
    // The files git ignores that the project carries, into the folder the
    // code landed in (what they replace kept in the move's folder for
    // undo); a step after this puts them back before the code.
    let mut carried = false;
    let mut replaced: Vec<String> = Vec::new();
    if let (Some(c), Some(a)) = (&mf.code, &applied) {
        if !c.ignored.is_empty() {
            let (top, archive, list, d) = (
                a.path.clone(),
                dir.join(ignored::FILE),
                c.ignored.clone(),
                dir.to_path_buf(),
            );
            let r = tokio::task::spawn_blocking(move || ignored::land(&top, &archive, &list, &d))
                .await
                .map_err(|e| ApiError::internal(e.to_string()));
            match r {
                Ok(Ok(l)) => {
                    carried = true;
                    replaced = l.replaced;
                }
                Ok(Err(why)) => {
                    code_back(&git, &folder, &code_dir, true).await;
                    return Err(ApiError::conflict(
                        "move_ignored_stopped",
                        format!("The files git ignores can't land here: {why}"),
                    ));
                }
                Err(e) => {
                    code_back(&git, &folder, &code_dir, true).await;
                    return Err(e);
                }
            }
        }
    }
    // The conversation.
    let job = crate::claude::transfer::new_job_id();
    update(state, id, |r| {
        r.landing = Some(LandingPart {
            job: Some(job.clone()),
            ..LandingPart::default()
        });
    });
    let host = crate::claude::backup::host_name();
    let project_dir = paths::project_dir_name(&cwd);
    let trust_key = mf
        .trust
        .then(|| crate::claude::backup::trust_key(&folder, true));
    let (archive, session, h, ur, j, acc) = (
        dir.join(session_pack::FILE),
        mf.session.clone(),
        home.clone(),
        undo_root.clone(),
        job.clone(),
        account.clone(),
    );
    let landed = tokio::task::spawn_blocking(move || {
        session_pack::land(&LandInputs {
            archive: &archive,
            id: &session,
            home: &h,
            project_dir: Some(&project_dir),
            undo_root: &ur,
            job_id: &j,
            trust_key: trust_key.as_deref(),
            host: &host,
            account: Some(&acc),
        })
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()));
    let report = match landed {
        Ok(Ok((report, _))) => report,
        Ok(Err(e)) => {
            ignored_back(dir, carried).await;
            code_back(&git, &folder, &code_dir, applied.is_some()).await;
            return Err(ApiError::conflict(e.code(), e.to_string()));
        }
        Err(e) => {
            ignored_back(dir, carried).await;
            code_back(&git, &folder, &code_dir, applied.is_some()).await;
            return Err(e);
        }
    };
    // Whose session it is here (a token account's own record).
    if state
        .accounts
        .get(&account)
        .is_some_and(|a| a.kind == crate::claude::accounts::AccountKind::Token)
    {
        state.accounts.remember_session(&mf.session, &account);
    }
    // The move note, for the session's next message here.
    let me = m::self_id(state).unwrap_or_default();
    let target_session = if mf.as_copy {
        uuid::Uuid::new_v4().to_string()
    } else {
        mf.session.clone()
    };
    let text = note::text(&Note {
        from_os: &mf.os,
        from_path: &mf.cwd,
        to_os: protocol::this_os(),
        to_path: &cwd,
        worktree_of: worktree.then_some(folder.as_str()),
        as_copy: mf.as_copy,
        recipe: &mf.recipe,
    });
    state.machines.leases.set_note(&target_session, text, &me);
    if let Err(e) = injected(state, "land") {
        // A failure after everything was placed: all of it goes back.
        state.machines.leases.take_note(&target_session);
        let (ur, j) = (undo_root.clone(), job.clone());
        let _ = tokio::task::spawn_blocking(move || crate::claude::transfer::undo(&ur, &j)).await;
        ignored_back(dir, carried).await;
        code_back(&git, &folder, &code_dir, applied.is_some()).await;
        return Err(e);
    }
    crate::claude::transfer::ops::rediscover(state).await;
    let transcript = {
        let (h, s) = (home.clone(), mf.session.clone());
        tokio::task::spawn_blocking(move || session_pack::transcript_state(&h, &s))
            .await
            .ok()
            .flatten()
    };
    let landing = LandingPart {
        job: Some(job),
        action: match &report.landed {
            Landed::New => "new",
            Landed::FastForward => "fastForward",
            Landed::Same => "same",
            Landed::Conflict { .. } => "conflict",
        }
        .to_string(),
        conflict: match &report.landed {
            Landed::Conflict { copy } => Some(copy.display().to_string()),
            _ => None,
        },
        trusted: report.trusted,
        transcript_bytes: transcript.as_ref().map(|t| t.1).unwrap_or(0),
        transcript_sha256: transcript.map(|t| t.2).unwrap_or_default(),
    };
    let mut warnings: Vec<Check> = report
        .warnings
        .iter()
        .map(|w| Check::new("landing", w.clone()))
        .collect();
    if !replaced.is_empty() {
        warnings.push(Check::new(
            "ignored_replaced",
            format!(
                "{} had its own {}: kept for Undo move.",
                m::this_name(state),
                replaced.join(", ")
            ),
        ));
    }
    if let Landed::Conflict { copy } = &report.landed {
        warnings.push(Check::new(
            "conflict_copy",
            format!(
                "It had gone on on {} too: those turns are kept beside it ({}).",
                m::this_name(state),
                copy.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ),
        ));
    }
    let rec = update(state, id, |r| {
        r.target_cwd = Some(cwd.clone());
        r.target_folder = Some(folder.clone());
        r.target_session = Some(target_session.clone());
        r.account.target = Some(account.clone());
        r.landing = Some(landing.clone());
        if let (Some(c), Some(a)) = (r.code.as_mut(), applied.clone()) {
            c.landed = Some(a);
        }
        r.warnings.extend(warnings.iter().cloned());
        r.undo_until = Some(m::now() + UNDO_KEEP_MS);
        r.reach(MoveState::Landed);
    })
    .ok_or_else(super::not_found)?;
    super::audit(
        state,
        "machine.move.landed",
        &rec,
        json!({ "action": landing.action, "code": rec.code.as_ref().map(|c| c.apply.clone()) }),
    );
    // A copy goes on at once, under its new id.
    if rec.as_copy {
        resume_landed(state, id, rec.dangerous).await;
    }
    let rec = load(state, id).ok_or_else(super::not_found)?;
    Ok(landed_answer(&rec))
}

/// Takes back the files git ignores a landing that failed had written (what
/// they replaced comes back), before the code goes back.
async fn ignored_back(move_dir: &Path, carried: bool) {
    if !carried {
        return;
    }
    let d = move_dir.to_path_buf();
    match tokio::task::spawn_blocking(move || ignored::undo(&d)).await {
        Ok(Ok(left)) if left.is_empty() => {}
        Ok(Ok(left)) => tracing::warn!(
            files = left.len(),
            "files git ignores a failed landing wrote changed meanwhile: left as they are"
        ),
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "files git ignores could not be put back after a failed landing")
        }
        Err(e) => tracing::warn!(error = %e, "putting files git ignores back did not run"),
    }
}

/// Puts the code back after a landing that failed once it was applied (off
/// the runtime: git runs as a child process).
async fn code_back(git: &code::GitRunner, folder: &str, code_dir: &Path, applied: bool) {
    if !applied {
        return;
    }
    let (g, repo, cd) = (git.clone(), PathBuf::from(folder), code_dir.to_path_buf());
    match tokio::task::spawn_blocking(move || code::undo(&g, &repo, &cd)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "a move's code could not be put back after a failed landing")
        }
        Err(e) => tracing::warn!(error = %e, "putting a move's code back did not run"),
    }
}

// ─── going on here ─────────────────────────────────────────────────────────────

/// A move that landed here goes on: the lease names this machine (or it is
/// a copy, which forks under its own id). A resume that fails leaves it
/// placed here and says why; trying again sends nothing twice.
pub(super) async fn resume_landed(state: &AppState, id: &str, may_dangerous: bool) {
    let Some(rec) = load(state, id) else { return };
    if rec.role != Role::In || !matches!(rec.state, MoveState::Landed | MoveState::Committed) {
        return;
    }
    if !rec.as_copy {
        let me = m::self_id(state).unwrap_or_default();
        let ours = state
            .machines
            .leases
            .get(&rec.session)
            .is_some_and(|l| l.home == me && l.epoch == rec.epoch);
        if !ours {
            return;
        }
        update(state, id, |r| {
            if r.state == MoveState::Landed {
                r.reach(MoveState::Committed);
            }
        });
    }
    let target_session = rec
        .target_session
        .clone()
        .unwrap_or_else(|| rec.session.clone());
    let account = rec
        .account
        .target
        .clone()
        .unwrap_or_else(|| crate::claude::accounts::DEFAULT_ID.to_string());
    let cwd = rec.target_cwd.clone().unwrap_or_default();
    let spec = Spec {
        session: &rec.session,
        fork_as: rec.as_copy.then_some(target_session.as_str()),
        cwd: &cwd,
        account: &account,
        settings: &rec.settings,
        may_dangerous,
        held: &rec.held,
    };
    let started = match injected(state, "resume") {
        Err(e) => Err(e),
        Ok(()) => resume::start(state, &spec).await,
    };
    let rec = match started {
        Ok(_) => update(state, id, |r| {
            r.resume_error = None;
            r.held.clear();
            r.reach(MoveState::Resumed);
        }),
        Err(e) => update(state, id, |r| r.resume_error = Some(resume::words(&e))),
    };
    if let Some(rec) = rec {
        let name = rec.title.clone().unwrap_or_else(|| "A session".into());
        if rec.state == MoveState::Resumed {
            super::audit(state, "machine.move.resumed", &rec, Value::Null);
            crate::server::ws::publish_notice(
                state,
                crate::claude::model::NoticeLevel::Success,
                format!(
                    "{name} now runs on this computer (it came from {}).",
                    rec.from.name
                ),
                Some(&target_session),
            );
        } else if let Some(e) = &rec.resume_error {
            crate::server::ws::publish_notice(
                state,
                crate::claude::model::NoticeLevel::Warning,
                format!(
                    "{name} came from {} but didn't start here: {e}",
                    rec.from.name
                ),
                Some(&target_session),
            );
        }
    }
}

/// `POST /peer/leases`: the source's leases (with this machine's `move`
/// grant for it). A move that landed here and that a lease now names this
/// machine for goes on at once, and the answer says whether it started; a
/// move this machine sent that came back is home again.
pub async fn peer_leases(
    state: &AppState,
    from: &str,
    lan: bool,
    body: wire::LeasesBody,
) -> ApiResult<Value> {
    let changed = lease::accept(state, from, lan, &body.leases).await;
    super::go_on_out(state, &changed).await;
    let me = m::self_id(state).unwrap_or_default();
    let mut resumed: Vec<wire::Resumed> = Vec::new();
    for w in &body.leases {
        let Some(l) = state.machines.leases.get(&w.session) else {
            continue;
        };
        if l.home != me || l.epoch != w.epoch {
            continue;
        }
        for rec in super::of_session(state, &w.session) {
            if rec.role == Role::In
                && !rec.as_copy
                && rec.from.id == from
                && rec.epoch == l.epoch
                && matches!(
                    rec.state,
                    MoveState::Landed | MoveState::Committed | MoveState::Resumed
                )
            {
                if rec.state != MoveState::Resumed {
                    resume_landed(state, &rec.id, rec.dangerous).await;
                }
                if let Some(after) = load(state, &rec.id) {
                    resumed.push(wire::Resumed {
                        move_id: after.id.clone(),
                        session: after.session.clone(),
                        ok: after.state == MoveState::Resumed,
                        error: after.resume_error.clone(),
                    });
                }
            }
        }
    }
    Ok(json!({ "leases": state.machines.leases.wire(), "resumed": resumed }))
}

// ─── undo, dropping ────────────────────────────────────────────────────────────

/// Undoes a move that landed here (decision 11): it must not have gone on
/// here since, and nothing may run it; the process stops, the code goes
/// back, the placed files go back into the undo folder (and what they
/// replaced comes back), the note goes, and the lease goes back to the
/// source (epoch + 1, under this move's id).
async fn undo_core(state: &AppState, id: &str) -> ApiResult<(MoveRecord, lease::WireLease)> {
    let _one = state.machines.moves.landing.lock().await;
    let rec = load(state, id).ok_or_else(super::not_found)?;
    let me = m::self_id(state).unwrap_or_default();
    let here = m::this_name(state);
    if rec.role != Role::In || rec.as_copy {
        return Err(ApiError::conflict(
            "move_not_undoable",
            "Only a move that came here is undone here.",
        ));
    }
    if !matches!(
        rec.state,
        MoveState::Landed | MoveState::Committed | MoveState::Resumed
    ) {
        return Err(ApiError::conflict(
            "move_not_undoable",
            "Only a move that finished can be undone.",
        ));
    }
    let lease = state.machines.leases.get(&rec.session);
    let leased_here = lease
        .as_ref()
        .is_some_and(|l| l.home == me && l.epoch == rec.epoch);
    if rec.state != MoveState::Landed && !leased_here {
        return Err(ApiError::conflict(
            "move_not_undoable",
            "It moved again since: undo that move first.",
        ));
    }
    let session = rec.session.clone();
    if crate::claude::transfer::ops::is_live(state, &session) && !owned::has_handle(state, &session)
    {
        return Err(ApiError::conflict(
            "session_live",
            format!("Another process runs it on {here}: stop it there first."),
        ));
    }
    if state.control.is_idle(&session) == Some(false) {
        return Err(ApiError::conflict(
            "session_busy",
            format!("It is working on {here} right now: try again when its turn ends."),
        ));
    }
    let landing = rec.landing.clone().unwrap_or_default();
    let account = rec
        .account
        .target
        .clone()
        .unwrap_or_else(|| crate::claude::accounts::DEFAULT_ID.to_string());
    let home = state
        .accounts
        .home(&account)
        .unwrap_or_else(|| state.home.clone());
    let (h, s) = (home.clone(), session.clone());
    let now = tokio::task::spawn_blocking(move || session_pack::transcript_state(&h, &s))
        .await
        .ok()
        .flatten();
    if let Some((_, size, sha)) = &now {
        if *size != landing.transcript_bytes || *sha != landing.transcript_sha256 {
            return Err(ApiError::conflict(
                "move_went_on",
                format!(
                    "It went on on {here} after the move: bring it back instead (an undo would \
                     drop what happened here)."
                ),
            ));
        }
    }
    // Stopped here (it was resumed, but nothing happened in it since).
    if owned::has_handle(state, &session) {
        if !state
            .control
            .stop_and_wait(&session, std::time::Duration::from_secs(20))
            .await
        {
            return Err(ApiError::internal(
                "Its process here did not stop in time, so the move was not undone.",
            ));
        }
        crate::pending::withdraw_session(state, &session, "the move was undone").await;
        crate::server::monitor::tick_once(state).await;
    }
    // The files git ignores it carried, first: the code's own check then
    // sees the landing as the move left it (whatever this computer ignores).
    // Each goes back only while it is what the move wrote.
    let move_dir = move_folder(state, id).ok_or_else(super::not_found)?;
    let carried = rec.code.as_ref().is_some_and(|c| !c.ignored.is_empty());
    let mut left: Vec<String> = Vec::new();
    if carried {
        let d = move_dir.clone();
        left = tokio::task::spawn_blocking(move || ignored::undo(&d))
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?
            .map_err(|e| {
                ApiError::conflict(
                    "move_ignored_changed",
                    format!("The files git ignores can't be put back here: {e}"),
                )
            })?;
    }
    // The code.
    if let (Some(c), Some(folder)) = (rec.code.as_ref(), rec.target_folder.as_ref()) {
        if c.landed.is_some() {
            let (g, repo, cd) = (
                state.machines.moves.git(),
                PathBuf::from(folder),
                move_dir.join("code"),
            );
            let undone = tokio::task::spawn_blocking(move || code::undo(&g, &repo, &cd))
                .await
                .map_err(|e| ApiError::internal(e.to_string()))?;
            match undone {
                Ok(_) | Err(code::CodeError::AlreadyUndone) => {}
                Err(e) => {
                    // The move stays: so do the files it carried.
                    if carried {
                        let d = move_dir.clone();
                        match tokio::task::spawn_blocking(move || ignored::redo(&d)).await {
                            Ok(Ok(())) => {}
                            Ok(Err(why)) => {
                                tracing::warn!(error = %why, "files git ignores could not land again after a refused undo")
                            }
                            Err(why) => {
                                tracing::warn!(error = %why, "landing files git ignores again did not run")
                            }
                        }
                    }
                    return Err(ApiError::conflict(
                        "move_code_changed",
                        format!("The code can't be put back here: {e}"),
                    ));
                }
            }
        }
    }
    // The conversation.
    if let Some(job) = &landing.job {
        match crate::claude::transfer::ops::undo(state, job).await {
            Ok(()) => {}
            Err(e) if e.code() == "undo_conflict" && e.to_string().contains("undone already") => {}
            Err(e) => {
                return Err(ApiError::conflict(
                    e.code(),
                    format!("The conversation can't be put back here: {e}"),
                ))
            }
        }
    }
    state.machines.leases.take_note(&session);
    // The lease back to where it came from.
    let epoch = lease.map(|l| l.epoch).unwrap_or(rec.epoch) + 1;
    let back = Lease {
        home: rec.from.id.clone(),
        epoch,
        since: m::now(),
        move_id: Some(rec.id.clone()),
        note: None,
    };
    state.machines.leases.set(&session, back, &me);
    lease::changed_here(state, std::slice::from_ref(&session)).await;
    let wire = state
        .machines
        .leases
        .wire_of(&session)
        .ok_or_else(|| ApiError::internal("The lease was not kept."))?;
    let rec = update(state, id, |r| {
        r.undo_until = None;
        r.held.clear();
        if !left.is_empty() {
            r.warnings.push(Check::new(
                "ignored_left",
                format!(
                    "Changed on {here} since the move, so left as they are: {}.",
                    left.join(", ")
                ),
            ));
        }
        r.reach(MoveState::Undone);
    })
    .ok_or_else(super::not_found)?;
    super::audit(state, "machine.move.undone", &rec, Value::Null);
    Ok((rec, wire))
}

/// `POST /peer/moves/{id}/undo`: the source asks this machine to undo a move
/// it sent; the answer carries the lease back.
pub async fn peer_undo(state: &AppState, from: &str, id: &str) -> ApiResult<Value> {
    incoming(state, id, from)?;
    let (rec, wire) = undo_core(state, id).await?;
    Ok(json!({ "undone": true, "lease": wire, "move": rec.view() }))
}

/// `POST /api/moves/{id}/undo` here, for a move that came here: undone, and
/// the source told (it learns at its next hello when it is away).
pub(super) async fn undo_here(
    state: &AppState,
    ctx: &ClientContext,
    rec: &MoveRecord,
) -> ApiResult<MoveRecord> {
    let (rec, wire) = undo_core(state, &rec.id).await?;
    let acting = m::acting::header_for(state, ctx);
    let body = json!({ "leases": [wire] });
    let _ = super::call(
        state,
        &rec.from.id,
        "POST",
        "/peer/leases",
        Some(&body),
        &acting,
        std::time::Duration::from_secs(60),
    )
    .await;
    Ok(rec)
}

/// Takes back what a move that did not land left here (the source failed,
/// was cancelled or restarted). A move that landed but whose lease never
/// came is undone the same way; one that went on is too late.
async fn drop_core(state: &AppState, id: &str, code: &str, message: &str) -> ApiResult<()> {
    let rec = load(state, id).ok_or_else(super::not_found)?;
    if rec.state.is_final() {
        // A landing that failed here put everything back already: only what
        // arrived is left to go.
        if matches!(rec.state, MoveState::Failed | MoveState::Cancelled) {
            if let Some(dir) = move_folder(state, id) {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
        return Ok(());
    }
    if matches!(rec.state, MoveState::Committed | MoveState::Resumed) {
        return Err(ApiError::conflict(
            "move_too_late",
            "It lives here now: undo the move instead.",
        ));
    }
    let dir = move_folder(state, id);
    if rec.state == MoveState::Landed || rec.state == MoveState::Landing {
        // Put back whatever the landing did.
        if let Some(job) = rec.landing.as_ref().and_then(|l| l.job.clone()) {
            if let Some(root) = crate::claude::transfer::ops::undo_root(state) {
                if crate::claude::transfer::manifest(&root, &job).is_some() {
                    let _ = crate::claude::transfer::ops::undo(state, &job).await;
                }
            }
        }
        if let Some(dir) = &dir {
            // The files git ignores it carried, before the code.
            let d = dir.clone();
            let _ = tokio::task::spawn_blocking(move || ignored::undo(&d)).await;
        }
        if let (Some(dir), Some(folder)) = (&dir, rec.target_folder.as_ref()) {
            let applied = dir.join("code").join(code::APPLY);
            if applied.exists() {
                let (g, repo, cd) = (
                    state.machines.moves.git(),
                    PathBuf::from(folder),
                    dir.join("code"),
                );
                let _ = tokio::task::spawn_blocking(move || code::undo(&g, &repo, &cd)).await;
            }
        }
        let target = rec
            .target_session
            .clone()
            .unwrap_or_else(|| rec.session.clone());
        state.machines.leases.take_note(&target);
    }
    if let Some(dir) = &dir {
        // The received files go; the code's record stays only while it
        // could matter (it is put back above).
        let _ = std::fs::remove_dir_all(dir);
    }
    fail_here(state, id, code, message);
    Ok(())
}

/// `DELETE /peer/moves/{id}`: the source gave the move up.
pub async fn peer_drop(state: &AppState, from: &str, id: &str) -> ApiResult<Value> {
    let _one = state.machines.moves.landing.lock().await;
    let rec = match incoming(state, id, from) {
        Ok(r) => r,
        Err(_) => return Ok(json!({ "dropped": false })),
    };
    let name = rec.from.name.clone();
    drop_core(
        state,
        id,
        "source_gave_up",
        &format!("{name} gave the move up."),
    )
    .await?;
    Ok(json!({ "dropped": true }))
}

/// After a restart, an incoming move that had not landed is dropped.
pub(super) async fn drop_after_restart(state: &AppState, rec: &MoveRecord) {
    let _one = state.machines.moves.landing.lock().await;
    let _ = drop_core(
        state,
        &rec.id,
        "interrupted",
        "Mother Claude stopped while it was coming here.",
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_under_a_top_is_plain_names_only() {
        assert!(safe_rel(""));
        assert!(safe_rel("src-tauri/src"));
        for bad in ["../x", "a/../b", "/abs", "a//b", r"a\b", "C:", "a/./b"] {
            assert!(!safe_rel(bad), "{bad}");
        }
        assert_eq!(
            join_rel(Path::new("base"), "a/b"),
            Path::new("base").join("a").join("b")
        );
    }

    #[test]
    fn only_the_known_files_of_a_package_have_a_place() {
        let d = Path::new("d");
        assert!(file_path(d, "session.tar.gz").is_some());
        assert!(file_path(d, "meta.json").is_some());
        assert!(file_path(d, "move.bundle").is_some());
        assert_eq!(
            file_path(d, "ignored.tar.gz"),
            Some(d.join("ignored.tar.gz"))
        );
        assert!(file_path(d, &format!("lfs.{}", "a".repeat(64))).is_some());
        for bad in ["../x", "settings.json", "lfs.zz", "lfs.", "code/meta.json"] {
            assert!(file_path(d, bad).is_none(), "{bad}");
        }
        assert_eq!(
            chunks::part_of(&d.join("session.tar.gz")),
            d.join("session.tar.gz.part")
        );
    }
}
