# Identity-Neutral Export Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `.krillnotes` export archives identity-neutral so any identity can import them as full owner.

**Architecture:** Two surgical changes in `export.rs` — strip identity fields during export, stamp importer's identity during import. No other files change.

**Tech Stack:** Rust, rusqlite, zip crate, serde_json, ed25519_dalek (for test signing keys)

---

## File Structure

| File | Action | Responsibility |
|------|--------|---------------|
| `krillnotes-core/src/core/export.rs` | Modify lines 217-224 and 260-267 | Strip identity from archive on export |
| `krillnotes-core/src/core/export.rs` | Modify lines 574-582 | Remove owner_pubkey restoration, stamp importer on notes |
| `krillnotes-core/src/core/export_tests.rs` | Add 2 new tests | Verify identity-neutral archive and round-trip identity |

---

### Task 1: Strip identity fields during export

**Files:**
- Modify: `krillnotes-core/src/core/export.rs:217-224` (notes identity strip)
- Modify: `krillnotes-core/src/core/export.rs:260-267` (workspace.json identity strip)

- [ ] **Step 1: Write test — archive contains no identity data**

Add this test to the end of `krillnotes-core/src/core/export_tests.rs`:

```rust
#[test]
fn test_export_archive_is_identity_neutral() {
    let temp = NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(
        temp.path(),
        "",
        "test-identity",
        ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]),
        test_gate(),
        None,
    )
    .unwrap();

    let root = ws.list_all_notes().unwrap()[0].clone();
    ws.create_note(&root.id, AddPosition::AsChild, "TextNote")
        .unwrap();

    let mut buf = Vec::new();
    export_workspace(&ws, Cursor::new(&mut buf), None).unwrap();

    let mut archive = zip::ZipArchive::new(Cursor::new(&buf)).unwrap();

    // workspace.json must not contain owner_pubkey
    let ws_file = archive.by_name("workspace.json").unwrap();
    let ws_meta: WorkspaceMetadata = serde_json::from_reader(ws_file).unwrap();
    assert!(
        ws_meta.owner_pubkey.is_none(),
        "exported workspace.json must not contain owner_pubkey"
    );

    // notes.json must have empty created_by / modified_by
    let notes_file = archive.by_name("notes.json").unwrap();
    let export_notes: ExportNotes = serde_json::from_reader(notes_file).unwrap();
    for note in &export_notes.notes {
        assert!(
            note.created_by.is_empty(),
            "note '{}' created_by should be empty, got '{}'",
            note.title,
            note.created_by
        );
        assert!(
            note.modified_by.is_empty(),
            "note '{}' modified_by should be empty, got '{}'",
            note.title,
            note.modified_by
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core test_export_archive_is_identity_neutral -- --nocapture`

Expected: FAIL — `owner_pubkey` is `Some(...)` and `created_by`/`modified_by` are non-empty.

- [ ] **Step 3: Strip identity fields in `export_workspace`**

In `krillnotes-core/src/core/export.rs`, make two changes:

**Change 1 — Clear note identity fields (lines 217-224).** Replace:

```rust
    // Write notes.json
    let export_notes = ExportNotes {
        version: 1,
        app_version: APP_VERSION.to_string(),
        notes,
    };
```

With:

```rust
    // Write notes.json — strip identity fields so the archive is identity-neutral
    let notes = notes
        .into_iter()
        .map(|mut n| {
            n.created_by = String::new();
            n.modified_by = String::new();
            n
        })
        .collect();
    let export_notes = ExportNotes {
        version: 1,
        app_version: APP_VERSION.to_string(),
        notes,
    };
```

**Change 2 — Omit owner_pubkey from workspace.json (line 265).** Replace:

```rust
    ws_meta.owner_pubkey = Some(workspace.owner_pubkey().to_string());
```

With:

```rust
    ws_meta.owner_pubkey = None;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p krillnotes-core test_export_archive_is_identity_neutral -- --nocapture`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add krillnotes-core/src/core/export.rs krillnotes-core/src/core/export_tests.rs
git commit -m "feat: strip identity fields from export archive (#155)"
```

---

### Task 2: Stamp importer's identity on imported notes

**Files:**
- Modify: `krillnotes-core/src/core/export.rs:574-582` (remove owner restoration, add identity stamp)

- [ ] **Step 1: Write test — importer becomes owner and author of all notes**

Add this test to the end of `krillnotes-core/src/core/export_tests.rs`:

```rust
#[test]
fn test_import_stamps_importer_identity_on_notes() {
    // Export from identity A
    let temp_src = NamedTempFile::new().unwrap();
    let key_a = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
    let mut ws_a = Workspace::create(
        temp_src.path(),
        "",
        "identity-a",
        key_a.clone(),
        test_gate(),
        None,
    )
    .unwrap();

    let root = ws_a.list_all_notes().unwrap()[0].clone();
    ws_a.create_note(&root.id, AddPosition::AsChild, "TextNote")
        .unwrap();

    let mut buf = Vec::new();
    export_workspace(&ws_a, Cursor::new(&mut buf), None).unwrap();

    // Import as identity B (different key)
    let temp_dst = NamedTempFile::new().unwrap();
    let key_b = ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]);
    import_workspace(
        Cursor::new(&buf),
        temp_dst.path(),
        None,
        "",
        "identity-b",
        key_b.clone(),
    )
    .unwrap();

    let ws_b = Workspace::open(
        temp_dst.path(),
        "",
        "identity-b",
        key_b,
        test_gate(),
        None,
    )
    .unwrap();

    // Importer is owner
    assert!(ws_b.is_owner(), "importer should be workspace owner");

    // All notes have importer's pubkey as created_by and modified_by
    let importer_pubkey = ws_b.identity_pubkey().to_string();
    for note in ws_b.list_all_notes().unwrap() {
        assert_eq!(
            note.created_by, importer_pubkey,
            "note '{}' created_by should be importer's pubkey",
            note.title
        );
        assert_eq!(
            note.modified_by, importer_pubkey,
            "note '{}' modified_by should be importer's pubkey",
            note.title
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core test_import_stamps_importer_identity_on_notes -- --nocapture`

Expected: FAIL — `created_by` and `modified_by` are empty strings (from export stripping), not the importer's pubkey.

- [ ] **Step 3: Remove owner restoration and add identity stamp in `import_workspace`**

In `krillnotes-core/src/core/export.rs`, replace lines 574-582:

```rust
    // Restore the original owner_pubkey from the archive, overriding the
    // importer's key that Workspace::open() inserted.
    if let Some(ref meta) = workspace_metadata {
        if let Some(ref original_owner) = meta.owner_pubkey {
            workspace
                .set_owner_pubkey(original_owner)
                .map_err(|e| ExportError::Database(e.to_string()))?;
        }
    }
```

With:

```rust
    // Stamp the importer's identity as author of all imported notes.
    workspace
        .connection()
        .execute(
            "UPDATE notes SET created_by = ?, modified_by = ?",
            [workspace.identity_pubkey(), workspace.identity_pubkey()],
        )
        .map_err(|e| ExportError::Database(e.to_string()))?;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p krillnotes-core test_import_stamps_importer_identity_on_notes -- --nocapture`

Expected: PASS

- [ ] **Step 5: Run full test suite to check for regressions**

Run: `cargo test -p krillnotes-core`

Expected: All tests pass. The existing `test_round_trip_export_import` test uses the same identity for export and import, so the stamped identity will match and existing assertions still hold.

- [ ] **Step 6: Commit**

```bash
git add krillnotes-core/src/core/export.rs krillnotes-core/src/core/export_tests.rs
git commit -m "feat: stamp importer identity on imported notes (#155)"
```

---

### Task 3: Verify and update existing tests

**Files:**
- Modify: `krillnotes-core/src/core/export_tests.rs:310-329` (update `test_export_includes_workspace_json`)

- [ ] **Step 1: Update existing workspace.json test**

The existing `test_export_includes_workspace_json` (line 310) currently only checks `ws_meta.version == 1`. Add an assertion that `owner_pubkey` is `None`:

After this line:
```rust
    assert_eq!(ws_meta.version, 1);
```

Add:
```rust
    assert!(
        ws_meta.owner_pubkey.is_none(),
        "exported workspace.json must not contain owner_pubkey"
    );
```

- [ ] **Step 2: Run full test suite**

Run: `cargo test -p krillnotes-core`

Expected: All tests pass.

- [ ] **Step 3: Commit**

```bash
git add krillnotes-core/src/core/export_tests.rs
git commit -m "test: assert identity-neutral workspace.json in existing export test (#155)"
```
