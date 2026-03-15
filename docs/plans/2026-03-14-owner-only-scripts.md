# Owner-Only Script Enforcement — Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enforce that only the workspace owner (creator) can create, update, or delete scripts — at the core API, sync ingest, swarm transport, and frontend layers.

**Architecture:** Store `owner_pubkey` in `workspace_meta` at creation, cache it on the `Workspace` struct, guard all six script mutation methods, skip unauthorized script ops during sync ingest, embed `owner_pubkey` in every `.swarm` header for cross-validation, and disable script mutation UI for non-owners.

**Tech Stack:** Rust (krillnotes-core), Tauri v2 commands, React 19 + TypeScript

**Spec:** `docs/plans/2026-03-14-owner-only-scripts-design.md`

---

## Chunk 1: Core Rust — Ownership Model

### Task 1: Add `NotOwner` error variant

**Files:**
- Modify: `krillnotes-core/src/core/error.rs:110` (add variant after `Zip`)
- Modify: `krillnotes-core/src/core/error.rs:189` (add `user_message()` arm)

- [ ] **Step 1: Add the enum variant**

In `error.rs`, after line 110 (`Zip(#[from] zip::result::ZipError),`), add:

```rust
    #[error("Only the workspace owner can modify scripts")]
    NotOwner,
```

- [ ] **Step 2: Add `user_message()` arm**

In `error.rs`, after line 189 (`Self::Zip(e) => ...`), add:

```rust
            Self::NotOwner => "Only the workspace owner can modify scripts".to_string(),
```

- [ ] **Step 3: Run tests to verify compilation**

Run: `cargo test -p krillnotes-core --no-run`
Expected: Compiles successfully

- [ ] **Step 4: Commit**

```bash
git add krillnotes-core/src/core/error.rs
git commit -m "feat: add NotOwner error variant for script ownership enforcement"
```

---

### Task 2: Add `owner_pubkey` field to `Workspace` struct and all creation methods

**Files:**
- Modify: `krillnotes-core/src/core/workspace/mod.rs`

- [ ] **Step 1: Add `owner_pubkey` field to the struct**

In `mod.rs`, after the `workspace_id` field (line 88), add:

```rust
    /// Base64-encoded Ed25519 public key of the workspace creator (owner).
    /// Only the owner may create, update, or delete scripts.
    owner_pubkey: String,
```

- [ ] **Step 2: Set `owner_pubkey` in `create()` method**

In the `create()` method, **after** the `identity_pubkey_b64` derivation block (around line 242 — the pubkey is NOT available at the `workspace_id` INSERT point on line 154), add:

```rust
        // Store the creator as workspace owner
        storage.connection().execute(
            "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
            rusqlite::params!["owner_pubkey", &identity_pubkey_b64],
        )?;
```

In the `Self { ... }` construction (line 292), add after `workspace_id,`:

```rust
            owner_pubkey: identity_pubkey_b64.clone(),
```

- [ ] **Step 3: Set `owner_pubkey` in `create_with_id()` method**

Same pattern as Step 2 — add the INSERT after `workspace_id` storage, and add the field to `Self { ... }`.

- [ ] **Step 4: Set `owner_pubkey` in `create_empty()` method**

Same pattern — add INSERT and struct field.

- [ ] **Step 5: Set `owner_pubkey` in `create_empty_with_id()` method**

Same pattern — add INSERT and struct field.

- [ ] **Step 6: Read `owner_pubkey` in `open()` method**

In the `open()` method, after the `identity_uuid` persistence block (around line 833), add:

```rust
        // Read owner_pubkey from workspace_meta. If absent (pre-existing workspace),
        // the current opener becomes the owner.
        let owner_pubkey: String = {
            let existing: std::result::Result<String, rusqlite::Error> = storage.connection().query_row(
                "SELECT value FROM workspace_meta WHERE key = 'owner_pubkey'",
                [],
                |row| row.get(0),
            );
            match existing {
                Ok(pk) => pk,
                Err(_) => {
                    let pk = identity_pubkey_b64.clone();
                    let _ = storage.connection().execute(
                        "INSERT OR IGNORE INTO workspace_meta (key, value) VALUES (?, ?)",
                        rusqlite::params!["owner_pubkey", &pk],
                    );
                    pk
                }
            }
        };
```

In the `Self { ... }` construction (line 835), add after `workspace_id,`:

```rust
            owner_pubkey,
```

- [ ] **Step 7: Add accessor methods**

In `mod.rs`, after the `identity_pubkey()` method (line 917), add:

```rust
    /// Returns the base64-encoded Ed25519 public key of the workspace owner (creator).
    pub fn owner_pubkey(&self) -> &str {
        &self.owner_pubkey
    }

    /// Returns `true` if the currently bound identity is the workspace owner.
    pub fn is_owner(&self) -> bool {
        self.current_identity_pubkey == self.owner_pubkey
    }

    /// Overwrites the cached owner pubkey and persists it to `workspace_meta`.
    /// Used when applying a snapshot bundle — the new workspace is created with
    /// the opener's identity as owner, then overwritten with the snapshot's true owner.
    pub fn set_owner_pubkey(&mut self, pubkey: &str) -> crate::Result<()> {
        self.storage.connection().execute(
            "INSERT OR REPLACE INTO workspace_meta (key, value) VALUES ('owner_pubkey', ?)",
            [pubkey],
        )?;
        self.owner_pubkey = pubkey.to_string();
        Ok(())
    }
```

- [ ] **Step 8: Run tests**

Run: `cargo test -p krillnotes-core --no-run`
Expected: Compiles successfully (existing tests still pass since they use `create()` which sets `owner_pubkey`)

- [ ] **Step 9: Commit**

```bash
git add krillnotes-core/src/core/workspace/mod.rs
git commit -m "feat: store owner_pubkey in workspace_meta and expose is_owner()"
```

---

### Task 3: Guard script mutation methods

**Files:**
- Modify: `krillnotes-core/src/core/workspace/scripts.rs`

- [ ] **Step 1: Write failing tests for non-owner rejection**

In `krillnotes-core/src/core/workspace/tests.rs`, add at the end:

```rust
    #[test]
    fn test_create_user_script_rejected_for_non_owner() {
        let temp = NamedTempFile::new().unwrap();
        // Create workspace with identity A
        let key_a = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let workspace = Workspace::create(temp.path(), "", "identity-a", key_a).unwrap();
        drop(workspace);

        // Re-open with identity B (different signing key → different pubkey)
        let key_b = ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]);
        let mut workspace = Workspace::open(temp.path(), "", "identity-b", key_b).unwrap();

        let source = "// @name: Evil Script\nschema(\"Evil\", #{ version: 1, fields: [] });";
        let result = workspace.create_user_script(source);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("owner"));
    }

    #[test]
    fn test_update_user_script_rejected_for_non_owner() {
        let temp = NamedTempFile::new().unwrap();
        let key_a = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let mut workspace = Workspace::create(temp.path(), "", "identity-a", key_a.clone()).unwrap();
        let source = "// @name: My Script\nschema(\"MyType\", #{ version: 1, fields: [] });";
        let (script, _) = workspace.create_user_script(source).unwrap();
        let script_id = script.id.clone();
        drop(workspace);

        let key_b = ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]);
        let mut workspace = Workspace::open(temp.path(), "", "identity-b", key_b).unwrap();
        let result = workspace.update_user_script(&script_id, "// @name: Hacked\nschema(\"Hacked\", #{ version: 1, fields: [] });");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("owner"));
    }

    #[test]
    fn test_delete_user_script_rejected_for_non_owner() {
        let temp = NamedTempFile::new().unwrap();
        let key_a = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let mut workspace = Workspace::create(temp.path(), "", "identity-a", key_a.clone()).unwrap();
        let source = "// @name: My Script\nschema(\"MyType\", #{ version: 1, fields: [] });";
        let (script, _) = workspace.create_user_script(source).unwrap();
        let script_id = script.id.clone();
        drop(workspace);

        let key_b = ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]);
        let mut workspace = Workspace::open(temp.path(), "", "identity-b", key_b).unwrap();
        let result = workspace.delete_user_script(&script_id);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("owner"));
    }

    #[test]
    fn test_toggle_user_script_rejected_for_non_owner() {
        let temp = NamedTempFile::new().unwrap();
        let key_a = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let mut workspace = Workspace::create(temp.path(), "", "identity-a", key_a.clone()).unwrap();
        let source = "// @name: My Script\nschema(\"MyType\", #{ version: 1, fields: [] });";
        let (script, _) = workspace.create_user_script(source).unwrap();
        let script_id = script.id.clone();
        drop(workspace);

        let key_b = ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]);
        let mut workspace = Workspace::open(temp.path(), "", "identity-b", key_b).unwrap();
        let result = workspace.toggle_user_script(&script_id, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("owner"));
    }

    #[test]
    fn test_reorder_user_script_rejected_for_non_owner() {
        let temp = NamedTempFile::new().unwrap();
        let key_a = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let mut workspace = Workspace::create(temp.path(), "", "identity-a", key_a.clone()).unwrap();
        let source = "// @name: My Script\nschema(\"MyType\", #{ version: 1, fields: [] });";
        let (script, _) = workspace.create_user_script(source).unwrap();
        let script_id = script.id.clone();
        drop(workspace);

        let key_b = ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]);
        let mut workspace = Workspace::open(temp.path(), "", "identity-b", key_b).unwrap();
        let result = workspace.reorder_user_script(&script_id, 0);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("owner"));
    }

    #[test]
    fn test_reorder_all_user_scripts_rejected_for_non_owner() {
        let temp = NamedTempFile::new().unwrap();
        let key_a = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let workspace = Workspace::create(temp.path(), "", "identity-a", key_a).unwrap();
        drop(workspace);

        let key_b = ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]);
        let mut workspace = Workspace::open(temp.path(), "", "identity-b", key_b).unwrap();
        let result = workspace.reorder_all_user_scripts(&[]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("owner"));
    }

    #[test]
    fn test_owner_pubkey_matches_creator() {
        let temp = NamedTempFile::new().unwrap();
        let key = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let workspace = Workspace::create(temp.path(), "", "test-id", key.clone()).unwrap();

        assert_eq!(workspace.owner_pubkey(), workspace.identity_pubkey());
        assert!(workspace.is_owner());
    }

    #[test]
    fn test_is_owner_false_for_different_identity() {
        let temp = NamedTempFile::new().unwrap();
        let key_a = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let workspace = Workspace::create(temp.path(), "", "identity-a", key_a).unwrap();
        drop(workspace);

        let key_b = ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]);
        let workspace = Workspace::open(temp.path(), "", "identity-b", key_b).unwrap();
        assert!(!workspace.is_owner());
    }

    #[test]
    fn test_open_legacy_workspace_without_owner_pubkey_assigns_opener() {
        let temp = NamedTempFile::new().unwrap();
        let key_a = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let workspace = Workspace::create(temp.path(), "", "identity-a", key_a.clone()).unwrap();
        // Manually remove owner_pubkey to simulate a pre-existing workspace
        workspace.connection().execute(
            "DELETE FROM workspace_meta WHERE key = 'owner_pubkey'", [],
        ).unwrap();
        drop(workspace);

        // Re-open — opener should become owner
        let key_b = ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]);
        let workspace = Workspace::open(temp.path(), "", "identity-b", key_b).unwrap();
        assert!(workspace.is_owner());
        assert_eq!(workspace.owner_pubkey(), workspace.identity_pubkey());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p krillnotes-core test_create_user_script_rejected_for_non_owner test_owner_pubkey_matches_creator test_is_owner_false_for_different_identity -- --test-threads=1`
Expected: FAIL — no ownership guard yet (script creation succeeds for non-owner)

- [ ] **Step 3: Add guards to all six methods in `scripts.rs`**

Add at the very top of each method body (before any other code):

For `create_user_script_with_category` (line 78, before `let fm = ...`). This is where the real work happens — `create_user_script` delegates to it, and `create_user_script_with_category` is also `pub` and called directly from Tauri commands:
```rust
        if !self.is_owner() {
            return Err(KrillnotesError::NotOwner);
        }
```

For `update_user_script` (line 148):
```rust
        if !self.is_owner() {
            return Err(KrillnotesError::NotOwner);
        }
```

For `delete_user_script` (line 237):
```rust
        if !self.is_owner() {
            return Err(KrillnotesError::NotOwner);
        }
```

For `toggle_user_script` (line 282):
```rust
        if !self.is_owner() {
            return Err(KrillnotesError::NotOwner);
        }
```

For `reorder_user_script` (line 324):
```rust
        if !self.is_owner() {
            return Err(KrillnotesError::NotOwner);
        }
```

For `reorder_all_user_scripts` (line 366):
```rust
        if !self.is_owner() {
            return Err(KrillnotesError::NotOwner);
        }
```

- [ ] **Step 4: Run all tests**

Run: `cargo test -p krillnotes-core -- --test-threads=1`
Expected: ALL PASS (including new ownership tests and existing script tests — existing tests create the workspace with the same key so `is_owner()` is true)

- [ ] **Step 5: Commit**

```bash
git add krillnotes-core/src/core/workspace/scripts.rs krillnotes-core/src/core/workspace/tests.rs
git commit -m "feat: guard all script mutation methods with owner-only check"
```

---

### Task 4: Enforce ownership in sync ingest

**Files:**
- Modify: `krillnotes-core/src/core/workspace/sync.rs:245-280`

- [ ] **Step 1: Write failing test for sync ingest rejection**

In `krillnotes-core/src/core/workspace/tests.rs`, add:

```rust
    #[test]
    fn test_apply_incoming_script_op_from_owner_applied() {
        let temp = NamedTempFile::new().unwrap();
        let key = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let mut workspace = Workspace::create(temp.path(), "", "test-id", key.clone()).unwrap();
        let owner_pubkey = workspace.identity_pubkey().to_string();

        // Build a CreateUserScript op signed by the owner
        let mut op = krillnotes_core::Operation::CreateUserScript {
            operation_id: uuid::Uuid::new_v4().to_string(),
            timestamp: krillnotes_core::HlcTimestamp { wall_ms: 1000, counter: 0, node_id: 99 },
            device_id: "remote-device".to_string(),
            script_id: uuid::Uuid::new_v4().to_string(),
            name: "Owner Script".to_string(),
            description: "From owner".to_string(),
            source_code: "// @name: Owner Script\n".to_string(),
            load_order: 99,
            enabled: true,
            created_by: owner_pubkey,
            signature: String::new(),
        };
        op.sign(&key);

        let applied = workspace.apply_incoming_operation(op).unwrap();
        assert!(applied);
    }

    #[test]
    fn test_apply_incoming_script_op_from_non_owner_skipped() {
        let temp = NamedTempFile::new().unwrap();
        let key_a = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let mut workspace = Workspace::create(temp.path(), "", "identity-a", key_a).unwrap();

        // Build a CreateUserScript op signed by a different identity
        let key_b = ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]);
        let pubkey_b = {
            use base64::Engine as _;
            let vk = ed25519_dalek::VerifyingKey::from(&key_b);
            base64::engine::general_purpose::STANDARD.encode(vk.as_bytes())
        };
        let script_id = uuid::Uuid::new_v4().to_string();

        let mut op = krillnotes_core::Operation::CreateUserScript {
            operation_id: uuid::Uuid::new_v4().to_string(),
            timestamp: krillnotes_core::HlcTimestamp { wall_ms: 1000, counter: 0, node_id: 99 },
            device_id: "attacker-device".to_string(),
            script_id: script_id.clone(),
            name: "Evil Script".to_string(),
            description: "From attacker".to_string(),
            source_code: "// @name: Evil Script\n".to_string(),
            load_order: 99,
            enabled: true,
            created_by: pubkey_b,
            signature: String::new(),
        };
        op.sign(&key_b);

        // Op is logged (returns true) but script should NOT appear in user_scripts
        let result = workspace.apply_incoming_operation(op).unwrap();
        assert!(result); // Logged to operations table

        // Verify the script was NOT applied to the working table
        let scripts = workspace.list_user_scripts().unwrap();
        assert!(!scripts.iter().any(|s| s.id == script_id));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p krillnotes-core test_apply_incoming_script_op_from_non_owner_skipped -- --test-threads=1`
Expected: FAIL — script gets applied because there's no ownership check

- [ ] **Step 3: Add ownership check to script match arms in `apply_incoming_operation()`**

In `workspace/sync.rs`, modify the three script match arms. Replace lines 245–280 with:

```rust
            Operation::CreateUserScript {
                created_by, script_id, name, description, source_code, load_order, enabled, ..
            } => {
                if created_by == &self.owner_pubkey {
                    let now_ms = ts.wall_ms as i64;
                    tx.execute(
                        "INSERT OR IGNORE INTO user_scripts \
                         (id, name, description, source_code, load_order, enabled, \
                          created_at, modified_at, category) \
                         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'user')",
                        rusqlite::params![
                            script_id, name, description, source_code,
                            load_order, *enabled as i32, now_ms, now_ms,
                        ],
                    )?;
                }
            }

            Operation::UpdateUserScript {
                modified_by, script_id, name, description, source_code, load_order, enabled, ..
            } => {
                if modified_by == &self.owner_pubkey {
                    let now_ms = ts.wall_ms as i64;
                    tx.execute(
                        "UPDATE user_scripts SET name = ?1, description = ?2, source_code = ?3, \
                         load_order = ?4, enabled = ?5, modified_at = ?6 WHERE id = ?7",
                        rusqlite::params![
                            name, description, source_code,
                            load_order, *enabled as i32, now_ms, script_id,
                        ],
                    )?;
                }
            }

            Operation::DeleteUserScript { deleted_by, script_id, .. } => {
                if deleted_by == &self.owner_pubkey {
                    tx.execute(
                        "DELETE FROM user_scripts WHERE id = ?1",
                        [script_id],
                    )?;
                }
            }
```

- [ ] **Step 4: Run all tests**

Run: `cargo test -p krillnotes-core -- --test-threads=1`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add krillnotes-core/src/core/workspace/sync.rs krillnotes-core/src/core/workspace/tests.rs
git commit -m "feat: skip non-owner script ops during sync ingest"
```

---

## Chunk 2: SwarmHeader & Bundle Validation

### Task 5: Add `owner_pubkey` to `SwarmHeader`

**Files:**
- Modify: `krillnotes-core/src/core/swarm/header.rs`

- [ ] **Step 1: Add field to `SwarmHeader` struct**

After line 73 (`pub has_attachments: bool,`), add:

```rust
    /// Ed25519 public key of the workspace owner (base64). Present in all new bundles.
    pub owner_pubkey: Option<String>,
```

- [ ] **Step 2: Update `sample_header` test helper**

In the `sample_header` function (around line 129–152), add after `has_attachments: false,`:

```rust
            owner_pubkey: None,
```

- [ ] **Step 3: Add a round-trip test for `owner_pubkey`**

Add a new test:

```rust
    #[test]
    fn test_header_roundtrip_with_owner_pubkey() {
        let mut h = sample_header(SwarmMode::Delta);
        h.since_operation_id = Some("op-uuid".to_string());
        h.owner_pubkey = Some("owner_b64_key".to_string());
        let json = serde_json::to_string(&h).unwrap();
        let back: SwarmHeader = serde_json::from_str(&json).unwrap();
        assert_eq!(back.owner_pubkey.as_deref(), Some("owner_b64_key"));
    }

    #[test]
    fn test_header_roundtrip_without_owner_pubkey() {
        let mut h = sample_header(SwarmMode::Delta);
        h.since_operation_id = Some("op-uuid".to_string());
        // owner_pubkey is None (backward compat)
        let json = serde_json::to_string(&h).unwrap();
        let back: SwarmHeader = serde_json::from_str(&json).unwrap();
        assert!(back.owner_pubkey.is_none());
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p krillnotes-core swarm -- --test-threads=1`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add krillnotes-core/src/core/swarm/header.rs
git commit -m "feat: add owner_pubkey field to SwarmHeader"
```

---

### Task 6: Set `owner_pubkey` in all bundle generators

**Files:**
- Modify: `krillnotes-core/src/core/swarm/delta.rs:60-81` (delta header construction)
- Modify: `krillnotes-core/src/core/swarm/snapshot.rs:79-100` (snapshot header construction)
- Modify: `krillnotes-core/src/core/swarm/invite.rs:61-82` (invite header), `192-213` (accept header), `38-48` (ParsedInvite)

- [ ] **Step 1: Add `owner_pubkey` field to `DeltaParams`**

In `delta.rs`, add to the `DeltaParams` struct (after `recipient_identity_id`, line 37):

```rust
    pub owner_pubkey: String,
```

Then in the `SwarmHeader { ... }` construction inside `create_delta_bundle()`, add:

```rust
        owner_pubkey: Some(params.owner_pubkey.clone()),
```

The caller (`generate_delta` in `swarm/sync.rs`) already has access to `workspace.owner_pubkey()` and must pass it when constructing `DeltaParams`.

- [ ] **Step 2: Add `owner_pubkey` field to `SnapshotParams`**

In `snapshot.rs`, add to the `SnapshotParams` struct (after `attachment_blobs`, line 35):

```rust
    pub owner_pubkey: String,
```

Then in the `SwarmHeader { ... }` construction inside `create_snapshot_bundle()`, add:

```rust
        owner_pubkey: Some(params.owner_pubkey.clone()),
```

The caller (Tauri command `create_snapshot_for_peers` in `commands/swarm.rs`) must pass `workspace.owner_pubkey().to_string()`.

- [ ] **Step 3: Update all callers that construct `DeltaParams` and `SnapshotParams`**

Search for `DeltaParams {` and `SnapshotParams {` across the codebase and add the `owner_pubkey` field:

- **`DeltaParams`** is constructed in `generate_delta()` at `swarm/sync.rs:96`, which takes `workspace: &Workspace` as a parameter. Add `owner_pubkey: workspace.owner_pubkey().to_string()`.
- **`SnapshotParams`** is constructed in the Tauri command `create_snapshot_for_peers` at `commands/swarm.rs:329`. The workspace is accessed via `state.workspaces.lock()` in a temporary borrow block. Extract `owner_pubkey` in that same block: `let owner_pk = ws.owner_pubkey().to_string();`, then pass it to `SnapshotParams`.

- [ ] **Step 4: Add `owner_pubkey` to `InviteParams` and set in invite header**

In `invite.rs`, add to the `InviteParams` struct:

```rust
    pub owner_pubkey: String,
```

Then in the invite `SwarmHeader { ... }` construction (around line 61), add:

```rust
        owner_pubkey: Some(params.owner_pubkey.clone()),
```

The caller (Tauri command `create_invite` in `commands/invites.rs`) must pass `workspace.owner_pubkey().to_string()`. This ensures the header contains the workspace owner's pubkey, not the inviter's own pubkey (which could differ if non-owner invites are ever allowed in the future).

- [ ] **Step 5: Add `owner_pubkey` to `ParsedInvite` struct**

In `invite.rs`, add a field to the `ParsedInvite` struct (around line 38):

```rust
    pub owner_pubkey: Option<String>,
```

And populate it in `parse_invite_bundle()` from `header.owner_pubkey`.

- [ ] **Step 6: Set `owner_pubkey` in accept bundle header**

In `invite.rs`, find the accept `SwarmHeader { ... }` construction (around line 192). Add:

```rust
        owner_pubkey: params.owner_pubkey.clone(),
```

Add `owner_pubkey: Option<String>` to the `AcceptParams` struct (line 169 in `invite.rs`):

```rust
    pub owner_pubkey: Option<String>,
```

The caller must set `params.owner_pubkey = parsed_invite.owner_pubkey` when constructing `AcceptParams`.

- [ ] **Step 7: Fix any compilation errors from tests that construct SwarmHeader**

Search for all places that construct `SwarmHeader` (in tests and production code) and add `owner_pubkey: None` or `owner_pubkey: Some(...)` as appropriate.

- [ ] **Step 8: Run tests**

Run: `cargo test -p krillnotes-core -- --test-threads=1`
Expected: ALL PASS

- [ ] **Step 9: Commit**

```bash
git add krillnotes-core/src/core/swarm/
git commit -m "feat: embed owner_pubkey in all .swarm bundle headers"
```

---

### Task 7: Validate `owner_pubkey` on bundle receive

**Files:**
- Modify: `krillnotes-core/src/core/swarm/delta.rs` (add `owner_pubkey` to `ParsedDelta`, propagate from header)
- Modify: `krillnotes-core/src/core/swarm/snapshot.rs` (add `owner_pubkey` to `ParsedSnapshot`, propagate from header)
- Modify: `krillnotes-core/src/core/swarm/sync.rs` (cross-check in `apply_delta`)
- Modify: `krillnotes-desktop/src-tauri/src/commands/swarm.rs` (cross-check in `apply_swarm_snapshot` Tauri command)

- [ ] **Step 1: Add `owner_pubkey` to `ParsedDelta`**

In `delta.rs`, add to the `ParsedDelta` struct (after `operations`, line 45):

```rust
    pub owner_pubkey: Option<String>,
```

In `parse_delta_bundle()`, propagate `header.owner_pubkey` into the `ParsedDelta` construction.

- [ ] **Step 2: Add `owner_pubkey` to `ParsedSnapshot`**

In `snapshot.rs`, add to the `ParsedSnapshot` struct (line 38):

```rust
    pub owner_pubkey: Option<String>,
```

In `parse_snapshot_bundle()`, propagate `header.owner_pubkey` into the `ParsedSnapshot` construction.

- [ ] **Step 3: Add `owner_pubkey` cross-check in `apply_delta`**

In `swarm/sync.rs`, in the `apply_delta` function, after the workspace_id matching check (around line 147), add:

```rust
        // Cross-check owner_pubkey if present in the delta
        if let Some(ref header_owner) = parsed.owner_pubkey {
            let local_owner = workspace.owner_pubkey();
            if header_owner != local_owner {
                return Err(KrillnotesError::Swarm(format!(
                    "owner_pubkey mismatch: delta header={}, local={}",
                    &header_owner[..header_owner.len().min(8)],
                    &local_owner[..local_owner.len().min(8)],
                )));
            }
        }
```

Note: `parsed` is the `ParsedDelta` returned by `parse_delta_bundle()`.

- [ ] **Step 4: Set correct `owner_pubkey` in snapshot application**

The snapshot is applied in the **Tauri command** `apply_swarm_snapshot` at `commands/swarm.rs:370`. After the workspace is created and scripts are imported (around line 438), if the parsed snapshot includes `owner_pubkey`, overwrite the default (which is the opener's identity):

```rust
        // Set the true workspace owner from the snapshot header
        if let Some(ref snapshot_owner) = parsed.owner_pubkey {
            ws.set_owner_pubkey(snapshot_owner)
                .map_err(|e| e.to_string())?;
        }
```

This uses the `set_owner_pubkey()` setter added in Task 2, which updates both the DB row and the in-memory field.

- [ ] **Step 5: Run tests**

Run: `cargo test -p krillnotes-core -- --test-threads=1`
Expected: ALL PASS

- [ ] **Step 6: Commit**

```bash
git add krillnotes-core/src/core/swarm/ krillnotes-desktop/src-tauri/src/commands/swarm.rs
git commit -m "feat: validate owner_pubkey on incoming .swarm bundles"
```

---

## Chunk 3: Tauri Command & Frontend

### Task 8: Add `is_workspace_owner` Tauri command

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/commands/workspace.rs`
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs:271-392` (generate_handler!)

- [ ] **Step 1: Add the command function**

In `commands/workspace.rs`, add after the `get_workspace_metadata` function:

```rust
#[tauri::command]
pub fn is_workspace_owner(
    window: tauri::Window,
    state: State<'_, AppState>,
) -> std::result::Result<bool, String> {
    let label = window.label();
    let workspaces = state.workspaces.lock().expect("Mutex poisoned");
    let workspace = workspaces.get(label).ok_or("No workspace open")?;
    Ok(workspace.is_owner())
}
```

- [ ] **Step 2: Register in `generate_handler!`**

In `lib.rs`, add `is_workspace_owner,` to the handler list (after `generate_deltas_for_peers,` at line 391):

```rust
            is_workspace_owner,
```

- [ ] **Step 3: Build to verify**

Run: `cd krillnotes-desktop && cargo build -p krillnotes-desktop-lib`
Expected: Compiles

- [ ] **Step 4: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/commands/workspace.rs krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: add is_workspace_owner Tauri command"
```

---

### Task 9: Add `isOwner` to `PeerInfo` and peers dialog

**Files:**
- Modify: `krillnotes-core/src/core/peer_registry.rs:38-55` (PeerInfo struct)
- Modify: `krillnotes-core/src/core/workspace/sync.rs` (list_peers_info — PeerInfo construction)
- Modify: `krillnotes-desktop/src/types.ts` (TS PeerInfo interface)
- Modify: `krillnotes-desktop/src/components/WorkspacePeersDialog.tsx`

- [ ] **Step 1: Add `is_owner` field to Rust `PeerInfo`**

In `peer_registry.rs`, after the `last_sync` field (line 54), add:

```rust
    /// True if this peer's identity is the workspace owner.
    pub is_owner: bool,
```

- [ ] **Step 2: Set `is_owner` in `list_peers_info()`**

In `workspace/sync.rs`, in the `list_peers_info` method's `into_iter().map(|peer| { ... })` closure, add to the `PeerInfo` construction:

```rust
                    is_owner: peer.peer_identity_id == self.owner_pubkey,
```

Note: use `self.owner_pubkey` (the field), not `self.owner_pubkey()` (accessor) — both work since we're in `impl Workspace`.

- [ ] **Step 3: Add `isOwner` to TS `PeerInfo` interface**

In `krillnotes-desktop/src/types.ts`, after the `lastSync` field (line 244), add:

```typescript
  isOwner?: boolean;
```

- [ ] **Step 4: Show owner badge in `WorkspacePeersDialog.tsx`**

In `WorkspacePeersDialog.tsx`, inside the peer card JSX (around line 143, after the trust badge block), add:

```tsx
                    {peer.isOwner && (
                      <span className="text-xs px-1.5 py-0.5 rounded-full font-medium bg-amber-500/20 text-amber-400">
                        Owner
                      </span>
                    )}
```

- [ ] **Step 5: Build and verify**

Run: `cd krillnotes-desktop && cargo build -p krillnotes-desktop-lib && npx tsc --noEmit`
Expected: Both compile

- [ ] **Step 6: Commit**

```bash
git add krillnotes-core/src/core/peer_registry.rs krillnotes-core/src/core/workspace/sync.rs krillnotes-desktop/src/types.ts krillnotes-desktop/src/components/WorkspacePeersDialog.tsx
git commit -m "feat: show Owner badge on workspace owner in peers dialog"
```

---

### Task 10: Disable script mutation UI for non-owners

**Files:**
- Modify: `krillnotes-desktop/src/components/ScriptManagerDialog.tsx`

- [ ] **Step 1: Add `isOwner` state and fetch on mount**

At the top of the component (near other state declarations), add:

```tsx
const [isOwner, setIsOwner] = useState(true); // optimistic default
```

In the `useEffect` that loads scripts on mount, add:

```tsx
invoke<boolean>('is_workspace_owner').then(setIsOwner).catch(() => setIsOwner(false));
```

- [ ] **Step 2: Add info banner for non-owners**

At the top of the dialog body (before the script list), add:

```tsx
{!isOwner && (
  <div className="mx-4 mt-3 p-2 rounded-md bg-amber-500/10 border border-amber-500/20 text-amber-400 text-xs">
    {t('scripts.ownerOnly', 'Only the workspace owner can modify scripts.')}
  </div>
)}
```

- [ ] **Step 3: Disable Save/Replace button**

Find the Save button (around line 493). Change `disabled={saving}` to:

```tsx
disabled={saving || !isOwner}
```

- [ ] **Step 4: Disable Delete button**

Find the Delete button (around line 476). Change `disabled={saving}` to:

```tsx
disabled={saving || !isOwner}
```

- [ ] **Step 5: Disable New Script / category selection**

Find the "New Script" button/area. Add `disabled={!isOwner}` and conditionally disable the category radio buttons or creation area.

- [ ] **Step 6: Disable Undo/Redo buttons**

Find the Undo button (around line 418). Change `disabled={!canScriptUndo}` to:

```tsx
disabled={!canScriptUndo || !isOwner}
```

Same for Redo (around line 426): change `disabled={!canScriptRedo}` to:

```tsx
disabled={!canScriptRedo || !isOwner}
```

- [ ] **Step 7: Disable toggle checkbox**

Find the enabled checkbox (around line 362). Add:

```tsx
disabled={!isOwner}
```

- [ ] **Step 8: Disable reorder drag handles**

Find the drag handlers (around lines 227–267). Make the drag handles/events conditional on `isOwner`:

```tsx
draggable={isOwner}
onDragStart={isOwner ? handleDragStart(script.id) : undefined}
```

- [ ] **Step 9: TypeScript check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: No errors

- [ ] **Step 10: Commit**

```bash
git add krillnotes-desktop/src/components/ScriptManagerDialog.tsx
git commit -m "feat: disable script mutation controls for non-owner users"
```

---

## Chunk 4: Final Verification

### Task 11: Full test suite and build

- [ ] **Step 1: Run full Rust test suite**

Run: `cargo test -p krillnotes-core -- --test-threads=1`
Expected: ALL PASS

- [ ] **Step 2: Run TypeScript type check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: No errors

- [ ] **Step 3: Run full Tauri build**

Run: `cd krillnotes-desktop && npm update && npm run tauri build`
Expected: Builds successfully

- [ ] **Step 4: Final commit if any fixups**

Only if needed from build issues.
