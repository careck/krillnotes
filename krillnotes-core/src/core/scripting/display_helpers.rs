//! HTML display helper functions for Rhai `on_view` hooks.
//!
//! Each function is registered as a top-level Rhai host function and returns
//! an HTML string styled with `kn-view-*` CSS classes. String parameters taken
//! directly from user scripts are HTML-escaped; array/map cell values are passed
//! through as-is so that HTML helpers like `link_to()` compose correctly.
//! DOMPurify in the frontend is the final XSS sanitization layer.

use pulldown_cmark::{html as md_html, Options, Parser};
use rhai::{Array, Map};
use crate::{FieldValue, Note};
use super::schema::Schema;

// ── Escaping ─────────────────────────────────────────────────────────────────

/// Escapes HTML special characters in a user-supplied string.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ── Markdown rendering ────────────────────────────────────────────────────────

/// Converts a CommonMark markdown string to an HTML string.
///
/// Enables strikethrough and tables (GFM extensions). The result is raw HTML —
/// the caller is responsible for XSS sanitisation (DOMPurify handles this on
/// the frontend for all view HTML).
pub fn render_markdown_to_html(text: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    let parser = Parser::new_ext(text, options);
    let mut html_output = String::new();
    md_html::push_html(&mut html_output, parser);
    html_output
}

/// Rhai host function wrapper for `render_markdown_to_html`.
///
/// Registered as `markdown(text)` in the Rhai engine so `on_view` hooks can
/// explicitly render markdown:
///
/// ```rhai
/// on_view("Note", |note| {
///     markdown(note.fields["body"])
/// });
/// ```
pub fn rhai_markdown(text: String) -> String {
    format!("<div class=\"kn-view-markdown\">{}</div>", render_markdown_to_html(&text))
}

// ── Structural helpers ────────────────────────────────────────────────────────

/// Wraps `content` in a titled section container.
///
/// ```rhai
/// section("My Section", table(...))
/// ```
pub fn section(title: String, content: String) -> String {
    format!(
        "<div class=\"kn-view-section\">\
           <div class=\"kn-view-section-title\">{}</div>\
           {}\
         </div>",
        html_escape(&title),
        content
    )
}

/// Stacks `items` vertically with consistent spacing.
///
/// ```rhai
/// stack([section(...), divider(), text("footer")])
/// ```
pub fn stack(items: Array) -> String {
    let inner: String = items.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("");
    format!("<div class=\"kn-view-stack\">{inner}</div>")
}

/// Lays `items` out as equal-width columns.
///
/// ```rhai
/// columns([section("Left", ...), section("Right", ...)])
/// ```
pub fn columns(items: Array) -> String {
    let count = items.len().max(1);
    let inner: String = items.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("");
    format!(
        "<div class=\"kn-view-columns\" style=\"grid-template-columns: repeat({count}, 1fr);\">{inner}</div>"
    )
}

// ── Content helpers ───────────────────────────────────────────────────────────

/// Renders a table with a header row and data rows.
///
/// `headers` is an array of strings; `rows` is an array of arrays of strings.
///
/// ```rhai
/// table(["Name", "Email"], contacts.map(|c| [c.title, c.fields.email ?? "-"]))
/// ```
pub fn table(headers: Array, rows: Array) -> String {
    let mut out = String::from("<table class=\"kn-view-table\"><thead><tr>");
    for h in &headers {
        out.push_str(&format!("<th class=\"kn-view-th\">{}</th>", html_escape(&h.to_string())));
    }
    out.push_str("</tr></thead><tbody>");
    for row in &rows {
        out.push_str("<tr class=\"kn-view-tr\">");
        if let Ok(cells) = row.clone().try_cast::<Array>().ok_or(()) {
            for cell in &cells {
                out.push_str(&format!("<td class=\"kn-view-td\">{}</td>", cell.to_string()));
            }
        }
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table>");
    out
}

/// Renders a single key-value field row.
///
/// ```rhai
/// field("Email", contact.fields.email ?? "-")
/// ```
pub fn field_row(label: String, value: String) -> String {
    format!(
        "<div class=\"kn-view-field-row\">\
           <span class=\"kn-view-field-label\">{}</span>\
           <span class=\"kn-view-field-value\">{}</span>\
         </div>",
        html_escape(&label),
        html_escape(&value)
    )
}

/// Renders all fields in `note` as key-value rows, skipping empty values.
///
/// Field key names are humanised: `"first_name"` → `"First Name"`.
///
/// ```rhai
/// fields(note)
/// ```
pub fn fields(note: Map) -> String {
    let fields_dyn = match note.get("fields").and_then(|v| v.clone().try_cast::<Map>()) {
        Some(m) => m,
        None => return String::new(),
    };

    let mut out = String::new();
    let mut pairs: Vec<(String, String)> = fields_dyn
        .iter()
        .filter_map(|(k, v)| {
            if v.is_unit() {
                return None;
            }
            let s = v.to_string();
            if s.is_empty() || s == "false" {
                return None;
            }
            let label = humanise_key(k);
            Some((label, s))
        })
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    for (label, value) in pairs {
        out.push_str(&field_row(label, value));
    }
    out
}

/// Renders a bold section heading.
///
/// ```rhai
/// heading("Project Details")
/// ```
pub fn heading(text: String) -> String {
    format!("<div class=\"kn-view-heading\">{}</div>", html_escape(&text))
}

/// Renders a whitespace-preserving paragraph.
///
/// Exposed as `"text"` in Rhai (the name `text` is not a reserved keyword in Rhai,
/// but using a distinct Rust name avoids any potential shadowing issues).
///
/// ```rhai
/// text("Some long description\nwith newlines.")
/// ```
pub fn view_text(content: String) -> String {
    format!("<p class=\"kn-view-text\">{}</p>", html_escape(&content))
}

/// Renders items as a bullet list.
///
/// ```rhai
/// list(tasks.map(|t| t.title))
/// ```
pub fn list(items: Array) -> String {
    let mut out = String::from("<ul class=\"kn-view-list\">");
    for item in &items {
        out.push_str(&format!("<li>{}</li>", item.to_string()));
    }
    out.push_str("</ul>");
    out
}

// ── Inline / decorative helpers ───────────────────────────────────────────────

/// Renders a neutral pill badge.
///
/// ```rhai
/// badge("Active")
/// ```
pub fn badge(text: String) -> String {
    format!("<span class=\"kn-view-badge\">{}</span>", html_escape(&text))
}

/// Renders a colored pill badge.
///
/// Supported colors: `"red"`, `"green"`, `"blue"`, `"yellow"`, `"gray"`,
/// `"orange"`, `"purple"`. Any other value falls back to the neutral badge.
///
/// ```rhai
/// badge("High", "red")
/// ```
pub fn badge_colored(text: String, color: String) -> String {
    let class = match color.as_str() {
        "red"    => "kn-view-badge kn-view-badge-red",
        "green"  => "kn-view-badge kn-view-badge-green",
        "blue"   => "kn-view-badge kn-view-badge-blue",
        "yellow" => "kn-view-badge kn-view-badge-yellow",
        "gray"   => "kn-view-badge kn-view-badge-gray",
        "orange" => "kn-view-badge kn-view-badge-orange",
        "purple" => "kn-view-badge kn-view-badge-purple",
        _        => "kn-view-badge",
    };
    format!("<span class=\"{class}\">{}</span>", html_escape(&text))
}

/// Renders a horizontal rule.
///
/// ```rhai
/// divider()
/// ```
pub fn divider() -> String {
    "<hr class=\"kn-view-divider\">".to_string()
}

/// Renders a clickable link that navigates to another note when clicked.
///
/// When clicked in the view panel, the app navigates to the linked note
/// and pushes the current note onto the navigation history stack so the
/// user can press the back button to return.
///
/// ```rhai
/// link_to(get_note(some_id))
/// ```
pub fn link_to(note: Map) -> String {
    let id = note
        .get("id")
        .and_then(|v| v.clone().into_string().ok())
        .unwrap_or_default();
    let title = note
        .get("title")
        .and_then(|v| v.clone().into_string().ok())
        .unwrap_or_default();
    format!(
        r#"<a class="kn-view-link" data-note-id="{}">{}</a>"#,
        html_escape(&id),
        html_escape(&title),
    )
}

// ── Utilities ─────────────────────────────────────────────────────────────────

/// Converts a snake_case field key to a Title Case display label.
///
/// `"first_name"` → `"First Name"`, `"email"` → `"Email"`.
fn humanise_key(key: &str) -> String {
    key.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ── Default view renderer ────────────────────────────────────────────────────

/// Like `field_row` but accepts pre-rendered HTML for the value (no escaping).
/// Used internally by `render_default_view` where the value may be markdown HTML.
fn field_row_html(label: &str, value_html: &str) -> String {
    format!(
        "<div class=\"kn-view-field-row\">\
           <span class=\"kn-view-field-label\">{}</span>\
           <div class=\"kn-view-field-value\">{}</div>\
         </div>",
        html_escape(label),
        value_html
    )
}

/// Formats a single field value as HTML, choosing between markdown rendering
/// (for `textarea`) and HTML-escaped plain text (for all other types).
fn format_field_value_html(value: &FieldValue, field_type: &str, max: i64) -> String {
    match (value, field_type) {
        (FieldValue::Text(s), "textarea") => {
            format!("<div class=\"kn-view-markdown\">{}</div>", render_markdown_to_html(s))
        }
        (FieldValue::Text(s), _) => {
            format!("<span>{}</span>", html_escape(s))
        }
        (FieldValue::Email(s), _) => {
            format!("<span>{}</span>", html_escape(s))
        }
        (FieldValue::Number(n), "rating") => {
            let rating = *n as i64;
            let max_stars = if max > 0 { max } else { 5 };
            let stars: String = (0..max_stars)
                .map(|i| if i < rating { '★' } else { '☆' })
                .collect();
            format!("<span>{stars}</span>")
        }
        (FieldValue::Number(n), _) => format!("<span>{n}</span>"),
        (FieldValue::Boolean(b), _) => {
            format!("<span>{}</span>", if *b { "Yes" } else { "No" })
        }
        (FieldValue::Date(Some(d)), _) => {
            format!("<span>{}</span>", d.format("%Y-%m-%d"))
        }
        (FieldValue::Date(None), _) => String::new(),
    }
}

/// Returns `true` if the field value is considered empty (and should be hidden).
fn is_field_empty(value: &FieldValue) -> bool {
    match value {
        FieldValue::Text(s) | FieldValue::Email(s) => s.is_empty(),
        FieldValue::Date(d) => d.is_none(),
        FieldValue::Number(_) | FieldValue::Boolean(_) => false,
    }
}

/// Generates a default HTML view for `note` when no `on_view` Rhai hook is
/// registered.
///
/// Schema fields are rendered in schema order, with `textarea` values converted
/// to CommonMark HTML and all other values HTML-escaped.
///
/// Fields present in `note.fields` but absent from the schema are appended in
/// a "Legacy Fields" section.
///
/// Accepts `None` for `schema` — in that case all fields are rendered as plain
/// text in sorted order.
pub fn render_default_view(note: &Note, schema: Option<&Schema>) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(schema) = schema {
        // Render schema-defined fields in declaration order.
        for field_def in &schema.fields {
            if !field_def.can_view {
                continue;
            }
            let Some(value) = note.fields.get(&field_def.name) else { continue };
            if is_field_empty(value) {
                continue;
            }
            let label = humanise_key(&field_def.name);
            let value_html =
                format_field_value_html(value, &field_def.field_type, field_def.max);
            if value_html.is_empty() {
                continue;
            }
            parts.push(field_row_html(&label, &value_html));
        }

        // Render any fields not in the schema as "legacy".
        let schema_names: std::collections::HashSet<&str> =
            schema.fields.iter().map(|f| f.name.as_str()).collect();
        let mut legacy: Vec<(&String, &FieldValue)> = note
            .fields
            .iter()
            .filter(|(k, _)| !schema_names.contains(k.as_str()))
            .collect();
        legacy.sort_by_key(|(k, _)| k.as_str());

        let mut legacy_parts: Vec<String> = Vec::new();
        for (key, value) in &legacy {
            if is_field_empty(value) {
                continue;
            }
            let label = humanise_key(key);
            let value_html = format_field_value_html(value, "text", 0);
            if !value_html.is_empty() {
                legacy_parts.push(field_row_html(&label, &value_html));
            }
        }
        if !legacy_parts.is_empty() {
            parts.push(format!(
                "<div class=\"kn-view-section kn-view-section--legacy\">\
                   <div class=\"kn-view-section-title\">Legacy Fields</div>\
                   {}\
                 </div>",
                legacy_parts.join("")
            ));
        }
    } else {
        // No schema — render all fields as plain text in sorted order.
        let mut all: Vec<(&String, &FieldValue)> = note.fields.iter().collect();
        all.sort_by_key(|(k, _)| k.as_str());
        for (key, value) in &all {
            if is_field_empty(value) {
                continue;
            }
            let label = humanise_key(key);
            let value_html = format_field_value_html(value, "text", 0);
            if !value_html.is_empty() {
                parts.push(field_row_html(&label, &value_html));
            }
        }
    }

    parts.join("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhai::Map;

    fn make_note_map(id: &str, title: &str) -> Map {
        let mut m = Map::new();
        m.insert("id".into(),    rhai::Dynamic::from(id.to_string()));
        m.insert("title".into(), rhai::Dynamic::from(title.to_string()));
        m
    }

    #[test]
    fn test_link_to_renders_anchor_with_id_and_title() {
        let m = make_note_map("abc-123", "My Note");
        let html = link_to(m);
        assert!(html.contains(r#"class="kn-view-link""#));
        assert!(html.contains(r#"data-note-id="abc-123""#));
        assert!(html.contains("My Note"));
    }

    #[test]
    fn test_link_to_escapes_title() {
        let m = make_note_map("id-1", "<script>alert('xss')</script>");
        let html = link_to(m);
        assert!(!html.contains("<script>"), "raw <script> tag must not appear in output");
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn test_link_to_escapes_id() {
        let m = make_note_map(r#"id"with"quotes"#, "Title");
        let html = link_to(m);
        assert!(!html.contains(r#"id"with"quotes"#), "raw quotes in id must be escaped");
    }

    #[test]
    fn test_link_to_empty_map_returns_anchor_with_empty_values() {
        let m = Map::new();
        let html = link_to(m);
        // Should not panic; should return a valid (empty-attribute) anchor
        assert!(html.contains("kn-view-link"));
    }

    #[test]
    fn test_rhai_markdown_wraps_in_kn_view_markdown() {
        let html = rhai_markdown("**bold text**".to_string());
        assert!(
            html.contains("kn-view-markdown"),
            "rhai_markdown must wrap output in kn-view-markdown div, got: {html}"
        );
        assert!(html.contains("<strong>bold text</strong>"), "got: {html}");
    }

    #[test]
    fn test_render_markdown_bold() {
        let html = render_markdown_to_html("**bold text**");
        assert!(html.contains("<strong>bold text</strong>"));
    }

    #[test]
    fn test_render_markdown_heading() {
        let html = render_markdown_to_html("# My Heading");
        assert!(html.contains("<h1>") && html.contains("My Heading"));
    }

    #[test]
    fn test_render_markdown_list() {
        let html = render_markdown_to_html("- item one\n- item two");
        assert!(html.contains("item one") && html.contains("item two"));
    }

    #[test]
    fn test_render_markdown_plain_text() {
        let html = render_markdown_to_html("just plain text");
        assert!(html.contains("just plain text"));
    }

    #[test]
    fn test_render_markdown_empty() {
        let html = render_markdown_to_html("");
        assert!(html.is_empty() || html == "\n");
    }

    #[test]
    fn test_render_default_view_textarea_renders_markdown() {
        use crate::{FieldValue, FieldDefinition, Note, Schema};
        use std::collections::HashMap;

        let mut fields = HashMap::new();
        fields.insert("notes".into(), FieldValue::Text("**bold**".into()));

        let note = Note {
            id: "id1".into(), title: "Test".into(), node_type: "T".into(),
            parent_id: None, position: 0, created_at: 0, modified_at: 0,
            created_by: 0, modified_by: 0, fields, is_expanded: false,
        };
        let schema = Schema {
            name: "T".into(),
            fields: vec![FieldDefinition {
                name: "notes".into(), field_type: "textarea".into(),
                required: false, can_view: true, can_edit: true,
                options: vec![], max: 0,
            }],
            title_can_view: true, title_can_edit: true,
            children_sort: "none".into(),
            allowed_parent_types: vec![], allowed_children_types: vec![],
        };

        let html = render_default_view(&note, Some(&schema));
        assert!(html.contains("<strong>bold</strong>"), "expected rendered markdown, got: {html}");
        assert!(html.contains("kn-view-markdown"), "expected markdown wrapper class");
    }

    #[test]
    fn test_render_default_view_text_field_html_escaped() {
        use crate::{FieldValue, FieldDefinition, Note, Schema};
        use std::collections::HashMap;

        let mut fields = HashMap::new();
        fields.insert("name".into(), FieldValue::Text("<script>alert(1)</script>".into()));

        let note = Note {
            id: "id2".into(), title: "T".into(), node_type: "T".into(),
            parent_id: None, position: 0, created_at: 0, modified_at: 0,
            created_by: 0, modified_by: 0, fields, is_expanded: false,
        };
        let schema = Schema {
            name: "T".into(),
            fields: vec![FieldDefinition {
                name: "name".into(), field_type: "text".into(),
                required: false, can_view: true, can_edit: true,
                options: vec![], max: 0,
            }],
            title_can_view: true, title_can_edit: true,
            children_sort: "none".into(),
            allowed_parent_types: vec![], allowed_children_types: vec![],
        };

        let html = render_default_view(&note, Some(&schema));
        assert!(!html.contains("<script>"), "raw script tag must not appear");
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn test_render_default_view_skips_can_view_false() {
        use crate::{FieldValue, FieldDefinition, Note, Schema};
        use std::collections::HashMap;

        let mut fields = HashMap::new();
        fields.insert("secret".into(), FieldValue::Text("hidden".into()));

        let note = Note {
            id: "id3".into(), title: "T".into(), node_type: "T".into(),
            parent_id: None, position: 0, created_at: 0, modified_at: 0,
            created_by: 0, modified_by: 0, fields, is_expanded: false,
        };
        let schema = Schema {
            name: "T".into(),
            fields: vec![FieldDefinition {
                name: "secret".into(), field_type: "text".into(),
                required: false, can_view: false, can_edit: true,
                options: vec![], max: 0,
            }],
            title_can_view: true, title_can_edit: true,
            children_sort: "none".into(),
            allowed_parent_types: vec![], allowed_children_types: vec![],
        };

        let html = render_default_view(&note, Some(&schema));
        assert!(!html.contains("hidden"), "can_view:false fields must not appear");
    }

    #[test]
    fn test_render_default_view_textarea_passes_html_to_domhpurify() {
        // pulldown-cmark passes inline HTML through; DOMPurify on the frontend
        // is the only XSS defence for the textarea path. This test documents
        // that contract so it is not accidentally "fixed" by escaping at this layer.
        use crate::{FieldValue, Note};
        use crate::{FieldDefinition, Schema};
        use std::collections::HashMap;

        let mut fields = HashMap::new();
        fields.insert("body".into(), FieldValue::Text("<em>italic</em> and **bold**".into()));

        let note = Note {
            id: "sec1".into(), title: "T".into(), node_type: "T".into(),
            parent_id: None, position: 0, created_at: 0, modified_at: 0,
            created_by: 0, modified_by: 0, fields, is_expanded: false,
        };
        let schema = Schema {
            name: "T".into(),
            fields: vec![FieldDefinition {
                name: "body".into(), field_type: "textarea".into(),
                required: false, can_view: true, can_edit: true,
                options: vec![], max: 0,
            }],
            title_can_view: true, title_can_edit: true,
            children_sort: "none".into(),
            allowed_parent_types: vec![], allowed_children_types: vec![],
        };

        let html = render_default_view(&note, Some(&schema));
        // Must be wrapped in the markdown class (backend renders it).
        assert!(html.contains("kn-view-markdown"), "got: {html}");
        // pulldown-cmark renders **bold** as <strong>bold</strong>
        assert!(html.contains("<strong>bold</strong>"), "got: {html}");
        // The inline HTML <em>italic</em> is passed through by pulldown-cmark
        // (not double-escaped) — DOMPurify handles final sanitisation.
        assert!(html.contains("<em>italic</em>"), "inline HTML should pass through, got: {html}");
    }

    #[test]
    fn test_render_default_view_legacy_fields_shown() {
        use crate::{FieldValue, FieldDefinition, Note, Schema};
        use std::collections::HashMap;

        let mut fields = HashMap::new();
        fields.insert("known".into(), FieldValue::Text("hello".into()));
        fields.insert("old_field".into(), FieldValue::Text("legacy value".into()));

        let note = Note {
            id: "id4".into(), title: "T".into(), node_type: "T".into(),
            parent_id: None, position: 0, created_at: 0, modified_at: 0,
            created_by: 0, modified_by: 0, fields, is_expanded: false,
        };
        let schema = Schema {
            name: "T".into(),
            fields: vec![FieldDefinition {
                name: "known".into(), field_type: "text".into(),
                required: false, can_view: true, can_edit: true,
                options: vec![], max: 0,
            }],
            title_can_view: true, title_can_edit: true,
            children_sort: "none".into(),
            allowed_parent_types: vec![], allowed_children_types: vec![],
        };

        let html = render_default_view(&note, Some(&schema));
        assert!(html.contains("legacy value"), "legacy fields must be shown");
        assert!(html.contains("Legacy Fields"), "legacy section header must appear");
    }
}
