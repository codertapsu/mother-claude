//! Validators for every id that ends up in a filesystem path, a glob or an
//! argv (design §10 B2, "claude::ids").
//!
//! Ids reach the server from URLs (axum percent-decodes `%2F`), query strings,
//! WebSocket frames and hook payloads. None of them may be trusted to build a
//! path: validate first, then join, then (for files that must stay inside a
//! known root) canonicalize and check the result with [`under_root`].
//!
//! * session id / uuid — canonical lowercase UUID (`8-4-4-4-12` hex);
//! * agent / task / tool-use / run ids — `^[A-Za-z0-9_-]{1,80}$`;
//! * workflow run ids — `^wf_[A-Za-z0-9_-]{1,64}$`;
//! * short background job ids — `^[0-9a-f]{6,16}$`.
//!
//! B3/B4/B5 use these at their routes and at hook ingestion too.

use std::path::{Path, PathBuf};

use axum::http::StatusCode;

use crate::server::error::{ApiError, ApiResult};

/// Canonical lowercase UUID: `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`.
pub fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 36 {
        return false;
    }
    b.iter().enumerate().all(|(i, c)| match i {
        8 | 13 | 18 | 23 => *c == b'-',
        _ => c.is_ascii_digit() || (b'a'..=b'f').contains(c),
    })
}

/// A Claude Code session id (always a canonical lowercase UUID).
pub fn is_session_id(s: &str) -> bool {
    is_uuid(s)
}

fn simple(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

/// Agent ids, task ids, tool-use ids: `^[A-Za-z0-9_-]{1,80}$`.
pub fn is_simple_id(s: &str) -> bool {
    simple(s, 80)
}

/// Agent id (`a1b2…`), as used in `agent-<id>.jsonl`.
pub fn is_agent_id(s: &str) -> bool {
    is_simple_id(s)
}

/// Background task id (`b…`, `w…`, `a…`).
pub fn is_task_id(s: &str) -> bool {
    is_simple_id(s)
}

/// Tool-use id (`toolu_…`).
pub fn is_tool_use_id(s: &str) -> bool {
    is_simple_id(s)
}

/// Workflow run id: `wf_` + `[A-Za-z0-9_-]{1,64}`.
pub fn is_run_id(s: &str) -> bool {
    s.strip_prefix("wf_").is_some_and(|rest| simple(rest, 64))
}

/// Short background job id (`claude stop|rm|respawn|logs|attach <short>`).
pub fn is_short_job_id(s: &str) -> bool {
    (6..=16).contains(&s.len())
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// Parse a media block path (`3` or `3.0`): dot-separated u16 indices, at
/// most 4 levels.
pub fn parse_block_path(s: &str) -> Option<Vec<usize>> {
    if s.is_empty() || s.len() > 32 {
        return None;
    }
    let parts: Option<Vec<usize>> = s
        .split('.')
        .map(|p| {
            if p.is_empty() || p.len() > 5 || !p.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            p.parse::<u16>().ok().map(usize::from)
        })
        .collect();
    parts.filter(|v| !v.is_empty() && v.len() <= 4)
}

/// `400 invalid_id` unless `id` is a session id.
pub fn require_session_id(id: &str) -> ApiResult<()> {
    if is_session_id(id) {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_id",
            "Not a valid session id.",
        ))
    }
}

/// `400` unless `id` is a valid agent/task/tool-use id.
pub fn require_simple_id(id: &str, what: &str) -> ApiResult<()> {
    if is_simple_id(id) {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_id",
            format!("Not a valid {what}."),
        ))
    }
}

/// Canonicalize `candidate` and return it only if it still lies under the
/// canonicalized `root` (symlinks that escape the root are rejected). Both
/// must exist.
pub fn under_root(root: &Path, candidate: &Path) -> Option<PathBuf> {
    let root = std::fs::canonicalize(root).ok()?;
    let real = std::fs::canonicalize(candidate).ok()?;
    real.starts_with(&root).then_some(real)
}

/// A relative path from a client (`?path=` of file-patch, `@` files): no
/// absolute paths, no `..`, no NUL, no Windows prefixes.
pub fn is_safe_relative_path(p: &str) -> bool {
    if p.is_empty()
        || p.len() > 4096
        || p.contains('\0')
        || p.starts_with('/')
        || p.starts_with('\\')
    {
        return false;
    }
    Path::new(p).components().all(|c| {
        matches!(
            c,
            std::path::Component::Normal(_) | std::path::Component::CurDir
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids() {
        assert!(is_uuid("e2b6972f-7126-4f6e-a490-5ce75cb6a9f9"));
        assert!(
            !is_uuid("E2B6972F-7126-4F6E-A490-5CE75CB6A9F9"),
            "uppercase is not canonical"
        );
        assert!(!is_uuid("e2b6972f71264f6ea4905ce75cb6a9f9"));
        assert!(!is_uuid("e2b6972f-7126-4f6e-a490-5ce75cb6a9f"));
        assert!(!is_uuid("../../../etc/passwd-aaaa-aaaa-aaaaaaaa"));
        assert!(!is_uuid("e2b6972f-7126-4f6e-a490-5ce75cb6a9g9"));
        assert!(!is_uuid("*"));
    }

    #[test]
    fn simple_ids_reject_traversal_and_globs() {
        for ok in [
            "a1b2c3d4e5f6a7b8c",
            "b534p4id0",
            "toolu_01RFgznTTZ",
            "w59or2obz",
        ] {
            assert!(is_simple_id(ok), "{ok}");
        }
        for bad in [
            "",
            "../x",
            "a/b",
            "a%2Fb",
            "*",
            "a[1]",
            "a b",
            "a.b",
            &"x".repeat(81),
        ] {
            assert!(!is_simple_id(bad), "{bad}");
        }
        assert!(is_run_id("wf_cc46e3d7-371"));
        assert!(!is_run_id("cc46e3d7-371"));
        assert!(!is_run_id("wf_../x"));
        assert!(is_short_job_id("631b387d"));
        assert!(!is_short_job_id("631B387D"));
        assert!(!is_short_job_id("631b3"));
        assert!(!is_short_job_id("631b387d-a7e5-4e12-ab38-522cb612803b"));
    }

    #[test]
    fn block_paths() {
        assert_eq!(parse_block_path("3"), Some(vec![3]));
        assert_eq!(parse_block_path("3.0"), Some(vec![3, 0]));
        assert_eq!(parse_block_path(""), None);
        assert_eq!(parse_block_path("3..0"), None);
        assert_eq!(parse_block_path("-1"), None);
        assert_eq!(parse_block_path("70000"), None);
        assert_eq!(parse_block_path("1.2.3.4.5"), None);
    }

    #[test]
    fn relative_paths() {
        assert!(is_safe_relative_path("src/main.rs"));
        assert!(is_safe_relative_path("./a/b"));
        assert!(!is_safe_relative_path("../etc/passwd"));
        assert!(!is_safe_relative_path("a/../../b"));
        assert!(!is_safe_relative_path("/etc/passwd"));
        assert!(!is_safe_relative_path(""));
    }

    #[test]
    fn under_root_rejects_escapes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/f.txt"), "x").unwrap();
        std::fs::write(dir.path().join("outside.txt"), "y").unwrap();
        assert!(under_root(&root, &root.join("sub/f.txt")).is_some());
        assert!(under_root(&root, &root.join("sub/../../outside.txt")).is_none());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.path().join("outside.txt"), root.join("link")).unwrap();
            assert!(under_root(&root, &root.join("link")).is_none());
        }
        assert!(under_root(&root, &root.join("missing")).is_none());
    }
}
