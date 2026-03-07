-- Notes table
CREATE TABLE IF NOT EXISTS notes (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    node_type TEXT NOT NULL,
    parent_id TEXT,
    position REAL NOT NULL DEFAULT 0.0,
    created_at INTEGER NOT NULL,
    modified_at INTEGER NOT NULL,
    created_by TEXT NOT NULL DEFAULT '',
    modified_by TEXT NOT NULL DEFAULT '',
    fields_json TEXT NOT NULL DEFAULT '{}',
    is_expanded INTEGER DEFAULT 1,
    schema_version INTEGER NOT NULL DEFAULT 1,
    FOREIGN KEY (parent_id) REFERENCES notes(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_notes_parent ON notes(parent_id, position);

-- Operations log (HLC timestamps: wall clock ms + logical counter + node id)
CREATE TABLE IF NOT EXISTS operations (
    operation_id TEXT NOT NULL PRIMARY KEY,
    timestamp_wall_ms INTEGER NOT NULL DEFAULT 0,
    timestamp_counter INTEGER NOT NULL DEFAULT 0,
    timestamp_node_id INTEGER NOT NULL DEFAULT 0,
    device_id TEXT NOT NULL,
    operation_type TEXT NOT NULL,
    operation_data TEXT NOT NULL,
    synced INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_operations_timestamp_wall_ms ON operations(timestamp_wall_ms);
CREATE INDEX IF NOT EXISTS idx_operations_synced ON operations(synced);

-- HLC (Hybrid Logical Clock) state — single row, id=1 always
CREATE TABLE IF NOT EXISTS hlc_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    wall_ms INTEGER NOT NULL,
    counter INTEGER NOT NULL,
    node_id INTEGER NOT NULL
);

-- Workspace metadata
CREATE TABLE IF NOT EXISTS workspace_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- User scripts
CREATE TABLE IF NOT EXISTS user_scripts (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    source_code TEXT NOT NULL,
    load_order INTEGER NOT NULL DEFAULT 0,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    modified_at INTEGER NOT NULL,
    category TEXT NOT NULL DEFAULT 'presentation'
);

-- Note tags (first-class, not schema-defined)
CREATE TABLE IF NOT EXISTS note_tags (
    note_id TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    tag     TEXT NOT NULL,
    PRIMARY KEY (note_id, tag)
);
CREATE INDEX IF NOT EXISTS idx_note_tags_tag ON note_tags(tag);

CREATE TABLE IF NOT EXISTS note_links (
    source_id  TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    field_name TEXT NOT NULL,
    target_id  TEXT NOT NULL REFERENCES notes(id) ON DELETE RESTRICT,
    PRIMARY KEY (source_id, field_name)
);
CREATE INDEX IF NOT EXISTS idx_note_links_target ON note_links(target_id);

-- Attachment metadata (encrypted files live on disk in attachments/ directory)
CREATE TABLE IF NOT EXISTS attachments (
    id          TEXT PRIMARY KEY,
    note_id     TEXT NOT NULL,
    filename    TEXT NOT NULL,
    mime_type   TEXT,
    size_bytes  INTEGER NOT NULL,
    hash_sha256 TEXT NOT NULL,
    salt        BLOB NOT NULL,
    created_at  INTEGER NOT NULL,
    FOREIGN KEY (note_id) REFERENCES notes(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_attachments_note_id ON attachments(note_id);
