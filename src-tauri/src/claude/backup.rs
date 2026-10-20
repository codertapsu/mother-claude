//! Backups of each account's data to a folder of the user's choosing (iCloud
//! Drive and Google Drive included), restore from them, and pull from a set
//! another machine pushed (docs/ACCOUNTS.md D6, D7, §4.3, §5.10, §5.11).
//!
//! **A set** is `<folder>/<account name>/`:
//!
//! ```text
//! set.json                    plaintext: version, account name, the set's public key, machines
//! identity.age                the set's X25519 key, encrypted with the backup passphrase (scrypt)
//! index.json.age              what is in the set, encrypted to the set's key
//! sessions/<id>.tar.gz.age    one archive per session (paths relative to the config directory)
//! config/<time>.tar.gz.age    the configuration bundle, the last five
//! ```
//!
//! Every archive is `tar` + `gzip` + `age` and opens without Mother Claude:
//! `age -d -o key.txt identity.age && age -d -i key.txt sessions/<id>.tar.gz.age
//! | tar xz`. The passphrase's work factor (about a second of scrypt) is paid
//! once per run, for `identity.age`, not once per archive.
//!
//! A session is re-archived only when its transcript size or its newest file
//! time changed; a live one waits until it is idle. Archives are written to a
//! temp name in the set, fsynced and renamed. Reading the source never touches
//! anything but the files backed up; the set's folder is the only thing
//! written. A restore writes only new files into the target directory, keeps
//! an undo manifest like a transfer's, and warns about a session Claude Code
//! would prune again (`cleanupPeriodDays`).

use std::collections::BTreeMap;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

use age::secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::home::ClaudeHome;
use super::secrets::Secret;
use super::transfer::{self, Created, Manifest, Mode, Progress, What};

/// The set format.
pub const SET_VERSION: u32 = 1;
/// Configuration bundles kept per set.
pub const CONFIG_KEEP: usize = 5;
/// A read of the set's folder that makes no progress for this long fails
/// that archive (a file the cloud provider never downloads).
const STALL: Duration = Duration::from_secs(60);
/// The default of Claude Code's `cleanupPeriodDays`.
pub const DEFAULT_CLEANUP_DAYS: i64 = 30;

/// The directories a session's own files live in (archived per session, so
/// never in the configuration bundle).
const SESSION_DIRS: [&str; 4] = ["projects", "file-history", "session-env", "todos"];

/// Never backed up (D6, plus what this machine showed to be runtime state).
const EXCLUDED_DIRS: [&str; 16] = [
    "sessions",
    "jobs",
    "daemon",
    "cache",
    "debug",
    "telemetry",
    "statsig",
    "paste-cache",
    "shell-snapshots",
    "backups",
    "ide",
    "tasks",
    "plugins/cache",
    "plugins/marketplaces",
    "plugins/synced",
    "skills/synced",
];

fn excluded_file(name: &str) -> bool {
    name == ".credentials.json"
        || name == "stats-cache.json"
        || name == ".DS_Store"
        || name.ends_with(".lock")
        || name.starts_with(".last-")
        || name.ends_with(".log")
        || name.ends_with(".tmp")
        || (name.ends_with(".json") && name.contains("cache"))
}

/// Why a backup, restore or pull failed.
#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("no backup folder is set")]
    NoFolder,
    #[error("no backup passphrase is set")]
    NoPassphrase,
    #[error("the passphrase does not open this backup set")]
    WrongPassphrase,
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Io(String),
    #[error(transparent)]
    Transfer(#[from] transfer::TransferError),
}

impl BackupError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoFolder => "no_backup_folder",
            Self::NoPassphrase => "no_backup_passphrase",
            Self::WrongPassphrase => "wrong_passphrase",
            Self::Invalid(_) => "invalid_backup",
            Self::Io(_) => "backup_io",
            Self::Transfer(e) => e.code(),
        }
    }
}

fn io(what: &str, path: &Path, e: impl std::fmt::Display) -> BackupError {
    BackupError::Io(format!("{what} {}: {e}", path.display()))
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64)
}

/// This machine's name, for `set.json` and the index.
pub fn host_name() -> String {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        // SAFETY: gethostname writes at most `buf.len()` bytes into `buf`.
        let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
        if rc == 0 {
            let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
            let name = String::from_utf8_lossy(&buf[..end]).trim().to_string();
            if !name.is_empty() {
                return name.trim_end_matches(".local").to_string();
            }
        }
    }
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "this-machine".to_string())
}

// ──────────────────────────────── the set ────────────────────────────────

/// One account's backup set.
#[derive(Debug, Clone)]
pub struct Set {
    pub dir: PathBuf,
}

impl Set {
    pub fn new(folder: &Path, account_name: &str) -> Self {
        Self {
            dir: folder.join(account_name),
        }
    }

    fn info_path(&self) -> PathBuf {
        self.dir.join("set.json")
    }
    fn identity_path(&self) -> PathBuf {
        self.dir.join("identity.age")
    }
    fn index_path(&self) -> PathBuf {
        self.dir.join("index.json.age")
    }
    fn sessions_dir(&self) -> PathBuf {
        self.dir.join("sessions")
    }
    fn config_dir(&self) -> PathBuf {
        self.dir.join("config")
    }
    /// `sessions/<id>.tar.gz.age`.
    pub fn archive(&self, id: &str) -> PathBuf {
        self.sessions_dir().join(format!("{id}.tar.gz.age"))
    }

    pub fn exists(&self) -> bool {
        self.identity_path().is_file()
    }
}

/// A machine that pushed to the set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Machine {
    pub host: String,
    #[serde(default)]
    pub last_push: Option<i64>,
}

/// `set.json` (the set's only plaintext: names and times).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetInfo {
    pub version: u32,
    pub account: String,
    pub created_at: i64,
    /// The set's public key (`age1…`): what archives are encrypted to.
    pub recipient: String,
    #[serde(default)]
    pub machines: Vec<Machine>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

/// One session in the set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexEntry {
    pub id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    /// Size of its transcript(s) when archived.
    pub transcript_bytes: u64,
    /// Its newest file's mtime (epoch ms) when archived.
    pub newest_mtime: i64,
    /// The machine that pushed it.
    pub host: String,
    pub pushed_at: i64,
    pub archive_sha256: String,
    pub archive_bytes: u64,
    pub files: u32,
}

/// One configuration bundle in the set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigEntry {
    /// `config/<name>`.
    pub name: String,
    pub created_at: i64,
    pub host: String,
    pub fingerprint: String,
}

/// `index.json.age`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetIndex {
    #[serde(default)]
    pub sessions: BTreeMap<String, IndexEntry>,
    #[serde(default)]
    pub config: Vec<ConfigEntry>,
    /// What each machine last saw of each session (`transcriptBytes`), for
    /// pull's "changed on both sides" rule (§5.11).
    #[serde(default)]
    pub seen: BTreeMap<String, BTreeMap<String, u64>>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

/// The set's key pair.
pub struct Keys {
    identity: age::x25519::Identity,
    recipient: age::x25519::Recipient,
}

fn secret(p: &Secret) -> SecretString {
    SecretString::from(p.expose().to_string())
}

/// Encrypts with the passphrase (scrypt at the work factor that takes about
/// a second here; tests use a small one).
fn passphrase_encryptor(p: &Secret) -> age::Encryptor {
    #[allow(unused_mut)]
    let mut recipient = age::scrypt::Recipient::new(secret(p));
    #[cfg(test)]
    recipient.set_work_factor(10);
    age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
        .expect("a single passphrase recipient is always valid")
}

/// Write `bytes` to `path` atomically: a temp name in the same directory,
/// fsync, rename.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), BackupError> {
    let dir = path
        .parent()
        .ok_or_else(|| BackupError::Invalid(format!("{} has no parent", path.display())))?;
    std::fs::create_dir_all(dir).map_err(|e| io("creating", dir, e))?;
    let tmp = temp_beside(path);
    let result = (|| -> Result<(), BackupError> {
        let mut f = std::fs::File::create(&tmp).map_err(|e| io("writing", &tmp, e))?;
        f.write_all(bytes).map_err(|e| io("writing", &tmp, e))?;
        f.sync_all().map_err(|e| io("syncing", &tmp, e))?;
        std::fs::rename(&tmp, path).map_err(|e| io("placing", path, e))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn temp_beside(path: &Path) -> PathBuf {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    path.with_file_name(format!(".{name}.mc-{}.tmp", uuid::Uuid::new_v4().simple()))
}

/// Encrypt `plain` to the set's key.
fn seal(recipient: &age::x25519::Recipient, plain: &[u8]) -> Result<Vec<u8>, BackupError> {
    let enc = age::Encryptor::with_recipients(std::iter::once(recipient as &dyn age::Recipient))
        .map_err(|e| BackupError::Io(e.to_string()))?;
    let mut out = Vec::new();
    let mut w = enc
        .wrap_output(&mut out)
        .map_err(|e| BackupError::Io(e.to_string()))?;
    w.write_all(plain)
        .map_err(|e| BackupError::Io(e.to_string()))?;
    w.finish().map_err(|e| BackupError::Io(e.to_string()))?;
    Ok(out)
}

/// Decrypt with the set's key.
fn open_sealed(identity: &age::x25519::Identity, bytes: &[u8]) -> Result<Vec<u8>, BackupError> {
    let dec = age::Decryptor::new_buffered(bytes).map_err(|e| BackupError::Io(e.to_string()))?;
    let mut r = dec
        .decrypt(std::iter::once(identity as &dyn age::Identity))
        .map_err(|e| BackupError::Io(format!("could not decrypt: {e}")))?;
    let mut out = Vec::new();
    r.read_to_end(&mut out)
        .map_err(|e| BackupError::Io(e.to_string()))?;
    Ok(out)
}

/// Open a set's key with the passphrase, or make a new set.
pub fn open_or_create(
    set: &Set,
    account_name: &str,
    passphrase: &Secret,
) -> Result<(Keys, SetInfo), BackupError> {
    if set.exists() {
        let keys = open_keys(set, passphrase)?;
        let info = read_info(set)?;
        return Ok((keys, info));
    }
    std::fs::create_dir_all(set.sessions_dir()).map_err(|e| io("creating", &set.dir, e))?;
    std::fs::create_dir_all(set.config_dir()).map_err(|e| io("creating", &set.dir, e))?;
    let identity = age::x25519::Identity::generate();
    let recipient = identity.to_public();
    let file = format!(
        "# created: {}\n# public key: {recipient}\n{}\n",
        chrono::Utc::now().to_rfc3339(),
        identity.to_string().expose_secret()
    );
    let enc = passphrase_encryptor(passphrase);
    let mut sealed = Vec::new();
    let mut w = enc
        .wrap_output(&mut sealed)
        .map_err(|e| BackupError::Io(e.to_string()))?;
    w.write_all(file.as_bytes())
        .map_err(|e| BackupError::Io(e.to_string()))?;
    w.finish().map_err(|e| BackupError::Io(e.to_string()))?;
    write_atomic(&set.identity_path(), &sealed)?;
    let info = SetInfo {
        version: SET_VERSION,
        account: account_name.to_string(),
        created_at: now_ms(),
        recipient: recipient.to_string(),
        machines: Vec::new(),
        extra: BTreeMap::new(),
    };
    write_info(set, &info)?;
    write_index(set, &recipient, &SetIndex::default())?;
    Ok((
        Keys {
            identity,
            recipient,
        },
        info,
    ))
}

/// The set's key, decrypted with the passphrase.
pub fn open_keys(set: &Set, passphrase: &Secret) -> Result<Keys, BackupError> {
    let sealed =
        std::fs::read(set.identity_path()).map_err(|e| io("reading", &set.identity_path(), e))?;
    let dec = age::Decryptor::new_buffered(&sealed[..])
        .map_err(|e| BackupError::Invalid(format!("identity.age is not an age file: {e}")))?;
    let mut id = age::scrypt::Identity::new(secret(passphrase));
    // Never more than ~16 s of scrypt for a file someone else could have put
    // in the folder.
    id.set_max_work_factor(22);
    let mut r = dec
        .decrypt(std::iter::once(&id as &dyn age::Identity))
        .map_err(|_| BackupError::WrongPassphrase)?;
    let mut text = String::new();
    r.read_to_string(&mut text)
        .map_err(|_| BackupError::WrongPassphrase)?;
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("AGE-SECRET-KEY-"))
        .ok_or_else(|| BackupError::Invalid("identity.age holds no key".into()))?;
    let identity: age::x25519::Identity = line
        .parse()
        .map_err(|_| BackupError::Invalid("identity.age holds no valid key".into()))?;
    let recipient = identity.to_public();
    Ok(Keys {
        identity,
        recipient,
    })
}

/// Re-encrypt the set's key with a new passphrase (the archives stay).
pub fn rekey(set: &Set, old: &Secret, new: &Secret) -> Result<(), BackupError> {
    let keys = open_keys(set, old)?;
    let file = format!(
        "# created: {}\n# public key: {}\n{}\n",
        chrono::Utc::now().to_rfc3339(),
        keys.recipient,
        keys.identity.to_string().expose_secret()
    );
    let enc = passphrase_encryptor(new);
    let mut sealed = Vec::new();
    let mut w = enc
        .wrap_output(&mut sealed)
        .map_err(|e| BackupError::Io(e.to_string()))?;
    w.write_all(file.as_bytes())
        .map_err(|e| BackupError::Io(e.to_string()))?;
    w.finish().map_err(|e| BackupError::Io(e.to_string()))?;
    write_atomic(&set.identity_path(), &sealed)
}

pub fn read_info(set: &Set) -> Result<SetInfo, BackupError> {
    let text =
        std::fs::read_to_string(set.info_path()).map_err(|e| io("reading", &set.info_path(), e))?;
    serde_json::from_str(&text).map_err(|e| BackupError::Invalid(format!("set.json: {e}")))
}

fn write_info(set: &Set, info: &SetInfo) -> Result<(), BackupError> {
    let bytes = serde_json::to_vec_pretty(info).map_err(|e| BackupError::Io(e.to_string()))?;
    write_atomic(&set.info_path(), &bytes)
}

/// The set's index (empty when there is none yet).
pub fn read_index(set: &Set, keys: &Keys) -> Result<SetIndex, BackupError> {
    let sealed = match read_stalling(&set.index_path()) {
        Ok(b) => b,
        Err(e) if !set.index_path().exists() => {
            let _ = e;
            return Ok(SetIndex::default());
        }
        Err(e) => return Err(e),
    };
    let plain = open_sealed(&keys.identity, &sealed)?;
    serde_json::from_slice(&plain).map_err(|e| BackupError::Invalid(format!("the index: {e}")))
}

fn write_index(
    set: &Set,
    recipient: &age::x25519::Recipient,
    index: &SetIndex,
) -> Result<(), BackupError> {
    let plain = serde_json::to_vec(index).map_err(|e| BackupError::Io(e.to_string()))?;
    write_atomic(&set.index_path(), &seal(recipient, &plain)?)
}

/// Write the index merged with what is in the set now: another machine may
/// have pushed since this one read it. Per session the newer push wins;
/// configuration bundles are joined; each machine's `seen` is its own.
fn write_index_merged(
    set: &Set,
    keys: &Keys,
    mine: &SetIndex,
    host: &str,
) -> Result<(), BackupError> {
    let theirs = read_index(set, keys).unwrap_or_default();
    let mut merged = theirs.clone();
    for (id, e) in &mine.sessions {
        match merged.sessions.get(id) {
            Some(t) if t.pushed_at > e.pushed_at => {}
            _ => {
                merged.sessions.insert(id.clone(), e.clone());
            }
        }
    }
    for c in &mine.config {
        if !merged.config.iter().any(|t| t.name == c.name) {
            merged.config.push(c.clone());
        }
    }
    merged.config.sort_by(|a, b| a.name.cmp(&b.name));
    while merged.config.len() > CONFIG_KEEP {
        let old = merged.config.remove(0);
        let _ = std::fs::remove_file(set.config_dir().join(&old.name));
    }
    if let Some(seen) = mine.seen.get(host) {
        merged.seen.insert(host.to_string(), seen.clone());
    }
    write_index(set, &keys.recipient, &merged)
}

/// Read a file of the set, failing when the read makes no progress for
/// [`STALL`] (a cloud placeholder that never downloads).
fn read_stalling(path: &Path) -> Result<Vec<u8>, BackupError> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<Option<Vec<u8>>, String>>();
    let p = path.to_path_buf();
    std::thread::spawn(move || {
        let mut f = match std::fs::File::open(&p) {
            Ok(f) => f,
            Err(e) => {
                let _ = tx.send(Err(e.to_string()));
                return;
            }
        };
        let mut buf = vec![0u8; 1024 * 1024];
        loop {
            match f.read(&mut buf) {
                Ok(0) => {
                    let _ = tx.send(Ok(None));
                    return;
                }
                Ok(n) => {
                    if tx.send(Ok(Some(buf[..n].to_vec()))).is_err() {
                        return;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                    return;
                }
            }
        }
    });
    let mut out = Vec::new();
    loop {
        match rx.recv_timeout(STALL) {
            Ok(Ok(Some(chunk))) => out.extend_from_slice(&chunk),
            Ok(Ok(None)) => return Ok(out),
            Ok(Err(e)) => return Err(io("reading", path, e)),
            Err(_) => {
                return Err(io(
                    "reading",
                    path,
                    "no progress for a minute (is the cloud folder downloading it?)",
                ))
            }
        }
    }
}

// ─────────────────────────────── pushing ────────────────────────────────

/// What one push did.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushReport {
    pub archived: Vec<String>,
    pub unchanged: usize,
    pub skipped_live: Vec<String>,
    pub failed: Vec<(String, String)>,
    /// The configuration bundle written, when it changed.
    pub config: Option<String>,
    pub bytes: u64,
}

/// What a push needs.
pub struct PushInputs<'a> {
    pub home: &'a ClaudeHome,
    pub set: &'a Set,
    pub account_name: &'a str,
    pub passphrase: &'a Secret,
    pub host: &'a str,
    pub is_live: &'a dyn Fn(&str) -> bool,
    pub progress: &'a Progress,
}

/// Every session id with a transcript in a config directory.
pub fn session_ids(home: &ClaudeHome) -> Vec<String> {
    let mut ids: Vec<String> = std::fs::read_dir(home.projects_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|d| d.path().is_dir())
        .flat_map(|d| std::fs::read_dir(d.path()).into_iter().flatten().flatten())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let id = name.strip_suffix(".jsonl")?.to_string();
            super::ids::is_session_id(&id).then_some(id)
        })
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// A session's files and the two numbers that decide whether it changed.
struct SessionNow {
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
    cwd: Option<String>,
    transcript_bytes: u64,
    newest_mtime: i64,
}

fn session_now(home: &ClaudeHome, id: &str) -> Result<SessionNow, BackupError> {
    let plan = transfer::plan_session(home, home, id)?;
    let mut transcript_bytes = 0u64;
    let mut newest = 0i64;
    for f in &plan.files {
        let meta = std::fs::metadata(&f.from).map_err(|e| io("reading", &f.from, e))?;
        newest = newest.max(mtime_ms(&meta));
        if f.from.extension().is_some_and(|e| e == "jsonl")
            && f.from.file_stem().and_then(|s| s.to_str()) == Some(id)
            && f.from
                .parent()
                .and_then(|p| p.parent())
                .is_some_and(|p| p == home.projects_dir())
        {
            transcript_bytes += meta.len();
        }
    }
    Ok(SessionNow {
        files: plan.files.iter().map(|f| f.from.clone()).collect(),
        dirs: plan.dirs.iter().map(|(d, _)| d.clone()).collect(),
        cwd: plan.cwd,
        transcript_bytes,
        newest_mtime: newest,
    })
}

/// Counts bytes and hashes them on the way to the file.
struct HashingWriter<W: Write> {
    inner: W,
    hasher: sha2::Sha256,
    bytes: u64,
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        use sha2::Digest;
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        self.bytes += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Write one archive: tar (paths relative to `base`) → gzip → age to the
/// set's key → a temp file in the set → fsync → rename. Returns its SHA-256
/// and size.
fn write_archive(
    dest: &Path,
    recipient: &age::x25519::Recipient,
    base: &Path,
    files: &[(PathBuf, PathBuf)],
    dirs: &[PathBuf],
    extra: &[(&str, Vec<u8>)],
) -> Result<(String, u64), BackupError> {
    use sha2::Digest;
    let dir = dest
        .parent()
        .ok_or_else(|| BackupError::Invalid("an archive needs a folder".into()))?;
    std::fs::create_dir_all(dir).map_err(|e| io("creating", dir, e))?;
    let tmp = temp_beside(dest);
    let result = (|| -> Result<(String, u64), BackupError> {
        let file = std::fs::File::create(&tmp).map_err(|e| io("writing", &tmp, e))?;
        let hashing = HashingWriter {
            inner: BufWriter::new(file),
            hasher: sha2::Sha256::new(),
            bytes: 0,
        };
        let enc =
            age::Encryptor::with_recipients(std::iter::once(recipient as &dyn age::Recipient))
                .map_err(|e| BackupError::Io(e.to_string()))?;
        let sealed = enc
            .wrap_output(hashing)
            .map_err(|e| io("encrypting", dest, e))?;
        let gz = flate2::write::GzEncoder::new(sealed, flate2::Compression::default());
        let mut tar = tar::Builder::new(gz);
        tar.follow_symlinks(false);
        for d in dirs {
            let rel = d.strip_prefix(base).unwrap_or(d);
            tar.append_dir(rel, d).map_err(|e| io("archiving", d, e))?;
        }
        for (path, name) in files {
            tar.append_path_with_name(path, name)
                .map_err(|e| io("archiving", path, e))?;
        }
        for (name, data) in extra {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o600);
            header.set_mtime((now_ms() / 1000) as u64);
            header.set_cksum();
            tar.append_data(&mut header, name, &data[..])
                .map_err(|e| io("archiving", Path::new(name), e))?;
        }
        let gz = tar.into_inner().map_err(|e| io("archiving", dest, e))?;
        let sealed = gz.finish().map_err(|e| io("compressing", dest, e))?;
        let hashing = sealed.finish().map_err(|e| io("encrypting", dest, e))?;
        let sha: String = hashing
            .hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let bytes = hashing.bytes;
        let file = hashing
            .inner
            .into_inner()
            .map_err(|e| io("writing", &tmp, e.error()))?;
        file.sync_all().map_err(|e| io("syncing", &tmp, e))?;
        std::fs::rename(&tmp, dest).map_err(|e| io("placing", dest, e))?;
        Ok((sha, bytes))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// The configuration files of a config directory: everything that is not a
/// session's, not excluded (D6) and not a symlink.
fn config_files(home: &ClaudeHome) -> Vec<(PathBuf, PathBuf)> {
    fn visit(base: &Path, dir: &Path, out: &mut Vec<(PathBuf, PathBuf)>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = e.path();
            let Ok(rel) = path.strip_prefix(base) else {
                continue;
            };
            let rel_s = rel.to_string_lossy().replace('\\', "/");
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                let top = rel_s.split('/').next().unwrap_or_default();
                if SESSION_DIRS.contains(&top) || EXCLUDED_DIRS.contains(&rel_s.as_str()) {
                    continue;
                }
                visit(base, &path, out);
            } else if ft.is_file() {
                let name = e.file_name().to_string_lossy().into_owned();
                if !excluded_file(&name) {
                    out.push((path.clone(), rel.to_path_buf()));
                }
            }
        }
    }
    let mut out = Vec::new();
    visit(home.base(), home.base(), &mut out);
    // The Default's global file lives beside the directory.
    let global = home.claude_json();
    if !global.starts_with(home.base()) && global.is_file() {
        out.push((global, PathBuf::from(".claude.json")));
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

fn config_fingerprint(files: &[(PathBuf, PathBuf)]) -> String {
    let mut text = String::new();
    for (path, rel) in files {
        let (size, mtime) = std::fs::metadata(path)
            .map(|m| (m.len(), mtime_ms(&m)))
            .unwrap_or((0, 0));
        text.push_str(&format!("{}\t{size}\t{mtime}\n", rel.display()));
    }
    super::secrets::sha256_hex(text.as_bytes())
}

/// Back up one account's directory into its set. Blocking.
pub fn push(inp: &PushInputs<'_>) -> Result<PushReport, BackupError> {
    let (keys, mut info) = open_or_create(inp.set, inp.account_name, inp.passphrase)?;
    let mut index = read_index(inp.set, &keys)?;
    let mut report = PushReport::default();
    let base = inp.home.base();
    for id in session_ids(inp.home) {
        if (inp.is_live)(&id) {
            report.skipped_live.push(id);
            continue;
        }
        let now = match session_now(inp.home, &id) {
            Ok(n) => n,
            Err(e) => {
                report.failed.push((id, e.to_string()));
                continue;
            }
        };
        let unchanged = index.sessions.get(&id).is_some_and(|e| {
            e.transcript_bytes == now.transcript_bytes && e.newest_mtime == now.newest_mtime
        }) && inp.set.archive(&id).is_file();
        if unchanged {
            report.unchanged += 1;
            continue;
        }
        let files: Vec<(PathBuf, PathBuf)> = now
            .files
            .iter()
            .map(|f| (f.clone(), f.strip_prefix(base).unwrap_or(f).to_path_buf()))
            .collect();
        let meta = serde_json::json!({
            "version": SET_VERSION,
            "id": id,
            "cwd": now.cwd,
            "projectEntry": now.cwd.as_deref()
                .and_then(|c| transfer::project_entry(&inp.home.claude_json(), c)),
        });
        let meta = serde_json::to_vec_pretty(&meta).unwrap_or_default();
        match write_archive(
            &inp.set.archive(&id),
            &keys.recipient,
            base,
            &files,
            &now.dirs,
            &[(".mc/meta.json", meta)],
        ) {
            Ok((sha, bytes)) => {
                inp.progress.files_done.fetch_add(1, Ordering::Relaxed);
                inp.progress.bytes_done.fetch_add(bytes, Ordering::Relaxed);
                report.bytes += bytes;
                index.sessions.insert(
                    id.clone(),
                    IndexEntry {
                        id: id.clone(),
                        cwd: now.cwd.clone(),
                        transcript_bytes: now.transcript_bytes,
                        newest_mtime: now.newest_mtime,
                        host: inp.host.to_string(),
                        pushed_at: now_ms(),
                        archive_sha256: sha,
                        archive_bytes: bytes,
                        files: files.len() as u32,
                    },
                );
                index
                    .seen
                    .entry(inp.host.to_string())
                    .or_default()
                    .insert(id.clone(), now.transcript_bytes);
                report.archived.push(id);
            }
            Err(e) => report.failed.push((id, e.to_string())),
        }
    }
    // The configuration bundle, when any of its files changed.
    let files = config_files(inp.home);
    let fingerprint = config_fingerprint(&files);
    if index.config.last().map(|c| c.fingerprint.as_str()) != Some(fingerprint.as_str()) {
        let name = format!(
            "{}.tar.gz.age",
            chrono::Utc::now().format("%Y-%m-%dT%H-%M-%SZ")
        );
        let dest = inp.set.config_dir().join(&name);
        write_archive(&dest, &keys.recipient, base, &files, &[], &[])?;
        index.config.push(ConfigEntry {
            name: name.clone(),
            created_at: now_ms(),
            host: inp.host.to_string(),
            fingerprint,
        });
        while index.config.len() > CONFIG_KEEP {
            let old = index.config.remove(0);
            let _ = std::fs::remove_file(inp.set.config_dir().join(&old.name));
        }
        report.config = Some(name);
    }
    write_index_merged(inp.set, &keys, &index, inp.host)?;
    match info.machines.iter_mut().find(|m| m.host == inp.host) {
        Some(m) => m.last_push = Some(now_ms()),
        None => info.machines.push(Machine {
            host: inp.host.to_string(),
            last_push: Some(now_ms()),
        }),
    }
    write_info(inp.set, &info)?;
    Ok(report)
}

// ─────────────────────────────── restoring ───────────────────────────────

/// A session restore did not bring back.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skip {
    pub id: String,
    pub code: String,
    pub reason: String,
}

/// A note about a restored session.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Warning {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    pub restored: Vec<String>,
    pub skipped: Vec<Skip>,
    pub warnings: Vec<Warning>,
}

/// What a restore needs.
pub struct RestoreInputs<'a> {
    pub set: &'a Set,
    pub passphrase: &'a Secret,
    pub target: &'a ClaudeHome,
    pub ids: &'a [String],
    /// Bring the project's trust (its `.claude.json` entry) back too.
    pub trust: bool,
    pub is_live: &'a dyn Fn(&str) -> bool,
    pub undo_root: &'a Path,
    pub job_id: &'a str,
    pub progress: &'a Progress,
    /// `restore` or `pull` (the job's kind).
    pub kind: &'a str,
}

/// The target's `cleanupPeriodDays` (user settings; default 30).
pub fn cleanup_days(home: &ClaudeHome) -> i64 {
    std::fs::read_to_string(home.user_settings())
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("cleanupPeriodDays").and_then(Value::as_i64))
        .filter(|d| *d > 0)
        .unwrap_or(DEFAULT_CLEANUP_DAYS)
}

/// The §11.7 warning, when Claude Code would prune the session again.
pub fn prune_warning(newest_mtime: i64, days: i64, now: i64) -> Option<String> {
    (newest_mtime < now - days * 24 * 3600 * 1000).then(|| {
        format!(
            "Claude Code will delete this again at its next cleanup unless \
             cleanupPeriodDays (now {days}) is raised."
        )
    })
}

/// Whether an archive entry may land in a session's files: relative, plain
/// components, under a session directory, naming this session.
fn allowed_entry(rel: &Path, id: &str) -> bool {
    let parts: Vec<&str> = match rel
        .components()
        .map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
    {
        Some(p) if !p.is_empty() => p,
        _ => return false,
    };
    match parts.as_slice() {
        ["projects", _enc, name] => *name == format!("{id}.jsonl") || *name == id,
        ["projects", _enc, dir, ..] => *dir == id,
        ["file-history", dir, ..] | ["session-env", dir, ..] => *dir == id,
        ["todos", name] => name.starts_with(id) && name.ends_with(".json"),
        _ => false,
    }
}

/// Unpack one session archive into `stage` (validated), returning its meta.
fn unpack(
    archive: &[u8],
    identity: &age::x25519::Identity,
    stage: &Path,
    id: &str,
) -> Result<Value, BackupError> {
    let dec = age::Decryptor::new_buffered(archive).map_err(|e| BackupError::Io(e.to_string()))?;
    let r = dec
        .decrypt(std::iter::once(identity as &dyn age::Identity))
        .map_err(|e| BackupError::Io(format!("could not decrypt: {e}")))?;
    let gz = flate2::read::GzDecoder::new(BufReader::new(r));
    let mut tar = tar::Archive::new(gz);
    tar.set_preserve_mtime(true);
    tar.set_preserve_permissions(false);
    tar.set_overwrite(false);
    let mut meta = Value::Null;
    for entry in tar.entries().map_err(|e| BackupError::Io(e.to_string()))? {
        let mut entry = entry.map_err(|e| BackupError::Io(e.to_string()))?;
        let rel = entry
            .path()
            .map_err(|e| BackupError::Io(e.to_string()))?
            .into_owned();
        if rel == Path::new(".mc/meta.json") {
            let mut text = String::new();
            entry
                .read_to_string(&mut text)
                .map_err(|e| BackupError::Io(e.to_string()))?;
            meta = serde_json::from_str(&text).unwrap_or(Value::Null);
            continue;
        }
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir()) || !allowed_entry(&rel, id) {
            return Err(BackupError::Invalid(format!(
                "the archive of {id} holds {} — not one of its own files",
                rel.display()
            )));
        }
        let unpacked = entry
            .unpack_in(stage)
            .map_err(|e| BackupError::Io(e.to_string()))?;
        if !unpacked {
            return Err(BackupError::Invalid(format!(
                "the archive of {id} holds an unsafe path"
            )));
        }
    }
    Ok(meta)
}

/// Every regular file and directory under `stage`, relative to it.
fn staged(stage: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    fn visit(base: &Path, dir: &Path, files: &mut Vec<PathBuf>, dirs: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = e.path();
            let rel = path.strip_prefix(base).unwrap_or(&path).to_path_buf();
            match e.file_type() {
                Ok(ft) if ft.is_dir() => {
                    dirs.push(rel);
                    visit(base, &path, files, dirs);
                }
                Ok(ft) if ft.is_file() => files.push(rel),
                _ => {}
            }
        }
    }
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    visit(stage, stage, &mut files, &mut dirs);
    (files, dirs)
}

/// Bring sessions back from a set into a config directory. Only new files
/// are written; an undo manifest is kept like a transfer's. Blocking.
pub fn restore(inp: &RestoreInputs<'_>) -> Result<(RestoreReport, Manifest), BackupError> {
    let keys = open_keys(inp.set, inp.passphrase)?;
    let index = read_index(inp.set, &keys)?;
    let now = now_ms();
    let days = cleanup_days(inp.target);
    let undo_dir = inp.undo_root.join(inp.job_id);
    std::fs::create_dir_all(&undo_dir).map_err(|e| io("creating", &undo_dir, e))?;
    transfer::private_dir(inp.undo_root);
    transfer::private_dir(&undo_dir);
    let mut m = Manifest {
        job_id: inp.job_id.to_string(),
        created_at: now,
        mode: Mode::Copy,
        what: What::Session {
            id: inp.ids.first().cloned().unwrap_or_default(),
        },
        from_dir: inp.set.dir.clone(),
        to_dir: inp.target.base().to_path_buf(),
        from_config: None,
        to_config: Some(inp.target.claude_json()),
        sessions: Vec::new(),
        created: Vec::new(),
        created_dirs: Vec::new(),
        moved: Vec::new(),
        moved_dirs: Vec::new(),
        added_entry: None,
        added_entries: Vec::new(),
        removed_entry: None,
        added_history: Vec::new(),
        removed_history: Vec::new(),
        undone_at: None,
        kind: Some(inp.kind.to_string()),
    };
    let mut report = RestoreReport::default();
    let target_busy = transfer::in_use(inp.target);
    let stage_root = inp
        .target
        .base()
        .join(format!(".mc-restore-{}", inp.job_id));
    for id in inp.ids {
        let skip = |code: &str, reason: String| Skip {
            id: id.clone(),
            code: code.to_string(),
            reason,
        };
        let Some(entry) = index.sessions.get(id) else {
            report
                .skipped
                .push(skip("not_in_set", "not in this backup set".into()));
            continue;
        };
        if (inp.is_live)(id) {
            report
                .skipped
                .push(skip("session_live", "it is running here".into()));
            continue;
        }
        if !transfer::transcripts_of(inp.target, id).is_empty() {
            report
                .skipped
                .push(skip("already_exists", "this account has it already".into()));
            continue;
        }
        let (files_before, dirs_before) = (m.created.len(), m.created_dirs.len());
        let one = (|| -> Result<Value, BackupError> {
            let bytes = read_stalling(&inp.set.archive(id))?;
            if super::secrets::sha256_hex(&bytes) != entry.archive_sha256 {
                return Err(BackupError::Invalid(format!(
                    "the archive of {id} is not the one the index names (truncated or changed)"
                )));
            }
            let stage = stage_root.join(id);
            std::fs::create_dir_all(&stage).map_err(|e| io("creating", &stage, e))?;
            let meta = unpack(&bytes, &keys.identity, &stage, id)?;
            let (files, dirs) = staged(&stage);
            // Nothing may be in the way.
            for rel in &files {
                if inp.target.base().join(rel).exists() {
                    return Err(BackupError::Transfer(
                        transfer::TransferError::AlreadyExists(rel.display().to_string()),
                    ));
                }
            }
            for rel in &dirs {
                for d in transfer::missing_dirs(&inp.target.base().join(rel), inp.target.base()) {
                    std::fs::create_dir(&d).map_err(|e| io("creating", &d, e))?;
                    m.created_dirs.push(d);
                }
            }
            for rel in &files {
                let dest = inp.target.base().join(rel);
                if let Some(parent) = dest.parent() {
                    for d in transfer::missing_dirs(parent, inp.target.base()) {
                        std::fs::create_dir(&d).map_err(|e| io("creating", &d, e))?;
                        m.created_dirs.push(d);
                    }
                }
                std::fs::rename(stage.join(rel), &dest).map_err(|e| io("placing", &dest, e))?;
                let (size, sha) =
                    transfer::sha256_file(&dest).map_err(|e| io("verifying", &dest, e))?;
                inp.progress.bytes_done.fetch_add(size, Ordering::Relaxed);
                m.created.push(Created {
                    path: dest,
                    size,
                    sha256: sha,
                });
            }
            inp.progress.files_done.fetch_add(1, Ordering::Relaxed);
            Ok(meta)
        })();
        match one {
            Ok(meta) => {
                m.sessions.push(id.clone());
                report.restored.push(id.clone());
                if let Some(w) = prune_warning(entry.newest_mtime, days, now) {
                    report.warnings.push(Warning {
                        id: id.clone(),
                        text: w,
                    });
                }
                // Trust only with the opt-in, never over an entry, never
                // while the directory is in use.
                if inp.trust {
                    let cwd = meta.get("cwd").and_then(Value::as_str);
                    let entry = meta.get("projectEntry").filter(|e| e.is_object());
                    if let (Some(cwd), Some(project)) = (cwd, entry) {
                        if let Some(busy) = &target_busy {
                            report.warnings.push(Warning {
                                id: id.clone(),
                                text: format!("The folder's trust was not restored: {busy}"),
                            });
                        } else if transfer::project_entry(&inp.target.claude_json(), cwd).is_none()
                        {
                            transfer::edit_projects(&inp.target.claude_json(), |p| {
                                p.entry(cwd.to_string()).or_insert_with(|| project.clone());
                            })?;
                            m.added_entries.push((cwd.to_string(), project.clone()));
                        }
                    }
                }
            }
            Err(e) => {
                // Nothing of a session that did not come back whole stays.
                for c in m.created.drain(files_before..) {
                    let _ = std::fs::remove_file(&c.path);
                }
                for d in m.created_dirs.drain(dirs_before..).rev() {
                    let _ = std::fs::remove_dir(&d);
                }
                report.skipped.push(skip(e.code(), e.to_string()));
            }
        }
        crate::persist::write_json_private(&undo_dir.join("manifest.json"), &m)
            .map_err(|e| io("writing", &undo_dir, e))?;
    }
    let _ = std::fs::remove_dir_all(&stage_root);
    Ok((report, m))
}

// ──────────────────────────────── pulling ────────────────────────────────

/// What a pull would do with one session of the set (§5.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PullAction {
    /// Not on this machine: restored.
    New,
    /// Newer in the set, unchanged here since: the local copy is replaced
    /// (the old one kept for undo).
    Update,
    /// Changed on both sides: the set's transcript lands beside the local
    /// one as `<id>.conflict-<host>-<time>.jsonl`, never merged.
    Conflict,
    /// The same here and in the set.
    Same,
    /// Newer here: the next push sends it.
    LocalNewer,
    /// Running here: never touched.
    Live,
}

/// One session of a pull plan.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullItem {
    pub id: String,
    pub action: PullAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub remote_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_bytes: Option<u64>,
    pub host: String,
    pub pushed_at: i64,
    pub newest_mtime: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prune_warning: Option<String>,
}

/// What the set holds, compared with this machine's directory.
pub fn compare(
    index: &SetIndex,
    home: &ClaudeHome,
    host: &str,
    is_live: &dyn Fn(&str) -> bool,
    now: i64,
) -> Vec<PullItem> {
    let days = cleanup_days(home);
    let seen = index.seen.get(host);
    let mut out: Vec<PullItem> = index
        .sessions
        .values()
        .map(|e| {
            let local = (!transfer::transcripts_of(home, &e.id).is_empty())
                .then(|| session_now(home, &e.id).ok())
                .flatten();
            let action = match &local {
                _ if is_live(&e.id) => PullAction::Live,
                None => PullAction::New,
                Some(l)
                    if l.transcript_bytes == e.transcript_bytes
                        && l.newest_mtime == e.newest_mtime =>
                {
                    PullAction::Same
                }
                Some(l)
                    if e.transcript_bytes > l.transcript_bytes
                        && e.newest_mtime > l.newest_mtime =>
                {
                    let previous = seen.and_then(|s| s.get(&e.id)).copied();
                    match previous {
                        // Unchanged here since this machine last had it.
                        Some(p) if l.transcript_bytes <= p => PullAction::Update,
                        _ => PullAction::Conflict,
                    }
                }
                Some(_) => PullAction::LocalNewer,
            };
            PullItem {
                id: e.id.clone(),
                action,
                cwd: e.cwd.clone(),
                remote_bytes: e.transcript_bytes,
                local_bytes: local.as_ref().map(|l| l.transcript_bytes),
                host: e.host.clone(),
                pushed_at: e.pushed_at,
                newest_mtime: e.newest_mtime,
                prune_warning: matches!(action, PullAction::New | PullAction::Update)
                    .then(|| prune_warning(e.newest_mtime, days, now))
                    .flatten(),
            }
        })
        .collect();
    out.sort_by_key(|p| std::cmp::Reverse(p.pushed_at));
    out
}

/// What a pull needs.
pub struct PullInputs<'a> {
    pub set: &'a Set,
    pub passphrase: &'a Secret,
    pub home: &'a ClaudeHome,
    pub host: &'a str,
    /// Only these sessions (`None`: every one the plan offers).
    pub ids: Option<&'a [String]>,
    pub trust: bool,
    pub is_live: &'a dyn Fn(&str) -> bool,
    pub undo_root: &'a Path,
    pub job_id: &'a str,
    pub progress: &'a Progress,
}

/// What one pull did.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullReport {
    pub restored: Vec<String>,
    pub updated: Vec<String>,
    /// `(session, the file written beside it)`.
    pub conflicts: Vec<(String, PathBuf)>,
    pub skipped: Vec<Skip>,
    pub warnings: Vec<Warning>,
    /// Prompt-history lines added.
    pub history_lines: usize,
}

/// Bring this machine up to date from the set (§5.11). Blocking.
pub fn pull(inp: &PullInputs<'_>) -> Result<(PullReport, Manifest), BackupError> {
    let keys = open_keys(inp.set, inp.passphrase)?;
    let mut index = read_index(inp.set, &keys)?;
    let now = now_ms();
    let plan = compare(&index, inp.home, inp.host, inp.is_live, now);
    let wanted = |id: &str| inp.ids.is_none_or(|ids| ids.iter().any(|i| i == id));
    let mut report = PullReport::default();

    // New sessions: a restore.
    let new: Vec<String> = plan
        .iter()
        .filter(|p| p.action == PullAction::New && wanted(&p.id))
        .map(|p| p.id.clone())
        .collect();
    let (restored, mut m) = restore(&RestoreInputs {
        set: inp.set,
        passphrase: inp.passphrase,
        target: inp.home,
        ids: &new,
        trust: inp.trust,
        is_live: inp.is_live,
        undo_root: inp.undo_root,
        job_id: inp.job_id,
        progress: inp.progress,
        kind: "pull",
    })?;
    report.restored = restored.restored;
    report.skipped = restored.skipped;
    report.warnings = restored.warnings;
    let undo_dir = inp.undo_root.join(inp.job_id);
    let base = inp.home.base().to_path_buf();

    for item in plan.iter().filter(|p| wanted(&p.id)) {
        match item.action {
            PullAction::Update => {
                let result = (|| -> Result<(), BackupError> {
                    // The local copy goes to the undo directory first.
                    let local = session_now(inp.home, &item.id)?;
                    for f in &local.files {
                        let rel = f.strip_prefix(&base).unwrap_or(f).to_path_buf();
                        let saved = PathBuf::from("moved").join(&rel);
                        let dest = undo_dir.join(&saved);
                        if let Some(parent) = dest.parent() {
                            std::fs::create_dir_all(parent)
                                .map_err(|e| io("creating", parent, e))?;
                        }
                        if std::fs::rename(f, &dest).is_err() {
                            std::fs::copy(f, &dest).map_err(|e| io("saving", f, e))?;
                            std::fs::remove_file(f).map_err(|e| io("removing", f, e))?;
                        }
                        m.moved.push(transfer::Moved {
                            original: f.clone(),
                            saved,
                        });
                    }
                    Ok(())
                })();
                if let Err(e) = result {
                    report.skipped.push(Skip {
                        id: item.id.clone(),
                        code: e.code().to_string(),
                        reason: e.to_string(),
                    });
                    continue;
                }
                let (r, extra) = restore(&RestoreInputs {
                    set: inp.set,
                    passphrase: inp.passphrase,
                    target: inp.home,
                    ids: std::slice::from_ref(&item.id),
                    trust: false,
                    is_live: inp.is_live,
                    undo_root: inp.undo_root,
                    job_id: &format!("{}-u", inp.job_id),
                    progress: inp.progress,
                    kind: "pull",
                })?;
                let _ = std::fs::remove_dir_all(inp.undo_root.join(format!("{}-u", inp.job_id)));
                if r.restored.is_empty() {
                    // Put the local copy back: the set's would not come.
                    for mv in m
                        .moved
                        .iter()
                        .filter(|mv| mv.original.to_string_lossy().contains(&item.id))
                    {
                        let _ = std::fs::rename(undo_dir.join(&mv.saved), &mv.original);
                    }
                    m.moved
                        .retain(|mv| !mv.original.to_string_lossy().contains(&item.id));
                    report.skipped.extend(r.skipped);
                } else {
                    m.created.extend(extra.created);
                    m.created_dirs.extend(extra.created_dirs);
                    m.sessions.push(item.id.clone());
                    report.updated.push(item.id.clone());
                    report.warnings.extend(r.warnings);
                }
            }
            PullAction::Conflict => match conflict_copy(inp, &keys, &index, &item.id, now) {
                Ok(Some(created)) => {
                    report
                        .conflicts
                        .push((item.id.clone(), created.path.clone()));
                    m.created.push(created);
                }
                Ok(None) => {}
                Err(e) => report.skipped.push(Skip {
                    id: item.id.clone(),
                    code: e.code().to_string(),
                    reason: e.to_string(),
                }),
            },
            _ => {}
        }
    }

    // Prompt history from the newest configuration bundle, appended
    // de-duplicated, and only while nothing here is writing it.
    if let Some(lines) = bundle_history(inp.set, &keys, &index)? {
        let have: std::collections::HashSet<(String, String)> =
            std::fs::read_to_string(inp.home.history_file())
                .unwrap_or_default()
                .lines()
                .filter_map(history_key)
                .collect();
        let fresh: Vec<String> = lines
            .into_iter()
            .filter(|l| history_key(l).is_some_and(|k| !have.contains(&k)))
            .collect();
        if !fresh.is_empty() {
            if let Some(busy) = transfer::in_use(inp.home) {
                report.warnings.push(Warning {
                    id: String::new(),
                    text: format!("Prompt history was not merged: {busy}"),
                });
            } else {
                transfer::append_lines(&inp.home.history_file(), &fresh)?;
                report.history_lines = fresh.len();
                m.added_history = fresh;
            }
        }
    }

    // What this machine now has, for the next comparison.
    let seen = index.seen.entry(inp.host.to_string()).or_default();
    for id in report.restored.iter().chain(&report.updated) {
        if let Some(e) = index.sessions.get(id) {
            seen.insert(id.clone(), e.transcript_bytes);
        }
    }
    let snapshot = index.clone();
    write_index_merged(inp.set, &keys, &snapshot, inp.host)?;
    crate::persist::write_json_private(&undo_dir.join("manifest.json"), &m)
        .map_err(|e| io("writing", &undo_dir, e))?;
    Ok((report, m))
}

fn history_key(line: &str) -> Option<(String, String)> {
    let v: Value = serde_json::from_str(line).ok()?;
    Some((
        v.get("timestamp").map(Value::to_string).unwrap_or_default(),
        v.get("display")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    ))
}

/// `history.jsonl` from the newest configuration bundle, if it has one.
fn bundle_history(
    set: &Set,
    keys: &Keys,
    index: &SetIndex,
) -> Result<Option<Vec<String>>, BackupError> {
    let Some(last) = index.config.last() else {
        return Ok(None);
    };
    let path = set.config_dir().join(&last.name);
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = read_stalling(&path)?;
    let dec =
        age::Decryptor::new_buffered(&bytes[..]).map_err(|e| BackupError::Io(e.to_string()))?;
    let r = dec
        .decrypt(std::iter::once(&keys.identity as &dyn age::Identity))
        .map_err(|e| BackupError::Io(format!("could not decrypt: {e}")))?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(BufReader::new(r)));
    for entry in tar.entries().map_err(|e| BackupError::Io(e.to_string()))? {
        let mut entry = entry.map_err(|e| BackupError::Io(e.to_string()))?;
        let is_history = entry
            .path()
            .map(|p| p == Path::new("history.jsonl"))
            .unwrap_or(false);
        if is_history {
            let mut text = String::new();
            entry
                .read_to_string(&mut text)
                .map_err(|e| BackupError::Io(e.to_string()))?;
            return Ok(Some(text.lines().map(str::to_string).collect()));
        }
    }
    Ok(None)
}

/// The set's transcript of a session changed on both sides, written beside
/// the local one as `<id>.conflict-<host>-<time>.jsonl`.
fn conflict_copy(
    inp: &PullInputs<'_>,
    keys: &Keys,
    index: &SetIndex,
    id: &str,
    now: i64,
) -> Result<Option<Created>, BackupError> {
    let Some(entry) = index.sessions.get(id) else {
        return Ok(None);
    };
    let local = transfer::transcripts_of(inp.home, id);
    let Some(local) = local.first() else {
        return Ok(None);
    };
    let bytes = read_stalling(&inp.set.archive(id))?;
    if super::secrets::sha256_hex(&bytes) != entry.archive_sha256 {
        return Err(BackupError::Invalid(format!(
            "the archive of {id} is not the one the index names"
        )));
    }
    let stage = inp
        .home
        .base()
        .join(format!(".mc-restore-{}-c", inp.job_id))
        .join(id);
    std::fs::create_dir_all(&stage).map_err(|e| io("creating", &stage, e))?;
    let result = (|| -> Result<Option<Created>, BackupError> {
        unpack(&bytes, &keys.identity, &stage, id)?;
        let (files, _) = staged(&stage);
        let Some(main) = files
            .iter()
            .find(|f| f.file_name().and_then(|n| n.to_str()) == Some(&format!("{id}.jsonl")))
        else {
            return Ok(None);
        };
        let stamp = chrono::DateTime::from_timestamp_millis(now)
            .unwrap_or_default()
            .format("%Y%m%dT%H%M%S");
        let host: String = entry
            .host
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        let dest = local.with_file_name(format!("{id}.conflict-{host}-{stamp}.jsonl"));
        if dest.exists() {
            return Ok(None);
        }
        std::fs::rename(stage.join(main), &dest).map_err(|e| io("placing", &dest, e))?;
        let (size, sha) = transfer::sha256_file(&dest).map_err(|e| io("verifying", &dest, e))?;
        Ok(Some(Created {
            path: dest,
            size,
            sha256: sha,
        }))
    })();
    let _ = std::fs::remove_dir_all(stage.parent().unwrap_or(&stage));
    result
}

/// Unpack the newest configuration bundle into `into` for the user to look
/// at (it never replaces the live configuration). Returns the folder.
pub fn extract_config(set: &Set, passphrase: &Secret, into: &Path) -> Result<PathBuf, BackupError> {
    let keys = open_keys(set, passphrase)?;
    let index = read_index(set, &keys)?;
    let last = index
        .config
        .last()
        .ok_or_else(|| BackupError::Invalid("this set has no configuration bundle".into()))?;
    let bytes = read_stalling(&set.config_dir().join(&last.name))?;
    let dec =
        age::Decryptor::new_buffered(&bytes[..]).map_err(|e| BackupError::Io(e.to_string()))?;
    let r = dec
        .decrypt(std::iter::once(&keys.identity as &dyn age::Identity))
        .map_err(|e| BackupError::Io(format!("could not decrypt: {e}")))?;
    let dest = into.join(last.name.trim_end_matches(".tar.gz.age"));
    std::fs::create_dir_all(&dest).map_err(|e| io("creating", &dest, e))?;
    transfer::private_dir(&dest);
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(BufReader::new(r)));
    tar.set_overwrite(false);
    for entry in tar.entries().map_err(|e| BackupError::Io(e.to_string()))? {
        let mut entry = entry.map_err(|e| BackupError::Io(e.to_string()))?;
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir()) {
            continue;
        }
        entry
            .unpack_in(&dest)
            .map_err(|e| BackupError::Io(e.to_string()))?;
    }
    Ok(dest)
}

pub mod ops;

#[cfg(test)]
mod tests;
