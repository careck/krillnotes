-- Notes table
CREATE TABLE IF NOT EXISTS notes (
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
    is_expanded INTEGER DEFAULT 1,
    FOREIGN KEY (parent_id) REFERENCES notes(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_notes_parent ON notes(parent_id, position);

-- Operations log
CREATE TABLE IF NOT EXISTS operations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    operation_id TEXT UNIQUE NOT NULL,
    timestamp INTEGER NOT NULL,
    device_id TEXT NOT NULL,
    operation_type TEXT NOT NULL,
    operation_data TEXT NOT NULL,
    synced INTEGER DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_operations_timestamp ON operations(timestamp);
CREATE INDEX IF NOT EXISTS idx_operations_synced ON operations(synced);

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
    modified_at INTEGER NOT NULL
);
