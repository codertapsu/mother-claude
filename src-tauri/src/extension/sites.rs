//! Site modes, the sensitive-site list, the browser settings and standing
//! grants (docs/BROWSER.md, docs/BROWSER-PROTOCOL.md "State the desktop owns").
//!
//! * A **site mode** is `off` (nothing), `read` (observe only) or `act`
//!   (observe and act), kept per key: `*` (the default for every other
//!   site), an origin `scheme://host[:port]` (default ports dropped), or a
//!   wildcard `scheme://*.domain[:port]` (every subdomain of `domain`, not
//!   `domain` itself). Resolution: the exact origin, else the longest
//!   matching wildcard, else `*`.
//! * **Sensitive sites** (banking, payment, identity providers; the built-in
//!   [`SENSITIVE`] list, matched by domain suffix) are `off` unless the user
//!   set that exact origin: neither `*` nor a wildcard ever opens them, and
//!   only the desktop (or a device holding the dangerous right) raises one to
//!   `act`. Every navigation to one and every action on one still needs an
//!   approval.
//! * **Standing grants** ("always allow `eval` on `https://localhost:3000`")
//!   skip one kind of approval on one exact origin. Creating one is itself a
//!   dangerous setting (desktop or a granted device), audited and revocable.
//!
//! Persisted in `<app data dir>/browser-settings.json` (0600, atomic). A
//! corrupt or partly unknown file never fails the load: unknown keys survive,
//! keys that do not parse are dropped.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What a session may do on a site.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum SiteMode {
    /// Nothing: not even reading its title.
    #[default]
    Off,
    /// Observe only: snapshots, reading, finding, screenshots, scrolling.
    Read,
    /// Observe and act: clicks, typing, forms, scripts (some need approval).
    Act,
}

impl SiteMode {
    pub fn as_str(self) -> &'static str {
        match self {
            SiteMode::Off => "off",
            SiteMode::Read => "read",
            SiteMode::Act => "act",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" => Some(SiteMode::Off),
            "read" => Some(SiteMode::Read),
            "act" => Some(SiteMode::Act),
            _ => None,
        }
    }
}

/// The key of the default mode.
pub const DEFAULT_KEY: &str = "*";

/// The built-in sensitive sites: `(domain, category)`, matched by domain
/// suffix (`paypal.com` covers `www.paypal.com`). Deliberately modest:
/// well-known payment providers, banks and brokers, identity providers and
/// password managers. Documented in docs/BROWSER.md.
pub const SENSITIVE: &[(&str, &str)] = &[
    // Payment
    ("paypal.com", "payment"),
    ("venmo.com", "payment"),
    ("stripe.com", "payment"),
    ("squareup.com", "payment"),
    ("wise.com", "payment"),
    ("revolut.com", "payment"),
    ("klarna.com", "payment"),
    ("pay.google.com", "payment"),
    ("payments.google.com", "payment"),
    ("coinbase.com", "payment"),
    ("binance.com", "payment"),
    ("kraken.com", "payment"),
    // Banking and brokerage
    ("chase.com", "banking"),
    ("bankofamerica.com", "banking"),
    ("wellsfargo.com", "banking"),
    ("citi.com", "banking"),
    ("capitalone.com", "banking"),
    ("usbank.com", "banking"),
    ("americanexpress.com", "banking"),
    ("discover.com", "banking"),
    ("schwab.com", "banking"),
    ("fidelity.com", "banking"),
    ("vanguard.com", "banking"),
    ("robinhood.com", "banking"),
    ("hsbc.com", "banking"),
    ("hsbc.co.uk", "banking"),
    ("barclays.co.uk", "banking"),
    ("lloydsbank.com", "banking"),
    ("natwest.com", "banking"),
    ("santander.com", "banking"),
    ("ing.com", "banking"),
    ("monzo.com", "banking"),
    ("n26.com", "banking"),
    ("chime.com", "banking"),
    // Identity providers and password managers
    ("accounts.google.com", "identity"),
    ("myaccount.google.com", "identity"),
    ("login.microsoftonline.com", "identity"),
    ("login.live.com", "identity"),
    ("account.microsoft.com", "identity"),
    ("appleid.apple.com", "identity"),
    ("idmsa.apple.com", "identity"),
    ("account.apple.com", "identity"),
    ("okta.com", "identity"),
    ("auth0.com", "identity"),
    ("login.yahoo.com", "identity"),
    ("1password.com", "identity"),
    ("bitwarden.com", "identity"),
    ("lastpass.com", "identity"),
];

/// The category of a sensitive host (`payment`, `banking`, `identity`).
pub fn sensitive_category(host: &str) -> Option<&'static str> {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    SENSITIVE.iter().find_map(|(domain, category)| {
        (host == *domain || host.ends_with(&format!(".{domain}"))).then_some(*category)
    })
}

/// The sensitive domains, for the extension and the page.
pub fn sensitive_domains() -> Vec<&'static str> {
    SENSITIVE.iter().map(|(d, _)| *d).collect()
}

/// The origin of a URL or of an origin string: `scheme://host[:port]` for
/// `http`/`https` (lower case, IDN in punycode, default port dropped,
/// trailing dot dropped). `None` for anything else (`file:`, `chrome:`,
/// `about:`, `data:`, `javascript:`, garbage).
pub fn origin_of(raw: &str) -> Option<String> {
    let url = url::Url::parse(raw.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    Some(match url.port() {
        Some(p) => format!("{}://{host}:{p}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    })
}

/// The host of an origin (`https://github.com:8443` → `github.com`; IPv6
/// keeps its brackets).
pub fn host_of(origin: &str) -> &str {
    let rest = origin.split_once("://").map_or(origin, |(_, r)| r);
    if rest.starts_with('[') {
        return rest
            .split_once(']')
            .map_or(rest, |(h, _)| h)
            .trim_start_matches('[');
    }
    rest.split(':').next().unwrap_or(rest)
}

/// `scheme://host[:port]` of a URL without its query and fragment, for the
/// activity log (a query can carry a token).
pub fn url_without_query(raw: &str) -> Option<String> {
    let url = url::Url::parse(raw.trim()).ok()?;
    let origin = origin_of(raw)?;
    let path = url.path();
    Some(if path == "/" {
        origin
    } else {
        format!("{origin}{path}")
    })
}

/// Normalise a site-mode key typed by a person or sent by the side panel:
/// `*`, an origin (or any URL: its origin is taken), or a wildcard
/// `scheme://*.domain[:port]`. The error is a sentence for the user.
pub fn normalize_key(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw == DEFAULT_KEY {
        return Ok(DEFAULT_KEY.to_string());
    }
    let bad = || {
        format!(
            "{raw:?} is not a site: use an origin like https://github.com, a wildcard like \
             https://*.github.com, or * for every other site."
        )
    };
    if raw.is_empty() || raw.len() > 300 {
        return Err(bad());
    }
    let Some((scheme, rest)) = raw.split_once("://") else {
        return Err(bad());
    };
    if let Some(after) = rest.strip_prefix("*.") {
        let origin = origin_of(&format!("{scheme}://{after}")).ok_or_else(bad)?;
        let host = host_of(&origin);
        // A wildcard names a domain with at least two labels, never an
        // address or a bare top-level domain.
        if host.parse::<std::net::Ipv4Addr>().is_ok() || origin.contains('[') || !host.contains('.')
        {
            return Err(bad());
        }
        let (scheme, tail) = origin.split_once("://").ok_or_else(bad)?;
        return Ok(format!("{scheme}://*.{tail}"));
    }
    origin_of(raw).ok_or_else(bad)
}

/// Whether a wildcard key `scheme://*.domain[:port]` covers `origin`.
fn wildcard_covers(key: &str, origin: &str) -> Option<usize> {
    let (k_scheme, k_rest) = key.split_once("://")?;
    let k_rest = k_rest.strip_prefix("*.")?;
    let (o_scheme, o_rest) = origin.split_once("://")?;
    if k_scheme != o_scheme {
        return None;
    }
    let (k_domain, k_port) = k_rest.split_once(':').unwrap_or((k_rest, ""));
    let o_host = host_of(origin);
    let o_port = o_rest
        .strip_prefix(o_host)
        .and_then(|p| p.strip_prefix(':'))
        .unwrap_or("");
    (k_port == o_port && o_host.ends_with(&format!(".{k_domain}"))).then_some(k_domain.len())
}

/// Where a site's mode came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "key")]
pub enum ModeSource {
    /// Its own entry.
    Site,
    /// A wildcard entry (the key).
    Wildcard(String),
    /// The sensitive list keeps it off.
    Sensitive,
    /// The default (`*`).
    Default,
}

/// A site's mode and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolved {
    pub origin: String,
    pub mode: SiteMode,
    pub source: ModeSource,
    /// The sensitive category, when the site is on the list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensitive: Option<&'static str>,
}

/// The kind of approval a standing grant skips on one origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GrantAction {
    /// `browser_eval`.
    Eval,
    /// `browser_upload`.
    Upload,
    /// Navigating to a sensitive site.
    Navigate,
    /// Acting on a sensitive site.
    Act,
    /// Filling or submitting a form with password, card or one-time-code
    /// fields (never typing into those fields: that is refused).
    SecretForm,
    /// More actions a minute than the limit.
    Burst,
    /// A grant written by a newer version: never matches.
    #[serde(other)]
    Unknown,
}

impl GrantAction {
    pub fn as_str(self) -> &'static str {
        match self {
            GrantAction::Eval => "eval",
            GrantAction::Upload => "upload",
            GrantAction::Navigate => "navigate",
            GrantAction::Act => "act",
            GrantAction::SecretForm => "secretForm",
            GrantAction::Burst => "burst",
            GrantAction::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "eval" => Some(GrantAction::Eval),
            "upload" => Some(GrantAction::Upload),
            "navigate" => Some(GrantAction::Navigate),
            "act" => Some(GrantAction::Act),
            "secretForm" => Some(GrantAction::SecretForm),
            "burst" => Some(GrantAction::Burst),
            _ => None,
        }
    }

    /// The words on a card: "run scripts", "upload files", ….
    pub fn label(self) -> &'static str {
        match self {
            GrantAction::Eval => "run scripts (browser_eval)",
            GrantAction::Upload => "upload files",
            GrantAction::Navigate => "open this sensitive site",
            GrantAction::Act => "act on this sensitive site",
            GrantAction::SecretForm => "fill and submit forms with password or card fields",
            GrantAction::Burst => "act more than the per-minute limit",
            GrantAction::Unknown => "an unknown action",
        }
    }
}

/// "Always allow <action> on <origin>".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Grant {
    pub id: String,
    pub action: GrantAction,
    /// An exact origin.
    pub origin: String,
    pub created_at: i64,
    /// Who created it ("Mac" or "PC" for this computer, a device's name).
    #[serde(default)]
    pub created_by: String,
}

/// The persisted browser settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// The mode of every site without its own (`*`).
    pub default_mode: SiteMode,
    /// `browser_screenshot` at all.
    pub screenshots: bool,
    /// Actions a minute on one site before each further one needs approval.
    pub actions_per_minute: u32,
    /// Always use this browser (`brw_…`) when several are connected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pinned_browser: Option<String>,
    /// Folders `browser_upload` may read besides the session's project.
    pub upload_folders: Vec<String>,
    /// Site → mode (never `*`: that is `default_mode`).
    pub sites: BTreeMap<String, SiteMode>,
    pub grants: Vec<Grant>,
    /// Fields a newer version wrote.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

/// Default actions a minute on one site.
pub const DEFAULT_ACTIONS_PER_MINUTE: u32 = 30;

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_mode: SiteMode::Off,
            screenshots: true,
            actions_per_minute: DEFAULT_ACTIONS_PER_MINUTE,
            pinned_browser: None,
            upload_folders: Vec::new(),
            sites: BTreeMap::new(),
            grants: Vec::new(),
            extra: serde_json::Map::new(),
        }
    }
}

impl Settings {
    /// The mode of `origin` (an origin from [`origin_of`]).
    pub fn resolve(&self, origin: &str) -> Resolved {
        let sensitive = sensitive_category(host_of(origin));
        if let Some(mode) = self.sites.get(origin) {
            return Resolved {
                origin: origin.to_string(),
                mode: *mode,
                source: ModeSource::Site,
                sensitive,
            };
        }
        if sensitive.is_some() {
            return Resolved {
                origin: origin.to_string(),
                mode: SiteMode::Off,
                source: ModeSource::Sensitive,
                sensitive,
            };
        }
        let wildcard = self
            .sites
            .iter()
            .filter_map(|(k, m)| wildcard_covers(k, origin).map(|len| (len, k, *m)))
            .max_by_key(|(len, _, _)| *len);
        if let Some((_, key, mode)) = wildcard {
            return Resolved {
                origin: origin.to_string(),
                mode,
                source: ModeSource::Wildcard(key.clone()),
                sensitive,
            };
        }
        Resolved {
            origin: origin.to_string(),
            mode: self.default_mode,
            source: ModeSource::Default,
            sensitive,
        }
    }

    /// The mode of a URL's site: `off` for anything that is not http(s).
    pub fn mode_of_url(&self, url: &str) -> SiteMode {
        origin_of(url).map_or(SiteMode::Off, |o| self.resolve(&o).mode)
    }

    /// `{"*": default, origin: mode, …}` as the extension gets it.
    pub fn site_modes_frame(&self) -> Value {
        let mut map = serde_json::Map::new();
        map.insert(DEFAULT_KEY.into(), Value::from(self.default_mode.as_str()));
        for (k, m) in &self.sites {
            map.insert(k.clone(), Value::from(m.as_str()));
        }
        Value::Object(map)
    }

    /// A grant covering `action` on `origin`.
    pub fn grant_for(&self, action: GrantAction, origin: &str) -> Option<&Grant> {
        self.grants
            .iter()
            .find(|g| g.action == action && g.origin == origin && action != GrantAction::Unknown)
    }

    /// Drop what does not parse (a hand-edited or newer file).
    fn sanitize(&mut self) {
        let sites = std::mem::take(&mut self.sites);
        for (k, m) in sites {
            match normalize_key(&k) {
                Ok(key) if key == DEFAULT_KEY => self.default_mode = m,
                Ok(key) => {
                    self.sites.insert(key, m);
                }
                Err(_) => tracing::warn!("dropping a browser site mode that does not parse"),
            }
        }
        self.grants.retain(|g| {
            !g.id.is_empty() && origin_of(&g.origin).as_deref() == Some(g.origin.as_str())
        });
        self.actions_per_minute = self.actions_per_minute.clamp(1, 600);
    }
}

/// The settings and their file.
#[derive(Debug)]
pub struct SettingsStore {
    path: Option<PathBuf>,
    inner: Mutex<Settings>,
}

impl SettingsStore {
    /// Load `browser-settings.json` from `data_dir` (missing or corrupt →
    /// defaults); `None` keeps them in memory (tests).
    pub fn load(data_dir: Option<&Path>) -> Self {
        let path = data_dir.map(|d| d.join("browser-settings.json"));
        let mut settings: Settings = path
            .as_deref()
            .and_then(crate::persist::read_json)
            .unwrap_or_default();
        settings.sanitize();
        Self {
            path,
            inner: Mutex::new(settings),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Settings> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// A copy of the settings.
    pub fn get(&self) -> Settings {
        self.lock().clone()
    }

    /// Change the settings with `f` and save them; `f`'s error leaves them
    /// as they were.
    pub fn update<T>(
        &self,
        f: impl FnOnce(&mut Settings) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut g = self.lock();
        let mut next = g.clone();
        let out = f(&mut next)?;
        if let Some(path) = &self.path {
            if let Err(e) = crate::persist::write_json_private(path, &next) {
                tracing::warn!(error = %e, "could not save the browser settings");
                return Err("The browser settings could not be saved.".into());
            }
        }
        *g = next;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_are_normalised() {
        let cases = [
            ("https://GitHub.com", Some("https://github.com")),
            ("https://github.com/", Some("https://github.com")),
            (
                "https://github.com:443/a/b?c=d#e",
                Some("https://github.com"),
            ),
            ("http://github.com:80", Some("http://github.com")),
            ("https://github.com:8443", Some("https://github.com:8443")),
            ("http://localhost:3000/x", Some("http://localhost:3000")),
            ("https://github.com./", Some("https://github.com")),
            ("http://[::1]:8080/", Some("http://[::1]:8080")),
            ("https://user:pw@example.com/", Some("https://example.com")),
            ("https://bücher.de", Some("https://xn--bcher-kva.de")),
            ("file:///etc/passwd", None),
            ("chrome://settings", None),
            ("about:blank", None),
            ("javascript:alert(1)", None),
            ("data:text/html,hi", None),
            ("github.com", None),
            ("", None),
        ];
        for (raw, want) in cases {
            assert_eq!(origin_of(raw).as_deref(), want, "{raw}");
        }
        assert_eq!(host_of("https://github.com:8443"), "github.com");
        assert_eq!(host_of("http://[::1]:8080"), "::1");
        assert_eq!(
            url_without_query("https://x.test/reset?token=abc#f").as_deref(),
            Some("https://x.test/reset")
        );
    }

    #[test]
    fn keys_accept_origins_wildcards_and_the_default() {
        assert_eq!(normalize_key("*").unwrap(), "*");
        assert_eq!(
            normalize_key("https://GitHub.com/issues").unwrap(),
            "https://github.com"
        );
        assert_eq!(
            normalize_key("https://*.GitHub.com").unwrap(),
            "https://*.github.com"
        );
        assert_eq!(
            normalize_key("https://*.example.com:8443").unwrap(),
            "https://*.example.com:8443"
        );
        for bad in [
            "",
            "github.com",
            "https://*.com",
            "https://*.10.0.0.1",
            "ftp://example.com",
            "https://*.[::1]",
            "**",
        ] {
            assert!(normalize_key(bad).is_err(), "{bad}");
        }
    }

    fn settings(default: SiteMode, sites: &[(&str, SiteMode)]) -> Settings {
        Settings {
            default_mode: default,
            sites: sites
                .iter()
                .map(|(k, m)| (normalize_key(k).unwrap(), *m))
                .collect(),
            ..Settings::default()
        }
    }

    #[test]
    fn resolution_prefers_the_site_then_the_longest_wildcard_then_the_default() {
        let s = settings(
            SiteMode::Read,
            &[
                ("https://github.com", SiteMode::Act),
                ("https://*.example.com", SiteMode::Read),
                ("https://*.a.example.com", SiteMode::Act),
                ("http://localhost:3000", SiteMode::Act),
            ],
        );
        let mode = |o: &str| s.resolve(o).mode;
        assert_eq!(mode("https://github.com"), SiteMode::Act);
        // Origins are exact: a subdomain or another scheme is another site.
        assert_eq!(mode("https://gist.github.com"), SiteMode::Read);
        assert_eq!(mode("http://github.com"), SiteMode::Read);
        // Ports are part of the origin.
        assert_eq!(mode("http://localhost:3000"), SiteMode::Act);
        assert_eq!(mode("http://localhost:3001"), SiteMode::Read);
        assert_eq!(mode("http://localhost"), SiteMode::Read);
        // Wildcards cover subdomains, not the domain itself; the longest wins.
        assert_eq!(mode("https://b.example.com"), SiteMode::Read);
        assert_eq!(mode("https://x.a.example.com"), SiteMode::Act);
        assert_eq!(mode("https://example.com"), SiteMode::Read);
        assert_eq!(
            s.resolve("https://x.a.example.com").source,
            ModeSource::Wildcard("https://*.a.example.com".into())
        );
        // A wildcard with a port covers that port only.
        let p = settings(
            SiteMode::Off,
            &[("https://*.example.com:8443", SiteMode::Act)],
        );
        assert_eq!(p.resolve("https://a.example.com:8443").mode, SiteMode::Act);
        assert_eq!(p.resolve("https://a.example.com").mode, SiteMode::Off);
        // Off by default.
        assert_eq!(
            Settings::default().resolve("https://x.test").mode,
            SiteMode::Off
        );
        assert_eq!(
            Settings::default().mode_of_url("about:blank"),
            SiteMode::Off
        );
    }

    #[test]
    fn sensitive_sites_open_only_by_their_own_entry() {
        assert_eq!(sensitive_category("www.paypal.com"), Some("payment"));
        assert_eq!(sensitive_category("accounts.google.com"), Some("identity"));
        assert_eq!(sensitive_category("secure.chase.com."), Some("banking"));
        assert_eq!(sensitive_category("notpaypal.com"), None);
        assert_eq!(sensitive_category("google.com"), None);
        let s = settings(
            SiteMode::Act,
            &[
                ("https://*.google.com", SiteMode::Act),
                ("https://www.chase.com", SiteMode::Read),
            ],
        );
        let r = s.resolve("https://www.paypal.com");
        assert_eq!((r.mode, r.source), (SiteMode::Off, ModeSource::Sensitive));
        assert_eq!(r.sensitive, Some("payment"));
        // A wildcard never opens one either.
        assert_eq!(s.resolve("https://accounts.google.com").mode, SiteMode::Off);
        assert_eq!(s.resolve("https://mail.google.com").mode, SiteMode::Act);
        // Its own entry does.
        let c = s.resolve("https://www.chase.com");
        assert_eq!((c.mode, c.source), (SiteMode::Read, ModeSource::Site));
        assert_eq!(c.sensitive, Some("banking"));
    }

    #[test]
    fn the_file_round_trips_and_tolerates_junk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("browser-settings.json");
        std::fs::write(
            &path,
            r#"{"defaultMode":"read","screenshots":false,"sites":{"HTTPS://GitHub.com/":"act",
               "not a site":"act","*":"off"},"grants":[{"id":"g_1","action":"eval",
               "origin":"http://localhost:3000","createdAt":1},{"id":"g_2","action":"later",
               "origin":"https://x.test","createdAt":2},{"id":"g_3","action":"eval",
               "origin":"https://x.test/path","createdAt":3}],"future":{"a":1}}"#,
        )
        .unwrap();
        let store = SettingsStore::load(Some(dir.path()));
        let s = store.get();
        assert_eq!(s.default_mode, SiteMode::Off, "`*` in sites is the default");
        assert!(!s.screenshots);
        assert_eq!(s.sites.len(), 1);
        assert_eq!(s.sites["https://github.com"], SiteMode::Act);
        assert_eq!(s.grants.len(), 2, "a grant with a path is dropped");
        assert_eq!(s.grants[1].action, GrantAction::Unknown);
        assert!(s
            .grant_for(GrantAction::Eval, "http://localhost:3000")
            .is_some());
        assert!(s
            .grant_for(GrantAction::Unknown, "https://x.test")
            .is_none());
        store
            .update(|s| {
                s.sites.insert("https://x.test".into(), SiteMode::Read);
                Ok(())
            })
            .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"future\""), "unknown fields survive");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let again = SettingsStore::load(Some(dir.path())).get();
        assert_eq!(again.sites["https://x.test"], SiteMode::Read);
        // A refused change leaves everything as it was.
        let err = store.update(|s| {
            s.screenshots = true;
            Err::<(), _>("no".to_string())
        });
        assert!(err.is_err());
        assert!(!store.get().screenshots);
        // The extension's map.
        let frame = store.get().site_modes_frame();
        assert_eq!(frame["*"], "off");
        assert_eq!(frame["https://github.com"], "act");
    }
}
