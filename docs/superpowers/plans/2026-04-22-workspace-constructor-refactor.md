# Workspace Constructor Refactor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collapse four near-identical workspace constructors (~710 lines of duplicated init code) into thin wrappers around a single `init_core` method, controlled by a `WorkspaceConfig` struct.

**Architecture:** Extract a `WorkspaceConfig` struct capturing the three axes of variation (`workspace_id`, `insert_root_note`, `seed_starter_scripts`). A private `fn init_core(config, ...)` handles all shared initialization. The four public constructors become 5–10 line wrappers that build a config and delegate. Public API signatures remain identical — all existing tests must pass without modification.

**Tech Stack:** Rust (krillnotes-core crate), rusqlite, ed25519-dalek, rhai

---

## File Map

| Action | File | Responsibility |
|--------|------|---------------|
| Modify | `krillnotes-core/src/core/workspace/mod.rs` | Add `WorkspaceConfig`, extract `init_core`, convert 4 constructors to wrappers |
| Modify | `krillnotes-core/src/core/workspace/tests.rs` | Add test for `create_empty_with_id` skipping starter scripts |

No new files. No database schema changes. No changes to `open()`.

## Current State (reference)

The four constructors live at these lines in `workspace/mod.rs`:
- `create`: line 134 (~220 lines)
- `create_with_id`: line 351 (~220 lines)
- `create_empty`: line 578 (~150 lines)
- `create_empty_with_id`: line 729 (~110 lines)

Three axes of variation:

| Constructor | `workspace_id` | `insert_root_note` | `seed_starter_scripts` |
|---|---|---|---|
| `create` | generate UUID | true | true |
| `create_with_id` | caller-supplied | true | true |
| `create_empty` | generate UUID | false | true |
| `create_empty_with_id` | caller-supplied | false | **false** |

The `seed_starter_scripts: false` case in `create_empty_with_id` is a subtle, intentional behavioral difference (snapshot restoration — the caller calls `reload_all_scripts()` after import). Today it's expressed via `let` vs `let mut` on `storage` and `script_registry`, which makes it invisible. The `WorkspaceConfig` field makes this explicit.

---

### Task 1: Establish Green Baseline

**Files:**
- Read: `krillnotes-core/src/core/workspace/mod.rs`
- Read: `krillnotes-core/src/core/workspace/tests.rs`

- [ ] **Step 1: Run the existing test suite**

Run: `cargo test -p krillnotes-core`
Expected: All tests pass. Record any pre-existing failures so we can distinguish regressions.

- [ ] **Step 2: Commit baseline (skip if clean)**

Only if there are uncommitted changes to stash or note. Otherwise proceed.

---

### Task 2: Add `WorkspaceConfig` Struct

**Files:**
- Modify: `krillnotes-core/src/core/workspace/mod.rs:125` (just before `impl Workspace`)

- [ ] **Step 1: Write the failing test**

Add to `krillnotes-core/src/core/workspace/tests.rs` near the other constructor tests (after line ~2930):

```rust
#[test]
fn test_create_empty_with_id_skips_starter_scripts() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("notes.db");
    let key = ed25519_dalek::SigningKey::from_bytes(&[5u8; 32]);
    let ws = Workspace::create_empty_with_id(
        &db_path, "", "test-id", key, "skip-scripts-uuid", test_gate(), None,
    )
    .unwrap();
    // create_empty_with_id intentionally skips starter script seeding.
    // The user_scripts table should be empty.
    let count: i64 = ws
        .connection()
        .query_row("SELECT COUNT(*) FROM user_scripts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0, "create_empty_with_id should not seed starter scripts");
}
```

- [ ] **Step 2: Run test to verify it passes (characterisation test)**

Run: `cargo test -p krillnotes-core test_create_empty_with_id_skips_starter_scripts -- --nocapture`
Expected: PASS — this documents existing behaviour. If it fails, the spec's assumption about `create_empty_with_id` is wrong and we need to investigate.

- [ ] **Step 3: Add `WorkspaceConfig` struct**

Insert just before the `impl Workspace {` line (currently line 126) in `krillnotes-core/src/core/workspace/mod.rs`:

```rust
/// Configuration for workspace creation, capturing the variable parts
/// across the four constructor variants.
pub(crate) struct WorkspaceConfig {
    /// If `Some`, use this as the workspace UUID (snapshot restore).
    /// If `None`, generate a fresh UUID.
    pub workspace_id: Option<String>,
    /// Whether to insert a default root note named after the workspace folder.
    pub insert_root_note: bool,
    /// Whether to seed the built-in starter scripts (TextNote, etc.).
    /// Set to `false` for snapshot restoration where the caller imports
    /// its own scripts via `reload_all_scripts()`.
    pub seed_starter_scripts: bool,
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p krillnotes-core`
Expected: Compiles with no errors (the struct is unused so far — no warnings in test cfg).

- [ ] **Step 5: Commit**

```bash
git add krillnotes-core/src/core/workspace/mod.rs krillnotes-core/src/core/workspace/tests.rs
git commit -m "refactor: add WorkspaceConfig struct and characterisation test for create_empty_with_id"
```

---

### Task 3: Extract `init_core` Method

**Files:**
- Modify: `krillnotes-core/src/core/workspace/mod.rs`

This is the core of the refactor. `init_core` contains all the shared logic from the four constructors.

- [ ] **Step 1: Write `init_core` as a private method**

Add inside `impl Workspace`, after the `WorkspaceConfig` struct and before the existing `create` method:

```rust
/// Shared initialisation for all `create*` constructors.
///
/// Handles: storage creation, device ID, metadata insertion, workspace root,
/// workspace_id (generate or use provided), attachment key, starter scripts
/// (if enabled), script loading, identity pubkey, owner_pubkey, root note
/// (if enabled), undo_limit, HLC, permission gate, RegisterDevice op.
fn init_core<P: AsRef<Path>>(
    config: WorkspaceConfig,
    path: P,
    password: &str,
    identity_uuid: &str,
    signing_key: ed25519_dalek::SigningKey,
    permission_gate: Box<dyn crate::core::permission::PermissionGate>,
    identity_dir: Option<&Path>,
) -> Result<Self> {
    let mut storage = Storage::create(&path, password)?;
    let mut script_registry = ScriptRegistry::new()?;
    let operation_log = OperationLog::new(PurgeStrategy::LocalOnly { keep_last: 100 });

    // Build composite device_id: {identity_uuid}:{device_uuid} when identity_dir is known.
    let device_id = if let Some(dir) = identity_dir {
        let device_uuid = crate::core::identity::ensure_device_uuid(dir)?;
        format!("{identity_uuid}:{device_uuid}")
    } else {
        identity_uuid.to_string()
    };

    // Store metadata
    storage.connection().execute(
        "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
        ["device_id", &device_id],
    )?;
    storage.connection().execute(
        "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
        ["identity_uuid", identity_uuid],
    )?;

    // Derive workspace root from db path
    let workspace_root = path.as_ref()
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf();
    // Create attachments directory (idempotent, best-effort)
    let _ = std::fs::create_dir_all(workspace_root.join("attachments"));

    // Workspace ID: use provided or generate fresh
    let workspace_id = config.workspace_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    storage.connection().execute(
        "INSERT OR IGNORE INTO workspace_meta (key, value) VALUES (?, ?)",
        rusqlite::params!["workspace_id", &workspace_id],
    )?;

    // Derive attachment key
    let attachment_key = if !password.is_empty() {
        Some(crate::core::attachment::derive_attachment_key(password, &workspace_id))
    } else {
        None
    };

    // Seed starter scripts if requested.
    let now = chrono::Utc::now().timestamp();
    if config.seed_starter_scripts {
        let starters = ScriptRegistry::starter_scripts();
        let tx = storage.connection_mut().transaction()?;
        for (load_order, starter) in starters.iter().enumerate() {
            let fm = user_script::parse_front_matter(&starter.source_code);
            let id = Uuid::new_v4().to_string();
            let category = if starter.filename.ends_with(".schema.rhai") {
                "schema"
            } else {
                "library"
            };
            tx.execute(
                "INSERT INTO user_scripts (id, name, description, source_code, load_order, enabled, created_at, modified_at, category)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    id, fm.name, fm.description, &starter.source_code,
                    load_order as i32, true, now, now, category
                ],
            )?;
        }
        tx.commit()?;
    }

    // Load all scripts from the DB into the registry (two-phase: library then schema).
    {
        let mut stmt = storage.connection().prepare(
            "SELECT id, name, description, source_code, load_order, enabled, created_at, modified_at, category
             FROM user_scripts ORDER BY load_order ASC, created_at ASC",
        )?;
        let scripts: Vec<UserScript> = stmt
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
                    category: row.get::<_, String>(8)
                        .unwrap_or_else(|_| "library".to_string()),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for script in scripts.iter().filter(|s| s.enabled && s.category == "library") {
            script_registry.set_loading_category(Some("library".to_string()));
            if let Err(e) = script_registry.load_script(&script.source_code, &script.name) {
                log::warn!("Failed to load starter script '{}': {}", script.name, e);
            }
        }
        for script in scripts.iter().filter(|s| s.enabled && s.category == "schema") {
            script_registry.set_loading_category(Some("schema".to_string()));
            if let Err(e) = script_registry.load_script(&script.source_code, &script.name) {
                log::warn!("Failed to load starter script '{}': {}", script.name, e);
            }
        }
        script_registry.resolve_bindings();
    }

    // Derive the base64-encoded public key from the signing key.
    let identity_pubkey_b64 = {
        use base64::Engine as _;
        let pubkey = ed25519_dalek::VerifyingKey::from(&signing_key);
        base64::engine::general_purpose::STANDARD.encode(pubkey.as_bytes())
    };

    // Store the creator as workspace owner
    storage.connection().execute(
        "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
        rusqlite::params!["owner_pubkey", &identity_pubkey_b64],
    )?;

    // Insert root note if requested.
    if config.insert_root_note {
        let filename = {
            let parent_name = path.as_ref()
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str());
            let db_stem = path.as_ref()
                .file_stem()
                .and_then(|s| s.to_str());
            match parent_name {
                Some(name) if !name.is_empty() && name != "." => name,
                _ => db_stem.unwrap_or("Untitled"),
            }
        };
        let title = humanize(filename);

        let root = Note {
            id: Uuid::new_v4().to_string(),
            title,
            schema: "TextNote".to_string(),
            parent_id: None,
            position: 0.0,
            created_at: now,
            modified_at: now,
            created_by: identity_pubkey_b64.clone(),
            modified_by: identity_pubkey_b64.clone(),
            fields: script_registry.get_schema("TextNote")?.default_fields(),
            is_expanded: true,
            tags: vec![],
            schema_version: 1,
            is_checked: false,
        };

        let tx = storage.connection_mut().transaction()?;
        tx.execute(
            "INSERT INTO notes (id, title, schema, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded, schema_version, is_checked)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                root.id, root.title, root.schema, root.parent_id,
                root.position, root.created_at, root.modified_at,
                root.created_by, root.modified_by,
                serde_json::to_string(&root.fields)?,
                true, root.schema_version, root.is_checked,
            ],
        )?;
        tx.commit()?;
    }

    storage.connection().execute(
        "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
        ["undo_limit", "50"],
    )?;
    let undo_limit: usize = 50;

    // Initialise HLC for this workspace.
    let device_uuid_str = crate::core::identity::device_part_from_device_id(&device_id);
    let node_id = crate::core::hlc::node_id_from_device(
        &uuid::Uuid::parse_str(device_uuid_str)
            .unwrap_or_else(|_| uuid::Uuid::new_v4()),
    );
    let hlc = HlcClock::new(node_id);

    // Initialise permission gate tables.
    permission_gate.ensure_schema(storage.connection())?;

    let mut workspace = Self {
        storage,
        script_registry,
        operation_log,
        device_id,
        identity_uuid: identity_uuid.to_string(),
        current_identity_pubkey: identity_pubkey_b64.clone(),
        workspace_root,
        workspace_id,
        owner_pubkey: identity_pubkey_b64,
        attachment_key,
        undo_stack: Vec::new(),
        redo_stack: Vec::new(),
        undo_limit,
        script_undo_stack: Vec::new(),
        script_redo_stack: Vec::new(),
        undo_group_buffer: None,
        inside_undo: false,
        hlc,
        signing_key,
        pending_migration_results: Vec::new(),
        permission_gate,
    };
    workspace.emit_register_device_if_needed()?;
    let _ = workspace.write_info_json();
    Ok(workspace)
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p krillnotes-core`
Expected: Compiles (method is unused — no warnings in non-test cfg since it's private).

- [ ] **Step 3: Commit**

```bash
git add krillnotes-core/src/core/workspace/mod.rs
git commit -m "refactor: extract init_core method for shared workspace initialisation"
```

---

### Task 4: Convert `create` to Wrapper

**Files:**
- Modify: `krillnotes-core/src/core/workspace/mod.rs`

- [ ] **Step 1: Replace the `create` method body**

Replace the entire body of `create` (lines ~134–346) with:

```rust
pub fn create<P: AsRef<Path>>(
    path: P,
    password: &str,
    identity_uuid: &str,
    signing_key: ed25519_dalek::SigningKey,
    permission_gate: Box<dyn crate::core::permission::PermissionGate>,
    identity_dir: Option<&Path>,
) -> Result<Self> {
    Self::init_core(
        WorkspaceConfig {
            workspace_id: None,
            insert_root_note: true,
            seed_starter_scripts: true,
        },
        path,
        password,
        identity_uuid,
        signing_key,
        permission_gate,
        identity_dir,
    )
}
```

- [ ] **Step 2: Run the full test suite**

Run: `cargo test -p krillnotes-core`
Expected: All tests pass. This is the highest-risk conversion since `create` is called by ~90% of tests.

- [ ] **Step 3: Commit**

```bash
git add krillnotes-core/src/core/workspace/mod.rs
git commit -m "refactor: convert Workspace::create to init_core wrapper"
```

---

### Task 5: Convert `create_with_id` to Wrapper

**Files:**
- Modify: `krillnotes-core/src/core/workspace/mod.rs`

- [ ] **Step 1: Replace the `create_with_id` method body**

Replace the entire body with:

```rust
pub fn create_with_id<P: AsRef<Path>>(
    path: P,
    password: &str,
    identity_uuid: &str,
    signing_key: ed25519_dalek::SigningKey,
    workspace_id: &str,
    permission_gate: Box<dyn crate::core::permission::PermissionGate>,
    identity_dir: Option<&Path>,
) -> Result<Self> {
    Self::init_core(
        WorkspaceConfig {
            workspace_id: Some(workspace_id.to_string()),
            insert_root_note: true,
            seed_starter_scripts: true,
        },
        path,
        password,
        identity_uuid,
        signing_key,
        permission_gate,
        identity_dir,
    )
}
```

- [ ] **Step 2: Run relevant tests**

Run: `cargo test -p krillnotes-core test_create_with_id`
Expected: `test_create_with_id_preserves_uuid` passes.

- [ ] **Step 3: Commit**

```bash
git add krillnotes-core/src/core/workspace/mod.rs
git commit -m "refactor: convert Workspace::create_with_id to init_core wrapper"
```

---

### Task 6: Convert `create_empty` to Wrapper

**Files:**
- Modify: `krillnotes-core/src/core/workspace/mod.rs`

- [ ] **Step 1: Replace the `create_empty` method body**

Replace the entire body with:

```rust
pub fn create_empty<P: AsRef<Path>>(
    path: P,
    password: &str,
    identity_uuid: &str,
    signing_key: ed25519_dalek::SigningKey,
    permission_gate: Box<dyn crate::core::permission::PermissionGate>,
    identity_dir: Option<&Path>,
) -> Result<Self> {
    Self::init_core(
        WorkspaceConfig {
            workspace_id: None,
            insert_root_note: false,
            seed_starter_scripts: true,
        },
        path,
        password,
        identity_uuid,
        signing_key,
        permission_gate,
        identity_dir,
    )
}
```

- [ ] **Step 2: Run the full test suite**

Run: `cargo test -p krillnotes-core`
Expected: All tests pass. `create_empty` is used in snapshot import tests.

- [ ] **Step 3: Commit**

```bash
git add krillnotes-core/src/core/workspace/mod.rs
git commit -m "refactor: convert Workspace::create_empty to init_core wrapper"
```

---

### Task 7: Convert `create_empty_with_id` to Wrapper

**Files:**
- Modify: `krillnotes-core/src/core/workspace/mod.rs`

- [ ] **Step 1: Replace the `create_empty_with_id` method body**

Replace the entire body with:

```rust
pub fn create_empty_with_id<P: AsRef<Path>>(
    path: P,
    password: &str,
    identity_uuid: &str,
    signing_key: ed25519_dalek::SigningKey,
    workspace_id: &str,
    permission_gate: Box<dyn crate::core::permission::PermissionGate>,
    identity_dir: Option<&Path>,
) -> Result<Self> {
    Self::init_core(
        WorkspaceConfig {
            workspace_id: Some(workspace_id.to_string()),
            insert_root_note: false,
            seed_starter_scripts: false,
        },
        path,
        password,
        identity_uuid,
        signing_key,
        permission_gate,
        identity_dir,
    )
}
```

- [ ] **Step 2: Run the targeted tests**

Run: `cargo test -p krillnotes-core test_create_empty_with_id`
Expected: Both `test_create_empty_with_id_no_root_note` and `test_create_empty_with_id_skips_starter_scripts` pass.

- [ ] **Step 3: Commit**

```bash
git add krillnotes-core/src/core/workspace/mod.rs
git commit -m "refactor: convert Workspace::create_empty_with_id to init_core wrapper"
```

---

### Task 8: Final Verification

**Files:**
- Read: `krillnotes-core/src/core/workspace/mod.rs` (spot-check)

- [ ] **Step 1: Run the full test suite across all crates**

Run: `cargo test -p krillnotes-core && cargo test -p krillnotes-rbac`
Expected: All tests pass in both crates.

- [ ] **Step 2: Run cargo clippy**

Run: `cargo clippy -p krillnotes-core -- -D warnings`
Expected: No warnings. If clippy flags the unused `WorkspaceConfig` fields (it won't — they're used in `init_core`), that's a bug to investigate.

- [ ] **Step 3: Verify the doc comments are preserved**

Read the four wrapper methods and `init_core` to confirm:
- Each wrapper retains its original `///` doc comment explaining when to use it
- `init_core` has a doc comment listing what it handles

- [ ] **Step 4: Verify line count reduction**

Run: `wc -l krillnotes-core/src/core/workspace/mod.rs`
Expected: Approximately 500–600 fewer lines than the original (~1560 lines → ~950–1050 lines).

- [ ] **Step 5: Final commit (if any doc comment touch-ups needed)**

```bash
git add krillnotes-core/src/core/workspace/mod.rs
git commit -m "refactor: workspace constructor refactor — cleanup and verification"
```

---

## Post-Completion Notes

- **Public API unchanged**: All four constructors retain their exact same signatures. No callers need modification.
- **Batch 2 dependency**: The `PurgeStrategy::LocalOnly { keep_last: 100 }` is now set in exactly one place (`init_core`), making M5 (configurable purge strategy) a one-line change.
- **`open()` is not touched**: It has fundamentally different logic (reads from DB vs writes to DB) and does not benefit from this refactor.
