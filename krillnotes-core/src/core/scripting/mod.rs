//! Rhai-based scripting registry for Krillnotes note types and hooks.
//!
//! [`ScriptRegistry`] is the public entry point. It owns the Rhai [`Engine`],
//! loads scripts, and delegates schema and hook concerns to internal sub-registries.

mod hooks;
mod schema;

pub use hooks::HookRegistry;
pub use schema::{FieldDefinition, Schema};

use crate::{FieldValue, KrillnotesError, Result};
use hooks::HookEntry;
use rhai::{Dynamic, Engine, EvalAltResult, FnPtr, AST};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Orchestrating registry that owns the Rhai engine and delegates to
/// [`SchemaRegistry`](schema::SchemaRegistry) and [`HookRegistry`].
///
/// This is the primary scripting entry point used by [`Workspace`](crate::Workspace).
#[derive(Debug)]
pub struct ScriptRegistry {
    engine: Engine,
    current_loading_ast: Arc<Mutex<Option<AST>>>,
    schema_registry: schema::SchemaRegistry,
    hook_registry: HookRegistry,
}

impl ScriptRegistry {
    /// Creates a new registry and loads the built-in system schemas.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError::Scripting`] if a bundled system script
    /// fails to parse or if any `schema(...)` call within it is malformed.
    pub fn new() -> Result<Self> {
        let mut engine = Engine::new();
        let schema_registry = schema::SchemaRegistry::new();
        let hook_registry = HookRegistry::new();
        let current_loading_ast: Arc<Mutex<Option<AST>>> = Arc::new(Mutex::new(None));

        // Register schema() host function — writes into SchemaRegistry.
        let schemas_arc = schema_registry.schemas_arc();
        let user_schemas_arc = schema_registry.user_schemas_arc();
        let source_arc = schema_registry.current_source_arc();
        engine.register_fn("schema", move |name: String, def: rhai::Map| -> std::result::Result<Dynamic, Box<EvalAltResult>> {
            let s = Schema::parse_from_rhai(&name, &def)
                .map_err(|e| -> Box<EvalAltResult> { e.to_string().into() })?;
            schemas_arc.lock().unwrap().insert(name.clone(), s);
            if *source_arc.lock().unwrap() == schema::ScriptSource::User {
                user_schemas_arc.lock().unwrap().push(name);
            }
            Ok(Dynamic::UNIT)
        });

        // Register on_save() host function — writes into HookRegistry.
        let hooks_arc = hook_registry.on_save_hooks_arc();
        let user_hooks_arc = hook_registry.user_hooks_arc();
        let ast_arc = Arc::clone(&current_loading_ast);
        let hook_source_arc = schema_registry.current_source_arc();
        engine.register_fn("on_save", move |name: String, fn_ptr: FnPtr| -> std::result::Result<Dynamic, Box<EvalAltResult>> {
            let maybe_ast = ast_arc.lock().unwrap().clone();
            let ast = maybe_ast.ok_or_else(|| -> Box<EvalAltResult> {
                "on_save called outside of load_script".to_string().into()
            })?;
            hooks_arc.lock().unwrap().insert(name.clone(), HookEntry { fn_ptr, ast });
            if *hook_source_arc.lock().unwrap() == schema::ScriptSource::User {
                user_hooks_arc.lock().unwrap().push(name);
            }
            Ok(Dynamic::UNIT)
        });

        // Register schema_exists() — query function for user scripts.
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

        let mut registry = Self {
            engine,
            current_loading_ast,
            schema_registry,
            hook_registry,
        };
        registry.load_script(include_str!("../../system_scripts/text_note.rhai"))?;
        registry.load_script(include_str!("../../system_scripts/contact.rhai"))?;

        Ok(registry)
    }

    /// Evaluates `script` and registers any schemas and hooks it defines.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError::Scripting`] if the script fails to evaluate.
    pub fn load_script(&mut self, script: &str) -> Result<()> {
        let ast = self
            .engine
            .compile(script)
            .map_err(|e| KrillnotesError::Scripting(e.to_string()))?;

        // SAFETY: mutex poisoning would require a panic while the lock is held,
        // which cannot happen in this codebase's single-threaded usage.
        *self.current_loading_ast.lock().unwrap() = Some(ast.clone());

        let result = self
            .engine
            .eval_ast::<()>(&ast)
            .map_err(|e| KrillnotesError::Scripting(e.to_string()));

        // Always clear: a failed script may have partially registered hooks;
        // leave no stale AST for the next load.
        *self.current_loading_ast.lock().unwrap() = None;

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

    /// Returns a reference to the [`HookRegistry`] for hook state queries.
    pub fn hooks(&self) -> &HookRegistry {
        &self.hook_registry
    }

    /// Runs the pre-save hook registered for `schema_name`, if any.
    ///
    /// Delegates to [`HookRegistry::run_on_save_hook`] with this registry's engine.
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
        self.hook_registry
            .run_on_save_hook(&self.engine, &schema, note_id, node_type, title, fields)
    }

    /// Loads a user script, marking all registrations as user-sourced.
    pub fn load_user_script(&mut self, script: &str) -> Result<()> {
        self.schema_registry.set_source(schema::ScriptSource::User);
        let result = self.load_script(script);
        self.schema_registry.set_source(schema::ScriptSource::System);
        result
    }

    /// Removes all schemas and hooks registered by user scripts.
    pub fn clear_user_registrations(&self) {
        self.schema_registry.clear_user();
        self.hook_registry.clear_user();
    }

    /// Returns `true` if a schema with `name` is registered.
    pub fn schema_exists(&self, name: &str) -> bool {
        self.schema_registry.exists(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── New test for the hooks() accessor ────────────────────────────────────

    #[test]
    fn test_hooks_accessor_returns_hook_registry() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry
            .load_script(
                r#"
                schema("Widget", #{
                    fields: [ #{ name: "label", type: "text", required: false } ]
                });
                on_save("Widget", |note| { note });
            "#,
            )
            .unwrap();
        assert!(registry.hooks().has_hook("Widget"));
        assert!(!registry.hooks().has_hook("Missing"));
    }

    // ── Ported tests (previously in scripting.rs) ────────────────────────────

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
            )
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
        };
        let defaults = schema.default_fields();
        assert_eq!(defaults.len(), 2);
        assert!(matches!(defaults.get("body"), Some(FieldValue::Text(_))));
        assert!(matches!(defaults.get("count"), Some(FieldValue::Number(_))));
    }

    #[test]
    fn test_text_note_schema_loaded() {
        let registry = ScriptRegistry::new().unwrap();
        let schema = registry.get_schema("TextNote").unwrap();
        assert_eq!(schema.name, "TextNote");
        assert_eq!(schema.fields.len(), 1);
        assert_eq!(schema.fields[0].name, "body");
        assert_eq!(schema.fields[0].field_type, "textarea");
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
        };
        let defaults = schema.default_fields();
        assert!(matches!(defaults.get("email_addr"), Some(FieldValue::Email(s)) if s.is_empty()));
    }

    #[test]
    fn test_contact_schema_loaded() {
        let registry = ScriptRegistry::new().unwrap();
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
                    ]
                });
                on_save("Person", |note| {
                    note.title = note.fields["last"] + ", " + note.fields["first"];
                    note
                });
            "#,
            )
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
        let registry = ScriptRegistry::new().unwrap();
        let fields = HashMap::new();
        let result = registry
            .run_on_save_hook("TextNote", "id-1", "TextNote", "title", &fields)
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_contact_on_save_hook_derives_title() {
        let registry = ScriptRegistry::new().unwrap();
        assert!(registry.hooks().has_hook("Contact"), "Contact schema should have an on_save hook");

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
    #[test]
    fn test_field_can_view_can_edit_defaults_to_true() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("TestVis", #{
                fields: [
                    #{ name: "f1", type: "text" },
                ]
            });
        "#).unwrap();
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
        "#).unwrap();
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
        "#).unwrap();
        let schema = registry.get_schema("TestVisExplicit").unwrap();
        assert!(schema.fields[0].can_view,  "explicit can_view: true should parse as true");
        assert!(schema.fields[0].can_edit,  "explicit can_edit: true should parse as true");
        assert!(!schema.fields[1].can_view, "explicit can_view: false should parse as false");
        assert!(!schema.fields[1].can_edit, "explicit can_edit: false should parse as false");
    }


    #[test]
    fn test_schema_title_flags_default_to_true() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("TitleTest", #{
                fields: [
                    #{ name: "name", type: "text" },
                ]
            });
        "#).unwrap();
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
        "#).unwrap();
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
        "#).unwrap();
        let schema = registry.get_schema("TitleExplicit").unwrap();
        assert!(schema.title_can_view,  "explicit title_can_view: true should parse as true");
        assert!(schema.title_can_edit,  "explicit title_can_edit: true should parse as true");

    }
    #[test]
    fn test_contact_title_can_edit_false() {
        let registry = ScriptRegistry::new().unwrap();
        let schema = registry.get_schema("Contact").unwrap();
        assert!(!schema.title_can_edit, "Contact title_can_edit should be false");
        assert!(schema.title_can_view, "Contact title_can_view should still be true");
    }

    #[test]
    fn test_boolean_field_defaults_to_false_when_absent_from_hook_result() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry
            .load_script(
                r#"
                schema("FlagNote", #{
                    fields: [
                        #{ name: "flag", type: "boolean", required: false },
                    ]
                });
                on_save("FlagNote", |note| {
                    // intentionally does NOT touch note.fields["flag"]
                    note
                });
            "#,
            )
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

    #[test]
    fn test_load_user_script_and_clear() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_user_script(r#"
            schema("UserType", #{ fields: [#{ name: "x", type: "text" }] });
        "#).unwrap();

        assert!(registry.get_schema("UserType").is_ok());

        registry.clear_user_registrations();

        assert!(registry.get_schema("UserType").is_err());
        // System schemas should still work
        assert!(registry.get_schema("TextNote").is_ok());
        assert!(registry.get_schema("Contact").is_ok());
    }

    #[test]
    fn test_clear_user_does_not_remove_system_schemas() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_user_script(r#"
            schema("Custom", #{ fields: [#{ name: "a", type: "text" }] });
        "#).unwrap();

        registry.clear_user_registrations();

        let types = registry.list_types().unwrap();
        assert!(types.contains(&"TextNote".to_string()));
        assert!(types.contains(&"Contact".to_string()));
        assert!(!types.contains(&"Custom".to_string()));
    }

    #[test]
    fn test_schema_exists_host_function() {
        let mut registry = ScriptRegistry::new().unwrap();
        assert!(registry.schema_exists("TextNote"));
        assert!(!registry.schema_exists("NonExistent"));

        // Test via script execution
        registry.load_script(r#"
            let exists = schema_exists("TextNote");
            if !exists { throw "TextNote should exist"; }
            let missing = schema_exists("Missing");
            if missing { throw "Missing should not exist"; }
        "#).unwrap();
    }

    #[test]
    fn test_get_schema_fields_host_function() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            let fields = get_schema_fields("TextNote");
            if fields.len() != 1 { throw "Expected 1 field, got " + fields.len(); }
            if fields[0].name != "body" { throw "Expected 'body', got " + fields[0].name; }
            if fields[0].type != "textarea" { throw "Expected 'textarea', got " + fields[0].type; }
            if fields[0].options.len() != 0 { throw "Expected options length 0, got " + fields[0].options.len(); }
            if fields[0].max != 0 { throw "Expected max 0, got " + fields[0].max; }
        "#).unwrap();
    }

    #[test]
    fn test_user_hooks_cleared_on_clear() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_user_script(r#"
            schema("Hooked", #{ fields: [#{ name: "x", type: "text" }] });
            on_save("Hooked", |note| { note });
        "#).unwrap();
        assert!(registry.hooks().has_hook("Hooked"));

        registry.clear_user_registrations();
        assert!(!registry.hooks().has_hook("Hooked"));
        // System hook should remain
        assert!(registry.hooks().has_hook("Contact"));
    }

    #[test]
    fn test_select_field_parses_options() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Ticket", #{
                fields: [
                    #{ name: "status", type: "select", options: ["TODO", "WIP", "DONE"], required: true }
                ]
            });
        "#).unwrap();
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
        "#).unwrap();
        let fields = get_schema_fields_for_test(&registry, "Review");
        assert_eq!(fields[0].max, 5);
    }

    #[test]
    fn test_regular_fields_have_empty_options_and_zero_max() {
        let registry = ScriptRegistry::new().unwrap();
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
        "#);
        assert!(result.is_err(), "non-string item in options should return a Scripting error");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("strings"), "error should mention 'strings', got: {msg}");
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
        "#);
        assert!(result.is_err(), "negative max should return a Scripting error");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("max"), "error should mention 'max', got: {msg}");
    }

}
