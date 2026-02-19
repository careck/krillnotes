//! Schema definitions and the private schema store for Krillnotes note types.

use crate::{FieldValue, KrillnotesError, Result};
use rhai::Map;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Describes a single typed field within a note schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldDefinition {
    pub name: String,
    pub field_type: String,
    pub required: bool,
}

/// A parsed note-type schema containing an ordered list of field definitions.
#[derive(Debug, Clone)]
pub struct Schema {
    pub name: String,
    pub fields: Vec<FieldDefinition>,
}

impl Schema {
    /// Returns a map of field names to their zero-value defaults.
    pub fn default_fields(&self) -> HashMap<String, FieldValue> {
        let mut fields = HashMap::new();
        for field_def in &self.fields {
            let default_value = match field_def.field_type.as_str() {
                "text" => FieldValue::Text(String::new()),
                "number" => FieldValue::Number(0.0),
                "boolean" => FieldValue::Boolean(false),
                "date" => FieldValue::Date(None),
                "email" => FieldValue::Email(String::new()),
                // Unknown types fall back to empty text; script validation catches typos.
                _ => FieldValue::Text(String::new()),
            };
            fields.insert(field_def.name.clone(), default_value);
        }
        fields
    }

    /// Parses a `Schema` from a Rhai object map produced by a `schema(...)` call.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError::Scripting`] if the map is malformed.
    pub(super) fn parse_from_rhai(name: &str, def: &Map) -> Result<Self> {
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

            fields.push(FieldDefinition { name: field_name, field_type, required });
        }

        Ok(Schema { name: name.to_string(), fields })
    }
}

/// Private store for registered schemas. No Rhai dependency.
pub(super) struct SchemaRegistry {
    schemas: Arc<Mutex<HashMap<String, Schema>>>,
}

impl SchemaRegistry {
    pub(super) fn new() -> Self {
        Self { schemas: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Returns a clone of the inner `Arc` so Rhai host-function closures can write into it.
    pub(super) fn schemas_arc(&self) -> Arc<Mutex<HashMap<String, Schema>>> {
        Arc::clone(&self.schemas)
    }

    pub(super) fn get(&self, name: &str) -> Result<Schema> {
        self.schemas
            .lock()
            .map_err(|_| KrillnotesError::Scripting("Schema registry lock poisoned".to_string()))?
            .get(name)
            .cloned()
            .ok_or_else(|| KrillnotesError::SchemaNotFound(name.to_string()))
    }

    pub(super) fn list(&self) -> Vec<String> {
        // SAFETY: mutex poisoning would require a panic while the lock is held,
        // which cannot happen in this codebase's single-threaded usage.
        self.schemas.lock().unwrap().keys().cloned().collect()
    }
}
