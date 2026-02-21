# User Scripts Examples + select/rating Field Types — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add `select` and `rating` Rhai field types, then create `user_scripts/` with five polished example scripts.

**Architecture:** `select` stores as `FieldValue::Text`, `rating` stores as `FieldValue::Number` — no new storage variants. New `options: Vec<String>` and `max: i64` metadata fields on `FieldDefinition` flow from Rust → Tauri IPC → TypeScript props → `FieldEditor` / `FieldDisplay` components. Example scripts live in `user_scripts/*.rhai` at the repo root.

**Tech Stack:** Rust + Rhai (backend scripting), Tauri v2 IPC, React + TypeScript (frontend), Tailwind CSS.

---

## Task 1: Rust — extend `FieldDefinition` with `options` and `max`

**Files:**
- Modify: `krillnotes-core/src/core/scripting/schema.rs`

**Step 1: Write the failing test**

Add at the bottom of the `#[cfg(test)]` block in `krillnotes-core/src/core/scripting/mod.rs`:

```rust
#[test]
fn test_select_field_parses_options() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("Ticket", #{
            fields: [
                #{ name: "status", type: "select", options: ["TODO", "WIP", "DONE"], required: true }
            ]
        });
    "#).unwrap();
    let fields = get_schema_fields_for_test(&registry, "Ticket");
    assert_eq!(fields[0].options, vec!["TODO", "WIP", "DONE"]);
}

#[test]
fn test_rating_field_parses_max() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("Review", #{
            fields: [
                #{ name: "stars", type: "rating", max: 5 }
            ]
        });
    "#).unwrap();
    let fields = get_schema_fields_for_test(&registry, "Review");
    assert_eq!(fields[0].max, 5);
}

#[test]
fn test_regular_fields_have_empty_options_and_zero_max() {
    let mut registry = ScriptRegistry::new().unwrap();
    let fields = get_schema_fields_for_test(&registry, "TextNote");
    assert!(fields[0].options.is_empty());
    assert_eq!(fields[0].max, 0);
}

// Helper — call the public get_schema_fields via the engine
fn get_schema_fields_for_test(registry: &ScriptRegistry, name: &str) -> Vec<FieldDefinition> {
    registry.get_schema(name).unwrap().fields
}
```

**Step 2: Run to confirm failure**

```bash
cargo test -p krillnotes-core test_select_field_parses_options 2>&1 | tail -5
```
Expected: compile error — `options` field does not exist on `FieldDefinition`.

**Step 3: Extend `FieldDefinition` and `parse_from_rhai`**

In `krillnotes-core/src/core/scripting/schema.rs`, change `FieldDefinition`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldDefinition {
    pub name: String,
    pub field_type: String,
    pub required: bool,
    pub can_view: bool,
    pub can_edit: bool,
    /// Non-empty only for `select` fields — the list of allowed option strings.
    #[serde(default)]
    pub options: Vec<String>,
    /// Non-zero only for `rating` fields — the maximum star count.
    #[serde(default)]
    pub max: i64,
}
```

In `parse_from_rhai`, after the `can_edit` extraction and before `fields.push(...)`:

```rust
let options: Vec<String> = field_map
    .get("options")
    .and_then(|v| v.clone().try_cast::<rhai::Array>())
    .unwrap_or_default()
    .into_iter()
    .filter_map(|item| item.try_cast::<String>())
    .collect();

let max: i64 = field_map
    .get("max")
    .and_then(|v| v.clone().try_cast::<i64>())
    .unwrap_or(0);

fields.push(FieldDefinition { name: field_name, field_type, required, can_view, can_edit, options, max });
```

**Step 4: Add `get_schema` public method** (needed by test helper above)

In `krillnotes-core/src/core/scripting/mod.rs`, add alongside `schema_exists`:

```rust
/// Returns a clone of the named schema, or an error if not registered.
pub fn get_schema(&self, name: &str) -> Result<Schema> {
    self.schema_registry.get(name)
}
```

**Step 5: Run tests**

```bash
cargo test -p krillnotes-core 2>&1 | tail -10
```
Expected: all tests pass, including the three new ones.

**Step 6: Commit**

```bash
git add krillnotes-core/src/core/scripting/schema.rs krillnotes-core/src/core/scripting/mod.rs
git commit -m "feat: add options and max metadata to FieldDefinition for select/rating types"
```

---

## Task 2: Rust — handle `select` and `rating` in hook value conversion

**Files:**
- Modify: `krillnotes-core/src/core/scripting/hooks.rs`

**Step 1: Write the failing tests**

Add to the `#[cfg(test)]` block in `mod.rs`:

```rust
#[test]
fn test_select_field_round_trips_through_hook() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("S", #{
            fields: [ #{ name: "status", type: "select", options: ["A", "B"] } ]
        });
        on_save("S", |note| {
            note.fields.status = "B";
            note
        });
    "#).unwrap();

    let mut fields = std::collections::HashMap::new();
    fields.insert("status".to_string(), crate::FieldValue::Text("A".to_string()));

    let result = registry.hooks().run_on_save_hook(
        registry.engine(),
        &registry.get_schema("S").unwrap(),
        "id1", "S", "title", &fields,
    ).unwrap().unwrap();
    assert_eq!(result.1["status"], crate::FieldValue::Text("B".to_string()));
}

#[test]
fn test_rating_field_round_trips_through_hook() {
    let mut registry = ScriptRegistry::new().unwrap();
    registry.load_script(r#"
        schema("R", #{
            fields: [ #{ name: "stars", type: "rating", max: 5 } ]
        });
        on_save("R", |note| {
            note.fields.stars = 4.0;
            note
        });
    "#).unwrap();

    let mut fields = std::collections::HashMap::new();
    fields.insert("stars".to_string(), crate::FieldValue::Number(0.0));

    let result = registry.hooks().run_on_save_hook(
        registry.engine(),
        &registry.get_schema("R").unwrap(),
        "id1", "R", "title", &fields,
    ).unwrap().unwrap();
    assert_eq!(result.1["stars"], crate::FieldValue::Number(4.0));
}
```

Note: these tests also require a public `engine()` accessor on `ScriptRegistry`. Add it:

```rust
/// Returns a reference to the Rhai engine (needed for hook execution in tests).
pub fn engine(&self) -> &Engine {
    &self.engine
}
```

**Step 2: Run to confirm failure**

```bash
cargo test -p krillnotes-core test_select_field_round_trips 2>&1 | tail -5
```
Expected: compile error (missing `engine()`) or runtime panic on unknown type `"select"`.

**Step 3: Implement in `hooks.rs`**

In `dynamic_to_field_value`, add two new match arms before the catch-all `_`:

```rust
"select" => {
    if d.is_unit() {
        return Ok(FieldValue::Text(String::new()));
    }
    let s = d
        .try_cast::<String>()
        .ok_or_else(|| KrillnotesError::Scripting("select field must be a string".into()))?;
    Ok(FieldValue::Text(s))
}
"rating" => {
    if d.is_unit() {
        return Ok(FieldValue::Number(0.0));
    }
    let n = d
        .try_cast::<f64>()
        .ok_or_else(|| KrillnotesError::Scripting("rating field must be a float".into()))?;
    Ok(FieldValue::Number(n))
}
```

**Step 4: Run all tests**

```bash
cargo test -p krillnotes-core 2>&1 | tail -10
```
Expected: all tests pass.

**Step 5: Commit**

```bash
git add krillnotes-core/src/core/scripting/hooks.rs krillnotes-core/src/core/scripting/mod.rs
git commit -m "feat: handle select and rating field types in hook value conversion"
```

---

## Task 3: TypeScript — extend types for `select` and `rating`

**Files:**
- Modify: `krillnotes-desktop/src/types.ts`

**Step 1: Update `FieldType` and `FieldDefinition`**

```typescript
export type FieldType = 'text' | 'textarea' | 'number' | 'boolean' | 'date' | 'email' | 'select' | 'rating';

export interface FieldDefinition {
  name: string;
  fieldType: FieldType;
  required: boolean;
  canView: boolean;
  canEdit: boolean;
  options: string[];   // non-empty for 'select' fields
  max: number;         // non-zero for 'rating' fields
}
```

**Step 2: Update `defaultValueForFieldType` in `InfoPanel.tsx`**

The existing `default: return { Text: '' }` already covers `'select'` (select stores as Text). Add `'rating'` explicitly:

```typescript
function defaultValueForFieldType(fieldType: string): FieldValue {
  switch (fieldType) {
    case 'boolean': return { Boolean: false };
    case 'number':  return { Number: 0 };
    case 'rating':  return { Number: 0 };
    case 'date':    return { Date: null };
    case 'email':   return { Email: '' };
    default:        return { Text: '' }; // covers 'text', 'textarea', 'select'
  }
}
```

**Step 3: Build to check for type errors**

```bash
cd krillnotes-desktop && npm run build 2>&1 | grep -E "error|warning" | head -20
```
Expected: TypeScript errors on `FieldDefinition` usages that don't supply `options`/`max` — this is expected and will be fixed in the next tasks.

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/types.ts krillnotes-desktop/src/components/InfoPanel.tsx
git commit -m "feat: add select and rating to FieldType; extend FieldDefinition with options and max"
```

---

## Task 4: `FieldEditor` — render dropdown for `select`, stars for `rating`

**Files:**
- Modify: `krillnotes-desktop/src/components/FieldEditor.tsx`

**Step 1: Update props interface and add rendering**

Replace the entire file:

```tsx
import type { FieldValue, FieldType } from '../types';

interface FieldEditorProps {
  fieldName: string;
  fieldType: FieldType;
  value: FieldValue;
  required: boolean;
  options: string[];   // for select
  max: number;         // for rating
  onChange: (value: FieldValue) => void;
}

function FieldEditor({ fieldName, fieldType, value, required, options, max, onChange }: FieldEditorProps) {
  const renderEditor = () => {
    if ('Text' in value) {
      if (fieldType === 'textarea') {
        return (
          <textarea
            value={value.Text}
            onChange={(e) => onChange({ Text: e.target.value })}
            className="w-full p-2 bg-background border border-border rounded-md min-h-[100px] resize-y"
            required={required}
          />
        );
      }
      if (fieldType === 'select') {
        return (
          <select
            value={value.Text}
            onChange={(e) => onChange({ Text: e.target.value })}
            className="w-full p-2 bg-background border border-border rounded-md"
            required={required}
          >
            <option value="">— select —</option>
            {options.map(opt => (
              <option key={opt} value={opt}>{opt}</option>
            ))}
          </select>
        );
      }
      return (
        <input
          type="text"
          value={value.Text}
          onChange={(e) => onChange({ Text: e.target.value })}
          className="w-full p-2 bg-background border border-border rounded-md"
          required={required}
        />
      );
    } else if ('Number' in value) {
      if (fieldType === 'rating') {
        const current = value.Number;
        const starCount = max > 0 ? max : 5;
        return (
          <div className="flex gap-1">
            {Array.from({ length: starCount }, (_, i) => i + 1).map(star => (
              <button
                key={star}
                type="button"
                onClick={() => onChange({ Number: star === current ? 0 : star })}
                className="text-2xl leading-none text-yellow-400 hover:scale-110 transition-transform"
                aria-label={`${star} star${star !== 1 ? 's' : ''}`}
              >
                {star <= current ? '★' : '☆'}
              </button>
            ))}
          </div>
        );
      }
      return (
        <input
          type="number"
          value={value.Number}
          onChange={(e) => onChange({ Number: parseFloat(e.target.value) || 0 })}
          className="w-full p-2 bg-background border border-border rounded-md"
          required={required}
        />
      );
    } else if ('Boolean' in value) {
      return (
        <input
          type="checkbox"
          checked={value.Boolean}
          onChange={(e) => onChange({ Boolean: e.target.checked })}
          className="rounded"
        />
      );
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
  };

  return (
    <div className="mb-4">
      <label className="block text-sm font-medium mb-1">
        {fieldName}
        {required && <span className="text-red-500 ml-1">*</span>}
      </label>
      {renderEditor()}
    </div>
  );
}

export default FieldEditor;
```

**Step 2: Build**

```bash
cd krillnotes-desktop && npm run build 2>&1 | grep "error" | head -20
```
Expected: TypeScript errors on the `FieldEditor` call sites in `InfoPanel.tsx` (missing `options`/`max` props). Fixed in Task 5.

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/FieldEditor.tsx
git commit -m "feat: FieldEditor renders select dropdown and rating star widget"
```

---

## Task 5: `FieldDisplay` — render stars for `rating`

**Files:**
- Modify: `krillnotes-desktop/src/components/FieldDisplay.tsx`

**Step 1: Update component**

Replace the entire file:

```tsx
import { Check, X } from 'lucide-react';
import type { FieldValue, FieldType } from '../types';

interface FieldDisplayProps {
  fieldName: string;
  fieldType: FieldType;
  value: FieldValue;
  max?: number;   // for rating — defaults to 5 if omitted
}

function FieldDisplay({ fieldName, fieldType, value, max = 5 }: FieldDisplayProps) {
  const renderValue = () => {
    if ('Number' in value && fieldType === 'rating') {
      const starCount = max > 0 ? max : 5;
      const filled = Math.round(value.Number);
      if (filled === 0) return <p className="text-muted-foreground italic">Not rated</p>;
      const stars = '★'.repeat(filled) + '☆'.repeat(Math.max(0, starCount - filled));
      return <p className="text-yellow-400 text-lg leading-none">{stars}</p>;
    }
    if ('Text' in value) {
      return <p className="whitespace-pre-wrap break-words">{value.Text}</p>;
    } else if ('Number' in value) {
      return <p>{value.Number}</p>;
    } else if ('Boolean' in value) {
      return (
        <span className="inline-flex items-center" aria-label={value.Boolean ? 'Yes' : 'No'}>
          {value.Boolean
            ? <Check size={18} className="text-green-500" aria-hidden="true" />
            : <X size={18} className="text-red-500" aria-hidden="true" />}
        </span>
      );
    } else if ('Email' in value) {
      return <a href={`mailto:${value.Email}`} className="text-primary underline">{value.Email}</a>;
    } else if ('Date' in value) {
      const formatted = new Date(`${value.Date}T00:00:00`).toLocaleDateString(undefined, {
        year: 'numeric', month: 'long', day: 'numeric',
      });
      return <p>{formatted}</p>;
    }
    return <span className="text-muted-foreground italic">(unknown type)</span>;
  };

  return (
    <>
      <dt className="text-sm font-medium text-muted-foreground self-start pt-0.5 whitespace-nowrap">
        {fieldName}
      </dt>
      <dd className="m-0 text-foreground">
        {renderValue()}
      </dd>
    </>
  );
}

export default FieldDisplay;
```

**Step 2: Thread props in `InfoPanel.tsx`**

In `InfoPanel.tsx`, update the `FieldEditor` call (edit mode, around line 265):

```tsx
<FieldEditor
  key={field.name}
  fieldName={field.name}
  fieldType={field.fieldType}
  value={editedFields[field.name] ?? defaultValueForFieldType(field.fieldType)}
  required={field.required}
  options={field.options}
  max={field.max}
  onChange={(value) => handleFieldChange(field.name, value)}
/>
```

Update the `FieldDisplay` call (view mode, around line 282):

```tsx
<FieldDisplay
  key={field.name}
  fieldName={field.name}
  fieldType={field.fieldType}
  value={selectedNote.fields[field.name] ?? defaultValueForFieldType(field.fieldType)}
  max={field.max}
/>
```

The legacy `FieldDisplay` calls (around line 323) use `fieldType="text"` hardcoded — that's fine as-is since legacy fields are always text. Pass `fieldType="text"` explicitly:

```tsx
<FieldDisplay
  key={name}
  fieldName={`${name} (legacy)`}
  fieldType="text"
  value={selectedNote.fields[name]}
/>
```

**Step 3: Build cleanly**

```bash
cd krillnotes-desktop && npm run build 2>&1 | grep "error" | head -20
```
Expected: zero TypeScript errors.

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/components/FieldDisplay.tsx krillnotes-desktop/src/components/InfoPanel.tsx
git commit -m "feat: FieldDisplay renders rating stars; thread options/max props from InfoPanel"
```

---

## Task 6: Create `user_scripts/task.rhai`

**Files:**
- Create: `user_scripts/task.rhai`

**Step 1: Create the file**

```rhai
//! Task — a trackable to-do item with status, priority, and due date.
//!
//! on_save hook:
//!   - Computes the note title as a status symbol + task name, e.g. "[✓] Buy groceries"
//!   - Derives `priority_label` (view-only) from the priority field
//!
//! Note: real-time urgency labels (e.g. "Due in 3 days") would require an on_view() hook,
//! which is not yet implemented. Use the due_date field to track deadlines for now.

schema("Task", #{
    title_can_edit: false,
    fields: [
        #{ name: "name",           type: "text",     required: true                     },
        #{ name: "status",         type: "select",   required: true,
           options: ["TODO", "WIP", "DONE"]                                             },
        #{ name: "priority",       type: "select",   required: false,
           options: ["low", "medium", "high"]                                           },
        #{ name: "due_date",       type: "date",     required: false                    },
        #{ name: "assignee",       type: "text",     required: false                    },
        #{ name: "priority_label", type: "text",     required: false,
           can_edit: false                                                               },
        #{ name: "notes",          type: "textarea", required: false                    },
    ]
});

on_save("Task", |note| {
    let name   = note.fields["name"];
    let status = note.fields["status"];

    let symbol = if status == "DONE" { "✓" }
                 else if status == "WIP" { "→" }
                 else { " " };

    note.title = "[" + symbol + "] " + name;

    let priority = note.fields["priority"];
    note.fields["priority_label"] =
        if priority == "high"   { "🔴 High" }
        else if priority == "medium" { "🟡 Medium" }
        else if priority == "low"    { "🟢 Low" }
        else                         { "" };

    note
});
```

**Step 2: Commit**

```bash
git add user_scripts/task.rhai
git commit -m "feat: add task.rhai example user script"
```

---

## Task 7: Create `user_scripts/project.rhai`

**Files:**
- Create: `user_scripts/project.rhai`

**Step 1: Create the file**

```rhai
//! Project — a piece of work with a status, priority, and optional dates.
//!
//! on_save hook:
//!   - Derives `health` (view-only) from the status field, e.g. "🚧 Active"

schema("Project", #{
    title_can_edit: true,
    fields: [
        #{ name: "status",      type: "select",   required: true,
           options: ["Planning", "Active", "On Hold", "Done"]                  },
        #{ name: "priority",    type: "select",   required: false,
           options: ["low", "medium", "high"]                                  },
        #{ name: "start_date",  type: "date",     required: false              },
        #{ name: "due_date",    type: "date",     required: false              },
        #{ name: "description", type: "textarea", required: false              },
        #{ name: "health",      type: "text",     required: false,
           can_edit: false                                                      },
    ]
});

on_save("Project", |note| {
    let status = note.fields["status"];

    note.fields["health"] =
        if status == "Done"     { "✅ Done" }
        else if status == "Active"   { "🚧 Active" }
        else if status == "On Hold"  { "⏸ On Hold" }
        else                         { "📋 Planning" };

    note
});
```

**Step 2: Commit**

```bash
git add user_scripts/project.rhai
git commit -m "feat: add project.rhai example user script"
```

---

## Task 8: Create `user_scripts/book.rhai`

**Files:**
- Create: `user_scripts/book.rhai`

**Step 1: Create the file**

```rhai
//! Book — reading tracker with star rating and derived read duration.
//!
//! on_save hook:
//!   - Computes title as "Author: Book Title"
//!   - Derives `rating_stars` (view-only) from the rating field
//!   - Derives `read_duration` (view-only) from started + finished dates
//!     (shown as "N days"; blank if either date is missing)

schema("Book", #{
    title_can_edit: false,
    fields: [
        #{ name: "book_title",    type: "text",     required: true                  },
        #{ name: "author",        type: "text",     required: true                  },
        #{ name: "genre",         type: "text",     required: false                 },
        #{ name: "status",        type: "select",   required: false,
           options: ["To Read", "Reading", "Read"]                                  },
        #{ name: "rating",        type: "rating",   required: false, max: 5         },
        #{ name: "started",       type: "date",     required: false                 },
        #{ name: "finished",      type: "date",     required: false                 },
        #{ name: "rating_stars",  type: "text",     required: false, can_edit: false },
        #{ name: "read_duration", type: "text",     required: false, can_edit: false },
        #{ name: "notes",         type: "textarea", required: false                 },
    ]
});

on_save("Book", |note| {
    let title  = note.fields["book_title"];
    let author = note.fields["author"];

    note.title = if author != "" && title != "" {
        author + ": " + title
    } else if title != "" {
        title
    } else {
        "Untitled Book"
    };

    // Star rating string
    let r = note.fields["rating"];
    note.fields["rating_stars"] = if r <= 0.0 {
        ""
    } else {
        let filled = (r + 0.5).floor().to_int();
        let empty  = 5 - filled;
        let stars  = "";
        for _ in 0..filled { stars += "★"; }
        for _ in 0..empty  { stars += "☆"; }
        stars
    };

    // Read duration from ISO date strings (YYYY-MM-DD)
    let started  = note.fields["started"];
    let finished = note.fields["finished"];
    note.fields["read_duration"] = if started != "" && finished != "" {
        // Parse dates: split on "-" and compute days naively
        let s_parts = started.split("-");
        let f_parts = finished.split("-");
        // Days-in-month approximation using year/month/day totals
        let s_days = parse_int(s_parts[0]) * 365 + parse_int(s_parts[1]) * 30 + parse_int(s_parts[2]);
        let f_days = parse_int(f_parts[0]) * 365 + parse_int(f_parts[1]) * 30 + parse_int(f_parts[2]);
        let diff = f_days - s_days;
        if diff > 0 { diff.to_string() + " days" } else { "" }
    } else {
        ""
    };

    note
});
```

> **Note on date arithmetic:** Rhai doesn't have a built-in date library. The day count above uses a simplified approximation (year×365 + month×30 + day). It gives reasonable "N days" values for display purposes — exact accuracy is not required here. The `parse_int` function is a Rhai built-in.

**Step 2: Commit**

```bash
git add user_scripts/book.rhai
git commit -m "feat: add book.rhai example user script"
```

---

## Task 9: Create `user_scripts/product.rhai`

**Files:**
- Create: `user_scripts/product.rhai`

**Step 1: Create the file**

```rhai
//! Product — an inventory item with auto-formatted title and stock status.
//!
//! on_save hook:
//!   - Computes title as "Product Name (SKU)" (or just name if SKU is blank)
//!   - Derives `stock_status` (view-only) from the stock count

schema("Product", #{
    title_can_edit: false,
    fields: [
        #{ name: "product_name", type: "text",     required: true                   },
        #{ name: "sku",          type: "text",     required: false                  },
        #{ name: "price",        type: "number",   required: false                  },
        #{ name: "stock",        type: "number",   required: false                  },
        #{ name: "category",     type: "text",     required: false                  },
        #{ name: "description",  type: "textarea", required: false                  },
        #{ name: "stock_status", type: "text",     required: false, can_edit: false  },
    ]
});

on_save("Product", |note| {
    let name = note.fields["product_name"];
    let sku  = note.fields["sku"];

    note.title = if sku != "" {
        name + " (" + sku + ")"
    } else {
        name
    };

    let stock = note.fields["stock"];
    note.fields["stock_status"] =
        if stock <= 0.0  { "❌ Out of Stock" }
        else if stock < 5.0   { "⚠️ Low Stock" }
        else                  { "✅ In Stock" };

    note
});
```

**Step 2: Commit**

```bash
git add user_scripts/product.rhai
git commit -m "feat: add product.rhai example user script"
```

---

## Task 10: Create `user_scripts/recipe.rhai`

**Files:**
- Create: `user_scripts/recipe.rhai`

**Step 1: Create the file**

```rhai
//! Recipe — a cooking recipe with ingredient list, steps, and derived total time.
//!
//! on_save hook:
//!   - Derives `total_time` (view-only) from prep_time + cook_time
//!     (formatted as "45 min" or "1h 15min")

schema("Recipe", #{
    title_can_edit: true,
    fields: [
        #{ name: "servings",    type: "number",   required: false                  },
        #{ name: "prep_time",   type: "number",   required: false                  },
        #{ name: "cook_time",   type: "number",   required: false                  },
        #{ name: "difficulty",  type: "select",   required: false,
           options: ["Easy", "Medium", "Hard"]                                     },
        #{ name: "ingredients", type: "textarea", required: false                  },
        #{ name: "steps",       type: "textarea", required: false                  },
        #{ name: "total_time",  type: "text",     required: false, can_edit: false  },
    ]
});

on_save("Recipe", |note| {
    let prep  = note.fields["prep_time"];
    let cook  = note.fields["cook_time"];
    let total = (prep + cook).to_int();

    note.fields["total_time"] = if total <= 0 {
        ""
    } else if total < 60 {
        total.to_string() + " min"
    } else {
        let h   = total / 60;
        let m   = total % 60;
        if m == 0 { h.to_string() + "h" }
        else      { h.to_string() + "h " + m.to_string() + "min" }
    };

    note
});
```

**Step 2: Commit**

```bash
git add user_scripts/recipe.rhai
git commit -m "feat: add recipe.rhai example user script"
```

---

## Task 11: End-to-end smoke test

**Step 1: Build and launch the app**

```bash
cd krillnotes-desktop && npm run tauri dev
```

**Step 2: Manual checks**

1. Open or create a workspace.
2. Open the Script Manager and paste in `user_scripts/task.rhai`. Save it.
3. Create a new note of type `Task` — verify the status field shows a dropdown with `TODO / WIP / DONE`.
4. Set priority to `high` and save — verify the note title shows `[ ] <name>` and the view panel shows `priority_label: 🔴 High`.
5. Change status to `DONE` — verify title becomes `[✓] <name>`.
6. Paste `user_scripts/book.rhai`. Create a Book note, set a rating — verify stars appear in the edit widget and the view panel shows `"★★★☆☆"` style display.
7. Paste `user_scripts/recipe.rhai`. Create a Recipe, set prep=20 and cook=45 — verify `total_time` shows `"1h 5min"`.
8. Paste `user_scripts/project.rhai` and `user_scripts/product.rhai`, repeat basic checks.

**Step 3: Final commit if any polish was needed**

```bash
git add -p   # stage only intentional changes
git commit -m "fix: polish after smoke test"
```
