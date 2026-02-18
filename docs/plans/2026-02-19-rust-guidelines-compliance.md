# Rust Guidelines Compliance Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Bring all Rust source files into conformance with the MS Rust Guidelines, eliminating the `unsafe impl Send + Sync` bypass, deserialization panics, and missing documentation.

**Architecture:** Two passes — (1) structural safety fixes that change behaviour or types, (2) purely-additive documentation and minor lint cleanup. Safety first so docs describe correct code.

**Tech Stack:** Rust 2021 edition, Cargo workspaces, rhai 1.17 (with `sync` feature), rusqlite 0.31, mimalloc 0.1, thiserror 1.0.

---

### Task 1: Enable `rhai/sync`, remove `unsafe impl`

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Modify: `krillnotes-core/src/core/workspace.rs`

**Step 1: Add `sync` feature to the `rhai` workspace dependency**

In `Cargo.toml` at the root, replace:
```toml
rhai = "1.17"
```
with:
```toml
rhai = { version = "1.17", features = ["sync"] }
```

**Step 2: Delete the two `unsafe impl` lines from `workspace.rs`**

Remove lines 23–27 (the safety comment and both `unsafe impl` blocks):
```rust
// SAFETY: Workspace contains SchemaRegistry with rhai::Engine that has Rc pointers.
// However, we ensure thread-safety by only accessing Workspace through Mutex in AppState.
// Each Workspace instance is associated with a single window and protected by a Mutex.
unsafe impl Send for Workspace {}
unsafe impl Sync for Workspace {}
```

**Step 3: Build**

```bash
cargo build -p krillnotes-core
```
Expected: compiles with no errors. `Workspace: Send + Sync` now holds naturally because `rhai::Engine` is `Send + Sync` under the `sync` feature.

**Step 4: Run tests**

```bash
cargo test -p krillnotes-core
```
Expected: all existing tests pass.

**Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock krillnotes-core/src/core/workspace.rs
git commit -m "fix(core): enable rhai/sync to eliminate unsafe impl Send + Sync on Workspace"
```

---

### Task 2: Fix `.unwrap()` on `fields_json` deserialization (TDD)

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`

**Background:** Two methods (`get_note` and `list_all_notes`) call `.unwrap()` on `serde_json::from_str(...)` inside rusqlite closures. Because the closure returns `rusqlite::Result`, serde errors cannot be propagated with `?` inside the closure — the fix is to move JSON parsing out of the closure.

**Step 1: Write the failing test**

In `workspace.rs`, inside the `#[cfg(test)]` `mod tests` block, add:

```rust
#[test]
fn test_get_note_with_corrupt_fields_json_returns_error() {
    let temp = NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(temp.path()).unwrap();
    let root = ws.list_all_notes().unwrap()[0].clone();

    // Corrupt the stored JSON directly.
    ws.storage.connection_mut().execute(
        "UPDATE notes SET fields_json = 'not valid json' WHERE id = ?",
        [&root.id],
    ).unwrap();

    // Should return Err, not panic.
    let result = ws.get_note(&root.id);
    assert!(result.is_err(), "get_note should return Err for corrupt fields_json");
}

#[test]
fn test_list_all_notes_with_corrupt_fields_json_returns_error() {
    let temp = NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(temp.path()).unwrap();
    let root = ws.list_all_notes().unwrap()[0].clone();

    ws.storage.connection_mut().execute(
        "UPDATE notes SET fields_json = 'not valid json' WHERE id = ?",
        [&root.id],
    ).unwrap();

    let result = ws.list_all_notes();
    assert!(result.is_err(), "list_all_notes should return Err for corrupt fields_json");
}
```

**Step 2: Run tests to verify they fail**

```bash
cargo test -p krillnotes-core test_get_note_with_corrupt -- --nocapture
cargo test -p krillnotes-core test_list_all_notes_with_corrupt -- --nocapture
```
Expected: both tests FAIL — the current code panics, which Rust's test harness reports as a test failure.

**Step 3: Fix `get_note` — move JSON parsing outside the closure**

Replace the entire `get_note` method:

```rust
pub fn get_note(&self, note_id: &str) -> Result<Note> {
    let (id, title, node_type, parent_id, position,
         created_at, modified_at, created_by, modified_by,
         fields_json, is_expanded_int) =
        self.connection().query_row(
            "SELECT id, title, node_type, parent_id, position,
                    created_at, modified_at, created_by, modified_by,
                    fields_json, is_expanded
             FROM notes WHERE id = ?",
            [note_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, i64>(10)?,
                ))
            },
        )?;

    Ok(Note {
        id,
        title,
        node_type,
        parent_id,
        position: position as i32,
        created_at,
        modified_at,
        created_by,
        modified_by,
        fields: serde_json::from_str(&fields_json)?,
        is_expanded: is_expanded_int == 1,
    })
}
```

**Step 4: Fix `list_all_notes` — collect raw tuples, then parse JSON**

Replace the entire `list_all_notes` method:

```rust
pub fn list_all_notes(&self) -> Result<Vec<Note>> {
    let mut stmt = self.connection().prepare(
        "SELECT id, title, node_type, parent_id, position,
                created_at, modified_at, created_by, modified_by,
                fields_json, is_expanded
         FROM notes ORDER BY parent_id, position",
    )?;

    let raw_rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, i64>(10)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    raw_rows
        .into_iter()
        .map(|(id, title, node_type, parent_id, position,
               created_at, modified_at, created_by, modified_by,
               fields_json, is_expanded_int)| {
            Ok(Note {
                id,
                title,
                node_type,
                parent_id,
                position: position as i32,
                created_at,
                modified_at,
                created_by,
                modified_by,
                fields: serde_json::from_str(&fields_json)?,
                is_expanded: is_expanded_int == 1,
            })
        })
        .collect()
}
```

**Step 5: Run all tests**

```bash
cargo test -p krillnotes-core
```
Expected: all tests pass, including the two new ones.

**Step 6: Commit**

```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "fix(core): propagate fields_json deserialization errors instead of panicking"
```

---

### Task 3: Extract `SECONDS_PER_DAY`, fix lint attributes, remove dead `greet` command

**Files:**
- Modify: `krillnotes-core/src/core/operation_log.rs`
- Modify: `krillnotes-core/src/core/workspace.rs`
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`

**Step 1: Extract constant in `operation_log.rs`**

At the top of `operation_log.rs` (before `use` statements), add:
```rust
/// Seconds in one day; used to convert `retention_days` to a Unix timestamp cutoff.
const SECONDS_PER_DAY: i64 = 86_400;
```

Then on line 47, replace the magic literal:
```rust
// Before:
let cutoff = chrono::Utc::now().timestamp() - (retention_days as i64 * 86400);
// After:
let cutoff = chrono::Utc::now().timestamp() - (retention_days as i64 * SECONDS_PER_DAY);
```

**Step 2: Fix `#[allow(dead_code)]` in `workspace.rs`**

On line 14, check whether removing the attribute causes a warning:
```bash
# Temporarily remove the attribute and build
cargo build -p krillnotes-core 2>&1 | grep "dead_code"
```
- If no warning appears: delete the `#[allow(dead_code)]` line entirely (the fields are used).
- If a warning appears: replace with:
  ```rust
  #[expect(dead_code, reason = "fields accessed exclusively through inherent methods")]
  ```

**Step 3: Remove dead `greet` command from `lib.rs`**

In `krillnotes-desktop/src-tauri/src/lib.rs`, delete the entire `greet` function (lines 29–32):
```rust
// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}
```

Also remove `greet,` from the `invoke_handler!` macro on line 371.

**Step 4: Build both crates**

```bash
cargo build -p krillnotes-core
cargo build -p krillnotes-desktop
```
Expected: no errors, no warnings about `dead_code` or unused items.

**Step 5: Run all tests**

```bash
cargo test -p krillnotes-core
```

**Step 6: Commit**

```bash
git add krillnotes-core/src/core/operation_log.rs \
        krillnotes-core/src/core/workspace.rs \
        krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "fix(core): extract SECONDS_PER_DAY constant, remove dead greet command, fix lint attributes"
```

---

### Task 4: Add `mimalloc` global allocator

**Files:**
- Modify: `krillnotes-desktop/src-tauri/Cargo.toml`
- Modify: `krillnotes-desktop/src-tauri/src/main.rs`

**Step 1: Add the dependency**

In `krillnotes-desktop/src-tauri/Cargo.toml`, under `[dependencies]`, add:
```toml
mimalloc = { version = "0.1", default-features = false }
```

**Step 2: Register the global allocator**

In `krillnotes-desktop/src-tauri/src/main.rs`, add two lines after the existing `cfg_attr` attribute so the file reads:
```rust
// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn main() {
    krillnotes_desktop_lib::run()
}
```

**Step 3: Build**

```bash
cargo build -p krillnotes-desktop
```
Expected: compiles successfully. `mimalloc` is now the process-wide allocator.

**Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/Cargo.toml \
        krillnotes-desktop/src-tauri/src/main.rs \
        Cargo.lock
git commit -m "feat(desktop): set mimalloc as global allocator"
```

---

### Task 5: Document `krillnotes-core` crate root and `core/mod.rs`

**Files:**
- Modify: `krillnotes-core/src/lib.rs`
- Modify: `krillnotes-core/src/core/mod.rs`

**Step 1: Replace `lib.rs` with documented version**

```rust
//! Core library for Krillnotes — a local-first, hierarchical note-taking application.
//!
//! The primary entry point is [`Workspace`], which represents an open `.krillnotes`
//! database file. All document mutations go through `Workspace` methods.
//!
//! Types are re-exported from their respective sub-modules for convenience;
//! consumers should import from the crate root rather than the `core` module.

pub mod core;

// Re-export commonly used types.
#[doc(inline)]
pub use core::{
    device::get_device_id,
    error::{KrillnotesError, Result},
    note::{FieldValue, Note},
    operation::Operation,
    operation_log::{OperationLog, PurgeStrategy},
    scripting::{FieldDefinition, Schema, SchemaRegistry},
    storage::Storage,
    workspace::{AddPosition, Workspace},
};
```

**Step 2: Replace `core/mod.rs` with documented version**

```rust
//! Internal domain modules for the Krillnotes core library.
//!
//! All public types from these modules are re-exported at the crate root
//! with `#[doc(inline)]`; import from there in preference to this module.

pub mod device;
pub mod error;
pub mod note;
pub mod operation;
pub mod operation_log;
pub mod scripting;
pub mod storage;
pub mod workspace;

#[doc(inline)]
pub use device::get_device_id;
#[doc(inline)]
pub use error::{KrillnotesError, Result};
#[doc(inline)]
pub use note::{FieldValue, Note};
#[doc(inline)]
pub use operation::Operation;
#[doc(inline)]
pub use operation_log::{OperationLog, PurgeStrategy};
#[doc(inline)]
pub use scripting::{FieldDefinition, Schema, SchemaRegistry};
#[doc(inline)]
pub use storage::Storage;
#[doc(inline)]
pub use workspace::{AddPosition, Workspace};
```

**Step 3: Build**

```bash
cargo build -p krillnotes-core
```

**Step 4: Commit**

```bash
git add krillnotes-core/src/lib.rs krillnotes-core/src/core/mod.rs
git commit -m "docs(core): add crate-level and core/mod.rs documentation"
```

---

### Task 6: Document `error.rs` and `note.rs`

**Files:**
- Modify: `krillnotes-core/src/core/error.rs`
- Modify: `krillnotes-core/src/core/note.rs`

**Step 1: Replace `error.rs` with documented version**

```rust
//! Error types for the Krillnotes core library.

use thiserror::Error;

/// All errors that can occur within the Krillnotes core library.
#[derive(Debug, Error)]
pub enum KrillnotesError {
    /// A SQLite operation failed.
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// A Rhai script failed to parse or execute.
    #[error("Scripting error: {0}")]
    Scripting(String),

    /// A schema was requested that has not been registered.
    #[error("Schema not found: {0}")]
    SchemaNotFound(String),

    /// A note ID was requested that does not exist in the database.
    #[error("Note not found: {0}")]
    NoteNotFound(String),

    /// The opened file is not a valid Krillnotes workspace.
    #[error("Invalid workspace: {0}")]
    InvalidWorkspace(String),

    /// An I/O operation on the filesystem failed.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Stored note data could not be deserialized from JSON.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Convenience alias that pins the error type to [`KrillnotesError`].
pub type Result<T> = std::result::Result<T, KrillnotesError>;

impl KrillnotesError {
    /// Returns a short, human-readable message suitable for display to the end user.
    pub fn user_message(&self) -> String {
        match self {
            Self::Database(e) => format!("Failed to save: {}", e),
            Self::SchemaNotFound(name) => format!("Unknown note type: {}", name),
            Self::NoteNotFound(_) => "Note no longer exists".to_string(),
            Self::InvalidWorkspace(_) => "Could not open workspace file".to_string(),
            Self::Scripting(e) => format!("Script error: {}", e),
            Self::Io(e) => format!("File error: {}", e),
            Self::Json(e) => format!("Data format error: {}", e),
        }
    }
}
```

**Step 2: Replace `note.rs` with documented version**

```rust
//! Note data types for the Krillnotes workspace.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A typed value stored in a note's schema-defined fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FieldValue {
    /// A plain-text string.
    Text(String),
    /// A 64-bit floating-point number.
    Number(f64),
    /// A boolean flag.
    Boolean(bool),
}

/// A single node in the workspace hierarchy.
///
/// Notes form a tree via `parent_id`; siblings are ordered by `position`.
/// Each note has a `node_type` that maps to a [`crate::Schema`] and a set
/// of typed `fields` validated against that schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    /// Stable UUID identifying this note.
    pub id: String,
    /// Human-readable title shown in the tree view.
    pub title: String,
    /// Schema name governing this note's `fields` (e.g. `"TextNote"`).
    pub node_type: String,
    /// ID of the parent note, or `None` for root-level notes.
    pub parent_id: Option<String>,
    /// Zero-based sort order among siblings that share the same `parent_id`.
    pub position: i32,
    /// Unix timestamp (seconds) when this note was created.
    pub created_at: i64,
    /// Unix timestamp (seconds) of the most recent modification.
    pub modified_at: i64,
    /// Device ID that created this note.
    pub created_by: i64,
    /// Device ID that last modified this note.
    pub modified_by: i64,
    /// Schema-defined field values keyed by field name.
    pub fields: HashMap<String, FieldValue>,
    /// Whether this node is currently expanded in the tree UI.
    pub is_expanded: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_note() {
        let note = Note {
            id: "test-id".to_string(),
            title: "Test Note".to_string(),
            node_type: "TextNote".to_string(),
            parent_id: None,
            position: 0,
            created_at: 1234567890,
            modified_at: 1234567890,
            created_by: 0,
            modified_by: 0,
            fields: HashMap::new(),
            is_expanded: true,
        };

        assert_eq!(note.title, "Test Note");
        assert_eq!(note.node_type, "TextNote");
        assert!(note.parent_id.is_none());
    }

    #[test]
    fn test_field_value_text() {
        let value = FieldValue::Text("Hello".to_string());
        match value {
            FieldValue::Text(s) => assert_eq!(s, "Hello"),
            _ => panic!("Wrong variant"),
        }
    }
}
```

**Step 3: Build and test**

```bash
cargo build -p krillnotes-core && cargo test -p krillnotes-core
```

**Step 4: Commit**

```bash
git add krillnotes-core/src/core/error.rs krillnotes-core/src/core/note.rs
git commit -m "docs(core): add documentation to error and note modules"
```

---

### Task 7: Document `operation.rs` and `operation_log.rs`

**Files:**
- Modify: `krillnotes-core/src/core/operation.rs`
- Modify: `krillnotes-core/src/core/operation_log.rs`

**Step 1: Replace `operation.rs` with documented version**

```rust
//! CRDT-style operation types for the Krillnotes operation log.

use crate::FieldValue;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A single document mutation recorded in the workspace operation log.
///
/// Operations capture the full intent of each change so they can be
/// replayed, merged, or synced across devices in a future sync phase.
/// Every variant carries a stable `operation_id`, a wall-clock `timestamp`,
/// and the `device_id` of the originating machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Operation {
    /// A new note was inserted into the workspace hierarchy.
    CreateNote {
        /// Stable UUID for this operation.
        operation_id: String,
        /// Unix timestamp (seconds) when the operation was created.
        timestamp: i64,
        /// ID of the device that performed this operation.
        device_id: String,
        /// ID assigned to the new note.
        note_id: String,
        /// Parent note ID, or `None` for a root note.
        parent_id: Option<String>,
        /// Zero-based position among siblings.
        position: i32,
        /// Schema type of the new note.
        node_type: String,
        /// Initial title of the new note.
        title: String,
        /// Initial field values of the new note.
        fields: HashMap<String, FieldValue>,
        /// Device ID logged as the creator.
        created_by: i64,
    },
    /// A single schema field on an existing note was updated.
    UpdateField {
        /// Stable UUID for this operation.
        operation_id: String,
        /// Unix timestamp (seconds) when the operation was created.
        timestamp: i64,
        /// ID of the device that performed this operation.
        device_id: String,
        /// ID of the note whose field was updated.
        note_id: String,
        /// Name of the field that changed.
        field: String,
        /// New value for the field.
        value: FieldValue,
        /// Device ID logged as the modifier.
        modified_by: i64,
    },
    /// A note (and all its descendants) was deleted.
    DeleteNote {
        /// Stable UUID for this operation.
        operation_id: String,
        /// Unix timestamp (seconds) when the operation was created.
        timestamp: i64,
        /// ID of the device that performed this operation.
        device_id: String,
        /// ID of the deleted note.
        note_id: String,
    },
    /// A note was relocated to a new parent or position.
    MoveNote {
        /// Stable UUID for this operation.
        operation_id: String,
        /// Unix timestamp (seconds) when the operation was created.
        timestamp: i64,
        /// ID of the device that performed this operation.
        device_id: String,
        /// ID of the note that was moved.
        note_id: String,
        /// New parent note ID, or `None` to move to root level.
        new_parent_id: Option<String>,
        /// New zero-based position among siblings.
        new_position: i32,
    },
}

impl Operation {
    /// Returns the stable identifier for this operation.
    pub fn operation_id(&self) -> &str {
        match self {
            Self::CreateNote { operation_id, .. } => operation_id,
            Self::UpdateField { operation_id, .. } => operation_id,
            Self::DeleteNote { operation_id, .. } => operation_id,
            Self::MoveNote { operation_id, .. } => operation_id,
        }
    }

    /// Returns the wall-clock Unix timestamp (seconds) when this operation was created.
    pub fn timestamp(&self) -> i64 {
        match self {
            Self::CreateNote { timestamp, .. } => *timestamp,
            Self::UpdateField { timestamp, .. } => *timestamp,
            Self::DeleteNote { timestamp, .. } => *timestamp,
            Self::MoveNote { timestamp, .. } => *timestamp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operation_serialization() {
        let op = Operation::CreateNote {
            operation_id: "op-123".to_string(),
            timestamp: 1234567890,
            device_id: "dev-1".to_string(),
            note_id: "note-1".to_string(),
            parent_id: None,
            position: 0,
            node_type: "TextNote".to_string(),
            title: "Test".to_string(),
            fields: HashMap::new(),
            created_by: 0,
        };

        let json = serde_json::to_string(&op).unwrap();
        let deserialized: Operation = serde_json::from_str(&json).unwrap();

        assert_eq!(op.operation_id(), deserialized.operation_id());
    }
}
```

**Step 2: Replace `operation_log.rs` with documented version**

```rust
//! Durable operation log and purge strategies for the Krillnotes workspace.

use crate::{Operation, Result};
use rusqlite::Transaction;

/// Seconds in one day; used to convert `retention_days` to a Unix timestamp cutoff.
const SECONDS_PER_DAY: i64 = 86_400;

/// Controls which old operations are removed from the log.
pub enum PurgeStrategy {
    /// Retain only the most recent `keep_last` operations.
    ///
    /// Used when sync is disabled and the log is local-only.
    LocalOnly { keep_last: usize },
    /// Retain synced operations for up to `retention_days` before removing them.
    ///
    /// Used when sync is enabled and remote peers may still need older operations.
    WithSync { retention_days: u32 },
}

/// Records document mutations to the `operations` table and purges stale entries.
pub struct OperationLog {
    strategy: PurgeStrategy,
}

impl OperationLog {
    /// Creates a new `OperationLog` with the given purge strategy.
    pub fn new(strategy: PurgeStrategy) -> Self {
        Self { strategy }
    }

    /// Serialises `op` and appends it to the `operations` table within `tx`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] if the INSERT fails, or
    /// [`crate::KrillnotesError::Json`] if `op` cannot be serialised.
    pub fn log(&self, tx: &Transaction, op: &Operation) -> Result<()> {
        let op_json = serde_json::to_string(op)?;

        tx.execute(
            "INSERT INTO operations (operation_id, timestamp, device_id, operation_type, operation_data, synced)
             VALUES (?, ?, ?, ?, ?, 0)",
            rusqlite::params![
                op.operation_id(),
                op.timestamp(),
                self.extract_device_id(op),
                self.operation_type_name(op),
                op_json,
            ],
        )?;

        Ok(())
    }

    /// Deletes old operations from the log according to the purge strategy.
    ///
    /// Call this after every [`log`](Self::log) call to keep the table bounded in size.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] if the DELETE fails.
    pub fn purge_if_needed(&self, tx: &Transaction) -> Result<()> {
        match self.strategy {
            PurgeStrategy::LocalOnly { keep_last } => {
                tx.execute(
                    "DELETE FROM operations WHERE id NOT IN (
                        SELECT id FROM operations ORDER BY id DESC LIMIT ?
                    )",
                    [keep_last],
                )?;
            }
            PurgeStrategy::WithSync { retention_days } => {
                let cutoff = chrono::Utc::now().timestamp()
                    - (retention_days as i64 * SECONDS_PER_DAY);
                tx.execute(
                    "DELETE FROM operations WHERE synced = 1 AND timestamp < ?",
                    [cutoff],
                )?;
            }
        }
        Ok(())
    }

    fn extract_device_id<'a>(&self, op: &'a Operation) -> &'a str {
        match op {
            Operation::CreateNote { device_id, .. } => device_id,
            Operation::UpdateField { device_id, .. } => device_id,
            Operation::DeleteNote { device_id, .. } => device_id,
            Operation::MoveNote { device_id, .. } => device_id,
        }
    }

    fn operation_type_name(&self, op: &Operation) -> &str {
        match op {
            Operation::CreateNote { .. } => "CreateNote",
            Operation::UpdateField { .. } => "UpdateField",
            Operation::DeleteNote { .. } => "DeleteNote",
            Operation::MoveNote { .. } => "MoveNote",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Storage;
    use std::collections::HashMap;
    use tempfile::NamedTempFile;

    #[test]
    fn test_log_and_purge() {
        let temp = NamedTempFile::new().unwrap();
        let mut storage = Storage::create(temp.path()).unwrap();
        let log = OperationLog::new(PurgeStrategy::LocalOnly { keep_last: 5 });

        let tx = storage.connection_mut().transaction().unwrap();

        for i in 0..10 {
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

        log.purge_if_needed(&tx).unwrap();
        tx.commit().unwrap();

        let count: i64 = storage
            .connection()
            .query_row("SELECT COUNT(*) FROM operations", [], |row| row.get(0))
            .unwrap();

        assert_eq!(count, 5);
    }
}
```

**Step 3: Build and test**

```bash
cargo build -p krillnotes-core && cargo test -p krillnotes-core
```

**Step 4: Commit**

```bash
git add krillnotes-core/src/core/operation.rs krillnotes-core/src/core/operation_log.rs
git commit -m "docs(core): add documentation to operation and operation_log modules"
```

---

### Task 8: Document `device.rs` and `scripting.rs`

**Files:**
- Modify: `krillnotes-core/src/core/device.rs`
- Modify: `krillnotes-core/src/core/scripting.rs`

**Step 1: Replace `device.rs` with documented version**

```rust
//! Stable hardware-based device identity for Krillnotes.

use crate::{KrillnotesError, Result};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Returns a stable device identifier derived from the machine's primary MAC address.
///
/// The MAC address bytes are hashed to produce an opaque identifier of the form
/// `device-<16 hex digits>`. The same hardware always yields the same identifier
/// across process restarts.
///
/// # Errors
///
/// Returns [`KrillnotesError::InvalidWorkspace`] if the system has no network
/// interfaces or the MAC address cannot be read.
pub fn get_device_id() -> Result<String> {
    match mac_address::get_mac_address() {
        Ok(Some(mac)) => {
            let mut hasher = DefaultHasher::new();
            mac.bytes().hash(&mut hasher);
            let hash = hasher.finish();
            Ok(format!("device-{:016x}", hash))
        }
        Ok(None) => Err(KrillnotesError::InvalidWorkspace(
            "Could not determine device MAC address".to_string(),
        )),
        Err(e) => Err(KrillnotesError::InvalidWorkspace(format!(
            "Failed to get MAC address: {}",
            e
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_id_is_stable() {
        let id1 = get_device_id();
        let id2 = get_device_id();

        match (id1, id2) {
            (Ok(id1), Ok(id2)) => {
                assert_eq!(id1, id2, "Device ID should be stable");
                assert!(id1.starts_with("device-"), "Device ID should have correct format");
            }
            (Err(_), Err(_)) => {
                // Both failed — acceptable in environments without network interfaces.
            }
            _ => panic!("Device ID generation is inconsistent"),
        }
    }
}
```

**Step 2: Replace `scripting.rs` with documented version**

```rust
//! Rhai-based schema registry for Krillnotes note types.
//!
//! Schemas are defined in `.rhai` scripts and loaded at workspace startup.
//! The [`SchemaRegistry`] keeps the Rhai [`Engine`] alive so that future
//! scripted views, commands, and action hooks can be evaluated at runtime.

use crate::{FieldValue, KrillnotesError, Result};
use rhai::{Engine, Map};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Describes a single typed field within a note schema.
#[derive(Debug, Clone)]
pub struct FieldDefinition {
    /// The field's unique name within its schema.
    pub name: String,
    /// The field type: `"text"`, `"number"`, or `"boolean"`.
    pub field_type: String,
    /// Whether the field must carry a non-default value before the note is saved.
    pub required: bool,
}

/// A parsed note-type schema containing an ordered list of field definitions.
#[derive(Debug, Clone)]
pub struct Schema {
    /// The unique name of this schema (e.g. `"TextNote"`).
    pub name: String,
    /// Ordered field definitions that make up this schema.
    pub fields: Vec<FieldDefinition>,
}

impl Schema {
    /// Returns a map of field names to their zero-value defaults.
    ///
    /// Text fields default to `""`, numbers to `0.0`, booleans to `false`.
    pub fn default_fields(&self) -> HashMap<String, FieldValue> {
        let mut fields = HashMap::new();
        for field_def in &self.fields {
            let default_value = match field_def.field_type.as_str() {
                "text" => FieldValue::Text(String::new()),
                "number" => FieldValue::Number(0.0),
                "boolean" => FieldValue::Boolean(false),
                _ => FieldValue::Text(String::new()),
            };
            fields.insert(field_def.name.clone(), default_value);
        }
        fields
    }
}

/// Registry of all note-type schemas loaded from Rhai scripts.
///
/// The Rhai [`Engine`] is kept alive as a field so that future scripted
/// views, commands, and action hooks can be evaluated at runtime without
/// reconstructing the engine from scratch.
#[derive(Debug)]
pub struct SchemaRegistry {
    engine: Engine,
    schemas: Arc<Mutex<HashMap<String, Schema>>>,
}

impl SchemaRegistry {
    /// Creates a new registry and loads the built-in system schemas.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError::Scripting`] if the bundled system script
    /// fails to parse or if any `schema(...)` call within it is malformed.
    pub fn new() -> Result<Self> {
        let mut engine = Engine::new();
        let schemas = Arc::new(Mutex::new(HashMap::new()));

        let schemas_clone = Arc::clone(&schemas);
        engine.register_fn("schema", move |name: String, def: Map| {
            let schema = Self::parse_schema(&name, &def).unwrap();
            schemas_clone.lock().unwrap().insert(name, schema);
        });

        let mut registry = Self { engine, schemas };
        registry.load_script(include_str!("../system_scripts/text_note.rhai"))?;

        Ok(registry)
    }

    /// Evaluates `script` and registers any schemas it defines via `schema(...)` calls.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError::Scripting`] if the script fails to evaluate.
    pub fn load_script(&mut self, script: &str) -> Result<()> {
        self.engine
            .eval::<()>(script)
            .map_err(|e| KrillnotesError::Scripting(e.to_string()))?;
        Ok(())
    }

    /// Returns the schema registered under `name`.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError::SchemaNotFound`] if no schema with that
    /// name has been registered.
    pub fn get_schema(&self, name: &str) -> Result<Schema> {
        self.schemas
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or_else(|| KrillnotesError::SchemaNotFound(name.to_string()))
    }

    /// Returns the names of all currently registered schemas.
    pub fn list_schemas(&self) -> Vec<String> {
        self.schemas.lock().unwrap().keys().cloned().collect()
    }

    /// Returns the names of all currently registered schemas.
    ///
    /// This is an alias for [`list_schemas`](Self::list_schemas).
    pub fn list_types(&self) -> Result<Vec<String>> {
        Ok(self.schemas.lock().unwrap().keys().cloned().collect())
    }

    fn parse_schema(name: &str, def: &Map) -> Result<Schema> {
        let fields_array = def
            .get("fields")
            .and_then(|v| v.clone().try_cast::<rhai::Array>())
            .ok_or_else(|| KrillnotesError::Scripting("Missing 'fields' array".to_string()))?;

        let mut fields = Vec::new();
        for field_item in fields_array {
            let field_map = field_item
                .try_cast::<Map>()
                .ok_or_else(|| KrillnotesError::Scripting("Field must be a map".to_string()))?;

            let field_name = field_map
                .get("name")
                .and_then(|v| v.clone().try_cast::<String>())
                .ok_or_else(|| KrillnotesError::Scripting("Field missing 'name'".to_string()))?;

            let field_type = field_map
                .get("type")
                .and_then(|v| v.clone().try_cast::<String>())
                .ok_or_else(|| KrillnotesError::Scripting("Field missing 'type'".to_string()))?;

            let required = field_map
                .get("required")
                .and_then(|v| v.clone().try_cast::<bool>())
                .unwrap_or(false);

            fields.push(FieldDefinition {
                name: field_name,
                field_type,
                required,
            });
        }

        Ok(Schema {
            name: name.to_string(),
            fields,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_registration() {
        let mut registry = SchemaRegistry::new().unwrap();

        let script = r#"
            schema("TestNote", #{
                fields: [
                    #{ name: "body", type: "text", required: false },
                    #{ name: "count", type: "number", required: false },
                ]
            });
        "#;

        registry.load_script(script).unwrap();

        let schema = registry.get_schema("TestNote").unwrap();
        assert_eq!(schema.name, "TestNote");
        assert_eq!(schema.fields.len(), 2);
        assert_eq!(schema.fields[0].name, "body");
        assert_eq!(schema.fields[0].field_type, "text");
    }

    #[test]
    fn test_default_fields() {
        let schema = Schema {
            name: "TestNote".to_string(),
            fields: vec![
                FieldDefinition {
                    name: "body".to_string(),
                    field_type: "text".to_string(),
                    required: false,
                },
                FieldDefinition {
                    name: "count".to_string(),
                    field_type: "number".to_string(),
                    required: false,
                },
            ],
        };

        let defaults = schema.default_fields();
        assert_eq!(defaults.len(), 2);
        assert!(matches!(defaults.get("body"), Some(FieldValue::Text(_))));
        assert!(matches!(defaults.get("count"), Some(FieldValue::Number(_))));
    }

    #[test]
    fn test_text_note_schema_loaded() {
        let registry = SchemaRegistry::new().unwrap();
        let schema = registry.get_schema("TextNote").unwrap();

        assert_eq!(schema.name, "TextNote");
        assert_eq!(schema.fields.len(), 1);
        assert_eq!(schema.fields[0].name, "body");
        assert_eq!(schema.fields[0].field_type, "text");
    }
}
```

**Step 3: Build and test**

```bash
cargo build -p krillnotes-core && cargo test -p krillnotes-core
```

**Step 4: Commit**

```bash
git add krillnotes-core/src/core/device.rs krillnotes-core/src/core/scripting.rs
git commit -m "docs(core): add documentation to device and scripting modules"
```

---

### Task 9: Document `storage.rs`

**Files:**
- Modify: `krillnotes-core/src/core/storage.rs`

**Step 1: Replace `storage.rs` with documented version**

```rust
//! SQLite connection management and schema migration for Krillnotes workspaces.

use crate::Result;
use rusqlite::Connection;
use std::path::Path;

/// Manages the SQLite connection for a Krillnotes workspace file.
///
/// `Storage` validates the database structure on open and applies
/// any pending column-level migrations before handing off the connection.
pub struct Storage {
    conn: Connection,
}

impl Storage {
    /// Creates a new workspace database at `path` and initialises the schema.
    ///
    /// The schema is loaded from the bundled `schema.sql` file. If a file
    /// already exists at `path` it will be opened and the schema re-applied
    /// (SQLite `CREATE TABLE IF NOT EXISTS` semantics).
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] if the file cannot be
    /// created or the schema SQL fails to execute.
    pub fn create<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(include_str!("schema.sql"))?;
        Ok(Self { conn })
    }

    /// Opens an existing workspace database at `path` and runs pending migrations.
    ///
    /// Validates that the file contains all three required tables (`notes`,
    /// `operations`, `workspace_meta`) before returning. Currently performs
    /// one migration: adds the `is_expanded` column to `notes` if absent.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::InvalidWorkspace`] if the file does not
    /// contain the expected tables (i.e. it is not a Krillnotes database), or
    /// [`crate::KrillnotesError::Database`] for any other SQLite error.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;

        // All three tables must exist; any other count means this is not a
        // valid Krillnotes workspace.
        let table_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type='table'
             AND name IN ('notes', 'operations', 'workspace_meta')",
            [],
            |row| row.get(0)
        )?;

        if table_count != 3 {
            return Err(crate::KrillnotesError::InvalidWorkspace(
                "Not a valid Krillnotes database".to_string()
            ));
        }

        // Migration: add is_expanded column if it was created before this column existed.
        let column_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('notes') WHERE name='is_expanded'",
                [],
                |row| row.get::<_, i64>(0).map(|count| count > 0)
            )?;

        if !column_exists {
            conn.execute(
                "ALTER TABLE notes ADD COLUMN is_expanded INTEGER DEFAULT 1",
                []
            )?;
        }

        Ok(Self { conn })
    }

    /// Returns a shared reference to the underlying SQLite connection.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Returns an exclusive reference to the underlying SQLite connection.
    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_create_storage() {
        let temp = NamedTempFile::new().unwrap();
        let storage = Storage::create(temp.path()).unwrap();

        let tables: Vec<String> = storage
            .connection()
            .prepare("SELECT name FROM sqlite_master WHERE type='table'")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();

        assert!(tables.contains(&"notes".to_string()));
        assert!(tables.contains(&"operations".to_string()));
        assert!(tables.contains(&"workspace_meta".to_string()));
    }

    #[test]
    fn test_open_existing_storage() {
        let temp = NamedTempFile::new().unwrap();
        Storage::create(temp.path()).unwrap();
        let storage = Storage::open(temp.path()).unwrap();

        let tables: Vec<String> = storage
            .connection()
            .prepare("SELECT name FROM sqlite_master WHERE type='table'")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();

        assert!(tables.contains(&"notes".to_string()));
        assert!(tables.contains(&"operations".to_string()));
        assert!(tables.contains(&"workspace_meta".to_string()));
    }

    #[test]
    fn test_open_invalid_database() {
        let temp = NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), "not a database").unwrap();
        let result = Storage::open(temp.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_migration_adds_is_expanded_column() {
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
                    fields_json TEXT NOT NULL
                )",
                [],
            ).unwrap();
            conn.execute("CREATE TABLE operations (id INTEGER PRIMARY KEY)", []).unwrap();
            conn.execute("CREATE TABLE workspace_meta (key TEXT PRIMARY KEY, value TEXT)", []).unwrap();
        }

        let storage = Storage::open(temp.path()).unwrap();

        let column_exists: bool = storage
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('notes') WHERE name='is_expanded'",
                [],
                |row| row.get::<_, i64>(0).map(|count| count > 0)
            )
            .unwrap();

        assert!(column_exists, "is_expanded column should exist after migration");
    }
}
```

**Step 2: Build and test**

```bash
cargo build -p krillnotes-core && cargo test -p krillnotes-core
```

**Step 3: Commit**

```bash
git add krillnotes-core/src/core/storage.rs
git commit -m "docs(core): add documentation to storage module"
```

---

### Task 10: Document `workspace.rs`

**Files:**
- Modify: `krillnotes-core/src/core/workspace.rs`

This file is large. Add the items below in order, leaving the existing function bodies completely unchanged.

**Step 1: Add module doc and `AddPosition` docs**

At the very top of the file (before `use` statements), add:
```rust
//! High-level workspace operations over a Krillnotes SQLite database.
```

Immediately before `pub enum AddPosition`, add:
```rust
/// Controls where a new note is inserted relative to the currently selected note.
```

Before the `AsChild` variant, add:
```rust
    /// Insert as the first child of the selected note.
```

Before the `AsSibling` variant, add:
```rust
    /// Insert immediately after the selected note within the same parent.
```

**Step 2: Add `Workspace` struct doc**

Replace the `#[allow(dead_code)]` line and `pub struct Workspace` declaration with:
```rust
/// An open Krillnotes workspace backed by a SQLite database.
///
/// `Workspace` is the primary interface for all document mutations. It combines
/// a [`Storage`] connection, a [`SchemaRegistry`] for note-type validation,
/// and an [`OperationLog`] for durable change history.
///
/// Each instance is bound to a single window and protected by a `Mutex` in
/// [`crate::krillnotes_desktop_lib::AppState`].
pub struct Workspace {
```

(Remove `#[allow(dead_code)]` entirely — the fields are all accessed through inherent methods.)

**Step 3: Add `impl Workspace` method docs**

Before each `pub fn`, insert the doc comments listed below. Do not change any function body.

```rust
    /// Creates a new workspace database at `path`, initialises the schema, and inserts
    /// a root note named after the file (e.g. `"My Notes"` for `my-notes.krillnotes`).
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] for any SQLite failure, or
    /// [`crate::KrillnotesError::InvalidWorkspace`] if the device ID cannot be obtained.
    pub fn create<P: AsRef<Path>>(path: P) -> Result<Self> {
```

```rust
    /// Opens an existing workspace database at `path` and reads stored metadata.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::InvalidWorkspace`] if the file is not a
    /// valid Krillnotes database, or [`crate::KrillnotesError::Database`] for
    /// any SQLite failure.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
```

```rust
    /// Returns a reference to the schema registry for this workspace.
    pub fn registry(&self) -> &SchemaRegistry {
```

```rust
    /// Returns the underlying SQLite connection.
    pub fn connection(&self) -> &Connection {
```

```rust
    /// Fetches a single note by ID.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] if the note is not found or
    /// if `fields_json` cannot be deserialised.
    pub fn get_note(&self, note_id: &str) -> Result<Note> {
```

```rust
    /// Creates a new note of `note_type` relative to `selected_note_id`.
    ///
    /// The new note is inserted as a child or sibling according to `position`.
    /// Sibling insertion bumps the positions of all following siblings to make room.
    ///
    /// Returns the ID of the newly created note.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::SchemaNotFound`] if `note_type` is unknown,
    /// or [`crate::KrillnotesError::Database`] for any SQLite failure.
    pub fn create_note(
```

```rust
    /// Creates a new root-level note of `node_type` with no parent.
    ///
    /// Returns the ID of the newly created note.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::SchemaNotFound`] if `node_type` is unknown,
    /// or [`crate::KrillnotesError::Database`] for any SQLite failure.
    pub fn create_note_root(&mut self, node_type: &str) -> Result<String> {
```

```rust
    /// Updates the title of `note_id` and logs an `UpdateField` operation.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] if the note is not found or
    /// the UPDATE fails.
    pub fn update_note_title(&mut self, note_id: &str, new_title: String) -> Result<()> {
```

```rust
    /// Returns all notes in the workspace, ordered by `parent_id` then `position`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] for any SQLite failure, or
    /// [`crate::KrillnotesError::Json`] if any row's `fields_json` is corrupt.
    pub fn list_all_notes(&self) -> Result<Vec<Note>> {
```

```rust
    /// Returns the names of all registered note types (schema names).
    ///
    /// # Errors
    ///
    /// This method currently does not fail, but returns `Result` for consistency.
    pub fn list_node_types(&self) -> Result<Vec<String>> {
```

```rust
    /// Toggles the `is_expanded` flag of `note_id` in the database.
    ///
    /// This is a UI-state mutation and is intentionally excluded from the
    /// operation log — expansion state is per-device and should not sync.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] if the note is not found.
    pub fn toggle_note_expansion(&mut self, note_id: &str) -> Result<()> {
```

```rust
    /// Persists the selected note ID to `workspace_meta`.
    ///
    /// Pass `None` to clear the selection. Like expansion state, selection is
    /// per-device UI state and is not written to the operation log.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] for any SQLite failure.
    pub fn set_selected_note(&mut self, note_id: Option<&str>) -> Result<()> {
```

```rust
    /// Returns the persisted selected note ID, or `None` if no selection is stored.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] for any SQLite error other
    /// than "no rows returned".
    pub fn get_selected_note(&self) -> Result<Option<String>> {
```

**Step 4: Build and test**

```bash
cargo build -p krillnotes-core && cargo test -p krillnotes-core
```

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/workspace.rs
git commit -m "docs(core): add documentation to workspace module"
```

---

### Task 11: Document `krillnotes-desktop` `lib.rs` and `menu.rs`

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs`
- Modify: `krillnotes-desktop/src-tauri/src/menu.rs`

**Step 1: Add module doc and item docs to `lib.rs`**

At the very top of `lib.rs` (before `pub mod menu;`), add:
```rust
//! Tauri application backend for Krillnotes.
//!
//! Exposes Tauri commands that the React frontend calls via `invoke()`.
//! Each command is scoped to the calling window's workspace via
//! [`AppState`] and the window label.
```

Before `pub struct AppState`, add:
```rust
/// Per-process state shared across all workspace windows.
///
/// Each window label maps to its open [`Workspace`] and the filesystem path
/// of its database file. Both maps are protected by a [`Mutex`] since Tauri
/// may call commands from multiple threads.
```

Before `pub struct WorkspaceInfo`, add:
```rust
/// Serialisable summary of an open workspace, returned to the frontend.
```

Before each `pub` field of `WorkspaceInfo`, add a field doc:
```rust
    /// File name without extension (used as the window title).
    pub filename: String,
    /// Absolute filesystem path to the `.krillnotes` database file.
    pub path: String,
    /// Total number of notes in the workspace.
    pub note_count: usize,
    /// ID of the note selected when the workspace was last saved, if any.
    pub selected_note_id: Option<String>,
```

Before each `#[tauri::command]` function, add a one-line doc:
```rust
/// Creates a new workspace database at `path` and opens it in a new window.
async fn create_workspace(...)

/// Opens an existing workspace database at `path` in a new window.
async fn open_workspace(...)

/// Returns the [`WorkspaceInfo`] for the calling window's workspace.
fn get_workspace_info(...)

/// Returns all notes in the calling window's workspace.
fn list_notes(...)

/// Returns the registered note types for the calling window's workspace.
fn get_node_types(...)

/// Toggles the expansion state of `note_id` in the calling window's workspace.
fn toggle_note_expansion(...)

/// Persists the selected note ID for the calling window's workspace.
fn set_selected_note(...)

/// Creates a new note and returns it; uses root insertion when `parent_id` is `None`.
async fn create_note_with_type(...)
```

**Step 2: Add module doc and function doc to `menu.rs`**

At the top of `menu.rs`, add:
```rust
//! Application menu construction for Krillnotes.
```

Before `pub fn build_menu`, add:
```rust
/// Builds the application menu with File, Edit, View, and Help submenus.
///
/// # Errors
///
/// Returns [`tauri::Error`] if any menu item or submenu fails to build.
```

**Step 3: Build**

```bash
cargo build -p krillnotes-desktop
```

**Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs \
        krillnotes-desktop/src-tauri/src/menu.rs
git commit -m "docs(desktop): add documentation to lib and menu modules"
```

---

### Task 12: Verify `cargo doc` produces no warnings

**Step 1: Run doc generation for both crates**

```bash
cargo doc --no-deps -p krillnotes-core 2>&1
cargo doc --no-deps -p krillnotes-desktop 2>&1
```

Expected: zero `warning:` lines. Any warnings indicate a missing doc comment or a broken `[...]` link — fix them before marking this task complete.

**Step 2: Run all tests one final time**

```bash
cargo test
```
Expected: all tests pass.

**Step 3: Commit (if any fixes were needed)**

```bash
git add -p   # stage only the doc fixes
git commit -m "docs: fix cargo doc warnings"
```
