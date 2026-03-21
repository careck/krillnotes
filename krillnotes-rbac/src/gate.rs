// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

use krillnotes_core::core::operation::Operation;
use krillnotes_core::core::permission::{PermissionError, PermissionGate};
use rusqlite::Connection;

/// RBAC permission gate for Krillnotes (open source).
///
/// Implements the 4-role model: Root Owner > Owner > Writer > Reader.
/// The Root Owner is identified by public key comparison, not by a
/// database entry. All other roles are stored in `note_permissions`.
pub struct RbacGate {
    /// Base64-encoded Ed25519 public key of the workspace creator.
    owner_pubkey: String,
}

impl RbacGate {
    pub fn new(owner_pubkey: String) -> Self {
        Self { owner_pubkey }
    }

    /// Returns true if the given actor is the Root Owner.
    fn is_root_owner(&self, actor: &str) -> bool {
        actor == self.owner_pubkey
    }
}

impl PermissionGate for RbacGate {
    fn protocol_id(&self) -> &'static str {
        "krillnotes/1"
    }

    fn authorize(
        &self,
        _conn: &Connection,
        _actor: &str,
        _operation: &Operation,
    ) -> Result<(), PermissionError> {
        // TODO: implement in Task 4
        Ok(())
    }

    fn apply_permission_op(
        &self,
        _conn: &Connection,
        _operation: &Operation,
    ) -> Result<(), PermissionError> {
        // TODO: implement in Task 5
        Ok(())
    }

    fn ensure_schema(&self, conn: &Connection) -> Result<(), PermissionError> {
        conn.execute_batch(include_str!("schema.sql"))?;
        Ok(())
    }
}
