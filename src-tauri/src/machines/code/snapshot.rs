//! The source's half (plan §5.10): what the target has, and the snapshot.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::check::{
    is_lfs_oid, is_oid, now_rfc3339, safe_name, valid_branch, valid_remote_name, valid_session,
    windows_bad_name, write_json,
};
use super::git::{GitOp, GitRunner};
use super::repo::{
    busy, checked_trees, config_of, identity_env, ls_tree, raw_of, repo_info, session_moves,
    status_z, tree_diff, unmerged, DirCache, TreeEntry,
};
use super::types::{
    io_err, BundleInfo, CodeError, Haves, LfsInfo, ManifestEntry, MoveRefs, RemoteInfo,
    SnapshotMeta, SnapshotRequest, Transfer, TransferKind, Warning, BUNDLE, FORMAT, MAX_HAVES,
    META,
};

/// Deletes the refs of `session`'s snapshot `epoch` in the repository at
/// `repo` (`refs/mother-claude/moves/<session>/<epoch>/{snap,index}`): one
/// made for a move that then sent another instead (a bundle over the limit
/// going through the remote). Its commits go with the next gc.
pub fn drop_snapshot(
    git: &GitRunner,
    repo: &Path,
    session: &str,
    epoch: &str,
) -> Result<(), CodeError> {
    if !valid_session(session) || epoch.is_empty() || !epoch.bytes().all(|b| b.is_ascii_digit()) {
        return Err(CodeError::Invalid {
            what: "snapshot",
            value: format!("{session}/{epoch}"),
        });
    }
    let op = git.begin()?;
    let info = repo_info(&op, repo)?;
    let refs = MoveRefs::for_move(session, epoch);
    op.cmd(&info.top)
        .args(["update-ref", "--stdin"])
        .input(format!("delete {}\ndelete {}\n", refs.snap, refs.index))
        .raw()?;
    Ok(())
}

/// The target's side of the preflight: the commits it has (HEAD, then
/// `session`'s earlier snapshots so chaining works (F6), branches, remote
/// branches, other moves, tags), at most [`MAX_HAVES`].
pub fn haves(git: &GitRunner, repo: &Path, session: Option<&str>) -> Result<Haves, CodeError> {
    let op = git.begin()?;
    let info = repo_info(&op, repo)?;
    let out = op.git(
        &info.top,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname) %(objecttype) %(*objectname) %(*objecttype)",
            "refs/heads",
            "refs/remotes",
            "refs/tags",
            "refs/mother-claude/moves",
        ],
    )?;
    let own = session.map(|s| format!("refs/mother-claude/moves/{s}/"));
    let rank = |name: &str| -> u8 {
        if own.as_deref().is_some_and(|p| name.starts_with(p)) {
            0
        } else if name.starts_with("refs/heads/") {
            1
        } else if name.starts_with("refs/remotes/") {
            2
        } else if name.starts_with("refs/mother-claude/") {
            3
        } else {
            4
        }
    };
    let mut found: Vec<(u8, String, String)> = Vec::new();
    for line in out.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        let (name, oid, kind) = (f[0], f.get(1).copied(), f.get(2).copied());
        let commit = match (kind, f.get(3), f.get(4)) {
            (Some("commit"), _, _) => oid,
            (_, Some(peeled), Some(&"commit")) => Some(*peeled),
            _ => None,
        };
        if let Some(c) = commit.filter(|c| is_oid(c)) {
            found.push((rank(name), name.to_string(), c.to_string()));
        }
    }
    // Newest snapshot first within a session (epochs sort as numbers).
    found.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| (b.1.len(), &b.1).cmp(&(a.1.len(), &a.1)))
    });
    let mut seen = HashSet::new();
    let mut list = Vec::new();
    for c in info
        .head
        .iter()
        .cloned()
        .chain(found.into_iter().map(|f| f.2))
    {
        if list.len() >= MAX_HAVES {
            break;
        }
        if seen.insert(c.clone()) {
            list.push(c);
        }
    }
    let shallow = op.git(&info.top, &["rev-parse", "--is-shallow-repository"])? == "true";
    Ok(Haves {
        haves: list,
        shallow,
        machine: safe_name(&crate::claude::backup::host_name()),
    })
}

/// The haves that are commits here (what a bundle may leave out).
pub(super) fn filter_haves(
    op: &GitOp,
    top: &Path,
    list: &[String],
) -> Result<Vec<String>, CodeError> {
    let list: Vec<&String> = list.iter().filter(|h| is_oid(h)).take(MAX_HAVES).collect();
    if list.is_empty() {
        return Ok(Vec::new());
    }
    let input: String = list.iter().map(|h| format!("{h}\n")).collect();
    let out = op
        .cmd(top)
        .args(["cat-file", "--batch-check=%(objectname) %(objecttype)"])
        .input(input)
        .text()?;
    Ok(out
        .lines()
        .filter_map(|l| l.split_once(' '))
        .filter(|(_, kind)| *kind == "commit")
        .map(|(oid, _)| oid.to_string())
        .collect())
}

fn first<T: Clone>(v: &[T], n: usize) -> Vec<T> {
    v.iter().take(n).cloned().collect()
}

/// What the move dialog warns about (plan §5.10, "Warnings in preflight").
fn warnings(
    op: &GitOp,
    top: &Path,
    entries: &[TreeEntry],
    idx_tree: &str,
    status: &str,
    base: Option<&str>,
) -> Result<Vec<Warning>, CodeError> {
    let mut w = Vec::new();
    let mut seen: HashMap<String, &str> = HashMap::new();
    let mut pairs = Vec::new();
    for e in entries {
        if let Some(other) = seen.insert(e.path.to_lowercase(), &e.path) {
            pairs.push((other.to_string(), e.path.clone()));
        }
    }
    if !pairs.is_empty() {
        w.push(Warning::CaseCollision {
            pairs: first(&pairs, 20),
        });
    }
    let bad: Vec<String> = entries
        .iter()
        .filter(|e| e.path.split('/').any(windows_bad_name))
        .map(|e| e.path.clone())
        .collect();
    if !bad.is_empty() {
        w.push(Warning::WindowsNames {
            paths: first(&bad, 20),
        });
    }
    let long: Vec<String> = entries
        .iter()
        .filter(|e| e.path.chars().count() > 200)
        .map(|e| e.path.clone())
        .collect();
    if !long.is_empty() {
        w.push(Warning::LongPath {
            paths: first(&long, 5),
        });
    }
    let gitlinks: Vec<String> = entries
        .iter()
        .filter(|e| e.mode == "160000")
        .map(|e| e.path.clone())
        .collect();
    if !gitlinks.is_empty() {
        let changed = super::repo::parse_status(status)
            .into_iter()
            .filter(|(p, xy)| gitlinks.contains(p) && xy != "  ")
            .map(|(p, _)| p)
            .collect();
        w.push(Warning::Submodules {
            paths: gitlinks,
            changed,
        });
    }
    for e in entries
        .iter()
        .filter(|e| e.path == ".gitattributes" || e.path.ends_with("/.gitattributes"))
    {
        let text = op.git(top, &["cat-file", "blob", &e.oid])?;
        if text.contains("filter=lfs") {
            w.push(Warning::Lfs);
            break;
        }
    }
    let idx: HashMap<String, TreeEntry> = ls_tree(op, top, idx_tree)?
        .into_iter()
        .map(|e| (e.path.clone(), e))
        .collect();
    let mode_only: Vec<String> = entries
        .iter()
        .filter(|e| {
            idx.get(&e.path).is_some_and(|i| {
                i.oid == e.oid && i.mode != e.mode && e.mode != "120000" && i.mode != "120000"
            })
        })
        .map(|e| e.path.clone())
        .collect();
    if !mode_only.is_empty() {
        w.push(Warning::UnstagedModeChange {
            paths: first(&mode_only, 20),
        });
    }
    let new_with = |mode: &str| -> Vec<String> {
        entries
            .iter()
            .filter(|e| !idx.contains_key(&e.path) && e.mode == mode)
            .map(|e| e.path.clone())
            .take(20)
            .collect()
    };
    let exec = new_with("100755");
    if !exec.is_empty() {
        w.push(Warning::UntrackedExec { paths: exec });
    }
    let links = new_with("120000");
    if !links.is_empty() {
        w.push(Warning::UntrackedSymlink { paths: links });
    }
    if op.git(top, &["rev-parse", "--is-shallow-repository"])? == "true" {
        w.push(Warning::ShallowSource);
    }
    if base.is_none() {
        w.push(Warning::Unborn);
    }
    Ok(w)
}

/// The Git LFS objects the snapshot's new blobs point at.
fn lfs_pointers(
    op: &GitOp,
    top: &Path,
    base: Option<&str>,
    revs: &[&str],
) -> Result<Vec<String>, CodeError> {
    let mut blobs = Vec::new();
    for rev in revs {
        match base {
            Some(b) => {
                for c in tree_diff(op, top, b, rev)? {
                    if c.status != "D" && c.dst_mode.starts_with("100") {
                        blobs.push(c.dst_oid);
                    }
                }
            }
            None => {
                for e in ls_tree(op, top, rev)? {
                    if e.kind == "blob" && e.mode != "120000" {
                        blobs.push(e.oid);
                    }
                }
            }
        }
    }
    blobs.sort();
    blobs.dedup();
    if blobs.is_empty() {
        return Ok(Vec::new());
    }
    let input: String = blobs.iter().map(|b| format!("{b}\n")).collect();
    let sizes = op
        .cmd(top)
        .args(["cat-file", "--batch-check=%(objectname) %(objectsize)"])
        .input(input)
        .text()?;
    let mut oids = Vec::new();
    for (oid, size) in sizes.lines().filter_map(|l| l.split_once(' ')) {
        if size.parse::<u64>().map_or(true, |s| s > 1024) {
            continue;
        }
        let text = op.git(top, &["cat-file", "blob", oid])?;
        let mut lines = text.lines();
        if lines.next() != Some("version https://git-lfs.github.com/spec/v1") {
            continue;
        }
        if let Some(o) = lines
            .next()
            .and_then(|l| l.strip_prefix("oid sha256:"))
            .filter(|o| is_lfs_oid(o))
        {
            oids.push(o.to_string());
        }
    }
    oids.sort();
    oids.dedup();
    Ok(oids)
}

/// Where Git LFS keeps an object under `root` (a common dir or a move folder).
pub(super) fn lfs_object(root: &Path, oid: &str) -> PathBuf {
    root.join("lfs")
        .join("objects")
        .join(&oid[..2])
        .join(&oid[2..4])
        .join(oid)
}

fn ms(t: Instant) -> u64 {
    t.elapsed().as_millis() as u64
}

/// §5.10 snapshot: the dirty state of the checkout at `repo` as two commits
/// (what is staged, and the working tree with untracked files and without
/// ignored ones), kept under `refs/mother-claude/moves/<session>/<epoch>`
/// and written into the move folder (a bundle) or pushed to hidden refs.
/// The checkout's index, HEAD, branches and files are never touched (F4).
pub fn snapshot(
    git: &GitRunner,
    repo: &Path,
    req: &SnapshotRequest,
) -> Result<SnapshotMeta, CodeError> {
    if !valid_session(&req.session) {
        return Err(CodeError::Invalid {
            what: "session id",
            value: req.session.clone(),
        });
    }
    if let Transfer::Remote { remote, .. } = &req.transfer {
        if !valid_remote_name(remote) {
            return Err(CodeError::Invalid {
                what: "remote name",
                value: remote.clone(),
            });
        }
    }
    if let Some(b) = req.branch.as_deref().filter(|b| !valid_branch(b)) {
        return Err(CodeError::Invalid {
            what: "branch",
            value: b.to_string(),
        });
    }
    let machine = safe_name(&req.machine);
    let epoch = req.epoch.to_string();
    let op = git.begin()?;
    let mut timings = BTreeMap::new();
    let mut t = Instant::now();
    let info = repo_info(&op, repo)?;
    let top = info.top.clone();
    if let Some(what) = busy(&info) {
        return Err(CodeError::InProgress(what));
    }
    if unmerged(&op, &top)? {
        return Err(CodeError::Unmerged);
    }

    let (idx_tree, wt_tree, wt_check) = checked_trees(&op, &info)?;
    timings.insert("trees".to_string(), ms(t));
    t = Instant::now();

    let base = info.head.clone();
    let branch = req.branch.clone().or_else(|| info.sym.clone());
    let bundle_move = req.transfer == Transfer::Bundle;
    let offered = req.haves.iter().filter(|h| is_oid(h)).count();
    let used = if bundle_move {
        filter_haves(&op, &top, &req.haves)?
    } else {
        Vec::new()
    };
    // F6: this session's earlier snapshots the target has become extra
    // parents (after base): otherwise `--not <earlier snapshot>` excludes
    // nothing (it is no ancestor) and every move re-sends the last one's.
    // A task's result names its own (the last result, made here).
    let chained: Vec<String> = if !req.extra_parents.is_empty() {
        filter_haves(&op, &top, &req.extra_parents)?
    } else if req.chain && bundle_move {
        session_moves(&op, &top, &req.session)?
            .into_iter()
            .map(|m| m.snap)
            .filter(|s| used.contains(s))
            .collect()
    } else {
        Vec::new()
    };
    // HEAD itself is the first parent already (never a parent twice).
    let chained: Vec<String> = chained
        .into_iter()
        .filter(|c| base.as_ref() != Some(c))
        .collect();
    let id_env = identity_env(&op, &top)?;
    let commit = |parents: &[&String], message: String, tree: &str| {
        let mut cmd = op.cmd(&top).args(["commit-tree", "--no-gpg-sign"]);
        for p in parents {
            cmd = cmd.arg("-p").arg(p);
        }
        for (k, v) in &id_env {
            cmd = cmd.env(k, v);
        }
        cmd.arg("-m").arg(message).arg(tree).text()
    };
    let base_parent: Vec<&String> = base.iter().collect();
    // Each snapshot's commits name its epoch: two snapshots of the same
    // trees in the same second would otherwise be one commit, which a
    // bundle leaves out when the target has the first (its ref with it).
    let what = if req.sync { "code sync" } else { "move" };
    let label = req
        .label
        .as_deref()
        .map(|l| l.chars().filter(|c| !c.is_control()).take(200).collect())
        .filter(|l: &String| !l.trim().is_empty());
    // The staged tree is not reachable from the snapshot: it travels as a
    // commit of its own with the same parent (F2), like git stash's.
    let index_commit = commit(
        &base_parent,
        if let Some(l) = &label {
            format!("mother-claude: staged state of {l} ({epoch})")
        } else if req.sync {
            format!(
                "mother-claude: staged state of code sync {} from {machine} ({epoch})",
                req.session
            )
        } else {
            format!(
                "mother-claude: staged state of {} from {machine} ({epoch})",
                req.session
            )
        },
        &idx_tree,
    )?;
    let snap_parents: Vec<&String> = base.iter().chain(chained.iter()).collect();
    let snap = commit(
        &snap_parents,
        match &label {
            Some(l) => format!("mother-claude: {l} ({epoch})"),
            None => format!(
                "mother-claude: {what} {} from {machine} ({epoch})",
                req.session
            ),
        },
        &wt_tree,
    )?;
    let refs = MoveRefs::for_move(&req.session, &epoch);
    for (name, oid) in [(&refs.snap, &snap), (&refs.index, &index_commit)] {
        op.cmd(&top)
            .args(["update-ref", "-m", "mother-claude snapshot"])
            .args([name, oid])
            .arg("")
            .raw()?;
    }
    timings.insert("commits".to_string(), ms(t));
    t = Instant::now();

    let entries = ls_tree(&op, &top, &wt_tree)?;
    let mut cache = DirCache::default();
    let mut manifest = Vec::with_capacity(entries.len());
    for e in &entries {
        manifest.push(ManifestEntry {
            path: e.path.clone(),
            mode: e.mode.clone(),
            kind: e.kind.clone(),
            oid: e.oid.clone(),
            raw: raw_of(&op, &top, &e.path, &mut cache)?,
        });
    }
    let status = status_z(&op, &top)?;
    let warnings = warnings(&op, &top, &entries, &idx_tree, &status, base.as_deref())?;
    timings.insert("manifest".to_string(), ms(t));
    t = Instant::now();

    let out_dir = std::path::absolute(&req.out_dir)
        .map_err(io_err(format!("resolve {}", req.out_dir.display())))?;
    std::fs::create_dir_all(&out_dir).map_err(io_err(format!("create {}", out_dir.display())))?;
    let (transfer, bundle, remote) = match &req.transfer {
        Transfer::Bundle => {
            let file = out_dir.join(BUNDLE);
            if file.exists() {
                std::fs::remove_file(&file)
                    .map_err(io_err(format!("replace {}", file.display())))?;
            }
            let mut cmd = op
                .cmd(&top)
                .args(["bundle", "create", "-q"])
                .arg(&file)
                .args([&refs.snap, &refs.index]);
            if !used.is_empty() {
                cmd = cmd.arg("--not").args(&used);
            }
            cmd.raw()?;
            let size = std::fs::metadata(&file)
                .map_err(io_err(format!("read {}", file.display())))?
                .len();
            timings.insert("bundle".to_string(), ms(t));
            let info = BundleInfo {
                file: BUNDLE.to_string(),
                size,
                haves_offered: offered,
                haves_used: used.len(),
            };
            (TransferKind::Bundle, Some(info), None)
        }
        Transfer::Remote {
            remote,
            push_branch,
        } => {
            let hidden = MoveRefs::hidden(&machine, &req.session);
            // Forced: a newer snapshot of the session replaces one never fetched.
            let mut specs = vec![
                format!("+{snap}:{}", hidden.snap),
                format!("+{index_commit}:{}", hidden.index),
            ];
            let pushed_branch = match (*push_branch, &branch, &base) {
                (true, Some(b), Some(base)) => {
                    specs.push(format!("{base}:{b}"));
                    true
                }
                _ => false,
            };
            op.cmd(&top)
                .args(["push", "--no-verify", "-q", "--recurse-submodules=no"])
                .arg(remote)
                .args(&specs)
                .raw()?;
            timings.insert("push".to_string(), ms(t));
            let info = RemoteInfo {
                remote: remote.clone(),
                hidden,
                pushed_branch,
            };
            (TransferKind::Remote, None, Some(info))
        }
    };
    t = Instant::now();

    let mut lfs = None;
    if warnings.contains(&Warning::Lfs) && op.cmd(&top).args(["lfs", "version"]).ok()? {
        let pointers = lfs_pointers(&op, &top, base.as_deref(), &[&snap, &index_commit])?;
        let (mut carried, mut missing) = (Vec::new(), Vec::new());
        if req.carry_lfs {
            for oid in &pointers {
                let src = lfs_object(&info.common_dir, oid);
                if !src.is_file() {
                    missing.push(oid.clone());
                    continue;
                }
                let dst = lfs_object(&out_dir, oid);
                if let Some(parent) = dst.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(io_err(format!("create {}", parent.display())))?;
                }
                std::fs::copy(&src, &dst)
                    .map_err(io_err(format!("copy the Git LFS object {oid}")))?;
                carried.push(oid.clone());
            }
        }
        lfs = Some(LfsInfo {
            pointers: pointers.len(),
            carried,
            missing,
            carried_disabled: !req.carry_lfs,
        });
        timings.insert("lfs".to_string(), ms(t));
    }

    let meta = SnapshotMeta {
        version: FORMAT,
        session: req.session.clone(),
        epoch,
        machine,
        platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        created: now_rfc3339(),
        branch,
        base,
        idx_tree,
        wt_tree,
        snap,
        index_commit,
        refs,
        transfer,
        bundle,
        remote,
        lfs,
        chained,
        config: config_of(&op, &top)?,
        status,
        manifest,
        warnings,
        wt_check,
        hook_intrusions: op.intrusions(),
        timings,
    };
    write_json(&out_dir.join(META), &meta)?;
    Ok(meta)
}
