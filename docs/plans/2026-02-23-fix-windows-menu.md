# Implementation Plan: Fix Windows Menu Actions

**Date:** 2026-02-23
**Branch:** feat/fix-windows-menu

## Steps

### 1. Edit `App.tsx`

File: `krillnotes-desktop/src/App.tsx`

Remove the focus import, the `isFocused` call, and the guard:

```diff
 const unlisten = listen<string>('menu-action', async (event) => {
-  // Only handle menu events if this window is focused
-  const { getCurrentWebviewWindow } = await import('@tauri-apps/api/webviewWindow');
-  const window = getCurrentWebviewWindow();
-  const isFocused = await window.isFocused();
-
-  if (!isFocused) return;
-
   const handler = handlers[event.payload as keyof typeof handlers];
   if (handler) handler();
 });
```

The listener no longer needs to be `async`.

### 2. Edit `WorkspaceView.tsx`

File: `krillnotes-desktop/src/components/WorkspaceView.tsx`

Remove the `isFocused` call and guard:

```diff
-const unlisten = listen<string>('menu-action', async (event) => {
-  const isFocused = await getCurrentWebviewWindow().isFocused();
-  if (!isFocused) return;
+const unlisten = listen<string>('menu-action', (event) => {
   if (event.payload === 'Edit > Add Note clicked') {
```

Also remove the `getCurrentWebviewWindow` import if it becomes unused after the edit.

### 3. Verify TypeScript

```
npx tsc --noEmit
```

Should produce no errors.
