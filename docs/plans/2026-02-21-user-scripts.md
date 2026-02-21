# User Scripts Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Enable user-defined Rhai scripts stored in the workspace database, managed through a dedicated dialog with CodeMirror editing.

**Architecture:** New `user_scripts` SQLite table stores script source with front-matter metadata. `ScriptRegistry` tracks system vs user origin, supports clearing and reloading user scripts. New Tauri commands expose CRUD. React dialog with CodeMirror provides the management UI.

**Tech Stack:** Rust/SQLite (backend), Rhai (scripting), React/TypeScript (frontend), CodeMirror 6 (editor), Tauri v2 (IPC)

---

### Task 1: Database schema and migration

**Files:**
- Modify: `krillnotes-core/src/core/schema.sql:37` (append new table)
- Modify: `krillnotes-core/src/core/storage.rs:62-75` (add migration)

**Step 1: Add user_scripts table to schema.sql**

Append after line 37 in `krillnotes-core/src/core/schema.sql`:

```sql

-- User scripts
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

**Step 2: Add migration in storage.rs**

After the `is_expanded` migration block (line 75) in `krillnotes-core/src/core/storage.rs`, add:

```rust
        // Migration: add user_scripts table if it doesn't exist.
        let user_scripts_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='user_scripts'",
                [],
                |row| row.get::<_, i64>(0).map(|count| count > 0),
            )?;

        if !user_scripts_exists {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS user_scripts (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL DEFAULT '',
                    description TEXT NOT NULL DEFAULT '',
                    source_code TEXT NOT NULL,
                    load_order INTEGER NOT NULL DEFAULT 0,
                    enabled INTEGER NOT NULL DEFAULT 1,
                    created_at INTEGER NOT NULL,
                    modified_at INTEGER NOT NULL
                )",
            )?;
        }
```

**Step 3: Write test for migration**

Add to the `#[cfg(test)] mod tests` in `krillnotes-core/src/core/storage.rs`:

```rust
    #[test]
    fn test_migration_creates_user_scripts_table() {
        let temp = NamedTempFile::new().unwrap();

        {
            let conn = Connection::open(temp.path()).unwrap();
            conn.execute(
                "CREATE TABLE notes (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    node_type TEXT NOT NULL,
                    parent_id TEXT,
                    position INTEGER NOT NULL,
                    created_at INTEGER NOT NULL,
                    modified_at INTEGER NOT NULL,
                    created_by INTEGER NOT NULL,
                    modified_by INTEGER NOT NULL,
                    fields_json TEXT NOT NULL,
                    is_expanded INTEGER DEFAULT 1
                )",
                [],
            ).unwrap();
            conn.execute("CREATE TABLE operations (id INTEGER PRIMARY KEY)", []).unwrap();
            conn.execute("CREATE TABLE workspace_meta (key TEXT PRIMARY KEY, value TEXT)", []).unwrap();
        }

        let storage = Storage::open(temp.path()).unwrap();

        let table_exists: bool = storage
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='user_scripts'",
                [],
                |row| row.get::<_, i64>(0).map(|count| count > 0),
            )
            .unwrap();

        assert!(table_exists, "user_scripts table should exist after migration");
    }
```

**Step 4: Run tests**

Run: `cargo test -p krillnotes-core storage::tests`
Expected: All pass including new `test_migration_creates_user_scripts_table`

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/schema.sql krillnotes-core/src/core/storage.rs
git commit -m "feat: add user_scripts table schema and migration"
```

---

### Task 2: UserScript type and front matter parser

**Files:**
- Create: `krillnotes-core/src/core/user_script.rs`
- Modify: `krillnotes-core/src/core/mod.rs:6-33` (add module + re-export)
- Modify: `krillnotes-core/src/lib.rs:13-22` (add re-export)

**Step 1: Write the failing test for front matter parsing**

Create `krillnotes-core/src/core/user_script.rs`:

```rust
//! User script storage type and front-matter parser.

use serde::{Deserialize, Serialize};

/// A user-defined Rhai script stored in the workspace database.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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

/// Parsed front-matter metadata from a script's leading comments.
#[derive(Debug, Clone, Default)]
pub struct FrontMatter {
    pub name: String,
    pub description: String,
}

/// Parses `// @key: value` front-matter lines from the top of a script.
///
/// Stops at the first line that is not a comment or does not contain `@`.
/// Returns a [`FrontMatter`] with any extracted `name` and `description`.
pub fn parse_front_matter(source: &str) -> FrontMatter {
    let mut fm = FrontMatter::default();
    for line in source.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("//") {
            if trimmed.is_empty() {
                continue;
            }
            break;
        }
        let comment_body = trimmed.trim_start_matches("//").trim();
        if !comment_body.starts_with('@') {
            continue;
        }
        let after_at = &comment_body[1..];
        if let Some((key, value)) = after_at.split_once(':') {
            let key = key.trim();
            let value = value.trim();
            match key {
                "name" => fm.name = value.to_string(),
                "description" => fm.description = value.to_string(),
                _ => {} // ignore unknown keys
            }
        }
    }
    fm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_front_matter_basic() {
        let source = r#"// @name: My Script
// @description: A test script

schema("Test", #{ fields: [] });
"#;
        let fm = parse_front_matter(source);
        assert_eq!(fm.name, "My Script");
        assert_eq!(fm.description, "A test script");
    }

    #[test]
    fn test_parse_front_matter_missing_description() {
        let source = "// @name: Only Name\nschema(\"X\", #{ fields: [] });";
        let fm = parse_front_matter(source);
        assert_eq!(fm.name, "Only Name");
        assert_eq!(fm.description, "");
    }

    #[test]
    fn test_parse_front_matter_no_front_matter() {
        let source = "schema(\"X\", #{ fields: [] });";
        let fm = parse_front_matter(source);
        assert_eq!(fm.name, "");
        assert_eq!(fm.description, "");
    }

    #[test]
    fn test_parse_front_matter_comment_without_at_is_skipped() {
        let source = "// This is a regular comment\n// @name: After Comment\nschema(\"X\", #{ fields: [] });";
        let fm = parse_front_matter(source);
        assert_eq!(fm.name, "After Comment");
    }

    #[test]
    fn test_parse_front_matter_blank_lines_before_code() {
        let source = "// @name: Spacey\n\n\nschema(\"X\", #{ fields: [] });";
        let fm = parse_front_matter(source);
        assert_eq!(fm.name, "Spacey");
    }
}
```

**Step 2: Add module to mod.rs**

In `krillnotes-core/src/core/mod.rs`, add after `pub mod storage;` (line 13):

```rust
pub mod user_script;
```

And add a re-export after the existing re-exports (line 32):

```rust
#[doc(inline)]
pub use user_script::UserScript;
```

**Step 3: Add re-export to lib.rs**

In `krillnotes-core/src/lib.rs`, add `user_script::UserScript` to the re-export block (line 21, after `storage::Storage,`):

```rust
    user_script::UserScript,
```

**Step 4: Run tests**

Run: `cargo test -p krillnotes-core user_script::tests`
Expected: All 5 tests pass

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/user_script.rs krillnotes-core/src/core/mod.rs krillnotes-core/src/lib.rs
git commit -m "feat: add UserScript type and front matter parser"
```

---

### Task 3: Workspace CRUD methods for user scripts

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs` (add methods before closing `}` of `impl Workspace` at line 793)

**Step 1: Write failing tests for user script CRUD**

Add to `#[cfg(test)] mod tests` in `krillnotes-core/src/core/workspace.rs`:

```rust
    #[test]
    fn test_list_user_scripts_empty() {
        let temp = NamedTempFile::new().unwrap();
        let workspace = Workspace::create(temp.path()).unwrap();
        let scripts = workspace.list_user_scripts().unwrap();
        assert!(scripts.is_empty());
    }

    #[test]
    fn test_create_user_script() {
        let temp = NamedTempFile::new().unwrap();
        let mut workspace = Workspace::create(temp.path()).unwrap();
        let source = "// @name: Test Script\n// @description: A test\nschema(\"TestType\", #{ fields: [] });";
        let script = workspace.create_user_script(source).unwrap();
        assert_eq!(script.name, "Test Script");
        assert_eq!(script.description, "A test");
        assert!(script.enabled);
        assert_eq!(script.load_order, 0);
    }

    #[test]
    fn test_create_user_script_missing_name_fails() {
        let temp = NamedTempFile::new().unwrap();
        let mut workspace = Workspace::create(temp.path()).unwrap();
        let source = "// no name here\nschema(\"X\", #{ fields: [] });";
        let result = workspace.create_user_script(source);
        assert!(result.is_err());
    }

    #[test]
    fn test_update_user_script() {
        let temp = NamedTempFile::new().unwrap();
        let mut workspace = Workspace::create(temp.path()).unwrap();
        let source = "// @name: Original\nschema(\"Orig\", #{ fields: [] });";
        let script = workspace.create_user_script(source).unwrap();

        let new_source = "// @name: Updated\nschema(\"Updated\", #{ fields: [] });";
        let updated = workspace.update_user_script(&script.id, new_source).unwrap();
        assert_eq!(updated.name, "Updated");
    }

    #[test]
    fn test_delete_user_script() {
        let temp = NamedTempFile::new().unwrap();
        let mut workspace = Workspace::create(temp.path()).unwrap();
        let source = "// @name: ToDelete\nschema(\"Del\", #{ fields: [] });";
        let script = workspace.create_user_script(source).unwrap();

        workspace.delete_user_script(&script.id).unwrap();
        let scripts = workspace.list_user_scripts().unwrap();
        assert!(scripts.is_empty());
    }

    #[test]
    fn test_toggle_user_script() {
        let temp = NamedTempFile::new().unwrap();
        let mut workspace = Workspace::create(temp.path()).unwrap();
        let source = "// @name: Toggle\nschema(\"Tog\", #{ fields: [] });";
        let script = workspace.create_user_script(source).unwrap();
        assert!(script.enabled);

        workspace.toggle_user_script(&script.id, false).unwrap();
        let scripts = workspace.list_user_scripts().unwrap();
        assert!(!scripts[0].enabled);
    }

    #[test]
    fn test_user_scripts_sorted_by_load_order() {
        let temp = NamedTempFile::new().unwrap();
        let mut workspace = Workspace::create(temp.path()).unwrap();
        let s1 = "// @name: Second\nschema(\"S2\", #{ fields: [] });";
        let s2 = "// @name: First\nschema(\"S1\", #{ fields: [] });";
        workspace.create_user_script(s1).unwrap();
        let second = workspace.create_user_script(s2).unwrap();
        workspace.reorder_user_script(&second.id, -1).unwrap();

        let scripts = workspace.list_user_scripts().unwrap();
        assert_eq!(scripts[0].name, "First");
        assert_eq!(scripts[1].name, "Second");
    }
```

**Step 2: Run tests to see them fail**

Run: `cargo test -p krillnotes-core workspace::tests::test_list_user_scripts_empty`
Expected: FAIL — method not found

**Step 3: Implement workspace CRUD methods**

Add these imports at the top of `krillnotes-core/src/core/workspace.rs` (in the existing `use` block near line 4):

```rust
use crate::user_script::{self, UserScript};
```

Add these methods before the closing `}` of `impl Workspace` (before line 793):

```rust
    // ── User-script CRUD ──────────────────────────────────────────

    /// Returns all user scripts, ordered by `load_order` ascending.
    pub fn list_user_scripts(&self) -> Result<Vec<UserScript>> {
        let mut stmt = self.connection().prepare(
            "SELECT id, name, description, source_code, load_order, enabled, created_at, modified_at
             FROM user_scripts ORDER BY load_order ASC, created_at ASC",
        )?;
        let scripts = stmt
            .query_map([], |row| {
                Ok(UserScript {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    source_code: row.get(3)?,
                    load_order: row.get(4)?,
                    enabled: row.get::<_, i64>(5).map(|v| v != 0)?,
                    created_at: row.get(6)?,
                    modified_at: row.get(7)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(scripts)
    }

    /// Returns a single user script by ID.
    pub fn get_user_script(&self, script_id: &str) -> Result<UserScript> {
        self.connection()
            .query_row(
                "SELECT id, name, description, source_code, load_order, enabled, created_at, modified_at
                 FROM user_scripts WHERE id = ?",
                [script_id],
                |row| {
                    Ok(UserScript {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        description: row.get(2)?,
                        source_code: row.get(3)?,
                        load_order: row.get(4)?,
                        enabled: row.get::<_, i64>(5).map(|v| v != 0)?,
                        created_at: row.get(6)?,
                        modified_at: row.get(7)?,
                    })
                },
            )
            .map_err(|_| KrillnotesError::NoteNotFound(format!("User script {script_id} not found")))
    }

    /// Creates a new user script from its source code, parsing front matter for name/description.
    ///
    /// Returns an error if `@name` is missing from the front matter.
    /// The script is compiled and executed; on failure it is saved but disabled.
    pub fn create_user_script(&mut self, source_code: &str) -> Result<UserScript> {
        let fm = user_script::parse_front_matter(source_code);
        if fm.name.is_empty() {
            return Err(KrillnotesError::ValidationFailed(
                "Script must include a '// @name:' front matter line".to_string(),
            ));
        }

        let now = chrono::Utc::now().timestamp();
        let id = uuid::Uuid::new_v4().to_string();

        // Determine next load_order
        let max_order: i32 = self
            .connection()
            .query_row("SELECT COALESCE(MAX(load_order), -1) FROM user_scripts", [], |row| row.get(0))
            .unwrap_or(-1);

        // Try to compile and load the script
        let compile_ok = self.script_registry.load_script(source_code).is_ok();

        self.connection().execute(
            "INSERT INTO user_scripts (id, name, description, source_code, load_order, enabled, created_at, modified_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![id, fm.name, fm.description, source_code, max_order + 1, compile_ok, now, now],
        )?;

        if !compile_ok {
            // Reload all user scripts to clean up any partial state
            self.reload_user_scripts()?;
        }

        self.get_user_script(&id)
    }

    /// Updates an existing user script's source code, re-parsing front matter.
    pub fn update_user_script(&mut self, script_id: &str, source_code: &str) -> Result<UserScript> {
        let fm = user_script::parse_front_matter(source_code);
        if fm.name.is_empty() {
            return Err(KrillnotesError::ValidationFailed(
                "Script must include a '// @name:' front matter line".to_string(),
            ));
        }

        let now = chrono::Utc::now().timestamp();
        let changes = self.connection().execute(
            "UPDATE user_scripts SET name = ?, description = ?, source_code = ?, modified_at = ? WHERE id = ?",
            rusqlite::params![fm.name, fm.description, source_code, now, script_id],
        )?;

        if changes == 0 {
            return Err(KrillnotesError::NoteNotFound(format!("User script {script_id} not found")));
        }

        self.reload_user_scripts()?;
        self.get_user_script(script_id)
    }

    /// Deletes a user script by ID and reloads remaining scripts.
    pub fn delete_user_script(&mut self, script_id: &str) -> Result<()> {
        self.connection().execute("DELETE FROM user_scripts WHERE id = ?", [script_id])?;
        self.reload_user_scripts()
    }

    /// Toggles the enabled state of a user script and reloads.
    pub fn toggle_user_script(&mut self, script_id: &str, enabled: bool) -> Result<()> {
        self.connection().execute(
            "UPDATE user_scripts SET enabled = ? WHERE id = ?",
            rusqlite::params![enabled, script_id],
        )?;
        self.reload_user_scripts()
    }

    /// Changes the load order of a user script and reloads.
    pub fn reorder_user_script(&mut self, script_id: &str, new_load_order: i32) -> Result<()> {
        self.connection().execute(
            "UPDATE user_scripts SET load_order = ? WHERE id = ?",
            rusqlite::params![new_load_order, script_id],
        )?;
        self.reload_user_scripts()
    }

    /// Clears all user-registered schemas/hooks and re-executes enabled user scripts.
    fn reload_user_scripts(&mut self) -> Result<()> {
        self.script_registry.clear_user_registrations();
        let scripts = self.list_user_scripts()?;
        for script in scripts.iter().filter(|s| s.enabled) {
            if let Err(e) = self.script_registry.load_user_script(&script.source_code) {
                eprintln!("Failed to load user script '{}': {}", script.name, e);
            }
        }
        Ok(())
    }
```

**Step 4: Run tests**

Run: `cargo test -p krillnotes-core workspace::tests`
Expected: Will fail because `ScriptRegistry::clear_user_registrations` and `load_user_script` don't exist yet — that's Task 4. The tests are written but we proceed to Task 4 before verifying.

**Step 5: Commit (tests + methods)**

```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "feat: add workspace CRUD methods for user scripts"
```

---

### Task 4: ScriptRegistry changes — origin tracking, clear/reload, new API

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs` (add methods, new host functions)
- Modify: `krillnotes-core/src/core/scripting/schema.rs` (add origin tracking, clear method)
- Modify: `krillnotes-core/src/core/scripting/hooks.rs` (add origin tracking, clear method)

**Step 1: Add ScriptSource tracking to schema.rs**

In `krillnotes-core/src/core/scripting/schema.rs`, add after the existing imports (line 7):

```rust
/// Tracks whether a registration came from a system or user script.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum ScriptSource {
    System,
    User,
}
```

Change the `SchemaRegistry` `schemas` field type and add `current_source`:

Replace the `SchemaRegistry` struct and impl (lines 143-171) with:

```rust
/// Private store for registered schemas. No Rhai dependency.
#[derive(Debug)]
pub(super) struct SchemaRegistry {
    schemas: Arc<Mutex<HashMap<String, Schema>>>,
    /// Tracks which schemas came from user scripts so they can be cleared on reload.
    user_schemas: Arc<Mutex<Vec<String>>>,
    /// Set to `User` while loading user scripts so new registrations are tracked.
    current_source: Arc<Mutex<ScriptSource>>,
}

impl SchemaRegistry {
    pub(super) fn new() -> Self {
        Self {
            schemas: Arc::new(Mutex::new(HashMap::new())),
            user_schemas: Arc::new(Mutex::new(Vec::new())),
            current_source: Arc::new(Mutex::new(ScriptSource::System)),
        }
    }

    /// Returns a clone of the inner `Arc` so Rhai host-function closures can write into it.
    pub(super) fn schemas_arc(&self) -> Arc<Mutex<HashMap<String, Schema>>> {
        Arc::clone(&self.schemas)
    }

    pub(super) fn user_schemas_arc(&self) -> Arc<Mutex<Vec<String>>> {
        Arc::clone(&self.user_schemas)
    }

    pub(super) fn current_source_arc(&self) -> Arc<Mutex<ScriptSource>> {
        Arc::clone(&self.current_source)
    }

    pub(super) fn set_source(&self, source: ScriptSource) {
        *self.current_source.lock().unwrap() = source;
    }

    pub(super) fn get(&self, name: &str) -> Result<Schema> {
        self.schemas
            .lock()
            .map_err(|_| KrillnotesError::Scripting("Schema registry lock poisoned".to_string()))?
            .get(name)
            .cloned()
            .ok_or_else(|| KrillnotesError::SchemaNotFound(name.to_string()))
    }

    pub(super) fn exists(&self, name: &str) -> bool {
        self.schemas.lock().unwrap().contains_key(name)
    }

    pub(super) fn list(&self) -> Vec<String> {
        self.schemas.lock().unwrap().keys().cloned().collect()
    }

    /// Removes all schemas that were registered by user scripts.
    pub(super) fn clear_user(&self) {
        let user_names: Vec<String> = self.user_schemas.lock().unwrap().drain(..).collect();
        let mut schemas = self.schemas.lock().unwrap();
        for name in user_names {
            schemas.remove(&name);
        }
    }
}
```

**Step 2: Update schema() host function to track source**

In `krillnotes-core/src/core/scripting/mod.rs`, update the `schema()` host function registration (lines 44-52) to also record user schemas.

Replace the schema host function registration block with:

```rust
        // Register schema() host function — writes into SchemaRegistry.
        let schemas_arc = schema_registry.schemas_arc();
        let user_schemas_arc = schema_registry.user_schemas_arc();
        let source_arc = schema_registry.current_source_arc();
        engine.register_fn("schema", move |name: String, def: rhai::Map| -> std::result::Result<Dynamic, Box<EvalAltResult>> {
            let s = Schema::parse_from_rhai(&name, &def)
                .map_err(|e| -> Box<EvalAltResult> { e.to_string().into() })?;
            schemas_arc.lock().unwrap().insert(name.clone(), s);
            if *source_arc.lock().unwrap() == schema::ScriptSource::User {
                user_schemas_arc.lock().unwrap().push(name);
            }
            Ok(Dynamic::UNIT)
        });
```

**Step 3: Add origin tracking to HookRegistry**

In `krillnotes-core/src/core/scripting/hooks.rs`, update the `HookRegistry` to track user hooks:

Replace the struct and `new`/`on_save_hooks_arc` (lines 23-36) with:

```rust
#[derive(Debug)]
pub struct HookRegistry {
    on_save_hooks: Arc<Mutex<HashMap<String, HookEntry>>>,
    /// Hook names registered by user scripts, so they can be cleared on reload.
    user_hooks: Arc<Mutex<Vec<String>>>,
}

impl HookRegistry {
    pub(super) fn new() -> Self {
        Self {
            on_save_hooks: Arc::new(Mutex::new(HashMap::new())),
            user_hooks: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn on_save_hooks_arc(&self) -> Arc<Mutex<HashMap<String, HookEntry>>> {
        Arc::clone(&self.on_save_hooks)
    }

    pub(super) fn user_hooks_arc(&self) -> Arc<Mutex<Vec<String>>> {
        Arc::clone(&self.user_hooks)
    }

    /// Removes all hooks that were registered by user scripts.
    pub(super) fn clear_user(&self) {
        let user_names: Vec<String> = self.user_hooks.lock().unwrap().drain(..).collect();
        let mut hooks = self.on_save_hooks.lock().unwrap();
        for name in user_names {
            hooks.remove(&name);
        }
    }
```

**Step 4: Update on_save host function to track source**

In `krillnotes-core/src/core/scripting/mod.rs`, update the `on_save()` host function registration to also record user hooks.

Replace the on_save registration block (lines 54-69) with:

```rust
        // Register on_save() host function — writes into HookRegistry.
        let hooks_arc = hook_registry.on_save_hooks_arc();
        let user_hooks_arc = hook_registry.user_hooks_arc();
        let ast_arc = Arc::clone(&current_loading_ast);
        let hook_source_arc = schema_registry.current_source_arc();
        engine.register_fn("on_save", move |name: String, fn_ptr: FnPtr| -> std::result::Result<Dynamic, Box<EvalAltResult>> {
            let maybe_ast = ast_arc.lock().unwrap().clone();
            let ast = maybe_ast.ok_or_else(|| -> Box<EvalAltResult> {
                "on_save called outside of load_script".to_string().into()
            })?;
            hooks_arc.lock().unwrap().insert(name.clone(), HookEntry { fn_ptr, ast });
            if *hook_source_arc.lock().unwrap() == schema::ScriptSource::User {
                user_hooks_arc.lock().unwrap().push(name);
            }
            Ok(Dynamic::UNIT)
        });
```

**Step 5: Add schema_exists and get_schema_fields host functions**

In `krillnotes-core/src/core/scripting/mod.rs`, after the `on_save` registration (before `let mut registry = Self {`), add:

```rust
        // Register schema_exists() — query function for user scripts.
        let exists_arc = schema_registry.schemas_arc();
        engine.register_fn("schema_exists", move |name: String| -> bool {
            exists_arc.lock().unwrap().contains_key(&name)
        });

        // Register get_schema_fields() — returns field definitions as Rhai array.
        let fields_arc = schema_registry.schemas_arc();
        engine.register_fn("get_schema_fields", move |name: String| -> std::result::Result<Dynamic, Box<EvalAltResult>> {
            let schemas = fields_arc.lock().unwrap();
            let schema = schemas.get(&name).ok_or_else(|| -> Box<EvalAltResult> {
                format!("Schema '{name}' not found").into()
            })?;
            let mut arr = rhai::Array::new();
            for field in &schema.fields {
                let mut map = rhai::Map::new();
                map.insert("name".into(), Dynamic::from(field.name.clone()));
                map.insert("type".into(), Dynamic::from(field.field_type.clone()));
                map.insert("required".into(), Dynamic::from(field.required));
                map.insert("can_view".into(), Dynamic::from(field.can_view));
                map.insert("can_edit".into(), Dynamic::from(field.can_edit));
                arr.push(Dynamic::from(map));
            }
            Ok(Dynamic::from(arr))
        });
```

**Step 6: Add load_user_script and clear_user_registrations methods to ScriptRegistry**

In `krillnotes-core/src/core/scripting/mod.rs`, add after the `run_on_save_hook` method (before the closing `}` of `impl ScriptRegistry`):

```rust
    /// Loads a user script, marking all registrations as user-sourced.
    pub fn load_user_script(&mut self, script: &str) -> Result<()> {
        self.schema_registry.set_source(schema::ScriptSource::User);
        let result = self.load_script(script);
        self.schema_registry.set_source(schema::ScriptSource::System);
        result
    }

    /// Removes all schemas and hooks registered by user scripts.
    pub fn clear_user_registrations(&self) {
        self.schema_registry.clear_user();
        self.hook_registry.clear_user();
    }

    /// Returns `true` if a schema with `name` is registered.
    pub fn schema_exists(&self, name: &str) -> bool {
        self.schema_registry.exists(name)
    }
```

**Step 7: Write tests for new ScriptRegistry functionality**

Add to the `#[cfg(test)] mod tests` in `krillnotes-core/src/core/scripting/mod.rs`:

```rust
    #[test]
    fn test_load_user_script_and_clear() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_user_script(r#"
            schema("UserType", #{ fields: [#{ name: "x", type: "text" }] });
        "#).unwrap();

        assert!(registry.get_schema("UserType").is_ok());

        registry.clear_user_registrations();

        assert!(registry.get_schema("UserType").is_err());
        // System schemas should still work
        assert!(registry.get_schema("TextNote").is_ok());
        assert!(registry.get_schema("Contact").is_ok());
    }

    #[test]
    fn test_clear_user_does_not_remove_system_schemas() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_user_script(r#"
            schema("Custom", #{ fields: [#{ name: "a", type: "text" }] });
        "#).unwrap();

        registry.clear_user_registrations();

        let types = registry.list_types().unwrap();
        assert!(types.contains(&"TextNote".to_string()));
        assert!(types.contains(&"Contact".to_string()));
        assert!(!types.contains(&"Custom".to_string()));
    }

    #[test]
    fn test_schema_exists_host_function() {
        let mut registry = ScriptRegistry::new().unwrap();
        assert!(registry.schema_exists("TextNote"));
        assert!(!registry.schema_exists("NonExistent"));

        // Test via script execution
        registry.load_script(r#"
            let exists = schema_exists("TextNote");
            if !exists { throw "TextNote should exist"; }
            let missing = schema_exists("Missing");
            if missing { throw "Missing should not exist"; }
        "#).unwrap();
    }

    #[test]
    fn test_get_schema_fields_host_function() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_script(r#"
            let fields = get_schema_fields("TextNote");
            if fields.len() != 1 { throw "Expected 1 field, got " + fields.len(); }
            if fields[0].name != "body" { throw "Expected 'body', got " + fields[0].name; }
            if fields[0].type != "textarea" { throw "Expected 'textarea', got " + fields[0].type; }
        "#).unwrap();
    }

    #[test]
    fn test_user_hooks_cleared_on_clear() {
        let mut registry = ScriptRegistry::new().unwrap();
        registry.load_user_script(r#"
            schema("Hooked", #{ fields: [#{ name: "x", type: "text" }] });
            on_save("Hooked", |note| { note });
        "#).unwrap();
        assert!(registry.hooks().has_hook("Hooked"));

        registry.clear_user_registrations();
        assert!(!registry.hooks().has_hook("Hooked"));
        // System hook should remain
        assert!(registry.hooks().has_hook("Contact"));
    }
```

**Step 8: Run all tests**

Run: `cargo test -p krillnotes-core`
Expected: All tests pass (including the Task 3 workspace tests which now have the required methods)

**Step 9: Commit**

```bash
git add krillnotes-core/src/core/scripting/mod.rs krillnotes-core/src/core/scripting/schema.rs krillnotes-core/src/core/scripting/hooks.rs
git commit -m "feat: ScriptRegistry origin tracking, clear/reload, schema_exists/get_schema_fields API"
```

---

### Task 5: Load user scripts on workspace open

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs:121-149` (open method)

**Step 1: Write failing test**

Add to `#[cfg(test)] mod tests` in `krillnotes-core/src/core/workspace.rs`:

```rust
    #[test]
    fn test_user_scripts_loaded_on_open() {
        let temp = NamedTempFile::new().unwrap();

        {
            let mut workspace = Workspace::create(temp.path()).unwrap();
            workspace.create_user_script(
                "// @name: TestOpen\nschema(\"OpenType\", #{ fields: [#{ name: \"x\", type: \"text\" }] });"
            ).unwrap();
        }

        let workspace = Workspace::open(temp.path()).unwrap();
        assert!(workspace.script_registry().get_schema("OpenType").is_ok());
    }

    #[test]
    fn test_disabled_user_scripts_not_loaded_on_open() {
        let temp = NamedTempFile::new().unwrap();

        {
            let mut workspace = Workspace::create(temp.path()).unwrap();
            let script = workspace.create_user_script(
                "// @name: Disabled\nschema(\"DisType\", #{ fields: [#{ name: \"x\", type: \"text\" }] });"
            ).unwrap();
            workspace.toggle_user_script(&script.id, false).unwrap();
        }

        let workspace = Workspace::open(temp.path()).unwrap();
        assert!(workspace.script_registry().get_schema("DisType").is_err());
    }
```

**Step 2: Modify Workspace::open to load user scripts**

In `krillnotes-core/src/core/workspace.rs`, modify the `open` method. After line 148 (`current_user_id,`), before the closing `})`:

Replace the end of the `open` method (lines 143-150) with:

```rust
        let mut ws = Self {
            storage,
            script_registry,
            operation_log,
            device_id,
            current_user_id,
        };

        // Load enabled user scripts in load_order.
        let user_scripts = ws.list_user_scripts()?;
        for script in user_scripts.iter().filter(|s| s.enabled) {
            if let Err(e) = ws.script_registry.load_user_script(&script.source_code) {
                eprintln!("Failed to load user script '{}': {}", script.name, e);
            }
        }

        Ok(ws)
```

**Step 3: Run tests**

Run: `cargo test -p krillnotes-core workspace::tests`
Expected: All pass

**Step 4: Commit**

```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "feat: load user scripts on workspace open"
```

---

### Task 6: Tauri commands for user scripts

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs` (add commands, register in handler)

**Step 1: Add Tauri commands**

Add before the `MENU_MESSAGES` constant (line 502) in `krillnotes-desktop/src-tauri/src/lib.rs`:

```rust
// ── User-script commands ──────────────────────────────────────────

/// Returns all user scripts for the calling window's workspace.
#[tauri::command]
fn list_user_scripts(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<Vec<UserScript>, String> {
    let label = window.label();
    state.workspaces.lock()
        .expect("Mutex poisoned")
        .get(label)
        .ok_or("No workspace open")?
        .list_user_scripts()
        .map_err(|e| e.to_string())
}

/// Returns a single user script by ID.
#[tauri::command]
fn get_user_script(
    window: tauri::Window,
    state: State<'_, AppState>,
    script_id: String,
) -> std::result::Result<UserScript, String> {
    let label = window.label();
    state.workspaces.lock()
        .expect("Mutex poisoned")
        .get(label)
        .ok_or("No workspace open")?
        .get_user_script(&script_id)
        .map_err(|e| e.to_string())
}

/// Creates a new user script from source code.
#[tauri::command]
fn create_user_script(
    window: tauri::Window,
    state: State<'_, AppState>,
    source_code: String,
) -> std::result::Result<UserScript, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;
    workspace.create_user_script(&source_code)
        .map_err(|e| e.to_string())
}

/// Updates an existing user script's source code.
#[tauri::command]
fn update_user_script(
    window: tauri::Window,
    state: State<'_, AppState>,
    script_id: String,
    source_code: String,
) -> std::result::Result<UserScript, String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;
    workspace.update_user_script(&script_id, &source_code)
        .map_err(|e| e.to_string())
}

/// Deletes a user script by ID.
#[tauri::command]
fn delete_user_script(
    window: tauri::Window,
    state: State<'_, AppState>,
    script_id: String,
) -> std::result::Result<(), String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;
    workspace.delete_user_script(&script_id)
        .map_err(|e| e.to_string())
}

/// Toggles the enabled state of a user script.
#[tauri::command]
fn toggle_user_script(
    window: tauri::Window,
    state: State<'_, AppState>,
    script_id: String,
    enabled: bool,
) -> std::result::Result<(), String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;
    workspace.toggle_user_script(&script_id, enabled)
        .map_err(|e| e.to_string())
}

/// Changes the load order of a user script.
#[tauri::command]
fn reorder_user_script(
    window: tauri::Window,
    state: State<'_, AppState>,
    script_id: String,
    new_load_order: i32,
) -> std::result::Result<(), String> {
    let label = window.label();
    let mut workspaces = state.workspaces.lock()
        .expect("Mutex poisoned");
    let workspace = workspaces.get_mut(label)
        .ok_or("No workspace open")?;
    workspace.reorder_user_script(&script_id, new_load_order)
        .map_err(|e| e.to_string())
}
```

**Step 2: Register commands in invoke_handler**

Add to the `tauri::generate_handler![]` macro (after `delete_note,` at line 566):

```rust
            list_user_scripts,
            get_user_script,
            create_user_script,
            update_user_script,
            delete_user_script,
            toggle_user_script,
            reorder_user_script,
```

**Step 3: Add menu item for script management**

In the menu file (need to check exact location — `krillnotes-desktop/src-tauri/src/menu.rs`), add a "Manage Scripts" entry under the Edit or Workspace menu. Also add the message mapping to `MENU_MESSAGES`:

```rust
    ("edit_manage_scripts", "Edit > Manage Scripts clicked"),
```

**Step 4: Build to verify compilation**

Run: `cd /Users/careck/Source/Krillnotes/krillnotes-desktop && cargo build -p krillnotes-desktop`
Expected: Compiles without errors

**Step 5: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: add Tauri commands for user script CRUD"
```

---

### Task 7: TypeScript types for user scripts

**Files:**
- Modify: `krillnotes-desktop/src/types.ts` (add UserScript interface)

**Step 1: Add UserScript interface**

Add at the end of `krillnotes-desktop/src/types.ts` (after `DeleteResult`):

```typescript

export interface UserScript {
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

**Step 2: Commit**

```bash
git add krillnotes-desktop/src/types.ts
git commit -m "feat: add UserScript TypeScript interface"
```

---

### Task 8: Install CodeMirror dependencies

**Files:**
- Modify: `krillnotes-desktop/package.json` (add CodeMirror deps)

**Step 1: Install packages**

Run: `cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npm install @codemirror/state @codemirror/view @codemirror/language @codemirror/lang-rust @codemirror/commands @codemirror/search`

We use `@codemirror/lang-rust` as the closest syntax mode for Rhai.

**Step 2: Verify build**

Run: `cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npm run build`
Expected: Build succeeds

**Step 3: Commit**

```bash
git add krillnotes-desktop/package.json krillnotes-desktop/package-lock.json
git commit -m "feat: install CodeMirror 6 dependencies"
```

---

### Task 9: CodeMirror editor wrapper component

**Files:**
- Create: `krillnotes-desktop/src/components/ScriptEditor.tsx`

**Step 1: Create the CodeMirror wrapper**

Create `krillnotes-desktop/src/components/ScriptEditor.tsx`:

```tsx
import { useRef, useEffect } from 'react';
import { EditorView, keymap, lineNumbers, highlightActiveLine, highlightActiveLineGutter } from '@codemirror/view';
import { EditorState } from '@codemirror/state';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { rust } from '@codemirror/lang-rust';
import { searchKeymap, highlightSelectionMatches } from '@codemirror/search';
import { syntaxHighlighting, defaultHighlightStyle, bracketMatching } from '@codemirror/language';

interface ScriptEditorProps {
  value: string;
  onChange: (value: string) => void;
}

function ScriptEditor({ value, onChange }: ScriptEditorProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;

  useEffect(() => {
    if (!containerRef.current) return;

    const state = EditorState.create({
      doc: value,
      extensions: [
        lineNumbers(),
        highlightActiveLine(),
        highlightActiveLineGutter(),
        history(),
        bracketMatching(),
        highlightSelectionMatches(),
        syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
        rust(),
        keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap]),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) {
            onChangeRef.current(update.state.doc.toString());
          }
        }),
        EditorView.theme({
          '&': {
            height: '100%',
            fontSize: '13px',
          },
          '.cm-scroller': {
            fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
          },
          '.cm-content': {
            padding: '8px 0',
          },
        }),
      ],
    });

    const view = new EditorView({
      state,
      parent: containerRef.current,
    });
    viewRef.current = view;

    return () => {
      view.destroy();
      viewRef.current = null;
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Update editor content when value changes externally (e.g. switching scripts)
  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    const currentContent = view.state.doc.toString();
    if (currentContent !== value) {
      view.dispatch({
        changes: { from: 0, to: currentContent.length, insert: value },
      });
    }
  }, [value]);

  return (
    <div
      ref={containerRef}
      className="border border-border rounded-md overflow-hidden h-full min-h-[300px]"
    />
  );
}

export default ScriptEditor;
```

**Step 2: Commit**

```bash
git add krillnotes-desktop/src/components/ScriptEditor.tsx
git commit -m "feat: add CodeMirror-based ScriptEditor component"
```

---

### Task 10: Script management dialog

**Files:**
- Create: `krillnotes-desktop/src/components/ScriptManagerDialog.tsx`

**Step 1: Create the dialog component**

Create `krillnotes-desktop/src/components/ScriptManagerDialog.tsx`:

```tsx
import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import ScriptEditor from './ScriptEditor';
import type { UserScript } from '../types';

interface ScriptManagerDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

const NEW_SCRIPT_TEMPLATE = `// @name: New Script
// @description:

schema("NewType", #{
    fields: [
        #{ name: "body", type: "textarea" },
    ]
});
`;

type View = 'list' | 'editor';

function ScriptManagerDialog({ isOpen, onClose }: ScriptManagerDialogProps) {
  const [view, setView] = useState<View>('list');
  const [scripts, setScripts] = useState<UserScript[]>([]);
  const [editingScript, setEditingScript] = useState<UserScript | null>(null);
  const [editorContent, setEditorContent] = useState('');
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);

  const loadScripts = useCallback(async () => {
    try {
      const result = await invoke<UserScript[]>('list_user_scripts');
      setScripts(result);
    } catch (err) {
      setError(`Failed to load scripts: ${err}`);
    }
  }, []);

  useEffect(() => {
    if (isOpen) {
      loadScripts();
      setView('list');
      setError('');
    }
  }, [isOpen, loadScripts]);

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        if (view === 'editor') {
          setView('list');
          setError('');
        } else {
          onClose();
        }
      }
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, view, onClose]);

  if (!isOpen) return null;

  const handleAdd = () => {
    setEditingScript(null);
    setEditorContent(NEW_SCRIPT_TEMPLATE);
    setError('');
    setView('editor');
  };

  const handleEdit = (script: UserScript) => {
    setEditingScript(script);
    setEditorContent(script.sourceCode);
    setError('');
    setView('editor');
  };

  const handleToggle = async (script: UserScript) => {
    try {
      await invoke('toggle_user_script', { scriptId: script.id, enabled: !script.enabled });
      await loadScripts();
    } catch (err) {
      setError(`Failed to toggle script: ${err}`);
    }
  };

  const handleSave = async () => {
    setSaving(true);
    setError('');
    try {
      if (editingScript) {
        await invoke<UserScript>('update_user_script', {
          scriptId: editingScript.id,
          sourceCode: editorContent,
        });
      } else {
        await invoke<UserScript>('create_user_script', {
          sourceCode: editorContent,
        });
      }
      await loadScripts();
      setView('list');
    } catch (err) {
      setError(`${err}`);
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async () => {
    if (!editingScript) return;
    const confirmed = window.confirm(
      "Deleting this script may remove schema definitions used by existing notes. " +
      "Their data will be preserved in the database but may not display correctly " +
      "until a compatible schema is re-registered. Delete anyway?"
    );
    if (!confirmed) return;
    try {
      await invoke('delete_user_script', { scriptId: editingScript.id });
      await loadScripts();
      setView('list');
      setError('');
    } catch (err) {
      setError(`Failed to delete: ${err}`);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-border rounded-lg w-[700px] max-h-[80vh] flex flex-col">
        {view === 'list' ? (
          <>
            {/* List View Header */}
            <div className="flex items-center justify-between p-4 border-b border-border">
              <h2 className="text-xl font-bold">User Scripts</h2>
              <button
                onClick={handleAdd}
                className="px-3 py-1.5 bg-primary text-primary-foreground rounded-md hover:bg-primary/90 text-sm"
              >
                + Add
              </button>
            </div>

            {/* Script List */}
            <div className="flex-1 overflow-y-auto p-4">
              {scripts.length === 0 ? (
                <p className="text-muted-foreground text-center py-8">
                  No user scripts yet. Click "+ Add" to create one.
                </p>
              ) : (
                <div className="space-y-2">
                  {scripts.map(script => (
                    <div
                      key={script.id}
                      className="flex items-center gap-3 p-3 border border-border rounded-md hover:bg-secondary/50"
                    >
                      <input
                        type="checkbox"
                        checked={script.enabled}
                        onChange={() => handleToggle(script)}
                        className="shrink-0"
                        title={script.enabled ? 'Disable script' : 'Enable script'}
                      />
                      <div className="flex-1 min-w-0">
                        <div className="font-medium truncate">
                          {script.name || '(unnamed)'}
                        </div>
                        {script.description && (
                          <div className="text-sm text-muted-foreground truncate">
                            {script.description}
                          </div>
                        )}
                      </div>
                      <span className="text-xs text-muted-foreground shrink-0">
                        #{script.loadOrder}
                      </span>
                      <button
                        onClick={() => handleEdit(script)}
                        className="px-2 py-1 text-sm border border-border rounded hover:bg-secondary"
                      >
                        Edit
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </div>

            {/* Error display */}
            {error && (
              <div className="px-4 pb-2">
                <div className="p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
                  {error}
                </div>
              </div>
            )}

            {/* Footer */}
            <div className="flex justify-end p-4 border-t border-border">
              <button
                onClick={onClose}
                className="px-4 py-2 border border-border rounded-md hover:bg-secondary"
              >
                Close
              </button>
            </div>
          </>
        ) : (
          <>
            {/* Editor View Header */}
            <div className="p-4 border-b border-border">
              <h2 className="text-xl font-bold">
                {editingScript ? `Edit: ${editingScript.name}` : 'New Script'}
              </h2>
            </div>

            {/* Editor */}
            <div className="flex-1 overflow-hidden p-4">
              <ScriptEditor value={editorContent} onChange={setEditorContent} />
            </div>

            {/* Error display */}
            {error && (
              <div className="px-4 pb-2">
                <div className="p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm whitespace-pre-wrap">
                  {error}
                </div>
              </div>
            )}

            {/* Footer */}
            <div className="flex justify-between p-4 border-t border-border">
              <div>
                {editingScript && (
                  <button
                    onClick={handleDelete}
                    className="px-4 py-2 bg-red-500 text-white rounded-md hover:bg-red-600"
                    disabled={saving}
                  >
                    Delete
                  </button>
                )}
              </div>
              <div className="flex gap-2">
                <button
                  onClick={() => { setView('list'); setError(''); }}
                  className="px-4 py-2 border border-border rounded-md hover:bg-secondary"
                  disabled={saving}
                >
                  Cancel
                </button>
                <button
                  onClick={handleSave}
                  className="px-4 py-2 bg-primary text-primary-foreground rounded-md hover:bg-primary/90"
                  disabled={saving}
                >
                  {saving ? 'Saving...' : 'Save'}
                </button>
              </div>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

export default ScriptManagerDialog;
```

**Step 2: Commit**

```bash
git add krillnotes-desktop/src/components/ScriptManagerDialog.tsx
git commit -m "feat: add ScriptManagerDialog component with list and editor views"
```

---

### Task 11: Wire up dialog to WorkspaceView and menu

**Files:**
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx` (add dialog + menu handler)
- Modify: `krillnotes-desktop/src-tauri/src/menu.rs` (add menu item)
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs` (add menu message mapping)

**Step 1: Add menu item in Rust**

In `krillnotes-desktop/src-tauri/src/menu.rs`, add a "Manage Scripts..." menu item under the Edit menu (exact location depends on the file structure — add after the existing edit menu items).

Add to the `MENU_MESSAGES` array in `lib.rs`:

```rust
    ("edit_manage_scripts", "Edit > Manage Scripts clicked"),
```

**Step 2: Add dialog to WorkspaceView**

In `krillnotes-desktop/src/components/WorkspaceView.tsx`:

Add import (after the existing imports):

```typescript
import ScriptManagerDialog from './ScriptManagerDialog';
```

Add state (after `requestEditMode` state, ~line 39):

```typescript
  const [showScriptManager, setShowScriptManager] = useState(false);
```

Add to the menu listener (after the `Edit > Add Note clicked` handler, ~line 54):

```typescript
        if (event.payload === 'Edit > Manage Scripts clicked') {
          setShowScriptManager(true);
        }
```

Add the dialog in the JSX return (before the closing `</div>`, after the DeleteConfirmDialog):

```tsx
        {/* Script Manager Dialog */}
        <ScriptManagerDialog
          isOpen={showScriptManager}
          onClose={() => setShowScriptManager(false)}
        />
```

**Step 3: Build and verify**

Run: `cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npm run build`
Expected: Build succeeds

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/components/WorkspaceView.tsx krillnotes-desktop/src-tauri/src/lib.rs krillnotes-desktop/src-tauri/src/menu.rs
git commit -m "feat: wire up ScriptManagerDialog to WorkspaceView and menu"
```

---

### Task 12: Refresh note types after script changes

**Files:**
- Modify: `krillnotes-desktop/src/components/ScriptManagerDialog.tsx` (emit event on save/delete/toggle)
- Modify: `krillnotes-desktop/src/components/WorkspaceView.tsx` (listen for refresh)

**Step 1: Add onScriptsChanged callback**

In `ScriptManagerDialog.tsx`, add a new prop:

```typescript
interface ScriptManagerDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onScriptsChanged?: () => void;
}
```

Call `onScriptsChanged?.()` after successful save, delete, and toggle operations.

**Step 2: Wire up in WorkspaceView**

Pass the callback to refresh notes after script changes:

```tsx
<ScriptManagerDialog
  isOpen={showScriptManager}
  onClose={() => setShowScriptManager(false)}
  onScriptsChanged={loadNotes}
/>
```

**Step 3: Build and verify**

Run: `cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npm run build`
Expected: Build succeeds

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/components/ScriptManagerDialog.tsx krillnotes-desktop/src/components/WorkspaceView.tsx
git commit -m "feat: refresh note types after user script changes"
```

---

### Task 13: Final verification

**Step 1: Run Rust tests**

Run: `cargo test -p krillnotes-core`
Expected: All tests pass

**Step 2: Run Clippy**

Run: `cargo clippy -p krillnotes-core -p krillnotes-desktop -- -D warnings`
Expected: No warnings

**Step 3: Build TypeScript**

Run: `cd /Users/careck/Source/Krillnotes/krillnotes-desktop && npm run build`
Expected: Build succeeds

**Step 4: Full Tauri build**

Run: `cd /Users/careck/Source/Krillnotes/krillnotes-desktop && cargo build -p krillnotes-desktop`
Expected: Build succeeds
