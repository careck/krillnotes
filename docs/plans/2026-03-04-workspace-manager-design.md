# Workspace Manager — Design Doc

**Issue:** #65
**Date:** 2026-03-04

## Summary

Replace the rudimentary `OpenWorkspaceDialog` with a full `WorkspaceManagerDialog` that supports viewing workspace info, opening, deleting, and duplicating workspaces — all without requiring a password for read-only metadata.

## Goals

- Show all workspaces with name, last-modified date, and size on disk
- Sort by name (asc) or modified date (desc) — transient, resets each open
- Per-workspace actions: Open, Info, Duplicate, Delete
- Info shows: created date, last modified, size on disk, note count, attachment count
- Delete with a big red irreversible warning
- Duplicate: prompts for new name + new password

## Non-Goals

- Persistent sort preference
- Browse for workspaces outside the configured workspace directory
- Real-time info updates while a workspace is open

---

## Architecture

### 1. `info.json` Sidecar (new)

Each workspace folder gets an unencrypted `info.json` alongside `notes.db`:

```json
{
  "created_at": 1740000000,
  "note_count": 42,
  "attachment_count": 12
}
```

- **Written by:** `Workspace::open()` (after DB is ready) and on workspace window close (same code path that saves `selected_note_id`)
- **Read by:** `list_workspace_files` — no DB open, no password needed
- **Missing file:** treated gracefully — counts/dates show as `null`; happens for workspaces created before this feature or if the app crashed

### 2. Extended `WorkspaceEntry` (Rust)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceEntry {
    name: String,
    path: String,
    is_open: bool,
    last_modified: i64,        // Unix timestamp from folder mtime
    size_bytes: u64,            // notes.db + attachments/ total
    created_at: Option<i64>,    // from info.json, None if missing
    note_count: Option<usize>,  // from info.json, None if missing
    attachment_count: Option<usize>, // from info.json, None if missing
}
```

`list_workspace_files` computes `last_modified` and `size_bytes` from the filesystem and reads `info.json` if present.

### 3. New Tauri Commands

#### `delete_workspace(path: String) -> Result<(), String>`
- Rejects if `path` is currently open in AppState
- Calls `std::fs::remove_dir_all(path)`

#### `duplicate_workspace(source_path, source_password, new_name, new_password) -> Result<(), String>`
- Derives `dest_path = workspace_dir / new_name`
- Rejects if `dest_path` already exists
- Opens source workspace with `source_password`
- Exports to a `tempfile`
- Imports from tempfile into `dest_path` with `new_password`
- Writes `info.json` for the new workspace

### 4. `info.json` Write Helper (krillnotes-core)

New method on `Workspace`:

```rust
pub fn write_info_json(&self) -> Result<()>
```

Counts notes (excluding root) and attachments, then writes `info.json` to `workspace_root()`. Called from:
- `Workspace::open()` — after successful open
- `Workspace::create()` — after workspace is created
- Tauri window-close handler — same place `selected_note_id` is persisted

---

## Frontend

### `WorkspaceManagerDialog.tsx` (replaces `OpenWorkspaceDialog.tsx`)

**Layout:**
```
┌─────────────────────────────────────────┐
│         Workspace Manager               │
│  Sort: [Name ▾] [Modified ▾]           │
├─────────────────────────────────────────┤
│ ▶ My Notes           2026-03-04  4.2 MB │
│   Work Notes         2026-03-01  1.1 MB │
│   Archive     (open) 2025-12-10  0.8 MB │
├─────────────────────────────────────────┤
│  [Open]  [Info]  [Duplicate]  [Delete]  │
├─────────────────────────────────────────┤
│  [New]                         [Close]  │
└─────────────────────────────────────────┘
```

**Toolbar button states:**
- All 4 action buttons disabled when nothing selected
- **Open** — disabled if selected workspace `isOpen`; triggers `EnterPasswordDialog` (cached password tried first)
- **Info** — always enabled when something selected; shows inline info panel or sub-modal with all stats
- **Duplicate** — disabled if selected workspace `isOpen`; opens inline duplicate form (new name + new password)
- **Delete** — disabled if selected workspace `isOpen`; replaces toolbar with red confirmation banner

**Delete confirmation (inline, no separate modal):**
```
⚠ This will permanently delete "My Notes" and all its data.
  This cannot be undone.
  [Cancel]  [Delete forever]
```

**Duplicate form (inline):**
- New name (text input, pre-filled with `"Copy of <name>"`)
- New password (optional, password input + confirm)
- Source password: try session cache first; if not cached, show `EnterPasswordDialog` before proceeding
- [Cancel] [Duplicate]

**Info display:**
Shows a small panel below the list (or modal) with:
- Created, Last modified, Size on disk, Notes, Attachments
- Fields with `null` values show "—"

### App.tsx changes
- Replace `<OpenWorkspaceDialog>` with `<WorkspaceManagerDialog>`
- Update the menu handler that opens the workspace dialog

---

## Data Flow

```
Dialog opens
  → invoke("list_workspace_files")
  → reads filesystem mtime + size + info.json per workspace
  → renders list (no DB open, no password)

User selects workspace, clicks Info
  → data already in list entry
  → display inline panel

User clicks Open
  → try cached password → invoke("open_workspace")
  → or show EnterPasswordDialog → invoke("open_workspace")
  → on success: close dialog

User clicks Duplicate
  → show inline form
  → if source password not cached: show EnterPasswordDialog
  → invoke("duplicate_workspace", {...})
  → on success: refresh list

User clicks Delete
  → show red confirmation banner
  → invoke("delete_workspace", { path })
  → on success: refresh list, clear selection
```

---

## Error Handling

- All errors displayed inline below the list
- Delete of open workspace: "Close the workspace before deleting it."
- Duplicate with existing name: "A workspace named 'X' already exists."
- Wrong password during duplicate: propagated from export step
- All `remove_dir_all` / IO errors surfaced as strings

---

## Testing

- Unit tests in `krillnotes-core`: `write_info_json` counts, `info.json` round-trip, missing file graceful handling
- Integration: `duplicate_workspace` produces valid importable workspace
- Frontend: manual smoke test of all four actions + sort toggle
