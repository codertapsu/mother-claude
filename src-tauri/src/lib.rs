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

pub mod claude;
pub mod permissions;
pub mod server;
pub mod state;

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
    let filter = EnvFilter::try_from_env("MOTHER_CLAUDE_LOG")
        .or_else(|_| EnvFilter::try_new("info"))
        .unwrap_or_default();
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

/// Build and run the Tauri application.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();

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
        .invoke_handler(tauri::generate_handler![
            check_full_disk_access,
            permissions_status,
            open_settings_pane,
            reveal_app_in_finder,
            app_location,
            server_info
        ])
        .run(tauri::generate_context!())
        .expect("error while running Mother Claude");
}
