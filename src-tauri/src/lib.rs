//! Mother Claude — Tauri application entry point.
//!
//! The desktop shell hosts an Angular webview and (in later commits) spawns the
//! embedded axum server that serves the same dashboard to LAN/mobile clients.
//!
//! NOTE: dashboard *data* must flow through the embedded HTTP/WS server so the
//! desktop webview and phone browsers share one code path. Tauri `invoke` is
//! reserved for desktop-only OS concerns (e.g. the Full Disk Access check below).

use tracing_subscriber::EnvFilter;

pub mod claude;
pub mod server;
pub mod state;

use state::{Inner, ServerConfig};

/// First-run check: can the app read `~/.claude/projects`? On packaged macOS
/// builds this requires Full Disk Access (a separate TCC grant from the dev
/// terminal). Exposed over `invoke` because it is a desktop-only OS concern.
#[tauri::command]
fn check_full_disk_access() -> bool {
    claude::ClaudeHome::resolve()
        .map(|h| h.has_full_disk_access())
        .unwrap_or(false)
}

/// Desktop-only: where the embedded dashboard server is listening, so the
/// webview can point its API client at it.
#[tauri::command]
fn server_info(state: tauri::State<'_, state::AppState>) -> serde_json::Value {
    serde_json::json!({
        "host": state.config.host,
        "port": state.config.port,
    })
}

/// Open the macOS Privacy & Security → Full Disk Access settings pane.
#[tauri::command]
fn open_privacy_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("Full Disk Access settings are macOS-only".to_string())
    }
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
                std::env::var_os("HOME")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_default()
                    .join(".claude"),
            )
        }
    };
    let app_state = Inner::new(home, ServerConfig::from_env());

    tauri::Builder::default()
        .manage(app_state.clone())
        .setup(move |_app| {
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
            open_privacy_settings,
            server_info
        ])
        .run(tauri::generate_context!())
        .expect("error while running Mother Claude");
}
