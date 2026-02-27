# Import Theme/Script from File — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add "Import from file" to both the Manage Themes dialog and the Script Manager dialog, loading the file into the editor with a conflict warning and a "Replace" button when a same-named item exists.

**Architecture:** One new Rust command `read_file_content` reads the path returned by the OS file picker. Both dialogs gain an "Import from file" button on their list views; the file content is loaded into the existing editor view. A new `importConflict` state controls the warning banner and changes "Save" to "Replace" (with a confirm guard). Setting `editingMeta`/`editingScript` to the conflicting record means the existing save logic is reused without changes.

**Tech Stack:** Rust (std::fs), Tauri 2 command, React/TypeScript, `@tauri-apps/plugin-dialog` (already in project), `tempfile` crate (already in dev-dependencies).

---

### Task 0: Create worktree

**Step 1: Create feature branch + worktree**

```bash
git -C /Users/careck/Source/Krillnotes worktree add .worktrees/feat/import-theme-script-from-file -b feat/import-theme-script-from-file
```

All subsequent work happens inside `/Users/careck/Source/Krillnotes/.worktrees/feat/import-theme-script-from-file/`.

---

### Task 1: Add `read_file_content` Rust command

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/lib.rs` (around line 1127, after `delete_theme`; and in `generate_handler![]` around line 1381)

**Step 1: Write the failing test**

Add at the bottom of `lib.rs`, before the final `}` of the file (create a new `#[cfg(test)]` module):

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn read_file_content_impl_returns_file_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.txt");
        std::fs::write(&path, "hello import").unwrap();
        let result = super::read_file_content_impl(path.to_str().unwrap());
        assert_eq!(result.unwrap(), "hello import");
    }

    #[test]
    fn read_file_content_impl_errors_on_missing_file() {
        let result = super::read_file_content_impl("/nonexistent/__krillnotes_test__.txt");
        assert!(result.is_err());
    }
}
```

**Step 2: Run test to verify it fails**

```bash
cargo test -p krillnotes-desktop --lib read_file_content_impl 2>&1 | tail -10
```

Expected: compile error — `read_file_content_impl` does not exist yet.

**Step 3: Add the implementation**

Insert after the `delete_theme` command (around line 1127 in `lib.rs`):

```rust
fn read_file_content_impl(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| e.to_string())
}

#[tauri::command]
fn read_file_content(path: String) -> std::result::Result<String, String> {
    read_file_content_impl(&path)
}
```

Also add `read_file_content,` to the `generate_handler![]` list after `delete_theme,` (around line 1381).

**Step 4: Run tests to verify they pass**

```bash
cargo test -p krillnotes-desktop --lib read_file_content 2>&1 | tail -10
```

Expected: `test tests::read_file_content_impl_returns_file_text ... ok`
Expected: `test tests::read_file_content_impl_errors_on_missing_file ... ok`

**Step 5: Run full Rust test suite to check for regressions**

```bash
cargo test 2>&1 | tail -5
```

Expected: all tests pass.

**Step 6: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/lib.rs
git commit -m "feat: add read_file_content Tauri command for file import"
```

---

### Task 2: "Import from file" in ManageThemesDialog

**Files:**
- Modify: `krillnotes-desktop/src/components/ManageThemesDialog.tsx`

**Context:** The dialog has two views: `'list'` and `'editor'`. `editingMeta` is `null` for new themes, or a `ThemeMeta` when editing an existing one. `handleSave` uses `editingMeta?.filename` to decide whether to overwrite or create. We exploit this: by setting `editingMeta` to the conflicting theme before entering the editor, `handleSave` will call `write_theme` with the existing filename — replacing it.

**Step 1: Add `open` import and `read_file_content` invoke**

At the top of the file, `open` is not yet imported. Add it:

```ts
import { open } from '@tauri-apps/plugin-dialog';
```

`invoke` is already imported from `@tauri-apps/api/core`.

**Step 2: Add `importConflict` state**

Inside the component, after the existing `useState` declarations:

```ts
const [importConflict, setImportConflict] = useState<ThemeMeta | null>(null);
```

**Step 3: Reset `importConflict` on navigation away from editor**

The existing `handleNew` function sets `editingMeta(null)` and navigates to editor. Add a reset there, and also in `handleEdit`:

In `handleNew`:
```ts
setImportConflict(null);
```

In `handleEdit` (add before `setView('editor')`):
```ts
setImportConflict(null);
```

Also reset when the user navigates back to list (the `← Back` button and Cancel already call `setView('list')`; add a wrapper or inline reset in those onClick handlers). Simplest: replace the back button's `onClick` with an arrow function:

```tsx
onClick={() => { setView('list'); setImportConflict(null); setError(''); }}
```

**Step 4: Add `handleImportFromFile` function**

Add after `handleDelete`:

```ts
const handleImportFromFile = async () => {
  const path = await open({
    filters: [{ name: 'Krillnotes Theme', extensions: ['krilltheme'] }],
    multiple: false,
  });
  if (!path) return;
  try {
    const content = await invoke<string>('read_file_content', { path });
    const cleaned = content
      .split('\n')
      .filter(line => !/^\s*\/\//.test(line))
      .join('\n')
      .replace(/,(\s*[}\]])/g, '$1');
    let parsed: { name?: string };
    try {
      parsed = JSON.parse(cleaned);
    } catch {
      setError('Invalid theme file — could not parse JSON.');
      return;
    }
    const name = parsed.name ?? 'unnamed';
    const conflict = themes.find(t => t.name === name) ?? null;
    setImportConflict(conflict);
    setEditingMeta(conflict ?? null);
    setEditorContent(content);
    setError('');
    setView('editor');
  } catch (e) {
    setError(`Failed to read file: ${e}`);
  }
};
```

**Step 5: Add `handleSaveOrReplace` wrapper**

Add after `handleImportFromFile`:

```ts
const handleSaveOrReplace = async () => {
  if (importConflict) {
    const confirmed = confirm(`Replace theme "${importConflict.name}"? This cannot be undone.`);
    if (!confirmed) return;
  }
  await handleSave();
};
```

Also, `handleSave` calls `setView('list')` on success. Add `setImportConflict(null)` to `handleSave` just before (or after) `setView('list')`:

```ts
setImportConflict(null);
setView('list');
```

**Step 6: Add "Import from file" button to list view footer**

In the list view footer `<div>` (around line 282), add the button alongside "+ New Theme":

```tsx
<div className="px-4 py-3 border-t border-border flex justify-between">
  <div className="flex gap-2">
    <button
      onClick={handleNew}
      className="text-sm px-3 py-1.5 rounded bg-primary text-primary-foreground hover:opacity-90"
    >
      + New Theme
    </button>
    <button
      onClick={handleImportFromFile}
      className="text-sm px-3 py-1.5 rounded border border-border text-foreground hover:bg-secondary"
    >
      Import from file…
    </button>
  </div>
  <button onClick={onClose} className="text-sm text-muted-foreground hover:text-foreground">
    Close
  </button>
</div>
```

**Step 7: Add conflict warning banner and change Save → Replace in editor view**

In the editor view section, add the warning banner directly above the editor container `<div ref={containerRef}>`:

```tsx
{importConflict && (
  <div className="px-4 py-2 text-sm text-yellow-700 bg-yellow-50 border-b border-yellow-200 dark:bg-yellow-900/20 dark:text-yellow-300">
    A theme named "{importConflict.name}" already exists. Saving will replace it.
  </div>
)}
```

Change the Save button's `onClick` to `handleSaveOrReplace` and update its label:

```tsx
<button
  onClick={handleSaveOrReplace}
  disabled={saving}
  className="text-sm px-3 py-1.5 rounded bg-primary text-primary-foreground hover:opacity-90 disabled:opacity-50"
>
  {saving ? 'Saving…' : (importConflict ? 'Replace' : 'Save')}
</button>
```

**Step 8: TypeScript build check**

```bash
cd krillnotes-desktop && npx tsc --noEmit 2>&1
```

Expected: no errors.

**Step 9: Commit**

```bash
git add krillnotes-desktop/src/components/ManageThemesDialog.tsx
git commit -m "feat: import theme from file in ManageThemesDialog"
```

---

### Task 3: "Import from file" in ScriptManagerDialog

**Files:**
- Modify: `krillnotes-desktop/src/components/ScriptManagerDialog.tsx`

**Context:** Scripts are identified by UUID. Setting `editingScript` to the conflicting `UserScript` before entering the editor causes `handleSave` to call `update_user_script` with that UUID — replacing it. A new script (no conflict) leaves `editingScript` as `null`, so `handleSave` calls `create_user_script`.

**Step 1: Add imports**

```ts
import { open } from '@tauri-apps/plugin-dialog';
```

`invoke` is already imported.

**Step 2: Add `parseFrontMatterName` helper**

Add above the component function:

```ts
function parseFrontMatterName(source: string): string {
  for (const line of source.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed.startsWith('//')) {
      if (trimmed === '') continue;
      break;
    }
    const body = trimmed.replace(/^\/\/\s*/, '');
    if (body.startsWith('@name:')) {
      return body.slice('@name:'.length).trim();
    }
  }
  return '';
}
```

**Step 3: Add `importConflict` state**

Inside the component after existing `useState` declarations:

```ts
const [importConflict, setImportConflict] = useState<UserScript | null>(null);
```

**Step 4: Reset `importConflict` on navigation**

In `handleAdd`:
```ts
setImportConflict(null);
```

In `handleEdit`:
```ts
setImportConflict(null);
```

In the Cancel button's onClick (editor footer):
```tsx
onClick={() => { setView('list'); setError(''); setImportConflict(null); }}
```

**Step 5: Add `handleImportFromFile`**

Add after `handleDelete`:

```ts
const handleImportFromFile = async () => {
  const path = await open({
    filters: [{ name: 'Rhai Script', extensions: ['rhai'] }],
    multiple: false,
  });
  if (!path) return;
  try {
    const content = await invoke<string>('read_file_content', { path });
    const name = parseFrontMatterName(content);
    const conflict = name ? (scripts.find(s => s.name === name) ?? null) : null;
    setImportConflict(conflict);
    setEditingScript(conflict ?? null);
    setEditorContent(content);
    setError('');
    setView('editor');
  } catch (e) {
    setError(`Failed to read file: ${e}`);
  }
};
```

**Step 6: Add `handleSaveOrReplace` wrapper**

```ts
const handleSaveOrReplace = async () => {
  if (importConflict) {
    const confirmed = confirm(`Replace script "${importConflict.name}"? This cannot be undone.`);
    if (!confirmed) return;
  }
  await handleSave();
};
```

Add `setImportConflict(null)` inside `handleSave` just before `setView('list')`:

```ts
setImportConflict(null);
setView('list');
```

**Step 7: Add "Import from file" button to list view header**

The existing header has `<h2>User Scripts</h2>` and `<button>+ Add</button>`. Add the import button alongside:

```tsx
<div className="flex items-center gap-2">
  <button
    onClick={handleAdd}
    className="px-3 py-1.5 bg-primary text-primary-foreground rounded-md hover:bg-primary/90 text-sm"
  >
    + Add
  </button>
  <button
    onClick={handleImportFromFile}
    className="px-3 py-1.5 border border-border rounded-md hover:bg-secondary text-sm"
  >
    Import from file…
  </button>
</div>
```

**Step 8: Add conflict warning banner in editor view**

Add the banner between the editor header and the editor area (after `<div className="p-4 border-b border-border">`):

```tsx
{importConflict && (
  <div className="px-4 py-2 text-sm text-yellow-700 bg-yellow-50 border-b border-yellow-200 dark:bg-yellow-900/20 dark:text-yellow-300">
    A script named "{importConflict.name}" already exists. Saving will replace it.
  </div>
)}
```

**Step 9: Change Save → Replace button**

In the editor footer, replace the Save button's `onClick` and label:

```tsx
<button
  onClick={handleSaveOrReplace}
  className="px-4 py-2 bg-primary text-primary-foreground rounded-md hover:bg-primary/90"
  disabled={saving}
>
  {saving ? 'Saving...' : (importConflict ? 'Replace' : 'Save')}
</button>
```

**Step 10: TypeScript build check**

```bash
cd krillnotes-desktop && npx tsc --noEmit 2>&1
```

Expected: no errors.

**Step 11: Commit**

```bash
git add krillnotes-desktop/src/components/ScriptManagerDialog.tsx
git commit -m "feat: import script from file in ScriptManagerDialog"
```

---

### Task 4: Final verification

**Step 1: Full Rust test suite**

```bash
cargo test 2>&1 | tail -5
```

Expected: all tests pass (previously 224+).

**Step 2: TypeScript build**

```bash
cd krillnotes-desktop && npx tsc --noEmit 2>&1
```

Expected: no errors.

**Step 3: Push branch**

```bash
git -C /Users/careck/Source/Krillnotes push -u github-https feat/import-theme-script-from-file
```

**Step 4: Open PR**

```bash
gh pr create \
  --repo careck/krillnotes \
  --base master \
  --title "feat: import theme/script from file (issue #31)" \
  --body "$(cat <<'EOF'
## Summary
- Adds "Import from file…" button to Manage Themes dialog (list view footer)
- Adds "Import from file…" button to Script Manager dialog (list view header)
- New `read_file_content` Tauri command reads the path returned by the OS file picker
- Imported content is loaded into the existing editor view for review before saving
- If a theme/script with the same name already exists: yellow warning banner appears and Save button changes to "Replace", which prompts a confirm dialog before overwriting

## Test plan
- [ ] Import a new `.krilltheme` file → it appears in the list
- [ ] Import a `.krilltheme` with the same name as an existing theme → warning banner shown, button reads "Replace", confirm dialog appears, confirms replacement
- [ ] Import a new `.rhai` file with `// @name:` front-matter → it appears in the script list
- [ ] Import a `.rhai` file whose `@name` matches an existing script → warning banner, "Replace" button, confirm dialog, script updated
- [ ] Cancel on file picker → no change
- [ ] Decline confirm on Replace → no change, editor stays open
- [ ] Import a malformed `.krilltheme` → error message shown, stays on list view
- [ ] Rust tests pass: `cargo test`
- [ ] TypeScript build clean: `npx tsc --noEmit`

Closes #31

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```
