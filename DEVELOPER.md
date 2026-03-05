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
│           ├── hlc.rs             # HlcTimestamp, HlcClock — Hybrid Logical Clock
│           ├── identity.rs        # Identity model (Ed25519 + Argon2id + AES-GCM + .swarmid)
│           ├── undo.rs            # RetractInverse enum — undo/redo inverse operations
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
    │   ├── build.rs               # Compile-time locale embedding (generates locales_generated.rs)
    │   └── src/
    │       ├── lib.rs             # Tauri commands + AppState
    │       ├── locales.rs         # menu_strings(lang) — locale lookup with English fallback
    │       └── menu.rs            # Native menu builder (accepts &serde_json::Value strings)
    └── src/                       # React 19 / TypeScript frontend
        ├── App.tsx                # React root, menu event wiring, export/import
        ├── types.ts               # Shared TypeScript interfaces
        ├── i18n/
        │   ├── index.ts           # i18next initialisation + language apply-on-startup
        │   └── locales/           # One JSON file per language (en, de, fr, es, ja, ko, zh)
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
            ├── AttachmentsSection.tsx     # File attachment panel (drag-drop, thumbnails)
            ├── WorkspaceManagerDialog.tsx # Full workspace manager (list, open, duplicate, delete)
            ├── IdentityManagerDialog.tsx  # Identity CRUD + .swarmid export/import
            ├── CreateIdentityDialog.tsx   # New identity form (name + passphrase)
            ├── UnlockIdentityDialog.tsx   # Passphrase prompt to unlock an identity
            ├── WelcomeDialog.tsx          # First-launch welcome
            ├── EmptyState.tsx             # No-workspace placeholder
            └── StatusMessage.tsx          # Transient success/error toast
```

`krillnotes-core` has no dependency on Tauri or any UI framework. It can be used as a standalone library, embedded in a CLI, or tested independently.

---

## Core Concepts

### 1. Local-First Design

All data lives in a **workspace folder** on the user's disk. The folder contains:

- `notes.db` — the SQLCipher-encrypted SQLite database (schema in [schema.sql](krillnotes-core/src/core/schema.sql)); contains 7 tables: `notes`, `note_tags`, `operations`, `workspace_meta`, `user_scripts`, `attachments`, `hlc_state`
- `attachments/<uuid>` — per-file ChaCha20-Poly1305 encrypted blobs
- `info.json` — unencrypted metadata sidecar (name, counts, workspace UUID) for the Workspace Manager

There is no server, no account, and no network requirement.

Local-first does not mean sync-never. The architecture is designed so that a future sync layer can be added without changing the core API (see Operation Log below).

### 2. Operation Log

Every document mutation — creating a note, updating a field, moving a node, deleting a note, or managing user scripts — is recorded as an `Operation` before being applied. The log is **always active**: it is required for undo/redo (`RetractOperation` entries) and will also drive CRDT sync when that ships.

```
User action
    │
    ▼
Workspace method (e.g. create_note)
    │
    ├── BEGIN TRANSACTION
    ├── Apply mutation to `notes` table
    ├── log_op(Operation { ... })   ← always appended
    ├── Purge old operations if over limit (configurable via undo_limit in workspace_meta)
    └── COMMIT
```

Operations are defined in [krillnotes-core/src/core/operation.rs](krillnotes-core/src/core/operation.rs):

```rust
pub enum Operation {
    // Note operations
    CreateNote  { operation_id, timestamp: HlcTimestamp, device_id, note_id, parent_id,
                  position: f64, node_type, title, fields, created_by, signature },
    UpdateNote  { operation_id, timestamp: HlcTimestamp, device_id, note_id,
                  title, modified_by, signature },
    UpdateField { operation_id, timestamp: HlcTimestamp, device_id, note_id,
                  field, value, modified_by, signature },
    DeleteNote  { operation_id, timestamp: HlcTimestamp, device_id, note_id, signature },
    MoveNote    { operation_id, timestamp: HlcTimestamp, device_id, note_id,
                  new_parent_id, new_position: f64, signature },
    SetTags     { operation_id, timestamp: HlcTimestamp, device_id, note_id,
                  tags: Vec<String>, modified_by, signature },

    // User script operations
    CreateUserScript { operation_id, timestamp: HlcTimestamp, device_id, script_id,
                       name, description, signature },
    UpdateUserScript { operation_id, timestamp: HlcTimestamp, device_id, script_id,
                       name, description, signature },
    DeleteUserScript { operation_id, timestamp: HlcTimestamp, device_id,
                       script_id, signature },

    // Undo/redo
    RetractOperation { operation_id, timestamp: HlcTimestamp, device_id,
                       retracted_ids: Vec<String>, inverse: RetractInverse,
                       propagate: bool },
}
```

**Undo/redo** is implemented by appending a `RetractOperation` that references the operation being undone. The `undo.rs` module computes a `RetractInverse` (the compensating action) from the original operation's stored data. Groups of operations triggered by a single user action are bracketed with `begin_undo_group()` / `end_undo_group()` so they collapse into one Cmd+Z step.

Each operation carries:
- A stable UUID (`operation_id`)
- An `HlcTimestamp` (Hybrid Logical Clock — `wall_ms`, `counter`, `node_id`) that is monotonic across devices and provides a total ordering for CRDT merge
- The `device_id` of the originating machine
- An Ed25519 `signature` over the canonical JSON payload (base64)

This makes the log replayable, mergeable, and cryptographically attributable. The `synced` flag on each row (0 = local, 1 = acknowledged by a remote) is reserved for the future sync phase.

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

**Field types:** `"text"`, `"textarea"`, `"number"`, `"boolean"`, `"date"`, `"email"`, `"select"`, `"rating"`, `"file"` (encrypted attachment reference), `"note_link"` (reference to another note by UUID).

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

Workspaces can be exported as `.krillnotes` archives ([export.rs](krillnotes-core/src/core/export.rs)). The format is a standard zip containing:

- `workspace.json` — All notes with their fields and tags, a global tag list, plus metadata (app version, export timestamp, note count).
- `scripts/*.rhai` — Each user script as a separate file.
- `attachments.json` — Attachment metadata manifest.
- `attachments/<uuid>` — Raw (plaintext) attachment bytes (re-encrypted on import).

Operations are excluded from exports as they are device-specific.

The zip can optionally be encrypted with AES-256 using the `zip` crate's built-in AES support. This is a separate layer from the workspace database encryption — the zip password protects the archive in transit.

Importing reads a zip (prompting for the zip password if encrypted), creates a fresh **SQLCipher-encrypted** workspace database bound to the currently unlocked identity, inserts all notes, scripts, and attachments (re-encrypting each attachment under the new workspace key), and opens the new workspace. A `peek_import` command allows inspecting the archive metadata (version, note count, script count) before committing to the import.

### 8. Identity System

The identity system ([identity.rs](krillnotes-core/src/core/identity.rs)) provides cryptographic identity management, replacing per-workspace user-visible passwords.

**Key concepts:**

- An **identity** is an Ed25519 signing keypair. The private key is encrypted at rest with AES-256-GCM using a key derived from the user's passphrase via Argon2id (64 MiB, 3 iterations in production).
- A **workspace binding** stores the workspace's randomly-generated SQLCipher password encrypted under the identity's public key.
- An **unlocked identity** (`UnlockedIdentity`) holds the decrypted signing key in memory for the duration of the session.

**On-disk layout** (in `~/.config/krillnotes/`):

```
identities/<uuid>.json       ← one per identity (IdentityFile)
identity_settings.json       ← registry of identity refs + workspace bindings
```

**Crypto chain for workspace access:**

```
passphrase → Argon2id → AES-256-GCM key → decrypt Ed25519 seed
                                                   │
                                    HKDF-SHA256 → per-workspace DB password key
                                                   │
                                    Decrypt stored DB password → open SQLCipher DB
```

**AppState additions** ([lib.rs](krillnotes-desktop/src-tauri/src/lib.rs)):

```rust
pub struct AppState {
    pub workspaces:            Arc<Mutex<HashMap<String, Workspace>>>,
    pub workspace_paths:       Arc<Mutex<HashMap<String, PathBuf>>>,
    pub identity_manager:      Arc<Mutex<IdentityManager>>,
    pub unlocked_identities:   Arc<Mutex<HashMap<Uuid, UnlockedIdentity>>>,
    pub paste_menu_items:      Arc<Mutex<HashMap<String, (MenuItem, MenuItem)>>>,
    pub workspace_menu_items:  Arc<Mutex<HashMap<String, Vec<MenuItem>>>>,
}
```

**`.swarmid` format** — a portable identity export: the `IdentityFile` JSON wrapped in a `SwarmIdFile` envelope (format version, export timestamp). Import preserves workspace bindings if the same UUID already exists (`import_swarmid_overwrite`).

**Tauri commands:** `list_identities`, `create_identity`, `unlock_identity`, `lock_identity`, `delete_identity`, `rename_identity`, `change_identity_passphrase`, `get_unlocked_identities`, `is_identity_unlocked`, `get_workspaces_for_identity`, `export_swarmid`, `import_swarmid`, `import_swarmid_overwrite`.

### 9. Multi-Window Architecture

Each open workspace gets its own Tauri window. The frontend is a single React application that loads in every window, but each instance fetches data from its own workspace via the window label.

**AppState** is documented in section 8 (Identity System) above. See [lib.rs](krillnotes-desktop/src-tauri/src/lib.rs) for the full definition.

Window labels are derived from the workspace folder name (e.g., `notes` for a folder named `notes`), with a numeric suffix appended on collision (`notes-2`, `notes-3`, ...).

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

Notes form an ordered tree via two columns: `parent_id` (nullable foreign key to `notes.id`) and `position` (a `REAL` / `f64` sort key among siblings).

The database enforces referential integrity: deleting a note cascades to all its descendants.

`position` values are fractional (`f64`), which allows inserting a note between two existing siblings by picking a value between their positions — no sibling rows need to be updated. This is the standard mid-point strategy used by CRDTs for ordered sequences.

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

## Adding a New Language

Krillnotes supports two translation surfaces — the React UI (via i18next) and the native application menu (via compile-time embedding). Both are driven by the same JSON files, so adding a language is a single-step process with no Rust changes required.

### 1. Create the locale JSON file

Add a new file at:

```
krillnotes-desktop/src/i18n/locales/<lang-code>.json
```

Use the [BCP 47 language tag](https://www.iana.org/assignments/language-subtag-registry/language-subtag-registry) as the filename (e.g. `pt.json` for Portuguese, `zh-TW.json` for Traditional Chinese).

The file must follow the same structure as `en.json`. Copy it as a starting point:

```bash
cp krillnotes-desktop/src/i18n/locales/en.json \
   krillnotes-desktop/src/i18n/locales/pt.json
```

Translate every value. The `"menu"` section (20 keys) is used by both the React UI and the native menu — translate those carefully using platform-idiomatic terms for your target OS. Leave keys untranslated in English rather than omitting them; the runtime merges translated keys over the English base, so missing keys always fall back gracefully.

Validate JSON syntax before committing:

```bash
python3 -m json.tool krillnotes-desktop/src/i18n/locales/pt.json > /dev/null && echo OK
```

### 2. Register the language in the frontend

Open [krillnotes-desktop/src/i18n/index.ts](krillnotes-desktop/src/i18n/index.ts) and add the new locale to the i18next `resources` map:

```ts
import pt from './locales/pt.json';

// inside the i18next.init({ resources: { ... } }) call:
pt: { translation: pt },
```

Also add it to the language dropdown in [SettingsDialog.tsx](krillnotes-desktop/src/components/SettingsDialog.tsx) (inside the `<select>` in the Appearance tab):

```tsx
<option value="pt">Português (pt)</option>
```

### 3. Build and test

```bash
# Verify Rust compiles (build.rs auto-picks up the new JSON file)
cargo build -p krillnotes-desktop

# Run all tests
cargo test --workspace

# Verify TypeScript
cd krillnotes-desktop && npx tsc --noEmit
```

The `build.rs` build script automatically scans `src/i18n/locales/*.json` at compile time and embeds the new file into the binary — no Rust code changes are needed. The native menu will switch to the new language as soon as the user selects it in Settings.

### How the i18n pipeline works

```
krillnotes-desktop/src/i18n/locales/<lang>.json
            │
            ├── React frontend (i18next)
            │     useTranslation() hook → t('key')
            │     FieldDisplay.tsx, SettingsDialog.tsx, …
            │
            └── Native menu (Rust, compile-time)
                  build.rs  →  $OUT_DIR/locales_generated.rs  (embedded &str pairs)
                  locales.rs::menu_strings(lang)  →  serde_json::Value
                  menu.rs::build_menu(app, &strings)  →  Tauri Menu
                  lib.rs::rebuild_menus()  →  called on language change in update_settings
```

The `locales::menu_strings` function merges the target locale's `menu` object over the English base, so any untranslated keys silently fall back to English without breaking the menu.

---

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Core library | Rust, rusqlite (bundled SQLCipher + vendored OpenSSL), rhai, serde, zip |
| Desktop backend | Tauri v2, mimalloc (global allocator) |
| Frontend | React 19, TypeScript 5, Vite, Tailwind CSS v4 |
| Internationalisation | i18next + react-i18next (frontend); compile-time JSON embedding via `build.rs` (native menu) |
| Workspace encryption | SQLCipher (AES-256-CBC, PBKDF2-HMAC-SHA512) for workspace DBs; ChaCha20-Poly1305 for attachment files |
| Archive encryption | AES-256 zip for `.krillnotes` export archives |
| Identity crypto | ed25519-dalek 2.x (keypairs), argon2 0.5 (KDF), aes-gcm 0.10 (key encryption), hkdf (workspace key derivation) |
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
| 10 — Undo / redo | Done | `RetractOperation` log entries; undo groups; per-workspace history limit |
| 11 — Identity model | Done | Ed25519 + Argon2id; workspace binding; `.swarmid` portable export |
| 12 — HLC + signed operations | Done | `HlcTimestamp` per-operation; Ed25519 `signature` on every op; `hlc_state` table; fractional `f64` positions; `SetTags` + `UpdateNote` variants |
| 13 — Sync infrastructure | Planned | CRDT merge, conflict resolution, `synced` flag, swarm discovery |

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
| [krillnotes-core/src/core/identity.rs](krillnotes-core/src/core/identity.rs) | Identity model — Ed25519 + Argon2id + AES-GCM; workspace bindings; `.swarmid` export/import |
| [krillnotes-core/src/core/undo.rs](krillnotes-core/src/core/undo.rs) | `RetractInverse` enum — maps operations to their compensating inverses |
| [krillnotes-core/src/core/hlc.rs](krillnotes-core/src/core/hlc.rs) | `HlcTimestamp` (wall_ms, counter, node_id) and `HlcClock` — Hybrid Logical Clock for cross-device ordering |
| [krillnotes-core/src/core/schema.sql](krillnotes-core/src/core/schema.sql) | Database DDL (7 tables: notes, note_tags, operations, workspace_meta, user_scripts, attachments, hlc_state) |
| [krillnotes-desktop/src-tauri/build.rs](krillnotes-desktop/src-tauri/build.rs) | Compile-time locale embedding — generates `locales_generated.rs` from `src/i18n/locales/*.json` |
| [krillnotes-desktop/src-tauri/src/lib.rs](krillnotes-desktop/src-tauri/src/lib.rs) | Tauri commands + AppState (identity manager, unlocked identities, menu item handles) |
| [krillnotes-desktop/src-tauri/src/locales.rs](krillnotes-desktop/src-tauri/src/locales.rs) | `menu_strings(lang)` — locale lookup with English merge-over fallback |
| [krillnotes-desktop/src-tauri/src/menu.rs](krillnotes-desktop/src-tauri/src/menu.rs) | Native menu builder — accepts `&serde_json::Value` locale strings |
| [krillnotes-desktop/src/App.tsx](krillnotes-desktop/src/App.tsx) | React root, menu event wiring, export/import |
| [krillnotes-desktop/src/types.ts](krillnotes-desktop/src/types.ts) | Shared TypeScript interfaces |
| [krillnotes-desktop/src/i18n/index.ts](krillnotes-desktop/src/i18n/index.ts) | i18next initialisation + language apply-on-startup |
| [krillnotes-desktop/src/i18n/locales/](krillnotes-desktop/src/i18n/locales/) | Locale JSON files (one per language; drop a new file here to add a language) |
