# Workspace Name Fixes Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Allow spaces in workspace names at creation (slugify to filename), and use the current workspace name as the export dialog default.

**Architecture:** Two pure TypeScript/React changes — add a `slugify` helper in `NewWorkspaceDialog.tsx` and pass `workspace` state into the `createMenuHandlers` factory in `App.tsx`. No Rust changes.

**Tech Stack:** React, TypeScript, Tauri v2 (`@tauri-apps/plugin-dialog`)

---

### Task 1: Create worktree

**Files:**
- (no code files)

**Step 1: Create feature worktree**

```bash
git -C /Users/careck/Source/Krillnotes worktree add .worktrees/feat/workspace-name-fixes -b feat/workspace-name-fixes
```

Expected: `Preparing worktree (new branch 'feat/workspace-name-fixes')`

All subsequent work happens inside `.worktrees/feat/workspace-name-fixes/`.

---

### Task 2: Slugify workspace name in creation dialog

**Files:**
- Modify: `krillnotes-desktop/src/components/NewWorkspaceDialog.tsx`

**Step 1: Add slugify helper**

At the top of the file (after imports, before the component), add:

```typescript
function slugify(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
}
```

**Step 2: Update handleCreate to use slugify**

Replace the current `handleCreate` body (lines 38–64). The new version slugifies the name for the filename, validates the slug is non-empty, and constructs the path from the slug:

```typescript
const handleCreate = async () => {
  const trimmed = name.trim();
  if (!trimmed) {
    setError('Please enter a workspace name.');
    return;
  }

  const slug = slugify(trimmed);
  if (!slug) {
    setError('Name must contain at least one letter or number.');
    return;
  }

  setCreating(true);
  setError('');

  const path = `${workspaceDir}/${slug}.db`;

  try {
    await invoke<WorkspaceInfo>('create_workspace', { path });
    onClose();
  } catch (err) {
    if (err !== 'focused_existing') {
      setError(`${err}`);
    }
    setCreating(false);
  }
};
```

**Step 3: Update the filename preview**

The preview currently shows `{name.trim()}`. Update it to show the slugified version so the user can see the actual filename:

Replace line 91:
```tsx
Will be saved to: {workspaceDir}/{name.trim() || '...'}.db
```
With:
```tsx
Will be saved to: {workspaceDir}/{slugify(name.trim()) || '...'}.db
```

**Step 4: Verify the build compiles**

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/workspace-name-fixes/krillnotes-desktop
npm run build
```

Expected: no TypeScript errors, build succeeds.

**Step 5: Manual test (if dev server available)**

Run `npm run dev` or `cargo tauri dev` in the worktree, create a workspace named "My Notes", verify:
- File created as `my-notes.db`
- Root note title shows "My Notes"
- Preview in dialog shows `my-notes.db`

**Step 6: Commit**

```bash
git add krillnotes-desktop/src/components/NewWorkspaceDialog.tsx
git commit -m "fix: slugify workspace name to produce valid filename

Allows spaces and special chars in the human-readable workspace name.
The name is slugified (lowercase, non-alphanum → dash) before use as
the .db filename. The existing humanize() on the Rust side converts
e.g. my-notes back to My Notes for the root note title."
```

---

### Task 3: Use workspace name as export default filename

**Files:**
- Modify: `krillnotes-desktop/src/App.tsx`

**Step 1: Update createMenuHandlers signature to accept workspace**

The function signature currently starts with (line 21):
```typescript
const createMenuHandlers = (
  setStatus: (msg: string, isError?: boolean) => void,
  setShowNewWorkspace: (show: boolean) => void,
  setShowOpenWorkspace: (show: boolean) => void,
  setShowSettings: (show: boolean) => void,
  setImportState: (state: ImportState | null) => void,
) => ({
```

Add `workspace: WorkspaceInfoType | null` as the last parameter:
```typescript
const createMenuHandlers = (
  setStatus: (msg: string, isError?: boolean) => void,
  setShowNewWorkspace: (show: boolean) => void,
  setShowOpenWorkspace: (show: boolean) => void,
  setShowSettings: (show: boolean) => void,
  setImportState: (state: ImportState | null) => void,
  workspace: WorkspaceInfoType | null,
) => ({
```

**Step 2: Update the export handler to derive default filename**

Replace the current export handler `defaultPath` line (line 40):
```typescript
defaultPath: 'workspace.krillnotes.zip',
```
With:
```typescript
defaultPath: `${(workspace?.filename ?? 'workspace').replace(/\.db$/, '')}.krillnotes.zip`,
```

**Step 3: Pass workspace into createMenuHandlers and add it to useEffect deps**

The `useEffect` that calls `createMenuHandlers` is at line 134–149. Update the call and its dependency array:

```typescript
useEffect(() => {
  const handlers = createMenuHandlers(
    statusSetter,
    setShowNewWorkspace,
    setShowOpenWorkspace,
    setShowSettings,
    setImportState,
    workspace,
  );

  const unlisten = getCurrentWebviewWindow().listen<string>('menu-action', (event) => {
    const handler = handlers[event.payload as keyof typeof handlers];
    if (handler) handler();
  });

  return () => { unlisten.then(f => f()); };
}, [workspace]);
```

Note: `statusSetter` is defined inside `App` but is stable (no deps) — it's fine in the dep array implicitly; adding `workspace` is the important change.

**Step 4: Verify the build compiles**

```bash
cd /Users/careck/Source/Krillnotes/.worktrees/feat/workspace-name-fixes/krillnotes-desktop
npm run build
```

Expected: no TypeScript errors.

**Step 5: Manual test**

Open a workspace, trigger File > Export Workspace. Verify the save dialog pre-fills the filename as `<workspace-name>.krillnotes.zip` rather than `workspace.krillnotes.zip`.

**Step 6: Commit**

```bash
git add krillnotes-desktop/src/App.tsx
git commit -m "fix: use workspace name as default export filename

Passes workspace state into createMenuHandlers so the export dialog
pre-fills the save filename as <workspace-name>.krillnotes.zip instead
of the hardcoded 'workspace.krillnotes.zip'."
```

---

### Task 4: Mark TODO items as done

**Files:**
- Modify: `TODO.md`

Find the two TODO items:
```
[ ] minor issue: when creating a new workspace...
[ ] another minor issue when exporting a workspace...
```

Mark both as done by changing `[ ]` to `[x]`.

**Commit:**
```bash
git add TODO.md
git commit -m "chore: mark workspace name fix TODOs as done"
```

---

### Task 5: Finish the branch

Invoke `superpowers:finishing-a-development-branch` to decide on merge/PR strategy.
