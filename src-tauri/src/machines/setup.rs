//! The setup's checks and fixes (docs/PLAN-MACHINES.md §5.14), Phase 1's
//! part: Mother Claude, keeping awake, the network (macOS: Local Network
//! and the application firewall; Windows: the network profile and the
//! Defender Firewall), the relay, and the pairing. Each check says why it
//! matters, how bad a failure is (`required` blocks, `recommended` works but
//! worse) and what fixes it.
//!
//! * The network checks block only with the *No relay* setting: with a
//!   relay, machines still meet, through it (the plan's B†).
//! * Detection never changes anything, and runs inside the app (macOS holds
//!   the app responsible for its Local Network permission).
//! * A fix runs only from the desktop of the machine it changes; an
//!   elevated one only after the OS's own administrator prompt. Until the
//!   app turns fixes on (`MachinesState::enable_fixes`), a fix that would
//!   open a settings page or ask for an administrator is only recorded, so
//!   no test ever does either.
//! * Results are kept in `machines/setup.json` (0600) for Machines health,
//!   which reruns the checks at start and every 30 minutes.
//!
//! Phase 3a adds the rest (plan §5.14, "the setup's rest"): git, long
//! paths, Claude Code, each account's sign-in and room for data
//! (`checks`); accounts matched by name across computers (`accounts`);
//! roots proposed from repositories' origins and *Clone it there*
//! (`folders`); what another computer asks this one to do, done only when
//! confirmed here (`asks`); the code options per project (`options`); and their
//! routes (`routes`).

pub mod accounts;
pub mod asks;
pub mod checks;
pub mod folders;
pub mod options;
pub mod routes;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::store::{CodeSettings, RelayMode};
use super::transport;
use crate::server::error::ApiError;
use crate::state::AppState;

pub const FILE: &str = "machines/setup.json";
const HEALTH_EVERY: Duration = Duration::from_secs(30 * 60);
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);
/// The readiness test's transfer.
pub const TEST_MIB: u64 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckState {
    Ok,
    Warning,
    Blocker,
    Unknown,
    /// Does not apply here.
    Na,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Required,
    Recommended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FixKind {
    /// Mother Claude does it.
    Auto,
    /// Opens the settings page where the user does it.
    Settings,
    /// Needs the OS's administrator prompt.
    Admin,
    /// The user does it (the text says how).
    Manual,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fix {
    pub kind: FixKind,
    pub label: String,
}

/// One check's outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub id: String,
    /// `app` | `power` | `network` | `relay` | `pairing`.
    pub area: String,
    pub title: String,
    pub state: CheckState,
    pub detail: String,
    /// Why it matters, one line.
    pub why: String,
    pub severity: Severity,
    #[serde(default)]
    pub fix: Option<Fix>,
    /// What a failure blocks when it is not all of Machines (the plan's
    /// "B for that account"): `account` — that account's sessions. Such a
    /// blocker shows red but does not keep Machines off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

impl Check {
    fn new(id: &str, area: &str, title: &str, why: &str) -> Self {
        Self {
            id: id.into(),
            area: area.into(),
            title: title.into(),
            state: CheckState::Unknown,
            detail: String::new(),
            why: why.into(),
            severity: Severity::Required,
            fix: None,
            scope: None,
        }
    }

    /// A blocker that keeps Machines off (one with a scope blocks only its
    /// part).
    pub fn gates(&self) -> bool {
        self.state == CheckState::Blocker && self.scope.is_none()
    }

    fn ok(mut self, detail: impl Into<String>) -> Self {
        self.state = CheckState::Ok;
        self.detail = detail.into();
        self.fix = None;
        self
    }

    fn unknown(mut self, detail: impl Into<String>) -> Self {
        self.state = CheckState::Unknown;
        self.detail = detail.into();
        self
    }

    /// A failure that blocks (`required`) or only degrades (`recommended`).
    fn failed(mut self, required: bool, detail: impl Into<String>, fix: Option<Fix>) -> Self {
        self.severity = if required {
            Severity::Required
        } else {
            Severity::Recommended
        };
        self.state = if required {
            CheckState::Blocker
        } else {
            CheckState::Warning
        };
        self.detail = detail.into();
        self.fix = fix;
        self
    }
}

fn fix(kind: FixKind, label: &str) -> Option<Fix> {
    Some(Fix {
        kind,
        label: label.into(),
    })
}

/// What a machine says of its tools beside its checks (`/peer/setup`): its
/// system, git's and Claude Code's versions, and its code options.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Facts {
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub git: Option<String>,
    #[serde(default)]
    pub claude: Option<String>,
    #[serde(default)]
    pub code: Option<CodeSettings>,
}

/// Another machine's checks, as it last answered.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerChecks {
    pub at: i64,
    #[serde(default)]
    pub checks: Vec<Check>,
    #[serde(default)]
    pub error: Option<String>,
    /// Its tools and options (absent from an older build).
    #[serde(default)]
    pub facts: Option<Facts>,
    /// What this machine asked of it, and how each went.
    #[serde(default)]
    pub asks: Vec<asks::Ask>,
}

/// One readiness-test line.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestLine {
    /// `connect` | `transfer` | `tools` | `code` | `accounts` | `folders`.
    pub id: String,
    /// `ok` | `warning` | `failed` | `skipped`.
    pub state: String,
    pub detail: String,
    #[serde(default)]
    pub ms: Option<f64>,
    #[serde(default)]
    pub mibps: Option<f64>,
}

/// The readiness test with one machine.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Readiness {
    pub machine: String,
    pub name: String,
    pub tests: Vec<TestLine>,
    pub at: i64,
}

/// The last results (`machines/setup.json`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cache {
    #[serde(default)]
    pub at: Option<i64>,
    #[serde(default)]
    pub checks: Vec<Check>,
    #[serde(default)]
    pub peers: BTreeMap<String, PeerChecks>,
    #[serde(default)]
    pub tests: BTreeMap<String, Readiness>,
    #[serde(default)]
    pub claude: Option<String>,
    /// git's version, as the checks last read it.
    #[serde(default)]
    pub git: Option<String>,
    /// What other machines asked this one to do (`asks`), newest last.
    #[serde(default)]
    pub asks: Vec<asks::Ask>,
    /// The last scan for git repositories (`folders`): when, the roots'
    /// folders it looked under, and what it found.
    #[serde(skip)]
    pub(crate) repos: Option<(i64, Vec<String>, Vec<folders::Repo>)>,
    /// Remote checks other computers asked for, running now (`folders`).
    #[serde(skip)]
    pub(crate) remote_checks: usize,
    #[serde(skip)]
    dir: Option<PathBuf>,
}

impl Cache {
    pub fn load(data_dir: Option<&Path>) -> Self {
        let mut c = data_dir
            .and_then(|d| crate::persist::read_json::<Cache>(&d.join(FILE)))
            .unwrap_or_default();
        c.dir = data_dir.map(Path::to_path_buf);
        c
    }

    fn save(&self) {
        if let Some(dir) = &self.dir {
            if let Err(e) = crate::persist::write_json_private(&dir.join(FILE), self) {
                tracing::warn!(error = %e, "machines/setup.json could not be saved");
            }
        }
    }
}

fn with_cache<R>(state: &AppState, f: impl FnOnce(&mut Cache) -> R) -> R {
    let mut c = state
        .machines
        .setup
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    f(&mut c)
}

/// The Default account's Claude Code version, as the checks last read it.
pub fn claude_version(state: &AppState) -> Option<String> {
    with_cache(state, |c| c.claude.clone())
}

// ─── running the checks ────────────────────────────────────────────────────────

/// Runs this machine's checks, keeps and publishes them.
pub async fn refresh(state: &AppState) -> Vec<Check> {
    let checks = run_local(state).await;
    with_cache(state, |c| {
        c.at = Some(super::now());
        c.checks = checks.clone();
        c.save();
    });
    super::publish(state, "setup");
    checks
}

/// This machine's checks.
pub async fn run_local(state: &AppState) -> Vec<Check> {
    let relay = state.machines.settings().relay;
    // The network checks block only without a relay (B†).
    let net_required = relay.mode == RelayMode::None;
    let mut out = vec![app_check(state).await, awake_check(state).await];
    out.extend(network_checks(net_required).await);
    out.push(relay_check(state).await);
    out.push(pairing_check(state));
    out.extend(peer_checks(state));
    // Phase 3a: what a move needs here (git, Claude Code, the accounts'
    // sign-ins, room for data).
    out.extend(checks::tools(state).await);
    out.extend(checks::accounts(state).await);
    out.push(checks::disk(state).await);
    out
}

/// What this machine tells another beside its checks (`/peer/setup`).
pub fn facts(state: &AppState) -> Facts {
    with_cache(state, |c| Facts {
        os: super::protocol::this_os().to_string(),
        git: c.git.clone(),
        claude: c.claude.clone(),
        code: Some(state.machines.settings().code),
    })
}

/// `GET /peer/setup`: this machine's checks for `machine`'s side-by-side
/// view, its tools, and what `machine` asked of it.
pub async fn peer_answer(state: &AppState, machine: &str) -> Value {
    let v = view(state, false).await;
    json!({
        "checks": v["self"]["checks"],
        "facts": facts(state),
        "asks": asks::made_by(state, machine),
    })
}

async fn app_check(state: &AppState) -> Check {
    let claude = probe(&crate::claude::claude_bin(), &["--version"])
        .await
        .and_then(|o| checks::parse_claude_version(&o));
    with_cache(state, |c| c.claude.clone_from(&claude));
    let c = Check::new(
        "app",
        "app",
        "Mother Claude",
        "Every computer needs a version that speaks the same Machines protocol.",
    );
    let mut detail = format!(
        "Version {} · Machines protocol {}",
        env!("CARGO_PKG_VERSION"),
        super::protocol::PROTOCOL
    );
    if let Some(v) = &claude {
        detail.push_str(&format!(" · Claude Code {v}"));
    }
    c.ok(detail)
}

async fn awake_check(state: &AppState) -> Check {
    let c = Check::new(
        "keepAwake",
        "power",
        "Stays awake while used",
        "A computer asleep cannot answer the others.",
    );
    let timer = sleep_timer().await;
    let timer_text = match timer {
        Some(0) => "It never sleeps on its own.".to_string(),
        Some(m) => format!("It sleeps after {m} min idle."),
        None => String::new(),
    };
    if state.machines.settings().keep_awake {
        c.ok(
            format!("Kept awake while another computer uses it. {timer_text}")
                .trim()
                .to_string(),
        )
    } else if timer == Some(0) {
        c.ok(timer_text)
    } else {
        c.failed(
            false,
            format!("It may sleep while another computer uses it. {timer_text}")
                .trim()
                .to_string(),
            fix(FixKind::Auto, "Keep awake while used"),
        )
    }
}

fn pairing_check(state: &AppState) -> Check {
    let c = Check::new(
        "pairing",
        "pairing",
        "Paired computers",
        "Machines works between computers you paired.",
    );
    let n = state.machines.store_read(|f| f.peers.len());
    if n == 0 {
        c.failed(
            true,
            "No other computer is paired yet.",
            fix(FixKind::Manual, "Pair a machine"),
        )
    } else {
        c.ok(format!(
            "Paired with {n} {}.",
            if n == 1 { "computer" } else { "computers" }
        ))
    }
}

/// Per paired machine: a compatible version and the same relay setting (and,
/// recommended, the same code options).
fn peer_checks(state: &AppState) -> Vec<Check> {
    let settings = state.machines.settings();
    let mine = settings.relay;
    let peers = state.machines.store_read(|f| f.peers.clone());
    peers
        .iter()
        .map(|p| {
            let name = p.display_name().to_string();
            let c = Check::new(
                &format!("peer:{}", p.id),
                "pairing",
                &name,
                "Each pair of computers needs the same Machines protocol and relay setting.",
            );
            let link = state.machines.link(&p.id);
            let their_code = with_cache(state, |c| {
                c.peers
                    .get(&p.id)
                    .and_then(|pc| pc.facts.as_ref())
                    .and_then(|f| f.code.clone())
            });
            let code_differs = their_code
                .as_ref()
                .is_some_and(|c| !c.same_as(&settings.code));
            match (&p.versions, &p.relay) {
                (Some(v), _) if !super::protocol::compatible(v.protocol) => c.failed(
                    true,
                    format!("{name} runs Mother Claude {}, which speaks another Machines protocol.", v.app),
                    fix(FixKind::Manual, &format!("Update Mother Claude on {name}")),
                ),
                (_, Some(r)) if !r.same_as(&mine) => c.failed(
                    true,
                    format!(
                        "{name} uses another relay setting ({}): two computers on different relays may never meet.",
                        r.mode.as_str()
                    ),
                    fix(FixKind::Auto, "Use this setting everywhere"),
                ),
                _ if code_differs => c.failed(
                    false,
                    format!(
                        "{name} moves code with other options: a move follows the options of the computer it starts from."
                    ),
                    fix(FixKind::Auto, "Use this setting everywhere"),
                ),
                _ if !link.online => c.failed(
                    false,
                    format!("{name} is offline: its own checks show once it is back."),
                    None,
                ),
                _ => c.ok(format!(
                    "Online{}.",
                    match link.path {
                        Some(transport::PathKind::Lan) => ", direct on the same network",
                        Some(transport::PathKind::Internet) => ", direct over the internet",
                        Some(transport::PathKind::Relay) => ", through the relay",
                        None => "",
                    }
                )),
            }
        })
        .collect()
}

async fn relay_check(state: &AppState) -> Check {
    let relay = state.machines.settings().relay;
    let c = Check::new(
        "relay",
        "relay",
        "The relay",
        "Across networks, your computers meet through the relay.",
    );
    if relay.mode == RelayMode::None {
        let mut c = c.ok("No relay: your computers meet on the same network only.");
        c.state = CheckState::Na;
        return c;
    }
    let Some(endpoint) = transport::current(state).await else {
        return c.unknown("Checked once Machines is in use.");
    };
    use iroh::Watcher;
    let mut statuses = endpoint.home_relay_status();
    let mut current = statuses.get();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !current.iter().any(|s| s.is_connected()) && tokio::time::Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, statuses.updated()).await {
            Ok(Ok(v)) => current = v,
            _ => break,
        }
    }
    match current.iter().find(|s| s.is_connected()) {
        Some(s) => c.ok(format!(
            "Connected to {}.",
            s.url().host_str().unwrap_or("the relay")
        )),
        None => c.failed(
            false,
            "The relay does not answer: on the same network your computers still meet, across networks they can't.",
            fix(FixKind::Manual, "Check the relay"),
        ),
    }
}

// ─── the OS's own checks ───────────────────────────────────────────────────────

/// Runs a program (argv only, never a shell string) with a timeout; its
/// trimmed output when it succeeded.
async fn probe(program: &str, args: &[&str]) -> Option<String> {
    run(program, args)
        .await
        .filter(|(ok, _)| *ok)
        .map(|(_, out)| out)
}

/// Runs a program; `(succeeded, stdout)`.
async fn run(program: &str, args: &[&str]) -> Option<(bool, String)> {
    run_for(program, args, PROBE_TIMEOUT).await
}

/// [`run`] for a step that may take minutes (`claude update`).
async fn run_long(program: &str, args: &[&str]) -> Option<(bool, String)> {
    run_for(program, args, Duration::from_secs(300)).await
}

async fn run_for(program: &str, args: &[&str], timeout: Duration) -> Option<(bool, String)> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        // No console window flashing up from a GUI app.
        cmd.creation_flags(0x0800_0000);
    }
    let out = tokio::time::timeout(timeout, cmd.output())
        .await
        .ok()?
        .ok()?;
    Some((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
    ))
}

#[cfg(windows)]
async fn powershell(script: &str) -> Option<String> {
    probe(
        "powershell.exe",
        &[
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ],
    )
    .await
}

/// Minutes of idle before the computer sleeps (`0` never).
async fn sleep_timer() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        let out = probe("pmset", &["-g"]).await?;
        parse_pmset_sleep(&out)
    }
    #[cfg(windows)]
    {
        let out = probe(
            "powercfg",
            &["/query", "SCHEME_CURRENT", "SUB_SLEEP", "STANDBYIDLE"],
        )
        .await?;
        parse_powercfg_ac(&out).map(|secs| secs / 60)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        None
    }
}

/// `pmset -g`'s ` sleep  N` line.
pub fn parse_pmset_sleep(out: &str) -> Option<u64> {
    out.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        (it.next() == Some("sleep"))
            .then(|| it.next()?.parse().ok())
            .flatten()
    })
}

/// `powercfg /query …`'s current AC value (seconds).
pub fn parse_powercfg_ac(out: &str) -> Option<u64> {
    out.lines().find_map(|l| {
        let l = l.trim();
        let hex = l.strip_prefix("Current AC Power Setting Index:")?.trim();
        u64::from_str_radix(hex.trim_start_matches("0x"), 16).ok()
    })
}

async fn network_checks(required: bool) -> Vec<Check> {
    let mut out = Vec::new();
    #[cfg(target_os = "macos")]
    {
        out.push(mac_local_network(required).await);
        out.push(mac_firewall(required).await);
    }
    #[cfg(windows)]
    {
        out.extend(windows_network(required).await);
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = required;
    }
    out
}

#[cfg(target_os = "macos")]
async fn mac_local_network(required: bool) -> Check {
    let c = Check::new(
        "localNetwork",
        "network",
        "Local Network access",
        "Without it this Mac reaches your other computers only through the relay, even at home.",
    );
    let gateway = probe("route", &["-n", "get", "default"])
        .await
        .and_then(|o| {
            o.lines().find_map(|l| {
                l.trim()
                    .strip_prefix("gateway:")
                    .map(|g| g.trim().to_string())
            })
        });
    let Some(gateway) = gateway.and_then(|g| g.parse::<std::net::IpAddr>().ok()) else {
        return c.unknown("No network to test on right now.");
    };
    let refused = tokio::task::spawn_blocking(move || {
        let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
        let lan = socket.send_to(b"mother-claude", (gateway, 9)).err();
        let mdns = socket.send_to(&mdns_query(), ("224.0.0.251", 5353)).err();
        Some(
            [lan, mdns]
                .into_iter()
                .flatten()
                .any(|e| e.raw_os_error() == Some(libc::EHOSTUNREACH)),
        )
    })
    .await
    .ok()
    .flatten();
    match refused {
        Some(false) => c.ok("Mother Claude may reach your local network."),
        Some(true) => c.failed(
            required,
            "macOS does not let Mother Claude reach your local network. Allow it when macOS asks, or switch it on in System Settings › Privacy & Security › Local Network.",
            fix(FixKind::Settings, "Open Local Network settings"),
        ),
        None => c.unknown("Could not test the local network."),
    }
}

/// A well-formed mDNS query (for iroh's own service), so the probe sends
/// nothing odd onto the network.
#[cfg(target_os = "macos")]
fn mdns_query() -> Vec<u8> {
    let mut q = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in ["_irohv1", "_udp", "local"] {
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.extend_from_slice(&[0, 0, 12, 0, 1]);
    q
}

#[cfg(target_os = "macos")]
async fn mac_firewall(required: bool) -> Check {
    const FW: &str = "/usr/libexec/ApplicationFirewall/socketfilterfw";
    let c = Check::new(
        "firewall",
        "network",
        "The firewall",
        "If it blocks Mother Claude, your other computers reach this Mac only through the relay.",
    );
    let Some(global) = probe(FW, &["--getglobalstate"]).await else {
        return c.unknown("Could not read the firewall's state.");
    };
    if mac_firewall_off(&global) {
        return c.ok("The firewall is off.");
    }
    if probe(FW, &["--getblockall"])
        .await
        .is_some_and(|s| mac_block_all(&s))
    {
        return c.failed(
            required,
            "The firewall blocks all incoming connections.",
            fix(FixKind::Settings, "Open Firewall settings"),
        );
    }
    let app = app_path();
    match probe(FW, &["--getappblocked", &app]).await {
        Some(s) if s.contains("blocked") && !s.contains("permitted") => c.failed(
            required,
            "The firewall blocks Mother Claude.",
            fix(FixKind::Settings, "Open Firewall settings"),
        ),
        _ => c.ok("The firewall lets Mother Claude in."),
    }
}

/// `socketfilterfw --getglobalstate`: off?
pub fn mac_firewall_off(out: &str) -> bool {
    out.contains("State = 0") || out.to_ascii_lowercase().contains("is disabled")
}

/// `socketfilterfw --getblockall`: blocking everything?
pub fn mac_block_all(out: &str) -> bool {
    let l = out.to_ascii_lowercase();
    l.contains("block all") && l.contains("enabled")
}

/// This app's bundle (macOS) or executable.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn app_path() -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .unwrap_or(&exe)
        .to_string_lossy()
        .into_owned()
}

#[cfg(windows)]
async fn windows_network(required: bool) -> Vec<Check> {
    let profile_check = Check::new(
        "networkProfile",
        "network",
        "The network type",
        "On a Public network, Windows blocks incoming connections.",
    );
    let firewall_check = Check::new(
        "firewall",
        "network",
        "The firewall",
        "Without a rule, your other computers reach this PC only through the relay.",
    );
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let script = format!(
        "$cat = (Get-NetConnectionProfile | Select-Object -First 1).NetworkCategory; \
         $profiles = Get-NetFirewallProfile | ForEach-Object {{ [pscustomobject]@{{ Name = \"$($_.Name)\"; Enabled = \"$($_.Enabled)\" }} }}; \
         $rules = Get-NetFirewallApplicationFilter -Program {} -ErrorAction SilentlyContinue | Get-NetFirewallRule | \
           Where-Object {{ \"$($_.Direction)\" -eq 'Inbound' }} | ForEach-Object {{ [pscustomobject]@{{ Enabled = \"$($_.Enabled)\"; Action = \"$($_.Action)\"; Profile = \"$($_.Profile)\" }} }}; \
         [pscustomobject]@{{ category = \"$cat\"; profiles = @($profiles); rules = @($rules) }} | ConvertTo-Json -Compress -Depth 4",
        ps_quote(&exe)
    );
    let Some(out) = powershell(&script).await else {
        return vec![
            profile_check.unknown("Could not read the network type."),
            firewall_check.unknown("Could not read the firewall."),
        ];
    };
    let Some(state) = parse_windows_network(&out) else {
        return vec![
            profile_check.unknown("Could not read the network type."),
            firewall_check.unknown("Could not read the firewall."),
        ];
    };
    vec![
        windows_profile_check(profile_check, &state, required),
        windows_firewall_check(firewall_check, &state, required),
    ]
}

/// What the Windows network probe read.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct WindowsNetwork {
    /// `Private` | `Public` | `DomainAuthenticated` | empty when offline.
    pub category: String,
    /// The firewall profile in force is on.
    pub firewall_on: bool,
    /// An enabled inbound rule allows this app on the profile in force.
    pub allowed: bool,
    /// An enabled inbound rule blocks this app on the profile in force.
    pub blocked: bool,
}

/// Parses the probe's JSON.
pub fn parse_windows_network(json_text: &str) -> Option<WindowsNetwork> {
    let v: Value = serde_json::from_str(json_text).ok()?;
    let category = v
        .get("category")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let profile = match category.as_str() {
        "DomainAuthenticated" => "Domain".to_string(),
        "" => "Public".to_string(),
        other => other.to_string(),
    };
    let as_list = |x: Option<&Value>| -> Vec<Value> {
        match x {
            Some(Value::Array(a)) => a.clone(),
            Some(Value::Object(_)) => vec![x.cloned().unwrap_or_default()],
            _ => Vec::new(),
        }
    };
    let field = |o: &Value, k: &str| o.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let firewall_on = as_list(v.get("profiles"))
        .iter()
        .any(|p| field(p, "Name") == profile && field(p, "Enabled") == "True");
    let covers = |p: &str| p == "Any" || p.split(',').any(|x| x.trim() == profile);
    let rules = as_list(v.get("rules"));
    let matching = |action: &str| {
        rules.iter().any(|r| {
            field(r, "Enabled") == "True"
                && field(r, "Action") == action
                && covers(&field(r, "Profile"))
        })
    };
    Some(WindowsNetwork {
        category,
        firewall_on,
        allowed: matching("Allow"),
        blocked: matching("Block"),
    })
}

#[cfg_attr(not(windows), allow(dead_code))]
fn windows_profile_check(c: Check, n: &WindowsNetwork, required: bool) -> Check {
    match n.category.as_str() {
        "Private" | "DomainAuthenticated" => c.ok(format!("This network is {}.", n.category)),
        "Public" if !n.firewall_on => c.ok("This network is Public, but its firewall is off."),
        "Public" => c.failed(
            required,
            "This network is Public: Windows blocks incoming connections on it. Set it to Private if it is your home or office network.",
            fix(FixKind::Settings, "Open network settings"),
        ),
        _ => c.unknown("Not connected to a network right now."),
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
fn windows_firewall_check(c: Check, n: &WindowsNetwork, required: bool) -> Check {
    if !n.firewall_on {
        return c.ok("The firewall is off for this network.");
    }
    if n.blocked {
        return c.failed(
            required,
            "A firewall rule blocks Mother Claude (Windows makes one when its first prompt is cancelled).",
            fix(FixKind::Admin, "Allow through the firewall"),
        );
    }
    if n.allowed {
        return c.ok("A firewall rule lets Mother Claude in.");
    }
    c.failed(
        required,
        "No firewall rule lets your other computers reach Mother Claude.",
        fix(FixKind::Admin, "Allow through the firewall"),
    )
}

/// A PowerShell single-quoted string (typographic quotes doubled too).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn ps_quote(s: &str) -> String {
    let mut out = String::from("'");
    for c in s.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
        out.push(c);
    }
    out.push('\'');
    out
}

// ─── fixes ─────────────────────────────────────────────────────────────────────

/// Records what a fix would do while fixes only record (tests): the
/// settings page it would open, the administrator prompt, the sign-in,
/// `claude update`, the clone.
pub fn note(state: &AppState, what: &str) {
    state
        .machines
        .fixes_asked
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(what.to_string());
}

/// Whether fixes run for real (the app turned them on).
pub fn fixes_live(state: &AppState) -> bool {
    state.machines.fixes_live.load(Ordering::Relaxed)
}

/// `POST /api/machines/setup/fix` (desktop only): runs a check's fix.
pub async fn fix_check(state: &AppState, id: &str) -> Result<(), ApiError> {
    let live = fixes_live(state);
    let record = |what: &str| note(state, what);
    match id {
        "keepAwake" => {
            state.machines.store_write(|f| f.settings.keep_awake = true);
            super::awake::update(state);
        }
        "localNetwork" | "firewall" | "networkProfile" => {
            let pane = match (id, cfg!(windows)) {
                ("localNetwork", _) => "local-network",
                ("networkProfile", _) => "network-status",
                ("firewall", true) => return admin_firewall_fix(state, live, record).await,
                _ => "firewall",
            };
            record(&format!("open {pane}"));
            if live {
                crate::permissions::open_settings_pane(pane)
                    .map_err(|e| ApiError::unprocessable("fix_failed", e))?;
            }
        }
        _ if id.starts_with("peer:") => {
            // "Use this setting everywhere": offer this machine's relay and
            // code options.
            super::routes::offer_settings(state).await;
        }
        "longPaths" => {
            // Git for Windows: for this user only (`~/.gitconfig`).
            record("git config --global core.longpaths true");
            if live {
                let done = tokio::task::spawn_blocking(|| {
                    let git = super::code::GitRunner::new();
                    let op = git.begin().map_err(|e| e.to_string())?;
                    let dir = op.dir().to_path_buf();
                    op.git(&dir, &["config", "--global", "core.longpaths", "true"])
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                })
                .await
                .map_err(|e| ApiError::internal(e.to_string()))?;
                done.map_err(|e| ApiError::unprocessable("fix_failed", e))?;
            }
        }
        "claudeCode" => {
            // `claude update`, run because the user pressed it here.
            record("claude update");
            if live {
                let ran = run_long(&crate::claude::claude_bin(), &["update"]).await;
                if !matches!(ran, Some((true, _))) {
                    return Err(ApiError::unprocessable(
                        "fix_failed",
                        "Claude Code could not update itself: run `claude update` in a terminal, or reinstall it.",
                    ));
                }
            }
        }
        _ => match id.strip_prefix("account:") {
            // An account's own sign-in, its browser here.
            Some(account) => accounts::sign_in(state, account).await?,
            None => {
                return Err(ApiError::unprocessable(
                    "fix_manual",
                    "This one is done by hand: the check says how.",
                ))
            }
        },
    }
    refresh(state).await;
    Ok(())
}

/// Windows: an allow rule for Mother Claude on the Private profile, and its
/// own block rules removed — after the UAC prompt.
async fn admin_firewall_fix(
    state: &AppState,
    live: bool,
    record: impl Fn(&str),
) -> Result<(), ApiError> {
    record("admin firewall");
    if !live {
        return Ok(());
    }
    #[cfg(windows)]
    {
        let exe = std::env::current_exe()
            .map(|p| p.to_string_lossy().into_owned())
            .map_err(|e| ApiError::internal(e.to_string()))?;
        let inner = format!(
            "$exe = {exe}; \
             Get-NetFirewallApplicationFilter -Program $exe -ErrorAction SilentlyContinue | Get-NetFirewallRule | \
               Where-Object {{ \"$($_.Direction)\" -eq 'Inbound' -and \"$($_.Action)\" -eq 'Block' }} | Remove-NetFirewallRule; \
             if (-not (Get-NetFirewallRule -DisplayName 'Mother Claude (Machines)' -ErrorAction SilentlyContinue)) {{ \
               New-NetFirewallRule -DisplayName 'Mother Claude (Machines)' -Program $exe -Direction Inbound -Action Allow -Protocol UDP -Profile Private | Out-Null }}",
            exe = ps_quote(&exe)
        );
        let encoded = encode_ps(&inner);
        let outer = format!(
            "Start-Process powershell.exe -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList \
             '-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-EncodedCommand','{encoded}'"
        );
        let ran = run(
            "powershell.exe",
            &[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &outer,
            ],
        )
        .await;
        match ran {
            Some((true, _)) => {
                refresh(state).await;
                Ok(())
            }
            _ => Err(ApiError::unprocessable(
                "fix_declined",
                "The firewall was not changed: the administrator prompt was declined or failed.",
            )),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = state;
        Err(ApiError::unprocessable(
            "fix_manual",
            "Open the firewall settings.",
        ))
    }
}

/// PowerShell's `-EncodedCommand`: base64 of UTF-16LE.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn encode_ps(script: &str) -> String {
    use base64::Engine;
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

// ─── the views, health and the readiness test ───────────────────────────────────

/// `GET /api/machines/setup`: this machine's checks and every paired one's.
pub async fn view(state: &AppState, fresh: bool) -> Value {
    let stale = with_cache(state, |c| {
        c.at.is_none_or(|t| super::now() - t > 30_000) || c.checks.is_empty()
    });
    if fresh || stale {
        refresh(state).await;
    }
    if fresh {
        fetch_peer_checks(state).await;
    }
    let (at, checks, peers, tests) = with_cache(state, |c| {
        (c.at, c.checks.clone(), c.peers.clone(), c.tests.clone())
    });
    let names: Vec<(String, String)> = state.machines.store_read(|f| {
        f.peers
            .iter()
            .map(|p| (p.id.clone(), p.display_name().to_string()))
            .collect()
    });
    let me = super::machine_info(state);
    let peer_list: Vec<Value> = names
        .iter()
        .map(|(id, name)| {
            let pc = peers.get(id).cloned().unwrap_or_default();
            json!({
                "machine": id,
                "name": name,
                "at": (pc.at > 0).then_some(pc.at),
                "checks": pc.checks,
                "error": pc.error,
                "facts": pc.facts,
                "asks": pc.asks,
            })
        })
        .collect();
    let (blockers, warnings) = count(&checks, peers.values());
    let gating = gating(&checks, peers.values());
    json!({
        "self": {
            "machine": me.id, "name": me.name, "at": at, "checks": checks, "error": null,
            "facts": facts(state),
        },
        "peers": peer_list,
        "blockers": blockers,
        "gating": gating,
        "warnings": warnings,
        "tests": tests.values().collect::<Vec<_>>(),
    })
}

/// The blockers that keep Machines off (a scoped one blocks only its part).
fn gating<'a>(own: &[Check], peers: impl Iterator<Item = &'a PeerChecks>) -> usize {
    own.iter()
        .chain(peers.flat_map(|p| p.checks.iter()))
        .filter(|c| c.gates())
        .count()
}

fn count<'a>(own: &[Check], peers: impl Iterator<Item = &'a PeerChecks>) -> (usize, usize) {
    let all = own.iter().chain(peers.flat_map(|p| p.checks.iter()));
    let mut b = 0;
    let mut w = 0;
    for c in all {
        match c.state {
            CheckState::Blocker => b += 1,
            CheckState::Warning => w += 1,
            _ => {}
        }
    }
    (b, w)
}

/// This machine's blockers right now (for turning Machines on): a blocker
/// with a scope (an account not signed in) blocks only its part.
pub fn blockers(state: &AppState) -> Vec<Check> {
    with_cache(state, |c| {
        c.checks
            .iter()
            .chain(c.peers.values().flat_map(|p| p.checks.iter()))
            .filter(|c| c.gates())
            .cloned()
            .collect()
    })
}

/// The health card's summary.
pub fn health_summary(state: &AppState) -> Value {
    with_cache(state, |c| {
        let (b, w) = count(&c.checks, c.peers.values());
        let s = if c.at.is_none() {
            "unknown"
        } else if b > 0 {
            "blocker"
        } else if w > 0 {
            "warning"
        } else {
            "ok"
        };
        json!({ "state": s, "blockers": b, "warnings": w, "at": c.at })
    })
}

/// One paired machine's checks, tools and the asks this machine made of it.
async fn fetch_one(state: &AppState, id: &str) -> PeerChecks {
    let r = transport::request(
        state,
        id,
        "GET",
        "/peer/setup",
        None,
        Duration::from_secs(25),
    )
    .await;
    let empty = |error: String| PeerChecks {
        at: super::now(),
        error: Some(error),
        ..PeerChecks::default()
    };
    match r {
        Ok((200, v)) => PeerChecks {
            at: super::now(),
            checks: serde_json::from_value(v.get("checks").cloned().unwrap_or_default())
                .unwrap_or_default(),
            error: None,
            facts: v
                .get("facts")
                .and_then(|f| serde_json::from_value(f.clone()).ok()),
            asks: v
                .get("asks")
                .and_then(|a| serde_json::from_value::<Vec<asks::Ask>>(a.clone()).ok())
                .unwrap_or_default()
                .into_iter()
                .filter(|a| asks::is_ask_id(&a.id))
                .collect(),
        },
        Ok((status, _)) => empty(format!("It answered {status}.")),
        Err(e) => empty(e),
    }
}

/// Keeps `results` (an unreachable machine keeps its last checks, with the
/// error) and forgets machines no longer paired.
fn keep_peer_checks(state: &AppState, results: Vec<(String, PeerChecks)>) {
    let known: Vec<String> = state
        .machines
        .store_read(|f| f.peers.iter().map(|p| p.id.clone()).collect());
    with_cache(state, |c| {
        for (id, pc) in results {
            let entry = c.peers.entry(id).or_default();
            if pc.error.is_some() {
                entry.error = pc.error;
            } else {
                *entry = pc;
            }
        }
        c.peers.retain(|id, _| known.contains(id));
        c.save();
    });
    super::publish(state, "setup");
}

/// Asks every online paired machine for its checks.
pub async fn fetch_peer_checks(state: &AppState) {
    let peers: Vec<String> = state
        .machines
        .store_read(|f| f.peers.iter().map(|p| p.id.clone()).collect());
    let asks = peers.into_iter().map(|id| {
        let st = state.clone();
        async move {
            let pc = fetch_one(&st, &id).await;
            (id, pc)
        }
    });
    let results = futures_util::future::join_all(asks).await;
    keep_peer_checks(state, results);
}

/// Asks one paired machine for its checks again (it said they changed).
pub async fn fetch_peer_checks_of(state: &AppState, machine: &str) {
    let pc = fetch_one(state, machine).await;
    keep_peer_checks(state, vec![(machine.to_string(), pc)]);
}

/// Remembers an ask this machine just made of `machine` (until its next
/// answer to `/peer/setup` says how it went).
pub fn note_asked(state: &AppState, machine: &str, ask: asks::Ask) {
    with_cache(state, |c| {
        let entry = c.peers.entry(machine.to_string()).or_default();
        entry.asks.retain(|a| a.id != ask.id);
        entry.asks.push(ask);
        c.save();
    });
    super::publish(state, "setup");
}

/// The readiness test with `machine`, or every paired machine.
pub async fn readiness(state: &AppState, machine: Option<&str>) -> Value {
    let peers: Vec<(String, String)> = state.machines.store_read(|f| {
        f.peers
            .iter()
            .filter(|p| machine.is_none_or(|m| m == p.id))
            .map(|p| (p.id.clone(), p.display_name().to_string()))
            .collect()
    });
    let runs = peers.into_iter().map(|(id, name)| {
        let st = state.clone();
        async move { test_one(&st, &id, &name).await }
    });
    let results = futures_util::future::join_all(runs).await;
    with_cache(state, |c| {
        for r in &results {
            c.tests.insert(r.machine.clone(), r.clone());
        }
        c.save();
    });
    super::publish(state, "setup");
    json!({ "results": results })
}

async fn test_one(state: &AppState, machine: &str, name: &str) -> Readiness {
    let mut tests = Vec::new();
    let key = state
        .machines
        .store_read(|f| f.peer(machine).map(|p| p.key.clone()))
        .unwrap_or_default();
    let started = std::time::Instant::now();
    let connected = tokio::time::timeout(
        Duration::from_secs(25),
        transport::connection_to(state, &key),
    )
    .await;
    match connected {
        Ok(Ok(conn)) => {
            let path = transport::path_of(&conn);
            let rtt = transport::rtt_of(&conn);
            tests.push(TestLine {
                id: "connect".into(),
                state: "ok".into(),
                detail: match path {
                    transport::PathKind::Lan => "Direct, on the same network.".into(),
                    transport::PathKind::Internet => "Direct, over the internet.".into(),
                    transport::PathKind::Relay => "Through the relay (slower).".into(),
                },
                ms: rtt.or(Some(started.elapsed().as_secs_f64() * 1000.0)),
                mibps: None,
            });
            match transport::bulk_from(state, machine, TEST_MIB).await {
                Ok((bytes, took)) => {
                    let mib = bytes as f64 / (1u64 << 20) as f64;
                    let rate = mib / took.as_secs_f64().max(1e-6);
                    tests.push(TestLine {
                        id: "transfer".into(),
                        state: "ok".into(),
                        detail: format!("{mib:.0} MiB at {rate:.1} MiB/s."),
                        ms: Some((took.as_secs_f64() * 1000.0).round()),
                        mibps: Some((rate * 10.0).round() / 10.0),
                    });
                }
                Err(e) => tests.push(TestLine {
                    id: "transfer".into(),
                    state: "failed".into(),
                    detail: format!("The transfer failed: {e}"),
                    ms: None,
                    mibps: None,
                }),
            }
        }
        Ok(Err(e)) => {
            tests.push(failed_connect(&format!("Can't reach {name}: {e}")));
            tests.push(skipped_transfer());
        }
        Err(_) => {
            tests.push(failed_connect(&format!("Can't reach {name} in time.")));
            tests.push(skipped_transfer());
        }
    }
    Readiness {
        machine: machine.to_string(),
        name: name.to_string(),
        tests,
        at: super::now(),
    }
}

fn failed_connect(detail: &str) -> TestLine {
    TestLine {
        id: "connect".into(),
        state: "failed".into(),
        detail: detail.into(),
        ms: None,
        mibps: None,
    }
}

fn skipped_transfer() -> TestLine {
    TestLine {
        id: "transfer".into(),
        state: "skipped".into(),
        detail: "Skipped: not connected.".into(),
        ms: None,
        mibps: None,
    }
}

/// Machines health: the checks at start and every 30 minutes (the paired
/// machines' too while the transport runs), once Machines is in use.
pub async fn health_loop(state: AppState) {
    let weak = std::sync::Arc::downgrade(&state);
    drop(state);
    loop {
        {
            let Some(state) = weak.upgrade() else { break };
            // Only once Machines is in use: on, paired, or its setup begun.
            // Nobody else pays for the probes.
            let in_use = state
                .machines
                .store_read(|f| f.enabled || !f.peers.is_empty() || f.setup_step.is_some());
            if in_use {
                refresh(&state).await;
                if transport::current(&state).await.is_some() {
                    fetch_peer_checks(&state).await;
                }
            }
        }
        tokio::time::sleep(HEALTH_EVERY).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pmset_and_powercfg_timers_parse() {
        let pm = " System-wide power settings:\nCurrently in use:\n standby              1\n sleep                1 (sleep prevented by coreaudiod)\n displaysleep         0\n";
        assert_eq!(parse_pmset_sleep(pm), Some(1));
        assert_eq!(parse_pmset_sleep("displaysleep 3"), None);
        let pc = "    Current AC Power Setting Index: 0x00000708\n    Current DC Power Setting Index: 0x00000384";
        assert_eq!(parse_powercfg_ac(pc), Some(1800));
        assert_eq!(
            parse_powercfg_ac("Current AC Power Setting Index: 0x00000000"),
            Some(0)
        );
    }

    #[test]
    fn the_macos_firewall_outputs_read_as_recorded() {
        assert!(mac_firewall_off("Firewall is disabled. (State = 0)"));
        assert!(!mac_firewall_off("Firewall is enabled. (State = 1)"));
        assert!(mac_block_all(
            "Firewall has block all state set to enabled."
        ));
        assert!(!mac_block_all(
            "Firewall has block all state set to disabled."
        ));
    }

    #[test]
    fn the_windows_network_probe_reads_profiles_and_rules() {
        // As recorded on the spike's Windows desktop: firewall off everywhere.
        let off = r#"{"category":"Private","profiles":[{"Name":"Domain","Enabled":"False"},{"Name":"Private","Enabled":"False"},{"Name":"Public","Enabled":"False"}],"rules":[]}"#;
        let n = parse_windows_network(off).unwrap();
        assert_eq!(n.category, "Private");
        assert!(!n.firewall_on && !n.allowed && !n.blocked);
        let c = windows_firewall_check(Check::new("firewall", "network", "", ""), &n, true);
        assert_eq!(c.state, CheckState::Ok);

        // On, Public, with the block rules a cancelled prompt leaves.
        let blocked = r#"{"category":"Public","profiles":[{"Name":"Public","Enabled":"True"}],"rules":[{"Enabled":"True","Action":"Block","Profile":"Public"},{"Enabled":"True","Action":"Block","Profile":"Private, Public"}]}"#;
        let n = parse_windows_network(blocked).unwrap();
        assert!(n.firewall_on && n.blocked && !n.allowed);
        let fw = windows_firewall_check(Check::new("firewall", "network", "", ""), &n, false);
        assert_eq!(
            fw.state,
            CheckState::Warning,
            "recommended while a relay is set"
        );
        assert_eq!(fw.fix.unwrap().kind, FixKind::Admin);
        let prof = windows_profile_check(Check::new("networkProfile", "network", "", ""), &n, true);
        assert_eq!(prof.state, CheckState::Blocker, "required with no relay");

        // One rule object (PowerShell does not wrap a single item), allowing.
        let allowed = r#"{"category":"Private","profiles":{"Name":"Private","Enabled":"True"},"rules":{"Enabled":"True","Action":"Allow","Profile":"Any"}}"#;
        let n = parse_windows_network(allowed).unwrap();
        assert!(n.firewall_on && n.allowed && !n.blocked);
        assert!(parse_windows_network("not json").is_none());
    }

    #[test]
    fn powershell_quoting_and_encoding_are_safe() {
        assert_eq!(
            ps_quote(r"C:\Program Files\Mother Claude\a.exe"),
            r"'C:\Program Files\Mother Claude\a.exe'"
        );
        assert_eq!(ps_quote("it's"), "'it''s'");
        assert_eq!(ps_quote("it\u{2019}s"), "'it\u{2019}\u{2019}s'");
        // "ab" in UTF-16LE is 61 00 62 00.
        assert_eq!(encode_ps("ab"), "YQBiAA==");
    }

    #[test]
    fn a_failure_is_required_or_recommended() {
        let c =
            Check::new("x", "network", "X", "why").failed(true, "bad", fix(FixKind::Auto, "Fix"));
        assert_eq!(
            (c.state, c.severity),
            (CheckState::Blocker, Severity::Required)
        );
        let c = Check::new("x", "network", "X", "why").failed(false, "meh", None);
        assert_eq!(
            (c.state, c.severity),
            (CheckState::Warning, Severity::Recommended)
        );
    }
}
