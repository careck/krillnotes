//! App-level theme file storage.
//!
//! Themes are stored as `.krilltheme` JSON files in the same config
//! directory as `settings.json`.

use std::fs;
use std::path::PathBuf;

/// Metadata returned when listing themes (excludes raw JSON content).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeMeta {
    pub name: String,
    pub filename: String,
    pub has_light: bool,
    pub has_dark: bool,
}

/// Returns the themes directory path, creating it if absent.
pub fn themes_dir() -> PathBuf {
    let base = {
        #[cfg(target_os = "windows")]
        { dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("Krillnotes") }
        #[cfg(not(target_os = "windows"))]
        { dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".config").join("krillnotes") }
    };
    let dir = base.join("themes");
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("krillnotes: failed to create themes directory {:?}: {e}", dir);
    }
    dir
}

/// Validates a theme filename and returns the full path inside `themes_dir()`.
/// Returns `Err` if the filename contains path separators, `..`, or does not
/// end with `.krilltheme`.
fn safe_theme_path(filename: &str) -> Result<PathBuf, String> {
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return Err(format!("Invalid theme filename: {filename}"));
    }
    if !filename.ends_with(".krilltheme") {
        return Err(format!("Filename must end with .krilltheme: {filename}"));
    }
    Ok(themes_dir().join(filename))
}

/// Lists all `.krilltheme` files in the themes directory.
pub fn list_themes() -> Result<Vec<ThemeMeta>, String> {
    let dir = themes_dir();
    let mut metas = Vec::new();
    let entries = fs::read_dir(&dir).map_err(|e| e.to_string())?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("krilltheme") {
            continue;
        }
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let json: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let name = json.get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unnamed")
            .to_string();
        let has_light = json.get("light-theme").is_some();
        let has_dark = json.get("dark-theme").is_some();
        let filename = path.file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("")
            .to_string();
        metas.push(ThemeMeta { name, filename, has_light, has_dark });
    }
    metas.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(metas)
}

/// Returns the raw JSON content of a theme file.
pub fn read_theme(filename: &str) -> Result<String, String> {
    let path = safe_theme_path(filename)?;
    fs::read_to_string(&path).map_err(|e| e.to_string())
}

/// Writes (creates or overwrites) a theme file.
pub fn write_theme(filename: &str, content: &str) -> Result<(), String> {
    // Validate JSON before saving.
    let _: serde_json::Value = serde_json::from_str(content)
        .map_err(|e| format!("Invalid JSON: {e}"))?;
    let path = safe_theme_path(filename)?;
    fs::write(&path, content).map_err(|e| e.to_string())
}

/// Deletes a theme file.
pub fn delete_theme(filename: &str) -> Result<(), String> {
    let path = safe_theme_path(filename)?;
    fs::remove_file(&path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialize all theme tests that touch the shared real filesystem directory.
    static FS_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn write_theme_rejects_invalid_json() {
        let _guard = FS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let result = write_theme("__test_invalid__.krilltheme", "not json at all");
        assert!(result.is_err());
        // Clean up if the file was somehow created
        let _ = delete_theme("__test_invalid__.krilltheme");
    }

    #[test]
    fn write_and_read_theme_roundtrip() {
        let _guard = FS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let content = r#"{"name":"Test","dark-theme":{"colors":{}}}"#;
        write_theme("__test_roundtrip__.krilltheme", content).unwrap();
        let read = read_theme("__test_roundtrip__.krilltheme").unwrap();
        assert_eq!(read, content);
        delete_theme("__test_roundtrip__.krilltheme").unwrap();
    }

    #[test]
    fn list_themes_detects_light_and_dark_variants() {
        let _guard = FS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let content = r#"{"name":"BothVariants","light-theme":{},"dark-theme":{}}"#;
        write_theme("__test_both__.krilltheme", content).unwrap();
        let themes = list_themes().unwrap();
        let found = themes.iter().find(|t| t.filename == "__test_both__.krilltheme");
        assert!(found.is_some(), "written theme should appear in list");
        let meta = found.unwrap();
        assert!(meta.has_light, "should detect light-theme key");
        assert!(meta.has_dark, "should detect dark-theme key");
        delete_theme("__test_both__.krilltheme").unwrap();
    }
}
