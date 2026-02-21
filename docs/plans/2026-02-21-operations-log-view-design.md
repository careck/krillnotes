# Operations Log View Design

## Context

The operations log records every workspace mutation (note CRUD, user script CRUD) for future cloud sync. Currently it is write-only — operations are logged and purged but never exposed to the user. This feature adds a read-only view so users can inspect pending operations and optionally purge the log to compress the database.

## Requirements

- Accessible via **View > Operations Log** menu item
- Flat list of operations with: date/time, target name (note title or script name), operation type
- Filter by specific operation type (all 7 types)
- Filter by date range (from/to)
- Purge button that deletes ALL operations from the database
- Newest operations shown first

## Architecture

Three layers: backend query methods -> Tauri commands -> React dialog.

### Backend

**New struct — `OperationSummary`** (in `operation_log.rs`):

| Field | Type | Description |
|-------|------|-------------|
| operation_id | String | UUID |
| timestamp | i64 | Unix seconds |
| device_id | String | Originating device |
| operation_type | String | e.g. "CreateNote" |
| target_name | String | Note title or script name |

Lightweight summary extracted from `operation_data` JSON — avoids sending full source code blobs to the frontend.

**New OperationLog methods:**
- `list(conn, type_filter, since, until) -> Result<Vec<OperationSummary>>` — queries operations table, deserializes JSON to extract target name, returns newest-first
- `purge_all(conn) -> Result<()>` — `DELETE FROM operations`

**New Workspace methods:**
- `list_operations(type_filter, since, until)` — delegates to operation_log
- `purge_all_operations()` — delegates to operation_log

**New Tauri commands:**
- `list_operations(type_filter?, since?, until?)` — returns `Vec<OperationSummary>`
- `purge_operations()` — deletes all operations, returns count deleted

### Frontend

**OperationsLogDialog** — modal dialog (pattern follows ScriptManagerDialog):

1. **Filter bar**: operation type dropdown + date range inputs (from/to)
2. **Scrollable list**: rows with date/time | target name | operation type badge
3. **Footer**: operation count + "Purge All" button (with confirmation dialog)

**Menu integration**: New "View > Operations Log" menu item, handled via existing `menu-action` event pattern.

**New TypeScript type:**
```typescript
interface OperationSummary {
  operationId: string;
  timestamp: number;
  deviceId: string;
  operationType: string;
  targetName: string;
}
```

### Target Name Extraction

From the `operation_data` JSON, extract:
- CreateNote / UpdateField / DeleteNote / MoveNote → `title` field (or note_id as fallback for delete/move)
- CreateUserScript / UpdateUserScript → `name` field
- DeleteUserScript → `script_id` (no name available after deletion)

## Files to Modify

**Backend (krillnotes-core):**
- `src/core/operation_log.rs` — add `OperationSummary`, `list()`, `purge_all()`
- `src/core/workspace.rs` — add `list_operations()`, `purge_all_operations()`
- `src/lib.rs` — export new types

**Tauri commands (krillnotes-desktop/src-tauri):**
- `src/lib.rs` — add `list_operations`, `purge_operations` commands

**Frontend (krillnotes-desktop/src):**
- `components/OperationsLogDialog.tsx` — new component
- `components/WorkspaceView.tsx` — add dialog state + menu-action handler
- `types.ts` — add `OperationSummary` type
- Menu setup — add "View > Operations Log" item
