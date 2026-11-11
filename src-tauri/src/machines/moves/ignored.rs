//! The files git ignores that a project lists to carry with a move
//! (`recipe.carryIgnored`, docs/PLAN-MACHINES.md §3.6, §5.8, §5.10): a
//! `.env.local`, a local certificate. They usually hold secrets, so setting
//! the list needs the dangerous right (`setup::options`) and so does a move
//! that carries them, on both computers (`source::check`, `target::resolve`).
//!
//! * **The source** ([`scan`], [`pack`]) reads each listed path (relative
//!   to the project, `/`-separated) under the repository's top, as a plain
//!   file only — never through a symbolic link (or a junction), at most
//!   [`MAX_BYTES`] each and [`MAX_FILES`] in all — and only the ones git
//!   ignores there: a tracked or an untracked file travels with the code
//!   already. They travel as one more file of the move, [`FILE`] (a gzipped
//!   tar of their paths under the top), hashed like the rest.
//! * **The target** ([`land`]) writes them into the folder the code landed
//!   in (the checkout or the worktree) once the code is there, through no
//!   link, each atomically; a file already there is first copied into the
//!   move's folder, and the record (`ignored.json`) says which it added and
//!   which it replaced. [`undo`] puts the replaced ones back and removes the
//!   added ones — each only while it is still what the move wrote — and
//!   [`redo`] lands them again when the rest of an undo could not run.
//!
//! Every path and entry the other computer sent is checked again before the
//! file system sees it.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::Check;
use crate::machines::code::{CodeError, GitRunner};

/// The package's file of them.
pub const FILE: &str = "ignored.tar.gz";
/// At most this many files travel.
pub const MAX_FILES: usize = 50;
/// The largest file that travels this way.
pub const MAX_BYTES: u64 = 1 << 20;
/// The target's record of what it wrote (in the move's folder).
const RECORD: &str = "ignored.json";
/// Where the target keeps the files they replaced (in the move's folder).
pub(crate) const KEPT: &str = "ignored-undo";

/// The fix of a listed path that can't travel.
const FIX: &str =
    "Leave them here, or change the files the project carries (Machines › Options › Each project).";

/// A file that travels, as the source found it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    /// As the project lists it (under the project's folder).
    pub listed: String,
    /// Under the repository's top, `/`-separated.
    pub path: String,
    pub size: u64,
}

/// What the source found of a project's list.
#[derive(Clone, Debug, Default)]
pub struct Scan {
    /// The files that travel (those git ignores), in the list's order.
    pub found: Vec<Found>,
    /// Listed paths that can't travel, said as the move's blockers.
    pub problems: Vec<Check>,
}

/// `rel` (a folder under the top, `""` for the top) joined with `p`.
fn under(rel: &str, p: &str) -> String {
    if rel.is_empty() {
        p.to_string()
    } else {
        format!("{rel}/{p}")
    }
}

/// Where a `/`-separated path under `top` is.
fn at(top: &Path, path: &str) -> PathBuf {
    let mut out = top.to_path_buf();
    for c in path.split('/') {
        out.push(c);
    }
    out
}

/// What is at a path under the top, every folder on the way checked.
#[derive(Debug, PartialEq, Eq)]
enum Spot {
    /// A plain file of this size, reached through plain folders only.
    File(u64),
    /// Nothing there (or a parent that is not a folder).
    Missing,
    /// A symbolic link (or a junction) on the way, or at the end.
    Link,
    /// A folder or something else that is not a plain file.
    NotFile,
    /// It could not be read.
    Unreadable(String),
}

/// What is at `path` under `top`: never followed through a link.
fn spot(top: &Path, path: &str) -> Spot {
    let parts: Vec<&str> = path.split('/').collect();
    let mut cur = top.to_path_buf();
    for (i, part) in parts.iter().enumerate() {
        cur.push(part);
        let meta = match std::fs::symlink_metadata(&cur) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Spot::Missing,
            Err(e) => return Spot::Unreadable(e.to_string()),
        };
        if meta.file_type().is_symlink() {
            return Spot::Link;
        }
        let last = i + 1 == parts.len();
        if !last {
            if !meta.is_dir() {
                return Spot::Missing;
            }
            continue;
        }
        if !meta.is_file() {
            return Spot::NotFile;
        }
        // Belt and braces: where it really is must be inside the top.
        let inside = match (dunce::canonicalize(&cur), dunce::canonicalize(top)) {
            (Ok(real), Ok(top)) => real.starts_with(&top),
            _ => false,
        };
        return if inside {
            Spot::File(meta.len())
        } else {
            Spot::Link
        };
    }
    Spot::Missing
}

/// The paths of `paths` (under `top`) git ignores there: tracked and
/// untracked files are not (they travel with the code).
fn ignored_by_git(git: &GitRunner, top: &Path, paths: &[String]) -> Result<Vec<String>, CodeError> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let op = git.begin()?;
    let input: Vec<u8> = paths
        .iter()
        .flat_map(|p| p.bytes().chain(std::iter::once(0)))
        .collect();
    let out = op
        .cmd(top)
        .args(["check-ignore", "-z", "--stdin"])
        .input(input)
        .output()?;
    // 0: some are ignored, 1: none is; anything else is git failing.
    if !out.success && out.code != 1 {
        return Err(CodeError::Git {
            command: "check-ignore -z --stdin".into(),
            code: out.code,
            stderr: out.stderr.trim().chars().take(500).collect(),
        });
    }
    Ok(out
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect())
}

/// Source: the listed files under the repository's `top` for a session
/// working in its folder `rel` (blocking). A listed path that is not on
/// this computer is left out; one that can't travel is a problem.
pub fn scan(git: &GitRunner, top: &Path, rel: &str, listed: &[String]) -> Scan {
    let mut out = Scan::default();
    if listed.len() > MAX_FILES {
        out.problems.push(
            Check::new(
                "ignored_too_many",
                format!(
                    "The project lists more than {MAX_FILES} files git ignores: at most \
                     {MAX_FILES} travel."
                ),
            )
            .fix(FIX),
        );
        return out;
    }
    let mut candidates: Vec<Found> = Vec::new();
    for raw in listed {
        let clean = crate::machines::setup::options::clean_carried(raw)
            .ok()
            .filter(|c| crate::machines::code::safe_rel_path(&under(rel, c)));
        let Some(clean) = clean else {
            out.problems.push(
                Check::new(
                    "ignored_bad_path",
                    format!(
                        "{} is not a file inside the project, so it can't travel.",
                        raw.trim()
                    ),
                )
                .fix(FIX),
            );
            continue;
        };
        let path = under(rel, &clean);
        match spot(top, &path) {
            Spot::Missing => {}
            Spot::File(size) if size > MAX_BYTES => out.problems.push(
                Check::new(
                    "ignored_too_large",
                    format!(
                        "{clean} is larger than 1 MB: files git ignores travel only up to 1 MB \
                         each."
                    ),
                )
                .fix(FIX),
            ),
            Spot::File(size) => {
                if !candidates.iter().any(|f| f.path == path) {
                    candidates.push(Found {
                        listed: clean,
                        path,
                        size,
                    });
                }
            }
            Spot::Link => out.problems.push(
                Check::new(
                    "ignored_link",
                    format!("{clean} is a symbolic link, or inside one: only plain files travel."),
                )
                .fix(FIX),
            ),
            Spot::NotFile => out.problems.push(
                Check::new(
                    "ignored_not_file",
                    format!("{clean} is not a plain file: only plain files travel."),
                )
                .fix(FIX),
            ),
            Spot::Unreadable(e) => out.problems.push(
                Check::new(
                    "ignored_unreadable",
                    format!("{clean} could not be read here: {e}"),
                )
                .fix(FIX),
            ),
        }
    }
    let paths: Vec<String> = candidates.iter().map(|f| f.path.clone()).collect();
    match ignored_by_git(git, top, &paths) {
        Ok(ignored) => {
            out.found = candidates
                .into_iter()
                .filter(|f| ignored.contains(&f.path))
                .collect();
        }
        Err(e) => out
            .problems
            .push(Check::new("code_unreadable", e.to_string())),
    }
    out
}

/// What the source packed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packed {
    pub size: u64,
    pub sha256: String,
    /// The files in it (one gone meanwhile is left out).
    pub files: Vec<Found>,
}

/// Source: packs `files` from under `top` into `out` (blocking), each
/// checked again as it is read. `Ok` with no files when none is left.
pub fn pack(top: &Path, files: &[Found], out: &Path) -> Result<Packed, String> {
    let mut entries: Vec<(Found, Vec<u8>)> = Vec::new();
    for f in files {
        let mut found = f.clone();
        match spot(top, &f.path) {
            Spot::Missing => continue,
            Spot::File(size) if size <= MAX_BYTES => found.size = size,
            Spot::File(_) => {
                return Err(format!("{} grew larger than 1 MB.", f.listed));
            }
            Spot::Link | Spot::NotFile => {
                return Err(format!("{} is no longer a plain file.", f.listed));
            }
            Spot::Unreadable(e) => return Err(format!("{}: {e}", f.listed)),
        }
        let mut bytes = Vec::new();
        std::fs::File::open(at(top, &f.path))
            .and_then(|file| file.take(MAX_BYTES + 1).read_to_end(&mut bytes))
            .map_err(|e| format!("{}: {e}", f.listed))?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(format!("{} grew larger than 1 MB.", f.listed));
        }
        found.size = bytes.len() as u64;
        entries.push((found, bytes));
    }
    if entries.is_empty() {
        return Ok(Packed {
            size: 0,
            sha256: String::new(),
            files: Vec::new(),
        });
    }
    let dir = out.parent().ok_or("The move's folder is missing.")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!(".ignored.mc-{}.tmp", uuid::Uuid::new_v4().simple()));
    let written = (|| -> std::io::Result<()> {
        let file = std::fs::File::create(&tmp)?;
        let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut tar = tar::Builder::new(gz);
        for (f, bytes) in &entries {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Regular);
            header.set_size(bytes.len() as u64);
            header.set_mode(0o600);
            header.set_mtime(0);
            tar.append_data(&mut header, &f.path, bytes.as_slice())?;
        }
        let gz = tar.into_inner()?;
        let file = gz.finish()?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("They could not be packed: {e}"));
    }
    if std::fs::symlink_metadata(out).is_ok() {
        std::fs::remove_file(out).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, out).map_err(|e| e.to_string())?;
    let (size, sha256) = crate::claude::transfer::sha256_file(out).map_err(|e| e.to_string())?;
    Ok(Packed {
        size,
        sha256,
        files: entries.into_iter().map(|(f, _)| f).collect(),
    })
}

/// Whether a list of paths, as another computer sent it, is one this
/// computer takes: at most [`MAX_FILES`], each safe under a top, once.
pub fn valid_list(paths: &[String]) -> bool {
    let mut seen = std::collections::HashSet::new();
    paths.len() <= MAX_FILES
        && paths
            .iter()
            .all(|p| crate::machines::code::safe_rel_path(p) && seen.insert(p.as_str()))
}

// ─── the target ────────────────────────────────────────────────────────────────

/// One file the target wrote.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Wrote {
    path: String,
    /// What the move wrote (undo leaves it when it changed since).
    sha256: String,
    /// A file of this computer's was there: kept in [`KEPT`].
    replaced: bool,
    /// Folders the move made for it (removed again when empty).
    #[serde(default)]
    made: Vec<String>,
}

/// The target's record (`ignored.json` in the move's folder).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    /// The folder they were written under (the landing's top).
    top: PathBuf,
    files: Vec<Wrote>,
    #[serde(default)]
    undone: bool,
}

/// What a landing of them did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Landed {
    pub written: usize,
    /// The files of this computer's they replaced (kept for undo).
    pub replaced: Vec<String>,
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Reads the archive the source sent: exactly the `expected` paths, each a
/// plain file of at most [`MAX_BYTES`].
fn read_archive(archive: &Path, expected: &[String]) -> Result<Vec<(String, Vec<u8>)>, String> {
    if !valid_list(expected) {
        return Err("The move names files this computer can't take.".into());
    }
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    for entry in tar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        if entry.header().entry_type() != tar::EntryType::Regular {
            return Err("The move carried something that is not a plain file.".into());
        }
        let path = String::from_utf8(entry.path_bytes().into_owned())
            .map_err(|_| "The move carried a file with an odd name.".to_string())?;
        if !expected.contains(&path) || out.iter().any(|(p, _)| *p == path) {
            return Err(format!("The move carried {path:?}, which it did not name."));
        }
        if entry.header().size().map_err(|e| e.to_string())? > MAX_BYTES {
            return Err(format!("{path} is larger than 1 MB."));
        }
        let mut bytes = Vec::new();
        entry
            .by_ref()
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(format!("{path} is larger than 1 MB."));
        }
        out.push((path, bytes));
        if out.len() > MAX_FILES {
            return Err("The move carried too many files.".into());
        }
    }
    if out.len() != expected.len() {
        return Err("Not every file it named came with the move.".into());
    }
    Ok(out)
}

/// What the landing will do at `path` under `top`: the folders to make,
/// and whether a file of this computer's is there. Never through a link.
fn plan(top: &Path, path: &str) -> Result<(Vec<String>, bool), String> {
    let parts: Vec<&str> = path.split('/').collect();
    let mut cur = top.to_path_buf();
    let mut made = Vec::new();
    let mut missing = false;
    for (i, part) in parts.iter().enumerate() {
        cur.push(part);
        let last = i + 1 == parts.len();
        if missing {
            if !last {
                made.push(parts[..=i].join("/"));
            }
            continue;
        }
        match std::fs::symlink_metadata(&cur) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                missing = true;
                if !last {
                    made.push(parts[..=i].join("/"));
                }
            }
            Err(e) => return Err(format!("{path}: {e}")),
            Ok(m) if m.file_type().is_symlink() => {
                return Err(format!(
                    "{path} would be written through a symbolic link here."
                ))
            }
            Ok(m) if !last && !m.is_dir() => {
                return Err(format!("{path}: a file is in the way here."))
            }
            Ok(m) if last && !m.is_file() => {
                return Err(format!(
                    "{path}: something that is not a plain file is there."
                ))
            }
            Ok(_) => {}
        }
    }
    Ok((made, !missing))
}

/// Writes `bytes` at `dest` through a temp file beside it (a new file is
/// private: these usually hold secrets).
fn write_atomic(dest: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = dest
        .parent()
        .ok_or_else(|| std::io::Error::other("no folder"))?;
    let tmp = dir.join(format!(".mc-{}.tmp", uuid::Uuid::new_v4().simple()));
    let written = (|| {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, dest)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// Copies the kept file back over `dest` (its own permissions with it)
/// through a temp file beside it.
fn restore_atomic(kept: &Path, dest: &Path) -> std::io::Result<()> {
    let dir = dest
        .parent()
        .ok_or_else(|| std::io::Error::other("no folder"))?;
    let tmp = dir.join(format!(".mc-{}.tmp", uuid::Uuid::new_v4().simple()));
    let copied = std::fs::copy(kept, &tmp).and_then(|_| std::fs::rename(&tmp, dest));
    if copied.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    copied
}

/// Removes the folders the move made under `top` for a file, deepest
/// first, while they are empty.
fn remove_made(top: &Path, made: &[String]) {
    for dir in made.iter().rev() {
        let _ = std::fs::remove_dir(at(top, dir));
    }
}

/// Puts one written file back (`kept_dir` holds what it replaced): only
/// while it is still what the move wrote. `false`: it changed since, so
/// it stays as it is.
fn put_back(top: &Path, kept_dir: &Path, w: &Wrote) -> Result<bool, String> {
    let dest = at(top, &w.path);
    let now = match std::fs::symlink_metadata(&dest) {
        Ok(m) if m.is_file() => Some(
            crate::claude::transfer::sha256_file(&dest)
                .map(|(_, sha)| sha)
                .map_err(|e| format!("{}: {e}", w.path))?,
        ),
        Ok(_) => return Ok(false),
        Err(_) => None,
    };
    if now.as_deref().is_some_and(|sha| sha != w.sha256) {
        return Ok(false);
    }
    if w.replaced {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", w.path))?;
        }
        restore_atomic(&at(kept_dir, &w.path), &dest).map_err(|e| format!("{}: {e}", w.path))?;
    } else if now.is_some() {
        std::fs::remove_file(&dest).map_err(|e| format!("{}: {e}", w.path))?;
        remove_made(top, &w.made);
    }
    Ok(true)
}

/// Target: writes the files of `archive` (exactly `expected`) under `top`,
/// the folder the code landed in, keeping what they replace in
/// `move_dir` (blocking). All or nothing: a failure puts back what it
/// wrote.
pub fn land(
    top: &Path,
    archive: &Path,
    expected: &[String],
    move_dir: &Path,
) -> Result<Landed, String> {
    let files = read_archive(archive, expected)?;
    // Everything checked before anything is written.
    let mut planned = Vec::new();
    for (path, bytes) in files {
        let (made, exists) = plan(top, &path)?;
        planned.push((path, bytes, made, exists));
    }
    let kept_dir = move_dir.join(KEPT);
    let mut record = Record {
        top: top.to_path_buf(),
        files: Vec::new(),
        undone: false,
    };
    let mut landed = Landed::default();
    for (path, bytes, made, exists) in planned {
        let step = (|| -> Result<Wrote, String> {
            let dest = at(top, &path);
            if exists {
                let kept = at(&kept_dir, &path);
                if let Some(parent) = kept.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::copy(&dest, &kept).map_err(|e| e.to_string())?;
            }
            // A folder an earlier file of this landing made is there now.
            let mut mine = Vec::new();
            for dir in &made {
                let p = at(top, dir);
                match std::fs::create_dir(&p) {
                    Ok(()) => mine.push(dir.clone()),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        let plain = std::fs::symlink_metadata(&p)
                            .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink());
                        if !plain {
                            remove_made(top, &mine);
                            return Err(format!("{dir} is not a plain folder here."));
                        }
                    }
                    Err(e) => {
                        remove_made(top, &mine);
                        return Err(e.to_string());
                    }
                }
            }
            if let Err(e) = write_atomic(&dest, &bytes) {
                remove_made(top, &mine);
                return Err(e.to_string());
            }
            Ok(Wrote {
                path: path.clone(),
                sha256: sha256_hex(&bytes),
                replaced: exists,
                made: mine,
            })
        })();
        match step {
            Ok(w) => {
                if w.replaced {
                    landed.replaced.push(w.path.clone());
                }
                record.files.push(w);
            }
            Err(e) => {
                for w in record.files.iter().rev() {
                    let _ = put_back(top, &kept_dir, w);
                }
                return Err(format!("{path} could not be written here: {e}"));
            }
        }
    }
    landed.written = record.files.len();
    if let Err(e) = crate::persist::write_json_private(&move_dir.join(RECORD), &record) {
        for w in record.files.iter().rev() {
            let _ = put_back(top, &kept_dir, w);
        }
        return Err(format!("The record of the files could not be kept: {e}"));
    }
    Ok(landed)
}

/// Target: undoes [`land`] (blocking): what it replaced comes back, what
/// it added goes — each only while it is what the move wrote. Answers the
/// files left as they are (changed since). Nothing to undo: `Ok`.
pub fn undo(move_dir: &Path) -> Result<Vec<String>, String> {
    let path = move_dir.join(RECORD);
    let Some(mut record) = crate::persist::read_json::<Record>(&path) else {
        return Ok(Vec::new());
    };
    if record.undone {
        return Ok(Vec::new());
    }
    let kept_dir = move_dir.join(KEPT);
    let mut left = Vec::new();
    for w in record.files.iter().rev() {
        if !put_back(&record.top, &kept_dir, w)? {
            left.push(w.path.clone());
        }
    }
    record.undone = true;
    crate::persist::write_json_private(&path, &record).map_err(|e| e.to_string())?;
    Ok(left)
}

/// Target: lands them again after [`undo`] when the rest of the undo could
/// not run (the move stays): from the archive still in the move's folder.
pub fn redo(move_dir: &Path) -> Result<(), String> {
    let path = move_dir.join(RECORD);
    let Some(record) = crate::persist::read_json::<Record>(&path) else {
        return Ok(());
    };
    if !record.undone {
        return Ok(());
    }
    let expected: Vec<String> = record.files.iter().map(|w| w.path.clone()).collect();
    land(&record.top, &move_dir.join(FILE), &expected, move_dir).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(dir: &Path, rel: &str, text: &str) {
        let p = at(dir, rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn found(path: &str) -> Found {
        Found {
            listed: path.into(),
            path: path.into(),
            size: 0,
        }
    }

    #[test]
    fn only_plain_files_inside_the_top_are_found() {
        let tmp = tempfile::tempdir().unwrap();
        let top = dunce::canonicalize(tmp.path()).unwrap();
        put(&top, ".env.local", "SECRET=1\n");
        put(&top, "config/local.json", "{}");
        std::fs::create_dir_all(top.join("dir")).unwrap();
        assert_eq!(spot(&top, ".env.local"), Spot::File(9));
        assert_eq!(spot(&top, "config/local.json"), Spot::File(2));
        assert_eq!(spot(&top, "nothing"), Spot::Missing);
        assert_eq!(spot(&top, ".env.local/x"), Spot::Missing);
        assert_eq!(spot(&top, "dir"), Spot::NotFile);
    }

    #[test]
    fn a_landing_writes_keeps_and_undoes_and_lands_again() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let moved = tempfile::tempdir().unwrap();
        let (src_top, dst_top) = (src.path(), dst.path());
        put(src_top, ".env.local", "FROM=source\n");
        put(src_top, "certs/dev.pem", "PEM\n");
        // The target has its own .env.local; no certs folder.
        put(dst_top, ".env.local", "FROM=target\n");
        let archive = moved.path().join(FILE);
        let files = [found(".env.local"), found("certs/dev.pem")];
        let packed = pack(src_top, &files, &archive).unwrap();
        assert_eq!(packed.files.len(), 2);
        assert_eq!(packed.files[0].size, 12);
        let expected: Vec<String> = files.iter().map(|f| f.path.clone()).collect();

        let landed = land(dst_top, &archive, &expected, moved.path()).unwrap();
        assert_eq!(landed.written, 2);
        assert_eq!(landed.replaced, [".env.local"]);
        let read = |rel: &str| std::fs::read_to_string(at(dst_top, rel)).unwrap();
        assert_eq!(read(".env.local"), "FROM=source\n");
        assert_eq!(read("certs/dev.pem"), "PEM\n");

        assert!(undo(moved.path()).unwrap().is_empty());
        assert_eq!(read(".env.local"), "FROM=target\n", "its own copy is back");
        assert!(!dst_top.join("certs").exists(), "what it added is gone");
        assert!(undo(moved.path()).unwrap().is_empty(), "once");

        // The rest of an undo failed: they land again.
        redo(moved.path()).unwrap();
        assert_eq!(read(".env.local"), "FROM=source\n");
        // Changed since: left as it is.
        put(dst_top, "certs/dev.pem", "EDITED\n");
        assert_eq!(undo(moved.path()).unwrap(), ["certs/dev.pem"]);
        assert_eq!(read("certs/dev.pem"), "EDITED\n");
        assert_eq!(read(".env.local"), "FROM=target\n");
    }

    #[test]
    fn an_archive_with_anything_else_is_refused_before_a_file_is_written() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let moved = tempfile::tempdir().unwrap();
        put(src.path(), "a.env", "a\n");
        put(src.path(), "b.env", "b\n");
        let archive = moved.path().join(FILE);
        pack(src.path(), &[found("a.env"), found("b.env")], &archive).unwrap();
        // It names only one of the two.
        let err = land(dst.path(), &archive, &["a.env".into()], moved.path()).unwrap_err();
        assert!(err.contains("did not name"), "{err}");
        assert!(!dst.path().join("a.env").exists());
        for bad in [vec!["../x".to_string()], vec![".git/config".to_string()]] {
            assert!(land(dst.path(), &archive, &bad, moved.path()).is_err());
        }
        assert!(!valid_list(&["a".to_string(), "a".to_string()]), "twice");
    }

    #[cfg(unix)]
    #[test]
    fn no_file_is_read_or_written_through_a_link() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let top = dunce::canonicalize(tmp.path()).unwrap();
        put(outside.path(), "secret", "x");
        std::os::unix::fs::symlink(outside.path(), top.join("linked")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), top.join("file-link")).unwrap();
        assert_eq!(spot(&top, "linked/secret"), Spot::Link);
        assert_eq!(spot(&top, "file-link"), Spot::Link);
        assert!(plan(&top, "linked/new").is_err());
        assert!(plan(&top, "file-link").is_err());
    }
}
