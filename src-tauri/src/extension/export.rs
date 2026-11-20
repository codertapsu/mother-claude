//! `browser_cookies` with `action: "export"`: a site's cookie values written
//! to a file for another program on this computer (such as a downloader that
//! takes a `cookies.txt`), so that they never reach a model. The model gets
//! the file's path, the cookies' names and expiry, and the names it asked for
//! that are not set.
//!
//! * The desktop writes the file, never the extension or the model:
//!   `<app data>/exports/cookies-<random>.txt|json`, 0600 (the folder 0700),
//!   atomically. It is in Mother Claude's own data, so `browser_upload` never
//!   takes it and no repository picks it up.
//! * The program that reads it deletes it. Mother Claude deletes it [`TTL`]
//!   after writing it, and every export left over at startup.
//! * Netscape format (curl, yt-dlp): one tab-separated line per cookie,
//!   `domain, includeSubdomains, path, secure, expires (0: a session
//!   cookie), name, value`; an HttpOnly cookie's domain is prefixed with
//!   `#HttpOnly_`. JSON: `[{name, value, domain, path, expires?, httpOnly,
//!   secure}]`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use serde_json::{json, Value};

/// How long an export waits for the program that reads it.
pub const TTL: Duration = Duration::from_secs(600);
/// Every export's file name starts with it.
const PREFIX: &str = "cookies-";

/// The file's format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Netscape,
    Json,
}

impl Format {
    pub fn parse(s: Option<&str>) -> Option<Self> {
        match s {
            None | Some("netscape") => Some(Format::Netscape),
            Some("json") => Some(Format::Json),
            Some(_) => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Format::Netscape => "netscape",
            Format::Json => "json",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Format::Netscape => "txt",
            Format::Json => "json",
        }
    }
}

/// Where exports go.
#[derive(Debug, Default)]
pub struct Exports {
    dir: Mutex<Option<PathBuf>>,
}

impl Exports {
    /// Exports in `dir` (`None`: nowhere until [`Exports::set_dir`]).
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self {
            dir: Mutex::new(dir),
        }
    }

    /// Tests: a folder of their own.
    pub fn set_dir(&self, dir: Option<PathBuf>) {
        *self.dir.lock().unwrap_or_else(|p| p.into_inner()) = dir;
    }

    pub fn dir(&self) -> Option<PathBuf> {
        self.dir.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Delete the exports older than `older_than` (every one with
    /// `Duration::ZERO`). Returns how many went.
    pub fn sweep(&self, older_than: Duration) -> usize {
        self.dir().map_or(0, |d| sweep_dir(&d, older_than))
    }

    /// Write one export of `cookies` (each with its `value`) and return its
    /// path.
    pub fn write(&self, format: Format, cookies: &[Value]) -> std::io::Result<PathBuf> {
        let dir = self.dir().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Mother Claude has no data folder to write the export to",
            )
        })?;
        std::fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
        let path = dir.join(format!(
            "{PREFIX}{}.{}",
            uuid::Uuid::new_v4().simple(),
            format.extension()
        ));
        crate::persist::write_private(&path, render(format, cookies).as_bytes())?;
        Ok(path)
    }
}

/// Delete `path` after [`TTL`], unless the program that read it already did.
pub fn remove_later(path: PathBuf) {
    if let Ok(rt) = tokio::runtime::Handle::try_current() {
        rt.spawn(async move {
            tokio::time::sleep(TTL).await;
            match std::fs::remove_file(&path) {
                Ok(()) => tracing::debug!("a cookie export nobody read was deleted"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => tracing::warn!(error = %e, "could not delete a cookie export"),
            }
        });
    }
}

fn sweep_dir(dir: &Path, older_than: Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let now = SystemTime::now();
    let mut gone = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(PREFIX) {
            continue;
        }
        // Never through a link.
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let old_enough = older_than.is_zero()
            || meta
                .modified()
                .ok()
                .and_then(|m| now.duration_since(m).ok())
                .is_some_and(|age| age >= older_than);
        if old_enough && std::fs::remove_file(entry.path()).is_ok() {
            gone += 1;
        }
    }
    gone
}

fn text<'a>(c: &'a Value, key: &str) -> &'a str {
    c.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn flag(c: &Value, key: &str) -> bool {
    c.get(key).and_then(Value::as_bool) == Some(true)
}

/// Epoch seconds, `None` for a session cookie.
fn expires(c: &Value) -> Option<i64> {
    c.get("expires")
        .and_then(Value::as_f64)
        .filter(|e| *e > 0.0)
        .map(|e| e as i64)
}

/// A field a line can hold: no tab or line break.
fn plain(s: &str) -> bool {
    !s.chars().any(|c| matches!(c, '\t' | '\n' | '\r'))
}

/// The file's text.
pub fn render(format: Format, cookies: &[Value]) -> String {
    match format {
        Format::Netscape => netscape(cookies),
        Format::Json => {
            let list: Vec<Value> = cookies
                .iter()
                .map(|c| {
                    let mut o = json!({
                        "name": text(c, "name"),
                        "value": text(c, "value"),
                        "domain": text(c, "domain"),
                        "path": match text(c, "path") { "" => "/", p => p },
                        "httpOnly": flag(c, "httpOnly"),
                        "secure": flag(c, "secure"),
                    });
                    if let Some(e) = expires(c) {
                        o["expires"] = json!(e);
                    }
                    o
                })
                .collect();
            serde_json::to_string_pretty(&list).unwrap_or_else(|_| "[]".into())
        }
    }
}

fn netscape(cookies: &[Value]) -> String {
    let mut out = String::from(
        "# Netscape HTTP Cookie File\n# Written by Mother Claude for one program, which deletes it after reading it.\n\n",
    );
    for c in cookies {
        let (name, value, domain) = (text(c, "name"), text(c, "value"), text(c, "domain"));
        let path = match text(c, "path") {
            "" => "/",
            p => p,
        };
        if name.is_empty()
            || domain.is_empty()
            || ![name, value, domain, path].iter().all(|f| plain(f))
        {
            continue;
        }
        // A domain cookie (".bilibili.com") goes to every subdomain; a
        // host-only one ("www.bilibili.com") to its host alone.
        let subdomains = if domain.starts_with('.') {
            "TRUE"
        } else {
            "FALSE"
        };
        let domain = if flag(c, "httpOnly") {
            format!("#HttpOnly_{domain}")
        } else {
            domain.to_string()
        };
        let secure = if flag(c, "secure") { "TRUE" } else { "FALSE" };
        out.push_str(&format!(
            "{domain}\t{subdomains}\t{path}\t{secure}\t{}\t{name}\t{value}\n",
            expires(c).unwrap_or(0)
        ));
    }
    out
}

/// What the model is told about one exported cookie: never its value.
pub fn described(c: &Value) -> Value {
    let mut o = json!({
        "name": text(c, "name"),
        "domain": text(c, "domain"),
        "httpOnly": flag(c, "httpOnly"),
        "secure": flag(c, "secure"),
    });
    match expires(c) {
        Some(e) => o["expires"] = json!(e),
        None => o["session"] = json!(true),
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookies() -> Vec<Value> {
        vec![
            json!({"name": "SESSDATA", "value": "abc%2C123", "domain": ".bilibili.com", "path": "/",
                   "httpOnly": true, "secure": true, "expires": 1_790_000_000.5}),
            json!({"name": "buvid3", "value": "B-1", "domain": ".bilibili.com", "path": "/",
                   "httpOnly": false, "secure": false, "session": true}),
            json!({"name": "host", "value": "h", "domain": "www.bilibili.com", "path": "",
                   "httpOnly": false, "secure": true, "expires": 1_800_000_000}),
            json!({"name": "bad", "value": "a\tb", "domain": ".bilibili.com", "path": "/"}),
        ]
    }

    #[test]
    fn netscape_lines_mark_http_only_and_host_only_cookies() {
        let text = render(Format::Netscape, &cookies());
        let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines[0], "# Netscape HTTP Cookie File");
        assert!(lines[1].starts_with("# "));
        assert_eq!(
            lines[2],
            "#HttpOnly_.bilibili.com\tTRUE\t/\tTRUE\t1790000000\tSESSDATA\tabc%2C123"
        );
        assert_eq!(lines[3], ".bilibili.com\tTRUE\t/\tFALSE\t0\tbuvid3\tB-1");
        assert_eq!(
            lines[4],
            "www.bilibili.com\tFALSE\t/\tTRUE\t1800000000\thost\th"
        );
        // A value no line can hold is left out.
        assert_eq!(lines.len(), 5);
    }

    #[test]
    fn json_keeps_values_and_leaves_out_a_session_expiry() {
        let v: Vec<Value> = serde_json::from_str(&render(Format::Json, &cookies()[..2])).unwrap();
        assert_eq!(v[0]["name"], "SESSDATA");
        assert_eq!(v[0]["value"], "abc%2C123");
        assert_eq!(v[0]["expires"], 1_790_000_000);
        assert_eq!(v[0]["httpOnly"], true);
        assert!(v[1].get("expires").is_none());
    }

    #[test]
    fn a_description_never_has_the_value() {
        let d = described(&cookies()[0]);
        assert!(d.get("value").is_none());
        assert_eq!(d["expires"], 1_790_000_000);
        assert_eq!(described(&cookies()[1])["session"], true);
    }

    #[test]
    fn formats_parse() {
        assert_eq!(Format::parse(None), Some(Format::Netscape));
        assert_eq!(Format::parse(Some("json")), Some(Format::Json));
        assert_eq!(Format::parse(Some("yaml")), None);
    }

    #[test]
    fn exports_are_private_files_that_a_sweep_removes() {
        let dir = tempfile::tempdir().unwrap();
        let ex = Exports::new(Some(dir.path().join("exports")));
        let path = ex.write(Format::Netscape, &cookies()).unwrap();
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("cookies-"));
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("SESSDATA\tabc%2C123"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
            let dmode = std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(dmode & 0o777, 0o700);
        }
        // Another file in the folder is not ours to delete.
        let other = path.parent().unwrap().join("keep.txt");
        std::fs::write(&other, "x").unwrap();
        // Too young for a sweep that keeps ten minutes.
        assert_eq!(ex.sweep(TTL), 0);
        assert!(path.exists());
        // At startup every export goes.
        assert_eq!(ex.sweep(Duration::ZERO), 1);
        assert!(!path.exists());
        assert!(other.exists());
        // No folder: nothing written.
        assert!(Exports::new(None).write(Format::Json, &[]).is_err());
    }
}
