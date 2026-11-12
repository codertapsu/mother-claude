//! What machines say to each other (docs/PLAN-MACHINES.md §8): the ALPNs,
//! the protocol version, capabilities and the `hello` handshake.

use serde::{Deserialize, Serialize};

use super::grants::Grants;
use super::store::RelaySetting;

/// The peer protocol: HTTP/1.1, one connection per QUIC stream, between two
/// paired machine keys.
pub const ALPN_PEER: &[u8] = b"mother-claude/peer/1";

/// The pairing rendezvous: the short-lived endpoint whose key is derived from
/// a pairing code (`pairing.rs`). It only hands the code's holder this
/// machine's address.
pub const ALPN_PAIR: &[u8] = b"mother-claude/pair/1";

/// The Machines protocol version. A feature is used only when both sides
/// list it in their capabilities, so machines one version apart still see
/// each other.
pub const PROTOCOL: u32 = 1;

/// What this build can do with another machine.
pub const CAPABILITIES: &[&str] = &[
    "hello",
    "setup",
    "settings",
    "bulk",
    super::roots::CAPABILITY,
    MOVES,
    IGNORED_FILES,
    super::work::CAPABILITY,
    super::work::tasks::CAPABILITY,
    super::work::roles::CAPABILITY,
];

/// The capability a machine needs to take (and send) sessions.
pub const MOVES: &str = "moves";

/// The capability a machine needs to take the files git ignores that a
/// project carries with a move (`moves::ignored`).
pub const IGNORED_FILES: &str = "ignoredFiles";

/// Versions a machine reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Versions {
    /// Mother Claude's version.
    pub app: String,
    /// The Machines protocol version.
    #[serde(default)]
    pub protocol: u32,
    /// The Default account's Claude Code version, when known.
    #[serde(default)]
    pub claude: Option<String>,
}

impl Versions {
    pub fn this_build(claude: Option<String>) -> Self {
        Self {
            app: env!("CARGO_PKG_VERSION").to_string(),
            protocol: PROTOCOL,
            claude,
        }
    }
}

/// Who a machine is, as it tells another.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineInfo {
    pub id: String,
    pub name: String,
    /// `macos` | `windows` | `linux` | `other`.
    pub os: String,
    #[serde(default)]
    pub os_version: Option<String>,
    pub versions: Versions,
    /// Its machine key (hex).
    pub key: String,
    /// The shell a run uses there (`zsh`, `pwsh`, `powershell`, …: Phase 4,
    /// `work::shell`). Absent from an older build.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
}

/// `POST /peer/hello`, both ways: who I am, my relay setting, what you may do
/// on me, and what I can do.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub machine: MachineInfo,
    pub relay: RelaySetting,
    /// What the receiver may do on the sender.
    pub grants_you_have: Grants,
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// The digest of the sender's roots, tombstones included, when it keeps
    /// roots: the other side exchanges them when its own differ
    /// (`roots::after_hello`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roots_digest: Option<String>,
    /// The sender's leases (docs/PLAN-MACHINES.md §5.7): which machine each
    /// session lives on, merged by the receiver when the sender holds its
    /// `move` grant. Absent from an older build.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub leases: Vec<super::lease::WireLease>,
}

/// This OS, as machines name it.
pub fn this_os() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "other"
    }
}

/// Whether a peer's protocol is one this build speaks.
pub fn compatible(protocol: u32) -> bool {
    protocol == PROTOCOL
}
