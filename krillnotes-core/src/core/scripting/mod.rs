//! Rhai-based scripting registry for Krillnotes note types and hooks.
//!
//! [`ScriptRegistry`] is the public entry point. It owns the Rhai [`Engine`],
//! loads scripts, and delegates schema and hook concerns to internal sub-registries.

mod display_helpers;
mod hooks;
mod schema;

// Re-exported for API stability; currently a placeholder for future global/lifecycle hooks.
pub use hooks::HookRegistry;
pub(crate) use schema::field_value_to_dynamic;
pub use schema::{AddChildResult, FieldDefinition, Schema};
// StarterScript is defined in this file and re-exported via lib.rs.

use crate::{FieldValue, KrillnotesError, Note, Result};
use schema::HookEntry;
use include_dir::{include_dir, Dir};
use rhai::{Dynamic, Engine, EvalAltResult, FnPtr, Map, AST};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Pre-built index of all workspace notes, populated before each `on_view` hook call
/// and cleared immediately afterwards.
///
/// Each note is stored as a Rhai map so it can be passed directly to scripts
/// without conversion overhead at query time.
#[derive(Debug)]
pub struct QueryContext {
    pub notes_by_id:    HashMap<String, Dynamic>,
    pub children_by_id: HashMap<String, Vec<Dynamic>>,
    pub notes_by_type:  HashMap<String, Vec<Dynamic>>,
}

static STARTER_SCRIPTS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/system_scripts");

/// A bundled starter script with its filename and source code.
pub struct StarterScript {
    /// The filename (e.g. `"00_text_note.rhai"`), used to derive load order.
    pub filename: String,
    /// The full Rhai source code.
    pub source_code: String,
}

/// Orchestrating registry that owns the Rhai engine and delegates to
/// [`SchemaRegistry`](schema::SchemaRegistry) for schema parsing and hook execution.
///
/// This is the primary scripting entry point used by [`Workspace`](crate::Workspace).
#[derive(Debug)]
pub struct ScriptRegistry {
    engine: Engine,
    current_loading_ast: Arc<Mutex<Option<AST>>>,
    current_loading_script_name: Arc<Mutex<Option<String>>>,
    schema_registry: schema::SchemaRegistry,
    query_context: Arc<Mutex<Option<QueryContext>>>,
}

impl ScriptRegistry {
    /// Creates a new, empty registry with no scripts loaded.
    ///
    /// Use [`starter_scripts()`](Self::starter_scripts) to get the bundled
    /// starter scripts for seeding a new workspace.
    pub fn new() -> Result<Self> {
        let mut engine = Engine::new();
        let schema_registry = schema::SchemaRegistry::new();
        let current_loading_ast: Arc<Mutex<Option<AST>>> = Arc::new(Mutex::new(None));
        let current_loading_script_name: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

        // Register schema() host function — writes schema and schema-bound hooks into SchemaRegistry.
        let schemas_arc       = schema_registry.schemas_arc();
        let on_save_arc       = schema_registry.on_save_hooks_arc();
        let on_view_arc       = schema_registry.on_view_hooks_arc();
        let on_add_child_arc  = schema_registry.on_add_child_hooks_arc();
        let schema_ast_arc    = Arc::clone(&current_loading_ast);
        let schema_name_arc   = Arc::clone(&current_loading_script_name);
        engine.register_fn("schema", move |name: String, def: rhai::Map| -> std::result::Result<Dynamic, Box<EvalAltResult>> {
            let s = Schema::parse_from_rhai(&name, &def)
                .map_err(|e| -> Box<EvalAltResult> { e.to_string().into() })?;
            schemas_arc.lock().unwrap().insert(name.clone(), s);

            // Extract optional on_save closure.
            if let Some(fn_ptr) = def.get("on_save").and_then(|v| v.clone().try_cast::<FnPtr>()) {
                let ast = schema_ast_arc.lock().unwrap().clone()
                    .ok_or_else(|| -> Box<EvalAltResult> {
                        "schema() called outside of load_script".to_string().into()
                    })?;
                let script_name = schema_name_arc.lock().unwrap()
                    .clone()
                    .unwrap_or_else(|| "<unknown>".to_string());
                on_save_arc.lock().unwrap().insert(name.clone(), HookEntry { fn_ptr, ast, script_name });
            }

            // Extract optional on_view closure.
            if let Some(fn_ptr) = def.get("on_view").and_then(|v| v.clone().try_cast::<FnPtr>()) {
                let ast = schema_ast_arc.lock().unwrap().clone()
                    .ok_or_else(|| -> Box<EvalAltResult> {
                        "schema() called outside of load_script".to_string().into()
                    })?;
                let script_name = schema_name_arc.lock().unwrap()
                    .clone()
                    .unwrap_or_else(|| "<unknown>".to_string());
                on_view_arc.lock().unwrap().insert(name.clone(), HookEntry { fn_ptr, ast, script_name });
            }

            // Extract optional on_add_child closure.
            if let Some(fn_ptr) = def.get("on_add_child").and_then(|v| v.clone().try_cast::<FnPtr>()) {
                let ast = schema_ast_arc.lock().unwrap().clone()
                    .ok_or_else(|| -> Box<EvalAltResult> {
                        "schema() called outside of load_script".to_string().into()
                    })?;
                let script_name = schema_name_arc.lock().unwrap()
                    .clone()
                    .unwrap_or_else(|| "<unknown>".to_string());
                on_add_child_arc.lock().unwrap().insert(name.clone(), HookEntry { fn_ptr, ast, script_name });
            }

            Ok(Dynamic::UNIT)
        });

        // Register schema_exists() — query function for scripts.
        let exists_arc = schema_registry.schemas_arc();
        engine.register_fn("schema_exists", move |name: String| -> bool {
            exists_arc.lock().unwrap().contains_key(&name)
        });

        // Register get_schema_fields() — returns field definitions as Rhai array.
        let fields_arc = schema_registry.schemas_arc();
        engine.register_fn("get_schema_fields", move |name: String| -> std::result::Result<Dynamic, Box<EvalAltResult>> {
            let schemas = fields_arc.lock().unwrap();
            let schema = schemas.get(&name).ok_or_else(|| -> Box<EvalAltResult> {
                format!("Schema '{name}' not found").into()
            })?;
            let mut arr = rhai::Array::new();
            for field in &schema.fields {
                let mut map = rhai::Map::new();
                map.insert("name".into(), Dynamic::from(field.name.clone()));
                map.insert("type".into(), Dynamic::from(field.field_type.clone()));
                map.insert("required".into(), Dynamic::from(field.required));
                map.insert("can_view".into(), Dynamic::from(field.can_view));
                map.insert("can_edit".into(), Dynamic::from(field.can_edit));
                map.insert("options".into(), Dynamic::from(
                    field.options.iter().map(|s| Dynamic::from(s.clone())).collect::<rhai::Array>()
                ));
                map.insert("max".into(), Dynamic::from(field.max));
                arr.push(Dynamic::from(map));
            }
            Ok(Dynamic::from(arr))
        });

        // ── Query context for on_view hooks ──────────────────────────────────
        let query_context: Arc<Mutex<Option<QueryContext>>> = Arc::new(Mutex::new(None));

        // Register get_children() — returns direct children of a note by ID.
        let qc1 = Arc::clone(&query_context);
        engine.register_fn("get_children", move |id: String| -> rhai::Array {
            let guard = qc1.lock().unwrap();
            guard.as_ref()
                .and_then(|ctx| ctx.children_by_id.get(&id).cloned())
                .unwrap_or_default()
        });

        // Register get_note() — returns any note by ID.
        let qc2 = Arc::clone(&query_context);
        engine.register_fn("get_note", move |id: String| -> Dynamic {
            let guard = qc2.lock().unwrap();
            guard.as_ref()
                .and_then(|ctx| ctx.notes_by_id.get(&id).cloned())
                .unwrap_or(Dynamic::UNIT)
        });

        // Register get_notes_of_type() — returns all notes of a given schema type.
        let qc3 = Arc::clone(&query_context);
        engine.register_fn("get_notes_of_type", move |node_type: String| -> rhai::Array {
            let guard = qc3.lock().unwrap();
            guard.as_ref()
                .and_then(|ctx| ctx.notes_by_type.get(&node_type).cloned())
                .unwrap_or_default()
        });

        // ── Display helpers for on_view hooks ─────────────────────────────────
        engine.register_fn("table",   display_helpers::table);
        engine.register_fn("section", display_helpers::section);
        engine.register_fn("stack",   display_helpers::stack);
        engine.register_fn("columns", display_helpers::columns);
        engine.register_fn("field",   display_helpers::field_row);
        engine.register_fn("fields",  display_helpers::fields);
        engine.register_fn("heading", display_helpers::heading);
        engine.register_fn("text",    display_helpers::view_text);
        engine.register_fn("list",    display_helpers::list);
        engine.register_fn("badge",   display_helpers::badge);
        engine.register_fn("badge",   display_helpers::badge_colored);
        engine.register_fn("divider", display_helpers::divider);
        engine.register_fn("link_to", display_helpers::link_to);
        engine.register_fn("markdown", display_helpers::rhai_markdown);

        Ok(Self {
            engine,
            current_loading_ast,
            current_loading_script_name,
            schema_registry,
            query_context,
        })
    }

    /// Returns the bundled starter scripts, sorted by filename (load order).
    ///
    /// These are embedded in the binary at compile time and used to seed new
    /// workspaces. Each script has a numbered prefix (e.g. `00_text_note.rhai`)
    /// that determines its load order.
    pub fn starter_scripts() -> Vec<StarterScript> {
        let mut scripts: Vec<StarterScript> = STARTER_SCRIPTS
            .files()
            .filter_map(|file| {
                let filename = file.path().file_name()?.to_str()?.to_string();
                let source_code = file.contents_utf8()?.to_string();
                Some(StarterScript { filename, source_code })
            })
            .collect();
        scripts.sort_by(|a, b| a.filename.cmp(&b.filename));
        scripts
    }

    /// Evaluates `script` and registers any schemas and hooks it defines.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError::Scripting`] if the script fails to evaluate.
    pub fn load_script(&mut self, script: &str, name: &str) -> Result<()> {
        let ast = self
            .engine
            .compile(script)
            .map_err(|e| KrillnotesError::Scripting(e.to_string()))?;

        // SAFETY: mutex poisoning would require a panic while the lock is held,
        // which cannot happen in this codebase's single-threaded usage.
        *self.current_loading_ast.lock().unwrap() = Some(ast.clone());
        *self.current_loading_script_name.lock().unwrap() = Some(name.to_string());

        let result = self
            .engine
            .eval_ast::<()>(&ast)
            .map_err(|e| KrillnotesError::Scripting(e.to_string()));

        // Always clear: a failed script may have partially registered hooks;
        // leave no stale AST for the next load.
        *self.current_loading_ast.lock().unwrap() = None;
        *self.current_loading_script_name.lock().unwrap() = None;

        result
    }

    /// Returns the schema registered under `name`.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError::SchemaNotFound`] if no schema with that name
    /// has been registered.
    pub fn get_schema(&self, name: &str) -> Result<Schema> {
        self.schema_registry.get(name)
    }

    /// Returns the names of all currently registered schemas.
    pub fn list_types(&self) -> Result<Vec<String>> {
        Ok(self.schema_registry.list())
    }

    /// Returns all registered schemas keyed by name.
    pub fn all_schemas(&self) -> HashMap<String, Schema> {
        self.schema_registry.all()
    }

    /// Runs the pre-save hook registered for `schema_name`, if any.
    ///
    /// Delegates to [`SchemaRegistry::run_on_save_hook`](schema::SchemaRegistry::run_on_save_hook) with this registry's engine.
    ///
    /// Returns `Ok(None)` when no hook is registered for `schema_name`.
    /// Returns `Ok(Some((title, fields)))` with the hook's output on success.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError::Scripting`] if the hook throws a Rhai error
    /// or returns a malformed map.
    pub fn run_on_save_hook(
        &self,
        schema_name: &str,
        note_id: &str,
        node_type: &str,
        title: &str,
        fields: &HashMap<String, FieldValue>,
    ) -> Result<Option<(String, HashMap<String, FieldValue>)>> {
        let schema = self.schema_registry.get(schema_name)?;
        self.schema_registry
            .run_on_save_hook(&self.engine, &schema, note_id, node_type, title, fields)
    }

    /// Runs the `on_add_child` hook registered for `parent_schema_name`, if any.
    ///
    /// Returns `Ok(None)` when no hook is registered for `parent_schema_name`.
    /// Returns `Ok(Some(AddChildResult))` with optional parent/child updates on success.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError::Scripting`] if the hook throws a Rhai error
    /// or returns a malformed map.
    pub fn run_on_add_child_hook(
        &self,
        parent_schema_name: &str,
        parent_id: &str,
        parent_type: &str,
        parent_title: &str,
        parent_fields: &HashMap<String, FieldValue>,
        child_id: &str,
        child_type: &str,
        child_title: &str,
        child_fields: &HashMap<String, FieldValue>,
    ) -> Result<Option<AddChildResult>> {
        let parent_schema = self.schema_registry.get(parent_schema_name)?;
        let child_schema  = self.schema_registry.get(child_type)?;
        self.schema_registry.run_on_add_child_hook(
            &self.engine,
            &parent_schema,
            parent_id, parent_type, parent_title, parent_fields,
            &child_schema,
            child_id, child_type, child_title, child_fields,
        )
    }

    /// Returns `true` if an on_save hook is registered for `schema_name`.
    pub fn has_hook(&self, schema_name: &str) -> bool {
        self.schema_registry.has_hook(schema_name)
    }

    /// Returns `true` if a view hook is registered for `schema_name`.
    pub fn has_view_hook(&self, schema_name: &str) -> bool {
        self.schema_registry.has_view_hook(schema_name)
    }

    /// Renders a default HTML view for `note` using schema field type information.
    ///
    /// Used when no `on_view` hook is registered for the note's type — the result
    /// is sent to the frontend instead of falling back to `FieldDisplay.tsx`.
    ///
    /// Textarea fields are rendered as CommonMark HTML; all other fields are
    /// HTML-escaped plain text. Fields not in the schema appear in a legacy section.
    pub fn render_default_view(&self, note: &Note) -> String {
        let schema = self.schema_registry.get(&note.node_type).ok();
        display_helpers::render_default_view(note, schema.as_ref())
    }

    /// Runs the view hook registered for the given note's schema, if any.
    ///
    /// Populates the query context from `context`, calls the hook, then clears
    /// the context so query functions return empty results outside of a hook call.
    ///
    /// Returns `Ok(None)` when no hook is registered for the note's schema.
    /// Returns `Ok(Some(html))` with the generated HTML on success.
    pub fn run_on_view_hook(
        &self,
        note: &Note,
        context: QueryContext,
    ) -> Result<Option<String>> {
        // Build the note map (same structure as on_save).
        let mut fields_map = Map::new();
        for (k, v) in &note.fields {
            fields_map.insert(k.as_str().into(), field_value_to_dynamic(v));
        }
        let mut note_map = Map::new();
        note_map.insert("id".into(), Dynamic::from(note.id.clone()));
        note_map.insert("node_type".into(), Dynamic::from(note.node_type.clone()));
        note_map.insert("title".into(), Dynamic::from(note.title.clone()));
        note_map.insert("fields".into(), Dynamic::from(fields_map));

        // Install query context, run hook, then clear.
        *self.query_context.lock().unwrap() = Some(context);
        let result = self.schema_registry.run_on_view_hook(&self.engine, note_map);
        *self.query_context.lock().unwrap() = None;
        result
    }

    /// Removes all registered schemas and hooks so scripts can be reloaded.
    pub fn clear_all(&self) {
        self.schema_registry.clear();
        *self.query_context.lock().unwrap() = None;
    }

    /// Returns `true` if a schema with `name` is registered.
    pub fn schema_exists(&self, name: &str) -> bool {
        self.schema_registry.exists(name)
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: loads the bundled TextNote starter script into a registry.
    fn load_text_note(registry: &mut ScriptRegistry) {
        registry.load_script(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/system_scripts/00_text_note.rhai"
        )), "TextNote").expect("TextNote starter script should load");
    }

    // ── hooks-inside-schema (new style) ─────────────────────────────────────

    #[test]
    fn test_on_save_inside_schema_sets_title() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Person", #{
                fields: [
                    #{ name: "first", type: "text", required: false },
                    #{ name: "last",  type: "text", required: false },
                ],
                on_save: |note| {
                    note.title = note.fields["last"] + ", " + note.fields["first"];
                    note
                }
            });
        "#, "test").unwrap();

        let mut fields = std::collections::HashMap::new();
        fields.insert("first".to_string(), FieldValue::Text("John".to_string()));
        fields.insert("last".to_string(), FieldValue::Text("Doe".to_string()));

        let result = registry
            .run_on_save_hook("Person", "id-1", "Person", "old title", &fields)
            .unwrap();

        assert!(result.is_some());
        let (new_title, _) = result.unwrap();
        assert_eq!(new_title, "Doe, John");
    }

    #[test]
    fn test_on_view_inside_schema_returns_html() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Folder", #{
                fields: [],
                on_view: |note| {
                    text("hello from view")
                }
            });
        "#, "test").unwrap();

        use crate::Note;
        let note = Note {
            id: "n1".to_string(), node_type: "Folder".to_string(),
            title: "F".to_string(), parent_id: None, position: 0,
            created_at: 0, modified_at: 0, created_by: 0, modified_by: 0,
            fields: std::collections::HashMap::new(), is_expanded: false,
        };
        let ctx = QueryContext {
            notes_by_id: std::collections::HashMap::new(),
            children_by_id: std::collections::HashMap::new(),
            notes_by_type: std::collections::HashMap::new(),
        };
        let html = registry.run_on_view_hook(&note, ctx).unwrap();
        assert!(html.is_some());
        assert!(html.unwrap().contains("hello from view"));
    }

    // ── has_hook() ─────────────────────────────────────────────────────────

    #[test]
    fn test_has_hook_after_schema_with_on_save() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry
            .load_script(
                r#"
                schema("Widget", #{
                    fields: [ #{ name: "label", type: "text", required: false } ],
                    on_save: |note| { note }
                });
            "#,
            "test")
            .unwrap();
        assert!(registry.has_hook("Widget"));
        assert!(!registry.has_hook("Missing"));
    }

    // ── Schema registration ─────────────────────────────────────────────────

    #[test]
    fn test_schema_registration() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry
            .load_script(
                r#"
                schema("TestNote", #{
                    fields: [
                        #{ name: "body", type: "text", required: false },
                        #{ name: "count", type: "number", required: false },
                    ]
                });
            "#,
            "test")
            .unwrap();
        let schema = registry.get_schema("TestNote").unwrap();
        assert_eq!(schema.name, "TestNote");
        assert_eq!(schema.fields.len(), 2);
        assert_eq!(schema.fields[0].name, "body");
        assert_eq!(schema.fields[0].field_type, "text");
    }

    #[test]
    fn test_default_fields() {
        let schema = Schema {
            name: "TestNote".to_string(),
            fields: vec![
                FieldDefinition {
                    name: "body".to_string(),
                    field_type: "text".to_string(),
                    required: false,
                    can_view: true,
                    can_edit: true,
                    options: vec![],
                    max: 0,
                },
                FieldDefinition {
                    name: "count".to_string(),
                    field_type: "number".to_string(),
                    required: false,
                    can_view: true,
                    can_edit: true,
                    options: vec![],
                    max: 0,
                },
            ],
            title_can_view: true,
            title_can_edit: true,
            children_sort: "none".to_string(),
            allowed_parent_types: vec![],
            allowed_children_types: vec![],
        };
        let defaults = schema.default_fields();
        assert_eq!(defaults.len(), 2);
        assert!(matches!(defaults.get("body"), Some(FieldValue::Text(_))));
        assert!(matches!(defaults.get("count"), Some(FieldValue::Number(_))));
    }

    #[test]
    fn test_text_note_schema_loaded_from_starter() {
        let mut registry = ScriptRegistry::new().unwrap();
        load_text_note(&mut registry);
        let schema = registry.get_schema("TextNote").unwrap();
        assert_eq!(schema.name, "TextNote");
        assert_eq!(schema.fields.len(), 1);
        assert_eq!(schema.fields[0].name, "body");
        assert_eq!(schema.fields[0].field_type, "textarea");
    }

    #[test]
    fn test_new_registry_is_empty() {
        let registry = ScriptRegistry::new().unwrap();
        assert!(registry.get_schema("TextNote").is_err(), "New registry should have no schemas");
        assert!(registry.list_types().unwrap().is_empty());
    }

    #[test]
    fn test_date_field_default() {
        let schema = Schema {
            name: "Test".to_string(),
            fields: vec![FieldDefinition {
                name: "birthday".to_string(),
                field_type: "date".to_string(),
                required: false,
                can_view: true,
                can_edit: true,
                options: vec![],
                max: 0,
            }],
            title_can_view: true,
            title_can_edit: true,
            children_sort: "none".to_string(),
            allowed_parent_types: vec![],
            allowed_children_types: vec![],
        };
        let defaults = schema.default_fields();
        assert!(matches!(defaults.get("birthday"), Some(FieldValue::Date(None))));
    }

    #[test]
    fn test_email_field_default() {
        let schema = Schema {
            name: "Test".to_string(),
            fields: vec![FieldDefinition {
                name: "email_addr".to_string(),
                field_type: "email".to_string(),
                required: false,
                can_view: true,
                can_edit: true,
                options: vec![],
                max: 0,
            }],
            title_can_view: true,
            title_can_edit: true,
            children_sort: "none".to_string(),
            allowed_parent_types: vec![],
            allowed_children_types: vec![],
        };
        let defaults = schema.default_fields();
        assert!(matches!(defaults.get("email_addr"), Some(FieldValue::Email(s)) if s.is_empty()));
    }

    #[test]
    fn test_contact_schema_loaded() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/system_scripts/01_contact.rhai"
        )), "Contact").unwrap();
        let schema = registry.get_schema("Contact").unwrap();
        assert_eq!(schema.name, "Contact");
        assert_eq!(schema.fields.len(), 12);
        let is_family_field = schema.fields.iter().find(|f| f.name == "is_family").unwrap();
        assert_eq!(is_family_field.field_type, "boolean");
        assert!(!is_family_field.required, "is_family should not be required");
        let email_field = schema.fields.iter().find(|f| f.name == "email").unwrap();
        assert_eq!(email_field.field_type, "email");
        let birthdate_field = schema.fields.iter().find(|f| f.name == "birthdate").unwrap();
        assert_eq!(birthdate_field.field_type, "date");
        let first_name_field = schema.fields.iter().find(|f| f.name == "first_name").unwrap();
        assert!(first_name_field.required, "first_name should be required");
        let last_name_field = schema.fields.iter().find(|f| f.name == "last_name").unwrap();
        assert!(last_name_field.required, "last_name should be required");
    }

    // ── on_save hooks ───────────────────────────────────────────────────────

    #[test]
    fn test_run_on_save_hook_sets_title() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry
            .load_script(
                r#"
                schema("Person", #{
                    fields: [
                        #{ name: "first", type: "text", required: false },
                        #{ name: "last",  type: "text", required: false },
                    ],
                    on_save: |note| {
                        note.title = note.fields["last"] + ", " + note.fields["first"];
                        note
                    }
                });
            "#,
            "test")
            .unwrap();

        let mut fields = HashMap::new();
        fields.insert("first".to_string(), FieldValue::Text("John".to_string()));
        fields.insert("last".to_string(), FieldValue::Text("Doe".to_string()));

        let result = registry
            .run_on_save_hook("Person", "id-1", "Person", "old title", &fields)
            .unwrap();

        assert!(result.is_some());
        let (new_title, new_fields) = result.unwrap();
        assert_eq!(new_title, "Doe, John");
        assert_eq!(new_fields.get("first"), Some(&FieldValue::Text("John".to_string())));
    }

    #[test]
    fn test_run_on_save_hook_no_hook_returns_none() {
        let mut registry = ScriptRegistry::new().unwrap();
        load_text_note(&mut registry);
        let fields = HashMap::new();
        let result = registry
            .run_on_save_hook("TextNote", "id-1", "TextNote", "title", &fields)
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_contact_on_save_hook_derives_title() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/system_scripts/01_contact.rhai"
        )), "Contact").unwrap();
        assert!(registry.has_hook("Contact"), "Contact schema should have an on_save hook");

        let mut fields = HashMap::new();
        fields.insert("first_name".to_string(), FieldValue::Text("Jane".to_string()));
        fields.insert("middle_name".to_string(), FieldValue::Text("".to_string()));
        fields.insert("last_name".to_string(), FieldValue::Text("Smith".to_string()));
        fields.insert("phone".to_string(), FieldValue::Text("".to_string()));
        fields.insert("mobile".to_string(), FieldValue::Text("".to_string()));
        fields.insert("email".to_string(), FieldValue::Email("".to_string()));
        fields.insert("birthdate".to_string(), FieldValue::Date(None));
        fields.insert("address_street".to_string(), FieldValue::Text("".to_string()));
        fields.insert("address_city".to_string(), FieldValue::Text("".to_string()));
        fields.insert("address_zip".to_string(), FieldValue::Text("".to_string()));
        fields.insert("address_country".to_string(), FieldValue::Text("".to_string()));
        fields.insert("is_family".to_string(), FieldValue::Boolean(false));

        let result = registry
            .run_on_save_hook("Contact", "id-1", "Contact", "", &fields)
            .unwrap()
            .unwrap();

        assert_eq!(result.0, "Smith, Jane");
    }

    // ── Field flags ─────────────────────────────────────────────────────────

    #[test]
    fn test_field_can_view_can_edit_defaults_to_true() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("TestVis", #{
                fields: [
                    #{ name: "f1", type: "text" },
                ]
            });
        "#, "test").unwrap();
        let schema = registry.get_schema("TestVis").unwrap();
        assert!(schema.fields[0].can_view, "can_view should default to true");
        assert!(schema.fields[0].can_edit, "can_edit should default to true");
    }

    #[test]
    fn test_field_can_view_can_edit_explicit_false() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("TestVis2", #{
                fields: [
                    #{ name: "view_only", type: "text", can_edit: false },
                    #{ name: "edit_only", type: "text", can_view: false },
                ]
            });
        "#, "test").unwrap();
        let schema = registry.get_schema("TestVis2").unwrap();
        assert!(schema.fields[0].can_view);
        assert!(!schema.fields[0].can_edit);
        assert!(!schema.fields[1].can_view);
        assert!(schema.fields[1].can_edit);
    }

    #[test]
    fn test_field_can_view_can_edit_explicit_true() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("TestVisExplicit", #{
                fields: [
                    #{ name: "both_true",  type: "text", can_view: true,  can_edit: true  },
                    #{ name: "both_false", type: "text", can_view: false, can_edit: false },
                ]
            });
        "#, "test").unwrap();
        let schema = registry.get_schema("TestVisExplicit").unwrap();
        assert!(schema.fields[0].can_view,  "explicit can_view: true should parse as true");
        assert!(schema.fields[0].can_edit,  "explicit can_edit: true should parse as true");
        assert!(!schema.fields[1].can_view, "explicit can_view: false should parse as false");
        assert!(!schema.fields[1].can_edit, "explicit can_edit: false should parse as false");
    }

    // ── Title flags ─────────────────────────────────────────────────────────

    #[test]
    fn test_schema_title_flags_default_to_true() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("TitleTest", #{
                fields: [
                    #{ name: "name", type: "text" },
                ]
            });
        "#, "test").unwrap();
        let schema = registry.get_schema("TitleTest").unwrap();
        assert!(schema.title_can_view, "title_can_view should default to true");
        assert!(schema.title_can_edit, "title_can_edit should default to true");
    }

    #[test]
    fn test_schema_title_can_edit_false() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("TitleHidden", #{
                title_can_edit: false,
                fields: [
                    #{ name: "name", type: "text" },
                ]
            });
        "#, "test").unwrap();
        let schema = registry.get_schema("TitleHidden").unwrap();
        assert!(schema.title_can_view);
        assert!(!schema.title_can_edit);
    }

    #[test]
    fn test_schema_title_flags_explicit_true() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("TitleExplicit", #{
                title_can_view: true,
                title_can_edit: true,
                fields: [
                    #{ name: "name", type: "text" },
                ]
            });
        "#, "test").unwrap();
        let schema = registry.get_schema("TitleExplicit").unwrap();
        assert!(schema.title_can_view,  "explicit title_can_view: true should parse as true");
        assert!(schema.title_can_edit,  "explicit title_can_edit: true should parse as true");
    }

    #[test]
    fn test_contact_title_can_edit_false() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/system_scripts/01_contact.rhai"
        )), "Contact").unwrap();
        let schema = registry.get_schema("Contact").unwrap();
        assert!(!schema.title_can_edit, "Contact title_can_edit should be false");
        assert!(schema.title_can_view, "Contact title_can_view should still be true");
    }

    // ── Boolean / default value edge cases ──────────────────────────────────

    #[test]
    fn test_boolean_field_defaults_to_false_when_absent_from_hook_result() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry
            .load_script(
                r#"
                schema("FlagNote", #{
                    fields: [
                        #{ name: "flag", type: "boolean", required: false },
                    ],
                    on_save: |note| {
                        // intentionally does NOT touch note.fields["flag"]
                        note
                    }
                });
            "#,
            "test")
            .unwrap();

        // Do NOT include "flag" in the submitted fields — it must default to false.
        let fields = HashMap::new();

        let result = registry
            .run_on_save_hook("FlagNote", "id-1", "FlagNote", "title", &fields)
            .unwrap()
            .unwrap();

        assert_eq!(
            result.1.get("flag"),
            Some(&FieldValue::Boolean(false)),
            "boolean field absent from hook result should default to false"
        );
    }

    // ── clear_all ───────────────────────────────────────────────────────────

    #[test]
    fn test_load_script_and_clear_all() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("MyType", #{ fields: [#{ name: "x", type: "text" }] });
        "#, "test").unwrap();

        assert!(registry.get_schema("MyType").is_ok());

        registry.clear_all();

        assert!(registry.get_schema("MyType").is_err());
    }

    #[test]
    fn test_clear_all_removes_everything() {
        let mut registry = ScriptRegistry::new().unwrap();
        load_text_note(&mut registry);
        registry.load_script(r#"
            schema("Custom", #{ fields: [#{ name: "a", type: "text" }] });
        "#, "test").unwrap();

        registry.clear_all();

        let types = registry.list_types().unwrap();
        assert!(types.is_empty(), "clear_all should remove all schemas");
    }

    // ── Host functions ──────────────────────────────────────────────────────

    #[test]
    fn test_schema_exists_host_function() {
        let mut registry = ScriptRegistry::new().unwrap();
        load_text_note(&mut registry);
        assert!(registry.schema_exists("TextNote"));
        assert!(!registry.schema_exists("NonExistent"));

        // Test via script execution
        registry.load_script(r#"
            let exists = schema_exists("TextNote");
            if !exists { throw "TextNote should exist"; }
            let missing = schema_exists("Missing");
            if missing { throw "Missing should not exist"; }
        "#, "test").unwrap();
    }

    #[test]
    fn test_get_schema_fields_host_function() {
        let mut registry = ScriptRegistry::new().unwrap();
        load_text_note(&mut registry);
        registry.load_script(r#"
            let fields = get_schema_fields("TextNote");
            if fields.len() != 1 { throw "Expected 1 field, got " + fields.len(); }
            if fields[0].name != "body" { throw "Expected 'body', got " + fields[0].name; }
            if fields[0].type != "textarea" { throw "Expected 'textarea', got " + fields[0].type; }
            if fields[0].options.len() != 0 { throw "Expected options length 0, got " + fields[0].options.len(); }
            if fields[0].max != 0 { throw "Expected max 0, got " + fields[0].max; }
        "#, "test").unwrap();
    }

    #[test]
    fn test_hooks_cleared_on_clear_all() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Hooked", #{
                fields: [#{ name: "x", type: "text" }],
                on_save: |note| { note }
            });
        "#, "test").unwrap();
        assert!(registry.has_hook("Hooked"));

        registry.clear_all();
        assert!(!registry.has_hook("Hooked"));
    }

    // ── Select / rating fields ──────────────────────────────────────────────

    #[test]
    fn test_select_field_parses_options() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Ticket", #{
                fields: [
                    #{ name: "status", type: "select", options: ["TODO", "WIP", "DONE"], required: true }
                ]
            });
        "#, "test").unwrap();
        let fields = get_schema_fields_for_test(&registry, "Ticket");
        assert_eq!(fields[0].options, vec!["TODO", "WIP", "DONE"]);
    }

    #[test]
    fn test_rating_field_parses_max() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Review", #{
                fields: [
                    #{ name: "stars", type: "rating", max: 5 }
                ]
            });
        "#, "test").unwrap();
        let fields = get_schema_fields_for_test(&registry, "Review");
        assert_eq!(fields[0].max, 5);
    }

    #[test]
    fn test_regular_fields_have_empty_options_and_zero_max() {
        let mut registry = ScriptRegistry::new().unwrap();
        load_text_note(&mut registry);
        let fields = get_schema_fields_for_test(&registry, "TextNote");
        assert!(fields[0].options.is_empty());
        assert_eq!(fields[0].max, 0);
    }

    fn get_schema_fields_for_test(registry: &ScriptRegistry, name: &str) -> Vec<FieldDefinition> {
        registry.get_schema(name).unwrap().fields
    }

    #[test]
    fn test_options_with_non_string_item_returns_error() {
        let mut registry = ScriptRegistry::new().unwrap();
        let result = registry.load_script(r#"
            schema("Bad", #{
                fields: [
                    #{ name: "status", type: "select", options: ["OK", 42] }
                ]
            });
        "#, "test");
        assert!(result.is_err(), "non-string item in options should return a Scripting error");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("strings"), "error should mention 'strings', got: {msg}");
    }

    // ── Starter scripts ─────────────────────────────────────────────────────

    #[test]
    fn test_starter_scripts_load_without_error() {
        let mut registry = ScriptRegistry::new().unwrap();
        let starters = ScriptRegistry::starter_scripts();
        assert!(!starters.is_empty(), "Should have bundled starter scripts");

        for starter in &starters {
            registry.load_script(&starter.source_code, &starter.filename)
                .unwrap_or_else(|e| panic!("{} should load: {e}", starter.filename));
        }

        assert!(registry.schema_exists("TextNote"));
        assert!(registry.schema_exists("Contact"));
        assert!(registry.schema_exists("ContactsFolder"));
        assert!(registry.schema_exists("Task"));
        assert!(registry.schema_exists("Project"));
        assert!(registry.schema_exists("Book"));
        assert!(registry.schema_exists("Product"));
        assert!(registry.schema_exists("Recipe"));
    }

    #[test]
    fn test_starter_scripts_sorted_by_filename() {
        let starters = ScriptRegistry::starter_scripts();
        let filenames: Vec<&str> = starters.iter().map(|s| s.filename.as_str()).collect();
        let mut sorted = filenames.clone();
        sorted.sort();
        assert_eq!(filenames, sorted, "Starter scripts should be sorted by filename");
    }

    #[test]
    fn test_negative_max_returns_error() {
        let mut registry = ScriptRegistry::new().unwrap();
        let result = registry.load_script(r#"
            schema("Bad", #{
                fields: [
                    #{ name: "stars", type: "rating", max: -1 }
                ]
            });
        "#, "test");
        assert!(result.is_err(), "negative max should return a Scripting error");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("max"), "error should mention 'max', got: {msg}");
    }

    #[test]
    fn test_select_and_rating_default_fields() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Widget", #{
                fields: [
                    #{ name: "status", type: "select", options: ["A", "B"] },
                    #{ name: "stars",  type: "rating",  max: 5 }
                ]
            });
        "#, "test").unwrap();
        let schema = registry.get_schema("Widget").unwrap();
        let defaults = schema.default_fields();
        assert_eq!(defaults["status"], crate::FieldValue::Text(String::new()));
        assert_eq!(defaults["stars"],  crate::FieldValue::Number(0.0));
    }

    #[test]
    fn test_select_field_round_trips_through_hook() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("S", #{
                fields: [ #{ name: "status", type: "select", options: ["A", "B"] } ],
                on_save: |note| {
                    note.fields.status = "B";
                    note
                }
            });
        "#, "test").unwrap();

        let mut fields = HashMap::new();
        fields.insert("status".to_string(), FieldValue::Text("A".to_string()));

        let result = registry
            .run_on_save_hook("S", "id1", "S", "title", &fields)
            .unwrap()
            .unwrap();
        assert_eq!(result.1["status"], FieldValue::Text("B".to_string()));
    }

    #[test]
    fn test_rating_field_round_trips_through_hook() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("R", #{
                fields: [ #{ name: "stars", type: "rating", max: 5 } ],
                on_save: |note| {
                    note.fields.stars = 4.0;
                    note
                }
            });
        "#, "test").unwrap();

        let mut fields = HashMap::new();
        fields.insert("stars".to_string(), FieldValue::Number(0.0));

        let result = registry
            .run_on_save_hook("R", "id1", "R", "title", &fields)
            .unwrap()
            .unwrap();
        assert_eq!(result.1["stars"], FieldValue::Number(4.0));
    }

    #[test]
    fn test_select_field_defaults_to_empty_text_when_absent_from_hook_result() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("S2", #{
                fields: [ #{ name: "status", type: "select", options: ["A", "B"] } ],
                on_save: |note| {
                    // deliberately do NOT set note.fields.status
                    note
                }
            });
        "#, "test").unwrap();

        let fields = HashMap::new(); // no status field
        let result = registry
            .run_on_save_hook("S2", "id1", "S2", "title", &fields)
            .unwrap()
            .unwrap();
        assert_eq!(result.1["status"], FieldValue::Text(String::new()));
    }

    #[test]
    fn test_rating_field_defaults_to_zero_when_absent_from_hook_result() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("R2", #{
                fields: [ #{ name: "stars", type: "rating", max: 5 } ],
                on_save: |note| {
                    // deliberately do NOT set note.fields.stars
                    note
                }
            });
        "#, "test").unwrap();

        let fields = HashMap::new(); // no stars field
        let result = registry
            .run_on_save_hook("R2", "id1", "R2", "title", &fields)
            .unwrap()
            .unwrap();
        assert_eq!(result.1["stars"], FieldValue::Number(0.0));
    }

    // ── children_sort ───────────────────────────────────────────────────────

    #[test]
    fn test_children_sort_defaults_to_none() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("SortTest", #{
                fields: [#{ name: "x", type: "text" }]
            });
        "#, "test").unwrap();
        let schema = registry.get_schema("SortTest").unwrap();
        assert_eq!(schema.children_sort, "none", "children_sort should default to 'none'");
    }

    #[test]
    fn test_children_sort_explicit_asc() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("SortAsc", #{
                children_sort: "asc",
                fields: [#{ name: "x", type: "text" }]
            });
        "#, "test").unwrap();
        let schema = registry.get_schema("SortAsc").unwrap();
        assert_eq!(schema.children_sort, "asc");
    }

    #[test]
    fn test_children_sort_explicit_desc() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("SortDesc", #{
                children_sort: "desc",
                fields: [#{ name: "x", type: "text" }]
            });
        "#, "test").unwrap();
        let schema = registry.get_schema("SortDesc").unwrap();
        assert_eq!(schema.children_sort, "desc");
    }

    // ── Book hook edge case ─────────────────────────────────────────────────

    #[test]
    fn test_book_hook_with_unset_dates_does_not_error() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/system_scripts/04_book.rhai"
        )), "Book").expect("book.rhai should load");

        let mut fields = std::collections::HashMap::new();
        fields.insert("book_title".to_string(), crate::FieldValue::Text("Dune".to_string()));
        fields.insert("author".to_string(), crate::FieldValue::Text("Herbert".to_string()));
        fields.insert("genre".to_string(), crate::FieldValue::Text(String::new()));
        fields.insert("status".to_string(), crate::FieldValue::Text(String::new()));
        fields.insert("rating".to_string(), crate::FieldValue::Number(0.0));
        fields.insert("started".to_string(), crate::FieldValue::Date(None));
        fields.insert("finished".to_string(), crate::FieldValue::Date(None));
        fields.insert("read_duration".to_string(), crate::FieldValue::Text(String::new()));
        fields.insert("notes".to_string(), crate::FieldValue::Text(String::new()));

        let result = registry.run_on_save_hook("Book", "id1", "Book", "Dune", &fields);
        assert!(result.is_ok(), "book hook should not error with unset dates: {:?}", result);
        let (title, out_fields) = result.unwrap().unwrap();
        assert_eq!(title, "Herbert: Dune");
        assert_eq!(out_fields["read_duration"], crate::FieldValue::Text(String::new()));
    }

    // ── render_default_view on ScriptRegistry ───────────────────────────────

    #[test]
    fn test_script_registry_render_default_view_textarea_markdown() {
        use crate::{FieldValue, Note};
        use std::collections::HashMap;

        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Memo", #{
                fields: [
                    #{ name: "body", type: "textarea", required: false }
                ]
            });
        "#, "test").unwrap();

        let mut fields = HashMap::new();
        fields.insert("body".into(), FieldValue::Text("**important**".into()));
        let note = Note {
            id: "n1".into(), title: "Test".into(), node_type: "Memo".into(),
            parent_id: None, position: 0, created_at: 0, modified_at: 0,
            created_by: 0, modified_by: 0, fields, is_expanded: false,
        };

        let html = registry.render_default_view(&note);
        assert!(html.contains("<strong>important</strong>"), "got: {html}");
    }

    // ── markdown() Rhai host function ───────────────────────────────────────

    #[test]
    fn test_markdown_rhai_function_renders_bold() {
        let mut registry = ScriptRegistry::new().unwrap();
        let script = r#"
            let result = markdown("**hello**");
            result
        "#;
        let result = registry.engine.eval::<String>(script).unwrap();
        assert!(result.contains("<strong>hello</strong>"), "got: {result}");
    }

    // ── link_to integration ─────────────────────────────────────────────────

    #[test]
    fn test_link_to_is_callable_from_on_view_script() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("LinkTest", #{
                fields: [#{ name: "ref_id", type: "text" }],
                on_view: |note| {
                    let target = #{ id: "target-id-123", title: "Target Note", fields: #{}, node_type: "TextNote" };
                    link_to(target)
                }
            });
        "#, "test").unwrap();

        let note = Note {
            id: "note-1".to_string(),
            node_type: "LinkTest".to_string(),
            title: "Test".to_string(),
            parent_id: None,
            position: 0,
            created_at: 0,
            modified_at: 0,
            created_by: 0,
            modified_by: 0,
            fields: HashMap::new(),
            is_expanded: false,
        };

        let context = QueryContext {
            notes_by_id: HashMap::new(),
            children_by_id: HashMap::new(),
            notes_by_type: HashMap::new(),
        };

        let result = registry.run_on_view_hook(&note, context).unwrap();
        assert!(result.is_some());
        let html = result.unwrap();
        assert!(html.contains("kn-view-link"), "html should contain kn-view-link class");
        assert!(html.contains("target-id-123"), "html should contain the target note id");
        assert!(html.contains("Target Note"), "html should contain the target note title");
    }


    #[test]
    fn test_on_save_runtime_error_includes_script_name() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(
            r#"
            schema("Boom", #{
                fields: [ #{ name: "x", type: "text" } ],
                on_save: |note| {
                    throw "intentional runtime error";
                    note
                }
            });
            "#,
            "My Exploding Script",
        ).unwrap();

        let fields = HashMap::new();
        let err = registry
            .run_on_save_hook("Boom", "id-1", "Boom", "title", &fields)
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("My Exploding Script"),
            "error should include script name, got: {msg}"
        );
    }

    // ── on_add_child hooks ──────────────────────────────────────────────────

    #[test]
    fn test_on_add_child_hook_modifies_parent_and_child() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Folder", #{
                fields: [
                    #{ name: "count", type: "number", required: false },
                ],
                on_add_child: |parent_note, child_note| {
                    parent_note.fields["count"] = parent_note.fields["count"] + 1.0;
                    parent_note.title = "Folder (" + parent_note.fields["count"].to_int().to_string() + ")";
                    child_note.title = "Child from hook";
                    #{ parent: parent_note, child: child_note }
                }
            });
            schema("Item", #{
                fields: [
                    #{ name: "name", type: "text", required: false },
                ],
            });
        "#, "test").unwrap();

        let mut parent_fields = std::collections::HashMap::new();
        parent_fields.insert("count".to_string(), FieldValue::Number(0.0));

        let mut child_fields = std::collections::HashMap::new();
        child_fields.insert("name".to_string(), FieldValue::Text("".to_string()));

        let result = registry
            .run_on_add_child_hook(
                "Folder",
                "parent-id", "Folder", "Folder", &parent_fields,
                "child-id",  "Item",   "Untitled", &child_fields,
            )
            .unwrap();

        let result = result.expect("hook should return a result");
        let (p_title, p_fields) = result.parent.expect("should have parent update");
        assert_eq!(p_title, "Folder (1)");
        assert_eq!(p_fields["count"], FieldValue::Number(1.0));

        let (c_title, _) = result.child.expect("should have child update");
        assert_eq!(c_title, "Child from hook");
    }

    #[test]
    fn test_on_add_child_hook_absent_returns_none() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Plain", #{
                fields: [],
            });
        "#, "test").unwrap();

        let result = registry
            .run_on_add_child_hook(
                "Plain",
                "p-id", "Plain", "Title", &std::collections::HashMap::new(),
                "c-id", "Plain", "Child", &std::collections::HashMap::new(),
            )
            .unwrap();

        assert!(result.is_none(), "no hook registered should return None");
    }

    #[test]
    fn test_on_view_runtime_error_includes_script_name() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(
            r#"
            schema("BoomView", #{
                fields: [],
                on_view: |note| {
                    throw "intentional runtime error";
                    text("x")
                }
            });
            "#,
            "My View Script",
        ).unwrap();

        use crate::Note;
        let note = Note {
            id: "n1".to_string(), node_type: "BoomView".to_string(),
            title: "T".to_string(), parent_id: None, position: 0,
            created_at: 0, modified_at: 0, created_by: 0, modified_by: 0,
            fields: HashMap::new(), is_expanded: false,
        };
        let ctx = QueryContext {
            notes_by_id: HashMap::new(),
            children_by_id: HashMap::new(),
            notes_by_type: HashMap::new(),
        };
        let err = registry.run_on_view_hook(&note, ctx).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("My View Script"),
            "error should include script name, got: {msg}"
        );
    }

}
