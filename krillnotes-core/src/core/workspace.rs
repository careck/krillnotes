// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! High-level workspace operations over a Krillnotes SQLite database.

use crate::core::attachment::{
    decrypt_attachment, encrypt_attachment, AttachmentMeta,
};
use crate::core::contact::{generate_fingerprint, TrustLevel};
use crate::core::export::WorkspaceMetadata;
use crate::core::hlc::{HlcClock, HlcTimestamp};
use crate::core::peer_registry::{PeerInfo, PeerRegistry};
use crate::core::user_script;
#[allow(unused_imports)]
use crate::{
    get_device_id, DeleteResult, DeleteStrategy, FieldValue, KrillnotesError, Note,
    Operation, OperationLog, PurgeStrategy, QueryContext, Result, RetractInverse, SaveResult,
    ScriptError, ScriptRegistry, Storage, UndoResult, UserScript,
};
use rhai::Dynamic;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// An entry on the in-memory undo stack.
pub(crate) struct UndoEntry {
    /// Operation IDs in the log that this entry covers.
    pub(crate) retracted_ids: Vec<String>,
    pub(crate) inverse: RetractInverse,
    pub(crate) propagate: bool,
}

/// A lightweight search result containing only the ID and title of a note.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteSearchResult {
    pub id: String,
    pub title: String,
}

/// Serializable snapshot of a workspace's notes and scripts for peer sync.
#[derive(Debug, Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    pub version: u32,
    pub notes: Vec<Note>,
    pub user_scripts: Vec<UserScript>,
    #[serde(default)]
    pub attachments: Vec<AttachmentMeta>,
}

/// Controls where a new note is inserted relative to the currently selected note.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AddPosition {
    /// Insert as the first child of the selected note.
    AsChild,
    /// Insert immediately after the selected note within the same parent.
    AsSibling,
}

/// An open Krillnotes workspace backed by a SQLite database.
///
/// `Workspace` is the primary interface for all document mutations. It combines
/// a [`Storage`] connection, a [`ScriptRegistry`] for note-type validation and hooks,
/// and an [`OperationLog`] for durable change history.
///
/// Each instance is bound to a single window and protected by a `Mutex` in
/// the desktop application's state.
pub struct Workspace {
    storage: Storage,
    script_registry: ScriptRegistry,
    operation_log: OperationLog,
    device_id: String,
    /// UUID of the identity bound to this workspace (stored in workspace_meta).
    identity_uuid: String,
    /// Base64-encoded Ed25519 public key of the bound identity.
    /// Stamped onto every note as `created_by` / `modified_by`.
    current_identity_pubkey: String,
    /// Root directory for this workspace (parent of `notes.db`).
    workspace_root: PathBuf,
    /// Stable UUID for this workspace, stored in `workspace_meta`.
    /// Included in `info.json` so the workspace manager can resolve identity
    /// bindings without opening the encrypted database.
    workspace_id: String,
    /// ChaCha20-Poly1305 attachment key derived from password + workspace_id.
    /// `None` for unencrypted workspaces (empty password).
    attachment_key: Option<[u8; 32]>,
    pub(crate) undo_stack: Vec<UndoEntry>,
    pub(crate) redo_stack: Vec<UndoEntry>,
    pub(crate) undo_limit: usize,
    /// Separate undo/redo stacks for script-only mutations (create/update/delete script).
    /// Isolated from the note undo stack so script saves don't interleave with note edits.
    pub(crate) script_undo_stack: Vec<UndoEntry>,
    pub(crate) script_redo_stack: Vec<UndoEntry>,
    /// When Some, mutations accumulate here instead of pushing to undo_stack.
    undo_group_buffer: Option<Vec<UndoEntry>>,
    /// When `true`, `push_undo` is a no-op. Set while `apply_retract_inverse_internal`
    /// is executing so that mutations called from within an undo/redo do not push
    /// spurious entries onto the undo stack.
    inside_undo: bool,
    /// Hybrid Logical Clock for monotonically-ordered operation timestamps.
    hlc: HlcClock,
    /// Ed25519 signing key bound to the active identity.
    /// Every logged operation is signed with this key.
    signing_key: ed25519_dalek::SigningKey,
    /// Migration results from Phase D (run on workspace open).
    /// Drained by Tauri after the workspace is stored in AppState, to emit events.
    pub pending_migration_results: Vec<(String, u32, u32, u32)>,
}

impl Workspace {
    /// Creates a new workspace database at `path`, initialises the schema, and inserts
    /// a root note named after the file (e.g. `"My Notes"` for `my-notes.krillnotes`).
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] for any SQLite failure, or
    /// [`crate::KrillnotesError::InvalidWorkspace`] if the device ID cannot be obtained.
    pub fn create<P: AsRef<Path>>(path: P, password: &str, identity_uuid: &str, signing_key: ed25519_dalek::SigningKey) -> Result<Self> {
        let mut storage = Storage::create(&path, password)?;
        let mut script_registry = ScriptRegistry::new()?;
        let operation_log = OperationLog::new(PurgeStrategy::LocalOnly { keep_last: 100 });

        // Get hardware-based device ID
        let device_id = get_device_id()?;

        // Store metadata
        storage.connection().execute(
            "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
            ["device_id", &device_id],
        )?;
        storage.connection().execute(
            "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
            ["identity_uuid", identity_uuid],
        )?;

        // Derive workspace root from db path
        let workspace_root = path.as_ref()
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_path_buf();
        // Create attachments directory (idempotent, best-effort)
        let _ = std::fs::create_dir_all(workspace_root.join("attachments"));

        // Generate and store a stable workspace ID (used for attachment key derivation)
        let workspace_id = uuid::Uuid::new_v4().to_string();
        storage.connection().execute(
            "INSERT OR IGNORE INTO workspace_meta (key, value) VALUES (?, ?)",
            rusqlite::params!["workspace_id", &workspace_id],
        )?;

        // Derive attachment key
        let attachment_key = if !password.is_empty() {
            Some(crate::core::attachment::derive_attachment_key(password, &workspace_id))
        } else {
            None
        };

        // Seed the workspace with bundled starter scripts.
        let now = chrono::Utc::now().timestamp();
        let starters = ScriptRegistry::starter_scripts();
        {
            let tx = storage.connection_mut().transaction()?;
            for (load_order, starter) in starters.iter().enumerate() {
                let fm = user_script::parse_front_matter(&starter.source_code);
                let id = Uuid::new_v4().to_string();
                let category = if starter.filename.ends_with(".schema.rhai") { "schema" } else { "presentation" };
                tx.execute(
                    "INSERT INTO user_scripts (id, name, description, source_code, load_order, enabled, created_at, modified_at, category)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    rusqlite::params![id, fm.name, fm.description, &starter.source_code, load_order as i32, true, now, now, category],
                )?;
            }
            tx.commit()?;
        }

        // Load all scripts from the DB into the registry.
        let scripts = {
            let mut stmt = storage.connection().prepare(
                "SELECT id, name, description, source_code, load_order, enabled, created_at, modified_at, category
                 FROM user_scripts ORDER BY load_order ASC, created_at ASC",
            )?;
            let results: Vec<UserScript> = stmt.query_map([], |row| {
                Ok(UserScript {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    source_code: row.get(3)?,
                    load_order: row.get(4)?,
                    enabled: row.get::<_, i64>(5).map(|v| v != 0)?,
                    created_at: row.get(6)?,
                    modified_at: row.get(7)?,
                    category: row.get::<_, String>(8).unwrap_or_else(|_| "presentation".to_string()),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
            results
        };
        // Two-phase loading: presentation first, then schema, then resolve.
        for script in scripts.iter().filter(|s| s.enabled && s.category == "presentation") {
            script_registry.set_loading_category(Some("presentation".to_string()));
            if let Err(e) = script_registry.load_script(&script.source_code, &script.name) {
                eprintln!("Failed to load starter script '{}': {}", script.name, e);
            }
        }
        for script in scripts.iter().filter(|s| s.enabled && s.category == "schema") {
            script_registry.set_loading_category(Some("schema".to_string()));
            if let Err(e) = script_registry.load_script(&script.source_code, &script.name) {
                eprintln!("Failed to load starter script '{}': {}", script.name, e);
            }
        }
        script_registry.resolve_bindings();

        // Derive root note title from workspace folder name (parent of notes.db), not the db filename
        let filename = {
            let parent_name = path.as_ref()
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str());
            let db_stem = path.as_ref()
                .file_stem()
                .and_then(|s| s.to_str());
            // If parent is a real named folder (not "" or "."), use it; otherwise fall back to db stem
            match parent_name {
                Some(name) if !name.is_empty() && name != "." => name,
                _ => db_stem.unwrap_or("Untitled"),
            }
        };
        let title = humanize(filename);

        // Derive the base64-encoded public key from the signing key so we can
        // stamp it onto notes as created_by / modified_by.
        let identity_pubkey_b64 = {
            use base64::Engine as _;
            let pubkey = ed25519_dalek::VerifyingKey::from(&signing_key);
            base64::engine::general_purpose::STANDARD.encode(pubkey.as_bytes())
        };

        let root = Note {
            id: Uuid::new_v4().to_string(),
            title,
            schema: "TextNote".to_string(),
            parent_id: None,
            position: 0.0,
            created_at: now,
            modified_at: now,
            created_by: identity_pubkey_b64.clone(),
            modified_by: identity_pubkey_b64.clone(),
            fields: script_registry.get_schema("TextNote")?.default_fields(),
            is_expanded: true,
            tags: vec![], schema_version: 1,
        };

        let tx = storage.connection_mut().transaction()?;
        tx.execute(
            "INSERT INTO notes (id, title, schema, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded, schema_version)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                root.id,
                root.title,
                root.schema,
                root.parent_id,
                root.position,
                root.created_at,
                root.modified_at,
                root.created_by,
                root.modified_by,
                serde_json::to_string(&root.fields)?,
                true,
                root.schema_version,
            ],
        )?;
        tx.commit()?;

        storage.connection().execute(
            "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
            ["undo_limit", "50"],
        )?;
        let undo_limit: usize = 50;

        // Initialise HLC for this workspace.
        let node_id = crate::core::hlc::node_id_from_device(
            &uuid::Uuid::parse_str(&device_id).unwrap_or_else(|_| uuid::Uuid::new_v4()),
        );
        let hlc = HlcClock::new(node_id);

        let workspace = Self {
            storage,
            script_registry,
            operation_log,
            device_id,
            identity_uuid: identity_uuid.to_string(),
            current_identity_pubkey: identity_pubkey_b64,
            workspace_root,
            workspace_id,
            attachment_key,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            undo_limit,
            script_undo_stack: Vec::new(),
            script_redo_stack: Vec::new(),
            undo_group_buffer: None,
            inside_undo: false,
            hlc,
            signing_key,
            pending_migration_results: Vec::new(),
        };
        let _ = workspace.write_info_json(); // best-effort; non-fatal
        Ok(workspace)
    }

    /// Like [`create`] but uses the provided `workspace_id` instead of generating a fresh UUID.
    /// Use when restoring a workspace from a snapshot so all peers share the same UUID.
    /// The attachment key derivation uses `workspace_id`, so it must use the supplied ID.
    pub fn create_with_id<P: AsRef<Path>>(
        path: P,
        password: &str,
        identity_uuid: &str,
        signing_key: ed25519_dalek::SigningKey,
        workspace_id: &str,
    ) -> Result<Self> {
        let mut storage = Storage::create(&path, password)?;
        let mut script_registry = ScriptRegistry::new()?;
        let operation_log = OperationLog::new(PurgeStrategy::LocalOnly { keep_last: 100 });

        // Get hardware-based device ID
        let device_id = get_device_id()?;

        // Store metadata
        storage.connection().execute(
            "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
            ["device_id", &device_id],
        )?;
        storage.connection().execute(
            "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
            ["identity_uuid", identity_uuid],
        )?;

        // Derive workspace root from db path
        let workspace_root = path.as_ref()
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_path_buf();
        // Create attachments directory (idempotent, best-effort)
        let _ = std::fs::create_dir_all(workspace_root.join("attachments"));

        // Use the caller-supplied workspace_id instead of generating a fresh UUID.
        let workspace_id = workspace_id.to_string();
        storage.connection().execute(
            "INSERT OR IGNORE INTO workspace_meta (key, value) VALUES (?, ?)",
            rusqlite::params!["workspace_id", &workspace_id],
        )?;

        // Derive attachment key
        let attachment_key = if !password.is_empty() {
            Some(crate::core::attachment::derive_attachment_key(password, &workspace_id))
        } else {
            None
        };

        // Seed the workspace with bundled starter scripts.
        let now = chrono::Utc::now().timestamp();
        let starters = ScriptRegistry::starter_scripts();
        {
            let tx = storage.connection_mut().transaction()?;
            for (load_order, starter) in starters.iter().enumerate() {
                let fm = user_script::parse_front_matter(&starter.source_code);
                let id = Uuid::new_v4().to_string();
                let category = if starter.filename.ends_with(".schema.rhai") { "schema" } else { "presentation" };
                tx.execute(
                    "INSERT INTO user_scripts (id, name, description, source_code, load_order, enabled, created_at, modified_at, category)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    rusqlite::params![id, fm.name, fm.description, &starter.source_code, load_order as i32, true, now, now, category],
                )?;
            }
            tx.commit()?;
        }

        // Load all scripts from the DB into the registry.
        let scripts = {
            let mut stmt = storage.connection().prepare(
                "SELECT id, name, description, source_code, load_order, enabled, created_at, modified_at, category
                 FROM user_scripts ORDER BY load_order ASC, created_at ASC",
            )?;
            let results: Vec<UserScript> = stmt.query_map([], |row| {
                Ok(UserScript {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    source_code: row.get(3)?,
                    load_order: row.get(4)?,
                    enabled: row.get::<_, i64>(5).map(|v| v != 0)?,
                    created_at: row.get(6)?,
                    modified_at: row.get(7)?,
                    category: row.get::<_, String>(8).unwrap_or_else(|_| "presentation".to_string()),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
            results
        };
        // Two-phase loading: presentation first, then schema, then resolve.
        for script in scripts.iter().filter(|s| s.enabled && s.category == "presentation") {
            script_registry.set_loading_category(Some("presentation".to_string()));
            if let Err(e) = script_registry.load_script(&script.source_code, &script.name) {
                eprintln!("Failed to load starter script '{}': {}", script.name, e);
            }
        }
        for script in scripts.iter().filter(|s| s.enabled && s.category == "schema") {
            script_registry.set_loading_category(Some("schema".to_string()));
            if let Err(e) = script_registry.load_script(&script.source_code, &script.name) {
                eprintln!("Failed to load starter script '{}': {}", script.name, e);
            }
        }
        script_registry.resolve_bindings();

        // Derive root note title from workspace folder name (parent of notes.db), not the db filename
        let filename = {
            let parent_name = path.as_ref()
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str());
            let db_stem = path.as_ref()
                .file_stem()
                .and_then(|s| s.to_str());
            // If parent is a real named folder (not "" or "."), use it; otherwise fall back to db stem
            match parent_name {
                Some(name) if !name.is_empty() && name != "." => name,
                _ => db_stem.unwrap_or("Untitled"),
            }
        };
        let title = humanize(filename);

        // Derive the base64-encoded public key from the signing key so we can
        // stamp it onto notes as created_by / modified_by.
        let identity_pubkey_b64 = {
            use base64::Engine as _;
            let pubkey = ed25519_dalek::VerifyingKey::from(&signing_key);
            base64::engine::general_purpose::STANDARD.encode(pubkey.as_bytes())
        };

        let root = Note {
            id: Uuid::new_v4().to_string(),
            title,
            schema: "TextNote".to_string(),
            parent_id: None,
            position: 0.0,
            created_at: now,
            modified_at: now,
            created_by: identity_pubkey_b64.clone(),
            modified_by: identity_pubkey_b64.clone(),
            fields: script_registry.get_schema("TextNote")?.default_fields(),
            is_expanded: true,
            tags: vec![], schema_version: 1,
        };

        let tx = storage.connection_mut().transaction()?;
        tx.execute(
            "INSERT INTO notes (id, title, schema, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded, schema_version)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                root.id,
                root.title,
                root.schema,
                root.parent_id,
                root.position,
                root.created_at,
                root.modified_at,
                root.created_by,
                root.modified_by,
                serde_json::to_string(&root.fields)?,
                true,
                root.schema_version,
            ],
        )?;
        tx.commit()?;

        storage.connection().execute(
            "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
            ["undo_limit", "50"],
        )?;
        let undo_limit: usize = 50;

        // Initialise HLC for this workspace.
        let node_id = crate::core::hlc::node_id_from_device(
            &uuid::Uuid::parse_str(&device_id).unwrap_or_else(|_| uuid::Uuid::new_v4()),
        );
        let hlc = HlcClock::new(node_id);

        let workspace = Self {
            storage,
            script_registry,
            operation_log,
            device_id,
            identity_uuid: identity_uuid.to_string(),
            current_identity_pubkey: identity_pubkey_b64,
            workspace_root,
            workspace_id,
            attachment_key,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            undo_limit,
            script_undo_stack: Vec::new(),
            script_redo_stack: Vec::new(),
            undo_group_buffer: None,
            inside_undo: false,
            hlc,
            signing_key,
            pending_migration_results: Vec::new(),
        };
        let _ = workspace.write_info_json(); // best-effort; non-fatal
        Ok(workspace)
    }

    /// Like [`create`] but does **not** insert a default root note.
    ///
    /// Use this when the workspace content will immediately be populated from an
    /// external source (e.g. a snapshot import), so the seed note would only create
    /// unwanted noise alongside the imported tree.
    pub fn create_empty<P: AsRef<Path>>(path: P, password: &str, identity_uuid: &str, signing_key: ed25519_dalek::SigningKey) -> Result<Self> {
        let mut storage = Storage::create(&path, password)?;
        let mut script_registry = ScriptRegistry::new()?;
        let operation_log = OperationLog::new(PurgeStrategy::LocalOnly { keep_last: 100 });

        let device_id = get_device_id()?;

        storage.connection().execute(
            "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
            ["device_id", &device_id],
        )?;
        storage.connection().execute(
            "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
            ["identity_uuid", identity_uuid],
        )?;

        let workspace_root = path.as_ref()
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_path_buf();
        let _ = std::fs::create_dir_all(workspace_root.join("attachments"));

        let workspace_id = uuid::Uuid::new_v4().to_string();
        storage.connection().execute(
            "INSERT OR IGNORE INTO workspace_meta (key, value) VALUES (?, ?)",
            rusqlite::params!["workspace_id", &workspace_id],
        )?;

        let attachment_key = if !password.is_empty() {
            Some(crate::core::attachment::derive_attachment_key(password, &workspace_id))
        } else {
            None
        };

        // Seed starter scripts (same as create).
        let now = chrono::Utc::now().timestamp();
        let starters = ScriptRegistry::starter_scripts();
        {
            let tx = storage.connection_mut().transaction()?;
            for (load_order, starter) in starters.iter().enumerate() {
                let fm = user_script::parse_front_matter(&starter.source_code);
                let id = Uuid::new_v4().to_string();
                let category = if starter.filename.ends_with(".schema.rhai") { "schema" } else { "presentation" };
                tx.execute(
                    "INSERT INTO user_scripts (id, name, description, source_code, load_order, enabled, created_at, modified_at, category)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    rusqlite::params![id, fm.name, fm.description, &starter.source_code, load_order as i32, true, now, now, category],
                )?;
            }
            tx.commit()?;
        }

        let scripts = {
            let mut stmt = storage.connection().prepare(
                "SELECT id, name, description, source_code, load_order, enabled, created_at, modified_at, category
                 FROM user_scripts ORDER BY load_order ASC, created_at ASC",
            )?;
            let results: Vec<UserScript> = stmt.query_map([], |row| {
                Ok(UserScript {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    source_code: row.get(3)?,
                    load_order: row.get(4)?,
                    enabled: row.get::<_, i64>(5).map(|v| v != 0)?,
                    created_at: row.get(6)?,
                    modified_at: row.get(7)?,
                    category: row.get::<_, String>(8).unwrap_or_else(|_| "presentation".to_string()),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
            results
        };
        for script in scripts.iter().filter(|s| s.enabled && s.category == "presentation") {
            script_registry.set_loading_category(Some("presentation".to_string()));
            let _ = script_registry.load_script(&script.source_code, &script.name);
        }
        for script in scripts.iter().filter(|s| s.enabled && s.category == "schema") {
            script_registry.set_loading_category(Some("schema".to_string()));
            let _ = script_registry.load_script(&script.source_code, &script.name);
        }
        script_registry.resolve_bindings();

        let identity_pubkey_b64 = {
            use base64::Engine as _;
            let pubkey = ed25519_dalek::VerifyingKey::from(&signing_key);
            base64::engine::general_purpose::STANDARD.encode(pubkey.as_bytes())
        };

        // No default root note — content will come from the imported snapshot.

        storage.connection().execute(
            "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
            ["undo_limit", "50"],
        )?;
        let undo_limit: usize = 50;

        let node_id = crate::core::hlc::node_id_from_device(
            &uuid::Uuid::parse_str(&device_id).unwrap_or_else(|_| uuid::Uuid::new_v4()),
        );
        let hlc = HlcClock::new(node_id);

        let workspace = Self {
            storage,
            script_registry,
            operation_log,
            device_id,
            identity_uuid: identity_uuid.to_string(),
            current_identity_pubkey: identity_pubkey_b64,
            workspace_root,
            workspace_id,
            attachment_key,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            undo_limit,
            script_undo_stack: Vec::new(),
            script_redo_stack: Vec::new(),
            undo_group_buffer: None,
            inside_undo: false,
            hlc,
            signing_key,
            pending_migration_results: Vec::new(),
        };
        let _ = workspace.write_info_json();
        Ok(workspace)
    }

    /// Like [`create_empty`] but uses the provided `workspace_id` instead of generating a fresh UUID.
    ///
    /// Use this when restoring a workspace from a snapshot so all peers share the same UUID,
    /// and no default root note is inserted (the snapshot import will populate notes itself).
    pub fn create_empty_with_id<P: AsRef<Path>>(
        path: P,
        password: &str,
        identity_uuid: &str,
        signing_key: ed25519_dalek::SigningKey,
        workspace_id: &str,
    ) -> Result<Self> {
        let storage = Storage::create(&path, password)?;
        let script_registry = ScriptRegistry::new()?;
        let operation_log = OperationLog::new(PurgeStrategy::LocalOnly { keep_last: 100 });

        let device_id = get_device_id()?;

        storage.connection().execute(
            "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
            ["device_id", &device_id],
        )?;
        storage.connection().execute(
            "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
            ["identity_uuid", identity_uuid],
        )?;

        let workspace_root = path.as_ref()
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_path_buf();
        let _ = std::fs::create_dir_all(workspace_root.join("attachments"));

        // Use the caller-supplied workspace_id instead of generating a fresh UUID.
        let workspace_id = workspace_id.to_string();
        storage.connection().execute(
            "INSERT OR IGNORE INTO workspace_meta (key, value) VALUES (?, ?)",
            rusqlite::params!["workspace_id", &workspace_id],
        )?;

        let attachment_key = if !password.is_empty() {
            Some(crate::core::attachment::derive_attachment_key(password, &workspace_id))
        } else {
            None
        };

        // No starter scripts: this constructor is for snapshot restoration.
        // The caller (apply_swarm_snapshot) will call reload_all_scripts()
        // after import_snapshot_json() to run the snapshot's own scripts.

        let identity_pubkey_b64 = {
            use base64::Engine as _;
            let pubkey = ed25519_dalek::VerifyingKey::from(&signing_key);
            base64::engine::general_purpose::STANDARD.encode(pubkey.as_bytes())
        };

        // No default root note — content will come from the imported snapshot.

        storage.connection().execute(
            "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
            ["undo_limit", "50"],
        )?;
        let undo_limit: usize = 50;

        let node_id = crate::core::hlc::node_id_from_device(
            &uuid::Uuid::parse_str(&device_id).unwrap_or_else(|_| uuid::Uuid::new_v4()),
        );
        let hlc = HlcClock::new(node_id);

        let workspace = Self {
            storage,
            script_registry,
            operation_log,
            device_id,
            identity_uuid: identity_uuid.to_string(),
            current_identity_pubkey: identity_pubkey_b64,
            workspace_root,
            workspace_id,
            attachment_key,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            undo_limit,
            script_undo_stack: Vec::new(),
            script_redo_stack: Vec::new(),
            undo_group_buffer: None,
            inside_undo: false,
            hlc,
            signing_key,
            pending_migration_results: Vec::new(),
        };
        let _ = workspace.write_info_json();
        Ok(workspace)
    }

    /// Opens an existing workspace database at `path` and reads stored metadata.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::WrongPassword`] if the password is
    /// incorrect, [`crate::KrillnotesError::UnencryptedWorkspace`] if the file
    /// is a plain unencrypted SQLite database, or
    /// [`crate::KrillnotesError::Database`] for any SQLite failure.
    pub fn open<P: AsRef<Path>>(path: P, password: &str, identity_uuid: &str, signing_key: ed25519_dalek::SigningKey) -> Result<Self> {
        let storage = Storage::open(&path, password)?;
        let script_registry = ScriptRegistry::new()?;
        let operation_log = OperationLog::new(PurgeStrategy::LocalOnly { keep_last: 100 });

        // Read metadata from database
        let device_id = storage.connection()
            .query_row(
                "SELECT value FROM workspace_meta WHERE key = 'device_id'",
                [],
                |row| row.get::<_, String>(0)
            )?;

        // Derive the base64-encoded public key from the signing key.
        let identity_pubkey_b64 = {
            use base64::Engine as _;
            let pubkey = ed25519_dalek::VerifyingKey::from(&signing_key);
            base64::engine::general_purpose::STANDARD.encode(pubkey.as_bytes())
        };

        let workspace_root = path.as_ref()
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_path_buf();
        let _ = std::fs::create_dir_all(workspace_root.join("attachments"));

        // Read workspace_id for key derivation; generate one if absent (older workspaces)
        let workspace_id: String = {
            let existing: std::result::Result<String, rusqlite::Error> = storage.connection().query_row(
                "SELECT value FROM workspace_meta WHERE key = 'workspace_id'",
                [],
                |row| row.get(0),
            );
            match existing {
                Ok(id) => id,
                Err(_) => {
                    // Older workspace — generate and persist a new workspace_id
                    let id = uuid::Uuid::new_v4().to_string();
                    storage.connection().execute(
                        "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
                        rusqlite::params!["workspace_id", &id],
                    )?;
                    id
                }
            }
        };

        let attachment_key = if !password.is_empty() {
            Some(crate::core::attachment::derive_attachment_key(password, &workspace_id))
        } else {
            None
        };

        let undo_limit: usize = storage
            .connection()
            .query_row(
                "SELECT value FROM workspace_meta WHERE key = 'undo_limit'",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(50);

        // Load HLC state from DB (uses stored node_id if present, else derives from device_id).
        let node_id = crate::core::hlc::node_id_from_device(
            &uuid::Uuid::parse_str(&device_id).unwrap_or_else(|_| uuid::Uuid::new_v4()),
        );
        let hlc = HlcClock::load_from_db(storage.connection(), node_id)
            .map_err(KrillnotesError::Database)?;

        // Persist identity_uuid into workspace_meta if not already present
        // (handles workspaces created before identity enforcement).
        let _ = storage.connection().execute(
            "INSERT OR IGNORE INTO workspace_meta (key, value) VALUES (?, ?)",
            rusqlite::params!["identity_uuid", identity_uuid],
        );

        let mut ws = Self {
            storage,
            script_registry,
            operation_log,
            device_id,
            identity_uuid: identity_uuid.to_string(),
            current_identity_pubkey: identity_pubkey_b64,
            workspace_root,
            workspace_id,
            attachment_key,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            undo_limit,
            script_undo_stack: Vec::new(),
            script_redo_stack: Vec::new(),
            undo_group_buffer: None,
            inside_undo: false,
            hlc,
            signing_key,
            pending_migration_results: Vec::new(),
        };

        // Two-phase script loading: presentation first, then schema, then resolve.
        let scripts = ws.list_user_scripts()?;
        for script in scripts.iter().filter(|s| s.enabled && s.category == "presentation") {
            ws.script_registry.set_loading_category(Some("presentation".to_string()));
            if let Err(e) = ws.script_registry.load_script(&script.source_code, &script.name) {
                eprintln!("Failed to load script '{}': {}", script.name, e);
            }
        }
        for script in scripts.iter().filter(|s| s.enabled && s.category == "schema") {
            ws.script_registry.set_loading_category(Some("schema".to_string()));
            if let Err(e) = ws.script_registry.load_script(&script.source_code, &script.name) {
                eprintln!("Failed to load script '{}': {}", script.name, e);
            }
        }
        ws.script_registry.resolve_bindings();

        // Phase D: batch-migrate notes whose schema_version is behind current schema.
        ws.pending_migration_results = ws.run_schema_migrations()?;

        // Clean up any .enc.trash files left from a previous session.
        // Undo stacks are in-session only, so prior-session trash is always safe to remove.
        ws.purge_attachment_trash();

        let _ = ws.write_info_json(); // best-effort; non-fatal
        Ok(ws)
    }

    /// Returns the derived attachment encryption key, or `None` for an unencrypted workspace.
    pub fn attachment_key(&self) -> Option<&[u8; 32]> {
        self.attachment_key.as_ref()
    }

    /// Returns the workspace root directory (parent of `notes.db`).
    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    /// Returns the unique workspace UUID (stored in `workspace_meta`).
    pub fn workspace_id(&self) -> &str {
        &self.workspace_id
    }

    /// Returns the UUID of the identity bound to this workspace.
    pub fn identity_uuid(&self) -> &str {
        &self.identity_uuid
    }

    /// Returns the base64-encoded Ed25519 public key of the bound identity.
    /// This value is stamped onto every note as `created_by` / `modified_by`.
    pub fn identity_pubkey(&self) -> &str {
        &self.current_identity_pubkey
    }

    /// Writes `info.json` to the workspace root with cached metadata.
    /// Called on open, create, and window close so the workspace manager
    /// can display counts without opening the encrypted database.
    pub fn write_info_json(&self) -> Result<()> {
        let note_count: i64 = self.connection()
            .query_row(
                "SELECT COUNT(*) FROM notes",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let attachment_count: i64 = self.connection()
            .query_row("SELECT COUNT(*) FROM attachments", [], |row| row.get(0))
            .unwrap_or(0);

        // created_at = oldest note's created_at (best proxy for workspace age)
        let created_at: i64 = self.connection()
            .query_row(
                "SELECT MIN(created_at) FROM notes",
                [],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| chrono::Utc::now().timestamp());

        let info = serde_json::json!({
            "workspace_id": self.workspace_id,
            "created_at": created_at,
            "note_count": note_count,
            "attachment_count": attachment_count,
        });

        let path = self.workspace_root().join("info.json");
        std::fs::write(&path, serde_json::to_string(&info)?)?;

        Ok(())
    }

    /// Returns a reference to the script registry for this workspace.
    pub fn script_registry(&self) -> &ScriptRegistry {
        &self.script_registry
    }

    /// Returns a mutable reference to the script registry for this workspace.
    #[cfg(test)]
    pub(crate) fn script_registry_mut(&mut self) -> &mut ScriptRegistry {
        &mut self.script_registry
    }

    /// Returns the underlying SQLite connection.
    pub fn connection(&self) -> &Connection {
        self.storage.connection()
    }

    /// Logs an operation to the always-active operation log.
    /// Takes the log as an explicit parameter to avoid a whole-`self` borrow
    /// conflict with the transaction (which is borrowed from `self.storage`).
    fn log_op(log: &OperationLog, tx: &rusqlite::Transaction, op: &Operation) -> Result<()> {
        log.log(tx, op)
    }

    /// Purges stale operations from the always-active operation log.
    /// Takes the log as an explicit parameter for the same borrow-checker reason.
    fn purge_ops_if_needed(log: &OperationLog, tx: &rusqlite::Transaction) -> Result<()> {
        log.purge_if_needed(tx)
    }

    /// Advances the HLC and returns the next timestamp.
    ///
    /// Call this before opening a transaction (it only updates in-memory HLC state).
    /// Pair with [`Self::save_hlc`] inside the transaction to persist the new state.
    fn advance_hlc(&mut self) -> HlcTimestamp {
        self.hlc.now()
    }

    /// Persists the current HLC state to the `hlc_state` table within a transaction.
    ///
    /// Takes the HLC fields as a standalone parameter to avoid a borrow conflict
    /// with the transaction (which is borrowed from `self.storage`).
    fn save_hlc(ts: &HlcTimestamp, tx: &rusqlite::Transaction) -> Result<()> {
        tx.execute(
            "INSERT OR REPLACE INTO hlc_state (id, wall_ms, counter, node_id) VALUES (1, ?, ?, ?)",
            rusqlite::params![ts.wall_ms as i64, ts.counter as i64, ts.node_id as i64],
        )?;
        Ok(())
    }

    /// Signs `op` in place if a signing key is present. No-op otherwise.
    ///
    /// Takes the signing key as an explicit parameter to avoid a borrow conflict
    /// with the transaction (which is borrowed from `self.storage`).
    fn sign_op_with(signing_key: &ed25519_dalek::SigningKey, op: &mut Operation) {
        op.sign(signing_key);
    }

    /// Phase D: batch-migrate notes whose `schema_version` is behind the current schema version.
    ///
    /// For each schema that has `migrations` defined, queries all notes of that type with a
    /// stale `schema_version`, chains the migration closures in order, and commits the results
    /// in a single transaction. Logs one `UpdateSchema` operation per migrated schema type.
    ///
    /// Returns `(schema_name, min_from_version, to_version, notes_migrated)` for each schema
    /// type that had migrations to run.
    fn run_schema_migrations(&mut self) -> Result<Vec<(String, u32, u32, u32)>> {
        let versioned_schemas = self.script_registry.get_versioned_schemas();
        let mut results = Vec::new();

        for (schema_name, schema_version, migrations, ast) in versioned_schemas {
            if migrations.is_empty() {
                continue;
            }

            // Query notes that are behind the current schema version.
            let stale_notes: Vec<(String, String, String, u32)> = {
                let conn = self.storage.connection();
                let mut stmt = conn.prepare(
                    "SELECT id, title, fields_json, schema_version \
                     FROM notes WHERE schema = ?1 AND schema_version < ?2",
                )?;
                let rows = stmt.query_map(
                    rusqlite::params![&schema_name, schema_version],
                    |row| Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, u32>(3)?,
                    )),
                )?.collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };

            if stale_notes.is_empty() {
                continue;
            }

            let min_version = stale_notes.iter().map(|n| n.3).min().unwrap_or(1);
            let notes_count = stale_notes.len() as u32;

            let ast = match ast {
                Some(a) => a,
                None => {
                    self.script_registry.add_warning(
                        &schema_name,
                        &format!("Schema '{}' has migrations but no AST — skipping Phase D", schema_name),
                    );
                    continue;
                }
            };

            let tx = self.storage.connection_mut().transaction()?;
            let mut any_failed = false;

            for (note_id, title, fields_json, note_version) in &stale_notes {
                let fields: std::collections::BTreeMap<String, crate::FieldValue> =
                    serde_json::from_str(fields_json).unwrap_or_default();

                // Build note map for the migration closure.
                let mut fields_map = rhai::Map::new();
                for (k, v) in &fields {
                    fields_map.insert(k.as_str().into(), crate::core::scripting::field_value_to_dynamic(v));
                }
                let mut note_map = rhai::Map::new();
                note_map.insert("title".into(),  rhai::Dynamic::from(title.clone()));
                note_map.insert("fields".into(), rhai::Dynamic::from(fields_map));

                // Chain migration closures from note_version+1 to schema_version.
                let mut migration_error = false;
                for target_ver in (*note_version + 1)..=schema_version {
                    if let Some(fn_ptr) = migrations.get(&target_ver) {
                        match fn_ptr.call::<rhai::Dynamic>(
                            self.script_registry.engine(),
                            &ast,
                            (rhai::Dynamic::from(note_map.clone()),),
                        ) {
                            Ok(returned) => {
                                if let Some(m) = returned.try_cast::<rhai::Map>() {
                                    note_map = m;
                                }
                            }
                            Err(e) => {
                                self.script_registry.add_warning(
                                    &schema_name,
                                    &format!(
                                        "Migration to v{} failed for note '{}': {}",
                                        target_ver, note_id, e
                                    ),
                                );
                                migration_error = true;
                                break;
                            }
                        }
                    }
                    // No closure for this version → pass-through (schema-compatible gap).
                }

                if migration_error {
                    any_failed = true;
                    break;
                }

                // Extract new title and fields from the migrated map.
                let new_title = note_map.get("title")
                    .and_then(|v| v.clone().try_cast::<String>())
                    .unwrap_or_else(|| title.clone());
                let new_fields_json = if let Some(fm) = note_map.get("fields")
                    .and_then(|v| v.clone().try_cast::<rhai::Map>())
                {
                    let converted = self.script_registry.rhai_map_to_fields(&fm, &schema_name)?;
                    serde_json::to_string(&converted)?
                } else {
                    fields_json.clone()
                };

                tx.execute(
                    "UPDATE notes SET title = ?1, fields_json = ?2, schema_version = ?3 WHERE id = ?4",
                    rusqlite::params![new_title, new_fields_json, schema_version, note_id],
                )?;
            }

            if any_failed {
                // tx is dropped without commit → automatic rollback.
                continue;
            }

            // Log the UpdateSchema operation.
            let ts = self.hlc.now();
            Self::save_hlc(&ts, &tx)?;
            let mut op = Operation::UpdateSchema {
                operation_id: uuid::Uuid::new_v4().to_string(),
                timestamp: ts,
                device_id: self.device_id.clone(),
                signature: String::new(),
                updated_by: String::new(),
                schema_name: schema_name.clone(),
                from_version: min_version,
                to_version: schema_version,
                notes_migrated: notes_count,
            };
            Self::sign_op_with(&self.signing_key, &mut op);
            Self::log_op(&self.operation_log, &tx, &op)?;

            tx.commit()?;
            results.push((schema_name, min_version, schema_version, notes_count));
        }

        Ok(results)
    }

    /// Returns `true` if there is at least one action to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Returns `true` if there is at least one action to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Returns `true` if there is at least one script action to undo.
    pub fn can_script_undo(&self) -> bool {
        !self.script_undo_stack.is_empty()
    }

    /// Returns `true` if there is at least one script action to redo.
    pub fn can_script_redo(&self) -> bool {
        !self.script_redo_stack.is_empty()
    }

    /// Undoes the most recent script mutation (create/update/delete script).
    ///
    /// Script undo is separate from note undo to prevent script saves from
    /// interleaving with note edits in the workspace undo stack.
    pub fn script_undo(&mut self) -> Result<UndoResult> {
        let entry = self.script_undo_stack.pop()
            .ok_or_else(|| KrillnotesError::ValidationFailed("Nothing to undo".into()))?;

        let redo_inverse = self.build_redo_inverse(&entry)?;

        self.inside_undo = true;
        let apply_result = self.apply_retract_inverse_internal(&entry.inverse);
        self.inside_undo = false;
        apply_result?;

        self.script_redo_stack.push(UndoEntry {
            retracted_ids: entry.retracted_ids,
            inverse: redo_inverse,
            propagate: entry.propagate,
        });
        Ok(UndoResult { affected_note_id: None })
    }

    /// Re-applies the most recently undone script mutation.
    pub fn script_redo(&mut self) -> Result<UndoResult> {
        let entry = self.script_redo_stack.pop()
            .ok_or_else(|| KrillnotesError::ValidationFailed("Nothing to redo".into()))?;

        let new_undo_inverse = self.build_redo_inverse(&entry)?;

        self.inside_undo = true;
        let apply_result = self.apply_retract_inverse_internal(&entry.inverse);
        self.inside_undo = false;
        apply_result?;

        self.script_undo_stack.push(UndoEntry {
            retracted_ids: entry.retracted_ids,
            inverse: new_undo_inverse,
            propagate: entry.propagate,
        });
        // Trim to undo_limit.
        if self.script_undo_stack.len() > self.undo_limit {
            self.script_undo_stack.drain(0..1);
        }
        Ok(UndoResult { affected_note_id: None })
    }

    /// Returns the current undo stack depth limit.
    pub fn get_undo_limit(&self) -> usize {
        self.undo_limit
    }

    /// Sets the undo stack depth limit, persisting it to `workspace_meta`.
    ///
    /// The value is clamped to `[1, 500]`. If the new limit is smaller than
    /// the current stack depth, the oldest entries are dropped.
    pub fn set_undo_limit(&mut self, limit: usize) -> Result<()> {
        let limit = limit.max(1).min(500);
        self.storage.connection().execute(
            "INSERT OR REPLACE INTO workspace_meta (key, value) VALUES ('undo_limit', ?)",
            [limit.to_string()],
        )?;
        self.undo_limit = limit;
        if self.undo_stack.len() > limit {
            let excess = self.undo_stack.len() - limit;
            self.undo_stack.drain(0..excess);
        }
        Ok(())
    }

    /// Pushes an entry onto the undo stack (or into the group buffer if a group
    /// is open). Clears the redo stack. Trims to `undo_limit`.
    ///
    /// When `inside_undo` is `true` (i.e. we are executing `apply_retract_inverse_internal`
    /// on behalf of an undo or redo call), this is a no-op so that mutations
    /// invoked internally (e.g. `move_note` called from `PositionRestore`) do not
    /// push spurious entries onto the stack.
    fn push_undo(&mut self, entry: UndoEntry) {
        if self.inside_undo {
            return;
        }
        if let Some(buf) = &mut self.undo_group_buffer {
            buf.push(entry);
            return;
        }
        self.redo_stack.clear();
        self.undo_stack.push(entry);
        if self.undo_stack.len() > self.undo_limit {
            self.undo_stack.drain(0..1);
        }
    }

    /// Pushes an entry onto the script undo stack. Clears the script redo stack.
    /// No-op while `inside_undo` is `true`.
    fn push_script_undo(&mut self, entry: UndoEntry) {
        if self.inside_undo {
            return;
        }
        self.script_redo_stack.clear();
        self.script_undo_stack.push(entry);
        if self.script_undo_stack.len() > self.undo_limit {
            self.script_undo_stack.drain(0..1);
        }
    }

    /// Opens an undo group. Subsequent mutations accumulate in a staging buffer
    /// until `end_undo_group` is called, at which point they are collapsed into
    /// a single `UndoEntry` with a `RetractInverse::Batch` inverse.
    ///
    /// Nested calls are ignored — the outermost begin/end pair wins.
    pub fn begin_undo_group(&mut self) {
        if self.undo_group_buffer.is_none() {
            self.undo_group_buffer = Some(Vec::new());
        }
    }

    /// Closes the undo group and pushes a single batched `UndoEntry`.
    /// If the buffer is empty or no group is open, this is a no-op.
    pub fn end_undo_group(&mut self) {
        let Some(mut buf) = self.undo_group_buffer.take() else { return };
        if buf.is_empty() { return; }

        let retracted_ids: Vec<String> = buf.iter()
            .flat_map(|e| e.retracted_ids.iter().cloned())
            .collect();
        let propagate = buf.iter().any(|e| e.propagate);
        // Build Batch in original order; undo will apply LIFO.
        let inverses: Vec<RetractInverse> = buf.drain(..).map(|e| e.inverse).collect();

        self.redo_stack.clear();
        self.undo_stack.push(UndoEntry {
            retracted_ids,
            inverse: RetractInverse::Batch(inverses),
            propagate,
        });
        if self.undo_stack.len() > self.undo_limit {
            self.undo_stack.drain(0..1);
        }
    }

    /// Undoes the most recent operation on the undo stack.
    ///
    /// Returns an [`UndoResult`] indicating which note (if any) should be
    /// re-selected in the UI.
    ///
    /// # Errors
    ///
    /// Returns an error if the undo stack is empty or if applying the inverse
    /// operation fails.
    pub fn undo(&mut self) -> Result<UndoResult> {
        let entry = self.undo_stack.pop()
            .ok_or_else(|| KrillnotesError::ValidationFailed("Nothing to undo".into()))?;

        // Build the redo inverse BEFORE applying the undo so that the current
        // DB state can be captured. For example, for DeleteNote (which is the
        // inverse of CreateNote), we need to snapshot the note's data into a
        // SubtreeRestore while the note still exists in the DB.
        let redo_inverse = self.build_redo_inverse(&entry)?;

        // Apply the inverse to the DB. Set inside_undo so that any mutations
        // called from within apply_retract_inverse_internal (e.g. move_note
        // called from PositionRestore) do not push spurious undo entries.
        self.inside_undo = true;
        let apply_result = self.apply_retract_inverse_internal(&entry.inverse);
        self.inside_undo = false;
        let affected_note_id = apply_result?;

        // Write RetractOperation to the log.
        let retract_ts = self.advance_hlc();
        let retract_op_id = uuid::Uuid::new_v4().to_string();
        let retract_op = Operation::RetractOperation {
            operation_id: retract_op_id,
            timestamp: retract_ts,
            device_id: self.device_id.clone(),
            retracted_ids: entry.retracted_ids.clone(),
            inverse: entry.inverse.clone(),
            propagate: entry.propagate,
        };
        {
            let tx = self.storage.connection_mut().transaction()?;
            Self::save_hlc(&retract_ts, &tx)?;
            Self::log_op(&self.operation_log, &tx, &retract_op)?;
            Self::purge_ops_if_needed(&self.operation_log, &tx)?;
            tx.commit()?;
        }

        // Push onto redo stack using the pre-captured redo inverse so that
        // redo() can re-apply the forward operation (e.g. re-insert the note).
        self.redo_stack.push(UndoEntry {
            retracted_ids: entry.retracted_ids,
            inverse: redo_inverse,
            propagate: entry.propagate,
        });

        Ok(UndoResult { affected_note_id })
    }

    /// Re-applies the most recently undone operation from the redo stack.
    ///
    /// Returns an [`UndoResult`] indicating which note (if any) should be
    /// re-selected in the UI.
    ///
    /// # Errors
    ///
    /// Returns an error if the redo stack is empty or if re-applying the
    /// operation fails.
    pub fn redo(&mut self) -> Result<UndoResult> {
        let entry = self.redo_stack.pop()
            .ok_or_else(|| KrillnotesError::ValidationFailed("Nothing to redo".into()))?;

        // Build the new undo inverse BEFORE applying so that the current DB state
        // can be captured for the "undo of redo" entry.
        // For example:
        //   - If entry.inverse is DeleteNote{id} (redo = re-delete a note),
        //     we must capture SubtreeRestore while the note still exists in DB.
        //   - If entry.inverse is SubtreeRestore{notes} (redo = re-insert a note),
        //     we can extract the root ID from notes[0] without a DB query.
        let new_undo_inverse = self.build_redo_inverse(&entry)?;

        // Apply the redo entry's inverse to the DB.
        self.inside_undo = true;
        let apply_result = self.apply_retract_inverse_internal(&entry.inverse);
        self.inside_undo = false;
        let affected_note_id = apply_result?;

        // Log redo as a new RetractOperation.
        let redo_ts = self.advance_hlc();
        let new_op_id = uuid::Uuid::new_v4().to_string();
        let redo_op = Operation::RetractOperation {
            operation_id: new_op_id,
            timestamp: redo_ts,
            device_id: self.device_id.clone(),
            retracted_ids: entry.retracted_ids.clone(),
            inverse: entry.inverse.clone(),
            propagate: entry.propagate,
        };
        {
            let tx = self.storage.connection_mut().transaction()?;
            Self::save_hlc(&redo_ts, &tx)?;
            Self::log_op(&self.operation_log, &tx, &redo_op)?;
            Self::purge_ops_if_needed(&self.operation_log, &tx)?;
            tx.commit()?;
        }

        // Push a new undo entry carrying the new_undo_inverse so the redo can
        // itself be undone.
        self.undo_stack.push(UndoEntry {
            retracted_ids: entry.retracted_ids,
            inverse: new_undo_inverse,
            propagate: entry.propagate,
        });

        Ok(UndoResult { affected_note_id })
    }

    /// Builds the inverse needed to reverse a redo operation — i.e. captures
    /// the current DB state so that the redo can itself be undone.
    ///
    /// For each `RetractInverse` variant this determines what "un-doing the redo"
    /// would require:
    ///
    /// - `DeleteNote`     (undo was: un-do a CreateNote) → redo re-deletes.
    ///                    Build `SubtreeRestore` from current state.
    /// - `SubtreeRestore` (undo was: un-do a DeleteNote) → redo re-deletes root.
    ///                    Build `DeleteNote`.
    /// - `NoteRestore`    → redo re-updates. Capture current state as `NoteRestore`.
    /// - `PositionRestore`→ redo re-moves. Capture current position as `PositionRestore`.
    /// - `DeleteScript`   → redo re-deletes. Script no longer exists post-undo;
    ///                    use a stub `ScriptRestore` (deletion needs no data).
    /// - `ScriptRestore`  → redo re-restores. Build `DeleteScript`.
    /// - `Batch`          → recurse in reverse LIFO order.
    fn build_redo_inverse(&self, undo_entry: &UndoEntry) -> Result<RetractInverse> {
        match &undo_entry.inverse {
            RetractInverse::DeleteNote { note_id } => {
                // Undo was DeleteNote (undoing CreateNote). Redo = re-delete.
                // Current state: note exists. Capture subtree for redo's undo.
                let notes = self.collect_subtree_notes(note_id)?;
                let attachments = self.get_attachments(note_id).unwrap_or_default();
                Ok(RetractInverse::SubtreeRestore { notes, attachments })
            }
            RetractInverse::SubtreeRestore { notes, .. } => {
                // Undo was SubtreeRestore (undoing DeleteNote). Redo = re-delete root.
                let root_id = notes.first().map(|n| n.id.clone())
                    .ok_or_else(|| KrillnotesError::ValidationFailed("empty subtree in redo inverse".into()))?;
                Ok(RetractInverse::DeleteNote { note_id: root_id })
            }
            RetractInverse::NoteRestore { note_id, .. } => {
                let current = self.get_note(note_id)?;
                Ok(RetractInverse::NoteRestore {
                    note_id: note_id.clone(),
                    old_title: current.title,
                    old_fields: current.fields,
                    old_tags: current.tags,
                })
            }
            RetractInverse::PositionRestore { note_id, .. } => {
                let current = self.get_note(note_id)?;
                Ok(RetractInverse::PositionRestore {
                    note_id: note_id.clone(),
                    old_parent_id: current.parent_id,
                    old_position: current.position,
                })
            }
            RetractInverse::DeleteScript { script_id } => {
                // Undo of CreateScript: redo should re-delete. Capture the
                // script's current state so that a subsequent undo-of-redo can
                // restore it fully (rather than using an empty placeholder).
                if let Ok(current) = self.get_user_script(script_id) {
                    Ok(RetractInverse::ScriptRestore {
                        script_id: script_id.clone(),
                        name: current.name,
                        description: current.description,
                        source_code: current.source_code,
                        load_order: current.load_order,
                        enabled: current.enabled,
                    })
                } else {
                    // Script already absent — redo entry is a no-op placeholder.
                    Ok(RetractInverse::ScriptRestore {
                        script_id: script_id.clone(),
                        name: String::new(),
                        description: String::new(),
                        source_code: String::new(),
                        load_order: 0,
                        enabled: false,
                    })
                }
            }
            RetractInverse::ScriptRestore { script_id, .. } => {
                // If the script exists now (undo of UpdateUserScript), redo must
                // restore it to its current (pre-undo) state, not delete it.
                // If it doesn't exist (undo of DeleteUserScript — script absent),
                // redo should delete it again.
                if let Ok(current) = self.get_user_script(script_id) {
                    Ok(RetractInverse::ScriptRestore {
                        script_id: script_id.clone(),
                        name: current.name,
                        description: current.description,
                        source_code: current.source_code,
                        load_order: current.load_order,
                        enabled: current.enabled,
                    })
                } else {
                    Ok(RetractInverse::DeleteScript { script_id: script_id.clone() })
                }
            }
            RetractInverse::AttachmentRestore { meta } => {
                // Undo was AttachmentRestore (undoing a DeleteAttachment).
                // build_redo_inverse is called BEFORE undo is applied, so the .enc.trash
                // file exists and the DB row is absent. Redo should soft-delete again.
                Ok(RetractInverse::AttachmentSoftDelete { attachment_id: meta.id.clone() })
            }
            RetractInverse::AttachmentSoftDelete { attachment_id } => {
                // Undo was AttachmentSoftDelete (redoing a DeleteAttachment).
                // build_redo_inverse is called BEFORE undo is applied, so the .enc file
                // exists and the DB row is present. Redo should restore to prior state.
                // Capture current meta from DB to populate the restore entry.
                let meta = self.get_attachment_meta(attachment_id)?;
                Ok(RetractInverse::AttachmentRestore { meta })
            }
            RetractInverse::Batch(items) => {
                // Build redo inverses in reverse order (LIFO mirror).
                let mut redo_items = Vec::with_capacity(items.len());
                for item in items.iter().rev() {
                    let entry = UndoEntry {
                        retracted_ids: vec![],
                        inverse: item.clone(),
                        propagate: undo_entry.propagate,
                    };
                    redo_items.push(self.build_redo_inverse(&entry)?);
                }
                Ok(RetractInverse::Batch(redo_items))
            }
        }
    }

    /// Fetches a single note by ID.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] if the note is not found or
    /// if `fields_json` cannot be deserialised.
    pub fn get_note(&self, note_id: &str) -> Result<Note> {
        let row = self.connection().query_row(
            "SELECT n.id, n.title, n.schema, n.parent_id, n.position,
                    n.created_at, n.modified_at, n.created_by, n.modified_by,
                    n.fields_json, n.is_expanded, n.schema_version,
                    GROUP_CONCAT(nt.tag, ',') AS tags_csv
             FROM notes n
             LEFT JOIN note_tags nt ON nt.note_id = n.id
             WHERE n.id = ?
             GROUP BY n.id",
            [note_id],
            map_note_row,
        )?;
        note_from_row_tuple(row)
    }

    /// Creates a new note of `note_type` relative to `selected_note_id`.
    ///
    /// The new note is inserted as a child or sibling according to `position`.
    /// Sibling insertion bumps the positions of all following siblings to make room.
    ///
    /// Returns the ID of the newly created note.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::SchemaNotFound`] if `note_type` is unknown,
    /// or [`crate::KrillnotesError::Database`] for any SQLite failure.
    pub fn create_note(
        &mut self,
        selected_note_id: &str,
        position: AddPosition,
        note_type: &str,
    ) -> Result<String> {
        let schema = self.script_registry.get_schema(note_type)?;
        let selected = self.get_note(selected_note_id)?;

        // Determine final parent and position
        let (final_parent, final_position) = match position {
            AddPosition::AsChild => (Some(selected.id.clone()), 0.0_f64),
            AddPosition::AsSibling => (selected.parent_id.clone(), selected.position + 1.0),
        };

        // Validate allowed_parent_schemas
        if !schema.allowed_parent_schemas.is_empty() {
            match &final_parent {
                None => return Err(KrillnotesError::InvalidMove(format!(
                    "Note type '{}' cannot be placed at root level", note_type
                ))),
                Some(pid) => {
                    let parent_note = self.get_note(pid)?;
                    if !schema.allowed_parent_schemas.contains(&parent_note.schema) {
                        return Err(KrillnotesError::InvalidMove(format!(
                            "Note type '{}' cannot be placed under '{}'",
                            note_type, parent_note.schema
                        )));
                    }
                }
            }
        }

        // Validate allowed_children_schemas on the parent schema
        if let Some(pid) = &final_parent {
            let parent_note = self.get_note(pid)?;
            let parent_schema = self.script_registry.get_schema(&parent_note.schema)?;
            if parent_schema.is_leaf {
                return Err(KrillnotesError::InvalidMove(format!(
                    "Cannot add children to a leaf note (schema: '{}')",
                    parent_note.schema
                )));
            }
            if !parent_schema.allowed_children_schemas.is_empty()
                && !parent_schema.allowed_children_schemas.contains(&note_type.to_string())
            {
                return Err(KrillnotesError::InvalidMove(format!(
                    "Note type '{}' is not allowed as a child of '{}'",
                    note_type, parent_note.schema
                )));
            }
        }

        // Fetch parent note before opening the transaction (avoids borrow conflict with `tx`).
        let hook_parent = if let Some(ref pid) = final_parent {
            Some(self.get_note(pid)?)
        } else {
            None
        };

        let now = chrono::Utc::now().timestamp();
        let mut note = Note {
            id: Uuid::new_v4().to_string(),
            title: "Untitled".to_string(),
            schema: note_type.to_string(),
            parent_id: final_parent,
            position: final_position,
            created_at: now,
            modified_at: now,
            created_by: self.current_identity_pubkey.clone(),
            modified_by: self.current_identity_pubkey.clone(),
            fields: schema.default_fields(),
            is_expanded: true,
            tags: vec![],
            schema_version: schema.version,
        };

        // Advance HLC and capture signing key before the transaction borrows self.storage.
        let ts = self.advance_hlc();
        let signing_key = self.signing_key.clone();

        let tx = self.storage.connection_mut().transaction()?;

        // For sibling insertion, bump positions of all following siblings to make room
        if let AddPosition::AsSibling = position {
            tx.execute(
                "UPDATE notes SET position = position + 1 WHERE parent_id IS ? AND position >= ?",
                rusqlite::params![note.parent_id, note.position],
            )?;
        }

        // Insert note
        tx.execute(
            "INSERT INTO notes (id, title, schema, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded, schema_version)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                note.id,
                note.title,
                note.schema,
                note.parent_id,
                note.position,
                note.created_at,
                note.modified_at,
                note.created_by,
                note.modified_by,
                serde_json::to_string(&note.fields)?,
                true,
                note.schema_version,
            ],
        )?;

        // Run on_add_child hook if the parent's schema defines one.
        // Allowed-parent and allowed-children checks have already passed above.
        if let Some(ref parent_note) = hook_parent {
            if let Some(hook_result) = self.script_registry.run_on_add_child_hook(
                &parent_note.schema,
                &parent_note.id, &parent_note.schema, &parent_note.title, &parent_note.fields,
                &note.id, &note.schema, &note.title, &note.fields,
            )? {
                let now = chrono::Utc::now().timestamp();
                if let Some((new_title, new_fields)) = hook_result.child {
                    let fields_json = serde_json::to_string(&new_fields)?;
                    tx.execute(
                        "UPDATE notes SET title = ?1, fields_json = ?2, modified_at = ?3 WHERE id = ?4",
                        rusqlite::params![new_title, fields_json, now, note.id],
                    )?;
                    // Keep note in sync with what was persisted so the operation log
                    // records the final stored values, not the pre-hook defaults.
                    note.title  = new_title;
                    note.fields = new_fields;
                }
                if let Some((new_title, new_fields)) = hook_result.parent {
                    let fields_json = serde_json::to_string(&new_fields)?;
                    tx.execute(
                        "UPDATE notes SET title = ?1, fields_json = ?2, modified_at = ?3 WHERE id = ?4",
                        rusqlite::params![new_title, fields_json, now, parent_note.id],
                    )?;
                }
            }
        }

        // Log operation
        Self::save_hlc(&ts, &tx)?;
        let mut op = Operation::CreateNote {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: ts,
            device_id: self.device_id.clone(),
            note_id: note.id.clone(),
            parent_id: note.parent_id.clone(),
            position: note.position,
            schema: note.schema.clone(),
            title: note.title.clone(),
            fields: note.fields.clone(),
            created_by: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&signing_key, &mut op);
        Self::log_op(&self.operation_log, &tx, &op)?;
        Self::purge_ops_if_needed(&self.operation_log, &tx)?;

        // Keep the note_links junction table in sync (no-op for default fields, correct for future use).
        // Must run inside the transaction so the link update is atomic with the note write.
        sync_note_links(&tx, &note.id, &note.fields)?;

        tx.commit()?;

        // Push undo entry — inverse of CreateNote is DeleteNote.
        let op_id = op.operation_id().to_string();
        let note_id = note.id.clone();
        self.push_undo(UndoEntry {
            retracted_ids: vec![op_id],
            inverse: RetractInverse::DeleteNote { note_id },
            propagate: true,
        });

        Ok(note.id)
    }

    /// Deep-copies the note at `source_id` and its entire descendant subtree,
    /// placing the copy at `target_id` with the given `position`.
    ///
    /// Returns the ID of the new root note.
    ///
    /// All notes in the subtree receive fresh UUIDs and current timestamps.
    /// Schema constraints (`allowed_parent_schemas`, `allowed_children_schemas`) are
    /// validated only for the root of the copy against the paste target.
    /// Children's internal parent/child relationships are trusted and not re-validated.
    pub fn deep_copy_note(
        &mut self,
        source_id: &str,
        target_id: &str,
        position: AddPosition,
    ) -> Result<String> {
        // 1. Load the full subtree rooted at source_id using an iterative BFS.
        let mut subtree: Vec<Note> = Vec::new();
        let mut queue: std::collections::VecDeque<String> = std::collections::VecDeque::new();
        queue.push_back(source_id.to_string());
        while let Some(current_id) = queue.pop_front() {
            let note = self.get_note(&current_id)?;
            // Enqueue children
            let child_ids: Vec<String> = self
                .connection()
                .prepare("SELECT id FROM notes WHERE parent_id = ? ORDER BY position")?
                .query_map([&current_id], |row| row.get(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            for cid in child_ids {
                queue.push_back(cid);
            }
            subtree.push(note);
        }

        if subtree.is_empty() {
            return Err(KrillnotesError::NoteNotFound(source_id.to_string()));
        }

        // 2. Validate the paste location for the root note only.
        let root_source = subtree[0].clone();
        let root_schema = self.script_registry.get_schema(&root_source.schema)?;
        let target_note = self.get_note(target_id)?;

        let (new_parent_id, new_position) = match position {
            AddPosition::AsChild => (Some(target_note.id.clone()), 0.0_f64),
            AddPosition::AsSibling => (target_note.parent_id.clone(), target_note.position + 1.0),
        };

        // Validate allowed_parent_schemas for the root copy
        if !root_schema.allowed_parent_schemas.is_empty() {
            match &new_parent_id {
                None => return Err(KrillnotesError::InvalidMove(format!(
                    "Note type '{}' cannot be placed at root level", root_source.schema
                ))),
                Some(pid) => {
                    let parent = self.get_note(pid)?;
                    if !root_schema.allowed_parent_schemas.contains(&parent.schema) {
                        return Err(KrillnotesError::InvalidMove(format!(
                            "Note type '{}' cannot be placed under '{}'",
                            root_source.schema, parent.schema
                        )));
                    }
                }
            }
        }

        // Validate allowed_children_schemas on the paste parent
        if let Some(pid) = &new_parent_id {
            let parent = self.get_note(pid)?;
            let parent_schema = self.script_registry.get_schema(&parent.schema)?;
            if parent_schema.is_leaf {
                return Err(KrillnotesError::InvalidMove(format!(
                    "Cannot add children to a leaf note (schema: '{}')",
                    parent.schema
                )));
            }
            if !parent_schema.allowed_children_schemas.is_empty()
                && !parent_schema.allowed_children_schemas.contains(&root_source.schema)
            {
                return Err(KrillnotesError::InvalidMove(format!(
                    "Note type '{}' is not allowed as a child of '{}'",
                    root_source.schema, parent.schema
                )));
            }
        }

        // 3. Build old_id → new_id remap table.
        let mut id_map: HashMap<String, String> = HashMap::new();
        for note in &subtree {
            id_map.insert(note.id.clone(), Uuid::new_v4().to_string());
        }

        let now = chrono::Utc::now().timestamp();

        // Pre-advance HLC once per note in the subtree, and capture signing key,
        // before the transaction borrows self.storage mutably.
        let subtree_timestamps: Vec<HlcTimestamp> = subtree.iter()
            .map(|_| self.advance_hlc())
            .collect();
        let signing_key = self.signing_key.clone();

        // 4. Insert all cloned notes in a single transaction.
        let tx = self.storage.connection_mut().transaction()?;

        // If pasting as sibling, bump positions of following siblings to make room.
        if let AddPosition::AsSibling = position {
            tx.execute(
                "UPDATE notes SET position = position + 1 WHERE parent_id IS ? AND position >= ?",
                rusqlite::params![new_parent_id, new_position],
            )?;
        }

        let root_new_id = id_map[source_id].clone();

        for (note, ts) in subtree.iter().zip(subtree_timestamps.iter()) {
            let new_id = id_map[&note.id].clone();
            let new_parent = if note.id == source_id {
                // Root of the copy gets the paste target as parent
                new_parent_id.clone()
            } else {
                // Children remap their parent_id through the id_map
                note.parent_id.as_ref().and_then(|pid| id_map.get(pid).cloned())
            };
            let this_position = if note.id == source_id { new_position } else { note.position };

            tx.execute(
                "INSERT INTO notes (id, title, schema, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded, schema_version)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    new_id,
                    note.title,
                    note.schema,
                    new_parent,
                    this_position,
                    now,
                    now,
                    self.current_identity_pubkey.clone(),
                    self.current_identity_pubkey.clone(),
                    serde_json::to_string(&note.fields)?,
                    note.is_expanded,
                    note.schema_version,
                ],
            )?;

            // Log a CreateNote operation for each inserted note.
            Self::save_hlc(ts, &tx)?;
            let mut op = Operation::CreateNote {
                operation_id: Uuid::new_v4().to_string(),
                timestamp: *ts,
                device_id: self.device_id.clone(),
                note_id: new_id.clone(),
                parent_id: new_parent,
                position: this_position as f64,
                schema: note.schema.clone(),
                title: note.title.clone(),
                fields: note.fields.clone(),
                created_by: String::new(),
                signature: String::new(),
            };
            Self::sign_op_with(&signing_key, &mut op);
            Self::log_op(&self.operation_log, &tx, &op)?;
        }

        Self::purge_ops_if_needed(&self.operation_log, &tx)?;
        tx.commit()?;

        self.push_undo(UndoEntry {
            retracted_ids: vec![],
            inverse: RetractInverse::DeleteNote { note_id: root_new_id.clone() },
            propagate: true,
        });

        Ok(root_new_id)
    }

    /// Creates a new root-level note of `node_type` with no parent.
    ///
    /// Returns the ID of the newly created note.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::SchemaNotFound`] if `node_type` is unknown,
    /// or [`crate::KrillnotesError::Database`] for any SQLite failure.
    pub fn create_note_root(&mut self, node_type: &str) -> Result<String> {
        let now = chrono::Utc::now().timestamp();
        let schema = self.script_registry.get_schema(node_type)?;

        // Validate allowed_parent_schemas — root notes have no parent
        if !schema.allowed_parent_schemas.is_empty() {
            return Err(KrillnotesError::InvalidMove(format!(
                "Note type '{}' cannot be placed at root level", node_type
            )));
        }

        let new_note = Note {
            id: Uuid::new_v4().to_string(),
            title: "Untitled".to_string(),
            schema: node_type.to_string(),
            parent_id: None,
            position: 0.0,
            created_at: now,
            modified_at: now,
            created_by: self.current_identity_pubkey.clone(),
            modified_by: self.current_identity_pubkey.clone(),
            fields: schema.default_fields(),
            is_expanded: true,
            tags: vec![], schema_version: 1,
        };

        let ts = self.advance_hlc();
        let signing_key = self.signing_key.clone();
        let tx = self.storage.connection_mut().transaction()?;

        tx.execute(
            "INSERT INTO notes (id, title, schema, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded, schema_version)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                new_note.id,
                new_note.title,
                new_note.schema,
                new_note.parent_id,
                new_note.position,
                new_note.created_at,
                new_note.modified_at,
                new_note.created_by,
                new_note.modified_by,
                serde_json::to_string(&new_note.fields)?,
                true,
                new_note.schema_version,
            ],
        )?;

        // Log operation
        Self::save_hlc(&ts, &tx)?;
        let mut op = Operation::CreateNote {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: ts,
            device_id: self.device_id.clone(),
            note_id: new_note.id.clone(),
            parent_id: new_note.parent_id.clone(),
            position: new_note.position,
            schema: new_note.schema.clone(),
            title: new_note.title.clone(),
            fields: new_note.fields.clone(),
            created_by: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&signing_key, &mut op);
        Self::log_op(&self.operation_log, &tx, &op)?;
        Self::purge_ops_if_needed(&self.operation_log, &tx)?;

        // Keep the note_links junction table in sync (no-op for default fields, correct for future use).
        // Must run inside the transaction so the link update is atomic with the note write.
        sync_note_links(&tx, &new_note.id, &new_note.fields)?;

        tx.commit()?;

        // Push undo entry — inverse of CreateNote is DeleteNote.
        let op_id = op.operation_id().to_string();
        let note_id = new_note.id.clone();
        self.push_undo(UndoEntry {
            retracted_ids: vec![op_id],
            inverse: RetractInverse::DeleteNote { note_id },
            propagate: true,
        });

        Ok(new_note.id)
    }

    /// Updates the title of `note_id` and logs an `UpdateNote` operation.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] if the note is not found or
    /// the UPDATE fails.
    pub fn update_note_title(&mut self, note_id: &str, new_title: String) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let ts = self.advance_hlc();
        let signing_key = self.signing_key.clone();
        let tx = self.storage.connection_mut().transaction()?;

        tx.execute(
            "UPDATE notes SET title = ?, modified_at = ?, modified_by = ? WHERE id = ?",
            rusqlite::params![new_title, now, self.current_identity_pubkey.clone(), note_id],
        )?;

        // Log operation
        Self::save_hlc(&ts, &tx)?;
        let mut op = Operation::UpdateNote {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: ts,
            device_id: self.device_id.clone(),
            note_id: note_id.to_string(),
            title: new_title,
            modified_by: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&signing_key, &mut op);
        Self::log_op(&self.operation_log, &tx, &op)?;
        Self::purge_ops_if_needed(&self.operation_log, &tx)?;

        tx.commit()?;
        Ok(())
    }

    /// Replaces all tags for `note_id` with the provided list.
    ///
    /// Tags are normalised (lowercased, trimmed, deduplicated) before storage.
    /// Deletes existing tags and re-inserts in a single transaction.
    pub fn update_note_tags(&mut self, note_id: &str, tags: Vec<String>) -> Result<()> {
        let mut normalised: Vec<String> = tags
            .into_iter()
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        normalised.sort();
        normalised.dedup();

        let ts = self.advance_hlc();
        let signing_key = self.signing_key.clone();

        let tx = self.storage.connection_mut().transaction()?;
        tx.execute("DELETE FROM note_tags WHERE note_id = ?", [note_id])?;
        for tag in &normalised {
            tx.execute(
                "INSERT INTO note_tags (note_id, tag) VALUES (?, ?)",
                rusqlite::params![note_id, tag],
            )?;
        }

        // Log a SetTags operation so peers can replicate tag changes.
        Self::save_hlc(&ts, &tx)?;
        let mut op = Operation::SetTags {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: ts,
            device_id: self.device_id.clone(),
            note_id: note_id.to_string(),
            tags: normalised,
            modified_by: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&signing_key, &mut op);
        Self::log_op(&self.operation_log, &tx, &op)?;
        Self::purge_ops_if_needed(&self.operation_log, &tx)?;

        tx.commit()?;
        Ok(())
    }

    /// Returns all distinct tags used across the workspace, sorted alphabetically.
    pub fn get_all_tags(&self) -> Result<Vec<String>> {
        let mut stmt = self.connection().prepare(
            "SELECT DISTINCT tag FROM note_tags ORDER BY tag"
        )?;
        let tags = stmt.query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(tags)
    }

    /// Returns all notes that have any of the provided tags (OR logic).
    ///
    /// Returns an empty vec if `tags` is empty.
    pub fn get_notes_for_tag(&self, tags: &[String]) -> Result<Vec<Note>> {
        if tags.is_empty() {
            return Ok(vec![]);
        }
        let placeholders = tags.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let sql = format!(
            "SELECT n.id, n.title, n.schema, n.parent_id, n.position,
                    n.created_at, n.modified_at, n.created_by, n.modified_by,
                    n.fields_json, n.is_expanded, n.schema_version,
                    GROUP_CONCAT(nt2.tag, ',') AS tags_csv
             FROM notes n
             JOIN note_tags nt ON nt.note_id = n.id AND nt.tag IN ({placeholders})
             LEFT JOIN note_tags nt2 ON nt2.note_id = n.id
             GROUP BY n.id
             ORDER BY n.parent_id, n.position"
        );
        let mut stmt = self.connection().prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> = tags.iter()
            .map(|t| t as &dyn rusqlite::ToSql)
            .collect();
        let rows = stmt.query_map(params.as_slice(), map_note_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(note_from_row_tuple).collect()
    }

    /// Returns all notes whose `note_link` fields point to `target_id`.
    ///
    /// Queries the `note_links` junction table for every source note that
    /// currently references `target_id`, then fetches each full `Note`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] for any SQLite failure.
    pub fn get_notes_with_link(&self, target_id: &str) -> Result<Vec<Note>> {
        let conn = self.connection();
        let mut stmt = conn.prepare(
            "SELECT nl.source_id FROM note_links nl WHERE nl.target_id = ?1",
        )?;
        let source_ids: Vec<String> = stmt
            .query_map([target_id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;

        let mut notes = Vec::new();
        for id in source_ids {
            notes.push(self.get_note(&id)?);
        }
        Ok(notes)
    }

    /// Searches for notes whose title or text-like field values contain `query`
    /// (case-insensitive substring match).
    ///
    /// If `target_type` is `Some`, only notes of that schema type are included.
    /// Returns an empty vec when `query` is blank.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] or
    /// [`crate::KrillnotesError::Json`] if the underlying note fetch fails.
    pub fn search_notes(
        &self,
        query: &str,
        target_schema: Option<&str>,
    ) -> Result<Vec<NoteSearchResult>> {
        let query_lower = query.to_lowercase();
        if query_lower.is_empty() {
            return Ok(vec![]);
        }

        let all_notes = self.list_all_notes()?;

        let results = all_notes
            .into_iter()
            .filter(|n| {
                if let Some(t) = target_schema {
                    n.schema == t
                } else {
                    true
                }
            })
            .filter(|n| {
                if n.title.to_lowercase().contains(&query_lower) {
                    return true;
                }
                for value in n.fields.values() {
                    match value {
                        FieldValue::Text(s) | FieldValue::Email(s) => {
                            if s.to_lowercase().contains(&query_lower) {
                                return true;
                            }
                        }
                        _ => {}
                    }
                }
                false
            })
            .map(|n| NoteSearchResult { id: n.id, title: n.title })
            .collect();

        Ok(results)
    }

    /// Rebuilds the `note_links` junction table from scratch by scanning all
    /// `fields_json` values for `NoteLink` entries.
    ///
    /// This is idempotent and safe to call at any time.  It is called
    /// automatically after a workspace import to restore link data that was not
    /// stored in the junction table at export time.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] or
    /// [`crate::KrillnotesError::Json`] if any note cannot be fetched.
    pub fn rebuild_note_links_index(&mut self) -> Result<()> {
        let all_notes = self.list_all_notes()?;
        let conn = self.storage.connection_mut();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM note_links", [])?;
        for note in &all_notes {
            for (field_name, value) in &note.fields {
                if let FieldValue::NoteLink(Some(target_id)) = value {
                    let exists: bool = tx.query_row(
                        "SELECT COUNT(*) FROM notes WHERE id = ?1",
                        [target_id],
                        |row| row.get::<_, i64>(0).map(|c| c > 0),
                    )?;
                    if exists {
                        tx.execute(
                            "INSERT INTO note_links (source_id, field_name, target_id)
                             VALUES (?1, ?2, ?3)",
                            [&note.id, field_name, target_id],
                        )?;
                    }
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Returns all notes in the workspace, ordered by `parent_id` then `position`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] for any SQLite failure, or
    /// [`crate::KrillnotesError::Json`] if any row's `fields_json` is corrupt.
    pub fn list_all_notes(&self) -> Result<Vec<Note>> {
        let mut stmt = self.connection().prepare(
            "SELECT n.id, n.title, n.schema, n.parent_id, n.position,
                    n.created_at, n.modified_at, n.created_by, n.modified_by,
                    n.fields_json, n.is_expanded, n.schema_version,
                    GROUP_CONCAT(nt.tag, ',') AS tags_csv
             FROM notes n
             LEFT JOIN note_tags nt ON nt.note_id = n.id
             GROUP BY n.id
             ORDER BY n.parent_id, n.position",
        )?;

        let rows = stmt
            .query_map([], map_note_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        rows.into_iter().map(note_from_row_tuple).collect()
    }

    /// Runs the `on_view` hook for the note's schema, falling back to a default
    /// HTML view when no hook is registered.
    ///
    /// The default view auto-renders `textarea` fields as CommonMark markdown.
    ///
    /// Builds a `QueryContext` from all notes and attachments in the workspace.
    fn build_query_context(&self) -> Result<QueryContext> {
        let all_notes = self.list_all_notes()?;
        let mut notes_by_id: HashMap<String, Dynamic> = HashMap::new();
        let mut children_by_id: HashMap<String, Vec<Dynamic>> = HashMap::new();
        let mut notes_by_type: HashMap<String, Vec<Dynamic>> = HashMap::new();
        let mut notes_by_tag: HashMap<String, Vec<Dynamic>> = HashMap::new();
        let mut notes_by_link_target: HashMap<String, Vec<Dynamic>> = HashMap::new();

        for n in &all_notes {
            let dyn_map = note_to_rhai_dynamic(n);
            notes_by_id.insert(n.id.clone(), dyn_map.clone());
            if let Some(pid) = &n.parent_id {
                children_by_id.entry(pid.clone()).or_default().push(dyn_map.clone());
            }
            notes_by_type.entry(n.schema.clone()).or_default().push(dyn_map.clone());
            for tag in &n.tags {
                notes_by_tag.entry(tag.clone()).or_default().push(dyn_map.clone());
            }
            for value in n.fields.values() {
                if let FieldValue::NoteLink(Some(target_id)) = value {
                    notes_by_link_target.entry(target_id.clone()).or_default().push(dyn_map.clone());
                }
            }
        }

        let mut attachments_by_note_id: HashMap<String, Vec<AttachmentMeta>> = HashMap::new();
        for att in self.list_all_attachments().unwrap_or_default() {
            attachments_by_note_id.entry(att.note_id.clone()).or_default().push(att);
        }

        Ok(QueryContext { notes_by_id, children_by_id, notes_by_type, notes_by_tag, notes_by_link_target, attachments_by_note_id })
    }

    /// # Errors
    ///
    /// Returns [`KrillnotesError::Database`] if the note or any workspace note
    /// cannot be fetched, or [`KrillnotesError::Scripting`] if the hook fails.
    pub fn run_view_hook(&self, note_id: &str) -> Result<String> {
        let note = self.get_note(note_id)?;

        // No hook registered: generate the default view without fetching all notes.
        if !self.script_registry.has_views(&note.schema) {
            // Pre-resolve NoteLink field targets to titles for the default renderer.
            let mut resolved_titles: std::collections::HashMap<String, String> = std::collections::HashMap::new();
            for value in note.fields.values() {
                if let FieldValue::NoteLink(Some(target_id)) = value {
                    if let Ok(linked) = self.get_note(target_id) {
                        resolved_titles.insert(target_id.clone(), linked.title);
                    }
                }
            }
            let attachments = self.get_attachments(&note.id).unwrap_or_default();
            return Ok(self.script_registry.render_default_view(&note, &resolved_titles, &attachments));
        }

        let all_notes = self.list_all_notes()?;

        let mut notes_by_id: std::collections::HashMap<String, Dynamic> =
            std::collections::HashMap::new();
        let mut children_by_id: std::collections::HashMap<String, Vec<Dynamic>> =
            std::collections::HashMap::new();
        let mut notes_by_type: std::collections::HashMap<String, Vec<Dynamic>> =
            std::collections::HashMap::new();
        let mut notes_by_tag: std::collections::HashMap<String, Vec<Dynamic>> =
            std::collections::HashMap::new();
        let mut notes_by_link_target: std::collections::HashMap<String, Vec<Dynamic>> =
            std::collections::HashMap::new();

        for n in &all_notes {
            let dyn_map = note_to_rhai_dynamic(n);
            notes_by_id.insert(n.id.clone(), dyn_map.clone());
            if let Some(pid) = &n.parent_id {
                children_by_id.entry(pid.clone()).or_default().push(dyn_map.clone());
            }
            notes_by_type.entry(n.schema.clone()).or_default().push(dyn_map.clone());
            for tag in &n.tags {
                notes_by_tag.entry(tag.clone()).or_default().push(dyn_map.clone());
            }
            for value in n.fields.values() {
                if let FieldValue::NoteLink(Some(target_id)) = value {
                    notes_by_link_target.entry(target_id.clone()).or_default().push(dyn_map.clone());
                }
            }
        }

        let mut attachments_by_note_id: HashMap<String, Vec<AttachmentMeta>> = HashMap::new();
        for att in self.list_all_attachments().unwrap_or_default() {
            attachments_by_note_id.entry(att.note_id.clone()).or_default().push(att);
        }
        let context = QueryContext { notes_by_id, children_by_id, notes_by_type, notes_by_tag, notes_by_link_target, attachments_by_note_id };

        // Set per-run context so markdown() and other helpers can resolve attachments.
        let attachments = self.get_attachments(&note.id).unwrap_or_default();
        self.script_registry.set_run_context(note.clone(), attachments);
        // RAII guard: ensures run_context is cleared even if hook panics
        struct RunContextGuard<'a>(&'a crate::core::scripting::ScriptRegistry);
        impl Drop for RunContextGuard<'_> {
            fn drop(&mut self) { self.0.clear_run_context(); }
        }
        let _guard = RunContextGuard(&self.script_registry);
        // run_on_view_hook returns Some(...) since we've confirmed a hook exists above.
        self
            .script_registry
            .run_on_view_hook(&note, context)
            .map(|opt| opt.unwrap_or_default())
            .map(|html| self.embed_attachment_images(html))
    }

    /// Runs the `on_hover` hook for the given note, if one is registered.
    ///
    /// Returns `Ok(None)` when no hook is registered for the note's schema type.
    /// Returns `Ok(Some(html))` with the generated HTML on success.
    pub fn run_hover_hook(&self, note_id: &str) -> Result<Option<String>> {
        let note = self.get_note(note_id)?;

        if !self.script_registry.has_hover(&note.schema) {
            return Ok(None);
        }

        let all_notes = self.list_all_notes()?;

        let mut notes_by_id: std::collections::HashMap<String, Dynamic> =
            std::collections::HashMap::new();
        let mut children_by_id: std::collections::HashMap<String, Vec<Dynamic>> =
            std::collections::HashMap::new();
        let mut notes_by_type: std::collections::HashMap<String, Vec<Dynamic>> =
            std::collections::HashMap::new();
        let mut notes_by_tag: std::collections::HashMap<String, Vec<Dynamic>> =
            std::collections::HashMap::new();
        let mut notes_by_link_target: std::collections::HashMap<String, Vec<Dynamic>> =
            std::collections::HashMap::new();

        for n in &all_notes {
            let dyn_map = note_to_rhai_dynamic(n);
            notes_by_id.insert(n.id.clone(), dyn_map.clone());
            if let Some(pid) = &n.parent_id {
                children_by_id.entry(pid.clone()).or_default().push(dyn_map.clone());
            }
            notes_by_type.entry(n.schema.clone()).or_default().push(dyn_map.clone());
            for tag in &n.tags {
                notes_by_tag.entry(tag.clone()).or_default().push(dyn_map.clone());
            }
            for value in n.fields.values() {
                if let FieldValue::NoteLink(Some(target_id)) = value {
                    notes_by_link_target.entry(target_id.clone()).or_default().push(dyn_map.clone());
                }
            }
        }

        let mut attachments_by_note_id: HashMap<String, Vec<AttachmentMeta>> = HashMap::new();
        for att in self.list_all_attachments().unwrap_or_default() {
            attachments_by_note_id.entry(att.note_id.clone()).or_default().push(att);
        }
        let context = QueryContext { notes_by_id, children_by_id, notes_by_type, notes_by_tag, notes_by_link_target, attachments_by_note_id };

        // Set per-run context so markdown() and other helpers can resolve attachments.
        let attachments = self.get_attachments(&note.id).unwrap_or_default();
        self.script_registry.set_run_context(note.clone(), attachments);
        // RAII guard: ensures run_context is cleared even if hook panics
        struct RunContextGuard<'a>(&'a crate::core::scripting::ScriptRegistry);
        impl Drop for RunContextGuard<'_> {
            fn drop(&mut self) { self.0.clear_run_context(); }
        }
        let _guard = RunContextGuard(&self.script_registry);
        self.script_registry
            .run_on_hover_hook(&note, context)
            .map(|opt| opt.map(|html| self.embed_attachment_images(html)))
    }

    /// Returns the names of all registered note types (schema names).
    ///
    /// # Errors
    ///
    /// This method currently does not fail, but returns `Result` for consistency.
    pub fn list_node_types(&self) -> Result<Vec<String>> {
        self.script_registry.list_types()
    }

    /// Runs the tree action named `label` on the note identified by `note_id`.
    ///
    /// Builds a full `QueryContext` (same as `run_view_hook`), calls the registered
    /// callback, and — if the callback returns an array of note IDs — reorders
    /// those notes by calling `move_note` in the given order.
    ///
    /// # Errors
    ///
    /// Returns an error if the note or any workspace note cannot be fetched, if
    /// no action is registered under `label`, or if the callback throws.
    pub fn run_tree_action(&mut self, note_id: &str, label: &str) -> Result<()> {
        self.begin_undo_group();
        let result = self.run_tree_action_inner(note_id, label);
        self.end_undo_group();
        result
    }

    fn run_tree_action_inner(&mut self, note_id: &str, label: &str) -> Result<()> {
        let note = self.get_note(note_id)?;
        let all_notes = self.list_all_notes()?;

        let mut notes_by_id: HashMap<String, Dynamic> = HashMap::new();
        let mut children_by_id: HashMap<String, Vec<Dynamic>> = HashMap::new();
        let mut notes_by_type: HashMap<String, Vec<Dynamic>> = HashMap::new();
        let mut notes_by_tag: HashMap<String, Vec<Dynamic>> = HashMap::new();
        let mut notes_by_link_target: HashMap<String, Vec<Dynamic>> = HashMap::new();
        for n in &all_notes {
            let dyn_map = note_to_rhai_dynamic(n);
            notes_by_id.insert(n.id.clone(), dyn_map.clone());
            if let Some(pid) = &n.parent_id {
                children_by_id.entry(pid.clone()).or_default().push(dyn_map.clone());
            }
            notes_by_type.entry(n.schema.clone()).or_default().push(dyn_map.clone());
            for tag in &n.tags {
                notes_by_tag.entry(tag.clone()).or_default().push(dyn_map.clone());
            }
            for value in n.fields.values() {
                if let FieldValue::NoteLink(Some(target_id)) = value {
                    notes_by_link_target.entry(target_id.clone()).or_default().push(dyn_map.clone());
                }
            }
        }
        let mut attachments_by_note_id: HashMap<String, Vec<AttachmentMeta>> = HashMap::new();
        for att in self.list_all_attachments().unwrap_or_default() {
            attachments_by_note_id.entry(att.note_id.clone()).or_default().push(att);
        }
        let context = QueryContext { notes_by_id, children_by_id, notes_by_type, notes_by_tag, notes_by_link_target, attachments_by_note_id };

        // invoke_tree_action_hook returns an error if the script throws — in that case
        // we propagate the error without touching the DB (implicit rollback).
        let result = self.script_registry.invoke_tree_action_hook(label, &note, context)?;

        // Apply pending notes from the SaveTransaction atomically, if any were queued.
        let tx_pending = result.transaction;
        // Separate the acted-upon note (is_new == false) from new child notes.
        // New notes are sorted topologically so parents are inserted before children —
        // this is required to satisfy the FK constraint when the parent itself is a new note.
        let all_pending: Vec<_> = tx_pending.pending_notes.into_values().collect();
        let (existing_updates, mut new_creates): (Vec<_>, Vec<_>) =
            all_pending.into_iter().partition(|p| !p.is_new);
        // Topological sort for new creates: a note whose parent_id is also a new note must
        // come after its parent. IDs of new notes collected for quick look-up.
        let new_ids: std::collections::HashSet<String> =
            new_creates.iter().map(|p| p.note_id.clone()).collect();
        let mut ordered_creates: Vec<_> = Vec::with_capacity(new_creates.len());
        let mut remaining = new_creates.len();
        let mut iters = 0usize;
        while !new_creates.is_empty() {
            iters += 1;
            if iters > new_creates.len() * new_creates.len() + 1 {
                // Cycle guard — should never happen in practice; break to avoid infinite loop.
                ordered_creates.extend(new_creates.drain(..));
                break;
            }
            let mut next = Vec::with_capacity(new_creates.len());
            for pending in new_creates.drain(..) {
                let parent_is_new = pending.parent_id.as_ref()
                    .map(|pid| new_ids.contains(pid.as_str()))
                    .unwrap_or(false);
                let parent_already_emitted = pending.parent_id.as_ref()
                    .map(|pid| ordered_creates.iter().any(|e: &crate::core::save_transaction::PendingNote| &e.note_id == pid))
                    .unwrap_or(true);
                if !parent_is_new || parent_already_emitted {
                    ordered_creates.push(pending);
                } else {
                    next.push(pending);
                }
            }
            new_creates = next;
            if new_creates.len() == remaining {
                // No progress — break to avoid infinite loop.
                ordered_creates.extend(new_creates.drain(..));
                break;
            }
            remaining = new_creates.len();
        }
        // Combine: existing updates first (or last — order doesn't matter between them and creates)
        // then topologically sorted creates.
        let pending_notes: Vec<_> = existing_updates.into_iter().chain(ordered_creates).collect();

        if !pending_notes.is_empty() {
            let now = chrono::Utc::now().timestamp();

            // Pre-advance HLC for each pending note before borrowing self.storage.
            // Creates need one timestamp; updates need one for title + one per field.
            let timestamps: Vec<(HlcTimestamp, Vec<HlcTimestamp>)> = pending_notes.iter()
                .map(|p| {
                    let main_ts = self.advance_hlc();
                    let field_tss: Vec<HlcTimestamp> = if p.is_new {
                        vec![]
                    } else {
                        p.effective_fields().keys().map(|_| self.advance_hlc()).collect()
                    };
                    (main_ts, field_tss)
                })
                .collect();
            let signing_key = self.signing_key.clone();

            let tx_db = self.storage.connection_mut().transaction()?;

            for (pending, (main_ts, field_tss)) in pending_notes.iter().zip(timestamps.iter()) {
                if pending.is_new {
                    // ── INSERT new note ──────────────────────────────────────────
                    let parent_id = pending.parent_id.as_deref().unwrap_or("");
                    let position: i32 = tx_db.query_row(
                        "SELECT COALESCE(MAX(position), -1) + 1 FROM notes WHERE parent_id = ?1",
                        rusqlite::params![parent_id],
                        |row| row.get(0),
                    )?;
                    let effective_fields = pending.effective_fields();
                    let fields_json = serde_json::to_string(&effective_fields)?;
                    let effective_title = pending.effective_title();

                    let schema_ver = self.script_registry.get_schema(&pending.schema)
                        .map(|s| s.version).unwrap_or(1);
                    tx_db.execute(
                        "INSERT INTO notes (id, title, schema, parent_id, position, \
                                            created_at, modified_at, created_by, modified_by, \
                                            fields_json, is_expanded, schema_version) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                        rusqlite::params![
                            pending.note_id, effective_title, pending.schema,
                            parent_id, position, now, now,
                            self.current_identity_pubkey.clone(), self.current_identity_pubkey.clone(), fields_json, true,
                            schema_ver,
                        ],
                    )?;

                    Self::save_hlc(main_ts, &tx_db)?;
                    let mut op = Operation::CreateNote {
                        operation_id: Uuid::new_v4().to_string(),
                        timestamp: *main_ts,
                        device_id: self.device_id.clone(),
                        note_id: pending.note_id.clone(),
                        parent_id: Some(parent_id.to_string()),
                        position: position as f64,
                        schema: pending.schema.clone(),
                        title: effective_title.to_string(),
                        fields: effective_fields,
                        created_by: String::new(),
                        signature: String::new(),
                    };
                    Self::sign_op_with(&signing_key, &mut op);
                    Self::log_op(&self.operation_log, &tx_db, &op)?;
                } else {
                    // ── UPDATE existing note ─────────────────────────────────────
                    let effective_fields = pending.effective_fields();
                    let fields_json = serde_json::to_string(&effective_fields)?;
                    let effective_title = pending.effective_title();

                    tx_db.execute(
                        "UPDATE notes SET title = ?1, fields_json = ?2, \
                                          modified_at = ?3, modified_by = ?4 \
                         WHERE id = ?5",
                        rusqlite::params![
                            effective_title, fields_json, now,
                            self.current_identity_pubkey.clone(), pending.note_id,
                        ],
                    )?;

                    Self::save_hlc(main_ts, &tx_db)?;
                    let mut title_op = Operation::UpdateNote {
                        operation_id: Uuid::new_v4().to_string(),
                        timestamp: *main_ts,
                        device_id: self.device_id.clone(),
                        note_id: pending.note_id.clone(),
                        title: effective_title.to_string(),
                        modified_by: String::new(),
                        signature: String::new(),
                    };
                    Self::sign_op_with(&signing_key, &mut title_op);
                    Self::log_op(&self.operation_log, &tx_db, &title_op)?;

                    for ((field_key, field_value), field_ts) in effective_fields.iter().zip(field_tss.iter()) {
                        Self::save_hlc(field_ts, &tx_db)?;
                        let mut field_op = Operation::UpdateField {
                            operation_id: Uuid::new_v4().to_string(),
                            timestamp: *field_ts,
                            device_id: self.device_id.clone(),
                            note_id: pending.note_id.clone(),
                            field: field_key.clone(),
                            value: field_value.clone(),
                            modified_by: String::new(),
                            signature: String::new(),
                        };
                        Self::sign_op_with(&signing_key, &mut field_op);
                        Self::log_op(&self.operation_log, &tx_db, &field_op)?;
                    }
                }
            }

            Self::purge_ops_if_needed(&self.operation_log, &tx_db)?;
            tx_db.commit()?;
        }

        // ── reorder path (unchanged) ───────────────────────────────────────────
        if let Some(ids) = result.reorder {
            for (position, id) in ids.iter().enumerate() {
                self.move_note(id, Some(note_id), position as f64)?;
            }
        }

        Ok(())
    }

    /// Returns a map of `note_type → [action_label, …]` from the script registry.
    pub fn tree_action_map(&self) -> HashMap<String, Vec<String>> {
        self.script_registry.menu_action_map()
    }

    pub fn get_views_for_type(&self, schema_name: &str) -> Vec<crate::core::scripting::ViewRegistration> {
        self.script_registry.get_views_for_type(schema_name)
    }

    pub fn get_script_warnings(&self) -> Vec<crate::core::scripting::ScriptWarning> {
        self.script_registry.get_script_warnings()
    }

    /// Renders a specific registered view tab for a note.
    pub fn render_view(&self, note_id: &str, view_label: &str) -> Result<String> {
        let note = self.get_note(note_id)?;
        let context = self.build_query_context()?;

        let attachments = self.get_attachments(&note.id).unwrap_or_default();
        self.script_registry.set_run_context(note.clone(), attachments);
        struct RunContextGuard<'a>(&'a crate::core::scripting::ScriptRegistry);
        impl Drop for RunContextGuard<'_> {
            fn drop(&mut self) { self.0.clear_run_context(); }
        }
        let _guard = RunContextGuard(&self.script_registry);

        self.script_registry
            .run_view(&note, view_label, context)
            .map(|html| self.embed_attachment_images(html))
    }

    // Note: toggle_note_expansion and set_selected_note intentionally do NOT write to the
    // operation log. These are transient UI state (not document mutations) and should not
    // participate in sync or undo. They are stored in workspace_meta / the notes table but
    // treated as per-device view state, not collaborative operations.
    /// Toggles the `is_expanded` flag of `note_id` in the database.
    ///
    /// This is a UI-state mutation and is intentionally excluded from the
    /// operation log — expansion state is per-device and should not sync.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] if the note is not found.
    pub fn toggle_note_expansion(&mut self, note_id: &str) -> Result<()> {
        let tx = self.storage.connection_mut().transaction()?;

        // Get current value
        let current: i64 = tx.query_row(
            "SELECT is_expanded FROM notes WHERE id = ?",
            [note_id],
            |row| row.get(0)
        )?;

        // Toggle
        let new_value = if current == 1 { 0 } else { 1 };

        tx.execute(
            "UPDATE notes SET is_expanded = ? WHERE id = ?",
            rusqlite::params![new_value, note_id],
        )?;

        tx.commit()?;
        Ok(())
    }

    /// Persists the selected note ID to `workspace_meta`.
    ///
    /// Pass `None` to clear the selection. Like expansion state, selection is
    /// per-device UI state and is not written to the operation log.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] for any SQLite failure.
    pub fn set_selected_note(&mut self, note_id: Option<&str>) -> Result<()> {
        let tx = self.storage.connection_mut().transaction()?;

        // Delete existing entry
        tx.execute(
            "DELETE FROM workspace_meta WHERE key = 'selected_note_id'",
            [],
        )?;

        // Insert new value if provided
        if let Some(id) = note_id {
            tx.execute(
                "INSERT INTO workspace_meta (key, value) VALUES ('selected_note_id', ?)",
                [id],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    /// Returns the persisted selected note ID, or `None` if no selection is stored.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] for any SQLite error other
    /// than "no rows returned".
    pub fn get_selected_note(&self) -> Result<Option<String>> {
        let result = self.storage.connection().query_row(
            "SELECT value FROM workspace_meta WHERE key = 'selected_note_id'",
            [],
            |row| row.get::<_, String>(0)
        );

        match result {
            Ok(id) => Ok(Some(id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Returns the workspace-level metadata (author, license, description, etc.).
    ///
    /// Returns a default (all-empty) [`WorkspaceMetadata`] when no metadata has been
    /// stored yet, so callers can always treat the result as present.
    pub fn get_workspace_metadata(&self) -> Result<WorkspaceMetadata> {
        let result = self.storage.connection().query_row(
            "SELECT value FROM workspace_meta WHERE key = 'workspace_metadata'",
            [],
            |row| row.get::<_, String>(0),
        );
        match result {
            Ok(json) => Ok(serde_json::from_str(&json).unwrap_or_default()),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(WorkspaceMetadata::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Persists workspace-level metadata (author, license, description, etc.).
    pub fn set_workspace_metadata(&mut self, metadata: &WorkspaceMetadata) -> Result<()> {
        let json = serde_json::to_string(metadata).map_err(|e| {
            rusqlite::Error::ToSqlConversionFailure(Box::new(e))
        })?;
        self.storage.connection().execute(
            "INSERT OR REPLACE INTO workspace_meta (key, value) VALUES ('workspace_metadata', ?)",
            [&json],
        )?;
        Ok(())
    }

    /// Moves a note to a new parent and/or position within the tree.
    ///
    /// The move is performed inside a single SQLite transaction. Positions in
    /// the old sibling group are closed (decremented) and positions in the new
    /// sibling group are opened (incremented) before the note itself is
    /// relocated. A `MoveNote` operation is logged for sync/undo.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError::InvalidMove`] if the move would make a note
    /// its own parent or create an ancestor cycle. Returns
    /// [`KrillnotesError::NoteNotFound`] if `note_id` does not exist. Returns
    /// [`KrillnotesError::Database`] for any SQLite failure.
    pub fn move_note(
        &mut self,
        note_id: &str,
        new_parent_id: Option<&str>,
        new_position: f64,
    ) -> Result<()> {
        // 1. Self-move check
        if new_parent_id == Some(note_id) {
            return Err(KrillnotesError::InvalidMove(
                "A note cannot be its own parent".to_string(),
            ));
        }

        // 2. Cycle check: walk ancestor chain of new_parent_id
        if let Some(target_parent) = new_parent_id {
            let mut current = target_parent.to_string();
            loop {
                let parent: Option<String> = self
                    .connection()
                    .query_row(
                        "SELECT parent_id FROM notes WHERE id = ?",
                        [&current],
                        |row| row.get(0),
                    )
                    .map_err(|_| {
                        KrillnotesError::NoteNotFound(current.clone())
                    })?;
                match parent {
                    Some(pid) => {
                        if pid == note_id {
                            return Err(KrillnotesError::InvalidMove(
                                "Move would create a cycle".to_string(),
                            ));
                        }
                        current = pid;
                    }
                    None => break,
                }
            }
        }

        // 3. Allowed-parent-schemas check
        let note_to_move = self.get_note(note_id)?;
        let schema = self.script_registry.get_schema(&note_to_move.schema)?;
        if !schema.allowed_parent_schemas.is_empty() {
            match new_parent_id {
                None => return Err(KrillnotesError::InvalidMove(format!(
                    "Note type '{}' cannot be placed at root level", note_to_move.schema
                ))),
                Some(pid) => {
                    let parent_note = self.get_note(pid)?;
                    if !schema.allowed_parent_schemas.contains(&parent_note.schema) {
                        return Err(KrillnotesError::InvalidMove(format!(
                            "Note type '{}' cannot be placed under '{}'",
                            note_to_move.schema, parent_note.schema
                        )));
                    }
                }
            }
        }

        // 3b. Allowed-children-schemas check on the new parent
        if let Some(pid) = new_parent_id {
            let parent_note = self.get_note(pid)?;
            let parent_schema = self.script_registry.get_schema(&parent_note.schema)?;
            if parent_schema.is_leaf {
                return Err(KrillnotesError::InvalidMove(format!(
                    "Cannot add children to a leaf note (schema: '{}')",
                    parent_note.schema
                )));
            }
            if !parent_schema.allowed_children_schemas.is_empty()
                && !parent_schema.allowed_children_schemas.contains(&note_to_move.schema)
            {
                return Err(KrillnotesError::InvalidMove(format!(
                    "Note type '{}' is not allowed as a child of '{}'",
                    note_to_move.schema, parent_note.schema
                )));
            }
        }

        // Fetch the new parent note before opening the transaction (avoids borrow conflict with `tx`).
        let hook_new_parent = if let Some(pid) = new_parent_id {
            Some(self.get_note(pid)?)
        } else {
            None
        };

        // 4. Get the note's current parent_id and position
        let note = self.get_note(note_id)?;
        let old_parent_id = note.parent_id.clone();
        let old_position = note.position;

        let now = chrono::Utc::now().timestamp();
        let ts = self.advance_hlc();
        let signing_key = self.signing_key.clone();
        let tx = self.storage.connection_mut().transaction()?;

        // 5. Close the gap in the old sibling group
        // Exclude the note itself: during a same-parent move it still occupies
        // old_position in the DB until step 7.
        tx.execute(
            "UPDATE notes SET position = position - 1 WHERE parent_id IS ? AND position > ? AND id != ?",
            rusqlite::params![old_parent_id, old_position, note_id],
        )?;

        // 6. Open a gap in the new sibling group
        tx.execute(
            "UPDATE notes SET position = position + 1 WHERE parent_id IS ? AND position >= ? AND id != ?",
            rusqlite::params![new_parent_id, new_position, note_id],
        )?;

        // 7. Update the note itself
        tx.execute(
            "UPDATE notes SET parent_id = ?, position = ?, modified_at = ? WHERE id = ?",
            rusqlite::params![new_parent_id, new_position, now, note_id],
        )?;

        // Run on_add_child hook if the new parent's schema defines one.
        if let Some(ref parent_note) = hook_new_parent {
            if let Some(hook_result) = self.script_registry.run_on_add_child_hook(
                &parent_note.schema,
                &parent_note.id, &parent_note.schema, &parent_note.title, &parent_note.fields,
                &note_to_move.id, &note_to_move.schema, &note_to_move.title, &note_to_move.fields,
            )? {
                let hook_now = chrono::Utc::now().timestamp();
                if let Some((new_title, new_fields)) = hook_result.child {
                    let fields_json = serde_json::to_string(&new_fields)?;
                    tx.execute(
                        "UPDATE notes SET title = ?1, fields_json = ?2, modified_at = ?3 WHERE id = ?4",
                        rusqlite::params![new_title, fields_json, hook_now, note_to_move.id],
                    )?;
                }
                if let Some((new_title, new_fields)) = hook_result.parent {
                    let fields_json = serde_json::to_string(&new_fields)?;
                    tx.execute(
                        "UPDATE notes SET title = ?1, fields_json = ?2, modified_at = ?3 WHERE id = ?4",
                        rusqlite::params![new_title, fields_json, hook_now, parent_note.id],
                    )?;
                }
            }
        }

        // 8. Log a MoveNote operation
        Self::save_hlc(&ts, &tx)?;
        let mut op = Operation::MoveNote {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: ts,
            device_id: self.device_id.clone(),
            note_id: note_id.to_string(),
            new_parent_id: new_parent_id.map(|s| s.to_string()),
            new_position,
            moved_by: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&signing_key, &mut op);
        Self::log_op(&self.operation_log, &tx, &op)?;
        Self::purge_ops_if_needed(&self.operation_log, &tx)?;

        // 9. Commit
        tx.commit()?;

        // Push undo entry — inverse of MoveNote is PositionRestore.
        let op_id = op.operation_id().to_string();
        self.push_undo(UndoEntry {
            retracted_ids: vec![op_id],
            inverse: RetractInverse::PositionRestore {
                note_id: note_id.to_string(),
                old_parent_id,
                old_position,
            },
            propagate: true,
        });

        Ok(())
    }

    /// Returns the direct children of `parent_id` as a [`Vec<Note>`], ordered
    /// by `position`.
    ///
    /// Only immediate children are returned; grandchildren and deeper
    /// descendants are not included.
    ///
    /// # Errors
    ///
    /// Returns [`KrillnotesError`] if the database query fails.
    pub fn get_children(&self, parent_id: &str) -> Result<Vec<Note>> {
        let mut stmt = self.connection().prepare(
            "SELECT n.id, n.title, n.schema, n.parent_id, n.position,
                    n.created_at, n.modified_at, n.created_by, n.modified_by,
                    n.fields_json, n.is_expanded, n.schema_version,
                    GROUP_CONCAT(nt.tag, ',') AS tags_csv
             FROM notes n
             LEFT JOIN note_tags nt ON nt.note_id = n.id
             WHERE n.parent_id = ?1
             GROUP BY n.id
             ORDER BY n.position",
        )?;

        let rows = stmt
            .query_map(rusqlite::params![parent_id], map_note_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        rows.into_iter().map(note_from_row_tuple).collect()
    }

    /// Deletes `note_id` and all of its descendants recursively.
    ///
    /// The entire subtree rooted at `note_id` is removed within a single
    /// SQLite transaction, so a mid-subtree failure leaves the database
    /// unchanged. Every note in the subtree is deleted from the `notes`
    /// table; no re-parenting occurs. The returned [`DeleteResult`] reports
    /// the total count of removed notes and every deleted ID.
    ///
    /// This operation is intentionally excluded from the operation log:
    /// destructive bulk deletes are not currently part of the collaborative
    /// sync model and would require tombstone handling to be safe.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] if any SQLite operation
    /// fails, including when `note_id` does not exist (the DELETE silently
    /// affects zero rows, but child queries will return empty results rather
    /// than errors in that case). The transaction is rolled back automatically
    /// on any failure.
    pub fn delete_note_recursive(&mut self, note_id: &str) -> Result<DeleteResult> {
        // Capture full subtree for undo before any deletion.
        let subtree_notes = self.collect_subtree_notes(note_id)?;
        let subtree_ids: Vec<&str> = subtree_notes.iter().map(|n| n.id.as_str()).collect();
        let attachments = self.list_all_attachments()
            .unwrap_or_default()
            .into_iter()
            .filter(|a| subtree_ids.contains(&a.note_id.as_str()))
            .collect::<Vec<_>>();

        // Generate a stable operation ID before the deletion transaction.
        let op_id = Uuid::new_v4().to_string();

        // Collect all IDs in the subtree that will be deleted, then clear any
        // incoming NoteLink fields from other notes before the deletion transaction
        // opens (satisfies the note_links.target_id ON DELETE RESTRICT constraint).
        let all_ids = self.collect_subtree_ids(note_id)?;
        for id in &all_ids {
            self.clear_links_to(id)?;
        }

        let tx = self.storage.connection_mut().transaction()?;
        let result = Self::delete_recursive_in_tx(&tx, note_id)?;
        tx.commit()?;

        // Log a DeleteNote operation for the root of the deleted subtree.
        // Uses a separate transaction since the deletion tx was already committed.
        // Advance HLC and capture signing key before the second transaction borrows self.storage.
        let ts = self.advance_hlc();
        let signing_key = self.signing_key.clone();
        {
            let tx = self.storage.connection_mut().transaction()?;
            Self::save_hlc(&ts, &tx)?;
            let mut op = Operation::DeleteNote {
                operation_id: op_id.clone(),
                timestamp: ts,
                device_id: self.device_id.clone(),
                note_id: note_id.to_string(),
                deleted_by: String::new(),
                signature: String::new(),
            };
            Self::sign_op_with(&signing_key, &mut op);
            Self::log_op(&self.operation_log, &tx, &op)?;
            Self::purge_ops_if_needed(&self.operation_log, &tx)?;
            tx.commit()?;
        }

        self.push_undo(UndoEntry {
            retracted_ids: vec![op_id],
            inverse: RetractInverse::SubtreeRestore { notes: subtree_notes, attachments },
            propagate: true,
        });

        Ok(result)
    }

    /// Recursively deletes `note_id` and all descendants within an existing transaction.
    ///
    /// Only child IDs are fetched (not full `Note` structs) to keep the query
    /// minimal. Deletion proceeds depth-first: children are removed before
    /// their parent so that any future foreign-key constraint can be satisfied.
    ///
    /// This helper must not open its own transaction; callers are responsible
    /// for wrapping the call in a transaction, as SQLite does not support
    /// nested transactions.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] for any SQLite failure.
    fn delete_recursive_in_tx(
        tx: &rusqlite::Transaction,
        note_id: &str,
    ) -> Result<DeleteResult> {
        let mut affected_ids = vec![note_id.to_string()];

        // Fetch only the IDs of direct children — avoids deserialising full
        // Note structs and keeps the recursive helper lightweight.
        let mut stmt = tx.prepare("SELECT id FROM notes WHERE parent_id = ?1")?;
        let child_ids: Vec<String> = stmt
            .query_map(rusqlite::params![note_id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        // Recurse into children before deleting this node (leaves-first order).
        for child_id in child_ids {
            let child_result = Self::delete_recursive_in_tx(tx, &child_id)?;
            affected_ids.extend(child_result.affected_ids);
        }

        // Delete this note after all descendants have been removed.
        tx.execute(
            "DELETE FROM notes WHERE id = ?1",
            rusqlite::params![note_id],
        )?;

        // Detect nonexistent root IDs: SQLite DELETE silently affects zero rows
        // when the ID does not exist. Surface this as NoteNotFound.
        if tx.changes() == 0 {
            return Err(KrillnotesError::NoteNotFound(note_id.to_string()));
        }

        Ok(DeleteResult {
            deleted_count: affected_ids.len(),
            affected_ids,
        })
    }

    /// Deletes `note_id` and promotes its children to its grandparent.
    ///
    /// The note identified by `note_id` is removed from the `notes` table while
    /// all of its direct children are re-parented to the deleted note's own
    /// parent. Children of children (grandchildren of the deleted note) are not
    /// affected — they retain their existing parent. The entire operation runs
    /// inside a single SQLite transaction, so any failure leaves the database
    /// unchanged.
    ///
    /// The returned [`DeleteResult`] always has `deleted_count == 1` and
    /// `affected_ids` containing only `note_id`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::NoteNotFound`] if no note with
    /// `note_id` exists in the database. Returns
    /// [`crate::KrillnotesError::Database`] for any other SQLite failure.
    /// The transaction is rolled back automatically on any failure.
    pub fn delete_note_promote(&mut self, note_id: &str) -> Result<DeleteResult> {
        // Capture before-state for undo before any mutations.
        // Map Database error (QueryReturnedNoRows) to NoteNotFound for a missing ID.
        let deleted_note = self.get_note(note_id)
            .map_err(|_| KrillnotesError::NoteNotFound(note_id.to_string()))?;
        let children = self.get_children(note_id)?;
        let deleted_attachments = self.get_attachments(note_id).unwrap_or_default();

        // Generate a stable operation ID before the deletion transaction.
        let op_id = Uuid::new_v4().to_string();

        // Clear incoming NoteLink fields from other notes before opening the
        // deletion transaction (satisfies note_links.target_id ON DELETE RESTRICT).
        self.clear_links_to(note_id)?;

        // Advance HLC and capture signing key before the transaction borrows self.storage.
        let ts = self.advance_hlc();
        let signing_key = self.signing_key.clone();

        let tx = self.storage.connection_mut().transaction()?;

        // Fetch the note's parent — surfaces NoteNotFound for missing IDs.
        let parent_id: Option<String> = tx
            .query_row(
                "SELECT parent_id FROM notes WHERE id = ?1",
                rusqlite::params![note_id],
                |row| row.get(0),
            )
            .map_err(|_| KrillnotesError::NoteNotFound(note_id.to_string()))?;

        // Re-parent all direct children to the grandparent (may be NULL).
        tx.execute(
            "UPDATE notes SET parent_id = ?1 WHERE parent_id = ?2",
            rusqlite::params![parent_id, note_id],
        )?;

        // Renumber all children of the new parent to avoid position collisions
        let child_ids: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT id FROM notes WHERE parent_id IS ?1 ORDER BY position, id",
            )?;
            let ids = stmt.query_map(rusqlite::params![parent_id], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<_>>()?;
            ids
        };
        for (position, id) in child_ids.iter().enumerate() {
            tx.execute(
                "UPDATE notes SET position = ?1 WHERE id = ?2",
                rusqlite::params![position as i64, id],
            )?;
        }

        // Delete the note itself after its children have been safely re-parented.
        tx.execute(
            "DELETE FROM notes WHERE id = ?1",
            rusqlite::params![note_id],
        )?;

        // Log a DeleteNote operation for the promoted note.
        Self::save_hlc(&ts, &tx)?;
        let mut op = Operation::DeleteNote {
            operation_id: op_id.clone(),
            timestamp: ts,
            device_id: self.device_id.clone(),
            note_id: note_id.to_string(),
            deleted_by: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&signing_key, &mut op);
        Self::log_op(&self.operation_log, &tx, &op)?;
        Self::purge_ops_if_needed(&self.operation_log, &tx)?;

        tx.commit()?;

        // Build the Batch undo entry.
        //
        // `apply_retract_inverse_internal` applies Batch items with `.iter().rev()`
        // (LIFO), so the last item pushed is applied first.
        //
        // Required execution order on undo:
        //   1. SubtreeRestore — recreates the deleted note (must exist before
        //      children can point to it).
        //   2. PositionRestore for each child — moves them back to point at the
        //      restored note (each child's old_parent_id was note_id).
        //
        // To achieve that with LIFO: push PositionRestores FIRST, SubtreeRestore LAST.
        let mut batch_items: Vec<RetractInverse> = Vec::new();
        for child in &children {
            batch_items.push(RetractInverse::PositionRestore {
                note_id: child.id.clone(),
                old_parent_id: Some(deleted_note.id.clone()),
                old_position: child.position,
            });
        }
        batch_items.push(RetractInverse::SubtreeRestore {
            notes: vec![deleted_note.clone()],
            attachments: deleted_attachments,
        });
        self.push_undo(UndoEntry {
            retracted_ids: vec![op_id],
            inverse: RetractInverse::Batch(batch_items),
            propagate: true,
        });

        Ok(DeleteResult {
            deleted_count: 1,
            affected_ids: vec![note_id.to_string()],
        })
    }

    /// Deletes `note_id` using the specified [`DeleteStrategy`].
    ///
    /// This is the single public entry-point for note deletion. It dispatches
    /// to one of two internal methods:
    ///
    /// - [`DeleteStrategy::DeleteAll`] — calls [`Self::delete_note_recursive`],
    ///   which removes the note and every descendant in a single atomic
    ///   transaction.
    /// - [`DeleteStrategy::PromoteChildren`] — calls [`Self::delete_note_promote`],
    ///   which removes only the note itself and re-parents its direct children
    ///   to the deleted note's former parent.
    ///
    /// The returned [`DeleteResult`] reports the total count of deleted notes
    /// and the IDs of every affected note.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::NoteNotFound`] (for `PromoteChildren`)
    /// or [`crate::KrillnotesError::Database`] (for either strategy) if the
    /// underlying operation fails. All database mutations are transactional;
    /// a failure leaves the workspace unchanged.
    pub fn delete_note(
        &mut self,
        note_id: &str,
        strategy: DeleteStrategy,
    ) -> Result<DeleteResult> {
        match strategy {
            DeleteStrategy::DeleteAll => self.delete_note_recursive(note_id),
            DeleteStrategy::PromoteChildren => self.delete_note_promote(note_id),
        }
    }

    /// Returns the number of direct children of `note_id`.
    ///
    /// Counts rows in the `notes` table whose `parent_id` equals `note_id`.
    /// Grandchildren and deeper descendants are not included.
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] for any SQLite failure,
    /// including when `note_id` does not exist (the count will be zero in
    /// that case rather than an error, but connection failures are surfaced).
    pub fn count_children(&self, note_id: &str) -> Result<usize> {
        let count: i64 = self.storage.connection().query_row(
            "SELECT COUNT(*) FROM notes WHERE parent_id = ?1",
            rusqlite::params![note_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Updates the `title` and `fields` of an existing note, refreshing `modified_at`.
    ///
    /// Both the title and the full fields map are replaced atomically within a
    /// single SQLite transaction. The `modified_at` timestamp is set to the
    /// current UTC second and `modified_by` is set to the active user ID.
    ///
    /// # Errors
    ///
    /// Full 7-step save pipeline with validation:
    ///
    /// 1. Evaluate group visibility
    /// 2. Run field `validate` closures (only on visible fields)
    /// 3. Check required constraints (only on visible fields)
    /// 4-7. Delegate to `update_note` (on_save hook + DB write)
    ///
    /// Returns `SaveResult::ValidationErrors` when any step produces errors.
    /// Returns `SaveResult::Ok(note)` on success.
    pub fn save_note_with_pipeline(
        &mut self,
        note_id: &str,
        title: String,
        fields: BTreeMap<String, FieldValue>,
    ) -> Result<SaveResult> {
        let note = self.get_note(note_id)
            .map_err(|_| KrillnotesError::NoteNotFound(note_id.to_string()))?;
        let schema = self.script_registry.get_schema(&note.schema)?;

        // Step 1: Evaluate group visibility.
        let visibility = self.script_registry.evaluate_group_visibility(
            &note.schema, &fields,
        )?;

        // Collect visible field names (top-level + fields from visible groups).
        let visible_field_names: std::collections::HashSet<String> = schema.fields.iter()
            .map(|f| f.name.clone())
            .chain(
                schema.field_groups.iter()
                    .filter(|g| visibility.get(&g.name).copied().unwrap_or(true))
                    .flat_map(|g| g.fields.iter().map(|f| f.name.clone()))
            )
            .collect();

        // Step 2: Run validate closures on visible fields.
        let all_errors = self.script_registry.validate_fields(&note.schema, &fields)?;
        let mut field_errors: BTreeMap<String, String> = all_errors.into_iter()
            .filter(|(k, _)| visible_field_names.contains(k))
            .collect();

        // Step 3: Required check on visible required fields.
        for field_def in schema.all_fields() {
            if field_def.required && visible_field_names.contains(&field_def.name) {
                let empty = match fields.get(&field_def.name) {
                    None => true,
                    Some(FieldValue::Text(s))   => s.is_empty(),
                    Some(FieldValue::Email(s))  => s.is_empty(),
                    Some(FieldValue::Date(None))
                    | Some(FieldValue::NoteLink(None))
                    | Some(FieldValue::File(None)) => true,
                    _ => false,
                };
                if empty && !field_errors.contains_key(&field_def.name) {
                    field_errors.insert(field_def.name.clone(), "Required".to_string());
                }
            }
        }

        if !field_errors.is_empty() {
            return Ok(SaveResult::ValidationErrors {
                field_errors,
                note_errors: vec![],
                preview_title: None,
                preview_fields: BTreeMap::new(),
            });
        }

        // Build final_fields: start from schema defaults, overlay existing note values,
        // then apply user-provided visible-field values. Hidden-group required fields
        // retain their existing/default values so update_note's validate_required_fields
        // doesn't reject them (that check is visibility-unaware).
        let mut final_fields = schema.default_fields();
        for (k, v) in &note.fields {
            final_fields.insert(k.clone(), v.clone());
        }
        for (k, v) in &fields {
            final_fields.insert(k.clone(), v.clone());
        }

        // Steps 4-7: update_note (runs on_save hook + writes to DB).
        match self.update_note(note_id, title, final_fields) {
            Ok(updated) => Ok(SaveResult::Ok(updated)),
            Err(KrillnotesError::ValidationFailed(msg)) => {
                Ok(SaveResult::ValidationErrors {
                    field_errors: BTreeMap::new(),
                    note_errors: vec![msg],
                    preview_title: None,
                    preview_fields: BTreeMap::new(),
                })
            }
            Err(e) => Err(e),
        }
    }

    /// Returns [`crate::KrillnotesError::NoteNotFound`] if no note with `note_id`
    /// exists in the database.  Returns [`crate::KrillnotesError::Json`] if
    /// `fields` cannot be serialised to JSON.  Returns
    /// [`crate::KrillnotesError::Database`] for any other SQLite failure.
    pub fn update_note(
        &mut self,
        note_id: &str,
        title: String,
        fields: BTreeMap<String, FieldValue>,
    ) -> Result<Note> {
        // Capture before-state for undo.
        // Map Database errors (e.g. QueryReturnedNoRows) to NoteNotFound so that
        // callers see a consistent error type when the note does not exist.
        let old_note = self.get_note(note_id)
            .map_err(|_| KrillnotesError::NoteNotFound(note_id.to_string()))?;

        // Look up this note's schema so the pre-save hook can be dispatched.
        let note_schema: String = self
            .storage
            .connection()
            .query_row(
                "SELECT schema FROM notes WHERE id = ?1",
                rusqlite::params![note_id],
                |row| row.get(0),
            )
            .map_err(|_| KrillnotesError::NoteNotFound(note_id.to_string()))?;

        // Run the pre-save hook via the gated SaveTransaction model.
        // - hook not registered         → no-op (keep passed-in title/fields)
        // - hook called commit()        → apply effective_title / effective_fields
        // - hook called reject(…)       → return ValidationFailed error
        // - hook returned Map (old API) → hard Scripting error with migration message
        let (title, fields) =
            match self
                .script_registry
                .run_on_save_hook(&note_schema, note_id, &note_schema, &title, &fields)?
            {
                None => (title, fields),
                Some(tx) if tx.committed => {
                    let pn = tx.pending_notes.get(note_id)
                        .ok_or_else(|| KrillnotesError::Scripting(
                            format!("on_save hook committed but pending note '{}' not found", note_id)
                        ))?;
                    (pn.effective_title().to_string(), pn.effective_fields())
                }
                Some(tx) if tx.has_errors() => {
                    let msgs: Vec<String> = tx.soft_errors.iter().map(|e| {
                        match &e.field {
                            Some(f) => format!("{}: {}", f, e.message),
                            None => e.message.clone(),
                        }
                    }).collect();
                    return Err(KrillnotesError::ValidationFailed(msgs.join("; ")));
                }
                Some(_) => (title, fields),  // hook ran but didn't commit → no-op
            };

        // Enforce required-field constraints defined in the schema.
        let schema = self.script_registry.get_schema(&note_schema)?;
        schema.validate_required_fields(&fields)?;

        let now = chrono::Utc::now().timestamp();
        let fields_json = serde_json::to_string(&fields)?;

        // Clean up replaced or cleared File field attachments before the note UPDATE.
        // Must run before connection_mut() is borrowed for the transaction below,
        // since delete_attachment uses connection() (shared ref) which conflicts with
        // an active connection_mut() Transaction.
        //
        // Note: if delete_attachment succeeds but the tx.commit() below fails, the
        // note row still references old_uuid while the attachment is already gone,
        // leaving a dangling File field reference. This is an accepted trade-off
        // for a single-writer local store where commit failures are rare.
        {
            let old_fields_json: String = self
                .storage
                .connection()
                .query_row(
                    "SELECT fields_json FROM notes WHERE id = ?1",
                    rusqlite::params![note_id],
                    |row| row.get(0),
                )
                .map_err(|_| KrillnotesError::NoteNotFound(note_id.to_string()))?;
            let old_fields: BTreeMap<String, FieldValue> =
                serde_json::from_str(&old_fields_json).unwrap_or_default();

            for (key, old_val) in &old_fields {
                if let FieldValue::File(Some(old_uuid)) = old_val {
                    let still_same = matches!(
                        fields.get(key),
                        Some(FieldValue::File(Some(u))) if u == old_uuid
                    );
                    if !still_same {
                        let _ = self.delete_attachment(old_uuid); // best-effort
                    }
                }
            }
        }

        // Collector for all operation IDs emitted during this update,
        // used to populate the undo entry's retracted_ids.
        let mut emitted_op_ids: Vec<String> = Vec::new();

        // Pre-advance HLC for title op + one per field, and capture signing key,
        // before the transaction borrows self.storage mutably.
        let title_ts = self.advance_hlc();
        let field_timestamps: Vec<HlcTimestamp> = fields.keys()
            .map(|_| self.advance_hlc())
            .collect();
        let signing_key = self.signing_key.clone();

        let tx = self.storage.connection_mut().transaction()?;

        let current_schema_version = self.script_registry
            .get_schema(&note_schema)
            .map(|s| s.version)
            .unwrap_or(1);
        tx.execute(
            "UPDATE notes SET title = ?1, fields_json = ?2, modified_at = ?3, modified_by = ?4, schema_version = ?5 WHERE id = ?6",
            rusqlite::params![title, fields_json, now, self.current_identity_pubkey.clone(), current_schema_version, note_id],
        )?;

        // Detect nonexistent IDs: SQLite UPDATE on a missing row succeeds but
        // touches zero rows. Surface this as NoteNotFound rather than silently
        // returning stale data.
        if tx.changes() == 0 {
            return Err(KrillnotesError::NoteNotFound(note_id.to_string()));
        }

        // Log an UpdateNote operation for the title, consistent with
        // update_note_title.
        Self::save_hlc(&title_ts, &tx)?;
        let title_op_id = Uuid::new_v4().to_string();
        emitted_op_ids.push(title_op_id.clone());
        let mut title_op = Operation::UpdateNote {
            operation_id: title_op_id,
            timestamp: title_ts,
            device_id: self.device_id.clone(),
            note_id: note_id.to_string(),
            title: title.clone(),
            modified_by: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&signing_key, &mut title_op);
        Self::log_op(&self.operation_log, &tx, &title_op)?;

        // Log one UpdateField operation per field value that was written.
        for ((field_key, field_value), field_ts) in fields.iter().zip(field_timestamps.iter()) {
            Self::save_hlc(field_ts, &tx)?;
            let field_op_id = Uuid::new_v4().to_string();
            emitted_op_ids.push(field_op_id.clone());
            let mut field_op = Operation::UpdateField {
                operation_id: field_op_id,
                timestamp: *field_ts,
                device_id: self.device_id.clone(),
                note_id: note_id.to_string(),
                field: field_key.clone(),
                value: field_value.clone(),
                modified_by: String::new(),
                signature: String::new(),
            };
            Self::sign_op_with(&signing_key, &mut field_op);
            Self::log_op(&self.operation_log, &tx, &field_op)?;
        }

        Self::purge_ops_if_needed(&self.operation_log, &tx)?;

        // Keep the note_links junction table in sync with the written field values.
        // Must run inside the transaction so the link update is atomic with the note write.
        sync_note_links(&tx, note_id, &fields)?;

        tx.commit()?;

        // Push undo entry — inverse of UpdateNote is NoteRestore.
        // textarea fields use CRDT on peers; mark as non-propagating for v1.
        self.push_undo(UndoEntry {
            retracted_ids: emitted_op_ids,
            inverse: RetractInverse::NoteRestore {
                note_id: note_id.to_string(),
                old_title: old_note.title,
                old_fields: old_note.fields,
                old_tags: old_note.tags,
            },
            propagate: false,
        });

        // Re-use get_note to fetch the persisted row, keeping row-mapping logic
        // in a single place.
        self.get_note(note_id)
    }

    // ── User-script CRUD ──────────────────────────────────────────

    /// Returns all user scripts, ordered by `load_order` ascending.
    pub fn list_user_scripts(&self) -> Result<Vec<UserScript>> {
        let mut stmt = self.connection().prepare(
            "SELECT id, name, description, source_code, load_order, enabled, created_at, modified_at, category
             FROM user_scripts ORDER BY load_order ASC, created_at ASC",
        )?;
        let scripts = stmt
            .query_map([], |row| {
                Ok(UserScript {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    source_code: row.get(3)?,
                    load_order: row.get(4)?,
                    enabled: row.get::<_, i64>(5).map(|v| v != 0)?,
                    created_at: row.get(6)?,
                    modified_at: row.get(7)?,
                    category: row.get::<_, String>(8).unwrap_or_else(|_| "presentation".to_string()),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(scripts)
    }

    /// Returns a single user script by ID.
    pub fn get_user_script(&self, script_id: &str) -> Result<UserScript> {
        self.connection()
            .query_row(
                "SELECT id, name, description, source_code, load_order, enabled, created_at, modified_at, category
                 FROM user_scripts WHERE id = ?",
                [script_id],
                |row| {
                    Ok(UserScript {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        description: row.get(2)?,
                        source_code: row.get(3)?,
                        load_order: row.get(4)?,
                        enabled: row.get::<_, i64>(5).map(|v| v != 0)?,
                        created_at: row.get(6)?,
                        modified_at: row.get(7)?,
                        category: row.get::<_, String>(8).unwrap_or_else(|_| "presentation".to_string()),
                    })
                },
            )
            .map_err(|_| KrillnotesError::NoteNotFound(format!("User script {script_id} not found")))
    }

    /// Creates a new user script from its source code, parsing front matter for name/description.
    ///
    /// Returns an error if `@name` is missing from the front matter, or if Rhai
    /// compilation fails. On failure nothing is written to the database.
    pub fn create_user_script(&mut self, source_code: &str) -> Result<(UserScript, Vec<ScriptError>)> {
        // Auto-detect category: if the source calls schema(), it's a schema script.
        let category = if source_code.contains("schema(") { "schema" } else { "presentation" };
        self.create_user_script_with_category(source_code, category)
    }

    /// Creates a user script with an explicit category ("schema" or "presentation").
    pub fn create_user_script_with_category(
        &mut self,
        source_code: &str,
        category: &str,
    ) -> Result<(UserScript, Vec<ScriptError>)> {
        let fm = user_script::parse_front_matter(source_code);
        if fm.name.is_empty() {
            return Err(KrillnotesError::ValidationFailed(
                "Script must include a '// @name:' front matter line".to_string(),
            ));
        }

        let now = chrono::Utc::now().timestamp();
        let id = Uuid::new_v4().to_string();

        // Pre-validation
        self.script_registry.set_loading_category(Some(category.to_string()));
        if let Err(e) = self.script_registry.load_script(source_code, &fm.name) {
            let _ = self.reload_scripts();
            return Err(e);
        }

        let ts = self.advance_hlc();
        let signing_key = self.signing_key.clone();

        let tx = self.storage.connection_mut().transaction()?;

        let max_order: i32 = tx
            .query_row("SELECT COALESCE(MAX(load_order), -1) FROM user_scripts", [], |row| row.get(0))
            .unwrap_or(-1);
        let load_order = max_order + 1;

        tx.execute(
            "INSERT INTO user_scripts (id, name, description, source_code, load_order, enabled, created_at, modified_at, category)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![id, fm.name, fm.description, source_code, load_order, true, now, now, category],
        )?;

        Self::save_hlc(&ts, &tx)?;
        let mut op = Operation::CreateUserScript {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: ts,
            device_id: self.device_id.clone(),
            script_id: id.clone(),
            name: fm.name.clone(),
            description: fm.description.clone(),
            source_code: source_code.to_string(),
            load_order,
            enabled: true,
            created_by: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&signing_key, &mut op);
        Self::log_op(&self.operation_log, &tx, &op)?;
        Self::purge_ops_if_needed(&self.operation_log, &tx)?;

        tx.commit()?;

        let op_id = op.operation_id().to_string();
        self.push_script_undo(UndoEntry {
            retracted_ids: vec![op_id],
            inverse: RetractInverse::DeleteScript { script_id: id.clone() },
            propagate: true,
        });

        let errors = self.reload_scripts()?;
        let script = self.get_user_script(&id)?;
        Ok((script, errors))
    }

    /// Updates an existing user script's source code, re-parsing front matter.
    ///
    /// Returns an error if `@name` is missing from the front matter, or if Rhai
    /// compilation fails. On failure nothing is written to the database.
    pub fn update_user_script(&mut self, script_id: &str, source_code: &str) -> Result<(UserScript, Vec<ScriptError>)> {
        let fm = user_script::parse_front_matter(source_code);
        if fm.name.is_empty() {
            return Err(KrillnotesError::ValidationFailed(
                "Script must include a '// @name:' front matter line".to_string(),
            ));
        }

        // Pre-validation: try to compile and evaluate the new source code.
        // The collision check allows same-script re-registration, so updating a script that
        // already owns some schemas will not falsely fire a collision error.
        // We must set the loading category so schema scripts get library sources prepended.
        let existing_category = self
            .get_user_script(script_id)
            .map(|s| s.category)
            .unwrap_or_else(|_| "presentation".to_string());
        self.script_registry.set_loading_category(Some(existing_category));
        if let Err(e) = self.script_registry.load_script(source_code, &fm.name) {
            let _ = self.reload_scripts(); // restore registry; ignore restoration errors
            return Err(e);
        }

        // Capture old script state BEFORE the update for undo.
        let old_script = self.get_user_script(script_id)?;

        let now = chrono::Utc::now().timestamp();
        // Advance HLC and capture signing key before the transaction borrows self.storage.
        let ts = self.advance_hlc();
        let signing_key = self.signing_key.clone();
        let tx = self.storage.connection_mut().transaction()?;

        let changes = tx.execute(
            "UPDATE user_scripts SET name = ?, description = ?, source_code = ?, modified_at = ? WHERE id = ?",
            rusqlite::params![fm.name, fm.description, source_code, now, script_id],
        )?;

        if changes == 0 {
            return Err(KrillnotesError::NoteNotFound(format!("User script {script_id} not found")));
        }

        // Read current full state for the operation log
        let (load_order, enabled): (i32, bool) = tx.query_row(
            "SELECT load_order, enabled FROM user_scripts WHERE id = ?",
            [script_id],
            |row| Ok((row.get(0)?, row.get::<_, i64>(1).map(|v| v != 0)?)),
        )?;

        // Log operation
        Self::save_hlc(&ts, &tx)?;
        let mut op = Operation::UpdateUserScript {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: ts,
            device_id: self.device_id.clone(),
            script_id: script_id.to_string(),
            name: fm.name.clone(),
            description: fm.description.clone(),
            source_code: source_code.to_string(),
            load_order,
            enabled,
            modified_by: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&signing_key, &mut op);
        Self::log_op(&self.operation_log, &tx, &op)?;
        Self::purge_ops_if_needed(&self.operation_log, &tx)?;

        tx.commit()?;

        // Push onto the script-specific undo stack (isolated from note undo).
        let op_id = op.operation_id().to_string();
        self.push_script_undo(UndoEntry {
            retracted_ids: vec![op_id],
            inverse: RetractInverse::ScriptRestore {
                script_id: script_id.to_string(),
                name: old_script.name,
                description: old_script.description,
                source_code: old_script.source_code,
                load_order: old_script.load_order,
                enabled: old_script.enabled,
            },
            propagate: true,
        });

        let errors = self.reload_scripts()?;
        let script = self.get_user_script(script_id)?;
        Ok((script, errors))
    }

    /// Deletes a user script by ID and reloads remaining scripts.
    pub fn delete_user_script(&mut self, script_id: &str) -> Result<Vec<ScriptError>> {
        // Capture old script state BEFORE deletion for undo.
        let old_script = self.get_user_script(script_id)?;

        let ts = self.advance_hlc();
        let signing_key = self.signing_key.clone();
        let tx = self.storage.connection_mut().transaction()?;

        tx.execute("DELETE FROM user_scripts WHERE id = ?", [script_id])?;

        // Log operation
        Self::save_hlc(&ts, &tx)?;
        let mut op = Operation::DeleteUserScript {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: ts,
            device_id: self.device_id.clone(),
            script_id: script_id.to_string(),
            deleted_by: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&signing_key, &mut op);
        Self::log_op(&self.operation_log, &tx, &op)?;
        Self::purge_ops_if_needed(&self.operation_log, &tx)?;

        tx.commit()?;

        // Push onto the script-specific undo stack (isolated from note undo).
        let op_id = op.operation_id().to_string();
        self.push_script_undo(UndoEntry {
            retracted_ids: vec![op_id],
            inverse: RetractInverse::ScriptRestore {
                script_id: script_id.to_string(),
                name: old_script.name,
                description: old_script.description,
                source_code: old_script.source_code,
                load_order: old_script.load_order,
                enabled: old_script.enabled,
            },
            propagate: true,
        });

        self.reload_scripts()
    }

    /// Toggles the enabled state of a user script and reloads.
    pub fn toggle_user_script(&mut self, script_id: &str, enabled: bool) -> Result<Vec<ScriptError>> {
        let ts = self.advance_hlc();
        let signing_key = self.signing_key.clone();
        let tx = self.storage.connection_mut().transaction()?;

        tx.execute(
            "UPDATE user_scripts SET enabled = ? WHERE id = ?",
            rusqlite::params![enabled, script_id],
        )?;

        // Read full current state for the operation log
        let (name, description, source_code, load_order): (String, String, String, i32) = tx.query_row(
            "SELECT name, description, source_code, load_order FROM user_scripts WHERE id = ?",
            [script_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;

        // Log operation
        Self::save_hlc(&ts, &tx)?;
        let mut op = Operation::UpdateUserScript {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: ts,
            device_id: self.device_id.clone(),
            script_id: script_id.to_string(),
            name,
            description,
            source_code,
            load_order,
            enabled,
            modified_by: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&signing_key, &mut op);
        Self::log_op(&self.operation_log, &tx, &op)?;
        Self::purge_ops_if_needed(&self.operation_log, &tx)?;

        tx.commit()?;

        self.reload_scripts()
    }

    /// Changes the load order of a user script and reloads.
    pub fn reorder_user_script(&mut self, script_id: &str, new_load_order: i32) -> Result<Vec<ScriptError>> {
        let ts = self.advance_hlc();
        let signing_key = self.signing_key.clone();
        let tx = self.storage.connection_mut().transaction()?;

        tx.execute(
            "UPDATE user_scripts SET load_order = ? WHERE id = ?",
            rusqlite::params![new_load_order, script_id],
        )?;

        // Read full current state for the operation log
        let (name, description, source_code, enabled): (String, String, String, bool) = tx.query_row(
            "SELECT name, description, source_code, enabled FROM user_scripts WHERE id = ?",
            [script_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get::<_, i64>(3).map(|v| v != 0)?)),
        )?;

        // Log operation
        Self::save_hlc(&ts, &tx)?;
        let mut op = Operation::UpdateUserScript {
            operation_id: Uuid::new_v4().to_string(),
            timestamp: ts,
            device_id: self.device_id.clone(),
            script_id: script_id.to_string(),
            name,
            description,
            source_code,
            load_order: new_load_order,
            enabled,
            modified_by: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&signing_key, &mut op);
        Self::log_op(&self.operation_log, &tx, &op)?;
        Self::purge_ops_if_needed(&self.operation_log, &tx)?;

        tx.commit()?;

        self.reload_scripts()
    }

    /// Re-assigns sequential load_order (0-based) to all scripts given in `ids` order, then reloads.
    pub fn reorder_all_user_scripts(&mut self, ids: &[String]) -> Result<Vec<ScriptError>> {
        // Bulk reorder is not logged to the operation log — it's a UI ordering gesture, not a sync-relevant change.
        {
            let conn = self.storage.connection_mut();
            let tx = conn.transaction()?;
            for (i, id) in ids.iter().enumerate() {
                tx.execute(
                    "UPDATE user_scripts SET load_order = ? WHERE id = ?",
                    rusqlite::params![i as i32, id],
                )?;
            }
            tx.commit()?;
        }
        self.reload_scripts()
    }

    // ── Operations log queries ───────────────────────────────────────

    /// Returns operation summaries matching the given filters, newest first.
    pub fn list_operations(
        &self,
        type_filter: Option<&str>,
        since: Option<i64>,
        until: Option<i64>,
    ) -> Result<Vec<crate::OperationSummary>> {
        self.operation_log.list(self.connection(), type_filter, since, until)
    }

    /// Returns the full JSON detail for a single operation by ID.
    pub fn get_operation_detail(&self, operation_id: &str) -> Result<serde_json::Value> {
        self.operation_log.get_detail(self.connection(), operation_id)
    }

    /// Deletes all operations from the log. Returns the number deleted.
    pub fn purge_all_operations(&self) -> Result<usize> {
        self.operation_log.purge_all(self.connection())
    }

    /// Clears all registered schemas/hooks and re-executes enabled scripts from the DB in order.
    ///
    /// Returns any errors that occurred during loading (e.g. schema collisions, Rhai errors).
    /// A failing script is skipped; subsequent scripts continue to load.
    fn reload_scripts(&mut self) -> Result<Vec<ScriptError>> {
        self.script_registry.clear_all();
        let scripts = self.list_user_scripts()?;
        Ok(self.load_scripts_two_phase(&scripts))
    }

    /// Two-phase script loading: presentation/library first, then schema, then resolve bindings.
    fn load_scripts_two_phase(&mut self, scripts: &[UserScript]) -> Vec<ScriptError> {
        let mut errors = Vec::new();

        // Phase A: load presentation/library scripts first
        for script in scripts.iter().filter(|s| s.enabled && s.category == "presentation") {
            self.script_registry.set_loading_category(Some("presentation".to_string()));
            if let Err(e) = self.script_registry.load_script(&script.source_code, &script.name) {
                errors.push(ScriptError {
                    script_name: script.name.clone(),
                    message: e.to_string(),
                });
            }
        }

        // Phase B: load schema scripts
        for script in scripts.iter().filter(|s| s.enabled && s.category == "schema") {
            self.script_registry.set_loading_category(Some("schema".to_string()));
            if let Err(e) = self.script_registry.load_script(&script.source_code, &script.name) {
                errors.push(ScriptError {
                    script_name: script.name.clone(),
                    message: e.to_string(),
                });
            }
        }

        // Phase C: resolve deferred bindings (views, hovers, menus)
        self.script_registry.resolve_bindings();

        errors
    }

    /// Public wrapper for reloading all scripts (e.g. from tests).
    pub fn reload_all_scripts(&mut self) -> Result<Vec<ScriptError>> {
        self.reload_scripts()
    }

    /// Collects the IDs of `note_id` and every descendant using a recursive CTE.
    ///
    /// Returns a flat `Vec<String>` containing the root ID plus all descendant
    /// IDs in an unspecified order.
    fn collect_subtree_ids(&self, note_id: &str) -> Result<Vec<String>> {
        let conn = self.connection();
        let mut stmt = conn.prepare(
            "WITH RECURSIVE subtree AS (
                SELECT id FROM notes WHERE id = ?1
                UNION ALL
                SELECT n.id FROM notes n JOIN subtree s ON n.parent_id = s.id
            )
            SELECT id FROM subtree",
        )?;
        let ids: Vec<String> = stmt
            .query_map([note_id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    /// Finds all notes that have a `NoteLink` field pointing to `target_id`,
    /// sets those fields to `NoteLink(None)` in `fields_json`, and removes
    /// the corresponding rows from the `note_links` junction table.
    ///
    /// This must be called BEFORE the target note is deleted so that the
    /// `note_links.target_id REFERENCES notes(id) ON DELETE RESTRICT`
    /// constraint is satisfied.
    ///
    /// All changes (field patches + junction-table delete) are committed in a
    /// single transaction. If no notes link to `target_id` the function
    /// returns immediately without touching the database.
    pub fn clear_links_to(&mut self, target_id: &str) -> Result<()> {
        // Find all notes linking to this target (read-only, uses shared ref).
        let links: Vec<(String, String)> = {
            let conn = self.connection();
            let mut stmt = conn.prepare(
                "SELECT source_id, field_name FROM note_links WHERE target_id = ?1",
            )?;
            let rows = stmt.query_map([target_id], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            rows
        };

        if links.is_empty() {
            return Ok(());
        }

        // For each linking note: load fields_json, patch the field to NoteLink(None), save back.
        let conn = self.storage.connection_mut();
        let tx = conn.transaction()?;
        for (source_id, field_name) in &links {
            let fields_json: String = tx.query_row(
                "SELECT fields_json FROM notes WHERE id = ?1",
                [source_id],
                |row| row.get(0),
            )?;
            let mut json_val: serde_json::Value = serde_json::from_str(&fields_json)?;
            if let Some(obj) = json_val.as_object_mut() {
                // NoteLink(None) serializes as {"NoteLink":null} under serde external tagging.
                obj.insert(field_name.clone(), serde_json::json!({"NoteLink": null}));
            }
            let updated_json = serde_json::to_string(&json_val)?;
            tx.execute(
                "UPDATE notes SET fields_json = ?1 WHERE id = ?2",
                [&updated_json, source_id],
            )?;
        }
        tx.execute("DELETE FROM note_links WHERE target_id = ?1", [target_id])?;
        tx.commit()?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Attachment methods
    // -------------------------------------------------------------------------

    /// Attaches a file to a note. Encrypts the bytes and writes them to
    /// `<workspace_root>/attachments/<uuid>.enc`, then inserts a DB metadata row.
    pub fn attach_file(
        &mut self,
        note_id: &str,
        filename: &str,
        mime_type: Option<&str>,
        data: &[u8],
    ) -> Result<AttachmentMeta> {
        // Enforce workspace size limit
        if let Some(limit) = self.attachment_max_size_bytes()? {
            if data.len() as u64 > limit {
                return Err(KrillnotesError::AttachmentTooLarge {
                    size: data.len() as u64,
                    limit,
                });
            }
        }

        // SHA-256 hash for integrity
        let hash = {
            let mut h = Sha256::new();
            h.update(data);
            format!("{:x}", h.finalize())
        };

        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();

        let (encrypted_bytes, file_salt) =
            encrypt_attachment(data, self.attachment_key.as_ref())?;

        // Write to disk
        let enc_path = self.workspace_root.join("attachments").join(format!("{id}.enc"));
        std::fs::write(&enc_path, &encrypted_bytes)?;

        // Insert DB row
        self.storage.connection().execute(
            "INSERT INTO attachments (id, note_id, filename, mime_type, size_bytes, hash_sha256, salt, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                id, note_id, filename, mime_type,
                data.len() as i64, hash, file_salt.as_slice(), now
            ],
        )?;

        Ok(AttachmentMeta {
            id,
            note_id: note_id.to_string(),
            filename: filename.to_string(),
            mime_type: mime_type.map(|s| s.to_string()),
            size_bytes: data.len() as i64,
            hash_sha256: hash,
            salt: hex::encode(file_salt),
            created_at: now,
        })
    }

    /// Import-only: attach a file with a pre-specified ID (preserves IDs from export).
    /// Does NOT enforce size limits (the size was already validated at export time).
    pub fn attach_file_with_id(
        &mut self,
        id: &str,
        note_id: &str,
        filename: &str,
        mime_type: Option<&str>,
        data: &[u8],
    ) -> Result<()> {
        let hash = {
            let mut h = Sha256::new();
            h.update(data);
            format!("{:x}", h.finalize())
        };
        let now = chrono::Utc::now().timestamp();
        let (encrypted_bytes, file_salt) = encrypt_attachment(data, self.attachment_key.as_ref())?;
        let enc_path = self.workspace_root.join("attachments").join(format!("{id}.enc"));
        std::fs::write(&enc_path, &encrypted_bytes)?;
        let _ = self.storage.connection().execute(
            "INSERT OR IGNORE INTO attachments (id, note_id, filename, mime_type, size_bytes, hash_sha256, salt, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![id, note_id, filename, mime_type, data.len() as i64, hash, file_salt.as_slice(), now],
        );
        Ok(())
    }

    /// Returns all attachment metadata for a note (no file I/O).
    pub fn get_attachments(&self, note_id: &str) -> Result<Vec<AttachmentMeta>> {
        let mut stmt = self.storage.connection().prepare(
            "SELECT id, note_id, filename, mime_type, size_bytes, hash_sha256, salt, created_at
             FROM attachments WHERE note_id = ? ORDER BY created_at ASC",
        )?;
        let results = stmt.query_map([note_id], |row| {
            let salt_bytes: Vec<u8> = row.get(6)?;
            Ok(AttachmentMeta {
                id: row.get(0)?,
                note_id: row.get(1)?,
                filename: row.get(2)?,
                mime_type: row.get(3)?,
                size_bytes: row.get(4)?,
                hash_sha256: row.get(5)?,
                salt: hex::encode(&salt_bytes),
                created_at: row.get(7)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(results)
    }

    /// Returns all attachments in the workspace (used for export).
    pub fn list_all_attachments(&self) -> Result<Vec<AttachmentMeta>> {
        let mut stmt = self.storage.connection().prepare(
            "SELECT id, note_id, filename, mime_type, size_bytes, hash_sha256, salt, created_at
             FROM attachments ORDER BY created_at ASC",
        )?;
        let results = stmt.query_map([], |row| {
            let salt_bytes: Vec<u8> = row.get(6)?;
            Ok(AttachmentMeta {
                id: row.get(0)?,
                note_id: row.get(1)?,
                filename: row.get(2)?,
                mime_type: row.get(3)?,
                size_bytes: row.get(4)?,
                hash_sha256: row.get(5)?,
                salt: hex::encode(&salt_bytes),
                created_at: row.get(7)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(results)
    }

    /// Decrypts and returns the plaintext bytes for an attachment.
    pub fn get_attachment_bytes(&self, attachment_id: &str) -> Result<Vec<u8>> {
        let (salt_bytes, _): (Vec<u8>, i64) = self.storage.connection().query_row(
            "SELECT salt, size_bytes FROM attachments WHERE id = ?",
            [attachment_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).map_err(|_| KrillnotesError::NoteNotFound(attachment_id.to_string()))?;

        let enc_path = self
            .workspace_root
            .join("attachments")
            .join(format!("{attachment_id}.enc"));
        let encrypted_bytes = std::fs::read(&enc_path)?;
        decrypt_attachment(&encrypted_bytes, self.attachment_key.as_ref(), &salt_bytes)
    }

    /// Returns decrypted attachment bytes together with the stored MIME type.
    pub fn get_attachment_bytes_and_mime(
        &self,
        attachment_id: &str,
    ) -> Result<(Vec<u8>, Option<String>)> {
        let (salt_bytes, _, mime_type): (Vec<u8>, i64, Option<String>) =
            self.storage.connection().query_row(
                "SELECT salt, size_bytes, mime_type FROM attachments WHERE id = ?",
                [attachment_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).map_err(|_| KrillnotesError::NoteNotFound(attachment_id.to_string()))?;

        let enc_path = self
            .workspace_root
            .join("attachments")
            .join(format!("{attachment_id}.enc"));
        let encrypted_bytes = std::fs::read(&enc_path)?;
        let bytes = decrypt_attachment(&encrypted_bytes, self.attachment_key.as_ref(), &salt_bytes)?;
        Ok((bytes, mime_type))
    }

    /// Replaces `<img data-kn-attach-id="UUID">` sentinels in `html` with real
    /// `src="data:mime;base64,..."` attributes and converts `data-kn-width="N"`
    /// to an inline `style="max-width:Npx;height:auto"`.
    ///
    /// Called after running `on_view` and `on_hover` hooks so the frontend
    /// receives fully-embedded HTML without needing client-side hydration.
    /// Sentinels whose attachment cannot be read are left in place so the
    /// client-side fallback can show an error message.
    fn embed_attachment_images(&self, html: String) -> String {
        use base64::Engine as _;
        use std::sync::OnceLock;

        static ID_RE: OnceLock<regex::Regex> = OnceLock::new();
        static WIDTH_RE: OnceLock<regex::Regex> = OnceLock::new();

        let id_re = ID_RE.get_or_init(|| {
            regex::Regex::new(r#"data-kn-attach-id="([^"]+)""#).expect("valid regex")
        });
        let width_re = WIDTH_RE.get_or_init(|| {
            regex::Regex::new(r#"data-kn-width="(\d+)""#).expect("valid regex")
        });

        // Collect unique attachment IDs present in the HTML.
        let ids: Vec<String> = id_re
            .captures_iter(&html)
            .map(|cap| cap[1].to_string())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        if ids.is_empty() {
            return html;
        }

        // Replace each sentinel with a real data URL.
        let mut result = html;
        for id in ids {
            if let Ok((bytes, mime_opt)) = self.get_attachment_bytes_and_mime(&id) {
                let mime = mime_opt.as_deref().unwrap_or("image/png");
                let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
                let src = format!(r#"src="data:{mime};base64,{encoded}""#);
                result = result.replace(&format!(r#"data-kn-attach-id="{id}""#), &src);
            }
            // If the attachment cannot be read, leave the sentinel; the client
            // hydration fallback will display an "Image not found" error.
        }

        // Convert data-kn-width="N" → style="max-width:Npx;height:auto".
        let result = width_re.replace_all(&result, |caps: &regex::Captures| {
            format!(r#"style="max-width:{}px;height:auto""#, &caps[1])
        });
        result.into_owned()
    }

    /// Deletes an attachment: removes the `.enc` file and the DB row.
    /// Returns the metadata for a single attachment by ID.
    fn get_attachment_meta(&self, attachment_id: &str) -> Result<AttachmentMeta> {
        let row = self.storage.connection().query_row(
            "SELECT id, note_id, filename, mime_type, size_bytes, hash_sha256, salt, created_at
             FROM attachments WHERE id = ?",
            [attachment_id],
            |row| {
                let salt_bytes: Vec<u8> = row.get(6)?;
                Ok(AttachmentMeta {
                    id: row.get(0)?,
                    note_id: row.get(1)?,
                    filename: row.get(2)?,
                    mime_type: row.get(3)?,
                    size_bytes: row.get(4)?,
                    hash_sha256: row.get(5)?,
                    salt: hex::encode(&salt_bytes),
                    created_at: row.get(7)?,
                })
            },
        )?;
        Ok(row)
    }

    /// Soft-deletes an attachment: renames `{id}.enc` → `{id}.enc.trash` and removes the
    /// DB row. Pushes an `AttachmentRestore` entry onto the undo stack so the deletion
    /// can be reversed. The `.enc.trash` file is cleaned up when the undo entry is
    /// discarded (workspace close or stack overflow past the limit).
    pub fn delete_attachment(&mut self, attachment_id: &str) -> Result<()> {
        let enc_path = self
            .workspace_root
            .join("attachments")
            .join(format!("{attachment_id}.enc"));
        let trash_path = self
            .workspace_root
            .join("attachments")
            .join(format!("{attachment_id}.enc.trash"));

        if enc_path.exists() {
            std::fs::rename(&enc_path, &trash_path)?;
        }
        self.storage.connection().execute(
            "DELETE FROM attachments WHERE id = ?",
            [attachment_id],
        )?;
        Ok(())
    }

    /// Restores a soft-deleted attachment: renames `.enc.trash` → `.enc` (if the
    /// trash file exists) and re-inserts the DB row. Used by the in-section "Restore"
    /// button. Safe to call even if the session ended and the trash file was purged —
    /// only the DB row is re-inserted in that case.
    pub fn restore_attachment(&mut self, meta: &AttachmentMeta) -> Result<()> {
        let trash_path = self.workspace_root.join("attachments")
            .join(format!("{}.enc.trash", meta.id));
        let enc_path = self.workspace_root.join("attachments")
            .join(format!("{}.enc", meta.id));
        if trash_path.exists() {
            std::fs::rename(&trash_path, &enc_path)?;
        }
        let salt_bytes = hex::decode(&meta.salt)
            .unwrap_or_else(|_| meta.salt.as_bytes().to_vec());
        self.storage.connection().execute(
            "INSERT OR IGNORE INTO attachments
             (id, note_id, filename, mime_type, size_bytes, hash_sha256, salt, created_at)
             VALUES (?,?,?,?,?,?,?,?)",
            rusqlite::params![
                meta.id, meta.note_id, meta.filename, meta.mime_type,
                meta.size_bytes as i64, meta.hash_sha256,
                salt_bytes.as_slice(), meta.created_at,
            ],
        )?;
        Ok(())
    }

    /// Purges any `.enc.trash` files left over from a previous session.
    ///
    /// Should be called once on workspace open. Since undo stacks are in-session
    /// only, all `.enc.trash` files from prior sessions are safe to remove.
    fn purge_attachment_trash(&self) {
        let trash_dir = self.workspace_root.join("attachments");
        if let Ok(entries) = std::fs::read_dir(&trash_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("trash") {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }

    /// Returns the workspace-level max attachment size in bytes, or `None` if unlimited.
    pub fn attachment_max_size_bytes(&self) -> Result<Option<u64>> {
        let val: std::result::Result<String, rusqlite::Error> = self.storage.connection().query_row(
            "SELECT value FROM workspace_meta WHERE key = 'attachment_max_size_bytes'",
            [],
            |row| row.get(0),
        );
        match val {
            Ok(s) => Ok(s.parse::<u64>().ok()),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Sets or clears the workspace-level max attachment size.
    pub fn set_attachment_max_size_bytes(&mut self, limit: Option<u64>) -> Result<()> {
        match limit {
            Some(n) => {
                self.storage.connection().execute(
                    "INSERT OR REPLACE INTO workspace_meta (key, value) VALUES ('attachment_max_size_bytes', ?)",
                    [n.to_string()],
                )?;
            }
            None => {
                self.storage.connection().execute(
                    "DELETE FROM workspace_meta WHERE key = 'attachment_max_size_bytes'",
                    [],
                )?;
            }
        }
        Ok(())
    }

    /// Returns full `Note` data for every node in the subtree rooted at `note_id`,
    /// ordered parent-first (root at index 0) via a recursive CTE.
    fn collect_subtree_notes(&self, note_id: &str) -> Result<Vec<Note>> {
        let conn = self.connection();
        let mut stmt = conn.prepare(
            "WITH RECURSIVE subtree AS (
                SELECT n.id, 0 AS depth FROM notes n WHERE n.id = ?1
                UNION ALL
                SELECT n.id, s.depth + 1 FROM notes n JOIN subtree s ON n.parent_id = s.id
            )
            SELECT n.id, n.title, n.schema, n.parent_id, n.position,
                   n.created_at, n.modified_at, n.created_by, n.modified_by,
                   n.fields_json, n.is_expanded, n.schema_version,
                   GROUP_CONCAT(nt.tag, ',') AS tags_csv
            FROM notes n
            JOIN subtree s ON n.id = s.id
            LEFT JOIN note_tags nt ON nt.note_id = n.id
            GROUP BY n.id
            ORDER BY s.depth ASC",
        )?;
        let rows = stmt.query_map([note_id], map_note_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(note_from_row_tuple).collect()
    }

    /// Applies `inverse` to the database without touching undo/redo stacks.
    ///
    /// Returns the note ID most relevant for UI re-selection, if any.
    pub(crate) fn apply_retract_inverse_internal(
        &mut self,
        inverse: &RetractInverse,
    ) -> Result<Option<String>> {
        match inverse {
            RetractInverse::DeleteNote { note_id } => {
                // Undo of CreateNote: delete the note (no children expected).
                let all_ids = self.collect_subtree_ids(note_id)?;
                for id in &all_ids {
                    self.clear_links_to(id)?;
                }
                let tx = self.storage.connection_mut().transaction()?;
                Self::delete_recursive_in_tx(&tx, note_id)?;
                tx.commit()?;
                Ok(None)
            }

            RetractInverse::SubtreeRestore { notes, attachments } => {
                // Undo of DeleteNote: re-insert notes (parent-first) and attachment rows.
                let root_id = notes.first().map(|n| n.id.clone());
                let conn = self.storage.connection_mut();
                let tx = conn.transaction()?;
                for note in notes {
                    let fields_json = serde_json::to_string(&note.fields)
                        .map_err(KrillnotesError::Json)?;
                    tx.execute(
                        "INSERT OR IGNORE INTO notes
                         (id, title, schema, parent_id, position,
                          created_at, modified_at, created_by, modified_by,
                          fields_json, is_expanded, schema_version)
                         VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
                        rusqlite::params![
                            note.id, note.title, note.schema, note.parent_id,
                            note.position, note.created_at, note.modified_at,
                            note.created_by, note.modified_by, fields_json,
                            note.is_expanded as i32, note.schema_version,
                        ],
                    )?;
                    for tag in &note.tags {
                        tx.execute(
                            "INSERT OR IGNORE INTO note_tags (note_id, tag) VALUES (?,?)",
                            rusqlite::params![note.id, tag],
                        )?;
                    }
                }
                for att in attachments {
                    // salt is hex-encoded in AttachmentMeta; DB stores raw bytes.
                    let salt_bytes = hex::decode(&att.salt)
                        .unwrap_or_else(|_| att.salt.as_bytes().to_vec());
                    tx.execute(
                        "INSERT OR IGNORE INTO attachments
                         (id, note_id, filename, mime_type, size_bytes, hash_sha256, salt, created_at)
                         VALUES (?,?,?,?,?,?,?,?)",
                        rusqlite::params![
                            att.id, att.note_id, att.filename, att.mime_type,
                            att.size_bytes as i64, att.hash_sha256,
                            salt_bytes.as_slice(), att.created_at,
                        ],
                    )?;
                }
                tx.commit()?;
                Ok(root_id)
            }

            RetractInverse::NoteRestore { note_id, old_title, old_fields, old_tags } => {
                // Restore title + fields + tags atomically.
                let fields_json = serde_json::to_string(old_fields)
                    .map_err(KrillnotesError::Json)?;
                let now = chrono::Utc::now().timestamp();
                let conn = self.storage.connection_mut();
                let tx = conn.transaction()?;
                tx.execute(
                    "UPDATE notes SET title=?, fields_json=?, modified_at=? WHERE id=?",
                    rusqlite::params![old_title, fields_json, now, note_id],
                )?;
                tx.execute("DELETE FROM note_tags WHERE note_id=?", [note_id])?;
                for tag in old_tags {
                    tx.execute(
                        "INSERT INTO note_tags (note_id, tag) VALUES (?,?)",
                        rusqlite::params![note_id, tag],
                    )?;
                }
                tx.commit()?;
                Ok(Some(note_id.clone()))
            }

            RetractInverse::PositionRestore { note_id, old_parent_id, old_position } => {
                self.move_note(note_id, old_parent_id.as_deref(), *old_position)?;
                Ok(Some(note_id.clone()))
            }

            RetractInverse::DeleteScript { script_id } => {
                self.storage.connection().execute(
                    "DELETE FROM user_scripts WHERE id=?",
                    [script_id],
                )?;
                self.reload_scripts()?;
                Ok(None)
            }

            RetractInverse::ScriptRestore {
                script_id, name, description,
                source_code, load_order, enabled,
            } => {
                let now = chrono::Utc::now().timestamp();
                self.storage.connection().execute(
                    "INSERT OR REPLACE INTO user_scripts
                     (id, name, description, source_code, load_order, enabled,
                      created_at, modified_at, category)
                     VALUES (?,?,?,?,?,?,?,?,?)",
                    rusqlite::params![
                        script_id, name, description, source_code,
                        load_order, enabled, now, now, "presentation",
                    ],
                )?;
                self.reload_scripts()?;
                Ok(None)
            }

            RetractInverse::AttachmentRestore { meta } => {
                let note_id = meta.note_id.clone();
                self.restore_attachment(meta)?;
                Ok(Some(note_id))
            }

            RetractInverse::AttachmentSoftDelete { attachment_id } => {
                // Redo of DeleteAttachment: rename .enc → .enc.trash, delete DB row.
                let note_id: Option<String> = self.storage.connection()
                    .query_row(
                        "SELECT note_id FROM attachments WHERE id = ?",
                        [attachment_id],
                        |row| row.get(0),
                    )
                    .ok();
                let enc_path = self.workspace_root.join("attachments")
                    .join(format!("{attachment_id}.enc"));
                let trash_path = self.workspace_root.join("attachments")
                    .join(format!("{attachment_id}.enc.trash"));
                if enc_path.exists() {
                    std::fs::rename(&enc_path, &trash_path)?;
                }
                self.storage.connection().execute(
                    "DELETE FROM attachments WHERE id = ?",
                    [attachment_id],
                )?;
                Ok(note_id)
            }

            RetractInverse::Batch(items) => {
                // Apply in reverse order (LIFO).
                let mut last_note = None;
                for item in items.iter().rev() {
                    if let Some(id) = self.apply_retract_inverse_internal(item)? {
                        last_note = Some(id);
                    }
                }
                Ok(last_note)
            }
        }
    }

    // ── Snapshot (peer sync) ───────────────────────────────────────

    /// Serialise all notes, user scripts, and attachment metadata to JSON bytes for a snapshot bundle.
    pub fn to_snapshot_json(&self) -> Result<Vec<u8>> {
        let notes = self.list_all_notes()?;
        let user_scripts = self.list_user_scripts()?;
        let attachments = self.list_all_attachments()?;
        let snapshot = WorkspaceSnapshot {
            version: 1,
            notes,
            user_scripts,
            attachments,
        };
        Ok(serde_json::to_vec(&snapshot)?)
    }

    /// Returns the `operation_id` of the most recent logged operation, or `None` if log is empty.
    pub fn get_latest_operation_id(&self) -> Result<Option<String>> {
        let conn = self.storage.connection();
        let mut stmt = conn.prepare(
            "SELECT operation_id FROM operations ORDER BY timestamp_wall_ms DESC, timestamp_counter DESC LIMIT 1"
        )?;
        match stmt.query_row([], |row| row.get::<_, String>(0)) {
            Ok(id) => Ok(Some(id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(KrillnotesError::Database(e)),
        }
    }

    /// Populate a workspace from snapshot JSON bytes.
    ///
    /// Notes and user scripts are inserted. Returns the number of notes imported.
    /// Designed for freshly created workspaces — duplicates will be skipped via INSERT OR IGNORE.
    pub fn import_snapshot_json(&mut self, data: &[u8]) -> Result<usize> {
        let snapshot: WorkspaceSnapshot = serde_json::from_slice(data)
            .map_err(|e| KrillnotesError::Json(e))?;

        let note_count = snapshot.notes.len();

        // Bulk-insert notes preserving original IDs.
        // Defer foreign-key checks so children can be inserted before parents.
        {
            self.storage
                .connection_mut()
                .execute_batch("PRAGMA defer_foreign_keys = ON;")?;
            let tx = self.storage.connection_mut().transaction()?;
            for note in &snapshot.notes {
                let fields_json = serde_json::to_string(&note.fields)?;
                tx.execute(
                    "INSERT OR IGNORE INTO notes (id, title, schema, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded, schema_version)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    rusqlite::params![
                        note.id,
                        note.title,
                        note.schema,
                        note.parent_id,
                        note.position,
                        note.created_at,
                        note.modified_at,
                        note.created_by,
                        note.modified_by,
                        fields_json,
                        note.is_expanded,
                        note.schema_version,
                    ],
                )?;
                for tag in &note.tags {
                    tx.execute(
                        "INSERT OR IGNORE INTO note_tags (note_id, tag) VALUES (?, ?)",
                        rusqlite::params![note.id, tag],
                    )?;
                }
            }
            tx.commit()?;
        }

        // Insert user scripts (preserve original IDs via INSERT OR IGNORE).
        if !snapshot.user_scripts.is_empty() {
            let tx = self.storage.connection_mut().transaction()?;
            for script in &snapshot.user_scripts {
                tx.execute(
                    "INSERT OR IGNORE INTO user_scripts (id, name, description, source_code, load_order, enabled, created_at, modified_at, category)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    rusqlite::params![
                        script.id,
                        script.name,
                        script.description,
                        script.source_code,
                        script.load_order,
                        script.enabled,
                        script.created_at,
                        script.modified_at,
                        script.category,
                    ],
                )?;
            }
            tx.commit()?;
        }

        Ok(note_count)
    }

    /// Returns a resolved view of all sync peers for this workspace, joining
    /// sync_peers with the given contact manager for name/trust resolution.
    /// Sorted by display_name ascending.
    pub fn list_peers_info(
        &self,
        contact_manager: &crate::core::contact::ContactManager,
    ) -> Result<Vec<PeerInfo>> {
        let conn = self.storage.connection();
        let registry = PeerRegistry::new(conn);
        let peers = registry.list_peers()?;
        let contacts = contact_manager.list_contacts()?;

        let mut result: Vec<PeerInfo> = peers
            .into_iter()
            .map(|peer| {
                let contact = contacts
                    .iter()
                    .find(|c| c.public_key == peer.peer_identity_id);

                let display_name = contact
                    .map(|c| c.local_name.clone().unwrap_or_else(|| c.declared_name.clone()))
                    .unwrap_or_else(|| {
                        let key = &peer.peer_identity_id;
                        format!("{}…", &key[..key.len().min(8)])
                    });

                let fingerprint = generate_fingerprint(&peer.peer_identity_id)
                    .unwrap_or_else(|_| format!("{}…", &peer.peer_identity_id[..peer.peer_identity_id.len().min(8)]));

                let trust_level = contact.map(|c| match c.trust_level {
                    TrustLevel::Tofu => "Tofu".to_string(),
                    TrustLevel::CodeVerified => "CodeVerified".to_string(),
                    TrustLevel::Vouched => "Vouched".to_string(),
                    TrustLevel::VerifiedInPerson => "VerifiedInPerson".to_string(),
                });

                PeerInfo {
                    peer_device_id: peer.peer_device_id,
                    peer_identity_id: peer.peer_identity_id,
                    display_name,
                    fingerprint,
                    trust_level,
                    contact_id: contact.map(|c| c.contact_id.to_string()),
                    last_sync: peer.last_sync,
                }
            })
            .collect();

        result.sort_by(|a, b| a.display_name.cmp(&b.display_name));
        Ok(result)
    }

    /// Pre-authorises a contact as a workspace sync peer before any .swarm exchange.
    /// Uses `identity:<peer_identity_id>` as a placeholder device ID.
    pub fn add_contact_as_peer(
        &self,
        peer_identity_id: &str,
    ) -> Result<()> {
        let placeholder_device_id = format!("identity:{}", peer_identity_id);
        let conn = self.storage.connection();
        let registry = PeerRegistry::new(conn);
        registry.add_peer(&placeholder_device_id, peer_identity_id)
    }

    /// Removes a peer from this workspace's sync peer list by device ID.
    pub fn remove_peer(
        &self,
        peer_device_id: &str,
    ) -> Result<()> {
        let conn = self.storage.connection();
        let registry = PeerRegistry::new(conn);
        registry.remove_peer(peer_device_id)
    }

    /// Update last_sent_op for a peer identified by their identity public key.
    /// Peers added via invite use placeholder device_id = "identity:<pubkey>".
    /// Uses upsert semantics: inserts a peer row if none exists yet.
    pub fn update_peer_last_sent_by_identity(&self, identity_pk: &str, op_id: &str) -> Result<()> {
        let conn = self.storage.connection();
        let registry = PeerRegistry::new(conn);
        let placeholder_device_id = format!("identity:{identity_pk}");
        registry.upsert_last_sent(&placeholder_device_id, identity_pk, op_id)
    }

    /// Insert or update a sync peer row. Pass `None` for watermark fields that
    /// should not overwrite an existing value.
    pub fn upsert_sync_peer(
        &self,
        device_id: &str,
        identity_id: &str,
        last_sent_op: Option<&str>,
        last_received_op: Option<&str>,
    ) -> Result<()> {
        let conn = self.storage.connection();
        let registry = PeerRegistry::new(conn);
        registry.upsert_sync_peer(device_id, identity_id, last_sent_op, last_received_op)
    }

}

/// Keeps the `note_links` junction table in sync with the current field values of a note.
///
/// Deletes all existing rows for `note_id` as source, then re-inserts one row
/// for each `FieldValue::NoteLink(Some(...))` present in `fields`. This
/// replace-all strategy is correct for a single-writer (local) store.
///
/// Must be called inside an open transaction so that the link update is
/// atomic with the note write that precedes it.
fn sync_note_links(tx: &rusqlite::Transaction, note_id: &str, fields: &BTreeMap<String, FieldValue>) -> Result<()> {
    // Clear all existing note_links rows for this source note (replace strategy).
    tx.execute("DELETE FROM note_links WHERE source_id = ?1", [note_id])?;
    // Re-insert for any non-null NoteLink fields. No duplicates possible after
    // the DELETE above, so plain INSERT is sufficient.
    for (field_name, value) in fields {
        if let FieldValue::NoteLink(Some(target_id)) = value {
            tx.execute(
                "INSERT INTO note_links (source_id, field_name, target_id) VALUES (?1, ?2, ?3)",
                [note_id, field_name.as_str(), target_id.as_str()],
            )?;
        }
    }
    Ok(())
}

/// Raw 12-column tuple extracted from a `notes` + `note_tags` SQLite row.
///
/// `position` is stored as REAL in the DB (to support fractional positions for future
/// CRDT ordering) but the Rust API still uses `i32`; we read it as `f64` and truncate.
type NoteRow = (String, String, String, Option<String>, f64, i64, i64, String, String, String, i64, u32, Option<String>);

/// Row-mapping closure for `rusqlite::Row` → raw tuple.
///
/// Returns the 13-column tuple that `note_from_row_tuple` converts into a `Note`.
/// Extracted to avoid duplicating column-index logic across every query.
fn map_note_row(row: &rusqlite::Row) -> rusqlite::Result<NoteRow> {
    Ok((
        row.get::<_, String>(0)?,           // id
        row.get::<_, String>(1)?,           // title
        row.get::<_, String>(2)?,           // schema
        row.get::<_, Option<String>>(3)?,   // parent_id
        row.get::<_, f64>(4)?,              // position
        row.get::<_, i64>(5)?,              // created_at
        row.get::<_, i64>(6)?,              // modified_at
        row.get::<_, String>(7).unwrap_or_default(),  // created_by
        row.get::<_, String>(8).unwrap_or_default(),  // modified_by
        row.get::<_, String>(9)?,           // fields_json
        row.get::<_, i64>(10)?,             // is_expanded
        row.get::<_, u32>(11)?,             // schema_version
        row.get::<_, Option<String>>(12)?,  // tags_csv
    ))
}

/// Converts a raw 13-column tuple into a [`Note`], parsing `fields_json` and `tags_csv`.
fn note_from_row_tuple(
    (id, title, schema, parent_id, position, created_at, modified_at, created_by, modified_by, fields_json, is_expanded_int, schema_version, tags_csv): NoteRow,
) -> Result<Note> {
    let mut tags: Vec<String> = tags_csv
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    tags.sort();
    Ok(Note {
        id,
        title,
        schema,
        parent_id,
        position,
        created_at,
        modified_at,
        created_by,
        modified_by,
        fields: serde_json::from_str(&fields_json)?,
        is_expanded: is_expanded_int == 1,
        tags,
        schema_version,
    })
}

/// Converts a [`Note`] into a Rhai `Dynamic` map for use in `on_view` query functions.
///
/// Produces the `{ id, schema, title, fields, tags }` shape used by `on_view`
/// hooks and `QueryContext` indexes. Note: `on_save` hooks receive a narrower
/// `{ id, schema, title, fields }` map without `tags`.
fn note_to_rhai_dynamic(note: &Note) -> Dynamic {
    use crate::core::scripting::field_value_to_dynamic;
    let mut fields_map = rhai::Map::new();
    for (k, v) in &note.fields {
        fields_map.insert(k.as_str().into(), field_value_to_dynamic(v));
    }
    let tags_array: rhai::Array = note.tags.iter()
        .map(|t| Dynamic::from(t.clone()))
        .collect();
    let mut note_map = rhai::Map::new();
    note_map.insert("id".into(), Dynamic::from(note.id.clone()));
    note_map.insert("schema".into(), Dynamic::from(note.schema.clone()));
    note_map.insert("title".into(), Dynamic::from(note.title.clone()));
    note_map.insert("fields".into(), Dynamic::from(fields_map));
    note_map.insert("tags".into(), Dynamic::from(tags_array));
    Dynamic::from(note_map)
}

fn humanize(filename: &str) -> String {
    filename
        .replace(['-', '_'], " ")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::contact::{ContactManager, TrustLevel};
    use crate::FieldValue;
    use std::collections::BTreeMap;
    use tempfile::NamedTempFile;

    #[test]
    fn test_create_workspace() {
        let temp = NamedTempFile::new().unwrap();
        let ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Verify root note exists
        let count: i64 = ws
            .connection()
            .query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))
            .unwrap();

        assert_eq!(count, 1);
    }

    #[test]
    fn test_humanize() {
        assert_eq!(humanize("my-project"), "My Project");
        assert_eq!(humanize("hello_world"), "Hello World");
        assert_eq!(humanize("test-case-123"), "Test Case 123");
    }

    #[test]
    fn test_create_and_get_note() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let child_id = ws
            .create_note(&root.id, AddPosition::AsChild, "TextNote")
            .unwrap();

        let child = ws.get_note(&child_id).unwrap();
        assert_eq!(child.title, "Untitled");
        assert_eq!(child.parent_id, Some(root.id));
    }

    #[test]
    fn test_update_note_title() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        ws.update_note_title(&root.id, "New Title".to_string())
            .unwrap();

        let updated = ws.get_note(&root.id).unwrap();
        assert_eq!(updated.title, "New Title");
    }

    #[test]
    fn test_open_existing_workspace() {
        let temp = NamedTempFile::new().unwrap();

        // Create workspace first
        {
            let ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
            let root = ws.list_all_notes().unwrap()[0].clone();
            assert_eq!(root.schema, "TextNote");
        }

        // Open it
        let ws = Workspace::open(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Verify we can read notes
        let notes = ws.list_all_notes().unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].schema, "TextNote");
    }

    #[test]
    fn test_is_expanded_defaults_to_true() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Check root note is expanded by default
        let root = ws.list_all_notes().unwrap()[0].clone();
        assert!(root.is_expanded, "Root note should be expanded by default");

        // Create a child note and verify it's expanded by default
        let child_id = ws
            .create_note(&root.id, AddPosition::AsChild, "TextNote")
            .unwrap();

        let child = ws.get_note(&child_id).unwrap();
        assert!(child.is_expanded, "New child note should be expanded by default");
    }

    #[test]
    fn test_is_expanded_persists_across_open() {
        let temp = NamedTempFile::new().unwrap();

        // Create workspace with notes
        {
            let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
            let root = ws.list_all_notes().unwrap()[0].clone();
            ws.create_note(&root.id, AddPosition::AsChild, "TextNote")
                .unwrap();
        }

        // Open and verify is_expanded is true
        let ws = Workspace::open(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let notes = ws.list_all_notes().unwrap();
        assert_eq!(notes.len(), 2);
        assert!(notes[0].is_expanded, "Root note should be expanded");
        assert!(notes[1].is_expanded, "Child note should be expanded");
    }

    #[test]
    fn test_toggle_note_expansion() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        assert!(root.is_expanded, "Root should start expanded");

        // Toggle to collapsed
        ws.toggle_note_expansion(&root.id).unwrap();
        let note = ws.get_note(&root.id).unwrap();
        assert!(!note.is_expanded, "Root should now be collapsed");

        // Toggle back to expanded
        ws.toggle_note_expansion(&root.id).unwrap();
        let note = ws.get_note(&root.id).unwrap();
        assert!(note.is_expanded, "Root should be expanded again");
    }

    #[test]
    fn test_toggle_note_expansion_with_child_notes() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let child_id = ws
            .create_note(&root.id, AddPosition::AsChild, "TextNote")
            .unwrap();

        // Toggle child note
        ws.toggle_note_expansion(&child_id).unwrap();
        let child = ws.get_note(&child_id).unwrap();
        assert!(!child.is_expanded, "Child should be collapsed");

        // Toggle back
        ws.toggle_note_expansion(&child_id).unwrap();
        let child = ws.get_note(&child_id).unwrap();
        assert!(child.is_expanded, "Child should be expanded");
    }

    #[test]
    fn test_toggle_note_expansion_nonexistent_note() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Try to toggle a note that doesn't exist
        let result = ws.toggle_note_expansion("nonexistent-id");
        assert!(result.is_err(), "Should error for nonexistent note");
    }

    #[test]
    fn test_set_and_get_selected_note() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();

        // Initially no selection
        let selected = ws.get_selected_note().unwrap();
        assert_eq!(selected, None, "Should have no selection initially");

        // Set selection
        ws.set_selected_note(Some(&root.id)).unwrap();
        let selected = ws.get_selected_note().unwrap();
        assert_eq!(selected, Some(root.id.clone()), "Should return selected note ID");

        // Clear selection
        ws.set_selected_note(None).unwrap();
        let selected = ws.get_selected_note().unwrap();
        assert_eq!(selected, None, "Should have no selection after clearing");
    }

    #[test]
    fn test_selected_note_persists_across_open() {
        let temp = NamedTempFile::new().unwrap();

        // Create workspace and set selection
        {
            let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
            let root = ws.list_all_notes().unwrap()[0].clone();
            ws.set_selected_note(Some(&root.id)).unwrap();
        }

        // Open workspace and verify selection persists
        let ws = Workspace::open(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let selected = ws.get_selected_note().unwrap();
        assert_eq!(selected, Some(root.id), "Selection should persist across open");
    }

    #[test]
    fn test_set_selected_note_overwrites_previous() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let child_id = ws
            .create_note(&root.id, AddPosition::AsChild, "TextNote")
            .unwrap();

        // Set first selection
        ws.set_selected_note(Some(&root.id)).unwrap();
        let selected = ws.get_selected_note().unwrap();
        assert_eq!(selected, Some(root.id.clone()));

        // Set second selection (should overwrite)
        ws.set_selected_note(Some(&child_id)).unwrap();
        let selected = ws.get_selected_note().unwrap();
        assert_eq!(selected, Some(child_id.clone()), "Should overwrite previous selection");
    }

    #[test]
    fn test_create_note_root() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Delete existing root note to simulate empty workspace
        let existing_root = ws.list_all_notes().unwrap()[0].clone();
        ws.storage.connection_mut().execute(
            "DELETE FROM notes WHERE id = ?",
            [&existing_root.id],
        ).unwrap();

        // Create a new root note
        let new_root_id = ws.create_note_root("TextNote").unwrap();
        let new_root = ws.get_note(&new_root_id).unwrap();

        assert_eq!(new_root.title, "Untitled");
        assert_eq!(new_root.schema, "TextNote");
        assert_eq!(new_root.parent_id, None, "Root note should have no parent");
        assert_eq!(new_root.position, 0.0, "Root note should be at position 0");
        assert!(new_root.is_expanded, "Root note should be expanded");
    }

    #[test]
    fn test_create_note_root_invalid_type() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Delete existing root note
        let existing_root = ws.list_all_notes().unwrap()[0].clone();
        ws.storage.connection_mut().execute(
            "DELETE FROM notes WHERE id = ?",
            [&existing_root.id],
        ).unwrap();

        // Try to create a root note with invalid type
        let result = ws.create_note_root("InvalidType");
        assert!(result.is_err(), "Should fail with invalid node type");
    }

    #[test]
    fn test_sibling_insertion_does_not_create_duplicate_positions() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();

        // Create child1 at position 0 under root
        let child1_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        // Create child2 as sibling after child1 → gets position 1
        let child2_id = ws.create_note(&child1_id, AddPosition::AsSibling, "TextNote").unwrap();
        // Create child3 as sibling after child1 → should push child2 to position 2, child3 at position 1
        let child3_id = ws.create_note(&child1_id, AddPosition::AsSibling, "TextNote").unwrap();

        let child1 = ws.get_note(&child1_id).unwrap();
        let child2 = ws.get_note(&child2_id).unwrap();
        let child3 = ws.get_note(&child3_id).unwrap();

        // All siblings should have unique positions
        assert_ne!(child1.position, child2.position, "child1 and child2 should not share a position");
        assert_ne!(child2.position, child3.position, "child2 and child3 should not share a position");
        assert_ne!(child1.position, child3.position, "child1 and child3 should not share a position");
    }

    #[test]
    fn test_get_note_with_corrupt_fields_json_returns_error() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();

        // Corrupt the stored JSON directly.
        ws.storage.connection_mut().execute(
            "UPDATE notes SET fields_json = 'not valid json' WHERE id = ?",
            [&root.id],
        ).unwrap();

        // Should return Err, not panic.
        let result = ws.get_note(&root.id);
        assert!(result.is_err(), "get_note should return Err for corrupt fields_json");
    }

    #[test]
    fn test_list_all_notes_with_corrupt_fields_json_returns_error() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();

        ws.storage.connection_mut().execute(
            "UPDATE notes SET fields_json = 'not valid json' WHERE id = ?",
            [&root.id],
        ).unwrap();

        let result = ws.list_all_notes();
        assert!(result.is_err(), "list_all_notes should return Err for corrupt fields_json");
    }

    #[test]
    fn test_sibling_insertion_preserves_correct_order() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();

        // Create child1 (position 0), child2 as sibling (position 1)
        let child1_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        let child2_id = ws.create_note(&child1_id, AddPosition::AsSibling, "TextNote").unwrap();
        // Insert child3 as sibling after child1 — should land between child1 and child2
        let child3_id = ws.create_note(&child1_id, AddPosition::AsSibling, "TextNote").unwrap();

        let child1 = ws.get_note(&child1_id).unwrap();
        let child2 = ws.get_note(&child2_id).unwrap();
        let child3 = ws.get_note(&child3_id).unwrap();

        // Expected order: child1 (0), child3 (1), child2 (2)
        assert_eq!(child1.position, 0.0, "child1 should remain at position 0");
        assert_eq!(child3.position, 1.0, "child3 (inserted after child1) should be at position 1");
        assert_eq!(child2.position, 2.0, "child2 should be bumped to position 2");
    }

    #[test]
    fn test_update_note() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Get the root note
        let notes = ws.list_all_notes().unwrap();
        let note_id = notes[0].id.clone();
        let original_modified = notes[0].modified_at;

        // Timestamp resolution is 1 s; sleep ensures modified_at advances.
        std::thread::sleep(std::time::Duration::from_secs(1));

        // Update the note
        let new_title = "Updated Title".to_string();
        let mut new_fields = BTreeMap::new();
        new_fields.insert("body".to_string(), FieldValue::Text("Updated body".to_string()));

        let updated = ws.update_note(&note_id, new_title.clone(), new_fields.clone()).unwrap();

        // Verify changes
        assert_eq!(updated.title, new_title);
        assert_eq!(updated.fields.get("body"), Some(&FieldValue::Text("Updated body".to_string())));
        assert!(updated.modified_at > original_modified);
    }

    #[test]
    fn test_update_note_not_found() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let result = ws.update_note("nonexistent-id", "Title".to_string(), BTreeMap::new());
        assert!(matches!(result, Err(KrillnotesError::NoteNotFound(_))));
    }

    #[test]
    fn test_count_children() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Get root note
        let notes = ws.list_all_notes().unwrap();
        let root_id = notes[0].id.clone();

        // Initially has 0 children
        let count = ws.count_children(&root_id).unwrap();
        assert_eq!(count, 0);

        // Create 3 child notes
        ws.create_note(&root_id, AddPosition::AsChild, "TextNote")
            .unwrap();
        ws.create_note(&root_id, AddPosition::AsChild, "TextNote")
            .unwrap();
        ws.create_note(&root_id, AddPosition::AsChild, "TextNote")
            .unwrap();

        // Now has 3 children
        let count = ws.count_children(&root_id).unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    fn test_delete_note_recursive() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Get root note
        let root = ws.list_all_notes().unwrap()[0].clone();
        let root_id = root.id.clone();

        // Create tree: root -> child1 -> grandchild1
        //                   -> child2
        let child1_id = ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        let child2_id = ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        let grandchild1_id = ws.create_note(&child1_id, AddPosition::AsChild, "TextNote").unwrap();

        // Count: root + child1 + child2 + grandchild1 = 4 notes
        assert_eq!(ws.list_all_notes().unwrap().len(), 4);

        // Delete child1 (should delete child1 + grandchild1)
        let result = ws.delete_note_recursive(&child1_id).unwrap();
        assert_eq!(result.deleted_count, 2);
        assert!(result.affected_ids.contains(&child1_id));
        assert!(result.affected_ids.contains(&grandchild1_id));

        // Now only root + child2 remain
        let remaining = ws.list_all_notes().unwrap();
        assert_eq!(remaining.len(), 2);
        assert!(remaining.iter().any(|n| n.id == root_id));
        assert!(remaining.iter().any(|n| n.id == child2_id));
    }

    #[test]
    fn test_delete_note_recursive_not_found() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let result = ws.delete_note_recursive("nonexistent-id");
        assert!(matches!(result, Err(KrillnotesError::NoteNotFound(_))));
    }

    #[test]
    fn test_delete_note_promote() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Get root note
        let root = ws.list_all_notes().unwrap()[0].clone();
        let root_id = root.id.clone();

        // Create tree: root -> middle -> child1
        //                              -> child2
        let middle_id = ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        let child1_id = ws.create_note(&middle_id, AddPosition::AsChild, "TextNote").unwrap();
        let child2_id = ws.create_note(&middle_id, AddPosition::AsChild, "TextNote").unwrap();

        // Count: 4 notes total
        assert_eq!(ws.list_all_notes().unwrap().len(), 4);

        // Delete middle (promote children)
        let result = ws.delete_note_promote(&middle_id).unwrap();
        assert_eq!(result.deleted_count, 1);
        assert_eq!(result.affected_ids, vec![middle_id.clone()]);

        // Now: root, child1, child2 (3 notes)
        let remaining = ws.list_all_notes().unwrap();
        assert_eq!(remaining.len(), 3);

        // Verify child1 and child2 now have root as parent
        let child1_updated = remaining.iter().find(|n| n.id == child1_id).unwrap();
        let child2_updated = remaining.iter().find(|n| n.id == child2_id).unwrap();
        assert_eq!(child1_updated.parent_id, Some(root_id.clone()));
        assert_eq!(child2_updated.parent_id, Some(root_id.clone()));
    }

    #[test]
    fn test_update_contact_rejects_empty_required_fields() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        // Contact schema is already loaded from starter scripts.

        let root_id = ws.list_all_notes().unwrap()[0].id.clone();
        // Contact must be created under a ContactsFolder (allowed_parent_schemas constraint).
        let folder_id = ws
            .create_note(&root_id, AddPosition::AsChild, "ContactsFolder")
            .unwrap();
        let contact_id = ws
            .create_note(&folder_id, AddPosition::AsChild, "Contact")
            .unwrap();

        // first_name is required but empty — save must fail.
        let mut fields = BTreeMap::new();
        fields.insert("first_name".to_string(), FieldValue::Text("".to_string()));
        fields.insert("middle_name".to_string(), FieldValue::Text("".to_string()));
        fields.insert("last_name".to_string(), FieldValue::Text("Smith".to_string()));
        fields.insert("phone".to_string(), FieldValue::Text("".to_string()));
        fields.insert("mobile".to_string(), FieldValue::Text("".to_string()));
        fields.insert("email".to_string(), FieldValue::Email("".to_string()));
        fields.insert("birthdate".to_string(), FieldValue::Date(None));
        fields.insert("address_street".to_string(), FieldValue::Text("".to_string()));
        fields.insert("address_city".to_string(), FieldValue::Text("".to_string()));
        fields.insert("address_zip".to_string(), FieldValue::Text("".to_string()));
        fields.insert("address_country".to_string(), FieldValue::Text("".to_string()));
        fields.insert("is_family".to_string(), FieldValue::Boolean(false));

        let result = ws.update_note(&contact_id, "".to_string(), fields);
        assert!(
            matches!(result, Err(KrillnotesError::ValidationFailed(_))),
            "Expected ValidationFailed, got {:?}", result
        );
    }

    /// Verify that `delete_note_promote` returns `NoteNotFound` when the given ID does not exist.
    #[test]
    fn test_delete_note_promote_not_found() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let result = ws.delete_note_promote("nonexistent-id");
        assert!(matches!(result, Err(KrillnotesError::NoteNotFound(_))));
    }

    /// Verifies that positions do not collide when children are promoted by
    /// `delete_note_promote`. Specifically, when a node with two children (sib1,
    /// sib2) is deleted, and sib1 itself has children (child1, child2), those
    /// grandchildren should receive sequential positions with no duplicates.
    #[test]
    fn test_delete_note_promote_no_position_collision() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Build tree: root -> sib1 (pos 0) -> child1 (pos 0)
        //                                   -> child2 (pos 1)
        //                  -> sib2 (pos 1)
        let root = ws.list_all_notes().unwrap()[0].clone();
        let sib1_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        let sib2_id = ws.create_note(&sib1_id, AddPosition::AsSibling, "TextNote").unwrap();
        let child1_id = ws.create_note(&sib1_id, AddPosition::AsChild, "TextNote").unwrap();
        let child2_id = ws.create_note(&child1_id, AddPosition::AsSibling, "TextNote").unwrap();

        // Delete sib1 with promote — child1 and child2 move up to root level
        ws.delete_note_promote(&sib1_id).unwrap();

        // Collect remaining notes at root level
        let notes = ws.list_all_notes().unwrap();

        // sib1 must be gone
        assert!(notes.iter().all(|n| n.id != sib1_id), "sib1 should be deleted");

        // Gather positions of the surviving root-level notes
        let root_level: Vec<_> = notes.iter().filter(|n| n.parent_id == Some(root.id.clone())).collect();
        let mut positions: Vec<f64> = root_level.iter().map(|n| n.position).collect();
        positions.sort_by(|a, b| a.partial_cmp(b).unwrap());

        // All positions must be unique
        let unique_count = {
            let mut deduped = positions.clone();
            deduped.dedup();
            deduped.len()
        };
        assert_eq!(
            positions.len(), unique_count,
            "Positions after promote must be unique, got: {:?}", positions
        );

        // sib2, child1, child2 should all be at root level
        let surviving_ids: Vec<_> = root_level.iter().map(|n| n.id.clone()).collect();
        assert!(surviving_ids.contains(&sib2_id), "sib2 should remain at root level");
        assert!(surviving_ids.contains(&child1_id), "child1 should be promoted to root level");
        assert!(surviving_ids.contains(&child2_id), "child2 should be promoted to root level");
    }

    #[test]
    fn test_update_contact_derives_title_from_hook() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        // Contact schema is already loaded from starter scripts.

        let notes = ws.list_all_notes().unwrap();
        let root_id = notes[0].id.clone();

        // Contact must be created under a ContactsFolder (allowed_parent_schemas constraint).
        let folder_id = ws
            .create_note(&root_id, AddPosition::AsChild, "ContactsFolder")
            .unwrap();
        let contact_id = ws
            .create_note(&folder_id, AddPosition::AsChild, "Contact")
            .unwrap();

        let mut fields = BTreeMap::new();
        fields.insert("first_name".to_string(), FieldValue::Text("Alice".to_string()));
        fields.insert("middle_name".to_string(), FieldValue::Text("".to_string()));
        fields.insert("last_name".to_string(), FieldValue::Text("Walker".to_string()));
        fields.insert("phone".to_string(), FieldValue::Text("".to_string()));
        fields.insert("mobile".to_string(), FieldValue::Text("".to_string()));
        fields.insert("email".to_string(), FieldValue::Email("".to_string()));
        fields.insert("birthdate".to_string(), FieldValue::Date(None));
        fields.insert("address_street".to_string(), FieldValue::Text("".to_string()));
        fields.insert("address_city".to_string(), FieldValue::Text("".to_string()));
        fields.insert("address_zip".to_string(), FieldValue::Text("".to_string()));
        fields.insert("address_country".to_string(), FieldValue::Text("".to_string()));
        fields.insert("is_family".to_string(), FieldValue::Boolean(false));

        let updated = ws
            .update_note(&contact_id, "ignored title".to_string(), fields)
            .unwrap();

        assert_eq!(updated.title, "Walker, Alice");
    }

    /// Verifies that `delete_note` dispatches correctly to both deletion strategies.
    ///
    /// - `DeleteAll` removes the target note and all descendants.
    /// - `PromoteChildren` removes only the target, re-parenting its children to
    ///   the grandparent.
    // ── User-script CRUD tests ──────────────────────────────────

    #[test]
    fn test_workspace_created_with_starter_scripts() {
        let temp = NamedTempFile::new().unwrap();
        let workspace = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let scripts = workspace.list_user_scripts().unwrap();
        assert!(!scripts.is_empty(), "New workspace should have starter scripts");
        // Verify starter scripts include both presentation and schema scripts
        let names: Vec<&str> = scripts.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Text Note"), "Should have Text Note schema");
        assert!(names.contains(&"Text Note Actions"), "Should have Text Note Actions");
    }

    #[test]
    fn test_create_user_script() {
        let temp = NamedTempFile::new().unwrap();
        let mut workspace = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let starter_count = workspace.list_user_scripts().unwrap().len();
        let source = "// @name: Test Script\n// @description: A test\nschema(\"TestType\", #{ version: 1, fields: [] });";
        let (script, errors) = workspace.create_user_script(source).unwrap();
        assert!(errors.is_empty());
        assert_eq!(script.name, "Test Script");
        assert_eq!(script.description, "A test");
        assert!(script.enabled);
        assert_eq!(script.load_order, starter_count as i32);
    }

    #[test]
    fn test_create_user_script_missing_name_fails() {
        let temp = NamedTempFile::new().unwrap();
        let mut workspace = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let source = "// no name here\nschema(\"X\", #{ version: 1, fields: [] });";
        let result = workspace.create_user_script(source);
        assert!(result.is_err());
    }

    #[test]
    fn test_update_user_script() {
        let temp = NamedTempFile::new().unwrap();
        let mut workspace = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let source = "// @name: Original\nschema(\"Orig\", #{ version: 1, fields: [] });";
        let (script, _) = workspace.create_user_script(source).unwrap();

        let new_source = "// @name: Updated\nschema(\"Updated\", #{ version: 1, fields: [] });";
        let (updated, errors) = workspace.update_user_script(&script.id, new_source).unwrap();
        assert!(errors.is_empty());
        assert_eq!(updated.name, "Updated");
    }

    #[test]
    fn test_delete_user_script() {
        let temp = NamedTempFile::new().unwrap();
        let mut workspace = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let initial_count = workspace.list_user_scripts().unwrap().len();
        let source = "// @name: ToDelete\nschema(\"Del\", #{ version: 1, fields: [] });";
        let (script, _) = workspace.create_user_script(source).unwrap();
        assert_eq!(workspace.list_user_scripts().unwrap().len(), initial_count + 1);

        workspace.delete_user_script(&script.id).unwrap();
        assert_eq!(workspace.list_user_scripts().unwrap().len(), initial_count);
    }

    #[test]
    fn test_toggle_user_script() {
        let temp = NamedTempFile::new().unwrap();
        let mut workspace = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let source = "// @name: Toggle\nschema(\"Tog\", #{ version: 1, fields: [] });";
        let (script, _) = workspace.create_user_script(source).unwrap();
        assert!(script.enabled);

        workspace.toggle_user_script(&script.id, false).unwrap();
        let updated = workspace.get_user_script(&script.id).unwrap();
        assert!(!updated.enabled);
    }

    #[test]
    fn test_user_scripts_sorted_by_load_order() {
        let temp = NamedTempFile::new().unwrap();
        let mut workspace = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let starter_count = workspace.list_user_scripts().unwrap().len();

        let s1 = "// @name: Second\nschema(\"S2\", #{ version: 1, fields: [] });";
        let s2 = "// @name: First\nschema(\"S1\", #{ version: 1, fields: [] });";
        workspace.create_user_script(s1).unwrap();
        let (second, _) = workspace.create_user_script(s2).unwrap();
        // Move "First" before all starters
        workspace.reorder_user_script(&second.id, -1).unwrap();

        let scripts = workspace.list_user_scripts().unwrap();
        assert_eq!(scripts[0].name, "First", "Reordered script should come first");
        // "Second" should come after all starters
        assert_eq!(scripts[starter_count + 1].name, "Second");
    }

    #[test]
    fn test_user_scripts_loaded_on_open() {
        let temp = NamedTempFile::new().unwrap();

        {
            let mut workspace = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
            workspace.create_user_script(
                "// @name: TestOpen\nschema(\"OpenType\", #{ version: 1, fields: [#{ name: \"x\", type: \"text\" }] });"
            ).unwrap(); // (UserScript, Vec<ScriptError>) — result not inspected here
        }

        let workspace = Workspace::open(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        assert!(workspace.script_registry().get_schema("OpenType").is_ok());
    }

    #[test]
    fn test_disabled_user_scripts_not_loaded_on_open() {
        let temp = NamedTempFile::new().unwrap();

        {
            let mut workspace = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
            let (script, _) = workspace.create_user_script(
                "// @name: Disabled\nschema(\"DisType\", #{ version: 1, fields: [#{ name: \"x\", type: \"text\" }] });"
            ).unwrap();
            workspace.toggle_user_script(&script.id, false).unwrap();
        }

        let workspace = Workspace::open(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        assert!(workspace.script_registry().get_schema("DisType").is_err());
    }

    #[test]
    fn test_delete_note_with_strategy() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let child_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();

        // Test DeleteAll strategy
        let result = ws.delete_note(&child_id, DeleteStrategy::DeleteAll).unwrap();
        assert_eq!(result.deleted_count, 1);

        // Create new child for PromoteChildren test
        let child2_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        let grandchild_id = ws.create_note(&child2_id, AddPosition::AsChild, "TextNote").unwrap();

        let result = ws.delete_note(&child2_id, DeleteStrategy::PromoteChildren).unwrap();
        assert_eq!(result.deleted_count, 1);

        // Verify grandchild promoted
        let notes = ws.list_all_notes().unwrap();
        let gc = notes.iter().find(|n| n.id == grandchild_id).unwrap();
        assert_eq!(gc.parent_id, Some(root.id));
    }

    // ── move_note tests ──────────────────────────────────────────

    /// Helper: create a workspace with a root note and N children under it.
    ///
    /// The first child is created with `AsChild` (position 0). Subsequent
    /// children are created with `AsSibling` relative to the previous child,
    /// giving them sequential positions 0, 1, 2, .... The returned `Vec`
    /// preserves that order: `child_ids[0]` is at position 0, etc.
    fn setup_with_children(n: usize) -> (Workspace, String, Vec<String>, NamedTempFile) {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let mut child_ids: Vec<String> = Vec::new();
        for i in 0..n {
            let id = if i == 0 {
                ws.create_note(&root.id, AddPosition::AsChild, "TextNote")
                    .unwrap()
            } else {
                ws.create_note(&child_ids[i - 1], AddPosition::AsSibling, "TextNote")
                    .unwrap()
            };
            child_ids.push(id);
        }
        (ws, root.id, child_ids, temp)
    }

    #[test]
    fn test_move_note_reorder_siblings() {
        let (mut ws, root_id, children, _temp) = setup_with_children(3);
        ws.move_note(&children[2], Some(&root_id), 0.0).unwrap();
        let kids = ws.get_children(&root_id).unwrap();
        assert_eq!(kids[0].id, children[2]);
        assert_eq!(kids[1].id, children[0]);
        assert_eq!(kids[2].id, children[1]);
        for (i, kid) in kids.iter().enumerate() {
            assert_eq!(kid.position, i as f64, "Position mismatch at index {i}");
        }
    }

    #[test]
    fn test_move_note_to_different_parent() {
        let (mut ws, root_id, children, _temp) = setup_with_children(2);
        ws.move_note(&children[1], Some(&children[0]), 0.0).unwrap();
        let root_kids = ws.get_children(&root_id).unwrap();
        assert_eq!(root_kids.len(), 1);
        assert_eq!(root_kids[0].id, children[0]);
        assert_eq!(root_kids[0].position, 0.0);
        let grandkids = ws.get_children(&children[0]).unwrap();
        assert_eq!(grandkids.len(), 1);
        assert_eq!(grandkids[0].id, children[1]);
        assert_eq!(grandkids[0].position, 0.0);
    }

    #[test]
    fn test_move_note_to_root() {
        let (mut ws, root_id, children, _temp) = setup_with_children(2);
        ws.move_note(&children[0], None, 1.0).unwrap();
        let root_kids = ws.get_children(&root_id).unwrap();
        assert_eq!(root_kids.len(), 1);
        assert_eq!(root_kids[0].id, children[1]);
        assert_eq!(root_kids[0].position, 0.0);
        let moved = ws.get_note(&children[0]).unwrap();
        assert_eq!(moved.parent_id, None);
        assert_eq!(moved.position, 1.0);
    }

    #[test]
    fn test_move_note_prevents_cycle() {
        let (mut ws, _root_id, children, _temp) = setup_with_children(1);
        let grandchild_id = ws
            .create_note(&children[0], AddPosition::AsChild, "TextNote")
            .unwrap();
        let result = ws.move_note(&children[0], Some(&grandchild_id), 0.0);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("cycle"), "Expected cycle error, got: {err}");
    }

    #[test]
    fn test_move_note_prevents_self_move() {
        let (mut ws, _root_id, children, _temp) = setup_with_children(1);
        let result = ws.move_note(&children[0], Some(&children[0]), 0.0);
        assert!(result.is_err());
    }

    #[test]
    fn test_move_note_logs_operation() {
        // The operation log is always active — MoveNote must be recorded.
        let (mut ws, root_id, children, _temp) = setup_with_children(2);
        ws.move_note(&children[1], Some(&root_id), 0.0).unwrap();
        let ops = ws.list_operations(None, None, None).unwrap();
        let move_ops: Vec<_> = ops.iter().filter(|o| o.operation_type == "MoveNote").collect();
        assert_eq!(move_ops.len(), 1, "Expected one MoveNote operation in always-on log");
    }

    #[test]
    fn test_move_note_positions_gapless_after_cross_parent_move() {
        let (mut ws, root_id, children, _temp) = setup_with_children(4);
        ws.move_note(&children[1], Some(&children[0]), 0.0).unwrap();
        let root_kids = ws.get_children(&root_id).unwrap();
        assert_eq!(root_kids.len(), 3);
        for (i, kid) in root_kids.iter().enumerate() {
            assert_eq!(kid.position, i as f64, "Gap at index {i}");
        }
    }

    #[test]
    fn test_run_view_hook_returns_html_without_hook() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Load a schema with a textarea field but no on_view hook.
        ws.create_user_script(
            r#"// @name: Memo
schema("Memo", #{ version: 1,
    fields: [
        #{ name: "body", type: "textarea", required: false }
    ]
});
"#,
        )
        .unwrap();

        // Create a Memo note under the root.
        let root = ws.list_all_notes().unwrap()[0].clone();
        let note_id = ws
            .create_note(&root.id, AddPosition::AsChild, "Memo")
            .unwrap();

        // Update the note's body field with Markdown content.
        let mut fields = BTreeMap::new();
        fields.insert("body".into(), FieldValue::Text("**hello**".into()));
        ws.update_note(&note_id, "My Memo".into(), fields).unwrap();

        let html = ws.run_view_hook(&note_id).unwrap();
        assert!(!html.is_empty(), "default view must return non-empty HTML");
        assert!(
            html.contains("<strong>hello</strong>"),
            "textarea body should be markdown-rendered, got: {html}"
        );
    }

    #[test]
    fn test_create_user_script_rejects_compile_error() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let initial_count = ws.list_user_scripts().unwrap().len();

        // Clearly invalid Rhai: assignment with no identifier
        let bad_script = "// @name: Bad Script\n\nlet = 5;";
        let result = ws.create_user_script(bad_script);

        assert!(result.is_err(), "Should return error for invalid Rhai");
        // Confirm nothing was saved
        let scripts = ws.list_user_scripts().unwrap();
        assert_eq!(scripts.len(), initial_count, "No script should be saved on compile error");
    }

    #[test]
    fn test_update_user_script_rejects_compile_error() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let initial_count = ws.list_user_scripts().unwrap().len();

        // Create a valid script first
        let valid_script = "// @name: Good Script\n\n// valid empty body";
        let (created, _) = ws.create_user_script(valid_script).unwrap();

        // Attempt update with invalid Rhai
        let bad_script = "// @name: Good Script\n\nlet = 5;";
        let result = ws.update_user_script(&created.id, bad_script);

        assert!(result.is_err(), "Should return error for invalid Rhai on update");

        // Original source code must be preserved
        let scripts = ws.list_user_scripts().unwrap();
        assert_eq!(scripts.len(), initial_count + 1, "Script count must be unchanged after failed update");
        let saved = scripts.iter().find(|s| s.id == created.id).unwrap();
        assert_eq!(
            saved.source_code, valid_script,
            "Source code must be unchanged after failed update"
        );
    }

    #[test]
    fn test_create_workspace_with_password() {
        let temp = NamedTempFile::new().unwrap();
        let ws = Workspace::create(temp.path(), "secret", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        // Should have at least one note (the root note)
        assert!(!ws.list_all_notes().unwrap().is_empty());
    }

    #[test]
    fn test_open_workspace_with_password() {
        let temp = NamedTempFile::new().unwrap();
        Workspace::create(temp.path(), "secret", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let ws = Workspace::open(temp.path(), "secret", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        assert!(!ws.list_all_notes().unwrap().is_empty());
    }

    #[test]
    fn test_open_workspace_wrong_password() {
        let temp = NamedTempFile::new().unwrap();
        Workspace::create(temp.path(), "secret", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let result = Workspace::open(temp.path(), "wrong", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]));
        assert!(matches!(result, Err(KrillnotesError::WrongPassword)));
    }

    #[test]
    fn test_deep_copy_note_as_child() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // root → child
        let root = ws.list_all_notes().unwrap()[0].clone();
        let child_id = ws
            .create_note(&root.id, AddPosition::AsChild, "TextNote")
            .unwrap();
        ws.update_note_title(&child_id, "Original Child".to_string())
            .unwrap();

        // Copy child as another child of root
        let copy_id = ws
            .deep_copy_note(&child_id, &root.id, AddPosition::AsChild)
            .unwrap();

        // Copy has a new ID
        assert_ne!(copy_id, child_id);

        // Copy has same title and schema
        let copy = ws.get_note(&copy_id).unwrap();
        assert_eq!(copy.title, "Original Child");
        assert_eq!(copy.schema, "TextNote");

        // Original is unchanged
        let original = ws.get_note(&child_id).unwrap();
        assert_eq!(original.title, "Original Child");
        assert_eq!(original.parent_id, Some(root.id.clone()));
    }

    #[test]
    fn test_deep_copy_note_recursive() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // root → note_a → note_b
        let root = ws.list_all_notes().unwrap()[0].clone();
        let note_a_id = ws
            .create_note(&root.id, AddPosition::AsChild, "TextNote")
            .unwrap();
        ws.update_note_title(&note_a_id, "Note A".to_string())
            .unwrap();
        let note_b_id = ws
            .create_note(&note_a_id, AddPosition::AsChild, "TextNote")
            .unwrap();
        ws.update_note_title(&note_b_id, "Note B".to_string())
            .unwrap();

        // Copy note_a (with note_b inside) as a child of root
        let copy_a_id = ws
            .deep_copy_note(&note_a_id, &root.id, AddPosition::AsChild)
            .unwrap();

        // copy of note_a exists with a new ID and correct title
        assert_ne!(copy_a_id, note_a_id);
        let copy_a = ws.get_note(&copy_a_id).unwrap();
        assert_eq!(copy_a.title, "Note A");

        // A copy of note_b also exists — find it by parent = copy_a
        let all_notes = ws.list_all_notes().unwrap();
        let copy_b = all_notes
            .iter()
            .find(|n| n.parent_id.as_deref() == Some(&copy_a_id) && n.title == "Note B")
            .expect("copy of note_b should exist under copy_a");

        // copy of note_b has a new ID (not the original)
        assert_ne!(copy_b.id, note_b_id);

        // originals are untouched
        let orig_a = ws.get_note(&note_a_id).unwrap();
        assert_eq!(orig_a.parent_id, Some(root.id.clone()));
        let orig_b = ws.get_note(&note_b_id).unwrap();
        assert_eq!(orig_b.parent_id, Some(note_a_id.clone()));
    }

    #[test]
    fn test_on_add_child_hook_fires_on_create() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        ws.script_registry_mut().load_script(r#"
            schema("Folder", #{ version: 1,
                fields: [
                    #{ name: "count", type: "number", required: false },
                ],
                on_add_child: |parent_note, child_note| {
                    let new_count = parent_note.fields["count"] + 1.0;
                    set_field(parent_note.id, "count", new_count);
                    set_title(parent_note.id, "Folder (1)");
                    commit();
                }
            });
            schema("Item", #{ version: 1,
                fields: [],
            });
        "#, "test").unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let folder_id = ws.create_note(&root.id, AddPosition::AsChild, "Folder").unwrap();

        // Create an Item under the Folder — this should trigger the hook
        ws.create_note(&folder_id, AddPosition::AsChild, "Item").unwrap();

        let folder = ws.get_note(&folder_id).unwrap();
        assert_eq!(folder.title, "Folder (1)");
        assert_eq!(folder.fields["count"], FieldValue::Number(1.0));
    }

    #[test]
    fn test_on_add_child_hook_fires_for_sibling_under_hooked_parent() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        ws.script_registry_mut().load_script(r#"
            schema("Folder", #{ version: 1,
                fields: [
                    #{ name: "count", type: "number", required: false },
                ],
                on_add_child: |parent_note, child_note| {
                    let new_count = parent_note.fields["count"] + 1.0;
                    set_field(parent_note.id, "count", new_count);
                    commit();
                }
            });
            schema("Item", #{ version: 1,
                fields: [],
            });
        "#, "test").unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let folder_id = ws.create_note(&root.id, AddPosition::AsChild, "Folder").unwrap();
        // First child created as child of Folder (hook fires, count=1)
        let first_item_id = ws.create_note(&folder_id, AddPosition::AsChild, "Item").unwrap();
        // Second item created as sibling of first (still a child of Folder, hook should fire again, count=2)
        ws.create_note(&first_item_id, AddPosition::AsSibling, "Item").unwrap();

        let folder = ws.get_note(&folder_id).unwrap();
        assert_eq!(folder.fields["count"], FieldValue::Number(2.0));
    }

    #[test]
    fn test_on_add_child_hook_does_not_fire_for_root_level_creation() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // No on_add_child hook registered — creating a sibling of root should work silently
        let root = ws.list_all_notes().unwrap()[0].clone();
        // This creates a sibling of root, which has no parent — should not panic or error
        let result = ws.create_note(&root.id, AddPosition::AsSibling, "TextNote");
        assert!(result.is_ok(), "sibling of root should succeed without hook");
    }

    #[test]
    fn test_on_add_child_hook_fires_on_move() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        ws.script_registry_mut().load_script(r#"
            schema("Folder", #{ version: 1,
                fields: [
                    #{ name: "count", type: "number", required: false },
                ],
                on_add_child: |parent_note, child_note| {
                    let new_count = parent_note.fields["count"] + 1.0;
                    set_field(parent_note.id, "count", new_count);
                    set_title(parent_note.id, "Folder (1)");
                    commit();
                }
            });
            schema("Item", #{ version: 1,
                fields: [],
            });
        "#, "test").unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        // Create Folder and Item as siblings (both children of root)
        let folder_id = ws.create_note(&root.id, AddPosition::AsChild, "Folder").unwrap();
        let item_id   = ws.create_note(&root.id, AddPosition::AsChild, "Item").unwrap();

        // Move Item under Folder — hook should fire
        ws.move_note(&item_id, Some(&folder_id), 0.0).unwrap();

        let folder = ws.get_note(&folder_id).unwrap();
        assert_eq!(folder.title, "Folder (1)");
        assert_eq!(folder.fields["count"], FieldValue::Number(1.0));
    }

    // ── tree actions ─────────────────────────────────────────────────────────

    #[test]
    fn test_run_tree_action_reorders_children() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let parent_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();

        // Create first child: "B Note" (position 0)
        let child_b_id = ws.create_note(&parent_id, AddPosition::AsChild, "TextNote").unwrap();
        ws.update_note_title(&child_b_id, "B Note".to_string()).unwrap();

        // Create second child as sibling: "A Note" (position 1)
        let child_a_id = ws.create_note(&child_b_id, AddPosition::AsSibling, "TextNote").unwrap();
        ws.update_note_title(&child_a_id, "A Note".to_string()).unwrap();

        // Verify initial order: B Note first, A Note second
        let kids_before = ws.get_children(&parent_id).unwrap();
        assert_eq!(kids_before[0].title, "B Note");
        assert_eq!(kids_before[1].title, "A Note");

        // Load a script that sorts children alphabetically
        ws.create_user_script(r#"
// @name: SortTest
register_menu("Sort A→Z", ["TextNote"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| a.title <= b.title);
    children.map(|c| c.id)
});
        "#).unwrap();

        ws.run_tree_action(&parent_id, "Sort A→Z").unwrap();

        let kids = ws.get_children(&parent_id).unwrap();
        assert_eq!(kids[0].title, "A Note");
        assert_eq!(kids[1].title, "B Note");
    }

    // ── tree action creates / updates ─────────────────────────────────────────

    #[test]
    fn test_tree_action_create_note_writes_to_db() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        ws.create_user_script(r#"
// @name: CreateAction
schema("TaFolder", #{ version: 1, fields: [] });
schema("TaItem", #{ version: 1, fields: [#{ name: "tag", type: "text", required: false }] });
register_menu("Add Item", ["TaFolder"], |folder| {
    let item = create_child(folder.id, "TaItem");
    set_title(item.id, "My Item");
    set_field(item.id, "tag", "hello");
    commit();
});
        "#).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let folder_id = ws.create_note(&root.id, AddPosition::AsChild, "TaFolder").unwrap();

        ws.run_tree_action(&folder_id, "Add Item").unwrap();

        let children = ws.get_children(&folder_id).unwrap();
        assert_eq!(children.len(), 1, "one child should have been created");
        assert_eq!(children[0].title, "My Item");
        assert_eq!(
            children[0].fields.get("tag"),
            Some(&FieldValue::Text("hello".into()))
        );
    }

    #[test]
    fn test_tree_action_update_note_writes_to_db() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        ws.create_user_script(r#"
// @name: UpdateAction
schema("TaTask", #{ version: 1, fields: [#{ name: "status", type: "text", required: false }] });
register_menu("Mark Done", ["TaTask"], |note| {
    set_title(note.id, "Done Task");
    set_field(note.id, "status", "done");
    commit();
});
        "#).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let task_id = ws.create_note(&root.id, AddPosition::AsChild, "TaTask").unwrap();

        ws.run_tree_action(&task_id, "Mark Done").unwrap();

        let updated = ws.get_note(&task_id).unwrap();
        assert_eq!(updated.title, "Done Task");
        assert_eq!(
            updated.fields.get("status"),
            Some(&FieldValue::Text("done".into()))
        );
    }

    #[test]
    fn test_tree_action_nested_create_builds_subtree() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        ws.create_user_script(r#"
// @name: NestedCreate
schema("TaSprint", #{ version: 1, fields: [] });
schema("TaSubTask", #{ version: 1, fields: [] });
register_menu("Add Sprint With Task", ["TaSprint"], |sprint| {
    let child_sprint = create_child(sprint.id, "TaSprint");
    set_title(child_sprint.id, "Child Sprint");
    let task = create_child(child_sprint.id, "TaSubTask");
    set_title(task.id, "Sprint Task");
    commit();
});
        "#).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let sprint_id = ws.create_note(&root.id, AddPosition::AsChild, "TaSprint").unwrap();

        ws.run_tree_action(&sprint_id, "Add Sprint With Task").unwrap();

        // The child sprint should be under sprint_id
        let sprint_children = ws.get_children(&sprint_id).unwrap();
        assert_eq!(sprint_children.len(), 1, "one child sprint expected");
        assert_eq!(sprint_children[0].title, "Child Sprint");

        // The task should be under the child sprint
        let task_children = ws.get_children(&sprint_children[0].id).unwrap();
        assert_eq!(task_children.len(), 1, "one task expected under child sprint");
        assert_eq!(task_children[0].title, "Sprint Task");
    }

    #[test]
    fn test_tree_action_error_rolls_back_all_writes() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        ws.create_user_script(r#"
// @name: ErrorAction
schema("TaErrFolder", #{ version: 1, fields: [] });
schema("TaErrItem", #{ version: 1, fields: [] });
register_menu("Create Then Fail", ["TaErrFolder"], |folder| {
    let item = create_child(folder.id, "TaErrItem");
    set_title(item.id, "Orphan");
    throw "deliberate error";
});
        "#).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let folder_id = ws.create_note(&root.id, AddPosition::AsChild, "TaErrFolder").unwrap();

        let result = ws.run_tree_action(&folder_id, "Create Then Fail");
        assert!(result.is_err(), "action should propagate the thrown error");

        // No note should have been created — the creates are not applied when the action errors
        let children = ws.get_children(&folder_id).unwrap();
        assert_eq!(children.len(), 0, "rollback: no child note should exist");
    }

    #[test]
    fn test_tree_action_create_child_gated() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.create_user_script(r#"
// @name: TAGated
schema("TAFolder", #{ version: 1, fields: [] });
schema("TAItem", #{ version: 1,
    fields: [
        #{ name: "value", type: "text", required: false },
    ],
});
register_menu("Add Item", ["TAFolder"], |note| {
    let child = create_child(note.id, "TAItem");
    set_title(child.id, "New Item");
    set_field(child.id, "value", "default");
    commit();
});
        "#).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        let folder_id = ws.create_note(&root.id, AddPosition::AsChild, "TAFolder").unwrap();
        ws.run_tree_action(&folder_id, "Add Item").unwrap();
        let children = ws.get_children(&folder_id).unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].title, "New Item");
        assert_eq!(
            children[0].fields.get("value"),
            Some(&FieldValue::Text("default".into()))
        );
    }

    #[test]
    fn test_note_tags_round_trip() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        assert!(root.tags.is_empty());

        ws.update_note_tags(&root.id, vec!["rust".into(), "design".into()]).unwrap();
        let note = ws.get_note(&root.id).unwrap();
        assert_eq!(note.tags, vec!["design", "rust"]); // sorted
    }

    #[test]
    fn test_get_all_tags_empty() {
        let temp = NamedTempFile::new().unwrap();
        let ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        assert!(ws.get_all_tags().unwrap().is_empty());
    }

    #[test]
    fn test_get_all_tags_sorted_distinct() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let child_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        ws.update_note_tags(&root.id, vec!["rust".into(), "design".into()]).unwrap();
        ws.update_note_tags(&child_id, vec!["rust".into(), "testing".into()]).unwrap();
        let tags = ws.get_all_tags().unwrap();
        assert_eq!(tags, vec!["design", "rust", "testing"]);
    }

    #[test]
    fn test_get_notes_for_tag() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let child_id = ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        ws.update_note_tags(&root.id, vec!["rust".into()]).unwrap();
        ws.update_note_tags(&child_id, vec!["design".into()]).unwrap();

        let rust_notes = ws.get_notes_for_tag(&["rust".into()]).unwrap();
        assert_eq!(rust_notes.len(), 1);
        assert_eq!(rust_notes[0].id, root.id);

        // OR logic: both notes returned when both tags queried
        let both = ws.get_notes_for_tag(&["rust".into(), "design".into()]).unwrap();
        assert_eq!(both.len(), 2);

        // Unknown tag returns empty
        let none = ws.get_notes_for_tag(&["unknown".into()]).unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn test_update_note_tags_replaces_existing() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        ws.update_note_tags(&root.id, vec!["old".into()]).unwrap();
        ws.update_note_tags(&root.id, vec!["new".into()]).unwrap();
        let tags = ws.get_all_tags().unwrap();
        assert_eq!(tags, vec!["new"]); // "old" removed
    }

    #[test]
    fn test_update_note_tags_normalises() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        ws.update_note_tags(&root.id, vec!["  Rust  ".into(), "RUST".into(), "rust".into()]).unwrap();
        let note = ws.get_note(&root.id).unwrap();
        assert_eq!(note.tags, vec!["rust"]); // deduped, lowercased, trimmed
    }

    // ── note_links helpers ────────────────────────────────────────────────────

    /// Creates a workspace with a single user script loaded (for schema setup).
    /// Returns the workspace ready to use.
    fn create_test_workspace_with_schema(schema_script: &str) -> Workspace {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        // Wrap the bare schema call in the required front matter so create_user_script accepts it.
        let source = format!("// @name: TestSchema\n{schema_script}");
        ws.create_user_script(&source).unwrap();
        // Leak the tempfile so the DB file stays alive for the duration of the test.
        std::mem::forget(temp);
        ws
    }

    /// Creates a new root-level note of `note_type` and returns the full `Note`.
    fn create_note_with_type(ws: &mut Workspace, note_type: &str) -> Note {
        let id = ws.create_note_root(note_type).unwrap();
        ws.get_note(&id).unwrap()
    }

    // ── note_links tests ──────────────────────────────────────────────────────

    #[test]
    fn test_sync_note_links_inserts_row() {
        let mut ws = create_test_workspace_with_schema(
            r#"schema("LinkTestType", #{ version: 1, fields: [#{ name: "link", type: "note_link" }] })"#
        );
        let target = create_note_with_type(&mut ws, "LinkTestType");
        let source = create_note_with_type(&mut ws, "LinkTestType");

        let mut fields = BTreeMap::new();
        fields.insert("link".into(), FieldValue::NoteLink(Some(target.id.clone())));
        ws.update_note(&source.id, source.title.clone(), fields).unwrap();

        let conn = ws.connection();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM note_links WHERE source_id = ?1 AND target_id = ?2",
            [&source.id, &target.id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_sync_note_links_removes_row_when_cleared() {
        let mut ws = create_test_workspace_with_schema(
            r#"schema("LinkTestType", #{ version: 1, fields: [#{ name: "link", type: "note_link" }] })"#
        );
        let target = create_note_with_type(&mut ws, "LinkTestType");
        let source = create_note_with_type(&mut ws, "LinkTestType");

        let mut fields = BTreeMap::new();
        fields.insert("link".into(), FieldValue::NoteLink(Some(target.id.clone())));
        ws.update_note(&source.id, source.title.clone(), fields).unwrap();

        let mut fields2 = BTreeMap::new();
        fields2.insert("link".into(), FieldValue::NoteLink(None));
        ws.update_note(&source.id, source.title.clone(), fields2).unwrap();

        let conn = ws.connection();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM note_links WHERE source_id = ?1",
            [&source.id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_clear_links_to_nulls_field_in_source_note() {
        let mut ws = create_test_workspace_with_schema(
            r#"schema("LinkTestType", #{ version: 1, fields: [#{ name: "link", type: "note_link" }] })"#
        );
        let target = create_note_with_type(&mut ws, "LinkTestType");
        let source = create_note_with_type(&mut ws, "LinkTestType");

        let mut fields = BTreeMap::new();
        fields.insert("link".into(), FieldValue::NoteLink(Some(target.id.clone())));
        ws.update_note(&source.id, source.title.clone(), fields).unwrap();

        ws.clear_links_to(&target.id).unwrap();

        let updated_source = ws.get_note(&source.id).unwrap();
        assert!(matches!(
            updated_source.fields.get("link").unwrap(),
            FieldValue::NoteLink(None)
        ));

        let conn = ws.connection();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM note_links WHERE target_id = ?1",
            [&target.id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_delete_note_nulls_links_in_other_notes() {
        let mut ws = create_test_workspace_with_schema(
            r#"schema("LinkTestType", #{ version: 1, fields: [#{ name: "link", type: "note_link" }] })"#
        );
        let target = create_note_with_type(&mut ws, "LinkTestType");
        let source = create_note_with_type(&mut ws, "LinkTestType");

        let mut fields = BTreeMap::new();
        fields.insert("link".into(), FieldValue::NoteLink(Some(target.id.clone())));
        ws.update_note(&source.id, source.title.clone(), fields).unwrap();

        ws.delete_note(&target.id, DeleteStrategy::DeleteAll).unwrap();

        let updated_source = ws.get_note(&source.id).unwrap();
        assert!(matches!(
            updated_source.fields.get("link").unwrap(),
            FieldValue::NoteLink(None)
        ));
    }

    #[test]
    fn test_delete_note_recursive_clears_links_for_entire_subtree() {
        let mut ws = create_test_workspace_with_schema(
            r#"schema("LinkTestType", #{ version: 1, fields: [#{ name: "link", type: "note_link" }] })"#
        );
        let parent = create_note_with_type(&mut ws, "LinkTestType");
        let child_id = ws.create_note(&parent.id, AddPosition::AsChild, "LinkTestType").unwrap();
        let child = ws.get_note(&child_id).unwrap();
        let observer = create_note_with_type(&mut ws, "LinkTestType");

        let mut fields = BTreeMap::new();
        fields.insert("link".into(), FieldValue::NoteLink(Some(child.id.clone())));
        ws.update_note(&observer.id, observer.title.clone(), fields).unwrap();

        ws.delete_note(&parent.id, DeleteStrategy::DeleteAll).unwrap();

        let updated_observer = ws.get_note(&observer.id).unwrap();
        assert!(matches!(
            updated_observer.fields.get("link").unwrap(),
            FieldValue::NoteLink(None)
        ));
    }

    // ── get_notes_with_link tests ─────────────────────────────────────────────

    #[test]
    fn test_get_notes_with_link_returns_linking_notes() {
        let mut ws = create_test_workspace_with_schema(
            r#"schema("LinkTestType", #{ version: 1, fields: [#{ name: "link", type: "note_link" }] })"#
        );
        let target = create_note_with_type(&mut ws, "LinkTestType");
        let source1 = create_note_with_type(&mut ws, "LinkTestType");
        let source2 = create_note_with_type(&mut ws, "LinkTestType");

        for source in [&source1, &source2] {
            let mut fields = BTreeMap::new();
            fields.insert("link".into(), FieldValue::NoteLink(Some(target.id.clone())));
            ws.update_note(&source.id, source.title.clone(), fields.clone()).unwrap();
        }

        let results = ws.get_notes_with_link(&target.id).unwrap();
        assert_eq!(results.len(), 2);
        let result_ids: Vec<&str> = results.iter().map(|n| n.id.as_str()).collect();
        assert!(result_ids.contains(&source1.id.as_str()));
        assert!(result_ids.contains(&source2.id.as_str()));
    }

    #[test]
    fn test_get_notes_with_link_returns_empty_for_unlinked_note() {
        let mut ws = create_test_workspace_with_schema(
            r#"schema("LinkTestType", #{ version: 1, fields: [#{ name: "link", type: "note_link" }] })"#
        );
        let note = create_note_with_type(&mut ws, "LinkTestType");
        let results = ws.get_notes_with_link(&note.id).unwrap();
        assert!(results.is_empty());
    }

    // ── search_notes tests ────────────────────────────────────────────────────

    #[test]
    fn test_search_notes_matches_title() {
        let mut ws = create_test_workspace_with_schema(
            r#"schema("LinkTestType", #{ version: 1, fields: [] })"#
        );
        let note = create_note_with_type(&mut ws, "LinkTestType");
        ws.update_note(&note.id, "Fix login bug".into(), BTreeMap::new()).unwrap();

        let results = ws.search_notes("login", None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, note.id);
    }

    #[test]
    fn test_search_notes_filters_by_target_type() {
        let mut ws = create_test_workspace_with_schema(
            r#"schema("TaskNote", #{ version: 1, fields: [] }); schema("OtherNote", #{ version: 1, fields: [] })"#
        );
        let task = create_note_with_type(&mut ws, "TaskNote");
        ws.update_note(&task.id, "login task".into(), BTreeMap::new()).unwrap();
        let other = create_note_with_type(&mut ws, "OtherNote");
        ws.update_note(&other.id, "login other".into(), BTreeMap::new()).unwrap();

        let results = ws.search_notes("login", Some("TaskNote")).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, task.id);
    }

    #[test]
    fn test_search_notes_matches_text_fields() {
        let mut ws = create_test_workspace_with_schema(
            r#"schema("ContactNote", #{ version: 1, fields: [#{ name: "email", type: "email" }] })"#
        );
        let c = create_note_with_type(&mut ws, "ContactNote");
        let mut fields = BTreeMap::new();
        fields.insert("email".into(), FieldValue::Email("alice@example.com".into()));
        ws.update_note(&c.id, "Alice".into(), fields).unwrap();

        let results = ws.search_notes("alice@example", None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, c.id);
    }

    // ── rebuild_note_links_index tests ────────────────────────────────────────

    #[test]
    fn test_rebuild_note_links_index_repopulates_from_fields_json() {
        let mut ws = create_test_workspace_with_schema(
            r#"schema("LinkTestType", #{ version: 1, fields: [#{ name: "link", type: "note_link" }] })"#
        );
        let target = create_note_with_type(&mut ws, "LinkTestType");
        let source = create_note_with_type(&mut ws, "LinkTestType");

        let mut fields = BTreeMap::new();
        fields.insert("link".into(), FieldValue::NoteLink(Some(target.id.clone())));
        ws.update_note(&source.id, source.title.clone(), fields).unwrap();

        // Manually wipe the index
        ws.connection().execute("DELETE FROM note_links", []).unwrap();

        // Rebuild
        ws.rebuild_note_links_index().unwrap();

        let conn = ws.connection();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM note_links WHERE source_id = ?1 AND target_id = ?2",
            [&source.id, &target.id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn operations_log_always_records_create_note() {
        // The operation log is always active — every mutation must be recorded.
        let temp = tempfile::NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        let ops = ws.list_operations(None, None, None).unwrap();
        assert!(!ops.is_empty(), "operation log must always be active");
        let create_ops: Vec<_> = ops.iter().filter(|o| o.operation_type == "CreateNote").collect();
        assert!(!create_ops.is_empty(), "CreateNote must be recorded in always-on log");
    }

    #[test]
    fn test_workspace_has_attachment_key_when_encrypted() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let ws = Workspace::create(&db_path, "hunter2", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        assert!(ws.attachment_key().is_some(), "Encrypted workspace must have attachment_key");
    }

    #[test]
    fn test_workspace_has_no_attachment_key_when_unencrypted() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let ws = Workspace::create(&db_path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        assert!(ws.attachment_key().is_none(), "Unencrypted workspace must have no attachment_key");
    }

    #[test]
    fn test_workspace_creates_attachments_directory() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        Workspace::create(&db_path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        assert!(dir.path().join("attachments").is_dir());
    }

    #[test]
    fn test_workspace_attachment_key_stable_across_open() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let ws1 = Workspace::create(&db_path, "mypass", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let key1 = ws1.attachment_key().unwrap().clone();
        drop(ws1);
        let ws2 = Workspace::open(&db_path, "mypass", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let key2 = ws2.attachment_key().unwrap();
        assert_eq!(key1, *key2, "Key must be derived deterministically from password + workspace_id");
    }

    #[test]
    fn test_get_set_workspace_metadata() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Fresh workspace returns default (no error)
        let initial = ws.get_workspace_metadata().unwrap();
        assert!(initial.author_name.is_none());
        assert!(initial.tags.is_empty());

        // Set and read back
        let meta = WorkspaceMetadata {
            version: 1,
            author_name: Some("Bob".to_string()),
            author_org: None,
            homepage_url: None,
            description: Some("My workspace".to_string()),
            license: Some("CC BY 4.0".to_string()),
            license_url: None,
            language: Some("en".to_string()),
            tags: vec!["productivity".to_string()],
        };
        ws.set_workspace_metadata(&meta).unwrap();

        let restored = ws.get_workspace_metadata().unwrap();
        assert_eq!(restored.author_name.as_deref(), Some("Bob"));
        assert_eq!(restored.description.as_deref(), Some("My workspace"));
        assert_eq!(restored.license.as_deref(), Some("CC BY 4.0"));
        assert_eq!(restored.language.as_deref(), Some("en"));
        assert_eq!(restored.tags, vec!["productivity"]);
        assert!(restored.author_org.is_none());

        // Overwrite with new values
        let meta2 = WorkspaceMetadata {
            version: 1,
            author_name: Some("Alice".to_string()),
            ..Default::default()
        };
        ws.set_workspace_metadata(&meta2).unwrap();
        let updated = ws.get_workspace_metadata().unwrap();
        assert_eq!(updated.author_name.as_deref(), Some("Alice"));
        assert!(updated.description.is_none());
    }

    #[test]
    fn test_attach_file_stores_metadata_and_file() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let mut ws = Workspace::create(&db_path, "testpass", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let notes = ws.list_all_notes().unwrap();
        let root_id = &notes[0].id;

        let data = b"hello attachment";
        let meta = ws.attach_file(root_id, "test.txt", Some("text/plain"), data).unwrap();
        assert_eq!(meta.filename, "test.txt");
        assert_eq!(meta.size_bytes, data.len() as i64);

        let enc_path = dir.path().join("attachments").join(format!("{}.enc", meta.id));
        assert!(enc_path.exists(), "Encrypted file must exist on disk");
    }

    #[test]
    fn test_get_attachment_bytes_decrypts_correctly() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let mut ws = Workspace::create(&db_path, "testpass", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.list_all_notes().unwrap()[0].id.clone();

        let data = b"secret file content";
        let meta = ws.attach_file(&root_id, "doc.txt", None, data).unwrap();
        let recovered = ws.get_attachment_bytes(&meta.id).unwrap();
        assert_eq!(recovered, data as &[u8]);
    }

    #[test]
    fn test_get_attachments_returns_metadata_list() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let mut ws = Workspace::create(&db_path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.list_all_notes().unwrap()[0].id.clone();

        ws.attach_file(&root_id, "a.pdf", None, b"data a").unwrap();
        ws.attach_file(&root_id, "b.pdf", None, b"data b").unwrap();

        let attachments = ws.get_attachments(&root_id).unwrap();
        assert_eq!(attachments.len(), 2);
        let names: Vec<&str> = attachments.iter().map(|a| a.filename.as_str()).collect();
        assert!(names.contains(&"a.pdf"));
        assert!(names.contains(&"b.pdf"));
    }

    #[test]
    fn test_delete_attachment_soft_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let mut ws = Workspace::create(&db_path, "testpass", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.list_all_notes().unwrap()[0].id.clone();

        let meta = ws.attach_file(&root_id, "bye.txt", None, b"temp").unwrap();
        let enc_path = dir.path().join("attachments").join(format!("{}.enc", meta.id));
        let trash_path = dir.path().join("attachments").join(format!("{}.enc.trash", meta.id));
        assert!(enc_path.exists());

        // Soft-delete: file moves to .enc.trash, DB row removed.
        // Attachment deletions do NOT go on the main undo stack (to avoid interfering
        // with note-edit undo/redo which uses the same Cmd+Z shortcut).
        ws.delete_attachment(&meta.id).unwrap();
        assert!(!enc_path.exists(), ".enc must be gone after soft-delete");
        assert!(trash_path.exists(), ".enc.trash must exist after soft-delete");
        assert!(ws.get_attachments(&root_id).unwrap().is_empty(), "DB row must be gone");
        assert!(!ws.can_undo(), "attachment deletion must NOT push to main undo stack");
    }

    #[test]
    fn test_attach_file_enforces_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let mut ws = Workspace::create(&db_path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.list_all_notes().unwrap()[0].id.clone();

        ws.set_attachment_max_size_bytes(Some(10)).unwrap();
        let big_data = vec![0u8; 100];
        let result = ws.attach_file(&root_id, "big.bin", None, &big_data);
        assert!(matches!(result, Err(KrillnotesError::AttachmentTooLarge { .. })));
    }

    #[test]
    fn test_update_note_cleans_up_replaced_file_field() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let mut ws = Workspace::create(&db_path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Use the root TextNote that is always created on workspace init.
        let root_id = ws.list_all_notes().unwrap()[0].id.clone();

        // Attach a first file to the note.
        let meta1 = ws.attach_file(&root_id, "a.png", Some("image/png"), b"fake_bytes_1").unwrap();

        // Set the File field to point at the first attachment.
        let mut fields = ws.get_note(&root_id).unwrap().fields.clone();
        fields.insert("photo".to_string(), FieldValue::File(Some(meta1.id.clone())));
        ws.update_note(&root_id, "Test".to_string(), fields).unwrap();

        // Attach a second file and replace the field value with it.
        let meta2 = ws.attach_file(&root_id, "b.png", Some("image/png"), b"fake_bytes_2").unwrap();
        let mut fields2 = ws.get_note(&root_id).unwrap().fields.clone();
        fields2.insert("photo".to_string(), FieldValue::File(Some(meta2.id.clone())));
        ws.update_note(&root_id, "Test".to_string(), fields2).unwrap();

        // The first attachment must have been deleted when the field was replaced.
        let result = ws.get_attachment_bytes(&meta1.id);
        assert!(result.is_err(), "old attachment should have been deleted when field value was replaced");

        // The second attachment must still be readable.
        assert!(ws.get_attachment_bytes(&meta2.id).is_ok(), "new attachment should still exist");
    }

    #[test]
    fn test_operation_log_always_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.create_note_root("TextNote").unwrap();

        // The operation log should record the CreateNote even without sync.
        let ops = ws.list_operations(None, None, None).unwrap();
        assert!(!ops.is_empty(), "operation log must always be active");
        assert_eq!(ops[0].operation_type, "CreateNote");
        let _ = root_id;
    }

    #[test]
    fn test_update_note_cleans_up_cleared_file_field() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let mut ws = Workspace::create(&db_path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let root_id = ws.list_all_notes().unwrap()[0].id.clone();

        // Attach a file and store its UUID in a File field.
        let meta = ws.attach_file(&root_id, "x.png", Some("image/png"), b"fake").unwrap();
        let mut fields = ws.get_note(&root_id).unwrap().fields.clone();
        fields.insert("photo".to_string(), FieldValue::File(Some(meta.id.clone())));
        ws.update_note(&root_id, "Test".to_string(), fields).unwrap();

        // Clear the File field (set to None) — the attachment should be deleted.
        let mut fields2 = ws.get_note(&root_id).unwrap().fields.clone();
        fields2.insert("photo".to_string(), FieldValue::File(None));
        ws.update_note(&root_id, "Test".to_string(), fields2).unwrap();

        let result = ws.get_attachment_bytes(&meta.id);
        assert!(result.is_err(), "attachment should have been deleted when field was cleared");
    }

    #[test]
    fn test_can_undo_initially_false() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        assert!(!ws.can_undo());
        assert!(!ws.can_redo());
    }

    #[test]
    fn test_collect_subtree_notes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.create_note_root("TextNote").unwrap();
        let child_id = ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        let _grandchild = ws.create_note(&child_id, AddPosition::AsChild, "TextNote").unwrap();

        let notes = ws.collect_subtree_notes(&root_id).unwrap();
        assert_eq!(notes.len(), 3);
        // Root must be first.
        assert_eq!(notes[0].id, root_id);
    }

    #[test]
    fn test_undo_group_collapses_to_one_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.create_note_root("TextNote").unwrap();
        // Clear the undo entry from root creation
        ws.undo_stack.clear();

        ws.begin_undo_group();
        ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        ws.end_undo_group();

        assert_eq!(ws.undo_stack.len(), 1, "group must collapse to one entry");
        match &ws.undo_stack[0].inverse {
            RetractInverse::Batch(items) => assert_eq!(items.len(), 2),
            _ => panic!("expected Batch"),
        }
    }

    #[test]
    fn test_undo_create_note() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.create_note_root("TextNote").unwrap();
        ws.undo_stack.clear(); // ignore root creation

        let child_id = ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        assert!(ws.can_undo());

        let result = ws.undo().unwrap();
        assert_eq!(result.affected_note_id, None);
        assert!(ws.get_note(&child_id).is_err(), "note must be gone after undo");
    }

    #[test]
    fn test_undo_update_note_restores_old_title() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.create_note_root("TextNote").unwrap();
        ws.undo_stack.clear();

        ws.update_note(&root_id, "New Title".into(), BTreeMap::new()).unwrap();
        assert!(ws.can_undo());

        // Check undo entry inverse
        match &ws.undo_stack[0].inverse {
            RetractInverse::NoteRestore { old_title, .. } => {
                // The original title from create_note_root should be preserved.
                assert_ne!(old_title, "New Title");
            }
            _ => panic!("expected NoteRestore"),
        }
    }

    #[test]
    fn test_undo_delete_note_restores_subtree() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.create_note_root("TextNote").unwrap();
        let child_id = ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        ws.undo_stack.clear();

        ws.delete_note_recursive(&child_id).unwrap();
        assert!(ws.can_undo());
        assert!(ws.get_note(&child_id).is_err(), "note gone after delete");

        // Undo entry must be SubtreeRestore.
        assert!(matches!(ws.undo_stack[0].inverse, RetractInverse::SubtreeRestore { .. }));
    }

    #[test]
    fn test_undo_move_note_restores_position() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.create_note_root("TextNote").unwrap();
        let child_id = ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        let sibling_id = ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        ws.undo_stack.clear();

        let old_note = ws.get_note(&sibling_id).unwrap();
        ws.move_note(&sibling_id, Some(&child_id), 0.0).unwrap();

        assert!(ws.can_undo());
        match &ws.undo_stack[0].inverse {
            RetractInverse::PositionRestore { note_id, old_parent_id, old_position } => {
                assert_eq!(note_id, &sibling_id);
                assert_eq!(old_parent_id, &old_note.parent_id);
                assert_eq!(*old_position, old_note.position);
            }
            _ => panic!("expected PositionRestore"),
        }
    }

    #[test]
    fn test_undo_delete_script_restores_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let src = "// @name: TestScript\n// @description: desc\n";
        let (script, _) = ws.create_user_script(src).unwrap();
        ws.script_undo_stack.clear();

        ws.delete_user_script(&script.id).unwrap();
        // Script operations now land on the separate script_undo_stack.
        assert!(ws.can_script_undo());
        assert!(!ws.can_undo(), "note undo stack must be unaffected by script ops");
        assert!(matches!(ws.script_undo_stack[0].inverse, RetractInverse::ScriptRestore { .. }));
    }

    #[test]
    fn test_undo_redo_create_note_cycle() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.create_note_root("TextNote").unwrap();
        ws.undo_stack.clear();

        let child_id = ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        assert!(ws.can_undo());
        assert!(!ws.can_redo());

        ws.undo().unwrap();
        assert!(ws.get_note(&child_id).is_err(), "note removed by undo");
        assert!(!ws.can_undo());
        assert!(ws.can_redo());

        ws.redo().unwrap();
        assert!(ws.can_undo());
        assert!(!ws.can_redo());
        // Note should be back — look it up by parent.
        let children = ws.get_children(&root_id).unwrap();
        assert_eq!(children.len(), 1);
    }

    #[test]
    fn test_undo_delete_note_full_cycle() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.create_note_root("TextNote").unwrap();
        let child_id = ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        ws.undo_stack.clear();

        ws.delete_note_recursive(&child_id).unwrap();
        ws.undo().unwrap();

        // Note must be back.
        assert!(ws.get_note(&child_id).is_ok());
    }

    #[test]
    fn test_tree_action_collapses_to_one_undo_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root_id = ws.create_note_root("TextNote").unwrap();
        ws.undo_stack.clear();

        // Simulate what run_tree_action does internally.
        ws.begin_undo_group();
        ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        ws.end_undo_group();

        assert_eq!(ws.undo_stack.len(), 1, "three creates must collapse to one undo step");
    }

    #[test]
    fn test_undo_limit_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.set_undo_limit(10).unwrap();
        drop(ws);

        let ws2 = Workspace::open(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        assert_eq!(ws2.undo_limit, 10);
    }

    #[test]
    fn test_undo_limit_clamp_and_trim() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        ws.set_undo_limit(0).unwrap();
        assert_eq!(ws.get_undo_limit(), 1);

        ws.set_undo_limit(9999).unwrap();
        assert_eq!(ws.get_undo_limit(), 500);

        // Grow the undo stack to 5 entries, then shrink
        ws.set_undo_limit(500).unwrap();
        let root_id = ws.create_note_root("TextNote").unwrap();
        ws.undo_stack.clear();
        for _ in 0..5 {
            ws.create_note(&root_id, AddPosition::AsChild, "TextNote").unwrap();
        }
        assert_eq!(ws.undo_stack.len(), 5);
        ws.set_undo_limit(3).unwrap();
        assert_eq!(ws.undo_stack.len(), 3, "oldest entries should have been dropped");
    }

    #[test]
    fn test_undo_redo_update_script_full_cycle() {
        // Regression: build_redo_inverse(ScriptRestore) used to always return
        // DeleteScript, causing redo to delete the script instead of re-applying
        // the update.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let src_v1 = "// @name: CycleScript\n// @description: v1\nlet x = 1;";
        let (script, _) = ws.create_user_script(src_v1).unwrap();
        ws.script_undo_stack.clear();

        let src_v2 = "// @name: CycleScript\n// @description: v2\nlet x = 2;";
        ws.update_user_script(&script.id, src_v2).unwrap();

        // Script undo: should restore v1.
        ws.script_undo().unwrap();
        let after_undo = ws.get_user_script(&script.id).unwrap();
        assert_eq!(after_undo.source_code, src_v1, "script_undo should restore v1");
        assert!(ws.can_script_redo());

        // Script redo: should restore v2 (not delete the script!).
        ws.script_redo().unwrap();
        let after_redo = ws.get_user_script(&script.id).unwrap();
        assert_eq!(after_redo.source_code, src_v2, "script_redo should re-apply v2");

        // Script undo again: back to v1.
        ws.script_undo().unwrap();
        let final_state = ws.get_user_script(&script.id).unwrap();
        assert_eq!(final_state.source_code, src_v1, "second script_undo should restore v1 again");
    }

    #[test]
    fn test_undo_redo_create_script_full_cycle() {
        // Undo of CreateScript (DeleteScript inverse) should be able to re-create
        // the script with its real content on redo, not an empty placeholder.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let src = "// @name: RedoScript\n// @description: test\nlet y = 42;";
        let (script, _) = ws.create_user_script(src).unwrap();
        ws.script_undo_stack.clear();
        // Re-push just the create entry we care about onto the script stack.
        ws.push_script_undo(UndoEntry {
            retracted_ids: vec!["test-op".into()],
            inverse: RetractInverse::DeleteScript { script_id: script.id.clone() },
            propagate: true,
        });

        // Script undo: script deleted.
        ws.script_undo().unwrap();
        assert!(ws.get_user_script(&script.id).is_err(), "script deleted by script_undo");

        // Script redo: script recreated with real content.
        ws.script_redo().unwrap();
        let after_redo = ws.get_user_script(&script.id).unwrap();
        assert_eq!(after_redo.source_code, src, "script_redo must restore real source, not empty");
    }

    #[test]
    fn test_write_info_json_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let ws = Workspace::create(&db_path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.write_info_json().unwrap();

        let info_path = dir.path().join("info.json");
        assert!(info_path.exists(), "info.json should be created");

        let content = std::fs::read_to_string(&info_path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(v["created_at"].is_number());
        assert_eq!(v["note_count"].as_u64().unwrap(), 1);
        assert_eq!(v["attachment_count"].as_u64().unwrap(), 0);
    }

    #[test]
    fn test_write_info_json_counts_notes() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let mut ws = Workspace::create(&db_path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        let root = ws.list_all_notes().unwrap()[0].clone();
        ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        ws.write_info_json().unwrap();

        let content = std::fs::read_to_string(dir.path().join("info.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["note_count"].as_u64().unwrap(), 3);
    }

    #[test]
    fn test_info_json_written_on_create() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        Workspace::create(&db_path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        assert!(dir.path().join("info.json").exists(), "info.json must exist after create");
    }

    #[test]
    fn test_info_json_written_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        Workspace::create(&db_path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        std::fs::remove_file(dir.path().join("info.json")).unwrap(); // remove it
        Workspace::open(&db_path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        assert!(dir.path().join("info.json").exists(), "info.json must be rewritten on open");
    }

    // ── HLC-specific tests ────────────────────────────────────────────────────

    #[test]
    fn test_hlc_counter_increments_for_rapid_ops() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hlc.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Create two notes in rapid succession.
        ws.create_note_root("TextNote").unwrap();
        ws.create_note_root("TextNote").unwrap();

        let ops = ws.list_operations(None, None, None).unwrap();
        assert!(ops.len() >= 2, "at least two operations must be logged");

        // Every logged timestamp must be a valid non-zero wall clock value.
        for op in &ops {
            assert!(op.timestamp_wall_ms > 0, "wall_ms must be non-zero");
        }

        // All operation IDs must be unique — HLC must not produce duplicate entries.
        let unique_ids: std::collections::HashSet<&str> =
            ops.iter().map(|o| o.operation_id.as_str()).collect();
        assert_eq!(unique_ids.len(), ops.len(), "all operation_ids must be unique");
    }

    #[test]
    fn test_set_tags_op_logged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tags_log.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();

        ws.update_note_tags(&root.id, vec!["crdt".into(), "hlc".into()]).unwrap();

        let ops = ws.list_operations(None, None, None).unwrap();
        let has_set_tags = ops.iter().any(|o| o.operation_type == "SetTags");
        assert!(has_set_tags, "SetTags operation must appear in the log after update_note_tags");
    }

    #[test]
    fn test_update_note_op_logged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update_note_log.krillnotes");
        let mut ws = Workspace::create(&path, "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();

        ws.update_note_title(&root.id, "HLC Title".to_string()).unwrap();

        // update_note_title logs an UpdateNote operation (not UpdateField).
        let ops = ws.list_operations(None, None, None).unwrap();
        let has_title_update = ops.iter().any(|o| o.operation_type == "UpdateNote");
        assert!(
            has_title_update,
            "UpdateNote operation must appear in the log after update_note_title"
        );
    }

    #[test]
    fn test_on_save_gated_model() {
        let mut ws = create_test_workspace_with_schema(r#"
            schema("GatedTest", #{ version: 1,
                fields: [
                    #{ name: "body", type: "text", required: false },
                ],
                on_save: |note| {
                    set_title(note.id, "Computed: " + note.fields["body"]);
                    commit();
                },
            });
        "#);

        let note_id = ws.create_note_root("GatedTest").unwrap();
        let mut fields = BTreeMap::new();
        fields.insert("body".to_string(), FieldValue::Text("hello".to_string()));
        let updated = ws.update_note(&note_id, "ignored".to_string(), fields).unwrap();
        assert_eq!(updated.title, "Computed: hello");
    }

    #[test]
    fn test_old_style_on_save_raises_error() {
        let mut ws = create_test_workspace_with_schema(r#"
            schema("OldStyle", #{ version: 1,
                fields: [
                    #{ name: "body", type: "text", required: false },
                ],
                on_save: |note| {
                    note.title = "Old Style";
                    note
                },
            });
        "#);

        let note_id = ws.create_note_root("OldStyle").unwrap();
        let result = ws.update_note(&note_id, "test".to_string(), BTreeMap::new());
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("set_field") || err_msg.contains("gated") || err_msg.contains("old"),
            "Expected migration error, got: {err_msg}"
        );
    }

    // ── save_note_with_pipeline ───────────────────────────────────────────────

    #[test]
    fn test_save_pipeline_success() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.create_user_script(r#"
// @name: PipelineOk
schema("PipeItem", #{ version: 1,
    fields: [
        #{ name: "value", type: "text", required: false },
    ],
    on_save: |note| { commit(); }
});
        "#).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let note_id = ws.create_note(&root.id, AddPosition::AsChild, "PipeItem").unwrap();
        let mut fields = BTreeMap::new();
        fields.insert("value".to_string(), FieldValue::Text("hello".into()));
        let result = ws.save_note_with_pipeline(&note_id, "My Item".to_string(), fields).unwrap();
        assert!(matches!(result, SaveResult::Ok(_)), "expected Ok, got: {:?}", result);
    }

    #[test]
    fn test_save_pipeline_validation_error() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.create_user_script(r#"
// @name: PipelineValidate
schema("RatedItem", #{ version: 1,
    fields: [
        #{
            name: "score", type: "number", required: false,
            validate: |v| if v < 0.0 { "Must be positive" } else { () },
        },
    ],
});
        "#).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let note_id = ws.create_note(&root.id, AddPosition::AsChild, "RatedItem").unwrap();
        let mut fields = BTreeMap::new();
        fields.insert("score".to_string(), FieldValue::Number(-1.0));
        let result = ws.save_note_with_pipeline(&note_id, "Item".to_string(), fields).unwrap();
        match result {
            SaveResult::ValidationErrors { field_errors, .. } => {
                assert!(field_errors.contains_key("score"), "expected score error");
            }
            other => panic!("expected ValidationErrors, got: {:?}", other),
        }
    }

    #[test]
    fn test_save_pipeline_reject_error() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.create_user_script(r#"
// @name: PipelineReject
schema("RejectItem", #{ version: 1,
    fields: [],
    on_save: |note| {
        reject("Always rejected");
        commit();
    }
});
        "#).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let note_id = ws.create_note(&root.id, AddPosition::AsChild, "RejectItem").unwrap();
        let result = ws.save_note_with_pipeline(&note_id, "Item".to_string(), BTreeMap::new()).unwrap();
        match result {
            SaveResult::ValidationErrors { note_errors, .. } => {
                assert!(!note_errors.is_empty(), "expected note_errors from reject()");
            }
            other => panic!("expected ValidationErrors, got: {:?}", other),
        }
    }

    /// Integration test: full pipeline with field groups, conditional visibility, validate
    /// closure, and note-level reject.
    ///
    /// Schema:
    ///   - top-level field "type" (select: ["A", "B"])
    ///   - field_group "B Details" visible only when type == "B"
    ///     - field "b_value" (number, required, validate: must be > 0)
    ///   - on_save: reject if type == "B" and b_value > 100
    ///
    /// Tests:
    ///   1. type="A" → success (B Details hidden, b_value not required)
    ///   2. type="B", b_value=-1 → validate error on b_value
    ///   3. type="B", b_value=200 → note-level reject error
    ///   4. type="B", b_value=50 → success
    #[test]
    fn test_full_pipeline_groups_validation_reject() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.create_user_script(r#"
// @name: PipelineGrouped
schema("Grouped", #{ version: 1,
    fields: [
        #{ name: "category", type: "select", options: ["A", "B"] }
    ],
    field_groups: [
        #{
            name: "B Details",
            collapsed: false,
            visible: |fields| fields["category"] == "B",
            fields: [
                #{
                    name: "b_value",
                    type: "number",
                    required: true,
                    validate: |v| if v <= 0.0 { "Must be positive" } else { () }
                }
            ]
        }
    ],
    on_save: |note| {
        if note.fields.category == "B" && note.fields.b_value > 100.0 {
            reject("b_value must be <= 100 for category B");
        }
        commit();
    }
});
        "#).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let note_id = ws.create_note(&root.id, AddPosition::AsChild, "Grouped").unwrap();

        // Test 1: category="A" — succeeds; b_value is in a hidden group, not required
        let mut fields = BTreeMap::new();
        fields.insert("category".to_string(), FieldValue::Text("A".to_string()));
        let result = ws.save_note_with_pipeline(&note_id, "Note".to_string(), fields).unwrap();
        assert!(matches!(result, SaveResult::Ok(_)), "test 1 failed: {:?}", result);

        // Test 2: category="B", b_value=-1 — validate closure error on b_value
        let mut fields = BTreeMap::new();
        fields.insert("category".to_string(), FieldValue::Text("B".to_string()));
        fields.insert("b_value".to_string(), FieldValue::Number(-1.0));
        let result = ws.save_note_with_pipeline(&note_id, "Note".to_string(), fields).unwrap();
        match result {
            SaveResult::ValidationErrors { field_errors, .. } => {
                assert!(field_errors.contains_key("b_value"), "expected b_value error, got: {:?}", field_errors);
            }
            other => panic!("test 2 failed: expected ValidationErrors, got: {:?}", other),
        }

        // Test 3: category="B", b_value=200 — note-level reject
        let mut fields = BTreeMap::new();
        fields.insert("category".to_string(), FieldValue::Text("B".to_string()));
        fields.insert("b_value".to_string(), FieldValue::Number(200.0));
        let result = ws.save_note_with_pipeline(&note_id, "Note".to_string(), fields).unwrap();
        match result {
            SaveResult::ValidationErrors { note_errors, .. } => {
                assert!(!note_errors.is_empty(), "test 3 failed: expected note_errors");
            }
            other => panic!("test 3 failed: expected ValidationErrors, got: {:?}", other),
        }

        // Test 4: category="B", b_value=50 — success
        let mut fields = BTreeMap::new();
        fields.insert("category".to_string(), FieldValue::Text("B".to_string()));
        fields.insert("b_value".to_string(), FieldValue::Number(50.0));
        let result = ws.save_note_with_pipeline(&note_id, "Note".to_string(), fields).unwrap();
        assert!(matches!(result, SaveResult::Ok(_)), "test 4 failed: {:?}", result);
    }

    /// Integration test: tree action that creates a child; the required field is left empty.
    /// The save pipeline should detect the required field and return a ValidationErrors result
    /// (no note is persisted).
    ///
    /// NOTE: tree actions use SaveTransaction internally but do NOT run the full
    /// save_note_with_pipeline (they commit the transaction directly). Required-field
    /// checking via save_note_with_pipeline is the frontend save path. This test verifies
    /// that save_note_with_pipeline catches the missing required field.
    #[test]
    fn test_tree_action_validates_created_notes() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.create_user_script(r#"
// @name: RequiredField
schema("RequiredItem", #{ version: 1,
    fields: [
        #{ name: "sku", type: "text", required: true }
    ],
    on_save: |note| { commit(); }
});
        "#).unwrap();
        let root = ws.list_all_notes().unwrap()[0].clone();
        let note_id = ws.create_note(&root.id, AddPosition::AsChild, "RequiredItem").unwrap();

        // Save without providing the required "sku" field → should return ValidationErrors
        let result = ws.save_note_with_pipeline(
            &note_id,
            "Item".to_string(),
            BTreeMap::new(), // no fields provided
        ).unwrap();

        match result {
            SaveResult::ValidationErrors { field_errors, .. } => {
                assert!(field_errors.contains_key("sku"), "expected sku required error, got: {:?}", field_errors);
            }
            other => panic!("expected ValidationErrors for missing required field, got: {:?}", other),
        }
    }

    // ── Migration pipeline tests ──────────────────────────────────────────────

    /// Create a workspace, seed it with a schema v1 note, then manually lower the note's
    /// `schema_version` in the DB (simulating stale state after a schema bump), load the
    /// v2 schema with a field-rename migration, call `run_schema_migrations()`, and assert
    /// the field was renamed and `schema_version` updated.
    #[test]
    fn migration_renames_field_on_version_bump() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Register a v1 schema with a "phone" field.
        ws.create_user_script(
            r#"// @name: MigTest
// @description: migration test type
schema("MigType", #{
    version: 1,
    fields: [
        #{ name: "phone", type: "text", required: false },
    ]
});"#,
        ).unwrap();
        ws.reload_scripts().unwrap();

        let root_id = ws.list_all_notes().unwrap()[0].id.clone();
        let note_id = ws.create_note(&root_id, AddPosition::AsChild, "MigType").unwrap();

        // Write "phone" field.
        let mut fields = BTreeMap::new();
        fields.insert("phone".to_string(), FieldValue::Text("555-1234".to_string()));
        ws.update_note(&note_id, "Contact".to_string(), fields).unwrap();

        // Manually set schema_version = 0 to simulate a stale note.
        ws.connection().execute(
            "UPDATE notes SET schema_version = 0 WHERE id = ?1",
            rusqlite::params![&note_id],
        ).unwrap();

        // Update the script to v2 with a migration that renames phone → mobile.
        let scripts = ws.list_user_scripts().unwrap();
        let script_id = scripts.iter().find(|s| s.name == "MigTest").unwrap().id.clone();
        ws.update_user_script(
            &script_id,
            r#"// @name: MigTest
// @description: migration test type
schema("MigType", #{
    version: 2,
    fields: [
        #{ name: "mobile", type: "text", required: false },
    ],
    migrate: #{
        "2": |note| {
            note.fields["mobile"] = note.fields["phone"];
            note.fields.remove("phone");
            note
        }
    }
});"#,
        ).unwrap();

        // Reload and run migrations.
        ws.reload_scripts().unwrap();
        let results = ws.run_schema_migrations().unwrap();

        assert_eq!(results.len(), 1, "expected 1 migration result, got: {:?}", results);
        assert_eq!(results[0].0, "MigType");
        assert_eq!(results[0].3, 1, "should have migrated 1 note");

        let note = ws.get_note(&note_id).unwrap();
        assert_eq!(note.schema_version, 2);
        assert_eq!(note.fields.get("mobile"), Some(&FieldValue::Text("555-1234".to_string())));
        assert!(!note.fields.contains_key("phone"), "old field should be removed");
    }

    /// Notes that are already at the current schema version must not be re-migrated.
    #[test]
    fn migration_skips_up_to_date_notes() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        ws.create_user_script(
            r#"// @name: UpToDate
schema("UpToDateType", #{
    version: 1,
    fields: [ #{ name: "val", type: "text", required: false } ]
});"#,
        ).unwrap();
        ws.reload_scripts().unwrap();

        let root_id = ws.list_all_notes().unwrap()[0].id.clone();
        ws.create_note(&root_id, AddPosition::AsChild, "UpToDateType").unwrap();

        // No stale notes → results should be empty.
        let results = ws.run_schema_migrations().unwrap();
        assert!(results.is_empty(), "expected no migrations, got: {:?}", results);
    }

    /// Migration closures must chain: a note at v1 with a schema at v3 should pass through
    /// closures for v2 and v3 in order.
    #[test]
    fn migration_chains_across_multiple_versions() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        ws.create_user_script(
            r#"// @name: ChainTest
schema("ChainType", #{
    version: 1,
    fields: [ #{ name: "val", type: "text", required: false } ]
});"#,
        ).unwrap();
        ws.reload_scripts().unwrap();

        let root_id = ws.list_all_notes().unwrap()[0].id.clone();
        let note_id = ws.create_note(&root_id, AddPosition::AsChild, "ChainType").unwrap();
        let mut fields = BTreeMap::new();
        fields.insert("val".to_string(), FieldValue::Text("original".to_string()));
        ws.update_note(&note_id, "N".to_string(), fields).unwrap();

        // Force note to schema_version = 0.
        ws.connection().execute(
            "UPDATE notes SET schema_version = 0 WHERE id = ?1",
            rusqlite::params![&note_id],
        ).unwrap();

        let scripts = ws.list_user_scripts().unwrap();
        let sid = scripts.iter().find(|s| s.name == "ChainTest").unwrap().id.clone();
        ws.update_user_script(
            &sid,
            r#"// @name: ChainTest
schema("ChainType", #{
    version: 3,
    fields: [ #{ name: "val", type: "text", required: false } ],
    migrate: #{
        "2": |note| { note.fields["val"] = note.fields["val"] + "_v2"; note },
        "3": |note| { note.fields["val"] = note.fields["val"] + "_v3"; note }
    }
});"#,
        ).unwrap();

        ws.reload_scripts().unwrap();
        ws.run_schema_migrations().unwrap();

        let note = ws.get_note(&note_id).unwrap();
        assert_eq!(note.schema_version, 3);
        assert_eq!(note.fields.get("val"), Some(&FieldValue::Text("original_v2_v3".to_string())));
    }

    /// Version downgrade must be rejected when a higher-version schema is already registered.
    #[test]
    fn schema_version_downgrade_rejected() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        // Register v2.
        ws.create_user_script(
            r#"// @name: DowngradeTest
schema("DowngradeType", #{
    version: 2,
    fields: []
});"#,
        ).unwrap();
        ws.reload_scripts().unwrap();
        assert!(ws.script_registry.schema_exists("DowngradeType"));

        // Try to update to v1 — update_user_script pre-validates and must reject downgrade.
        let scripts = ws.list_user_scripts().unwrap();
        let sid = scripts.iter().find(|s| s.name == "DowngradeTest").unwrap().id.clone();
        let result = ws.update_user_script(
            &sid,
            r#"// @name: DowngradeTest
schema("DowngradeType", #{
    version: 1,
    fields: []
});"#,
        );
        assert!(result.is_err(), "downgrade should be rejected");

        // The schema must still be at v2 after the failed update.
        let schema = ws.script_registry.get_schema("DowngradeType").unwrap();
        assert_eq!(schema.version, 2, "schema must not have been downgraded");
    }

    /// Re-registering a schema with the same version number must succeed without error.
    #[test]
    fn schema_same_version_reregistration_allowed() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();

        ws.create_user_script(
            r#"// @name: SameVerTest
schema("SameVerType", #{
    version: 1,
    fields: [ #{ name: "alpha", type: "text", required: false } ]
});"#,
        ).unwrap();
        ws.reload_scripts().unwrap();

        let scripts = ws.list_user_scripts().unwrap();
        let sid = scripts.iter().find(|s| s.name == "SameVerTest").unwrap().id.clone();
        ws.update_user_script(
            &sid,
            r#"// @name: SameVerTest
schema("SameVerType", #{
    version: 1,
    fields: [
        #{ name: "alpha", type: "text", required: false },
        #{ name: "beta",  type: "text", required: false },
    ]
});"#,
        ).unwrap();
        ws.reload_scripts().unwrap();

        // Should now have 2 fields, no error.
        let schema = ws.script_registry.get_schema("SameVerType").unwrap();
        assert_eq!(schema.version, 1);
        assert_eq!(schema.fields.len(), 2, "expected 2 fields after same-version re-registration");
    }

    #[test]
    fn test_to_snapshot_json_roundtrip() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(
            temp.path(),
            "",
            "test-identity",
            ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]),
        ).unwrap();
        // Add a note so the snapshot has more than just the root.
        let root = ws.list_all_notes().unwrap()[0].clone();
        ws.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        let json = ws.to_snapshot_json().unwrap();
        assert!(!json.is_empty());
        let snap: WorkspaceSnapshot = serde_json::from_slice(&json).unwrap();
        // Workspace::create inserts a root note, so we have 2 notes total.
        assert_eq!(snap.notes.len(), 2);
    }

    #[test]
    fn test_to_snapshot_json_includes_attachments() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let key = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let mut ws = Workspace::create(&db_path, "", "test-id", key).unwrap();
        let root_id = ws.list_all_notes().unwrap()[0].id.clone();
        ws.attach_file(&root_id, "test.txt", None, b"hello bytes").unwrap();
        let json = ws.to_snapshot_json().unwrap();
        let snap: WorkspaceSnapshot = serde_json::from_slice(&json).unwrap();
        assert_eq!(snap.attachments.len(), 1);
        assert_eq!(snap.attachments[0].filename, "test.txt");
    }

    #[test]
    fn test_get_latest_operation_id_empty_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let key = ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]);
        let ws = Workspace::create(&db_path, "", "test-id", key).unwrap();
        // A freshly created workspace has no operations logged yet.
        assert!(ws.get_latest_operation_id().unwrap().is_none());
    }

    #[test]
    fn test_create_with_id_preserves_uuid() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let key = ed25519_dalek::SigningKey::from_bytes(&[3u8; 32]);
        let custom_id = "my-fixed-workspace-uuid";
        let ws = Workspace::create_with_id(&db_path, "", "test-id", key, custom_id).unwrap();
        assert_eq!(ws.workspace_id(), custom_id);
    }

    #[test]
    fn test_create_empty_with_id_no_root_note() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("notes.db");
        let key = ed25519_dalek::SigningKey::from_bytes(&[4u8; 32]);
        let custom_id = "snapshot-workspace-uuid";
        let ws = Workspace::create_empty_with_id(&db_path, "", "test-id", key, custom_id).unwrap();
        assert_eq!(ws.workspace_id(), custom_id);
        // No root note should be auto-inserted — snapshot restoration will add its own notes.
        assert_eq!(ws.list_all_notes().unwrap().len(), 0);
    }

    #[test]
    fn test_import_snapshot_json_round_trip() {
        let src_temp = NamedTempFile::new().unwrap();
        let mut src = Workspace::create(
            src_temp.path(),
            "",
            "src-identity",
            ed25519_dalek::SigningKey::from_bytes(&[2u8; 32]),
        ).unwrap();
        // Replace the default root note title and add two children.
        let root = src.list_all_notes().unwrap()[0].clone();
        src.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        src.create_note(&root.id, AddPosition::AsChild, "TextNote").unwrap();
        let json = src.to_snapshot_json().unwrap();

        // Destination workspace.
        let dst_temp = NamedTempFile::new().unwrap();
        let mut dst = Workspace::create(
            dst_temp.path(),
            "",
            "dst-identity",
            ed25519_dalek::SigningKey::from_bytes(&[3u8; 32]),
        ).unwrap();
        // Remove the auto-created root so we start from a clean slate.
        let dst_root = dst.list_all_notes().unwrap()[0].clone();
        dst.storage.connection_mut().execute(
            "DELETE FROM notes WHERE id = ?",
            [&dst_root.id],
        ).unwrap();

        let count = dst.import_snapshot_json(&json).unwrap();
        // src had: 1 original root + 2 children = 3 notes.
        assert_eq!(count, 3);
        let notes = dst.list_all_notes().unwrap();
        assert_eq!(notes.len(), 3);
    }

    #[test]
    fn test_is_leaf_defaults_to_false() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.create_user_script(
            "// @name: NormalSchema\nschema(\"NormalType\", #{ version: 1, fields: [] });"
        ).unwrap();
        let schema = ws.script_registry().get_schema("NormalType").unwrap();
        assert!(!schema.is_leaf, "is_leaf should default to false");
    }

    #[test]
    fn test_is_leaf_explicit_true() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.create_user_script(
            "// @name: LeafSchema\nschema(\"LeafType\", #{ version: 1, is_leaf: true, fields: [] });"
        ).unwrap();
        let schema = ws.script_registry().get_schema("LeafType").unwrap();
        assert!(schema.is_leaf, "is_leaf should be true when explicitly set");
    }

    #[test]
    fn test_is_leaf_blocks_create_child() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.create_user_script(
            "// @name: IsLeafSchemas\nschema(\"LeafType\", #{ version: 1, is_leaf: true, fields: [] });\nschema(\"ChildType\", #{ version: 1, fields: [] });"
        ).unwrap();

        // Create a root LeafType note
        let leaf_id = ws.create_note_root("LeafType").unwrap();

        // Trying to create a child under it must fail
        let result = ws.create_note(&leaf_id, AddPosition::AsChild, "ChildType");
        assert!(result.is_err(), "expected error when adding child to leaf note");
        let err = result.unwrap_err().to_string();
        assert!(err.to_lowercase().contains("leaf"), "expected 'leaf' in error: {err}");
    }

    #[test]
    fn test_is_leaf_blocks_move_note() {
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.create_user_script(
            "// @name: IsLeafMoveSchemas\nschema(\"LeafType\", #{ version: 1, is_leaf: true, fields: [] });\nschema(\"ChildType\", #{ version: 1, fields: [] });"
        ).unwrap();

        let leaf_id  = ws.create_note_root("LeafType").unwrap();
        let child_id = ws.create_note_root("ChildType").unwrap();

        // Moving child under leaf must fail
        let result = ws.move_note(&child_id, Some(&leaf_id), 0.0);
        assert!(result.is_err(), "expected error when moving note under leaf");
        let err = result.unwrap_err().to_string();
        assert!(err.to_lowercase().contains("leaf"), "expected 'leaf' in error: {err}");
    }

    #[test]
    fn test_is_leaf_blocks_deep_copy() {
        // deep_copy_note (paste) should also be blocked when the target parent is a leaf
        let temp = NamedTempFile::new().unwrap();
        let mut ws = Workspace::create(temp.path(), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.create_user_script(
            "// @name: IsLeafDeepCopySchemas\nschema(\"LeafType\", #{ version: 1, is_leaf: true, fields: [] });\nschema(\"ChildType\", #{ version: 1, fields: [] });"
        ).unwrap();

        // Create a root ChildType note (the note to copy)
        let child_id = ws.create_note_root("ChildType").unwrap();
        // Create a root LeafType note (the intended paste target)
        let leaf_id = ws.create_note_root("LeafType").unwrap();

        // Pasting child under leaf must fail
        let result = ws.deep_copy_note(&child_id, &leaf_id, AddPosition::AsChild);
        assert!(result.is_err(), "expected error when deep-copying note under leaf");
        let err = result.unwrap_err().to_string();
        assert!(err.to_lowercase().contains("leaf"), "expected 'leaf' in error: {err}");
    }

    #[test]
    fn test_list_peers_info_unknown_peer() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::create(dir.path().join("ws.db"), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        ws.add_contact_as_peer("AAAAAAAAAAAAAAAA").unwrap();

        let cm_dir = tempfile::tempdir().unwrap();
        let key = [0u8; 32];
        let cm = ContactManager::for_identity(cm_dir.path().to_path_buf(), key).unwrap();

        let peers = ws.list_peers_info(&cm).unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].display_name, "AAAAAAAA…");
        assert!(peers[0].trust_level.is_none());
        assert!(peers[0].contact_id.is_none());
        assert!(!peers[0].fingerprint.is_empty());
    }

    #[test]
    fn test_list_peers_info_known_contact() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::create(dir.path().join("ws.db"), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let pubkey = "BBBBBBBBBBBBBBBB";
        ws.add_contact_as_peer(pubkey).unwrap();

        let cm_dir = tempfile::tempdir().unwrap();
        let key = [1u8; 32];
        let cm = ContactManager::for_identity(cm_dir.path().to_path_buf(), key).unwrap();
        let contact = cm.create_contact("Bob", pubkey, TrustLevel::CodeVerified).unwrap();

        let peers = ws.list_peers_info(&cm).unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].display_name, "Bob");
        assert_eq!(peers[0].trust_level.as_deref(), Some("CodeVerified"));
        let expected_contact_id = contact.contact_id.to_string();
        assert_eq!(peers[0].contact_id.as_deref(), Some(expected_contact_id.as_str()));
    }

    #[test]
    fn test_add_and_remove_peer() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::create(dir.path().join("ws.db"), "", "test-identity", ed25519_dalek::SigningKey::from_bytes(&[1u8; 32])).unwrap();
        let pubkey = "CCCCCCCCCCCCCCCC";

        ws.add_contact_as_peer(pubkey).unwrap();
        let cm_dir = tempfile::tempdir().unwrap();
        let cm = ContactManager::for_identity(cm_dir.path().to_path_buf(), [0u8; 32]).unwrap();
        let peers = ws.list_peers_info(&cm).unwrap();
        assert_eq!(peers.len(), 1);

        let placeholder = format!("identity:{}", pubkey);
        ws.remove_peer(&placeholder).unwrap();
        let peers = ws.list_peers_info(&cm).unwrap();
        assert_eq!(peers.len(), 0);
    }
}
