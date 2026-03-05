// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Core library for Krillnotes — a local-first, hierarchical note-taking application.
//!
//! The primary entry point is [`Workspace`], which represents an open `.krillnotes`
//! database file. All document mutations go through `Workspace` methods.
//!
//! Types are re-exported from their respective sub-modules for convenience;
//! consumers should import from the crate root rather than the `core` module.

pub mod core;

// Re-export commonly used types.
#[doc(inline)]
pub use core::{
    attachment::AttachmentMeta,
    delete::{DeleteResult, DeleteStrategy},
    export::{
        export_workspace, import_workspace, peek_import, ExportError, ExportNotes, ImportResult,
        ScriptManifest, ScriptManifestEntry, WorkspaceMetadata, APP_VERSION,
    },
    device::get_device_id,
    error::{KrillnotesError, Result},
    note::{FieldValue, Note},
    operation::Operation,
    operation_log::{OperationLog, OperationSummary, PurgeStrategy},
    scripting::{FieldDefinition, HookRegistry, QueryContext, Schema, ScriptError, ScriptRegistry, StarterScript},
    hlc::{HlcClock, HlcTimestamp},
    identity::{IdentityFile, IdentityManager, IdentitySettings, IdentityRef, WorkspaceBinding, UnlockedIdentity, SwarmIdFile},
    storage::Storage,
    undo::{RetractInverse, UndoResult},
    user_script::UserScript,
    workspace::{AddPosition, NoteSearchResult, Workspace},
};

// Re-export SigningKey so consumers don't need a direct ed25519-dalek dependency.
#[doc(inline)]
pub use ed25519_dalek::SigningKey as Ed25519SigningKey;
