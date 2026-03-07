// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Rhai-based scripting registry for Krillnotes note types and hooks.
//!
//! [`ScriptRegistry`] is the public entry point. It owns the Rhai [`Engine`],
//! loads scripts, and delegates schema and hook concerns to internal sub-registries.

mod display_helpers;
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
    note_map.insert("node_type".into(), Dynamic::from(pending.node_type.clone()));
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
    /// Tracks which script name registered each schema name, for collision detection.
    schema_owners: Arc<Mutex<HashMap<String, String>>>,
    schema_registry: schema::SchemaRegistry,
    query_context: Arc<Mutex<Option<QueryContext>>>,
    /// Per-run note + attachment context set before a hook call and cleared after.
    pub run_context: Arc<Mutex<Option<NoteRunContext>>>,
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
        let current_loading_category: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let schema_owners: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));

        // Register register_view/hover/menu — deferred binding functions
        let deferred_arc = schema_registry.deferred_bindings_arc();
        let view_ast_arc = Arc::clone(&current_loading_ast);
        let view_name_arc = Arc::clone(&current_loading_script_name);

        // 3-arg form: register_view(type, label, closure)
        let d1 = Arc::clone(&deferred_arc);
        let a1 = Arc::clone(&view_ast_arc);
        let n1 = Arc::clone(&view_name_arc);
        engine.register_fn("register_view",
            move |target_type: String, label: String, fn_ptr: FnPtr|
            -> std::result::Result<Dynamic, Box<EvalAltResult>>
            {
                let ast = a1.lock().unwrap().clone()
                    .ok_or_else(|| -> Box<EvalAltResult> {
                        "register_view() called outside of load_script".to_string().into()
                    })?;
                let script_name = n1.lock().unwrap().clone().unwrap_or_default();
                d1.lock().unwrap().push(DeferredBinding {
                    kind: BindingKind::View,
                    target_type,
                    fn_ptr,
                    ast: Arc::new(ast),
                    script_name,
                    display_first: false,
                    label: Some(label),
                    applies_to: vec![],
                });
                Ok(Dynamic::UNIT)
            }
        );

        // 4-arg form: register_view(type, label, options, closure)
        let d2 = Arc::clone(&deferred_arc);
        let a2 = Arc::clone(&view_ast_arc);
        let n2 = Arc::clone(&view_name_arc);
        engine.register_fn("register_view",
            move |target_type: String, label: String, options: rhai::Map, fn_ptr: FnPtr|
            -> std::result::Result<Dynamic, Box<EvalAltResult>>
            {
                let ast = a2.lock().unwrap().clone()
                    .ok_or_else(|| -> Box<EvalAltResult> {
                        "register_view() called outside of load_script".to_string().into()
                    })?;
                let script_name = n2.lock().unwrap().clone().unwrap_or_default();
                let display_first = options.get("display_first")
                    .and_then(|v| v.as_bool().ok())
                    .unwrap_or(false);
                d2.lock().unwrap().push(DeferredBinding {
                    kind: BindingKind::View,
                    target_type,
                    fn_ptr,
                    ast: Arc::new(ast),
                    script_name,
                    display_first,
                    label: Some(label),
                    applies_to: vec![],
                });
                Ok(Dynamic::UNIT)
            }
        );

        // register_hover(target_type, closure)
        let d3 = Arc::clone(&deferred_arc);
        let a3 = Arc::clone(&view_ast_arc);
        let n3 = Arc::clone(&view_name_arc);
        engine.register_fn("register_hover",
            move |target_type: String, fn_ptr: FnPtr|
            -> std::result::Result<Dynamic, Box<EvalAltResult>>
            {
                let ast = a3.lock().unwrap().clone()
                    .ok_or_else(|| -> Box<EvalAltResult> {
                        "register_hover() called outside of load_script".to_string().into()
                    })?;
                let script_name = n3.lock().unwrap().clone().unwrap_or_default();
                d3.lock().unwrap().push(DeferredBinding {
                    kind: BindingKind::Hover,
                    target_type,
                    fn_ptr,
                    ast: Arc::new(ast),
                    script_name,
                    display_first: false,
                    label: None,
                    applies_to: vec![],
                });
                Ok(Dynamic::UNIT)
            }
        );

        // register_menu(label, target_types, closure)
        let d4 = Arc::clone(&deferred_arc);
        let a4 = Arc::clone(&view_ast_arc);
        let n4 = Arc::clone(&view_name_arc);
        engine.register_fn("register_menu",
            move |label: String, types: rhai::Array, fn_ptr: FnPtr|
            -> std::result::Result<Dynamic, Box<EvalAltResult>>
            {
                let ast = a4.lock().unwrap().clone()
                    .ok_or_else(|| -> Box<EvalAltResult> {
                        "register_menu() called outside of load_script".to_string().into()
                    })?;
                let script_name = n4.lock().unwrap().clone().unwrap_or_default();
                let applies_to: Vec<String> = types.into_iter()
                    .filter_map(|v| v.into_string().ok())
                    .collect();
                d4.lock().unwrap().push(DeferredBinding {
                    kind: BindingKind::Menu,
                    target_type: String::new(),
                    fn_ptr,
                    ast: Arc::new(ast),
                    script_name,
                    display_first: false,
                    label: Some(label),
                    applies_to,
                });
                Ok(Dynamic::UNIT)
            }
        );

        // Register schema() host function — writes schema and schema-bound hooks into SchemaRegistry.
        let schemas_arc       = schema_registry.schemas_arc();
        let on_save_arc       = schema_registry.on_save_hooks_arc();
        let on_add_child_arc  = schema_registry.on_add_child_hooks_arc();
        let schema_ast_arc    = Arc::clone(&current_loading_ast);
        let schema_name_arc   = Arc::clone(&current_loading_script_name);
        let schema_cat_arc    = Arc::clone(&current_loading_category);
        let schema_owners_arc = Arc::clone(&schema_owners);
        engine.register_fn("schema", move |name: String, def: rhai::Map| -> std::result::Result<Dynamic, Box<EvalAltResult>> {
            // Gate: schema() can only be called from schema-category scripts.
            let cat = schema_cat_arc.lock().unwrap();
            if cat.as_deref() == Some("presentation") {
                return Err("schema() can only be called from schema-category scripts, not presentation/library scripts".into());
            }
            drop(cat);

            let script_name = schema_name_arc.lock().unwrap()
                .clone()
                .unwrap_or_else(|| "<unknown>".to_string());

            // Collision check: first script to register a schema name wins.
            // A script is allowed to re-register a schema it already owns (e.g. during
            // update pre-validation where the live registry still holds its previous schemas).
            {
                let owners = schema_owners_arc.lock().unwrap();
                if let Some(owner) = owners.get(&name) {
                    if owner != &script_name {
                        return Err(format!(
                            "Schema '{}' is already defined by script '{}'. Schema names must be unique.",
                            name, owner
                        ).into());
                    }
                }
            }
            schema_owners_arc.lock().unwrap().insert(name.clone(), script_name.clone());

            let mut s = Schema::parse_from_rhai(&name, &def)
                .map_err(|e| -> Box<EvalAltResult> { e.to_string().into() })?;

            // Version guard: prevent downgrades
            {
                let schemas = schemas_arc.lock().unwrap();
                if let Some(existing) = schemas.get(&name) {
                    if s.version < existing.version {
                        return Err(format!(
                            "Schema '{}' version {} cannot replace existing version {} — downgrade not allowed",
                            name, s.version, existing.version
                        ).into());
                    }
                }
            }

            // Store the script AST so validate/visible closures can be called later.
            s.ast = schema_ast_arc.lock().unwrap().clone();
            schemas_arc.lock().unwrap().insert(name.clone(), s);

            // Extract optional on_save closure.
            if let Some(fn_ptr) = def.get("on_save").and_then(|v| v.clone().try_cast::<FnPtr>()) {
                let ast = schema_ast_arc.lock().unwrap().clone()
                    .ok_or_else(|| -> Box<EvalAltResult> {
                        "schema() called outside of load_script".to_string().into()
                    })?;
                on_save_arc.lock().unwrap().insert(name.clone(), HookEntry { fn_ptr, ast, script_name: script_name.clone() });
            }

            // Extract optional on_add_child closure.
            if let Some(fn_ptr) = def.get("on_add_child").and_then(|v| v.clone().try_cast::<FnPtr>()) {
                let ast = schema_ast_arc.lock().unwrap().clone()
                    .ok_or_else(|| -> Box<EvalAltResult> {
                        "schema() called outside of load_script".to_string().into()
                    })?;
                on_add_child_arc.lock().unwrap().insert(name.clone(), HookEntry { fn_ptr, ast, script_name: script_name.clone() });
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

        // ── Per-run note + attachment context ─────────────────────────────────
        let run_context: Arc<Mutex<Option<NoteRunContext>>> = Arc::new(Mutex::new(None));

        // Register get_children() — returns direct children of a note by ID.
        let qc1 = Arc::clone(&query_context);
        engine.register_fn("get_children", move |id: String| -> rhai::Array {
            // Collect pre-existing children from the snapshot.
            let mut result: rhai::Array = {
                let guard = qc1.lock().unwrap();
                guard.as_ref()
                    .and_then(|ctx| ctx.children_by_id.get(&id).cloned())
                    .unwrap_or_default()
            };

            // Also include any in-flight new pending notes from the thread-local SAVE_TX.
            SAVE_TX.with(|cell| {
                if let Some(tx) = cell.borrow().as_ref() {
                    for pending in tx.pending_notes.values() {
                        if pending.is_new && pending.parent_id.as_deref() == Some(&id) {
                            result.push(pending_note_to_dynamic(pending));
                        }
                    }
                }
            });

            result
        });

        // Register get_note() — returns any note by ID.
        let qc2 = Arc::clone(&query_context);
        engine.register_fn("get_note", move |id: String| -> Dynamic {
            // Check thread-local SAVE_TX first (in-flight pending notes).
            let found = SAVE_TX.with(|cell| {
                cell.borrow().as_ref().and_then(|tx| {
                    tx.pending_notes.get(&id).filter(|p| p.is_new).map(pending_note_to_dynamic)
                })
            });
            if let Some(dyn_note) = found {
                return dyn_note;
            }
            // Fall back to snapshot.
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

        // Register get_notes_for_tag(tags) — returns notes carrying any of the given tags (OR).
        let qc4 = Arc::clone(&query_context);
        engine.register_fn("get_notes_for_tag", move |tags: rhai::Array| -> rhai::Array {
            let guard = qc4.lock().unwrap();
            let Some(ctx) = guard.as_ref() else { return vec![]; };
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut result: rhai::Array = Vec::new();
            for tag_dyn in &tags {
                let tag = tag_dyn.to_string();
                if let Some(notes) = ctx.notes_by_tag.get(&tag) {
                    for note in notes {
                        // Extract the id to dedup; safe to clone Dynamic.
                        let id = note.clone().try_cast::<rhai::Map>()
                            .and_then(|m| m.get("id").and_then(|v| v.clone().into_string().ok()))
                            .unwrap_or_default();
                        if seen.insert(id) {
                            result.push(note.clone());
                        }
                    }
                }
            }
            result
        });

        // Register get_notes_with_link(target_id) — returns all notes whose note_link field
        // points to the given target note ID.
        let qc5 = Arc::clone(&query_context);
        engine.register_fn("get_notes_with_link", move |target_id: String| -> rhai::Array {
            let guard = qc5.lock().unwrap();
            guard.as_ref()
                .and_then(|ctx| ctx.notes_by_link_target.get(&target_id).cloned())
                .unwrap_or_default()
        });

        // Register get_attachments(note_id) — returns attachment metadata for a note.
        let qc6 = Arc::clone(&query_context);
        engine.register_fn("get_attachments", move |note_id: String| -> rhai::Array {
            let guard = qc6.lock().unwrap();
            guard.as_ref()
                .and_then(|ctx| ctx.attachments_by_note_id.get(&note_id).cloned())
                .unwrap_or_default()
                .into_iter()
                .map(|att| {
                    let mut m = rhai::Map::new();
                    m.insert("id".into(),        Dynamic::from(att.id));
                    m.insert("filename".into(),  Dynamic::from(att.filename));
                    m.insert("mime_type".into(), att.mime_type
                        .map(Dynamic::from)
                        .unwrap_or(Dynamic::UNIT));
                    m.insert("size_bytes".into(), Dynamic::from(att.size_bytes));
                    Dynamic::from(m)
                })
                .collect()
        });

        // create_child(parent_id, node_type) — available inside add_tree_action closures.
        // Queues a new pending note into the thread-local SaveTransaction and returns a note map.
        let create_child_schemas = schema_registry.clone();
        engine.register_fn("create_child",
            move |parent_id: String, node_type: String|
            -> std::result::Result<Dynamic, Box<EvalAltResult>>
            {
                let default_fields = {
                    let schemas_arc = create_child_schemas.schemas_arc();
                    let registry = schemas_arc.lock().unwrap();
                    let schema = registry.get(&node_type).ok_or_else(|| -> Box<EvalAltResult> {
                        format!("create_child: unknown schema {:?}", node_type).into()
                    })?;
                    schema.default_fields()
                };
                let note_id = uuid::Uuid::new_v4().to_string();

                with_save_tx(|tx| {
                    tx.add_new_note(
                        note_id.clone(),
                        parent_id.clone(),
                        node_type.clone(),
                        String::new(),
                        default_fields.clone(),
                    );
                    Ok(())
                })?;

                // Return a note map to the Rhai script.
                let mut fields_map = rhai::Map::new();
                for (k, v) in &default_fields {
                    fields_map.insert(k.as_str().into(), schema::field_value_to_dynamic(v));
                }
                let mut map = rhai::Map::new();
                map.insert("id".into(),        Dynamic::from(note_id));
                map.insert("parent_id".into(), Dynamic::from(parent_id));
                map.insert("node_type".into(), Dynamic::from(node_type));
                map.insert("title".into(),     Dynamic::from(String::new()));
                map.insert("fields".into(),    Dynamic::from(fields_map));
                map.insert("tags".into(),      Dynamic::from(rhai::Array::new()));
                Ok(Dynamic::from_map(map))
            }
        );

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
        engine.register_fn("embed_media", |url: String| -> String {
            display_helpers::make_media_embed_html(&url)
        });
        let ctx_for_markdown = Arc::clone(&run_context);
        engine.register_fn("markdown", move |text: String| -> String {
            let guard = ctx_for_markdown.lock().expect("run_context poisoned");
            let maybe_context = guard.as_ref().map(|ctx| (ctx.note.fields.clone(), ctx.attachments.clone()));
            drop(guard);  // release lock before any further work
            let after_images = if let Some((fields, attachments)) = maybe_context {
                display_helpers::preprocess_image_blocks(&text, &fields, &attachments)
            } else {
                text
            };
            let processed = display_helpers::preprocess_media_embeds(&after_images);
            display_helpers::rhai_markdown_raw(processed)
        });
        engine.register_fn("render_tags",  display_helpers::rhai_render_tags);

        engine.register_fn("display_image", |uuid: Dynamic, width: i64, alt: String| -> String {
            match uuid.into_string() {
                Ok(id) if !id.is_empty() => display_helpers::make_display_image_html(&id, width, &alt),
                _ => "<span class=\"kn-image-error\">No image set</span>".to_string(),
            }
        });

        engine.register_fn("display_download_link", |uuid: Dynamic, label: String| -> String {
            match uuid.into_string() {
                Ok(id) if !id.is_empty() => display_helpers::make_download_link_html(&id, &label),
                _ => "<span class=\"kn-image-error\">No file set</span>".to_string(),
            }
        });
        engine.register_fn("stars",        display_helpers::rhai_stars_default);
        engine.register_fn("stars",        display_helpers::rhai_stars);

        // ── Date helpers ──────────────────────────────────────────────────────
        engine.register_fn("today", || Local::now().format("%Y-%m-%d").to_string());

        // ── Gated operations API (set_field / set_title / reject / commit) ────
        // These functions write into the thread-local SaveTransaction set by the
        // hook runner before calling the Rhai closure.
        //
        // set_field is registered via register_raw_fn so that it receives a
        // NativeCallContext, which allows calling validate FnPtrs without
        // needing a separate reference to the Engine.
        let set_field_schemas = schema_registry.schemas_arc();
        use rhai::NativeCallContext;
        // register_raw_fn is used so the closure receives a NativeCallContext,
        // which allows calling validate FnPtrs (closures) without needing a
        // separate engine reference. TypeId::of::<Dynamic>() for the third
        // argument means Rhai will dispatch any value type to this function.
        engine.register_raw_fn(
            "set_field",
            &[
                std::any::TypeId::of::<rhai::ImmutableString>(),
                std::any::TypeId::of::<rhai::ImmutableString>(),
                std::any::TypeId::of::<Dynamic>(),
            ],
            move |ctx: NativeCallContext, args: &mut [&mut Dynamic]| -> std::result::Result<Dynamic, Box<EvalAltResult>> {
                let note_id    = args[0].clone().cast::<rhai::ImmutableString>().to_string();
                let field_name = args[1].clone().cast::<rhai::ImmutableString>().to_string();
                let value      = args[2].clone();

                // Infer FieldValue from the Dynamic type.
                let fv = if value.is::<f64>() {
                    FieldValue::Number(value.cast::<f64>())
                } else if value.is::<bool>() {
                    FieldValue::Boolean(value.cast::<bool>())
                } else if value.is_unit() {
                    // Use Text empty as a sensible default for unit/nil.
                    FieldValue::Text(String::new())
                } else {
                    let s = value.into_string().map_err(|e| -> Box<EvalAltResult> {
                        format!("set_field: cannot convert value to string: {e}").into()
                    })?;
                    FieldValue::Text(s)
                };

                // Run the field's validate closure (if any) as a hard error.
                // Look up the note's schema type from the in-flight SAVE_TX.
                let node_type_opt: Option<String> = SAVE_TX.with(|cell| {
                    cell.borrow()
                        .as_ref()
                        .and_then(|tx| tx.pending_notes.get(&note_id).map(|p| p.node_type.clone()))
                });
                if let Some(node_type) = node_type_opt {
                    // Clone the data we need before releasing the lock.
                    let (validate_fn_opt, ast_opt) = {
                        let schemas = set_field_schemas.lock().unwrap();
                        if let Some(schema) = schemas.get(&node_type) {
                            let field_def = schema.all_fields()
                                .into_iter()
                                .find(|fd| fd.name == field_name)
                                .cloned();
                            if let Some(fd) = field_def {
                                (fd.validate.clone(), schema.ast.clone())
                            } else {
                                (None, None)
                            }
                        } else {
                            (None, None)
                        }
                    };
                    // Call validate if both closure and AST are available.
                    if let (Some(validate_fn), Some(ast)) = (validate_fn_opt, ast_opt) {
                        let dyn_val = schema::field_value_to_dynamic(&fv);
                        // Temporarily push the schema AST into the call context.
                        let validate_result: rhai::Dynamic = validate_fn
                            .call_within_context::<rhai::Dynamic>(&ctx, (dyn_val,))
                            .map_err(|e| -> Box<EvalAltResult> {
                                format!("set_field validate error for field '{}': {e}", field_name).into()
                            })?;
                        // Validate returns () for valid, or a String error message.
                        if let Some(err_msg) = validate_result.try_cast::<String>() {
                            return Err(err_msg.into());
                        }
                        // () or non-string return = valid; proceed normally.
                        let _ = ast; // ast was used via ctx which carries the active AST
                    }
                }

                with_save_tx(|tx| tx.set_field(&note_id, field_name, fv))?;
                Ok(Dynamic::UNIT)
            }
        );

        engine.register_fn("set_title",
            |note_id: String, title: String|
            -> std::result::Result<Dynamic, Box<EvalAltResult>>
            {
                with_save_tx(|tx| tx.set_title(&note_id, title))?;
                Ok(Dynamic::UNIT)
            }
        );

        // reject(message) — note-level soft error
        engine.register_fn("reject",
            |message: String| -> std::result::Result<Dynamic, Box<EvalAltResult>> {
                with_save_tx(|tx| { tx.reject_note(message); Ok(()) })?;
                Ok(Dynamic::UNIT)
            }
        );

        // reject(field, message) — field-pinned soft error
        engine.register_fn("reject",
            |field: String, message: String| -> std::result::Result<Dynamic, Box<EvalAltResult>> {
                with_save_tx(|tx| { tx.reject_field(field, message); Ok(()) })?;
                Ok(Dynamic::UNIT)
            }
        );

        engine.register_fn("commit",
            || -> std::result::Result<Dynamic, Box<EvalAltResult>> {
                with_save_tx(|tx| {
                    tx.commit().map_err(|errors| {
                        let msgs: Vec<String> = errors.iter().map(|e| {
                            match &e.field {
                                Some(f) => format!("{}: {}", f, e.message),
                                None => e.message.clone(),
                            }
                        }).collect();
                        format!("Validation failed: {}", msgs.join("; "))
                    })
                })?;
                Ok(Dynamic::UNIT)
            }
        );

        Ok(Self {
            engine,
            current_loading_ast,
            current_loading_script_name,
            current_loading_category,
            schema_owners,
            schema_registry,
            query_context,
            run_context,
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
    /// Sets the category for the next `load_script()` call.
    /// Used by two-phase loading to gate `schema()` to schema-category scripts.
    pub fn set_loading_category(&mut self, category: Option<String>) {
        *self.current_loading_category.lock().unwrap() = category;
    }

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
        node_type: &str,
        title: &str,
        fields: &BTreeMap<String, FieldValue>,
    ) -> Result<Option<SaveTransaction>> {
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
        let schema = self.schema_registry.get(&note.node_type).ok();
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
        note_map.insert("node_type".into(), Dynamic::from(note.node_type.clone()));
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
        note_map.insert("node_type".into(), Dynamic::from(note.node_type.clone()));
        note_map.insert("title".into(),     Dynamic::from(note.title.clone()));
        note_map.insert("fields".into(),    Dynamic::from(fields_map));

        // Install query context and a SaveTransaction pre-seeded with the acted-upon note,
        // so that set_title() / set_field() can reference it immediately.
        *self.query_context.lock().unwrap() = Some(context);
        let initial_tx = SaveTransaction::for_existing_note(
            note.id.clone(),
            note.node_type.clone(),
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
mod tests {
    use super::*;

    /// Helper: loads the bundled TextNote starter script into a registry.
    fn load_text_note(registry: &mut ScriptRegistry) {
        registry.load_script(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/system_scripts/00_text_note.rhai"
        )), "Text Note Actions").expect("TextNote presentation script should load");
        registry.load_script(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/system_scripts/00_text_note.schema.rhai"
        )), "Text Note").expect("TextNote schema script should load");
        registry.resolve_bindings();
    }

    // ── hooks-inside-schema (new style) ─────────────────────────────────────

    #[test]
    fn test_on_save_inside_schema_sets_title() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Person", #{ version: 1,
                fields: [
                    #{ name: "first", type: "text", required: false },
                    #{ name: "last",  type: "text", required: false },
                ],
                on_save: |note| {
                    set_title(note.id, note.fields["last"] + ", " + note.fields["first"]);
                    commit();
                }
            });
        "#, "test").unwrap();

        let mut fields = BTreeMap::new();
        fields.insert("first".to_string(), FieldValue::Text("John".to_string()));
        fields.insert("last".to_string(), FieldValue::Text("Doe".to_string()));

        let tx = registry
            .run_on_save_hook("Person", "id-1", "Person", "old title", &fields)
            .unwrap()
            .unwrap();

        assert!(tx.committed);
        let pn = tx.pending_notes.get("id-1").unwrap();
        assert_eq!(pn.effective_title(), "Doe, John");
    }

    #[test]
    fn test_on_view_inside_schema_returns_html() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            register_view("Folder", "Default", |note| {
                text("hello from view")
            });
        "#, "test_views.rhai").unwrap();
        registry.load_script(r#"
            schema("Folder", #{ version: 1,
                fields: [],
            });
        "#, "test.schema.rhai").unwrap();
        registry.resolve_bindings();

        use crate::Note;
        let note = Note {
            id: "n1".to_string(), node_type: "Folder".to_string(),
            title: "F".to_string(), parent_id: None, position: 0.0,
            created_at: 0, modified_at: 0, created_by: String::new(), modified_by: String::new(),
            fields: std::collections::BTreeMap::new(), is_expanded: false, tags: vec![], schema_version: 1,
        };
        let ctx = QueryContext {
            notes_by_id: std::collections::HashMap::new(),
            children_by_id: std::collections::HashMap::new(),
            notes_by_type: std::collections::HashMap::new(),
            notes_by_tag: std::collections::HashMap::new(),
            notes_by_link_target: Default::default(),
            attachments_by_note_id: Default::default(),
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
                schema("Widget", #{ version: 1,
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
                schema("TestNote", #{ version: 1,
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

    // ── Schema collision detection ───────────────────────────────────────────

    #[test]
    fn test_schema_collision_returns_error() {
        let mut registry = ScriptRegistry::new().unwrap();

        // First script registers "Contact" — should succeed.
        registry.load_script(r#"
            schema("Contact", #{ version: 1, fields: [] });
        "#, "script_a").expect("first registration should succeed");

        // Second script tries to register "Contact" — should fail.
        let err = registry.load_script(r#"
            schema("Contact", #{ version: 1, fields: [] });
        "#, "script_b").expect_err("second registration should fail");

        let msg = err.to_string();
        assert!(msg.contains("Contact"), "error should mention the schema name");
        assert!(msg.contains("script_a"), "error should name the owning script");
    }

    #[test]
    fn test_first_schema_wins_after_collision() {
        let mut registry = ScriptRegistry::new().unwrap();

        registry.load_script(r#"
            schema("Widget", #{ version: 1,
                fields: [ #{ name: "color", type: "text", required: false } ],
            });
        "#, "owner_script").unwrap();

        // Collision attempt — should fail.
        let _ = registry.load_script(r#"
            schema("Widget", #{ version: 1,
                fields: [ #{ name: "size", type: "number", required: false } ],
            });
        "#, "intruder_script");

        // The schema registered by the first script must still be intact.
        let schema = registry.get_schema("Widget").unwrap();
        assert_eq!(schema.fields.len(), 1);
        assert_eq!(schema.fields[0].name, "color", "first script's field definition should win");
    }

    #[test]
    fn test_clear_all_resets_owners_for_reload() {
        let mut registry = ScriptRegistry::new().unwrap();

        registry.load_script(r#"
            schema("Reloadable", #{ version: 1, fields: [] });
        "#, "script_one").unwrap();

        // After clear_all, the owner record is gone — so the same name can be registered again.
        registry.clear_all();

        registry.load_script(r#"
            schema("Reloadable", #{ version: 1, fields: [] });
        "#, "script_one").expect("re-registration after clear_all should succeed");
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
                    target_type: None,
                    show_on_hover: false,
                    allowed_types: vec![], validate: None,
                },
                FieldDefinition {
                    name: "count".to_string(),
                    field_type: "number".to_string(),
                    required: false,
                    can_view: true,
                    can_edit: true,
                    options: vec![],
                    max: 0,
                    target_type: None,
                    show_on_hover: false,
                    allowed_types: vec![], validate: None,
                },
            ],
            title_can_view: true,
            title_can_edit: true,
            children_sort: "none".to_string(),
            allowed_parent_types: vec![],
            allowed_children_types: vec![],
            allow_attachments: false,
            attachment_types: vec![], field_groups: vec![], ast: None, version: 1, migrations: std::collections::BTreeMap::new(),
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
                target_type: None,
                show_on_hover: false,
                allowed_types: vec![], validate: None,
            }],
            title_can_view: true,
            title_can_edit: true,
            children_sort: "none".to_string(),
            allowed_parent_types: vec![],
            allowed_children_types: vec![],
            allow_attachments: false,
            attachment_types: vec![], field_groups: vec![], ast: None, version: 1, migrations: std::collections::BTreeMap::new(),
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
                target_type: None,
                show_on_hover: false,
                allowed_types: vec![], validate: None,
            }],
            title_can_view: true,
            title_can_edit: true,
            children_sort: "none".to_string(),
            allowed_parent_types: vec![],
            allowed_children_types: vec![],
            allow_attachments: false,
            attachment_types: vec![], field_groups: vec![], ast: None, version: 1, migrations: std::collections::BTreeMap::new(),
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
        )), "Contacts View").unwrap();
        registry.load_script(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/system_scripts/01_contact.schema.rhai"
        )), "Contacts").unwrap();
        registry.resolve_bindings();
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
                schema("Person", #{ version: 1,
                    fields: [
                        #{ name: "first", type: "text", required: false },
                        #{ name: "last",  type: "text", required: false },
                    ],
                    on_save: |note| {
                        set_title(note.id, note.fields["last"] + ", " + note.fields["first"]);
                        commit();
                    }
                });
            "#,
            "test")
            .unwrap();

        let mut fields = BTreeMap::new();
        fields.insert("first".to_string(), FieldValue::Text("John".to_string()));
        fields.insert("last".to_string(), FieldValue::Text("Doe".to_string()));

        let tx = registry
            .run_on_save_hook("Person", "id-1", "Person", "old title", &fields)
            .unwrap()
            .unwrap();

        assert!(tx.committed);
        let pn = tx.pending_notes.get("id-1").unwrap();
        assert_eq!(pn.effective_title(), "Doe, John");
        assert_eq!(pn.effective_fields().get("first"), Some(&FieldValue::Text("John".to_string())));
    }

    #[test]
    fn test_run_on_save_hook_no_hook_returns_none() {
        let mut registry = ScriptRegistry::new().unwrap();
        load_text_note(&mut registry);
        let fields = BTreeMap::new();
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
        )), "Contacts View").unwrap();
        registry.load_script(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/system_scripts/01_contact.schema.rhai"
        )), "Contacts").unwrap();
        registry.resolve_bindings();
        assert!(registry.has_hook("Contact"), "Contact schema should have an on_save hook");

        let mut fields = BTreeMap::new();
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

        let tx = registry
            .run_on_save_hook("Contact", "id-1", "Contact", "", &fields)
            .unwrap()
            .unwrap();

        assert!(tx.committed);
        let pn = tx.pending_notes.get("id-1").unwrap();
        assert_eq!(pn.effective_title(), "Smith, Jane");
    }

    // ── Field flags ─────────────────────────────────────────────────────────

    #[test]
    fn test_field_can_view_can_edit_defaults_to_true() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("TestVis", #{ version: 1,
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
            schema("TestVis2", #{ version: 1,
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
            schema("TestVisExplicit", #{ version: 1,
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
            schema("TitleTest", #{ version: 1,
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
            schema("TitleHidden", #{ version: 1,
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
            schema("TitleExplicit", #{ version: 1,
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
    fn test_schema_allow_attachments_defaults_to_false() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("AttachTest", #{ version: 1,
                fields: [#{ name: "name", type: "text" }]
            });
        "#, "test").unwrap();
        let schema = registry.get_schema("AttachTest").unwrap();
        assert!(!schema.allow_attachments, "allow_attachments should default to false");
        assert!(schema.attachment_types.is_empty(), "attachment_types should default to empty");
    }

    #[test]
    fn test_schema_allow_attachments_explicit_with_types() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("PhotoNote", #{ version: 1,
                allow_attachments: true,
                attachment_types: ["image/jpeg", "image/png"],
                fields: [#{ name: "caption", type: "text" }]
            });
        "#, "test").unwrap();
        let schema = registry.get_schema("PhotoNote").unwrap();
        assert!(schema.allow_attachments);
        assert_eq!(schema.attachment_types, vec!["image/jpeg", "image/png"]);
    }

    #[test]
    fn test_contact_title_can_edit_false() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/system_scripts/01_contact.rhai"
        )), "Contacts View").unwrap();
        registry.load_script(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/system_scripts/01_contact.schema.rhai"
        )), "Contacts").unwrap();
        registry.resolve_bindings();
        let schema = registry.get_schema("Contact").unwrap();
        assert!(!schema.title_can_edit, "Contact title_can_edit should be false");
        assert!(schema.title_can_view, "Contact title_can_view should still be true");
    }

    // ── Boolean / default value edge cases ──────────────────────────────────

    #[test]
    fn test_boolean_field_not_set_by_hook_is_absent_from_effective_fields() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry
            .load_script(
                r#"
                schema("FlagNote", #{ version: 1,
                    fields: [
                        #{ name: "flag", type: "boolean", required: false },
                    ],
                    on_save: |note| {
                        // intentionally does NOT set flag
                        commit();
                    }
                });
            "#,
            "test")
            .unwrap();

        // Do NOT include "flag" in the submitted fields.
        // In the gated model, effective_fields() = original_fields merged with pending_fields.
        // Since neither contain "flag", it is absent from the output.
        let fields = BTreeMap::new();

        let tx = registry
            .run_on_save_hook("FlagNote", "id-1", "FlagNote", "title", &fields)
            .unwrap()
            .unwrap();

        assert!(tx.committed);
        let pn = tx.pending_notes.get("id-1").unwrap();
        // Field was not in submitted fields and hook did not set it — absent from output.
        assert_eq!(
            pn.effective_fields().get("flag"),
            None,
            "boolean field absent from input and not set by hook should be absent from effective_fields"
        );
    }

    // ── clear_all ───────────────────────────────────────────────────────────

    #[test]
    fn test_load_script_and_clear_all() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("MyType", #{ version: 1, fields: [#{ name: "x", type: "text" }] });
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
            schema("Custom", #{ version: 1, fields: [#{ name: "a", type: "text" }] });
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
            schema("Hooked", #{ version: 1,
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
            schema("Ticket", #{ version: 1,
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
            schema("Review", #{ version: 1,
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
            schema("Bad", #{ version: 1,
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
            schema("Bad", #{ version: 1,
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
            schema("Widget", #{ version: 1,
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
            schema("S", #{ version: 1,
                fields: [ #{ name: "status", type: "select", options: ["A", "B"] } ],
                on_save: |note| {
                    set_field(note.id, "status", "B");
                    commit();
                }
            });
        "#, "test").unwrap();

        let mut fields = BTreeMap::new();
        fields.insert("status".to_string(), FieldValue::Text("A".to_string()));

        let tx = registry
            .run_on_save_hook("S", "id1", "S", "title", &fields)
            .unwrap()
            .unwrap();
        assert!(tx.committed);
        let pn = tx.pending_notes.get("id1").unwrap();
        assert_eq!(pn.effective_fields()["status"], FieldValue::Text("B".to_string()));
    }

    #[test]
    fn test_rating_field_round_trips_through_hook() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("R", #{ version: 1,
                fields: [ #{ name: "stars", type: "rating", max: 5 } ],
                on_save: |note| {
                    set_field(note.id, "stars", 4.0);
                    commit();
                }
            });
        "#, "test").unwrap();

        let mut fields = BTreeMap::new();
        fields.insert("stars".to_string(), FieldValue::Number(0.0));

        let tx = registry
            .run_on_save_hook("R", "id1", "R", "title", &fields)
            .unwrap()
            .unwrap();
        assert!(tx.committed);
        let pn = tx.pending_notes.get("id1").unwrap();
        assert_eq!(pn.effective_fields()["stars"], FieldValue::Number(4.0));
    }

    #[test]
    fn test_select_field_not_set_by_hook_is_absent_from_effective_fields() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("S2", #{ version: 1,
                fields: [ #{ name: "status", type: "select", options: ["A", "B"] } ],
                on_save: |note| {
                    // deliberately do NOT set status
                    commit();
                }
            });
        "#, "test").unwrap();

        // In the gated model, effective_fields() = original_fields merged with pending_fields.
        // Since neither contain "status", it is absent from the output.
        let fields = BTreeMap::new(); // no status field
        let tx = registry
            .run_on_save_hook("S2", "id1", "S2", "title", &fields)
            .unwrap()
            .unwrap();
        assert!(tx.committed);
        let pn = tx.pending_notes.get("id1").unwrap();
        // Field was not in submitted fields and hook did not set it — absent from output.
        assert_eq!(pn.effective_fields().get("status"), None);
    }

    #[test]
    fn test_rating_field_not_set_by_hook_is_absent_from_effective_fields() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("R2", #{ version: 1,
                fields: [ #{ name: "stars", type: "rating", max: 5 } ],
                on_save: |note| {
                    // deliberately do NOT set stars
                    commit();
                }
            });
        "#, "test").unwrap();

        // In the gated model, effective_fields() = original_fields merged with pending_fields.
        // Since neither contain "stars", it is absent from the output.
        let fields = BTreeMap::new(); // no stars field
        let tx = registry
            .run_on_save_hook("R2", "id1", "R2", "title", &fields)
            .unwrap()
            .unwrap();
        assert!(tx.committed);
        let pn = tx.pending_notes.get("id1").unwrap();
        // Field was not in submitted fields and hook did not set it — absent from output.
        assert_eq!(pn.effective_fields().get("stars"), None);
    }

    // ── children_sort ───────────────────────────────────────────────────────

    #[test]
    fn test_children_sort_defaults_to_none() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("SortTest", #{ version: 1,
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
            schema("SortAsc", #{ version: 1,
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
            schema("SortDesc", #{ version: 1,
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
            "/../templates/book_collection.rhai"
        )), "Book Collection Views").expect("Book presentation script should load");
        registry.load_script(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../templates/book_collection.schema.rhai"
        )), "Book Collection").expect("Book schema script should load");
        registry.resolve_bindings();

        let mut fields = BTreeMap::new();
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
        let tx = result.unwrap().unwrap();
        assert!(tx.committed);
        let pn = tx.pending_notes.get("id1").unwrap();
        assert_eq!(pn.effective_title(), "Herbert: Dune");
        assert_eq!(pn.effective_fields()["read_duration"], crate::FieldValue::Text(String::new()));
    }

    // ── render_default_view on ScriptRegistry ───────────────────────────────

    #[test]
    fn test_script_registry_render_default_view_textarea_markdown() {
        use crate::{FieldValue, Note};
        use std::collections::BTreeMap;

        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Memo", #{ version: 1,
                fields: [
                    #{ name: "body", type: "textarea", required: false }
                ]
            });
        "#, "test").unwrap();

        let mut fields = BTreeMap::new();
        fields.insert("body".into(), FieldValue::Text("**important**".into()));
        let note = Note {
            id: "n1".into(), title: "Test".into(), node_type: "Memo".into(),
            parent_id: None, position: 0.0, created_at: 0, modified_at: 0,
            created_by: String::new(), modified_by: String::new(), fields, is_expanded: false, tags: vec![], schema_version: 1,
        };

        let html = registry.render_default_view(&note, &Default::default(), &[]);
        assert!(html.contains("<strong>important</strong>"), "got: {html}");
    }

    // ── markdown() Rhai host function ───────────────────────────────────────

    #[test]
    fn test_markdown_rhai_function_renders_bold() {
        let registry = ScriptRegistry::new().unwrap();
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
            register_view("LinkTest", "Default", |note| {
                let target = #{ id: "target-id-123", title: "Target Note", fields: #{}, node_type: "TextNote" };
                link_to(target)
            });
        "#, "test_views.rhai").unwrap();
        registry.load_script(r#"
            schema("LinkTest", #{ version: 1,
                fields: [#{ name: "ref_id", type: "text" }],
            });
        "#, "test.schema.rhai").unwrap();
        registry.resolve_bindings();

        let note = Note {
            id: "note-1".to_string(),
            node_type: "LinkTest".to_string(),
            title: "Test".to_string(),
            parent_id: None,
            position: 0.0,
            created_at: 0,
            modified_at: 0,
            created_by: String::new(),
            modified_by: String::new(),
            fields: BTreeMap::new(),
            is_expanded: false, tags: vec![], schema_version: 1,
        };

        let context = QueryContext {
            notes_by_id: HashMap::new(),
            children_by_id: HashMap::new(),
            notes_by_type: HashMap::new(),
            notes_by_tag: HashMap::new(),
            notes_by_link_target: Default::default(),
            attachments_by_note_id: Default::default(),
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
            schema("Boom", #{ version: 1,
                fields: [ #{ name: "x", type: "text" } ],
                on_save: |note| {
                    throw "intentional runtime error";
                    note
                }
            });
            "#,
            "My Exploding Script",
        ).unwrap();

        let fields = BTreeMap::new();
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
            schema("Folder", #{ version: 1,
                fields: [
                    #{ name: "count", type: "number", required: false },
                ],
                on_add_child: |parent_note, child_note| {
                    let new_count = parent_note.fields["count"] + 1.0;
                    set_field(parent_note.id, "count", new_count);
                    set_title(parent_note.id, "Folder (" + new_count.to_int().to_string() + ")");
                    set_title(child_note.id, "Child from hook");
                    commit();
                }
            });
            schema("Item", #{ version: 1,
                fields: [
                    #{ name: "name", type: "text", required: false },
                ],
            });
        "#, "test").unwrap();

        let mut parent_fields = BTreeMap::new();
        parent_fields.insert("count".to_string(), FieldValue::Number(0.0));

        let mut child_fields = BTreeMap::new();
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
            schema("Plain", #{ version: 1,
                fields: [],
            });
        "#, "test").unwrap();

        let result = registry
            .run_on_add_child_hook(
                "Plain",
                "p-id", "Plain", "Title", &std::collections::BTreeMap::new(),
                "c-id", "Plain", "Child", &std::collections::BTreeMap::new(),
            )
            .unwrap();

        assert!(result.is_none(), "no hook registered should return None");
    }

    #[test]
    fn test_on_add_child_hook_returns_unit_gives_no_modifications() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Folder", #{ version: 1,
                fields: [],
                on_add_child: |parent_note, child_note| {
                    ()
                }
            });
            schema("Item", #{ version: 1,
                fields: [],
            });
        "#, "test").unwrap();

        let result = registry
            .run_on_add_child_hook(
                "Folder",
                "p-id", "Folder", "Title", &std::collections::BTreeMap::new(),
                "c-id", "Item",   "Child", &std::collections::BTreeMap::new(),
            )
            .unwrap();

        // Some(result) because hook exists, but both modifications are None
        let result = result.expect("hook present: should return Some");
        assert!(result.parent.is_none(), "unit return: parent should not be modified");
        assert!(result.child.is_none(),  "unit return: child should not be modified");
    }

    #[test]
    fn test_on_add_child_hook_parent_only_modification() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Folder", #{ version: 1,
                fields: [
                    #{ name: "count", type: "number", required: false },
                ],
                on_add_child: |parent_note, child_note| {
                    set_field(parent_note.id, "count", 5.0);
                    commit();
                }
            });
            schema("Item", #{ version: 1,
                fields: [],
            });
        "#, "test").unwrap();

        let mut parent_fields = BTreeMap::new();
        parent_fields.insert("count".to_string(), FieldValue::Number(0.0));

        let result = registry
            .run_on_add_child_hook(
                "Folder",
                "p-id", "Folder", "Folder", &parent_fields,
                "c-id", "Item",   "Untitled", &std::collections::BTreeMap::new(),
            )
            .unwrap();

        let result = result.expect("hook present: should return Some");
        let (_, p_fields) = result.parent.expect("parent modification expected");
        assert_eq!(p_fields["count"], FieldValue::Number(5.0));
        assert!(result.child.is_none(), "child should not be modified");
    }

    #[test]
    fn test_on_add_child_hook_child_only_modification() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Folder", #{ version: 1,
                fields: [],
                on_add_child: |parent_note, child_note| {
                    set_title(child_note.id, "Initialized by hook");
                    commit();
                }
            });
            schema("Item", #{ version: 1,
                fields: [],
            });
        "#, "test").unwrap();

        let result = registry
            .run_on_add_child_hook(
                "Folder",
                "p-id", "Folder", "Folder", &std::collections::BTreeMap::new(),
                "c-id", "Item",   "Untitled", &std::collections::BTreeMap::new(),
            )
            .unwrap();

        let result = result.expect("hook present: should return Some");
        assert!(result.parent.is_none(), "parent should not be modified");
        let (c_title, _) = result.child.expect("child modification expected");
        assert_eq!(c_title, "Initialized by hook");
    }

    #[test]
    fn test_on_add_child_hook_runtime_error_includes_script_name() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Folder", #{ version: 1,
                fields: [],
                on_add_child: |parent_note, child_note| {
                    throw "deliberate error";
                }
            });
            schema("Item", #{ version: 1,
                fields: [],
            });
        "#, "my_test_script").unwrap();

        let err = registry
            .run_on_add_child_hook(
                "Folder",
                "p-id", "Folder", "Title", &std::collections::BTreeMap::new(),
                "c-id", "Item",   "Child", &std::collections::BTreeMap::new(),
            )
            .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("my_test_script"), "error should include script name, got: {msg}");
        assert!(msg.contains("on_add_child"), "error should mention hook name, got: {msg}");
    }

    #[test]
    fn test_on_add_child_hook_old_style_returns_helpful_error() {
        // A hook that returns a map (old-style) should be rejected with a migration message.
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Folder", #{ version: 1,
                fields: [
                    #{ name: "count", type: "number", required: false },
                ],
                on_add_child: |parent_note, child_note| {
                    parent_note.fields["count"] = 1.0;
                    #{ parent: parent_note, child: child_note }
                }
            });
            schema("Item", #{ version: 1,
                fields: [],
            });
        "#, "my_script").unwrap();

        let mut parent_fields = BTreeMap::new();
        parent_fields.insert("count".to_string(), FieldValue::Number(0.0));

        let err = registry
            .run_on_add_child_hook(
                "Folder",
                "p-id", "Folder", "Title", &parent_fields,
                "c-id", "Item",   "Child", &std::collections::BTreeMap::new(),
            )
            .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("my_script"), "error should include script name, got: {msg}");
        assert!(msg.contains("gated model"), "error should mention migration, got: {msg}");
    }

    #[test]
    fn test_on_view_runtime_error_includes_script_name() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(
            r#"
            register_view("BoomView", "Default", |note| {
                throw "intentional runtime error";
                text("x")
            });
            "#,
            "My View Script",
        ).unwrap();
        registry.load_script(
            r#"
            schema("BoomView", #{ version: 1,
                fields: [],
            });
            "#,
            "BoomView Schema",
        ).unwrap();
        registry.resolve_bindings();

        use crate::Note;
        let note = Note {
            id: "n1".to_string(), node_type: "BoomView".to_string(),
            title: "T".to_string(), parent_id: None, position: 0.0,
            created_at: 0, modified_at: 0, created_by: String::new(), modified_by: String::new(),
            fields: BTreeMap::new(), is_expanded: false, tags: vec![], schema_version: 1,
        };
        let ctx = QueryContext {
            notes_by_id: HashMap::new(),
            children_by_id: HashMap::new(),
            notes_by_type: HashMap::new(),
            notes_by_tag: HashMap::new(),
            notes_by_link_target: Default::default(),
            attachments_by_note_id: Default::default(),
        };
        let err = registry.run_on_view_hook(&note, ctx).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("My View Script"),
            "error should include script name, got: {msg}"
        );
    }

    // ── tree actions ─────────────────────────────────────────────────────────

    #[test]
    fn test_add_tree_action_registers_entry() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("TextNote", #{ version: 1, fields: [] });
            register_menu("Sort Children", ["TextNote"], |note| { () });
        "#, "test_script").unwrap();
        registry.resolve_bindings();
        let map = registry.tree_action_map();
        assert_eq!(map.get("TextNote"), Some(&vec!["Sort Children".to_string()]));
    }

    #[test]
    fn test_tree_action_map_empty_before_load() {
        let registry = ScriptRegistry::new().unwrap();
        assert!(registry.tree_action_map().is_empty());
    }

    #[test]
    fn test_clear_all_removes_tree_actions() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("TextNote", #{ version: 1, fields: [] });
            register_menu("Do Thing", ["TextNote"], |note| { () });
        "#, "test_script").unwrap();
        registry.resolve_bindings();
        registry.clear_all();
        assert!(registry.tree_action_map().is_empty());
    }

    #[test]
    fn test_invoke_tree_action_hook_calls_callback() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("TextNote", #{ version: 1, fields: [] });
            register_menu("Noop", ["TextNote"], |note| { () });
        "#, "test_script").unwrap();
        registry.resolve_bindings();
        let note = crate::Note {
            id: "n1".into(), title: "Hello".into(),
            node_type: "TextNote".into(), parent_id: None,
            fields: std::collections::BTreeMap::new(), position: 0.0,
            created_at: 0, modified_at: 0, created_by: String::new(), modified_by: String::new(),
            is_expanded: false, tags: vec![], schema_version: 1,
        };
        let ctx = QueryContext {
            notes_by_id: Default::default(),
            children_by_id: Default::default(),
            notes_by_type: Default::default(),
            notes_by_tag: Default::default(),
            notes_by_link_target: Default::default(),
            attachments_by_note_id: Default::default(),
        };
        let result = registry.invoke_tree_action_hook("Noop", &note, ctx).unwrap();
        assert!(result.reorder.is_none(), "callback returning () should yield no reorder");
    }

    #[test]
    fn test_invoke_tree_action_returns_id_vec_when_callback_returns_array() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("TextNote", #{ version: 1, fields: [] });
            register_menu("Sort", ["TextNote"], |note| { ["id-b", "id-a"] });
        "#, "test_script").unwrap();
        registry.resolve_bindings();
        let note = crate::Note {
            id: "p1".into(), title: "Parent".into(),
            node_type: "TextNote".into(), parent_id: None,
            fields: std::collections::BTreeMap::new(), position: 0.0,
            created_at: 0, modified_at: 0, created_by: String::new(), modified_by: String::new(),
            is_expanded: false, tags: vec![], schema_version: 1,
        };
        let ctx = QueryContext {
            notes_by_id: Default::default(),
            children_by_id: Default::default(),
            notes_by_type: Default::default(),
            notes_by_tag: Default::default(),
            notes_by_link_target: Default::default(),
            attachments_by_note_id: Default::default(),
        };
        let result = registry.invoke_tree_action_hook("Sort", &note, ctx).unwrap();
        assert_eq!(result.reorder, Some(vec!["id-b".to_string(), "id-a".to_string()]));
    }

    #[test]
    fn test_invoke_tree_action_unknown_label_errors() {
        let registry = ScriptRegistry::new().unwrap();
        let note = crate::Note {
            id: "n1".into(), title: "T".into(),
            node_type: "TextNote".into(), parent_id: None,
            fields: std::collections::BTreeMap::new(), position: 0.0,
            created_at: 0, modified_at: 0, created_by: String::new(), modified_by: String::new(),
            is_expanded: false, tags: vec![], schema_version: 1,
        };
        let ctx = QueryContext {
            notes_by_id: Default::default(),
            children_by_id: Default::default(),
            notes_by_type: Default::default(),
            notes_by_tag: Default::default(),
            notes_by_link_target: Default::default(),
            attachments_by_note_id: Default::default(),
        };
        let err = registry.invoke_tree_action_hook("No Such Action", &note, ctx).unwrap_err();
        assert!(err.to_string().contains("unknown tree action"), "got: {err}");
    }

    #[test]
    fn test_invoke_tree_action_runtime_error_includes_script_name() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("TextNote", #{ version: 1, fields: [] });
            register_menu("Boom", ["TextNote"], |note| { throw "intentional"; });
        "#, "my_script").unwrap();
        registry.resolve_bindings();
        let note = crate::Note {
            id: "n1".into(), title: "T".into(),
            node_type: "TextNote".into(), parent_id: None,
            fields: std::collections::BTreeMap::new(), position: 0.0,
            created_at: 0, modified_at: 0, created_by: String::new(), modified_by: String::new(),
            is_expanded: false, tags: vec![], schema_version: 1,
        };
        let ctx = QueryContext {
            notes_by_id: Default::default(),
            children_by_id: Default::default(),
            notes_by_type: Default::default(),
            notes_by_tag: Default::default(),
            notes_by_link_target: Default::default(),
            attachments_by_note_id: Default::default(),
        };
        let err = registry.invoke_tree_action_hook("Boom", &note, ctx).unwrap_err();
        assert!(err.to_string().contains("my_script"), "error should include script name, got: {err}");
    }

    // ── create_child host function ────────────────────────────────────────────

    fn make_test_note(id: &str, node_type: &str) -> crate::Note {
        crate::Note {
            id: id.into(), title: "Test".into(),
            node_type: node_type.into(), parent_id: None,
            fields: Default::default(), position: 0.0,
            created_at: 0, modified_at: 0, created_by: String::new(), modified_by: String::new(),
            is_expanded: false, tags: vec![], schema_version: 1,
        }
    }

    fn make_empty_ctx() -> QueryContext {
        QueryContext {
            notes_by_id:              Default::default(),
            children_by_id:           Default::default(),
            notes_by_type:            Default::default(),
            notes_by_tag:             Default::default(),
            notes_by_link_target:     Default::default(),
            attachments_by_note_id:   Default::default(),
        }
    }

    #[test]
    fn test_create_note_returns_note_map_with_defaults() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Task", #{ version: 1,
                fields: [
                    #{ name: "status", type: "text", required: false },
                ]
            });
            register_menu("Make Task", ["Task"], |note| {
                let t = create_child(note.id, "Task");
                if t.node_type != "Task" { throw "node_type must be Task"; }
                if t.id == ""           { throw "id must not be empty"; }
                if t.fields.status != "" { throw "status must default to empty string"; }
                commit();
            });
        "#, "test").unwrap();
        registry.resolve_bindings();

        let note = make_test_note("parent1", "Task");
        let ctx  = make_empty_ctx();
        let result = registry.invoke_tree_action_hook("Make Task", &note, ctx).unwrap();
        let new_notes: Vec<_> = result.transaction.pending_notes.values()
            .filter(|p| p.is_new).collect();
        assert_eq!(new_notes.len(), 1, "one new pending note expected");
        assert_eq!(new_notes[0].node_type, "Task");
        assert_eq!(new_notes[0].parent_id.as_deref(), Some("parent1"));
    }

    // ── set_title / set_field on existing notes ──────────────────────────────

    #[test]
    fn test_update_note_queues_update_for_existing_note() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Task", #{ version: 1,
                fields: [
                    #{ name: "status", type: "text", required: false },
                ]
            });
            register_menu("Mark Done", ["Task"], |note| {
                set_title(note.id, "Completed");
                set_field(note.id, "status", "Done");
                commit();
            });
        "#, "test").unwrap();
        registry.resolve_bindings();

        let mut fields = BTreeMap::new();
        fields.insert("status".to_string(), FieldValue::Text("Open".to_string()));
        let mut note = make_test_note("n1", "Task");
        note.fields = fields;
        let result = registry.invoke_tree_action_hook("Mark Done", &note, make_empty_ctx()).unwrap();
        assert!(result.transaction.committed, "transaction should be committed");
        let pending = result.transaction.pending_notes.get("n1").expect("n1 should have a pending note");
        assert!(!pending.is_new, "should be an update to an existing note");
        assert_eq!(pending.effective_title(), "Completed");
        assert_eq!(
            pending.effective_fields().get("status"),
            Some(&crate::core::note::FieldValue::Text("Done".into())),
        );
    }

    #[test]
    fn test_update_note_on_inflight_note_updates_create_spec() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Task", #{ version: 1,
                fields: [#{ name: "status", type: "text", required: false }]
            });
            register_menu("New Task", ["Task"], |note| {
                let t = create_child(note.id, "Task");
                set_title(t.id, "My Task");
                set_field(t.id, "status", "Open");
                commit();
            });
        "#, "test").unwrap();
        registry.resolve_bindings();

        let note = make_test_note("parent1", "Task");
        let result = registry.invoke_tree_action_hook("New Task", &note, make_empty_ctx()).unwrap();

        let new_notes: Vec<_> = result.transaction.pending_notes.values()
            .filter(|p| p.is_new).collect();
        assert_eq!(new_notes.len(), 1, "one new pending note expected");
        assert!(result.transaction.committed, "transaction should be committed");
        let pending = new_notes[0];
        assert!(pending.is_new, "should be a new note create");
        assert_eq!(pending.effective_title(), "My Task");
        assert_eq!(
            pending.effective_fields().get("status"),
            Some(&crate::core::note::FieldValue::Text("Open".into())),
        );
    }

    #[test]
    fn test_get_children_sees_inflight_creates() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Task", #{ version: 1, fields: [] });
            register_menu("Verify Children", ["Task"], |note| {
                let t = create_child(note.id, "Task");
                let children = get_children(note.id);
                let found = children.filter(|c| c.id == t.id);
                if found.len() != 1 { throw "inflight note not visible in get_children"; }
                commit();
            });
        "#, "test").unwrap();
        registry.resolve_bindings();

        let note = make_test_note("parent1", "Task");
        registry.invoke_tree_action_hook("Verify Children", &note, make_empty_ctx()).unwrap();
    }

    #[test]
    fn test_get_note_sees_inflight_create() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Task", #{ version: 1, fields: [] });
            register_menu("Verify get_note", ["Task"], |note| {
                let t = create_child(note.id, "Task");
                let fetched = get_note(t.id);
                if fetched == () { throw "inflight note not visible via get_note"; }
                if fetched.id != t.id { throw "wrong note returned"; }
                commit();
            });
        "#, "test").unwrap();
        registry.resolve_bindings();

        let note = make_test_note("parent1", "Task");
        registry.invoke_tree_action_hook("Verify get_note", &note, make_empty_ctx()).unwrap();
    }

    #[test]
    fn test_on_view_note_has_tags() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            register_view("Tagged", "Default", |note| {
                let t = note.tags;
                text(t.len().to_string() + ":" + t[0])
            });
        "#, "test_views.rhai").unwrap();
        registry.load_script(r#"
            schema("Tagged", #{ version: 1,
                fields: [],
            });
        "#, "test.schema.rhai").unwrap();
        registry.resolve_bindings();

        let note = Note {
            id: "n1".to_string(), node_type: "Tagged".to_string(),
            title: "T".to_string(), parent_id: None, position: 0.0,
            created_at: 0, modified_at: 0, created_by: String::new(), modified_by: String::new(),
            fields: std::collections::BTreeMap::new(), is_expanded: false,
            tags: vec!["rust".to_string(), "notes".to_string()], schema_version: 1,
        };
        let ctx = QueryContext {
            notes_by_id: std::collections::HashMap::new(),
            children_by_id: std::collections::HashMap::new(),
            notes_by_type: std::collections::HashMap::new(),
            notes_by_tag: std::collections::HashMap::new(),
            notes_by_link_target: Default::default(),
            attachments_by_note_id: Default::default(),
        };
        let html = registry.run_on_view_hook(&note, ctx).unwrap().unwrap();
        assert!(html.contains("2:rust"), "expected '2:rust' in output, got: {html}");
    }

    #[test]
    fn test_today_returns_yyyy_mm_dd() {
        let mut registry = ScriptRegistry::new().unwrap();
        // Wrap today() in an on_save hook so we test it through the normal hook path
        registry.load_script(r#"
            schema("DateTest", #{ version: 1,
                fields: [#{ name: "dummy", type: "text", required: false }],
                on_save: |note| {
                    set_title(note.id, today());
                    commit();
                }
            });
        "#, "test").unwrap();

        let tx = registry
            .run_on_save_hook("DateTest", "id1", "DateTest", "", &BTreeMap::new())
            .unwrap()
            .unwrap();
        assert!(tx.committed);
        let title = tx.pending_notes.get("id1").unwrap().effective_title().to_string();
        // Must be exactly 10 chars: YYYY-MM-DD
        assert_eq!(title.len(), 10, "expected YYYY-MM-DD (10 chars), got: {title}");
        assert_eq!(&title[4..5], "-", "missing year-month separator: {title}");
        assert_eq!(&title[7..8], "-", "missing month-day separator: {title}");
    }

    #[test]
    fn test_zettel_on_save_sets_date_title() {
        use crate::FieldValue;
        let mut registry = ScriptRegistry::new().unwrap();
        // Inline the exact on_save logic from templates/zettelkasten.rhai
        registry.load_script(r#"
            schema("ZettelTest", #{ version: 1,
                title_can_edit: false,
                fields: [#{ name: "body", type: "textarea", required: false }],
                on_save: |note| {
                    let body = note.fields["body"] ?? "";
                    let words = body.split(" ").filter(|w| w != "");
                    let snippet = if words.len() == 0 {
                        "Untitled"
                    } else {
                        let take = if words.len() > 6 { 6 } else { words.len() };
                        let s = ""; let i = 0;
                        while i < take { s += words[i] + " "; i += 1; }
                        s.trim();
                        if words.len() > 6 { s + " …" } else { s }
                    };
                    set_title(note.id, today() + " — " + snippet);
                    commit();
                }
            });
        "#, "test").unwrap();

        let mut fields = BTreeMap::new();
        fields.insert("body".to_string(),
            FieldValue::Text("Emergence is when simple rules produce complex behaviour".to_string()));

        let tx = registry
            .run_on_save_hook("ZettelTest", "id1", "ZettelTest", "", &fields)
            .unwrap().unwrap();
        assert!(tx.committed);
        let title = tx.pending_notes.get("id1").unwrap().effective_title().to_string();

        // Title must start with YYYY-MM-DD (10 chars, dashes at [4] and [7])
        assert_eq!(&title[4..5], "-", "missing year-month separator: {title}");
        assert_eq!(&title[7..8], "-", "missing month-day separator: {title}");
        // Must contain the first 6 words
        assert!(title.contains("Emergence is when simple rules produce"),
            "snippet missing: {title}");
        // Body has 8 words — title must end with ellipsis
        assert!(title.ends_with('…'), "expected truncation ellipsis: {title}");
    }

    #[test]
    fn test_zettel_on_save_empty_body_uses_untitled() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("ZettelEmpty", #{ version: 1,
                title_can_edit: false,
                fields: [#{ name: "body", type: "textarea", required: false }],
                on_save: |note| {
                    let body = note.fields["body"] ?? "";
                    let words = body.split(" ").filter(|w| w != "");
                    let snippet = if words.len() == 0 {
                        "Untitled"
                    } else {
                        let take = if words.len() > 6 { 6 } else { words.len() };
                        let s = ""; let i = 0;
                        while i < take { s += words[i] + " "; i += 1; }
                        s.trim();
                        if words.len() > 6 { s + " …" } else { s }
                    };
                    set_title(note.id, today() + " — " + snippet);
                    commit();
                }
            });
        "#, "test").unwrap();

        let tx = registry
            .run_on_save_hook("ZettelEmpty", "id2", "ZettelEmpty", "", &std::collections::BTreeMap::new())
            .unwrap().unwrap();
        assert!(tx.committed);
        let title = tx.pending_notes.get("id2").unwrap().effective_title().to_string();
        assert!(title.contains("Untitled"), "expected Untitled fallback: {title}");
        // Must still have the date prefix
        assert_eq!(&title[4..5], "-", "missing date separator in untitled title: {title}");
    }

    #[test]
    fn test_default_field_for_note_link_is_none() {
        let schema = Schema {
            name: "Test".to_string(),
            fields: vec![FieldDefinition {
                name: "linked_note".to_string(),
                field_type: "note_link".to_string(),
                required: false,
                can_view: true,
                can_edit: true,
                options: vec![],
                max: 0,
                target_type: None,
                show_on_hover: false,
                allowed_types: vec![], validate: None,
            }],
            title_can_view: true,
            title_can_edit: true,
            children_sort: "none".to_string(),
            allowed_parent_types: vec![],
            allowed_children_types: vec![],
            allow_attachments: false,
            attachment_types: vec![], field_groups: vec![], ast: None, version: 1, migrations: std::collections::BTreeMap::new(),
        };
        let defaults = schema.default_fields();
        assert!(matches!(defaults.get("linked_note"), Some(FieldValue::NoteLink(None))));
    }

    #[test]
    fn test_parse_note_link_target_type() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Task", #{ version: 1,
                fields: [
                    #{ name: "project", type: "note_link", target_type: "Project" }
                ]
            });
        "#, "test").unwrap();
        let fields = get_schema_fields_for_test(&registry, "Task");
        assert_eq!(fields[0].field_type, "note_link");
        assert_eq!(fields[0].target_type, Some("Project".to_string()));
    }

    // ── on_hover hook ────────────────────────────────────────────────────────

    #[test]
    fn test_has_hover_hook_registered() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            register_hover("WithHover", |note| { "hover: " + note.title });
        "#, "HoverHook_views.rhai").unwrap();
        registry.load_script(r#"
            schema("WithHover", #{ version: 1,
                fields: [#{ name: "body", type: "text" }],
            });
        "#, "HoverHook.schema.rhai").unwrap();
        registry.resolve_bindings();
        assert!(registry.has_hover("WithHover"));
        assert!(!registry.has_hover("Nonexistent"));
    }

    #[test]
    fn test_run_on_hover_hook_returns_html() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            register_hover("HoverRun", |note| { "HOVER:" + note.title });
        "#, "HoverRun_views.rhai").unwrap();
        registry.load_script(r#"
            schema("HoverRun", #{ version: 1,
                fields: [#{ name: "body", type: "text" }],
            });
        "#, "HoverRun.schema.rhai").unwrap();
        registry.resolve_bindings();
        let note = crate::Note {
            id: "id1".into(), title: "Test Note".into(), node_type: "HoverRun".into(),
            parent_id: None, position: 0.0, created_at: 0, modified_at: 0,
            created_by: String::new(), modified_by: String::new(),
            fields: std::collections::BTreeMap::new(), is_expanded: false, tags: vec![], schema_version: 1,
        };
        let ctx = QueryContext {
            notes_by_id: Default::default(), children_by_id: Default::default(),
            notes_by_type: Default::default(), notes_by_tag: Default::default(),
            notes_by_link_target: Default::default(),
            attachments_by_note_id: Default::default(),
        };
        let html = registry.run_on_hover_hook(&note, ctx).unwrap();
        assert_eq!(html, Some("HOVER:Test Note".to_string()));
    }

    // ── show_on_hover ────────────────────────────────────────────────────────

    #[test]
    fn test_field_show_on_hover_parsed() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            // @name: HoverTest
            schema("HoverTest", #{ version: 1,
                fields: [
                    #{ name: "summary", type: "text", show_on_hover: true },
                    #{ name: "internal", type: "text" },
                ],
            });
        "#, "HoverTest").unwrap();
        let schema = registry.get_schema("HoverTest").unwrap();
        assert!(schema.fields[0].show_on_hover);
        assert!(!schema.fields[1].show_on_hover);
    }

    #[test]
    fn test_get_attachments_returns_array_of_maps() {
        use crate::core::attachment::AttachmentMeta;

        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            register_view("PhotoNote", "Default", |note| {
                let atts = get_attachments(note.id);
                if atts.len() == 0 { return text("none"); }
                let first = atts[0];
                text(first.id + "|" + first.filename)
            });
        "#, "test_views.rhai").unwrap();
        registry.load_script(r#"
            schema("PhotoNote", #{ version: 1,
                fields: [],
            });
        "#, "test.schema.rhai").unwrap();
        registry.resolve_bindings();

        let note = make_test_note("note-1", "PhotoNote");

        let mut ctx = make_empty_ctx();
        ctx.attachments_by_note_id.insert(
            "note-1".to_string(),
            vec![AttachmentMeta {
                id: "att-uuid-1".to_string(),
                note_id: "note-1".to_string(),
                filename: "photo.png".to_string(),
                mime_type: Some("image/png".to_string()),
                size_bytes: 100,
                hash_sha256: "abc".to_string(),
                salt: "00".repeat(32),
                created_at: 0,
            }],
        );

        let html = registry.run_on_view_hook(&note, ctx).unwrap().unwrap();
        assert!(html.contains("att-uuid-1"), "got: {html}");
        assert!(html.contains("photo.png"), "got: {html}");
    }

    #[test]
    fn test_rhai_display_image_with_uuid_in_field() {
        use crate::core::note::{FieldValue, Note};

        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            register_view("PhotoNote", "Default", |note| {
                display_image(note.fields["photo"], 300, "My alt")
            });
        "#, "test_views.rhai").unwrap();
        registry.load_script(r#"
            schema("PhotoNote", #{ version: 1,
                fields: [#{ name: "photo", type: "file", required: false }],
            });
        "#, "test.schema.rhai").unwrap();
        registry.resolve_bindings();

        let mut fields = BTreeMap::new();
        fields.insert("photo".to_string(), FieldValue::File(Some("abc-uuid-123".to_string())));
        let note = Note {
            id: "n1".to_string(), node_type: "PhotoNote".to_string(),
            title: "T".to_string(), parent_id: None, fields, tags: vec![], schema_version: 1,
            created_at: 0, modified_at: 0, position: 0.0,
            created_by: String::new(), modified_by: String::new(), is_expanded: false,
        };

        let html = registry.run_on_view_hook(&note, make_empty_ctx()).unwrap().unwrap();
        assert!(html.contains("data-kn-attach-id=\"abc-uuid-123\""), "got: {html}");
        assert!(html.contains("data-kn-width=\"300\""), "got: {html}");
    }

    #[test]
    fn test_rhai_display_image_unset_field_shows_error() {
        use crate::core::note::{FieldValue, Note};

        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            register_view("PhotoNote", "Default", |note| {
                display_image(note.fields["photo"], 0, "")
            });
        "#, "test_views.rhai").unwrap();
        registry.load_script(r#"
            schema("PhotoNote", #{ version: 1,
                fields: [#{ name: "photo", type: "file", required: false }],
            });
        "#, "test.schema.rhai").unwrap();
        registry.resolve_bindings();

        let mut fields = BTreeMap::new();
        fields.insert("photo".to_string(), FieldValue::File(None));
        let note = Note {
            id: "n2".to_string(), node_type: "PhotoNote".to_string(),
            title: "T".to_string(), parent_id: None, fields, tags: vec![], schema_version: 1,
            created_at: 0, modified_at: 0, position: 0.0,
            created_by: String::new(), modified_by: String::new(), is_expanded: false,
        };

        let html = registry.run_on_view_hook(&note, make_empty_ctx()).unwrap().unwrap();
        assert!(html.contains("kn-image-error"), "got: {html}");
    }

    // ── embed_media() Rhai host function ────────────────────────────────────

    #[test]
    fn test_embed_media_rhai_function_youtube() {
        let registry = ScriptRegistry::new().unwrap();
        let result = registry
            .engine
            .eval::<String>(
                r#"embed_media("https://www.youtube.com/watch?v=dQw4w9WgXcQ")"#,
            )
            .expect("eval failed");
        assert!(result.contains("data-kn-embed-type=\"youtube\""), "got: {result}");
        assert!(result.contains("data-kn-embed-id=\"dQw4w9WgXcQ\""), "got: {result}");
    }

    #[test]
    fn test_embed_media_rhai_function_unknown_returns_empty() {
        let registry = ScriptRegistry::new().unwrap();
        let result = registry
            .engine
            .eval::<String>(r#"embed_media("https://example.com")"#)
            .expect("eval failed");
        assert!(result.is_empty(), "got: {result}");
    }

    // ── validate_field ────────────────────────────────────────────────────────

    #[test]
    fn test_validate_field_returns_error_on_invalid() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Rated", #{ version: 1,
                fields: [
                    #{
                        name: "score", type: "number", required: false,
                        validate: |v| if v < 0.0 { "Must be positive" } else { () },
                    }
                ]
            });
        "#, "validate_test").unwrap();

        let err = registry.validate_field(
            "Rated", "score",
            &crate::core::note::FieldValue::Number(-1.0)
        ).unwrap();
        assert_eq!(err, Some("Must be positive".into()));
    }

    #[test]
    fn test_validate_field_returns_none_on_valid() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Rated", #{ version: 1,
                fields: [
                    #{
                        name: "score", type: "number", required: false,
                        validate: |v| if v < 0.0 { "Must be positive" } else { () },
                    }
                ]
            });
        "#, "validate_test").unwrap();

        let err = registry.validate_field(
            "Rated", "score",
            &crate::core::note::FieldValue::Number(5.0)
        ).unwrap();
        assert_eq!(err, None);
    }

    #[test]
    fn test_validate_field_no_closure_returns_none() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Plain", #{ version: 1,
                fields: [ #{ name: "title", type: "text", required: false } ]
            });
        "#, "validate_test").unwrap();

        let err = registry.validate_field(
            "Plain", "title",
            &crate::core::note::FieldValue::Text("anything".into())
        ).unwrap();
        assert_eq!(err, None);
    }

    // ── evaluate_group_visibility ─────────────────────────────────────────────

    #[test]
    fn test_evaluate_group_visibility_with_closure() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Typed", #{ version: 1,
                fields: [],
                field_groups: [
                    #{
                        name: "Special Group",
                        fields: [],
                        visible: |fields| fields["kind"] == "special",
                    }
                ]
            });
        "#, "visibility_test").unwrap();

        let mut fields_special = std::collections::BTreeMap::new();
        fields_special.insert("kind".into(), crate::core::note::FieldValue::Text("special".into()));
        let vis = registry.evaluate_group_visibility("Typed", &fields_special).unwrap();
        assert_eq!(vis.get("Special Group"), Some(&true));

        let mut fields_other = std::collections::BTreeMap::new();
        fields_other.insert("kind".into(), crate::core::note::FieldValue::Text("other".into()));
        let vis2 = registry.evaluate_group_visibility("Typed", &fields_other).unwrap();
        assert_eq!(vis2.get("Special Group"), Some(&false));
    }

    #[test]
    fn test_evaluate_group_visibility_no_closure_always_true() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Simple", #{ version: 1,
                fields: [],
                field_groups: [
                    #{ name: "Always Visible", fields: [] }
                ]
            });
        "#, "visibility_test").unwrap();

        let vis = registry.evaluate_group_visibility("Simple", &Default::default()).unwrap();
        assert_eq!(vis.get("Always Visible"), Some(&true));
    }

    // ── set_field validate hard error ─────────────────────────────────────────

    #[test]
    fn test_set_field_validate_hard_error() {
        // When set_field is called from an on_save hook with a value that fails
        // the field's validate closure, the hook should abort with a hard error
        // that contains the validation message.
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Validated", #{ version: 1,
                fields: [
                    #{
                        name: "score", type: "number", required: false,
                        validate: |v| if v < 0.0 { "Negative!" } else { () },
                    }
                ],
                on_save: |note| {
                    set_field(note.id, "score", -1.0);
                    commit();
                }
            });
        "#, "test").unwrap();

        let result = registry.run_on_save_hook(
            "Validated",
            "n1",
            "Validated",
            "Test Note",
            &Default::default(),
        );

        assert!(result.is_err(), "Expected hard error from validate in set_field");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("Negative!"),
            "Expected 'Negative!' in error, got: {msg}"
        );
    }

    #[test]
    fn test_set_field_validate_passes_when_valid() {
        // When set_field is called with a value that passes validation, the hook
        // should succeed normally.
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            schema("Validated", #{ version: 1,
                fields: [
                    #{
                        name: "score", type: "number", required: false,
                        validate: |v| if v < 0.0 { "Negative!" } else { () },
                    }
                ],
                on_save: |note| {
                    set_field(note.id, "score", 5.0);
                    commit();
                }
            });
        "#, "test").unwrap();

        let result = registry.run_on_save_hook(
            "Validated",
            "n1",
            "Validated",
            "Test Note",
            &Default::default(),
        );

        assert!(result.is_ok(), "Expected success with valid value, got: {:?}", result.err());
        let tx = result.unwrap().expect("hook should return a transaction");
        assert!(tx.committed, "Transaction should be committed");
        let pending = tx.pending_notes.get("n1").expect("note should be in tx");
        assert_eq!(
            pending.pending_fields.get("score"),
            Some(&FieldValue::Number(5.0)),
        );
    }

}
