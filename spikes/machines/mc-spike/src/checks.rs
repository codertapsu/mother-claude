//! S5: the read-only half of the setup checks of §5.14 — what each check can
//! detect, as a standard user, without changing anything. Run from a terminal,
//! macOS attributes the Local Network probe to the terminal app (the
//! "responsible process"), not to this binary; the product runs it inside the app.
//!
//! Nothing here prints an account's email or a token: `claude auth status` is
//! reduced to whether it is signed in and how.

use std::{
    net::{SocketAddr, UdpSocket},
    process::Command,
    time::Instant,
};

use anyhow::Result;
use serde_json::{json, Map, Value};

use crate::{args::Args, net};

pub async fn cmd_checks(_a: &Args) -> Result<()> {
    let mut v = Map::new();
    v.insert("os".into(), json!(std::env::consts::OS));
    v.insert("arch".into(), json!(std::env::consts::ARCH));
    v.insert("machine".into(), json!(crate::http::hostname()));
    v.insert("git".into(), git_checks());
    v.insert("claudeCode".into(), claude_checks());
    v.insert("network".into(), network_probe());
    v.insert("app".into(), app_checks());
    os_checks(&mut v);
    println!("{}", serde_json::to_string_pretty(&Value::Object(v))?);
    Ok(())
}

/// Runs a program with argv (never a shell string); records the outcome.
fn run(prog: &str, args: &[&str]) -> Value {
    let t = Instant::now();
    match Command::new(prog).args(args).output() {
        Ok(o) => json!({
            "ok": o.status.success(),
            "code": o.status.code(),
            "out": cap(String::from_utf8_lossy(&o.stdout).trim()),
            "err": cap(String::from_utf8_lossy(&o.stderr).trim()),
            "ms": net::ms(t.elapsed()),
        }),
        Err(e) => json!({ "ok": false, "spawnError": e.to_string() }),
    }
}

fn out(v: &Value) -> &str {
    v["out"].as_str().unwrap_or("")
}

fn cap(s: &str) -> String {
    if s.len() > 2000 {
        format!(
            "{}…",
            &s[..s
                .char_indices()
                .take_while(|(i, _)| *i < 2000)
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0)]
        )
    } else {
        s.to_string()
    }
}

fn git_checks() -> Value {
    json!({
        "version": run("git", &["--version"]),
        "lfs": run("git", &["lfs", "version"]),
        "autocrlf": run("git", &["config", "--global", "--get", "core.autocrlf"]),
        "symlinks": run("git", &["config", "--get", "core.symlinks"]),
        "longpaths": run("git", &["config", "--global", "--get", "core.longpaths"]),
        "credentialHelper": run("git", &["config", "--get", "credential.helper"]),
    })
}

fn claude_checks() -> Value {
    let version = run("claude", &["--version"]);
    let auth = Command::new("claude")
        .args(["auth", "status", "--json"])
        .output();
    let auth = match auth {
        Ok(o) => match serde_json::from_slice::<Value>(&o.stdout) {
            // Only whether it is signed in and how; never the email or org.
            Ok(j) => json!({
                "exit": o.status.code(),
                "loggedIn": j.get("loggedIn").cloned().unwrap_or(Value::Null),
                "authMethod": j.get("authMethod").cloned().unwrap_or(Value::Null),
                "apiProvider": j.get("apiProvider").cloned().unwrap_or(Value::Null),
                "fields": j.as_object().map(|m| m.keys().cloned().collect::<Vec<_>>()),
            }),
            Err(_) => json!({ "exit": o.status.code(), "parsed": false }),
        },
        Err(e) => json!({ "spawnError": e.to_string() }),
    };
    json!({ "version": version, "auth": auth })
}

/// Which address the machine uses on its LAN, and whether a datagram may be
/// sent to the LAN (the gateway's discard port) and to the mDNS group. On
/// macOS a denied Local Network permission makes these fail with
/// `EHOSTUNREACH` ("No route to host", os error 65).
fn network_probe() -> Value {
    let primary = UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| s.connect("8.8.8.8:53").map(|_| s))
        .and_then(|s| s.local_addr())
        .map(|a| a.ip().to_string())
        .map_err(|e| e.to_string());
    let gateway = default_gateway();
    let to_gateway = gateway
        .as_ref()
        .and_then(|g| g.parse::<std::net::IpAddr>().ok())
        .map(|ip| send_probe(SocketAddr::new(ip, 9), b"mc-spike"));
    json!({
        "primaryAddr": primary.unwrap_or_else(|e| format!("error: {e}")),
        "gateway": gateway,
        "sendToGateway": to_gateway,
        "sendMdnsQuery": send_probe("224.0.0.251:5353".parse().expect("static"), &mdns_query()),
    })
}

fn send_probe(dst: SocketAddr, payload: &[u8]) -> Value {
    let s = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    match s.send_to(payload, dst) {
        Ok(n) => json!({ "ok": true, "sent": n, "to": dst.to_string() }),
        Err(e) => {
            json!({ "ok": false, "to": dst.to_string(), "error": e.to_string(), "os": e.raw_os_error() })
        }
    }
}

/// A well-formed mDNS PTR query for iroh's own service name.
fn mdns_query() -> Vec<u8> {
    let mut q = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in ["_irohv1", "_udp", "local"] {
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.extend_from_slice(&[0, 0, 12, 0, 1]);
    q
}

#[cfg(unix)]
fn default_gateway() -> Option<String> {
    #[cfg(target_os = "macos")]
    let v = run("route", &["-n", "get", "default"]);
    #[cfg(not(target_os = "macos"))]
    let v = run("ip", &["route", "show", "default"]);
    let text = out(&v).to_string();
    text.lines().find_map(|l| {
        let l = l.trim();
        l.strip_prefix("gateway:")
            .map(|g| g.trim().to_string())
            .or_else(|| {
                l.strip_prefix("default via ")
                    .and_then(|r| r.split_whitespace().next())
                    .map(str::to_string)
            })
    })
}

#[cfg(windows)]
fn default_gateway() -> Option<String> {
    let v = ps("(Get-NetRoute -DestinationPrefix '0.0.0.0/0' | Sort-Object RouteMetric | Select-Object -First 1).NextHop");
    let g = out(&v).trim().to_string();
    (!g.is_empty()).then_some(g)
}

/// Mother Claude itself: running, its data dir, its script token present (never read here).
fn app_checks() -> Value {
    let token = crate::http::app_script_token_path();
    json!({
        "dataDir": token.as_ref().and_then(|p| p.parent()).map(|p| p.display().to_string()),
        "scriptTokenPresent": token.as_ref().map(|p| p.exists()),
        "health": run("curl", &["-s", "-m", "3", "http://127.0.0.1:6725/api/health"]),
    })
}

#[cfg(target_os = "macos")]
fn os_checks(v: &mut Map<String, Value>) {
    let fw = "/usr/libexec/ApplicationFirewall/socketfilterfw";
    v.insert(
        "macosFirewall".into(),
        json!({
            "globalState": run(fw, &["--getglobalstate"]),
            "blockAll": run(fw, &["--getblockall"]),
            "allowSigned": run(fw, &["--getallowsigned"]),
            "stealth": run(fw, &["--getstealthmode"]),
            "motherClaudeApp": run(fw, &["--getappblocked", "/Applications/Mother Claude.app"]),
        }),
    );
    let pm = run("pmset", &["-g"]);
    let settings: Map<String, Value> = out(&pm)
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let k = it.next()?;
            let val = it.next()?;
            [
                "sleep",
                "displaysleep",
                "disksleep",
                "standby",
                "powernap",
                "womp",
                "tcpkeepalive",
            ]
            .contains(&k)
            .then(|| (k.to_string(), json!(val)))
        })
        .collect();
    v.insert(
        "macosPower".into(),
        json!({ "pmset": settings, "raw": pm["ok"] }),
    );
    v.insert("macosVersion".into(), run("sw_vers", &["-productVersion"]));
    let root = net::home().join("development/projects");
    v.insert(
        "roots".into(),
        json!({
            "path": root.display().to_string(),
            "listable": std::fs::read_dir(&root).map(|d| d.count()).map_err(|e| e.to_string()).map_or_else(|e| json!({ "error": e }), |n| json!({ "entries": n })),
        }),
    );
}

#[cfg(windows)]
fn ps(script: &str) -> Value {
    run(
        "powershell.exe",
        &[
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ],
    )
}

#[cfg(windows)]
fn ps_quote(s: &str) -> String {
    format!(
        "'{}'",
        s.replace('\'', "''")
            .replace('\u{2019}', "\u{2019}\u{2019}")
            .replace('\u{2018}', "\u{2018}\u{2018}")
    )
}

#[cfg(windows)]
fn os_checks(v: &mut Map<String, Value>) {
    v.insert(
        "windowsVersion".into(),
        ps("[System.Environment]::OSVersion.VersionString"),
    );
    v.insert(
        "elevated".into(),
        ps("([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)"),
    );
    v.insert(
        "networkProfiles".into(),
        ps("Get-NetConnectionProfile | Select-Object Name,InterfaceAlias,NetworkCategory,IPv4Connectivity | ConvertTo-Json -Compress"),
    );
    v.insert(
        "firewallProfiles".into(),
        ps("Get-NetFirewallProfile | Select-Object Name,Enabled,DefaultInboundAction,AllowInboundRules,NotifyOnListen | ConvertTo-Json -Compress"),
    );
    let mut programs = Vec::new();
    if let Ok(me) = std::env::current_exe() {
        programs.push(me.display().to_string());
    }
    for base in [
        std::env::var("LOCALAPPDATA").ok(),
        std::env::var("ProgramFiles").ok(),
    ]
    .into_iter()
    .flatten()
    {
        for rel in [
            "Mother Claude\\mother-claude.exe",
            "Mother Claude\\Mother Claude.exe",
            "Programs\\Mother Claude\\mother-claude.exe",
        ] {
            let p = std::path::Path::new(&base).join(rel);
            if p.exists() {
                programs.push(p.display().to_string());
            }
        }
    }
    let rules: Vec<Value> = programs
        .iter()
        .map(|p| {
            json!({
                "program": p,
                "rules": ps(&format!(
                    "Get-NetFirewallApplicationFilter -Program {} -ErrorAction SilentlyContinue | Get-NetFirewallRule | Select-Object DisplayName,Enabled,Direction,Action,Profile | ConvertTo-Json -Compress",
                    ps_quote(p)
                )),
            })
        })
        .collect();
    v.insert("firewallRulesForPrograms".into(), json!(rules));
    v.insert(
        "controlledFolderAccess".into(),
        ps("(Get-MpPreference).EnableControlledFolderAccess"),
    );
    v.insert(
        "standbyIdle".into(),
        run(
            "powercfg",
            &["/query", "SCHEME_CURRENT", "SUB_SLEEP", "STANDBYIDLE"],
        ),
    );
    v.insert(
        "longPathsEnabled".into(),
        ps("Get-ItemPropertyValue -Path 'HKLM:\\SYSTEM\\CurrentControlSet\\Control\\FileSystem' -Name LongPathsEnabled"),
    );
    v.insert(
        "developerMode".into(),
        ps("Get-ItemPropertyValue -Path 'HKLM:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\AppModelUnlock' -Name AllowDevelopmentWithoutDevLicense -ErrorAction SilentlyContinue"),
    );
    let bash_env = std::env::var("CLAUDE_CODE_GIT_BASH_PATH").ok();
    v.insert(
        "gitBash".into(),
        json!({
            "env": bash_env,
            "programFiles": std::path::Path::new("C:\\Program Files\\Git\\bin\\bash.exe").exists(),
        }),
    );
    let root = std::path::Path::new("D:\\development\\projects");
    v.insert(
        "roots".into(),
        json!({
            "path": root.display().to_string(),
            "listable": std::fs::read_dir(root).map(|d| d.count()).map_err(|e| e.to_string()).map_or_else(|e| json!({ "error": e }), |n| json!({ "entries": n })),
        }),
    );
}

#[cfg(not(any(target_os = "macos", windows)))]
fn os_checks(_v: &mut Map<String, Value>) {}
