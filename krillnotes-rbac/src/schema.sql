-- RBAC permission entries. One row per (note, user) pair.
CREATE TABLE IF NOT EXISTS note_permissions (
    note_id     TEXT NOT NULL,
    user_id     TEXT NOT NULL,
    role        TEXT NOT NULL CHECK(role IN ('owner', 'writer', 'reader')),
    granted_by  TEXT NOT NULL,
    PRIMARY KEY (note_id, user_id)
);
