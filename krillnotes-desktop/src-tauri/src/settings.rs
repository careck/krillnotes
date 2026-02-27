//! Application settings persistence for Krillnotes.
//!
//! Stores user preferences (e.g. default workspace directory) in a JSON file
//! at an OS-appropriate location.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Persisted application settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    /// Directory where new workspaces are created and listed from.
    pub workspace_directory: String,
    /// When true, the app caches workspace passwords in memory for the
    /// duration of the session so the user is not re-prompted on reopen.
    #[serde(default)]
    pub cache_workspace_passwords: bool,
    /// Current theme mode: "light", "dark", or "system".
    #[serde(default = "default_theme_mode")]
    pub active_theme_mode: String,
    /// Name of the theme to use in light mode.
    #[serde(default = "default_light_theme")]
    pub light_theme: String,
    /// Name of the theme to use in dark mode.
    #[serde(default = "default_dark_theme")]
    pub dark_theme: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            workspace_directory: default_workspace_directory()
                .to_string_lossy()
                .to_string(),
            cache_workspace_passwords: false,
            active_theme_mode: default_theme_mode(),
            light_theme: default_light_theme(),
            dark_theme: default_dark_theme(),
        }
    }
}

/// Returns the path to the settings JSON file.
///
/// - macOS / Linux: `~/.config/krillnotes/settings.json`
/// - Windows: `%APPDATA%/Krillnotes/settings.json`
pub fn settings_file_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        base.join("Krillnotes").join("settings.json")
    }
    #[cfg(not(target_os = "windows"))]
    {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".config").join("krillnotes").join("settings.json")
    }
}

fn default_theme_mode() -> String { "system".to_string() }
fn default_light_theme() -> String { "light".to_string() }
fn default_dark_theme() -> String { "dark".to_string() }

/// Returns the default workspace directory: `~/Documents/Krillnotes`.
pub fn default_workspace_directory() -> PathBuf {
    dirs::document_dir()
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("Documents")
        })
        .join("Krillnotes")
}

/// Loads settings from disk; returns defaults if the file is missing or corrupt.
pub fn load_settings() -> AppSettings {
    let path = settings_file_path();
    match fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => AppSettings::default(),
    }
}

/// Saves settings to disk, creating parent directories as needed.
pub fn save_settings(settings: &AppSettings) -> Result<(), String> {
    let path = settings_file_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create settings directory: {e}"))?;
    }
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("Failed to serialize settings: {e}"))?;
    fs::write(&path, json)
        .map_err(|e| format!("Failed to write settings: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_legacy_settings_without_theme_fields() {
        let json = r#"{"workspaceDirectory":"/tmp","cacheWorkspacePasswords":false}"#;
        let s: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.active_theme_mode, "system");
        assert_eq!(s.light_theme, "light");
        assert_eq!(s.dark_theme, "dark");
    }
}
