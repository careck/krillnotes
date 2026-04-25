# Identity-Neutral Export

**Issue:** [#155](https://github.com/2pisoftware/krillnotes/issues/155) — Exporting a shared workspace as owner creates unusable workspace

**Date:** 2026-04-25

## Problem

When an owner exports a shared workspace and a different identity imports it, the archive carries the original `owner_pubkey`. Import restores that key, so the importer is not recognized as root owner and cannot use the workspace.

## Principle

A `.krillnotes` archive is completely identity-neutral. It contains content, attachments, and scripts — nothing else.

## Scope

Export path only. Duplicate reuses export/import but the same identity owns both sides, so it works correctly today.

## Changes

### 1. Export (`export_workspace` in `export.rs`)

- Set `ws_meta.owner_pubkey = None` before writing `workspace.json` (currently writes the owner's pubkey).
- Clear `created_by` and `modified_by` to `""` on each note before writing `notes.json`.

After these changes the archive contains zero identity data.

### 2. Import (`import_workspace` in `export.rs`)

- Remove the `set_owner_pubkey` restoration block (lines 574-582). The importer's identity, set by `Workspace::open()`, naturally becomes root owner.
- After bulk-inserting notes, run `UPDATE notes SET created_by = ?, modified_by = ?` with `workspace.identity_pubkey()` so the importer is recorded as creator/modifier of all imported notes.

### 3. Tests (`export_tests.rs`)

- **Archive contents test:** Export a workspace with notes, read the zip, deserialize `workspace.json` and `notes.json`. Assert `owner_pubkey` is absent and all `created_by`/`modified_by` fields are empty strings.
- **Round-trip identity test:** Export workspace A (owned by identity X), import as workspace B (owned by identity Y). Assert `owner_pubkey` of B matches identity Y. Assert all notes in B have `created_by` and `modified_by` equal to identity Y's pubkey.

## Identity Data Audit

| Location | Field | Current | After fix |
|----------|-------|---------|-----------|
| `workspace.json` | `owner_pubkey` | Original owner's pubkey | `None` (omitted) |
| `notes.json` | `created_by` | Original author's pubkey | `""` (empty) |
| `notes.json` | `modified_by` | Last modifier's pubkey | `""` (empty) |
| `operations` table | (all columns) | N/A — already excluded from export | No change |
| `attachments` | (no identity fields) | Clean | No change |
| `user_scripts` | (no identity fields) | Clean | No change |

## Non-Goals

- Backward compatibility with old archives carrying `owner_pubkey` (no such archives exist without the bug).
- Changes to the duplicate workflow (works correctly today).
- Stripping `verified_by` (lives on the operations table, already excluded from export).
