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
    pub can_view: bool,
    pub can_edit: bool,
    /// Non-empty only for `select` fields — the list of allowed option strings.
    #[serde(default)]
    pub options: Vec<String>,
    /// Non-zero only for `rating` fields — the maximum star count.
    #[serde(default)]
    pub max: i64,
}

/// A parsed note-type schema containing an ordered list of field definitions.
#[derive(Debug, Clone)]
pub struct Schema {
    pub name: String,
    pub fields: Vec<FieldDefinition>,
    pub title_can_view: bool,
    pub title_can_edit: bool,
    pub children_sort: String,
    /// Note types that are allowed as parents of this note type.
    /// Empty means no restriction (any parent or root is allowed).
    pub allowed_parent_types: Vec<String>,
    /// Note types that this schema allows as direct children.
    /// Empty means no restriction (any child type is allowed here).
    pub allowed_children_types: Vec<String>,
}

impl Schema {
    /// Checks that all fields marked `required: true` have non-empty values.
    ///
    /// "Empty" means:
    /// - `Text` / `Email`: the string is `""`
    /// - `Date`: the value is `None`
    /// - `Number` / `Boolean`: always considered non-empty
    ///
    /// Returns `Ok(())` when all required fields are satisfied.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError::ValidationFailed`] for the first required
    /// field that is empty, naming the field in the error message.
    pub fn validate_required_fields(&self, fields: &HashMap<String, FieldValue>) -> crate::Result<()> {
        for field_def in &self.fields {
            if !field_def.required {
                continue;
            }
            let empty = match fields.get(&field_def.name) {
                Some(FieldValue::Text(s)) => s.is_empty(),
                Some(FieldValue::Email(s)) => s.is_empty(),
                Some(FieldValue::Date(d)) => d.is_none(),
                Some(FieldValue::Number(_) | FieldValue::Boolean(_)) => false,
                None => true,
            };
            if empty {
                return Err(KrillnotesError::ValidationFailed(format!(
                    "Required field '{}' must not be empty",
                    field_def.name
                )));
            }
        }
        Ok(())
    }

    /// Returns a map of field names to their zero-value defaults.
    pub fn default_fields(&self) -> HashMap<String, FieldValue> {
        let mut fields = HashMap::new();
        for field_def in &self.fields {
            let default_value = match field_def.field_type.as_str() {
                "text" | "textarea" => FieldValue::Text(String::new()),
                "number" => FieldValue::Number(0.0),
                "boolean" => FieldValue::Boolean(false),
                "date" => FieldValue::Date(None),
                "email" => FieldValue::Email(String::new()),
                "select" => FieldValue::Text(String::new()),
                "rating" => FieldValue::Number(0.0),
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

            let can_view = field_map
                .get("can_view")
                .and_then(|v| v.clone().try_cast::<bool>())
                .unwrap_or(true);

            let can_edit = field_map
                .get("can_edit")
                .and_then(|v| v.clone().try_cast::<bool>())
                .unwrap_or(true);

            let mut options: Vec<String> = Vec::new();
            if let Some(arr) = field_map
                .get("options")
                .and_then(|v| v.clone().try_cast::<rhai::Array>())
            {
                for item in arr {
                    let s = item.try_cast::<String>().ok_or_else(|| {
                        KrillnotesError::Scripting("options array must contain only strings".into())
                    })?;
                    options.push(s);
                }
            }

            let max: i64 = field_map
                .get("max")
                .and_then(|v| v.clone().try_cast::<i64>())
                .unwrap_or(0);

            if max < 0 {
                return Err(KrillnotesError::Scripting(
                    format!("field '{}': max must be >= 0, got {}", field_name, max)
                ));
            }

            fields.push(FieldDefinition { name: field_name, field_type, required, can_view, can_edit, options, max });
        }

        let title_can_view = def
            .get("title_can_view")
            .and_then(|v| v.clone().try_cast::<bool>())
            .unwrap_or(true);

        let title_can_edit = def
            .get("title_can_edit")
            .and_then(|v| v.clone().try_cast::<bool>())
            .unwrap_or(true);

        let children_sort = def
            .get("children_sort")
            .and_then(|v| v.clone().try_cast::<String>())
            .unwrap_or_else(|| "none".to_string());

        let mut allowed_parent_types: Vec<String> = Vec::new();
        if let Some(arr) = def
            .get("allowed_parent_types")
            .and_then(|v| v.clone().try_cast::<rhai::Array>())
        {
            for item in arr {
                let s = item.try_cast::<String>().ok_or_else(|| {
                    KrillnotesError::Scripting("allowed_parent_types must contain only strings".into())
                })?;
                allowed_parent_types.push(s);
            }
        }

        let mut allowed_children_types: Vec<String> = Vec::new();
        if let Some(arr) = def
            .get("allowed_children_types")
            .and_then(|v| v.clone().try_cast::<rhai::Array>())
        {
            for item in arr {
                let s = item.try_cast::<String>().ok_or_else(|| {
                    KrillnotesError::Scripting("allowed_children_types must contain only strings".into())
                })?;
                allowed_children_types.push(s);
            }
        }

        Ok(Schema { name: name.to_string(), fields, title_can_view, title_can_edit, children_sort, allowed_parent_types, allowed_children_types })
    }
}

/// Private store for registered schemas. No Rhai dependency.
#[derive(Debug)]
pub(super) struct SchemaRegistry {
    schemas: Arc<Mutex<HashMap<String, Schema>>>,
}

impl SchemaRegistry {
    pub(super) fn new() -> Self {
        Self {
            schemas: Arc::new(Mutex::new(HashMap::new())),
        }
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

    pub(super) fn exists(&self, name: &str) -> bool {
        self.schemas.lock().unwrap().contains_key(name)
    }

    pub(super) fn list(&self) -> Vec<String> {
        self.schemas.lock().unwrap().keys().cloned().collect()
    }

    pub(super) fn all(&self) -> HashMap<String, Schema> {
        self.schemas.lock().unwrap().clone()
    }

    /// Removes all registered schemas.
    pub(super) fn clear(&self) {
        self.schemas.lock().unwrap().clear();
    }
}
