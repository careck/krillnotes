# User Scripts Design

## Overview

Enable user-defined Rhai scripts stored in the workspace database, loaded after system scripts. Scripts are self-contained files with front matter metadata. A dedicated management dialog allows listing, creating, editing, toggling, reordering, and deleting user scripts.

## Data Model

### SQLite Table

```sql
CREATE TABLE IF NOT EXISTS user_scripts (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    source_code TEXT NOT NULL,
    load_order INTEGER NOT NULL DEFAULT 0,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    modified_at INTEGER NOT NULL
);
```

- `name` and `description` are cached values extracted from script front matter on save
- `load_order` controls execution order (ascending)
- `enabled` allows toggling scripts without deletion

### Front Matter Format

Scripts embed metadata as comment-based front matter:

```rhai
// @name: Project Task
// @description: Custom task tracking for projects

schema("ProjectTask", #{
    fields: [
        #{ name: "status", type: "text" },
    ]
});
```

Parser reads `// @key: value` lines from the top of the source, stopping at the first non-comment or non-`@` line. `@name` is required; `@description` is optional (defaults to empty string).

### Rust Type

```rust
pub struct UserScript {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source_code: String,
    pub load_order: i32,
    pub enabled: bool,
    pub created_at: i64,
    pub modified_at: i64,
}
```

### TypeScript Type

```typescript
interface UserScript {
    id: string;
    name: string;
    description: string;
    sourceCode: string;
    loadOrder: number;
    enabled: boolean;
    createdAt: number;
    modifiedAt: number;
}
```

## Script Loading Lifecycle

### On Workspace Open

After system scripts load in `ScriptRegistry::new()`, the workspace queries `user_scripts` table (sorted by `load_order`), then loads each enabled script. If a script fails to compile/execute, it is skipped with error logged; workspace still opens.

### On Script Save (New or Edit)

1. Parse front matter to extract `name` and `description`
2. Validate: `@name` is required, must be unique among user scripts
3. Save source code and cached metadata to `user_scripts` table
4. Full reload: clear all user-registered schemas and hooks, re-execute all enabled user scripts in `load_order` order
5. Return success or compilation/execution error to UI

If compilation fails, the script is still saved but `enabled` is set to `false`.

### On Script Delete

1. Show warning dialog about potential data loss
2. Remove from `user_scripts` table
3. Full reload of remaining user scripts

### Script Override Policy

User scripts can override system schemas. Scripts are responsible for checking existing state and implementing migration strategies. New Rhai API functions support this:

- `schema_exists(name: String) -> bool` -- check if a schema is registered
- `get_schema_fields(name: String) -> Array` -- get current field definitions

## Backend API

### Tauri Commands

| Command | Parameters | Returns | Purpose |
|---------|-----------|---------|---------|
| `list_user_scripts` | window_label | `Vec<UserScript>` | List all scripts sorted by load_order |
| `get_user_script` | window_label, script_id | `UserScript` | Get single script with source |
| `create_user_script` | window_label, source_code | `Result<UserScript, String>` | Create + reload |
| `update_user_script` | window_label, script_id, source_code | `Result<UserScript, String>` | Update + reload |
| `delete_user_script` | window_label, script_id | `Result<(), String>` | Delete + reload |
| `toggle_user_script` | window_label, script_id, enabled | `Result<(), String>` | Enable/disable + reload |
| `reorder_user_script` | window_label, script_id, new_load_order | `Result<(), String>` | Change order + reload |

### ScriptRegistry Changes

- Track schema/hook origin with `ScriptSource` enum (`System` | `User`)
- `clear_user_scripts()` -- remove all user-registered schemas and hooks
- `reload_user_scripts(scripts: &[UserScript])` -- clear then load each enabled script in order

## Frontend UI

### Script Management Dialog

Opens from workspace menu. Two views within a single dialog:

#### List View (default)

- Header with "User Scripts" title and "Add" button
- Each row: enabled checkbox, name, load order, description, edit button
- Checkbox toggles call `toggle_user_script`
- "Add" switches to editor with template
- "Edit" switches to editor with script loaded
- "Close" button at bottom

#### Editor View

- Full CodeMirror editor with the script source (including front matter)
- Error display area below editor (compilation errors)
- "Delete" button (left-aligned, shows confirmation dialog)
- "Cancel" and "Save" buttons (right-aligned)
- New script template pre-populates with `// @name:` and `// @description:` lines

### CodeMirror Integration

- Use `@codemirror/view` + `@codemirror/state` with a Rust-like syntax mode (closest to Rhai)
- Line numbers enabled
- Theme matching the application's light/dark mode

## Error Handling

- **Compilation errors on save**: script saved but auto-disabled, error shown in editor
- **Missing @name**: validation error, save blocked
- **Duplicate name**: validation error, save blocked
- **Workspace open failures**: script skipped, workspace opens normally
- **Reload failures**: change saved, report which scripts failed to reload
- **Delete confirmation**: warning about schema removal and potential display issues for existing notes

## Migration

In `storage.rs` `open()` method, check if `user_scripts` table exists; create it if missing. Follows the existing `is_expanded` migration pattern.
