//! What a checkout holds, read without touching it (F1, F4): both trees
//! come from copies of the index, and `git status` runs without optional
//! locks, so the real index is never rewritten.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use super::check::hex;
use super::git::{trim_text, GitOp};
use super::types::{io_err, CodeError, Raw, RepoConfig, WtCheck};

/// A checkout: where it is and what HEAD says.
#[derive(Clone, Debug)]
pub struct RepoInfo {
    pub top: PathBuf,
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
    pub index_path: PathBuf,
    /// The branch HEAD points at (`refs/heads/…`), none when detached.
    pub sym: Option<String>,
    /// HEAD's commit, none on an unborn branch.
    pub head: Option<String>,
}

pub fn repo_info(op: &GitOp, repo: &Path) -> Result<RepoInfo, CodeError> {
    let not_repo = || CodeError::NotARepository(repo.to_path_buf());
    if !repo.is_dir() {
        return Err(not_repo());
    }
    let out = op
        .cmd(repo)
        .args(["rev-parse", "--show-toplevel"])
        .output()?;
    let top = trim_text(&out.stdout);
    if !out.success || top.is_empty() {
        return Err(not_repo());
    }
    let top = PathBuf::from(top);
    let lines = op.git(
        &top,
        &[
            "rev-parse",
            "--absolute-git-dir",
            "--git-common-dir",
            "--git-path",
            "index",
        ],
    )?;
    let mut it = lines.lines().map(|l| top.join(l));
    let (Some(git_dir), Some(common_dir), Some(index_path)) = (it.next(), it.next(), it.next())
    else {
        return Err(not_repo());
    };
    let sym = op.cmd(&top).args(["symbolic-ref", "-q", "HEAD"]).output()?;
    let sym = Some(trim_text(&sym.stdout)).filter(|s| sym.success && !s.is_empty());
    let head = op.rev(&top, "HEAD^{commit}")?;
    Ok(RepoInfo {
        top,
        git_dir,
        common_dir,
        index_path,
        sym,
        head,
    })
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" | "" => Some(false),
        _ => None,
    }
}

pub fn config_of(op: &GitOp, top: &Path) -> Result<RepoConfig, CodeError> {
    let out = op
        .cmd(top)
        .args([
            "config",
            "-z",
            "--get-regexp",
            r"^core\.(autocrlf|eol|filemode|symlinks|ignorecase|precomposeunicode|longpaths)$",
        ])
        .output()?;
    let mut map: HashMap<String, Option<String>> = HashMap::new();
    for rec in out.stdout.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let rec = String::from_utf8_lossy(rec);
        let (k, v) = match rec.split_once('\n') {
            Some((k, v)) => (k, Some(v.to_string())),
            None => (rec.as_ref(), None),
        };
        map.insert(k.to_ascii_lowercase(), v);
    }
    let boolean = |k: &str, dflt: bool| match map.get(k) {
        None => dflt,
        Some(None) => true,
        Some(Some(v)) => parse_bool(v).unwrap_or(dflt),
    };
    let text = |k: &str| map.get(k).cloned().flatten();
    Ok(RepoConfig {
        autocrlf: text("core.autocrlf")
            .map(|v| v.to_ascii_lowercase())
            .unwrap_or_else(|| "false".to_string()),
        eol: text("core.eol"),
        file_mode: boolean("core.filemode", !cfg!(windows)),
        symlinks: boolean("core.symlinks", !cfg!(windows)),
        ignore_case: boolean("core.ignorecase", false),
        precompose_unicode: boolean("core.precomposeunicode", false),
        longpaths: boolean("core.longpaths", false),
    })
}

/// A copy of the index that git reads like the original (F1). A copy dated
/// now makes git trust stale stat data and miss a same-size edit made in
/// the index's second, so the copy keeps the original's mtime floored to
/// the second; `racy` dates it 1 s after the epoch, so git hashes every
/// file whose stat looks unchanged.
pub fn copy_index(op: &GitOp, info: &RepoInfo, racy: bool) -> Result<PathBuf, CodeError> {
    let dest = op.scratch_path(if racy { "index-racy" } else { "index" });
    let meta = match std::fs::metadata(&info.index_path) {
        Ok(m) => m,
        // No index yet: git reads a missing index file as an empty one.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(dest),
        Err(e) => return Err(io_err("read the index")(e)),
    };
    std::fs::copy(&info.index_path, &dest).map_err(io_err("copy the index"))?;
    let when = if racy {
        UNIX_EPOCH + Duration::from_secs(1)
    } else {
        let modified = meta.modified().map_err(io_err("read the index's time"))?;
        let secs = modified
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        UNIX_EPOCH + Duration::from_secs(secs)
    };
    std::fs::OpenOptions::new()
        .write(true)
        .open(&dest)
        .and_then(|f| f.set_modified(when))
        .map_err(io_err("date the index copy"))?;
    Ok(dest)
}

/// What is staged, as a tree.
pub fn index_tree(op: &GitOp, info: &RepoInfo) -> Result<String, CodeError> {
    let idx = copy_index(op, info, false)?;
    op.cmd(&info.top)
        .arg("write-tree")
        .env("GIT_INDEX_FILE", &idx)
        .text()
}

/// The working tree as a tree (untracked included, ignored not): `add -A`
/// into a copy of the index.
pub fn worktree_tree(op: &GitOp, info: &RepoInfo, racy: bool) -> Result<String, CodeError> {
    let idx = copy_index(op, info, racy)?;
    op.cmd(&info.top)
        .args(["add", "-A"])
        .env("GIT_INDEX_FILE", &idx)
        .fail_on_unreadable()
        .raw()?;
    op.cmd(&info.top)
        .arg("write-tree")
        .env("GIT_INDEX_FILE", &idx)
        .text()
}

pub fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn split_z(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect()
}

/// `git status --porcelain=v1 -z -uall`, as on the source.
pub fn status_z(op: &GitOp, top: &Path) -> Result<String, CodeError> {
    Ok(lossy(
        &op.cmd(top)
            .args(["status", "--porcelain=v1", "-z", "-uall"])
            .fail_on_unreadable()
            .raw()?,
    ))
}

/// F1's cross-check: the paths `git status` (the real index, git's own racy
/// logic) reports changed in the working tree are exactly the paths that
/// differ between the staged tree and the working-tree tree.
pub fn worktree_check(
    op: &GitOp,
    top: &Path,
    idx_tree: &str,
    wt_tree: &str,
) -> Result<WtCheck, CodeError> {
    let st = op
        .cmd(top)
        .args([
            "status",
            "--porcelain=v1",
            "-z",
            "-uall",
            "--ignore-submodules=dirty",
        ])
        .fail_on_unreadable()
        .raw()?;
    let want: BTreeSet<String> = parse_status(&lossy(&st))
        .into_iter()
        .filter(|(_, xy)| xy.as_bytes().get(1) != Some(&b' '))
        .map(|(p, _)| p)
        .collect();
    let diff = op
        .cmd(top)
        .args(["diff-tree", "-r", "-z", "--name-only", "--no-renames"])
        .args([idx_tree, wt_tree])
        .raw()?;
    let got: BTreeSet<String> = split_z(&diff).into_iter().collect();
    let missing: Vec<String> = want.difference(&got).cloned().collect();
    let extra: Vec<String> = got.difference(&want).cloned().collect();
    Ok(WtCheck {
        ok: missing.is_empty() && extra.is_empty(),
        missing,
        extra,
        ..WtCheck::default()
    })
}

/// The empty tree's id in this repository (SHA-1 or SHA-256).
pub fn empty_tree(op: &GitOp, top: &Path) -> Result<String, CodeError> {
    op.cmd(top)
        .args(["hash-object", "-w", "-t", "tree", "--stdin"])
        .input(Vec::new())
        .text()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeEntry {
    pub path: String,
    pub mode: String,
    pub kind: String,
    pub oid: String,
}

pub fn ls_tree(op: &GitOp, top: &Path, tree: &str) -> Result<Vec<TreeEntry>, CodeError> {
    let out = op
        .cmd(top)
        .args(["ls-tree", "-r", "-z", "--full-tree", tree])
        .raw()?;
    let mut entries = Vec::new();
    for rec in split_z(&out) {
        let Some((head, path)) = rec.split_once('\t') else {
            continue;
        };
        let mut parts = head.split(' ');
        let (Some(mode), Some(kind), Some(oid)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        entries.push(TreeEntry {
            path: path.to_string(),
            mode: mode.to_string(),
            kind: kind.to_string(),
            oid: oid.to_string(),
        });
    }
    Ok(entries)
}

/// One difference between two trees, classified.
#[derive(Clone, Debug)]
pub struct Change {
    pub path: String,
    pub src_mode: String,
    pub dst_mode: String,
    pub src_oid: String,
    pub dst_oid: String,
    pub status: String,
    /// `content`, `submodule`, `symlink-as-file`, `filemode` or `case`.
    pub kind: &'static str,
}

pub fn tree_diff(op: &GitOp, top: &Path, a: &str, b: &str) -> Result<Vec<Change>, CodeError> {
    let out = op
        .cmd(top)
        .args(["diff-tree", "-r", "-z", "--no-renames", a, b])
        .raw()?;
    let parts: Vec<String> = out
        .split(|b| *b == 0)
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect();
    let mut changes = Vec::new();
    let mut i = 0;
    while i + 1 < parts.len() {
        let head = &parts[i];
        i += 1;
        let Some(meta) = head.strip_prefix(':') else {
            continue;
        };
        let f: Vec<&str> = meta.split(' ').collect();
        if f.len() < 5 {
            continue;
        }
        changes.push(Change {
            path: parts[i].clone(),
            src_mode: f[0].to_string(),
            dst_mode: f[1].to_string(),
            src_oid: f[2].to_string(),
            dst_oid: f[3].to_string(),
            status: f[4].to_string(),
            kind: "content",
        });
        i += 1;
    }
    let kinds: Vec<&'static str> = changes.iter().map(|c| classify(c, &changes)).collect();
    for (c, k) in changes.iter_mut().zip(kinds) {
        c.kind = k;
    }
    Ok(changes)
}

fn classify(c: &Change, all: &[Change]) -> &'static str {
    if c.src_mode == "160000" || c.dst_mode == "160000" {
        return "submodule";
    }
    if c.src_oid == c.dst_oid && (c.status == "M" || c.status == "T") {
        if c.src_mode == "120000" || c.dst_mode == "120000" {
            return "symlink-as-file";
        }
        if c.src_mode != c.dst_mode {
            return "filemode";
        }
    }
    if (c.status == "A" || c.status == "D")
        && all
            .iter()
            .any(|o| o.path != c.path && o.path.to_lowercase() == c.path.to_lowercase())
    {
        return "case";
    }
    "content"
}

/// `status --porcelain=v1 -z` as path → XY (a rename's source skipped).
pub fn parse_status(z: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut parts = z.split('\0');
    while let Some(p) = parts.next() {
        let (Some(xy), Some(path)) = (p.get(..2), p.get(3..)) else {
            continue;
        };
        if path.is_empty() {
            continue;
        }
        out.insert(path.trim_end_matches('/').to_string(), xy.to_string());
        if xy.starts_with('R') || xy.starts_with('C') {
            parts.next();
        }
    }
    out
}

/// A path whose status differs from the source's.
#[derive(Clone, Debug)]
pub struct StatusDiff {
    pub path: String,
    pub want: Option<String>,
    pub got: Option<String>,
    /// An intent-to-add entry that arrived as an untracked file.
    pub ita: bool,
}

pub fn status_diff(want: &str, got: &str) -> Vec<StatusDiff> {
    let a = parse_status(want);
    let b = parse_status(got);
    let paths: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    paths
        .into_iter()
        .filter_map(|p| {
            let x = a.get(p);
            let y = b.get(p);
            (x != y).then(|| StatusDiff {
                path: p.clone(),
                want: x.cloned(),
                got: y.cloned(),
                ita: x.map(String::as_str) == Some(" A") && y.map(String::as_str) == Some("??"),
            })
        })
        .collect()
}

/// `top` joined with a tree path.
pub fn at(top: &Path, rel: &str) -> PathBuf {
    let mut p = top.to_path_buf();
    for c in rel.split('/') {
        p.push(c);
    }
    p
}

/// Directory listings, for names that differ from git's only in case.
#[derive(Default)]
pub struct DirCache(HashMap<PathBuf, Option<Vec<String>>>);

impl DirCache {
    fn case_on_disk(&mut self, file: &Path) -> Option<String> {
        let dir = file.parent()?.to_path_buf();
        let base = file.file_name()?.to_string_lossy().into_owned();
        let names = self.0.entry(dir.clone()).or_insert_with(|| {
            std::fs::read_dir(&dir).ok().map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
        });
        let names = names.as_ref()?;
        if names.contains(&base) {
            return None;
        }
        let lower = base.to_lowercase();
        names.iter().find(|n| n.to_lowercase() == lower).cloned()
    }
}

/// SHA-256, SHA-256 with CRLF read as LF, size and whether a CRLF occurs,
/// in one pass.
fn hash_file(path: &Path) -> std::io::Result<(u64, String, String, bool)> {
    let mut f = std::fs::File::open(path)?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut full = Sha256::new();
    let mut lf = Sha256::new();
    let (mut size, mut crlf, mut pending_cr) = (0u64, false, false);
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let chunk = &buf[..n];
        size += n as u64;
        full.update(chunk);
        if pending_cr {
            pending_cr = false;
            if chunk[0] == b'\n' {
                crlf = true;
            } else {
                lf.update(b"\r");
            }
        }
        let mut start = 0;
        let mut i = 0;
        while let Some(pos) = chunk[i..].iter().position(|&b| b == b'\r') {
            let cr = i + pos;
            if cr + 1 == n {
                lf.update(&chunk[start..cr]);
                start = n;
                pending_cr = true;
            } else if chunk[cr + 1] == b'\n' {
                lf.update(&chunk[start..cr]);
                start = cr + 1;
                crlf = true;
            }
            i = cr + 1;
        }
        if start < n {
            lf.update(&chunk[start..]);
        }
    }
    if pending_cr {
        lf.update(b"\r");
    }
    Ok((size, hex(&full.finalize()), hex(&lf.finalize()), crlf))
}

#[cfg(unix)]
fn exec_bit(meta: &std::fs::Metadata) -> Option<bool> {
    use std::os::unix::fs::PermissionsExt;
    Some(meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn exec_bit(_meta: &std::fs::Metadata) -> Option<bool> {
    None
}

/// A tree path's bytes on disk.
pub fn raw_of(op: &GitOp, top: &Path, rel: &str, cache: &mut DirCache) -> Result<Raw, CodeError> {
    let p = at(top, rel);
    let Ok(meta) = std::fs::symlink_metadata(&p) else {
        return Ok(Raw::Missing);
    };
    let case_name = cache.case_on_disk(&p);
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(&p)
            .map_err(io_err(format!("read the link {}", p.display())))?
            .to_string_lossy()
            .into_owned();
        let target = if cfg!(windows) {
            target.replace('\\', "/")
        } else {
            target
        };
        return Ok(Raw::Symlink { target, case_name });
    }
    if meta.is_dir() {
        let head = if p.join(".git").exists() {
            op.rev(&p, "HEAD^{commit}")?
        } else {
            None
        };
        return Ok(Raw::Dir { head, case_name });
    }
    let (size, sha256, lf_sha256, crlf) =
        hash_file(&p).map_err(io_err(format!("read {}", p.display())))?;
    Ok(Raw::File {
        size,
        sha256,
        lf_sha256,
        crlf,
        exec: exec_bit(&meta),
        case_name,
    })
}

pub fn same_raw(a: &Raw, b: &Raw) -> bool {
    match (a, b) {
        (Raw::File { sha256: x, .. }, Raw::File { sha256: y, .. }) => x == y,
        (Raw::Symlink { target: x, .. }, Raw::Symlink { target: y, .. }) => x == y,
        _ => a.kind() == b.kind(),
    }
}

/// A merge, rebase, cherry-pick, revert or bisect in progress.
pub fn busy(info: &RepoInfo) -> Option<String> {
    [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-merge",
        "rebase-apply",
        "BISECT_LOG",
    ]
    .into_iter()
    .find(|f| info.git_dir.join(f).exists())
    .map(str::to_string)
}

/// The machine's own identity; only where it has none, a stand-in, so
/// `commit-tree` works.
pub fn identity_env(op: &GitOp, top: &Path) -> Result<Vec<(&'static str, String)>, CodeError> {
    let mut env = Vec::new();
    if op.is_isolated() {
        return Ok(env);
    }
    for (key, vars, stand_in) in [
        (
            "user.name",
            ["GIT_AUTHOR_NAME", "GIT_COMMITTER_NAME"],
            "Mother Claude",
        ),
        (
            "user.email",
            ["GIT_AUTHOR_EMAIL", "GIT_COMMITTER_EMAIL"],
            "mother-claude@localhost",
        ),
    ] {
        let out = op.cmd(top).args(["config", "--get", key]).output()?;
        if out.success && !trim_text(&out.stdout).trim().is_empty() {
            continue;
        }
        for v in vars {
            if std::env::var_os(v).is_none() {
                env.push((v, stand_in.to_string()));
            }
        }
    }
    Ok(env)
}

/// A snapshot of a session this repository has (made here or received).
#[derive(Clone, Debug)]
pub struct Move {
    pub epoch: String,
    pub snap: String,
    pub index: String,
}

/// Every snapshot of `session` here, newest first.
pub fn session_moves(op: &GitOp, top: &Path, session: &str) -> Result<Vec<Move>, CodeError> {
    let prefix = format!("refs/mother-claude/moves/{session}/");
    let out = op
        .cmd(top)
        .args(["for-each-ref", "--format=%(refname) %(objectname)"])
        .arg(&prefix)
        .text()?;
    let mut by_epoch: BTreeMap<String, (Option<String>, Option<String>)> = BTreeMap::new();
    for line in out.lines() {
        let Some((name, oid)) = line.split_once(' ') else {
            continue;
        };
        let Some((epoch, which)) = name.strip_prefix(&prefix).and_then(|r| r.split_once('/'))
        else {
            continue;
        };
        let e = by_epoch.entry(epoch.to_string()).or_default();
        match which {
            "snap" => e.0 = Some(oid.to_string()),
            "index" => e.1 = Some(oid.to_string()),
            _ => {}
        }
    }
    let mut moves: Vec<Move> = by_epoch
        .into_iter()
        .filter_map(|(epoch, (snap, index))| {
            Some(Move {
                epoch,
                snap: snap?,
                index: index?,
            })
        })
        .collect();
    moves.sort_by(|a, b| (b.epoch.len(), b.epoch.as_str()).cmp(&(a.epoch.len(), a.epoch.as_str())));
    Ok(moves)
}

/// A checkout holding exactly a state of this session it sent or received.
#[derive(Clone, Debug)]
pub struct Parked {
    pub epoch: String,
    pub wt_tree: String,
    pub idx_tree: String,
}

/// F7: a checkout whose HEAD is a snapshot's base, index its staged tree and
/// working tree its tree — for a snapshot of *this* session — is parked:
/// the session left from here and nobody touched the files since, so it
/// counts as clean for that session.
pub fn parked_state(
    op: &GitOp,
    info: &RepoInfo,
    session: &str,
) -> Result<Option<Parked>, CodeError> {
    let moves = session_moves(op, &info.top, session)?;
    if moves.is_empty() {
        return Ok(None);
    }
    let idx_now = index_tree(op, info)?;
    let wt_now = worktree_tree(op, info, true)?;
    for m in moves {
        let trees = op.git(
            &info.top,
            &[
                "rev-parse",
                &format!("{}^{{tree}}", m.snap),
                &format!("{}^{{tree}}", m.index),
            ],
        )?;
        let mut it = trees.lines();
        let (Some(wt), Some(idx)) = (it.next(), it.next()) else {
            continue;
        };
        // The index commit's only parent is the snapshot's base.
        let base = op.rev(&info.top, &format!("{}^1", m.index))?;
        if wt == wt_now && idx == idx_now && base == info.head {
            return Ok(Some(Parked {
                epoch: m.epoch,
                wt_tree: wt.to_string(),
                idx_tree: idx.to_string(),
            }));
        }
    }
    Ok(None)
}

/// Removes `file`'s parent folders up to `top` while they are empty.
pub fn remove_empty_parents(top: &Path, file: &Path) {
    let mut d = file.parent();
    while let Some(dir) = d {
        if dir == top || !dir.starts_with(top) {
            break;
        }
        let empty = std::fs::read_dir(dir).is_ok_and(|mut r| r.next().is_none());
        if !empty || std::fs::remove_dir(dir).is_err() {
            break;
        }
        d = dir.parent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_parses_renames_and_untracked_folders() {
        let z = "R  new.txt\0old.txt\0 M a.txt\0?? nested/\0?? x y.txt\0";
        let st = parse_status(z);
        assert_eq!(st.get("new.txt").map(String::as_str), Some("R "));
        assert!(!st.contains_key("old.txt"));
        assert_eq!(st.get("nested").map(String::as_str), Some("??"));
        assert_eq!(st.get("x y.txt").map(String::as_str), Some("??"));
        let d = status_diff(" A h.txt\0", "?? h.txt\0");
        assert!(d.len() == 1 && d[0].ita);
    }

    #[test]
    fn line_endings_hash_as_lf_across_chunks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("f");
        let mut crlf = Vec::new();
        let mut lf = Vec::new();
        for i in 0..40_000u32 {
            let line = format!("line {i}\r");
            crlf.extend_from_slice(line.as_bytes());
            crlf.push(b'\n');
            lf.extend_from_slice(line.trim_end_matches('\r').as_bytes());
            lf.push(b'\n');
        }
        crlf.extend_from_slice(b"lone\rcr\r");
        lf.extend_from_slice(b"lone\rcr\r");
        std::fs::write(&p, &crlf).expect("write");
        let (size, _, lf_sha, has_crlf) = hash_file(&p).expect("hash");
        assert_eq!(size, crlf.len() as u64);
        assert!(has_crlf);
        assert_eq!(lf_sha, super::super::check::sha256_hex(&lf));
    }
}
