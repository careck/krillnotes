# Workspace Name Fixes — Design

**Date:** 2026-02-24

## Summary

Two small UX fixes related to workspace naming:

1. **Creation dialog** — allow spaces in workspace names; slugify the input to produce a valid filename.
2. **Export dialog** — use the current workspace name as the default save filename instead of the hardcoded `workspace.krillnotes.zip`.

## Issue 1: Workspace Creation with Spaces

### Root Cause

`NewWorkspaceDialog.tsx` puts the raw user input directly into the `.db` path. The Tauri backend generates a window label from the filename stem, and Tauri window labels cannot contain spaces. This causes a runtime error when a user enters a name like "My Notes".

### Approach

Add a `slugify` helper in `NewWorkspaceDialog.tsx`:

- Lowercase the input
- Replace runs of non-alphanumeric characters (including spaces) with `-`
- Trim leading/trailing dashes

The slugified name is used for the filename only. The Rust `humanize()` function already converts `my-notes` → `My Notes` when creating the root note title, so the round-trip is seamless.

Replace the current character-blacklist validation with a check that the slug is non-empty (handles the "all special chars" edge case).

**Example:**
```
Input:     "My Notes"
Slug:      "my-notes"
File:      my-notes.db
Root note: "My Notes"  (via existing humanize() on Rust side)
Preview:   Will be saved to: ~/Workspaces/my-notes.db
```

## Issue 2: Export Default Filename

### Root Cause

`App.tsx` exports handler hardcodes `defaultPath: 'workspace.krillnotes.zip'`. The `createMenuHandlers` factory does not receive the `workspace` state, so it cannot derive a name.

### Approach

- Pass `workspace: WorkspaceInfoType | null` into `createMenuHandlers`
- Derive the export default name: strip `.db` from `workspace.filename`, append `.krillnotes.zip`
- Fall back to `'workspace'` if `workspace` is null
- Add `workspace` to the `useEffect` dependency array so the handler always sees the current value

**Example:**
```
Workspace file: my-notes.db
Export default: my-notes.krillnotes.zip
```

## Files Changed

| File | Change |
|------|--------|
| `krillnotes-desktop/src/components/NewWorkspaceDialog.tsx` | Add `slugify`, update path construction and validation |
| `krillnotes-desktop/src/App.tsx` | Pass `workspace` into `createMenuHandlers`, update export default path, add dep to `useEffect` |

No Rust changes required.
