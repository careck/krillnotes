// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Schema definitions and the private schema store for Krillnotes note types.

use crate::{FieldValue, KrillnotesError, Result};
use crate::core::save_transaction::SaveTransaction;
use rhai::{Dynamic, Engine, FnPtr, Map, AST};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

/// A stored hook entry: the Rhai closure and the AST it was defined in.
#[derive(Clone, Debug)]
pub(super) struct HookEntry {
    pub(super) fn_ptr: FnPtr,
    pub(super) ast: AST,
    pub(super) script_name: String,
}

/// A registered custom view tab for a note type.
#[derive(Debug, Clone)]
pub struct ViewRegistration {
    pub label: String,
    pub display_first: bool,
    pub fn_ptr: FnPtr,
    pub ast: Arc<AST>,
    pub script_name: String,
}

/// A registered context menu action for note types.
#[derive(Debug, Clone)]
pub struct MenuRegistration {
    pub label: String,
    pub fn_ptr: FnPtr,
    pub ast: Arc<AST>,
    pub script_name: String,
}

/// A deferred binding queued during script loading, resolved after all scripts load.
#[derive(Debug, Clone)]
pub struct DeferredBinding {
    pub kind: BindingKind,
    pub target_schema: String,
    pub fn_ptr: FnPtr,
    pub ast: Arc<AST>,
    pub script_name: String,
    pub display_first: bool,
    pub label: Option<String>,
    pub applies_to: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum BindingKind {
    View,
    Hover,
    Menu,
}

/// A warning about an unresolved deferred binding.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptWarning {
    pub script_name: String,
    pub message: String,
}

/// Result returned by [`SchemaRegistry::run_on_add_child_hook`].
///
/// Each field is `Some((new_title, new_fields))` when the hook returned
/// modifications for that note, or `None` when the hook left it unchanged.
#[derive(Debug)]
pub struct AddChildResult {
    pub parent: Option<(String, BTreeMap<String, FieldValue>)>,
    pub child:  Option<(String, BTreeMap<String, FieldValue>)>,
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
    pub target_schema: Option<String>,
    /// When `true`, this field is included in the hover-tooltip simple-path renderer.
    /// Defaults to `false` (opt-in).
    #[serde(default)]
    pub show_on_hover: bool,
    /// MIME types accepted by `file` fields; empty means all types are allowed.
    /// Ignored for non-`file` field types.
    #[serde(default)]
    pub allowed_types: Vec<String>,
    /// Field-level validation closure. Receives the field value, returns `()`
    /// for valid or a `String` error message for invalid.
    #[serde(skip)]
    pub validate: Option<rhai::FnPtr>,
}

/// A named group of fields with optional conditional visibility.
#[derive(Debug, Clone)]
pub struct FieldGroup {
    pub name: String,
    pub fields: Vec<FieldDefinition>,
    /// Visibility closure: `|fields_map| -> bool`. `None` means always visible.
    pub visible: Option<rhai::FnPtr>,
    /// Initial collapsed state in the UI.
    pub collapsed: bool,
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
    pub allowed_parent_schemas: Vec<String>,
    /// Note types that this schema allows as direct children.
    /// Empty means no restriction (any child type is allowed here).
    pub allowed_children_schemas: Vec<String>,
    /// When `true`, the note-level attachments panel is shown for this schema.
    /// Defaults to `false` (opt-in).
    pub allow_attachments: bool,
    /// MIME types accepted by the note-level attachments panel; empty means all types are allowed.
    /// Ignored when `allow_attachments` is `false`.
    pub attachment_types: Vec<String>,
    /// Named field groups with optional visibility rules.
    pub field_groups: Vec<FieldGroup>,
    /// AST of the script that defined this schema. Required to call `validate`
    /// and `visible` closures stored on individual fields and groups.
    pub ast: Option<rhai::AST>,
    /// Schema version number — must be >= 1 and must not decrease on re-registration.
    pub version: u32,
    /// Migration closures keyed by target version (2..=version).
    /// Each closure receives `#{ title, fields }` and returns the mutated map.
    pub migrations: std::collections::BTreeMap<u32, rhai::FnPtr>,
}

impl Schema {
    /// All fields in declaration order: top-level first, then each group's fields.
    pub fn all_fields(&self) -> Vec<&FieldDefinition> {
        self.fields.iter()
            .chain(self.field_groups.iter().flat_map(|g| g.fields.iter()))
            .collect()
    }

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
    pub fn validate_required_fields(&self, fields: &BTreeMap<String, FieldValue>) -> crate::Result<()> {
        for field_def in self.all_fields() {
            if !field_def.required {
                continue;
            }
            let empty = match fields.get(&field_def.name) {
                Some(FieldValue::Text(s)) => s.is_empty(),
                Some(FieldValue::Email(s)) => s.is_empty(),
                Some(FieldValue::Date(d)) => d.is_none(),
                Some(FieldValue::Number(_) | FieldValue::Boolean(_)) => false,
                Some(FieldValue::NoteLink(id)) => id.is_none(),
                Some(FieldValue::File(id)) => id.is_none(),
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
    pub fn default_fields(&self) -> BTreeMap<String, FieldValue> {
        let mut fields = BTreeMap::new();
        for field_def in self.all_fields() {
            let default_value = match field_def.field_type.as_str() {
                "text" | "textarea" => FieldValue::Text(String::new()),
                "number" => FieldValue::Number(0.0),
                "boolean" => FieldValue::Boolean(false),
                "date" => FieldValue::Date(None),
                "email" => FieldValue::Email(String::new()),
                "select" => FieldValue::Text(String::new()),
                "rating" => FieldValue::Number(0.0),
                "note_link" => FieldValue::NoteLink(None),
                "file" => FieldValue::File(None),
                // Unknown types fall back to empty text; script validation catches typos.
                _ => FieldValue::Text(String::new()),
            };
            fields.insert(field_def.name.clone(), default_value);
        }
        fields
    }

    /// Parses a single `FieldDefinition` from a Rhai Dynamic (must be a map).
    pub(super) fn parse_field_def(field_item: &Dynamic) -> Result<FieldDefinition> {
        let field_map = field_item
            .clone()
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

        let target_schema: Option<String> = field_map
            .get("target_schema")
            .and_then(|v| v.clone().try_cast::<String>());

        let show_on_hover = field_map
            .get("show_on_hover")
            .and_then(|v| v.clone().try_cast::<bool>())
            .unwrap_or(false);

        let mut allowed_types: Vec<String> = Vec::new();
        if let Some(arr) = field_map
            .get("allowed_types")
            .and_then(|v| v.clone().try_cast::<rhai::Array>())
        {
            for item in arr {
                let s = item.try_cast::<String>().ok_or_else(|| {
                    KrillnotesError::Scripting("allowed_types array must contain only strings".into())
                })?;
                allowed_types.push(s);
            }
        }

        let validate: Option<rhai::FnPtr> = field_map
            .get("validate")
            .and_then(|v| v.clone().try_cast::<rhai::FnPtr>());

        Ok(FieldDefinition { name: field_name, field_type, required, can_view, can_edit, options, max, target_schema, show_on_hover, allowed_types, validate })
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
        let mut all_field_names = std::collections::HashSet::new();
        for field_item in fields_array {
            let field = Self::parse_field_def(&field_item)?;
            if !all_field_names.insert(field.name.clone()) {
                return Err(KrillnotesError::Scripting(format!(
                    "Duplicate field name '{}' in schema '{}'", field.name, name
                )));
            }
            fields.push(field);
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

        let mut allowed_parent_schemas: Vec<String> = Vec::new();
        if let Some(arr) = def
            .get("allowed_parent_schemas")
            .and_then(|v| v.clone().try_cast::<rhai::Array>())
        {
            for item in arr {
                let s = item.try_cast::<String>().ok_or_else(|| {
                    KrillnotesError::Scripting("allowed_parent_schemas must contain only strings".into())
                })?;
                allowed_parent_schemas.push(s);
            }
        }

        let mut allowed_children_schemas: Vec<String> = Vec::new();
        if let Some(arr) = def
            .get("allowed_children_schemas")
            .and_then(|v| v.clone().try_cast::<rhai::Array>())
        {
            for item in arr {
                let s = item.try_cast::<String>().ok_or_else(|| {
                    KrillnotesError::Scripting("allowed_children_schemas must contain only strings".into())
                })?;
                allowed_children_schemas.push(s);
            }
        }

        let allow_attachments = def
            .get("allow_attachments")
            .and_then(|v| v.clone().try_cast::<bool>())
            .unwrap_or(false);

        let mut attachment_types: Vec<String> = Vec::new();
        if let Some(arr) = def
            .get("attachment_types")
            .and_then(|v| v.clone().try_cast::<rhai::Array>())
        {
            for item in arr {
                let s = item.try_cast::<String>().ok_or_else(|| {
                    KrillnotesError::Scripting("attachment_types array must contain only strings".into())
                })?;
                attachment_types.push(s);
            }
        }

        let mut field_groups: Vec<FieldGroup> = Vec::new();
        if let Some(groups_array) = def
            .get("field_groups")
            .and_then(|v| v.clone().try_cast::<rhai::Array>())
        {
            for group_item in groups_array {
                let group_map = group_item
                    .try_cast::<Map>()
                    .ok_or_else(|| KrillnotesError::Scripting("field_groups entry must be a map".to_string()))?;

                let group_name = group_map
                    .get("name")
                    .and_then(|v| v.clone().try_cast::<String>())
                    .ok_or_else(|| KrillnotesError::Scripting("field group missing 'name'".to_string()))?;

                let collapsed = group_map
                    .get("collapsed")
                    .and_then(|v| v.clone().try_cast::<bool>())
                    .unwrap_or(false);

                let visible: Option<rhai::FnPtr> = group_map
                    .get("visible")
                    .and_then(|v| v.clone().try_cast::<rhai::FnPtr>());

                let group_fields_array = group_map
                    .get("fields")
                    .and_then(|v| v.clone().try_cast::<rhai::Array>())
                    .ok_or_else(|| KrillnotesError::Scripting(
                        format!("field group '{}' missing 'fields' array", group_name)
                    ))?;

                let mut group_fields = Vec::new();
                for field_item in group_fields_array {
                    let field = Self::parse_field_def(&field_item)?;
                    if !all_field_names.insert(field.name.clone()) {
                        return Err(KrillnotesError::Scripting(format!(
                            "Duplicate field name '{}' in schema '{}'", field.name, name
                        )));
                    }
                    group_fields.push(field);
                }

                field_groups.push(FieldGroup { name: group_name, fields: group_fields, visible, collapsed });
            }
        }

        // version is required — hard error if missing or < 1
        let version = def
            .get("version")
            .and_then(|v| v.clone().try_cast::<i64>())
            .ok_or_else(|| KrillnotesError::Scripting(
                format!("Schema '{}' missing required 'version' key", name)
            ))?;
        if version < 1 {
            return Err(KrillnotesError::Scripting(
                format!("Schema '{}' version must be >= 1, got {}", name, version)
            ));
        }
        let version = version as u32;

        // migrate map is optional — keyed by target version, values are closures
        let mut migrations = std::collections::BTreeMap::new();
        if let Some(migrate_map) = def
            .get("migrate")
            .and_then(|v| v.clone().try_cast::<rhai::Map>())
        {
            for (key, val) in migrate_map.iter() {
                let target_ver = key.to_string().parse::<u32>().map_err(|_| {
                    KrillnotesError::Scripting(
                        format!("Schema '{}' migrate key '{}' must be an integer", name, key)
                    )
                })?;
                if target_ver < 2 || target_ver > version {
                    return Err(KrillnotesError::Scripting(
                        format!(
                            "Schema '{}' migrate key {} out of range (must be 2..={})",
                            name, target_ver, version
                        )
                    ));
                }
                let fn_ptr = val.clone().try_cast::<rhai::FnPtr>().ok_or_else(|| {
                    KrillnotesError::Scripting(
                        format!("Schema '{}' migrate[{}] must be a closure", name, target_ver)
                    )
                })?;
                migrations.insert(target_ver, fn_ptr);
            }
        }

        Ok(Schema { name: name.to_string(), fields, title_can_view, title_can_edit, children_sort, allowed_parent_schemas, allowed_children_schemas, allow_attachments, attachment_types, field_groups, ast: None, version, migrations })
    }
}

/// Private store for registered schemas plus per-schema hook side-tables.
#[derive(Debug, Clone)]
pub(super) struct SchemaRegistry {
    schemas:              Arc<Mutex<HashMap<String, Schema>>>,
    on_save_hooks:        Arc<Mutex<HashMap<String, HookEntry>>>,
    on_add_child_hooks:   Arc<Mutex<HashMap<String, HookEntry>>>,
    view_registrations:   Arc<Mutex<HashMap<String, Vec<ViewRegistration>>>>,
    hover_registrations:  Arc<Mutex<HashMap<String, HookEntry>>>,
    menu_registrations:   Arc<Mutex<HashMap<String, Vec<MenuRegistration>>>>,
    deferred_bindings:    Arc<Mutex<Vec<DeferredBinding>>>,
    warnings:             Arc<Mutex<Vec<ScriptWarning>>>,
}

impl SchemaRegistry {
    pub(super) fn new() -> Self {
        Self {
            schemas:              Arc::new(Mutex::new(HashMap::new())),
            on_save_hooks:        Arc::new(Mutex::new(HashMap::new())),
            on_add_child_hooks:   Arc::new(Mutex::new(HashMap::new())),
            view_registrations:   Arc::new(Mutex::new(HashMap::new())),
            hover_registrations:  Arc::new(Mutex::new(HashMap::new())),
            menu_registrations:   Arc::new(Mutex::new(HashMap::new())),
            deferred_bindings:    Arc::new(Mutex::new(Vec::new())),
            warnings:             Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Returns a clone of the inner `Arc` so Rhai host-function closures can write into it.
    pub(super) fn schemas_arc(&self) -> Arc<Mutex<HashMap<String, Schema>>> {
        Arc::clone(&self.schemas)
    }

    pub(super) fn on_save_hooks_arc(&self) -> Arc<Mutex<HashMap<String, HookEntry>>> {
        Arc::clone(&self.on_save_hooks)
    }

    pub(super) fn on_add_child_hooks_arc(&self) -> Arc<Mutex<HashMap<String, HookEntry>>> {
        Arc::clone(&self.on_add_child_hooks)
    }

    pub(super) fn menu_registrations_arc(&self) -> Arc<Mutex<HashMap<String, Vec<MenuRegistration>>>> {
        Arc::clone(&self.menu_registrations)
    }

    pub(super) fn deferred_bindings_arc(&self) -> Arc<Mutex<Vec<DeferredBinding>>> {
        Arc::clone(&self.deferred_bindings)
    }

    pub fn get_views_for_type(&self, schema_name: &str) -> Vec<ViewRegistration> {
        self.view_registrations.lock().unwrap()
            .get(schema_name).cloned().unwrap_or_default()
    }

    pub fn get_warnings(&self) -> Vec<ScriptWarning> {
        self.warnings.lock().unwrap().clone()
    }

    pub(super) fn add_warning(&self, script_name: &str, message: &str) {
        self.warnings.lock().unwrap().push(ScriptWarning {
            script_name: script_name.to_string(),
            message: message.to_string(),
        });
    }

    /// Returns `(schema_name, schema_version, migrations, ast)` for every registered schema.
    /// Used by the Phase D migration pipeline to detect and migrate stale notes.
    pub(super) fn get_versioned_schemas(&self) -> Vec<(String, u32, std::collections::BTreeMap<u32, FnPtr>, Option<rhai::AST>)> {
        self.schemas.lock().unwrap()
            .values()
            .map(|s| (s.name.clone(), s.version, s.migrations.clone(), s.ast.clone()))
            .collect()
    }

    /// Returns a map of note_type -> [menu_label, ...] for all registered menu actions.
    pub fn menu_action_map(&self) -> HashMap<String, Vec<String>> {
        let regs = self.menu_registrations.lock().unwrap();
        regs.iter()
            .map(|(k, v)| (k.clone(), v.iter().map(|r| r.label.clone()).collect()))
            .collect()
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

    pub(super) fn clear(&self) {
        self.schemas.lock().unwrap().clear();
        self.on_save_hooks.lock().unwrap().clear();
        self.on_add_child_hooks.lock().unwrap().clear();
        self.view_registrations.lock().unwrap().clear();
        self.hover_registrations.lock().unwrap().clear();
        self.menu_registrations.lock().unwrap().clear();
        self.deferred_bindings.lock().unwrap().clear();
        self.warnings.lock().unwrap().clear();
    }

    /// Returns `true` if an on_save hook is registered for `schema_name`.
    pub(super) fn has_hook(&self, schema_name: &str) -> bool {
        self.on_save_hooks.lock().unwrap().contains_key(schema_name)
    }

    /// Returns `true` if any view registrations exist for `schema_name`.
    pub(super) fn has_views(&self, schema_name: &str) -> bool {
        self.view_registrations.lock().unwrap()
            .get(schema_name).is_some_and(|v| !v.is_empty())
    }

    /// Returns `true` if a hover registration exists for `schema_name`.
    pub(super) fn has_hover(&self, schema_name: &str) -> bool {
        self.hover_registrations.lock().unwrap().contains_key(schema_name)
    }

    /// Resolves deferred bindings against currently registered schemas.
    pub fn resolve_bindings(&self) {
        let mut bindings = self.deferred_bindings.lock().unwrap();
        let schemas = self.schemas.lock().unwrap();
        let mut views = self.view_registrations.lock().unwrap();
        let mut hovers = self.hover_registrations.lock().unwrap();
        let mut menus = self.menu_registrations.lock().unwrap();
        let mut warnings = self.warnings.lock().unwrap();

        for binding in bindings.drain(..) {
            match binding.kind {
                BindingKind::View => {
                    if schemas.contains_key(&binding.target_schema) {
                        let label = binding.label.unwrap_or_else(|| binding.target_schema.clone());
                        let slot = views.entry(binding.target_schema).or_default();
                        // Deduplicate: library source is prepended to each schema compilation,
                        // so register_view() in a library script fires once per schema loaded.
                        // Keep only the first registration for each (type, label) pair.
                        if !slot.iter().any(|v| v.label == label) {
                            slot.push(ViewRegistration {
                                label,
                                display_first: binding.display_first,
                                fn_ptr: binding.fn_ptr,
                                ast: binding.ast,
                                script_name: binding.script_name,
                            });
                        }
                    } else {
                        warnings.push(ScriptWarning {
                            script_name: binding.script_name,
                            message: format!(
                                "register_view('{}', '{}') -- schema not found",
                                binding.target_schema,
                                binding.label.unwrap_or_default()
                            ),
                        });
                    }
                }
                BindingKind::Hover => {
                    if schemas.contains_key(&binding.target_schema) {
                        let entry = HookEntry {
                            fn_ptr: binding.fn_ptr,
                            ast: binding.ast.as_ref().clone(),
                            script_name: binding.script_name,
                        };
                        hovers.insert(binding.target_schema, entry);
                    } else {
                        warnings.push(ScriptWarning {
                            script_name: binding.script_name,
                            message: format!(
                                "register_hover('{}') -- schema not found",
                                binding.target_schema
                            ),
                        });
                    }
                }
                BindingKind::Menu => {
                    for target_type in &binding.applies_to {
                        if schemas.contains_key(target_type) {
                            let label = binding.label.clone().unwrap_or_default();
                            let slot = menus.entry(target_type.clone()).or_default();
                            if !slot.iter().any(|m| m.label == label) {
                                slot.push(MenuRegistration {
                                    label,
                                    fn_ptr: binding.fn_ptr.clone(),
                                    ast: Arc::clone(&binding.ast),
                                    script_name: binding.script_name.clone(),
                                });
                            }
                        } else {
                            warnings.push(ScriptWarning {
                                script_name: binding.script_name.clone(),
                                message: format!(
                                    "register_menu('{}', ['{}']) -- type not found",
                                    binding.label.as_deref().unwrap_or(""),
                                    target_type
                                ),
                            });
                        }
                    }
                }
            }
        }
    }

    /// Runs the on_save hook for `schema_name`, if registered.
    ///
    /// Returns `Ok(None)` when no hook is registered.
    /// Returns `Ok(Some(tx))` with the populated [`SaveTransaction`] on success.
    /// The transaction's `committed` flag indicates whether `commit()` was called.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError::Scripting`] if the hook throws a Rhai error, or if
    /// the hook returns a Map (old-style mutation) instead of using `set_field`/`commit`.
    ///
    /// Called from [`ScriptRegistry::run_on_save_hook`](super::ScriptRegistry::run_on_save_hook).
    pub(super) fn run_on_save_hook(
        &self,
        engine: &Engine,
        schema: &Schema,
        note_id: &str,
        schema_type: &str,
        title: &str,
        fields: &BTreeMap<String, FieldValue>,
    ) -> Result<Option<SaveTransaction>> {
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
        note_map.insert("schema".into(),    Dynamic::from(schema_type.to_string()));
        note_map.insert("title".into(),     Dynamic::from(title.to_string()));
        note_map.insert("fields".into(),    Dynamic::from(fields_map));

        // Populate the thread-local SaveTransaction before calling the hook.
        let tx = SaveTransaction::for_existing_note(
            note_id.to_string(),
            schema_type.to_string(),
            title.to_string(),
            fields.clone(),
        );
        super::set_save_tx(tx);

        let result = entry
            .fn_ptr
            .call::<Dynamic>(engine, &entry.ast, (Dynamic::from(note_map),));

        // Always take the SaveTransaction back before checking errors.
        let tx = super::take_save_tx().unwrap_or_default();

        // If the hook threw (e.g. commit() failing because reject() was called), but the
        // tx has soft errors, treat as ValidationFailed rather than a Scripting error.
        // This preserves the structured error data from reject() calls.
        let result = match result {
            Err(_) if tx.has_errors() => {
                let msgs: Vec<String> = tx.soft_errors.iter().map(|e| {
                    match &e.field {
                        Some(f) => format!("{}: {}", f, e.message),
                        None => e.message.clone(),
                    }
                }).collect();
                return Err(KrillnotesError::ValidationFailed(msgs.join("; ")));
            }
            Err(e) => return Err(KrillnotesError::Scripting(
                format!("on_save hook error in '{}': {e}", entry.script_name)
            )),
            Ok(v) => v,
        };

        // Old-style hooks return the note map. Detect and reject with a clear migration message.
        if result.is::<Map>() {
            return Err(KrillnotesError::Scripting(
                format!(
                    "on_save hook in '{}' uses the old direct-mutation style (returns the note map). \
                     Migrate to the gated model: use set_field(note.id, \"field\", value), \
                     set_title(note.id, \"title\"), and commit() instead.",
                    entry.script_name
                )
            ));
        }

        Ok(Some(tx))
    }

    /// Renders the default view for a note type (first display_first, or first registered).
    ///
    /// Returns `Ok(None)` when no views are registered for the type.
    pub(super) fn run_default_view(
        &self,
        engine: &Engine,
        note_map: Map,
    ) -> Result<Option<String>> {
        let schema_name = note_map
            .get("schema")
            .and_then(|v| v.clone().try_cast::<String>())
            .unwrap_or_default();

        let views = self.view_registrations.lock().unwrap();
        let view_list = match views.get(&schema_name) {
            Some(v) if !v.is_empty() => v,
            _ => return Ok(None),
        };

        // Pick the first display_first view, or just the first one
        let view = view_list.iter().find(|v| v.display_first).unwrap_or(&view_list[0]);
        let result = view
            .fn_ptr
            .call::<Dynamic>(engine, &view.ast, (Dynamic::from(note_map),))
            .map_err(|e| KrillnotesError::Scripting(format!("view '{}' error in '{}': {e}", view.label, view.script_name)))?;

        let html = result.try_cast::<String>().ok_or_else(|| {
            KrillnotesError::Scripting("view must return a string".to_string())
        })?;

        Ok(Some(html))
    }

    /// Renders a specific named view for a note type.
    pub(super) fn run_view(
        &self,
        engine: &Engine,
        note_map: Map,
        view_label: &str,
    ) -> Result<String> {
        let schema_name = note_map
            .get("schema")
            .and_then(|v| v.clone().try_cast::<String>())
            .unwrap_or_default();

        let views = self.view_registrations.lock().unwrap();
        let view_list = views.get(&schema_name).ok_or_else(|| {
            KrillnotesError::Scripting(format!("No views registered for type '{schema_name}'"))
        })?;

        let view = view_list.iter().find(|v| v.label == view_label).ok_or_else(|| {
            KrillnotesError::Scripting(format!("View '{view_label}' not found for type '{schema_name}'"))
        })?;

        let result = view
            .fn_ptr
            .call::<Dynamic>(engine, &view.ast, (Dynamic::from(note_map),))
            .map_err(|e| KrillnotesError::Scripting(format!("view '{}' error in '{}': {e}", view.label, view.script_name)))?;

        result.try_cast::<String>().ok_or_else(|| {
            KrillnotesError::Scripting("view must return a string".to_string())
        })
    }

    /// Runs the hover registration for `schema_name`, if registered.
    ///
    /// Returns `Ok(None)` when no hover is registered.
    pub(super) fn run_on_hover_hook(
        &self,
        engine: &Engine,
        note_map: Map,
    ) -> Result<Option<String>> {
        let schema_name = note_map
            .get("schema")
            .and_then(|v| v.clone().try_cast::<String>())
            .unwrap_or_default();

        let entry = {
            let hovers = self.hover_registrations
                .lock()
                .map_err(|_| KrillnotesError::Scripting("hover registration lock poisoned".to_string()))?;
            hovers.get(&schema_name).cloned()
        };
        let entry = match entry {
            Some(e) => e,
            None => return Ok(None),
        };

        let result = entry
            .fn_ptr
            .call::<Dynamic>(engine, &entry.ast, (Dynamic::from(note_map),))
            .map_err(|e| KrillnotesError::Scripting(format!("hover error in '{}': {e}", entry.script_name)))?;

        let html = result.try_cast::<String>().ok_or_else(|| {
            KrillnotesError::Scripting("hover must return a string".to_string())
        })?;

        Ok(Some(html))
    }

    /// Runs the on_add_child hook for `parent_schema`, if registered.
    ///
    /// Called from [`ScriptRegistry::run_on_add_child_hook`](super::ScriptRegistry::run_on_add_child_hook).
    ///
    /// Returns `Ok(None)` when no hook is registered for the parent schema.
    /// Returns `Ok(Some(AddChildResult))` with optional parent/child updates on success.
    ///
    /// The hook must use the gated SaveTransaction API (`set_field`, `set_title`, `commit`).
    /// Hooks that return a map (old-style direct mutation) are rejected with a migration error.
    pub(super) fn run_on_add_child_hook(
        &self,
        engine: &Engine,
        parent_schema: &Schema,
        parent_id: &str,
        parent_type: &str,
        parent_title: &str,
        parent_fields: &BTreeMap<String, FieldValue>,
        child_schema: &Schema,
        child_id: &str,
        child_type: &str,
        child_title: &str,
        child_fields: &BTreeMap<String, FieldValue>,
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
        parent_map.insert("schema".into(), Dynamic::from(parent_type.to_string()));
        parent_map.insert("title".into(),     Dynamic::from(parent_title.to_string()));
        parent_map.insert("fields".into(),    Dynamic::from(p_fields_map));

        // Build child note map
        let mut c_fields_map = Map::new();
        for (k, v) in child_fields {
            c_fields_map.insert(k.as_str().into(), field_value_to_dynamic(v));
        }
        let mut child_map = Map::new();
        child_map.insert("id".into(),        Dynamic::from(child_id.to_string()));
        child_map.insert("schema".into(), Dynamic::from(child_type.to_string()));
        child_map.insert("title".into(),     Dynamic::from(child_title.to_string()));
        child_map.insert("fields".into(),    Dynamic::from(c_fields_map));

        // Pre-seed the SaveTransaction with both parent and child so that
        // set_field / set_title calls inside the hook can target either note.
        let mut tx = SaveTransaction::new();
        tx.register_existing_note(
            parent_id.to_string(),
            parent_type.to_string(),
            parent_title.to_string(),
            parent_fields.clone(),
        );
        tx.register_existing_note(
            child_id.to_string(),
            child_type.to_string(),
            child_title.to_string(),
            child_fields.clone(),
        );
        super::set_save_tx(tx);

        let result = entry
            .fn_ptr
            .call::<Dynamic>(engine, &entry.ast, (Dynamic::from(parent_map), Dynamic::from(child_map)));

        // Always take the SaveTransaction back before inspecting errors.
        let tx = super::take_save_tx().unwrap_or_default();

        let result = match result {
            Err(e) => return Err(KrillnotesError::Scripting(
                format!("on_add_child hook error in '{}': {e}", entry.script_name)
            )),
            Ok(v) => v,
        };

        // Old-style hooks return a map #{ parent: ..., child: ... }.
        // Detect and reject with a clear migration message.
        if result.is::<Map>() {
            return Err(KrillnotesError::Scripting(
                format!(
                    "on_add_child hook in '{}' uses the old direct-mutation style (returns a map). \
                     Migrate to the gated model: use set_field(id, \"field\", value), \
                     set_title(id, \"title\"), and commit() instead.",
                    entry.script_name
                )
            ));
        }

        // Extract modifications from the transaction's pending notes.
        let parent_update = extract_note_update(&tx, parent_id, parent_schema);
        let child_update  = extract_note_update(&tx, child_id,  child_schema);

        Ok(Some(AddChildResult { parent: parent_update, child: child_update }))
    }
}

/// Extracts title and fields updates from a completed [`SaveTransaction`] for one note.
///
/// Returns `Some((title, fields))` if the note has any pending changes, or `None`
/// if the hook left it unmodified.
fn extract_note_update(
    tx: &SaveTransaction,
    note_id: &str,
    schema: &Schema,
) -> Option<(String, BTreeMap<String, FieldValue>)> {
    let pending = tx.pending_notes.get(note_id)?;

    // Only return Some if the hook actually changed something.
    let title_changed  = pending.pending_title.is_some();
    let fields_changed = !pending.pending_fields.is_empty();

    if !title_changed && !fields_changed {
        return None;
    }

    let new_title = pending.effective_title().to_string();

    // Build the effective fields map restricted to schema-defined fields.
    let effective = pending.effective_fields();
    let mut new_fields = BTreeMap::new();
    for field_def in schema.all_fields() {
        if let Some(fv) = effective.get(&field_def.name) {
            new_fields.insert(field_def.name.clone(), fv.clone());
        }
    }

    Some((new_title, new_fields))
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
        FieldValue::File(None) => Dynamic::UNIT,
        FieldValue::File(Some(id)) => Dynamic::from(id.clone()),
    }
}

/// Converts a Rhai [`Dynamic`] value back to a [`FieldValue`] using the field type hint
/// from the schema definition.  Used by the Phase D migration pipeline after closures run.
pub(super) fn dynamic_to_field_value(d: Dynamic, field_type: &str) -> FieldValue {
    use chrono::NaiveDate;
    match field_type {
        "number" | "rating" => {
            let n = d.clone().try_cast::<f64>()
                .or_else(|| d.clone().try_cast::<i64>().map(|i| i as f64))
                .unwrap_or(0.0);
            FieldValue::Number(n)
        }
        "boolean" => FieldValue::Boolean(d.try_cast::<bool>().unwrap_or(false)),
        "date" => {
            let s = d.try_cast::<String>().unwrap_or_default();
            FieldValue::Date(NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok())
        }
        "email" => FieldValue::Email(d.try_cast::<String>().unwrap_or_default()),
        "note_link" => FieldValue::NoteLink(
            d.try_cast::<String>().filter(|s| !s.is_empty())
        ),
        "file" => FieldValue::File(
            d.try_cast::<String>().filter(|s| !s.is_empty())
        ),
        _ => FieldValue::Text(d.try_cast::<String>().unwrap_or_default()),
    }
}

