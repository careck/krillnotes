//! User script storage type and front-matter parser.

use serde::{Deserialize, Serialize};

/// A user-defined Rhai script stored in the workspace database.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserScript {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source_code: String,
    pub load_order: i32,
    pub enabled: bool,
    pub created_at: i64,
    pub modified_at: i64,
}

/// Parsed front-matter metadata from a script's leading comments.
#[derive(Debug, Clone, Default)]
pub struct FrontMatter {
    pub name: String,
    pub description: String,
}

/// Parses `// @key: value` front-matter lines from the top of a script.
///
/// Stops at the first non-empty, non-comment line.
/// Returns a [`FrontMatter`] with any extracted `name` and `description`.
pub fn parse_front_matter(source: &str) -> FrontMatter {
    let mut fm = FrontMatter::default();
    for line in source.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("//") {
            if trimmed.is_empty() {
                continue;
            }
            break;
        }
        let comment_body = trimmed.trim_start_matches("//").trim();
        if !comment_body.starts_with('@') {
            continue;
        }
        let after_at = &comment_body[1..];
        if let Some((key, value)) = after_at.split_once(':') {
            let key = key.trim();
            let value = value.trim();
            match key {
                "name" => fm.name = value.to_string(),
                "description" => fm.description = value.to_string(),
                _ => {} // ignore unknown keys
            }
        }
    }
    fm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_front_matter_basic() {
        let source = r#"// @name: My Script
// @description: A test script

schema("Test", #{ fields: [] });
"#;
        let fm = parse_front_matter(source);
        assert_eq!(fm.name, "My Script");
        assert_eq!(fm.description, "A test script");
    }

    #[test]
    fn test_parse_front_matter_missing_description() {
        let source = "// @name: Only Name\nschema(\"X\", #{ fields: [] });";
        let fm = parse_front_matter(source);
        assert_eq!(fm.name, "Only Name");
        assert_eq!(fm.description, "");
    }

    #[test]
    fn test_parse_front_matter_no_front_matter() {
        let source = "schema(\"X\", #{ fields: [] });";
        let fm = parse_front_matter(source);
        assert_eq!(fm.name, "");
        assert_eq!(fm.description, "");
    }

    #[test]
    fn test_parse_front_matter_comment_without_at_is_skipped() {
        let source = "// This is a regular comment\n// @name: After Comment\nschema(\"X\", #{ fields: [] });";
        let fm = parse_front_matter(source);
        assert_eq!(fm.name, "After Comment");
    }

    #[test]
    fn test_parse_front_matter_blank_lines_before_code() {
        let source = "// @name: Spacey\n\n\nschema(\"X\", #{ fields: [] });";
        let fm = parse_front_matter(source);
        assert_eq!(fm.name, "Spacey");
    }
}
