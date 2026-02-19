# Design: Auto-select and enter edit mode after note creation

**Date:** 2026-02-20

## Problem

After creating a note, the user must manually click it in the tree and then click the Edit button. The note should be immediately selected and in edit mode.

## Approach

Thread the new note's ID through the existing frontend callback chain. The backend command already returns the created note — we just use that ID to drive selection and edit-mode entry.

## Changes

### `AddNoteDialog.tsx`

- Change `onNoteCreated` prop type from `() => void` to `(noteId: string) => void`
- Pass the returned note's ID to the callback after successful creation

### `WorkspaceView.tsx`

- Update `handleNoteCreated` signature to `(noteId: string)`
- After `loadNotes()` completes, call `handleSelectNote(noteId)`
- Then increment `requestEditMode` to signal `InfoPanel` to enter edit mode

## No backend changes

The `create_note_with_type` Tauri command already returns the created note. No Rust changes required.

## Edge cases

- Works for both the menu shortcut (`CmdOrCtrl+Shift+N`) and the context menu "Add Note" — both use the same `AddNoteDialog`
- Selection change normally resets edit mode in `InfoPanel`; by sequencing select-then-trigger-edit in `handleNoteCreated`, the edit mode signal arrives after the reset, so it correctly activates
