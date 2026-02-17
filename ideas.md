# Local-First Personal Information Manager - Design Document

## Vision

A powerful, local-first personal information manager and note-taking application that combines:
- **User-defined structured data** (like Notion's databases)
- **Hierarchical outliner** (like WorkFlowy/Logseq)
- **Local-first architecture** (like Obsidian - data lives on device)
- **Extensible via scripting** (like jEdit's BeanShell macros)
- **Native performance** (Rust + iced GUI)

**Target users:** Power users who want flexibility, control, and scriptable automation without giving up their data to the cloud.

## Core Architecture

### Technology Stack

**Backend (Rust):**
- Core data models and business logic
- SQLite for local storage (one .db file per workspace)
- CRDT-based sync engine for conflict-free replication
- Rhai scripting runtime for extensibility
- Schema registry (populated by scripts)

**Frontend (Rust/iced):**
- Native desktop GUI using iced framework
- Cross-platform: Windows, macOS, Linux
- Built-in view types: Tree, List, Table, Cards, Kanban, Calendar, Timeline, Graph

**Future extensibility:**
- WASM plugins for performance-critical extensions
- Mobile: Extract Rust core, wrap with Flutter (later phase)

### Layer Architecture
```
┌─────────────────────────────────────────────────────┐
│ Rhai Script Layer (User-defined semantics)         │
│ • Schema definitions (note types, fields)           │
│ • Commands (user actions, automations)              │
│ • View configurations (which view, how to display)  │
│ • Event hooks (on_create, on_change, etc.)          │
│ • Queries (saved searches, filters)                 │
└─────────────────────────────────────────────────────┘
                         ▲
                         │ Script API
                         ▼
┌─────────────────────────────────────────────────────┐
│ Rust Core (Schema-agnostic engine)                  │
│ • Note CRUD (create, read, update, delete)          │
│ • Tree operations (add_child, move, reorder)        │
│ • Generic storage (key-value fields per note)       │
│ • Query engine (filter, sort, aggregate)            │
│ • Sync engine (CRDT-based conflict resolution)      │
│ • Rhai runtime integration                          │
└─────────────────────────────────────────────────────┘
                         ▲
                         │ Persistence
                         ▼
┌─────────────────────────────────────────────────────┐
│ Storage Layer                                        │
│ • SQLite per workspace                               │
│ • Local file system                                  │
│ • OS keychain for auth tokens                        │
└─────────────────────────────────────────────────────┘
```

## Data Model

### Core Rust Types (Schema-agnostic)
```rust
// A note is a generic container with user-defined fields
struct Note {
    id: String,                              // UUID
    note_type: String,                       // User-defined type name
    parent_id: Option<String>,               // For tree hierarchy
    position: i32,                           // Order among siblings
    fields: HashMap<String, FieldValue>,     // Generic field storage
    children: Vec<String>,                   // Child note IDs
    created_at: DateTime,
    updated_at: DateTime,
}

// Generic field values
enum FieldValue {
    Text(String),
    Number(f64),
    Date(NaiveDate),
    DateTime(DateTime),
    Boolean(bool),
    Select(String),
    MultiSelect(Vec<String>),
    Reference(String),                       // ID of another note
    File(PathBuf),
}

// Tree operations
enum TreeOperation {
    InsertNode { node_id: String, parent_id: Option<String>, position: i32 },
    MoveNode { node_id: String, new_parent_id: Option<String>, new_position: i32 },
    DeleteNode { node_id: String },
    UpdateField { node_id: String, field: String, value: FieldValue },
}

// Query structure
struct Query {
    note_type: Option<String>,
    parent_id: Option<String>,               // Filter by parent (subtree)
    filters: Vec<FilterCriteria>,
    sort_by: Vec<SortCriteria>,
    limit: Option<usize>,
}
```

### Schema Definitions (Rhai Scripts)

Schemas are **defined in Rhai scripts**, not hardcoded. This enables:
- Sharing schemas via plugin marketplace
- Versioning schemas with git
- Composing/extending schemas
- Custom behaviors via hooks

**Example schema definition:**
```rhai
// schemas/task.rhai
schema("Task", {
    fields: [
        { name: "title", type: "text", required: true },
        { name: "status", type: "select", options: ["Todo", "InProgress", "Done"] },
        { name: "priority", type: "select", options: ["Low", "Medium", "High"] },
        { name: "due_date", type: "date" },
        { name: "assigned_to", type: "reference", target: "Person" },
        { name: "tags", type: "multi_select", options: [] },
        { name: "notes", type: "text" },
    ],
    
    // Lifecycle hooks
    on_create: |note| {
        note.set_field("status", "Todo");
        note.set_field("created_date", today());
    },
    
    on_field_change: |note, field_name, old_value, new_value| {
        if field_name == "status" && new_value == "Done" {
            note.set_field("completed_date", today());
        }
    },
    
    // View configurations
    views: {
        kanban: {
            group_by: "status",
            sort_by: "priority",
            card_fields: ["title", "due_date", "assigned_to"]
        },
        calendar: {
            date_field: "due_date"
        },
        list: {
            sort_by: "due_date",
            columns: ["title", "status", "priority", "due_date"]
        }
    }
});
```

## Hierarchical Structure

All notes exist in a **tree hierarchy** (outliner style):
```
Project Alpha (note, type: "Project")
├── Overview (note, type: "Document")
├── Tasks (folder note)
│   ├── Design mockups (note, type: "Task")
│   ├── Implement backend (note, type: "Task")
│   └── Write tests (note, type: "Task")
├── Meeting Notes (folder note)
│   ├── 2024-01-15 Kickoff (note, type: "Meeting")
│   └── 2024-01-22 Review (note, type: "Meeting")
└── Research (folder note)
    └── Competitor Analysis (note, type: "Research")
```

**Key properties:**
- Any note can have children (tree structure)
- Notes can be different types at different levels
- Drag-drop reordering within parent
- Expand/collapse in tree view
- Notes inherit no behavior from parent (flat type system)

## View System

### Built-in View Types (Rust/iced)

Core view types are **implemented in Rust** for performance and consistency:

1. **Tree View** - Hierarchical outline with expand/collapse, drag-drop
2. **List View** - Flat list with sorting, filtering
3. **Table View** - Spreadsheet-style rows and columns
4. **Cards View** - Grid of cards (like Trello board without columns)
5. **Kanban View** - Columns with cards, drag-drop between columns
6. **Calendar View** - Month/week/day views based on date field
7. **Timeline View** - Gantt-chart style with start/end dates
8. **Graph View** - Network visualization of relationships

### View Configuration (Rhai Scripts)

Scripts **configure** these views declaratively, they don't implement rendering:
```rhai
// Define a custom view configuration
view("my_tasks_board", {
    type: "kanban",                          // Choose built-in view type
    note_type: "Task",                       // Which notes to display
    group_by: "status",                      // Kanban columns
    sort_within_column: "priority",          // Sort cards in each column
    card_fields: ["title", "due_date", "assigned_to"],
    filters: {
        status: not_equal("Done")            // Hide completed tasks
    }
});

view("project_timeline", {
    type: "timeline",
    note_type: "Task",
    start_field: "start_date",
    end_field: "due_date",
    group_by: "project",
    color_by: "priority"
});

view("team_graph", {
    type: "graph",
    note_types: ["Person", "Project", "Task"],
    show_relationships: ["assigned_to", "reports_to", "depends_on"],
    layout: "force_directed"
});
```

## Scripting System (Rhai)

### Script API Surface

**Core note operations:**
```rhai
// CRUD
let note = app.create_note("Task", { title: "New task" });
let existing = app.get_note(note_id);
app.update_field(note_id, "status", "InProgress");
app.delete_note(note_id);

// Tree operations
app.add_child(parent_id, child_id, position);
app.move_note(note_id, new_parent_id, new_position);

// Queries
let results = app.query()
    .type("Task")
    .under_parent("Project Alpha")
    .where("priority", "High")
    .where("due_date", between(today(), week_from_now()))
    .sort_by("due_date")
    .execute();

// Batch operations
for note in results {
    note.set_field("status", "InProgress");
}
```

**Schema definition:**
```rhai
schema("TypeName", {
    fields: [ /* field definitions */ ],
    on_create: |note| { /* hook */ },
    on_field_change: |note, field, old, new| { /* hook */ },
    views: { /* view configs */ }
});
```

**Commands (user actions):**
```rhai
// Define a command that appears in UI
command("quick_capture", || {
    let text = ui.prompt("Quick capture:");
    app.create_note("Inbox", { item: text, processed: false });
});

command("weekly_review", || {
    let projects = app.get_notes_by_type("Project")
        .filter(|p| p.get_field("status") == "Active");
    
    for project in projects {
        let next_action = project.get_field("next_action");
        if next_action == null {
            ui.highlight(project, "⚠️ Missing next action");
        }
    }
});
```

**Automations:**
```rhai
// Event-based
on_note_created("Task", |note| {
    // Automatically tag tasks based on content
    if note.get_field("title").contains("urgent") {
        note.add_tag("priority");
    }
});

// Scheduled
on_schedule("daily", "08:00", || {
    let overdue = app.query()
        .type("Task")
        .where("due_date", before(today()))
        .where("status", not_equal("Done"))
        .execute();
    
    if overdue.len() > 0 {
        notify("You have " + overdue.len() + " overdue tasks");
    }
});
```

### Plugin Marketplace

Plugins are **git repositories** containing Rhai scripts:
```
marketplace/
├── gtd-system/
│   ├── manifest.toml
│   │   [plugin]
│   │   name = "GTD System"
│   │   version = "1.0.0"
│   │   author = "username"
│   │   description = "Getting Things Done methodology"
│   ├── schemas.rhai       (Project, Action, Inbox types)
│   ├── commands.rhai      (Quick capture, weekly review)
│   ├── automations.rhai   (Auto-process inbox)
│   └── README.md
│
├── zettelkasten/
│   ├── manifest.toml
│   ├── schemas.rhai       (Note, Index, Reference types)
│   └── views.rhai         (Graph view config)
│
└── crm/
    ├── manifest.toml
    ├── schemas.rhai       (Person, Company, Deal)
    └── automations.rhai   (Follow-up reminders)
```

**Plugin installation flow:**
1. Browse marketplace (or paste git URL)
2. Preview schema/commands it adds
3. Install → scripts downloaded to `~/.myapp/plugins/`
4. Scripts are loaded and schemas registered
5. User can now create notes of new types

**Plugin composition:**
```rhai
// User can extend/combine plugins
import "gtd";
import "crm";

// Extend GTD's Action with CRM reference
extend_schema("Action", {
    fields: [
        { name: "related_person", type: "reference", target: "Person" }
    ]
});
```

## Workspace & Authentication

### Workspace Modes

Users can work in three modes:

1. **Local Mode** - No authentication, no sync, all data local
2. **Personal Mode** - Authenticated, sync across user's devices
3. **Team Mode** - Authenticated, shared workspace with team members

### Workspace Structure
```rust
enum WorkspaceMode {
    Local,
    Personal { 
        account: Account, 
        sync_enabled: bool 
    },
    Team { 
        account: Account, 
        workspace_id: String, 
        members: Vec<User> 
    },
}

struct Workspace {
    mode: WorkspaceMode,
    name: String,
    db_path: PathBuf,              // Local SQLite database
    sync_engine: Option<SyncEngine>,
}
```

### Startup Flow

**First launch:**
```
┌────────────────────────────────┐
│  Welcome!                      │
├────────────────────────────────┤
│  How would you like to start?  │
│                                │
│  📁 Use Locally                │
│  No account needed             │
│  Data stays on this device     │
│                                │
│  ☁️  Create Account             │
│  Sync across devices           │
│  Collaborate with teams        │
└────────────────────────────────┘
```

**Subsequent launches:**
```
┌────────────────────────────────────────┐
│  Select Workspace                      │
├────────────────────────────────────────┤
│                                        │
│  📁 My Personal Notes                  │
│     Local • Last opened 2 hours ago    │
│                                        │
│  ☁️  john@example.com                  │
│     Personal • Synced 5 minutes ago    │
│                                        │
│  👥 Acme Corp Team                     │
│     Team • Synced 1 hour ago           │
│                                        │
├────────────────────────────────────────┤
│  + New Local Workspace                 │
│  🔐 Sign in to Account                 │
└────────────────────────────────────────┘
```

### Data Isolation

Each workspace has its own SQLite database:
- Local: `~/.myapp/local_workspace_1.db`
- Personal: `~/.myapp/cache_user_abc123.db`
- Team: `~/.myapp/cache_team_xyz789.db`

Complete isolation - no data leakage between workspaces.

### Authentication

**Simple auth flow:**
- Email/password
- OAuth (Google, GitHub) - later phase
- Auth token stored in OS keychain (secure)

**No authentication required for:**
- Local mode
- Viewing/using the app offline
- Exporting data

### Upgrade Path

**Local → Personal:**
```rust
// User clicks "Enable Sync" in local workspace
async fn upgrade_to_personal(
    local_workspace: &Workspace,
    account: Account
) -> Result<Workspace> {
    // Create online account workspace
    let online = create_online_workspace(account).await?;
    
    // Copy all data
    let notes = local_workspace.export_all()?;
    online.import_all(notes).await?;
    
    // Initial sync
    online.sync().await?;
    
    Ok(online)
}
```

### Monetization Tiers (Future)
```rust
enum AccountTier {
    Free {
        max_devices: 2,
        storage_mb: 100,
    },
    Pro {
        max_devices: usize::MAX,
        storage_gb: 10,
        team_workspaces: 3,
    },
    Team {
        seats: usize,              // Number of team members
        storage_gb: 100,
        priority_support: true,
    },
}
```

## Sync Engine

### Requirements

- **Offline-first:** App fully functional offline, queues sync operations
- **Conflict-free:** CRDT-based merging, no "choose version A or B"
- **Multi-device:** Personal account syncs across user's devices
- **Multi-user:** Team workspaces handle concurrent edits from team members
- **Efficient:** Only sync changes, not entire database

### CRDT Approach

Use **Operation-based CRDTs** for tree operations:
```rust
// Operations are timestamped and have unique IDs
struct Operation {
    id: OperationId,           // Unique, monotonic
    timestamp: Lamport,         // Lamport timestamp for ordering
    device_id: String,
    op: TreeOperation,
}

// Operations are commutative when applied in causal order
enum TreeOperation {
    InsertNode { 
        node_id: String, 
        parent_id: Option<String>, 
        position: Position,      // CRDT position (not integer)
        data: Note 
    },
    MoveNode { 
        node_id: String, 
        new_parent: Option<String>, 
        new_position: Position 
    },
    DeleteNode { node_id: String },
    UpdateField { 
        node_id: String, 
        field: String, 
        value: FieldValue 
    },
}

// Position uses fractional indexing for CRDT ordering
struct Position(String);  // e.g., "a0", "a1", "a0.5"
```

**Conflict resolution strategies:**

1. **Field updates:** Last-write-wins (LWW) with Lamport timestamps
2. **Tree structure:** Concurrent moves → both succeed, position uses CRDT ordering
3. **Deletions:** Tombstones, eventual consistency
4. **Schema changes:** Additive only (new fields OK, removing fields requires migration script)

### Sync Protocol
```rust
async fn sync(&mut self) -> Result<()> {
    // 1. Get operations since last sync
    let local_ops = self.get_local_operations_since(self.last_sync)?;
    
    // 2. Push to server
    self.push_operations(local_ops).await?;
    
    // 3. Fetch remote operations
    let remote_ops = self.fetch_remote_operations_since(self.last_sync).await?;
    
    // 4. Apply remote operations (CRDTs guarantee convergence)
    self.apply_operations(remote_ops)?;
    
    // 5. Update sync cursor
    self.last_sync = now();
    
    Ok(())
}
```

**Optimizations:**
- Batch operations
- Compress operation log
- Garbage collect old operations after all devices sync
- Delta sync (only changes, not full state)

## Implementation Phases

### Phase 1: Core Engine (Local-only)

**Goal:** Single-user, local-only, basic functionality

**Deliverables:**
- SQLite storage for notes (generic key-value fields)
- Tree operations (CRUD, add_child, move, reorder)
- Basic iced UI with Tree view
- Simple note types (hardcoded in Rust initially)
- Query engine (filter, sort)

**Success criteria:**
- Can create hierarchical notes
- Can view in tree/list
- Data persists across app restarts

### Phase 2: Scripting Foundation

**Goal:** Enable user-defined schemas via Rhai

**Deliverables:**
- Rhai runtime integration
- Script API for note operations
- Schema definition in scripts
- Load schemas from `~/.myapp/schemas/`
- Basic schema validation

**Success criteria:**
- Users can define custom note types via scripts
- Scripts can create/query/update notes
- Changes persist and reload correctly

### Phase 3: Rich Views & Commands

**Goal:** Multiple view types and user automation

**Deliverables:**
- Implement all 8 view types (Table, Cards, Kanban, Calendar, Timeline, Graph)
- View configuration via scripts
- Command system (user-defined actions)
- Event hooks (on_create, on_change)
- Scheduled automations

**Success criteria:**
- Users can view data in multiple formats
- Users can write automation scripts
- Scripts can respond to events

### Phase 4: Sync Engine (Personal)

**Goal:** Single-user multi-device sync

**Deliverables:**
- CRDT-based sync engine
- Personal account authentication
- Workspace mode selection (Local vs Personal)
- Cloud sync backend (simple REST API)
- Conflict-free merging

**Success criteria:**
- User can sync across 2+ devices
- Offline changes merge correctly
- No data loss from conflicts

### Phase 5: Plugin Marketplace

**Goal:** Community-driven extensibility

**Deliverables:**
- Plugin manifest format
- Plugin installation UI
- Browse/search marketplace
- Plugin versioning
- Example plugins (GTD, Zettelkasten, CRM)

**Success criteria:**
- Users can install community plugins
- Plugins define new note types
- Plugins can be shared/forked

### Phase 6: Team Workspaces

**Goal:** Multi-user collaboration

**Deliverables:**
- Team workspace mode
- Invite/remove team members
- Permissions system (read/write/admin)
- Real-time presence (optional)
- Audit log

**Success criteria:**
- Team members can collaborate on shared workspace
- Concurrent edits merge correctly
- Permissions respected

### Phase 7+: Advanced Features

- WASM plugin support (performance-critical extensions)
- End-to-end encryption
- Mobile apps (Flutter + Rust core)
- Custom view components (advanced scripting)
- Self-hosted sync server option
- API for external integrations

## Technical Considerations

### Performance

- **Fast startup:** Lazy-load schemas, index frequently accessed notes
- **Responsive UI:** Background threads for sync, don't block main thread
- **Large databases:** Pagination, virtual scrolling, incremental loading
- **Search:** Full-text search index (SQLite FTS5)

### Security

- **Local encryption:** Optional encryption at rest (user password-derived key)
- **Auth tokens:** Store in OS keychain, never in plaintext
- **Script sandboxing:** Rhai scripts can't access filesystem directly
- **Plugin permissions:** (future) Declare required permissions in manifest

### Data Portability

- **Export:** Always allow full export to JSON/Markdown
- **Import:** Support common formats (Notion, Obsidian, etc.)
- **No lock-in:** Even paid users can export all data

### Testing Strategy

- **Unit tests:** Core note operations, tree operations, query engine
- **Integration tests:** Sync engine, conflict resolution
- **Script tests:** Example schemas with test cases
- **UI tests:** iced UI interactions (where possible)

## Open Questions / Future Decisions

1. **Schema evolution:** How to handle breaking schema changes?
2. **Attachments:** Store files in SQLite as blobs or filesystem?
3. **Undo/redo:** Operation-based (CRDT-friendly) or state-based?
4. **Mobile UI:** Reuse iced or build separate Flutter app?
5. **Real-time sync:** WebSocket for live updates or polling?
6. **Plugin permissions:** Sandbox filesystem access? Network access?
7. **Custom view rendering:** Allow WASM components or keep declarative?

## Success Metrics

**User success:**
- Can users create their own schemas without documentation?
- Do power users actually write scripts?
- Is the app fast enough for daily use?

**Technical success:**
- Does sync work reliably across devices?
- Do conflicts resolve correctly?
- Can the codebase scale to 10+ view types, 100+ plugins?

**Business success (future):**
- Are users willing to pay for sync?
- Do teams adopt team workspaces?
- Does the marketplace grow organically?

## Business Model

### Open Source Core (Free)

**License:** MIT or Apache 2.0

**What's included:**
- Full Rust core engine (notes, tree operations, queries)
- All iced UI components (8 view types)
- Rhai scripting runtime and API
- Local SQLite storage
- Plugin system infrastructure
- Schema marketplace access
- Complete local-only functionality

**Distribution:**
- Source code on GitHub
- Binary releases (Windows, macOS, Linux)
- Build from source instructions
- Community contributions welcome

**What users get:**
- ✅ Unlimited local notes and workspaces
- ✅ User-defined schemas via Rhai
- ✅ All view types (Tree, Kanban, Calendar, Graph, etc.)
- ✅ Scriptable automation
- ✅ Community plugins and schemas
- ✅ Full data export (no lock-in)
- ❌ Cloud sync
- ❌ Multi-device access
- ❌ Team workspaces

### Premium Sync Plugin (Paid)

**License:** Proprietary, closed source

**Distribution:**
- Pre-compiled WASM plugin (`premium_sync.wasm`)
- Downloaded after purchase/subscription from website
- License key validation (embedded or separate)
- Plugin signature verification to prevent tampering

**What's included:**
- Cloud sync implementation
- Conflict resolution (CRDT-based)
- Authentication with sync servers
- Team workspace features
- Real-time collaboration (future)

**Subscription tiers:**
```
Free (Open Source)
├─ Local only
├─ Unlimited notes
├─ All view types
└─ Community plugins

Pro ($5-10/month)
├─ Everything in Free
├─ Cloud sync (up to 5 devices)
├─ 5GB storage
└─ Email support

Team ($15-20/user/month)
├─ Everything in Pro
├─ Unlimited devices
├─ Team workspaces
├─ 50GB storage per user
├─ Priority support
└─ Admin controls
```

### Plugin Architecture

**Core defines sync interface (open source):**
```rust
// src/sync_api.rs - Public trait in open source core
pub trait SyncProvider: Send + Sync {
    /// Authenticate with sync service
    fn authenticate(&mut self, credentials: Credentials) -> Result<Account>;
    
    /// Push local operations to remote
    fn push_operations(&mut self, ops: Vec<Operation>) -> Result<PushResult>;
    
    /// Fetch remote operations since cursor
    fn fetch_operations(&mut self, since: SyncCursor) -> Result<Vec<Operation>>;
    
    /// Create or join team workspace
    fn create_team_workspace(&mut self, name: &str) -> Result<WorkspaceId>;
    
    /// List available workspaces for authenticated user
    fn list_workspaces(&self) -> Result<Vec<WorkspaceInfo>>;
}

// Registration mechanism
pub fn register_sync_provider(provider: Box<dyn SyncProvider>) {
    SYNC_REGISTRY.lock().unwrap().register(provider);
}
```

**Premium plugin implements interface (proprietary):**
```rust
// premium-sync-plugin/src/lib.rs - Proprietary WASM plugin
use myapp_core::SyncProvider;
use wasm_bindgen::prelude::*;

struct PremiumSyncProvider {
    auth_token: Option<String>,
    endpoint: String,
    // Proprietary sync implementation details
}

impl SyncProvider for PremiumSyncProvider {
    fn authenticate(&mut self, credentials: Credentials) -> Result<Account> {
        // Proprietary authentication logic
        // Validates subscription status
        // Returns account with sync capabilities
    }
    
    fn push_operations(&mut self, ops: Vec<Operation>) -> Result<PushResult> {
        // Proprietary sync protocol
        // CRDT merging algorithms
        // Conflict resolution
    }
    
    // ... rest of trait implementation
}

#[wasm_bindgen]
pub fn init_sync_plugin() -> Result<(), JsValue> {
    let provider = Box::new(PremiumSyncProvider::new());
    myapp_core::register_sync_provider(provider);
    Ok(())
}
```

**App adapts UI based on plugin availability:**
```rust
// src/main.rs
fn load_sync_plugins() {
    let plugin_path = get_plugin_dir().join("premium_sync.wasm");
    
    if plugin_path.exists() {
        match load_wasm_plugin(&plugin_path) {
            Ok(_) => {
                info!("Premium sync enabled");
                // UI shows sync features
            }
            Err(e) => {
                warn!("Failed to load sync plugin: {}", e);
                // UI shows upgrade prompt
            }
        }
    } else {
        info!("Free version - local only");
        // UI hides sync features, shows upgrade option
    }
}

// src/ui/workspace_selector.rs
fn render_workspace_options(&self) -> Element<Message> {
    let mut options = vec![
        button("New Local Workspace").on_press(Message::CreateLocal),
    ];
    
    if SYNC_REGISTRY.lock().unwrap().has_provider() {
        // Premium plugin loaded - show sync options
        options.push(
            button("Sign In to Sync Account")
                .on_press(Message::ShowSignIn)
        );
    } else {
        // No plugin - show upgrade prompt
        options.push(
            button("Unlock Cloud Sync (Premium)")
                .on_press(Message::ShowUpgradePrompt)
                .style(theme::Button::Premium)
        );
    }
    
    column(options).into()
}
```

### Security & Anti-Tampering

**Plugin signature verification:**
```rust
fn verify_plugin_signature(wasm_bytes: &[u8], signature: &[u8]) -> Result<()> {
    let public_key = include_bytes!("../keys/plugin_public.key");
    
    // Verify Ed25519 signature
    if !verify_signature(wasm_bytes, signature, public_key) {
        return Err(Error::InvalidPluginSignature);
    }
    
    Ok(())
}
```

**License validation (in premium plugin):**
```rust
// Option 1: Online validation
impl PremiumSyncProvider {
    fn authenticate(&mut self, credentials: Credentials) -> Result<Account> {
        let response = self.auth_api.login(credentials)?;
        
        // Server validates subscription status
        if !response.has_active_subscription {
            return Err(SyncError::SubscriptionRequired);
        }
        
        Ok(response.account)
    }
}

// Option 2: Cryptographic license key
#[wasm_bindgen]
pub fn validate_license(license_key: &str) -> Result<bool, JsValue> {
    // Validate signature against public key
    let valid = check_license_signature(license_key)?;
    if !valid {
        return Err(JsValue::from_str("Invalid license"));
    }
    Ok(true)
}
```

### Benefits of This Model

**For users:**
- **Try before buy:** Full-featured app locally, upgrade only when cloud sync is needed
- **No lock-in:** Can always export data and use free version
- **Transparent pricing:** Clear what premium provides (sync service)
- **Community benefits:** Open source enables plugins, improvements, and trust through code inspection

**For business:**
- **Protect IP:** Sync algorithms and protocols remain proprietary
- **Recurring revenue:** Subscription model for ongoing sync service
- **Community growth:** Free version builds user base and ecosystem
- **Lower support costs:** Community can help with core features
- **Innovation leverage:** Community contributions improve core product

**For ecosystem:**
- **Plugin marketplace grows:** Free users contribute schemas and plugins
- **Trust through transparency:** Open source core can be audited
- **Competitive pressure:** Community could build alternative sync plugins (keeps pricing fair)
- **Forks possible:** Ensures you maintain quality (users have alternatives)

### Community Sync Alternatives

The open plugin API allows community to build alternative sync implementations:

**Possible community plugins:**
- `sync-plugin-s3` - Sync via Amazon S3 bucket
- `sync-plugin-gdrive` - Sync via Google Drive
- `sync-plugin-webdav` - Sync via WebDAV server
- `sync-plugin-git` - Sync via git repository (like Obsidian Git)
- `sync-plugin-syncthing` - Sync via Syncthing
- `sync-plugin-local-network` - Sync over LAN

**Why this is good:**
- Validates the plugin architecture
- Provides free alternatives for DIY users
- Most users will still prefer official hosted sync (convenience)
- Creates healthy competition to maintain service quality

### Prior Art (Proven Business Models)

This approach has been successfully used by:

- **Obsidian** - Free app, paid sync ($8/month). Community sync plugins exist, but most users pay for official sync.
- **Tailscale** - Free for personal use, paid for teams. Open source client, proprietary coordination server.
- **GitLab** - Open source self-hostable, paid managed hosting.
- **Discourse** - Open source forum software, paid hosting service.

### Implementation Phases

**Phase 1-3:** Build open source core (local-only functionality)

**Phase 4:** Define and implement sync plugin API
- Design `SyncProvider` trait
- Implement plugin loading (WASM)
- Add UI hooks for plugin presence

**Phase 5:** Build proprietary sync plugin
- Implement sync protocol
- CRDT conflict resolution
- Authentication system
- Team workspace features

**Phase 6:** Launch premium service
- Set up sync backend infrastructure
- Payment/subscription system
- Plugin distribution system
- License validation

**Phase 7+:** Enhance premium features
- Real-time collaboration
- Advanced team features
- Mobile sync clients
- Enterprise self-hosting option

## Summary

This design creates a **power user's dream tool:**
- Full control over data structure (user-defined schemas)
- Flexible views (tree, kanban, calendar, graph, etc.)
- Scriptable automation (Rhai = modern BeanShell)
- Local-first (works offline, fast, private)
- Native performance (Rust + iced)
- Optional sync (start local, upgrade to cloud when needed)
- Extensible (plugin marketplace for sharing schemas)
- Free, open source core platform with premium business features (paid)

The key insight: **Schemas as code** enables a plugin ecosystem where users share complete productivity systems (GTD, Zettelkasten, PARA, etc.), not just individual features.

This is the spiritual successor to jEdit, but for personal knowledge management instead of text editing.

---

## Next Steps for Claude Code

1. **Validate architecture:** Review data model and sync approach
2. **Prototype core:** Implement Phase 1 (local-only, basic tree operations)
3. **Test Rhai integration:** Proof-of-concept for schema definition
4. **Design sync protocol:** Detailed CRDT implementation strategy
5. **UI mockups:** Sketch iced layouts for different view types

Ready to start building!