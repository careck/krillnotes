// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

use crate::AppState;
use tauri::State;
use std::collections::{BTreeMap, HashMap};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptMutationResult<T: serde::Serialize> {
    pub data: T,
    pub load_errors: Vec<crate::ScriptError>,
}

/// Serializable field definition with an extra `has_validate` flag for the frontend.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldDefInfo {
    pub name: String,
    pub field_type: String,
    pub required: bool,
    pub can_view: bool,
    pub can_edit: bool,
    pub options: Vec<String>,
    pub max: i64,
    pub target_schema: Option<String>,
    pub show_on_hover: bool,
    pub allowed_types: Vec<String>,
    /// `true` if a `validate` closure is registered for this field.
    pub has_validate: bool,
}

impl From<&crate::FieldDefinition> for FieldDefInfo {
    fn from(f: &crate::FieldDefinition) -> Self {
        Self {
            name: f.name.clone(),
            field_type: f.field_type.clone(),
            required: f.required,
            can_view: f.can_view,
            can_edit: f.can_edit,
            options: f.options.clone(),
            max: f.max,
            target_schema: f.target_schema.clone(),
            show_on_hover: f.show_on_hover,
            allowed_types: f.allowed_types.clone(),
            has_validate: f.validate.is_some(),
        }
    }
}

/// Serializable field group for the SchemaInfo Tauri response.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldGroupInfo {
    pub name: String,
    pub fields: Vec<FieldDefInfo>,
    pub collapsed: bool,
    pub has_visible_closure: bool,
}

/// Response type for the `get_schema_fields` Tauri command, bundling field
/// definitions with schema-level title visibility flags.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaInfo {
    pub fields: Vec<FieldDefInfo>,
    pub title_can_view: bool,
    pub title_can_edit: bool,
    pub children_sort: String,
    pub allowed_parent_schemas: Vec<String>,
    pub allowed_children_schemas: Vec<String>,
    pub allow_attachments: bool,
    pub attachment_types: Vec<String>,
    pub has_views: bool,
    pub has_hover: bool,
    pub field_groups: Vec<FieldGroupInfo>,
    pub is_leaf: bool,
}

pub(crate) fn schema_to_info(schema: &crate::Schema, has_views: bool, has_hover: bool) -> SchemaInfo {
    SchemaInfo {
        has_views,
        has_hover,
        fields: schema.fields.iter().map(FieldDefInfo::from).collect(),
        title_can_view: schema.title_can_view,
        title_can_edit: schema.title_can_edit,
        children_sort: schema.children_sort.clone(),
        allowed_parent_schemas: schema.allowed_parent_schemas.clone(),
        allowed_children_schemas: schema.allowed_children_schemas.clone(),
        allow_attachments: schema.allow_attachments,
        attachment_types: schema.attachment_types.clone(),
        field_groups: schema.field_groups.iter().map(|g| FieldGroupInfo {
            name: g.name.clone(),
            fields: g.fields.iter().map(FieldDefInfo::from).collect(),
            collapsed: g.collapsed,
            has_visible_closure: g.visible.is_some(),
        }).collect(),
        is_leaf: schema.is_leaf,
    }
}

/// Returns the field definitions for the schema identified by `schema`.
///
/// Looks up the schema registered under `schema` in the calling window's
/// workspace and returns its list of [`FieldDefinition`] values so the
/// frontend can render an appropriate editing form.
///
/// # Errors
///
/// Returns an error string if no workspace is open for the calling window,
/// or if `schema` is not registered in the schema registry.
#[tauri::command]
pub fn get_schema_fields(
    window: tauri::Window,
    state: State<'_, AppState>,
    schema: String,
) -> std::result::Result<SchemaInfo, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;

    let schema_def = workspace.script_registry().get_schema(&schema)
        .map_err(|e: crate::KrillnotesError| e.to_string())?;

    Ok(schema_to_info(
        &schema_def,
        workspace.script_registry().has_views(&schema),
        workspace.script_registry().has_hover(&schema),
    ))
}

/// Returns all schema infos keyed by node type name.
#[tauri::command]
pub fn get_all_schemas(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<HashMap<String, SchemaInfo>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;

    let schemas = workspace.script_registry().all_schemas();
    let mut result = HashMap::new();
    for (name, schema) in schemas {
        let has_view_hook = workspace.script_registry().has_views(&name);
        let has_hover_hook = workspace.script_registry().has_hover(&name);
        result.insert(name, schema_to_info(&schema, has_view_hook, has_hover_hook));
    }
    Ok(result)
}

/// Returns a map of `note_type → [action_label, …]` for all registered tree actions.
#[tauri::command]
pub fn get_tree_action_map(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<HashMap<String, Vec<String>>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    Ok(workspace.tree_action_map())
}

/// Runs the tree action `label` on `note_id`.
#[tauri::command]
pub fn invoke_tree_action(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
    label: String,
) -> std::result::Result<(), String> {
    let window_label = window.label().to_string();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(&window_label).ok_or("No workspace open")?;
    workspace.run_tree_action(&note_id, &label)
        .map_err(|e| e.to_string())
}

/// Returns the custom HTML view for a note generated by its `on_view` hook, if any.
/// Returns the HTML view for a note.
///
/// When an `on_view` Rhai hook is registered for the note's schema the hook
/// generates the HTML; otherwise a default view is generated, with `textarea`
/// fields rendered as CommonMark markdown.
///
/// # Errors
///
/// Returns an error string if no workspace is open or if the hook fails.
#[tauri::command]
pub fn get_note_view(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
) -> std::result::Result<String, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    workspace.run_view_hook(&note_id).map_err(|e| e.to_string())
}

/// Returns the on_hover hook HTML for a note, or `null` if no hook is registered.
#[tauri::command]
pub fn get_note_hover(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
) -> std::result::Result<Option<String>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    workspace.run_hover_hook(&note_id).map_err(|e| e.to_string())
}

/// Returns the list of registered views for a note type.
#[tauri::command]
pub fn get_views_for_type(
    window: tauri::Window,
    state: State<'_, AppState>,
    schema_name: String,
) -> std::result::Result<Vec<ViewInfo>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    let views = workspace.get_views_for_type(&schema_name);
    Ok(views.iter().map(|v| ViewInfo {
        label: v.label.clone(),
        display_first: v.display_first,
    }).collect())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewInfo {
    pub label: String,
    pub display_first: bool,
}

/// Renders a specific named view tab for a note.
#[tauri::command]
pub fn render_view(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
    view_label: String,
) -> std::result::Result<String, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    workspace.render_view(&note_id, &view_label).map_err(|e| e.to_string())
}

/// Renders a single textarea field value as markdown HTML with attachment images embedded.
#[tauri::command]
pub fn render_markdown_field(
    window: tauri::Window,
    state: State<'_, AppState>,
    note_id: String,
    text: String,
) -> std::result::Result<String, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    workspace.render_markdown_field(&note_id, &text).map_err(|e| e.to_string())
}

/// Returns script warnings (unresolved bindings).
#[tauri::command]
pub fn get_script_warnings(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<Vec<crate::ScriptWarning>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    Ok(workspace.get_script_warnings())
}

/// Runs the `validate` closure for a single field.
///
/// Returns `None` when the field is valid or has no validate closure.
/// Returns `Some(error_message)` when the closure returns an error.
#[tauri::command]
pub fn validate_field(
    window: tauri::Window,
    state: State<'_, AppState>,
    schema_name: String,
    field_name: String,
    value: serde_json::Value,
) -> std::result::Result<Option<String>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get(label)
        .ok_or("No workspace open")?;

    let fv: crate::FieldValue = serde_json::from_value(value).map_err(|e| e.to_string())?;
    workspace.script_registry()
        .validate_field(&schema_name, &field_name, &fv)
        .map_err(|e| e.to_string())
}

/// Runs `validate` closures for all fields that have them.
///
/// Returns a map of `field_name → error_message` for each invalid field.
#[tauri::command]
pub fn validate_fields(
    window: tauri::Window,
    state: State<'_, AppState>,
    schema_name: String,
    fields: BTreeMap<String, crate::FieldValue>,
) -> std::result::Result<BTreeMap<String, String>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get(label)
        .ok_or("No workspace open")?;

    workspace.script_registry()
        .validate_fields(&schema_name, &fields)
        .map_err(|e| e.to_string())
}

/// Evaluates `visible` closures for each `FieldGroup`.
///
/// Returns a map of `group_name → bool`.
#[tauri::command]
pub fn evaluate_group_visibility(
    window: tauri::Window,
    state: State<'_, AppState>,
    schema_name: String,
    fields: BTreeMap<String, crate::FieldValue>,
) -> std::result::Result<BTreeMap<String, bool>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get(label)
        .ok_or("No workspace open")?;

    workspace.script_registry()
        .evaluate_group_visibility(&schema_name, &fields)
        .map_err(|e| e.to_string())
}
