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

## Architecture

**Data flow:** React component → `invoke("tauri_command")` → `lib.rs` → `workspace.rs` method (BEGIN TX → mutate → log_op → COMMIT) → return to frontend

**Key deps:** `rusqlite` (SQLCipher), `rhai` (sync feature), `serde`/`serde_json`, `uuid`, `chrono`, `thiserror`

**AppState** (in `lib.rs`): `workspaces: Arc<Mutex<HashMap<String, Workspace>>>` keyed by window label, plus `workspace_paths` and `workspace_passwords` maps

## Note Fields and actions (hooks) are defined in scripts
- see `SCRIPTING.md` for how to define schemas and hooks

## Coding Conventions

### Rust
- `KrillnotesError` (thiserror) in core; Tauri commands return `Result<T, String>`
- Workspace methods wrap mutations in SQL transactions
- Rhai Engine is long-lived (`sync` feature = `Send + Sync`); schemas + hooks in `schema()` calls
- `include_dir!` embeds system scripts at compile time
- Tests: `#[cfg(test)]` with in-memory workspaces. UUIDs for IDs, `chrono::DateTime<Utc>` for timestamps
- **Serde boundary rules** — structs crossing Rust→TS MUST have `#[serde(rename_all = "camelCase")]`. Enum `rename_all` only renames variants, NOT struct variant fields. Always verify JSON keys match TS interfaces. See `gotchas.md` in memory for full details.

### TypeScript / React
- React 19 + Tailwind v4 + i18next (7 languages) + CodeMirror for script editor
- All Tauri IPC via `@tauri-apps/api` `invoke()`
- **i18n is mandatory** — NEVER use hardcoded English strings in JSX. Every user-facing string must use `t('section.key')` from `useTranslation()`. When adding new UI text:
  1. Add the English key to `krillnotes-desktop/src/i18n/locales/en.json`
  2. Add translated keys to ALL 6 other locale files (de, es, fr, ja, ko, zh)
  3. Use `t('section.key')` in the component, with `{{ interpolation }}` for dynamic values
  4. For date/number formatting, pass `i18n.language` to `toLocaleDateString()`/`toLocaleString()`
  5. All 7 locale files must have identical key structures — no missing keys

## Context Management (MANDATORY)

**Context is a scarce resource. Every file read, every test output, every error trace consumes it. Follow these rules strictly to preserve context for actual implementation work.**

### Use LSP Instead of Reading Files

The rust-analyzer LSP tool is available and MUST be the first choice for code navigation in Rust files. It returns only the information you need, not entire files.

| Need | LSP operation | Instead of |
|------|--------------|------------|
| What type is this? | `hover` on the symbol | Reading the file to find the definition |
| Where is this defined? | `goToDefinition` | `grep -rn "fn name\|struct name"` |
| Who calls this? | `findReferences` or `incomingCalls` | `grep -rn "method_name"` across the project |
| What's in this file? | `documentSymbol` | Reading the entire file |
| Find a type across crates | `workspaceSymbol` | Grepping multiple directories |
| What implements this trait? | `goToImplementation` | Manual search |

**Rules:**
1. **NEVER read an entire Rust file just to check a type signature or find a function** — use `hover` or `documentSymbol` instead
2. **NEVER grep the whole project to find references** — use `findReferences` instead
3. **Only read full files when you are about to make edits across the whole file** (refactor, review)
4. When you need to read code you're about to edit, use `Read` with `offset`/`limit` to read just that section — find the line number with LSP or grep first

### Fallback Navigation (when LSP isn't applicable)

- For quick string searches: `Grep` tool (not bash grep)
- For file discovery: `Glob` tool (not bash find)
- For TypeScript types: `Grep` in `types.ts` or `documentSymbol` via LSP if TS server is available
- Read specific line ranges with `Read` offset/limit, not full files

### Do NOT read these directories — they are large and not useful:
- `node_modules/`, `target/`, `krillnotes-desktop/dist/`
- `docs/plans/` (design docs for completed features — only read if asked)

## Parallelise with Subagents (MANDATORY for cross-cutting features)

**When a task has 2+ independent subtasks, use the Agent tool to run them in parallel.** This is not optional for features that touch multiple layers (Rust core, Tauri commands, frontend). Subagents protect the main context window from bloat and allow parallel execution.

### When to use subagents

- **Always** when implementing independent modules (e.g., "add Rust methods" + "update TypeScript types" + "write tests" can each be a subagent)
- **Always** when updating multiple scripts/templates with similar changes
- **Always** for research tasks (exploring existing code) before writing implementation plans
- **Never** for sequential work where step 2 depends on step 1's output

### Subagent patterns for this codebase

| Task pattern | Subagent split |
|-------------|---------------|
| New feature across Rust + Tauri + React | Agent 1: Rust core methods + tests. Agent 2: Tauri commands. Agent 3: React components + types |
| Update all .rhai scripts | One agent per script (or batch related scripts) |
| Bug investigation | Agent 1: check Rust side. Agent 2: check TypeScript side. Compare results |
| Add new Operation variant | Agent 1: operation.rs + workspace.rs. Agent 2: export/import. Agent 3: frontend |

### Rules:
1. **Plan first, then dispatch** — write a clear plan, then farm out independent tasks to subagents
2. **Give subagents precise scope** — file paths, function names, expected inputs/outputs
3. **Don't duplicate work** — if a subagent is researching something, don't also do the same search yourself
4. **Collect and integrate** — after subagents complete, integrate their work in the main context


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

`notes`, `note_tags`, `operations` (append-only mutation log), `workspace_meta` (per-device state), `user_scripts`

## Things to Know

- **Encryption mandatory** — `PRAGMA key` first after open; empty password only in tests
- **Multi-window** — each Tauri window = one workspace, routed by `window.label()`
- **Tree positions** — gapless zero-based integers; inserts shift siblings in same transaction
- **Cross-platform** — all features must work on Windows, Linux, macOS
- **krillnotes-core must be reusable** — no Tauri deps; future targets include mobile, web, headless

## Branching Strategy

- **`master`** — stable 1.x release branch. Only bugfixes and releases land here.
- **`development`** — active development branch for new features.
- **Feature branches** (`feat/`) → PR targets `development`
- **Fix branches** (`fix/`) → PR targets `master` (unless fixing something only on `development`)

## How to work
- Always start each implementation in a new worktree
- Always commit and push a pull request to github
- New features target `development`; bugfixes target `master`
- Always update `CHANGELOG.md` with the changes you made after the PR has been merged
- Use terse, functional language to preserve context tokens!