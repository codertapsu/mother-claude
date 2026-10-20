//! Backups of each account's data to a folder of the user's choosing (iCloud
//! Drive and Google Drive included), restore from them, and pull from a set
//! another machine pushed (docs/ACCOUNTS.md D6, D7, §4.3, §5.10, §5.11).
//!
//! **A set** is `<folder>/<account name>/`:
//!
//! ```text
//! set.json                    plaintext, display only: version, account name, the set's public key, machines
//! identity.age                the set's X25519 key, encrypted with the backup passphrase (scrypt)
//! index.json.age              what is in the set, encrypted to the set's key and authenticated
//! sessions/<id>.tar.gz.age    one archive per session (paths relative to the config directory)
//! config/<time>.tar.gz.age    the configuration bundle, the last five
//! ```
//!
//! Every archive is `tar` + `gzip` + `age` and opens without Mother Claude:
//! `age -d -o key.txt identity.age && age -d -i key.txt sessions/<id>.tar.gz.age
//! | tar xz`. The passphrase's work factor (about a second of scrypt) is paid
//! once per run, for `identity.age`, not once per archive.
//!
//! **Nothing but `identity.age` is trusted on its own.** The folder is shared
//! (a cloud folder) and age's X25519 encryption does not say who encrypted:
//! anyone who can write the folder can encrypt an index or an archive to the
//! public key in `set.json`. So the index's first line is an HMAC-SHA256 of the
//! rest, keyed from the set's secret key (which only the passphrase opens), and
//! an index without a valid one is refused; every archive and bundle is checked
//! against the SHA-256 the authenticated index names before it is decrypted.
//! `set.json`'s public key is never used: archives are encrypted to the key
//! derived from the identity.
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

/// The set format (2: the index is authenticated).
pub const SET_VERSION: u32 = 2;
/// The first word of an index's first line.
const INDEX_MAGIC: &str = "MC-INDEX-2";
/// What the index's HMAC key is derived with (from the set's secret key).
const INDEX_MAC_LABEL: &[u8] = b"mother-claude backup set index v2";
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
    /// The index (or `set.json`'s key) is not what this set's key wrote:
    /// changed by someone without the passphrase, or made by another format.
    #[error("{0}")]
    Tampered(String),
    /// A set is there but its key (`identity.age`) is not: never replaced by
    /// a new one, which would make every archive in it unreadable.
    #[error(
        "the backup set in {0} has no key (identity.age) here; if the folder is in the cloud, \
         wait until it has downloaded (or choose Keep Downloaded); to start over, move the set away"
    )]
    KeyMissing(String),
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
            Self::Tampered(_) => "backup_unauthenticated",
            Self::KeyMissing(_) => "set_key_missing",
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

/// This machine as sets know it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThisMachine {
    /// Random, made once per install (`ops::machine_id`): what each set's
    /// `seen` and machine list are keyed by. Never the host name, which two
    /// Macs set up from one another (or left on the default) share.
    pub id: String,
    /// The host name: display only (the machine list, conflict copies).
    pub name: String,
}

/// A machine id: 32 lowercase hex characters.
pub fn is_machine_id(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// This machine's name, for display in `set.json` and the index.
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

fn is_hash12(s: &str) -> bool {
    s.len() == 12 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A configuration bundle's name, exactly as a push writes it:
/// `YYYY-MM-DDTHH-MM-SSZ-<12 hex>.tar.gz.age` (the hex: the start of its
/// SHA-256). A set's folder is shared (a cloud folder), so an index naming
/// anything else is never followed.
pub fn is_config_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".tar.gz.age") else {
        return false;
    };
    if !stem.is_ascii() || stem.len() != 33 {
        return false;
    }
    let (t, hash) = (&stem.as_bytes()[..20], &stem[20..]);
    let digits = [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18];
    digits.iter().all(|&i| t[i].is_ascii_digit())
        && [
            (4, b'-'),
            (7, b'-'),
            (10, b'T'),
            (13, b'-'),
            (16, b'-'),
            (19, b'Z'),
        ]
        .iter()
        .all(|&(i, c)| t[i] == c)
        && hash.strip_prefix('-').is_some_and(is_hash12)
}

/// A session archive's name, exactly as a push writes it:
/// `<id>.<12 hex>.tar.gz.age`. Archives are never rewritten: each push of a
/// session writes a new one, and the index names the current one.
pub fn is_archive_name(name: &str, id: &str) -> bool {
    super::ids::is_session_id(id)
        && name
            .strip_prefix(id)
            .and_then(|r| r.strip_prefix('.'))
            .and_then(|r| r.strip_suffix(".tar.gz.age"))
            .is_some_and(is_hash12)
}

/// Drop what an index may not name: session ids that are not session ids (or
/// differ from their key), archives and configuration bundles with any other
/// name.
fn sanitize_index(mut index: SetIndex) -> SetIndex {
    index.sessions.retain(|key, e| {
        let ok = super::ids::is_session_id(key) && e.id == *key && is_archive_name(&e.archive, key);
        if !ok {
            tracing::warn!("a backup index names an invalid session or archive; ignored");
        }
        ok
    });
    index.config.retain(|c| {
        let ok = is_config_name(&c.name);
        if !ok {
            tracing::warn!("a backup index names an invalid configuration bundle; ignored");
        }
        ok
    });
    index.seen.retain(|machine, _| is_machine_id(machine));
    for seen in index.seen.values_mut() {
        seen.retain(|id, _| super::ids::is_session_id(id));
    }
    index
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
    /// The archive an index entry names (`sessions/<id>.<hash>.tar.gz.age`),
    /// for a valid name only.
    pub fn session_archive(&self, e: &IndexEntry) -> Option<PathBuf> {
        is_archive_name(&e.archive, &e.id).then(|| self.sessions_dir().join(&e.archive))
    }
    /// `config/<name>`, for a valid bundle name only (never a path out of
    /// `config/`).
    fn config_bundle(&self, name: &str) -> Option<PathBuf> {
        is_config_name(name).then(|| self.config_dir().join(name))
    }
    /// A file of the set by its index path (`sessions/<name>` or
    /// `config/<name>`), for a valid one only.
    fn file(&self, rel: &str) -> Option<PathBuf> {
        match rel.split_once('/')? {
            ("sessions", name) => {
                let id = name.split('.').next()?;
                is_archive_name(name, id).then(|| self.sessions_dir().join(name))
            }
            ("config", name) => self.config_bundle(name),
            _ => None,
        }
    }

    pub fn exists(&self) -> bool {
        self.identity_path().is_file()
    }

    /// Something of a set is in its folder (a file, or a cloud placeholder)
    /// but not its key.
    pub fn key_missing(&self) -> bool {
        !self.exists() && has_files(&self.dir)
    }
}

/// Whether `dir` holds any file (at any depth) but Finder's `.DS_Store`: an
/// iCloud placeholder (`.<name>.icloud`) counts, an empty folder does not.
fn has_files(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| match e.file_type() {
            Ok(t) if t.is_dir() => has_files(&e.path()),
            Ok(_) => e.file_name() != ".DS_Store",
            Err(_) => true,
        })
}

/// A machine that pushed to the set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Machine {
    /// Its [`ThisMachine::id`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Its host name (display only).
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
    /// The host name of the machine that pushed it (display only).
    pub host: String,
    /// That machine's id.
    #[serde(default)]
    pub machine: String,
    pub pushed_at: i64,
    /// Its archive's file name in `sessions/` (`<id>.<hash>.tar.gz.age`).
    #[serde(default)]
    pub archive: String,
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
    /// The machine whose configuration it is (each machine keeps its own
    /// last [`CONFIG_KEEP`]).
    #[serde(default)]
    pub machine: String,
    pub fingerprint: String,
    /// The bundle file's SHA-256 and size, checked before it is decrypted.
    #[serde(default)]
    pub archive_sha256: String,
    #[serde(default)]
    pub archive_bytes: u64,
}

/// `index.json.age`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetIndex {
    #[serde(default)]
    pub sessions: BTreeMap<String, IndexEntry>,
    #[serde(default)]
    pub config: Vec<ConfigEntry>,
    /// What each machine (by id) last saw of each session
    /// (`transcriptBytes`), for pull's "changed on both sides" rule (§5.11).
    #[serde(default)]
    pub seen: BTreeMap<String, BTreeMap<String, u64>>,
    /// Files the index stopped naming, and when: deleted a day later (a push
    /// on another machine may have read an index that still names them).
    #[serde(default)]
    pub superseded: Vec<Superseded>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

/// A file a push replaced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Superseded {
    /// `sessions/<name>` or `config/<name>`.
    pub path: String,
    pub at: i64,
}

impl SetIndex {
    /// Every file the index names, as `sessions/<name>` and `config/<name>`.
    fn named(&self) -> std::collections::HashSet<String> {
        self.sessions
            .values()
            .map(|e| format!("sessions/{}", e.archive))
            .chain(self.config.iter().map(|c| format!("config/{}", c.name)))
            .collect()
    }
}

/// The set's key pair, and the index's HMAC key derived from its secret.
pub struct Keys {
    identity: age::x25519::Identity,
    recipient: age::x25519::Recipient,
    mac_key: [u8; 32],
}

impl Keys {
    fn from_identity(identity: age::x25519::Identity) -> Self {
        let recipient = identity.to_public();
        let mac_key = hmac_sha256(
            identity.to_string().expose_secret().as_bytes(),
            INDEX_MAC_LABEL,
        );
        Self {
            identity,
            recipient,
            mac_key,
        }
    }
}

fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    use hmac::{Hmac, KeyInit, Mac};
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(key).expect("HMAC takes a key of any size");
    mac.update(msg);
    let mut out = [0u8; 32];
    out.copy_from_slice(&mac.finalize().into_bytes());
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
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

/// Open a set's key with the passphrase, or make a new set: only in a folder
/// with no file of a set in it (a set whose key is missing or not downloaded
/// is never replaced; its archives would become unreadable).
pub fn open_or_create(
    set: &Set,
    account_name: &str,
    passphrase: &Secret,
) -> Result<(Keys, SetInfo), BackupError> {
    if set.exists() {
        let keys = open_keys(set, passphrase)?;
        let info = match read_info(set) {
            Ok(info) if info.recipient != keys.recipient.to_string() => {
                return Err(BackupError::Tampered(format!(
                    "set.json in {} names another key than the set's own; it was changed \
                     by someone without the passphrase",
                    set.dir.display()
                )))
            }
            Ok(info) => info,
            // Display only: rebuilt from the key when it is gone or broken.
            Err(e) => {
                tracing::warn!(error = %e, "a backup set's set.json is unreadable; rewritten");
                new_info(account_name, &keys)
            }
        };
        return Ok((keys, info));
    }
    if set.key_missing() {
        return Err(BackupError::KeyMissing(set.dir.display().to_string()));
    }
    std::fs::create_dir_all(set.sessions_dir()).map_err(|e| io("creating", &set.dir, e))?;
    std::fs::create_dir_all(set.config_dir()).map_err(|e| io("creating", &set.dir, e))?;
    let keys = Keys::from_identity(age::x25519::Identity::generate());
    write_atomic(&set.identity_path(), &seal_identity(&keys, passphrase)?)?;
    let info = new_info(account_name, &keys);
    write_info(set, &info)?;
    write_index(set, &keys, &SetIndex::default())?;
    Ok((keys, info))
}

fn new_info(account_name: &str, keys: &Keys) -> SetInfo {
    SetInfo {
        version: SET_VERSION,
        account: account_name.to_string(),
        created_at: now_ms(),
        recipient: keys.recipient.to_string(),
        machines: Vec::new(),
        extra: BTreeMap::new(),
    }
}

/// `identity.age`: the set's secret key, encrypted with the passphrase.
fn seal_identity(keys: &Keys, passphrase: &Secret) -> Result<Vec<u8>, BackupError> {
    let file = format!(
        "# created: {}\n# public key: {}\n{}\n",
        chrono::Utc::now().to_rfc3339(),
        keys.recipient,
        keys.identity.to_string().expose_secret()
    );
    let enc = passphrase_encryptor(passphrase);
    let mut sealed = Vec::new();
    let mut w = enc
        .wrap_output(&mut sealed)
        .map_err(|e| BackupError::Io(e.to_string()))?;
    w.write_all(file.as_bytes())
        .map_err(|e| BackupError::Io(e.to_string()))?;
    w.finish().map_err(|e| BackupError::Io(e.to_string()))?;
    Ok(sealed)
}

/// The set's key, decrypted with the passphrase.
pub fn open_keys(set: &Set, passphrase: &Secret) -> Result<Keys, BackupError> {
    if !set.exists() {
        return Err(if set.key_missing() {
            BackupError::KeyMissing(set.dir.display().to_string())
        } else {
            BackupError::Invalid(format!("there is no backup set in {}", set.dir.display()))
        });
    }
    let sealed = read_stalling(&set.identity_path())?;
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
    Ok(Keys::from_identity(identity))
}

/// Re-encrypt the set's key with a new passphrase (the archives stay).
pub fn rekey(set: &Set, old: &Secret, new: &Secret) -> Result<(), BackupError> {
    let keys = open_keys(set, old)?;
    write_atomic(&set.identity_path(), &seal_identity(&keys, new)?)
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

/// An iCloud placeholder of `path` (`.<name>.icloud`): the file is in the
/// cloud but not downloaded here.
fn placeholder(path: &Path) -> PathBuf {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    path.with_file_name(format!(".{name}.icloud"))
}

/// The set's index (empty when there is none yet), authenticated: an index
/// without a valid HMAC from the set's key is refused, never used.
pub fn read_index(set: &Set, keys: &Keys) -> Result<SetIndex, BackupError> {
    let path = set.index_path();
    if !path.exists() {
        if placeholder(&path).exists() {
            return Err(io(
                "reading",
                &path,
                "it is in the cloud and not downloaded yet",
            ));
        }
        return Ok(SetIndex::default());
    }
    let sealed = read_stalling(&path)?;
    let tampered = || {
        BackupError::Tampered(format!(
            "the index of the backup set in {} is not authentic: it was not written with this \
             set's key (changed by someone without the passphrase, or made by an older build)",
            set.dir.display()
        ))
    };
    let plain = open_sealed(&keys.identity, &sealed).map_err(|_| tampered())?;
    let nl = plain
        .iter()
        .position(|b| *b == b'\n')
        .ok_or_else(tampered)?;
    let tag = std::str::from_utf8(&plain[..nl])
        .ok()
        .and_then(|l| l.strip_prefix(INDEX_MAGIC))
        .and_then(|l| l.strip_prefix(' '))
        .and_then(unhex)
        .ok_or_else(tampered)?;
    let json = &plain[nl + 1..];
    {
        use hmac::{Hmac, KeyInit, Mac};
        let mut mac =
            Hmac::<sha2::Sha256>::new_from_slice(&keys.mac_key).expect("any key size works");
        mac.update(json);
        // Constant time.
        mac.verify_slice(&tag).map_err(|_| tampered())?;
    }
    serde_json::from_slice(json)
        .map(sanitize_index)
        .map_err(|e| BackupError::Invalid(format!("the index: {e}")))
}

/// [`read_index`] for a write: an index that is missing while archives are
/// in the set is an error (not downloaded yet, or lost), never an empty one
/// that would drop every other machine's sessions.
fn read_index_for_merge(set: &Set, keys: &Keys) -> Result<SetIndex, BackupError> {
    if !set.index_path().exists()
        && (has_files(&set.sessions_dir()) || has_files(&set.config_dir()))
    {
        return Err(BackupError::Invalid(format!(
            "the backup set in {} has archives but no index here; if the folder is in the \
             cloud, wait until it has downloaded",
            set.dir.display()
        )));
    }
    read_index(set, keys)
}

/// `index.json.age`: `MC-INDEX-2 <hmac>\n<json>`, encrypted to the set's key.
fn write_index(set: &Set, keys: &Keys, index: &SetIndex) -> Result<(), BackupError> {
    let json = serde_json::to_vec(index).map_err(|e| BackupError::Io(e.to_string()))?;
    let mut plain = format!(
        "{INDEX_MAGIC} {}\n",
        hex(&hmac_sha256(&keys.mac_key, &json))
    )
    .into_bytes();
    plain.extend_from_slice(&json);
    write_atomic(&set.index_path(), &seal(&keys.recipient, &plain)?)
}

/// How long a file no index names stays in the set: a push on another
/// machine may have read an index that still names it.
const GRACE_MS: i64 = 24 * 3600 * 1000;

/// Whether a file of the set is there, downloaded or not.
fn present(path: &Path) -> bool {
    path.exists() || placeholder(path).exists()
}

/// What one push or pull changes in a set's index.
#[derive(Debug, Default)]
struct Changes {
    /// The sessions this push archived: each new entry and the archive of
    /// the entry it replaces (`None`: the set had none). It lands only when
    /// the set still holds that one.
    sessions: BTreeMap<String, (IndexEntry, Option<String>)>,
    /// A new configuration bundle.
    config: Option<ConfigEntry>,
    /// This machine's whole `seen`.
    seen: Option<BTreeMap<String, u64>>,
}

/// The index as written, and the sessions whose push lost to another
/// machine's.
struct Merged {
    index: SetIndex,
    lost: Vec<String>,
}

/// Write the index merged with what is in the set now: another machine may
/// have pushed since this one read it. A session's new entry replaces only
/// the entry it was made from (compare and swap on the archive name):
/// otherwise the other machine's push stands and this one's archive is
/// removed again. Bundles are joined, each machine keeping its last
/// [`CONFIG_KEEP`]; each machine's `seen` is its own; what the index stops
/// naming is listed as superseded, never deleted here.
fn write_index_merged(
    set: &Set,
    keys: &Keys,
    changes: &Changes,
    machine: &str,
    now: i64,
) -> Result<Merged, BackupError> {
    let theirs = read_index_for_merge(set, keys)?;
    let mut merged = theirs.clone();
    let mut lost = Vec::new();
    let mut superseded = std::mem::take(&mut merged.superseded);
    for (id, (entry, base)) in &changes.sessions {
        let current = merged.sessions.get(id).map(|t| t.archive.clone());
        if current == *base {
            if let Some(old) = base.as_ref().filter(|old| **old != entry.archive) {
                superseded.push(Superseded {
                    path: format!("sessions/{old}"),
                    at: now,
                });
            }
            merged.sessions.insert(id.clone(), entry.clone());
        } else {
            // Ours alone and named by nothing.
            if let Some(path) = set.session_archive(entry) {
                let _ = std::fs::remove_file(path);
            }
            lost.push(id.clone());
        }
    }
    if let Some(c) = &changes.config {
        if !merged.config.iter().any(|t| t.name == c.name) {
            merged.config.push(c.clone());
        }
    }
    merged.config.sort_by(|a, b| a.name.cmp(&b.name));
    let mut kept: BTreeMap<String, usize> = BTreeMap::new();
    let mut config = Vec::new();
    for c in merged.config.drain(..).rev() {
        let n = kept.entry(c.machine.clone()).or_default();
        *n += 1;
        if *n <= CONFIG_KEEP {
            config.push(c);
        } else {
            superseded.push(Superseded {
                path: format!("config/{}", c.name),
                at: now,
            });
        }
    }
    config.reverse();
    merged.config = config;
    if let Some(seen) = &changes.seen {
        let mut mine = seen.clone();
        // A session whose push lost keeps what this machine had before.
        let before = theirs.seen.get(machine);
        for id in &lost {
            match before.and_then(|s| s.get(id)) {
                Some(prev) => mine.insert(id.clone(), *prev),
                None => mine.remove(id),
            };
        }
        merged.seen.insert(machine.to_string(), mine);
    }
    // What the index names again is not superseded; what is long gone is
    // forgotten.
    let named = merged.named();
    superseded.retain(|s| {
        !named.contains(&s.path)
            && set
                .file(&s.path)
                .is_some_and(|p| now - s.at < GRACE_MS || present(&p))
    });
    superseded.sort_by(|a, b| a.path.cmp(&b.path).then(a.at.cmp(&b.at)));
    superseded.dedup_by(|b, a| a.path == b.path);
    merged.superseded = superseded;
    write_index(set, keys, &merged)?;
    Ok(Merged {
        index: merged,
        lost,
    })
}

/// Remove what no index names: superseded files a day after they were
/// superseded, files nothing ever named (a push that crashed, or lost to
/// another machine's) a day after they were written, and stale temp files.
/// Only names a push writes are ever touched.
fn collect_garbage(set: &Set, index: &SetIndex, now: i64) -> usize {
    let named = index.named();
    let superseded: BTreeMap<&str, i64> = index
        .superseded
        .iter()
        .map(|s| (s.path.as_str(), s.at))
        .collect();
    let mut removed = 0;
    for (dir, sub) in [
        (set.sessions_dir(), "sessions"),
        (set.config_dir(), "config"),
    ] {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let rel = format!("{sub}/{name}");
            let temp = name.starts_with('.') && name.contains(".mc-") && name.ends_with(".tmp");
            if named.contains(&rel) || !(temp || set.file(&rel).is_some()) {
                continue;
            }
            let Ok(meta) = std::fs::symlink_metadata(e.path()) else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            let since = superseded
                .get(rel.as_str())
                .copied()
                .unwrap_or_else(|| mtime_ms(&meta));
            if now - since >= GRACE_MS && std::fs::remove_file(e.path()).is_ok() {
                removed += 1;
            }
        }
    }
    removed
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
    /// Changed here and, since this machine last had them, in the set by
    /// another machine: not pushed (a pull brings the other copy beside).
    pub conflicts: Vec<String>,
    /// The configuration bundle written, when it changed.
    pub config: Option<String>,
    pub bytes: u64,
    /// What was left out without failing the push (an unreadable
    /// configuration file, a bundle that could not be written…).
    pub warnings: Vec<String>,
    /// Files no index names any more, removed.
    pub removed: usize,
}

/// What a push needs.
pub struct PushInputs<'a> {
    pub home: &'a ClaudeHome,
    pub set: &'a Set,
    pub account_name: &'a str,
    pub passphrase: &'a Secret,
    pub machine: &'a ThisMachine,
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
    dir: &Path,
    name_for: &dyn Fn(&str) -> String,
    recipient: &age::x25519::Recipient,
    base: &Path,
    files: &[(PathBuf, PathBuf)],
    dirs: &[PathBuf],
    extra: &[(&str, Vec<u8>)],
) -> Result<(String, String, u64), BackupError> {
    use sha2::Digest;
    std::fs::create_dir_all(dir).map_err(|e| io("creating", dir, e))?;
    let tmp = dir.join(format!(".archive.mc-{}.tmp", uuid::Uuid::new_v4().simple()));
    let dest = &tmp;
    let result = (|| -> Result<(String, String, u64), BackupError> {
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
        // Named after what it holds: never over another archive.
        let name = name_for(&sha);
        let place = dir.join(&name);
        std::fs::rename(&tmp, &place).map_err(|e| io("placing", &place, e))?;
        Ok((name, sha, bytes))
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
///
/// Crash-safe: every archive is a new file, the index naming it is written
/// only after it, and a file the index stops naming is removed a day later,
/// so the set always opens as of its last index. Nothing after the archives
/// (the configuration bundle, `set.json`) can cost the index.
pub fn push(inp: &PushInputs<'_>) -> Result<PushReport, BackupError> {
    let (keys, mut info) = open_or_create(inp.set, inp.account_name, inp.passphrase)?;
    let index = read_index(inp.set, &keys)?;
    let mut report = PushReport::default();
    let mut seen = index.seen.get(&inp.machine.id).cloned().unwrap_or_default();
    let mut changes = Changes::default();
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
        let current = index.sessions.get(&id);
        let unchanged = current.is_some_and(|e| {
            e.transcript_bytes == now.transcript_bytes
                && e.newest_mtime == now.newest_mtime
                && inp.set.session_archive(e).is_some_and(|p| present(&p))
        });
        if unchanged {
            report.unchanged += 1;
            continue;
        }
        match archive_session(inp, &keys, &id, &now) {
            Ok(entry) => {
                inp.progress.files_done.fetch_add(1, Ordering::Relaxed);
                inp.progress
                    .bytes_done
                    .fetch_add(entry.archive_bytes, Ordering::Relaxed);
                report.bytes += entry.archive_bytes;
                seen.insert(id.clone(), now.transcript_bytes);
                changes
                    .sessions
                    .insert(id.clone(), (entry, current.map(|e| e.archive.clone())));
                report.archived.push(id);
            }
            Err(e) => report.failed.push((id, e.to_string())),
        }
    }
    match push_config(inp, &keys, &index) {
        Ok((entry, unreadable)) => {
            for rel in unreadable {
                report.warnings.push(format!(
                    "{rel} could not be read and is not in the configuration bundle"
                ));
            }
            if let Some(c) = entry {
                report.config = Some(c.name.clone());
                changes.config = Some(c);
            }
        }
        Err(e) => report
            .warnings
            .push(format!("The configuration bundle was not written: {e}")),
    }
    changes.seen = Some(seen);
    let at = now_ms();
    let merged = write_index_merged(inp.set, &keys, &changes, &inp.machine.id, at)?;
    for id in merged.lost {
        report.archived.retain(|a| *a != id);
        report.conflicts.push(id);
    }
    report.removed = collect_garbage(inp.set, &merged.index, at);
    note_push(&mut info, inp.machine);
    if let Err(e) = write_info(inp.set, &info) {
        report
            .warnings
            .push(format!("set.json was not updated: {e}"));
    }
    Ok(report)
}

/// One session's archive, written as a new file, and its index entry.
fn archive_session(
    inp: &PushInputs<'_>,
    keys: &Keys,
    id: &str,
    now: &SessionNow,
) -> Result<IndexEntry, BackupError> {
    let base = inp.home.base();
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
    let (archive, sha, bytes) = write_archive(
        &inp.set.sessions_dir(),
        &|sha| format!("{id}.{}.tar.gz.age", &sha[..12]),
        &keys.recipient,
        base,
        &files,
        &now.dirs,
        &[(".mc/meta.json", meta)],
    )?;
    Ok(IndexEntry {
        id: id.to_string(),
        cwd: now.cwd.clone(),
        transcript_bytes: now.transcript_bytes,
        newest_mtime: now.newest_mtime,
        host: inp.machine.name.clone(),
        machine: inp.machine.id.clone(),
        pushed_at: now_ms(),
        archive,
        archive_sha256: sha,
        archive_bytes: bytes,
        files: files.len() as u32,
    })
}

/// This machine's configuration bundle, when it changed since this machine's
/// last one; a file that cannot be read is left out (and named), never a
/// reason for no bundle.
fn push_config(
    inp: &PushInputs<'_>,
    keys: &Keys,
    index: &SetIndex,
) -> Result<(Option<ConfigEntry>, Vec<String>), BackupError> {
    let mut unreadable = Vec::new();
    let files: Vec<(PathBuf, PathBuf)> = config_files(inp.home)
        .into_iter()
        .filter(|(path, rel)| {
            let ok = std::fs::File::open(path).is_ok();
            if !ok {
                unreadable.push(rel.display().to_string());
            }
            ok
        })
        .collect();
    let fingerprint = config_fingerprint(&files);
    let mine = index
        .config
        .iter()
        .rev()
        .find(|c| c.machine == inp.machine.id);
    if mine.is_some_and(|c| c.fingerprint == fingerprint) {
        return Ok((None, unreadable));
    }
    let stamp = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%SZ").to_string();
    let (name, sha, bytes) = write_archive(
        &inp.set.config_dir(),
        &|sha| format!("{stamp}-{}.tar.gz.age", &sha[..12]),
        &keys.recipient,
        inp.home.base(),
        &files,
        &[],
        &[],
    )?;
    Ok((
        Some(ConfigEntry {
            name,
            created_at: now_ms(),
            host: inp.machine.name.clone(),
            machine: inp.machine.id.clone(),
            fingerprint,
            archive_sha256: sha,
            archive_bytes: bytes,
        }),
        unreadable,
    ))
}

/// This machine's line in `set.json`: by id; its name as it is now.
fn note_push(info: &mut SetInfo, machine: &ThisMachine) {
    let at = Some(now_ms());
    match info
        .machines
        .iter_mut()
        .find(|m| m.id.as_deref() == Some(machine.id.as_str()))
    {
        Some(m) => {
            m.host = machine.name.clone();
            m.last_push = at;
        }
        None => info.machines.push(Machine {
            id: Some(machine.id.clone()),
            host: machine.name.clone(),
            last_push: at,
        }),
    }
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
    // Any positive integer is a valid setting: "keep forever" may be 10^12.
    let cutoff = now.saturating_sub(days.saturating_mul(24 * 3600 * 1000));
    (newest_mtime < cutoff).then(|| {
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
    // Ours alone: a stage that exists already (or a link planted there) is
    // never written into.
    std::fs::create_dir(&stage_root).map_err(|e| io("creating", &stage_root, e))?;
    let result = restore_into(
        inp,
        &keys,
        &index,
        &stage_root,
        &mut m,
        &mut report,
        now,
        days,
        &undo_dir,
        &target_busy,
    );
    let _ = std::fs::remove_dir_all(&stage_root);
    result.map(|()| (report, m))
}

/// The per-session part of [`restore`], staging under `stage_root`.
#[allow(clippy::too_many_arguments)]
fn restore_into(
    inp: &RestoreInputs<'_>,
    keys: &Keys,
    index: &SetIndex,
    stage_root: &Path,
    m: &mut Manifest,
    report: &mut RestoreReport,
    now: i64,
    days: i64,
    undo_dir: &Path,
    target_busy: &Option<transfer::TransferError>,
) -> Result<(), BackupError> {
    for id in inp.ids {
        let skip = |code: &str, reason: String| Skip {
            id: id.clone(),
            code: code.to_string(),
            reason,
        };
        if !super::ids::is_session_id(id) {
            report
                .skipped
                .push(skip("invalid_id", "not a session id".into()));
            continue;
        }
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
        let one = stage_session(inp.set, keys, entry, stage_root).and_then(|st| {
            place_staged(&st, inp.target, m, inp.progress)?;
            inp.progress.files_done.fetch_add(1, Ordering::Relaxed);
            Ok(st.meta)
        });
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
                        if let Some(busy) = target_busy {
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
            // Nothing of a session that did not come back whole stays.
            Err(e) => report.skipped.push(skip(e.code(), e.to_string())),
        }
        crate::persist::write_json_private(&undo_dir.join("manifest.json"), &*m)
            .map_err(|e| io("writing", undo_dir, e))?;
    }
    Ok(())
}

/// The archive an index entry names, read whole (it may still be
/// downloading).
fn read_archive(set: &Set, entry: &IndexEntry) -> Result<Vec<u8>, BackupError> {
    let path = set.session_archive(entry).ok_or_else(|| {
        BackupError::Invalid(format!("the index names no valid archive of {}", entry.id))
    })?;
    read_stalling(&path)
}

/// A session of the set: fetched, checked against the index and unpacked
/// into its own directory under `stage_root`.
struct Staged {
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
    dir: PathBuf,
    meta: Value,
}

fn stage_session(
    set: &Set,
    keys: &Keys,
    entry: &IndexEntry,
    stage_root: &Path,
) -> Result<Staged, BackupError> {
    let id = &entry.id;
    let bytes = read_archive(set, entry)?;
    if super::secrets::sha256_hex(&bytes) != entry.archive_sha256 {
        return Err(BackupError::Invalid(format!(
            "the archive of {id} is not the one the index names (truncated or changed)"
        )));
    }
    let dir = stage_root.join(id);
    std::fs::create_dir(&dir).map_err(|e| io("creating", &dir, e))?;
    let meta = unpack(&bytes, &keys.identity, &dir, id)?;
    let (files, dirs) = staged(&dir);
    Ok(Staged {
        files,
        dirs,
        dir,
        meta,
    })
}

/// Move a staged session into `target`, where nothing may be in the way,
/// recording what it made in `m`. On failure what this made is removed again.
fn place_staged(
    st: &Staged,
    target: &ClaudeHome,
    m: &mut Manifest,
    progress: &Progress,
) -> Result<(), BackupError> {
    let base = target.base();
    let (files_before, dirs_before) = (m.created.len(), m.created_dirs.len());
    let result = (|| -> Result<(), BackupError> {
        for rel in &st.files {
            // Not even a dangling link.
            if std::fs::symlink_metadata(base.join(rel)).is_ok() {
                return Err(BackupError::Transfer(
                    transfer::TransferError::AlreadyExists(rel.display().to_string()),
                ));
            }
        }
        for rel in &st.dirs {
            for d in transfer::missing_dirs(&base.join(rel), base) {
                std::fs::create_dir(&d).map_err(|e| io("creating", &d, e))?;
                m.created_dirs.push(d);
            }
        }
        for rel in &st.files {
            let dest = base.join(rel);
            if let Some(parent) = dest.parent() {
                for d in transfer::missing_dirs(parent, base) {
                    std::fs::create_dir(&d).map_err(|e| io("creating", &d, e))?;
                    m.created_dirs.push(d);
                }
            }
            std::fs::rename(st.dir.join(rel), &dest).map_err(|e| io("placing", &dest, e))?;
            let (size, sha) =
                transfer::sha256_file(&dest).map_err(|e| io("verifying", &dest, e))?;
            progress.bytes_done.fetch_add(size, Ordering::Relaxed);
            m.created.push(Created {
                path: dest,
                size,
                sha256: sha,
            });
        }
        Ok(())
    })();
    if result.is_err() {
        for c in m.created.drain(files_before..) {
            let _ = std::fs::remove_file(&c.path);
        }
        for d in m.created_dirs.drain(dirs_before..).rev() {
            let _ = std::fs::remove_dir(&d);
        }
    }
    result
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

/// What the set holds, compared with this machine's directory (`machine`:
/// this machine's id).
pub fn compare(
    index: &SetIndex,
    home: &ClaudeHome,
    machine: &str,
    is_live: &dyn Fn(&str) -> bool,
    now: i64,
) -> Vec<PullItem> {
    let days = cleanup_days(home);
    let seen = index.seen.get(machine);
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
    pub machine: &'a ThisMachine,
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
    let plan = compare(&index, inp.home, &inp.machine.id, inp.is_live, now);
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
            PullAction::Conflict => match conflict_copy(inp, &keys, &index, &item.id) {
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
    // de-duplicated, and only while nothing here is writing it. It never
    // fails the pull: what came back stays, with its undo.
    if let Err(e) = merge_history(inp, &keys, &index, &mut m, &mut report) {
        report.warnings.push(Warning {
            id: String::new(),
            text: format!("Prompt history was not merged: {e}"),
        });
    }

    // The undo first: whatever happens to the index, what was written here
    // can be taken back.
    crate::persist::write_json_private(&undo_dir.join("manifest.json"), &m)
        .map_err(|e| io("writing", &undo_dir, e))?;
    // What this machine now has, for the next comparison.
    let seen = index.seen.entry(inp.machine.id.clone()).or_default();
    for id in report.restored.iter().chain(&report.updated) {
        if let Some(e) = index.sessions.get(id) {
            seen.insert(id.clone(), e.transcript_bytes);
        }
    }
    let changes = Changes {
        seen: index.seen.get(&inp.machine.id).cloned(),
        ..Changes::default()
    };
    if let Err(e) = write_index_merged(inp.set, &keys, &changes, &inp.machine.id, now_ms()) {
        report.warnings.push(Warning {
            id: String::new(),
            text: format!("The set does not know yet what this machine has: {e}"),
        });
    }
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

/// The prompt history of every other machine's newest bundle appended to
/// this machine's, without the lines it has (by time and text).
fn merge_history(
    inp: &PullInputs<'_>,
    keys: &Keys,
    index: &SetIndex,
    m: &mut Manifest,
    report: &mut PullReport,
) -> Result<(), BackupError> {
    let mut newest: BTreeMap<&str, &ConfigEntry> = BTreeMap::new();
    for c in index.config.iter().filter(|c| c.machine != inp.machine.id) {
        if newest
            .get(c.machine.as_str())
            .is_none_or(|n| n.name < c.name)
        {
            newest.insert(&c.machine, c);
        }
    }
    let mut lines = Vec::new();
    for bundle in newest.values() {
        lines.extend(bundle_history(inp.set, keys, bundle)?.unwrap_or_default());
    }
    if lines.is_empty() {
        return Ok(());
    }
    let path = inp.home.history_file();
    let local = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(io("reading", &path, e)),
    };
    let mut have: std::collections::HashSet<(String, String)> = local
        .split(|b| *b == b'\n')
        .filter_map(|l| std::str::from_utf8(l).ok())
        .filter_map(history_key)
        .collect();
    // Once each, also across bundles.
    let fresh: Vec<String> = lines
        .into_iter()
        .filter(|l| history_key(l).is_some_and(|k| have.insert(k)))
        .collect();
    if fresh.is_empty() {
        return Ok(());
    }
    if let Some(busy) = transfer::in_use(inp.home) {
        return Err(BackupError::Transfer(busy));
    }
    transfer::append_lines(&path, &fresh)?;
    report.history_lines = fresh.len();
    m.added_history = fresh;
    Ok(())
}

/// `history.jsonl` from a configuration bundle, if it has one.
fn bundle_history(
    set: &Set,
    keys: &Keys,
    bundle: &ConfigEntry,
) -> Result<Option<Vec<String>>, BackupError> {
    let bytes = read_bundle(set, bundle)?;
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
            let mut raw = Vec::new();
            entry
                .read_to_end(&mut raw)
                .map_err(|e| BackupError::Io(e.to_string()))?;
            // A line cut by a crash is left out, not the whole history.
            let lines = raw
                .split(|b| *b == b'\n')
                .filter_map(|l| std::str::from_utf8(l).ok())
                .map(|l| l.trim_end_matches('\r'))
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect();
            return Ok(Some(lines));
        }
    }
    Ok(None)
}

/// A configuration bundle's bytes, checked against the SHA-256 the
/// (authenticated) index names.
fn read_bundle(set: &Set, entry: &ConfigEntry) -> Result<Vec<u8>, BackupError> {
    let path = set
        .config_bundle(&entry.name)
        .ok_or_else(|| BackupError::Invalid("the index names an invalid bundle".into()))?;
    let bytes = read_stalling(&path)?;
    if super::secrets::sha256_hex(&bytes) != entry.archive_sha256 {
        return Err(BackupError::Invalid(format!(
            "the configuration bundle {} is not the one the index names (truncated or changed)",
            entry.name
        )));
    }
    Ok(bytes)
}

/// The set's transcript of a session changed on both sides, written beside
/// the local one as `<id>.conflict-<host>-<time it was pushed>.jsonl` (one
/// file per pushed version, however often it is pulled).
fn conflict_copy(
    inp: &PullInputs<'_>,
    keys: &Keys,
    index: &SetIndex,
    id: &str,
) -> Result<Option<Created>, BackupError> {
    let Some(entry) = index.sessions.get(id) else {
        return Ok(None);
    };
    let local = transfer::transcripts_of(inp.home, id);
    let Some(local) = local.first() else {
        return Ok(None);
    };
    let bytes = read_archive(inp.set, entry)?;
    if super::secrets::sha256_hex(&bytes) != entry.archive_sha256 {
        return Err(BackupError::Invalid(format!(
            "the archive of {id} is not the one the index names"
        )));
    }
    let stage_root = inp
        .home
        .base()
        .join(format!(".mc-restore-{}-c", inp.job_id));
    let stage = stage_root.join(id);
    // Ours alone, like a restore's stage.
    std::fs::create_dir(&stage_root).map_err(|e| io("creating", &stage_root, e))?;
    let result = (|| -> Result<Option<Created>, BackupError> {
        std::fs::create_dir(&stage).map_err(|e| io("creating", &stage, e))?;
        unpack(&bytes, &keys.identity, &stage, id)?;
        let (files, _) = staged(&stage);
        let Some(main) = files
            .iter()
            .find(|f| f.file_name().and_then(|n| n.to_str()) == Some(&format!("{id}.jsonl")))
        else {
            return Ok(None);
        };
        let stamp = chrono::DateTime::from_timestamp_millis(entry.pushed_at)
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
        if std::fs::symlink_metadata(&dest).is_ok() {
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
    let _ = std::fs::remove_dir_all(&stage_root);
    result
}

/// Unpack the newest configuration bundle into `into` for the user to look
/// at (it never replaces the live configuration): `machine`'s own when it
/// has one, else the newest of any machine. Returns the folder.
pub fn extract_config(
    set: &Set,
    passphrase: &Secret,
    into: &Path,
    machine: Option<&str>,
) -> Result<PathBuf, BackupError> {
    let keys = open_keys(set, passphrase)?;
    let index = read_index(set, &keys)?;
    let last = machine
        .and_then(|m| index.config.iter().rev().find(|c| c.machine == m))
        .or_else(|| index.config.iter().max_by(|a, b| a.name.cmp(&b.name)))
        .ok_or_else(|| BackupError::Invalid("this set has no configuration bundle".into()))?;
    let bytes = read_bundle(set, last)?;
    let dec =
        age::Decryptor::new_buffered(&bytes[..]).map_err(|e| BackupError::Io(e.to_string()))?;
    let r = dec
        .decrypt(std::iter::once(&keys.identity as &dyn age::Identity))
        .map_err(|e| BackupError::Io(format!("could not decrypt: {e}")))?;
    // One plain component (the name is checked): `into/<time>`.
    let stem = last.name.trim_end_matches(".tar.gz.age");
    let dest = into.join(stem);
    std::fs::create_dir_all(into).map_err(|e| io("creating", into, e))?;
    transfer::private_dir(into);
    // Unpacked beside it first, so a second look replaces the first whole.
    let tmp = into.join(format!(".{stem}.mc-{}.tmp", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir(&tmp).map_err(|e| io("creating", &tmp, e))?;
    transfer::private_dir(&tmp);
    let result = (|| -> Result<(), BackupError> {
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(BufReader::new(r)));
        tar.set_overwrite(false);
        for entry in tar.entries().map_err(|e| BackupError::Io(e.to_string()))? {
            let mut entry = entry.map_err(|e| BackupError::Io(e.to_string()))?;
            let kind = entry.header().entry_type();
            if !(kind.is_file() || kind.is_dir()) {
                continue;
            }
            entry
                .unpack_in(&tmp)
                .map_err(|e| BackupError::Io(e.to_string()))?;
        }
        if std::fs::symlink_metadata(&dest).is_ok() {
            std::fs::remove_dir_all(&dest).map_err(|e| io("replacing", &dest, e))?;
        }
        std::fs::rename(&tmp, &dest).map_err(|e| io("placing", &dest, e))
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    result.map(|()| dest)
}

pub mod ops;

#[cfg(test)]
mod tests;
