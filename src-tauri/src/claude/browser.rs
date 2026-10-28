//! Opening a sign-in link in a browser (docs/ACCOUNTS.md §15.2): the default
//! browser (macOS `open`, Windows `rundll32 url.dll,FileProtocolHandler`,
//! Linux `xdg-open`), Google Chrome, or a private Chrome window
//! (`--incognito`).
//!
//! The sign-in runs give the CLI `BROWSER=true`, so it never opens anything
//! itself; the link it shows is opened here, and only a [`Link`]: an `https`
//! link to `claude.ai`, `claude.com` or `anthropic.com` (or a subdomain),
//! without credentials or a port. Nothing else can be turned into a command.
//!
//! The command is built by a pure function per platform and browser, so it is
//! tested without spawning anything; [`Opener`] runs it. Unlike
//! [`super::reveal::Revealer`], an opener starts out only *recording* what it
//! would run, and only the app itself switches it to opening for real
//! ([`Opener::open_for_real`], in `lib.rs`): a login opens the browser on its
//! own as soon as its link is known, so a test (or any other embedding) that
//! forgot to switch it off would otherwise open the developer's browser.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

pub use super::reveal::Platform;

/// The hosts a sign-in link may point at (or a subdomain of one).
pub const ALLOWED_HOSTS: [&str; 3] = ["claude.ai", "claude.com", "anthropic.com"];

/// Longest link opened.
const LINK_MAX: usize = 4096;

/// A sign-in link that may be opened: `https`, a host in [`ALLOWED_HOSTS`]
/// or a subdomain of one, no user name, password or port. Only
/// [`Link::parse`] makes one, so no command is ever built from anything else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link(String);

impl Link {
    /// The link, when it is one Mother Claude may open; its normalized form
    /// (what was checked is exactly what is opened).
    pub fn parse(raw: &str) -> Option<Self> {
        if raw.len() > LINK_MAX || raw.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return None;
        }
        let url = reqwest::Url::parse(raw).ok()?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
        {
            return None;
        }
        // `domain()` is `None` for an IP address; the parser lowercases it
        // and turns an international name into punycode.
        let host = url.domain()?;
        let allowed = ALLOWED_HOSTS.iter().any(|d| {
            host == *d
                || host
                    .strip_suffix(d)
                    .is_some_and(|sub| sub.len() > 1 && sub.ends_with('.'))
        });
        allowed.then(|| Self(url.as_str().to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Which browser to open a link in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Browser {
    /// The system's default browser.
    Default,
    /// Google Chrome.
    Chrome,
    /// A private (incognito) Google Chrome window: no other account is signed
    /// in there, so the page cannot hand another account's consent over.
    ChromePrivate,
}

impl Browser {
    pub const ALL: [Browser; 3] = [Self::Default, Self::Chrome, Self::ChromePrivate];
}

/// The browsers found on this machine besides the default one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Installed {
    /// Google Chrome: the app bundle on macOS, the executable elsewhere.
    pub chrome: Option<PathBuf>,
}

impl Installed {
    /// What `platform` has, looked up in the usual places.
    pub fn detect(platform: Platform) -> Self {
        let env = |key: &str| std::env::var_os(key).filter(|v| !v.is_empty());
        let home = super::home::user_home_dir();
        Self {
            chrome: chrome_candidates(platform, &env, home.as_deref())
                .into_iter()
                .find(|p| match platform {
                    Platform::MacOs => p.is_dir(),
                    _ => p.is_file(),
                }),
        }
    }

    pub fn has(&self, browser: Browser) -> bool {
        match browser {
            Browser::Default => true,
            Browser::Chrome | Browser::ChromePrivate => self.chrome.is_some(),
        }
    }

    /// The browsers a link can be opened in here, the default first (`GET
    /// /api/accounts/{id}/sign-in/open` lists their ids).
    pub fn list(&self) -> Vec<Browser> {
        Browser::ALL.into_iter().filter(|b| self.has(*b)).collect()
    }
}

/// Where Google Chrome usually is, most likely first.
pub fn chrome_candidates(
    platform: Platform,
    env: &dyn Fn(&str) -> Option<OsString>,
    home: Option<&Path>,
) -> Vec<PathBuf> {
    match platform {
        Platform::MacOs => {
            let mut out = vec![PathBuf::from("/Applications/Google Chrome.app")];
            if let Some(home) = home {
                out.push(home.join("Applications/Google Chrome.app"));
            }
            out
        }
        Platform::Windows => ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
            .into_iter()
            .filter_map(env)
            .map(|base| {
                PathBuf::from(base)
                    .join("Google")
                    .join("Chrome")
                    .join("Application")
                    .join("chrome.exe")
            })
            .collect(),
        Platform::Linux => env("PATH")
            .map(|path| {
                std::env::split_paths(&path)
                    .flat_map(|dir| {
                        ["google-chrome", "google-chrome-stable"].map(|name| dir.join(name))
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// A command line (no shell: the link is one argument).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub program: OsString,
    pub args: Vec<OsString>,
}

/// The command that opens `link` in `browser` on `platform`; `None` when the
/// browser is not installed. `system_root` is Windows' `%SystemRoot%`.
pub fn command(
    platform: Platform,
    browser: Browser,
    installed: &Installed,
    link: &Link,
    system_root: Option<&Path>,
) -> Option<Command> {
    let url = OsString::from(link.as_str());
    let chrome = || installed.chrome.clone().map(PathBuf::into_os_string);
    Some(match (platform, browser) {
        (Platform::MacOs, Browser::Default) => Command {
            program: "/usr/bin/open".into(),
            args: vec![url],
        },
        (Platform::MacOs, Browser::Chrome) => Command {
            program: "/usr/bin/open".into(),
            args: vec!["-a".into(), chrome()?, url],
        },
        // `-n`: a new instance hands its arguments to a Chrome that already
        // runs (which then opens the private window); without it they would
        // be dropped and Chrome only brought to the front.
        (Platform::MacOs, Browser::ChromePrivate) => Command {
            program: "/usr/bin/open".into(),
            args: vec![
                "-n".into(),
                "-a".into(),
                chrome()?,
                "--args".into(),
                "--incognito".into(),
                url,
            ],
        },
        (Platform::Windows, Browser::Default) => Command {
            program: system_root
                .map(|root| root.join("System32").join("rundll32.exe").into_os_string())
                .unwrap_or_else(|| "rundll32.exe".into()),
            args: vec!["url.dll,FileProtocolHandler".into(), url],
        },
        (Platform::Linux, Browser::Default) => Command {
            program: "xdg-open".into(),
            args: vec![url],
        },
        (Platform::Windows | Platform::Linux, Browser::Chrome) => Command {
            program: chrome()?,
            args: vec![url],
        },
        (Platform::Windows | Platform::Linux, Browser::ChromePrivate) => Command {
            program: chrome()?,
            args: vec!["--incognito".into(), url],
        },
    })
}

/// Why a link was not opened.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    /// Only Chrome can be missing: the default browser is always there.
    #[error("Google Chrome is not installed on this machine")]
    Unavailable(Browser),
    #[error("could not open the browser: {0}")]
    Spawn(String),
}

impl OpenError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unavailable(_) => "browser_unavailable",
            Self::Spawn(_) => "browser_failed",
        }
    }
}

/// Run a command without waiting for it (reaped in the background: `open`
/// and `rundll32` return at once, Chrome hands over to a running one).
fn spawn(cmd: &Command) -> std::io::Result<()> {
    let mut child = std::process::Command::new(&cmd.program)
        .args(&cmd.args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Opens links in a browser. Lives in the app state and starts out recording
/// only (see the module docs).
#[derive(Debug)]
pub struct Opener {
    /// `Some`: record instead of running.
    recorded: Mutex<Option<Vec<Command>>>,
    /// What tests say this machine has (instead of looking).
    pretend: Mutex<Option<Installed>>,
}

impl Default for Opener {
    fn default() -> Self {
        Self {
            recorded: Mutex::new(Some(Vec::new())),
            pretend: Mutex::new(None),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Opener {
    /// From now on, open links for real (the app, at startup).
    pub fn open_for_real(&self) {
        lock(&self.recorded).take();
    }

    /// Record each command instead of running it again (tests).
    pub fn record_only(&self) {
        lock(&self.recorded).get_or_insert_with(Vec::new);
    }

    /// The commands recorded so far.
    pub fn recorded(&self) -> Vec<Command> {
        lock(&self.recorded).clone().unwrap_or_default()
    }

    /// Answer [`Opener::installed`] with this instead of looking (tests).
    pub fn pretend_installed(&self, installed: Installed) {
        *lock(&self.pretend) = Some(installed);
    }

    /// The browsers this machine has.
    pub fn installed(&self) -> Installed {
        lock(&self.pretend)
            .clone()
            .unwrap_or_else(|| Installed::detect(Platform::current()))
    }

    /// Open `link` in `browser`.
    pub fn open(&self, browser: Browser, link: &Link) -> Result<(), OpenError> {
        let root = std::env::var_os("SystemRoot").map(PathBuf::from);
        let cmd = command(
            Platform::current(),
            browser,
            &self.installed(),
            link,
            root.as_deref(),
        )
        .ok_or(OpenError::Unavailable(browser))?;
        if let Some(recorded) = lock(&self.recorded).as_mut() {
            recorded.push(cmd);
            return Ok(());
        }
        spawn(&cmd).map_err(|e| OpenError::Spawn(e.kind().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUTHORIZE: &str = "https://claude.ai/oauth/authorize?code=true&client_id=x&state=s";

    fn link(s: &str) -> Link {
        Link::parse(s).unwrap_or_else(|| panic!("{s} should be allowed"))
    }

    fn args(c: &Command) -> Vec<String> {
        c.args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    fn chrome_at(p: &str) -> Installed {
        Installed {
            chrome: Some(PathBuf::from(p)),
        }
    }

    #[test]
    fn only_https_links_to_anthropics_hosts_are_links() {
        for ok in [
            AUTHORIZE,
            "https://claude.com/cai/oauth/authorize?x=1",
            "https://platform.claude.com/oauth/authorize?x=1",
            "https://console.anthropic.com/oauth/authorize?x=1",
            "https://CLAUDE.AI/oauth/authorize",
            "https://claude.ai:443/oauth/authorize",
        ] {
            assert!(Link::parse(ok).is_some(), "{ok}");
        }
        for bad in [
            "http://claude.ai/oauth/authorize",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "https://evilclaude.ai/oauth/authorize",
            "https://claude.ai.evil.example/oauth/authorize",
            "https://claude.ai@evil.example/oauth/authorize",
            "https://user:pw@claude.ai/oauth/authorize",
            "https://claude.ai:8443/oauth/authorize",
            "https://claude.ai./oauth/authorize",
            "https://.claude.ai/oauth/authorize",
            "https://anthropic.co/oauth/authorize",
            "https://127.0.0.1/oauth/authorize",
            "https://[::1]/oauth/authorize",
            "https://xn--claude-xx.ai/oauth/authorize",
            "https://claude.ai/oauth authorize",
            "https://claude.ai/oauth\n/authorize",
            "--incognito",
            "",
        ] {
            assert!(Link::parse(bad).is_none(), "{bad:?} must be refused");
        }
        let long = format!("{AUTHORIZE}&pad={}", "a".repeat(LINK_MAX));
        assert!(Link::parse(&long).is_none());
        // What is checked is what is opened.
        assert_eq!(
            link("https://CLAUDE.AI/oauth/authorize").as_str(),
            "https://claude.ai/oauth/authorize"
        );
    }

    #[test]
    fn macos_opens_the_default_browser_chrome_or_a_private_window() {
        let app = "/Applications/Google Chrome.app";
        let c = command(
            Platform::MacOs,
            Browser::Default,
            &Installed::default(),
            &link(AUTHORIZE),
            None,
        )
        .unwrap();
        assert_eq!(c.program, "/usr/bin/open");
        assert_eq!(args(&c), [AUTHORIZE]);
        let c = command(
            Platform::MacOs,
            Browser::Chrome,
            &chrome_at(app),
            &link(AUTHORIZE),
            None,
        )
        .unwrap();
        assert_eq!(args(&c), ["-a", app, AUTHORIZE]);
        let c = command(
            Platform::MacOs,
            Browser::ChromePrivate,
            &chrome_at(app),
            &link(AUTHORIZE),
            None,
        )
        .unwrap();
        assert_eq!(c.program, "/usr/bin/open");
        assert_eq!(
            args(&c),
            ["-n", "-a", app, "--args", "--incognito", AUTHORIZE]
        );
        // Without Chrome there is no Chrome command.
        for b in [Browser::Chrome, Browser::ChromePrivate] {
            assert!(command(
                Platform::MacOs,
                b,
                &Installed::default(),
                &link(AUTHORIZE),
                None
            )
            .is_none());
        }
    }

    #[test]
    fn windows_and_linux_have_their_own_commands() {
        let c = command(
            Platform::Windows,
            Browser::Default,
            &Installed::default(),
            &link(AUTHORIZE),
            Some(Path::new(r"C:\Windows")),
        )
        .unwrap();
        assert_eq!(
            PathBuf::from(&c.program),
            Path::new(r"C:\Windows")
                .join("System32")
                .join("rundll32.exe")
        );
        assert_eq!(args(&c), ["url.dll,FileProtocolHandler", AUTHORIZE]);
        let exe = r"C:\Program Files\Google\Chrome\Application\chrome.exe";
        let c = command(
            Platform::Windows,
            Browser::ChromePrivate,
            &chrome_at(exe),
            &link(AUTHORIZE),
            None,
        )
        .unwrap();
        assert_eq!(c.program, exe);
        assert_eq!(args(&c), ["--incognito", AUTHORIZE]);

        let c = command(
            Platform::Linux,
            Browser::Default,
            &Installed::default(),
            &link(AUTHORIZE),
            None,
        )
        .unwrap();
        assert_eq!(c.program, "xdg-open");
        assert_eq!(args(&c), [AUTHORIZE]);
        let c = command(
            Platform::Linux,
            Browser::Chrome,
            &chrome_at("/usr/bin/google-chrome"),
            &link(AUTHORIZE),
            None,
        )
        .unwrap();
        assert_eq!(c.program, "/usr/bin/google-chrome");
        assert_eq!(args(&c), [AUTHORIZE]);
    }

    #[test]
    fn chrome_is_looked_for_where_it_usually_is() {
        let none = |_: &str| None;
        assert_eq!(
            chrome_candidates(Platform::MacOs, &none, Some(Path::new("/Users/me"))),
            [
                PathBuf::from("/Applications/Google Chrome.app"),
                PathBuf::from("/Users/me/Applications/Google Chrome.app")
            ]
        );
        let win = |key: &str| match key {
            "ProgramFiles" => Some(OsString::from(r"C:\Program Files")),
            "LOCALAPPDATA" => Some(OsString::from(r"C:\Users\me\AppData\Local")),
            _ => None,
        };
        let found = chrome_candidates(Platform::Windows, &win, None);
        assert_eq!(found.len(), 2);
        assert!(found[0].ends_with("chrome.exe"));
        let path = std::env::join_paths(["/usr/local/bin", "/usr/bin"]).unwrap();
        let linux = move |key: &str| (key == "PATH").then(|| path.clone());
        assert_eq!(
            chrome_candidates(Platform::Linux, &linux, None),
            [
                PathBuf::from("/usr/local/bin/google-chrome"),
                PathBuf::from("/usr/local/bin/google-chrome-stable"),
                PathBuf::from("/usr/bin/google-chrome"),
                PathBuf::from("/usr/bin/google-chrome-stable"),
            ]
        );
    }

    #[test]
    fn the_list_names_only_what_is_there() {
        assert_eq!(Installed::default().list(), [Browser::Default]);
        assert_eq!(
            chrome_at("/Applications/Google Chrome.app").list(),
            Browser::ALL
        );
        assert_eq!(
            serde_json::to_value(chrome_at("/x").list()).unwrap(),
            serde_json::json!(["default", "chrome", "chrome-private"])
        );
        let asked: Browser = serde_json::from_value(serde_json::json!("chrome-private")).unwrap();
        assert_eq!(asked, Browser::ChromePrivate);
        assert!(serde_json::from_value::<Browser>(serde_json::json!("firefox")).is_err());
    }

    #[test]
    fn an_opener_records_until_the_app_lets_it_open() {
        let o = Opener::default();
        o.pretend_installed(Installed::default());
        o.open(Browser::Default, &link(AUTHORIZE)).unwrap();
        assert_eq!(o.recorded().len(), 1, "recorded, nothing opened");
        assert!(matches!(
            o.open(Browser::ChromePrivate, &link(AUTHORIZE)),
            Err(OpenError::Unavailable(Browser::ChromePrivate))
        ));
        assert_eq!(o.recorded().len(), 1);
        o.pretend_installed(chrome_at("/Applications/Google Chrome.app"));
        o.open(Browser::ChromePrivate, &link(AUTHORIZE)).unwrap();
        assert_eq!(o.recorded().len(), 2);
    }
}
