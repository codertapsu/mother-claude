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

/// First-run check: can the app read `~/.claude/projects`? On packaged macOS
/// builds this requires Full Disk Access (a separate TCC grant from the dev
/// terminal). Exposed over `invoke` because it is a desktop-only OS concern.
#[tauri::command]
fn check_full_disk_access() -> bool {
    claude::ClaudeHome::resolve()
        .map(|h| h.has_full_disk_access())
        .unwrap_or(false)
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

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            check_full_disk_access,
            open_privacy_settings
        ])
        .run(tauri::generate_context!())
        .expect("error while running Mother Claude");
}
