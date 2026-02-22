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
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            workspace_directory: default_workspace_directory()
                .to_string_lossy()
                .to_string(),
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
