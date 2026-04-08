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
    DeleteResult, DeleteStrategy, FieldValue, KrillnotesError, Note,
    Operation, OperationLog, PurgeStrategy, QueryContext, Result, RetractInverse, SaveResult,
    ScriptError, ScriptRegistry, Storage, UndoResult, UserScript,
};
use rhai::Dynamic;
use rusqlite::{Connection, OptionalExtension};
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
    /// Permission operations (SetPermission / RevokePermission) so that the
    /// recipient's permission gate can reconstruct access grants.
    #[serde(default)]
    pub permission_ops: Vec<Operation>,
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
    /// Base64-encoded Ed25519 public key of the workspace creator (owner).
    /// Only the owner may create, update, or delete scripts.
    owner_pubkey: String,
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
    /// Pluggable permission gate (e.g. RBAC).
    /// Every mutating operation is checked via `authorize()` before being applied.
    /// Use `AllowAllGate` for tests or builds without a specific gate feature.
    permission_gate: Box<dyn crate::core::permission::PermissionGate>,
}

impl Workspace {
    /// Creates a new workspace database at `path`, initialises the schema, and inserts
    /// a root note named after the file (e.g. `"My Notes"` for `my-notes.krillnotes`).
    ///
    /// # Errors
    ///
    /// Returns [`crate::KrillnotesError::Database`] for any SQLite failure, or
    /// [`crate::KrillnotesError::InvalidWorkspace`] if the device ID cannot be obtained.
    pub fn create<P: AsRef<Path>>(path: P, password: &str, identity_uuid: &str, signing_key: ed25519_dalek::SigningKey, permission_gate: Box<dyn crate::core::permission::PermissionGate>, identity_dir: Option<&Path>) -> Result<Self> {
        let mut storage = Storage::create(&path, password)?;
        let mut script_registry = ScriptRegistry::new()?;
        let operation_log = OperationLog::new(PurgeStrategy::LocalOnly { keep_last: 100 });

        // Build composite device_id: {identity_uuid}:{device_uuid} when identity_dir is known.
        let device_id = if let Some(dir) = identity_dir {
            let device_uuid = crate::core::identity::ensure_device_uuid(dir)?;
            format!("{identity_uuid}:{device_uuid}")
        } else {
            identity_uuid.to_string()
        };

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
                let category = if starter.filename.ends_with(".schema.rhai") { "schema" } else { "library" };
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
                    category: row.get::<_, String>(8).unwrap_or_else(|_| "library".to_string()),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
            results
        };
        // Two-phase loading: library first, then schema, then resolve.
        for script in scripts.iter().filter(|s| s.enabled && s.category == "library") {
            script_registry.set_loading_category(Some("library".to_string()));
            if let Err(e) = script_registry.load_script(&script.source_code, &script.name) {
                log::warn!("Failed to load starter script '{}': {}", script.name, e);
            }
        }
        for script in scripts.iter().filter(|s| s.enabled && s.category == "schema") {
            script_registry.set_loading_category(Some("schema".to_string()));
            if let Err(e) = script_registry.load_script(&script.source_code, &script.name) {
                log::warn!("Failed to load starter script '{}': {}", script.name, e);
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

        // Store the creator as workspace owner
        storage.connection().execute(
            "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
            rusqlite::params!["owner_pubkey", &identity_pubkey_b64],
        )?;

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
        let device_uuid_str = crate::core::identity::device_part_from_device_id(&device_id);
        let node_id = crate::core::hlc::node_id_from_device(
            &uuid::Uuid::parse_str(device_uuid_str).unwrap_or_else(|_| uuid::Uuid::new_v4()),
        );
        let hlc = HlcClock::new(node_id);

        // Initialise permission gate tables.
        permission_gate.ensure_schema(storage.connection())?;

        let mut workspace = Self {
            storage,
            script_registry,
            operation_log,
            device_id,
            identity_uuid: identity_uuid.to_string(),
            current_identity_pubkey: identity_pubkey_b64.clone(),
            workspace_root,
            workspace_id,
            owner_pubkey: identity_pubkey_b64,
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
            permission_gate,
        };
        // Emit a RegisterDevice operation for the creating device.
        workspace.emit_register_device_if_needed()?;
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
        permission_gate: Box<dyn crate::core::permission::PermissionGate>,
        identity_dir: Option<&Path>,
    ) -> Result<Self> {
        let mut storage = Storage::create(&path, password)?;
        let mut script_registry = ScriptRegistry::new()?;
        let operation_log = OperationLog::new(PurgeStrategy::LocalOnly { keep_last: 100 });

        // Build composite device_id: {identity_uuid}:{device_uuid} when identity_dir is known.
        let device_id = if let Some(dir) = identity_dir {
            let device_uuid = crate::core::identity::ensure_device_uuid(dir)?;
            format!("{identity_uuid}:{device_uuid}")
        } else {
            identity_uuid.to_string()
        };

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
                let category = if starter.filename.ends_with(".schema.rhai") { "schema" } else { "library" };
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
                    category: row.get::<_, String>(8).unwrap_or_else(|_| "library".to_string()),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
            results
        };
        // Two-phase loading: library first, then schema, then resolve.
        for script in scripts.iter().filter(|s| s.enabled && s.category == "library") {
            script_registry.set_loading_category(Some("library".to_string()));
            if let Err(e) = script_registry.load_script(&script.source_code, &script.name) {
                log::warn!("Failed to load starter script '{}': {}", script.name, e);
            }
        }
        for script in scripts.iter().filter(|s| s.enabled && s.category == "schema") {
            script_registry.set_loading_category(Some("schema".to_string()));
            if let Err(e) = script_registry.load_script(&script.source_code, &script.name) {
                log::warn!("Failed to load starter script '{}': {}", script.name, e);
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

        // Store the creator as workspace owner
        storage.connection().execute(
            "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
            rusqlite::params!["owner_pubkey", &identity_pubkey_b64],
        )?;

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
        let device_uuid_str = crate::core::identity::device_part_from_device_id(&device_id);
        let node_id = crate::core::hlc::node_id_from_device(
            &uuid::Uuid::parse_str(device_uuid_str).unwrap_or_else(|_| uuid::Uuid::new_v4()),
        );
        let hlc = HlcClock::new(node_id);

        // Initialise permission gate tables.
        permission_gate.ensure_schema(storage.connection())?;

        let mut workspace = Self {
            storage,
            script_registry,
            operation_log,
            device_id,
            identity_uuid: identity_uuid.to_string(),
            current_identity_pubkey: identity_pubkey_b64.clone(),
            workspace_root,
            workspace_id,
            owner_pubkey: identity_pubkey_b64,
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
            permission_gate,
        };
        // Emit a RegisterDevice operation for the creating device.
        workspace.emit_register_device_if_needed()?;
        let _ = workspace.write_info_json(); // best-effort; non-fatal
        Ok(workspace)
    }

    /// Like [`create`] but does **not** insert a default root note.
    ///
    /// Use this when the workspace content will immediately be populated from an
    /// external source (e.g. a snapshot import), so the seed note would only create
    /// unwanted noise alongside the imported tree.
    pub fn create_empty<P: AsRef<Path>>(path: P, password: &str, identity_uuid: &str, signing_key: ed25519_dalek::SigningKey, permission_gate: Box<dyn crate::core::permission::PermissionGate>, identity_dir: Option<&Path>) -> Result<Self> {
        let mut storage = Storage::create(&path, password)?;
        let mut script_registry = ScriptRegistry::new()?;
        let operation_log = OperationLog::new(PurgeStrategy::LocalOnly { keep_last: 100 });

        // Build composite device_id: {identity_uuid}:{device_uuid} when identity_dir is known.
        let device_id = if let Some(dir) = identity_dir {
            let device_uuid = crate::core::identity::ensure_device_uuid(dir)?;
            format!("{identity_uuid}:{device_uuid}")
        } else {
            identity_uuid.to_string()
        };

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
                let category = if starter.filename.ends_with(".schema.rhai") { "schema" } else { "library" };
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
                    category: row.get::<_, String>(8).unwrap_or_else(|_| "library".to_string()),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
            results
        };
        for script in scripts.iter().filter(|s| s.enabled && s.category == "library") {
            script_registry.set_loading_category(Some("library".to_string()));
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

        // Store the creator as workspace owner
        storage.connection().execute(
            "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
            rusqlite::params!["owner_pubkey", &identity_pubkey_b64],
        )?;

        // No default root note — content will come from the imported snapshot.

        storage.connection().execute(
            "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
            ["undo_limit", "50"],
        )?;
        let undo_limit: usize = 50;

        let device_uuid_str = crate::core::identity::device_part_from_device_id(&device_id);
        let node_id = crate::core::hlc::node_id_from_device(
            &uuid::Uuid::parse_str(device_uuid_str).unwrap_or_else(|_| uuid::Uuid::new_v4()),
        );
        let hlc = HlcClock::new(node_id);

        // Initialise permission gate tables.
        permission_gate.ensure_schema(storage.connection())?;

        let mut workspace = Self {
            storage,
            script_registry,
            operation_log,
            device_id,
            identity_uuid: identity_uuid.to_string(),
            current_identity_pubkey: identity_pubkey_b64.clone(),
            workspace_root,
            workspace_id,
            owner_pubkey: identity_pubkey_b64,
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
            permission_gate,
        };
        // Emit a RegisterDevice operation for the creating device.
        workspace.emit_register_device_if_needed()?;
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
        permission_gate: Box<dyn crate::core::permission::PermissionGate>,
        identity_dir: Option<&Path>,
    ) -> Result<Self> {
        let storage = Storage::create(&path, password)?;
        let script_registry = ScriptRegistry::new()?;
        let operation_log = OperationLog::new(PurgeStrategy::LocalOnly { keep_last: 100 });

        // Build composite device_id: {identity_uuid}:{device_uuid} when identity_dir is known.
        let device_id = if let Some(dir) = identity_dir {
            let device_uuid = crate::core::identity::ensure_device_uuid(dir)?;
            format!("{identity_uuid}:{device_uuid}")
        } else {
            identity_uuid.to_string()
        };

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

        // Store the creator as workspace owner
        storage.connection().execute(
            "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
            rusqlite::params!["owner_pubkey", &identity_pubkey_b64],
        )?;

        // No default root note — content will come from the imported snapshot.

        storage.connection().execute(
            "INSERT INTO workspace_meta (key, value) VALUES (?, ?)",
            ["undo_limit", "50"],
        )?;
        let undo_limit: usize = 50;

        let device_uuid_str = crate::core::identity::device_part_from_device_id(&device_id);
        let node_id = crate::core::hlc::node_id_from_device(
            &uuid::Uuid::parse_str(device_uuid_str).unwrap_or_else(|_| uuid::Uuid::new_v4()),
        );
        let hlc = HlcClock::new(node_id);

        // Initialise permission gate tables.
        permission_gate.ensure_schema(storage.connection())?;

        let mut workspace = Self {
            storage,
            script_registry,
            operation_log,
            device_id,
            identity_uuid: identity_uuid.to_string(),
            current_identity_pubkey: identity_pubkey_b64.clone(),
            workspace_root,
            workspace_id,
            owner_pubkey: identity_pubkey_b64,
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
            permission_gate,
        };
        // Emit a RegisterDevice operation for the creating device.
        workspace.emit_register_device_if_needed()?;
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
    pub fn open<P: AsRef<Path>>(path: P, password: &str, identity_uuid: &str, signing_key: ed25519_dalek::SigningKey, mut permission_gate: Box<dyn crate::core::permission::PermissionGate>, identity_dir: Option<&Path>) -> Result<Self> {
        let storage = Storage::open(&path, password)?;
        let script_registry = ScriptRegistry::new()?;
        let operation_log = OperationLog::new(PurgeStrategy::LocalOnly { keep_last: 100 });

        // Read metadata from database
        let mut device_id: String = storage.connection()
            .query_row(
                "SELECT value FROM workspace_meta WHERE key = 'device_id'",
                [],
                |row| row.get::<_, String>(0)
            )?;

        // If an identity directory is provided, always recompute composite device_id
        // so the format is {identity_uuid}:{device_uuid} regardless of what is stored.
        if let Some(dir) = identity_dir {
            let device_uuid = crate::core::identity::ensure_device_uuid(dir)?;
            let composite = format!("{identity_uuid}:{device_uuid}");
            if device_id != composite {
                let tx = storage.connection().unchecked_transaction()?;
                tx.execute(
                    "UPDATE workspace_meta SET value = ? WHERE key = 'device_id'",
                    [&composite],
                )?;
                tx.commit()?;
                device_id = composite;
            }
        }

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
        let device_uuid_str = crate::core::identity::device_part_from_device_id(&device_id);
        let node_id = crate::core::hlc::node_id_from_device(
            &uuid::Uuid::parse_str(device_uuid_str).unwrap_or_else(|_| uuid::Uuid::new_v4()),
        );
        let hlc = HlcClock::load_from_db(storage.connection(), node_id)
            .map_err(KrillnotesError::Database)?;

        // Persist identity_uuid into workspace_meta if not already present
        // (handles workspaces created before identity enforcement).
        let _ = storage.connection().execute(
            "INSERT OR IGNORE INTO workspace_meta (key, value) VALUES (?, ?)",
            rusqlite::params!["identity_uuid", identity_uuid],
        );

        // Read owner_pubkey from workspace_meta. If absent (pre-existing workspace),
        // the current opener becomes the owner.
        let owner_pubkey: String = {
            let existing: std::result::Result<String, rusqlite::Error> = storage.connection().query_row(
                "SELECT value FROM workspace_meta WHERE key = 'owner_pubkey'",
                [],
                |row| row.get(0),
            );
            match existing {
                Ok(pk) => pk,
                Err(_) => {
                    let pk = identity_pubkey_b64.clone();
                    let _ = storage.connection().execute(
                        "INSERT OR IGNORE INTO workspace_meta (key, value) VALUES (?, ?)",
                        rusqlite::params!["owner_pubkey", &pk],
                    );
                    pk
                }
            }
        };

        // Tell the gate who the real workspace owner is (the value just
        // read from workspace_meta), so root-owner bypass works correctly
        // even when the workspace is opened by a different identity.
        permission_gate.init_owner(&owner_pubkey);

        // Initialise permission gate tables.
        permission_gate.ensure_schema(storage.connection())?;

        let mut ws = Self {
            storage,
            script_registry,
            operation_log,
            device_id,
            identity_uuid: identity_uuid.to_string(),
            current_identity_pubkey: identity_pubkey_b64,
            workspace_root,
            workspace_id,
            owner_pubkey,
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
            permission_gate,
        };

        // Two-phase script loading: library first, then schema, then resolve.
        let scripts = ws.list_user_scripts()?;
        for script in scripts.iter().filter(|s| s.enabled && s.category == "library") {
            ws.script_registry.set_loading_category(Some("library".to_string()));
            if let Err(e) = ws.script_registry.load_script(&script.source_code, &script.name) {
                log::warn!("Failed to load script '{}': {}", script.name, e);
            }
        }
        for script in scripts.iter().filter(|s| s.enabled && s.category == "schema") {
            ws.script_registry.set_loading_category(Some("schema".to_string()));
            if let Err(e) = ws.script_registry.load_script(&script.source_code, &script.name) {
                log::warn!("Failed to load script '{}': {}", script.name, e);
            }
        }
        ws.script_registry.resolve_bindings();

        // Phase D: batch-migrate notes whose schema_version is behind current schema.
        ws.pending_migration_results = ws.run_schema_migrations()?;

        // Emit a RegisterDevice operation the first time this device opens this workspace.
        ws.emit_register_device_if_needed()?;

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

    /// Returns the device identifier used for this workspace's operations.
    ///
    /// This is the identity UUID of the workspace owner, not the hardware
    /// device ID, so that two identities on the same machine have distinct
    /// device IDs and echo prevention works correctly during delta sync.
    pub fn device_id(&self) -> &str {
        &self.device_id
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

    /// Returns the base64-encoded Ed25519 public key of the workspace owner (creator).
    pub fn owner_pubkey(&self) -> &str {
        &self.owner_pubkey
    }

    /// Returns `true` if the currently bound identity is the workspace owner.
    pub fn is_owner(&self) -> bool {
        self.current_identity_pubkey == self.owner_pubkey
    }

    /// Overwrites the cached owner pubkey and persists it to `workspace_meta`.
    /// Used when applying a snapshot bundle — the new workspace is created with
    /// the opener's identity as owner, then overwritten with the snapshot's true owner.
    ///
    /// Internal only — called during snapshot import. Not exposed via Tauri.
    /// Authorization is handled by the caller's context (AllowAllGate during import).
    pub fn set_owner_pubkey(&mut self, pubkey: &str) -> crate::Result<()> {
        self.storage.connection().execute(
            "INSERT OR REPLACE INTO workspace_meta (key, value) VALUES ('owner_pubkey', ?)",
            [pubkey],
        )?;
        self.owner_pubkey = pubkey.to_string();
        // Keep the permission gate in sync so root-owner bypass is correct.
        self.permission_gate.init_owner(pubkey);
        Ok(())
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

    /// Returns the protocol identifier from the installed permission gate.
    /// Used to stamp outbound .swarm bundle headers and validate inbound ones.
    pub fn protocol_id(&self) -> &str {
        self.permission_gate.protocol_id()
    }

    /// Check permission before applying an operation.
    fn authorize(&self, operation: &Operation) -> Result<()> {
        self.permission_gate.authorize(
            self.storage.connection(),
            &self.current_identity_pubkey,
            operation,
        )?;
        Ok(())
    }

    /// Apply a permission-modifying operation through the gate.
    /// Takes the gate as an explicit parameter to avoid a whole-`self` borrow
    /// conflict with the transaction (which is borrowed from `self.storage`).
    fn apply_permission_op_via(
        gate: &dyn crate::core::permission::PermissionGate,
        conn: &Connection,
        operation: &Operation,
    ) -> Result<()> {
        gate.apply_permission_op(conn, operation)?;
        Ok(())
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

    /// Emits a `RegisterDevice` operation the first time this device opens this workspace.
    ///
    /// Checks whether a `RegisterDevice` operation for this `device_uuid` already exists
    /// in the operations log. If not, creates and logs one. This is idempotent across
    /// multiple opens of the same workspace on the same device.
    pub(crate) fn emit_register_device_if_needed(&mut self) -> Result<()> {
        let device_uuid = crate::core::identity::device_part_from_device_id(&self.device_id).to_string();

        // Determine a human-readable device name from the hostname (outside the
        // transaction so we don't hold the lock while calling OS APIs).
        let device_name = {
            let raw = hostname::get()
                .ok()
                .and_then(|n| n.into_string().ok())
                .unwrap_or_default();
            let trimmed = raw.trim().to_lowercase();
            if trimmed.is_empty() || trimmed == "localhost" || trimmed == "unknown" {
                format!("{} Device", std::env::consts::OS)
            } else {
                raw.trim().to_string()
            }
        };

        let ts = self.hlc.now();

        // Open the write transaction first, then check for duplicates inside it.
        // This eliminates the TOCTOU race between the SELECT COUNT and the INSERT.
        let tx = self.storage.connection_mut().transaction()?;

        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM operations \
             WHERE operation_type = 'RegisterDevice' \
             AND json_extract(operation_data, '$.device_uuid') = ?1",
            rusqlite::params![&device_uuid],
            |row| row.get(0),
        )?;

        if count > 0 {
            tx.commit()?;
            return Ok(());
        }

        Self::save_hlc(&ts, &tx)?;
        let mut op = Operation::RegisterDevice {
            operation_id: uuid::Uuid::new_v4().to_string(),
            timestamp: ts,
            device_id: self.device_id.clone(),
            device_uuid,
            device_name,
            identity_public_key: String::new(),
            signature: String::new(),
        };
        Self::sign_op_with(&self.signing_key, &mut op);
        Self::log_op(&self.operation_log, &tx, &op)?;
        tx.commit()?;

        Ok(())
    }

}

// ── Domain sub-modules (split from this file for readability) ──────

mod undo;
mod notes;
mod hooks;
mod scripts;
mod attachments;
mod sync;
pub mod permissions;

// ── Free functions shared across domain sub-modules ─────────────────

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
mod tests;
