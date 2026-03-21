// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

use crate::resolver::Role;
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

    /// Determine which note_id to use as the scope for permission checking.
    fn resolve_scope(&self, operation: &Operation) -> Result<Option<String>, PermissionError> {
        match operation {
            Operation::CreateNote { parent_id, .. } => Ok(parent_id.clone()),
            Operation::UpdateNote { note_id, .. }
            | Operation::UpdateField { note_id, .. }
            | Operation::DeleteNote { note_id, .. }
            | Operation::SetTags { note_id, .. } => Ok(Some(note_id.clone())),
            Operation::MoveNote { note_id, .. } => Ok(Some(note_id.clone())),
            Operation::SetPermission { note_id, .. } => Ok(note_id.clone()),
            Operation::RevokePermission { note_id, .. } => Ok(note_id.clone()),
            Operation::CreateUserScript { .. }
            | Operation::UpdateUserScript { .. }
            | Operation::DeleteUserScript { .. }
            | Operation::RemovePeer { .. }
            | Operation::TransferRootOwnership { .. }
            | Operation::UpdateSchema { .. }
            | Operation::RetractOperation { .. }
            | Operation::JoinWorkspace { .. } => Ok(None),
        }
    }

    /// Check whether the resolved role permits the given operation.
    fn check_role_for_operation(
        &self,
        conn: &Connection,
        actor: &str,
        role: Role,
        operation: &Operation,
    ) -> Result<(), PermissionError> {
        match operation {
            Operation::CreateNote { .. } => {
                require_at_least(role, Role::Writer)?;
            }
            Operation::UpdateNote { .. }
            | Operation::UpdateField { .. }
            | Operation::SetTags { .. } => {
                require_at_least(role, Role::Writer)?;
            }
            Operation::DeleteNote { note_id, .. } => {
                if role < Role::Owner {
                    self.require_authorship(conn, actor, note_id, role)?;
                }
            }
            Operation::MoveNote { note_id, .. } => {
                if role < Role::Owner {
                    self.require_authorship(conn, actor, note_id, role)?;
                }
            }
            Operation::RetractOperation { .. } => {
                // Handled at workspace level (Root Owner only)
            }
            Operation::SetPermission {
                role: granted_role, ..
            } => {
                require_at_least(role, Role::Owner)?;
                if let Some(target_role) = Role::from_str(granted_role) {
                    if target_role > role {
                        return Err(PermissionError::Denied(format!(
                            "cannot grant {} (you hold {})",
                            granted_role,
                            role.as_str()
                        )));
                    }
                } else {
                    return Err(PermissionError::Denied(format!(
                        "invalid role: {}",
                        granted_role
                    )));
                }
            }
            Operation::RevokePermission { .. } => {
                require_at_least(role, Role::Owner)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// For Writer delete/move: verify the actor authored the target note.
    fn require_authorship(
        &self,
        conn: &Connection,
        actor: &str,
        note_id: &str,
        role: Role,
    ) -> Result<(), PermissionError> {
        require_at_least(role, Role::Writer)?;
        let created_by: String = conn
            .query_row(
                "SELECT created_by FROM notes WHERE id = ?1",
                rusqlite::params![note_id],
                |row| row.get(0),
            )
            .map_err(|_| PermissionError::Denied("note not found".into()))?;
        if created_by != actor {
            return Err(PermissionError::Denied(
                "writers can only delete/move notes they authored".into(),
            ));
        }
        Ok(())
    }
}

fn require_at_least(actual: Role, minimum: Role) -> Result<(), PermissionError> {
    if actual >= minimum {
        Ok(())
    } else {
        Err(PermissionError::Denied(format!(
            "requires at least {} (you hold {})",
            minimum.as_str(),
            actual.as_str()
        )))
    }
}

impl PermissionGate for RbacGate {
    fn protocol_id(&self) -> &'static str {
        "krillnotes/1"
    }

    fn authorize(
        &self,
        conn: &Connection,
        actor: &str,
        operation: &Operation,
    ) -> Result<(), PermissionError> {
        // Root Owner bypasses all checks
        if self.is_root_owner(actor) {
            return Ok(());
        }

        // Determine the scope note for this operation
        let scope_note_id = self.resolve_scope(operation)?;

        // Workspace-level operations are Root Owner only
        if scope_note_id.is_none() {
            return Err(PermissionError::Denied(
                "workspace-level operations require Root Owner".into(),
            ));
        }

        let note_id = scope_note_id.unwrap();
        let role = crate::resolver::resolve_role(conn, actor, &note_id)?
            .ok_or_else(|| PermissionError::Denied("no access to this subtree".into()))?;

        self.check_role_for_operation(conn, actor, role, operation)
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
