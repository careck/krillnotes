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

    /// After revoking a user's grant, check all grants they issued.
    /// If the granter no longer holds a sufficient role for the grant,
    /// invalidate it and recurse.
    fn cascade_revoke(
        &self,
        conn: &Connection,
        revoked_user: &str,
    ) -> Result<(), PermissionError> {
        let mut stmt = conn.prepare(
            "SELECT note_id, user_id, role FROM note_permissions WHERE granted_by = ?1",
        )?;
        let downstream: Vec<(String, String, String)> = stmt
            .query_map(rusqlite::params![revoked_user], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        for (note_id, user_id, granted_role) in downstream {
            let granter_role = crate::resolver::resolve_role(conn, revoked_user, &note_id)?;
            let granted = Role::from_str(&granted_role);

            let still_valid = match (granter_role, granted) {
                (Some(granter), Some(granted)) => granter >= granted,
                _ => false,
            };

            if !still_valid {
                conn.execute(
                    "DELETE FROM note_permissions WHERE note_id = ?1 AND user_id = ?2",
                    rusqlite::params![note_id, user_id],
                )?;
                self.cascade_revoke(conn, &user_id)?;
            }
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
        conn: &Connection,
        operation: &Operation,
    ) -> Result<(), PermissionError> {
        match operation {
            Operation::SetPermission {
                note_id,
                user_id,
                role,
                granted_by,
                ..
            } => {
                let note_id = note_id.as_ref().ok_or_else(|| {
                    PermissionError::Denied("RBAC requires a note_id".into())
                })?;
                // Validate role
                Role::from_str(role).ok_or_else(|| {
                    PermissionError::Denied(format!("invalid role: {}", role))
                })?;
                conn.execute(
                    "INSERT INTO note_permissions (note_id, user_id, role, granted_by)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(note_id, user_id) DO UPDATE SET role = ?3, granted_by = ?4",
                    rusqlite::params![note_id, user_id, role, granted_by],
                )?;
                Ok(())
            }
            Operation::RevokePermission {
                note_id, user_id, ..
            } => {
                let note_id = note_id.as_ref().ok_or_else(|| {
                    PermissionError::Denied("RBAC requires a note_id".into())
                })?;
                conn.execute(
                    "DELETE FROM note_permissions WHERE note_id = ?1 AND user_id = ?2",
                    rusqlite::params![note_id, user_id],
                )?;
                Ok(())
            }
            Operation::RemovePeer { user_id, .. } => {
                conn.execute(
                    "DELETE FROM note_permissions WHERE user_id = ?1",
                    rusqlite::params![user_id],
                )?;
                Ok(())
            }
            Operation::TransferRootOwnership {
                new_owner,
                transferred_by,
                ..
            } => {
                let root_note_ids: Vec<String> = {
                    let mut stmt =
                        conn.prepare("SELECT id FROM notes WHERE parent_id IS NULL")?;
                    let ids = stmt
                        .query_map([], |row| row.get(0))?
                        .collect::<Result<Vec<_>, _>>()?;
                    ids
                };
                for root_id in root_note_ids {
                    conn.execute(
                        "INSERT OR REPLACE INTO note_permissions (note_id, user_id, role, granted_by)
                         VALUES (?1, ?2, 'owner', ?3)",
                        rusqlite::params![root_id, transferred_by, new_owner],
                    )?;
                }
                Ok(())
            }
            _ => Err(PermissionError::NotAPermissionOp),
        }
    }

    fn ensure_schema(&self, conn: &Connection) -> Result<(), PermissionError> {
        conn.execute_batch(include_str!("schema.sql"))?;
        Ok(())
    }
}

#[cfg(test)]
impl RbacGate {
    pub fn cascade_revoke_public(
        &self,
        conn: &Connection,
        user_id: &str,
    ) -> Result<(), PermissionError> {
        self.cascade_revoke(conn, user_id)
    }
}
