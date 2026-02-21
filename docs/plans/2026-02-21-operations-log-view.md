# Operations Log View Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a read-only view showing all operations in the log with filtering by type and date range, plus a purge button.

**Architecture:** New `OperationSummary` struct returned by backend query methods, exposed via two Tauri commands (`list_operations`, `purge_operations`), rendered in a new `OperationsLogDialog` React component accessible from the View menu.

**Tech Stack:** Rust (rusqlite, serde_json), Tauri 2, React 19, TypeScript, Tailwind CSS

---

### Task 1: Add `OperationSummary` struct and `list` / `purge_all` methods to `OperationLog`

**Files:**
- Modify: `krillnotes-core/src/core/operation_log.rs`

**Step 1: Write the failing test for `list`**

Add to the existing `#[cfg(test)] mod tests` block at the bottom of `operation_log.rs` (after the existing `test_log_and_purge` test):

```rust
#[test]
fn test_list_operations() {
    let temp = NamedTempFile::new().unwrap();
    let mut storage = Storage::create(temp.path()).unwrap();
    let log = OperationLog::new(PurgeStrategy::LocalOnly { keep_last: 100 });

    let tx = storage.connection_mut().transaction().unwrap();

    let op1 = Operation::CreateNote {
        operation_id: "op-1".to_string(),
        timestamp: 1000,
        device_id: "dev-1".to_string(),
        note_id: "note-1".to_string(),
        parent_id: None,
        position: 0,
        node_type: "TextNote".to_string(),
        title: "My Note".to_string(),
        fields: HashMap::new(),
        created_by: 0,
    };
    let op2 = Operation::CreateUserScript {
        operation_id: "op-2".to_string(),
        timestamp: 2000,
        device_id: "dev-1".to_string(),
        script_id: "script-1".to_string(),
        name: "My Script".to_string(),
        description: "A script".to_string(),
        source_code: "// code".to_string(),
        load_order: 0,
        enabled: true,
    };
    log.log(&tx, &op1).unwrap();
    log.log(&tx, &op2).unwrap();
    tx.commit().unwrap();

    // List all operations (newest first)
    let results = log.list(storage.connection(), None, None, None).unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].operation_type, "CreateUserScript");
    assert_eq!(results[0].target_name, "My Script");
    assert_eq!(results[1].operation_type, "CreateNote");
    assert_eq!(results[1].target_name, "My Note");

    // Filter by type
    let filtered = log.list(storage.connection(), Some("CreateNote"), None, None).unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].target_name, "My Note");

    // Filter by date range
    let ranged = log.list(storage.connection(), None, Some(1500), None).unwrap();
    assert_eq!(ranged.len(), 1);
    assert_eq!(ranged[0].operation_type, "CreateUserScript");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test test_list_operations -p krillnotes-core -- --nocapture`
Expected: FAIL — `list` method does not exist.

**Step 3: Write the `OperationSummary` struct and `list` / `purge_all` methods**

Add `use rusqlite::Connection;` to the imports at the top (line 4).

Add the `OperationSummary` struct before the `OperationLog` struct (before line 21):

```rust
/// Lightweight summary of an operation for display in the UI.
#[derive(Debug, Clone, serde::Serialize)]
pub struct OperationSummary {
    pub operation_id: String,
    pub timestamp: i64,
    pub device_id: String,
    pub operation_type: String,
    pub target_name: String,
}
```

Add two public methods to the `impl OperationLog` block, after `purge_if_needed` (after line 83):

```rust
    /// Returns operations matching the given filters, newest first.
    ///
    /// - `type_filter`: if set, only return operations of this type (e.g. `"CreateNote"`)
    /// - `since`: if set, only return operations at or after this Unix timestamp
    /// - `until`: if set, only return operations at or before this Unix timestamp
    pub fn list(
        &self,
        conn: &Connection,
        type_filter: Option<&str>,
        since: Option<i64>,
        until: Option<i64>,
    ) -> Result<Vec<OperationSummary>> {
        let mut sql = String::from(
            "SELECT operation_id, timestamp, device_id, operation_type, operation_data FROM operations WHERE 1=1"
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(op_type) = type_filter {
            sql.push_str(" AND operation_type = ?");
            params.push(Box::new(op_type.to_string()));
        }
        if let Some(s) = since {
            sql.push_str(" AND timestamp >= ?");
            params.push(Box::new(s));
        }
        if let Some(u) = until {
            sql.push_str(" AND timestamp <= ?");
            params.push(Box::new(u));
        }
        sql.push_str(" ORDER BY timestamp DESC, id DESC");

        let mut stmt = conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            let operation_data: String = row.get(4)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                operation_data,
            ))
        })?;

        let mut results = Vec::new();
        for row in rows {
            let (operation_id, timestamp, device_id, operation_type, operation_data) = row?;
            let target_name = Self::extract_target_name(&operation_data);
            results.push(OperationSummary {
                operation_id,
                timestamp,
                device_id,
                operation_type,
                target_name,
            });
        }
        Ok(results)
    }

    /// Deletes all operations from the log.
    pub fn purge_all(&self, conn: &Connection) -> Result<usize> {
        let count = conn.execute("DELETE FROM operations", [])?;
        Ok(count)
    }
```

Add the private helper as a method on `OperationLog`, after `operation_type_name` (after line 95):

```rust
    /// Extracts a human-readable target name from the operation_data JSON.
    fn extract_target_name(json: &str) -> String {
        let Ok(val) = serde_json::from_str::<serde_json::Value>(json) else {
            return String::new();
        };
        // Note operations: prefer "title", fall back to "note_id"
        if let Some(title) = val.get("title").and_then(|v| v.as_str()) {
            return title.to_string();
        }
        // User script operations: prefer "name", fall back to "script_id"
        if let Some(name) = val.get("name").and_then(|v| v.as_str()) {
            return name.to_string();
        }
        // UpdateField: use "field" name
        if let Some(field) = val.get("field").and_then(|v| v.as_str()) {
            return field.to_string();
        }
        // Fallback: note_id or script_id
        val.get("note_id")
            .or_else(|| val.get("script_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }
```

**Step 4: Run test to verify it passes**

Run: `cargo test test_list_operations -p krillnotes-core -- --nocapture`
Expected: PASS

**Step 5: Write failing test for `purge_all`**

Add to the test module:

```rust
#[test]
fn test_purge_all() {
    let temp = NamedTempFile::new().unwrap();
    let mut storage = Storage::create(temp.path()).unwrap();
    let log = OperationLog::new(PurgeStrategy::LocalOnly { keep_last: 100 });

    let tx = storage.connection_mut().transaction().unwrap();
    for i in 0..5 {
        let op = Operation::CreateNote {
            operation_id: format!("op-{}", i),
            timestamp: 1000 + i,
            device_id: "dev-1".to_string(),
            note_id: format!("note-{}", i),
            parent_id: None,
            position: i as i32,
            node_type: "TextNote".to_string(),
            title: format!("Note {}", i),
            fields: HashMap::new(),
            created_by: 0,
        };
        log.log(&tx, &op).unwrap();
    }
    tx.commit().unwrap();

    let count = log.purge_all(storage.connection()).unwrap();
    assert_eq!(count, 5);

    let remaining = log.list(storage.connection(), None, None, None).unwrap();
    assert!(remaining.is_empty());
}
```

**Step 6: Run test to verify it passes** (already implemented above)

Run: `cargo test test_purge_all -p krillnotes-core -- --nocapture`
Expected: PASS

**Step 7: Commit**

```bash
git add krillnotes-core/src/core/operation_log.rs
git commit -m "feat: add OperationSummary, list and purge_all to OperationLog"
```

---

### Task 2: Export `OperationSummary` and add Workspace wrapper methods

**Files:**
- Modify: `krillnotes-core/src/lib.rs:19`
- Modify: `krillnotes-core/src/core/workspace.rs`

**Step 1: Export `OperationSummary` from crate root**

In `krillnotes-core/src/lib.rs`, change line 19 from:

```rust
    operation_log::{OperationLog, PurgeStrategy},
```

to:

```rust
    operation_log::{OperationLog, OperationSummary, PurgeStrategy},
```

**Step 2: Add `list_operations` and `purge_all_operations` methods to `Workspace`**

In `krillnotes-core/src/core/workspace.rs`, add after the `reorder_user_script` method (before the `reload_user_scripts` private method):

```rust
    // ── Operations log queries ───────────────────────────────────────

    /// Returns operation summaries matching the given filters, newest first.
    pub fn list_operations(
        &self,
        type_filter: Option<&str>,
        since: Option<i64>,
        until: Option<i64>,
    ) -> Result<Vec<crate::OperationSummary>> {
        self.operation_log.list(self.connection(), type_filter, since, until)
    }

    /// Deletes all operations from the log. Returns the number deleted.
    pub fn purge_all_operations(&self) -> Result<usize> {
        self.operation_log.purge_all(self.connection())
    }
```

**Step 3: Run all tests**

Run: `cargo test -p krillnotes-core`
Expected: All tests pass.

**Step 4: Commit**

```bash
git add krillnotes-core/src/lib.rs krillnotes-core/src/core/workspace.rs
git commit -m "feat: add list_operations and purge_all_operations to Workspace"
```

---

### Task 3: Add Tauri commands for operations

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Add the `list_operations` Tauri command**

After the `reorder_user_script` command (after line 640), add:

```rust
// ── Operations log commands ──────────────────────────────────────

/// Returns operation summaries matching the given filters.
#[tauri::command]
fn list_operations(
    window: tauri::Window,
    state: State<'_, AppState>,
    type_filter: Option<String>,
    since: Option<i64>,
    until: Option<i64>,
) -> std::result::Result<Vec<krillnotes_core::OperationSummary>, String> {
    let label = window.label();
    state.workspaces.lock()
        .expect("Mutex poisoned")
        .get(label)
        .ok_or("No workspace open")?
        .list_operations(type_filter.as_deref(), since, until)
        .map_err(|e| e.to_string())
}

/// Deletes all operations from the log.
#[tauri::command]
fn purge_operations(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<usize, String> {
    let label = window.label();
    state.workspaces.lock()
        .expect("Mutex poisoned")
        .get(label)
        .ok_or("No workspace open")?
        .purge_all_operations()
        .map_err(|e| e.to_string())
}
```

**Step 2: Register commands in the invoke_handler**

In the `tauri::generate_handler![]` block (around line 696), add `list_operations` and `purge_operations` after `reorder_user_script`.

**Step 3: Build to verify compilation**

Run: `cargo build`
Expected: Compiles successfully.

**Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: add list_operations and purge_operations Tauri commands"
```

---

### Task 4: Add "View > Operations Log" menu item

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/menu.rs:48-57` (View submenu)
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs:643-651` (MENU_MESSAGES)

**Step 1: Add menu item to View submenu**

In `menu.rs`, modify the View submenu (lines 48-57). Add the Operations Log item before the Refresh item:

```rust
            // View menu
            &SubmenuBuilder::new(app, "View")
                .items(&[
                    &PredefinedMenuItem::fullscreen(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &MenuItemBuilder::with_id("view_operations_log", "Operations Log...")
                        .build(app)?,
                    &PredefinedMenuItem::separator(app)?,
                    &MenuItemBuilder::with_id("view_refresh", "Refresh")
                        .accelerator("CmdOrCtrl+R")
                        .build(app)?,
                ])
                .build()?,
```

**Step 2: Add menu message mapping**

In `lib.rs`, add to the `MENU_MESSAGES` array (around line 643):

```rust
    ("view_operations_log", "View > Operations Log clicked"),
```

**Step 3: Build to verify**

Run: `cargo build`
Expected: Compiles successfully.

**Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/menu.rs krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: add View > Operations Log menu item"
```

---

### Task 5: Add `OperationSummary` TypeScript type

**Files:**
- Modify: `krillnotes-desktop/src/types.ts`

**Step 1: Add the type**

Append after the `UserScript` interface (after line 72):

```typescript
export interface OperationSummary {
  operationId: string;
  timestamp: number;
  deviceId: string;
  operationType: string;
  targetName: string;
}
```

**Step 2: Commit**

```bash
git add krillnotes-desktop/src/types.ts
git commit -m "feat: add OperationSummary TypeScript type"
```

---

### Task 6: Create `OperationsLogDialog` component

**Files:**
- Create: `krillnotes-desktop/src/components/OperationsLogDialog.tsx`

**Step 1: Create the component**

```typescript
import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ListFilter, Trash2 } from 'lucide-react';
import type { OperationSummary } from '../types';

interface OperationsLogDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

const OPERATION_TYPES = [
  'CreateNote',
  'UpdateField',
  'DeleteNote',
  'MoveNote',
  'CreateUserScript',
  'UpdateUserScript',
  'DeleteUserScript',
] as const;

function formatTimestamp(unix: number): string {
  const date = new Date(unix * 1000);
  return date.toLocaleString();
}

function OperationsLogDialog({ isOpen, onClose }: OperationsLogDialogProps) {
  const [operations, setOperations] = useState<OperationSummary[]>([]);
  const [typeFilter, setTypeFilter] = useState<string>('');
  const [sinceDate, setSinceDate] = useState('');
  const [untilDate, setUntilDate] = useState('');
  const [error, setError] = useState('');
  const [confirmPurge, setConfirmPurge] = useState(false);

  const loadOperations = useCallback(async () => {
    try {
      const since = sinceDate
        ? Math.floor(new Date(sinceDate + 'T00:00:00').getTime() / 1000)
        : undefined;
      const until = untilDate
        ? Math.floor(new Date(untilDate + 'T23:59:59').getTime() / 1000)
        : undefined;

      const result = await invoke<OperationSummary[]>('list_operations', {
        typeFilter: typeFilter || null,
        since: since ?? null,
        until: until ?? null,
      });
      setOperations(result);
      setError('');
    } catch (err) {
      setError(`Failed to load operations: ${err}`);
    }
  }, [typeFilter, sinceDate, untilDate]);

  useEffect(() => {
    if (isOpen) {
      setTypeFilter('');
      setSinceDate('');
      setUntilDate('');
      setConfirmPurge(false);
      setError('');
    }
  }, [isOpen]);

  useEffect(() => {
    if (isOpen) {
      loadOperations();
    }
  }, [isOpen, loadOperations]);

  const handlePurge = async () => {
    try {
      await invoke('purge_operations');
      setConfirmPurge(false);
      loadOperations();
    } catch (err) {
      setError(`Failed to purge operations: ${err}`);
    }
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-border rounded-lg shadow-lg w-[700px] max-h-[80vh] flex flex-col">
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-border">
          <h2 className="text-lg font-semibold flex items-center gap-2">
            <ListFilter className="w-5 h-5" />
            Operations Log
          </h2>
          <button
            onClick={onClose}
            className="text-muted-foreground hover:text-foreground text-xl leading-none px-1"
          >
            &times;
          </button>
        </div>

        {/* Filters */}
        <div className="flex items-center gap-3 px-4 py-2 border-b border-border bg-muted/30">
          <select
            value={typeFilter}
            onChange={(e) => setTypeFilter(e.target.value)}
            className="bg-background border border-input rounded px-2 py-1 text-sm"
          >
            <option value="">All types</option>
            {OPERATION_TYPES.map((t) => (
              <option key={t} value={t}>{t}</option>
            ))}
          </select>

          <label className="text-sm text-muted-foreground">From:</label>
          <input
            type="date"
            value={sinceDate}
            onChange={(e) => setSinceDate(e.target.value)}
            className="bg-background border border-input rounded px-2 py-1 text-sm"
          />

          <label className="text-sm text-muted-foreground">To:</label>
          <input
            type="date"
            value={untilDate}
            onChange={(e) => setUntilDate(e.target.value)}
            className="bg-background border border-input rounded px-2 py-1 text-sm"
          />
        </div>

        {/* Error */}
        {error && (
          <div className="px-4 py-2 text-sm text-red-600 bg-red-50 border-b border-border">
            {error}
          </div>
        )}

        {/* Operations list */}
        <div className="flex-1 overflow-y-auto">
          {operations.length === 0 ? (
            <div className="px-4 py-8 text-center text-muted-foreground text-sm">
              No operations found.
            </div>
          ) : (
            <table className="w-full text-sm">
              <thead className="bg-muted/30 sticky top-0">
                <tr>
                  <th className="text-left px-4 py-2 font-medium text-muted-foreground">Date &amp; Time</th>
                  <th className="text-left px-4 py-2 font-medium text-muted-foreground">Target</th>
                  <th className="text-right px-4 py-2 font-medium text-muted-foreground">Type</th>
                </tr>
              </thead>
              <tbody>
                {operations.map((op) => (
                  <tr key={op.operationId} className="border-b border-border/50 hover:bg-muted/20">
                    <td className="px-4 py-2 text-muted-foreground whitespace-nowrap">
                      {formatTimestamp(op.timestamp)}
                    </td>
                    <td className="px-4 py-2 truncate max-w-[250px]" title={op.targetName}>
                      {op.targetName || <span className="text-muted-foreground italic">—</span>}
                    </td>
                    <td className="px-4 py-2 text-right">
                      <span className="inline-block bg-muted text-muted-foreground rounded px-2 py-0.5 text-xs font-mono">
                        {op.operationType}
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-between px-4 py-3 border-t border-border">
          <span className="text-sm text-muted-foreground">
            {operations.length} operation{operations.length !== 1 ? 's' : ''}
          </span>
          <div className="flex items-center gap-2">
            {confirmPurge ? (
              <>
                <span className="text-sm text-red-600">Delete all operations?</span>
                <button
                  onClick={handlePurge}
                  className="bg-red-600 text-white px-3 py-1 rounded text-sm hover:bg-red-700"
                >
                  Confirm
                </button>
                <button
                  onClick={() => setConfirmPurge(false)}
                  className="bg-muted text-foreground px-3 py-1 rounded text-sm hover:bg-muted/80"
                >
                  Cancel
                </button>
              </>
            ) : (
              <button
                onClick={() => setConfirmPurge(true)}
                className="flex items-center gap-1 text-sm text-muted-foreground hover:text-red-600 px-3 py-1 rounded border border-border hover:border-red-300"
              >
                <Trash2 className="w-3.5 h-3.5" />
                Purge All
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

export default OperationsLogDialog;
```

**Step 2: Commit**

```bash
git add krillnotes-desktop/src/components/OperationsLogDialog.tsx
git commit -m "feat: create OperationsLogDialog component"
```

---

### Task 7: Wire up `OperationsLogDialog` in `WorkspaceView`

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx`

**Step 1: Add import**

Add with the other component imports at the top of the file:

```typescript
import OperationsLogDialog from './OperationsLogDialog';
```

**Step 2: Add dialog state**

After the `showScriptManager` state (line 43), add:

```typescript
  const [showOperationsLog, setShowOperationsLog] = useState(false);
```

**Step 3: Add menu-action listener**

In the `listen<string>('menu-action', ...)` callback (around line 89-91), add a new condition:

```typescript
      if (event.payload === 'View > Operations Log clicked') {
        setShowOperationsLog(true);
      }
```

**Step 4: Render the dialog**

After the `ScriptManagerDialog` render (after line 408), add:

```tsx
      <OperationsLogDialog
        isOpen={showOperationsLog}
        onClose={() => setShowOperationsLog(false)}
      />
```

**Step 5: Build and run the app to manually verify**

Run: `cargo build`
Expected: Compiles. Open the app, go to View > Operations Log, verify the dialog opens and shows operations.

**Step 6: Commit**

```bash
git add krillnotes-desktop/src/components/WorkspaceView.tsx
git commit -m "feat: wire up Operations Log dialog in WorkspaceView"
```
