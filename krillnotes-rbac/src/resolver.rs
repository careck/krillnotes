// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

use rusqlite::Connection;
use rusqlite::OptionalExtension;

/// RBAC roles, ordered from most to least privileged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Reader = 1,
    Writer = 2,
    Owner = 3,
}

impl Role {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "owner" => Some(Self::Owner),
            "writer" => Some(Self::Writer),
            "reader" => Some(Self::Reader),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Writer => "writer",
            Self::Reader => "reader",
        }
    }
}

/// Resolve the effective role of `user_id` on `note_id` by walking up the tree.
///
/// Returns `None` if no explicit grant exists anywhere in the ancestry chain
/// (default-deny).
pub fn resolve_role(
    conn: &Connection,
    user_id: &str,
    note_id: &str,
) -> Result<Option<Role>, rusqlite::Error> {
    let mut current_id = Some(note_id.to_string());

    while let Some(id) = current_id {
        // Check for explicit grant at this node
        let role: Option<String> = conn
            .query_row(
                "SELECT role FROM note_permissions WHERE note_id = ?1 AND user_id = ?2",
                rusqlite::params![id, user_id],
                |row| row.get(0),
            )
            .optional()?;

        if let Some(role_str) = role {
            return Ok(Role::from_str(&role_str));
        }

        // Walk up to parent
        current_id = conn
            .query_row(
                "SELECT parent_id FROM notes WHERE id = ?1",
                rusqlite::params![id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
    }

    Ok(None) // default-deny
}
