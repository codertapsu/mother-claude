//! Hook session binding: does this hook request really come from the session
//! it names?
//!
//! The hook token sits in a 0600 file, but any process running as the user
//! (including a prompt-injected model's Bash) can read it. With it alone, a
//! forger could plant approval cards, win the wake listener and read the
//! phone's messages, or consume a relay / withdraw cards with a fake Stop.
//! So before anything sensitive happens for session S, the TCP connection is
//! traced back: some descendant of S's claude process (the hook's `sh`, then
//! `curl`) must own the socket whose local port is the request's peer port.
//!
//! The check needs the live roster's pid for S (never a pid the request
//! names) plus `ps` and `lsof`; when either is missing or slow the answer is
//! [`Binding::Unknown`] and the caller carries on, so a missing tool never
//! breaks foreign control.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::time::Duration;

/// The outcome of a check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    /// A descendant of the session's process owns the connection.
    Verified,
    /// Nobody in the session's process tree owns it: forged.
    Mismatch,
    /// Could not tell (tools missing, too slow, not loopback).
    Unknown,
}

/// The whole budget of one check.
const BUDGET: Duration = Duration::from_millis(1500);
/// How deep below the claude process to look (claude → sh → curl).
const MAX_DEPTH: usize = 6;

/// `ps -A -o pid=,ppid=` output → children by parent.
pub fn parse_ps_tree(out: &str) -> HashMap<u32, Vec<u32>> {
    let mut tree: HashMap<u32, Vec<u32>> = HashMap::new();
    for line in out.lines() {
        let mut it = line.split_whitespace();
        let (Some(pid), Some(ppid)) = (it.next(), it.next()) else {
            continue;
        };
        if let (Ok(pid), Ok(ppid)) = (pid.parse::<u32>(), ppid.parse::<u32>()) {
            tree.entry(ppid).or_default().push(pid);
        }
    }
    tree
}

/// Every descendant of `root` (not `root` itself), breadth first.
pub fn descendants(tree: &HashMap<u32, Vec<u32>>, root: u32, max_depth: usize) -> Vec<u32> {
    let mut out = Vec::new();
    let mut seen = HashSet::from([root]);
    let mut queue = VecDeque::from([(root, 0usize)]);
    while let Some((pid, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }
        for &child in tree.get(&pid).map(Vec::as_slice).unwrap_or_default() {
            if seen.insert(child) {
                out.push(child);
                queue.push_back((child, depth + 1));
            }
        }
    }
    out
}

/// Local ports of the TCP sockets in `lsof -F pn` output.
pub fn parse_lsof_local_ports(out: &str) -> Vec<u16> {
    out.lines()
        .filter_map(|l| l.strip_prefix('n'))
        .filter_map(|name| {
            let local = name.split("->").next()?;
            local.rsplit(':').next()?.parse::<u16>().ok()
        })
        .collect()
}

async fn output(cmd: &str, args: &[String]) -> Option<String> {
    let out = tokio::process::Command::new(cmd)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    // lsof exits 1 when some pid has no matching file: the listing is still
    // good.
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn lsof_bin() -> &'static str {
    ["/usr/sbin/lsof", "/usr/bin/lsof", "/sbin/lsof"]
        .into_iter()
        .find(|p| std::path::Path::new(p).is_file())
        .unwrap_or("lsof")
}

async fn check(claude_pid: u32, peer: SocketAddr) -> Binding {
    let Some(ps) = output("ps", &["-A", "-o", "pid=,ppid="].map(String::from)).await else {
        return Binding::Unknown;
    };
    let tree = parse_ps_tree(&ps);
    if tree.is_empty() {
        return Binding::Unknown;
    }
    let me = std::process::id();
    let pids: Vec<u32> = descendants(&tree, claude_pid, MAX_DEPTH)
        .into_iter()
        .filter(|p| *p != me)
        .collect();
    if pids.is_empty() {
        return Binding::Mismatch;
    }
    let list = pids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let args = ["-nP", "-a", "-p", &list, "-iTCP", "-F", "pn"].map(String::from);
    let Some(lsof) = output(lsof_bin(), &args).await else {
        return Binding::Unknown;
    };
    if parse_lsof_local_ports(&lsof).contains(&peer.port()) {
        Binding::Verified
    } else {
        Binding::Mismatch
    }
}

/// Whether the connection from `peer` belongs to `claude_pid`'s process
/// tree.
pub async fn verify(claude_pid: u32, peer: SocketAddr) -> Binding {
    if !peer.ip().to_canonical().is_loopback() || peer.port() == 0 {
        return Binding::Unknown;
    }
    tokio::time::timeout(BUDGET, check(claude_pid, peer))
        .await
        .unwrap_or(Binding::Unknown)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_trees_parse() {
        let tree =
            parse_ps_tree("  1   0\n 100   1\n 200 100\n 300 200\n 400 100\nbad line\n 500 999\n");
        let mut d = descendants(&tree, 100, 6);
        d.sort();
        assert_eq!(d, vec![200, 300, 400]);
        assert_eq!(descendants(&tree, 100, 1), vec![200, 400]);
        assert!(descendants(&tree, 300, 6).is_empty());
        // A cycle cannot loop forever.
        let cyclic = parse_ps_tree("1 2\n2 1\n");
        assert_eq!(descendants(&cyclic, 1, 6), vec![2]);
    }

    #[test]
    fn lsof_output_parses() {
        let out = "p4242\nn127.0.0.1:54321->127.0.0.1:6725\nn*:8080\np4243\nn[::1]:6000->[::1]:1\n";
        assert_eq!(parse_lsof_local_ports(out), vec![54321, 8080, 6000]);
        assert!(parse_lsof_local_ports("").is_empty());
    }
}
