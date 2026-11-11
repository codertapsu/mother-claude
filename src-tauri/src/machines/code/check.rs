//! Names, ids and paths a move may hold, and the records' files.
//!
//! A move's record comes from another machine: nothing in it reaches git's
//! argv or the file system before these checks pass.

use std::path::Path;

use serde::de::DeserializeOwned;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::types::{
    io_err, ApplyRecord, CodeError, MoveRefs, SnapshotMeta, TransferKind, BUNDLE, FORMAT,
};

/// A record larger than this is refused.
const MAX_RECORD: u64 = 512 << 20;

/// A full object id (SHA-1 or SHA-256), lowercase hex.
pub fn is_oid(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A Git LFS object id (SHA-256, lowercase hex).
pub fn is_lfs_oid(s: &str) -> bool {
    s.len() == 64 && is_oid(s)
}

/// A session id: letters, digits, `-` and `_` (a Claude session's uuid).
pub fn valid_session(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && !s.starts_with('-')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn valid_epoch(s: &str) -> bool {
    !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit())
}

/// A machine's name made safe for a ref: letters, digits, `-` and `_`.
pub fn safe_name(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
            dash = false;
        } else if !dash {
            out.push('-');
            dash = true;
        }
    }
    let out: String = out.trim_matches('-').chars().take(40).collect();
    let out = out.trim_end_matches('-');
    if out.is_empty() {
        "machine".to_string()
    } else {
        out.to_string()
    }
}

/// A branch's full name (`refs/heads/…`), per `git check-ref-format`.
pub fn valid_branch(s: &str) -> bool {
    let Some(short) = s.strip_prefix("refs/heads/") else {
        return false;
    };
    !short.is_empty()
        && s.len() <= 255
        && !s.contains("..")
        && !s.contains("@{")
        && !s.ends_with('/')
        && !s.ends_with('.')
        && s.bytes().all(|b| {
            b > 0x20 && b != 0x7f && !matches!(b, b'~' | b'^' | b':' | b'?' | b'*' | b'[' | b'\\')
        })
        && short
            .split('/')
            .all(|c| !c.is_empty() && !c.starts_with('.') && !c.ends_with(".lock"))
}

/// A remote's name: never an option, a URL or a path.
pub fn valid_remote_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && !s.starts_with('-')
        && !s.starts_with('.')
        && !s.contains("..")
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// A Windows device name (`con`, `aux.txt`, `COM1`, …).
pub fn is_windows_reserved(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_lowercase();
    matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || ((stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.len() == 4
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

/// A name Windows cannot hold.
pub fn windows_bad_name(name: &str) -> bool {
    is_windows_reserved(name)
        || name.chars().any(|c| {
            matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\\') || (c as u32) < 0x20
        })
        || name.ends_with('.')
        || name.ends_with(' ')
}

/// A path from a tree, safe to join to a checkout: relative, no `.`, `..`
/// or `.git` component, nothing a file system would read as another place.
pub fn safe_rel_path(p: &str) -> bool {
    if p.is_empty() || p.len() > 4096 || p.starts_with('/') || p.contains('\\') || p.contains('\0')
    {
        return false;
    }
    p.split('/').all(|c| {
        let bare = c.trim_end_matches(['.', ' ']);
        let lower = bare.to_ascii_lowercase();
        !c.is_empty()
            && !bare.is_empty()
            && lower != ".git"
            && lower != "git~1"
            // A drive or an alternate data stream on Windows.
            && !(cfg!(windows) && c.contains(':'))
    })
}

/// A path this computer's file system holds as itself: on Windows no
/// device name and no trailing dot or space (Windows would open another
/// file, or a device). Git refuses to check such paths out there too.
pub fn holdable_here(p: &str) -> bool {
    !cfg!(windows)
        || !p
            .split('/')
            .any(|c| is_windows_reserved(c) || c.ends_with('.') || c.ends_with(' '))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// `20261111T101500-3fa2`: a time-ordered, unique-enough name.
pub fn stamp() -> String {
    let rand = uuid::Uuid::new_v4().simple().to_string();
    format!(
        "{}-{}",
        chrono::Utc::now().format("%Y%m%dT%H%M%S"),
        &rand[..4]
    )
}

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, CodeError> {
    let len = std::fs::metadata(path)
        .map_err(io_err(format!("read {}", path.display())))?
        .len();
    if len > MAX_RECORD {
        return Err(CodeError::BadMove(format!(
            "{} is too large",
            path.display()
        )));
    }
    let bytes = std::fs::read(path).map_err(io_err(format!("read {}", path.display())))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| CodeError::BadMove(format!("{}: {e}", path.display())))
}

/// Writes a record whole or not at all (temp file + rename).
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), CodeError> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|e| CodeError::BadMove(format!("cannot serialize: {e}")))?;
    bytes.push(b'\n');
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4().simple()));
    std::fs::write(&tmp, &bytes).map_err(io_err(format!("write {}", tmp.display())))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io_err(format!("write {}", path.display()))(e)
    })
}

fn bad<T>(why: impl Into<String>) -> Result<T, CodeError> {
    Err(CodeError::BadMove(why.into()))
}

impl SnapshotMeta {
    /// Checks every name, id and path before anything uses one.
    pub fn validate(&self) -> Result<(), CodeError> {
        if self.version != FORMAT {
            return bad(format!(
                "format {} (this version reads {FORMAT})",
                self.version
            ));
        }
        if !valid_session(&self.session) {
            return bad("the session id");
        }
        if !valid_epoch(&self.epoch) {
            return bad("the epoch");
        }
        if safe_name(&self.machine) != self.machine {
            return bad("the machine's name");
        }
        for (what, oid) in [
            ("idxTree", &self.idx_tree),
            ("wtTree", &self.wt_tree),
            ("snap", &self.snap),
            ("indexCommit", &self.index_commit),
        ] {
            if !is_oid(oid) {
                return bad(format!("{what} is not an object id"));
            }
        }
        if self.base.as_deref().is_some_and(|b| !is_oid(b)) {
            return bad("base is not an object id");
        }
        match &self.branch {
            Some(b) if !valid_branch(b) => return bad("the branch name"),
            None if self.base.is_none() => return bad("a detached snapshot without a base"),
            _ => {}
        }
        if self.refs != MoveRefs::for_move(&self.session, &self.epoch) {
            return bad("the move's refs");
        }
        match self.transfer {
            TransferKind::Bundle => match &self.bundle {
                Some(b) if b.file == BUNDLE => {}
                _ => return bad("the bundle"),
            },
            TransferKind::Remote => match &self.remote {
                Some(r)
                    if valid_remote_name(&r.remote)
                        && r.hidden == MoveRefs::hidden(&self.machine, &self.session) => {}
                _ => return bad("the remote"),
            },
        }
        if self.chained.iter().any(|c| !is_oid(c)) {
            return bad("a chained snapshot is not an object id");
        }
        for e in &self.manifest {
            if !safe_rel_path(&e.path) {
                return bad(format!("the path {:?} points outside the checkout", e.path));
            }
            if !holdable_here(&e.path) {
                return bad(format!("{:?} is a name this computer cannot hold", e.path));
            }
            if !is_oid(&e.oid)
                || !matches!(e.mode.as_str(), "100644" | "100755" | "120000" | "160000")
            {
                return bad(format!("the entry for {:?}", e.path));
            }
        }
        if let Some(l) = &self.lfs {
            if l.carried.iter().any(|o| !is_lfs_oid(o)) {
                return bad("a Git LFS object id");
            }
        }
        Ok(())
    }
}

impl ApplyRecord {
    /// The record is ours, but undo acts on what it names.
    pub fn validate(&self) -> Result<(), CodeError> {
        if let Some(a) = &self.applied {
            if a.wt_branch.as_deref().is_some_and(|b| !valid_branch(b)) {
                return bad("the worktree's branch");
            }
        }
        if let Some(u) = &self.undo {
            let oids = [&u.prev_head, &u.branch_old];
            if oids
                .iter()
                .any(|o| o.as_deref().is_some_and(|o| !is_oid(o)))
                || u.prev_sym.as_deref().is_some_and(|s| !valid_branch(s))
                || u.parked
                    .as_ref()
                    .is_some_and(|p| !is_oid(&p.wt_tree) || !is_oid(&p.idx_tree))
            {
                return bad("the undo record");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_paths_from_another_machine_are_checked() {
        assert!(valid_session("0b5c3a1e-9a7f-4c55-8d1e-1c2b3a4d5e6f"));
        assert!(!valid_session("../x") && !valid_session("-x") && !valid_session(""));
        assert_eq!(safe_name("Khanh's MacBook.local"), "Khanh-s-MacBook-local");
        assert_eq!(safe_name("..."), "machine");
        assert!(valid_branch("refs/heads/feature/x"));
        for b in [
            "refs/heads/",
            "refs/heads/a..b",
            "refs/heads/.x",
            "refs/heads/x.lock",
            "refs/heads/a b",
            "refs/tags/v1",
            "--upload-pack=x",
        ] {
            assert!(!valid_branch(b), "{b}");
        }
        assert!(valid_remote_name("origin"));
        assert!(!valid_remote_name("--upload-pack=x") && !valid_remote_name("ext::sh"));
        assert!(safe_rel_path("src/main.rs") && safe_rel_path("über/naïve café.txt"));
        for p in [
            "",
            "/etc/passwd",
            "../x",
            "a/../../x",
            "a/./b",
            "a//b",
            ".git/hooks/pre-commit",
            "x/.GIT/config",
            "a\\b",
        ] {
            assert!(!safe_rel_path(p), "{p}");
        }
        assert_eq!(holdable_here("x/aux.txt"), !cfg!(windows));
        assert_eq!(holdable_here("trailing."), !cfg!(windows));
        assert!(holdable_here("auxiliary/notes.txt"));
        assert!(is_windows_reserved("aux.txt") && is_windows_reserved("COM1"));
        assert!(!is_windows_reserved("auxiliary.txt") && !is_windows_reserved("com0"));
        assert!(windows_bad_name("what?.txt") && windows_bad_name("trailing."));
        assert!(is_oid(&"a".repeat(40)) && !is_oid(&"A".repeat(40)) && !is_oid("abc"));
    }
}
