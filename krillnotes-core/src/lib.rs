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
    delete::{DeleteResult, DeleteStrategy},
    device::get_device_id,
    error::{KrillnotesError, Result},
    note::{FieldValue, Note},
    operation::Operation,
    operation_log::{OperationLog, PurgeStrategy},
    scripting::{FieldDefinition, Schema, SchemaRegistry},
    storage::Storage,
    workspace::{AddPosition, Workspace},
};
