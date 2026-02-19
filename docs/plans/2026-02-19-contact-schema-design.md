# Contact Schema + New Field Types — Design

**Date:** 2026-02-19
**Status:** Approved

## Overview

Add a built-in Contact note schema and two new field types (`date`, `email`) to support structured personal contact information. This includes data layer changes in Rust and basic UI rendering in the React frontend.

## Scope

**In scope:**
- New `FieldValue::Date(chrono::NaiveDate)` and `FieldValue::Email(String)` Rust enum variants
- New `contact.rhai` system script loaded at startup
- Frontend type union extension and field rendering/editing for the new types

**Out of scope:**
- Visual grouping of `address_*` fields in the UI (deferred)
- Server-side email format validation in Rust (HTML5 input handles client-side validation)
- Sorting or filtering notes by date field values (deferred)

---

## Data Layer (krillnotes-core)

### FieldValue enum — `note.rs`

Two new variants added to the existing enum:

```rust
pub enum FieldValue {
    Text(String),
    Number(f64),
    Boolean(bool),
    Date(chrono::NaiveDate),  // ISO 8601, serializes as "YYYY-MM-DD"
    Email(String),             // Raw string; format validated client-side
}
```

`NaiveDate` was chosen over a plain `String` to get parse-time validation and type safety. It serializes via `serde` to the ISO 8601 string `"YYYY-MM-DD"`, which sorts correctly as plain text in SQLite and is compatible with SQLite's `date()` and `json_extract()` functions.

Note: all fields are stored in a single `fields_json` TEXT blob, so there is no per-field SQL column. ISO string representation is the correct choice for future date-based queries via `json_extract`.

### Default fields — `scripting.rs`

Two new match arms in `Schema::default_fields()`:
- `"date"` → `FieldValue::Date(chrono::NaiveDate::default())` (epoch: 1970-01-01)
- `"email"` → `FieldValue::Email(String::new())`

No changes required to `FieldDefinition` or `parse_schema()` — `field_type` is already stored as a plain string and passes through transparently.

---

## Contact Schema Script

New file: `krillnotes-core/src/system_scripts/contact.rhai`

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

Loaded in `SchemaRegistry::new()` immediately after `text_note.rhai`.

**Address design decision:** Address is represented as four flat `text` fields with the `address_` prefix rather than a nested group or a new `Address` FieldValue variant. Grouping is a pure UI/display concern and is deferred. ZIP codes are `text` (not `number`) to preserve leading zeros and support non-numeric postal codes (e.g. UK postcodes).

---

## Frontend (krillnotes-desktop)

### types.ts

```typescript
export type FieldValue =
  | { Text: string }
  | { Number: number }
  | { Boolean: boolean }
  | { Date: string }    // ISO string "YYYY-MM-DD" from HTML date input
  | { Email: string };
```

### FieldDisplay.tsx — read-only rendering

New cases in `renderValue()`:

- **Email:** Rendered as `<a href="mailto:{value.Email}">{value.Email}</a>`. Shows `(empty)` italic placeholder if blank.
- **Date:** Rendered as a localized date string via `new Date(value.Date).toLocaleDateString()`. Shows `(empty)` italic placeholder if blank (empty string).

### FieldEditor.tsx — edit mode rendering

New cases in `renderEditor()`:

- **Email:** `<input type="email" value={value.Email} onChange={...}>` — HTML5 provides format hints and basic client-side validation.
- **Date:** `<input type="date" value={value.Date} onChange={...}>` — native browser date picker; value is the ISO string `"YYYY-MM-DD"`.

---

## Testing

New unit tests to be added in `scripting.rs`:

1. `test_contact_schema_loaded` — verifies Contact schema registers with 11 fields
2. `test_date_default` — verifies `default_fields()` returns `FieldValue::Date` for `"date"` type
3. `test_email_default` — verifies `default_fields()` returns `FieldValue::Email` for `"email"` type

---

## Key Decisions Log

| Decision | Choice | Rationale |
|---|---|---|
| New type approach | New `FieldValue` variants | Clean semantics; enables future sort/filter by type |
| Date storage | `chrono::NaiveDate` | Parse-time validation; ISO serialization sorts correctly |
| Email storage | Plain `String` | Format validated client-side via HTML5; server-side validation deferred |
| Address structure | Flat prefixed text fields | Grouping is a UI concern; no new schema infrastructure needed |
