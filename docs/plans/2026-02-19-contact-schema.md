# Contact Schema + New Field Types Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add `Date` and `Email` field types to the schema system and a built-in Contact schema with 11 fields, plus basic frontend rendering for the new types.

**Architecture:** Extend the `FieldValue` Rust enum with `Date(Option<NaiveDate>)` and `Email(String)` variants; add a new `contact.rhai` system script loaded at startup alongside `text_note.rhai`; extend the TypeScript `FieldValue` discriminated union and add rendering/editing cases in `FieldDisplay` and `FieldEditor`.

**Tech Stack:** Rust (chrono already a workspace dep with serde feature), Rhai 1.24, TypeScript + React, Vite/TSC for type-checking.

---

### Task 1: Add `Date` and `Email` default-value tests (failing)

**Files:**
- Modify: `krillnotes-core/src/core/scripting.rs`

**Step 1: Add two failing tests in the `tests` module at the bottom of scripting.rs**

The tests module already exists. Add these two new tests inside the existing `#[cfg(test)] mod tests { ... }` block:

```rust
#[test]
fn test_date_field_default() {
    let schema = Schema {
        name: "Test".to_string(),
        fields: vec![FieldDefinition {
            name: "birthday".to_string(),
            field_type: "date".to_string(),
            required: false,
        }],
    };
    let defaults = schema.default_fields();
    assert!(matches!(defaults.get("birthday"), Some(FieldValue::Date(None))));
}

#[test]
fn test_email_field_default() {
    let schema = Schema {
        name: "Test".to_string(),
        fields: vec![FieldDefinition {
            name: "email_addr".to_string(),
            field_type: "email".to_string(),
            required: false,
        }],
    };
    let defaults = schema.default_fields();
    assert!(matches!(defaults.get("email_addr"), Some(FieldValue::Email(_))));
}
```

**Step 2: Run tests — expect compile error (variants don't exist yet)**

```bash
cargo test -p krillnotes-core
```

Expected: compile error mentioning `FieldValue::Date` and `FieldValue::Email` don't exist.

---

### Task 2: Add `Date` and `Email` variants to `FieldValue`

**Files:**
- Modify: `krillnotes-core/src/core/note.rs`
- Modify: `krillnotes-core/src/core/scripting.rs`

**Step 1: Add `use chrono::NaiveDate;` and the two new variants to `note.rs`**

Add the import at the top of `note.rs` (after the existing `use` statements):

```rust
use chrono::NaiveDate;
```

Extend the `FieldValue` enum (add two new variants after `Boolean`):

```rust
pub enum FieldValue {
    Text(String),
    Number(f64),
    Boolean(bool),
    /// A calendar date. `None` represents "not set".
    /// Serializes as ISO 8601 `"YYYY-MM-DD"` or JSON `null`.
    Date(Option<NaiveDate>),
    /// An email address string. Format is validated client-side.
    Email(String),
}
```

**Step 2: Add two new match arms in `Schema::default_fields()` in `scripting.rs`**

Find the `match field_def.field_type.as_str()` block in `default_fields()`. It currently ends with `_ => FieldValue::Text(String::new())`. Add two arms before the wildcard:

```rust
"date" => FieldValue::Date(None),
"email" => FieldValue::Email(String::new()),
```

After the change the full match looks like:

```rust
let default_value = match field_def.field_type.as_str() {
    "text" => FieldValue::Text(String::new()),
    "number" => FieldValue::Number(0.0),
    "boolean" => FieldValue::Boolean(false),
    "date" => FieldValue::Date(None),
    "email" => FieldValue::Email(String::new()),
    _ => FieldValue::Text(String::new()),
};
```

**Step 3: Run tests — expect both new tests to pass**

```bash
cargo test -p krillnotes-core
```

Expected: all tests pass including `test_date_field_default` and `test_email_field_default`.

**Step 4: Commit**

```bash
git add krillnotes-core/src/core/note.rs krillnotes-core/src/core/scripting.rs
git commit -m "feat(core): add Date and Email FieldValue variants"
```

---

### Task 3: Add Contact schema test (failing)

**Files:**
- Modify: `krillnotes-core/src/core/scripting.rs`

**Step 1: Add a failing test in the `tests` module**

```rust
#[test]
fn test_contact_schema_loaded() {
    let registry = SchemaRegistry::new().unwrap();
    let schema = registry.get_schema("Contact").unwrap();
    assert_eq!(schema.name, "Contact");
    assert_eq!(schema.fields.len(), 11);
    let email_field = schema.fields.iter().find(|f| f.name == "email").unwrap();
    assert_eq!(email_field.field_type, "email");
    let birthdate_field = schema.fields.iter().find(|f| f.name == "birthdate").unwrap();
    assert_eq!(birthdate_field.field_type, "date");
}
```

**Step 2: Run test — expect failure**

```bash
cargo test -p krillnotes-core test_contact_schema_loaded
```

Expected: FAIL with `SchemaNotFound("Contact")`.

---

### Task 4: Create `contact.rhai` and load it

**Files:**
- Create: `krillnotes-core/src/system_scripts/contact.rhai`
- Modify: `krillnotes-core/src/core/scripting.rs`

**Step 1: Create the script file**

Create `krillnotes-core/src/system_scripts/contact.rhai` with this exact content:

```rhai
schema("Contact", #{
    fields: [
        #{ name: "first_name",      type: "text",  required: true  },
        #{ name: "middle_name",     type: "text",  required: false },
        #{ name: "last_name",       type: "text",  required: true  },
        #{ name: "phone",           type: "text",  required: false },
        #{ name: "mobile",          type: "text",  required: false },
        #{ name: "email",           type: "email", required: false },
        #{ name: "birthdate",       type: "date",  required: false },
        #{ name: "address_street",  type: "text",  required: false },
        #{ name: "address_city",    type: "text",  required: false },
        #{ name: "address_zip",     type: "text",  required: false },
        #{ name: "address_country", type: "text",  required: false },
    ]
});
```

**Step 2: Load the script in `SchemaRegistry::new()`**

In `scripting.rs`, the `new()` method currently has:

```rust
registry.load_script(include_str!("../system_scripts/text_note.rhai"))?;
```

Add the contact script load immediately after:

```rust
registry.load_script(include_str!("../system_scripts/text_note.rhai"))?;
registry.load_script(include_str!("../system_scripts/contact.rhai"))?;
```

**Step 3: Run the contact schema test**

```bash
cargo test -p krillnotes-core test_contact_schema_loaded
```

Expected: PASS.

**Step 4: Run the full test suite**

```bash
cargo test -p krillnotes-core
```

Expected: all tests pass.

**Step 5: Commit**

```bash
git add krillnotes-core/src/system_scripts/contact.rhai krillnotes-core/src/core/scripting.rs
git commit -m "feat(core): add Contact schema with 11 fields"
```

---

### Task 5: Update TypeScript `FieldValue` type

**Files:**
- Modify: `krillnotes-desktop/src/types.ts`

**Step 1: Extend the `FieldValue` union**

In `types.ts`, find:

```typescript
export type FieldValue =
  | { Text: string }
  | { Number: number }
  | { Boolean: boolean };
```

Replace with:

```typescript
export type FieldValue =
  | { Text: string }
  | { Number: number }
  | { Boolean: boolean }
  | { Date: string | null }   // ISO "YYYY-MM-DD" or null when not set
  | { Email: string };
```

Also update the `FieldDefinition` comment on the same file:

```typescript
export interface FieldDefinition {
  name: string;
  fieldType: string;  // "text" | "number" | "boolean" | "date" | "email"
  required: boolean;
}
```

**Step 2: Verify TypeScript compiles**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```

Expected: no errors. (The two component files will have TypeScript exhaustiveness warnings until Tasks 6 and 7 are done — that is fine.)

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/types.ts
git commit -m "feat(frontend): add Date and Email to FieldValue type"
```

---

### Task 6: Update `FieldDisplay` for Date and Email

**Files:**
- Modify: `krillnotes-desktop/src/components/FieldDisplay.tsx`

**Step 1: Add `Date` and `Email` cases to `renderValue()`**

In `FieldDisplay.tsx`, find the section ending with:

```tsx
    }
    return <span className="text-muted-foreground italic">(unknown type)</span>;
```

Insert two new `else if` branches before the final `return`:

```tsx
    } else if ('Email' in value) {
      return value.Email
        ? <a href={`mailto:${value.Email}`} className="text-primary underline">{value.Email}</a>
        : <span className="text-muted-foreground italic">(empty)</span>;
    } else if ('Date' in value) {
      if (!value.Date) {
        return <span className="text-muted-foreground italic">(empty)</span>;
      }
      const formatted = new Date(value.Date).toLocaleDateString(undefined, {
        year: 'numeric', month: 'long', day: 'numeric',
      });
      return <p>{formatted}</p>;
    }
    return <span className="text-muted-foreground italic">(unknown type)</span>;
```

**Step 2: Verify TypeScript compiles**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```

Expected: no errors.

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/FieldDisplay.tsx
git commit -m "feat(frontend): render Email as mailto link, Date as formatted string"
```

---

### Task 7: Update `FieldEditor` for Date and Email

**Files:**
- Modify: `krillnotes-desktop/src/components/FieldEditor.tsx`

**Step 1: Add `Date` and `Email` cases to `renderEditor()`**

In `FieldEditor.tsx`, find the section ending with:

```tsx
    }
    return <span className="text-red-500">Unknown field type</span>;
```

Insert two new `else if` branches before the final `return`:

```tsx
    } else if ('Email' in value) {
      return (
        <input
          type="email"
          value={value.Email}
          onChange={(e) => onChange({ Email: e.target.value })}
          className="w-full p-2 bg-background border border-border rounded-md"
          required={required}
        />
      );
    } else if ('Date' in value) {
      return (
        <input
          type="date"
          value={value.Date ?? ''}
          onChange={(e) => onChange({ Date: e.target.value || null })}
          className="w-full p-2 bg-background border border-border rounded-md"
          required={required}
        />
      );
    }
    return <span className="text-red-500">Unknown field type</span>;
```

Note: `e.target.value || null` converts an empty date input (user cleared the field) to `null`, which round-trips correctly to `FieldValue::Date(None)` on the Rust side.

**Step 2: Verify TypeScript compiles**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```

Expected: no errors.

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/FieldEditor.tsx
git commit -m "feat(frontend): add date picker and email input for new field types"
```

---

### Task 8: Smoke test in the running app

**Step 1: Run the dev server**

```bash
cd krillnotes-desktop && npm run tauri dev
```

**Step 2: Manual verification checklist**

- [ ] Open a workspace, create a new note, observe "Contact" is available in the type selector
- [ ] Create a Contact note — all 11 fields appear (first_name, middle_name, last_name, phone, mobile, email, birthdate, address_*)
- [ ] In edit mode: email field shows a text input, date field shows a date picker
- [ ] Set an email address and save — read-only view shows it as a clickable mailto link
- [ ] Set a birthdate and save — read-only view shows a formatted date (e.g. "February 19, 2026")
- [ ] Clear the birthdate and save — read-only view shows "(empty)"
- [ ] Existing TextNote notes are unaffected

**Step 3: Final commit if any fixups were needed**

```bash
git add -p   # add only the fixup changes
git commit -m "fix: contact schema smoke test fixups"
```

---

## Summary of Files Changed

| File | Change |
|---|---|
| `krillnotes-core/src/core/note.rs` | Add `Date(Option<NaiveDate>)` and `Email(String)` variants |
| `krillnotes-core/src/core/scripting.rs` | New `default_fields` arms + load contact.rhai + 3 new tests |
| `krillnotes-core/src/system_scripts/contact.rhai` | New file — 11-field Contact schema |
| `krillnotes-desktop/src/types.ts` | Extend `FieldValue` union + update comment |
| `krillnotes-desktop/src/components/FieldDisplay.tsx` | Add Email (mailto) + Date (formatted) rendering |
| `krillnotes-desktop/src/components/FieldEditor.tsx` | Add email input + date picker editing |
