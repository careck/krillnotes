//! Schema definitions and the private schema store for Krillnotes note types.

use crate::{FieldValue, KrillnotesError, Result};
use chrono::NaiveDate;
use rhai::{Dynamic, Engine, FnPtr, Map, AST};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A stored hook entry: the Rhai closure and the AST it was defined in.
#[derive(Clone, Debug)]
pub(super) struct HookEntry {
    pub(super) fn_ptr: FnPtr,
    pub(super) ast: AST,
    pub(super) script_name: String,
}

/// Result returned by [`SchemaRegistry::run_on_add_child_hook`].
///
/// Each field is `Some((new_title, new_fields))` when the hook returned
/// modifications for that note, or `None` when the hook left it unchanged.
#[derive(Debug)]
pub struct AddChildResult {
    pub parent: Option<(String, HashMap<String, FieldValue>)>,
    pub child:  Option<(String, HashMap<String, FieldValue>)>,
}

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
    /// Optional schema type filter for `note_link` fields.
    /// If set, the picker only shows notes of this type. Ignored for all other field types.
    #[serde(default)]
    pub target_type: Option<String>,
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
                Some(FieldValue::NoteLink(id)) => id.is_none(),
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
                "note_link" => FieldValue::NoteLink(None),
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

            let target_type: Option<String> = field_map
                .get("target_type")
                .and_then(|v| v.clone().try_cast::<String>());

            fields.push(FieldDefinition { name: field_name, field_type, required, can_view, can_edit, options, max, target_type });
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

/// Private store for registered schemas plus per-schema hook side-tables.
#[derive(Debug, Clone)]
pub(super) struct SchemaRegistry {
    schemas:            Arc<Mutex<HashMap<String, Schema>>>,
    on_save_hooks:      Arc<Mutex<HashMap<String, HookEntry>>>,
    on_view_hooks:      Arc<Mutex<HashMap<String, HookEntry>>>,
    on_add_child_hooks: Arc<Mutex<HashMap<String, HookEntry>>>,
}

impl SchemaRegistry {
    pub(super) fn new() -> Self {
        Self {
            schemas:            Arc::new(Mutex::new(HashMap::new())),
            on_save_hooks:      Arc::new(Mutex::new(HashMap::new())),
            on_view_hooks:      Arc::new(Mutex::new(HashMap::new())),
            on_add_child_hooks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Returns a clone of the inner `Arc` so Rhai host-function closures can write into it.
    pub(super) fn schemas_arc(&self) -> Arc<Mutex<HashMap<String, Schema>>> {
        Arc::clone(&self.schemas)
    }

    pub(super) fn on_save_hooks_arc(&self) -> Arc<Mutex<HashMap<String, HookEntry>>> {
        Arc::clone(&self.on_save_hooks)
    }

    pub(super) fn on_view_hooks_arc(&self) -> Arc<Mutex<HashMap<String, HookEntry>>> {
        Arc::clone(&self.on_view_hooks)
    }

    pub(super) fn on_add_child_hooks_arc(&self) -> Arc<Mutex<HashMap<String, HookEntry>>> {
        Arc::clone(&self.on_add_child_hooks)
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
        // SAFETY: mutex poisoning would require a panic while the lock is held,
        // which cannot happen in this codebase's single-threaded usage.
        self.schemas.lock().unwrap().contains_key(name)
    }

    pub(super) fn list(&self) -> Vec<String> {
        // SAFETY: mutex poisoning would require a panic while the lock is held,
        // which cannot happen in this codebase's single-threaded usage.
        self.schemas.lock().unwrap().keys().cloned().collect()
    }

    pub(super) fn all(&self) -> HashMap<String, Schema> {
        // SAFETY: mutex poisoning would require a panic while the lock is held,
        // which cannot happen in this codebase's single-threaded usage.
        self.schemas.lock().unwrap().clone()
    }

    pub(super) fn clear(&self) {
        // SAFETY: mutex poisoning would require a panic while the lock is held,
        // which cannot happen in this codebase's single-threaded usage.
        self.schemas.lock().unwrap().clear();
        self.on_save_hooks.lock().unwrap().clear();
        self.on_view_hooks.lock().unwrap().clear();
        self.on_add_child_hooks.lock().unwrap().clear();
    }

    /// Returns `true` if an on_save hook is registered for `schema_name`.
    pub(super) fn has_hook(&self, schema_name: &str) -> bool {
        // SAFETY: mutex poisoning would require a panic while the lock is held,
        // which cannot happen in this codebase's single-threaded usage.
        self.on_save_hooks.lock().unwrap().contains_key(schema_name)
    }

    /// Returns `true` if an on_view hook is registered for `schema_name`.
    pub(super) fn has_view_hook(&self, schema_name: &str) -> bool {
        // SAFETY: mutex poisoning would require a panic while the lock is held,
        // which cannot happen in this codebase's single-threaded usage.
        self.on_view_hooks.lock().unwrap().contains_key(schema_name)
    }

    /// Runs the on_save hook for `schema_name`, if registered.
    ///
    /// Called from [`ScriptRegistry::run_on_save_hook`](super::ScriptRegistry::run_on_save_hook).
    pub(super) fn run_on_save_hook(
        &self,
        engine: &Engine,
        schema: &Schema,
        note_id: &str,
        node_type: &str,
        title: &str,
        fields: &HashMap<String, FieldValue>,
    ) -> Result<Option<(String, HashMap<String, FieldValue>)>> {
        let entry = {
            let hooks = self.on_save_hooks
                .lock()
                .map_err(|_| KrillnotesError::Scripting("on_save hook lock poisoned".to_string()))?;
            hooks.get(&schema.name).cloned()
        };
        let entry = match entry {
            Some(e) => e,
            None => return Ok(None),
        };

        let mut fields_map = Map::new();
        for (k, v) in fields {
            fields_map.insert(k.as_str().into(), field_value_to_dynamic(v));
        }
        let mut note_map = Map::new();
        note_map.insert("id".into(),        Dynamic::from(note_id.to_string()));
        note_map.insert("node_type".into(), Dynamic::from(node_type.to_string()));
        note_map.insert("title".into(),     Dynamic::from(title.to_string()));
        note_map.insert("fields".into(),    Dynamic::from(fields_map));

        let result = entry
            .fn_ptr
            .call::<Dynamic>(engine, &entry.ast, (Dynamic::from(note_map),))
            .map_err(|e| KrillnotesError::Scripting(format!("on_save hook error in '{}': {e}", entry.script_name)))?;

        let result_map = result.try_cast::<Map>().ok_or_else(|| {
            KrillnotesError::Scripting("on_save hook must return the note map".to_string())
        })?;

        let new_title = result_map
            .get("title")
            .and_then(|v| v.clone().try_cast::<String>())
            .ok_or_else(|| KrillnotesError::Scripting("hook result 'title' must be a string".to_string()))?;

        let new_fields_dyn = result_map
            .get("fields")
            .and_then(|v| v.clone().try_cast::<Map>())
            .ok_or_else(|| KrillnotesError::Scripting("hook result 'fields' must be a map".to_string()))?;

        let mut new_fields = HashMap::new();
        for field_def in &schema.fields {
            let dyn_val = new_fields_dyn
                .get(field_def.name.as_str())
                .cloned()
                .unwrap_or(Dynamic::UNIT);
            let fv = dynamic_to_field_value(dyn_val, &field_def.field_type).map_err(|e| {
                KrillnotesError::Scripting(format!("field '{}': {}", field_def.name, e))
            })?;
            new_fields.insert(field_def.name.clone(), fv);
        }

        Ok(Some((new_title, new_fields)))
    }

    /// Runs the on_view hook for `schema_name`, if registered.
    ///
    /// Called from [`ScriptRegistry::run_on_view_hook`](super::ScriptRegistry::run_on_view_hook).
    pub(super) fn run_on_view_hook(
        &self,
        engine: &Engine,
        note_map: Map,
    ) -> Result<Option<String>> {
        let schema_name = note_map
            .get("node_type")
            .and_then(|v| v.clone().try_cast::<String>())
            .unwrap_or_default();

        let entry = {
            let hooks = self.on_view_hooks
                .lock()
                .map_err(|_| KrillnotesError::Scripting("on_view hook lock poisoned".to_string()))?;
            hooks.get(&schema_name).cloned()
        };
        let entry = match entry {
            Some(e) => e,
            None => return Ok(None),
        };

        let result = entry
            .fn_ptr
            .call::<Dynamic>(engine, &entry.ast, (Dynamic::from(note_map),))
            .map_err(|e| KrillnotesError::Scripting(format!("on_view hook error in '{}': {e}", entry.script_name)))?;

        let html = result.try_cast::<String>().ok_or_else(|| {
            KrillnotesError::Scripting("on_view hook must return a string".to_string())
        })?;

        Ok(Some(html))
    }

    /// Runs the on_add_child hook for `parent_schema`, if registered.
    ///
    /// Called from [`ScriptRegistry::run_on_add_child_hook`](super::ScriptRegistry::run_on_add_child_hook).
    ///
    /// Returns `Ok(None)` when no hook is registered for the parent schema.
    /// Returns `Ok(Some(AddChildResult))` with optional parent/child updates on success.
    pub(super) fn run_on_add_child_hook(
        &self,
        engine: &Engine,
        parent_schema: &Schema,
        parent_id: &str,
        parent_type: &str,
        parent_title: &str,
        parent_fields: &HashMap<String, FieldValue>,
        child_schema: &Schema,
        child_id: &str,
        child_type: &str,
        child_title: &str,
        child_fields: &HashMap<String, FieldValue>,
    ) -> Result<Option<AddChildResult>> {
        let entry = {
            let hooks = self.on_add_child_hooks
                .lock()
                .map_err(|_| KrillnotesError::Scripting("on_add_child hook lock poisoned".to_string()))?;
            hooks.get(&parent_schema.name).cloned()
        };
        let entry = match entry {
            Some(e) => e,
            None => return Ok(None),
        };

        // Note: on_add_child maps intentionally omit `tags`; on_view maps include them.
        // Build parent note map
        let mut p_fields_map = Map::new();
        for (k, v) in parent_fields {
            p_fields_map.insert(k.as_str().into(), field_value_to_dynamic(v));
        }
        let mut parent_map = Map::new();
        parent_map.insert("id".into(),        Dynamic::from(parent_id.to_string()));
        parent_map.insert("node_type".into(), Dynamic::from(parent_type.to_string()));
        parent_map.insert("title".into(),     Dynamic::from(parent_title.to_string()));
        parent_map.insert("fields".into(),    Dynamic::from(p_fields_map));

        // Build child note map
        let mut c_fields_map = Map::new();
        for (k, v) in child_fields {
            c_fields_map.insert(k.as_str().into(), field_value_to_dynamic(v));
        }
        let mut child_map = Map::new();
        child_map.insert("id".into(),        Dynamic::from(child_id.to_string()));
        child_map.insert("node_type".into(), Dynamic::from(child_type.to_string()));
        child_map.insert("title".into(),     Dynamic::from(child_title.to_string()));
        child_map.insert("fields".into(),    Dynamic::from(c_fields_map));

        let result = entry
            .fn_ptr
            .call::<Dynamic>(engine, &entry.ast, (Dynamic::from(parent_map), Dynamic::from(child_map)))
            .map_err(|e| KrillnotesError::Scripting(
                format!("on_add_child hook error in '{}': {e}", entry.script_name)
            ))?;

        // If the hook returned unit (no-op), treat as no modification
        if result.is_unit() {
            return Ok(Some(AddChildResult { parent: None, child: None }));
        }

        let result_map = result.try_cast::<Map>().ok_or_else(|| {
            KrillnotesError::Scripting(
                "on_add_child hook must return a map #{ parent: ..., child: ... } or ()".to_string()
            )
        })?;

        // Extract optional parent modifications
        let parent_update = if let Some(pm) = result_map.get("parent").and_then(|v| v.clone().try_cast::<Map>()) {
            let new_title = pm.get("title")
                .and_then(|v| v.clone().try_cast::<String>())
                .ok_or_else(|| KrillnotesError::Scripting("hook result parent 'title' must be a string".to_string()))?;
            let new_fields_dyn = pm.get("fields")
                .and_then(|v| v.clone().try_cast::<Map>())
                .ok_or_else(|| KrillnotesError::Scripting("hook result parent 'fields' must be a map".to_string()))?;
            let mut new_fields = HashMap::new();
            for field_def in &parent_schema.fields {
                let dyn_val = new_fields_dyn.get(field_def.name.as_str()).cloned().unwrap_or(Dynamic::UNIT);
                let fv = dynamic_to_field_value(dyn_val, &field_def.field_type)
                    .map_err(|e| KrillnotesError::Scripting(format!("parent field '{}': {e}", field_def.name)))?;
                new_fields.insert(field_def.name.clone(), fv);
            }
            Some((new_title, new_fields))
        } else {
            None
        };

        // Extract optional child modifications
        let child_update = if let Some(cm) = result_map.get("child").and_then(|v| v.clone().try_cast::<Map>()) {
            let new_title = cm.get("title")
                .and_then(|v| v.clone().try_cast::<String>())
                .ok_or_else(|| KrillnotesError::Scripting("hook result child 'title' must be a string".to_string()))?;
            let new_fields_dyn = cm.get("fields")
                .and_then(|v| v.clone().try_cast::<Map>())
                .ok_or_else(|| KrillnotesError::Scripting("hook result child 'fields' must be a map".to_string()))?;
            let mut new_fields = HashMap::new();
            for field_def in &child_schema.fields {
                let dyn_val = new_fields_dyn.get(field_def.name.as_str()).cloned().unwrap_or(Dynamic::UNIT);
                let fv = dynamic_to_field_value(dyn_val, &field_def.field_type)
                    .map_err(|e| KrillnotesError::Scripting(format!("child field '{}': {e}", field_def.name)))?;
                new_fields.insert(field_def.name.clone(), fv);
            }
            Some((new_title, new_fields))
        } else {
            None
        };

        Ok(Some(AddChildResult { parent: parent_update, child: child_update }))
    }
}

/// Converts a [`FieldValue`] to a Rhai [`Dynamic`] for passing into hook closures.
///
/// `Date(None)` maps to `Dynamic::UNIT` (`()`).
/// `Date(Some(d))` maps to an ISO 8601 string `"YYYY-MM-DD"`.
/// All other variants map to their natural Rhai primitive.
pub(crate) fn field_value_to_dynamic(fv: &FieldValue) -> Dynamic {
    match fv {
        FieldValue::Text(s) => Dynamic::from(s.clone()),
        FieldValue::Number(n) => Dynamic::from(*n),
        FieldValue::Boolean(b) => Dynamic::from(*b),
        FieldValue::Date(None) => Dynamic::UNIT,
        FieldValue::Date(Some(d)) => Dynamic::from(d.format("%Y-%m-%d").to_string()),
        FieldValue::Email(s) => Dynamic::from(s.clone()),
        FieldValue::NoteLink(None) => Dynamic::UNIT,
        FieldValue::NoteLink(Some(id)) => Dynamic::from(id.clone()),
    }
}

/// Converts a Rhai [`Dynamic`] back to a [`FieldValue`] given the field's type string.
///
/// Returns [`KrillnotesError::Scripting`] if the Dynamic value cannot be
/// converted to the expected Rust type.
pub(super) fn dynamic_to_field_value(d: Dynamic, field_type: &str) -> Result<FieldValue> {
    match field_type {
        "text" | "textarea" => {
            if d.is_unit() {
                return Ok(FieldValue::Text(String::new()));
            }
            let s = d
                .try_cast::<String>()
                .ok_or_else(|| KrillnotesError::Scripting("text field must be a string".into()))?;
            Ok(FieldValue::Text(s))
        }
        "number" => {
            if d.is_unit() {
                return Ok(FieldValue::Number(0.0));
            }
            let n = d
                .try_cast::<f64>()
                .ok_or_else(|| KrillnotesError::Scripting("number field must be a float".into()))?;
            Ok(FieldValue::Number(n))
        }
        "boolean" => {
            if d.is_unit() {
                return Ok(FieldValue::Boolean(false));
            }
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
                    KrillnotesError::Scripting(format!("invalid date '{s}': {e}"))
                })?;
                Ok(FieldValue::Date(Some(nd)))
            }
        }
        "email" => {
            if d.is_unit() {
                return Ok(FieldValue::Email(String::new()));
            }
            let s = d
                .try_cast::<String>()
                .ok_or_else(|| KrillnotesError::Scripting("email field must be a string".into()))?;
            Ok(FieldValue::Email(s))
        }
        "select" => {
            if d.is_unit() {
                return Ok(FieldValue::Text(String::new()));
            }
            let s = d
                .try_cast::<String>()
                .ok_or_else(|| KrillnotesError::Scripting("select field must be a string".into()))?;
            Ok(FieldValue::Text(s))
        }
        "rating" => {
            if d.is_unit() {
                return Ok(FieldValue::Number(0.0));
            }
            let n = d
                .try_cast::<f64>()
                .ok_or_else(|| KrillnotesError::Scripting("rating field must be a float".into()))?;
            Ok(FieldValue::Number(n))
        }
        "note_link" => {
            if d.is_unit() {
                return Ok(FieldValue::NoteLink(None));
            }
            let s = d
                .try_cast::<String>()
                .ok_or_else(|| KrillnotesError::Scripting("note_link field must be a string or ()".into()))?;
            if s.is_empty() {
                return Ok(FieldValue::NoteLink(None));
            }
            Ok(FieldValue::NoteLink(Some(s)))
        }
        _ => Ok(FieldValue::Text(String::new())),
    }
}
