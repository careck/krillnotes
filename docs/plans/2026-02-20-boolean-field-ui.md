# Boolean Field UI Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make boolean fields render as checkboxes in edit mode and as colored check/X icons in view mode, and fix the backend `Dynamic::UNIT` handling that currently breaks the on-save hook for any contact note missing an optional boolean field.

**Architecture:** Three layers of fixes: (1) Rust backend — `dynamic_to_field_value` in `hooks.rs` must return type defaults for `Dynamic::UNIT` inputs; (2) TypeScript frontend — `InfoPanel` initializes `editedFields` from schema-typed defaults on schema load; (3) UI — `FieldDisplay` replaces the disabled checkbox with lucide-react icons.

**Tech Stack:** Rust (cargo test), React 19, TypeScript, Tailwind CSS, lucide-react, Tauri 2

---

### Task 1: Fix `dynamic_to_field_value` — handle `Dynamic::UNIT` for all types

The `"boolean"`, `"text"`, `"number"`, and `"email"` arms of `dynamic_to_field_value` in `hooks.rs` do not handle `Dynamic::UNIT` (the Rhai nil value). When an optional schema field is absent from the submitted fields map, the hook receives `Dynamic::UNIT` and errors. The `"date"` arm already handles this correctly (lines 173-183) — the others must match.

**Files:**
- Modify: `krillnotes-core/src/core/scripting/hooks.rs:152-193`
- Test: `krillnotes-core/src/core/scripting/mod.rs`

**Step 1: Write a failing test**

In `krillnotes-core/src/core/scripting/mod.rs`, add this test inside the `#[cfg(test)]` block (after the last existing test):

```rust
#[test]
fn test_boolean_field_defaults_to_false_when_absent_from_hook_result() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry
        .load_script(
            r#"
            schema("FlagNote", #{
                fields: [
                    #{ name: "flag", type: "boolean", required: false },
                ]
            });
            on_save("FlagNote", |note| {
                // intentionally does NOT touch note.fields["flag"]
                note
            });
        "#,
        )
        .unwrap();

    // Do NOT include "flag" in the submitted fields — it must default to false.
    let fields = HashMap::new();

    let result = registry
        .run_on_save_hook("FlagNote", "id-1", "FlagNote", "title", &fields)
        .unwrap()
        .unwrap();

    assert_eq!(
        result.1.get("flag"),
        Some(&FieldValue::Boolean(false)),
        "boolean field absent from hook result should default to false"
    );
}
```

**Step 2: Run test to confirm it fails**

```bash
cargo test --manifest-path krillnotes-core/Cargo.toml test_boolean_field_defaults_to_false_when_absent_from_hook_result 2>&1 | tail -15
```

Expected: FAIL with `"boolean field must be a bool"`

**Step 3: Fix `dynamic_to_field_value` in `hooks.rs`**

In `krillnotes-core/src/core/scripting/hooks.rs`, update the four arms that do not yet handle `Dynamic::UNIT`. Replace lines 154-191 with:

```rust
        "text" => {
            if d.is_unit() {
                return Ok(FieldValue::Text(String::new()));
            }
            let s = d
                .try_cast::<String>()
                .ok_or_else(|| KrillnotesError::Scripting("text field must be a string".into()))?;
            Ok(FieldValue::Text(s))
        }
        "number" => {
            if d.is_unit() {
                return Ok(FieldValue::Number(0.0));
            }
            let n = d
                .try_cast::<f64>()
                .ok_or_else(|| KrillnotesError::Scripting("number field must be a float".into()))?;
            Ok(FieldValue::Number(n))
        }
        "boolean" => {
            if d.is_unit() {
                return Ok(FieldValue::Boolean(false));
            }
            let b = d
                .try_cast::<bool>()
                .ok_or_else(|| KrillnotesError::Scripting("boolean field must be a bool".into()))?;
            Ok(FieldValue::Boolean(b))
        }
        "date" => {
            if d.is_unit() {
                Ok(FieldValue::Date(None))
            } else {
                let s = d.try_cast::<String>().ok_or_else(|| {
                    KrillnotesError::Scripting("date field must be a string or ()".into())
                })?;
                let nd = NaiveDate::parse_from_str(&s, "%Y-%m-%d").map_err(|e| {
                    KrillnotesError::Scripting(format!("invalid date '{}': {}", s, e))
                })?;
                Ok(FieldValue::Date(Some(nd)))
            }
        }
        "email" => {
            if d.is_unit() {
                return Ok(FieldValue::Email(String::new()));
            }
            let s = d
                .try_cast::<String>()
                .ok_or_else(|| KrillnotesError::Scripting("email field must be a string".into()))?;
            Ok(FieldValue::Email(s))
        }
```

**Step 4: Run new test to confirm it passes**

```bash
cargo test --manifest-path krillnotes-core/Cargo.toml test_boolean_field_defaults_to_false_when_absent_from_hook_result 2>&1 | tail -5
```

Expected: PASS

---

### Task 2: Fix existing Contact tests broken by the new `is_family` field

The Contact schema now has 12 fields (was 11). Three tests pass field maps without `is_family`; one test asserts the old count of 11.

**Files:**
- Modify: `krillnotes-core/src/core/scripting/mod.rs:282`
- Modify: `krillnotes-core/src/core/scripting/mod.rs:338-361`
- Modify: `krillnotes-core/src/core/workspace.rs:1400-1412`
- Modify: `krillnotes-core/src/core/workspace.rs:1493-1504`

**Step 1: Update field count assertion**

In `mod.rs`, the test `test_contact_schema_loaded` at line 282 asserts `schema.fields.len() == 11`. Change it to `12` and add an assertion for the new field:

```rust
assert_eq!(schema.fields.len(), 12);
let is_family_field = schema.fields.iter().find(|f| f.name == "is_family").unwrap();
assert_eq!(is_family_field.field_type, "boolean");
assert!(!is_family_field.required, "is_family should not be required");
```

**Step 2: Add `is_family` to `test_contact_on_save_hook_derives_title` (mod.rs ~line 342)**

After the existing `fields.insert("address_country"...)` line, add:

```rust
fields.insert("is_family".to_string(), FieldValue::Boolean(false));
```

**Step 3: Add `is_family` to `test_update_contact_rejects_empty_required_fields` (workspace.rs ~line 1401)**

After the existing `fields.insert("address_country"...)` line, add:

```rust
fields.insert("is_family".to_string(), FieldValue::Boolean(false));
```

**Step 4: Add `is_family` to `test_update_contact_derives_title_from_hook` (workspace.rs ~line 1493)**

After the existing `fields.insert("address_country"...)` line, add:

```rust
fields.insert("is_family".to_string(), FieldValue::Boolean(false));
```

**Step 5: Run all Rust tests to confirm all pass**

```bash
cargo test --manifest-path krillnotes-core/Cargo.toml 2>&1 | tail -10
```

Expected: `test result: ok. N passed; 0 failed`

**Step 6: Commit**

```bash
git add krillnotes-core/src/core/scripting/hooks.rs \
        krillnotes-core/src/core/scripting/mod.rs \
        krillnotes-core/src/core/workspace.rs
git commit -m "fix: handle Dynamic::UNIT as type default in hook field conversion, update Contact tests for is_family field"
```

---

### Task 3: Install lucide-react

**Files:**
- Modify: `krillnotes-desktop/package.json` (via npm install)

**Step 1: Install the package**

```bash
npm install lucide-react --prefix krillnotes-desktop
```

Expected: package added to `dependencies` in `package.json`.

**Step 2: Verify the build still compiles**

```bash
cd krillnotes-desktop && npm run build 2>&1 | tail -10
```

Expected: build succeeds with no errors.

**Step 3: Commit**

```bash
git add krillnotes-desktop/package.json krillnotes-desktop/package-lock.json
git commit -m "chore: add lucide-react icon library"
```

---

### Task 4: Update `FieldDisplay` to show icons for boolean values

**Files:**
- Modify: `krillnotes-desktop/src/components/FieldDisplay.tsx:1,18-29`

**Step 1: Replace the boolean rendering block**

In `FieldDisplay.tsx`, change the import at line 1 to add lucide-react:

```typescript
import { Check, X } from 'lucide-react';
import type { FieldValue } from '../types';
```

Then replace the boolean rendering block (lines 18-29):

```typescript
    } else if ('Boolean' in value) {
      return value.Boolean
        ? <Check size={18} className="text-green-500" />
        : <X size={18} className="text-red-500" />;
```

**Step 2: Build to confirm no TypeScript errors**

```bash
cd krillnotes-desktop && npm run build 2>&1 | tail -10
```

Expected: build succeeds.

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/FieldDisplay.tsx
git commit -m "feat: display boolean fields as green check / red X icons in view mode"
```

---

### Task 5: Schema-aware field initialization in `InfoPanel`

When the schema loads, merge schema-typed defaults into `editedFields` for any field not yet present in the note. Also fix the two render-time fallbacks to use `??` and the same helper.

**Files:**
- Modify: `krillnotes-desktop/src/components/InfoPanel.tsx`

**Step 1: Add the `defaultValueForFieldType` helper**

Add this function just before the `InfoPanel` component declaration (before line 14):

```typescript
function defaultValueForFieldType(fieldType: string): FieldValue {
  switch (fieldType) {
    case 'boolean': return { Boolean: false };
    case 'number':  return { Number: 0 };
    case 'date':    return { Date: null };
    case 'email':   return { Email: '' };
    default:        return { Text: '' };
  }
}
```

**Step 2: Merge schema defaults on schema load**

In the `get_schema_fields` `.then()` callback (around line 41), after `setSchemaInfo(info)`, add a `setEditedFields` merge that fills in any missing typed defaults:

Replace:
```typescript
      .then(info => {
        setSchemaInfo(info);
        schemaLoadedRef.current = true;
```

With:
```typescript
      .then(info => {
        setSchemaInfo(info);
        setEditedFields(prev => {
          const merged = { ...prev };
          for (const field of info.fields) {
            if (!(field.name in merged)) {
              merged[field.name] = defaultValueForFieldType(field.fieldType);
            }
          }
          return merged;
        });
        schemaLoadedRef.current = true;
```

**Step 3: Fix render-time fallbacks to use `??`**

At line 220 (edit mode field value):
```typescript
value={editedFields[field.name] ?? defaultValueForFieldType(field.fieldType)}
```

At line 228 (view mode field value):
```typescript
value={selectedNote.fields[field.name] ?? defaultValueForFieldType(field.fieldType)}
```

**Step 4: Build to confirm no TypeScript errors**

```bash
cd krillnotes-desktop && npm run build 2>&1 | tail -10
```

Expected: build succeeds.

**Step 5: Commit**

```bash
git add krillnotes-desktop/src/components/InfoPanel.tsx
git commit -m "feat: initialize boolean (and all typed) fields from schema defaults in InfoPanel"
```

---

### Task 6: Manual verification

Start the app and verify end-to-end:

```bash
cd krillnotes-desktop && npm run tauri dev
```

1. Open or create a Contact note
2. **View mode**: `is_family` field shows a red X icon (false by default)
3. **Edit mode**: `is_family` field shows a checkbox (unchecked by default)
4. Check the checkbox and Save — view mode now shows a green check icon
5. Reload / reopen the note — boolean value persists correctly
