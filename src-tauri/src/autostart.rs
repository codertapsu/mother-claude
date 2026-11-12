//! *Start Mother Claude when you sign in, in the tray* (docs/PLAN-MACHINES.md
//! §5.13, its first step): a desktop setting, off by default, so a computer
//! nobody sits at — a build box, a desktop after a restart — is reachable by
//! your other computers and your phone without anyone opening the app.
//!
//! * **The system's own login item is the setting**, read back each time:
//!   on Windows the per-user `Run` key (`HKCU\…\CurrentVersion\Run`, value
//!   `Mother Claude`: the app, quoted, with [`TRAY_ARG`]), and whether Task
//!   Manager's *Startup apps* switched it off; on macOS a LaunchAgent in
//!   `~/Library/LaunchAgents` (`dev.motherclaude.desktop.plist`) that
//!   starts the app with [`TRAY_ARG`]. Nothing is written but on the
//!   user's switch (`PUT /api/app/autostart`, the desktop only, audited),
//!   or the setup's *Turn on* (its `autostart` check, the desktop's own
//!   fix, or one another computer asked for and this desktop confirmed).
//! * Started with [`TRAY_ARG`], the app runs with no window — a tray icon
//!   (the menu bar on a Mac) — and serves as usual; *Open Mother Claude*
//!   there shows the window, and closing the window hides it again
//!   (`lib.rs`).
//! * **Real effects only in the app**: until `lib.rs` calls
//!   [`Autostart::enable_for_real`], the switch is only recorded here, so no
//!   test ever touches the real login items.
//!
//! No new dependency: the registry through `windows-sys` (already used for
//! the credential store and job objects), the LaunchAgent as a file.
//! (`tauri-plugin-autostart` was weighed: it writes the `Run` value with the
//! app's path unquoted, machine-wide when the app runs elevated, and brings
//! four crates.)

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::server::auth::{Authed, DesktopOnly};
use crate::server::devices::audit::AuditEntry;
use crate::server::error::{ApiError, ApiResult};
use crate::server::routes::Body;
use crate::state::AppState;

/// The argument the login item starts the app with: no window, a tray icon.
pub const TRAY_ARG: &str = "--tray";
/// The `Run` value's name on Windows.
pub const ENTRY_NAME: &str = "Mother Claude";
/// The LaunchAgent's label (and file name) on macOS.
pub const AGENT_LABEL: &str = "dev.motherclaude.desktop";

/// Where the setting stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// This system has a login item Mother Claude can set (macOS, Windows).
    pub supported: bool,
    /// Mother Claude starts when you sign in.
    pub on: bool,
    /// On, but switched off in the system's own list of startup apps
    /// (Windows' Task Manager › Startup apps): it does not start.
    pub overridden: bool,
}

/// The setting, and how this run of the app started.
pub struct Autostart {
    /// The app turned real effects on.
    live: AtomicBool,
    /// Until then (tests): the setting, recorded instead of written.
    recorded: Mutex<bool>,
    /// This run started at sign-in, in the tray.
    in_tray: AtomicBool,
}

impl Default for Autostart {
    fn default() -> Self {
        Self {
            live: AtomicBool::new(false),
            recorded: Mutex::new(false),
            in_tray: AtomicBool::new(false),
        }
    }
}

impl std::fmt::Debug for Autostart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Autostart")
            .field("live", &self.live.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl Autostart {
    /// The app, at start: the switch reads and writes the system's login
    /// item from now on.
    pub fn enable_for_real(&self) {
        self.live.store(true, Ordering::Relaxed);
    }

    /// The app, at start: this run began in the tray ([`TRAY_ARG`]).
    pub fn note_started_in_tray(&self) {
        self.in_tray.store(true, Ordering::Relaxed);
    }

    /// This run began in the tray.
    pub fn started_in_tray(&self) -> bool {
        self.in_tray.load(Ordering::Relaxed)
    }

    /// Where the setting stands (the system's login item, read now).
    pub fn status(&self) -> Status {
        if !self.live.load(Ordering::Relaxed) {
            return Status {
                supported: true,
                on: *self.recorded.lock().unwrap_or_else(|e| e.into_inner()),
                overridden: false,
            };
        }
        platform::status()
    }

    /// Turns it on or off (the user's switch, or the setup's fix).
    pub fn set(&self, on: bool) -> Result<Status, String> {
        if !self.live.load(Ordering::Relaxed) {
            *self.recorded.lock().unwrap_or_else(|e| e.into_inner()) = on;
            return Ok(self.status());
        }
        platform::set(on)?;
        Ok(platform::status())
    }
}

// ─── what is written ─────────────────────────────────────────────────────────

/// Windows' `Run` value: the app, quoted (its folder has spaces), started
/// in the tray.
pub fn windows_run_value(exe: &Path) -> String {
    format!("\"{}\" {TRAY_ARG}", exe.display())
}

/// Whether Windows' `StartupApproved\Run` bytes say the item is switched
/// off: Task Manager writes `03 …` (with the time) when you disable it,
/// `02 …` when you enable it.
pub fn approved_disabled(bytes: &[u8]) -> bool {
    bytes.first().is_some_and(|b| b & 1 == 1)
}

/// The LaunchAgent macOS runs at sign-in: the app's executable with
/// [`TRAY_ARG`], once, in the user's graphical session.
pub fn launch_agent(exe: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{label}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{exe}</string>
		<string>{arg}</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>ProcessType</key>
	<string>Interactive</string>
	<key>LimitLoadToSessionType</key>
	<string>Aqua</string>
</dict>
</plist>
"#,
        label = AGENT_LABEL,
        exe = xml_escape(&exe.to_string_lossy()),
        arg = TRAY_ARG,
    )
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

// ─── the system's login item ─────────────────────────────────────────────────

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_BINARY,
        REG_ROUTINE_FLAGS, REG_SZ, RRF_RT_REG_BINARY, RRF_RT_REG_SZ,
    };

    use super::{Status, ENTRY_NAME};

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const APPROVED_KEY: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    /// A value under `HKCU\<key>`: its bytes, `None` when absent.
    fn read(key: &str, flags: REG_ROUTINE_FLAGS) -> Result<Option<Vec<u8>>, String> {
        let (k, n) = (wide(key), wide(ENTRY_NAME));
        let mut size: u32 = 0;
        // SAFETY: both strings are NUL-terminated and outlive the call; a
        // null buffer asks for the size only.
        let rc = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                k.as_ptr(),
                n.as_ptr(),
                flags,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if rc == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if rc != ERROR_SUCCESS {
            return Err(format!("the registry could not be read (error {rc})"));
        }
        let mut buf = vec![0u8; size as usize];
        // SAFETY: `buf` holds `size` bytes, as the call is told.
        let rc = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                k.as_ptr(),
                n.as_ptr(),
                flags,
                std::ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if rc == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if rc != ERROR_SUCCESS {
            return Err(format!("the registry could not be read (error {rc})"));
        }
        buf.truncate(size as usize);
        Ok(Some(buf))
    }

    fn write(key: &str, kind: u32, data: &[u8]) -> Result<(), String> {
        let (k, n) = (wide(key), wide(ENTRY_NAME));
        // SAFETY: the strings are NUL-terminated, `data` holds `len` bytes.
        let rc = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                k.as_ptr(),
                n.as_ptr(),
                kind,
                data.as_ptr().cast(),
                data.len() as u32,
            )
        };
        if rc == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!("the registry could not be written (error {rc})"))
        }
    }

    fn delete(key: &str) -> Result<(), String> {
        let (k, n) = (wide(key), wide(ENTRY_NAME));
        // SAFETY: both strings are NUL-terminated and outlive the call.
        let rc = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ptr()) };
        if rc == ERROR_SUCCESS || rc == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(format!("the registry could not be written (error {rc})"))
        }
    }

    pub fn status() -> Status {
        let on = matches!(read(RUN_KEY, RRF_RT_REG_SZ), Ok(Some(_)));
        let overridden = on
            && matches!(
                read(APPROVED_KEY, RRF_RT_REG_BINARY),
                Ok(Some(b)) if super::approved_disabled(&b)
            );
        Status {
            supported: true,
            on,
            overridden,
        }
    }

    pub fn set(on: bool) -> Result<(), String> {
        if !on {
            delete(RUN_KEY)?;
            return delete(APPROVED_KEY);
        }
        let exe = std::env::current_exe()
            .map_err(|e| format!("Mother Claude's own program could not be found: {e}"))?;
        let value: Vec<u8> = super::windows_run_value(&exe)
            .encode_utf16()
            .chain(Some(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        write(RUN_KEY, REG_SZ, &value)?;
        // Switched off in Task Manager before: you asked for it here, so it
        // is switched on there too.
        if matches!(read(APPROVED_KEY, RRF_RT_REG_BINARY), Ok(Some(_))) {
            write(
                APPROVED_KEY,
                REG_BINARY,
                &[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            )?;
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::path::PathBuf;

    use super::{Status, AGENT_LABEL};

    fn agent_path() -> Option<PathBuf> {
        crate::claude::user_home_dir().map(|h| {
            h.join("Library")
                .join("LaunchAgents")
                .join(format!("{AGENT_LABEL}.plist"))
        })
    }

    pub fn status() -> Status {
        Status {
            supported: true,
            on: agent_path().is_some_and(|p| p.is_file()),
            overridden: false,
        }
    }

    pub fn set(on: bool) -> Result<(), String> {
        let path = agent_path().ok_or("Your home folder could not be found.")?;
        if !on {
            return match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(format!("The login item could not be removed: {e}")),
            };
        }
        let exe = std::env::current_exe()
            .and_then(|p| p.canonicalize())
            .map_err(|e| format!("Mother Claude's own program could not be found: {e}"))?;
        crate::persist::write_private(&path, super::launch_agent(&exe).as_bytes())
            .map_err(|e| format!("The login item could not be written: {e}"))
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    use super::Status;

    pub fn status() -> Status {
        Status::default()
    }

    pub fn set(_on: bool) -> Result<(), String> {
        Err("Starting at sign-in is not available on this system.".into())
    }
}

// ─── the routes ──────────────────────────────────────────────────────────────

/// `GET` (anyone signed in: a phone sees it) and `PUT` (the desktop only)
/// `/api/app/autostart`.
pub fn api_router() -> Router<AppState> {
    Router::new().route("/app/autostart", get(get_autostart).put(put_autostart))
}

fn view(state: &AppState) -> Value {
    let s = state.autostart.status();
    json!({
        "supported": s.supported,
        "enabled": s.on,
        "overridden": s.overridden,
        "startedInTray": state.autostart.started_in_tray(),
    })
}

async fn get_autostart(Authed(_): Authed, State(state): State<AppState>) -> Json<Value> {
    Json(view(&state))
}

#[derive(Deserialize)]
struct Switch {
    enabled: bool,
}

async fn put_autostart(
    DesktopOnly(ctx): DesktopOnly,
    State(state): State<AppState>,
    Body(req): Body<Switch>,
) -> ApiResult<Json<Value>> {
    let status = tokio::task::spawn_blocking({
        let st = state.clone();
        move || st.autostart.set(req.enabled)
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::unprocessable("autostart_failed", e))?;
    state.devices.audit.record(AuditEntry {
        detail: Some(json!({ "on": status.on })),
        ..AuditEntry::from_ctx(&ctx, "app.autostart")
    });
    // The setup's `autostart` check follows.
    crate::machines::publish(&state, "autostart");
    Ok(Json(view(&state)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn until_the_app_turns_it_on_the_switch_is_only_recorded() {
        let a = Autostart::default();
        assert_eq!(
            a.status(),
            Status {
                supported: true,
                on: false,
                overridden: false
            },
            "off by default"
        );
        assert!(a.set(true).unwrap().on);
        assert!(a.status().on);
        assert!(!a.set(false).unwrap().on);
        assert!(!a.started_in_tray());
        a.note_started_in_tray();
        assert!(a.started_in_tray());
    }

    #[test]
    fn the_run_value_quotes_the_app_and_starts_it_in_the_tray() {
        let exe = Path::new(r"C:\Users\me\AppData\Local\Mother Claude\mother-claude.exe");
        assert_eq!(
            windows_run_value(exe),
            r#""C:\Users\me\AppData\Local\Mother Claude\mother-claude.exe" --tray"#
        );
    }

    #[test]
    fn task_managers_switch_reads_as_recorded() {
        assert!(!approved_disabled(&[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert!(approved_disabled(&[
            3, 0, 0, 0, 0x5c, 0x1d, 0x93, 0x0b, 0, 0, 0, 0
        ]));
        assert!(!approved_disabled(&[6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert!(approved_disabled(&[7, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8]));
        assert!(!approved_disabled(&[]));
    }

    #[test]
    fn the_launch_agent_starts_the_app_in_the_tray_and_escapes_its_path() {
        let plist = launch_agent(Path::new(
            "/Applications/Mother Claude & Co.app/Contents/MacOS/mother-claude",
        ));
        assert!(plist.contains("<string>dev.motherclaude.desktop</string>"));
        assert!(plist.contains(
            "<string>/Applications/Mother Claude &amp; Co.app/Contents/MacOS/mother-claude</string>"
        ));
        assert!(plist.contains("<string>--tray</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>\n\t<true/>"));
        assert!(
            !plist.contains("KeepAlive"),
            "started once, never restarted"
        );
    }
}
