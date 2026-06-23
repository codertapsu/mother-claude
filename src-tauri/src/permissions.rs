//! macOS permission detection and deep-links, used by the first-run onboarding
//! flow. Everything here is a desktop-only OS concern, surfaced over Tauri
//! `invoke` (the one sanctioned use of `invoke` — it is not dashboard data).

use std::path::PathBuf;

use serde::Serialize;

/// One permission the user may need to grant, with guidance for doing so.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionInfo {
    pub id: String,
    pub label: String,
    pub description: String,
    /// `Some(true/false)` when detectable; `None` when it can't be checked.
    pub granted: Option<bool>,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings_url: Option<String>,
    pub steps: Vec<String>,
}

#[cfg(target_os = "macos")]
const FDA_URL: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles";
#[cfg(target_os = "macos")]
const LOCAL_NETWORK_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_LocalNetwork";

/// Whether the app can read `~/.claude/projects` (the practical Full Disk Access
/// probe on packaged macOS builds).
pub fn full_disk_access_granted() -> bool {
    crate::claude::ClaudeHome::resolve()
        .map(|h| h.has_full_disk_access())
        .unwrap_or(false)
}

/// The `.app` bundle that contains the running executable (so we can point the
/// user at the right thing to add to Full Disk Access), or the executable path.
pub fn app_location() -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    for ancestor in exe.ancestors() {
        if ancestor.extension().and_then(|e| e.to_str()) == Some("app") {
            return ancestor.to_string_lossy().to_string();
        }
    }
    exe.to_string_lossy().to_string()
}

fn app_bundle_or_exe() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    for ancestor in exe.ancestors() {
        if ancestor.extension().and_then(|e| e.to_str()) == Some("app") {
            return ancestor.to_path_buf();
        }
    }
    exe
}

/// The list of permissions for this platform with current state.
pub fn status() -> Vec<PermissionInfo> {
    #[cfg(target_os = "macos")]
    {
        vec![
            PermissionInfo {
                id: "full-disk-access".into(),
                label: "Full Disk Access".into(),
                description: "Lets Mother Claude read your Claude Code transcripts and session \
                              state under ~/.claude, which macOS protects."
                    .into(),
                granted: Some(full_disk_access_granted()),
                required: true,
                settings_url: Some(FDA_URL.into()),
                steps: vec![
                    "Click “Open System Settings”.".into(),
                    "Under Privacy & Security → Full Disk Access, find “Mother Claude”. If it \
                     isn’t listed, click +, then add it — use “Reveal app in Finder” to locate it."
                        .into(),
                    "Turn the switch on (macOS may ask you to quit and reopen the app).".into(),
                    "Come back here and click “Re-check”.".into(),
                ],
            },
            PermissionInfo {
                id: "local-network".into(),
                label: "Local Network".into(),
                description: "Lets your phone reach the dashboard over Wi-Fi. macOS asks the \
                              first time a device connects."
                    .into(),
                granted: None,
                required: false,
                settings_url: Some(LOCAL_NETWORK_URL.into()),
                steps: vec![
                    "When macOS asks “Mother Claude would like to find devices on your local \
                     network”, click Allow."
                        .into(),
                    "If you declined earlier, enable Mother Claude in this pane.".into(),
                ],
            },
        ]
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

/// Open a known System Settings privacy pane.
pub fn open_settings_pane(pane_id: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let url = match pane_id {
            "full-disk-access" => FDA_URL,
            "local-network" => LOCAL_NETWORK_URL,
            other => return Err(format!("unknown settings pane: {other}")),
        };
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pane_id;
        Err("System Settings panes are macOS-only".into())
    }
}

/// Reveal the app bundle in Finder so the user can drag it into a permission list.
pub fn reveal_in_finder() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(app_bundle_or_exe())
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("Reveal in Finder is macOS-only".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_pane_is_rejected() {
        // On macOS this returns the unknown-pane error; off macOS it's the
        // platform error. Either way it must be Err.
        assert!(open_settings_pane("does-not-exist").is_err());
    }

    #[test]
    fn app_location_is_nonempty() {
        assert!(!app_location().is_empty());
    }
}
