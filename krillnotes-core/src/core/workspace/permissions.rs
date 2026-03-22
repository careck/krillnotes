// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Workspace-level wrappers for RBAC permission queries.
//!
//! These are read-only queries against the `note_permissions` and `notes`
//! tables.  They live in `krillnotes-core` (rather than calling through
//! `krillnotes_rbac::queries`) because the dependency arrow goes the other
//! way (`krillnotes-rbac` depends on `krillnotes-core`).

use crate::core::workspace::Workspace;
use crate::Result;
use rusqlite::OptionalExtension;
use serde::Serialize;
use std::collections::HashMap;

// ── Return types ────────────────────────────────────────────────────

/// A single permission grant row from `note_permissions`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionGrantRow {
    pub note_id: String,
    pub user_id: String,
    pub role: String,
    pub granted_by: String,
}

/// Extended role info including where the grant was anchored.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveRoleInfo {
    /// "owner", "writer", "reader", "root_owner", or "none"
    pub role: String,
    /// note_id where the grant is anchored, None if root_owner or no access
    pub inherited_from: Option<String>,
    /// Title of the anchor note (for display)
    pub inherited_from_title: Option<String>,
    /// Public key of who granted access, None if root_owner
    pub granted_by: Option<String>,
}

/// A grant inherited from an ancestor, with the anchor location.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InheritedGrant {
    pub grant: PermissionGrantRow,
    pub anchor_note_id: String,
    pub anchor_note_title: Option<String>,
}

/// A grant that would be invalidated by a cascade, with explanation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CascadeImpactRow {
    pub grant: PermissionGrantRow,
    pub reason: String,
}

// ── Workspace methods ───────────────────────────────────────────────

impl Workspace {
    /// Get explicit permission grants anchored at `note_id`.
    pub fn get_note_permissions(
        &self,
        note_id: &str,
    ) -> Result<Vec<PermissionGrantRow>> {
        let conn = self.connection();
        let mut stmt = conn.prepare(
            "SELECT note_id, user_id, role, granted_by \
             FROM note_permissions WHERE note_id = ?1",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![note_id], |row| {
                Ok(PermissionGrantRow {
                    note_id: row.get(0)?,
                    user_id: row.get(1)?,
                    role: row.get(2)?,
                    granted_by: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Get the effective role for the current user on `note_id`.
    ///
    /// Walks up the note tree from `note_id` to the root, returning the
    /// first matching grant.  The workspace owner is short-circuited to
    /// `"root_owner"` without any DB lookup.
    pub fn get_effective_role(
        &self,
        note_id: &str,
    ) -> Result<EffectiveRoleInfo> {
        let conn = self.connection();
        let user_id = self.identity_pubkey();
        let owner_pubkey = self.owner_pubkey();

        // Root owner short-circuit
        if user_id == owner_pubkey {
            return Ok(EffectiveRoleInfo {
                role: "root_owner".to_string(),
                inherited_from: None,
                inherited_from_title: None,
                granted_by: None,
            });
        }

        let mut current_id = Some(note_id.to_string());
        while let Some(id) = current_id {
            // Check for explicit grant at this node
            let grant: Option<(String, String)> = conn
                .query_row(
                    "SELECT role, granted_by FROM note_permissions \
                     WHERE note_id = ?1 AND user_id = ?2",
                    rusqlite::params![id, user_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;

            if let Some((role, granted_by)) = grant {
                let inherited_from = if id != note_id {
                    Some(id.clone())
                } else {
                    None
                };
                let inherited_from_title = if let Some(ref anchor_id) = inherited_from {
                    conn.query_row(
                        "SELECT title FROM notes WHERE id = ?1",
                        [anchor_id],
                        |row| row.get(0),
                    )
                    .optional()?
                } else {
                    None
                };
                return Ok(EffectiveRoleInfo {
                    role,
                    inherited_from,
                    inherited_from_title,
                    granted_by: Some(granted_by),
                });
            }

            // Walk up
            current_id = conn
                .query_row(
                    "SELECT parent_id FROM notes WHERE id = ?1",
                    rusqlite::params![id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()?
                .flatten();
        }

        Ok(EffectiveRoleInfo {
            role: "none".to_string(),
            inherited_from: None,
            inherited_from_title: None,
            granted_by: None,
        })
    }

    /// Get grants inherited from ancestors of `note_id`.
    ///
    /// Walks up from the parent of `note_id` to the root, collecting all
    /// grants found along the way (excluding grants anchored on `note_id`
    /// itself).
    pub fn get_inherited_permissions(
        &self,
        note_id: &str,
    ) -> Result<Vec<InheritedGrant>> {
        let conn = self.connection();
        let mut results = Vec::new();

        // Start from parent, not self
        let mut current_id: Option<String> = conn
            .query_row(
                "SELECT parent_id FROM notes WHERE id = ?1",
                [note_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();

        while let Some(id) = current_id {
            let title: Option<String> = conn
                .query_row(
                    "SELECT title FROM notes WHERE id = ?1",
                    [&id],
                    |row| row.get(0),
                )
                .optional()?;

            let mut stmt = conn.prepare(
                "SELECT note_id, user_id, role, granted_by \
                 FROM note_permissions WHERE note_id = ?1",
            )?;
            let grants = stmt
                .query_map([&id], |row| {
                    Ok(PermissionGrantRow {
                        note_id: row.get(0)?,
                        user_id: row.get(1)?,
                        role: row.get(2)?,
                        granted_by: row.get(3)?,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;

            for grant in grants {
                results.push(InheritedGrant {
                    grant,
                    anchor_note_id: id.clone(),
                    anchor_note_title: title.clone(),
                });
            }

            // Walk up
            current_id = conn
                .query_row(
                    "SELECT parent_id FROM notes WHERE id = ?1",
                    [&id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()?
                .flatten();
        }

        Ok(results)
    }

    /// Batch-compute effective roles for all notes (for tree dot rendering).
    ///
    /// Uses top-down grant propagation to avoid per-note ancestor walks.
    /// The workspace owner is short-circuited to `"root_owner"` for every note.
    pub fn get_all_effective_roles(&self) -> Result<HashMap<String, String>> {
        let conn = self.connection();
        let user_id = self.identity_pubkey();
        let owner_pubkey = self.owner_pubkey();

        // 1. Root owner: every note gets "root_owner"
        if user_id == owner_pubkey {
            let mut result = HashMap::new();
            let mut stmt = conn.prepare("SELECT id FROM notes")?;
            let ids = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            for id in ids {
                result.insert(id, "root_owner".to_string());
            }
            return Ok(result);
        }

        // 2. Fetch all grants for this user
        let mut stmt = conn.prepare(
            "SELECT note_id, role FROM note_permissions WHERE user_id = ?1",
        )?;
        let grants: Vec<(String, String)> = stmt
            .query_map([user_id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        if grants.is_empty() {
            return Ok(HashMap::new());
        }

        // Collect grant anchor note_ids for quick lookup
        let grant_anchors: HashMap<String, String> = grants.into_iter().collect();

        // 3. Build parent->children adjacency
        let mut children_map: HashMap<String, Vec<String>> = HashMap::new();
        let mut stmt = conn.prepare("SELECT id, parent_id FROM notes")?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        for (id, parent_id) in &rows {
            if let Some(pid) = parent_id {
                children_map
                    .entry(pid.clone())
                    .or_default()
                    .push(id.clone());
            }
        }

        // 4. BFS from each grant anchor downward
        let mut result = HashMap::new();
        for (anchor_id, role) in &grant_anchors {
            let mut queue = std::collections::VecDeque::new();
            queue.push_back(anchor_id.clone());

            while let Some(current) = queue.pop_front() {
                // If this node has its own grant and it's not the starting anchor, skip
                if current != *anchor_id && grant_anchors.contains_key(&current) {
                    continue;
                }
                result.insert(current.clone(), role.clone());

                if let Some(children) = children_map.get(&current) {
                    for child in children {
                        queue.push_back(child.clone());
                    }
                }
            }
        }

        Ok(result)
    }

    /// Preview which downstream grants would be invalidated if `user_id`
    /// were changed to `new_role` on `note_id`.
    ///
    /// For each grant where `granted_by = user_id`, checks whether the
    /// new role would still satisfy the "must be Owner to grant" rule.
    ///
    /// This is a read-only preview -- no data is modified.
    pub fn preview_cascade(
        &self,
        _note_id: &str,
        user_id: &str,
        new_role: &str,
    ) -> Result<Vec<CascadeImpactRow>> {
        let can_still_grant = new_role == "owner";

        // If the user would still be Owner, no grants are invalidated
        if can_still_grant {
            return Ok(Vec::new());
        }

        let conn = self.connection();

        // Find all grants issued by this user
        let mut stmt = conn.prepare(
            "SELECT note_id, user_id, role, granted_by \
             FROM note_permissions WHERE granted_by = ?1",
        )?;
        let rows = stmt
            .query_map([user_id], |row| {
                Ok(PermissionGrantRow {
                    note_id: row.get(0)?,
                    user_id: row.get(1)?,
                    role: row.get(2)?,
                    granted_by: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let reason = format!(
            "no longer Owner \u{2014} cannot grant any role (demoted to {})",
            new_role
        );

        Ok(rows
            .into_iter()
            .map(|grant| CascadeImpactRow {
                grant,
                reason: reason.clone(),
            })
            .collect())
    }
}
