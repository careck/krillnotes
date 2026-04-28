// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Rhai engine setup and configuration for ScriptRegistry.

use super::*;

impl ScriptRegistry {
    /// Creates a new, empty registry with no scripts loaded.
    ///
    /// Use [`starter_scripts()`](Self::starter_scripts) to get the bundled
    /// starter scripts for seeding a new workspace.
    pub fn new() -> Result<Self> {
        let mut engine = Engine::new();
        engine.set_max_operations(200_000);
        engine.set_max_call_levels(64);
        engine.set_max_string_size(1_000_000);
        engine.set_max_array_size(100_000);
        let schema_registry = schema::SchemaRegistry::new();
        let current_loading_ast: Arc<Mutex<Option<AST>>> = Arc::new(Mutex::new(None));
        let current_loading_script_name: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let current_loading_category: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let schema_owners: Arc<Mutex<HashMap<String, String>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // Register register_view/hover/menu — deferred binding functions
        let deferred_arc = schema_registry.deferred_bindings_arc();
        let view_ast_arc = Arc::clone(&current_loading_ast);
        let view_name_arc = Arc::clone(&current_loading_script_name);

        // 3-arg form: register_view(type, label, closure)
        let d1 = Arc::clone(&deferred_arc);
        let a1 = Arc::clone(&view_ast_arc);
        let n1 = Arc::clone(&view_name_arc);
        engine.register_fn(
            "register_view",
            move |target_type: String,
                  label: String,
                  fn_ptr: FnPtr|
                  -> std::result::Result<Dynamic, Box<EvalAltResult>> {
                let ast = a1
                    .lock()
                    .unwrap()
                    .clone()
                    .ok_or_else(|| -> Box<EvalAltResult> {
                        "register_view() called outside of load_script"
                            .to_string()
                            .into()
                    })?;
                let script_name = n1.lock().unwrap().clone().unwrap_or_default();
                d1.lock().unwrap().push(DeferredBinding {
                    kind: BindingKind::View,
                    target_schema: target_type,
                    fn_ptr,
                    ast: Arc::new(ast),
                    script_name,
                    display_first: false,
                    label: Some(label),
                    applies_to: vec![],
                });
                Ok(Dynamic::UNIT)
            },
        );

        // 4-arg form: register_view(type, label, options, closure)
        let d2 = Arc::clone(&deferred_arc);
        let a2 = Arc::clone(&view_ast_arc);
        let n2 = Arc::clone(&view_name_arc);
        engine.register_fn(
            "register_view",
            move |target_type: String,
                  label: String,
                  options: rhai::Map,
                  fn_ptr: FnPtr|
                  -> std::result::Result<Dynamic, Box<EvalAltResult>> {
                let ast = a2
                    .lock()
                    .unwrap()
                    .clone()
                    .ok_or_else(|| -> Box<EvalAltResult> {
                        "register_view() called outside of load_script"
                            .to_string()
                            .into()
                    })?;
                let script_name = n2.lock().unwrap().clone().unwrap_or_default();
                let display_first = options
                    .get("display_first")
                    .and_then(|v| v.as_bool().ok())
                    .unwrap_or(false);
                d2.lock().unwrap().push(DeferredBinding {
                    kind: BindingKind::View,
                    target_schema: target_type,
                    fn_ptr,
                    ast: Arc::new(ast),
                    script_name,
                    display_first,
                    label: Some(label),
                    applies_to: vec![],
                });
                Ok(Dynamic::UNIT)
            },
        );

        // register_hover(target_type, closure)
        let d3 = Arc::clone(&deferred_arc);
        let a3 = Arc::clone(&view_ast_arc);
        let n3 = Arc::clone(&view_name_arc);
        engine.register_fn(
            "register_hover",
            move |target_type: String,
                  fn_ptr: FnPtr|
                  -> std::result::Result<Dynamic, Box<EvalAltResult>> {
                let ast = a3
                    .lock()
                    .unwrap()
                    .clone()
                    .ok_or_else(|| -> Box<EvalAltResult> {
                        "register_hover() called outside of load_script"
                            .to_string()
                            .into()
                    })?;
                let script_name = n3.lock().unwrap().clone().unwrap_or_default();
                d3.lock().unwrap().push(DeferredBinding {
                    kind: BindingKind::Hover,
                    target_schema: target_type,
                    fn_ptr,
                    ast: Arc::new(ast),
                    script_name,
                    display_first: false,
                    label: None,
                    applies_to: vec![],
                });
                Ok(Dynamic::UNIT)
            },
        );

        // register_menu(label, target_types, closure)
        let d4 = Arc::clone(&deferred_arc);
        let a4 = Arc::clone(&view_ast_arc);
        let n4 = Arc::clone(&view_name_arc);
        engine.register_fn(
            "register_menu",
            move |label: String,
                  types: rhai::Array,
                  fn_ptr: FnPtr|
                  -> std::result::Result<Dynamic, Box<EvalAltResult>> {
                let ast = a4
                    .lock()
                    .unwrap()
                    .clone()
                    .ok_or_else(|| -> Box<EvalAltResult> {
                        "register_menu() called outside of load_script"
                            .to_string()
                            .into()
                    })?;
                let script_name = n4.lock().unwrap().clone().unwrap_or_default();
                let applies_to: Vec<String> = types
                    .into_iter()
                    .filter_map(|v| v.into_string().ok())
                    .collect();
                d4.lock().unwrap().push(DeferredBinding {
                    kind: BindingKind::Menu,
                    target_schema: String::new(),
                    fn_ptr,
                    ast: Arc::new(ast),
                    script_name,
                    display_first: false,
                    label: Some(label),
                    applies_to,
                });
                Ok(Dynamic::UNIT)
            },
        );

        // Register schema() host function — writes schema and schema-bound hooks into SchemaRegistry.
        let schemas_arc = schema_registry.schemas_arc();
        let on_save_arc = schema_registry.on_save_hooks_arc();
        let on_add_child_arc = schema_registry.on_add_child_hooks_arc();
        let schema_ast_arc = Arc::clone(&current_loading_ast);
        let schema_name_arc = Arc::clone(&current_loading_script_name);
        let schema_cat_arc = Arc::clone(&current_loading_category);
        let schema_owners_arc = Arc::clone(&schema_owners);
        engine.register_fn("schema", move |name: String, def: rhai::Map| -> std::result::Result<Dynamic, Box<EvalAltResult>> {
            // Gate: schema() can only be called from schema-category scripts.
            let cat = schema_cat_arc.lock().unwrap();
            if cat.as_deref() == Some("library") {
                return Err("schema() can only be called from schema-category scripts, not library scripts".into());
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
        engine.register_fn(
            "get_schema_fields",
            move |name: String| -> std::result::Result<Dynamic, Box<EvalAltResult>> {
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
                    map.insert(
                        "options".into(),
                        Dynamic::from(
                            field
                                .options
                                .iter()
                                .map(|s| Dynamic::from(s.clone()))
                                .collect::<rhai::Array>(),
                        ),
                    );
                    map.insert("max".into(), Dynamic::from(field.max));
                    arr.push(Dynamic::from(map));
                }
                Ok(Dynamic::from(arr))
            },
        );

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
                guard
                    .as_ref()
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
                    tx.pending_notes
                        .get(&id)
                        .filter(|p| p.is_new)
                        .map(pending_note_to_dynamic)
                })
            });
            if let Some(dyn_note) = found {
                return dyn_note;
            }
            // Fall back to snapshot.
            let guard = qc2.lock().unwrap();
            guard
                .as_ref()
                .and_then(|ctx| ctx.notes_by_id.get(&id).cloned())
                .unwrap_or(Dynamic::UNIT)
        });

        // Register get_notes_of_type() — returns all notes of a given schema type.
        let qc3 = Arc::clone(&query_context);
        engine.register_fn(
            "get_notes_of_type",
            move |node_type: String| -> rhai::Array {
                let guard = qc3.lock().unwrap();
                guard
                    .as_ref()
                    .and_then(|ctx| ctx.notes_by_type.get(&node_type).cloned())
                    .unwrap_or_default()
            },
        );

        // Register get_notes_for_tag(tags) — returns notes carrying any of the given tags (OR).
        let qc4 = Arc::clone(&query_context);
        engine.register_fn(
            "get_notes_for_tag",
            move |tags: rhai::Array| -> rhai::Array {
                let guard = qc4.lock().unwrap();
                let Some(ctx) = guard.as_ref() else {
                    return vec![];
                };
                let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                let mut result: rhai::Array = Vec::new();
                for tag_dyn in &tags {
                    let tag = tag_dyn.to_string();
                    if let Some(notes) = ctx.notes_by_tag.get(&tag) {
                        for note in notes {
                            // Extract the id to dedup; safe to clone Dynamic.
                            let id = note
                                .clone()
                                .try_cast::<rhai::Map>()
                                .and_then(|m| {
                                    m.get("id").and_then(|v| v.clone().into_string().ok())
                                })
                                .unwrap_or_default();
                            if seen.insert(id) {
                                result.push(note.clone());
                            }
                        }
                    }
                }
                result
            },
        );

        // Register get_notes_with_link(target_id) — returns all notes whose note_link field
        // points to the given target note ID.
        let qc5 = Arc::clone(&query_context);
        engine.register_fn(
            "get_notes_with_link",
            move |target_id: String| -> rhai::Array {
                let guard = qc5.lock().unwrap();
                guard
                    .as_ref()
                    .and_then(|ctx| ctx.notes_by_link_target.get(&target_id).cloned())
                    .unwrap_or_default()
            },
        );

        // Register get_attachments(note_id) — returns attachment metadata for a note.
        let qc6 = Arc::clone(&query_context);
        engine.register_fn("get_attachments", move |note_id: String| -> rhai::Array {
            let guard = qc6.lock().unwrap();
            guard
                .as_ref()
                .and_then(|ctx| ctx.attachments_by_note_id.get(&note_id).cloned())
                .unwrap_or_default()
                .into_iter()
                .map(|att| {
                    let mut m = rhai::Map::new();
                    m.insert("id".into(), Dynamic::from(att.id));
                    m.insert("filename".into(), Dynamic::from(att.filename));
                    m.insert(
                        "mime_type".into(),
                        att.mime_type.map(Dynamic::from).unwrap_or(Dynamic::UNIT),
                    );
                    m.insert("size_bytes".into(), Dynamic::from(att.size_bytes));
                    Dynamic::from(m)
                })
                .collect()
        });

        // create_child(parent_id, node_type) — available inside add_tree_action closures.
        // Queues a new pending note into the thread-local SaveTransaction and returns a note map.
        let create_child_schemas = schema_registry.clone();
        engine.register_fn(
            "create_child",
            move |parent_id: String,
                  node_type: String|
                  -> std::result::Result<Dynamic, Box<EvalAltResult>> {
                let default_fields = {
                    let schemas_arc = create_child_schemas.schemas_arc();
                    let registry = schemas_arc.lock().unwrap();
                    let schema =
                        registry
                            .get(&node_type)
                            .ok_or_else(|| -> Box<EvalAltResult> {
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
                map.insert("id".into(), Dynamic::from(note_id));
                map.insert("parent_id".into(), Dynamic::from(parent_id));
                map.insert("schema".into(), Dynamic::from(node_type));
                map.insert("title".into(), Dynamic::from(String::new()));
                map.insert("fields".into(), Dynamic::from(fields_map));
                map.insert("tags".into(), Dynamic::from(rhai::Array::new()));
                Ok(Dynamic::from_map(map))
            },
        );

        // ── Display helpers for on_view hooks ─────────────────────────────────
        engine.register_fn("table", display_helpers::table);
        engine.register_fn("section", display_helpers::section);
        engine.register_fn("stack", display_helpers::stack);
        engine.register_fn("columns", display_helpers::columns);
        engine.register_fn("field", display_helpers::field_row);
        engine.register_fn("fields", display_helpers::fields);
        engine.register_fn("heading", display_helpers::heading);
        engine.register_fn("text", display_helpers::view_text);
        engine.register_fn("list", display_helpers::list);
        engine.register_fn("badge", display_helpers::badge);
        engine.register_fn("badge", display_helpers::badge_colored);
        engine.register_fn("divider", display_helpers::divider);
        engine.register_fn("link_to", display_helpers::link_to);
        engine.register_fn("embed_media", |url: String| -> String {
            display_helpers::make_media_embed_html(&url)
        });
        let ctx_for_markdown = Arc::clone(&run_context);
        engine.register_fn("markdown", move |text: String| -> String {
            let guard = ctx_for_markdown.lock().expect("run_context poisoned");
            let maybe_context = guard
                .as_ref()
                .map(|ctx| (ctx.note.fields.clone(), ctx.attachments.clone()));
            drop(guard); // release lock before any further work
            let after_images = if let Some((fields, attachments)) = maybe_context {
                display_helpers::preprocess_image_blocks(&text, &fields, &attachments)
            } else {
                text
            };
            let processed = display_helpers::preprocess_media_embeds(&after_images);
            display_helpers::rhai_markdown_raw(processed)
        });
        engine.register_fn("render_tags", display_helpers::rhai_render_tags);

        engine.register_fn(
            "display_image",
            |uuid: Dynamic, width: i64, alt: String| -> String {
                match uuid.into_string() {
                    Ok(id) if !id.is_empty() => {
                        display_helpers::make_display_image_html(&id, width, &alt)
                    }
                    _ => "<span class=\"kn-image-error\">No image set</span>".to_string(),
                }
            },
        );

        engine.register_fn(
            "display_download_link",
            |uuid: Dynamic, label: String| -> String {
                match uuid.into_string() {
                    Ok(id) if !id.is_empty() => {
                        display_helpers::make_download_link_html(&id, &label)
                    }
                    _ => "<span class=\"kn-image-error\">No file set</span>".to_string(),
                }
            },
        );
        engine.register_fn("stars", display_helpers::rhai_stars_default);
        engine.register_fn("stars", display_helpers::rhai_stars);

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
            [
                std::any::TypeId::of::<rhai::ImmutableString>(),
                std::any::TypeId::of::<rhai::ImmutableString>(),
                std::any::TypeId::of::<Dynamic>(),
            ],
            move |ctx: NativeCallContext,
                  args: &mut [&mut Dynamic]|
                  -> std::result::Result<Dynamic, Box<EvalAltResult>> {
                let note_id = args[0].clone().cast::<rhai::ImmutableString>().to_string();
                let field_name = args[1].clone().cast::<rhai::ImmutableString>().to_string();
                let value = args[2].clone();

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
                        .and_then(|tx| tx.pending_notes.get(&note_id).map(|p| p.schema.clone()))
                });
                if let Some(node_type) = node_type_opt {
                    // Clone the data we need before releasing the lock.
                    let (validate_fn_opt, ast_opt) = {
                        let schemas = set_field_schemas.lock().unwrap();
                        if let Some(schema) = schemas.get(&node_type) {
                            let field_def = schema
                                .all_fields()
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
                                format!("set_field validate error for field '{}': {e}", field_name)
                                    .into()
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
            },
        );

        engine.register_fn(
            "set_title",
            |note_id: String, title: String| -> std::result::Result<Dynamic, Box<EvalAltResult>> {
                with_save_tx(|tx| tx.set_title(&note_id, title))?;
                Ok(Dynamic::UNIT)
            },
        );

        engine.register_fn(
            "set_checked",
            |note_id: String, checked: bool| -> std::result::Result<Dynamic, Box<EvalAltResult>> {
                with_save_tx(|tx| tx.set_checked(&note_id, checked))?;
                Ok(Dynamic::UNIT)
            },
        );

        // reject(message) — note-level soft error
        engine.register_fn(
            "reject",
            |message: String| -> std::result::Result<Dynamic, Box<EvalAltResult>> {
                with_save_tx(|tx| {
                    tx.reject_note(message);
                    Ok(())
                })?;
                Ok(Dynamic::UNIT)
            },
        );

        // reject(field, message) — field-pinned soft error
        engine.register_fn(
            "reject",
            |field: String, message: String| -> std::result::Result<Dynamic, Box<EvalAltResult>> {
                with_save_tx(|tx| {
                    tx.reject_field(field, message);
                    Ok(())
                })?;
                Ok(Dynamic::UNIT)
            },
        );

        engine.register_fn(
            "commit",
            || -> std::result::Result<Dynamic, Box<EvalAltResult>> {
                with_save_tx(|tx| {
                    tx.commit().map_err(|errors| {
                        let msgs: Vec<String> = errors
                            .iter()
                            .map(|e| match &e.field {
                                Some(f) => format!("{}: {}", f, e.message),
                                None => e.message.clone(),
                            })
                            .collect();
                        format!("Validation failed: {}", msgs.join("; "))
                    })
                })?;
                Ok(Dynamic::UNIT)
            },
        );

        let library_sources: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        Ok(Self {
            engine,
            current_loading_ast,
            current_loading_script_name,
            current_loading_category,
            library_sources,
            schema_owners,
            schema_registry,
            query_context,
            run_context,
        })
    }
}
