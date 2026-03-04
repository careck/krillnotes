# CLAUDE.md — Krillnotes

## Project Overview

Krillnotes is a local-first, hierarchical note-taking app. Notes live in a tree, each has a schema-defined type (via Rhai scripts), and every mutation is logged for future CRDT sync. Built with Rust + Tauri v2 + React 19 + SQLCipher.

## Workspace Structure (Two Rust Crates)

```
Krillnotes/
├── krillnotes-core/          # Pure Rust library — NO Tauri, no UI
│   └── src/
│       ├── lib.rs             # Crate root, public re-exports
│       └── core/
│           ├── workspace.rs       # PRIMARY API — all document mutations
│           ├── note.rs            # Note + FieldValue types
│           ├── operation.rs       # Operation enum (CRDT mutations)
│           ├── operation_log.rs   # Append-only log + purge strategies
│           ├── delete.rs          # DeleteAll / PromoteChildren strategies
│           ├── user_script.rs     # UserScript CRUD
│           ├── export.rs          # Zip-based workspace export/import
│           ├── storage.rs         # SQLCipher connection, PRAGMA key, migrations
│           ├── device.rs          # Stable hardware device ID (mac_address hash)
│           ├── error.rs           # KrillnotesError enum (thiserror)
│           ├── schema.sql         # Database DDL (notes, note_tags, operations, workspace_meta, user_scripts)
│           ├── scripting/
│           │   ├── mod.rs             # Engine setup, hook dispatch, query fns
│           │   ├── schema.rs          # Schema registry (field types, flags)
│           │   ├── hooks.rs           # Hook registry (placeholder for global hooks)
│           │   └── display_helpers.rs # HTML helpers (text, table, render_tags, link_to)
│           └── system_scripts/
│               └── text_note.rhai     # Built-in TextNote schema
│
├── krillnotes-desktop/
│   ├── src-tauri/             # Tauri v2 Rust backend
│   │   ├── build.rs           # Compile-time locale embedding → locales_generated.rs
│   │   └── src/
│   │       ├── lib.rs         # Tauri commands + AppState
│   │       ├── locales.rs     # menu_strings(lang) — locale lookup
│   │       └── menu.rs        # Native menu builder
│   └── src/                   # React 19 / TypeScript / Vite / Tailwind v4
│       ├── App.tsx            # Root component, menu event wiring
│       ├── types.ts           # Shared TS interfaces
│       ├── i18n/              # i18next setup + locale JSON files
│       └── components/
│           ├── WorkspaceView.tsx      # Main layout, resizable panels, keyboard nav
│           ├── TreeView.tsx           # Tree rendering
│           ├── TreeNode.tsx           # Individual tree node
│           ├── InfoPanel.tsx          # Detail/edit panel with schema-aware fields
│           ├── FieldEditor.tsx        # Field edit widgets (all field types)
│           ├── FieldDisplay.tsx       # Read-only field display
│           ├── SearchBar.tsx          # Live search with fuzzy matching
│           ├── ScriptManagerDialog.tsx # User script list + CRUD
│           ├── ScriptEditor.tsx       # Rhai code editor (CodeMirror)
│           ├── OperationsLogDialog.tsx # Operations history + purge
│           ├── AddNoteDialog.tsx       # New note dialog
│           ├── DeleteConfirmDialog.tsx # Delete with strategy choice
│           └── ContextMenu.tsx        # Right-click tree menu
│
└── templates/                 # Ready-to-use Rhai templates
    ├── book_collection.rhai
    ├── photo_note.rhai
    └── zettelkasten.rhai
```

## Key Workspace Dependencies

```toml
rusqlite = { version = "0.38", features = ["bundled-sqlcipher-vendored-openssl"] }
rhai = { version = "1.24", features = ["sync"] }    # Arc/Mutex, Send+Sync Engine
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
uuid = { version = "1.7", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
thiserror = "2"
```

## Architecture — How Things Connect

```
User action → React component → invoke("tauri_command", {...})
                                        ↓
                              lib.rs: Tauri command
                              window.label() → lookup Workspace in AppState
                                        ↓
                              workspace.rs method (BEGIN TX → mutate → log_op → COMMIT)
                                        ↓
                              Return result → frontend updates state
```

**AppState** (in `krillnotes-desktop/src-tauri/src/lib.rs`):
- `workspaces: Arc<Mutex<HashMap<String, Workspace>>>` — label → Workspace
- `workspace_paths: Arc<Mutex<HashMap<String, PathBuf>>>` — label → file path
- `workspace_passwords: Arc<Mutex<HashMap<PathBuf, String>>>` — session-only cache
- `paste_menu_items` / `workspace_menu_items` — per-window native menu handles

## Note Fields and actions (hooks) are defined in scripts
- see `SCRIPTING.md` for how to define schemas and hooks

## Coding Conventions

### Rust
- All errors use `KrillnotesError` (thiserror) in core; Tauri commands map to `Result<T, String>`
- Workspace methods wrap mutations in SQL transactions
- The Rhai `Engine` is long-lived (not recreated per request) — `rhai/sync` feature makes it `Send + Sync`
- Schema + hooks are defined together inside `schema()` calls in Rhai scripts
- `include_dir!` embeds system scripts at compile time
- Tests use `#[cfg(test)]` modules with temp in-memory workspaces
- UUIDs for all IDs (`uuid::Uuid`), `chrono::DateTime<Utc>` for timestamps

### TypeScript / React
- React 19 with functional components + hooks
- Tailwind CSS v4 for styling
- i18next for internationalisation (7 languages)
- All Tauri IPC via `@tauri-apps/api` `invoke()`
- CodeMirror for the script editor

## Navigation Tips for Claude Code

**Before reading full source files, prefer these approaches:**
1. Use `grep -rn "symbol_name" krillnotes-core/src/` to locate symbols
2. Use `grep -rn "fn method_name" krillnotes-core/src/core/workspace.rs` for API methods
3. Read specific line ranges: `sed -n '100,150p' path/to/file.rs`
4. For TypeScript types: `grep -rn "interface\|type " krillnotes-desktop/src/types.ts`
5. For Tauri commands: `grep -n "pub async fn" krillnotes-desktop/src-tauri/src/lib.rs`
6. For component props/state: `grep -n "useState\|useEffect\|Props" krillnotes-desktop/src/components/ComponentName.tsx`

**Do NOT read these directories — they are large and not useful:**
- `node_modules/`
- `target/`
- `krillnotes-desktop/dist/`
- `docs/plans/` (design docs for completed features — only read if asked)

## Using rust-analyzer for Code Navigation

rust-analyzer is installed and available. **Use it instead of reading entire files** when you need
to understand types, find definitions, or trace references. This saves significant context.

### Preferred Navigation Strategy (in order of preference)

1. **Structural search** — find patterns in the codebase:
```bash
   # Find all calls matching a pattern (e.g. all .lock().unwrap() calls)
   rust-analyzer search '$a.lock().unwrap()'
   
   # Find specific patterns
   rust-analyzer search 'Workspace::$fn_name($args)'
```

2. **Diagnostics** — check for errors without reading files:
```bash
   rust-analyzer diagnostics .
```

3. **Symbols from a file** — get function/struct/enum signatures without reading the whole file:
```bash
   cat krillnotes-core/src/core/workspace.rs | rust-analyzer symbols
```

4. **cargo doc output** — understand public API and types:
```bash
   # Generate docs, then read specific type docs
   cargo doc --no-deps -p krillnotes-core --document-private-items 2>/dev/null
   # Then check target/doc/krillnotes_core/
```

5. **grep + line ranges** — when you know what you're looking for:
```bash
   grep -rn "fn create_note" krillnotes-core/src/
   sed -n '100,130p' krillnotes-core/src/core/workspace.rs
```

### When NOT to use rust-analyzer

- For quick string/symbol searches, `grep -rn` is faster
- For understanding file structure, `cat file | rust-analyzer symbols` or `grep -n "^pub\|^impl\|^fn\|^struct\|^enum\|^trait" file` is enough
- Don't run `rust-analyzer analysis-stats` — it analyses the whole project and is slow

### Rule: Never Read a Full File to Find One Thing

If you need to understand a single function, type, or trait:
1. First `grep -n` to find the line number
2. Then `sed -n 'START,ENDp'` to read just that section
3. Only read full files if you're doing a comprehensive review or refactor of that file


## Key Types Quick Reference

### Core Rust Types (krillnotes-core)

| Type | File | Purpose |
|------|------|---------|
| `Workspace` | `workspace.rs` | Central API — owns DB connection + Rhai Engine |
| `Note` | `note.rs` | Tree node: id, title, parent_id, position, node_type, fields, tags |
| `FieldValue` | `note.rs` | Enum: Text, Number, Boolean, Date, Email, Select, Rating |
| `Operation` | `operation.rs` | CRDT mutation: CreateNote, UpdateField, DeleteNote, MoveNote, + script ops |
| `OperationLog` | `operation_log.rs` | Append-only log with purge strategies |
| `UserScript` | `user_script.rs` | Rhai script stored per-workspace |
| `KrillnotesError` | `error.rs` | Error enum (thiserror) |
| `SchemaRegistry` | `scripting/schema.rs` | Registered note type schemas |

### Frontend Types (krillnotes-desktop)

| Type | File | Purpose |
|------|------|---------|
| `Note` | `types.ts` | TS mirror of Rust Note |
| `FieldDef` | `types.ts` | Schema field definition |
| `SchemaInfo` | `types.ts` | Schema metadata from core |

## Common Tasks

### Adding a new Tauri command
1. Write `pub async fn my_command(window: Window, state: State<AppState>, ...) -> Result<T, String>` in `lib.rs`
2. Add to `tauri::generate_handler![..., my_command]`
3. Call from React: `invoke("my_command", { ... })`

### Adding a new note type
- UI: Script Manager → New Script → call `schema("TypeName", #{ fields: [...] })`
- Code: Add `.rhai` to `krillnotes-core/src/core/system_scripts/` (auto-embedded)

### Adding a new language
- Copy `en.json` to `krillnotes-desktop/src/i18n/locales/<lang>.json`, translate values
- Register in `i18n/index.ts` resources map + SettingsDialog dropdown
- `build.rs` auto-embeds it at compile time — no Rust changes needed

## Build & Test

```bash
# Dev mode (hot-reload frontend + Rust backend)
cd krillnotes-desktop && npm update && npm run tauri dev

# Core library tests
cargo test -p krillnotes-core

# TypeScript type check
cd krillnotes-desktop && npx tsc --noEmit

# Release build
cd krillnotes-desktop && npm update && npm run tauri build
```

## Database Schema (5 tables)

- `notes` — id, title, node_type, parent_id, position, fields (JSON), is_expanded, created/modified timestamps
- `note_tags` — note_id, tag (junction table, no master tag list)
- `operations` — Append-only mutation log (operation_id, type, data JSON, device_id, timestamp, synced flag)
- `workspace_meta` — Per-device state (device_id, selected_note_id)
- `user_scripts` — id, name, description (Rhai source), load_order, enabled flag

## Things to Know

- **Encryption is mandatory** — `PRAGMA key` is first SQL after open. Empty password only in tests.
- **Rhai `sync` feature** — Engine uses `Arc`/`Mutex` internally, no `unsafe` needed.
- **Multi-window** — Each Tauri window = one workspace. `window.label()` routes commands.
- **`is_expanded` / `selected_note_id`** — View state only, not logged, not synced.
- **Tree positions** are gapless zero-based integers; inserts shift siblings in the same transaction.
- **The application is CROSS Platform** - all features work on Windows, Linux and MacOs.
- **krillnotes-core (in Rust) should be reusable for future applications** - that could include, mobile, web and even headless applications

## How to work
- Always start each implementation in a new worktree
- Always commit and push a pull request to github
- Always update `CHANGELOG.md` with the changes you made after the PR has been merged