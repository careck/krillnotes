//! HTML display helper functions for Rhai `on_view` hooks.
//!
//! Each function is registered as a top-level Rhai host function and returns
//! an HTML string styled with `kn-view-*` CSS classes. All user-supplied
//! content is HTML-escaped before insertion to prevent XSS.

use rhai::{Array, Dynamic, Map};

// ── Escaping ─────────────────────────────────────────────────────────────────

/// Escapes HTML special characters in a user-supplied string.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Converts a Rhai `Dynamic` to a display string, HTML-escaped.
fn dyn_to_escaped(d: &Dynamic) -> String {
    if d.is_unit() {
        String::new()
    } else {
        html_escape(&d.to_string())
    }
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
                out.push_str(&format!("<td class=\"kn-view-td\">{}</td>", dyn_to_escaped(cell)));
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
        out.push_str(&format!("<li>{}</li>", dyn_to_escaped(item)));
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
