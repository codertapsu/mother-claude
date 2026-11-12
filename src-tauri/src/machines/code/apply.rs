//! The target's half (plan §5.10): landing a move, and undoing it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::check::{
    now_rfc3339, read_json, safe_name, sha256_hex, stamp, valid_remote_name, write_json,
};
use super::git::{GitOp, GitRunner};
use super::repo::{
    at, busy, checked_trees, empty_tree, index_tree, ls_tree, parked_state, parse_status, raw_of,
    remove_empty_parents, repo_info, same_raw, status_z, tree_diff, unmerged, worktree_tree,
    DirCache, Parked, RepoInfo,
};
use super::snapshot::lfs_object;
use super::types::{
    io_err, Applied, ApplyOutcome, ApplyRecord, ApplyRequest, ApplyStop, BranchAction, CodeError,
    Landing, LandingKind, ParkedTrees, Raw, Reset, SnapshotMeta, SubmoduleLanding, SubmoduleState,
    TransferKind, TreeState, UndoInfo, Undone, APPLY, BUNDLE, FORMAT, META,
};

/// The move's record, checked (it comes from another machine).
pub(super) fn load_meta(in_dir: &Path) -> Result<SnapshotMeta, CodeError> {
    let meta: SnapshotMeta = read_json(&in_dir.join(META))?;
    meta.validate()?;
    Ok(meta)
}

pub(super) fn load_record(in_dir: &Path) -> Result<ApplyRecord, CodeError> {
    let path = in_dir.join(APPLY);
    if !path.exists() {
        return Err(CodeError::NotApplied);
    }
    let record: ApplyRecord = read_json(&path)?;
    record.validate()?;
    Ok(record)
}

fn fetch_args() -> [&'static str; 6] {
    // F5: never follow submodules (a commit missing on a submodule's remote
    // would fail the whole fetch); no tags, no FETCH_HEAD, no maintenance.
    [
        "fetch",
        "-q",
        "--no-tags",
        "--no-write-fetch-head",
        "--no-auto-maintenance",
        "--no-recurse-submodules",
    ]
}

/// Brings the move's commits into `refs/mother-claude/moves/…` and checks
/// they are what the record says.
fn fetch_move(
    op: &GitOp,
    top: &Path,
    in_dir: &Path,
    meta: &SnapshotMeta,
    req: &ApplyRequest,
    record: &mut ApplyRecord,
) -> Result<(), CodeError> {
    fetch_into(
        op,
        top,
        in_dir,
        meta,
        req.remote.as_deref(),
        req.keep_remote_refs,
    )?;
    record.fetched = true;
    Ok(())
}

/// [`fetch_move`]'s git: the snapshot's two commits fetched (from the
/// bundle in `in_dir`, or the remote) into its refs, then checked against
/// the record — the trees, the base, every object, the manifest. Shared
/// with a run's work folder (`super::work`).
pub(super) fn fetch_into(
    op: &GitOp,
    top: &Path,
    in_dir: &Path,
    meta: &SnapshotMeta,
    remote: Option<&str>,
    keep_remote_refs: bool,
) -> Result<(), CodeError> {
    let refs = &meta.refs;
    let have = |r: &str| op.rev(top, &format!("{r}^{{commit}}"));
    let already = have(&refs.snap)?.as_deref() == Some(meta.snap.as_str())
        && have(&refs.index)?.as_deref() == Some(meta.index_commit.as_str());
    match meta.transfer {
        TransferKind::Bundle => {
            if !already {
                let file = in_dir.join(BUNDLE);
                op.cmd(top)
                    .args(["bundle", "verify", "-q"])
                    .arg(&file)
                    .raw()?;
                op.cmd(top)
                    .args(fetch_args())
                    .arg(&file)
                    .arg(format!("{0}:{0}", refs.snap))
                    .arg(format!("{0}:{0}", refs.index))
                    .raw()?;
            }
        }
        TransferKind::Remote => {
            let r = meta.remote.as_ref().ok_or(CodeError::FetchMismatch)?;
            let name = remote.unwrap_or(&r.remote);
            if !valid_remote_name(name) {
                return Err(CodeError::Invalid {
                    what: "remote name",
                    value: name.to_string(),
                });
            }
            let url = op
                .cmd(top)
                .args(["config", "--get"])
                .arg(format!("remote.{name}.url"))
                .ok()?;
            if !url {
                return Err(CodeError::UnknownRemote(name.to_string()));
            }
            if !already {
                op.cmd(top)
                    .args(fetch_args())
                    .arg(name)
                    .arg(format!("{}:{}", r.hidden.snap, refs.snap))
                    .arg(format!("{}:{}", r.hidden.index, refs.index))
                    .raw()?;
                if !keep_remote_refs {
                    // Kept here for undo; gone from the remote.
                    let deleted = op
                        .cmd(top)
                        .args(["push", "--no-verify", "-q", "--recurse-submodules=no"])
                        .arg(name)
                        .args(["--delete", &r.hidden.snap, &r.hidden.index])
                        .output()?;
                    if !deleted.success {
                        tracing::warn!(remote = %name, error = %deleted.stderr.trim(), "a move's hidden refs could not be deleted from the remote");
                    }
                }
            }
        }
    }
    if have(&refs.snap)?.as_deref() != Some(meta.snap.as_str())
        || have(&refs.index)?.as_deref() != Some(meta.index_commit.as_str())
    {
        return Err(CodeError::FetchMismatch);
    }
    // The commits name the trees and the base the record says…
    let names = op.git(
        top,
        &[
            "rev-parse",
            &format!("{}^{{tree}}", meta.snap),
            &format!("{}^{{tree}}", meta.index_commit),
        ],
    )?;
    let first_parent = |c: &str| op.rev(top, &format!("{c}^1"));
    if names.lines().collect::<Vec<_>>() != [meta.wt_tree.as_str(), meta.idx_tree.as_str()]
        || first_parent(&meta.snap)? != meta.base
        || first_parent(&meta.index_commit)? != meta.base
    {
        return Err(CodeError::FetchMismatch);
    }
    // …their objects are all here (a bundle that relied on objects this
    // repository lacks fails here; submodule commits are not followed)…
    let mut objects = op
        .cmd(top)
        .args(["rev-list", "--objects", "--missing=error", "--quiet"])
        .args([&meta.snap, &meta.index_commit]);
    if let Some(base) = &meta.base {
        objects = objects.arg("--not").arg(base);
    }
    objects.raw()?;
    // …and the manifest is exactly the snapshot's tree.
    let tree: Vec<(String, String, String)> = ls_tree(op, top, &meta.wt_tree)?
        .into_iter()
        .map(|e| (e.path, e.mode, e.oid))
        .collect();
    let listed: Vec<(String, String, String)> = meta
        .manifest
        .iter()
        .map(|e| (e.path.clone(), e.mode.clone(), e.oid.clone()))
        .collect();
    if tree != listed {
        return Err(CodeError::FetchMismatch);
    }
    Ok(())
}

/// Copies the Git LFS objects the move carried (each checked against its id).
pub(super) fn install_lfs(
    info: &RepoInfo,
    in_dir: &Path,
    meta: &SnapshotMeta,
) -> Result<usize, CodeError> {
    let Some(lfs) = &meta.lfs else {
        return Ok(0);
    };
    let mut n = 0;
    for oid in &lfs.carried {
        let src = lfs_object(in_dir, oid);
        let dst = lfs_object(&info.common_dir, oid);
        if dst.exists() {
            continue;
        }
        let bytes =
            std::fs::read(&src).map_err(io_err(format!("read the Git LFS object {oid}")))?;
        if sha256_hex(&bytes) != *oid {
            tracing::warn!(%oid, "a carried Git LFS object does not match its id; skipped");
            continue;
        }
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)
                .map_err(io_err(format!("create {}", parent.display())))?;
        }
        let tmp = dst.with_extension("mc-tmp");
        std::fs::write(&tmp, &bytes).map_err(io_err(format!("write the Git LFS object {oid}")))?;
        std::fs::rename(&tmp, &dst).map_err(io_err(format!("write the Git LFS object {oid}")))?;
        n += 1;
    }
    Ok(n)
}

pub(super) fn same_place(a: &Path, b: &Path) -> bool {
    match (dunce::canonicalize(a), dunce::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// Another worktree that has `branch` checked out.
pub(super) fn other_worktree_on(
    op: &GitOp,
    top: &Path,
    branch: &str,
) -> Result<Option<PathBuf>, CodeError> {
    let out = op
        .cmd(top)
        .args(["worktree", "list", "--porcelain", "-z"])
        .raw()?;
    let mut wt: Option<PathBuf> = None;
    for field in out.split(|b| *b == 0) {
        let line = String::from_utf8_lossy(field);
        if let Some(p) = line.strip_prefix("worktree ") {
            wt = Some(PathBuf::from(p));
        } else if line.strip_prefix("branch ") == Some(branch) {
            if let Some(w) = wt.as_ref().filter(|w| !same_place(w, top)) {
                return Ok(Some(w.clone()));
            }
        }
    }
    Ok(None)
}

/// Submodule commits the snapshot names, checked out (plumbing) where the
/// submodule is initialized, clean and has the commit; otherwise reported.
fn land_submodules(
    op: &GitOp,
    top: &Path,
    meta: &SnapshotMeta,
) -> Result<Vec<SubmoduleLanding>, CodeError> {
    let mut out = Vec::new();
    for e in meta.manifest.iter().filter(|e| e.mode == "160000") {
        let p = at(top, &e.path);
        let landing = |state, from: Option<String>| SubmoduleLanding {
            path: e.path.clone(),
            state,
            want: Some(e.oid.clone()),
            from,
        };
        if !p.join(".git").exists() {
            out.push(landing(SubmoduleState::NotInitialized, None));
            continue;
        }
        let head = op.rev(&p, "HEAD^{commit}")?;
        if head.as_deref() == Some(e.oid.as_str()) {
            out.push(landing(SubmoduleState::Same, head));
            continue;
        }
        if !op
            .cmd(&p)
            .args(["cat-file", "-e", &format!("{}^{{commit}}", e.oid)])
            .ok()?
        {
            out.push(landing(SubmoduleState::Missing, head));
            continue;
        }
        if !status_z(op, &p)?.is_empty() {
            out.push(landing(SubmoduleState::Dirty, head));
            continue;
        }
        let from = match &head {
            Some(h) => h.clone(),
            None => empty_tree(op, &p)?,
        };
        op.cmd(&p)
            .args(["read-tree", "-m", "-u", &from, &e.oid])
            .raw()?;
        op.cmd(&p)
            .args(["update-ref", "--no-deref", "HEAD", &e.oid])
            .raw()?;
        out.push(landing(SubmoduleState::Updated, head));
    }
    Ok(out)
}

pub(super) fn refresh(op: &GitOp, top: &Path) -> Result<(), CodeError> {
    // `read-tree` leaves no stat data; a refresh re-hashes and records it.
    // Exit 1 only means some entries need updating.
    op.cmd(top)
        .args(["update-index", "-q", "--refresh"])
        .output()?;
    Ok(())
}

/// §5.10 apply: lands the move in `in_dir` at `repo` — its checkout when it
/// is clean or parked for this session (F7), or a new worktree. A stop
/// leaves the checkout as it was; `apply.json` records either.
pub fn apply(
    git: &GitRunner,
    repo: &Path,
    in_dir: &Path,
    req: &ApplyRequest,
) -> Result<Applied, ApplyStop> {
    let op = git.begin()?;
    let meta = load_meta(in_dir)?;
    if meta.session != req.session {
        return Err(CodeError::WrongSession.into());
    }
    let info = repo_info(&op, repo)?;
    let mut record = ApplyRecord {
        version: FORMAT,
        session: meta.session.clone(),
        epoch: meta.epoch.clone(),
        machine: safe_name(&req.machine),
        created: now_rfc3339(),
        outcome: ApplyOutcome::Stopped,
        landing: match req.landing {
            Landing::Checkout => LandingKind::Checkout,
            Landing::Worktree { .. } => LandingKind::Worktree,
        },
        applied: None,
        undo: None,
        stop: None,
        offer: None,
        fetched: false,
        hook_intrusions: Vec::new(),
        undone: false,
    };
    let result = match &req.landing {
        Landing::Checkout => land_checkout(&op, &info, in_dir, &meta, req, &mut record),
        Landing::Worktree { root } => {
            land_worktree(&op, &info, in_dir, &meta, req, root, &mut record)
        }
    };
    record.hook_intrusions = op.intrusions();
    match result {
        Ok(applied) => {
            record.outcome = ApplyOutcome::Applied;
            record.applied = Some(applied.clone());
            write_json(&in_dir.join(APPLY), &record)?;
            Ok(applied)
        }
        Err(ApplyStop::Failed(e)) => Err(ApplyStop::Failed(e)),
        Err(stop) => {
            record.stop = Some(stop.to_string());
            record.offer = stop.offer();
            write_json(&in_dir.join(APPLY), &record)?;
            Err(stop)
        }
    }
}

fn short_branch(b: &str) -> String {
    b.strip_prefix("refs/heads/").unwrap_or(b).to_string()
}

/// A code sync's parked rule (plan §5.15): the checkout holds exactly
/// `want` — HEAD, its branch, the staged tree and the working tree — or
/// says what changed.
fn parked_at(op: &GitOp, info: &RepoInfo, want: &TreeState) -> Result<Parked, ApplyStop> {
    let not = |what: &str| {
        Err(ApplyStop::NotParked {
            what: what.to_string(),
        })
    };
    if info.sym != want.branch {
        return not("another branch is checked out");
    }
    if info.head != want.head {
        return not("HEAD moved");
    }
    if unmerged(op, &info.top)? {
        return not("a merge is half done");
    }
    let (idx_tree, wt_tree, _) = checked_trees(op, info)?;
    if idx_tree != want.idx_tree {
        return not("what is staged changed");
    }
    if wt_tree != want.wt_tree {
        return not("files changed");
    }
    Ok(Parked {
        epoch: "sync".to_string(),
        wt_tree,
        idx_tree,
    })
}

fn land_checkout(
    op: &GitOp,
    info: &RepoInfo,
    in_dir: &Path,
    meta: &SnapshotMeta,
    req: &ApplyRequest,
    record: &mut ApplyRecord,
) -> Result<Applied, ApplyStop> {
    let top = &info.top;
    if let Some(what) = busy(info) {
        return Err(ApplyStop::Busy { what });
    }
    let status = status_z(op, top)?;
    let parked: Option<Parked> = match &req.parked {
        // A code sync: parked exactly at its last synced state, clean or not.
        Some(want) => {
            let p = parked_at(op, info, want)?;
            if status.is_empty() {
                None
            } else {
                Some(p)
            }
        }
        None if status.is_empty() => None,
        None => match parked_state(op, info, &meta.session)? {
            Some(p) => Some(p),
            None => {
                return Err(ApplyStop::Dirty {
                    changed: parse_status(&status).len(),
                })
            }
        },
    };
    fetch_move(op, top, in_dir, meta, req, record)?;
    let lfs_installed = install_lfs(info, in_dir, meta)?;
    let empty = empty_tree(op, top)?;

    let (action, tip) = match &meta.branch {
        Some(b) => {
            let tip = op.rev(top, &format!("{b}^{{commit}}"))?;
            let action = match (&tip, &meta.base) {
                (Some(t), Some(base)) if t == base => BranchAction::Same,
                (Some(t), Some(base))
                    if op
                        .cmd(top)
                        .args(["merge-base", "--is-ancestor", t, base])
                        .ok()? =>
                {
                    BranchAction::FastForward
                }
                // Commits the base lacks: moved only as the request allows
                // (its tip is kept under the undo ref below).
                (Some(t), Some(_))
                    if match &req.reset {
                        Reset::Never => false,
                        Reset::IfTip(tip) => tip == t,
                        Reset::Always => true,
                    } =>
                {
                    BranchAction::Reset
                }
                (Some(t), _) => {
                    return Err(ApplyStop::Diverged {
                        branch: short_branch(b),
                        target_tip: t.clone(),
                    })
                }
                (None, Some(_)) => BranchAction::Create,
                (None, None) => BranchAction::Unborn,
            };
            if let Some(worktree) = other_worktree_on(op, top, b)? {
                return Err(ApplyStop::CheckedOutElsewhere {
                    branch: short_branch(b),
                    worktree,
                });
            }
            (action, tip)
        }
        None => (BranchAction::Detach, None),
    };

    // Clean means nothing git shows; files it ignores are not shown, and
    // `read-tree -u` overwrites them without a word.
    let from = match (&parked, &info.head) {
        (Some(p), _) => p.wt_tree.clone(),
        (None, Some(h)) => h.clone(),
        (None, None) => empty.clone(),
    };
    let base_rev = meta.base.clone().unwrap_or_else(|| empty.clone());
    let blocked = overwrites(op, top, &from, &[&base_rev, &meta.snap])?;
    if !blocked.is_empty() {
        return Err(ApplyStop::WouldOverwrite { paths: blocked });
    }

    // Undo: HEAD and the branch's old tip, kept as refs. A branch that is
    // reset always has its old tip under `…/branch` (plan §5.10: *keeping
    // T's tip*), even when it was HEAD too.
    let stamp = stamp();
    let undo_ref = format!("refs/mother-claude/undo/{stamp}");
    if let Some(h) = &info.head {
        op.cmd(top)
            .args(["update-ref", &format!("{undo_ref}/head"), h, ""])
            .raw()?;
    }
    if let Some(t) = tip
        .as_ref()
        .filter(|t| Some(*t) != info.head.as_ref() || action == BranchAction::Reset)
    {
        op.cmd(top)
            .args(["update-ref", &format!("{undo_ref}/branch"), t, ""])
            .raw()?;
    }
    record.undo = Some(UndoInfo {
        stamp,
        ref_name: undo_ref.clone(),
        prev_sym: info.sym.clone(),
        prev_head: info.head.clone(),
        branch_old: tip.clone(),
        action,
        parked: parked.as_ref().map(|p| ParkedTrees {
            epoch: p.epoch.clone(),
            wt_tree: p.wt_tree.clone(),
            idx_tree: p.idx_tree.clone(),
        }),
    });

    // A step below that fails (a file git cannot write) puts the checkout
    // back as it was.
    let put_back = |moved: bool| {
        if moved {
            let _ = op
                .cmd(top)
                .args(["read-tree", "-m", "-u", &base_rev, &from])
                .raw();
        }
        if let Some(p) = &parked {
            let _ = op.cmd(top).args(["read-tree", &p.idx_tree]).raw();
        }
        let _ = refresh(op, top);
    };
    // 1. The checkout at base. A clean checkout's index is its HEAD's tree;
    //    a parked one's working tree is the parked snapshot's tree, so the
    //    index is set to that tree first and the move starts from there.
    if let Some(p) = &parked {
        if let Err(e) = op.cmd(top).args(["read-tree", &p.wt_tree]).raw() {
            put_back(false);
            return Err(e.into());
        }
        refresh(op, top)?;
    }
    let moved = from != base_rev;
    if moved {
        if let Err(e) = op
            .cmd(top)
            .args(["read-tree", "-m", "-u", &from, &base_rev])
            .raw()
        {
            put_back(false);
            return Err(e.into());
        }
    }
    // 2. The working tree = the snapshot.
    if let Err(e) = op
        .cmd(top)
        .args(["read-tree", "-m", "-u", &base_rev, &meta.snap])
        .raw()
    {
        put_back(moved);
        return Err(e.into());
    }
    // 3. The branch at base (created, fast-forwarded or reset), HEAD on it.
    match (&meta.branch, &meta.base, action) {
        (Some(b), Some(base), BranchAction::FastForward | BranchAction::Reset) => {
            let old = tip.clone().unwrap_or_default();
            op.cmd(top).args(["update-ref", b, base, &old]).raw()?;
        }
        (Some(b), Some(base), BranchAction::Create) => {
            op.cmd(top).args(["update-ref", b, base, ""]).raw()?;
        }
        _ => {}
    }
    match (&meta.branch, &meta.base) {
        (Some(b), _) if info.sym.as_ref() != Some(b) => {
            op.cmd(top).args(["symbolic-ref", "HEAD", b]).raw()?;
        }
        (None, Some(base)) => {
            op.cmd(top)
                .args(["update-ref", "--no-deref", "HEAD", base])
                .raw()?;
        }
        _ => {}
    }
    // 4. The index = what was staged.
    op.cmd(top).args(["read-tree", &meta.idx_tree]).raw()?;
    refresh(op, top)?;
    let submodules = land_submodules(op, top, meta)?;
    Ok(Applied {
        landing: LandingKind::Checkout,
        path: top.clone(),
        action: Some(action),
        parked: parked.map(|p| p.epoch),
        wt_branch: None,
        undo_ref: Some(undo_ref),
        submodules,
        lfs_installed,
    })
}

/// Paths the landing would write (in `revs`, not in `from`) where the
/// checkout already has an untracked entry (a file it ignores), or where a
/// parent folder is such a file.
pub(super) fn overwrites(
    op: &GitOp,
    top: &Path,
    from: &str,
    revs: &[&str],
) -> Result<Vec<String>, CodeError> {
    // Tracked under any case: git itself manages those.
    let tracked: HashSet<String> = ls_tree(op, top, from)?
        .into_iter()
        .map(|e| e.path.to_lowercase())
        .collect();
    let mut added = std::collections::BTreeSet::new();
    for rev in revs.iter().filter(|r| **r != from) {
        let out = op
            .cmd(top)
            .args(["diff-tree", "-r", "-z", "--name-only", "--no-renames"])
            .args(["--diff-filter=A", from, rev])
            .raw()?;
        for p in out.split(|b| *b == 0).filter(|p| !p.is_empty()) {
            added.insert(String::from_utf8_lossy(p).into_owned());
        }
    }
    let mut found = Vec::new();
    for p in added {
        if tracked.contains(&p.to_lowercase()) {
            continue;
        }
        let mut blocked = std::fs::symlink_metadata(at(top, &p)).is_ok();
        let parts: Vec<&str> = p.split('/').collect();
        for i in 1..parts.len() {
            if blocked {
                break;
            }
            let prefix = parts[..i].join("/");
            if tracked.contains(&prefix.to_lowercase()) {
                continue;
            }
            match std::fs::symlink_metadata(at(top, &prefix)) {
                Ok(m) => blocked = !m.is_dir(),
                Err(_) => break,
            }
        }
        if blocked {
            found.push(p);
            if found.len() >= 20 {
                break;
            }
        }
    }
    Ok(found)
}

/// `mother-claude/<session>`, or the first `-<n>` that is free.
fn free_branch(op: &GitOp, top: &Path, session: &str) -> Result<String, CodeError> {
    for n in 1..100 {
        let name = if n == 1 {
            format!("refs/heads/mother-claude/{session}")
        } else {
            format!("refs/heads/mother-claude/{session}-{n}")
        };
        if op.rev(top, &name)?.is_none() {
            return Ok(name);
        }
    }
    Err(CodeError::Invalid {
        what: "worktree branch",
        value: format!("mother-claude/{session}"),
    })
}

fn land_worktree(
    op: &GitOp,
    info: &RepoInfo,
    in_dir: &Path,
    meta: &SnapshotMeta,
    req: &ApplyRequest,
    root: &Path,
    record: &mut ApplyRecord,
) -> Result<Applied, ApplyStop> {
    let top = &info.top;
    let root = std::path::absolute(root).map_err(io_err(format!("resolve {}", root.display())))?;
    let dir = root.join(&meta.session);
    if std::fs::read_dir(&dir).is_ok_and(|mut d| d.next().is_some()) {
        return Err(ApplyStop::WorktreeExists { path: dir });
    }
    fetch_move(op, top, in_dir, meta, req, record)?;
    let lfs_installed = install_lfs(info, in_dir, meta)?;
    let empty = empty_tree(op, top)?;
    std::fs::create_dir_all(&root).map_err(io_err(format!("create {}", root.display())))?;
    let wt_branch = match &meta.branch {
        Some(_) => Some(free_branch(op, top, &meta.session)?),
        None => None,
    };
    let add = op.cmd(top).args(["worktree", "add", "-q"]);
    let add = match (&meta.base, &wt_branch) {
        (Some(base), Some(b)) => add
            .args(["--no-checkout", "-b", &short_branch(b)])
            .arg(&dir)
            .arg(base),
        (Some(base), None) => add.args(["--no-checkout", "--detach"]).arg(&dir).arg(base),
        (None, Some(b)) => add.args(["--orphan", "-b", &short_branch(b)]).arg(&dir),
        (None, None) => {
            return Err(CodeError::BadMove("a detached snapshot without a base".into()).into())
        }
    };
    add.raw()?;
    op.cmd(&dir)
        .args(["read-tree", "-m", "-u", &empty, &meta.snap])
        .raw()?;
    op.cmd(&dir).args(["read-tree", &meta.idx_tree]).raw()?;
    refresh(op, &dir)?;
    let submodules = land_submodules(op, &dir, meta)?;
    Ok(Applied {
        landing: LandingKind::Worktree,
        path: dir,
        action: None,
        parked: None,
        wt_branch,
        undo_ref: None,
        submodules,
        lfs_installed,
    })
}

/// Whether the landing still holds exactly what the apply left (known
/// limits aside), so undo loses nothing.
fn unchanged_since(
    op: &GitOp,
    info: &RepoInfo,
    meta: &SnapshotMeta,
    sym: Option<&String>,
) -> Result<(), CodeError> {
    if info.head != meta.base || info.sym.as_ref() != sym {
        return Err(CodeError::ChangedSinceApply("HEAD moved".into()));
    }
    let limit = |kind: &str| matches!(kind, "submodule" | "filemode" | "symlink-as-file" | "case");
    let idx_now = index_tree(op, info)?;
    if idx_now != meta.idx_tree
        && tree_diff(op, &info.top, &meta.idx_tree, &idx_now)?
            .iter()
            .any(|c| !limit(c.kind))
    {
        return Err(CodeError::ChangedSinceApply(
            "the staged files changed".into(),
        ));
    }
    let wt_now = worktree_tree(op, info, true)?;
    if wt_now != meta.wt_tree {
        if let Some(c) = tree_diff(op, &info.top, &meta.wt_tree, &wt_now)?
            .into_iter()
            .find(|c| !limit(c.kind))
        {
            return Err(CodeError::ChangedSinceApply(format!("{} changed", c.path)));
        }
    }
    Ok(())
}

/// Undoes a landing while it is still as the apply left it: a worktree is
/// removed with its branch; a checkout gets its files, index, branch and
/// HEAD back (a parked one its parked state), and the files that came
/// untracked with the move are removed only while still exactly as moved.
pub fn undo(git: &GitRunner, repo: &Path, in_dir: &Path) -> Result<Undone, CodeError> {
    let op = git.begin()?;
    let meta = load_meta(in_dir)?;
    let mut record = load_record(in_dir)?;
    let applied = match (&record.outcome, &record.applied) {
        (ApplyOutcome::Applied, Some(a)) => a.clone(),
        _ => return Err(CodeError::NotApplied),
    };
    if record.undone {
        return Err(CodeError::AlreadyUndone);
    }
    let info = repo_info(&op, repo)?;
    let landed = repo_info(&op, &applied.path)?;
    let kept = match applied.landing {
        LandingKind::Worktree => {
            unchanged_since(&op, &landed, &meta, applied.wt_branch.as_ref())?;
            op.cmd(&info.top)
                .args(["worktree", "remove", "--force"])
                .arg(&applied.path)
                .raw()?;
            if let Some(b) = &applied.wt_branch {
                op.cmd(&info.top).args(["update-ref", "-d", b]).raw()?;
            }
            Vec::new()
        }
        LandingKind::Checkout => {
            if !same_place(&landed.top, &info.top) {
                return Err(CodeError::NotApplied);
            }
            unchanged_since(&op, &info, &meta, meta.branch.as_ref())?;
            let u = record.undo.clone().ok_or(CodeError::NotApplied)?;
            undo_checkout(&op, &info, &meta, &u)?
        }
    };
    record.undone = true;
    write_json(&in_dir.join(APPLY), &record)?;
    Ok(Undone {
        landing: applied.landing,
        kept,
    })
}

fn undo_checkout(
    op: &GitOp,
    info: &RepoInfo,
    meta: &SnapshotMeta,
    u: &UndoInfo,
) -> Result<Vec<String>, CodeError> {
    let top = &info.top;
    let empty = empty_tree(op, top)?;
    let prev = match (&u.parked, &u.prev_head) {
        (Some(p), _) => p.wt_tree.clone(),
        (None, Some(h)) => h.clone(),
        (None, None) => empty,
    };
    let paths = |tree: &str| -> Result<HashSet<String>, CodeError> {
        Ok(ls_tree(op, top, tree)?
            .into_iter()
            .map(|e| e.path)
            .collect())
    };
    let prev_paths = paths(&prev)?;
    let idx_paths = paths(&meta.idx_tree)?;
    op.cmd(top)
        .args(["read-tree", "--reset", "-u", &prev])
        .raw()?;
    // Files that were untracked on the source: removed only while still
    // exactly as moved (line endings aside).
    let mut kept = Vec::new();
    let mut cache = DirCache::default();
    for e in &meta.manifest {
        if idx_paths.contains(&e.path) || prev_paths.contains(&e.path) {
            continue;
        }
        let now = raw_of(op, top, &e.path, &mut cache)?;
        let unchanged = same_raw(&now, &e.raw)
            || matches!((&now, &e.raw), (Raw::File { lf_sha256: a, .. }, Raw::File { lf_sha256: b, .. }) if a == b);
        match now {
            Raw::Missing => {}
            _ if unchanged => {
                let p = at(top, &e.path);
                if std::fs::remove_file(&p).is_ok() {
                    remove_empty_parents(top, &p);
                } else {
                    kept.push(e.path.clone());
                }
            }
            _ => kept.push(e.path.clone()),
        }
    }
    if let Some(p) = &u.parked {
        // The parked staged state; the files stay as they are.
        op.cmd(top).args(["read-tree", &p.idx_tree]).raw()?;
    }
    match (&meta.branch, &meta.base, u.action) {
        (Some(b), Some(base), BranchAction::FastForward | BranchAction::Reset) => {
            let old = u.branch_old.clone().unwrap_or_default();
            op.cmd(top).args(["update-ref", b, &old, base]).raw()?;
        }
        (Some(b), Some(base), BranchAction::Create) => {
            op.cmd(top).args(["update-ref", "-d", b, base]).raw()?;
        }
        _ => {}
    }
    match (&u.prev_sym, &u.prev_head) {
        (Some(sym), _) => {
            op.cmd(top).args(["symbolic-ref", "HEAD", sym]).raw()?;
        }
        (None, Some(head)) => {
            op.cmd(top)
                .args(["update-ref", "--no-deref", "HEAD", head])
                .raw()?;
        }
        (None, None) => {}
    }
    refresh(op, top)?;
    Ok(kept)
}
