// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Rhai-based scripting registry for Krillnotes note types and hooks.
//!
//! [`ScriptRegistry`] is the public entry point. It owns the Rhai [`Engine`],
//! loads scripts, and delegates schema and hook concerns to internal sub-registries.

pub(crate) mod display_helpers;
mod hooks;
mod schema;

pub(crate) use schema::field_value_to_dynamic;
pub use schema::{AddChildResult, FieldDefinition, FieldGroup, Schema, ViewRegistration, ScriptWarning};
use schema::{DeferredBinding, BindingKind};

use crate::{FieldValue, KrillnotesError, Note, Result};
use crate::core::attachment::AttachmentMeta;
use crate::core::save_transaction::SaveTransaction;
use schema::HookEntry;
use chrono::Local;
use include_dir::{include_dir, Dir};
use rhai::{Dynamic, Engine, EvalAltResult, FnPtr, Map, AST};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

// ── Thread-local SaveTransaction for Rhai write-path hooks ────────────────────

thread_local! {
    static SAVE_TX: RefCell<Option<SaveTransaction>> = RefCell::new(None);
}

/// Sets the active [`SaveTransaction`] for the current thread.
///
/// Called by hook runners before invoking any Rhai write-path closure.
pub(crate) fn set_save_tx(tx: SaveTransaction) {
    SAVE_TX.with(|cell| *cell.borrow_mut() = Some(tx));
}

/// Takes the active [`SaveTransaction`] from the current thread.
///
/// Called by hook runners after the Rhai closure returns, to retrieve results.
pub(crate) fn take_save_tx() -> Option<SaveTransaction> {
    SAVE_TX.with(|cell| cell.borrow_mut().take())
}

/// Converts a [`PendingNote`](crate::core::save_transaction::PendingNote) to a Rhai map
/// with the same shape as the note maps passed to hook callbacks.
///
/// Used by `get_children` and `get_note` to expose in-flight new notes to scripts.
fn pending_note_to_dynamic(pending: &crate::core::save_transaction::PendingNote) -> Dynamic {
    let mut fields_map = rhai::Map::new();
    for (k, v) in pending.effective_fields() {
        fields_map.insert(k.as_str().into(), schema::field_value_to_dynamic(&v));
    }
    let mut note_map = rhai::Map::new();
    note_map.insert("id".into(),        Dynamic::from(pending.note_id.clone()));
    note_map.insert("schema".into(), Dynamic::from(pending.schema.clone()));
    note_map.insert("title".into(),     Dynamic::from(pending.effective_title().to_string()));
    note_map.insert("fields".into(),    Dynamic::from(fields_map));
    note_map.insert("tags".into(),      Dynamic::from(rhai::Array::new()));
    Dynamic::from_map(note_map)
}

/// Accesses the active [`SaveTransaction`] for the current thread.
///
/// Used internally by the registered Rhai native functions (`set_field`, etc.).
fn with_save_tx<F, R>(f: F) -> std::result::Result<R, Box<EvalAltResult>>
where
    F: FnOnce(&mut SaveTransaction) -> std::result::Result<R, String>,
{
    SAVE_TX.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let tx = borrow.as_mut().ok_or_else(|| -> Box<EvalAltResult> {
            "set_field/set_title/reject/commit called outside a write context".to_string().into()
        })?;
        f(tx).map_err(|e| -> Box<EvalAltResult> { e.into() })
    })
}

/// Per-run context injected before executing a Rhai script so that
/// context-aware Rhai helpers (markdown, display_image, etc.) can resolve
/// attachment references for the current note.
#[derive(Debug)]
pub struct NoteRunContext {
    pub note: crate::core::note::Note,
    pub attachments: Vec<AttachmentMeta>,
}

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
    /// Maps each tag to all notes carrying that tag (pre-built for O(1) look-up).
    pub notes_by_tag:   HashMap<String, Vec<Dynamic>>,
    /// Maps each target note ID to all source notes that link to it
    /// via a `note_link` field (pre-built for O(1) look-up).
    pub notes_by_link_target: HashMap<String, Vec<Dynamic>>,
    /// Maps each note ID to its attachments, pre-built for O(1) script-time look-up.
    pub attachments_by_note_id: HashMap<String, Vec<AttachmentMeta>>,
}

static STARTER_SCRIPTS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/system_scripts");

/// A bundled starter script with its filename and source code.
pub struct StarterScript {
    /// The filename (e.g. `"00_text_note.rhai"`), used to derive load order.
    pub filename: String,
    /// The full Rhai source code.
    pub source_code: String,
}

/// An error that occurred while loading a user script during a full registry reload.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScriptError {
    /// The `name` field of the script that failed.
    pub script_name: String,
    /// The Rhai error or collision message.
    pub message: String,
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
    current_loading_category: Arc<Mutex<Option<String>>>,
    /// Source code of all successfully loaded library/presentation scripts.
    /// Schema scripts are compiled together with this library source so they can call library
    /// helpers at both load time and at runtime (the merged AST is stored in every hook entry).
    library_sources: Arc<Mutex<Vec<String>>>,
    /// Tracks which script name registered each schema name, for collision detection.
    schema_owners: Arc<Mutex<HashMap<String, String>>>,
    schema_registry: schema::SchemaRegistry,
    query_context: Arc<Mutex<Option<QueryContext>>>,
    /// Per-run note + attachment context set before a hook call and cleared after.
    pub run_context: Arc<Mutex<Option<NoteRunContext>>>,
}

mod engine;

impl ScriptRegistry {
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
    /// Sets the category for the next `load_script()` call.
    /// Used by two-phase loading to gate `schema()` to schema-category scripts.
    pub fn set_loading_category(&mut self, category: Option<String>) {
        *self.current_loading_category.lock().unwrap() = category;
    }

    pub fn load_script(&mut self, script: &str, name: &str) -> Result<()> {
        // Schema scripts get all library source prepended so they can call library helpers.
        // The resulting AST (library + schema) is stored in every hook entry so runtime
        // calls also resolve library functions correctly.
        let category = self.current_loading_category.lock().unwrap().clone();
        let is_schema = category.as_deref() == Some("schema");

        let source_to_compile: String = if is_schema {
            let lib = self.library_sources.lock().unwrap();
            if lib.is_empty() {
                script.to_string()
            } else {
                format!("{}\n\n{}", lib.join("\n\n"), script)
            }
        } else {
            script.to_string()
        };

        let ast = self
            .engine
            .compile(&source_to_compile)
            .map_err(|e| KrillnotesError::Scripting(e.to_string()))?;

        // SAFETY: mutex poisoning would require a panic while the lock is held,
        // which cannot happen in this codebase's single-threaded usage.
        *self.current_loading_ast.lock().unwrap() = Some(ast.clone());
        *self.current_loading_script_name.lock().unwrap() = Some(name.to_string());

        let result = self
            .engine
            .eval_ast::<()>(&ast)
            .map_err(|e| KrillnotesError::Scripting(e.to_string()));

        // Accumulate successful library/presentation scripts so later schema scripts can use them.
        if result.is_ok() && !is_schema {
            self.library_sources.lock().unwrap().push(script.to_string());
        }

        // Always clear: a failed script may have partially registered hooks;
        // leave no stale AST for the next load.
        *self.current_loading_ast.lock().unwrap() = None;
        *self.current_loading_script_name.lock().unwrap() = None;
        *self.current_loading_category.lock().unwrap() = None;

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

    /// Exposes the Rhai engine for direct `FnPtr::call` use (e.g. migration closures).
    pub(crate) fn engine(&self) -> &Engine {
        &self.engine
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
        schema: &str,
        title: &str,
        fields: &BTreeMap<String, FieldValue>,
    ) -> Result<Option<SaveTransaction>> {
        let schema_def = self.schema_registry.get(schema_name)?;
        self.schema_registry
            .run_on_save_hook(&self.engine, &schema_def, note_id, schema, title, fields)
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
        parent_fields: &BTreeMap<String, FieldValue>,
        child_id: &str,
        child_type: &str,
        child_title: &str,
        child_fields: &BTreeMap<String, FieldValue>,
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

    /// Returns `true` if any view registrations exist for `schema_name`.
    pub fn has_views(&self, schema_name: &str) -> bool {
        self.schema_registry.has_views(schema_name)
    }

    /// Returns `true` if a hover registration exists for `schema_name`.
    pub fn has_hover(&self, schema_name: &str) -> bool {
        self.schema_registry.has_hover(schema_name)
    }

    /// Returns the view registrations for a schema type.
    pub fn get_views_for_type(&self, schema_name: &str) -> Vec<ViewRegistration> {
        self.schema_registry.get_views_for_type(schema_name)
    }

    /// Returns warnings from unresolved deferred bindings.
    pub fn get_script_warnings(&self) -> Vec<ScriptWarning> {
        self.schema_registry.get_warnings()
    }

    /// Adds a warning to the script warning list.
    pub fn add_warning(&self, script_name: &str, message: &str) {
        self.schema_registry.add_warning(script_name, message);
    }

    /// Returns `(schema_name, schema_version, migrations, ast)` for every registered schema.
    pub fn get_versioned_schemas(&self) -> Vec<(String, u32, std::collections::BTreeMap<u32, FnPtr>, Option<AST>)> {
        self.schema_registry.get_versioned_schemas()
    }

    /// Converts a Rhai map (returned by a migration closure) into typed [`FieldValue`]s,
    /// using the schema's field type definitions to guide the conversion.
    pub fn rhai_map_to_fields(
        &self,
        map: &Map,
        schema_name: &str,
    ) -> Result<BTreeMap<String, FieldValue>> {
        let schema = self.schema_registry.get(schema_name)?;
        let mut result = BTreeMap::new();
        for (key, val) in map {
            let field_type = schema.fields.iter()
                .find(|f| f.name == key.as_str())
                .map(|f| f.field_type.as_str())
                .unwrap_or("text");
            result.insert(
                key.to_string(),
                schema::dynamic_to_field_value(val.clone(), field_type),
            );
        }
        Ok(result)
    }

    /// Resolves deferred bindings against registered schemas.
    pub fn resolve_bindings(&self) {
        self.schema_registry.resolve_bindings();
    }

    /// Renders a default HTML view for `note` using schema field type information.
    pub fn render_default_view(&self, note: &Note, resolved_titles: &std::collections::HashMap<String, String>, attachments: &[crate::core::attachment::AttachmentMeta]) -> String {
        let schema = self.schema_registry.get(&note.schema).ok();
        display_helpers::render_default_view(note, schema.as_ref(), resolved_titles, attachments)
    }

    /// Builds a note map for view/hover hooks.
    fn build_note_map(&self, note: &Note) -> Map {
        let mut fields_map = Map::new();
        for (k, v) in &note.fields {
            fields_map.insert(k.as_str().into(), field_value_to_dynamic(v));
        }
        let mut note_map = Map::new();
        note_map.insert("id".into(), Dynamic::from(note.id.clone()));
        note_map.insert("schema".into(), Dynamic::from(note.schema.clone()));
        note_map.insert("title".into(), Dynamic::from(note.title.clone()));
        note_map.insert("fields".into(), Dynamic::from(fields_map));
        let tags_array: rhai::Array = note.tags.iter()
            .map(|t| Dynamic::from(t.clone()))
            .collect();
        note_map.insert("tags".into(), Dynamic::from(tags_array));
        note_map
    }

    /// Runs the default view for a note (first display_first, or first registered).
    ///
    /// Returns `Ok(None)` when no views are registered for the note's schema.
    pub fn run_on_view_hook(
        &self,
        note: &Note,
        context: QueryContext,
    ) -> Result<Option<String>> {
        let note_map = self.build_note_map(note);
        *self.query_context.lock().unwrap() = Some(context);
        let result = self.schema_registry.run_default_view(&self.engine, note_map);
        *self.query_context.lock().unwrap() = None;
        result
    }

    /// Renders a specific named view for a note.
    pub fn run_view(
        &self,
        note: &Note,
        view_label: &str,
        context: QueryContext,
    ) -> Result<String> {
        let note_map = self.build_note_map(note);
        *self.query_context.lock().unwrap() = Some(context);
        let result = self.schema_registry.run_view(&self.engine, note_map, view_label);
        *self.query_context.lock().unwrap() = None;
        result
    }

    /// Runs the hover registration for a note, if any.
    pub fn run_on_hover_hook(
        &self,
        note: &Note,
        context: QueryContext,
    ) -> Result<Option<String>> {
        let note_map = self.build_note_map(note);
        *self.query_context.lock().unwrap() = Some(context);
        let result = self.schema_registry.run_on_hover_hook(&self.engine, note_map);
        *self.query_context.lock().unwrap() = None;
        result
    }

    /// Removes all registered schemas, hooks, and owner records so scripts can be reloaded.
    pub fn clear_all(&self) {
        self.schema_registry.clear();
        self.schema_owners.lock().unwrap().clear();
        self.library_sources.lock().unwrap().clear();
        *self.query_context.lock().unwrap() = None;
        *self.run_context.lock().unwrap() = None;
    }

    /// Returns a map of `note_type → [action_label, …]` for every registered menu action.
    pub fn tree_action_map(&self) -> HashMap<String, Vec<String>> {
        self.schema_registry.menu_action_map()
    }

    /// Returns a map of `note_type → [action_label, …]` — alias for `tree_action_map`.
    pub fn menu_action_map(&self) -> HashMap<String, Vec<String>> {
        self.schema_registry.menu_action_map()
    }

    /// Runs the menu action registered under `label`, passing `note` to the callback.
    pub fn invoke_tree_action_hook(
        &self,
        label: &str,
        note: &Note,
        context: QueryContext,
    ) -> Result<hooks::TreeActionResult> {
        let regs = self.schema_registry.menu_registrations_arc();
        let regs_guard = regs.lock().unwrap();
        let entry = regs_guard.values()
            .flat_map(|v| v.iter())
            .find(|r| r.label == label);

        let entry = entry.ok_or_else(|| {
            KrillnotesError::Scripting(format!("unknown tree action: {label:?}"))
        })?;
        let fn_ptr = entry.fn_ptr.clone();
        let ast = entry.ast.as_ref().clone();
        let script_name = entry.script_name.clone();
        drop(regs_guard);

        // Build note map — same shape as on_save / on_view.
        let mut fields_map = Map::new();
        for (k, v) in &note.fields {
            fields_map.insert(k.as_str().into(), field_value_to_dynamic(v));
        }
        let mut note_map = Map::new();
        note_map.insert("id".into(),        Dynamic::from(note.id.clone()));
        note_map.insert("schema".into(),    Dynamic::from(note.schema.clone()));
        note_map.insert("title".into(),     Dynamic::from(note.title.clone()));
        note_map.insert("fields".into(),    Dynamic::from(fields_map));

        // Install query context and a SaveTransaction pre-seeded with the acted-upon note,
        // so that set_title() / set_field() can reference it immediately.
        *self.query_context.lock().unwrap() = Some(context);
        let initial_tx = SaveTransaction::for_existing_note(
            note.id.clone(),
            note.schema.clone(),
            note.title.clone(),
            note.fields.clone(),
        );
        set_save_tx(initial_tx);
        let raw = fn_ptr
            .call::<Dynamic>(&self.engine, &ast, (Dynamic::from_map(note_map),))
            .map_err(|e| KrillnotesError::Scripting(
                format!("[{script_name}] tree action {label:?}: {e}")
            ));
        *self.query_context.lock().unwrap() = None;
        let transaction = take_save_tx().unwrap_or_default();
        let raw = raw?;

        // If callback returns an Array of Strings, treat as reorder request.
        let reorder = if let Some(arr) = raw.try_cast::<rhai::Array>() {
            let ids: Vec<String> = arr.into_iter()
                .filter_map(|v| v.try_cast::<String>())
                .collect();
            Some(ids)
        } else {
            None
        };

        Ok(hooks::TreeActionResult { reorder, transaction })
    }

    /// Returns `true` if a schema with `name` is registered.
    pub fn schema_exists(&self, name: &str) -> bool {
        self.schema_registry.exists(name)
    }

    /// Runs the `validate` closure for a single field, if one is registered.
    ///
    /// Returns `Ok(None)` when the field is valid or has no validate closure.
    /// Returns `Ok(Some(msg))` when the closure returns an error message.
    pub fn validate_field(
        &self,
        schema_name: &str,
        field_name: &str,
        value: &crate::core::note::FieldValue,
    ) -> crate::Result<Option<String>> {
        let schema = self.schema_registry.get(schema_name)?;
        let Some(ast) = schema.ast.as_ref() else { return Ok(None); };

        let field = schema.all_fields().into_iter()
            .find(|f| f.name == field_name);
        let Some(field_def) = field else { return Ok(None); };
        let Some(fn_ptr) = field_def.validate.as_ref() else { return Ok(None); };

        let dyn_value = schema::field_value_to_dynamic(value);
        let result = fn_ptr
            .call::<rhai::Dynamic>(&self.engine, ast, (dyn_value,))
            .map_err(|e| KrillnotesError::Scripting(
                format!("[{schema_name}] validate {field_name:?}: {e}")
            ))?;

        // () = valid; String = error message
        if result.is_unit() {
            Ok(None)
        } else if let Some(msg) = result.try_cast::<String>() {
            Ok(Some(msg))
        } else {
            Ok(None)
        }
    }

    /// Runs `validate` closures for all fields that have them and have a value.
    ///
    /// Returns a map of `field_name → error_message` for each invalid field.
    pub fn validate_fields(
        &self,
        schema_name: &str,
        fields: &std::collections::BTreeMap<String, crate::core::note::FieldValue>,
    ) -> crate::Result<std::collections::BTreeMap<String, String>> {
        let mut errors = std::collections::BTreeMap::new();
        let schema = self.schema_registry.get(schema_name)?;
        let Some(ast) = schema.ast.as_ref() else { return Ok(errors); };

        for field_def in schema.all_fields() {
            let Some(fn_ptr) = field_def.validate.as_ref() else { continue; };
            let Some(value) = fields.get(&field_def.name) else { continue; };

            let dyn_value = schema::field_value_to_dynamic(value);
            let result = fn_ptr
                .call::<rhai::Dynamic>(&self.engine, ast, (dyn_value,))
                .map_err(|e| KrillnotesError::Scripting(
                    format!("[{schema_name}] validate {:?}: {e}", field_def.name)
                ))?;

            if let Some(msg) = result.try_cast::<String>() {
                errors.insert(field_def.name.clone(), msg);
            }
        }
        Ok(errors)
    }

    /// Evaluates the `visible` closure for each `FieldGroup`.
    ///
    /// Returns a map of `group_name → bool`.
    /// Groups with no `visible` closure are always `true`.
    pub fn evaluate_group_visibility(
        &self,
        schema_name: &str,
        fields: &std::collections::BTreeMap<String, crate::core::note::FieldValue>,
    ) -> crate::Result<std::collections::BTreeMap<String, bool>> {
        let schema = self.schema_registry.get(schema_name)?;
        let Some(ast) = schema.ast.as_ref() else {
            return Ok(schema.field_groups.iter()
                .map(|g| (g.name.clone(), true))
                .collect());
        };

        let mut fields_map = rhai::Map::new();
        for (k, v) in fields {
            fields_map.insert(k.as_str().into(), schema::field_value_to_dynamic(v));
        }

        let mut result = std::collections::BTreeMap::new();
        for group in &schema.field_groups {
            let visible = match group.visible.as_ref() {
                None => true,
                Some(fn_ptr) => {
                    let ret = fn_ptr
                        .call::<rhai::Dynamic>(&self.engine, ast, (Dynamic::from_map(fields_map.clone()),))
                        .map_err(|e| KrillnotesError::Scripting(
                            format!("[{schema_name}] visible {:?}: {e}", group.name)
                        ))?;
                    ret.try_cast::<bool>().unwrap_or(true)
                }
            };
            result.insert(group.name.clone(), visible);
        }
        Ok(result)
    }

    /// Sets the per-run note and attachment context before executing a hook.
    pub fn set_run_context(&self, note: crate::core::note::Note, attachments: Vec<AttachmentMeta>) {
        *self.run_context.lock().expect("run_context poisoned") =
            Some(NoteRunContext { note, attachments });
    }

    /// Clears the per-run context after a hook has finished executing.
    pub fn clear_run_context(&self) {
        *self.run_context.lock().expect("run_context poisoned") = None;
    }

}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
