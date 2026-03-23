![Krillnotes](marketing/KrillNotesBanner.jpeg)

# Krillnotes

A local-first, hierarchical note-taking application with multi-device sync. Notes live in a tree, each note has a schema-defined type, and every change is recorded in an operation log that syncs between peers via relay server, shared folder, or manual file exchange.

Built with Rust, Tauri v2, React, and SQLCipher (encrypted SQLite).

---

## Features

- **Hierarchical notes** — Organize notes in an infinite tree. Each note can have children, with configurable sort order (alphabetical ascending/descending, or manual positioning).
- **Typed note schemas** — Note types are defined as [Rhai](https://rhai.rs/) scripts. The built-in `TextNote` type ships out of the box; custom types support fields of type `text`, `textarea`, `number`, `boolean`, `date`, `email`, `select`, `rating`, `file`, and `note_link`.
- **User scripts** — Each workspace stores its own Rhai scripts in the database. Scripts come in two categories: **Schema** (`.schema.rhai`) define note types via `schema()`, and **Library/Presentation** (`.rhai`) define views, hover tooltips, and context-menu actions via `register_view()`, `register_hover()`, and `register_menu()`. Create, edit, enable/disable, reorder, and delete scripts from a built-in script manager — no file system access required.
- **Template gallery** — Ready-to-use templates live in the `templates/` folder: a book collection organiser and a Zettelkasten atomic-note system. Copy the Rhai source into the Script Manager to activate a template in any workspace.
- **Tags** — Attach free-form tags to any note. Tags are displayed as colour-coded pills in the note view, shown in the tree's tag cloud panel, and matched by the search bar. Scripts can read `note.tags` in view closures and query all notes carrying a given tag with `get_notes_for_tag()`.
- **On-save hooks** — Rhai `on_save` hooks use a transactional API (`set_field`, `set_title`, `reject`, `commit`) to compute derived fields safely. Field-level `validate` closures run on every keystroke. Field groups with optional `visible` closures keep complex schemas organised.
- **Tabbed note views** — Schemas with `register_view()` registrations show a tab bar in the detail panel. Custom view tabs appear in registration order; `display_first: true` moves a tab to the leftmost position. The Fields tab is always present. Types with no registered views show the plain field grid.
- **Schema versioning and migrations** — Schemas declare a `version` number and optional `migrate` closures. When the workspace opens, stale notes (those at an older `schema_version`) are migrated automatically in a single transaction per schema type. A toast notification reports how many notes were updated.
- **Search** — A live search bar with debounced fuzzy matching across note titles and all text fields. Keyboard-navigable results; selecting a match expands collapsed ancestors and scrolls the note into view.
- **Export / Import** — Export an entire workspace as a `.krillnotes` archive (notes + attachments + user scripts), with an optional AES-256 password. Import an archive into a new workspace; the app detects encrypted archives and prompts for the password before importing.
- **File attachments** — Attach any file to a note. Attachments are encrypted at rest alongside the database. Images render as thumbnails; all file types can be downloaded or opened. Attachment size limit is configurable per workspace.
- **Undo / Redo** — Cmd+Z / Cmd+Shift+Z (toolbar buttons also available). Undoes note creates, edits, deletes, and moves. Multi-step tree actions collapse into a single undo step. History limit is configurable per workspace (default 50, max 500). The script editor has its own independent undo stack that does not mix with the note-tree history.
- **Multi-device sync** — Sync workspaces between devices using three channels: **Relay** (HTTP relay server with mailbox routing), **Folder** (shared local/network directory), or **Manual** (export/import `.swarm` delta files). Each peer can use a different channel, switchable at any time. Delta bundles carry only new operations since the last sync; watermarks self-heal via delivery confirmation and ACK-based correction. All data in transit is end-to-end encrypted (X25519 + AES-256-GCM).
- **Peer management** — Invite peers via signed `.swarm` invite files or one-click relay links, exchange workspace snapshots for initial sync, and manage peers from the Workspace Peers dialog (trust badges, sync status, channel config, force resync). Background polling automatically picks up incoming invites and snapshots.
- **Subtree permissions (RBAC)** — Workspace owners can grant peers granular access to subtrees with five roles: owner, admin, editor, reader, and none. Permissions cascade from parent to child — the nearest explicit grant wins. The tree shows colour-coded role dots, share anchor icons, and ghost ancestor styling. A Share dialog lets you grant access; a Cascade preview shows the impact before demotion or revocation. All UI actions are role-aware — edit, delete, move, and create controls are disabled when the user lacks permission.
- **Contact book** — An encrypted per-identity address book stores peer contacts with trust levels (TOFU, verified-in-person), local names, and notes. Contacts are AES-256-GCM encrypted at rest under an HKDF-derived key that only exists in memory while the identity is unlocked.
- **Operations log viewer** — Browse the full mutation history, filter by operation type or date range, and purge old entries to reclaim space.
- **Operation log** — Every mutation (create, update, move, delete, script changes, undo/redo) is appended to an immutable log before being applied, forming the basis for CRDT-style sync between peers.
- **Identity system** — A cryptographic identity (Ed25519 keypair, passphrase-protected via Argon2id) manages workspace access. Unlock your identity once per session with your passphrase; all bound workspaces open without additional prompts. Identities are portable via `.swarmid` export/import — move your identity to another device and all your workspaces follow.
- **Workspace Manager** — Browse, open, duplicate, and delete workspaces from a dedicated manager. Each entry shows name, size, last-modified date, note count, and attachment count — all without needing to unlock the workspace.
- **Internationalisation** — 7 language packs ship out of the box: English, German, French, Spanish, Japanese, Korean, and Simplified Chinese. The active language is chosen from Settings and takes effect immediately, including the native application menu.
- **Tree keyboard navigation** — Arrow keys to move between nodes, Right/Left to expand/collapse, Enter to edit the selected note.
- **Resizable panels** — Drag the divider between the tree and the detail panel to resize.
- **Context menu** — Right-click on any tree node for quick actions (Add Note, Edit, Delete).
- **Multi-window** — Open multiple workspaces simultaneously, each in its own window.
- **Encrypted workspaces** — Every workspace is encrypted at rest with SQLCipher (AES-256-CBC, PBKDF2-HMAC-SHA512 key derivation). Workspace passwords are randomly generated and managed by the identity system — you never type a workspace password directly.
- **Local-first** — All data is stored on disk. No account, no cloud dependency, no internet connection required. Sync is opt-in and works offline — changes are queued and exchanged when peers reconnect.
- **Cross-platform** — Runs on macOS, Linux, and Windows via Tauri.

---

## Requirements

| Tool | Version |
|------|---------|
| Rust | 1.78+ |
| Node.js | 20+ |
| Tauri CLI | v2 |

Install the Tauri prerequisites for your platform by following the [Tauri v2 setup guide](https://v2.tauri.app/start/prerequisites/).

---

## Build & Run

```bash
# Clone the repository
git clone <repo-url>
cd Krillnotes

# Install Node dependencies
cd krillnotes-desktop
npm install

# Run in development mode (hot-reload frontend + Rust backend)
npm run tauri dev

# Build a release binary
npm run tauri build
```

The compiled application is placed in `krillnotes-desktop/src-tauri/target/release/bundle/`.

---

## Running Tests

```bash
# Core library unit tests
cargo test -p krillnotes-core
```

---

## File Format

Each workspace is a **folder** on disk containing:

- `notes.db` — a SQLCipher-encrypted SQLite database
- `attachments/` — per-attachment encrypted files (ChaCha20-Poly1305)
- `info.json` — unencrypted metadata sidecar (name, note count, size, workspace UUID) readable without a password

The database contains eight tables:

| Table | Purpose |
|-------|---------|
| `notes` | The note tree (id, title, type, parent, position, fields, `schema_version`) |
| `note_tags` | Many-to-many junction between notes and tags |
| `operations` | Append-only mutation log (CRDT-style, HLC-timestamped, Ed25519-signed) |
| `workspace_meta` | Per-device metadata (device ID, selection state, undo limit, `owner_pubkey`) |
| `user_scripts` | Per-workspace Rhai scripts (id, name, source code, load order, enabled flag, `category`) |
| `attachments` | Attachment metadata (id, note_id, filename, MIME type, size, hash) |
| `peer_registry` | Known sync peers and their state (device ID, identity ID, channel type, watermarks, sync status) |
| `note_permissions` | RBAC permission grants (peer public key, note scope, role, granted/revoked by) |

The database uses AES-256-CBC encryption (SQLCipher v4 defaults: PBKDF2-HMAC-SHA512, 256,000 iterations). Workspace passwords are randomly generated and stored encrypted under your identity key — you need SQLCipher-aware tooling **and** the correct randomly-generated password to open the file outside of Krillnotes.

> **Migrating from v0.2.x:** Workspaces created with v0.2.x used a user-supplied password. Export them in v0.2.x via **File → Export Workspace**, then import the `.krillnotes` archive into v0.3.0 using **New Workspace from Archive**.

> **Migrating from v0.1.x:** Unencrypted workspaces are rejected with a migration message. Open them in v0.1.x, export via **File → Export Workspace**, then import here.

---

## macOS: "App is damaged" warning

macOS Gatekeeper blocks unsigned apps with an "app is damaged and can't be opened" message. To bypass this after installing from the `.dmg`:

```bash
xattr -cr /Applications/Krillnotes.app
```

This removes the quarantine flag macOS adds when mounting a DMG. The app will open normally afterwards.

---

## License

MPL-2.0 — see [LICENSE](LICENSE).
