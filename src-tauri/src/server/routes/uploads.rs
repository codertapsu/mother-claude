//! `POST /api/sessions/{id}/uploads` — keep a file the user attached so the
//! message can reference it as `@<path>`.
//!
//! Sessions that only take text (hook relays, tmux) and non-image files for
//! owned sessions reach Claude this way (`ControlCaps.attach == path`): the
//! composer uploads the file, then inserts the returned absolute path into
//! the message. Files land in `<app data>/uploads/<session id>/` (0700 dir,
//! 0600 files) under a sanitized, randomised name; nothing is ever executed
//! or served back. Sizes are capped per file and per session.

use std::path::{Path as FsPath, PathBuf};

use axum::extract::{Path, State};
use axum::Json;
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Value};

use super::Body;
use crate::server::auth::Authed;
use crate::server::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Largest decoded file (the composer refuses bigger ones up front).
pub const UPLOAD_MAX: usize = 50 * 1024 * 1024;
/// Request body limit: base64 of [`UPLOAD_MAX`] plus the JSON around it.
pub const UPLOAD_BODY_MAX: usize = UPLOAD_MAX / 3 * 4 + 64 * 1024;
/// Everything kept for one session.
pub const SESSION_QUOTA: u64 = 200 * 1024 * 1024;
/// Files kept for one session.
pub const SESSION_FILES_MAX: usize = 100;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UploadBody {
    /// The file's own name (sanitized before use).
    pub name: String,
    /// Informational only; never trusted.
    #[serde(default)]
    pub media_type: Option<String>,
    /// Base64 (a `data:` URL prefix is tolerated).
    pub data: String,
}

/// A file name safe on every filesystem: `[A-Za-z0-9._-]`, no leading dot,
/// at most 80 characters, never empty.
pub fn sanitize_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let mut out = String::with_capacity(base.len().min(80));
    let mut last_sep = false;
    for c in base.chars() {
        let keep = c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_');
        if keep {
            out.push(c);
            last_sep = false;
        } else if !last_sep {
            out.push('_');
            last_sep = true;
        }
    }
    let trimmed = out.trim_start_matches(['.', '_']).trim_end_matches('_');
    let mut clean: String = trimmed.chars().take(80).collect();
    if clean.is_empty() {
        clean = "file".into();
    }
    clean
}

fn uploads_root(state: &AppState) -> Option<PathBuf> {
    state.auth.config_dir.as_ref().map(|d| d.join("uploads"))
}

/// Bytes and files already kept in `dir`.
fn usage(dir: &FsPath) -> (u64, usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (0, 0);
    };
    entries
        .flatten()
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .fold((0, 0), |(bytes, n), m| (bytes + m.len(), n + 1))
}

fn decode(data: &str) -> ApiResult<Vec<u8>> {
    let data = data.trim();
    let data = match data.find(";base64,") {
        Some(i) if data.starts_with("data:") => &data[i + ";base64,".len()..],
        _ => data,
    };
    if data.len() > UPLOAD_BODY_MAX {
        return Err(too_large());
    }
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|_| ApiError::unprocessable("invalid_upload", "The file is not valid base64."))
}

fn too_large() -> ApiError {
    ApiError::new(
        axum::http::StatusCode::PAYLOAD_TOO_LARGE,
        "upload_too_large",
        "Files can be at most 50 MB.",
    )
}

/// `POST /api/sessions/{id}/uploads {name, mediaType?, data}` →
/// `{path, name, size}` (the absolute path to reference as `@<path>`).
pub async fn upload(
    Authed(_ctx): Authed,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<UploadBody>,
) -> ApiResult<Json<Value>> {
    crate::claude::ids::require_session_id(&id)?;
    if state.find_session(&id).await.is_none() {
        return Err(ApiError::no_such_session(&id));
    }
    let Some(root) = uploads_root(&state) else {
        return Err(ApiError::new(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "uploads_unavailable",
            "This server has no data directory to keep uploads in.",
        ));
    };
    let bytes = decode(&body.data)?;
    if bytes.is_empty() {
        return Err(ApiError::unprocessable(
            "invalid_upload",
            "The file is empty.",
        ));
    }
    if bytes.len() > UPLOAD_MAX {
        return Err(too_large());
    }
    let dir = root.join(&id);
    let name = sanitize_name(&body.name);
    let path = dir.join(format!(
        "{}-{name}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    ));
    let size = bytes.len();
    let written = tokio::task::spawn_blocking(move || -> ApiResult<PathBuf> {
        std::fs::create_dir_all(&dir)
            .map_err(|e| ApiError::internal(format!("Could not create the upload folder: {e}")))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
        let (used, files) = usage(&dir);
        if files >= SESSION_FILES_MAX || used + size as u64 > SESSION_QUOTA {
            return Err(ApiError::new(
                axum::http::StatusCode::INSUFFICIENT_STORAGE,
                "upload_quota",
                "This session already keeps too many uploaded files.",
            ));
        }
        crate::persist::write_private(&path, &bytes)
            .map_err(|e| ApiError::internal(format!("Could not store the file: {e}")))?;
        Ok(path)
    })
    .await
    .map_err(|e| ApiError::internal(format!("upload task failed: {e}")))??;
    tracing::info!(session = %id, size, "stored an upload");
    Ok(Json(json!({
        "path": written.to_string_lossy(),
        "name": name,
        "size": size,
        "mediaType": body.media_type,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_sanitized() {
        assert_eq!(sanitize_name("report.pdf"), "report.pdf");
        assert_eq!(sanitize_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_name("..\\..\\x.txt"), "x.txt");
        assert_eq!(sanitize_name(".env"), "env");
        assert_eq!(sanitize_name("my file (1).png"), "my_file_1_.png");
        assert_eq!(sanitize_name(""), "file");
        assert_eq!(sanitize_name("日本語"), "file");
        assert_eq!(sanitize_name(&"a".repeat(200)).len(), 80);
    }

    #[test]
    fn data_urls_and_bad_base64() {
        assert_eq!(decode("data:text/plain;base64,aGk=").unwrap(), b"hi");
        assert_eq!(decode("aGk=").unwrap(), b"hi");
        assert_eq!(decode("!!!").unwrap_err().code, "invalid_upload");
    }
}
