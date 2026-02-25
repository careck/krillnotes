# Script Collision Detection — Implementation Plan

**Date:** 2026-02-25
**Issue:** #6
**Branch:** `feat/script-collision-detection`
**Design doc:** `2026-02-25-script-collision-detection-design.md`

## Tasks

### 1. Add `ScriptError` type — `krillnotes-core/src/core/scripting/mod.rs`

Add near the top of the file:
```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScriptError {
    pub script_name: String,
    pub message: String,
}
```

### 2. Add `schema_owners` to `ScriptRegistry` — `scripting/mod.rs`

Add field to struct:
```rust
schema_owners: Arc<Mutex<HashMap<String, String>>>,
```

Initialize in `new()`:
```rust
let schema_owners: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));
```

Clear in `clear_all()`:
```rust
self.schema_owners.lock().unwrap().clear();
```

### 3. Collision detection in `schema()` host fn — `scripting/mod.rs` line 76

Before line 79 (`schemas_arc...insert`), add:
```rust
// Collision check: first script to register a name wins.
{
    let owners = schema_owners_arc.lock().unwrap();
    if let Some(owner) = owners.get(&name) {
        return Err(format!(
            "Schema '{}' is already defined by script '{}'. Schema names must be unique.",
            name, owner
        ).into());
    }
}
schema_owners_arc.lock().unwrap().insert(name.clone(), script_name.clone());
```

Requires cloning `schema_name_arc` into the closure as `schema_owners_arc` and `schema_name_arc`.

### 4. `reload_scripts()` collects errors — `workspace.rs`

Change signature to return errors:
```rust
fn reload_scripts(&mut self) -> Result<Vec<ScriptError>> {
    self.script_registry.clear_all();
    let scripts = self.list_user_scripts()?;
    let mut errors = Vec::new();
    for script in scripts.iter().filter(|s| s.enabled) {
        if let Err(e) = self.script_registry.load_script(&script.source_code, &script.name) {
            errors.push(ScriptError {
                script_name: script.name.clone(),
                message: e.to_string(),
            });
        }
    }
    Ok(errors)
}
```

### 5. Workspace mutation methods return errors

Change return types to include `Vec<ScriptError>`:
- `create_user_script` → `Result<(UserScript, Vec<ScriptError>)>`
- `update_user_script` → `Result<(UserScript, Vec<ScriptError>)>`
- `delete_user_script` → `Result<Vec<ScriptError>>`
- `toggle_user_script` → `Result<Vec<ScriptError>>`
- `reorder_user_script` → `Result<Vec<ScriptError>>`
- `reorder_all_user_scripts` → `Result<Vec<ScriptError>>`

### 6. Re-export `ScriptError` — `krillnotes-core/src/lib.rs`

Add to the `pub use core::scripting::{ ... }` block.

### 7. Update Tauri commands — `krillnotes-desktop/src-tauri/src/lib.rs`

New return types for Tauri commands:
- `create_user_script` → `Result<ScriptMutationResult<UserScript>, String>`
- `update_user_script` → `Result<ScriptMutationResult<UserScript>, String>`
- `delete_user_script` → `Result<Vec<ScriptError>, String>`
- `toggle_user_script` → `Result<Vec<ScriptError>, String>`
- `reorder_user_script` → `Result<Vec<ScriptError>, String>`

Where:
```rust
#[derive(Serialize)]
struct ScriptMutationResult<T: Serialize> {
    data: T,
    errors: Vec<ScriptError>,
}
```

### 8. Write a test — `krillnotes-core`

Add a test that:
1. Creates two scripts that both define `schema("TestCollision", ...)`
2. Calls `reload_scripts()`
3. Asserts that `Vec<ScriptError>` has exactly one error referencing the second script
4. Asserts the first script's schema is still registered

### 9. `cargo test` and `cargo build`

Verify all 3 existing tests pass and the project builds.
