# Editor Scroll + Cut Fix — Design

## Summary

Fix two bugs in the theme and script editors (issue #46):

1. **No scroll**: Both editors fail to scroll vertically.
2. **Cmd/Ctrl+X broken**: The cut shortcut is a no-op everywhere in the app.

## Root Causes

### Bug 1a — ScriptEditor, missing `overflow: auto` on scroller

`ScriptEditor.tsx` sets up a CodeMirror editor with `height: '100%'` on the root element
but never sets `overflow: auto` on `.cm-scroller`. Without that, the scroller div never
becomes a scroll container; content just overflows and is clipped by the outer
`overflow-hidden` container.

**Fix:** add `overflow: 'auto'` to the `.cm-scroller` rule in the `EditorView.theme()` call.

### Bug 1b — ManageThemesDialog, missing `min-h-0` on flex container

`ManageThemesDialog.tsx` already sets `overflow: auto` on `.cm-scroller`, but the
container div is `flex-1 overflow-hidden` without `min-h-0`. Flex items default to
`min-height: auto`, meaning the container grows to accommodate its content rather than
being constrained by the parent's `max-h-[80vh]`. CodeMirror therefore never needs to
scroll — the layout never constrains its height.

**Fix:** add `min-h-0` to the editor container's className.

### Bug 2 — Edit menu missing `cut:` selector

`menu.rs` registers `PredefinedMenuItem::copy` and `PredefinedMenuItem::paste` in the
Edit menu, wiring up the macOS `copy:` and `paste:` responder-chain selectors. It never
adds `PredefinedMenuItem::cut`, so macOS has no `cut:` selector to route `Cmd+X` through.
`Cmd+A` (select all) is also absent for the same reason.

**Fix:** declare `cut` and `select_all` predefined items and add them to the Edit submenu.

## Affected Files

| File | Change |
|---|---|
| `krillnotes-desktop/src/components/ScriptEditor.tsx` | Add `overflow: 'auto'` to `.cm-scroller` theme |
| `krillnotes-desktop/src/components/ManageThemesDialog.tsx` | Add `min-h-0` to editor container className |
| `krillnotes-desktop/src-tauri/src/menu.rs` | Add `cut` + `select_all` predefined items to Edit menu |

## Out of Scope

- Scrollbar styling
- Any other editor keyboard shortcut gaps
- Windows/Linux clipboard behaviour (already works via browser)
