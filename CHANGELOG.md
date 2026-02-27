# Changelog

All notable changes to Krillnotes will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Hover tooltip on tree nodes** — Hovering a tree node for 600ms shows a compact speech-bubble tooltip to the right of the tree panel, without needing to navigate to the note. Two render paths are supported: mark any field with `show_on_hover: true` for an instant inline preview (no IPC), or define an `on_hover` hook in `schema()` to return fully custom HTML via the Rhai scripting engine. The tooltip is a React portal, position-clamped to the viewport, with a left-pointing spike that tracks the hovered row. It dismisses immediately on mouse-leave, click, or drag start.
- **`on_hover` hook** — A new optional hook inside `schema()` blocks. Like `on_view`, it receives a note map and has access to all query functions (`get_children`, `get_notes_for_tag`, etc.) and display helpers (`field`, `stack`, `markdown`, …). The return value is rendered as HTML in the tooltip.
- **`show_on_hover` field flag** — Fields defined with `show_on_hover: true` are surfaced in the hover tooltip without any scripting. Useful for quick previews of a single key field (e.g. a body or description).
- **Zettelkasten template updated** — The bundled `zettelkasten.rhai` now demos both hover paths: Zettel notes show the body field on hover; Kasten folders show a live child-count badge via `on_hover`.

---

## [0.2.4] — 2026-02-27

### Added
- **Theme support** — Choose between Light, Dark, and System (follows OS preference) modes from Settings. The active theme applies to all open workspace windows simultaneously; changing the theme in one window instantly updates every other open window.
- **Manage Themes dialog** — Browse, preview, create, edit, and delete custom `.krilltheme` files from a dedicated dialog in Settings. Built-in Light and Dark themes are always available as a baseline.
- **Import theme from file** — A new "Import from file…" button in the Manage Themes dialog lets you load a `.krilltheme` file from disk directly into the editor. If a theme with the same name already exists, a warning banner appears and the Save button becomes "Replace", with a confirmation dialog before overwriting.
- **Import script from file** — A matching "Import from file…" button in the Script Manager loads a `.rhai` file from disk into the script editor. Conflict detection is by `@name` front-matter; same replace-with-confirm flow applies.
- **Split Add Note** — The "Add Note" button is now split into three distinct actions — **Add Child**, **Add Sibling**, and **Add Root Note** — eliminating the type-selection dialog when only one target position makes sense.

### Fixed
- **Theme settings are now application-wide** — Theme mode (light/dark/system) is stored in the shared `settings.json` and applies to all workspaces. Previously, opening a new workspace window could show the wrong theme because the Settings dialog was clobbering the theme fields on every save.
- **Settings save no longer resets theme** — `update_settings` now accepts a partial patch and merges it onto the current settings on disk, so callers that only update workspace directory or password-caching cannot inadvertently reset unrelated fields to their defaults.
- **Workspace menu items disabled until a workspace is open** — File › Export Workspace and other workspace-specific menu items are now greyed out on the initial launch screen and only enabled once a workspace window is open.
- **`window.confirm()` replaced with async dialog** — Native `window.confirm()` is non-blocking in Tauri's WKWebView on macOS (always returns `true` immediately). All confirmation dialogs now use `await confirm()` from `@tauri-apps/plugin-dialog`, fixing silent data-loss on destructive actions.
- **`.krillnotes` file format** — Export archives now use the `.krillnotes` extension. The underlying format is unchanged (standard zip); only the file extension and dialog filters have changed.
- **Importing older archives** — Archives exported before the tags feature (v0.2.3) no longer fail to import. The missing `tags` field on notes now defaults to an empty list instead of causing a deserialisation error.

### Changed
- **App renamed to Krillnotes** — The application bundle, window title, and bundle identifier are now `Krillnotes` / `com.careck.krillnotes` (previously `krillnotes-desktop` / `com.careck.krillnotes-desktop`).

---

## [0.2.3] — 2026-02-26

### Added
- **`note_link` field type** — A new field type that stores a reference to another note by its ID. In edit mode an inline search dropdown lets you find and link a note by title or any text field; an optional `target_type` restricts the picker to notes of a specific schema type. In view mode (default and `on_view` hooks) the linked note's title is rendered as a clickable navigation link. If the linked note is deleted, the field is automatically set to null in all source notes.
- **`get_notes_with_link(note_id)` query function** — Returns all notes that have any `note_link` field pointing to the given note ID. Available in `on_view` hooks and `add_tree_action` closures. Use this to display backlinks on a target note (e.g. show all Tasks that link to a Project).
- **Tags** — Any note can carry free-form tags. Add and remove tags from the tag pill editor in the InfoPanel. Tag pills are shown in the default note view. A resizable tag cloud panel in the tree sidebar lets you browse all tags in the workspace at a glance.
- **Tag search** — The search bar now matches tags in addition to note titles and text fields.
- **Template gallery** — `templates/` ships two ready-to-use template scripts: `book_collection.rhai` (a library organiser with an `on_view` table and sort actions) and `zettelkasten.rhai` (an atomic-note system with auto-titling and related-note discovery via shared tags). Copy a template into the Script Manager to activate it.
- **`note.tags` in `on_view` hooks** — The note map passed to `on_view` now includes a `tags` array, enabling scripts to read and display the note's tags.
- **`render_tags(tags)` display helper** — Renders a `note.tags` array as coloured pill badges.
- **`get_notes_for_tag(tags)` query function** — Returns all notes that carry any of the given tags (OR semantics, deduplicated). Available in `on_view` hooks and `add_tree_action` closures.
- **`today()` scripting function** — Returns today's date as a `"YYYY-MM-DD"` string. Useful in `on_save` hooks to auto-stamp date fields or derive a date-prefixed title.
- **Tags in export / import** — `workspace.json` now includes a global tag list and each note's tags array. Import restores all tag assignments.
- **Book collection template** — A full library management template (previously a bundled system script) moved to the template gallery as `templates/book_collection.rhai`.

---

## [0.2.2] — 2026-02-26

### Added
- **`create_note` and `update_note` in tree actions** — `add_tree_action` closures can now create new notes and modify existing ones, not just reorder children. `create_note(parent_id, node_type)` inserts a note with schema defaults and returns a map you can edit; `update_note(note)` persists title and field changes back to the database. All writes from a single action execute inside one SQLite transaction — any error rolls back everything. Notes created earlier in the same closure are immediately visible to `get_children()` and `get_note()`, so full subtrees can be built in one action.

---

## [0.2.1] — 2026-02-25

### Added
- **`on_add_child` hook** — Scripts can now define an `on_add_child` hook that fires whenever a child note is created under or moved to a parent note. The hook receives the parent and the new child, and can modify either before the operation completes.
- **Tree context menu actions** — Scripts can register custom actions via `add_tree_action(label, fn)`. Registered actions appear in the right-click context menu of tree nodes and are invoked with the selected note as an argument. The bundled Text Note script includes a "Sort Children A→Z" example action.
- **Schema name collision detection** — Krillnotes now detects when two scripts register schemas with the same name and reports an error at load time instead of silently overwriting one with the other.

### Fixed
- Note struct state is now synced with any `on_add_child` hook modifications before being written to the operations log, ensuring the logged snapshot reflects the final saved values.

---

## [0.2.0] — 2026-02-24

> **Breaking change:** The workspace file format has changed due to database encryption. Workspaces created with v0.1.x cannot be opened directly — export them from the old version and re-import into v0.2.0.

### Added
- **Database encryption** — All workspaces are now encrypted at rest using SQLCipher (AES-256). Passwords are stored in the OS keychain by default, with a toggle to cache them in-session only. Existing unencrypted workspaces must be exported and re-imported.
- **Encrypted exports** — Export archives can be password-protected with AES-256. Krillnotes automatically detects encrypted archives on import and prompts for the password.
- **Markdown rendering** — Textarea fields are rendered as Markdown in view mode. The raw text is still accessible in scripts and edit mode. A `markdown()` helper is also available in `on_view` scripts.
- **Hooks inside schema** — `on_save` and `on_view` hooks are now defined directly inside the `schema()` block, making scripts self-contained and removing any ambiguity about which hook runs for a given note type.
- **Script compile error reporting** — Saving a user script that contains a syntax or compile error now shows an error message instead of silently discarding the save.
- **Script name in hook error messages** — Runtime errors thrown by `on_save` or `on_view` hooks now include the name of the script that caused the error, making debugging much easier.
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

[Unreleased]: https://github.com/careck/krillnotes/compare/v0.2.4...HEAD
[0.2.4]: https://github.com/careck/krillnotes/compare/v0.2.3...v0.2.4
[0.2.3]: https://github.com/careck/krillnotes/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/careck/krillnotes/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/careck/krillnotes/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/careck/krillnotes/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/careck/krillnotes/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/careck/krillnotes/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/careck/krillnotes/releases/tag/v0.1.0
