# Krillnotes — Developer Guide

This document explains the codebase layout, core concepts, and the key decisions that shaped the architecture. Read this before making significant changes.

---

## Repository Layout

```
Krillnotes/
├── Cargo.toml                     # Workspace manifest (two Rust crates)
├── user_scripts/                  # Example Rhai scripts (Task, Book, Contact, etc.)
├── templates/                     # Template gallery — copy into Script Manager to activate
│   ├── book_collection.rhai       # Library organiser with on_view table and sort actions
│   └── zettelkasten.rhai          # Zettelkasten atomic-note system with related-note discovery
├── krillnotes-core/               # Pure Rust library — no UI, no Tauri
│   └── src/
│       ├── lib.rs                 # Crate root, public re-exports
│       └── core/
│           ├── workspace.rs       # Primary API surface (Workspace struct)
│           ├── note.rs            # Note + FieldValue types
│           ├── operation.rs       # Operation enum (CRDT mutations)
│           ├── operation_log.rs   # Append-only log + purge strategies
│           ├── delete.rs          # Delete strategies (DeleteAll, PromoteChildren)
│           ├── user_script.rs     # UserScript type + CRUD operations
│           ├── export.rs          # Workspace export/import (zip archives)
│           ├── scripting/
│           │   ├── mod.rs         # Scripting module root — engine setup, hook dispatch, query fns
│           │   ├── schema.rs      # Schema registry (field types, flags)
│           │   ├── hooks.rs       # Hook registry (placeholder for future global/lifecycle hooks)
│           │   └── display_helpers.rs # HTML helpers (text, table, render_tags, link_to, …)
│           ├── storage.rs         # SQLite connection + migrations
│           ├── device.rs          # Stable hardware device ID
│           ├── error.rs           # KrillnotesError enum
│           ├── schema.sql         # Database DDL
│           └── system_scripts/
│               └── text_note.rhai # Built-in TextNote schema
└── krillnotes-desktop/
    ├── src-tauri/                 # Tauri v2 Rust backend
    │   └── src/
    │       ├── lib.rs             # Tauri commands + AppState
    │       └── menu.rs            # Native menu builder
    └── src/                       # React 19 / TypeScript frontend
        ├── App.tsx                # React root, menu event wiring, export/import
        ├── types.ts               # Shared TypeScript interfaces
        └── components/
            ├── WorkspaceView.tsx          # Main layout, resizable panels, keyboard nav
            ├── TreeView.tsx               # Tree rendering
            ├── TreeNode.tsx               # Individual tree node
            ├── InfoPanel.tsx              # Detail/edit panel with schema-aware fields
            ├── FieldEditor.tsx            # Field edit widgets (all field types)
            ├── FieldDisplay.tsx           # Read-only field display
            ├── SearchBar.tsx              # Live search with dropdown results
            ├── ScriptManagerDialog.tsx     # User script list + CRUD
            ├── ScriptEditor.tsx           # Rhai code editor
            ├── OperationsLogDialog.tsx     # Operations history viewer + purge
            ├── AddNoteDialog.tsx           # New note dialog (type + position)
            ├── DeleteConfirmDialog.tsx     # Delete confirmation with strategy choice
            ├── ContextMenu.tsx            # Right-click tree menu
            ├── SetPasswordDialog.tsx      # Set (+ confirm) workspace password
            ├── EnterPasswordDialog.tsx    # Enter password to open a workspace
            ├── WelcomeDialog.tsx          # First-launch welcome
            ├── EmptyState.tsx             # No-workspace placeholder
            └── StatusMessage.tsx          # Transient success/error toast
```

`krillnotes-core` has no dependency on Tauri or any UI framework. It can be used as a standalone library, embedded in a CLI, or tested independently.

---

## Core Concepts

### 1. Local-First Design

All data lives in a single file on the user's disk — a SQLite database with the `.krillnotes` extension. There is no server, no account, and no network requirement.

The file format is intentionally simple: inspect or back it up with any SQLite tool. The schema is in [krillnotes-core/src/core/schema.sql](krillnotes-core/src/core/schema.sql).

Local-first does not mean sync-never. The architecture is designed so that a future sync layer can be added without changing the core API (see Operation Log below).

### 2. Operation Log

Every document mutation — creating a note, updating a field, moving a node, deleting a note, or managing user scripts — is recorded as an `Operation` before being applied.

```
User action
    │
    ▼
Workspace method (e.g. create_note)
    │
    ├── BEGIN TRANSACTION
    ├── Apply mutation to `notes` table
    ├── Append Operation to `operations` table   ← immutable record
    ├── Purge old operations if over limit
    └── COMMIT
```

Operations are defined in [krillnotes-core/src/core/operation.rs](krillnotes-core/src/core/operation.rs):

```rust
pub enum Operation {
    // Note operations
    CreateNote { operation_id, timestamp, device_id, note_id, parent_id,
                 position, node_type, title, fields, created_by },
    UpdateField { operation_id, timestamp, device_id, note_id, field, value, modified_by },
    DeleteNote  { operation_id, timestamp, device_id, note_id },
    MoveNote    { operation_id, timestamp, device_id, note_id, new_parent_id, new_position },

    // User script operations
    CreateUserScript { operation_id, timestamp, device_id, script_id, name, description },
    UpdateUserScript { operation_id, timestamp, device_id, script_id, name, description },
    DeleteUserScript { operation_id, timestamp, device_id, script_id },
}
```

Each operation carries:
- A stable UUID (`operation_id`)
- A wall-clock timestamp
- The `device_id` of the originating machine

This makes the log replayable and mergeable. The `synced` flag on each row (0 = local, 1 = acknowledged by a remote) is reserved for the future sync phase.

**Purge strategies** ([operation_log.rs](krillnotes-core/src/core/operation_log.rs)):

| Strategy | Behaviour |
|----------|-----------|
| `LocalOnly { keep_last: N }` | Keep the N most recent operations; delete the rest. Default: 1000. |
| `WithSync { retention_days: D }` | Keep unsynced operations indefinitely; delete synced ones older than D days. For future use. |

### 3. Scripting Architecture (Schema Registry, Hook Registry, User Scripts)

Note types are not hard-coded. Each type is a *schema* defined by a [Rhai](https://rhai.rs/) script. Rhai is a lightweight, embeddable scripting language with Rust-native types.

The scripting system is split into three registries:

- **Schema Registry** ([scripting/schema.rs](krillnotes-core/src/core/scripting/schema.rs)) — Holds field definitions, per-schema flags, and schema-bound hooks (`on_save`, `on_view`). Scripts call `schema("TypeName", #{ ... })` to register types and their hooks together.
- **Hook Registry** ([scripting/hooks.rs](krillnotes-core/src/core/scripting/hooks.rs)) — Placeholder for future global/lifecycle hooks (`on_load`, `on_export`, menu hooks). Currently empty.
- **User Script storage** ([user_script.rs](krillnotes-core/src/core/user_script.rs)) — CRUD for per-workspace scripts stored in the `user_scripts` database table.

**How it works:**

1. On workspace open, the `rhai::Engine` is created and the built-in `text_note.rhai` system script is evaluated.
2. All enabled user scripts (from the `user_scripts` table, ordered by `load_order`) are evaluated next.
3. Each script calls `schema()` to register a type. Hooks (`on_save`, `on_view`) are defined as keys inside the `schema()` map — there are no standalone hook functions.

**Defining a schema with hooks:**

```rhai
schema("Task", #{
    fields: [
        #{ name: "name",     type: "text",    required: true },
        #{ name: "status",   type: "select",  required: false,
           options: ["TODO", "WIP", "DONE"] },
        #{ name: "priority", type: "number",  required: false },
        #{ name: "due_date", type: "date",    required: false },
        #{ name: "notes",    type: "textarea", required: false },
    ],
    title_can_edit: false,   // title is computed by the on_save hook
    children_sort: "asc",    // sort child notes alphabetically
    on_save: |note| {
        let name   = note.fields["name"];
        let status = note.fields["status"];
        note.title = "[" + status + "] " + name;
        note
    }
});
```

**Field types:** `"text"`, `"textarea"`, `"number"`, `"boolean"`, `"date"`, `"email"`, `"select"`, `"rating"`.

**Schema flags:**

| Flag | Type | Default | Purpose |
|------|------|---------|---------|
| `title_can_edit` | bool | `true` | Whether the title field is shown in edit mode |
| `title_can_view` | bool | `true` | Whether the title field is shown in view mode |
| `children_sort` | string | `"none"` | Sort children: `"asc"`, `"desc"`, or `"none"` (use position) |

**Per-field flags:**

| Flag | Type | Default | Purpose |
|------|------|---------|---------|
| `can_view` | bool | `true` | Show this field in view mode |
| `can_edit` | bool | `true` | Show this field in edit mode |
| `options` | array | — | Choice list for `select` fields |

**Why keep the Engine alive?**

The `rhai::Engine` is a long-lived field, not reconstructed per request. This avoids the overhead of re-parsing scripts on every invocation and allows hooks to be called efficiently on each save.

The `rhai/sync` Cargo feature is enabled, which replaces `Rc`/`RefCell` internals with `Arc`/`Mutex`, making `Engine: Send + Sync` without any `unsafe` code.

### 4. Tags

Tags are free-form strings attached to notes via a `note_tags` junction table (`note_id`, `tag`). There is no tags master table — tags are implicit (they exist as long as at least one row references them).

**Database:** `update_note_tags(note_id, tags: Vec<String>)` in `workspace.rs` replaces all tag rows for a note in a single transaction (DELETE + INSERT). All note-fetching queries use a `LEFT JOIN note_tags + GROUP_CONCAT` to populate `Note.tags: Vec<String>`.

**Tauri commands:** `update_note_tags`, `get_all_tags`, `get_notes_for_tag` are exposed as Tauri commands and called from the frontend tag editor and tag cloud panel.

**Frontend:** `TagPill.tsx` renders individual tags. `tagColor.ts` produces a deterministic colour per tag string (hash-based, stable across runs). The tag cloud panel in `WorkspaceView.tsx` is resizable and lists every tag in the workspace.

**Scripting:** The note map passed to `on_view` hooks includes a `tags: Array` key. `render_tags(note.tags)` renders tags as coloured pills. `get_notes_for_tag(tags)` returns all notes carrying any of the listed tags (OR semantics), deduplicated. The `QueryContext` pre-builds a `notes_by_tag` index so these lookups are O(1) per hook call.

**Export / Import:** `workspace.json` carries a top-level `tags: [string]` list and each note's `tags` array. On import, `note_tags` rows are reconstructed from the note objects.

### 5. Encryption

Every workspace database is encrypted with **SQLCipher** using the `bundled-sqlcipher-vendored-openssl` rusqlite feature. OpenSSL is compiled from source and statically linked — there are no system-level OpenSSL dependencies on any platform.

**How it works:**

- `PRAGMA key = '<password>'` is the very first SQL operation after opening a connection, before any schema access.
- SQLCipher uses **AES-256-CBC** with **PBKDF2-HMAC-SHA512** (256,000 iterations) by default.
- An empty password string is treated as "no encryption" (only used in tests; not reachable from the UI).

**Opening an old unencrypted workspace:**

1. Open connection, issue `PRAGMA key` with the provided password.
2. Query `sqlite_master` for the three expected tables.
3. If they are not found, open a second connection **without** a key.
4. If the plain connection sees the tables → return `KrillnotesError::UnencryptedWorkspace`.
5. Otherwise → return `KrillnotesError::WrongPassword`.

These two error variants are mapped to sentinel strings (`"UNENCRYPTED_WORKSPACE"`, `"WRONG_PASSWORD"`) for the frontend to handle with appropriate UI.

**Session password caching:**

When the `cache_workspace_passwords` setting is enabled, the password is stored in `AppState.workspace_passwords` (keyed by file path) after a successful open. On subsequent opens of the same workspace within the same session, the cached password is used without prompting. The cache is never written to disk and is cleared when the app exits.

### 6. User Script Management

User scripts are stored in the `user_scripts` table inside each workspace database. This means each workspace has its own set of custom note types and hooks.

Scripts are managed via Tauri commands (`list_user_scripts`, `create_user_script`, `update_user_script`, `delete_user_script`, `toggle_user_script`, `reorder_user_script`) and the Script Manager dialog in the frontend.

When a script is created or updated, all registries are reloaded: the schema and hook registries are cleared, system scripts are re-evaluated, then all enabled user scripts are re-evaluated in load order.

### 7. Export / Import

Workspaces can be exported as `.zip` archives ([export.rs](krillnotes-core/src/core/export.rs)). The archive contains:

- `workspace.json` — All notes with their fields and tags, a global tag list, plus metadata (app version, export timestamp, note count).
- `scripts/*.rhai` — Each user script as a separate file.

Operations are excluded from exports as they are device-specific.

The zip can optionally be encrypted with AES-256 using the `zip` crate's built-in AES support. This is a separate layer from the workspace database encryption — the zip password protects the archive in transit, and the workspace password protects the database at rest.

Importing reads a zip (prompting for the zip password if encrypted), creates a fresh **SQLCipher-encrypted** workspace database (prompting for a new workspace password), inserts all notes and scripts, and opens the new workspace. A `peek_import` command allows inspecting the archive metadata (version, note count, script count) before committing to the import.

### 8. Multi-Window Architecture

Each open workspace gets its own Tauri window. The frontend is a single React application that loads in every window, but each instance fetches data from its own workspace via the window label.

**AppState** ([lib.rs](krillnotes-desktop/src-tauri/src/lib.rs)):

```rust
pub struct AppState {
    pub workspaces:          Arc<Mutex<HashMap<String, Workspace>>>,  // label → Workspace
    pub workspace_paths:     Arc<Mutex<HashMap<String, PathBuf>>>,    // label → path on disk
    pub focused_window:      Arc<Mutex<Option<String>>>,              // for menu routing
    pub workspace_passwords: Arc<Mutex<HashMap<PathBuf, String>>>,    // path → password (session cache)
}
```

`workspace_passwords` is populated only when the `cache_workspace_passwords` setting is enabled. It is never persisted to disk — passwords are cleared when the app quits.

Window labels are derived from the workspace filename (e.g., `notes` for `notes.krillnotes`), with a numeric suffix appended on collision (`notes-2`, `notes-3`, ...).

Every Tauri command receives a `window: tauri::Window` parameter and uses `window.label()` to look up the correct `Workspace`. This means all commands are automatically scoped to the calling window with no extra routing logic.

When a window is destroyed, its entry is removed from `AppState` to free the database connection.

### 9. Per-Device UI State vs. Document State

Not all state belongs in the operation log. The distinction matters for sync:

| State | Storage | Logged? | Synced? |
|-------|---------|---------|---------|
| Note title, fields | `notes` table | Yes | Yes (future) |
| Create / delete / move | `notes` table | Yes | Yes (future) |
| Tree expansion (`is_expanded`) | `notes.is_expanded` | **No** | **No** |
| Selected note | `workspace_meta.selected_note_id` | **No** | **No** |

Expansion and selection are *view state* — local to the device and not meaningful to other devices. They are stored persistently (so they survive app restarts) but are never written to the operation log and will not participate in sync.

### 10. Tree Hierarchy

Notes form an ordered tree via two columns: `parent_id` (nullable foreign key to `notes.id`) and `position` (zero-based integer sort order among siblings).

The database enforces referential integrity: deleting a note cascades to all its descendants.

When a note is inserted as a sibling, all following siblings have their `position` incremented in the same transaction before the new row is inserted. This keeps positions gapless and consistent.

---

## Adding a New Note Type

**From the UI (recommended):**

1. Open the Script Manager (View menu).
2. Click "New Script" and write a Rhai script that calls `schema("MyType", #{ fields: [...] })`.
3. Optionally add an `on_save: |note| { ... }` hook key inside the `schema()` map.
4. Save — the registries reload automatically and the new type appears in the Add Note dialog.

**From code (system scripts):**

1. Add a `.rhai` file to [krillnotes-core/src/core/system_scripts/](krillnotes-core/src/core/system_scripts/).
2. It will be included via `include_dir!` and evaluated on every workspace open.

No Rust changes are needed for a purely additive new type. Six example scripts are provided in the [user_scripts/](user_scripts/) folder for reference.

---

## Adding a New Tauri Command

1. Write a `pub async fn my_command(window: tauri::Window, state: tauri::State<AppState>, ...) -> Result<T, String>` function in [lib.rs](krillnotes-desktop/src-tauri/src/lib.rs).
2. Add it to `tauri::generate_handler![..., my_command]`.
3. Call it from the frontend via `invoke("my_command", { ... })`.

Access the calling window's workspace with:

```rust
let workspaces = state.workspaces.lock().unwrap();
let workspace = workspaces.get(window.label()).ok_or("workspace not found")?;
```

---

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Core library | Rust, rusqlite (bundled SQLCipher + vendored OpenSSL), rhai, serde, zip |
| Desktop backend | Tauri v2, mimalloc (global allocator) |
| Frontend | React 19, TypeScript 5, Vite, Tailwind CSS v4 |
| Encryption | SQLCipher (AES-256-CBC, PBKDF2-HMAC-SHA512) for workspace DBs; AES-256 zip for exports |
| Error handling | thiserror |
| IDs | uuid v4 |
| Timestamps | chrono |
| Device fingerprint | mac_address (hashed) |

---

## Testing

```bash
# Run all core library tests
cargo test -p krillnotes-core

# Run with output
cargo test -p krillnotes-core -- --nocapture

# Check documentation builds cleanly
cargo doc --no-deps -p krillnotes-core
cargo doc --no-deps -p krillnotes-desktop
```

Tests live alongside the code they test in `#[cfg(test)]` modules at the bottom of each source file. Each test creates a temporary in-memory or temp-file workspace so there are no shared fixtures.

---

## Roadmap

| Phase | Status | Description |
|-------|--------|-------------|
| 1 — Core library | Done | Workspace, Note, Operation, Schema, Storage |
| 2 — Workspace integration | Done | Multi-window, file picker, AppState |
| 3 — Tree view | Done | Hierarchical display, selection, expansion, keyboard nav |
| 4 — Detail view & editing | Done | Field editing, title updates, detail panel, context menu |
| 5 — User scripts & hooks | Done | In-app script manager, on_save hooks, per-workspace storage |
| 6 — Search | Done | Live search across titles and fields with ancestor expansion |
| 7 — Operations log viewer | Done | Filterable history, date/type filters, purge |
| 8 — Export / Import | Done | Zip-based workspace export and import with version checks |
| 9 — Encryption | Done | SQLCipher AES-256 for workspace DBs; optional AES-256 zip for exports |
| 10 — Undo / redo | Planned | Replay / invert operation log |
| 11 — Sync infrastructure | Planned | CRDT merge, conflict resolution, `synced` flag |

---

## Key Files at a Glance

| File | Role |
|------|------|
| [krillnotes-core/src/core/workspace.rs](krillnotes-core/src/core/workspace.rs) | Primary API — all document mutations go here |
| [krillnotes-core/src/core/operation.rs](krillnotes-core/src/core/operation.rs) | Operation enum definition (notes + user scripts) |
| [krillnotes-core/src/core/operation_log.rs](krillnotes-core/src/core/operation_log.rs) | Log append + purge strategies |
| [krillnotes-core/src/core/scripting/schema.rs](krillnotes-core/src/core/scripting/schema.rs) | Schema registry (field types, flags) |
| [krillnotes-core/src/core/scripting/hooks.rs](krillnotes-core/src/core/scripting/hooks.rs) | Hook registry (placeholder for future global/lifecycle hooks) |
| [krillnotes-core/src/core/user_script.rs](krillnotes-core/src/core/user_script.rs) | UserScript type + CRUD |
| [krillnotes-core/src/core/export.rs](krillnotes-core/src/core/export.rs) | Workspace export/import (zip) |
| [krillnotes-core/src/core/delete.rs](krillnotes-core/src/core/delete.rs) | Delete strategies (DeleteAll, PromoteChildren) |
| [krillnotes-core/src/core/storage.rs](krillnotes-core/src/core/storage.rs) | SQLCipher connection management, PRAGMA key, migrations, unencrypted-workspace detection |
| [krillnotes-core/src/core/schema.sql](krillnotes-core/src/core/schema.sql) | Database DDL (4 tables) |
| [krillnotes-desktop/src-tauri/src/lib.rs](krillnotes-desktop/src-tauri/src/lib.rs) | Tauri commands (25 commands) + AppState (incl. password cache) |
| [krillnotes-desktop/src-tauri/src/menu.rs](krillnotes-desktop/src-tauri/src/menu.rs) | Native menu (File, Edit, View, Window, Help) |
| [krillnotes-desktop/src/App.tsx](krillnotes-desktop/src/App.tsx) | React root, menu event wiring, export/import |
| [krillnotes-desktop/src/types.ts](krillnotes-desktop/src/types.ts) | Shared TypeScript interfaces |
