//! Hook storage and execution for Krillnotes scripting events.

use super::schema::Schema;
use crate::{FieldValue, KrillnotesError, Result};
use chrono::NaiveDate;
use rhai::{Dynamic, Engine, FnPtr, Map, AST};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A stored pre-save hook: the Rhai closure and the AST it was defined in.
#[derive(Clone, Debug)]
pub(super) struct HookEntry {
    pub(super) fn_ptr: FnPtr,
    pub(super) ast: AST,
}

/// Public registry of event hooks loaded from Rhai scripts.
///
/// Execution methods accept a `&Engine` from the caller ([`ScriptRegistry`])
/// rather than owning one, keeping this type free of Rhai engine lifecycle concerns.
///
/// [`ScriptRegistry`]: super::ScriptRegistry
#[derive(Debug)]
pub struct HookRegistry {
    on_save_hooks: Arc<Mutex<HashMap<String, HookEntry>>>,
    /// Hook names registered by user scripts, so they can be cleared on reload.
    user_hooks: Arc<Mutex<Vec<String>>>,
}

impl HookRegistry {
    pub(super) fn new() -> Self {
        Self {
            on_save_hooks: Arc::new(Mutex::new(HashMap::new())),
            user_hooks: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Returns a clone of the inner `Arc` so Rhai host-function closures can write into it.
    pub(super) fn on_save_hooks_arc(&self) -> Arc<Mutex<HashMap<String, HookEntry>>> {
        Arc::clone(&self.on_save_hooks)
    }

    pub(super) fn user_hooks_arc(&self) -> Arc<Mutex<Vec<String>>> {
        Arc::clone(&self.user_hooks)
    }

    /// Removes all hooks that were registered by user scripts.
    pub(super) fn clear_user(&self) {
        let user_names: Vec<String> = self.user_hooks.lock().unwrap().drain(..).collect();
        let mut hooks = self.on_save_hooks.lock().unwrap();
        for name in user_names {
            hooks.remove(&name);
        }
    }

    /// Returns `true` if a pre-save hook is registered for `schema_name`.
    pub fn has_hook(&self, schema_name: &str) -> bool {
        // SAFETY: mutex poisoning would require a panic while the lock is held,
        // which cannot happen in this codebase's single-threaded usage.
        self.on_save_hooks.lock().unwrap().contains_key(schema_name)
    }

    /// Runs the pre-save hook registered for `schema_name`, if any.
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
        engine: &Engine,
        schema: &Schema,
        note_id: &str,
        node_type: &str,
        title: &str,
        fields: &HashMap<String, FieldValue>,
    ) -> Result<Option<(String, HashMap<String, FieldValue>)>> {
        // Clone the entry out of the mutex so the lock is not held during the call.
        let entry = {
            let hooks = self.on_save_hooks
                .lock()
                .map_err(|_| KrillnotesError::Scripting("on_save hook registry lock poisoned".to_string()))?;
            hooks.get(&schema.name).cloned()
        };
        let entry = match entry {
            Some(e) => e,
            None => return Ok(None),
        };

        // Build the fields sub-map.
        let mut fields_map = Map::new();
        for (k, v) in fields {
            fields_map.insert(k.as_str().into(), field_value_to_dynamic(v));
        }

        // Build the top-level note map.
        let mut note_map = Map::new();
        note_map.insert("id".into(), Dynamic::from(note_id.to_string()));
        note_map.insert("node_type".into(), Dynamic::from(node_type.to_string()));
        note_map.insert("title".into(), Dynamic::from(title.to_string()));
        note_map.insert("fields".into(), Dynamic::from(fields_map));

        // Call the closure.
        let result = entry
            .fn_ptr
            .call::<Dynamic>(engine, &entry.ast, (Dynamic::from(note_map),))
            .map_err(|e| KrillnotesError::Scripting(format!("on_save hook error: {e}")))?;

        // Parse the returned map.
        let result_map = result.try_cast::<Map>().ok_or_else(|| {
            KrillnotesError::Scripting("on_save hook must return the note map".to_string())
        })?;

        let new_title = result_map
            .get("title")
            .and_then(|v| v.clone().try_cast::<String>())
            .ok_or_else(|| {
                KrillnotesError::Scripting("hook result 'title' must be a string".to_string())
            })?;

        let new_fields_dyn = result_map
            .get("fields")
            .and_then(|v| v.clone().try_cast::<Map>())
            .ok_or_else(|| {
                KrillnotesError::Scripting("hook result 'fields' must be a map".to_string())
            })?;

        // Convert each field back, guided by the schema's type definitions.
        // Fields present in the schema but absent from the hook result are passed
        // as Dynamic::UNIT to dynamic_to_field_value, yielding the type's zero value.
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
}

/// Converts a [`FieldValue`] to a Rhai [`Dynamic`] for passing into hook closures.
///
/// `Date(None)` maps to `Dynamic::UNIT` (`()`).
/// `Date(Some(d))` maps to an ISO 8601 string `"YYYY-MM-DD"`.
/// All other variants map to their natural Rhai primitive.
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
fn dynamic_to_field_value(d: Dynamic, field_type: &str) -> Result<FieldValue> {
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
        _ => Ok(FieldValue::Text(String::new())),
    }
}
