# Changelog

All notable changes to Krillnotes will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Database encryption** — All workspaces are now encrypted at rest using SQLCipher (AES-256). Passwords are stored in the OS keychain by default, with a toggle to cache them in-session only. Existing unencrypted workspaces must be exported and re-imported.
- **Encrypted exports** — Export archives can be password-protected with AES-256. Krillnotes automatically detects encrypted archives on import and prompts for the password.
- **Markdown rendering** — Textarea fields are rendered as Markdown in view mode. The raw text is still accessible in scripts and edit mode. A `markdown()` helper is also available in `on_view` scripts.
- **Hooks inside schema** — `on_save` and `on_view` hooks are now defined directly inside the `schema()` block, making scripts self-contained and removing any ambiguity about which hook runs for a given note type.
- **Script compile error reporting** — Saving a user script that contains a syntax or compile error now shows an error message instead of silently discarding the save.
- **Script name in hook error messages** — Runtime errors thrown by `on_save` or `on_view` hooks now include the name of the script that caused the error, making debugging much easier.

### Added
- **Copy and paste notes** — Any note (and its entire descendant subtree) can be copied and pasted as a child or sibling of any compatible target note. Available via right-click context menu, Edit menu, and keyboard shortcuts (⌘C / ⌘V / ⌘⇧V). Schema constraints are enforced silently — invalid paste targets are ignored, matching the behaviour of drag-and-drop move.
- **Humanised field labels** — field names are now displayed in Title Case in both view and edit mode (e.g. `note_title` → "Note Title", `first_name` → "First Name").
- **Script load-order drag reordering** — User scripts in the Script Manager can now be reordered by dragging the grip handle on the left of each row. The visual order in the list is immediately persisted to the database and the script engine reloads in the new order.

### Fixed
- Workspace names containing spaces are now accepted; the name is stored as-is and only the filename is slugified automatically.
- Exported archive filenames now default to the workspace name instead of a generic placeholder.
- `on_view` hook runtime errors are now surfaced to the user instead of silently falling back to the default view.

---

## [0.1.2] — 2026-02-23

### Fixed
- On Windows, workspace windows opened after startup were missing the menu bar. They now correctly receive the full application menu at creation time.

---

## [0.1.1] — 2026-02-23

### Fixed
- On Windows, menu events were incorrectly broadcast to all open windows. Events are now routed only to the focused window.

---

## [0.1.0] — 2026-02-23 — First release

### Added

#### Core note-taking
- Hierarchical tree-based note structure with unlimited nesting.
- Create, view, edit, and delete notes from the tree or via keyboard shortcuts.
- Notes are auto-selected and opened in edit mode immediately on creation.
- Drag-and-drop reordering: move notes among siblings or reparent them anywhere in the tree.
- Keyboard navigation: arrow keys move through the tree, Enter opens edit mode, Escape cancels.
- Resizable split between the tree panel and the note view/edit panel.
- Global search bar with instant dropdown results and automatic ancestor expansion so the matched note is always visible in the tree.

#### Scripting and note schemas
- Note types are defined via [Rhai](https://rhai.rs) scripts, giving full control over fields, validation, and display.
- **User scripts** are stored inside the workspace database — no separate files to manage. Each workspace has its own independent set of scripts.
- **Script Manager** UI: list, create, edit (CodeMirror editor), reload, and delete scripts. A warning is shown before deleting a script that defines a schema with existing data.
- System scripts are seeded into every new workspace and can be edited or deleted freely.
- **Field types**: `text` (single-line), `textarea` (multi-line), `date`, `email`, `boolean`, `select` (dropdown), `rating` (star widget).
- **Field visibility flags**: control whether a field appears in view mode, edit mode, or both. Optionally lock the note title from being edited (e.g. when it is derived by an `on_save` hook).
- **`on_save` hook**: transform or derive field values before a note is saved (e.g. auto-build a contact's display name from first and last name fields).
- **`on_view` hook**: return custom HTML to render a note, with access to the note's children. Includes a simple DSL — `table()`, `heading()`, `paragraph()`, `link_to()`, and more — so scripts stay readable without raw HTML string building.
- **`link_to(note)`**: creates a clickable link in a view that navigates to another note. Includes full back-navigation history and a back button.
- **Children sort**: schemas can specify whether child notes are sorted by title (ascending or descending) or kept in manual drag-and-drop order.
- **Parent/child constraints**: a schema can declare which parent types it may be placed under, and which child types are allowed beneath it. The tree enforces these constraints during drag-and-drop and note creation.

#### Built-in note types (bundled scripts)
- **Text Note** — title and multi-line body
- **Contact** — first name, last name, email, phone, address, notes, family flag; title auto-derived
- **Book** — title, author, genre, status, rating, date started/finished, notes
- **Task** — title, description, due date, priority, status, tags
- **Project** — title, description, status, start/end dates, owner, budget, notes
- **Product** — name, SKU, category, price, stock, description
- **Recipe** — title, cuisine, servings, prep/cook time, ingredients, instructions

#### Workspaces
- Each workspace is a self-contained SQLite database file.
- Configurable default workspace directory with sensible OS defaults (`~/Documents/Krillnotes`).
- New Workspace dialog and Open Workspace list dialog; no raw file pickers needed.
- Multiple workspaces can be open simultaneously, each in its own window.

#### Operations log
- Every create, update, and delete action is recorded with a timestamp and the affected note title.
- Operations log viewer with filtering by type and date range.
- Purge button to compact the log and reduce database size.

#### Export / Import
- Export a workspace as a ZIP archive containing a JSON data file and all user scripts as `.rhai` files — suitable for sharing or backup.
- Import a ZIP archive into a new workspace.

#### UI and application
- Compact grid layout for note fields in view mode; empty fields are hidden automatically.
- Collapsible metadata section for system-level fields.
- Right-click context menus on tree nodes (edit, delete with confirmation).
- Platform-aware menus: macOS app menu, Edit menu with standard shortcuts; Tools menu for Operations Log and Script Manager.
- Cross-platform release workflow via GitHub Actions (macOS, Windows, Linux).

[Unreleased]: https://github.com/careck/krillnotes/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/careck/krillnotes/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/careck/krillnotes/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/careck/krillnotes/releases/tag/v0.1.0
