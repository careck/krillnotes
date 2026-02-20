# Collapsible Metadata Section Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the note metadata panel (Type, Created, Modified, ID) a collapsible `<details>` element that is hidden by default.

**Architecture:** Single file change in `InfoPanel.tsx`. Replace the metadata `<div>` with a `<details>`/`<summary>` using native HTML — no React state, no new dependencies. `ChevronRight` from the already-installed lucide-react rotates 90° when open via a Tailwind arbitrary variant.

**Tech Stack:** React 19, TypeScript, Tailwind CSS v4, lucide-react. No test framework — verification is `tsc --noEmit` + visual check.

---

### Task 1: Replace metadata div with collapsible details element

**Files:**
- Modify: `krillnotes-desktop/src/components/InfoPanel.tsx:1` (import line)
- Modify: `krillnotes-desktop/src/components/InfoPanel.tsx:323-341` (metadata section)

**Step 1: Add `ChevronRight` to the lucide-react import**

The file currently has no lucide-react import (icons are only used in `FieldDisplay.tsx`). Add this import after the existing imports at the top of the file (after line 5):

```tsx
import { ChevronRight } from 'lucide-react';
```

**Step 2: Replace the metadata section**

Find this block (lines 323–341):

```tsx
      {/* Metadata Section */}
      <div className="bg-secondary p-6 rounded-lg space-y-4">
        <div>
          <p className="text-sm text-muted-foreground">Type</p>
          <p className="text-lg">{selectedNote.nodeType}</p>
        </div>
        <div>
          <p className="text-sm text-muted-foreground">Created</p>
          <p className="text-sm">{formatTimestamp(selectedNote.createdAt)}</p>
        </div>
        <div>
          <p className="text-sm text-muted-foreground">Modified</p>
          <p className="text-sm">{formatTimestamp(selectedNote.modifiedAt)}</p>
        </div>
        <div>
          <p className="text-sm text-muted-foreground">ID</p>
          <p className="text-xs font-mono">{selectedNote.id}</p>
        </div>
      </div>
```

Replace with:

```tsx
      {/* Metadata Section */}
      <details className="bg-secondary rounded-lg">
        <summary className="px-6 py-4 cursor-pointer list-none flex items-center gap-2 text-sm font-medium text-muted-foreground select-none">
          <ChevronRight size={16} className="[details[open]_&]:rotate-90 transition-transform" />
          Info
        </summary>
        <div className="px-6 pb-6 space-y-4">
          <div>
            <p className="text-sm text-muted-foreground">Type</p>
            <p className="text-lg">{selectedNote.nodeType}</p>
          </div>
          <div>
            <p className="text-sm text-muted-foreground">Created</p>
            <p className="text-sm">{formatTimestamp(selectedNote.createdAt)}</p>
          </div>
          <div>
            <p className="text-sm text-muted-foreground">Modified</p>
            <p className="text-sm">{formatTimestamp(selectedNote.modifiedAt)}</p>
          </div>
          <div>
            <p className="text-sm text-muted-foreground">ID</p>
            <p className="text-xs font-mono">{selectedNote.id}</p>
          </div>
        </div>
      </details>
```

Key points:
- No `open` attribute on `<details>` → collapsed by default
- `list-none` on `<summary>` removes the browser's built-in disclosure triangle
- `[details[open]_&]:rotate-90` rotates the chevron when the `<details>` parent has the `open` attribute
- `transition-transform` animates the rotation smoothly
- The four metadata rows are unchanged, just indented inside the content `<div>`

**Step 3: Type-check**

```bash
cd krillnotes-desktop && npx tsc --noEmit
```

Expected: no errors.

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/components/InfoPanel.tsx
git commit -m "feat: collapse metadata section into details/summary toggle"
```
