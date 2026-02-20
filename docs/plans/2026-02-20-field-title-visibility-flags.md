# Field & Title Visibility Flags Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add `can_view` / `can_edit` flags to schema field definitions, and `title_can_view` / `title_can_edit` flags at the schema level, so each note type can control which fields are visible in view vs. edit mode and whether the title input appears in edit mode.

**Architecture:** The Rust `FieldDefinition` and `Schema` structs each get new boolean fields (defaulting to `true`). The Rhai parser reads them with optional fallback. The Tauri command `get_schema_fields` is updated to return a richer `SchemaInfo` type that includes title flags alongside field definitions. The frontend `InfoPanel` reads these flags and conditionally renders fields and the title input.

**Tech Stack:** Rust (rhai, serde), Tauri IPC, TypeScript/React

---

### Task 1: Add `can_view` / `can_edit` to `FieldDefinition`

**Files:**
- Modify: `krillnotes-core/src/core/scripting/schema.rs:12-16` (struct)
- Modify: `krillnotes-core/src/core/scripting/schema.rs:71-76` (Rhai parsing)
- Test: `krillnotes-core/src/core/scripting/mod.rs`

**Step 1: Write the failing test**

In `mod.rs`, add inside the existing `#[cfg(test)] mod tests { ... }` block (after the existing tests):

```rust
#[test]
fn test_field_can_view_can_edit_defaults_to_true() {
    let mut registry = ScriptRegistry::new();
    registry.load_script(r#"
        schema("TestVis", #{
            fields: [
                #{ name: "f1", type: "text" },
            ]
        });
    "#).unwrap();
    let schema = registry.get_schema("TestVis").unwrap();
    assert!(schema.fields[0].can_view, "can_view should default to true");
    assert!(schema.fields[0].can_edit, "can_edit should default to true");
}

#[test]
fn test_field_can_view_can_edit_explicit_false() {
    let mut registry = ScriptRegistry::new();
    registry.load_script(r#"
        schema("TestVis2", #{
            fields: [
                #{ name: "view_only", type: "text", can_edit: false },
                #{ name: "edit_only", type: "text", can_view: false },
            ]
        });
    "#).unwrap();
    let schema = registry.get_schema("TestVis2").unwrap();
    assert!(schema.fields[0].can_view);
    assert!(!schema.fields[0].can_edit);
    assert!(!schema.fields[1].can_view);
    assert!(schema.fields[1].can_edit);
}
```

**Step 2: Run tests to verify they fail**

```bash
cargo test -p krillnotes-core test_field_can_view -- --nocapture
```

Expected: FAIL — `FieldDefinition` has no `can_view` / `can_edit` fields.

**Step 3: Update `FieldDefinition` struct**

In `krillnotes-core/src/core/scripting/schema.rs`, replace lines 12–16:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldDefinition {
    pub name: String,
    pub field_type: String,
    pub required: bool,
    pub can_view: bool,
    pub can_edit: bool,
}
```

**Step 4: Update Rhai parsing to read the new flags**

In `parse_from_rhai`, after the existing `required` block (after line 74), and before `fields.push(...)`:

```rust
let can_view = field_map
    .get("can_view")
    .and_then(|v| v.clone().try_cast::<bool>())
    .unwrap_or(true);

let can_edit = field_map
    .get("can_edit")
    .and_then(|v| v.clone().try_cast::<bool>())
    .unwrap_or(true);
```

Update the `fields.push(...)` call at line 76 to include the new fields:

```rust
fields.push(FieldDefinition {
    name: field_name,
    field_type,
    required,
    can_view,
    can_edit,
});
```

**Step 5: Run tests to verify they pass**

```bash
cargo test -p krillnotes-core -- --nocapture
```

Expected: All tests PASS (existing tests also still pass since `can_view`/`can_edit` default to `true`).

**Step 6: Commit**

```bash
git add krillnotes-core/src/core/scripting/schema.rs krillnotes-core/src/core/scripting/mod.rs
git commit -m "feat: add can_view and can_edit flags to FieldDefinition"
```

---

### Task 2: Add `title_can_view` / `title_can_edit` to `Schema`

**Files:**
- Modify: `krillnotes-core/src/core/scripting/schema.rs:19-23` (Schema struct)
- Modify: `krillnotes-core/src/core/scripting/schema.rs:49-80` (parse_from_rhai)
- Test: `krillnotes-core/src/core/scripting/mod.rs`

**Step 1: Write the failing tests**

Add to the test block in `mod.rs`:

```rust
#[test]
fn test_schema_title_flags_default_to_true() {
    let mut registry = ScriptRegistry::new();
    registry.load_script(r#"
        schema("TitleTest", #{
            fields: [
                #{ name: "name", type: "text" },
            ]
        });
    "#).unwrap();
    let schema = registry.get_schema("TitleTest").unwrap();
    assert!(schema.title_can_view, "title_can_view should default to true");
    assert!(schema.title_can_edit, "title_can_edit should default to true");
}

#[test]
fn test_schema_title_can_edit_false() {
    let mut registry = ScriptRegistry::new();
    registry.load_script(r#"
        schema("TitleHidden", #{
            title_can_edit: false,
            fields: [
                #{ name: "name", type: "text" },
            ]
        });
    "#).unwrap();
    let schema = registry.get_schema("TitleHidden").unwrap();
    assert!(schema.title_can_view);
    assert!(!schema.title_can_edit);
}
```

**Step 2: Run tests to verify they fail**

```bash
cargo test -p krillnotes-core test_schema_title -- --nocapture
```

Expected: FAIL — `Schema` has no `title_can_view` / `title_can_edit` fields.

**Step 3: Update `Schema` struct**

In `schema.rs`, replace lines 19–23:

```rust
#[derive(Debug, Clone)]
pub struct Schema {
    pub name: String,
    pub fields: Vec<FieldDefinition>,
    pub title_can_view: bool,
    pub title_can_edit: bool,
}
```

**Step 4: Update `parse_from_rhai` to read title flags and update the return value**

At the end of `parse_from_rhai`, after the `fields` vec is built (before `Ok(...)`), add:

```rust
let title_can_view = def
    .get("title_can_view")
    .and_then(|v| v.clone().try_cast::<bool>())
    .unwrap_or(true);

let title_can_edit = def
    .get("title_can_edit")
    .and_then(|v| v.clone().try_cast::<bool>())
    .unwrap_or(true);

Ok(Schema { name: name.to_string(), fields, title_can_view, title_can_edit })
```

Remove the old `Ok(Schema { name: name.to_string(), fields })` line.

**Step 5: Run all tests**

```bash
cargo test -p krillnotes-core -- --nocapture
```

Expected: All tests PASS.

**Step 6: Commit**

```bash
git add krillnotes-core/src/core/scripting/schema.rs krillnotes-core/src/core/scripting/mod.rs
git commit -m "feat: add title_can_view and title_can_edit flags to Schema"
```

---

### Task 3: Update Tauri command to expose title flags to the frontend

The `get_schema_fields` command in `lib.rs` currently returns `Vec<FieldDefinition>`. We need it to also return the title flags. The cleanest approach is to return a new `SchemaInfo` struct.

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs:381-394`

**Step 1: Add `SchemaInfo` struct and update `get_schema_fields`**

Find the imports block near the top of `lib.rs` where `FieldDefinition` is imported from `krillnotes_core`. Add a new serializable wrapper struct just before the `get_schema_fields` function (around line 375):

```rust
/// Response type for the `get_schema_fields` Tauri command, bundling field
/// definitions with schema-level title visibility flags.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SchemaInfo {
    fields: Vec<FieldDefinition>,
    title_can_view: bool,
    title_can_edit: bool,
}
```

Then update `get_schema_fields` (lines 381–394) to return `SchemaInfo`:

```rust
#[tauri::command]
fn get_schema_fields(
    window: tauri::Window,
    state: State<'_, AppState>,
    node_type: String,
) -> std::result::Result<SchemaInfo, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;

    let schema = workspace.script_registry().get_schema(&node_type)
        .map_err(|e: KrillnotesError| e.to_string())?;

    Ok(SchemaInfo {
        fields: schema.fields,
        title_can_view: schema.title_can_view,
        title_can_edit: schema.title_can_edit,
    })
}
```

**Step 2: Verify compilation**

```bash
cargo build -p krillnotes-desktop
```

Expected: Builds without errors.

**Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: expose title visibility flags via get_schema_fields command"
```

---

### Task 4: Update TypeScript types

**Files:**
- Modify: `krillnotes-desktop/src/types.ts:34-38`

**Step 1: Update `FieldDefinition` interface**

Find the `FieldDefinition` interface (lines 34–38) and add the two new optional fields:

```typescript
export interface FieldDefinition {
  name: string;
  fieldType: string;
  required: boolean;
  canView: boolean;
  canEdit: boolean;
}
```

**Step 2: Add `SchemaInfo` interface**

Add a new interface below `FieldDefinition`:

```typescript
export interface SchemaInfo {
  fields: FieldDefinition[];
  titleCanView: boolean;
  titleCanEdit: boolean;
}
```

**Step 3: Verify TypeScript compiles**

```bash
cd krillnotes-desktop && npm run build 2>&1 | head -50
```

Expected: No type errors (there may be errors in the next step about `invoke` return type — that's expected and will be fixed in Task 5).

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/types.ts
git commit -m "feat: add canView, canEdit to FieldDefinition and SchemaInfo type"
```

---

### Task 5: Update InfoPanel to use visibility flags

**Files:**
- Modify: `krillnotes-desktop/src/components/InfoPanel.tsx`

**Step 1: Update state and `invoke` call to use `SchemaInfo`**

At the top of `InfoPanel.tsx`, update the import:

```typescript
import type { Note, FieldDefinition, FieldValue, SchemaInfo } from '../types';
```

Change the `schemaFields` state (line 15) to store the full schema info:

```typescript
const [schemaInfo, setSchemaInfo] = useState<SchemaInfo>({
  fields: [],
  titleCanView: true,
  titleCanEdit: true,
});
```

Update the `invoke` call (lines 29–34) from:

```typescript
invoke<FieldDefinition[]>('get_schema_fields', { nodeType: selectedNote.nodeType })
  .then(fields => setSchemaFields(fields))
  .catch(err => {
    console.error('Failed to fetch schema fields:', err);
    setSchemaFields([]);
  });
```

To:

```typescript
invoke<SchemaInfo>('get_schema_fields', { nodeType: selectedNote.nodeType })
  .then(info => setSchemaInfo(info))
  .catch(err => {
    console.error('Failed to fetch schema fields:', err);
    setSchemaInfo({ fields: [], titleCanView: true, titleCanEdit: true });
  });
```

Also update the reset in the first `useEffect` (around line 24) from `setSchemaFields([])` to:

```typescript
setSchemaInfo({ fields: [], titleCanView: true, titleCanEdit: true });
```

**Step 2: Update `schemaFieldNames` and legacy field derivation**

Find the two lines using `schemaFields` (around line 117–119) and update to use `schemaInfo.fields`:

```typescript
const schemaFieldNames = new Set(schemaInfo.fields.map(f => f.name));
const allFieldNames = Object.keys(selectedNote.fields);
const legacyFieldNames = allFieldNames.filter(name => !schemaFieldNames.has(name));
```

**Step 3: Add conditional title rendering**

Find the title block in the header section (lines 125–138).

The view-mode title (the `<h1>`) is wrapped in the `else` branch. Update that branch to be conditional:

```typescript
{isEditing ? (
  schemaInfo.titleCanEdit ? (
    <input
      ref={titleInputRef}
      type="text"
      value={editedTitle}
      onChange={(e) => {
        setEditedTitle(e.target.value);
        setIsDirty(true);
      }}
      className="text-4xl font-bold bg-background border border-border rounded-md px-2 py-1 flex-1"
    />
  ) : (
    <div className="flex-1" /> /* empty spacer so the Save/Cancel buttons stay right-aligned */
  )
) : (
  schemaInfo.titleCanView ? (
    <h1 className="text-4xl font-bold">{selectedNote.title}</h1>
  ) : null
)}
```

**Step 4: Update field rendering to filter by mode flags**

Find the `schemaFields.map(...)` block (lines 178–194). Replace it so that fields are filtered by the current mode:

```typescript
{schemaInfo.fields
  .filter(field => isEditing ? field.canEdit : field.canView)
  .map(field => (
    isEditing ? (
      <FieldEditor
        key={field.name}
        fieldName={field.name}
        value={editedFields[field.name] || { Text: '' }}
        required={field.required}
        onChange={(value) => handleFieldChange(field.name, value)}
      />
    ) : (
      <FieldDisplay
        key={field.name}
        fieldName={field.name}
        value={selectedNote.fields[field.name] || { Text: '' }}
      />
    )
  ))
}
```

**Step 5: Verify TypeScript compiles**

```bash
cd krillnotes-desktop && npm run build 2>&1 | head -50
```

Expected: No type errors.

**Step 6: Commit**

```bash
git add krillnotes-desktop/src/components/InfoPanel.tsx
git commit -m "feat: apply can_view/can_edit and title visibility flags in InfoPanel"
```

---

### Task 6: Update contact.rhai with `title_can_edit: false`

**Files:**
- Modify: `krillnotes-core/src/system_scripts/contact.rhai`

**Step 1: Update the contact schema definition**

Open `contact.rhai`. The `schema("Contact", #{ ... })` call starts the definition. Add `title_can_edit: false` as the first key in the map, before `fields`:

```rhai
schema("Contact", #{
    title_can_edit: false,
    fields: [
        #{ name: "first_name",      type: "text",  required: true  },
        ...
    ]
});
```

**Step 2: Verify the existing contact schema test still passes**

```bash
cargo test -p krillnotes-core test_contact -- --nocapture
```

Expected: All contact tests PASS. The test `test_contact_schema_loaded` may not check `title_can_edit` yet — that's fine, the schema still loads correctly.

**Step 3: Add a test confirming contact's title_can_edit is false**

Add to the test block in `mod.rs`:

```rust
#[test]
fn test_contact_title_can_edit_false() {
    let mut registry = ScriptRegistry::new();
    // ScriptRegistry::new() loads built-in scripts (including contact.rhai)
    let schema = registry.get_schema("Contact").unwrap();
    assert!(!schema.title_can_edit, "Contact title_can_edit should be false");
    assert!(schema.title_can_view, "Contact title_can_view should still be true");
}
```

**Step 4: Run the new test**

```bash
cargo test -p krillnotes-core test_contact_title_can_edit -- --nocapture
```

Expected: PASS.

**Step 5: Run all tests**

```bash
cargo test -p krillnotes-core -- --nocapture
```

Expected: All tests PASS.

**Step 6: Commit**

```bash
git add krillnotes-core/src/system_scripts/contact.rhai krillnotes-core/src/core/scripting/mod.rs
git commit -m "feat: set title_can_edit false on Contact schema and add test"
```

---

### Task 7: Manual smoke test

Start the app and verify:

1. Open a **Text Note** — title shows in view mode, title input shows in edit mode, all fields visible in both modes.
2. Open a **Contact Note** — title shows in view mode (`title_can_view: true` default), but the title input is **absent** in edit mode (`title_can_edit: false`). All contact fields (first_name, last_name, etc.) show as before.
3. After saving a Contact note, the title is still auto-derived by `on_save` and displays correctly in view mode.

```bash
cd krillnotes-desktop && npm run tauri dev
```
