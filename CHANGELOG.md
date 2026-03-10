# Changelog

All notable changes to Krillnotes will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Per-identity encrypted contact book (Phase A)** — contacts are stored per identity under
  `~/.config/krillnotes/identities/<uuid>/contacts/` as AES-256-GCM encrypted blobs. The
  encryption key is derived via HKDF-SHA256 from the identity seed and is only in memory while
  the identity is unlocked. Full CRUD via six new Tauri commands (`list_contacts`, `get_contact`,
  `create_contact`, `update_contact`, `delete_contact`, `get_fingerprint`). UI accessible via
  a "Contacts (n)" button in the Identity Manager — opens `ContactBookDialog` with search,
  trust-level badges, `AddContactDialog` (with live fingerprint preview and in-person
  verification gate), and `EditContactDialog` (with local name override, notes, and delete).
- **`is_leaf` schema option** — When `is_leaf: true` is set on a schema, notes of that
  type cannot have children. Blocked in core (`create_note`, `move_note`, `deep_copy_note`)
  and observed in the UI ("Add Child" and "Paste as Child" are greyed out; drag-drop onto
  leaf notes is blocked).
- **Swarm contacts data model** — `Contact` struct and core CRUD in `krillnotes-core`; UI not yet wired (A1)
- **Sync peers table and PeerRegistry** — tracks known peers and their sync state per workspace (A2)
- **SwarmHeader codec and bundle-level signatures** — all `.swarm` file payloads are signed with Ed25519 and verified on open (A3, A4)
- **Hybrid encryption for `.swarm` payloads** — X25519 key exchange + AES-256-GCM payload encryption (A5)
- **Invite and accept bundle codec** — generate and parse peer invite bundles (A6, A7)
- **Snapshot bundle generation and parsing** — full workspace snapshot serialised to `.swarm` file (A10, A11)
- **Delta bundle generation and ingest stub** — incremental sync payload codec (A12, A13)
- **`SetPermission`, `RevokePermission`, `JoinWorkspace` operation variants** — CRDT operations for future RBAC sync
- **`.swarm` file association** — OS registers `.swarm` files with Krillnotes; double-click opens the correct dialog
- **File > Invite Peer and Open .swarm File menu items**
- **SwarmInviteDialog** — UI for generating and sharing a peer invite bundle
- **SwarmOpenDialog** — UI for accepting an invite, opening a snapshot, or ingesting a delta bundle
- **`create_snapshot_bundle_cmd` and `open_swarm_file_cmd` Tauri commands**
- **`WorkspaceSnapshot` struct and `to_snapshot_json` / `import_snapshot_json`** — serialise/deserialise full workspace state for peer handoff
- **Show and copy public key and fingerprint in Identity Manager** — for sharing with peers
- Auto-prompt to unlock required identity when opening an invite or snapshot file
- i18n strings for all swarm dialogs (invite, accept, snapshot modes)

### Added
- **Hover indicator caret on tree nodes** — a subtle `›` is shown on the right of tree node rows when the note type has an `on_hover` hook or `showOnHover` fields defined
- **Identity/contact name in note Info panel** — Created and Modified timestamps now show the author's display name inline (local identity first, then contact address book, then 8-char fingerprint for unknown keys)
- **`resolve_identity_name` Tauri command** — resolves a public key to a display name; used by both the info panel and the operations log
- **`ContactManager` wired into `AppState`** — enables contact address book lookups from any Tauri command

### Fixed
- **Workspace Properties dialog no longer crashes on workspaces with no tags** — `meta.tags` is now guarded with `?? []` before calling `.join()` (was `TypeError: undefined is not an object`)
- **Script category preserved on export/import** — `ScriptManifestEntry` now includes the `category` field so schema vs. presentation classification survives a `.krillnotes` archive round-trip. Previously all scripts were imported as `"presentation"`, causing schema scripts to fail with *"schema() can only be called from schema-category scripts"* and library-defined functions to be unavailable (PR #89)
- Library script functions are now visible to schema scripts and their hooks — library source is prepended when compiling schema scripts so functions defined in `.rhai` library scripts are available at both load time and hook call time
- `register_view` and `register_menu` no longer produce duplicate tabs/entries when a library script is loaded alongside multiple schema scripts (deduplication by type + label in `resolve_bindings`)
- Snapshot import no longer seeds a default root note, preserving the imported workspace structure
- Identity file path resolved relative to `config_dir` in `get_identity_public_key`
- `source_display_name` correctly populated in invite bundles
- Unlocked identity UUID refreshes when Identity Manager closes or Swarm dialog opens
- Documented that top-level `const`/variable declarations in library scripts are not available inside hook closures — use `fn` returning a value instead; SCRIPTING.md updated with examples
- Schema script pre-validation in `update_user_script` now sets the loading category so library functions are available during validation — previously caused false "function not found" errors when saving a schema script that calls a library function
- Hover tooltip no longer appears for notes whose type has no `on_hover` hook and no `showOnHover` fields
- Operations log now checks the contact address book when resolving author names, in addition to local identities
- Note Info panel metadata now uses the same `dl/dt/dd` grid layout as the fields view
- Note Info panel metadata section is hidden on custom view tabs and only shown on the Fields tab

### Changed
- **Breaking (Rhai scripts):** `note.node_type` renamed to `note.schema` in all Rhai script contexts.
  Update any user scripts that reference `note.node_type` → `note.schema`.
- `Note` JSON key changed from `nodeType` to `schema` in workspace exports.
  Old `.krillnotes` archives with `nodeType` are still importable (backward compat preserved via serde alias).
- **Breaking (Rhai scripts):** Schema constraint keys renamed — `allowed_parent_types` → `allowed_parent_schemas`,
  `allowed_children_types` → `allowed_children_schemas`. Update any schema definitions that use the old keys.
- **Breaking (Rhai scripts):** `note_link` field option `target_type` renamed to `target_schema`.
  Update any schema definitions that use `target_type` on a `note_link` field.

## [0.3.0] — 2026-03-07

> **Breaking changes:** This release introduces an identity-based authentication system (workspaces from v0.2.x must be exported and re-imported), a new scripting API (`save_note` replaces `update_note`, `register_view`/`register_hover`/`register_menu` replace inline hooks, schema versioning is now required), and HLC-based operation timestamps that update the database schema. Additionally, the project is now licensed under MPL-2.0 (previously MIT).

### Added
- **Operation detail panel** — Clicking any row in the Operations Log now opens a side panel showing all fields stored for that operation. The dialog expands from 700 px to 1080 px; clicking the selected row or the ✕ button closes the panel. Author-key fields (`created_by`, `modified_by`, etc.) display the resolved identity display name below the raw public-key hash.
- **Identity system** — A cryptographic identity (an Ed25519 keypair protected by an Argon2id-derived passphrase) now manages workspace access. Each workspace is bound to an identity; the workspace's randomly-generated database password is stored encrypted under the identity key. You unlock your identity once per session with your passphrase, and all bound workspaces open without any additional password prompts.
- **Identity Manager** — A new Identity Manager dialog (accessible from Settings) lets you create, rename, unlock, lock, and delete identities. Each identity shows its UUID and the list of workspaces bound to it.
- **`.swarmid` export/import** — Identities can be exported as a portable `.swarmid` file (encrypted JSON containing your key material). Import a `.swarmid` file on another device to access the same workspaces. On import, an existing identity with the same UUID can be overwritten while preserving all workspace bindings.
- **Workspace Manager** — Replaces the minimal Open Workspace dialog with a full manager. The list shows each workspace's name, last-modified date, and size on disk, sortable by name or modified date. Selecting a workspace reveals an info panel with created date, note count, attachment count, and size — all read from an unencrypted `info.json` sidecar so no password is required just to view metadata. Per-workspace actions: **Open** (requires the bound identity to be unlocked; also triggered by double-clicking a row), **Duplicate** (uses the export→import pipeline; prompts for new name), **Delete** (irreversible red confirmation banner; blocked if the workspace is currently open), and **New** (opens the New Workspace dialog and binds the new workspace to your unlocked identity).
- **Random workspace passwords** — New workspaces no longer ask for a user-visible password. A cryptographically random 32-byte base64 key is generated at creation time, used as the SQLCipher database password, and immediately encrypted under the bound identity. Users never see or type a workspace password.
- **HLC timestamps on operations** — Every mutation is now timestamped with a Hybrid Logical Clock (`wall_ms`, `counter`, `node_id`) instead of a plain Unix integer. HLC timestamps provide causal ordering guarantees even when clocks skew across devices, which is a prerequisite for CRDT merge.
- **Ed25519-signed operations** — Each mutation carries an Ed25519 signature produced by the unlocked identity's signing key. Operations can be verified against the author's public key, laying the foundation for trustless multi-device sync.
- **`UpdateNote` and `SetTags` operation variants** — Title changes now emit a dedicated `UpdateNote` operation (separate from field-level `UpdateField`) to enable last-write-wins conflict resolution on note titles. Tag assignments now emit `SetTags` and are recorded in the operations log for the first time.
- **Author display in Operations Log** — Each row in the Operations Log now shows a short author identifier (first 8 characters of the base64-encoded public key), resolved to the identity's display name when the identity is loaded.
- **Gated operations model (`SaveTransaction`)** — Replaces direct-mutation `on_save` hooks with a transactional API. Scripts now use `set_field()`, `set_title()`, `reject()`, and `commit()` to express mutations declaratively. A 7-step save pipeline (`save_note_with_pipeline`) runs visibility → validate → required → update, ensuring hooks cannot leave a note in an inconsistent state.
- **Field groups** — Schemas can define `field_groups` in `schema()` to visually organise related fields under collapsible sections. Each group supports an optional `visible` closure that dynamically shows or hides the section based on the current field values (e.g. show "Completion details" only when status is "done").
- **Field-level `validate` closures** — Individual field definitions accept a `validate: |v| ...` closure that returns an error string or `()`. Validation runs on-blur in the frontend (inline error under the field) and as a hard gate inside `set_field()` during saves.
- **Note-level `reject()`** — `on_save` hooks can call `reject("message")` to abort a save with a structured error. The frontend displays rejected messages in a note-level error banner above the fields.
- **Script categories** — Scripts are now divided into two categories: **Schema** (`.schema.rhai`) and **Library/Presentation** (`.rhai`). Schema scripts define note types via `schema()`. Presentation scripts define views, hover renderers, and context-menu actions via `register_view()`, `register_hover()`, and `register_menu()`. Calling `schema()` from a presentation script raises a hard error.
- **Two-phase script loading** — On workspace open, presentation scripts load first (Phase A), then schema scripts (Phase B), then deferred view/hover/menu bindings are resolved (Phase C). Library helper functions defined in `.rhai` files are available when schema `on_save` hooks execute.
- **`register_view(type, label, closure)` / `register_view(type, label, options, closure)`** — Registers a named view tab for a note type from a presentation script. Replaces the `on_view` key inside `schema()`. Closures have access to all query functions and display helpers. `display_first: true` pushes the tab to the leftmost position.
- **`register_hover(type, closure)`** — Registers a hover tooltip renderer for a note type from a presentation script. Replaces the `on_hover` key inside `schema()`. Last registration wins.
- **`register_menu(label, types, closure)`** — Registers a context-menu action for one or more note types from a presentation script. Replaces `add_tree_action()`. Closures use the SaveTransaction API for mutations.
- **Tabbed view mode** — When a schema has registered views, the note detail panel shows a tab bar. Custom view tabs appear in registration order; `display_first: true` tabs are leftmost; the Fields tab is always present and always rightmost. No tab bar is shown for types with no registered views.
- **Script Manager category badges and creation flow** — Each script in the manager shows a coloured badge: blue **Schema** or amber **Library**. The "New Script" dialog includes a category selector with starter templates for each category. Scripts with unresolved bindings show a warning icon.
- **Schema versioning** — `schema()` now requires a `version: N` key (integer ≥ 1). All built-in schemas and templates ship at version 1. Registering a schema at a version lower than the currently registered version is a hard error at load time.
- **Data migration closures** — Schemas can declare a `migrate` map keyed by target version number. Each closure receives a note map (`title`, `fields`) and mutates it in place. Migration closures run automatically on workspace open for any notes whose `schema_version` is below the current schema version.
- **Batch migration on load** — After scripts load (Phase D), Krillnotes queries stale notes and runs migration closures in a single transaction per schema type. Multi-version jumps chain closures in order (e.g. a note at v1 against a v3 schema runs the v2 closure then the v3 closure). Any migration error rolls back the entire batch for that schema type; other types continue independently.
- **`schema_version` on notes** — Each note carries a `schema_version` integer stamped with the schema's current version at create time and updated after successful save.
- **`UpdateSchema` operation** — A new operation variant logged once per schema type after a successful batch migration, recording `schema_name`, `from_version`, `to_version`, and `notes_migrated`.
- **Migration toast notification** — After a batch migration, a transient toast appears: *"Contact schema updated — 12 notes migrated to version 3"*. Auto-dismisses after a few seconds.

### Changed
- **License: MIT → MPL-2.0** — Krillnotes is now published under the [Mozilla Public License 2.0](https://mozilla.org/MPL/2.0/). Existing integrations that relied on the MIT license should review the MPL-2.0 terms (file-level copyleft; compatible with GPL).
- **Workspace opening requires an unlocked identity** — `EnterPasswordDialog` and `SetPasswordDialog` are removed. Opening a workspace now requires unlocking the bound identity first. If no identity is unlocked, the Workspace Manager prompts you to unlock one before opening.
- **Note positions changed from integer to float** — `notes.position` in the database is now a `REAL` (f64) column. This enables future fractional mid-point insertion for CRDT reordering without rewriting sibling positions. Existing positions are migrated automatically.
- **Operations table schema updated** — The `timestamp` column is replaced by three HLC columns (`timestamp_wall_ms`, `timestamp_counter`, `timestamp_node_id`). A new `hlc_state` table persists the HLC clock state across sessions. Existing workspaces are migrated automatically on first open.
- **`HashMap` → `BTreeMap` for note fields** — `Note.fields`, `CreateNote.fields`, and related action types now use `BTreeMap` to guarantee deterministic serialization order. This is required for reproducible Ed25519 signatures across processes.
- **`on_save` hook API** — All `on_save` hooks (system scripts and templates) have been migrated from direct note mutation to the new `SaveTransaction` gated model. The `on_add_child` hook is also migrated, with both parent and child pre-seeded into the transaction.
- **`save_note` replaces `update_note` IPC** — The frontend now calls `save_note` instead of `update_note`, which runs the full save pipeline including validation and hooks. The old `update_note` command is removed.
- **`on_view`, `on_hover`, and `add_tree_action` removed** — These APIs no longer exist. All system scripts and templates have been migrated to the new split-file format (`.schema.rhai` + `.rhai`) using `register_view`, `register_hover`, and `register_menu`.
- **`category` column on `user_scripts`** — A `category TEXT NOT NULL DEFAULT 'presentation'` column is added to the `user_scripts` table. Existing user scripts default to `"presentation"`.
- **Version guard on schema registration** — Re-registering an existing schema with a lower version number raises a hard error at load time. Same version allows hooks and fields to be updated freely; higher version triggers Phase D migration.
- **`schema_version` column in `notes` table** — DDL updated to include `schema_version INTEGER NOT NULL DEFAULT 1`. Existing notes default to version 1.

### Fixed
- **Serde camelCase on `SaveResult::ValidationErrors`** — Added explicit `#[serde(rename)]` attributes for `fieldErrors`, `noteErrors`, `previewTitle`, and `previewFields` fields. Enum-level `rename_all` only renames variant tags, not struct variant fields.
- **`evaluate_group_visibility` and `validate_field` invoke parameters** — Fixed frontend invoke calls to pass `schemaName` instead of `noteId`, matching the Tauri command signatures.

---

## [0.2.6] — 2026-03-04

### Added
- **Undo / Redo** — Cmd+Z undoes the most recent note-tree action; Cmd+Shift+Z redoes it. Toolbar buttons are also available. Supported operations: note create, title and field edits, delete (with full subtree restored), move / reorder, and script create / update / delete. Tree hook side-effects (e.g. auto-entering a title immediately after creating a note) are collapsed into a single undo step so one Cmd+Z reverses the whole action. The history limit is configurable in Settings (default 50, max 500) and stored per workspace in `workspace_meta`.
- **Separate script editor undo** — The CodeMirror editor in the Script Manager maintains its own independent undo history. Cmd+Z inside the editor undoes text changes within the editor only and does not affect the note-tree undo stack.
- **Attachment Restore** — Deleting an attachment now shows a "Recently deleted" strip below the attachment list with a per-item Restore button. Deleted attachments can be recovered for the duration of the app session, including after navigating away from the note and returning.

### Changed
- **Operations log always active** — The operations log is now populated for every workspace, regardless of sync settings. Previously it was gated behind sync being enabled (v0.2.5 change); it must be unconditionally active because undo/redo is recorded as first-class `RetractOperation` entries in the same log.

---

## [0.2.5] — 2026-03-02

### Added
- **File attachments** — Any note can have files attached to it. Attachments are encrypted at rest alongside the workspace database using ChaCha20-Poly1305. A drag-and-drop attachment panel in the InfoPanel lets you attach, preview (images show a thumbnail), open, and delete files. Attachments are included in workspace export/import archives and re-encrypted on import. A configurable max attachment size guard is enforced at attach time.
- **`file` field type** — Schema fields can now be typed `file`, storing a reference to a single attached file. In view mode, images render as a thumbnail; other files show a paperclip icon and filename. In edit mode a file picker opens filtered by `allowed_types` MIME types. Replacing a file atomically attaches the new one before deleting the old.
- **`display_image(source, width, alt)` Rhai helper** — Embeds an attached image directly in `on_view` or `on_hover` hook output. `source` is either `"field:fieldName"` (reads the UUID from a `file` field) or `"attach:filename"` (finds by filename). Images are base64-encoded server-side so the frontend renders them without any asynchronous hydration step.
- **`display_download_link(source, label)` Rhai helper** — Renders a clickable download link for an attachment in `on_view` output. Clicking the link decrypts the file on demand and triggers a browser download.
- **`{{image: …}}` markdown syntax** — Textarea fields rendered as markdown now support inline image blocks: `{{image: field:cover, width: 400, alt: My caption}}` or `{{image: attach:photo.png}}`. Images are resolved and embedded server-side during rendering.
- **`get_attachments(note_id)` query function** — Returns attachment metadata for any note. Available in `on_view`, `on_hover`, and `add_tree_action` closures.
- **`stars(value)` / `stars(value, max)` display helpers** — Renders a numeric rating as filled (★) and empty (☆) star characters in `on_view` hook output. Defaults to 5 stars; pass a second argument to use a different scale. Returns `"—"` for a zero or negative value, matching the default field view.
- **Internationalisation (i18n)** — 7 language packs ship out of the box: English, German, French, Spanish, Japanese, Korean, and Simplified Chinese. The active language is chosen from a new dropdown in Settings and takes effect live without restarting the app. Dates and numbers are formatted using the locale's conventions (via `Intl.DateTimeFormat` / `Intl.NumberFormat`).
- **Native menu i18n** — The Tauri native application menu (File, Edit, Tools, View, Help) is also translated. All 20 menu-item labels are read from the same locale JSON files as the React frontend. Changing the language in Settings rebuilds and reapplies all open window menus immediately — no restart required. Locale data is embedded at compile time by `build.rs`, so there is zero runtime I/O overhead.
- **Hover tooltip on tree nodes** — Hovering a tree node for 600ms shows a compact speech-bubble tooltip to the right of the tree panel, without needing to navigate to the note. Two render paths are supported: mark any field with `show_on_hover: true` for an instant inline preview (no IPC), or define an `on_hover` hook in `schema()` to return fully custom HTML via the Rhai scripting engine. The tooltip is a React portal, position-clamped to the viewport, with a left-pointing spike that tracks the hovered row. It dismisses immediately on mouse-leave, click, or drag start.
- **`on_hover` hook** — A new optional hook inside `schema()` blocks. Like `on_view`, it receives a note map and has access to all query functions (`get_children`, `get_notes_for_tag`, etc.) and display helpers (`field`, `stack`, `markdown`, …). The return value is rendered as HTML in the tooltip.
- **`show_on_hover` field flag** — Fields defined with `show_on_hover: true` are surfaced in the hover tooltip without any scripting. Useful for quick previews of a single key field (e.g. a body or description).
- **Zettelkasten template updated** — The bundled `zettelkasten.rhai` now demos both hover paths: Zettel notes show the body field on hover; Kasten folders show a live child-count badge via `on_hover`.
- **Appearance tab in Settings** — Appearance settings (language, light/dark mode, and theme pickers) have been moved from the General tab into their own dedicated Appearance tab. The Settings dialog now has three tabs: General, Appearance, and Sync.
- **Sync tab in Settings** — A locked Sync placeholder tab has been added to the Settings dialog in preparation for the upcoming sync feature.

### Fixed
- **Editor scroll in dialogs** — The CodeMirror script editor inside the Manage Themes and Script Manager dialogs now scrolls correctly. The fix uses a definite `height` instead of `max-height` on the dialog container and adds `will-change: transform` to anchor macOS overlay scrollbars to the correct compositing layer.
- **Cmd+X and Cmd+A in text fields** — Cut and Select All keyboard shortcuts now work correctly on macOS. Previously these were no-ops because the native menu bar was missing `PredefinedMenuItem::cut` and `select_all` entries.
- **Sync settings not translated** — The General and Sync tab labels, and the Sync placeholder text, were displayed in English regardless of the selected language. All six non-English language packs (de, fr, es, ja, ko, zh) now include correct translations for these strings.

### Changed
- **Operations log gated behind sync** — The operations log is no longer populated unless sync is enabled. Since sync is not yet implemented, the log is always empty and the Operations Log menu item is permanently greyed out until sync ships.

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

[0.3.0]: https://github.com/careck/krillnotes/compare/v0.2.6...v0.3.0
[0.2.6]: https://github.com/careck/krillnotes/compare/v0.2.5...v0.2.6
[0.2.5]: https://github.com/careck/krillnotes/compare/v0.2.4...v0.2.5
[0.2.4]: https://github.com/careck/krillnotes/compare/v0.2.3...v0.2.4
[0.2.3]: https://github.com/careck/krillnotes/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/careck/krillnotes/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/careck/krillnotes/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/careck/krillnotes/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/careck/krillnotes/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/careck/krillnotes/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/careck/krillnotes/releases/tag/v0.1.0
