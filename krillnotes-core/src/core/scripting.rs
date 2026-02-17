use crate::{FieldValue, KrillnotesError, Result};
use rhai::{Engine, Map};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct FieldDefinition {
    pub name: String,
    pub field_type: String,
    pub required: bool,
}

#[derive(Debug, Clone)]
pub struct Schema {
    pub name: String,
    pub fields: Vec<FieldDefinition>,
}

impl Schema {
    pub fn default_fields(&self) -> HashMap<String, FieldValue> {
        let mut fields = HashMap::new();
        for field_def in &self.fields {
            let default_value = match field_def.field_type.as_str() {
                "text" => FieldValue::Text(String::new()),
                "number" => FieldValue::Number(0.0),
                "boolean" => FieldValue::Boolean(false),
                _ => FieldValue::Text(String::new()),
            };
            fields.insert(field_def.name.clone(), default_value);
        }
        fields
    }
}

pub struct SchemaRegistry {
    engine: Engine,
    schemas: Arc<Mutex<HashMap<String, Schema>>>,
}

impl SchemaRegistry {
    pub fn new() -> Result<Self> {
        let mut engine = Engine::new();
        let schemas = Arc::new(Mutex::new(HashMap::new()));

        let schemas_clone = Arc::clone(&schemas);
        engine.register_fn("schema", move |name: String, def: Map| {
            let schema = Self::parse_schema(&name, &def).unwrap();
            schemas_clone.lock().unwrap().insert(name, schema);
        });

        let mut registry = Self { engine, schemas };

        // Load system scripts
        registry.load_script(include_str!("../../../system_scripts/text_note.rhai"))?;

        Ok(registry)
    }

    pub fn load_script(&mut self, script: &str) -> Result<()> {
        self.engine
            .eval::<()>(script)
            .map_err(|e| KrillnotesError::Scripting(e.to_string()))?;
        Ok(())
    }

    pub fn get_schema(&self, name: &str) -> Result<Schema> {
        self.schemas
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or_else(|| KrillnotesError::SchemaNotFound(name.to_string()))
    }

    pub fn list_schemas(&self) -> Vec<String> {
        self.schemas.lock().unwrap().keys().cloned().collect()
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
}
