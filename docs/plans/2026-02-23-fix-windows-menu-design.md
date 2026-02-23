# Design: Fix Windows Menu Actions

**Date:** 2026-02-23
**Branch:** feat/fix-windows-menu

## Problem

Menu actions work on macOS and Linux but silently do nothing on Windows.

## Root Cause

`App.tsx` and `WorkspaceView.tsx` both guard every incoming `menu-action` event with an async `isFocused()` check and discard the event if the window isn't focused:

```typescript
const isFocused = await window.isFocused();
if (!isFocused) return;
```

On **macOS**, the app menu is the global OS menu bar — clicking it never unfocuses the application window, so the check passes.

On **Linux**, Tauri uses GTK menus rendered as part of the window — same behaviour, check passes.

On **Windows**, the native menu is a separate Win32 control. Activating it briefly transfers focus away from the application window. By the time the Tauri event arrives in JS and the async `isFocused()` call resolves, the window is still unfocused, so every event is silently discarded.

## Fix

Remove the `isFocused()` guard from both listener sites. This is safe because:

- The check was intended to prevent multiple windows reacting to the same menu event, but the correct mechanism for that is window-targeted emit on the Rust side (`emit_to`), not a frontend focus poll.
- Krillnotes does not currently have a scenario where multiple workspace windows are open simultaneously and need to handle different menu events independently.
- Removing the check is platform-neutral — no behaviour change on macOS or Linux.
