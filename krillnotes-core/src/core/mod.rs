// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Internal domain modules for the Krillnotes core library.
//!
//! All public types from these modules are re-exported at the crate root
//! with `#[doc(inline)]`; import from there in preference to this module.

pub mod attachment;
pub mod contact;
pub mod delete;
pub mod hlc;
pub mod identity;
pub mod invite;
pub mod export;
pub mod device;
pub mod error;
pub mod note;
pub mod operation;
pub mod operation_log;
pub mod peer_registry;
pub mod save_transaction;
pub mod scripting;
pub mod swarm;
pub mod storage;
pub mod sync;
pub mod user_script;
pub mod undo;
pub mod workspace;

#[doc(inline)]
pub use attachment::AttachmentMeta;
#[doc(inline)]
pub use contact::{Contact, ContactManager, TrustLevel, generate_fingerprint};
#[doc(inline)]
pub use delete::{DeleteResult, DeleteStrategy};
#[doc(inline)]
pub use export::{
    export_workspace, import_workspace, peek_import, ExportError, ExportNotes, ImportResult,
    ScriptManifest, ScriptManifestEntry, APP_VERSION,
};
#[doc(inline)]
pub use device::get_device_id;
#[doc(inline)]
pub use error::{KrillnotesError, Result};
#[doc(inline)]
pub use note::{FieldValue, Note};
#[doc(inline)]
pub use operation::Operation;
#[doc(inline)]
pub use peer_registry::{PeerRegistry, SyncPeer};
#[doc(inline)]
pub use operation_log::{OperationLog, OperationSummary, PurgeStrategy};
#[doc(inline)]
pub use scripting::{FieldDefinition, Schema, ScriptRegistry};
#[doc(inline)]
pub use storage::Storage;
#[doc(inline)]
pub use swarm::header::{RecipientEntry, SwarmHeader, SwarmMode};
#[doc(inline)]
pub use undo::{RetractInverse, UndoResult};
#[doc(inline)]
pub use user_script::UserScript;
#[doc(inline)]
pub use workspace::{AddPosition, NoteSearchResult, Workspace};
