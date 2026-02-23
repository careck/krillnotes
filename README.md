# Krillnotes

A local-first, hierarchical note-taking application. Notes live in a tree, each note has a schema-defined type, and every change is recorded in an operation log — laying the groundwork for offline-first sync.

Built with Rust, Tauri v2, React, and SQLite.

---

## Features

- **Hierarchical notes** — Organize notes in an infinite tree. Each note can have children, with configurable sort order (alphabetical ascending/descending, or manual positioning).
- **Typed note schemas** — Note types are defined as [Rhai](https://rhai.rs/) scripts. The built-in `TextNote` type ships out of the box; custom types support fields of type `text`, `textarea`, `number`, `boolean`, `date`, `email`, `select`, and `rating`.
- **User scripts** — Each workspace stores its own Rhai scripts in the database. Create, edit, enable/disable, reorder, and delete scripts from a built-in script manager — no file system access required. Six example scripts ship in the `user_scripts/` folder (Task, Book, Contact, Product, Recipe, Project).
- **On-save hooks** — Rhai scripts can register `on_save` hooks that compute derived fields (e.g. auto-generating a note title from first name + last name, calculating a read duration, or setting a status badge).
- **Search** — A live search bar with debounced fuzzy matching across note titles and all text fields. Keyboard-navigable results; selecting a match expands collapsed ancestors and scrolls the note into view.
- **Export / Import** — Export an entire workspace as a `.zip` archive (notes + user scripts). Import a zip into a new workspace, with version-compatibility checks before importing.
- **Operations log viewer** — Browse the full mutation history, filter by operation type or date range, and purge old entries to reclaim space.
- **Operation log** — Every mutation (create, update, move, delete, script changes) is appended to an immutable log before being applied, enabling future undo/redo and device sync.
- **Tree keyboard navigation** — Arrow keys to move between nodes, Right/Left to expand/collapse, Enter to edit the selected note.
- **Resizable panels** — Drag the divider between the tree and the detail panel to resize.
- **Context menu** — Right-click on any tree node for quick actions (Add Note, Edit, Delete).
- **Multi-window** — Open multiple workspaces simultaneously, each in its own window.
- **Local-first** — All data is stored in a single `.krillnotes` file on disk. No account, no cloud dependency, no internet connection required.
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

Each workspace is a single SQLite database with the `.krillnotes` extension. The file contains four tables:

| Table | Purpose |
|-------|---------|
| `notes` | The note tree (id, title, type, parent, position, fields) |
| `operations` | Append-only mutation log (CRDT-style) |
| `workspace_meta` | Per-device metadata (device ID, selection state) |
| `user_scripts` | Per-workspace Rhai scripts (id, name, source code, load order, enabled flag) |

The file is a standard SQLite 3 database and can be opened with any SQLite browser for inspection or backup.

---

## macOS: "App is damaged" warning

macOS Gatekeeper blocks unsigned apps with an "app is damaged and can't be opened" message. To bypass this after installing from the `.dmg`:

```bash
xattr -cr /Applications/krillnotes-desktop.app
```

This removes the quarantine flag macOS adds when mounting a DMG. The app will open normally afterwards.

---

## License

MIT — see [LICENSE](LICENSE).
