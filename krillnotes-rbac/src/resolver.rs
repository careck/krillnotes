// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

use rusqlite::Connection;

/// RBAC roles, ordered from most to least privileged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Reader = 1,
    Writer = 2,
    Owner = 3,
}

impl Role {
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
    // TODO: implement in Task 3
    let _ = (conn, user_id, note_id);
    Ok(None)
}
