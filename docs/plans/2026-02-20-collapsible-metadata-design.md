# Collapsible Metadata Section Design

**Date:** 2026-02-20
**Status:** Approved

## Problem

The metadata section (Type, Created, Modified, ID) is always visible at the bottom of the note view, taking up space even when the user doesn't need it.

## Solution

Replace the metadata `<div>` with a native HTML `<details>`/`<summary>` element. Collapsed by default. No React state needed.

## Structure

```tsx
<details className="bg-secondary rounded-lg">
  <summary className="px-6 py-4 cursor-pointer list-none flex items-center gap-2
                       text-sm font-medium text-muted-foreground select-none">
    <ChevronRight size={16} className="[details[open]_&]:rotate-90 transition-transform" />
    Info
  </summary>
  <div className="px-6 pb-6 space-y-4">
    {/* existing Type / Created / Modified / ID rows */}
  </div>
</details>
```

## Key points

- No `open` attribute → collapsed by default
- `list-none` removes the browser's built-in triangle marker
- `ChevronRight` from lucide-react (already a dependency) rotates 90° when open via `[details[open]_&]:rotate-90`
- Existing metadata rows are unchanged, just moved inside the content `<div>`
- `ChevronRight` import added to the existing lucide-react import line in `InfoPanel.tsx`
