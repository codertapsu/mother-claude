//! `browser_upload`: which files the desktop reads for the extension (the
//! extension never reads the disk).
//!
//! A file may come only from the calling session's project folder (its cwd)
//! or a folder the user approved for uploads (Browser › Settings), resolved
//! with symlinks followed. Never from any account's config directory, Mother
//! Claude's own data, `~/.ssh`, shell startup files, `.claude/`, `.mcp.json`
//! or git hooks (`pending::sensitive_path`), and never a file that usually
//! holds secrets (`.env`, keys, `.npmrc`, `.netrc`, …): the user uploads
//! those by hand. At most [`MAX_FILES`] files and [`MAX_BYTES`] in all, so
//! the `upload` frame stays under the 8 MiB frame limit.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde_json::{json, Value};

use crate::mcp::ToolError;
use crate::state::AppState;

/// Files in one upload.
pub const MAX_FILES: usize = 10;
/// Bytes in one upload (base64 makes it about 6.7 MiB on the wire).
pub const MAX_BYTES: u64 = 5 * 1024 * 1024;

/// A file that passed the checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadFile {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub mime: &'static str,
}

/// File names that usually hold secrets.
fn secret_file(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == ".env"
        || n.starts_with(".env.")
        || n.ends_with(".env")
        || n.starts_with("id_rsa")
        || n.starts_with("id_ed25519")
        || n.starts_with("id_ecdsa")
        || n.starts_with("id_dsa")
        || [
            ".pem",
            ".key",
            ".p12",
            ".pfx",
            ".keystore",
            ".jks",
            ".kdbx",
            ".asc",
            ".gpg",
        ]
        .iter()
        .any(|ext| n.ends_with(ext))
        || [
            ".npmrc",
            ".pypirc",
            ".netrc",
            "_netrc",
            ".pgpass",
            ".git-credentials",
            ".dockercfg",
            "credentials",
            "credentials.json",
            "secrets.json",
            "secrets.yaml",
            "secrets.yml",
            "service-account.json",
            "known_hosts",
            "authorized_keys",
        ]
        .contains(&n.as_str())
}

/// A MIME type from the extension.
pub fn mime_for(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => "application/octet-stream",
    }
}

fn refuse(message: impl Into<String>) -> ToolError {
    ToolError::new("upload_refused", message)
}

/// `p` resolved (symlinks followed), when it exists.
fn canonical(p: &Path) -> Option<PathBuf> {
    dunce::canonicalize(p).ok()
}

/// Where uploads may come from: the session's project and the approved
/// folders, resolved.
fn roots(cwd: Option<&str>, folders: &[String]) -> Vec<PathBuf> {
    cwd.filter(|c| !c.trim().is_empty())
        .map(PathBuf::from)
        .into_iter()
        .chain(folders.iter().map(PathBuf::from))
        .filter(|p| p.is_absolute())
        .filter_map(|p| canonical(&p))
        .collect()
}

/// Every account's config directory (and the Default's `~/.claude.json`),
/// resolved.
fn config_dirs(state: &AppState) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = state
        .accounts
        .homes()
        .into_iter()
        .map(|(_, h)| h.base().to_path_buf())
        .chain(std::iter::once(state.home.base().to_path_buf()))
        .flat_map(|p| {
            let c = canonical(&p);
            std::iter::once(p).chain(c)
        })
        .collect();
    out.extend(crate::persist::app_data_dir());
    out
}

/// Check the paths a model asked to upload.
pub fn check(
    state: &AppState,
    cwd: Option<&str>,
    paths: &[String],
) -> Result<Vec<UploadFile>, ToolError> {
    if paths.is_empty() {
        return Err(ToolError::invalid("Give at least one file in `paths`."));
    }
    if paths.len() > MAX_FILES {
        return Err(ToolError::invalid(format!(
            "At most {MAX_FILES} files in one upload."
        )));
    }
    let folders = state.extension.settings.get().upload_folders;
    let allowed = roots(cwd, &folders);
    if allowed.is_empty() {
        return Err(refuse(
            "Mother Claude does not know this session's project folder, and no upload folder \
             is approved: the user can approve one in Mother Claude (Browser › Settings).",
        ));
    }
    let forbidden = config_dirs(state);
    let env = crate::pending::DangerEnv::current(cwd);
    let mut total = 0u64;
    let mut out = Vec::new();
    for raw in paths {
        let raw = raw.trim();
        let given = if let Some(rest) = raw.strip_prefix("~/") {
            crate::claude::user_home_dir()
                .map(|h| h.join(rest))
                .ok_or_else(|| refuse("No home folder to resolve ~ against."))?
        } else {
            PathBuf::from(raw)
        };
        let given = if given.is_absolute() {
            given
        } else {
            match cwd {
                Some(c) if !c.is_empty() => Path::new(c).join(given),
                _ => {
                    return Err(ToolError::invalid(format!(
                        "{raw} is relative and the session's folder is unknown: give a full \
                         path."
                    )))
                }
            }
        };
        let Some(path) = canonical(&given) else {
            return Err(ToolError::new(
                "not_found",
                format!("{raw} does not exist."),
            ));
        };
        let meta = std::fs::metadata(&path)
            .map_err(|e| ToolError::new("not_found", format!("{raw} cannot be read: {e}.")))?;
        if !meta.is_file() {
            return Err(ToolError::invalid(format!("{raw} is not a file.")));
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if forbidden.iter().any(|d| path.starts_with(d)) {
            return Err(refuse(format!(
                "{raw} is in a Claude Code configuration folder or Mother Claude's own data: \
                 never uploaded."
            )));
        }
        if let Some(why) = crate::pending::sensitive_path(&path.to_string_lossy(), &env) {
            return Err(refuse(format!("{raw} is {why}: never uploaded.")));
        }
        if secret_file(&name) {
            return Err(refuse(format!(
                "{name} usually holds secrets: Mother Claude never uploads it for a model. The \
                 user can upload it by hand."
            )));
        }
        if !allowed.iter().any(|r| path.starts_with(r)) {
            return Err(refuse(format!(
                "{raw} is outside this session's project folder and the approved upload \
                 folders (Mother Claude › Browser › Settings)."
            )));
        }
        total += meta.len();
        if total > MAX_BYTES {
            return Err(refuse(format!(
                "The files add up to more than {} MiB, the most one upload carries.",
                MAX_BYTES / (1024 * 1024)
            )));
        }
        out.push(UploadFile {
            mime: mime_for(&name),
            name,
            size: meta.len(),
            path,
        });
    }
    Ok(out)
}

/// The `files` of an `upload` command (read after the approval).
pub fn read(files: &[UploadFile]) -> Result<Vec<Value>, ToolError> {
    files
        .iter()
        .map(|f| {
            let bytes = std::fs::read(&f.path).map_err(|e| {
                ToolError::new("not_found", format!("{} cannot be read: {e}.", f.name))
            })?;
            if bytes.len() as u64 > MAX_BYTES {
                return Err(refuse(format!("{} grew too large to upload.", f.name)));
            }
            Ok(json!({
                "name": f.name,
                "mimeType": f.mime,
                "data": base64::engine::general_purpose::STANDARD.encode(bytes),
            }))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> AppState {
        crate::state::Inner::new(
            crate::claude::ClaudeHome::with_base(std::env::temp_dir().join("mc-no-such-home")),
            crate::state::ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            crate::server::auth::Auth::ephemeral(),
        )
    }

    #[test]
    fn only_project_files_that_are_not_secrets() {
        let st = state();
        let project = tempfile::tempdir().unwrap();
        let p = project.path();
        std::fs::write(p.join("report.pdf"), b"%PDF").unwrap();
        std::fs::write(p.join(".env"), b"KEY=1").unwrap();
        std::fs::create_dir_all(p.join(".claude")).unwrap();
        std::fs::write(p.join(".claude/settings.json"), b"{}").unwrap();
        std::fs::create_dir_all(p.join("sub")).unwrap();
        let cwd = p.to_string_lossy().into_owned();

        let ok = check(&st, Some(&cwd), &["report.pdf".into()]).unwrap();
        assert_eq!(ok.len(), 1);
        assert_eq!(ok[0].name, "report.pdf");
        assert_eq!(ok[0].mime, "application/pdf");
        let files = read(&ok).unwrap();
        assert_eq!(files[0]["data"], "JVBERg==");

        let code = |paths: &[&str]| {
            check(
                &st,
                Some(&cwd),
                &paths.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            )
            .unwrap_err()
            .code
        };
        assert_eq!(code(&[".env"]), "upload_refused");
        assert_eq!(code(&[".claude/settings.json"]), "upload_refused");
        assert_eq!(code(&["sub"]), "invalid_arguments");
        assert_eq!(code(&["missing.txt"]), "not_found");
        assert_eq!(code(&[]), "invalid_arguments");

        // Outside the project: refused, unless the folder is approved.
        let other = tempfile::tempdir().unwrap();
        std::fs::write(other.path().join("a.png"), b"png").unwrap();
        let outside = other.path().join("a.png").to_string_lossy().into_owned();
        assert_eq!(code(&[&outside]), "upload_refused");
        st.extension
            .settings
            .update(|s| {
                s.upload_folders
                    .push(other.path().to_string_lossy().into_owned());
                Ok(())
            })
            .unwrap();
        assert!(check(&st, Some(&cwd), &[outside]).is_ok());

        // No project and no folder: nothing.
        let st2 = state();
        assert_eq!(
            check(&st2, None, &["/tmp/x".into()]).unwrap_err().code,
            "upload_refused"
        );
    }

    #[test]
    fn secret_names_and_mime_types() {
        for n in [
            ".env",
            ".env.local",
            "prod.env",
            "id_rsa",
            "id_ed25519.pub",
            "cert.pem",
            ".npmrc",
        ] {
            assert!(secret_file(n), "{n}");
        }
        for n in ["report.pdf", "env.md", "keyboard.png", "README"] {
            assert!(!secret_file(n), "{n}");
        }
        assert_eq!(mime_for("a.PNG"), "image/png");
        assert_eq!(mime_for("noext"), "application/octet-stream");
    }
}
