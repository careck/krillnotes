# Phase 4: Detail View & Editing Design

**Date:** 2026-02-18
**Status:** Approved
**Follows:** Phase 3 (Tree View)
**Next:** Phase 5 (TBD)

## Overview

Add editing capabilities to the detail panel with clear view/edit mode separation. Users can toggle between viewing note information and editing title + custom fields, with explicit save/cancel actions.

**Key Features:**
- View/Edit mode toggle in InfoPanel
- Edit title and custom fields (plain text inputs)
- Hybrid field rendering (schema fields + legacy fields preserved)
- Explicit Save/Cancel buttons in edit mode
- Delete note with confirmation dialog
- Smart delete: ask user whether to delete children or promote them

**Architecture Decision:** View/Edit mode separation provides clean foundation for future scripted custom views while keeping editing logic consistent.

## Goals

- Enable editing of note title and custom fields
- Preserve data during schema evolution (hybrid field approach)
- Clear visual distinction between view and edit modes
- Explicit save workflow (no auto-save)
- Safe deletion with confirmation and child handling options
- Plain text editing for all field types (markdown/rich text deferred to future phase)

## User Experience

### View Mode

**Layout:**
- Title (large, prominent)
- All field values displayed as labeled read-only sections
  - Schema fields rendered in order
  - Legacy fields (not in current schema) shown below schema fields
- Metadata section: node type, created/modified timestamps, note ID
- Action buttons in header: "Edit" and "Delete"

**Field Display:**
- Text fields: Formatted text block (preserves whitespace)
- Number fields: Formatted number
- Boolean fields: Checkbox icon or Yes/No text
- Clean, readable layout with labels

### Edit Mode

**Layout:**
- Title input field (text input)
- Dynamic field editors for all schema + legacy fields
  - Text: `<textarea>` for multi-line content
  - Number: `<input type="number">`
  - Boolean: `<input type="checkbox">`
- Metadata section (read-only, same as view mode)
- Action buttons: "Save" and "Cancel"
- Visual indicator of edit mode (border or background change)

**Interactions:**
1. Click "Edit" → switches to edit mode, copies current values to edit state
2. Modify fields → updates local edit state only
3. Click "Save" → persists changes to database, switches back to view mode
4. Click "Cancel" → discards changes, returns to view mode
   - If changes made: shows confirmation "Discard changes?"
5. Navigate to different note with unsaved changes → warns "You have unsaved changes. Leave?"

### Delete Flow

**Without Children:**
- Click "Delete" → simple confirmation dialog
- "Delete [note title]?" with Cancel/Delete buttons
- Confirm → note deleted, next note auto-selected

**With Children:**
- Click "Delete" → confirmation dialog with strategy options
- Shows: "Delete [note title]? This note has X children."
- Radio buttons:
  - "Delete this note and all descendants" (Y notes total)
  - "Delete this note and promote children to parent level"
- Cancel/Delete buttons
- Confirm → executes chosen strategy

## Architecture

### Component Structure

**Modified Components:**

**1. InfoPanel.tsx** - Main detail panel with view/edit modes
- State:
  - `isEditing: boolean` - current mode
  - `editedTitle: string` - working title in edit mode
  - `editedFields: Record<string, FieldValue>` - working field values
  - `schemaFields: FieldDefinition[]` - field definitions from schema
  - `isDirty: boolean` - tracks if changes made
- View Mode:
  - Title (large heading)
  - Render FieldDisplay for each field (schema + legacy)
  - Metadata section
  - "Edit" and "Delete" buttons
- Edit Mode:
  - Title input
  - Render FieldEditor for each field
  - Metadata section (read-only)
  - "Save" and "Cancel" buttons
- Fetch schema fields on mount/note change
- Track dirty state for navigation warnings

**2. WorkspaceView.tsx** - Handle note updates and deletions
- Add `handleUpdateNote(noteId, title, fields)` callback
- Add `handleDeleteNote(noteId, strategy)` callback
- Refresh note list after updates/deletions
- Handle selection changes after delete (auto-select next note)

**New Components:**

**3. FieldDisplay.tsx** - Read-only field value renderer (view mode)
- Props: `fieldName: string`, `fieldType: string`, `value: FieldValue`
- Renders based on type:
  - "text": Formatted text block with preserved whitespace
  - "number": Formatted number display
  - "boolean": Checkbox icon (checked/unchecked) or Yes/No text
- Styled with label and readable layout

**4. FieldEditor.tsx** - Editable field input (edit mode)
- Props: `fieldName: string`, `fieldType: string`, `value: FieldValue`, `required: boolean`, `onChange: (value: FieldValue) => void`
- Renders based on field_type:
  - "text": `<textarea>` with multiple rows
  - "number": `<input type="number">`
  - "boolean": `<input type="checkbox">`
- Includes label, validation, and error messages
- Shows required indicator (*)

**5. DeleteConfirmDialog.tsx** - Deletion confirmation modal
- Props: `noteTitle: string`, `childCount: number`, `onConfirm: (strategy: DeleteStrategy) => void`, `onCancel: () => void`
- If `childCount === 0`: Simple confirmation
- If `childCount > 0`: Shows strategy radio buttons
- Calculates total affected notes for "Delete all" option
- Cancel/Delete buttons

### Data Flow

**Viewing a Note:**
```
1. User selects note in tree
2. WorkspaceView passes selectedNote to InfoPanel
3. InfoPanel fetches schema fields: invoke('get_schema_fields', { nodeType })
4. Render view mode:
   - Display title
   - For each schema field: render FieldDisplay with note.fields[name]
   - For each legacy field (in note.fields but not in schema): render FieldDisplay
   - Display metadata
```

**Editing a Note:**
```
1. User clicks "Edit" button
2. Copy current values to edit state:
   - editedTitle = note.title
   - editedFields = { ...note.fields }
3. Set isEditing = true, isDirty = false
4. Render edit mode with FieldEditor for each field
5. User modifies fields:
   - Update editedTitle or editedFields
   - Set isDirty = true
6. User clicks "Save":
   - invoke('update_note', { noteId, title: editedTitle, fields: editedFields })
   - Backend updates database, returns updated Note
   - Update WorkspaceView note list
   - Set isEditing = false, isDirty = false
7. Tree view updates if title changed
```

**Deleting a Note:**
```
1. User clicks "Delete" button
2. Invoke('count_children', { noteId }) to check for children
3. Show DeleteConfirmDialog with appropriate options:
   - If childCount = 0: simple confirmation
   - If childCount > 0: show strategy radio buttons
4. User chooses strategy and confirms
5. Invoke('delete_note', { noteId, strategy })
6. Backend deletes note(s) and updates positions
7. WorkspaceView refreshes note list
8. Auto-select next available note:
   - Try next sibling
   - Else try previous sibling
   - Else try parent
   - Else show empty state
```

**Hybrid Field Rendering:**
```
1. Get schema fields for note type
2. Create Set of schema field names
3. Render schema fields in order from schema
4. Find legacy fields: note.fields keys not in schema Set
5. Render legacy fields after schema fields
6. Mark legacy fields visually (e.g., "(legacy)" label)
```

## Backend Changes

### New Commands

**1. `get_schema_fields(node_type: String) -> Vec<FieldDefinition>`**
```rust
#[tauri::command]
fn get_schema_fields(
    window: Window,
    state: State<'_, AppState>,
    node_type: String,
) -> Result<Vec<FieldDefinition>, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;

    let schema = workspace.registry.get_schema(&node_type)
        .map_err(|e| e.to_string())?;

    Ok(schema.fields)
}
```
- Returns field definitions from SchemaRegistry
- Used for rendering view/edit forms
- Error if schema not found → frontend falls back to legacy-only rendering

**2. `update_note(note_id: String, title: String, fields: HashMap<String, FieldValue>) -> Note`**
```rust
#[tauri::command]
fn update_note(
    window: Window,
    state: State<'_, AppState>,
    note_id: String,
    title: String,
    fields: HashMap<String, FieldValue>,
) -> Result<Note, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;

    workspace.update_note(&note_id, title, fields)
        .map_err(|e| e.to_string())
}
```
- Updates note's title and fields in database
- Updates `modified_at` timestamp automatically
- Returns updated Note
- Soft validation: allows legacy fields, validates types for schema fields

**3. `delete_note(note_id: String, strategy: DeleteStrategy) -> DeleteResult`**
```rust
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeleteStrategy {
    DeleteAll,
    PromoteChildren,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteResult {
    deleted_count: usize,
    affected_ids: Vec<String>,
}

#[tauri::command]
fn delete_note(
    window: Window,
    state: State<'_, AppState>,
    note_id: String,
    strategy: DeleteStrategy,
) -> Result<DeleteResult, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label).ok_or("No workspace open")?;

    workspace.delete_note(&note_id, strategy)
        .map_err(|e| e.to_string())
}
```
- `DeleteAll`: Recursively delete note and all descendants
- `PromoteChildren`: Delete note, move children to parent's level (update parent_id)
- Updates position values for affected siblings
- Returns count of deleted notes and their IDs

**4. `count_children(note_id: String) -> usize`**
```rust
#[tauri::command]
fn count_children(
    window: Window,
    state: State<'_, AppState>,
    note_id: String,
) -> Result<usize, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;

    workspace.count_children(&note_id)
        .map_err(|e| e.to_string())
}
```
- Counts direct children of note (where parent_id = note_id)
- Used by frontend to determine delete dialog options

### Core Library Changes

**Add to `Workspace` implementation:**

```rust
impl Workspace {
    pub fn update_note(
        &mut self,
        note_id: &str,
        title: String,
        fields: HashMap<String, FieldValue>,
    ) -> Result<Note> {
        let now = chrono::Utc::now().timestamp();

        self.storage.connection().execute(
            "UPDATE notes SET title = ?1, fields = ?2, modified_at = ?3 WHERE id = ?4",
            params![title, serde_json::to_string(&fields)?, now, note_id],
        )?;

        self.get_note(note_id)
    }

    pub fn delete_note(
        &mut self,
        note_id: &str,
        strategy: DeleteStrategy,
    ) -> Result<DeleteResult> {
        match strategy {
            DeleteStrategy::DeleteAll => self.delete_note_recursive(note_id),
            DeleteStrategy::PromoteChildren => self.delete_note_promote(note_id),
        }
    }

    fn delete_note_recursive(&mut self, note_id: &str) -> Result<DeleteResult> {
        let mut affected_ids = vec![note_id.to_string()];
        let children = self.get_children(note_id)?;

        for child in children {
            let result = self.delete_note_recursive(&child.id)?;
            affected_ids.extend(result.affected_ids);
        }

        self.storage.connection().execute(
            "DELETE FROM notes WHERE id = ?1",
            params![note_id],
        )?;

        Ok(DeleteResult {
            deleted_count: affected_ids.len(),
            affected_ids,
        })
    }

    fn delete_note_promote(&mut self, note_id: &str) -> Result<DeleteResult> {
        let note = self.get_note(note_id)?;

        // Update children to point to grandparent
        self.storage.connection().execute(
            "UPDATE notes SET parent_id = ?1 WHERE parent_id = ?2",
            params![note.parent_id, note_id],
        )?;

        // Delete the note
        self.storage.connection().execute(
            "DELETE FROM notes WHERE id = ?1",
            params![note_id],
        )?;

        // Reorder siblings
        self.reorder_siblings(note.parent_id.as_deref())?;

        Ok(DeleteResult {
            deleted_count: 1,
            affected_ids: vec![note_id.to_string()],
        })
    }

    pub fn count_children(&self, note_id: &str) -> Result<usize> {
        let count: usize = self.storage.connection().query_row(
            "SELECT COUNT(*) FROM notes WHERE parent_id = ?1",
            params![note_id],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    fn get_children(&self, parent_id: &str) -> Result<Vec<Note>> {
        // Query notes where parent_id = parent_id
        // Implementation similar to list_all_notes with filter
    }

    fn reorder_siblings(&mut self, parent_id: Option<&str>) -> Result<()> {
        // Fetch siblings, renumber positions 0, 1, 2...
    }
}
```

### Database Schema

**No changes needed** - all operations use existing columns:
- `notes.title` (TEXT)
- `notes.fields` (JSON)
- `notes.parent_id` (TEXT)
- `notes.position` (INTEGER)
- `notes.modified_at` (INTEGER)

## Frontend Changes

### Type Definitions (types.ts)

```typescript
// Add to existing types

export interface FieldDefinition {
  name: string;
  fieldType: string;  // "text" | "number" | "boolean"
  required: boolean;
}

export enum DeleteStrategy {
  DeleteAll = "DeleteAll",
  PromoteChildren = "PromoteChildren",
}

export interface DeleteResult {
  deletedCount: number;
  affectedIds: string[];
}
```

### New Tauri Command Bindings

```typescript
// Add to tauri command wrappers

export async function getSchemaFields(nodeType: string): Promise<FieldDefinition[]> {
  return invoke('get_schema_fields', { nodeType });
}

export async function updateNote(
  noteId: string,
  title: string,
  fields: Record<string, FieldValue>
): Promise<Note> {
  return invoke('update_note', { noteId, title, fields });
}

export async function deleteNote(
  noteId: string,
  strategy: DeleteStrategy
): Promise<DeleteResult> {
  return invoke('delete_note', { noteId, strategy });
}

export async function countChildren(noteId: string): Promise<number> {
  return invoke('count_children', { noteId });
}
```

## Error Handling

### Frontend Errors

**Unsaved Changes:**
- Track dirty state: `isDirty = (editedTitle !== note.title) || (JSON.stringify(editedFields) !== JSON.stringify(note.fields))`
- On navigate to different note: if dirty, show confirm dialog "You have unsaved changes. Leave?"
- On "Cancel" button: if dirty, show confirm dialog "Discard changes?"
- On window close: browser's `beforeunload` event if dirty

**Validation:**
- Required fields: Check on save, show error if empty
- Number fields: Validate numeric input, show error for non-numbers
- Show inline validation errors below field
- Don't submit if validation fails

**Backend Command Errors:**
- `get_schema_fields` fails → fallback to showing only legacy fields, show warning banner
- `update_note` fails → show error in StatusMessage, stay in edit mode, preserve changes
- `delete_note` fails → show error in StatusMessage, close dialog, note remains
- `count_children` fails → log error, disable delete button

### Backend Errors

**Database Errors:**
- Connection lost → return error message to frontend
- Note not found → return "Note not found" (may be deleted in another window)
- Foreign key constraint violations → shouldn't happen with our delete strategies

**Schema Errors:**
- Schema not found for node type → return empty field list, frontend handles gracefully
- Invalid field type in schema → validation prevents this at schema load time

### Edge Cases

**Note Deleted in Another Window:**
- User has note selected in window A
- User deletes same note in window B
- User clicks "Edit" or "Save" in window A → backend returns "Note not found"
- Show error: "This note was deleted. Refreshing..." → refresh tree view

**Concurrent Edits:**
- Window A and B both editing same note
- Window A saves changes
- Window B saves changes → last write wins (overwrites A's changes)
- No conflict resolution in Phase 4
- Show success message but note it's last-write-wins behavior

**Empty Workspace After Delete:**
- User deletes last note → auto-select returns None
- Show empty state in InfoPanel: "No notes yet. Use Edit > Add Note to create one."

**Legacy Fields:**
- Note has field "old_field" not in current schema
- View mode: render "old_field" below schema fields with "(legacy)" label
- Edit mode: allow editing, preserve value on save
- User can't delete legacy fields (would need explicit "Remove field" feature)

## Testing

### Manual Testing Scenarios

**1. View Mode Field Display**
- Select note with various field types
- Verify title displays prominently
- Verify all schema fields render with correct values
- Verify legacy fields (not in schema) display with "(legacy)" indicator
- Verify metadata section shows type, timestamps, ID
- Verify "Edit" and "Delete" buttons present

**2. Edit Mode Toggle**
- Click "Edit" → verify switches to edit mode
- Verify title becomes input field
- Verify all fields become appropriate editors (text area, number input, checkbox)
- Verify metadata still shows (read-only)
- Verify "Save" and "Cancel" buttons present
- Verify visual indicator of edit mode (border/background)

**3. Edit and Save**
- Edit title → Save → verify title updates in tree and detail panel
- Edit text field → Save → verify value persists, shows in view mode
- Edit number field → Save → verify number stored and displayed correctly
- Edit boolean field → Save → verify checkbox state persists
- Close workspace → reopen → verify all changes persisted

**4. Cancel Changes**
- Edit several fields → Cancel (no changes) → returns to view mode instantly
- Edit several fields → Cancel (with changes) → confirms "Discard changes?"
- Confirm discard → returns to view mode, changes lost
- Deny discard → stays in edit mode

**5. Dirty State Tracking**
- Edit field → try to select different note → warns "You have unsaved changes. Leave?"
- Stay → remains in edit mode on same note
- Leave → switches to other note, changes discarded
- Edit field → try to close window → browser confirms

**6. Validation**
- Set required text field to empty → Save → shows error "This field is required"
- Enter "abc" in number field → shows error "Must be a number"
- Fix errors → Save → succeeds

**7. Delete Note Without Children**
- Select leaf note (no children) → Delete
- Verify simple confirmation dialog "Delete [title]?"
- Cancel → dialog closes, note remains
- Delete → Confirm → note disappears from tree
- Verify next sibling auto-selected (or previous, or parent)

**8. Delete Note With Children**
- Select note with 2 children → Delete
- Verify dialog shows: "This note has 2 children"
- Verify radio buttons: "Delete all (3 notes total)" and "Promote children"
- Choose "Delete all" → Confirm → entire subtree disappears
- Undo (recreate structure) → Delete again → Choose "Promote children"
- Verify note deleted, children moved up to grandparent level
- Verify sibling positions updated correctly

**9. Hybrid Field Rendering**
- Create note, add custom field via direct DB edit
- Restart app (or schema change that removes field)
- Select note → verify custom field shows in view mode with "(legacy)"
- Edit mode → verify can edit legacy field
- Save → verify legacy field preserved

**10. Schema Not Found Fallback**
- Manually corrupt schema or use unknown node type
- Select note → verify warning banner "Schema not found, showing existing fields only"
- Verify all existing fields from note.fields render
- Edit mode → verify can edit all existing fields

**11. Multi-Window Scenarios**
- Open same workspace in two windows
- Edit and save note in window A
- Window B still shows old data → refresh tree to see changes
- Delete note in window A → window B tries to edit same note → shows "Note was deleted" error

**12. Empty Workspace After Delete**
- Delete all notes until workspace empty
- Verify empty state shows: "No notes yet"
- Add note via menu → tree populates again

### Backend Unit Tests

**`update_note` tests:**
- Updates title correctly
- Updates fields correctly
- Updates modified_at timestamp
- Returns updated Note
- Errors if note_id not found

**`delete_note` tests:**
- `DeleteAll` deletes note and all descendants recursively
- `DeleteAll` returns correct deleted_count and affected_ids
- `PromoteChildren` deletes note only
- `PromoteChildren` moves children to grandparent
- `PromoteChildren` updates sibling positions
- Errors if note_id not found

**`count_children` tests:**
- Returns 0 for leaf notes
- Returns correct count for notes with children
- Errors if note_id not found

**`get_schema_fields` tests:**
- Returns field definitions for valid node type
- Errors for unknown node type

## Implementation Notes

### Phase Boundaries

**Phase 4 (this):**
- View/Edit mode toggle
- Edit title and custom fields
- Plain text editing only
- Delete with confirmation and child handling
- Hybrid field rendering

**Phase 5 (future):**
- Markdown support for text fields
- Rich text editing
- Field type validation improvements
- Conflict resolution for concurrent edits
- Batch operations (multi-select + delete multiple)

### Future Enhancements (Not Phase 4)

- Markdown preview for text fields
- Rich text editor (WYSIWYG)
- Add/remove custom fields via UI
- Reorder fields via drag-drop
- Field-level undo/redo
- Auto-save (debounced)
- Keyboard shortcuts (Ctrl+S to save, Esc to cancel)
- Field history/versioning

### Dependencies

- No new external dependencies
- Uses existing: React, Tailwind CSS, Tauri commands, serde

## Success Criteria

Phase 4 is complete when:
- ✅ View mode shows title, all fields (schema + legacy), metadata
- ✅ Edit mode allows editing title and all fields with appropriate inputs
- ✅ Save button persists changes to database
- ✅ Cancel button discards changes and returns to view mode
- ✅ Dirty state tracking warns on navigation/close
- ✅ Validation prevents saving invalid data
- ✅ Delete works for notes without children (simple confirmation)
- ✅ Delete works for notes with children (strategy selection)
- ✅ Hybrid field rendering shows schema + legacy fields
- ✅ Schema not found handled gracefully (legacy-only mode)
- ✅ All manual testing scenarios pass
- ✅ All backend unit tests pass
