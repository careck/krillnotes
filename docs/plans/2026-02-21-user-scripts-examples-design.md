# User Scripts Examples + New Field Types — Design Document

**Date:** 2026-02-21
**Status:** Approved
**Scope:** Add `select` and `rating` field types; create `user_scripts/` folder with five example scripts

---

## Overview

This feature adds two new Rhai field types (`select`, `rating`) and a top-level `user_scripts/` folder containing five example scripts that showcase the scripting system's capabilities. The examples serve as copy-paste starting points for users and demonstrate progressively richer `on_save` hooks.

---

## 1. New Field Types

### `select`

A dropdown of predefined string options. Stored as `FieldValue::Text` — no new storage variant required.

**Rhai schema syntax:**
```rhai
#{ name: "status", type: "select", options: ["TODO", "WIP", "DONE"], required: true }
```

- Edit mode: `<select>` dropdown populated from `options`
- View mode: plain text (identical to `text`)

### `rating`

A 1–N star rating. Stored as `FieldValue::Number` — no new storage variant required.

**Rhai schema syntax:**
```rhai
#{ name: "rating", type: "rating", max: 5, required: false }
```

- Edit mode: clickable star row (1…max stars)
- View mode: Unicode star string, e.g. `"★★★★☆"`

---

## 2. Backend Changes (Rust)

### `krillnotes-core/src/core/scripting/schema.rs`

- `FieldDefinition` gains two optional metadata fields:
  - `options: Vec<String>` — non-empty only for `select` fields
  - `max: i64` — non-zero only for `rating` fields (0 = not applicable)
- `parse_from_rhai` reads `options` (Rhai array → `Vec<String>`) and `max` (Rhai integer → `i64`) from each field map; both default gracefully when absent.
- `FieldDefinition` remains serde-serializable; new fields serialize as camelCase (`options`, `max`).

### `krillnotes-core/src/core/scripting/hooks.rs`

- `dynamic_to_field_value`: `"select"` branch handled identically to `"text"` (stores `FieldValue::Text`).
- `dynamic_to_field_value`: `"rating"` branch handled identically to `"number"` (stores `FieldValue::Number`).
- `field_value_to_dynamic`: no changes needed (Text and Number already handled).

---

## 3. Frontend Changes (TypeScript / React)

### `types.ts`

- `FieldType` union: add `'select'` and `'rating'`
- `FieldDefinition` interface: add `options: string[]` and `max: number`

### `FieldEditor.tsx`

- Receive `options: string[]` and `max: number` as additional props
- `select`: when `fieldType === 'select'` and value is `{ Text }`, render `<select>` with one `<option>` per entry in `options`
- `rating`: when `fieldType === 'rating'` and value is `{ Number }`, render a row of `max` clickable star characters; clicking star `i` sets value to `i`

### `FieldDisplay.tsx`

- Receive `fieldType: FieldType` and `max: number` as additional props
- `select`: render identical to `Text` (plain string)
- `rating`: when `fieldType === 'rating'`, build star string from `value.Number` and `max` (filled `★` up to value, empty `☆` for remainder)

### Prop threading

`FieldEditor` and `FieldDisplay` are called from `WorkspaceView` (edit panel) and the note view panel respectively. The `SchemaInfo` / `FieldDefinition` already flows down to these components; `options` and `max` ride along the same path.

---

## 4. `user_scripts/` Folder

Five `.rhai` files at the repository root. Each file is a standalone user script that can be pasted directly into the Script Manager.

### `task.rhai` — richest hook

Follows the Contact pattern (`title_can_edit: false`). The user fills in a `name` field; `on_save` computes the note title and a derived `priority_label` shown only in view mode.

**Fields:**
| name | type | flags |
|---|---|---|
| `name` | `text` | `required: true` |
| `status` | `select` | options: `["TODO", "WIP", "DONE"]`, `required: true` |
| `priority` | `select` | options: `["low", "medium", "high"]` |
| `due_date` | `date` | — |
| `assignee` | `text` | — |
| `priority_label` | `text` | `can_edit: false` (view-only, derived) |
| `notes` | `textarea` | — |

**on_save hook:**
- Derives `title` → `"[ ] Buy groceries"` / `"[→] Write report"` / `"[✓] Call dentist"` from `status` + `name`
- Derives `priority_label` → `"🔴 High"` / `"🟡 Medium"` / `"🟢 Low"` from `priority`
- Note in script: real-time urgency (days until due) would require an `on_view()` hook (not yet implemented)

### `project.rhai` — medium hook

`title_can_edit: true` — the user names the project directly.

**Fields:**
| name | type | flags |
|---|---|---|
| `status` | `select` | options: `["Planning", "Active", "On Hold", "Done"]`, `required: true` |
| `priority` | `select` | options: `["low", "medium", "high"]` |
| `start_date` | `date` | — |
| `due_date` | `date` | — |
| `description` | `textarea` | — |
| `health` | `text` | `can_edit: false` (view-only, derived) |

**on_save hook:**
- Derives `health` → `"✅ Done"` / `"🚧 Active"` / `"⏸ On Hold"` / `"📋 Planning"` from `status`

### `book.rhai` — medium hook

`title_can_edit: false`. Computed as `"Author: Title"`.

**Fields:**
| name | type | flags |
|---|---|---|
| `book_title` | `text` | `required: true` |
| `author` | `text` | `required: true` |
| `genre` | `text` | — |
| `status` | `select` | options: `["To Read", "Reading", "Read"]` |
| `rating` | `rating` | `max: 5` |
| `started` | `date` | — |
| `finished` | `date` | — |
| `rating_stars` | `text` | `can_edit: false` (view-only, derived) |
| `read_duration` | `text` | `can_edit: false` (view-only, derived) |
| `notes` | `textarea` | — |

**on_save hook:**
- Derives `title` → `"Tolkien: The Hobbit"`
- Derives `rating_stars` → `"★★★★☆"` from `rating` (0 → `"Not rated"`)
- Derives `read_duration` → `"14 days"` from `started` + `finished` (blank if either is unset)

### `product.rhai` — simple hook

`title_can_edit: false`. Computed as `"Name (SKU)"`.

**Fields:**
| name | type | flags |
|---|---|---|
| `product_name` | `text` | `required: true` |
| `sku` | `text` | — |
| `price` | `number` | — |
| `stock` | `number` | — |
| `category` | `text` | — |
| `description` | `textarea` | — |
| `stock_status` | `text` | `can_edit: false` (view-only, derived) |

**on_save hook:**
- Derives `title` → `"Wireless Mouse (WM-4821)"` (falls back to name alone if SKU empty)
- Derives `stock_status` → `"Out of Stock"` (0) / `"Low Stock"` (1–4) / `"In Stock"` (5+)

### `recipe.rhai` — simplest hook

`title_can_edit: true`.

**Fields:**
| name | type | flags |
|---|---|---|
| `servings` | `number` | — |
| `prep_time` | `number` | — (minutes) |
| `cook_time` | `number` | — (minutes) |
| `difficulty` | `select` | options: `["Easy", "Medium", "Hard"]` |
| `ingredients` | `textarea` | — |
| `steps` | `textarea` | — |
| `total_time` | `text` | `can_edit: false` (view-only, derived) |

**on_save hook:**
- Derives `total_time` from `prep_time + cook_time`: formats as `"45 min"` or `"1h 15min"`

---

## 5. Out of Scope

- `on_view()` hook (planned separately; would enable real-time date-relative urgency labels)
- Star picker animation or hover effects
- Validation that `select` values match the options list (Rhai hook scripts can enforce this manually if needed)
