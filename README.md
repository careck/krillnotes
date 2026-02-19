# Krillnotes

A local-first, hierarchical note-taking application. Notes live in a tree, each note has a schema-defined type, and every change is recorded in an operation log — laying the groundwork for offline-first sync.

Built with Rust, Tauri v2, React, and SQLite.

---

## Features

- **Hierarchical notes** — Organize notes in an infinite tree. Each note can have children.
- **Typed note schemas** — Note types are defined as Rhai scripts. The built-in `TextNote` type ships with a `body` field; custom types are straightforward to add.
- **Operation log** — Every mutation (create, update, move, delete) is appended to an immutable log before being applied, enabling future undo/redo and device sync.
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

Each workspace is a single SQLite database with the `.krillnotes` extension. The file contains three tables:

| Table | Purpose |
|-------|---------|
| `notes` | The note tree (id, title, type, parent, position, fields) |
| `operations` | Append-only mutation log (CRDT-style) |
| `workspace_meta` | Per-device metadata (device ID, selection state) |

The file is a standard SQLite 3 database and can be opened with any SQLite browser for inspection or backup.

---

## License

MIT — see [LICENSE](LICENSE).
