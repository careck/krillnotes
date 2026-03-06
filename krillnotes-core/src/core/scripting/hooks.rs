// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Result types for tree menu action invocations.
//!
//! Menu actions are now registered via `register_menu()` and stored in
//! [`SchemaRegistry`](super::schema::SchemaRegistry). This module provides the
//! result type returned by `invoke_tree_action_hook`.

use crate::core::save_transaction::SaveTransaction;

/// Return value from `invoke_tree_action_hook`.
#[derive(Debug, Default)]
pub struct TreeActionResult {
    /// If the closure returned an array of IDs, they are placed here (reorder path).
    pub reorder:     Option<Vec<String>>,
    /// Gated notes queued via `create_child` / `set_title` / `set_field` / `commit()`.
    pub transaction: SaveTransaction,
}
