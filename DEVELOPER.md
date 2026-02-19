# Krillnotes — Developer Guide

This document explains the codebase layout, core concepts, and the key decisions that shaped the architecture. Read this before making significant changes.

---

## Repository Layout

```
Krillnotes/
├── Cargo.toml                     # Workspace manifest (two Rust crates)
├── krillnotes-core/               # Pure Rust library — no UI, no Tauri
│   └── src/
│       ├── lib.rs                 # Crate root, public re-exports
│       └── core/
│           ├── workspace.rs       # Primary API surface (Workspace struct)
│           ├── note.rs            # Note + FieldValue types
│           ├── operation.rs       # Operation enum (CRDT mutations)
│           ├── operation_log.rs   # Append-only log + purge strategies
│           ├── scripting.rs       # Rhai schema registry
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
        ├── App.tsx
        ├── types.ts               # Shared TypeScript interfaces
        └── components/            # UI components
```

`krillnotes-core` has no dependency on Tauri or any UI framework. It can be used as a standalone library, embedded in a CLI, or tested independently.

---

## Core Concepts

### 1. Local-First Design

All data lives in a single file on the user's disk — a SQLite database with the `.krillnotes` extension. There is no server, no account, and no network requirement.

The file format is intentionally simple: inspect or back it up with any SQLite tool. The schema is in [krillnotes-core/src/core/schema.sql](krillnotes-core/src/core/schema.sql).

Local-first does not mean sync-never. The architecture is designed so that a future sync layer can be added without changing the core API (see Operation Log below).

### 2. Operation Log

Every document mutation — creating a note, updating a field, moving a node, deleting a note — is recorded as an `Operation` before being applied to the `notes` table.

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
    CreateNote { operation_id, timestamp, device_id, note_id, parent_id,
                 position, node_type, title, fields, created_by },
    UpdateField { operation_id, timestamp, device_id, note_id, field, value, modified_by },
    DeleteNote  { operation_id, timestamp, device_id, note_id },
    MoveNote    { operation_id, timestamp, device_id, note_id, new_parent_id, new_position },
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

### 3. Schema Registry & Rhai Scripting

Note types are not hard-coded. Each type is a *schema* defined by a [Rhai](https://rhai.rs/) script. Rhai is a lightweight, embeddable scripting language with Rust-native types.

**How it works:**

1. `SchemaRegistry::new()` creates a `rhai::Engine` and registers a `schema()` host function.
2. The built-in `text_note.rhai` script is evaluated, which calls `schema("TextNote", ...)` to register the `TextNote` type.
3. Any additional `.rhai` files (user-defined types) can be loaded via `SchemaRegistry::load_script()`.

**Defining a schema:**

```rhai
// my_task.rhai
schema("Task", #{
    fields: [
        #{ name: "body",     type: "text",    required: false },
        #{ name: "done",     type: "boolean", required: false },
        #{ name: "priority", type: "number",  required: false },
    ]
});
```

Field types: `"text"`, `"number"`, `"boolean"`.

**Why keep the Engine alive?**

The `rhai::Engine` is a long-lived field on `SchemaRegistry`, not reconstructed per request. This is intentional: future phases will use the same engine to evaluate scripted *views* (computed fields, filtered subtrees), *commands* (bulk operations invoked by the user), and *action hooks* (pre/post-save triggers). Keeping the engine alive avoids the overhead of re-parsing scripts on every invocation.

The `rhai/sync` Cargo feature is enabled, which replaces `Rc`/`RefCell` internals with `Arc`/`Mutex`, making `Engine: Send + Sync` without any `unsafe` code.

### 4. Multi-Window Architecture

Each open workspace gets its own Tauri window. The frontend is a single React application that loads in every window, but each instance fetches data from its own workspace via the window label.

**AppState** ([lib.rs](krillnotes-desktop/src-tauri/src/lib.rs)):

```rust
pub struct AppState {
    pub workspaces:      Arc<Mutex<HashMap<String, Workspace>>>,  // label → Workspace
    pub workspace_paths: Arc<Mutex<HashMap<String, PathBuf>>>,    // label → path on disk
}
```

Window labels are derived from the workspace filename (e.g., `notes` for `notes.krillnotes`), with a numeric suffix appended on collision (`notes-2`, `notes-3`, ...).

Every Tauri command receives a `window: tauri::Window` parameter and uses `window.label()` to look up the correct `Workspace`. This means all commands are automatically scoped to the calling window with no extra routing logic.

When a window is destroyed, its entry is removed from `AppState` to free the database connection.

### 5. Per-Device UI State vs. Document State

Not all state belongs in the operation log. The distinction matters for sync:

| State | Storage | Logged? | Synced? |
|-------|---------|---------|---------|
| Note title, fields | `notes` table | Yes | Yes (future) |
| Create / delete / move | `notes` table | Yes | Yes (future) |
| Tree expansion (`is_expanded`) | `notes.is_expanded` | **No** | **No** |
| Selected note | `workspace_meta.selected_note_id` | **No** | **No** |

Expansion and selection are *view state* — local to the device and not meaningful to other devices. They are stored persistently (so they survive app restarts) but are never written to the operation log and will not participate in sync.

### 6. Tree Hierarchy

Notes form an ordered tree via two columns: `parent_id` (nullable foreign key to `notes.id`) and `position` (zero-based integer sort order among siblings).

The database enforces referential integrity: deleting a note cascades to all its descendants.

When a note is inserted as a sibling, all following siblings have their `position` incremented in the same transaction before the new row is inserted. This keeps positions gapless and consistent.

---

## Adding a New Note Type

1. Create a Rhai script that calls `schema("MyType", #{ fields: [...] })`.
2. Load it via `SchemaRegistry::load_script(code)` in `Workspace::open` / `Workspace::create`.
3. The new type will appear in `list_node_types()` and can be selected in the UI.

No Rust changes are needed for a purely additive new type.

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
| Core library | Rust, rusqlite (bundled SQLite), rhai, serde |
| Desktop backend | Tauri v2, mimalloc (global allocator) |
| Frontend | React 19, TypeScript 5, Vite, Tailwind CSS v4 |
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
| 3 — Tree view | Done | Hierarchical display, selection, expansion |
| 4 — Detail view & editing | In progress | Field editing, title updates, detail panel |
| 5 — Undo / redo | Planned | Replay / invert operation log |
| 6 — Sync infrastructure | Planned | CRDT merge, conflict resolution, `synced` flag |
| 7 — Custom schema UI | Planned | In-app schema editor (generate `.rhai` scripts) |
| 8 — Scripted views & hooks | Planned | Use long-lived Rhai engine for views, commands, triggers |

---

## Key Files at a Glance

| File | Role |
|------|------|
| [krillnotes-core/src/core/workspace.rs](krillnotes-core/src/core/workspace.rs) | Primary API — all document mutations go here |
| [krillnotes-core/src/core/operation.rs](krillnotes-core/src/core/operation.rs) | Operation enum definition |
| [krillnotes-core/src/core/operation_log.rs](krillnotes-core/src/core/operation_log.rs) | Log append + purge strategies |
| [krillnotes-core/src/core/scripting.rs](krillnotes-core/src/core/scripting.rs) | Rhai engine + schema registration |
| [krillnotes-core/src/core/storage.rs](krillnotes-core/src/core/storage.rs) | SQLite connection management + migrations |
| [krillnotes-core/src/core/schema.sql](krillnotes-core/src/core/schema.sql) | Database DDL |
| [krillnotes-desktop/src-tauri/src/lib.rs](krillnotes-desktop/src-tauri/src/lib.rs) | Tauri commands + AppState |
| [krillnotes-desktop/src-tauri/src/menu.rs](krillnotes-desktop/src-tauri/src/menu.rs) | Native menu |
| [krillnotes-desktop/src/App.tsx](krillnotes-desktop/src/App.tsx) | React root, menu event wiring |
| [krillnotes-desktop/src/types.ts](krillnotes-desktop/src/types.ts) | Shared TypeScript interfaces |
