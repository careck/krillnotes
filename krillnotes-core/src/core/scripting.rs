//! Rhai-based schema registry for Krillnotes note types.
//!
//! Schemas are defined in `.rhai` scripts and loaded at workspace startup.
//! The [`SchemaRegistry`] keeps the Rhai [`Engine`] alive so that future
//! scripted views, commands, and action hooks can be evaluated at runtime.

use crate::{FieldValue, KrillnotesError, Result};
use chrono::NaiveDate;
use rhai::{Dynamic, Engine, FnPtr, Map, AST};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Describes a single typed field within a note schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldDefinition {
    /// The field's unique name within its schema.
    pub name: String,
    /// The field type: `"text"`, `"number"`, `"boolean"`, `"date"`, or `"email"`.
    pub field_type: String,
    /// Whether the field must carry a non-default value before the note is saved.
    pub required: bool,
}

/// A parsed note-type schema containing an ordered list of field definitions.
#[derive(Debug, Clone)]
pub struct Schema {
    /// The unique name of this schema (e.g. `"TextNote"`).
    pub name: String,
    /// Ordered field definitions that make up this schema.
    pub fields: Vec<FieldDefinition>,
}

impl Schema {
    /// Returns a map of field names to their zero-value defaults.
    ///
    /// Text fields default to `""`, numbers to `0.0`, booleans to `false`,
    /// dates to `Date(None)`, emails to `Email("")`.
    pub fn default_fields(&self) -> HashMap<String, FieldValue> {
        let mut fields = HashMap::new();
        for field_def in &self.fields {
            let default_value = match field_def.field_type.as_str() {
                "text" => FieldValue::Text(String::new()),
                "number" => FieldValue::Number(0.0),
                "boolean" => FieldValue::Boolean(false),
                "date" => FieldValue::Date(None),
                "email" => FieldValue::Email(String::new()),
                _ => FieldValue::Text(String::new()),
            };
            fields.insert(field_def.name.clone(), default_value);
        }
        fields
    }
}

/// A stored pre-save hook: the Rhai closure and the AST it was defined in.
// Fields are read by `run_on_save_hook` (Task 3). Remove this allow once that method exists.
#[allow(dead_code)]
#[derive(Clone, Debug)]
struct HookEntry {
    fn_ptr: FnPtr,
    ast: AST,
}

/// Registry of all note-type schemas loaded from Rhai scripts.
///
/// The Rhai [`Engine`] is kept alive as a field so that future scripted
/// views, commands, and action hooks can be evaluated at runtime without
/// reconstructing the engine from scratch.
#[derive(Debug)]
pub struct SchemaRegistry {
    engine: Engine,
    schemas: Arc<Mutex<HashMap<String, Schema>>>,
    hooks: Arc<Mutex<HashMap<String, HookEntry>>>,
    current_loading_ast: Arc<Mutex<Option<AST>>>,
}

impl SchemaRegistry {
    /// Creates a new registry and loads the built-in system schemas.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError::Scripting`] if the bundled system script
    /// fails to parse or if any `schema(...)` call within it is malformed.
    pub fn new() -> Result<Self> {
        let mut engine = Engine::new();
        let schemas = Arc::new(Mutex::new(HashMap::new()));
        let hooks: Arc<Mutex<HashMap<String, HookEntry>>> = Arc::new(Mutex::new(HashMap::new()));
        let current_loading_ast: Arc<Mutex<Option<AST>>> = Arc::new(Mutex::new(None));

        let schemas_clone = Arc::clone(&schemas);
        engine.register_fn("schema", move |name: String, def: Map| {
            let schema = Self::parse_schema(&name, &def).unwrap();
            schemas_clone.lock().unwrap().insert(name, schema);
        });

        let hooks_for_fn = Arc::clone(&hooks);
        let ast_for_fn = Arc::clone(&current_loading_ast);
        engine.register_fn("on_save", move |name: String, fn_ptr: FnPtr| {
            let ast = ast_for_fn
                .lock()
                .unwrap()
                .clone()
                .expect("on_save called outside of load_script");
            hooks_for_fn
                .lock()
                .unwrap()
                .insert(name, HookEntry { fn_ptr, ast });
        });

        let mut registry = Self {
            engine,
            schemas,
            hooks,
            current_loading_ast,
        };
        registry.load_script(include_str!("../system_scripts/text_note.rhai"))?;
        registry.load_script(include_str!("../system_scripts/contact.rhai"))?;

        Ok(registry)
    }

    /// Evaluates `script` and registers any schemas it defines via `schema(...)` calls.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError::Scripting`] if the script fails to evaluate.
    pub fn load_script(&mut self, script: &str) -> Result<()> {
        let ast = self
            .engine
            .compile(script)
            .map_err(|e| KrillnotesError::Scripting(e.to_string()))?;

        *self.current_loading_ast.lock().unwrap() = Some(ast.clone());

        let result = self
            .engine
            .eval_ast::<()>(&ast)
            .map_err(|e| KrillnotesError::Scripting(e.to_string()));

        *self.current_loading_ast.lock().unwrap() = None;

        result
    }

    /// Returns the schema registered under `name`.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError::SchemaNotFound`] if no schema with that
    /// name has been registered.
    pub fn get_schema(&self, name: &str) -> Result<Schema> {
        self.schemas
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or_else(|| KrillnotesError::SchemaNotFound(name.to_string()))
    }

    /// Returns the names of all currently registered schemas.
    pub fn list_schemas(&self) -> Vec<String> {
        self.schemas.lock().unwrap().keys().cloned().collect()
    }

    /// Returns the names of all currently registered schemas.
    ///
    /// This is an alias for [`list_schemas`](Self::list_schemas).
    pub fn list_types(&self) -> Result<Vec<String>> {
        Ok(self.schemas.lock().unwrap().keys().cloned().collect())
    }

    /// Returns `true` if a pre-save hook is registered for `schema_name`.
    pub fn has_hook(&self, schema_name: &str) -> bool {
        self.hooks.lock().unwrap().contains_key(schema_name)
    }

    fn parse_schema(name: &str, def: &Map) -> Result<Schema> {
        let fields_array = def
            .get("fields")
            .and_then(|v| v.clone().try_cast::<rhai::Array>())
            .ok_or_else(|| KrillnotesError::Scripting("Missing 'fields' array".to_string()))?;

        let mut fields = Vec::new();
        for field_item in fields_array {
            let field_map = field_item
                .try_cast::<Map>()
                .ok_or_else(|| KrillnotesError::Scripting("Field must be a map".to_string()))?;

            let field_name = field_map
                .get("name")
                .and_then(|v| v.clone().try_cast::<String>())
                .ok_or_else(|| KrillnotesError::Scripting("Field missing 'name'".to_string()))?;

            let field_type = field_map
                .get("type")
                .and_then(|v| v.clone().try_cast::<String>())
                .ok_or_else(|| KrillnotesError::Scripting("Field missing 'type'".to_string()))?;

            let required = field_map
                .get("required")
                .and_then(|v| v.clone().try_cast::<bool>())
                .unwrap_or(false);

            fields.push(FieldDefinition {
                name: field_name,
                field_type,
                required,
            });
        }

        Ok(Schema {
            name: name.to_string(),
            fields,
        })
    }
}

/// Converts a [`FieldValue`] to a Rhai [`Dynamic`] for passing into hook closures.
///
/// `Date(None)` maps to `Dynamic::UNIT` (`()`).
/// `Date(Some(d))` maps to an ISO 8601 string `"YYYY-MM-DD"`.
/// All other variants map to their natural Rhai primitive.
// Used by `run_on_save_hook` (Task 3). Remove this allow once that method exists.
#[allow(dead_code)]
fn field_value_to_dynamic(fv: &FieldValue) -> Dynamic {
    match fv {
        FieldValue::Text(s) => Dynamic::from(s.clone()),
        FieldValue::Number(n) => Dynamic::from(*n),
        FieldValue::Boolean(b) => Dynamic::from(*b),
        FieldValue::Date(None) => Dynamic::UNIT,
        FieldValue::Date(Some(d)) => Dynamic::from(d.format("%Y-%m-%d").to_string()),
        FieldValue::Email(s) => Dynamic::from(s.clone()),
    }
}

/// Converts a Rhai [`Dynamic`] back to a [`FieldValue`] given the field's type string.
///
/// Returns [`KrillnotesError::Scripting`] if the Dynamic value cannot be
/// converted to the expected Rust type.
// Used by `run_on_save_hook` (Task 3). Remove this allow once that method exists.
#[allow(dead_code)]
fn dynamic_to_field_value(d: Dynamic, field_type: &str) -> Result<FieldValue> {
    match field_type {
        "text" => {
            let s = d
                .try_cast::<String>()
                .ok_or_else(|| KrillnotesError::Scripting("text field must be a string".into()))?;
            Ok(FieldValue::Text(s))
        }
        "number" => {
            let n = d
                .try_cast::<f64>()
                .ok_or_else(|| KrillnotesError::Scripting("number field must be a float".into()))?;
            Ok(FieldValue::Number(n))
        }
        "boolean" => {
            let b = d
                .try_cast::<bool>()
                .ok_or_else(|| KrillnotesError::Scripting("boolean field must be a bool".into()))?;
            Ok(FieldValue::Boolean(b))
        }
        "date" => {
            if d.is_unit() {
                Ok(FieldValue::Date(None))
            } else {
                let s = d.try_cast::<String>().ok_or_else(|| {
                    KrillnotesError::Scripting("date field must be a string or ()".into())
                })?;
                let nd = NaiveDate::parse_from_str(&s, "%Y-%m-%d").map_err(|e| {
                    KrillnotesError::Scripting(format!("invalid date '{}': {}", s, e))
                })?;
                Ok(FieldValue::Date(Some(nd)))
            }
        }
        "email" => {
            let s = d
                .try_cast::<String>()
                .ok_or_else(|| KrillnotesError::Scripting("email field must be a string".into()))?;
            Ok(FieldValue::Email(s))
        }
        _ => Ok(FieldValue::Text(String::new())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_registration() {
        let mut registry = SchemaRegistry::new().unwrap();

        let script = r#"
            schema("TestNote", #{
                fields: [
                    #{ name: "body", type: "text", required: false },
                    #{ name: "count", type: "number", required: false },
                ]
            });
        "#;

        registry.load_script(script).unwrap();

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
        let registry = SchemaRegistry::new().unwrap();
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
        let registry = SchemaRegistry::new().unwrap();
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
        let mut registry = SchemaRegistry::new().unwrap();
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
        assert!(registry.has_hook("Widget"));
        assert!(!registry.has_hook("Missing"));
    }

    #[test]
    fn test_field_value_to_dynamic_text() {
        let d = field_value_to_dynamic(&FieldValue::Text("hello".into()));
        assert_eq!(d.try_cast::<String>().unwrap(), "hello");
    }

    #[test]
    fn test_field_value_to_dynamic_number() {
        let d = field_value_to_dynamic(&FieldValue::Number(3.14));
        assert!((d.try_cast::<f64>().unwrap() - 3.14).abs() < f64::EPSILON);
    }

    #[test]
    fn test_field_value_to_dynamic_boolean() {
        let d = field_value_to_dynamic(&FieldValue::Boolean(true));
        assert!(d.try_cast::<bool>().unwrap());
    }

    #[test]
    fn test_field_value_to_dynamic_date_none() {
        let d = field_value_to_dynamic(&FieldValue::Date(None));
        assert!(d.is_unit());
    }

    #[test]
    fn test_field_value_to_dynamic_date_some() {
        use chrono::NaiveDate;
        let date = NaiveDate::from_ymd_opt(2026, 2, 19).unwrap();
        let d = field_value_to_dynamic(&FieldValue::Date(Some(date)));
        assert_eq!(d.try_cast::<String>().unwrap(), "2026-02-19");
    }

    #[test]
    fn test_field_value_to_dynamic_email() {
        let d = field_value_to_dynamic(&FieldValue::Email("a@b.com".into()));
        assert_eq!(d.try_cast::<String>().unwrap(), "a@b.com");
    }

    #[test]
    fn test_dynamic_to_field_value_text() {
        let fv = dynamic_to_field_value(Dynamic::from("hi".to_string()), "text").unwrap();
        assert_eq!(fv, FieldValue::Text("hi".into()));
    }

    #[test]
    fn test_dynamic_to_field_value_number() {
        let fv = dynamic_to_field_value(Dynamic::from(2.0_f64), "number").unwrap();
        assert_eq!(fv, FieldValue::Number(2.0));
    }

    #[test]
    fn test_dynamic_to_field_value_boolean() {
        let fv = dynamic_to_field_value(Dynamic::from(false), "boolean").unwrap();
        assert_eq!(fv, FieldValue::Boolean(false));
    }

    #[test]
    fn test_dynamic_to_field_value_date_none() {
        let fv = dynamic_to_field_value(Dynamic::UNIT, "date").unwrap();
        assert_eq!(fv, FieldValue::Date(None));
    }

    #[test]
    fn test_dynamic_to_field_value_date_some() {
        use chrono::NaiveDate;
        let d = Dynamic::from("2026-02-19".to_string());
        let fv = dynamic_to_field_value(d, "date").unwrap();
        assert_eq!(fv, FieldValue::Date(Some(NaiveDate::from_ymd_opt(2026, 2, 19).unwrap())));
    }

    #[test]
    fn test_dynamic_to_field_value_email() {
        let fv = dynamic_to_field_value(Dynamic::from("x@y.com".to_string()), "email").unwrap();
        assert_eq!(fv, FieldValue::Email("x@y.com".into()));
    }
}
