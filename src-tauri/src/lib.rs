//! Mother Claude — Tauri application entry point.
//!
//! The desktop shell hosts an Angular webview and (in later commits) spawns the
//! embedded axum server that serves the same dashboard to LAN/mobile clients.
//!
//! NOTE: dashboard *data* must flow through the embedded HTTP/WS server so the
//! desktop webview and phone browsers share one code path. Tauri `invoke` is
//! reserved for desktop-only OS concerns (e.g. the Full Disk Access check below).

use tauri::Manager;
use tracing_subscriber::EnvFilter;

pub mod autostart;
pub mod bridge;
pub mod claude;
pub mod extension;
pub mod machines;
pub mod mcp;
pub mod orchestration;
pub mod pending;
pub mod permissions;
pub mod persist;
pub mod server;
pub mod state;
mod tray;

use server::auth::Auth;
use state::{Inner, ServerConfig};

/// First-run check: can the app read `~/.claude/projects`? On packaged macOS
/// builds this requires Full Disk Access (a separate TCC grant from the dev
/// terminal). Exposed over `invoke` because it is a desktop-only OS concern.
#[tauri::command]
fn check_full_disk_access() -> bool {
    permissions::full_disk_access_granted()
}

/// The list of OS permissions and their current state (for the onboarding UI).
#[tauri::command]
fn permissions_status() -> Vec<permissions::PermissionInfo> {
    permissions::status()
}

/// Open a System Settings privacy pane by id (e.g. `full-disk-access`).
#[tauri::command]
fn open_settings_pane(pane_id: String) -> Result<(), String> {
    permissions::open_settings_pane(&pane_id)
}

/// Reveal the app bundle in Finder so the user can add it to a permission list.
#[tauri::command]
fn reveal_app_in_finder() -> Result<(), String> {
    permissions::reveal_in_finder()
}

/// The app bundle path, shown in the onboarding guide.
#[tauri::command]
fn app_location() -> String {
    permissions::app_location()
}

/// Desktop-only: how the webview should reach the embedded dashboard server
/// (host, port, scheme, and the API token). This is the one piece of bootstrap
/// data the webview needs before it can talk to the server over HTTP/WS.
#[tauri::command]
fn server_info(state: tauri::State<'_, state::AppState>) -> serde_json::Value {
    // The desktop webview always talks to the loopback HTTP endpoint (served
    // regardless of LAN TLS), so there is no self-signed-cert friction.
    serde_json::json!({
        "host": "127.0.0.1",
        "port": state.config.port,
        "scheme": "http",
        "token": state.auth.token,
    })
}

fn init_tracing() {
    let mut filter = EnvFilter::try_from_env("MOTHER_CLAUDE_LOG")
        .or_else(|_| EnvFilter::try_new("info"))
        .unwrap_or_default();
    // Log hygiene: the WebSocket library traces whole frames (the first one
    // carries a token), so it never logs below info, whatever the filter says.
    for directive in ["tungstenite=info", "tokio_tungstenite=info"] {
        if let Ok(d) = directive.parse() {
            filter = filter.add_directive(d);
        }
    }
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

/// True if `bin` is an executable file in one of the current `PATH` directories.
#[cfg(unix)]
fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p)
                .any(|dir| !dir.as_os_str().is_empty() && dir.join(bin).is_file())
        })
        .unwrap_or(false)
}

/// The `PATH` from the user's interactive login shell, or `None`.
///
/// We need *interactive* + *login* (`-ilc`) because version managers like nvm /
/// fnm / volta inject their bin dir from `.zshrc`/`.bashrc` (interactive), not
/// just the login profile. The value is wrapped in unique markers so shell-init
/// banners or terminal-integration escape sequences printed by the rc files
/// don't corrupt it, and the query runs on a worker thread with a timeout so a
/// misbehaving rc file can't hang startup.
#[cfg(unix)]
fn login_shell_path() -> Option<String> {
    use std::sync::mpsc;
    use std::time::Duration;

    const MARK: &str = "@@MCPATH@@";
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let out = std::process::Command::new(&shell)
            .args(["-ilc", &format!("printf '{MARK}%s{MARK}' \"$PATH\"")])
            .stdin(std::process::Stdio::null())
            .output();
        let _ = tx.send(out);
    });
    let out = rx.recv_timeout(Duration::from_secs(4)).ok()?.ok()?;
    if !out.status.success() {
        return None;
    }
    extract_marked(&String::from_utf8_lossy(&out.stdout), MARK)
}

/// Pull the value printed between two `mark` delimiters, ignoring any
/// shell-init banner or terminal-integration escape codes printed around it.
#[cfg(unix)]
fn extract_marked(s: &str, mark: &str) -> Option<String> {
    s.split(mark)
        .nth(1)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
}

/// Recover a usable `PATH` for a GUI launch.
///
/// macOS/Linux apps launched from Finder/Dock inherit a minimal `PATH`
/// (`/usr/bin:/bin:/usr/sbin:/sbin`), not the user's shell `PATH` — so spawning
/// `claude` or `node` fails with "failed to spawn". When they aren't already
/// resolvable (the packaged case; in `tauri dev` they are, so this is a no-op),
/// merge the login-shell `PATH` plus common install dirs into this process's
/// `PATH`. Call before any subprocess is spawned.
#[cfg(unix)]
fn recover_path_for_gui() {
    use std::collections::HashSet;

    // Fast path: in dev the inherited PATH already resolves both, so skip the
    // (slowish) interactive-shell query entirely.
    if on_path("claude") && on_path("node") {
        return;
    }

    let mut dirs: Vec<String> = Vec::new();
    if let Some(p) = login_shell_path() {
        dirs.extend(p.split(':').filter(|s| !s.is_empty()).map(str::to_string));
    }
    // Fallback common locations (covers claude/node even if the shell query fails).
    if let Some(home) = claude::user_home_dir() {
        for rel in [".local/bin", ".claude/local", ".bun/bin", ".cargo/bin"] {
            dirs.push(home.join(rel).to_string_lossy().into_owned());
        }
    }
    for d in ["/opt/homebrew/bin", "/opt/homebrew/sbin", "/usr/local/bin"] {
        dirs.push(d.to_string());
    }
    if let Ok(existing) = std::env::var("PATH") {
        dirs.extend(
            existing
                .split(':')
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        );
    }

    let mut seen = HashSet::new();
    let merged: Vec<String> = dirs
        .into_iter()
        .filter(|d| seen.insert(d.clone()))
        .collect();
    if !merged.is_empty() {
        // Safe: run() is still single-threaded here (before the server starts).
        std::env::set_var("PATH", merged.join(":"));
        tracing::info!(
            resolved_claude = on_path("claude"),
            "recovered PATH for GUI launch"
        );
    }
}

/// Build and run the Tauri application.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();
    #[cfg(unix)]
    recover_path_for_gui();

    let home = match claude::ClaudeHome::resolve() {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(error = %e, "could not resolve Claude config dir");
            claude::ClaudeHome::with_base(
                claude::user_home_dir().unwrap_or_default().join(".claude"),
            )
        }
    };
    let app_state = Inner::new(home, ServerConfig::from_env(), Auth::load_or_create());
    // Sign-in links open for real only in the app (the opener records
    // otherwise, so no test opens a browser).
    app_state.opener.open_for_real();
    // Machines' fixes may open System Settings and ask for an administrator.
    app_state.machines.enable_fixes();
    // A session moved here starts with `claude` (tests only record it).
    app_state.machines.enable_resumes();
    // So does a task another computer hands over (tests record it too).
    app_state.machines.enable_task_sessions();
    // Code sync watches its projects (tests drive its steps by hand).
    app_state.machines.enable_sync_watch();
    // *Start Mother Claude when you sign in, in the tray*: the switch writes
    // the system's login item (tests only record it), and a run the login
    // item started has no window, only a tray icon.
    app_state.autostart.enable_for_real();
    let in_tray = std::env::args().any(|a| a == autostart::TRAY_ARG);
    if in_tray {
        app_state.autostart.note_started_in_tray();
    }
    let exit_state = app_state.clone();

    tauri::Builder::default()
        .manage(app_state.clone())
        .plugin(tauri_plugin_process::init())
        .setup(move |app| {
            // In-app auto-updater (desktop-only). It checks the GitHub Release
            // `latest.json` against this build's version; the UI drives
            // download/install/relaunch. See docs/AUTOUPDATE.md.
            #[cfg(desktop)]
            app.handle()
                .plugin(tauri_plugin_updater::Builder::new().build())?;

            // The window is made hidden (tauri.conf.json): shown at once,
            // unless the app started at sign-in, in the tray.
            if in_tray {
                #[cfg(target_os = "macos")]
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
                tray::build(app)?;
                tracing::info!("started in the tray");
            } else if let Some(window) = app.get_webview_window("main") {
                window.show()?;
            }

            // In a packaged build the Path A sidecar is bundled under the app's
            // resource dir; point the control layer at it so owned sessions use
            // it automatically. In dev, control.rs falls back to the repo copy.
            if let Ok(resource_dir) = app.path().resource_dir() {
                let bundled = resource_dir.join("sidecar/dist/agent-bridge.js");
                if bundled.is_file() {
                    std::env::set_var("MOTHER_CLAUDE_SIDECAR_PATH", &bundled);
                    tracing::info!(path = %bundled.display(), "using bundled Path A sidecar");
                }
            }

            // The dashboard server shares the Tokio runtime so its broadcast bus
            // can feed both the webview and LAN clients over one path.
            tauri::async_runtime::spawn(async move {
                if let Err(e) = server::serve(app_state).await {
                    tracing::error!(error = %e, "embedded server stopped");
                }
            });
            Ok(())
        })
        .on_window_event(move |window, event| {
            // Started in the tray: closing the window hides it, and the app
            // goes on serving (*Quit* is in the tray's menu).
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if in_tray && window.label() == "main" {
                    api.prevent_close();
                    tray::hide_main(window);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            check_full_disk_access,
            permissions_status,
            open_settings_pane,
            reveal_app_in_finder,
            app_location,
            server_info
        ])
        .build(tauri::generate_context!())
        .expect("error while building Mother Claude")
        .run(move |_app, event| {
            // A Mac: the Dock icon (or opening the app again) shows the
            // window a tray start hid.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen {
                has_visible_windows: false,
                ..
            } = event
            {
                tray::show_main(_app);
            }
            if let tauri::RunEvent::Exit = event {
                // Keep the transcript index's progress for the next launch
                // (it is otherwise checkpointed every 30 s while dirty).
                exit_state.index.persist(true);
                // A plan-usage helper mid-read exits cleanly (its pidfile
                // goes with it) instead of being killed with the runtime.
                claude::usage::shutdown(&exit_state);
            }
        });
}

#[cfg(all(test, unix))]
mod tests {
    use super::extract_marked;

    const MARK: &str = "@@MCPATH@@";

    #[test]
    fn extracts_path_through_shell_init_noise() {
        // Real zsh/iTerm output prints shell-integration escape codes and banners
        // around the value; the markers must still isolate the PATH exactly.
        let noisy = "\x1b]1337;ShellIntegrationVersion=14\x07banner line\n\
                     @@MCPATH@@/Users/me/.local/bin:/Users/me/.nvm/versions/node/v24.15.0/bin:/opt/homebrew/bin@@MCPATH@@";
        assert_eq!(
            extract_marked(noisy, MARK).as_deref(),
            Some(
                "/Users/me/.local/bin:/Users/me/.nvm/versions/node/v24.15.0/bin:/opt/homebrew/bin"
            ),
        );
    }

    #[test]
    fn returns_none_when_absent_or_empty() {
        assert_eq!(extract_marked("no markers at all", MARK), None);
        assert_eq!(extract_marked("@@MCPATH@@@@MCPATH@@", MARK), None);
    }
}
