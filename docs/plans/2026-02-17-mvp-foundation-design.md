# Krillnotes MVP Foundation - Design Document

> **SUPERSEDED:** This iced-based design was implemented but then superseded by Tauri v2.
> **See:** [2026-02-17-tauri-migration-design.md](./2026-02-17-tauri-migration-design.md) for current architecture.
> **Status:** ✅ Core backend complete, UI migrated to Tauri + React + TypeScript

---

**Date:** 2026-02-17
**Status:** Superseded (Core implemented, UI migrated to Tauri)
**Scope:** Minimum viable product - desktop shell, core data structure, local storage, Rhai scripting

---

## Overview

This MVP establishes the foundation for Krillnotes: a local-first personal information manager with user-defined schemas via Rhai scripting. The goal is to build the absolute minimum to prove the core architecture works.

### What's In Scope

- iced desktop application with menu bar (File, Edit, View, Help)
- Split view UI: tree sidebar + detail pane
- Core data model: hierarchical notes with title + schema-defined fields
- Local SQLite storage with operation log (for future sync)
- Rhai scripting system with embedded "TextNote" schema
- Auto-save on every edit
- Basic note operations: create, edit, delete, copy, paste

### What's Out of Scope (Future Phases)

- Multiple view types (only tree view for now)
- Drag-drop reordering
- Sync / cloud features
- User-defined scripts (only system scripts embedded)
- Undo/redo
- Advanced tree operations (move, bulk operations)
- Search/filtering

---

## 1. Overall Architecture & Project Structure

### Project Layout

```
krillnotes/
├── Cargo.toml
├── src/
│   ├── main.rs              # Desktop app entry, iced setup
│   ├── ui/                  # UI components
│   │   ├── mod.rs
│   │   ├── app.rs           # Main iced Application
│   │   ├── tree_view.rs     # Left sidebar tree
│   │   ├── detail_view.rs   # Right pane editor
│   │   └── menu.rs          # Menu bar
│   └── lib.rs               # Re-export core (optional, for future CLI)
├── core/                    # Core logic (no UI dependencies)
│   ├── mod.rs
│   ├── note.rs              # Note struct, core fields
│   ├── workspace.rs         # Workspace, tree operations
│   ├── operation.rs         # Operation types, log, purge
│   ├── storage.rs           # SQLite interface
│   ├── scripting.rs         # Rhai runtime, schema registry
│   ├── sync_provider.rs     # Trait (empty impl for MVP)
│   └── error.rs             # Error types
└── system_scripts/          # Embedded .rhai files
    └── text_note.rhai
```

### Key Principles

- **UI depends on core, not vice versa** - Core is testable independently
- **Rhai from day one** - Even system note types defined via scripts
- **Offline-first with operation log** - Auto-save + CRDT foundation for future sync
- **Hybrid structure** - Single crate now, easy to extract `krillnotes-core` later

### Dependencies

```toml
[dependencies]
iced = "0.12"           # Desktop GUI
rusqlite = "0.31"       # SQLite
rhai = "1.17"           # Scripting
serde = "1.0"           # JSON serialization
serde_json = "1.0"
uuid = "1.7"            # Note IDs
chrono = "0.4"          # Timestamps
thiserror = "1.0"       # Error handling
```

---

## 2. Data Model & Storage

### Core Note Structure

```rust
// core/note.rs
pub struct Note {
    // Core fields (always present, not schema-defined)
    pub id: String,                          // UUID
    pub title: String,                       // REQUIRED, never empty
    pub node_type: String,                   // e.g., "TextNote"
    pub parent_id: Option<String>,           // None = root level
    pub position: i32,                       // Order among siblings
    pub created_at: i64,                     // Unix timestamp
    pub modified_at: i64,                    // Unix timestamp
    pub created_by: i64,                     // 0 = local, user ID when synced
    pub modified_by: i64,                    // Last modifier

    // Schema-defined fields (from Rhai)
    pub fields: HashMap<String, FieldValue>,
}

pub enum FieldValue {
    Text(String),
    Number(f64),
    Boolean(bool),
    // More types later (Date, Reference, etc.)
}
```

**Design rationale:**
- **Title is core** - Every note must have a title, it's universal
- **User tracking from day one** - `created_by`/`modified_by` enable team features later
- **Generic fields** - Schema-defined fields stored as HashMap for flexibility

### SQLite Schema

```sql
-- Notes table
CREATE TABLE notes (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    node_type TEXT NOT NULL,
    parent_id TEXT,
    position INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    modified_at INTEGER NOT NULL,
    created_by INTEGER NOT NULL DEFAULT 0,
    modified_by INTEGER NOT NULL DEFAULT 0,
    fields_json TEXT NOT NULL DEFAULT '{}',

    FOREIGN KEY (parent_id) REFERENCES notes(id) ON DELETE CASCADE
);

CREATE INDEX idx_notes_parent ON notes(parent_id, position);

-- Operations log (for future sync + undo)
CREATE TABLE operations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    operation_id TEXT UNIQUE NOT NULL,       -- UUID for CRDT
    timestamp INTEGER NOT NULL,
    device_id TEXT NOT NULL,
    operation_type TEXT NOT NULL,            -- "CreateNote", "UpdateField", etc.
    operation_data TEXT NOT NULL,            -- JSON of operation details
    synced INTEGER DEFAULT 0
);

CREATE INDEX idx_operations_timestamp ON operations(timestamp);
CREATE INDEX idx_operations_synced ON operations(synced);

-- Workspace metadata
CREATE TABLE workspace_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
-- Stores: device_id, current_user_id, purge_strategy, etc.
```

### Operation Log & Purge Strategy

**Purpose:**
- Log all mutations as operations (CRDT foundation for future sync)
- Enable undo/redo later
- Keep database size bounded

**Purge strategies:**

```rust
pub enum PurgeStrategy {
    // MVP: No sync provider, keep last N operations
    LocalOnly { keep_last: usize },  // e.g., 1000

    // Future: With sync provider, keep unsynced + recent synced
    WithSync { retention_days: u32 },  // e.g., 30 days
}
```

**MVP behavior:**
- Use `LocalOnly { keep_last: 1000 }`
- Auto-purge after every write (cheap: DELETE WHERE id NOT IN top N)
- Operations available as undo buffer

**Future with sync:**
- Switch to `WithSync { retention_days: 30 }`
- Keep all unsynced operations
- Purge old synced operations only

---

## 3. Rhai Scripting System

### System Script (Embedded)

```rhai
// system_scripts/text_note.rhai
schema("TextNote", #{
    fields: [
        #{ name: "body", type: "text", required: false },
    ],

    // Future hooks (not implemented in MVP)
    // on_create: |note| { ... },
    // on_field_change: |note, field, old_val, new_val| { ... },
});
```

### Schema Registry (Rust)

```rust
// core/scripting.rs
pub struct SchemaRegistry {
    engine: rhai::Engine,
    schemas: HashMap<String, Schema>,
}

pub struct Schema {
    pub name: String,                    // e.g., "TextNote"
    pub fields: Vec<FieldDefinition>,
}

pub struct FieldDefinition {
    pub name: String,                    // e.g., "body"
    pub field_type: String,              // "text", "number", "boolean"
    pub required: bool,
}

impl SchemaRegistry {
    pub fn new() -> Result<Self> {
        let mut engine = rhai::Engine::new();
        let schemas = HashMap::new();

        let mut registry = Self { engine, schemas };

        // Register Rust functions callable from Rhai
        registry.register_api();

        // Load system scripts at startup
        registry.load_script(include_str!("../system_scripts/text_note.rhai"))?;

        Ok(registry)
    }

    fn register_api(&mut self) {
        // Make schema() function available to Rhai
        self.engine.register_fn("schema",
            move |name: String, def: rhai::Map| {
                let schema = Schema::from_rhai(&name, &def);
                // Store in registry
            }
        );
    }

    pub fn load_script(&mut self, script_content: &str) -> Result<()> {
        // Execute the Rhai script (calls schema() functions)
        self.engine.eval::<()>(script_content)?;
        Ok(())
    }

    pub fn get_schema(&self, name: &str) -> Result<&Schema> {
        self.schemas.get(name).ok_or(KrillnotesError::SchemaNotFound(name.into()))
    }

    pub fn list_schemas(&self) -> Vec<String> {
        self.schemas.keys().cloned().collect()
    }
}
```

### MVP Scope

- Only `schema()` function implemented
- Only "text", "number", "boolean" field types
- No hooks, no validation beyond required fields
- System scripts only (user scripts in future phase)

---

## 4. UI Architecture (iced)

### Main Application

```rust
// src/ui/app.rs
pub struct KrillnotesApp {
    workspace: Option<Workspace>,
    selected_note_id: Option<String>,

    // UI state
    tree_state: TreeState,
    editing_title: String,
    editing_fields: HashMap<String, String>,  // field_name -> input value

    // Dialogs
    show_file_picker: bool,
    show_add_note_dialog: bool,
    add_note_position: AddPosition,
    add_note_type: String,
}

#[derive(Debug, Clone)]
pub enum Message {
    // File menu
    FileNew,
    FileOpen,

    // Edit menu
    EditAddNote,
    EditAddNoteConfirm { position: AddPosition, note_type: String },
    EditDeleteNote,
    EditCopyNote,
    EditPasteNote,

    // Tree interaction
    NoteSelected(String),  // note_id

    // Note editing
    TitleChanged(String),
    FieldChanged { field_name: String, value: String },

    // Dialog state
    AddNoteDialogPositionChanged(AddPosition),
    AddNoteDialogTypeChanged(String),
    DialogClose,

    // Internal
    WorkspaceLoaded(Result<Workspace>),
    ShowError(KrillnotesError),
}

pub enum AddPosition {
    AsChild,
    AsSibling,
}
```

### Layout Structure

```rust
impl Application for KrillnotesApp {
    fn view(&self) -> Element<Message> {
        let content = row![
            // Left: Tree view (30% width)
            container(self.render_tree_view())
                .width(Length::FillPortion(3)),

            // Right: Detail view (70% width)
            container(self.render_detail_view())
                .width(Length::FillPortion(7)),
        ];

        column![
            self.render_menu_bar(),
            content,
        ].into()
    }
}
```

### Menu Bar

```rust
fn render_menu_bar(&self) -> Element<Message> {
    menu_bar![
        menu("File", vec![
            item("New", Message::FileNew),
            item("Open...", Message::FileOpen),
            separator(),
            // item("Open Recent", submenu),  // Future
        ]),
        menu("Edit", vec![
            item("Add Note", Message::EditAddNote),
            item("Delete Note", Message::EditDeleteNote),
            separator(),
            item("Copy", Message::EditCopyNote),
            item("Paste", Message::EditPasteNote),
        ]),
        menu("View", vec![
            // Future: different view types
        ]),
        menu("Help", vec![
            item("About", Message::HelpAbout),
        ]),
    ]
}
```

### Tree View (Left Pane)

- Hierarchical list with indentation
- Click to select note
- Highlight selected note
- Show note titles only (not fields)
- Future: expand/collapse, drag-drop

### Detail View (Right Pane)

```rust
fn render_detail_view(&self) -> Element<Message> {
    if let Some(note) = self.get_selected_note() {
        let schema = self.workspace.registry.get_schema(&note.node_type)?;

        let mut fields = vec![
            // Title (always visible)
            text_input("Title", &self.editing_title)
                .on_input(Message::TitleChanged)
                .into(),
        ];

        // Schema-defined fields
        for field_def in &schema.fields {
            let input = match field_def.field_type.as_str() {
                "text" => text_input(
                    &field_def.name,
                    &self.editing_fields[&field_def.name]
                )
                .on_input({
                    let name = field_def.name.clone();
                    move |value| Message::FieldChanged {
                        field_name: name.clone(),
                        value,
                    }
                }),
                // Add number, boolean later
                _ => text_input("", "Unsupported type"),
            };
            fields.push(input.into());
        }

        column(fields).into()
    } else {
        text("No note selected").into()
    }
}
```

### Add Note Dialog

```rust
fn render_add_note_dialog(&self) -> Element<Message> {
    let selected_note = self.get_selected_note().unwrap();

    column![
        text("Add Note").size(20),

        // Position selection
        radio("As child of selected", AddPosition::AsChild,
            Some(self.add_note_position), Message::AddNoteDialogPositionChanged),
        radio("As sibling after selected", AddPosition::AsSibling,
            Some(self.add_note_position), Message::AddNoteDialogPositionChanged),

        // Note type selection (default to selected note's type)
        text("Note type:"),
        pick_list(
            self.workspace.registry.list_schemas(),
            Some(self.add_note_type.clone()),
            Message::AddNoteDialogTypeChanged,
        ),

        row![
            button("Create").on_press(Message::EditAddNoteConfirm {
                position: self.add_note_position,
                note_type: self.add_note_type.clone(),
            }),
            button("Cancel").on_press(Message::DialogClose),
        ],
    ]
}
```

### Auto-Save Behavior

- Every `TitleChanged` / `FieldChanged` immediately writes to database
- No "Save" button, no dirty tracking
- Update triggers: note write + operation log + purge (in transaction)

---

## 5. Core Operations & Data Flow

### File > New Workspace

```rust
pub fn create_workspace(path: PathBuf) -> Result<Workspace> {
    let db = Connection::open(&path)?;

    // Initialize schema
    db.execute_batch(include_str!("schema.sql"))?;

    // Store metadata
    let device_id = get_or_create_device_id();
    db.execute("INSERT INTO workspace_meta VALUES ('device_id', ?)", [device_id])?;
    db.execute("INSERT INTO workspace_meta VALUES ('current_user_id', '0')", [])?;

    // Create root note from filename
    let filename = path.file_stem().unwrap().to_str().unwrap();
    let title = humanize(filename);  // "my-project" → "My Project"

    let root = Note {
        id: Uuid::new_v4().to_string(),
        title,
        node_type: "TextNote".into(),
        parent_id: None,
        position: 0,
        created_at: now(),
        modified_at: now(),
        created_by: 0,
        modified_by: 0,
        fields: hashmap!{"body".into() => FieldValue::Text(String::new())},
    };

    db.execute(
        "INSERT INTO notes VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![root.id, root.title, root.node_type, root.parent_id,
                root.position, root.created_at, root.modified_at,
                root.created_by, root.modified_by,
                serde_json::to_string(&root.fields)?]
    )?;

    Ok(Workspace::open(path)?)
}

fn humanize(filename: &str) -> String {
    filename
        .replace('-', " ")
        .replace('_', " ")
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
```

### Edit > Add Note

```rust
pub fn create_note(
    &mut self,
    parent_id: Option<String>,
    position: AddPosition,
    note_type: String,
) -> Result<String> {
    let schema = self.registry.get_schema(&note_type)?;
    let selected = self.get_selected_note()?;

    // Determine final parent and position
    let (final_parent, final_position) = match position {
        AddPosition::AsChild => {
            // Add as first child of selected
            (Some(selected.id.clone()), 0)
        }
        AddPosition::AsSibling => {
            // Add after selected at same level
            (selected.parent_id.clone(), selected.position + 1)
        }
    };

    // Create note with default fields from schema
    let note = Note {
        id: Uuid::new_v4().to_string(),
        title: "Untitled".into(),
        node_type: note_type,
        parent_id: final_parent,
        position: final_position,
        created_at: now(),
        modified_at: now(),
        created_by: self.current_user_id,
        modified_by: self.current_user_id,
        fields: schema.default_fields(),
    };

    // Transaction: insert note + log operation + purge
    let tx = self.db.transaction()?;

    tx.execute(
        "INSERT INTO notes VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![/* note fields */]
    )?;

    let op = Operation::CreateNote { /* ... */ };
    self.operation_log.log(&tx, op)?;
    self.operation_log.purge_if_needed(&tx)?;

    tx.commit()?;

    Ok(note.id)
}
```

### Note Editing (Auto-Save)

```rust
pub fn update_note_title(&mut self, note_id: &str, new_title: String) -> Result<()> {
    let tx = self.db.transaction()?;

    // Update note
    tx.execute(
        "UPDATE notes SET title = ?, modified_at = ?, modified_by = ? WHERE id = ?",
        params![new_title, now(), self.current_user_id, note_id],
    )?;

    // Log operation
    let op = Operation::UpdateField {
        operation_id: Uuid::new_v4().to_string(),
        timestamp: now(),
        device_id: self.device_id.clone(),
        note_id: note_id.into(),
        field: "title".into(),
        value: FieldValue::Text(new_title),
    };
    self.operation_log.log(&tx, op)?;
    self.operation_log.purge_if_needed(&tx)?;

    tx.commit()?;
    Ok(())
}

// Similar for update_note_field()
```

### Operation Types

```rust
// core/operation.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Operation {
    CreateNote {
        operation_id: String,
        timestamp: i64,
        device_id: String,
        note_id: String,
        parent_id: Option<String>,
        position: i32,
        node_type: String,
        title: String,
        fields: HashMap<String, FieldValue>,
        created_by: i64,
    },
    UpdateField {
        operation_id: String,
        timestamp: i64,
        device_id: String,
        note_id: String,
        field: String,  // "title" or schema field name
        value: FieldValue,
        modified_by: i64,
    },
    DeleteNote {
        operation_id: String,
        timestamp: i64,
        device_id: String,
        note_id: String,
    },
    MoveNote {
        operation_id: String,
        timestamp: i64,
        device_id: String,
        note_id: String,
        new_parent_id: Option<String>,
        new_position: i32,
    },
}
```

---

## 6. Error Handling & Testing

### Error Types

```rust
// core/error.rs
#[derive(Debug, thiserror::Error)]
pub enum KrillnotesError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("Scripting error: {0}")]
    Scripting(String),

    #[error("Schema not found: {0}")]
    SchemaNotFound(String),

    #[error("Note not found: {0}")]
    NoteNotFound(String),

    #[error("Invalid workspace: {0}")]
    InvalidWorkspace(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, KrillnotesError>;

impl KrillnotesError {
    pub fn user_message(&self) -> String {
        match self {
            Self::Database(e) => format!("Failed to save: {}", e),
            Self::SchemaNotFound(name) => format!("Unknown note type: {}", name),
            Self::NoteNotFound(_) => "Note no longer exists".into(),
            Self::InvalidWorkspace(_) => "Could not open workspace file".into(),
            Self::Scripting(e) => format!("Script error: {}", e),
            Self::Io(e) => format!("File error: {}", e),
        }
    }
}
```

### Testing Strategy

**Unit tests (core logic):**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_workspace() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let ws = Workspace::create(temp.path().to_path_buf()).unwrap();
        let notes = ws.list_all_notes().unwrap();
        assert_eq!(notes.len(), 1);  // Root note only
        assert_eq!(notes[0].title, "Temp");  // Humanized filename
    }

    #[test]
    fn test_add_child_note() {
        let ws = create_test_workspace();
        let root = ws.get_root_note().unwrap();
        let child = ws.create_note(
            Some(root.id.clone()),
            AddPosition::AsChild,
            "TextNote".into()
        ).unwrap();

        let child_note = ws.get_note(&child).unwrap();
        assert_eq!(child_note.parent_id, Some(root.id));
        assert_eq!(child_note.position, 0);
    }

    #[test]
    fn test_operation_log_purge() {
        let mut ws = create_test_workspace();

        // Create 1100 notes
        for i in 0..1100 {
            ws.create_note(None, AddPosition::AsChild, "TextNote".into()).unwrap();
        }

        // Verify only last 1000 operations remain
        let count: i64 = ws.db.query_row(
            "SELECT COUNT(*) FROM operations",
            [],
            |row| row.get(0)
        ).unwrap();

        assert_eq!(count, 1000);
    }

    #[test]
    fn test_schema_registration() {
        let registry = SchemaRegistry::new().unwrap();
        let schema = registry.get_schema("TextNote").unwrap();
        assert_eq!(schema.name, "TextNote");
        assert_eq!(schema.fields.len(), 1);
        assert_eq!(schema.fields[0].name, "body");
    }

    #[test]
    fn test_humanize_filename() {
        assert_eq!(humanize("my-project"), "My Project");
        assert_eq!(humanize("hello_world"), "Hello World");
        assert_eq!(humanize("test-case-123"), "Test Case 123");
    }
}
```

**Integration tests:**

```rust
// tests/integration_test.rs
#[test]
fn test_full_workflow() {
    // Create workspace
    let temp = tempfile::NamedTempFile::new().unwrap();
    let mut ws = Workspace::create(temp.path().to_path_buf()).unwrap();

    // Add multiple notes
    let root = ws.get_root_note().unwrap();
    let child1 = ws.create_note(Some(root.id.clone()), AddPosition::AsChild, "TextNote".into()).unwrap();
    let child2 = ws.create_note(Some(root.id.clone()), AddPosition::AsChild, "TextNote".into()).unwrap();

    // Update fields
    ws.update_note_title(&child1, "First Note".into()).unwrap();
    ws.update_note_field(&child1, "body", FieldValue::Text("Content here".into())).unwrap();

    // Verify persistence
    let note = ws.get_note(&child1).unwrap();
    assert_eq!(note.title, "First Note");

    // Close and reopen
    drop(ws);
    let ws2 = Workspace::open(temp.path().to_path_buf()).unwrap();

    // Verify data intact
    let note2 = ws2.get_note(&child1).unwrap();
    assert_eq!(note2.title, "First Note");
}
```

**MVP testing scope:**
- Unit tests for all core operations
- Schema registry and Rhai integration
- Operation log and purge logic
- UI: manual testing only (iced testing is complex)

### Data Safety

**Built-in safeguards:**
- All mutations in SQLite transactions (atomic)
- Foreign key constraints (parent_id references notes)
- Auto-save eliminates "forgot to save" errors
- Operation log provides audit trail

**Future enhancements:**
- Periodic backups to `~/.krillnotes/backups/`
- Export to JSON/Markdown
- Corruption detection and repair

---

## 7. Sync Provider Hook (Future-Ready)

**MVP:** Trait defined but not implemented.

```rust
// core/sync_provider.rs
pub trait SyncProvider: Send + Sync {
    fn authenticate(&mut self, credentials: Credentials) -> Result<Account>;
    fn push_operations(&mut self, ops: Vec<Operation>) -> Result<PushResult>;
    fn fetch_operations(&mut self, since: SyncCursor) -> Result<Vec<Operation>>;
    fn create_team_workspace(&mut self, name: &str) -> Result<WorkspaceId>;
    fn list_workspaces(&self) -> Result<Vec<WorkspaceInfo>>;
}

pub struct Workspace {
    db: Connection,
    operation_log: OperationLog,
    sync_provider: Option<Box<dyn SyncProvider>>,  // Always None in MVP
    // ...
}
```

**Later:** Premium plugin implements `SyncProvider`, registers at runtime.

---

## Summary

This MVP design establishes:

✅ **Solid foundation** - Clean separation of core logic and UI
✅ **Rhai from day one** - Even system types are script-defined
✅ **Operation log** - Ready for sync and undo later
✅ **Auto-save** - No data loss, no manual save needed
✅ **Future-proof** - SyncProvider hook, user tracking, purge strategy
✅ **Testable** - Core logic has no UI dependencies

**Next step:** Create implementation plan from this design.
