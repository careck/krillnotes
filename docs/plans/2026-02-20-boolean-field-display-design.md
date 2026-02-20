# Boolean Field Display & Edit Fix

**Date:** 2026-02-20

## Problem

Boolean fields (e.g. `is_family` in the Contact schema) render as text inputs in edit mode and as a disabled checkbox + "Yes/No" text in view mode. The desired behaviour is a checkbox in edit mode and colored icons (green check / red X) in view mode.

**Root cause:** In `InfoPanel.tsx`, missing field values fall back to `{ Text: '' }` regardless of field type. When a note has no stored value for a boolean field (field added after note creation, or new note), `FieldEditor` receives `{ Text: '' }` and renders a textarea.

## Solution

### 1. Add lucide-react icon library

Install `lucide-react` (MIT, tree-shakeable, Tailwind-compatible) as a production dependency.

### 2. Schema-aware field initialization in InfoPanel (Option B)

When the schema loads successfully, merge schema-typed defaults into `editedFields` for any field not already present in the note. This ensures all fields are pre-populated with the correct type before rendering, covering:
- New notes whose fields map is empty
- Notes created before a field was added to the schema

Add a helper `defaultValueForFieldType(fieldType: string): FieldValue`:
- `"boolean"` → `{ Boolean: false }`
- `"number"` → `{ Number: 0 }`
- `"date"` → `{ Date: null }`
- `"email"` → `{ Email: '' }`
- default → `{ Text: '' }`

Also replace the render-time `|| { Text: '' }` fallbacks with `??` (nullish coalescing) as a safety net.

### 3. Boolean view mode icons in FieldDisplay

Replace the disabled checkbox + "Yes/No" span with lucide-react icons:
- `true` → `<Check size={18} className="text-green-500" />`
- `false` → `<X size={18} className="text-red-500" />`

## Files Changed

| File | Change |
|------|--------|
| `krillnotes-desktop/package.json` | Add `lucide-react` dependency |
| `krillnotes-desktop/src/components/InfoPanel.tsx` | Add `defaultValueForFieldType` helper; merge defaults on schema load; replace `\|\|` with `??` at render fallbacks |
| `krillnotes-desktop/src/components/FieldDisplay.tsx` | Import `Check`, `X` from lucide-react; replace boolean rendering with icons |

## Out of Scope

- Styling the checkbox in edit mode beyond what already exists
- Any other field types
