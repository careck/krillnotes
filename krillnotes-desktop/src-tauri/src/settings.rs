// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Application settings persistence for Krillnotes.
//!
//! Settings live in the Krillnotes home folder (`~/Krillnotes/settings.json`).
//! A breadcrumb file at the OS config directory stores a custom home path
//! if the user overrides the default.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Persisted application settings.
///
/// Stored at `{home_dir}/settings.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    /// Current theme mode: "light", "dark", or "system".
    #[serde(default = "default_theme_mode")]
    pub active_theme_mode: String,
    /// Name of the theme to use in light mode.
    #[serde(default = "default_light_theme")]
    pub light_theme: String,
    /// Name of the theme to use in dark mode.
    #[serde(default = "default_dark_theme")]
    pub dark_theme: String,
    /// Language code for the UI ("en", "de", "fr", "es", "ja", "ko", "zh").
    #[serde(default = "default_language")]
    pub language: String,
    /// Controls when sharing permission indicators (coloured dots) appear in the tree.
    /// "on" = always, "off" = never, "auto" = only when the workspace has peers.
    #[serde(default = "default_sharing_indicator_mode")]
    pub sharing_indicator_mode: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            active_theme_mode: default_theme_mode(),
            light_theme: default_light_theme(),
            dark_theme: default_dark_theme(),
            language: default_language(),
            sharing_indicator_mode: default_sharing_indicator_mode(),
        }
    }
}

// ── Breadcrumb (custom home folder override) ─────────────────────────

/// Returns the OS-appropriate breadcrumb directory.
/// - macOS / Linux: `~/.config/krillnotes/`
/// - Windows: `%APPDATA%/Krillnotes/`
fn breadcrumb_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Krillnotes")
    }
    #[cfg(not(target_os = "windows"))]
    {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config")
            .join("krillnotes")
    }
}

/// Path to the breadcrumb file that stores a custom home folder location.
fn breadcrumb_path() -> PathBuf {
    breadcrumb_dir().join("home_path")
}

// ── Home directory ───────────────────────────────────────────────────

/// Returns the default home directory: `~/Krillnotes/`.
fn default_home_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Krillnotes")
}

/// Returns the Krillnotes home directory.
///
/// Reads the breadcrumb file if it exists; otherwise returns the
/// platform default (`~/Krillnotes/` on all platforms).
pub fn home_dir() -> PathBuf {
    let bp = breadcrumb_path();
    if let Ok(content) = fs::read_to_string(&bp) {
        let trimmed = content.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    default_home_dir()
}

/// Writes a custom home directory path to the breadcrumb file.
pub fn set_home_dir(path: &str) -> Result<(), String> {
    let bp = breadcrumb_path();
    if let Some(parent) = bp.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create breadcrumb directory: {e}"))?;
    }
    fs::write(&bp, path.trim())
        .map_err(|e| format!("Failed to write breadcrumb file: {e}"))
}

/// Temporary shim — remove after all callers are migrated.
#[deprecated(note = "Use home_dir() instead")]
pub fn config_dir() -> PathBuf {
    home_dir()
}

// ── Settings I/O ─────────────────────────────────────────────────────

fn default_theme_mode() -> String { "system".to_string() }
fn default_light_theme() -> String { "light".to_string() }
fn default_dark_theme() -> String { "dark".to_string() }
fn default_language() -> String { "en".to_string() }
fn default_sharing_indicator_mode() -> String { "auto".to_string() }

/// Returns the path to the settings JSON file: `{home_dir}/settings.json`.
pub fn settings_file_path() -> PathBuf {
    home_dir().join("settings.json")
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
    fn home_dir_returns_default_when_no_breadcrumb() {
        let dir = home_dir();
        assert!(dir.to_string_lossy().ends_with("Krillnotes"));
    }

    #[test]
    fn settings_deserializes_without_workspace_directory() {
        let json = r#"{"activeThemeMode":"dark","language":"fr"}"#;
        let s: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.active_theme_mode, "dark");
        assert_eq!(s.language, "fr");
    }

    #[test]
    fn settings_ignores_legacy_workspace_directory_field() {
        // Old settings files with workspaceDirectory should still deserialize
        let json = r#"{"workspaceDirectory":"/old/path","language":"en"}"#;
        let s: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.language, "en");
    }

    #[test]
    fn settings_defaults_are_applied() {
        let json = r#"{}"#;
        let s: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.active_theme_mode, "system");
        assert_eq!(s.light_theme, "light");
        assert_eq!(s.dark_theme, "dark");
        assert_eq!(s.language, "en");
        assert_eq!(s.sharing_indicator_mode, "auto");
    }
}
