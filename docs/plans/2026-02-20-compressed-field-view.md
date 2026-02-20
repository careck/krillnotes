# Compressed Field View Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Render note fields in a compact two-column grid (label | value) in view mode, hiding empty fields entirely.

**Architecture:** `FieldDisplay` returns a React Fragment (`<dt>` + `<dd>`) so both elements participate directly in a CSS grid declared on the parent `<dl>` in `InfoPanel`. Empty-field filtering lives in `InfoPanel` as a small helper. Edit mode is unchanged.

**Tech Stack:** React 19, TypeScript, Tailwind CSS v4, Tauri desktop app. No test framework exists — verification is TypeScript type-check (`tsc --noEmit`) + visual check in dev server.

---

### Task 1: Add `isEmptyFieldValue` helper to `InfoPanel.tsx`

**Files:**
- Modify: `krillnotes-desktop/src/components/InfoPanel.tsx`

**Step 1: Add the helper function**

Insert this function just above `function InfoPanel(` (around line 24):

```ts
function isEmptyFieldValue(value: FieldValue): boolean {
  if ('Text' in value)    return value.Text === '';
  if ('Email' in value)   return value.Email === '';
  if ('Date' in value)    return value.Date === null;
  return false; // Number and Boolean are never empty
}
```

**Step 2: Type-check**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```

Expected: no errors.

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/InfoPanel.tsx
git commit -m "feat: add isEmptyFieldValue helper"
```

---

### Task 2: Refactor `FieldDisplay.tsx` to return a Fragment

**Files:**
- Modify: `krillnotes-desktop/src/components/FieldDisplay.tsx`

**Step 1: Replace the return statement**

Replace the entire `return (...)` block (lines 43–52) with:

```tsx
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
```

Key points:
- `self-start` on `<dt>` pins the label to the top of the row when the value wraps to multiple lines
- `whitespace-nowrap` prevents labels from wrapping within the label column
- `m-0` resets the browser's default left margin on `<dd>`
- The outer `<div className="mb-4">` and inner `<label>` are removed entirely

**Step 2: Type-check**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```

Expected: no errors.

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/FieldDisplay.tsx
git commit -m "feat: refactor FieldDisplay to render dt/dd fragment for grid layout"
```

---

### Task 3: Update `InfoPanel.tsx` — schema fields list

**Files:**
- Modify: `krillnotes-desktop/src/components/InfoPanel.tsx`

**Step 1: Replace the schema fields block**

Find this block (around lines 229–251):

```tsx
{/* Fields Section */}
<div className="mb-6">
  <h2 className="text-xl font-semibold mb-4">Fields</h2>

  {schemaInfo.fields
    .filter(field => isEditing ? field.canEdit : field.canView)
    .map(field => (
      isEditing ? (
        <FieldEditor
          key={field.name}
          fieldName={field.name}
          value={editedFields[field.name] ?? defaultValueForFieldType(field.fieldType)}
          required={field.required}
          onChange={(value) => handleFieldChange(field.name, value)}
        />
      ) : (
        <FieldDisplay
          key={field.name}
          fieldName={field.name}
          value={selectedNote.fields[field.name] ?? defaultValueForFieldType(field.fieldType)}
        />
      )
    ))
  }
```

Replace with:

```tsx
{/* Fields Section */}
<div className="mb-6">
  <h2 className="text-xl font-semibold mb-4">Fields</h2>

  {isEditing ? (
    schemaInfo.fields
      .filter(field => field.canEdit)
      .map(field => (
        <FieldEditor
          key={field.name}
          fieldName={field.name}
          value={editedFields[field.name] ?? defaultValueForFieldType(field.fieldType)}
          required={field.required}
          onChange={(value) => handleFieldChange(field.name, value)}
        />
      ))
  ) : (
    <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-1">
      {schemaInfo.fields
        .filter(field => field.canView)
        .filter(field => !isEmptyFieldValue(selectedNote.fields[field.name] ?? defaultValueForFieldType(field.fieldType)))
        .map(field => (
          <FieldDisplay
            key={field.name}
            fieldName={field.name}
            value={selectedNote.fields[field.name] ?? defaultValueForFieldType(field.fieldType)}
          />
        ))
      }
    </dl>
  )}
```

**Step 2: Type-check**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```

Expected: no errors.

**Step 3: Commit**

```bash
git add krillnotes-desktop/src/components/InfoPanel.tsx
git commit -m "feat: use dl grid for schema fields, hide empty fields in view mode"
```

---

### Task 4: Update `InfoPanel.tsx` — legacy fields + "No fields" fallback

**Files:**
- Modify: `krillnotes-desktop/src/components/InfoPanel.tsx`

**Step 1: Update the legacy fields block**

Find the legacy fields block (around lines 253–276):

```tsx
{legacyFieldNames.length > 0 && (
  <>
    <h3 className="text-lg font-medium text-muted-foreground mt-6 mb-3">
      Legacy Fields
    </h3>
    {legacyFieldNames.map(name => (
      isEditing ? (
        <FieldEditor
          key={name}
          fieldName={`${name} (legacy)`}
          value={editedFields[name] ?? { Text: '' }}
          required={false}
          onChange={(value) => handleFieldChange(name, value)}
        />
      ) : (
        <FieldDisplay
          key={name}
          fieldName={`${name} (legacy)`}
          value={selectedNote.fields[name]}
        />
      )
    ))}
  </>
)}
```

Replace with:

```tsx
{legacyFieldNames.length > 0 && (() => {
  if (isEditing) {
    return (
      <>
        <h3 className="text-lg font-medium text-muted-foreground mt-6 mb-3">
          Legacy Fields
        </h3>
        {legacyFieldNames.map(name => (
          <FieldEditor
            key={name}
            fieldName={`${name} (legacy)`}
            value={editedFields[name] ?? { Text: '' }}
            required={false}
            onChange={(value) => handleFieldChange(name, value)}
          />
        ))}
      </>
    );
  }
  const visibleLegacy = legacyFieldNames.filter(
    name => !isEmptyFieldValue(selectedNote.fields[name])
  );
  if (visibleLegacy.length === 0) return null;
  return (
    <>
      <h3 className="text-lg font-medium text-muted-foreground mt-6 mb-3">
        Legacy Fields
      </h3>
      <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-1">
        {visibleLegacy.map(name => (
          <FieldDisplay
            key={name}
            fieldName={`${name} (legacy)`}
            value={selectedNote.fields[name]}
          />
        ))}
      </dl>
    </>
  );
})()}
```

**Step 2: Update the "No fields" fallback**

Find (around line 278):

```tsx
{schemaInfo.fields.length === 0 && legacyFieldNames.length === 0 && (
  <p className="text-muted-foreground italic">No fields</p>
)}
```

Replace with:

```tsx
{!isEditing &&
  schemaInfo.fields.filter(f =>
    f.canView && !isEmptyFieldValue(selectedNote.fields[f.name] ?? defaultValueForFieldType(f.fieldType))
  ).length === 0 &&
  legacyFieldNames.filter(n => !isEmptyFieldValue(selectedNote.fields[n])).length === 0 && (
    <p className="text-muted-foreground italic">No fields</p>
  )
}
{isEditing && schemaInfo.fields.length === 0 && legacyFieldNames.length === 0 && (
  <p className="text-muted-foreground italic">No fields</p>
)}
```

**Step 3: Type-check**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```

Expected: no errors.

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/components/InfoPanel.tsx
git commit -m "feat: apply grid layout and empty filter to legacy fields"
```

---

### Task 5: Visual verification

**Step 1: Start the dev server**

```bash
cd krillnotes-desktop && npm run dev
```

Open in browser at `http://localhost:1420` (or whichever port Vite reports).

**Step 2: Verify these scenarios**

| Scenario | Expected |
|----------|----------|
| Note with all fields filled | All fields on one row, labels left-aligned in column 1, values in column 2 |
| Note with some empty text/email/date fields | Empty fields are not shown at all |
| Note with a long multi-line text value | Label pins to the top of its row, value wraps naturally within column 2 |
| Boolean field (true or false) | Always shown |
| Number field with value 0 | Always shown |
| Edit mode | Unchanged — stacked FieldEditor blocks, no grid |
| Note with all fields empty | "No fields" message shown |

**Step 3: Final commit (if any fixes were needed)**

```bash
git add -p
git commit -m "fix: address visual verification findings"
```
