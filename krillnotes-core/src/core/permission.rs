// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Pluggable permission enforcement for workspaces.
//!
//! The [`PermissionGate`] trait defines the interface that permission backends
//! (such as the RBAC crate) must implement. The workspace holds an optional
//! gate; when present, every mutating operation is checked via
//! [`PermissionGate::authorize`] before being applied.

use rusqlite::Connection;

use crate::core::operation::Operation;

/// A pluggable permission enforcement backend.
///
/// The workspace holds an optional gate. When present, every mutating
/// operation is checked via `authorize()` before being applied.
/// The gate owns its own database tables and manages them via
/// `ensure_schema()` and `apply_permission_op()`.
pub trait PermissionGate: Send + Sync {
    /// Protocol discriminator embedded in every outbound .swarm bundle header.
    /// Krillnotes RBAC: `"krillnotes/1"`
    fn protocol_id(&self) -> &'static str;

    /// Authorise an operation before it is applied.
    ///
    /// Called for every mutating operation — both locally generated and
    /// inbound from a .swarm bundle — before the operation is written
    /// to the database.
    ///
    /// `actor` is the base64-encoded Ed25519 public key of the identity
    /// performing the operation.
    ///
    /// Returns `Ok(())` if permitted, `Err(PermissionError)` if denied.
    fn authorize(
        &self,
        conn: &Connection,
        actor: &str,
        operation: &Operation,
    ) -> Result<(), PermissionError>;

    /// Apply a permission-modifying operation to the gate's own tables.
    ///
    /// Called after `authorize()` has returned `Ok(())` for a
    /// `SetPermission` or `RevokePermission` operation, within the
    /// same database transaction.
    fn apply_permission_op(
        &self,
        conn: &Connection,
        operation: &Operation,
    ) -> Result<(), PermissionError>;

    /// Create or migrate the gate's database tables.
    /// Called once when the workspace is opened.
    fn ensure_schema(&self, conn: &Connection) -> Result<(), PermissionError>;
}

/// Errors that can occur during permission checking or mutation.
#[derive(Debug, thiserror::Error)]
pub enum PermissionError {
    /// The actor does not have the required permission for this operation.
    #[error("operation denied: {0}")]
    Denied(String),

    /// The permission delegation chain is invalid (e.g. circular or broken).
    #[error("invalid permission chain: {0}")]
    InvalidChain(String),

    /// The operation is not a permission-related operation.
    #[error("operation is not a permission operation")]
    NotAPermissionOp,

    /// A database error occurred while reading or writing permission state.
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
}
