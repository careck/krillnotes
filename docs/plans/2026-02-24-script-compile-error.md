# Script Compile Error on Save — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** When saving a Rhai script that has a syntax or runtime error, return an error and do not save the script to the database.

**Architecture:** Both `create_user_script` and `update_user_script` in `workspace.rs` call `load_script()` for validation. Currently the result is either ignored (update) or used only to set `enabled` (create). The fix changes both to return `Err` on compile failure, restoring registry state via `reload_scripts()` before returning.

**Tech Stack:** Rust, Rhai scripting engine, rusqlite, Tauri v2, React/TypeScript frontend (no frontend changes needed).

---

## Worktree Setup

Before any code changes, create an isolated worktree:

```bash
git -C /Users/careck/Source/Krillnotes worktree add .worktrees/feat/script-compile-error -b feat/script-compile-error
```

All implementation work happens in `/Users/careck/Source/Krillnotes/.worktrees/feat/script-compile-error/`.

---

### Task 1: Tests for `create_user_script` rejecting compile errors

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs` (tests module at the bottom, after line 1422)

**Step 1: Add the failing test for create**

In the `mod tests` block at the bottom of `workspace.rs`, add:

```rust
#[test]
fn test_create_user_script_rejects_compile_error() {
    let temp = NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(temp.path()).unwrap();

    // Clearly invalid Rhai: assignment with no identifier
    let bad_script = "// @name: Bad Script\n\nlet = 5;";
    let result = ws.create_user_script(bad_script);

    assert!(result.is_err(), "Should return error for invalid Rhai");
    // Confirm nothing was saved
    let scripts = ws.list_user_scripts().unwrap();
    assert_eq!(scripts.len(), 0, "No script should be saved on compile error");
}
```

**Step 2: Run the test to confirm it fails**

```bash
cargo test -p krillnotes-core test_create_user_script_rejects_compile_error -- --nocapture
```

Expected: FAIL — the test asserts `result.is_err()` but currently `create_user_script` returns `Ok` (saves as disabled).

**Step 3: Implement the fix in `create_user_script`**

In `krillnotes-core/src/core/workspace.rs` around line 1131, replace:

```rust
        // Try to compile and load the script (before opening transaction)
        let compile_ok = self.script_registry.load_script(source_code).is_ok();
```

with:

```rust
        // Try to compile and load the script — return error immediately on failure
        if let Err(e) = self.script_registry.load_script(source_code) {
            // Restore the registry to its pre-validation state
            self.reload_scripts()?;
            return Err(e);
        }
```

Then replace the INSERT statement at line 1141–1144, changing `compile_ok` to `true`:

```rust
        tx.execute(
            "INSERT INTO user_scripts (id, name, description, source_code, load_order, enabled, created_at, modified_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![id, fm.name, fm.description, source_code, load_order, true, now, now],
        )?;
```

Also update the operation log at line 1157, changing `enabled: compile_ok` to `enabled: true`:

```rust
            enabled: true,
```

Finally, remove the cleanup block at lines 1164–1167:

```rust
        if !compile_ok {
            // Reload all scripts to clean up any partial state
            self.reload_scripts()?;
        }
```

(Delete these 4 lines entirely.)

**Step 4: Run the test to confirm it passes**

```bash
cargo test -p krillnotes-core test_create_user_script_rejects_compile_error -- --nocapture
```

Expected: PASS

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "fix: reject create_user_script when Rhai compilation fails"
```

---

### Task 2: Tests for `update_user_script` rejecting compile errors

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs` (tests module)

**Step 1: Add the failing test for update**

In the `mod tests` block, add:

```rust
#[test]
fn test_update_user_script_rejects_compile_error() {
    let temp = NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(temp.path()).unwrap();

    // Create a valid script first
    let valid_script = "// @name: Good Script\n\n// valid empty body";
    let created = ws.create_user_script(valid_script).unwrap();

    // Attempt update with invalid Rhai
    let bad_script = "// @name: Good Script\n\nlet = 5;";
    let result = ws.update_user_script(&created.id, bad_script);

    assert!(result.is_err(), "Should return error for invalid Rhai on update");

    // Original source code must be preserved
    let scripts = ws.list_user_scripts().unwrap();
    assert_eq!(scripts.len(), 1);
    assert_eq!(
        scripts[0].source_code, valid_script,
        "Source code must be unchanged after failed update"
    );
}
```

**Step 2: Run the test to confirm it fails**

```bash
cargo test -p krillnotes-core test_update_user_script_rejects_compile_error -- --nocapture
```

Expected: FAIL — `update_user_script` currently saves without any compile check.

**Step 3: Implement the fix in `update_user_script`**

In `workspace.rs` around line 1181 (right after the `@name` validation, before `let now = ...`), add:

```rust
        // Try to compile and load the script — return error immediately on failure
        if let Err(e) = self.script_registry.load_script(source_code) {
            // Restore the registry to its pre-validation state
            self.reload_scripts()?;
            return Err(e);
        }
```

No other changes to `update_user_script` are needed — the existing `self.reload_scripts()?` at line 1217 already handles cleanup on the success path.

**Step 4: Run the test to confirm it passes**

```bash
cargo test -p krillnotes-core test_update_user_script_rejects_compile_error -- --nocapture
```

Expected: PASS

**Step 5: Run the full test suite**

```bash
cargo test -p krillnotes-core -- --nocapture
```

Expected: all tests pass.

**Step 6: Commit**

```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "fix: reject update_user_script when Rhai compilation fails"
```

---

### Task 3: Mark TODO and merge

**Step 1: Mark the TODO item as done**

In `/Users/careck/Source/Krillnotes/TODO.md`, find:

```
[ ] When saving a rhai script in the script editor it fails silently if the script does not compile due to an error. It would be better to show an error message in that case and not save the edited script at all.
```

Change `[ ]` to `✅ DONE!`:

```
✅ DONE! When saving a rhai script in the script editor it fails silently if the script does not compile due to an error. It would be better to show an error message in that case and not save the edited script at all.
```

**Step 2: Commit**

```bash
git add TODO.md
git commit -m "chore: mark script compile error task as done"
```

**Step 3: Invoke finishing-a-development-branch skill**

Use `superpowers:finishing-a-development-branch` to merge or PR the worktree branch.
