//! Versions and skew between this computer and another (docs/PLAN-MACHINES.md
//! §8, Phase 5): which one runs the older Mother Claude or Claude Code, and
//! what this build does with other computers that another one can't until
//! it updates (the capabilities its hello lacks). The Machines page names
//! them; an incompatible Machines protocol stays a blocker of the setup
//! (`setup::peer_checks`), an older Mother Claude is a warning there
//! (`setup::app_check`, *Update it*), and an older Claude Code too
//! (`setup::checks::claude_check`).

use serde::Serialize;

use super::protocol::{self, Versions};
use super::setup::checks::older;
use super::store::PeerRecord;

/// The capabilities the Machines page names when another computer lacks
/// them: what a user does with it (moves, code sync, runs, tasks, team
/// roles, *Revoke everywhere*). The handshake's own
/// (`hello`, `setup`, `settings`, `bulk`) came with protocol 1 and are left
/// out.
pub const NAMED: &[&str] = &[
    protocol::MOVES,
    protocol::IGNORED_FILES,
    super::roots::CAPABILITY,
    protocol::SYNC,
    super::work::CAPABILITY,
    super::work::tasks::CAPABILITY,
    super::work::roles::CAPABILITY,
    protocol::REVOKE_EVERYWHERE,
];

/// Another computer's version against this one's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Order {
    /// Its version is older than this computer's.
    Older,
    /// Newer.
    Newer,
}

/// `theirs` against `mine`; `None` when they are the same, or either is not
/// a version.
pub fn order(theirs: Option<&str>, mine: Option<&str>) -> Option<Order> {
    let (t, m) = (theirs?, mine?);
    if older(t, m) {
        Some(Order::Older)
    } else if older(m, t) {
        Some(Order::Newer)
    } else {
        None
    }
}

/// How another computer differs from this one (`GET /api/machines`,
/// `peers[].skew`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skew {
    /// Its Mother Claude against this computer's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<Order>,
    /// Its Claude Code (the Default account's) against this computer's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claude: Option<Order>,
    /// What this computer does with others that it can't until it updates
    /// ([`NAMED`], in that order). Empty until it said hello.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lacks: Vec<String>,
}

/// How `peer` differs from this build (`mine`: its versions).
pub fn skew(peer: &PeerRecord, mine: &Versions) -> Skew {
    let Some(theirs) = &peer.versions else {
        return Skew::default();
    };
    Skew {
        app: order(Some(&theirs.app), Some(&mine.app)),
        claude: order(theirs.claude.as_deref(), mine.claude.as_deref()),
        lacks: lacks(&peer.capabilities),
    }
}

/// The named capabilities this build has that `theirs` does not list; none
/// while it has listed nothing (it has not said hello yet).
pub fn lacks(theirs: &[String]) -> Vec<String> {
    if theirs.is_empty() {
        return Vec::new();
    }
    NAMED
        .iter()
        .filter(|c| protocol::CAPABILITIES.contains(c) && !theirs.iter().any(|t| t == *c))
        .map(|c| c.to_string())
        .collect()
}

/// Whether `peer` lists `capability`, or is too old to list any (it has not
/// said hello yet: try, and let its answer tell).
pub fn may_have(peer: &PeerRecord, capability: &str) -> bool {
    peer.capabilities.is_empty() || peer.capabilities.iter().any(|c| c == capability)
}

/// The newest Mother Claude among the paired computers that is newer than
/// this one's: `(its name, its version)` (the setup's `app` check).
pub fn newer_app<'a>(
    peers: impl IntoIterator<Item = &'a PeerRecord>,
    mine: &str,
) -> Option<(String, String)> {
    peers
        .into_iter()
        .filter_map(|p| {
            let v = p.versions.as_ref()?;
            older(mine, &v.app).then(|| (p.display_name().to_string(), v.app.clone()))
        })
        .reduce(|a, b| if older(&a.1, &b.1) { b } else { a })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(app: &str, claude: Option<&str>, caps: &[&str]) -> PeerRecord {
        let mut p = crate::machines::pairing::peer_record(
            &protocol::MachineInfo {
                id: "a".repeat(32),
                name: "DESKTOP-PC".into(),
                os: "windows".into(),
                os_version: None,
                versions: Versions {
                    app: app.into(),
                    protocol: protocol::PROTOCOL,
                    claude: claude.map(str::to_string),
                },
                key: "b".repeat(64),
                shell: None,
            },
            crate::machines::grants::Grants::defaults(),
            None,
        );
        p.capabilities = caps.iter().map(|c| c.to_string()).collect();
        p
    }

    fn mine(app: &str, claude: Option<&str>) -> Versions {
        Versions {
            app: app.into(),
            protocol: protocol::PROTOCOL,
            claude: claude.map(str::to_string),
        }
    }

    #[test]
    fn versions_order_by_their_numbers() {
        assert_eq!(order(Some("0.11.0"), Some("0.12.0")), Some(Order::Older));
        assert_eq!(order(Some("0.12.1"), Some("0.12.0")), Some(Order::Newer));
        assert_eq!(
            order(Some("0.12"), Some("0.12.0")),
            None,
            "missing parts are 0"
        );
        assert_eq!(order(Some("2.1.330"), Some("2.1.322")), Some(Order::Newer));
        assert_eq!(order(None, Some("0.12.0")), None);
        assert_eq!(order(Some("unknown"), Some("0.12.0")), None);
    }

    #[test]
    fn the_skew_names_what_an_older_computer_lacks() {
        let all: Vec<&str> = protocol::CAPABILITIES.to_vec();
        let same = peer(env!("CARGO_PKG_VERSION"), Some("2.1.322"), &all);
        let me = mine(env!("CARGO_PKG_VERSION"), Some("2.1.322"));
        assert_eq!(skew(&same, &me), Skew::default(), "nothing differs");

        let old = peer("0.0.1", Some("2.1.300"), &["hello", "setup", "moves"]);
        let s = skew(&old, &me);
        assert_eq!(s.app, Some(Order::Older));
        assert_eq!(s.claude, Some(Order::Older));
        assert!(s.lacks.contains(&"tasks".to_string()), "{s:?}");
        assert!(!s.lacks.contains(&"moves".to_string()));
        assert!(!s.lacks.iter().any(|c| c == "hello" || c == "bulk"));

        // Nothing said yet: nothing named.
        let quiet = peer("0.0.1", None, &[]);
        assert!(skew(&quiet, &me).lacks.is_empty());
        assert!(may_have(&quiet, "tasks"));
        assert!(!may_have(&old, "tasks"));
    }

    #[test]
    fn the_newest_newer_app_is_named() {
        let a = peer("0.12.0", None, &[]);
        let mut b = peer("0.13.1", None, &[]);
        b.name = "BUILD-BOX".into();
        let c = peer("0.10.0", None, &[]);
        assert_eq!(
            newer_app([&a, &b, &c], "0.11.0"),
            Some(("BUILD-BOX".to_string(), "0.13.1".to_string()))
        );
        assert_eq!(newer_app([&c], "0.11.0"), None);
    }
}
