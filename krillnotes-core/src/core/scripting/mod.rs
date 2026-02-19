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
        engine.register_fn("schema", move |name: String, def: rhai::Map| -> std::result::Result<Dynamic, Box<EvalAltResult>> {
            let s = Schema::parse_from_rhai(&name, &def)
                .map_err(|e| -> Box<EvalAltResult> { e.to_string().into() })?;
            // SAFETY: mutex poisoning would require a panic while the lock is held,
            // which cannot happen in this codebase's single-threaded usage.
            schemas_arc.lock().unwrap().insert(name, s);
            Ok(Dynamic::UNIT)
        });

        // Register on_save() host function — writes into HookRegistry.
        let hooks_arc = hook_registry.on_save_hooks_arc();
        let ast_arc = Arc::clone(&current_loading_ast);
        engine.register_fn("on_save", move |name: String, fn_ptr: FnPtr| {
            let maybe_ast = ast_arc.lock().unwrap().clone();
            let ast = maybe_ast.expect("on_save called outside of load_script");
            hooks_arc
                .lock()
                .unwrap()
                .insert(name, HookEntry { fn_ptr, ast });
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
                },
                FieldDefinition {
                    name: "count".to_string(),
                    field_type: "number".to_string(),
                    required: false,
                },
            ],
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
        assert_eq!(schema.fields[0].field_type, "text");
    }

    #[test]
    fn test_date_field_default() {
        let schema = Schema {
            name: "Test".to_string(),
            fields: vec![FieldDefinition {
                name: "birthday".to_string(),
                field_type: "date".to_string(),
                required: false,
            }],
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
            }],
        };
        let defaults = schema.default_fields();
        assert!(matches!(defaults.get("email_addr"), Some(FieldValue::Email(s)) if s.is_empty()));
    }

    #[test]
    fn test_contact_schema_loaded() {
        let registry = ScriptRegistry::new().unwrap();
        let schema = registry.get_schema("Contact").unwrap();
        assert_eq!(schema.name, "Contact");
        assert_eq!(schema.fields.len(), 11);
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
    fn test_hook_registered_via_on_save() {
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

        let result = registry
            .run_on_save_hook("Contact", "id-1", "Contact", "", &fields)
            .unwrap()
            .unwrap();

        assert_eq!(result.0, "Smith, Jane");
    }
}
