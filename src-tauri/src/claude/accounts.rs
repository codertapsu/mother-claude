//! The Claude accounts Mother Claude can run sessions under
//! (docs/ACCOUNTS.md).
//!
//! An account is a **directory** (its own config directory, so its own
//! sign-in, settings, plugins and data) or a **token** (a long-lived token
//! from `claude setup-token`, used with the Default account's directory). The
//! **Default** account is the directory Mother Claude resolved at startup
//! (`CLAUDE_CONFIG_DIR` of its own environment, else `~/.claude`); it is always
//! present and cannot be removed.
//!
//! The registry lives in `<app data dir>/accounts.json` (0600) and is parsed
//! tolerantly: unknown fields are kept, a bad entry is skipped with a log line
//! and written back unchanged by the next save (another run may use it).
//! It is written only when the user changes something, so a machine with only
//! the Default account never gets the file. Secrets never go in it
//! ([`super::secrets`]); a token account keeps only its token's SHA-256, for
//! matching a running process to the account (D8).
//!
//! [`AccountEnv`] composes a child's environment for an account (D5): the
//! Default inherits ours unchanged, a directory account gets
//! `CLAUDE_CONFIG_DIR`, a token account `CLAUDE_CODE_OAUTH_TOKEN`, and both
//! lose the variables that would make the child use another identity
//! ([`ACCOUNT_OVERRIDE_ENV`]).

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::home::{user_home_dir, ClaudeHome};
use super::secrets::{Secret, SecretError, SecretName, SecretStore};

pub mod capture;
pub mod identity;
pub mod sessions;
pub mod signin;
pub mod status;
pub mod view;

/// The Default account's id.
pub const DEFAULT_ID: &str = "default";
/// The Default account's name until the user renames it.
pub const DEFAULT_NAME: &str = "Default";
/// The registry file in the app data dir.
pub const FILE_NAME: &str = "accounts.json";
/// Longest account name, in characters.
pub const NAME_MAX: usize = 40;
/// The Default account's colour until the user changes it.
const DEFAULT_COLOR: &str = "#8b5cf6";
/// Colours handed to new accounts in turn.
const PALETTE: [&str; 8] = [
    "#0ea5e9", "#f59e0b", "#10b981", "#ef4444", "#ec4899", "#14b8a6", "#6366f1", "#84cc16",
];

/// The variables that would make a child use another identity than its
/// account's (D5). Removed for directory and token accounts; the Default
/// account keeps whatever the app found, like every child before accounts.
pub const ACCOUNT_OVERRIDE_ENV: [&str; 6] = [
    "CLAUDE_CODE_OAUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
];

/// The pseudo-account of a session that runs with a token no account holds
/// (D8: "token (unregistered)"). Never a real id: nothing runs under it.
pub const UNREGISTERED_TOKEN_ID: &str = "unregistered";

/// The variable a directory account sets.
pub const CONFIG_DIR_ENV: &str = "CLAUDE_CONFIG_DIR";
/// The variable a token account sets.
pub const OAUTH_TOKEN_ENV: &str = "CLAUDE_CODE_OAUTH_TOKEN";

/// `acc_` + 12 lowercase hex characters.
pub fn is_account_id(id: &str) -> bool {
    id.strip_prefix("acc_").is_some_and(|hex| {
        hex.len() == 12
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// `default` or an [`is_account_id`].
pub fn is_valid_id(id: &str) -> bool {
    id == DEFAULT_ID || is_account_id(id)
}

fn new_account_id() -> String {
    let bytes = crate::server::auth::random_bytes(6);
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("acc_{hex}")
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ─────────────────────────────── the model ────────────────────────────────

/// The two kinds of account (D1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountKind {
    /// Its own config directory and its own sign-in.
    Directory,
    /// A long-lived token used with the Default account's directory.
    Token,
}

/// One registered account.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub name: String,
    pub kind: AccountKind,
    /// A directory account's directory; `None` for the Default (its
    /// directory is the one resolved at startup) and for token accounts.
    #[serde(default)]
    pub dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default)]
    pub created_at: i64,
    /// The hooks plugin's state in this account's directory, as last seen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hooks_plugin: Option<String>,
    /// SHA-256 of a token account's token (the token is in the secret store).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_sha256: Option<String>,
    /// The Anthropic account's opaque id, learned from the first
    /// `initialize.account` seen for it (never an email).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_uuid: Option<String>,
    /// SHA-256 of the email the account signed in as (never the email):
    /// who it should be, for the identity warnings (docs/ACCOUNTS.md §15.3,
    /// [`identity`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_sha256: Option<String>,
    /// Fields a newer version wrote; kept as they were.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl Account {
    pub fn is_default(&self) -> bool {
        self.id == DEFAULT_ID
    }

    /// The colour to show (the Default's when none was chosen).
    pub fn color_or_default(&self) -> &str {
        self.color.as_deref().unwrap_or(DEFAULT_COLOR)
    }

    fn default_account() -> Self {
        Self {
            id: DEFAULT_ID.to_string(),
            name: DEFAULT_NAME.to_string(),
            kind: AccountKind::Directory,
            dir: None,
            color: Some(DEFAULT_COLOR.to_string()),
            created_at: 0,
            hooks_plugin: None,
            token_sha256: None,
            account_uuid: None,
            identity_sha256: None,
            extra: BTreeMap::new(),
        }
    }
}

/// Lowercase hex SHA-256, as the registry keeps hashes: 64 hex characters.
fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Which account new work runs under, when the caller does not say.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDefaults {
    /// New sessions started from the dashboard.
    #[serde(default = "default_id")]
    pub new_session: String,
    /// Bridge conversations (the API screen's start-time default).
    #[serde(default = "default_id")]
    pub bridge: String,
    /// Project directory → account; the longest matching directory wins and
    /// beats `newSession`.
    #[serde(default)]
    pub by_project: BTreeMap<String, String>,
}

impl Default for AccountDefaults {
    fn default() -> Self {
        Self {
            new_session: default_id(),
            bridge: default_id(),
            by_project: BTreeMap::new(),
        }
    }
}

fn default_id() -> String {
    DEFAULT_ID.to_string()
}

/// "Copy sessions from `from` to `to` for *continue on* without asking" (§11.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopyConsent {
    pub from: String,
    pub to: String,
}

/// Where and how often backups go (§5.10).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSettings {
    /// The folder the backup sets live in (one sub-folder per account).
    #[serde(default)]
    pub folder: Option<PathBuf>,
    /// Back up every this many hours while idle; 0 = only by hand.
    #[serde(default)]
    pub every_hours: u32,
    /// Pull from the set at launch when no session is live.
    #[serde(default)]
    pub pull_on_launch: bool,
    /// More storages besides the backup folder (§13.1): folders of sets only
    /// Move data reads and writes. Left out while there are none.
    #[serde(
        default,
        deserialize_with = "tolerant_storages",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub storages: Vec<StorageEntry>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

/// The backup folder's id as a storage.
pub const BACKUP_STORAGE_ID: &str = "backup";

/// `stor_` + 12 lowercase hex characters.
pub fn is_storage_id(id: &str) -> bool {
    id.strip_prefix("stor_").is_some_and(|hex| {
        hex.len() == 12
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// A new storage id.
pub fn new_storage_id() -> String {
    let hex: String = crate::server::auth::random_bytes(6)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("stor_{hex}")
}

/// A storage besides the backup folder (§13.1): another cloud's folder, a
/// NAS, an external disk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageEntry {
    /// `stor_` + 12 hex.
    pub id: String,
    pub label: String,
    /// Absolute.
    pub path: PathBuf,
    #[serde(default)]
    pub created_at: i64,
    /// Fields a newer version wrote; kept as they were.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

/// `storages` as read from the file: an entry that does not validate (or a
/// list that is not one) is skipped with a log line, never the whole backup
/// settings.
fn tolerant_storages<'de, D>(d: D) -> Result<Vec<StorageEntry>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let Value::Array(items) = Value::deserialize(d)? else {
        tracing::warn!("accounts.json: ignoring storages that are not a list");
        return Ok(Vec::new());
    };
    let mut out: Vec<StorageEntry> = Vec::new();
    for raw in items {
        match serde_json::from_value::<StorageEntry>(raw) {
            Ok(s)
                if is_storage_id(&s.id)
                    && s.path.is_absolute()
                    && !out.iter().any(|o| o.id == s.id) =>
            {
                out.push(s)
            }
            _ => tracing::warn!("accounts.json: skipping an unreadable storage"),
        }
    }
    Ok(out)
}

/// The whole registry, as kept in memory.
#[derive(Debug, Clone, PartialEq)]
pub struct Registry {
    /// The Default first, then the rest in the order they were added.
    pub accounts: Vec<Account>,
    pub defaults: AccountDefaults,
    pub copy_consent: Vec<CopyConsent>,
    pub backup: BackupSettings,
    extra: BTreeMap<String, Value>,
    /// What this run could not use, written back as it was.
    held: Held,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            accounts: vec![Account::default_account()],
            defaults: AccountDefaults::default(),
            copy_consent: Vec::new(),
            backup: BackupSettings::default(),
            extra: BTreeMap::new(),
            held: Held::default(),
        }
    }
}

/// The parts of `accounts.json` a run skipped — an account whose directory
/// is this run's Default (the app started with another `CLAUDE_CONFIG_DIR`),
/// a newer version's kind, a hand edit — and what named them. Saves write
/// them back unchanged, so a run that cannot use an account never erases it
/// (saves happen without a user action too, e.g. a plugin state learned at
/// startup).
#[derive(Debug, Clone, Default, PartialEq)]
struct Held {
    /// The raw entries, in file order.
    accounts: Vec<Value>,
    /// Their ids, where they have one.
    ids: std::collections::BTreeSet<String>,
    /// Copy consents naming a held account.
    copy_consent: Vec<Value>,
    /// `byProject` entries naming a held account.
    by_project: BTreeMap<String, String>,
    /// `newSession` / `bridge` named a held account (until the user picks
    /// the defaults again).
    new_session: Option<String>,
    bridge: Option<String>,
}

impl Registry {
    pub fn get(&self, id: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.id == id)
    }

    fn get_mut(&mut self, id: &str) -> Option<&mut Account> {
        self.accounts.iter_mut().find(|a| a.id == id)
    }

    /// Only the Default is registered (the v0.4 behaviour, goal 7).
    pub fn is_default_only(&self) -> bool {
        self.accounts.len() == 1
    }

    /// A token account is registered: only then does a token in a session's
    /// environment say anything about its account.
    pub fn has_token_accounts(&self) -> bool {
        self.accounts.iter().any(|a| a.kind == AccountKind::Token)
    }

    /// The account a token belongs to, by the token's SHA-256 (D8).
    pub fn by_token_sha256(&self, sha: &str) -> Option<&Account> {
        self.accounts
            .iter()
            .find(|a| a.kind == AccountKind::Token && a.token_sha256.as_deref() == Some(sha))
    }

    /// The account new work in `cwd` runs under: the longest `byProject`
    /// directory containing `cwd`, else `newSession`.
    pub fn account_for_cwd(&self, cwd: &str) -> String {
        self.project_account(cwd)
            .unwrap_or_else(|| self.defaults.new_session.clone())
    }

    /// The account of the longest `byProject` directory containing `cwd`.
    pub fn project_account(&self, cwd: &str) -> Option<String> {
        let cwd = Path::new(cwd);
        self.defaults
            .by_project
            .iter()
            .filter(|(dir, id)| cwd.starts_with(dir) && self.get(id).is_some())
            .max_by_key(|(dir, _)| Path::new(dir).components().count())
            .map(|(_, id)| id.clone())
    }

    pub fn has_copy_consent(&self, from: &str, to: &str) -> bool {
        self.copy_consent
            .iter()
            .any(|c| c.from == from && c.to == to)
    }

    fn to_file(&self) -> RegistryFile {
        let mut defaults = serde_json::to_value(&self.defaults).unwrap_or(Value::Null);
        if let Some(d) = defaults.as_object_mut() {
            if let Some(id) = &self.held.new_session {
                d.insert("newSession".into(), Value::from(id.clone()));
            }
            if let Some(id) = &self.held.bridge {
                d.insert("bridge".into(), Value::from(id.clone()));
            }
            if let Some(Value::Object(by)) = d.get_mut("byProject") {
                for (dir, id) in &self.held.by_project {
                    by.entry(dir.clone())
                        .or_insert_with(|| Value::from(id.clone()));
                }
            }
        }
        RegistryFile {
            version: 1,
            accounts: self
                .accounts
                .iter()
                .filter_map(|a| serde_json::to_value(a).ok())
                .chain(self.held.accounts.iter().cloned())
                .collect(),
            defaults,
            copy_consent: self
                .copy_consent
                .iter()
                .filter_map(|c| serde_json::to_value(c).ok())
                .chain(self.held.copy_consent.iter().cloned())
                .collect(),
            backup: serde_json::to_value(&self.backup).unwrap_or(Value::Null),
            extra: self.extra.clone(),
        }
    }

    /// Hold an entry this run cannot use (see [`Held`]).
    fn hold(&mut self, raw: Value) {
        if let Some(id) = raw.get("id").and_then(Value::as_str) {
            if is_account_id(id) && self.get(id).is_none() {
                self.held.ids.insert(id.to_string());
            }
        }
        self.held.accounts.push(raw);
    }

    /// Build from the file's contents, skipping (and holding) what does not
    /// validate.
    fn from_file(file: RegistryFile, default_dir: &Path) -> Self {
        let mut reg = Registry {
            extra: file.extra,
            ..Default::default()
        };
        for raw in file.accounts {
            let mut account: Account = match serde_json::from_value(raw.clone()) {
                Ok(a) => a,
                Err(e) => {
                    tracing::warn!(error = %e, "accounts.json: skipping an unreadable account");
                    reg.hold(raw);
                    continue;
                }
            };
            if account.id == DEFAULT_ID {
                // The Default keeps a chosen name and colour; its kind and
                // directory are not the file's to decide.
                let default = &mut reg.accounts[0];
                if let Ok(name) = clean_name(&account.name) {
                    default.name = name;
                }
                if account.color.as_deref().is_some_and(is_color) {
                    default.color = account.color;
                }
                default.hooks_plugin = account.hooks_plugin;
                default.account_uuid = account.account_uuid;
                default.identity_sha256 = account.identity_sha256.filter(|s| is_sha256_hex(s));
                default.extra = account.extra;
                continue;
            }
            if let Err(why) = validate_loaded(&reg, &mut account, default_dir) {
                tracing::warn!(id = %account.id, why, "accounts.json: skipping an account");
                reg.hold(raw);
                continue;
            }
            reg.accounts.push(account);
        }
        // An id both valid and held (a duplicate) is the valid one's.
        let valid: Vec<String> = reg.accounts.iter().map(|a| a.id.clone()).collect();
        reg.held.ids.retain(|id| !valid.contains(id));
        // A duplicate name of the Default (renamed after the others loaded).
        let default_name = reg.accounts[0].name.to_lowercase();
        if reg.accounts[1..]
            .iter()
            .any(|a| a.name.to_lowercase() == default_name)
        {
            reg.accounts[0].name = DEFAULT_NAME.to_string();
        }
        match serde_json::from_value::<AccountDefaults>(file.defaults) {
            Ok(mut d) => {
                if reg.get(&d.new_session).is_none() {
                    if reg.held.ids.contains(&d.new_session) {
                        reg.held.new_session = Some(d.new_session.clone());
                    }
                    d.new_session = default_id();
                }
                if reg.get(&d.bridge).is_none() {
                    if reg.held.ids.contains(&d.bridge) {
                        reg.held.bridge = Some(d.bridge.clone());
                    }
                    d.bridge = default_id();
                }
                let mut held = BTreeMap::new();
                d.by_project.retain(|dir, id| {
                    let absolute = Path::new(dir).is_absolute();
                    if absolute && reg.held.ids.contains(id) {
                        held.insert(dir.clone(), id.clone());
                    }
                    absolute && reg.get(id).is_some()
                });
                reg.held.by_project = held;
                reg.defaults = d;
            }
            Err(e) => tracing::warn!(error = %e, "accounts.json: ignoring unreadable defaults"),
        }
        for raw in file.copy_consent {
            if let Ok(c) = serde_json::from_value::<CopyConsent>(raw.clone()) {
                let known = |id: &str| reg.get(id).is_some();
                let held = |id: &str| reg.held.ids.contains(id);
                if known(&c.from) && known(&c.to) {
                    if !reg.has_copy_consent(&c.from, &c.to) {
                        reg.copy_consent.push(c);
                    }
                } else if (held(&c.from) || known(&c.from)) && (held(&c.to) || known(&c.to)) {
                    reg.held.copy_consent.push(raw);
                }
            }
        }
        if let Ok(b) = serde_json::from_value::<BackupSettings>(file.backup) {
            if b.folder.as_deref().is_none_or(Path::is_absolute) {
                reg.backup = b;
            }
        }
        reg
    }
}

/// `accounts.json` as written: each part kept raw so one bad part never costs
/// the others.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegistryFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    accounts: Vec<Value>,
    #[serde(default)]
    defaults: Value,
    #[serde(default)]
    copy_consent: Vec<Value>,
    #[serde(default)]
    backup: Value,
    #[serde(flatten, default)]
    extra: BTreeMap<String, Value>,
}

/// Re-check an entry read from the file (the file is ours, but it may have
/// been edited or written by another version).
fn validate_loaded(reg: &Registry, a: &mut Account, default_dir: &Path) -> Result<(), String> {
    if !is_account_id(&a.id) {
        return Err("not a valid account id".into());
    }
    if reg.get(&a.id).is_some() {
        return Err("duplicate id".into());
    }
    a.name = clean_name(&a.name).map_err(|e| e.to_string())?;
    if name_taken(reg, &a.name, None) {
        return Err("duplicate name".into());
    }
    if a.color.as_deref().is_some_and(|c| !is_color(c)) {
        a.color = None;
    }
    if a.identity_sha256
        .as_deref()
        .is_some_and(|s| !is_sha256_hex(s))
    {
        a.identity_sha256 = None;
    }
    match a.kind {
        AccountKind::Directory => {
            let dir = a
                .dir
                .as_deref()
                .ok_or("a directory account without a dir")?;
            let dir = normalize_dir(dir).map_err(|e| e.to_string())?;
            check_dir_free(reg, &dir, default_dir, None).map_err(|e| e.to_string())?;
            a.dir = Some(dir);
            a.token_sha256 = None;
        }
        AccountKind::Token => {
            a.dir = None;
            if a.token_sha256
                .as_deref()
                .is_some_and(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                a.token_sha256 = None;
            }
        }
    }
    Ok(())
}

// ─────────────────────────────── validation ───────────────────────────────

/// Why a change to the registry was refused. Never carries a secret.
#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error("no account {0:?}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    #[error("an account named {0:?} already exists")]
    NameTaken(String),
    #[error("the Default account cannot be removed")]
    DefaultFixed,
    #[error("{0}")]
    DirUnavailable(String),
    #[error("this is not a token account")]
    NotToken,
    #[error("the account's token is not stored on this machine; enter it again")]
    TokenMissing,
    #[error(transparent)]
    Secret(#[from] SecretError),
    #[error("could not save the accounts: {0}")]
    Save(String),
}

impl AccountError {
    /// A stable code for API errors.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "account_not_found",
            Self::Invalid(_) => "invalid_account",
            Self::NameTaken(_) => "account_name_taken",
            Self::DefaultFixed => "default_account_fixed",
            Self::DirUnavailable(_) => "account_dir_unavailable",
            Self::NotToken => "not_a_token_account",
            Self::TokenMissing => "account_token_missing",
            Self::Secret(_) => "secret_store_failed",
            Self::Save(_) => "accounts_not_saved",
        }
    }
}

/// Trim and check a name: 1–40 characters, no control characters and nothing
/// that cannot be a folder name (it names the account's backup set).
pub fn clean_name(name: &str) -> Result<String, AccountError> {
    let name = name.trim();
    let n = name.chars().count();
    if n == 0 || n > NAME_MAX {
        return Err(AccountError::Invalid(format!(
            "a name has 1 to {NAME_MAX} characters"
        )));
    }
    if name.starts_with('.')
        || name.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
    {
        return Err(AccountError::Invalid(
            "a name cannot start with a dot or contain / \\ : * ? \" < > |".into(),
        ));
    }
    Ok(name.to_string())
}

fn name_taken(reg: &Registry, name: &str, except: Option<&str>) -> bool {
    let lower = name.to_lowercase();
    reg.accounts
        .iter()
        .any(|a| Some(a.id.as_str()) != except && a.name.to_lowercase() == lower)
}

/// `#rrggbb`.
pub fn is_color(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// An absolute path without `.`/`..`, a leading `~/` expanded, symlinks in
/// its existing part resolved (so `/tmp/x` and `/private/tmp/x` compare equal).
pub fn normalize_dir(dir: &Path) -> Result<PathBuf, AccountError> {
    let dir = match dir.strip_prefix("~") {
        Ok(rest) => user_home_dir()
            .ok_or_else(|| AccountError::Invalid("no home directory to expand ~ in".into()))?
            .join(rest),
        Err(_) => dir.to_path_buf(),
    };
    if !dir.is_absolute() {
        return Err(AccountError::Invalid(
            "the directory must be an absolute path".into(),
        ));
    }
    if dir
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(AccountError::Invalid(
            "the directory cannot contain . or ..".into(),
        ));
    }
    // Resolve the longest prefix that exists; keep the rest as given.
    let mut existing = dir.as_path();
    let mut rest: Vec<&OsStr> = Vec::new();
    while !existing.exists() {
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name);
                existing = parent;
            }
            _ => break,
        }
    }
    let mut out = existing
        .canonicalize()
        .unwrap_or_else(|_| existing.to_path_buf());
    for name in rest.iter().rev() {
        out.push(name);
    }
    Ok(out)
}

/// `dir` may become a directory account's: not the Default directory, not
/// the home directory or the root, and neither inside nor around another
/// account's directory (the CLI would read one inside the other).
fn check_dir_free(
    reg: &Registry,
    dir: &Path,
    default_dir: &Path,
    except: Option<&str>,
) -> Result<(), AccountError> {
    let default_dir = normalize_dir(default_dir).unwrap_or_else(|_| default_dir.to_path_buf());
    let overlaps = |other: &Path| dir.starts_with(other) || other.starts_with(dir);
    if overlaps(&default_dir) {
        return Err(AccountError::DirUnavailable(format!(
            "{} overlaps the Default account's directory {}",
            dir.display(),
            default_dir.display()
        )));
    }
    if dir.parent().is_none()
        || user_home_dir()
            .and_then(|h| normalize_dir(&h).ok())
            .is_some_and(|h| h == dir)
    {
        return Err(AccountError::DirUnavailable(
            "the home directory or the root cannot be an account's directory".into(),
        ));
    }
    for a in &reg.accounts {
        if Some(a.id.as_str()) == except {
            continue;
        }
        if let Some(other) = &a.dir {
            if overlaps(other) {
                return Err(AccountError::DirUnavailable(format!(
                    "{} overlaps the directory of account {:?}",
                    dir.display(),
                    a.name
                )));
            }
        }
    }
    if dir.exists() && !dir.is_dir() {
        return Err(AccountError::DirUnavailable(format!(
            "{} exists and is not a directory",
            dir.display()
        )));
    }
    Ok(())
}

/// Create an account's directory (0700) when it is missing.
fn ensure_dir(dir: &Path) -> Result<(), AccountError> {
    if dir.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(dir).map_err(|e| {
        AccountError::DirUnavailable(format!("could not create {}: {e}", dir.display()))
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

// ─────────────────────────────── the registry ─────────────────────────────

/// What `PATCH /accounts/{id}` may change.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountPatch {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
}

/// The registry and its secrets. Every change is saved before it returns.
pub struct AccountRegistry {
    path: Option<PathBuf>,
    default_home: ClaudeHome,
    secrets: Arc<dyn SecretStore>,
    data: RwLock<Registry>,
    /// Bumped on every change (for the `accounts` frame).
    generation: AtomicU64,
    /// What is known about each account now (sign-in, captures).
    pub runtime: status::AccountRuntime,
    /// Which token account a session ran under, across restarts.
    sessions: sessions::SessionAccounts,
}

impl fmt::Debug for AccountRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccountRegistry")
            .field("path", &self.path)
            .field("accounts", &self.read().accounts.len())
            .field("secrets", &self.secrets.kind())
            .finish()
    }
}

impl AccountRegistry {
    /// Load `<data dir>/accounts.json` (missing → only the Default). Without a
    /// data dir nothing is read or written.
    pub fn load(
        data_dir: Option<&Path>,
        default_home: ClaudeHome,
        secrets: Arc<dyn SecretStore>,
    ) -> Self {
        let path = data_dir.map(|d| d.join(FILE_NAME));
        let data = path
            .as_deref()
            .and_then(crate::persist::read_json::<RegistryFile>)
            .map(|file| Registry::from_file(file, default_home.base()))
            .unwrap_or_default();
        let sessions = sessions::SessionAccounts::load(data_dir, |id| {
            data.get(id).is_some_and(|a| a.kind == AccountKind::Token)
        });
        Self {
            path,
            default_home,
            secrets,
            data: RwLock::new(data),
            generation: AtomicU64::new(0),
            runtime: status::AccountRuntime::default(),
            sessions,
        }
    }

    /// Whether a token account's token is stored on this machine (the
    /// secret store is asked once per account, then remembered).
    pub fn has_token(&self, id: &str) -> bool {
        if let Some(known) = self
            .runtime
            .tokens
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
        {
            return *known;
        }
        let present = self
            .secrets
            .get(&SecretName::AccountToken(id.to_string()))
            .map(|t| t.is_some_and(|t| !t.is_empty()))
            .unwrap_or(false);
        self.note_token(id, present);
        present
    }

    fn note_token(&self, id: &str, present: bool) {
        self.runtime
            .tokens
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.to_string(), present);
    }

    fn read(&self) -> RwLockReadGuard<'_, Registry> {
        self.data.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> RwLockWriteGuard<'_, Registry> {
        self.data.write().unwrap_or_else(|e| e.into_inner())
    }

    /// Persist `next` and make it current; on a failed save nothing changes.
    fn commit(
        &self,
        guard: &mut RwLockWriteGuard<'_, Registry>,
        next: Registry,
    ) -> Result<(), AccountError> {
        if let Some(path) = &self.path {
            crate::persist::write_json_private(path, &next.to_file())
                .map_err(|e| AccountError::Save(e.to_string()))?;
        }
        **guard = next;
        self.generation.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// A copy of the whole registry.
    pub fn snapshot(&self) -> Registry {
        self.read().clone()
    }

    pub fn accounts(&self) -> Vec<Account> {
        self.read().accounts.clone()
    }

    pub fn get(&self, id: &str) -> Option<Account> {
        self.read().get(id).cloned()
    }

    pub fn is_default_only(&self) -> bool {
        self.read().is_default_only()
    }

    /// Changes so far (0 until the first one).
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    pub fn secret_store(&self) -> &Arc<dyn SecretStore> {
        &self.secrets
    }

    /// The Default account's home.
    pub fn default_home(&self) -> &ClaudeHome {
        &self.default_home
    }

    /// The config directory an account's processes use: its own for a
    /// directory account, the Default's for the Default and token accounts
    /// (and for a session that runs with a token no account holds, which
    /// lives in the Default's directory too).
    pub fn home(&self, id: &str) -> Option<ClaudeHome> {
        if id == UNREGISTERED_TOKEN_ID {
            return Some(self.default_home.clone());
        }
        let reg = self.read();
        let account = reg.get(id)?;
        Some(match (&account.kind, &account.dir) {
            (AccountKind::Directory, Some(dir)) => ClaudeHome::for_account_dir(dir),
            _ => self.default_home.clone(),
        })
    }

    /// Every distinct config directory: the Default's, then each directory
    /// account's (token accounts share the Default's).
    pub fn homes(&self) -> Vec<(String, ClaudeHome)> {
        let reg = self.read();
        let mut out = vec![(DEFAULT_ID.to_string(), self.default_home.clone())];
        for a in &reg.accounts {
            if let (AccountKind::Directory, Some(dir)) = (&a.kind, &a.dir) {
                out.push((a.id.clone(), ClaudeHome::for_account_dir(dir)));
            }
        }
        out
    }

    /// The directory account whose directory is `dir` (the Default's included).
    pub fn account_for_dir(&self, dir: &Path) -> Option<String> {
        let dir = normalize_dir(dir).unwrap_or_else(|_| dir.to_path_buf());
        self.homes().into_iter().find_map(|(id, home)| {
            let base = normalize_dir(home.base()).unwrap_or_else(|_| home.base().to_path_buf());
            (base == dir).then_some(id)
        })
    }

    /// See [`Registry::has_token_accounts`].
    pub fn has_token_accounts(&self) -> bool {
        self.read().has_token_accounts()
    }

    fn is_token_account(&self, id: &str) -> bool {
        self.read()
            .get(id)
            .is_some_and(|a| a.kind == AccountKind::Token)
    }

    /// A session was seen running under `account` (by a source that knows:
    /// its owned handle or bridge conversation, its pidfile's directory, its
    /// token). Remembered for this run; a token account's also across
    /// restarts ([`sessions`]).
    pub fn remember_session(&self, session_id: &str, account: &str) {
        self.runtime.remember(session_id, account);
        self.sessions
            .note(session_id, account, self.is_token_account(account));
    }

    /// The token account a session ran under in an earlier run of the app.
    pub fn remembered_token_account(&self, session_id: &str) -> Option<String> {
        self.sessions
            .get(session_id)
            .filter(|a| self.is_token_account(a))
    }

    /// Forget the remembered sessions whose transcript is gone (`exists`
    /// says which still are).
    pub fn prune_remembered_sessions(&self, exists: impl Fn(&str) -> bool) {
        self.sessions.prune(exists);
    }

    /// The account a token belongs to, by the token's SHA-256 (D8).
    pub fn account_for_token_sha256(&self, sha: &str) -> Option<String> {
        self.read().by_token_sha256(sha).map(|a| a.id.clone())
    }

    /// The account new work in `cwd` runs under.
    pub fn account_for_cwd(&self, cwd: &str) -> String {
        self.read().account_for_cwd(cwd)
    }

    /// The account a bridge conversation in `cwd` runs under when nothing
    /// names one: the project's, else the bridge default.
    pub fn account_for_bridge(&self, cwd: &str) -> String {
        let reg = self.read();
        reg.project_account(cwd)
            .unwrap_or_else(|| reg.defaults.bridge.clone())
    }

    /// Whether `id` is an account sessions can run under (not the
    /// unregistered-token pseudo-account, not a removed one).
    pub fn is_usable(&self, id: &str) -> bool {
        self.read().get(id).is_some()
    }

    pub fn defaults(&self) -> AccountDefaults {
        self.read().defaults.clone()
    }

    pub fn backup_settings(&self) -> BackupSettings {
        self.read().backup.clone()
    }

    pub fn has_copy_consent(&self, from: &str, to: &str) -> bool {
        self.read().has_copy_consent(from, to)
    }

    /// The environment a child of `id` gets (D5). A token account whose
    /// token is not on this machine is an error, never a silent fallback to
    /// the Default sign-in.
    pub fn env(&self, id: &str) -> Result<AccountEnv, AccountError> {
        let account = self
            .get(id)
            .ok_or_else(|| AccountError::NotFound(id.to_string()))?;
        match account.kind {
            _ if account.is_default() => Ok(AccountEnv::Inherit),
            AccountKind::Directory => account
                .dir
                .map(AccountEnv::Directory)
                .ok_or_else(|| AccountError::Invalid("the account has no directory".into())),
            AccountKind::Token => self
                .secrets
                .get(&SecretName::AccountToken(account.id.clone()))?
                .filter(|t| !t.is_empty())
                .map(AccountEnv::Token)
                .ok_or(AccountError::TokenMissing),
        }
    }

    /// Register a directory account; its directory is created (0700) when
    /// missing.
    pub fn add_directory(
        &self,
        name: &str,
        dir: &Path,
        color: Option<&str>,
    ) -> Result<Account, AccountError> {
        let name = clean_name(name)?;
        let dir = normalize_dir(dir)?;
        let mut guard = self.write();
        if name_taken(&guard, &name, None) {
            return Err(AccountError::NameTaken(name));
        }
        check_dir_free(&guard, &dir, self.default_home.base(), None)?;
        let color = pick_color(&guard, color)?;
        ensure_dir(&dir)?;
        let account = Account {
            id: new_account_id(),
            name,
            kind: AccountKind::Directory,
            dir: Some(dir),
            color: Some(color),
            created_at: now_ms(),
            hooks_plugin: None,
            token_sha256: None,
            account_uuid: None,
            identity_sha256: None,
            extra: BTreeMap::new(),
        };
        let mut next = guard.clone();
        next.accounts.push(account.clone());
        self.commit(&mut guard, next)?;
        Ok(account)
    }

    /// Register a token account. The token goes to the secret store; the
    /// registry keeps its SHA-256. Without a token the account waits for one
    /// (`setup-token` or a paste) and cannot run sessions until then.
    pub fn add_token(
        &self,
        name: &str,
        token: Option<&Secret>,
        color: Option<&str>,
    ) -> Result<Account, AccountError> {
        let name = clean_name(name)?;
        if let Some(token) = token {
            check_token_shape(token)?;
        }
        let mut guard = self.write();
        if name_taken(&guard, &name, None) {
            return Err(AccountError::NameTaken(name));
        }
        let sha = token.map(Secret::sha256_hex);
        if let Some(other) = sha.as_deref().and_then(|s| guard.by_token_sha256(s)) {
            return Err(AccountError::Invalid(format!(
                "this token is already account {:?}",
                other.name
            )));
        }
        let color = pick_color(&guard, color)?;
        let account = Account {
            id: new_account_id(),
            name,
            kind: AccountKind::Token,
            dir: None,
            color: Some(color),
            created_at: now_ms(),
            hooks_plugin: None,
            token_sha256: sha,
            account_uuid: None,
            identity_sha256: None,
            extra: BTreeMap::new(),
        };
        let secret = SecretName::AccountToken(account.id.clone());
        if let Some(token) = token {
            self.secrets.set(&secret, token)?;
        }
        let mut next = guard.clone();
        next.accounts.push(account.clone());
        if let Err(e) = self.commit(&mut guard, next) {
            if token.is_some() {
                let _ = self.secrets.delete(&secret);
            }
            return Err(e);
        }
        self.note_token(&account.id, token.is_some());
        Ok(account)
    }

    /// Replace a token account's token (a new `setup-token`, or a paste).
    pub fn set_token(&self, id: &str, token: &Secret) -> Result<Account, AccountError> {
        check_token_shape(token)?;
        let mut guard = self.write();
        let account = guard
            .get(id)
            .ok_or_else(|| AccountError::NotFound(id.to_string()))?;
        if account.kind != AccountKind::Token {
            return Err(AccountError::NotToken);
        }
        let sha = token.sha256_hex();
        if let Some(other) = guard.by_token_sha256(&sha).filter(|o| o.id != id) {
            return Err(AccountError::Invalid(format!(
                "this token is already account {:?}",
                other.name
            )));
        }
        let previous = self
            .secrets
            .get(&SecretName::AccountToken(id.to_string()))?;
        self.secrets
            .set(&SecretName::AccountToken(id.to_string()), token)?;
        self.note_token(id, true);
        let mut next = guard.clone();
        let entry = next.get_mut(id).expect("checked above");
        entry.token_sha256 = Some(sha);
        // A new token may belong to another Anthropic account.
        entry.account_uuid = None;
        let updated = entry.clone();
        if let Err(e) = self.commit(&mut guard, next) {
            let name = SecretName::AccountToken(id.to_string());
            match previous {
                Some(p) => {
                    let _ = self.secrets.set(&name, &p);
                }
                None => {
                    let _ = self.secrets.delete(&name);
                }
            }
            return Err(e);
        }
        Ok(updated)
    }

    /// Rename and/or recolour an account.
    pub fn update(&self, id: &str, patch: &AccountPatch) -> Result<Account, AccountError> {
        let mut guard = self.write();
        let mut next = guard.clone();
        let name = patch.name.as_deref().map(clean_name).transpose()?;
        if let Some(name) = &name {
            if name_taken(&next, name, Some(id)) {
                return Err(AccountError::NameTaken(name.clone()));
            }
        }
        if let Some(c) = &patch.color {
            if !is_color(c) {
                return Err(AccountError::Invalid("a colour is #rrggbb".into()));
            }
        }
        let entry = next
            .get_mut(id)
            .ok_or_else(|| AccountError::NotFound(id.to_string()))?;
        if let Some(name) = name {
            entry.name = name;
        }
        if let Some(c) = &patch.color {
            entry.color = Some(c.to_lowercase());
        }
        let updated = entry.clone();
        if next != *guard {
            self.commit(&mut guard, next)?;
        }
        Ok(updated)
    }

    /// Forget an account: never its directory or its Anthropic sign-in, only
    /// our record of it, its stored token, and the defaults that named it.
    pub fn remove(&self, id: &str) -> Result<Account, AccountError> {
        if id == DEFAULT_ID {
            return Err(AccountError::DefaultFixed);
        }
        let mut guard = self.write();
        let account = guard
            .get(id)
            .cloned()
            .ok_or_else(|| AccountError::NotFound(id.to_string()))?;
        let mut next = guard.clone();
        next.accounts.retain(|a| a.id != id);
        if next.defaults.new_session == id {
            next.defaults.new_session = default_id();
        }
        if next.defaults.bridge == id {
            next.defaults.bridge = default_id();
        }
        next.defaults.by_project.retain(|_, a| a != id);
        next.copy_consent.retain(|c| c.from != id && c.to != id);
        self.commit(&mut guard, next)?;
        drop(guard);
        if account.kind == AccountKind::Token {
            if let Err(e) = self.secrets.delete(&SecretName::AccountToken(id.into())) {
                tracing::warn!(id, error = %e, "could not delete a removed account's token");
            }
        }
        self.runtime.forget(id);
        self.sessions.forget_account(id);
        Ok(account)
    }

    /// Replace the defaults (every id must exist; `byProject` keys absolute).
    pub fn set_defaults(&self, defaults: AccountDefaults) -> Result<AccountDefaults, AccountError> {
        let mut guard = self.write();
        let mut clean = AccountDefaults {
            new_session: defaults.new_session,
            bridge: defaults.bridge,
            by_project: BTreeMap::new(),
        };
        for id in [&clean.new_session, &clean.bridge] {
            if guard.get(id).is_none() {
                return Err(AccountError::NotFound(id.clone()));
            }
        }
        for (dir, id) in defaults.by_project {
            if guard.get(&id).is_none() {
                return Err(AccountError::NotFound(id));
            }
            let path = Path::new(dir.trim());
            if !path.is_absolute()
                || path
                    .components()
                    .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
            {
                return Err(AccountError::Invalid(format!(
                    "{dir:?} is not an absolute project directory"
                )));
            }
            let key = path.to_string_lossy().trim_end_matches('/').to_string();
            let key = if key.is_empty() { "/".to_string() } else { key };
            clean.by_project.insert(key, id);
        }
        let mut next = guard.clone();
        next.defaults = clean.clone();
        // The user chose: a default kept for an account this run skipped
        // gives way (its project entries stay).
        next.held.new_session = None;
        next.held.bridge = None;
        if next != *guard {
            self.commit(&mut guard, next)?;
        }
        Ok(clean)
    }

    /// Remember (or forget) "copy from `from` to `to` without asking".
    pub fn set_copy_consent(
        &self,
        from: &str,
        to: &str,
        granted: bool,
    ) -> Result<(), AccountError> {
        let mut guard = self.write();
        for id in [from, to] {
            if guard.get(id).is_none() {
                return Err(AccountError::NotFound(id.to_string()));
            }
        }
        let mut next = guard.clone();
        next.copy_consent
            .retain(|c| !(c.from == from && c.to == to));
        if granted {
            next.copy_consent.push(CopyConsent {
                from: from.to_string(),
                to: to.to_string(),
            });
        }
        if next != *guard {
            self.commit(&mut guard, next)?;
        }
        Ok(())
    }

    /// Replace the backup settings.
    pub fn set_backup_settings(&self, settings: BackupSettings) -> Result<(), AccountError> {
        if settings.folder.as_deref().is_some_and(|f| !f.is_absolute()) {
            return Err(AccountError::Invalid(
                "the backup folder must be an absolute path".into(),
            ));
        }
        let mut guard = self.write();
        let mut next = guard.clone();
        next.backup = settings;
        if next != *guard {
            self.commit(&mut guard, next)?;
        }
        Ok(())
    }

    /// Change the backup settings in place (`f` sees the current ones under
    /// the registry's lock, so two changes never lose each other); saved when
    /// `f` succeeds and changed something.
    pub fn update_backup<T>(
        &self,
        f: impl FnOnce(&mut BackupSettings) -> Result<T, AccountError>,
    ) -> Result<T, AccountError> {
        let mut guard = self.write();
        let mut next = guard.clone();
        let out = f(&mut next.backup)?;
        if next
            .backup
            .folder
            .as_deref()
            .is_some_and(|f| !f.is_absolute())
        {
            return Err(AccountError::Invalid(
                "the backup folder must be an absolute path".into(),
            ));
        }
        if next != *guard {
            self.commit(&mut guard, next)?;
        }
        Ok(out)
    }

    /// Record the Anthropic account id an account's process reported. Saved
    /// only when it changed.
    pub fn note_account_uuid(&self, id: &str, uuid: &str) {
        let uuid = uuid.trim();
        if uuid.is_empty() || uuid.len() > 128 {
            return;
        }
        let mut guard = self.write();
        if guard
            .get(id)
            .is_none_or(|a| a.account_uuid.as_deref() == Some(uuid))
        {
            return;
        }
        let mut next = guard.clone();
        if let Some(a) = next.get_mut(id) {
            a.account_uuid = Some(uuid.to_string());
        }
        if let Err(e) = self.commit(&mut guard, next) {
            tracing::debug!(error = %e, "could not record an account id");
        }
    }

    /// Record the hooks plugin's state in an account's directory.
    pub fn note_hooks_plugin(&self, id: &str, state: &str) {
        let mut guard = self.write();
        if guard
            .get(id)
            .is_none_or(|a| a.hooks_plugin.as_deref() == Some(state))
        {
            return;
        }
        let mut next = guard.clone();
        if let Some(a) = next.get_mut(id) {
            a.hooks_plugin = Some(state.to_string());
        }
        if let Err(e) = self.commit(&mut guard, next) {
            tracing::debug!(error = %e, "could not record the plugin state");
        }
    }

    /// A registry with exactly this content, for tests elsewhere.
    #[cfg(test)]
    pub fn replace_for_test(&self, reg: Registry) {
        *self.write() = reg;
        self.generation.fetch_add(1, Ordering::Relaxed);
    }
}

fn pick_color(reg: &Registry, asked: Option<&str>) -> Result<String, AccountError> {
    if let Some(c) = asked {
        if !is_color(c) {
            return Err(AccountError::Invalid("a colour is #rrggbb".into()));
        }
        return Ok(c.to_lowercase());
    }
    let used: Vec<&str> = reg
        .accounts
        .iter()
        .filter_map(|a| a.color.as_deref())
        .collect();
    Ok(PALETTE
        .iter()
        .find(|c| !used.contains(c))
        .copied()
        .unwrap_or(PALETTE[(reg.accounts.len() - 1) % PALETTE.len()])
        .to_string())
}

/// A token is one line of printable ASCII without spaces — enough to keep a
/// pasted paragraph or a newline out of a child's environment. Whether it
/// works is for `claude auth status` to say.
fn check_token_shape(token: &Secret) -> Result<(), AccountError> {
    let t = token.expose();
    if t.len() < 20 || t.len() > 512 || !t.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(AccountError::Invalid(
            "that does not look like a token from `claude setup-token`".into(),
        ));
    }
    Ok(())
}

// ─────────────────────────── a child's environment ─────────────────────────

/// How a child's environment is composed for one account (D5).
#[derive(Clone, Default, PartialEq, Eq)]
pub enum AccountEnv {
    /// The Default account: our environment as the app found it.
    #[default]
    Inherit,
    /// A directory account: `CLAUDE_CONFIG_DIR` set to it, overrides removed.
    Directory(PathBuf),
    /// A token account: overrides removed, `CLAUDE_CODE_OAUTH_TOKEN` set.
    Token(Secret),
}

impl fmt::Debug for AccountEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Inherit => f.write_str("Inherit"),
            Self::Directory(d) => f.debug_tuple("Directory").field(d).finish(),
            Self::Token(_) => f.write_str("Token(…)"),
        }
    }
}

/// Something a child's environment is built on.
pub trait EnvTarget {
    fn set_var(&mut self, key: &str, value: &OsStr);
    fn remove_var(&mut self, key: &str);
}

impl EnvTarget for tokio::process::Command {
    fn set_var(&mut self, key: &str, value: &OsStr) {
        self.env(key, value);
    }
    fn remove_var(&mut self, key: &str) {
        self.env_remove(key);
    }
}

impl EnvTarget for std::process::Command {
    fn set_var(&mut self, key: &str, value: &OsStr) {
        self.env(key, value);
    }
    fn remove_var(&mut self, key: &str) {
        self.env_remove(key);
    }
}

/// What an account changes in an environment that is otherwise inherited: the
/// shape the bridge host applies to the SDK's per-query `Options.env`.
#[derive(Clone, Default, PartialEq, Eq, Serialize)]
pub struct EnvOverrides {
    pub set: BTreeMap<String, String>,
    pub unset: Vec<String>,
}

impl fmt::Debug for EnvOverrides {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnvOverrides")
            .field("set", &self.set.keys().collect::<Vec<_>>())
            .field("unset", &self.unset)
            .finish()
    }
}

impl AccountEnv {
    /// Apply to a child's environment (after the scrub, before the spawn).
    pub fn apply(&self, target: &mut dyn EnvTarget) {
        match self {
            Self::Inherit => {}
            Self::Directory(dir) => {
                for key in ACCOUNT_OVERRIDE_ENV {
                    target.remove_var(key);
                }
                target.set_var(CONFIG_DIR_ENV, dir.as_os_str());
            }
            Self::Token(token) => {
                for key in ACCOUNT_OVERRIDE_ENV {
                    target.remove_var(key);
                }
                target.set_var(OAUTH_TOKEN_ENV, OsStr::new(token.expose()));
            }
        }
    }

    /// The same change as data, for a process that builds the environment
    /// itself (the bridge host, per query).
    pub fn overrides(&self) -> EnvOverrides {
        let mut out = EnvOverrides::default();
        match self {
            Self::Inherit => {}
            Self::Directory(dir) => {
                out.unset = ACCOUNT_OVERRIDE_ENV.iter().map(|s| s.to_string()).collect();
                out.set.insert(
                    CONFIG_DIR_ENV.to_string(),
                    dir.to_string_lossy().into_owned(),
                );
            }
            Self::Token(token) => {
                out.unset = ACCOUNT_OVERRIDE_ENV
                    .iter()
                    .filter(|k| **k != OAUTH_TOKEN_ENV)
                    .map(|s| s.to_string())
                    .collect();
                out.set
                    .insert(OAUTH_TOKEN_ENV.to_string(), token.expose().to_string());
            }
        }
        out
    }

    pub fn is_inherit(&self) -> bool {
        matches!(self, Self::Inherit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::secrets::MemoryStore;

    fn registry(dir: &Path) -> (AccountRegistry, PathBuf) {
        let default_dir = dir.join("default-claude");
        std::fs::create_dir_all(&default_dir).unwrap();
        let reg = AccountRegistry::load(
            Some(dir),
            ClaudeHome::with_base(&default_dir),
            Arc::new(MemoryStore::default()),
        );
        (reg, default_dir)
    }

    fn token(n: u8) -> Secret {
        Secret::new(format!("sk-ant-oat01-{}", format!("{n:02}").repeat(20)))
    }

    #[test]
    fn ids_are_default_or_acc_hex() {
        assert!(is_valid_id("default"));
        assert!(is_valid_id("acc_0123456789ab"));
        assert!(is_account_id(&new_account_id()));
        for bad in [
            "acc_0123456789AB",
            "acc_0123456789a",
            "acc_0123456789abc",
            "acc_",
            "x",
        ] {
            assert!(!is_valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn a_missing_file_is_only_the_default_and_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let (reg, default_dir) = registry(tmp.path());
        assert!(reg.is_default_only());
        let snap = reg.snapshot();
        assert_eq!(snap.accounts[0].id, DEFAULT_ID);
        assert_eq!(snap.defaults.new_session, DEFAULT_ID);
        assert_eq!(reg.account_for_cwd("/anything"), DEFAULT_ID);
        assert_eq!(reg.env(DEFAULT_ID).unwrap(), AccountEnv::Inherit);
        assert_eq!(reg.home(DEFAULT_ID).unwrap().base(), default_dir);
        assert_eq!(reg.homes().len(), 1);
        assert!(!tmp.path().join(FILE_NAME).exists(), "goal 7: no file");
    }

    #[test]
    fn accounts_round_trip_and_the_file_is_private() {
        let tmp = tempfile::tempdir().unwrap();
        let (reg, default_dir) = registry(tmp.path());
        let work_dir = tmp.path().join("claude-work");
        let work = reg.add_directory("  Work ", &work_dir, None).unwrap();
        assert_eq!(work.name, "Work");
        assert!(work_dir.is_dir(), "created");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&work_dir), 0o700);
            assert_eq!(mode(&tmp.path().join(FILE_NAME)), 0o600);
        }
        let pro = reg
            .add_token("Marcus (Pro)", Some(&token(1)), Some("#F59E0B"))
            .unwrap();
        assert_eq!(pro.color.as_deref(), Some("#f59e0b"));
        assert_eq!(
            pro.token_sha256.as_deref(),
            Some(token(1).sha256_hex().as_str())
        );
        reg.set_defaults(AccountDefaults {
            new_session: pro.id.clone(),
            bridge: work.id.clone(),
            by_project: [("/p/gitweaver/".to_string(), work.id.clone())].into(),
        })
        .unwrap();
        reg.set_copy_consent(DEFAULT_ID, &work.id, true).unwrap();

        // The secret is not in the file.
        let text = std::fs::read_to_string(tmp.path().join(FILE_NAME)).unwrap();
        assert!(!text.contains(token(1).expose()));
        assert!(text.contains(&token(1).sha256_hex()));

        // A second load sees the same registry (the store is per test).
        let again = AccountRegistry::load(
            Some(tmp.path()),
            ClaudeHome::with_base(&default_dir),
            reg.secret_store().clone(),
        );
        assert_eq!(again.snapshot(), reg.snapshot());
        assert_eq!(again.account_for_cwd("/p/gitweaver/src"), work.id);
        assert_eq!(again.account_for_cwd("/p/gitweaverx"), pro.id);
        assert!(again.has_copy_consent(DEFAULT_ID, &work.id));
        assert!(!again.has_copy_consent(&work.id, DEFAULT_ID));
        assert_eq!(
            again.homes().len(),
            2,
            "the token account shares the Default's"
        );
        assert_eq!(
            again.account_for_dir(&work_dir).as_deref(),
            Some(work.id.as_str())
        );
        assert_eq!(
            again.account_for_dir(&default_dir).as_deref(),
            Some(DEFAULT_ID)
        );
        assert_eq!(
            again.account_for_token_sha256(&token(1).sha256_hex()),
            Some(pro.id.clone())
        );
        assert_eq!(again.env(&pro.id).unwrap(), AccountEnv::Token(token(1)));
        assert_eq!(
            again.env(&work.id).unwrap(),
            AccountEnv::Directory(normalize_dir(&work_dir).unwrap())
        );
        assert_eq!(
            again.home(&work.id).unwrap().claude_json(),
            normalize_dir(&work_dir).unwrap().join(".claude.json")
        );
    }

    #[test]
    fn names_and_directories_are_validated() {
        let tmp = tempfile::tempdir().unwrap();
        let (reg, default_dir) = registry(tmp.path());
        reg.add_directory("Work", &tmp.path().join("w"), None)
            .unwrap();
        for bad in ["", "   ", ".hidden", "a/b", "x:y", &"n".repeat(41)] {
            assert!(
                matches!(
                    reg.add_directory(bad, &tmp.path().join("other"), None),
                    Err(AccountError::Invalid(_))
                ),
                "{bad:?}"
            );
        }
        assert!(matches!(
            reg.add_directory("work", &tmp.path().join("other"), None),
            Err(AccountError::NameTaken(_))
        ));
        assert!(matches!(
            reg.add_directory("Default", &tmp.path().join("other"), None),
            Err(AccountError::NameTaken(_))
        ));
        for dir in [
            default_dir.clone(),
            default_dir.join("inside"),
            tmp.path().to_path_buf(),
            tmp.path().join("w"),
            tmp.path().join("w/deeper"),
        ] {
            assert!(
                matches!(
                    reg.add_directory("Other", &dir, None),
                    Err(AccountError::DirUnavailable(_))
                ),
                "{}",
                dir.display()
            );
        }
        assert!(matches!(
            reg.add_directory("Other", Path::new("relative/dir"), None),
            Err(AccountError::Invalid(_))
        ));
        assert!(matches!(
            reg.add_directory("Other", &tmp.path().join("a/../b"), None),
            Err(AccountError::Invalid(_))
        ));
        std::fs::write(tmp.path().join("file"), "x").unwrap();
        assert!(matches!(
            reg.add_directory("Other", &tmp.path().join("file"), None),
            Err(AccountError::DirUnavailable(_))
        ));
        assert!(matches!(
            reg.add_token("Short", Some(&Secret::new("abc")), None),
            Err(AccountError::Invalid(_))
        ));
        assert!(matches!(
            reg.add_token(
                "Spaces",
                Some(&Secret::new(format!("{} x", "a".repeat(30)))),
                None
            ),
            Err(AccountError::Invalid(_))
        ));
        reg.add_token("T1", Some(&token(2)), None).unwrap();
        assert!(matches!(
            reg.add_token("T2", Some(&token(2)), None),
            Err(AccountError::Invalid(_))
        ));
        assert!(matches!(
            reg.update("acc_000000000000", &AccountPatch::default()),
            Err(AccountError::NotFound(_))
        ));
        assert!(matches!(
            reg.update(
                DEFAULT_ID,
                &AccountPatch {
                    color: Some("red".into()),
                    ..Default::default()
                }
            ),
            Err(AccountError::Invalid(_))
        ));
        assert!(matches!(
            reg.remove(DEFAULT_ID),
            Err(AccountError::DefaultFixed)
        ));
    }

    #[test]
    fn removing_an_account_cleans_up_after_it() {
        let tmp = tempfile::tempdir().unwrap();
        let (reg, _) = registry(tmp.path());
        let t = reg.add_token("T", Some(&token(3)), None).unwrap();
        let w = reg.add_directory("W", &tmp.path().join("w"), None).unwrap();
        reg.set_defaults(AccountDefaults {
            new_session: t.id.clone(),
            bridge: t.id.clone(),
            by_project: [
                ("/p".to_string(), t.id.clone()),
                ("/q".to_string(), w.id.clone()),
            ]
            .into(),
        })
        .unwrap();
        reg.set_copy_consent(&t.id, &w.id, true).unwrap();
        let generation = reg.generation();
        reg.remove(&t.id).unwrap();
        assert!(reg.generation() > generation);
        let d = reg.defaults();
        assert_eq!(d.new_session, DEFAULT_ID);
        assert_eq!(d.bridge, DEFAULT_ID);
        assert_eq!(d.by_project.len(), 1);
        assert!(!reg.has_copy_consent(&t.id, &w.id));
        assert!(reg
            .secret_store()
            .get(&SecretName::AccountToken(t.id.clone()))
            .unwrap()
            .is_none());
        assert!(
            tmp.path().join("w").is_dir(),
            "its directory is never deleted"
        );
        assert!(matches!(reg.env(&t.id), Err(AccountError::NotFound(_))));
    }

    #[test]
    fn a_token_account_without_its_token_is_an_error_not_the_default() {
        let tmp = tempfile::tempdir().unwrap();
        let (reg, default_dir) = registry(tmp.path());
        let t = reg.add_token("T", Some(&token(4)), None).unwrap();
        // Another machine: same registry, empty secret store.
        let elsewhere = AccountRegistry::load(
            Some(tmp.path()),
            ClaudeHome::with_base(&default_dir),
            Arc::new(MemoryStore::default()),
        );
        assert!(matches!(
            elsewhere.env(&t.id),
            Err(AccountError::TokenMissing)
        ));
        elsewhere.set_token(&t.id, &token(5)).unwrap();
        assert_eq!(elsewhere.env(&t.id).unwrap(), AccountEnv::Token(token(5)));
        assert_eq!(
            elsewhere.get(&t.id).unwrap().token_sha256,
            Some(token(5).sha256_hex())
        );
        assert!(matches!(
            elsewhere.set_token(DEFAULT_ID, &token(6)),
            Err(AccountError::NotToken)
        ));
    }

    #[test]
    fn a_hand_edited_file_loses_only_its_bad_parts() {
        let tmp = tempfile::tempdir().unwrap();
        let default_dir = tmp.path().join("default-claude");
        std::fs::create_dir_all(&default_dir).unwrap();
        let work = tmp.path().join("work");
        let kept = "ab".repeat(32);
        let file = serde_json::json!({
            "version": 1,
            "futureTopLevel": {"keep": true},
            "accounts": [
                {"id": "default", "name": "Personal", "kind": "token", "dir": "/elsewhere", "color": "#123456",
                 "identitySha256": kept},
                {"id": "acc_aaaaaaaaaaaa", "name": "Work", "kind": "directory", "dir": work, "createdAt": 5, "futureField": 1,
                 "identitySha256": "not a hash"},
                {"id": "acc_bbbbbbbbbbbb", "name": "work", "kind": "token"},
                {"id": "acc_cccccccccccc", "name": "Nested", "kind": "directory", "dir": work.join("inner")},
                {"id": "acc_dddddddddddd", "name": "Relative", "kind": "directory", "dir": "rel"},
                {"id": "bogus", "name": "Bad id", "kind": "token"},
                {"id": "acc_eeeeeeeeeeee", "name": "Pro", "kind": "token", "tokenSha256": "nothex", "dir": "/ignored"},
                {"id": "acc_ffffffffffff", "kind": "directory"},
                42
            ],
            "defaults": {"newSession": "acc_bbbbbbbbbbbb", "bridge": "acc_eeeeeeeeeeee",
                         "byProject": {"/p": "acc_aaaaaaaaaaaa", "rel": "acc_aaaaaaaaaaaa", "/q": "acc_nope00000000"}},
            "copyConsent": [{"from": "default", "to": "acc_aaaaaaaaaaaa"}, {"from": "x", "to": "y"}, "junk"],
            "backup": {"folder": "/backups", "everyHours": 6, "futureBackup": "x"}
        });
        std::fs::write(tmp.path().join(FILE_NAME), file.to_string()).unwrap();
        let reg = AccountRegistry::load(
            Some(tmp.path()),
            ClaudeHome::with_base(&default_dir),
            Arc::new(MemoryStore::default()),
        );
        let snap = reg.snapshot();
        let ids: Vec<&str> = snap.accounts.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["default", "acc_aaaaaaaaaaaa", "acc_eeeeeeeeeeee"]);
        let default = &snap.accounts[0];
        assert_eq!(default.name, "Personal");
        assert_eq!(default.kind, AccountKind::Directory);
        assert!(default.dir.is_none());
        assert_eq!(default.color.as_deref(), Some("#123456"));
        assert_eq!(default.identity_sha256.as_deref(), Some(kept.as_str()));
        assert_eq!(
            snap.accounts[1].identity_sha256, None,
            "not a hash: dropped"
        );
        assert_eq!(snap.accounts[1].extra["futureField"], 1);
        assert!(snap.accounts[2].token_sha256.is_none());
        assert!(snap.accounts[2].dir.is_none());
        assert_eq!(snap.defaults.new_session, DEFAULT_ID);
        assert_eq!(snap.defaults.bridge, "acc_eeeeeeeeeeee");
        assert_eq!(snap.defaults.by_project.len(), 1);
        assert_eq!(snap.copy_consent.len(), 1);
        assert_eq!(snap.backup.every_hours, 6);
        assert_eq!(snap.backup.extra["futureBackup"], "x");
        // Unknown parts survive the next save.
        reg.update(
            DEFAULT_ID,
            &AccountPatch {
                color: Some("#654321".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let text = std::fs::read_to_string(tmp.path().join(FILE_NAME)).unwrap();
        assert!(text.contains("futureTopLevel") && text.contains("futureField"));

        // Garbage is only the Default.
        std::fs::write(tmp.path().join(FILE_NAME), "{not json").unwrap();
        let reg = AccountRegistry::load(
            Some(tmp.path()),
            ClaudeHome::with_base(&default_dir),
            Arc::new(MemoryStore::default()),
        );
        assert!(reg.is_default_only());
    }

    #[test]
    fn accounts_a_run_cannot_use_survive_its_saves() {
        let tmp = tempfile::tempdir().unwrap();
        let usual = tmp.path().join("claude-usual");
        let other = tmp.path().join("claude-other");
        std::fs::create_dir_all(&usual).unwrap();
        let reg = AccountRegistry::load(
            Some(tmp.path()),
            ClaudeHome::with_base(&usual),
            Arc::new(MemoryStore::default()),
        );
        let work = reg.add_directory("Work", &other, None).unwrap();
        let pro = reg.add_token("Pro", Some(&token(3)), None).unwrap();
        reg.set_defaults(AccountDefaults {
            new_session: work.id.clone(),
            bridge: work.id.clone(),
            by_project: [("/p/acme".to_string(), work.id.clone())].into(),
        })
        .unwrap();
        reg.set_copy_consent(DEFAULT_ID, &work.id, true).unwrap();
        reg.set_copy_consent(&work.id, &pro.id, true).unwrap();
        let store = reg.secret_store().clone();

        // A run started with CLAUDE_CONFIG_DIR=<Work's directory>: there
        // Work's directory is the Default's, so Work is skipped.
        let odd = AccountRegistry::load(
            Some(tmp.path()),
            ClaudeHome::with_base(&other),
            store.clone(),
        );
        let ids: Vec<String> = odd.accounts().into_iter().map(|a| a.id).collect();
        assert_eq!(ids, [DEFAULT_ID.to_string(), pro.id.clone()]);
        assert_eq!(odd.defaults().new_session, DEFAULT_ID);
        assert_eq!(odd.account_for_cwd("/p/acme/x"), DEFAULT_ID);
        assert!(!odd.has_copy_consent(DEFAULT_ID, &work.id));
        // A save nobody asked for (a plugin state learned at startup).
        odd.note_hooks_plugin(DEFAULT_ID, "installed");

        // The usual run again: nothing was lost.
        let back = AccountRegistry::load(Some(tmp.path()), ClaudeHome::with_base(&usual), store);
        assert_eq!(back.get(&work.id).unwrap().dir, work.dir);
        assert_eq!(back.defaults().new_session, work.id);
        assert_eq!(back.defaults().bridge, work.id);
        assert_eq!(back.account_for_cwd("/p/acme/x"), work.id);
        assert!(back.has_copy_consent(DEFAULT_ID, &work.id));
        assert!(back.has_copy_consent(&work.id, &pro.id));
        assert_eq!(
            back.get(DEFAULT_ID).unwrap().hooks_plugin.as_deref(),
            Some("installed")
        );

        // Choosing the defaults in the odd run replaces the held ones; the
        // project entry stays.
        let odd = AccountRegistry::load(
            Some(tmp.path()),
            ClaudeHome::with_base(&other),
            back.secret_store().clone(),
        );
        odd.set_defaults(AccountDefaults {
            new_session: pro.id.clone(),
            ..Default::default()
        })
        .unwrap();
        let back = AccountRegistry::load(
            Some(tmp.path()),
            ClaudeHome::with_base(&usual),
            odd.secret_store().clone(),
        );
        assert_eq!(back.defaults().new_session, pro.id);
        assert_eq!(back.defaults().bridge, DEFAULT_ID);
        assert_eq!(back.account_for_cwd("/p/acme/x"), work.id);
    }

    #[test]
    fn learned_facts_are_saved_only_when_they_change() {
        let tmp = tempfile::tempdir().unwrap();
        let (reg, _) = registry(tmp.path());
        let g = reg.generation();
        reg.note_account_uuid(DEFAULT_ID, "uuid-1");
        assert_eq!(reg.generation(), g + 1);
        reg.note_account_uuid(DEFAULT_ID, "uuid-1");
        reg.note_account_uuid("acc_000000000000", "uuid-2");
        reg.note_account_uuid(DEFAULT_ID, "  ");
        assert_eq!(reg.generation(), g + 1);
        reg.note_hooks_plugin(DEFAULT_ID, "installed");
        reg.note_hooks_plugin(DEFAULT_ID, "installed");
        assert_eq!(reg.generation(), g + 2);
        assert_eq!(
            reg.get(DEFAULT_ID).unwrap().account_uuid.as_deref(),
            Some("uuid-1")
        );
    }

    #[test]
    fn backup_settings_need_an_absolute_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let (reg, _) = registry(tmp.path());
        assert!(reg
            .set_backup_settings(BackupSettings {
                folder: Some("relative".into()),
                ..Default::default()
            })
            .is_err());
        reg.set_backup_settings(BackupSettings {
            folder: Some("/backups".into()),
            every_hours: 6,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(reg.backup_settings().every_hours, 6);
    }

    /// §13.3 rule 8: storages are read tolerantly — a bad entry, or a list
    /// that is not one, never costs the backup folder — and left out of the
    /// file while there are none.
    #[test]
    fn storages_are_read_tolerantly() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(FILE_NAME),
            serde_json::json!({
                "version": 1,
                "accounts": [],
                "backup": {
                    "folder": "/backups",
                    "storages": [
                        {"id": "stor_0123456789ab", "label": "Disk", "path": "/Volumes/Disk/Claude",
                         "createdAt": 1, "fromTheFuture": true},
                        {"id": "stor_0123456789ab", "label": "Twice", "path": "/twice"},
                        {"id": "bad", "label": "Bad id", "path": "/bad"},
                        {"id": "stor_aaaaaaaaaaaa", "label": "Relative", "path": "relative"},
                        "not an object"
                    ]
                }
            })
            .to_string(),
        )
        .unwrap();
        let (reg, _) = registry(tmp.path());
        let b = reg.backup_settings();
        assert_eq!(b.folder.as_deref(), Some(Path::new("/backups")));
        assert_eq!(b.storages.len(), 1, "{:?}", b.storages);
        assert_eq!(b.storages[0].label, "Disk");
        assert_eq!(b.storages[0].extra["fromTheFuture"], true, "kept as it was");

        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(FILE_NAME),
            serde_json::json!({"backup": {"folder": "/backups", "storages": {"x": 1}}}).to_string(),
        )
        .unwrap();
        let (reg, _) = registry(tmp.path());
        assert_eq!(
            reg.backup_settings().folder.as_deref(),
            Some(Path::new("/backups"))
        );
        assert!(reg.backup_settings().storages.is_empty());
        let file = serde_json::to_value(reg.backup_settings()).unwrap();
        assert!(file.get("storages").is_none(), "{file}");
    }

    /// Records what an [`EnvTarget`] was told, in order.
    #[derive(Default)]
    struct Recorder(Vec<(String, Option<String>)>);

    impl EnvTarget for Recorder {
        fn set_var(&mut self, key: &str, value: &OsStr) {
            self.0
                .push((key.to_string(), Some(value.to_string_lossy().into_owned())));
        }
        fn remove_var(&mut self, key: &str) {
            self.0.push((key.to_string(), None));
        }
    }

    impl Recorder {
        /// The final value per key (`Some(None)` = removed).
        fn result(&self) -> BTreeMap<String, Option<String>> {
            self.0.iter().cloned().collect()
        }
    }

    #[test]
    fn the_environment_is_composed_per_kind() {
        let dir = PathBuf::from("/Users/x/.claude-work");
        let tok = token(7);
        type Expect = fn(&BTreeMap<String, Option<String>>, &str);
        let cases: [(AccountEnv, Expect); 3] = [
            (AccountEnv::Inherit, |r, _| {
                assert!(r.is_empty(), "the Default changes nothing: {r:?}");
            }),
            (AccountEnv::Directory(dir.clone()), |r, _| {
                assert_eq!(r[CONFIG_DIR_ENV].as_deref(), Some("/Users/x/.claude-work"));
                for k in ACCOUNT_OVERRIDE_ENV {
                    assert_eq!(r[k], None, "{k} removed");
                }
            }),
            (AccountEnv::Token(tok.clone()), |r, t| {
                assert!(!r.contains_key(CONFIG_DIR_ENV), "left as found");
                assert_eq!(r[OAUTH_TOKEN_ENV].as_deref(), Some(t));
                for k in ACCOUNT_OVERRIDE_ENV
                    .iter()
                    .filter(|k| **k != OAUTH_TOKEN_ENV)
                {
                    assert_eq!(r[*k], None, "{k} removed");
                }
            }),
        ];
        for (env, expect) in cases {
            let mut rec = Recorder::default();
            env.apply(&mut rec);
            expect(&rec.result(), tok.expose());
            // The data form says the same.
            let o = env.overrides();
            let mut from_data = Recorder::default();
            for k in &o.unset {
                from_data.remove_var(k);
            }
            for (k, v) in &o.set {
                from_data.set_var(k, OsStr::new(v));
            }
            expect(&from_data.result(), tok.expose());
            // Neither form prints the token.
            assert!(!format!("{env:?} {o:?}").contains(tok.expose()));
        }
    }
}
