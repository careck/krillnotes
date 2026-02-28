# Editor Scroll + Cut Fix — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix vertical scrolling in both editors and restore Cmd/Ctrl+X (cut) app-wide.

**Architecture:** Three isolated one-line or two-line changes across two frontend files and one Rust file. No new dependencies, no new abstractions, no state changes.

**Tech Stack:** React/TypeScript (CodeMirror 6), Rust/Tauri v2 menu API.

---

### Task 1: Fix scroll in ScriptEditor

**Files:**
- Modify: `krillnotes-desktop/src/components/ScriptEditor.tsx:52-57`

**Step 1: Open the file and locate the `.cm-scroller` theme rule**

The relevant section is the `EditorView.theme({...})` call starting around line 47.
Currently `.cm-scroller` only sets `fontFamily`.

**Step 2: Add `overflow: 'auto'` to the `.cm-scroller` rule**

Change:
```ts
'.cm-scroller': {
  fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
},
```
To:
```ts
'.cm-scroller': {
  overflow: 'auto',
  fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
},
```

**Step 3: Verify TypeScript compiles**

```bash
cd /path/to/worktree/krillnotes-desktop
npm run typecheck   # or: npx tsc --noEmit
```
Expected: no errors.

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/components/ScriptEditor.tsx
git commit -m "fix: add overflow:auto to ScriptEditor cm-scroller so it scrolls"
```

---

### Task 2: Fix scroll in ManageThemesDialog

**Files:**
- Modify: `krillnotes-desktop/src/components/ManageThemesDialog.tsx:384`

**Step 1: Locate the editor container div**

Search for the line that reads:
```tsx
<div ref={containerRef} className="flex-1 overflow-hidden border-b border-border" />
```
It's inside the `{view === 'editor' && ...}` block.

**Step 2: Add `min-h-0` to the className**

Change:
```tsx
<div ref={containerRef} className="flex-1 overflow-hidden border-b border-border" />
```
To:
```tsx
<div ref={containerRef} className="flex-1 min-h-0 overflow-hidden border-b border-border" />
```

**Step 3: Verify TypeScript compiles**

```bash
npx tsc --noEmit
```
Expected: no errors.

**Step 4: Commit**

```bash
git add krillnotes-desktop/src/components/ManageThemesDialog.tsx
git commit -m "fix: add min-h-0 to ManageThemesDialog editor container so CodeMirror can scroll"
```

---

### Task 3: Fix Cmd+X (cut) and Cmd+A (select-all) in the native Edit menu

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/menu.rs:142-191`

**Step 1: Locate the predefined item declarations in `build_edit_menu`**

Around line 166–169:
```rust
let undo  = PredefinedMenuItem::undo(app, None)?;
let redo  = PredefinedMenuItem::redo(app, None)?;
let copy  = PredefinedMenuItem::copy(app, None)?;
let paste = PredefinedMenuItem::paste(app, None)?;
```

**Step 2: Add `cut` and `select_all` declarations immediately after `copy`**

```rust
let undo       = PredefinedMenuItem::undo(app, None)?;
let redo       = PredefinedMenuItem::redo(app, None)?;
let cut        = PredefinedMenuItem::cut(app, None)?;
let copy       = PredefinedMenuItem::copy(app, None)?;
let paste      = PredefinedMenuItem::paste(app, None)?;
let select_all = PredefinedMenuItem::select_all(app, None)?;
```

**Step 3: Add the new items to the submenu builder (line 184)**

Change:
```rust
let submenu = builder.items(&[&undo, &redo, &copy, &paste]).build()?;
```
To:
```rust
let submenu = builder.items(&[&undo, &redo, &cut, &copy, &paste, &select_all]).build()?;
```

**Step 4: Run Rust tests to confirm nothing broke**

```bash
cd /path/to/worktree/krillnotes-desktop/src-tauri
cargo test
```
Expected: all tests pass (currently 235).

**Step 5: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/menu.rs
git commit -m "fix: add cut and select_all predefined menu items so Cmd+X/A work on macOS"
```

---

### Task 4: Manual smoke test + push

**Step 1: Run the app**

```bash
cd /path/to/worktree/krillnotes-desktop
npm run tauri dev
```

**Step 2: Verify scroll in Script editor**

- Open a workspace → Tools → Manage Scripts → edit any script (or create new)
- Paste in a script with 30+ lines so it overflows
- Check scrollbar appears and scroll wheel / trackpad scroll works

**Step 3: Verify scroll in Theme editor**

- Settings → Manage Themes → edit any custom theme
- Paste in extra lines to overflow
- Check scrollbar appears and scroll wheel / trackpad scroll works

**Step 4: Verify Cmd+X works**

- In any text field (note body, search bar, or inside either editor)
- Select some text, press Cmd+X — text should be cut and land on clipboard
- Press Cmd+V to confirm paste works

**Step 5: Push branch and open PR**

```bash
git push -u github-https fix/editor-scroll-cut
gh pr create \
  --title "fix: editor scroll and Cmd+X" \
  --body "Fixes #46 — vertical scroll in both editors, Cmd+X (cut) everywhere."
```
