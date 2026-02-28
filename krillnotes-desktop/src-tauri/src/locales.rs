//! Embedded locale data for the native application menu.
//!
//! `LOCALES` is generated at compile time by `build.rs` from all JSON files in
//! `krillnotes-desktop/src/i18n/locales/`. To add a new language, create a
//! JSON file there — no Rust changes are needed.

include!(concat!(env!("OUT_DIR"), "/locales_generated.rs"));

use serde_json::Value;

/// Returns the `menu` section of the locale for `lang`, merging over the
/// English base so that partially-translated locales always have all keys.
/// Falls back to English entirely if `lang` is not found.
pub fn menu_strings(lang: &str) -> Value {
    let en_menu = parse_menu("en").unwrap_or_else(|| Value::Object(Default::default()));

    if lang == "en" {
        return en_menu;
    }

    let Some(target_menu) = parse_menu(lang) else {
        return en_menu;
    };

    // Merge: start with English, overlay translated keys.
    let mut result = en_menu;
    if let (Value::Object(ref mut base), Value::Object(target)) = (&mut result, target_menu) {
        for (k, v) in target {
            base.insert(k, v);
        }
    }
    result
}

fn parse_menu(lang: &str) -> Option<Value> {
    let json_str = LOCALES.iter().find(|(l, _)| *l == lang).map(|(_, s)| *s)?;
    let root: Value = serde_json::from_str(json_str).ok()?;
    root.get("menu").cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_has_expected_menu_keys() {
        let s = menu_strings("en");
        assert_eq!(s["file"].as_str(), Some("File"));
        assert_eq!(s["edit"].as_str(), Some("Edit"));
        assert_eq!(s["newWorkspace"].as_str(), Some("New Workspace"));
        assert_eq!(s["refresh"].as_str(), Some("Refresh"));
    }

    #[test]
    fn german_menu_is_translated() {
        let s = menu_strings("de");
        assert_eq!(s["file"].as_str(), Some("Datei"));
        assert_eq!(s["edit"].as_str(), Some("Bearbeiten"));
        assert_eq!(s["newWorkspace"].as_str(), Some("Neuer Arbeitsbereich"));
    }

    #[test]
    fn unknown_language_falls_back_to_english() {
        let s = menu_strings("xx");
        assert_eq!(s["file"].as_str(), Some("File"));
        assert_eq!(s["newWorkspace"].as_str(), Some("New Workspace"));
    }
}
