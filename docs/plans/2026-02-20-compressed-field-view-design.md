# Compressed Field View Design

**Date:** 2026-02-20
**Status:** Approved

## Problem

The default note view renders each field as a stacked label-above-value block with `mb-4` spacing. This is verbose and wastes vertical space. Fields with no value still show an "(empty)" placeholder.

## Goals

- Align all field labels in one column and all values in another (table-like)
- Keep long values in their column, wrapping naturally (label stays top-left)
- Hide fields with no value entirely in view mode
- Leave edit mode layout unchanged

## Approach: CSS Grid on parent, Fragment pattern

The parent container becomes a CSS grid. `FieldDisplay` renders a React Fragment (`<dt>` + `<dd>`) whose children participate directly in the grid — giving true column alignment across all fields.

## Changes

### `FieldDisplay.tsx`

- Return `<><dt>label</dt><dd>value</dd></>` instead of `<div><label><div>`.
- `dt` classes: `text-sm font-medium text-muted-foreground self-start pt-0.5 whitespace-nowrap`
  - `self-start` pins the label to the top of the row when the value wraps
  - `whitespace-nowrap` prevents labels from wrapping themselves
- `dd` classes: `text-foreground` with browser margin reset (`m-0`)

### `InfoPanel.tsx`

**New helper:**
```ts
function isEmptyFieldValue(value: FieldValue): boolean
```

| Field type | Hidden when |
|------------|-------------|
| Text       | `value.Text === ""`   |
| Email      | `value.Email === ""`  |
| Date       | `value.Date === null` |
| Number     | never hidden          |
| Boolean    | never hidden          |

**View mode field list:**

```tsx
<dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-1">
  {schemaInfo.fields
    .filter(field => field.canView && !isEmptyFieldValue(noteValue))
    .map(field => <FieldDisplay key={field.name} ... />)}
</dl>
```

Same filter applied to legacy fields.

**"No fields" fallback:** shown when schema has zero fields AND all field values are empty after filtering.

## Out of scope

- Metadata section (Type, Created, Modified, ID) — layout unchanged
- Edit mode — layout unchanged
