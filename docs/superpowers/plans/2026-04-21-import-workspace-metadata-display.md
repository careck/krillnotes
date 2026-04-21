# Import Workspace Metadata Display — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show workspace properties (author, description, license, tags, etc.) in the import confirmation dialog, matching the collapsible `<details>` block already used in `AcceptInviteWorkflow`.

**Architecture:** Extend `ImportResult` to carry an optional `WorkspaceMetadata`, read `workspace.json` during `peek_import`, pass it through to the frontend `ImportState`, and render a collapsible details block in the import dialog. New `dialogs.import.*` i18n keys across all 7 locales.

**Tech Stack:** Rust (krillnotes-core), TypeScript/React (krillnotes-desktop), i18next (7 locales)

---

## File Structure

| Action | File | Responsibility |
|--------|------|----------------|
| Modify | `krillnotes-core/src/core/export.rs` | Add `metadata` field to `ImportResult`, read `workspace.json` in `peek_import` |
| Modify | `krillnotes-core/src/core/export_tests.rs` | Test metadata round-trip through `peek_import` |
| Modify | `krillnotes-desktop/src/hooks/useDialogState.ts` | Extend `ImportState` with metadata fields |
| Modify | `krillnotes-desktop/src/App.tsx` | Pass metadata into state + render collapsible block |
| Modify | `krillnotes-desktop/src/i18n/locales/en.json` | Add `dialogs.import.by`, `.homepage`, `.license` keys |
| Modify | `krillnotes-desktop/src/i18n/locales/de.json` | German translations |
| Modify | `krillnotes-desktop/src/i18n/locales/es.json` | Spanish translations |
| Modify | `krillnotes-desktop/src/i18n/locales/fr.json` | French translations |
| Modify | `krillnotes-desktop/src/i18n/locales/ja.json` | Japanese translations |
| Modify | `krillnotes-desktop/src/i18n/locales/ko.json` | Korean translations |
| Modify | `krillnotes-desktop/src/i18n/locales/zh.json` | Chinese translations |

---

### Task 1: Extend `ImportResult` and `peek_import` in Rust

**Files:**
- Modify: `krillnotes-core/src/core/export.rs:84-91` (ImportResult struct)
- Modify: `krillnotes-core/src/core/export.rs:327-339` (peek_import body)

- [ ] **Step 1: Add `metadata` field to `ImportResult`**

In `krillnotes-core/src/core/export.rs`, add the metadata field to the `ImportResult` struct (around line 87):

```rust
/// Result returned after reading an export archive's metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub app_version: String,
    pub note_count: usize,
    pub script_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<WorkspaceMetadata>,
}
```

- [ ] **Step 2: Read `workspace.json` in `peek_import`**

In `peek_import` (line ~327-339), after the `script_count` match block and before the `Ok(ImportResult {...})`, add workspace.json reading. Replace the existing `Ok(ImportResult { ... })` block:

```rust
    let metadata: Option<WorkspaceMetadata> =
        try_read_entry(&mut archive, "workspace.json", password)
            .and_then(|cursor| serde_json::from_reader(cursor).ok());

    Ok(ImportResult {
        app_version: export_notes.app_version,
        note_count: export_notes.notes.len(),
        script_count,
        metadata,
    })
```

- [ ] **Step 3: Update `import_workspace` return to include metadata**

In `import_workspace` (line ~546-550), the `Ok(ImportResult { ... })` also needs the new field. The metadata was already read at line 502 into `workspace_metadata`. Add it:

```rust
    Ok(ImportResult {
        app_version: export_notes.app_version,
        note_count: export_notes.notes.len(),
        script_count,
        metadata: workspace_metadata,
    })
```

- [ ] **Step 4: Run `cargo check -p krillnotes-core`**

Run: `cargo check -p krillnotes-core`
Expected: compiles successfully

- [ ] **Step 5: Commit**

```bash
git add krillnotes-core/src/core/export.rs
git commit -m "feat: include WorkspaceMetadata in ImportResult from peek_import"
```

---

### Task 2: Test metadata round-trip in `peek_import`

**Files:**
- Modify: `krillnotes-core/src/core/export_tests.rs`

- [ ] **Step 1: Write a test that sets workspace metadata, exports, and peeks**

Add this test after the existing `test_peek_import_reads_metadata` test (around line 115):

```rust
#[test]
fn test_peek_import_includes_workspace_metadata() {
    let temp = NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(
        temp.path(), "", "test-identity",
        ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]),
        test_gate(), None,
    ).unwrap();

    let meta = WorkspaceMetadata {
        version: 1,
        author_name: Some("Alice".to_string()),
        author_org: Some("Acme".to_string()),
        homepage_url: None,
        description: Some("A test workspace".to_string()),
        license: Some("MIT".to_string()),
        license_url: None,
        language: Some("en".to_string()),
        tags: vec!["test".to_string(), "demo".to_string()],
    };
    ws.set_workspace_metadata(&meta).unwrap();

    let mut buf = Vec::new();
    export_workspace(&ws, Cursor::new(&mut buf), None).unwrap();

    let result = peek_import(Cursor::new(&buf), None).unwrap();
    let peeked = result.metadata.expect("metadata should be present");
    assert_eq!(peeked.author_name.as_deref(), Some("Alice"));
    assert_eq!(peeked.author_org.as_deref(), Some("Acme"));
    assert_eq!(peeked.description.as_deref(), Some("A test workspace"));
    assert_eq!(peeked.license.as_deref(), Some("MIT"));
    assert_eq!(peeked.language.as_deref(), Some("en"));
    assert_eq!(peeked.tags, vec!["test".to_string(), "demo".to_string()]);
}

#[test]
fn test_peek_import_returns_none_metadata_for_old_archives() {
    // Build a minimal zip with only notes.json (no workspace.json)
    let mut buf = Vec::new();
    {
        let cursor = Cursor::new(&mut buf);
        let mut zip = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default();

        let notes = serde_json::json!({
            "version": 1,
            "app_version": "0.0.0",
            "notes": []
        });
        zip.start_file("notes.json", options).unwrap();
        zip.write_all(serde_json::to_string(&notes).unwrap().as_bytes()).unwrap();
        zip.finish().unwrap();
    }

    let result = peek_import(Cursor::new(&buf), None).unwrap();
    assert!(result.metadata.is_none(), "old archives without workspace.json should return None metadata");
}
```

- [ ] **Step 2: Run the new tests**

Run: `cargo test -p krillnotes-core test_peek_import_includes_workspace_metadata test_peek_import_returns_none_metadata_for_old_archives -- --nocapture`
Expected: both PASS

- [ ] **Step 3: Run all export tests to check for regressions**

Run: `cargo test -p krillnotes-core export_tests -- --nocapture`
Expected: all tests PASS

- [ ] **Step 4: Commit**

```bash
git add krillnotes-core/src/core/export_tests.rs
git commit -m "test: verify peek_import returns workspace metadata"
```

---

### Task 3: Extend `ImportState` and wire metadata through the frontend

**Files:**
- Modify: `krillnotes-desktop/src/hooks/useDialogState.ts:11-15`
- Modify: `krillnotes-desktop/src/App.tsx:152-174`

- [ ] **Step 1: Add metadata fields to `ImportState`**

In `krillnotes-desktop/src/hooks/useDialogState.ts`, extend the interface:

```typescript
export interface ImportState {
  zipPath: string;
  noteCount: number;
  scriptCount: number;
  metadata?: {
    authorName?: string;
    authorOrg?: string;
    homepageUrl?: string;
    description?: string;
    license?: string;
    licenseUrl?: string;
    language?: string;
    tags?: string[];
  };
}
```

- [ ] **Step 2: Pass metadata from peek result into `importState`**

In `App.tsx`, the `proceedWithImport` function (line 152-188) invokes `peek_import_cmd` and sets `importState`. Update the invoke type and the `setImportState` call:

Change the invoke type (line 154):
```typescript
const result = await invoke<{
  appVersion: string;
  noteCount: number;
  scriptCount: number;
  metadata?: {
    authorName?: string;
    authorOrg?: string;
    homepageUrl?: string;
    description?: string;
    license?: string;
    licenseUrl?: string;
    language?: string;
    tags?: string[];
  };
}>('peek_import_cmd', { zipPath, password });
```

Update the `setImportState` call (line 170-174):
```typescript
setImportState({
  zipPath,
  noteCount: result.noteCount,
  scriptCount: result.scriptCount,
  metadata: result.metadata,
});
```

- [ ] **Step 3: Run TypeScript type check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: no errors

- [ ] **Step 4: Commit**

```bash
git add krillnotes-desktop/src/hooks/useDialogState.ts krillnotes-desktop/src/App.tsx
git commit -m "feat: pass workspace metadata from peek_import to ImportState"
```

---

### Task 4: Add i18n keys to all 7 locale files

**Files:**
- Modify: all 7 files in `krillnotes-desktop/src/i18n/locales/`

- [ ] **Step 1: Add keys to `en.json`**

In the `"import"` section (inside `"dialogs"`), add these keys after `"versionMismatchTitle"`:

```json
"by": "By",
"homepage": "Homepage",
"license": "License"
```

The full `dialogs.import` section should become:
```json
"import": {
  "title": "Import Workspace",
  "importedPlaceholder": "imported-workspace",
  "versionMismatch": "This export was created with Krillnotes v{{version}}, but you are running v{{currentVersion}}. Some data may not import correctly.\n\nImport anyway?",
  "versionMismatchTitle": "Version Mismatch",
  "by": "By",
  "homepage": "Homepage",
  "license": "License"
},
```

- [ ] **Step 2: Add keys to `de.json`**

```json
"by": "Von",
"homepage": "Homepage",
"license": "Lizenz"
```

- [ ] **Step 3: Add keys to `es.json`**

```json
"by": "Por",
"homepage": "Página web",
"license": "Licencia"
```

- [ ] **Step 4: Add keys to `fr.json`**

```json
"by": "Par",
"homepage": "Page d'accueil",
"license": "Licence"
```

- [ ] **Step 5: Add keys to `ja.json`**

```json
"by": "作成者",
"homepage": "ホームページ",
"license": "ライセンス"
```

- [ ] **Step 6: Add keys to `ko.json`**

```json
"by": "작성자",
"homepage": "홈페이지",
"license": "라이선스"
```

- [ ] **Step 7: Add keys to `zh.json`**

```json
"by": "作者",
"homepage": "主页",
"license": "许可证"
```

- [ ] **Step 8: Commit**

```bash
git add krillnotes-desktop/src/i18n/locales/*.json
git commit -m "i18n: add import metadata display keys to all 7 locales"
```

---

### Task 5: Render workspace metadata in the import dialog

**Files:**
- Modify: `krillnotes-desktop/src/App.tsx:380-450` (import dialog JSX)

- [ ] **Step 1: Add the collapsible metadata block**

In `App.tsx`, inside the import dialog (after the "importing progress" `<p>` at line 385-386 and before the name label `<div>` at line 387), add a collapsible `<details>` block. This matches the pattern in `AcceptInviteWorkflow.tsx:376-410`:

```tsx
{importState.metadata && (importState.metadata.description || importState.metadata.authorName || importState.metadata.license || (importState.metadata.tags && importState.metadata.tags.length > 0)) && (
  <details className="border border-secondary rounded-md mb-4">
    <summary className="px-3 py-2 text-sm font-medium cursor-pointer select-none hover:bg-secondary/50">
      {t('workspace.propertiesTitle')}
    </summary>
    <div className="px-3 pb-3 pt-1 space-y-1">
      {importState.metadata.description && (
        <p className="text-sm text-muted-foreground">{importState.metadata.description}</p>
      )}
      {importState.metadata.authorName && (
        <p className="text-xs text-muted-foreground">
          {t('dialogs.import.by')} {importState.metadata.authorName}
          {importState.metadata.authorOrg && ` (${importState.metadata.authorOrg})`}
        </p>
      )}
      {importState.metadata.homepageUrl && (
        <p className="text-xs text-muted-foreground">
          {t('dialogs.import.homepage')}: {importState.metadata.homepageUrl}
        </p>
      )}
      {importState.metadata.license && (
        <p className="text-xs text-muted-foreground">
          {t('dialogs.import.license')}: {importState.metadata.license}
        </p>
      )}
      {importState.metadata.tags && importState.metadata.tags.length > 0 && (
        <div className="flex flex-wrap gap-1 pt-1">
          {importState.metadata.tags.map((tag) => (
            <span key={tag} className="text-xs bg-secondary px-2 py-0.5 rounded-full">
              {tag}
            </span>
          ))}
        </div>
      )}
    </div>
  </details>
)}
```

Note: `t('workspace.propertiesTitle')` resolves to "Workspace Properties" — this key already exists in `en.json` at the `workspace.propertiesTitle` path (line 128).

- [ ] **Step 2: Run TypeScript type check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: no errors

- [ ] **Step 3: Commit**

```bash
git add krillnotes-desktop/src/App.tsx
git commit -m "feat: display workspace metadata in import confirmation dialog"
```

---

### Task 6: Manual verification

- [ ] **Step 1: Run all Rust tests**

Run: `cargo test -p krillnotes-core`
Expected: all tests PASS

- [ ] **Step 2: Run TypeScript type check**

Run: `cd krillnotes-desktop && npx tsc --noEmit`
Expected: no errors

- [ ] **Step 3: Start dev mode and test the import flow**

Run: `cd krillnotes-desktop && npm run tauri dev`

Test steps:
1. Export a workspace that has metadata set (author, description, license, tags) — use the Workspace Properties dialog to set these first
2. Import the `.krillnotes` file
3. Verify the import dialog shows a collapsible "Workspace Properties" section
4. Expand it — verify description, author, license, and tags are shown
5. Import an old archive (without workspace.json) — verify the section is hidden gracefully
6. Check that note count and script count still display correctly

- [ ] **Step 4: Final commit if any fixes were needed**
