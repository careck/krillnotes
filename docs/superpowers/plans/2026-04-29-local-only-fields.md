# Local-Only Fields Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Allow schema field definitions to declare `local_only: true`, so that field values are stored locally but never synced to peers via delta bundles or snapshots, and optionally excluded from `.krillnotes` exports.

**Architecture:** Add a `local_only` boolean to `FieldDefinition` (Rust + TS). Filter `UpdateField` ops and `CreateNote` field maps at the sync boundary (`operations_since`, `operations_since_with_verified_by`, `to_snapshot_json`). Add an `include_local_only` option to export. Show a subtle UI indicator on local-only fields.

**Tech Stack:** Rust (krillnotes-core), TypeScript/React (krillnotes-desktop), Rhai scripting, Tailwind CSS, i18next

---

## File Map

| Action | File | Responsibility |
|--------|------|----------------|
| Modify | `krillnotes-core/src/core/scripting/schema.rs:84-112` | Add `local_only: bool` to `FieldDefinition`, parse it in `parse_field_def` |
| Modify | `krillnotes-core/src/core/workspace/sync.rs:89-197` | Filter local-only `UpdateField` ops in `operations_since` |
| Modify | `krillnotes-core/src/core/workspace/sync.rs:205-301` | Filter local-only `UpdateField` ops in `operations_since_with_verified_by` |
| Modify | `krillnotes-core/src/core/workspace/sync.rs:19-36` | Strip local-only fields from notes in `to_snapshot_json` |
| Modify | `krillnotes-core/src/core/export.rs:197-303` | Add `include_local_only` param to `export_workspace`, strip fields when false |
| Modify | `krillnotes-core/src/lib.rs:26` | Re-export updated `export_workspace` signature (param change) |
| Modify | `krillnotes-core/src/core/workspace/tests.rs` | Add tests for all filtering behaviours |
| Modify | `krillnotes-desktop/src/types.ts:57-69` | Add `localOnly: boolean` to `FieldDefinition` |
| Modify | `krillnotes-desktop/src/components/InfoPanel.tsx:501-516` | Show local-only indicator on field labels |
| Modify | `krillnotes-desktop/src/App.tsx:260-310` | Add "include local-only data" checkbox to export dialog |
| Modify | `krillnotes-desktop/src-tauri/src/commands/workspace.rs:734-753` | Pass `include_local_only` param to core |
| Modify | `krillnotes-desktop/src/i18n/locales/en.json` | Add i18n keys for local-only indicator + export checkbox |
| Modify | `krillnotes-desktop/src/i18n/locales/{de,es,fr,ja,ko,zh}.json` | Add translated keys |

---

## Task 1: Add `local_only` to `FieldDefinition` and parse it from Rhai

**Files:**
- Modify: `krillnotes-core/src/core/scripting/schema.rs:84-112` (struct)
- Modify: `krillnotes-core/src/core/scripting/schema.rs:319-331` (parse_field_def return)
- Test: `krillnotes-core/src/core/workspace/tests.rs`

- [ ] **Step 1: Write test — schema with local_only field parses correctly**

Add to `krillnotes-core/src/core/workspace/tests.rs`:

```rust
#[test]
fn test_schema_local_only_field_parsed() {
    let temp = NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(
        temp.path(),
        "",
        "id-1",
        ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]),
        test_gate(),
        None,
    )
    .unwrap();

    let script = r#"
        schema("LocalTest", #{
            version: 1,
            fields: [
                #{ name: "public_field", type: "text" },
                #{ name: "private_notes", type: "text", local_only: true },
            ]
        });
    "#;
    ws.create_user_script("local_test", script, "schema")
        .unwrap();

    let schema = ws.script_registry().get_schema("LocalTest").unwrap();
    let public_f = schema.all_fields().iter().find(|f| f.name == "public_field").unwrap();
    let private_f = schema.all_fields().iter().find(|f| f.name == "private_notes").unwrap();
    assert!(!public_f.local_only);
    assert!(private_f.local_only);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core test_schema_local_only_field_parsed`
Expected: FAIL — `local_only` field does not exist on `FieldDefinition`

- [ ] **Step 3: Add `local_only: bool` to `FieldDefinition`**

In `krillnotes-core/src/core/scripting/schema.rs`, add the field after `show_on_hover` (around line 103):

```rust
    /// When `true`, this field is included in the hover-tooltip simple-path renderer.
    /// Defaults to `false` (opt-in).
    #[serde(default)]
    pub show_on_hover: bool,
    /// When `true`, values for this field are never synced to peers.
    /// The field exists in the schema for all peers, but each peer stores
    /// their own independent value locally.
    #[serde(default)]
    pub local_only: bool,
```

- [ ] **Step 4: Parse `local_only` from Rhai map in `parse_field_def`**

In the same file, around line 298 (after `show_on_hover` parsing), add:

```rust
        let local_only = field_map
            .get("local_only")
            .and_then(|v| v.clone().try_cast::<bool>())
            .unwrap_or(false);
```

And add `local_only,` to the `Ok(FieldDefinition { ... })` struct literal at line ~329 (after `show_on_hover,`).

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p krillnotes-core test_schema_local_only_field_parsed`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add krillnotes-core/src/core/scripting/schema.rs krillnotes-core/src/core/workspace/tests.rs
git commit -m "feat: add local_only flag to FieldDefinition schema parsing"
```

---

## Task 2: Add helper — `Workspace::is_field_local_only`

This helper resolves whether a given `(schema_name, field_name)` pair is local-only. Used by the sync filter in Tasks 3–4.

**Files:**
- Modify: `krillnotes-core/src/core/workspace/sync.rs` (add helper method)
- Test: `krillnotes-core/src/core/workspace/tests.rs`

- [ ] **Step 1: Write test**

```rust
#[test]
fn test_is_field_local_only() {
    let temp = NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(
        temp.path(),
        "",
        "id-1",
        ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]),
        test_gate(),
        None,
    )
    .unwrap();

    let script = r#"
        schema("LoTest", #{
            version: 1,
            fields: [
                #{ name: "public_f", type: "text" },
                #{ name: "private_f", type: "text", local_only: true },
            ]
        });
    "#;
    ws.create_user_script("lo_test", script, "schema").unwrap();

    assert!(!ws.is_field_local_only("LoTest", "public_f"));
    assert!(ws.is_field_local_only("LoTest", "private_f"));
    // Unknown schema or field → false (safe default: don't suppress sync)
    assert!(!ws.is_field_local_only("UnknownSchema", "whatever"));
    assert!(!ws.is_field_local_only("LoTest", "unknown_field"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core test_is_field_local_only`
Expected: FAIL — method does not exist

- [ ] **Step 3: Implement `is_field_local_only`**

Add to `krillnotes-core/src/core/workspace/sync.rs` inside `impl Workspace`:

```rust
    /// Returns `true` if `field_name` is marked `local_only` in the schema for `schema_name`.
    /// Returns `false` if the schema or field is unknown (safe default: don't suppress sync).
    pub fn is_field_local_only(&self, schema_name: &str, field_name: &str) -> bool {
        self.script_registry
            .get_schema(schema_name)
            .ok()
            .and_then(|s| s.all_fields().into_iter().find(|f| f.name == field_name))
            .map_or(false, |f| f.local_only)
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p krillnotes-core test_is_field_local_only`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add krillnotes-core/src/core/workspace/sync.rs krillnotes-core/src/core/workspace/tests.rs
git commit -m "feat: add is_field_local_only helper for sync filtering"
```

---

## Task 3: Filter local-only `UpdateField` ops from `operations_since`

The existing `operations_since` already filters `RetractOperation { propagate: false }`. Extend this to also filter `UpdateField` ops that target local-only fields.

**Files:**
- Modify: `krillnotes-core/src/core/workspace/sync.rs:179-196` (`operations_since`)
- Modify: `krillnotes-core/src/core/workspace/sync.rs:284-301` (`operations_since_with_verified_by`)
- Test: `krillnotes-core/src/core/workspace/tests.rs`

- [ ] **Step 1: Write test — UpdateField on local_only field excluded from operations_since**

```rust
#[test]
fn test_operations_since_filters_local_only_update_field() {
    let temp = NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(
        temp.path(),
        "",
        "id-1",
        ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]),
        test_gate(),
        None,
    )
    .unwrap();

    let script = r#"
        schema("LoSync", #{
            version: 1,
            fields: [
                #{ name: "shared_text", type: "text" },
                #{ name: "private_text", type: "text", local_only: true },
            ]
        });
    "#;
    ws.create_user_script("lo_sync", script, "schema").unwrap();

    let root = ws.list_all_notes().unwrap()[0].clone();
    let note_id = ws
        .create_note(&root.id, AddPosition::AsChild, "LoSync")
        .unwrap();

    // Update both fields
    ws.update_note(
        &note_id,
        None,
        Some(vec![
            ("shared_text".to_string(), FieldValue::Text("hello".into())),
            ("private_text".to_string(), FieldValue::Text("secret".into())),
        ]),
        None,
    )
    .unwrap();

    let ops = ws.operations_since(None, "other-device").unwrap();
    for op in &ops {
        if let Operation::UpdateField { field, .. } = op {
            assert_ne!(
                field, "private_text",
                "local_only field must not appear in operations_since"
            );
        }
    }
    // Shared field should still be present
    assert!(
        ops.iter().any(|op| matches!(op, Operation::UpdateField { field, .. } if field == "shared_text")),
        "shared field must still appear"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core test_operations_since_filters_local_only_update_field`
Expected: FAIL — `private_text` op is not filtered yet

- [ ] **Step 3: Implement filtering in `operations_since`**

In `krillnotes-core/src/core/workspace/sync.rs`, replace the existing `ops.retain(...)` block in `operations_since` (around line 186-194) with:

```rust
        // Filter local-only retracts (propagate = false) and UpdateField ops
        // that target local_only schema fields.
        ops.retain(|op| match op {
            Operation::RetractOperation {
                propagate: false, ..
            } => false,
            Operation::UpdateField {
                note_id, field, ..
            } => !self.is_op_field_local_only(note_id, field),
            _ => true,
        });
```

- [ ] **Step 4: Add `is_op_field_local_only` helper**

This helper resolves the note's schema from the DB, then checks `is_field_local_only`. Add to `impl Workspace` in `sync.rs`:

```rust
    /// Checks if an UpdateField op targets a local_only field.
    /// Looks up the note's schema from the DB, then checks the field definition.
    fn is_op_field_local_only(&self, note_id: &str, field_name: &str) -> bool {
        self.get_note(note_id)
            .ok()
            .flatten()
            .map_or(false, |note| self.is_field_local_only(&note.schema, field_name))
    }
```

- [ ] **Step 5: Apply same filter to `operations_since_with_verified_by`**

Replace the existing `ops.retain(...)` block in `operations_since_with_verified_by` (around line 290-298) with:

```rust
        // Filter local-only retracts and UpdateField ops targeting local_only fields.
        ops.retain(|(op, _)| match op {
            Operation::RetractOperation {
                propagate: false, ..
            } => false,
            Operation::UpdateField {
                note_id, field, ..
            } => !self.is_op_field_local_only(note_id, field),
            _ => true,
        });
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo test -p krillnotes-core test_operations_since_filters_local_only_update_field`
Expected: PASS

- [ ] **Step 7: Run full test suite to check for regressions**

Run: `cargo test -p krillnotes-core`
Expected: All tests pass

- [ ] **Step 8: Commit**

```bash
git add krillnotes-core/src/core/workspace/sync.rs krillnotes-core/src/core/workspace/tests.rs
git commit -m "feat: filter local_only UpdateField ops from sync deltas"
```

---

## Task 4: Strip local-only field values from `CreateNote` ops in sync

When `CreateNote` is synced, its `fields` map should not contain local-only field values. Unlike `UpdateField` filtering (which drops entire ops), here we need to clone and modify the op before including it.

**Files:**
- Modify: `krillnotes-core/src/core/workspace/sync.rs:179-196` (`operations_since`)
- Modify: `krillnotes-core/src/core/workspace/sync.rs:284-301` (`operations_since_with_verified_by`)
- Test: `krillnotes-core/src/core/workspace/tests.rs`

- [ ] **Step 1: Write test — CreateNote op has local_only fields stripped**

```rust
#[test]
fn test_operations_since_strips_local_only_from_create_note() {
    let temp = NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(
        temp.path(),
        "",
        "id-1",
        ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]),
        test_gate(),
        None,
    )
    .unwrap();

    let script = r#"
        schema("LoCreate", #{
            version: 1,
            fields: [
                #{ name: "shared_f", type: "text" },
                #{ name: "private_f", type: "text", local_only: true },
            ]
        });
    "#;
    ws.create_user_script("lo_create", script, "schema").unwrap();

    let root = ws.list_all_notes().unwrap()[0].clone();
    let _note_id = ws
        .create_note(&root.id, AddPosition::AsChild, "LoCreate")
        .unwrap();

    let ops = ws.operations_since(None, "other-device").unwrap();
    for op in &ops {
        if let Operation::CreateNote { schema, fields, .. } = op {
            if schema == "LoCreate" {
                assert!(
                    !fields.contains_key("private_f"),
                    "local_only field must be stripped from CreateNote op"
                );
                assert!(
                    fields.contains_key("shared_f"),
                    "shared fields must remain in CreateNote op"
                );
            }
        }
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core test_operations_since_strips_local_only_from_create_note`
Expected: FAIL — `private_f` is still in the fields map

- [ ] **Step 3: Implement CreateNote field stripping**

Refactor the `operations_since` filter to use `filter_map` instead of `retain`, so we can both drop ops and modify ops. Replace the filter block with:

```rust
        // Filter local-only content from outbound operations:
        // 1. Drop RetractOperation { propagate: false }
        // 2. Drop UpdateField targeting local_only fields
        // 3. Strip local_only fields from CreateNote.fields
        let ops = ops
            .into_iter()
            .filter(|op| !matches!(
                op,
                Operation::RetractOperation { propagate: false, .. }
            ))
            .filter(|op| match op {
                Operation::UpdateField { note_id, field, .. } => {
                    !self.is_op_field_local_only(note_id, field)
                }
                _ => true,
            })
            .map(|op| match op {
                Operation::CreateNote {
                    schema, ref fields, ..
                } if self.has_local_only_fields(&schema) => {
                    self.strip_local_only_from_create_note(op)
                }
                other => other,
            })
            .collect();

        Ok(ops)
```

- [ ] **Step 4: Add `has_local_only_fields` and `strip_local_only_from_create_note` helpers**

```rust
    /// Returns `true` if any field in the schema is marked `local_only`.
    fn has_local_only_fields(&self, schema_name: &str) -> bool {
        self.script_registry
            .get_schema(schema_name)
            .ok()
            .map_or(false, |s| s.all_fields().iter().any(|f| f.local_only))
    }

    /// Clones a CreateNote op with local_only fields removed from its fields map.
    fn strip_local_only_from_create_note(&self, op: Operation) -> Operation {
        if let Operation::CreateNote {
            operation_id, timestamp, device_id, note_id, parent_id,
            position, schema, title, mut fields, created_by, signature,
        } = op
        {
            if let Ok(s) = self.script_registry.get_schema(&schema) {
                let local_names: Vec<&str> = s
                    .all_fields()
                    .iter()
                    .filter(|f| f.local_only)
                    .map(|f| f.name.as_str())
                    .collect();
                for name in local_names {
                    fields.remove(name);
                }
            }
            Operation::CreateNote {
                operation_id, timestamp, device_id, note_id, parent_id,
                position, schema, title, fields, created_by, signature,
            }
        } else {
            op
        }
    }
```

- [ ] **Step 5: Apply same changes to `operations_since_with_verified_by`**

Replace the filter block with the same pattern, adapted for the `(Operation, String)` tuple:

```rust
        let ops = ops
            .into_iter()
            .filter(|(op, _)| !matches!(
                op,
                Operation::RetractOperation { propagate: false, .. }
            ))
            .filter(|(op, _)| match op {
                Operation::UpdateField { note_id, field, .. } => {
                    !self.is_op_field_local_only(note_id, field)
                }
                _ => true,
            })
            .map(|(op, vb)| match op {
                Operation::CreateNote {
                    ref schema, ..
                } if self.has_local_only_fields(schema) => {
                    (self.strip_local_only_from_create_note(op), vb)
                }
                _ => (op, vb),
            })
            .collect();

        Ok(ops)
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo test -p krillnotes-core test_operations_since_strips_local_only_from_create_note`
Expected: PASS

- [ ] **Step 7: Run full test suite**

Run: `cargo test -p krillnotes-core`
Expected: All tests pass

- [ ] **Step 8: Commit**

```bash
git add krillnotes-core/src/core/workspace/sync.rs krillnotes-core/src/core/workspace/tests.rs
git commit -m "feat: strip local_only fields from CreateNote ops in sync"
```

---

## Task 5: Strip local-only fields from snapshot notes

When building `WorkspaceSnapshot`, strip local-only field values from each note's `fields` map.

**Files:**
- Modify: `krillnotes-core/src/core/workspace/sync.rs:19-36` (`to_snapshot_json`)
- Test: `krillnotes-core/src/core/workspace/tests.rs`

- [ ] **Step 1: Write test — snapshot notes have local_only fields stripped**

```rust
#[test]
fn test_snapshot_strips_local_only_fields() {
    let temp = NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(
        temp.path(),
        "",
        "id-1",
        ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]),
        test_gate(),
        None,
    )
    .unwrap();

    let script = r#"
        schema("LoSnap", #{
            version: 1,
            fields: [
                #{ name: "shared", type: "text" },
                #{ name: "private", type: "text", local_only: true },
            ]
        });
    "#;
    ws.create_user_script("lo_snap", script, "schema").unwrap();

    let root = ws.list_all_notes().unwrap()[0].clone();
    let note_id = ws
        .create_note(&root.id, AddPosition::AsChild, "LoSnap")
        .unwrap();
    ws.update_note(
        &note_id,
        None,
        Some(vec![
            ("shared".to_string(), FieldValue::Text("visible".into())),
            ("private".to_string(), FieldValue::Text("secret".into())),
        ]),
        None,
    )
    .unwrap();

    let snapshot_bytes = ws.to_snapshot_json().unwrap();
    let snapshot: serde_json::Value = serde_json::from_slice(&snapshot_bytes).unwrap();
    let notes = snapshot["notes"].as_array().unwrap();
    let lo_note = notes.iter().find(|n| n["id"] == note_id).unwrap();
    let fields = lo_note["fields"].as_object().unwrap();
    assert!(fields.contains_key("shared"), "shared field must be in snapshot");
    assert!(!fields.contains_key("private"), "local_only field must be stripped from snapshot");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p krillnotes-core test_snapshot_strips_local_only_fields`
Expected: FAIL — `private` field is present in snapshot

- [ ] **Step 3: Implement field stripping in `to_snapshot_json`**

In `krillnotes-core/src/core/workspace/sync.rs`, modify `to_snapshot_json`:

```rust
    pub fn to_snapshot_json(&self) -> Result<Vec<u8>> {
        log::info!(target: "krillnotes::sync", "generating snapshot JSON");
        let notes = self.list_all_notes()?;
        let user_scripts = self.list_user_scripts()?;
        let attachments = self.list_all_attachments()?;
        let permission_ops = self.collect_permission_ops()?;

        // Strip local_only field values from each note
        let notes: Vec<Note> = notes
            .into_iter()
            .map(|mut note| {
                if let Ok(schema) = self.script_registry.get_schema(&note.schema) {
                    let local_fields: Vec<String> = schema
                        .all_fields()
                        .iter()
                        .filter(|f| f.local_only)
                        .map(|f| f.name.clone())
                        .collect();
                    for name in &local_fields {
                        note.fields.remove(name);
                    }
                }
                note
            })
            .collect();

        log::debug!(target: "krillnotes::sync",
            "snapshot: {} notes, {} scripts, {} attachments, {} permission ops",
            notes.len(), user_scripts.len(), attachments.len(), permission_ops.len());
        let snapshot = WorkspaceSnapshot {
            version: 1,
            notes,
            user_scripts,
            attachments,
            permission_ops,
        };
        Ok(serde_json::to_vec(&snapshot)?)
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p krillnotes-core test_snapshot_strips_local_only_fields`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add krillnotes-core/src/core/workspace/sync.rs krillnotes-core/src/core/workspace/tests.rs
git commit -m "feat: strip local_only fields from workspace snapshots"
```

---

## Task 6: Add `include_local_only` option to export

**Files:**
- Modify: `krillnotes-core/src/core/export.rs:197-303`
- Modify: `krillnotes-core/src/lib.rs:26` (re-export)
- Modify: `krillnotes-desktop/src-tauri/src/commands/workspace.rs:734-753`
- Test: `krillnotes-core/src/core/workspace/tests.rs`

- [ ] **Step 1: Write test — export without local_only strips fields**

```rust
#[test]
fn test_export_strips_local_only_when_flag_false() {
    use std::io::Cursor;

    let temp = NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(
        temp.path(),
        "",
        "id-1",
        ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]),
        test_gate(),
        None,
    )
    .unwrap();

    let script = r#"
        schema("LoExport", #{
            version: 1,
            fields: [
                #{ name: "shared", type: "text" },
                #{ name: "private", type: "text", local_only: true },
            ]
        });
    "#;
    ws.create_user_script("lo_export", script, "schema").unwrap();

    let root = ws.list_all_notes().unwrap()[0].clone();
    let note_id = ws
        .create_note(&root.id, AddPosition::AsChild, "LoExport")
        .unwrap();
    ws.update_note(
        &note_id,
        None,
        Some(vec![
            ("shared".to_string(), FieldValue::Text("visible".into())),
            ("private".to_string(), FieldValue::Text("secret".into())),
        ]),
        None,
    )
    .unwrap();

    // Export WITHOUT local-only data
    let mut buf = Cursor::new(Vec::new());
    crate::export_workspace(&ws, &mut buf, None, false).unwrap();

    // Read back
    buf.set_position(0);
    let mut zip = zip::ZipArchive::new(buf).unwrap();
    let notes_json: serde_json::Value =
        serde_json::from_reader(zip.by_name("notes.json").unwrap()).unwrap();
    let notes = notes_json["notes"].as_array().unwrap();
    let lo_note = notes.iter().find(|n| n["id"] == note_id).unwrap();
    let fields = lo_note["fields"].as_object().unwrap();
    assert!(fields.contains_key("shared"));
    assert!(!fields.contains_key("private"), "local_only field must be stripped when include_local_only=false");
}

#[test]
fn test_export_includes_local_only_when_flag_true() {
    use std::io::Cursor;

    let temp = NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(
        temp.path(),
        "",
        "id-1",
        ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]),
        test_gate(),
        None,
    )
    .unwrap();

    let script = r#"
        schema("LoExport2", #{
            version: 1,
            fields: [
                #{ name: "shared", type: "text" },
                #{ name: "private", type: "text", local_only: true },
            ]
        });
    "#;
    ws.create_user_script("lo_export2", script, "schema").unwrap();

    let root = ws.list_all_notes().unwrap()[0].clone();
    let note_id = ws
        .create_note(&root.id, AddPosition::AsChild, "LoExport2")
        .unwrap();
    ws.update_note(
        &note_id,
        None,
        Some(vec![
            ("shared".to_string(), FieldValue::Text("visible".into())),
            ("private".to_string(), FieldValue::Text("secret".into())),
        ]),
        None,
    )
    .unwrap();

    // Export WITH local-only data
    let mut buf = Cursor::new(Vec::new());
    crate::export_workspace(&ws, &mut buf, None, true).unwrap();

    buf.set_position(0);
    let mut zip = zip::ZipArchive::new(buf).unwrap();
    let notes_json: serde_json::Value =
        serde_json::from_reader(zip.by_name("notes.json").unwrap()).unwrap();
    let notes = notes_json["notes"].as_array().unwrap();
    let lo_note = notes.iter().find(|n| n["id"] == note_id).unwrap();
    let fields = lo_note["fields"].as_object().unwrap();
    assert!(fields.contains_key("shared"));
    assert!(fields.contains_key("private"), "local_only field must be present when include_local_only=true");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p krillnotes-core test_export_strips_local_only_when_flag_false test_export_includes_local_only_when_flag_true`
Expected: FAIL — `export_workspace` does not accept 4 args

- [ ] **Step 3: Add `include_local_only` parameter to `export_workspace`**

In `krillnotes-core/src/core/export.rs`, change the signature:

```rust
pub fn export_workspace<W: Write + Seek>(
    workspace: &Workspace,
    writer: W,
    password: Option<&str>,
    include_local_only: bool,
) -> Result<(), ExportError> {
```

After the identity stripping block (line ~225, the `.collect()` after clearing `created_by`/`modified_by`), add local-only field stripping:

```rust
    // Optionally strip local_only field values
    let notes: Vec<Note> = if include_local_only {
        notes
    } else {
        notes
            .into_iter()
            .map(|mut note| {
                if let Ok(schema) = workspace.script_registry().get_schema(&note.schema) {
                    for fd in schema.all_fields() {
                        if fd.local_only {
                            note.fields.remove(&fd.name);
                        }
                    }
                }
                note
            })
            .collect()
    };
```

- [ ] **Step 4: Update all callers of `export_workspace`**

In `krillnotes-core/src/lib.rs`, the re-export just passes through the function — no change needed there if it's a plain `pub use`. Update the Tauri command:

In `krillnotes-desktop/src-tauri/src/commands/workspace.rs:734`, add the parameter:

```rust
pub fn export_workspace_cmd(
    window: tauri::Window,
    state: State<'_, AppState>,
    path: String,
    password: Option<String>,
    include_local_only: bool,
) -> std::result::Result<(), String> {
    // ... existing code ...
    krillnotes_core::export_workspace(workspace, file, password.as_deref(), include_local_only)
        .map_err(|e| {
            log::error!("export_workspace failed: {e}");
            e.to_string()
        })
}
```

- [ ] **Step 5: Fix any other callers**

Search for other callers: `grep -rn "export_workspace(" krillnotes-core/` — update tests or internal callers to pass `true` (include all) as default for backward compat in tests.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p krillnotes-core test_export_strips_local_only_when_flag_false test_export_includes_local_only_when_flag_true`
Expected: PASS

- [ ] **Step 7: Run full test suite**

Run: `cargo test -p krillnotes-core`
Expected: All tests pass (existing export tests may need `true` added as 4th arg)

- [ ] **Step 8: Commit**

```bash
git add krillnotes-core/src/core/export.rs krillnotes-core/src/lib.rs krillnotes-core/src/core/workspace/tests.rs krillnotes-desktop/src-tauri/src/commands/workspace.rs
git commit -m "feat: add include_local_only option to workspace export"
```

---

## Task 7: Frontend — TypeScript type + UI indicator

**Files:**
- Modify: `krillnotes-desktop/src/types.ts:57-69`
- Modify: `krillnotes-desktop/src/components/InfoPanel.tsx:501-516`
- Modify: `krillnotes-desktop/src/i18n/locales/en.json`
- Modify: `krillnotes-desktop/src/i18n/locales/{de,es,fr,ja,ko,zh}.json`

- [ ] **Step 1: Add `localOnly` to TypeScript `FieldDefinition`**

In `krillnotes-desktop/src/types.ts`, add after `hasValidate`:

```typescript
export interface FieldDefinition {
  name: string;
  fieldType: FieldType;
  required: boolean;
  canView: boolean;
  canEdit: boolean;
  options: string[];
  max: number;
  targetSchema?: string;
  showOnHover: boolean;
  allowedTypes: string[];
  hasValidate: boolean;
  localOnly: boolean;
}
```

- [ ] **Step 2: Add i18n key for local-only indicator**

In `krillnotes-desktop/src/i18n/locales/en.json`, add under the `"fields"` section:

```json
"localOnlyHint": "Local only — not synced to peers"
```

Add translated equivalents to all 6 other locale files:
- `de.json`: `"localOnlyHint": "Nur lokal — wird nicht mit Peers synchronisiert"`
- `es.json`: `"localOnlyHint": "Solo local — no se sincroniza con pares"`
- `fr.json`: `"localOnlyHint": "Local uniquement — non synchronisé avec les pairs"`
- `ja.json`: `"localOnlyHint": "ローカルのみ — ピアとは同期されません"`
- `ko.json`: `"localOnlyHint": "로컬 전용 — 피어와 동기화되지 않음"`
- `zh.json`: `"localOnlyHint": "仅本地 — 不会同步到对等节点"`

- [ ] **Step 3: Show local-only indicator on field labels in InfoPanel**

In `krillnotes-desktop/src/components/InfoPanel.tsx`, modify the `FieldEditor` rendering for top-level fields (around line 501-516). Wrap the `FieldEditor` in a container that shows a local-only badge when applicable.

Find the top-level field editor loop:

```tsx
{schemaInfo.fields.filter(field => field.canEdit).map(field => (
  <FieldEditor
    key={field.name}
    ...
  />
))}
```

Replace with:

```tsx
{schemaInfo.fields.filter(field => field.canEdit).map(field => (
  <div key={field.name}>
    {field.localOnly && (
      <span
        className="inline-flex items-center gap-1 text-xs text-muted-foreground mb-0.5"
        title={t('fields.localOnlyHint')}
      >
        <svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 24 24"
          fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <rect width="18" height="11" x="3" y="11" rx="2" ry="2"/>
          <path d="M7 11V7a5 5 0 0 1 10 0v4"/>
        </svg>
        {t('fields.localOnlyHint')}
      </span>
    )}
    <FieldEditor
      fieldName={field.name}
      fieldType={field.fieldType}
      value={editedFields[field.name] ?? defaultValueForFieldType(field.fieldType)}
      required={field.required}
      options={field.options}
      max={field.max}
      targetSchema={field.targetSchema}
      noteId={selectedNote.id}
      fieldDef={field}
      error={fieldErrors[field.name]}
      onBlur={() => handleFieldBlur(field.name, field)}
      onChange={(value) => handleFieldChange(field.name, value)}
    />
  </div>
))}
```

Apply the same pattern to:
- Group fields loop (~line 541)
- Read-only FieldDisplay sections (~line 565+)

- [ ] **Step 4: TypeScript type check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: No errors

- [ ] **Step 5: Commit**

```bash
git add krillnotes-desktop/src/types.ts krillnotes-desktop/src/components/InfoPanel.tsx krillnotes-desktop/src/i18n/locales/*.json
git commit -m "feat: show local-only indicator on field labels in UI"
```

---

## Task 8: Frontend — Export dialog checkbox

**Files:**
- Modify: `krillnotes-desktop/src/App.tsx:260-310`
- Modify: `krillnotes-desktop/src/i18n/locales/en.json`
- Modify: `krillnotes-desktop/src/i18n/locales/{de,es,fr,ja,ko,zh}.json`

- [ ] **Step 1: Add i18n keys for export checkbox**

In `en.json`, add under `"dialogs" > "password"` section:

```json
"includeLocalOnly": "Include local-only field data",
"includeLocalOnlyHint": "When unchecked, fields marked as local-only will be empty in the export."
```

Add translated equivalents to all 6 other locale files:
- `de.json`: `"includeLocalOnly": "Lokale Felddaten einbeziehen"`, `"includeLocalOnlyHint": "Wenn deaktiviert, sind als lokal markierte Felder im Export leer."`
- `es.json`: `"includeLocalOnly": "Incluir datos de campos locales"`, `"includeLocalOnlyHint": "Cuando está desmarcado, los campos marcados como solo locales estarán vacíos en la exportación."`
- `fr.json`: `"includeLocalOnly": "Inclure les données des champs locaux"`, `"includeLocalOnlyHint": "Si décoché, les champs marqués comme locaux uniquement seront vides dans l'export."`
- `ja.json`: `"includeLocalOnly": "ローカル専用フィールドデータを含める"`, `"includeLocalOnlyHint": "チェックを外すと、ローカル専用のフィールドはエクスポートで空になります。"`
- `ko.json`: `"includeLocalOnly": "로컬 전용 필드 데이터 포함"`, `"includeLocalOnlyHint": "선택 해제 시 로컬 전용으로 표시된 필드는 내보내기에서 비어 있습니다."`
- `zh.json`: `"includeLocalOnly": "包含仅本地字段数据"`, `"includeLocalOnlyHint": "取消勾选后，标记为仅本地的字段在导出中将为空。"`

- [ ] **Step 2: Add checkbox state to export dialog in App.tsx**

In `krillnotes-desktop/src/App.tsx`, find the export state variables near line 50-55 and add:

```typescript
const [exportIncludeLocalOnly, setExportIncludeLocalOnly] = useState(true);
```

- [ ] **Step 3: Add checkbox UI to export dialog**

In the export password dialog (around line 280, after the confirm password input and before the mismatch warning), add:

```tsx
<label className="flex items-start gap-2 mb-4 cursor-pointer">
  <input
    type="checkbox"
    checked={exportIncludeLocalOnly}
    onChange={(e) => setExportIncludeLocalOnly(e.target.checked)}
    className="mt-0.5"
  />
  <div>
    <span className="text-sm font-medium">{t('dialogs.password.includeLocalOnly')}</span>
    <p className="text-xs text-muted-foreground">{t('dialogs.password.includeLocalOnlyHint')}</p>
  </div>
</label>
```

- [ ] **Step 4: Pass `includeLocalOnly` to the Tauri command**

In the `handleExportConfirm` function (around line 132-150), update the invoke call:

```typescript
await invoke('export_workspace_cmd', {
  path,
  password,
  includeLocalOnly: exportIncludeLocalOnly,
});
```

- [ ] **Step 5: TypeScript type check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: No errors

- [ ] **Step 6: Commit**

```bash
git add krillnotes-desktop/src/App.tsx krillnotes-desktop/src/i18n/locales/*.json
git commit -m "feat: add 'include local-only data' checkbox to export dialog"
```

---

## Task 9: Update SCRIPTING.md documentation

**Files:**
- Modify: `SCRIPTING.md`

- [ ] **Step 1: Add `local_only` to field property documentation**

In `SCRIPTING.md`, find the section documenting field properties (the table or list that explains `name`, `type`, `required`, etc.) and add:

```markdown
| `local_only` | `bool` | `false` | When `true`, this field's values are never synced to peers. Each peer stores their own independent value locally. Useful for personal annotations, draft notes, or private ratings on shared data. |
```

- [ ] **Step 2: Add a usage example**

Add an example section:

```markdown
### Local-Only Fields

Fields marked `local_only: true` are visible to all peers (the schema is shared), but each peer stores their own value independently. Values are never included in sync deltas, snapshots, or exports (unless the user opts in during export).

```rhai
schema("SharedDocument", #{
    version: 1,
    fields: [
        #{ name: "content",    type: "textarea" },
        #{ name: "status",     type: "select", options: ["Draft", "Review", "Final"] },
        #{ name: "my_notes",   type: "textarea", local_only: true },
        #{ name: "my_rating",  type: "rating", max: 5, local_only: true },
    ]
});
```​
```

- [ ] **Step 3: Commit**

```bash
git add SCRIPTING.md
git commit -m "docs: document local_only field property in SCRIPTING.md"
```

---

## Task 10: Integration test — full round-trip

**Files:**
- Test: `krillnotes-core/src/core/workspace/tests.rs`

- [ ] **Step 1: Write full integration test**

This test creates a workspace, adds a schema with local-only fields, creates a note, updates both field types, and verifies all three sync boundaries filter correctly.

```rust
#[test]
fn test_local_only_field_full_round_trip() {
    let temp = NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(
        temp.path(),
        "",
        "id-1",
        ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]),
        test_gate(),
        None,
    )
    .unwrap();

    let script = r#"
        schema("RoundTrip", #{
            version: 1,
            fields: [
                #{ name: "title_field", type: "text" },
                #{ name: "my_notes", type: "text", local_only: true },
                #{ name: "my_rating", type: "rating", max: 5, local_only: true },
            ]
        });
    "#;
    ws.create_user_script("rt_test", script, "schema").unwrap();

    let root = ws.list_all_notes().unwrap()[0].clone();
    let note_id = ws
        .create_note(&root.id, AddPosition::AsChild, "RoundTrip")
        .unwrap();
    ws.update_note(
        &note_id,
        None,
        Some(vec![
            ("title_field".to_string(), FieldValue::Text("hello".into())),
            ("my_notes".to_string(), FieldValue::Text("private note".into())),
            ("my_rating".to_string(), FieldValue::Number(4.0)),
        ]),
        None,
    )
    .unwrap();

    // 1. Verify local state has all fields
    let note = ws.get_note(&note_id).unwrap().unwrap();
    assert_eq!(note.fields.get("my_notes"), Some(&FieldValue::Text("private note".into())));
    assert_eq!(note.fields.get("my_rating"), Some(&FieldValue::Number(4.0)));

    // 2. Verify operations_since filters local-only UpdateField ops
    let ops = ws.operations_since(None, "other-device").unwrap();
    for op in &ops {
        if let Operation::UpdateField { field, .. } = op {
            assert!(
                field != "my_notes" && field != "my_rating",
                "local_only field '{}' leaked into operations_since", field
            );
        }
        // Also verify CreateNote has fields stripped
        if let Operation::CreateNote { schema, fields, .. } = op {
            if schema == "RoundTrip" {
                assert!(!fields.contains_key("my_notes"));
                assert!(!fields.contains_key("my_rating"));
                assert!(fields.contains_key("title_field"));
            }
        }
    }

    // 3. Verify snapshot strips local-only fields
    let snapshot_bytes = ws.to_snapshot_json().unwrap();
    let snapshot: serde_json::Value = serde_json::from_slice(&snapshot_bytes).unwrap();
    let notes = snapshot["notes"].as_array().unwrap();
    let rt_note = notes.iter().find(|n| n["id"] == note_id).unwrap();
    let fields = rt_note["fields"].as_object().unwrap();
    assert!(!fields.contains_key("my_notes"));
    assert!(!fields.contains_key("my_rating"));
    assert!(fields.contains_key("title_field"));

    // 4. Verify undo still works locally
    ws.undo().unwrap();
    let note = ws.get_note(&note_id).unwrap().unwrap();
    // After undo, the last update is reverted
    assert!(
        note.fields.get("my_notes") != Some(&FieldValue::Text("private note".into()))
            || note.fields.get("title_field") != Some(&FieldValue::Text("hello".into())),
        "undo should revert the last update"
    );
}
```

- [ ] **Step 2: Run the integration test**

Run: `cargo test -p krillnotes-core test_local_only_field_full_round_trip`
Expected: PASS

- [ ] **Step 3: Run the full test suite one final time**

Run: `cargo test -p krillnotes-core`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add krillnotes-core/src/core/workspace/tests.rs
git commit -m "test: add full round-trip integration test for local_only fields"
```
