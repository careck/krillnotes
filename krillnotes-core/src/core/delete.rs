//! Delete strategy and result types for node removal operations.
//!
//! This module defines [`DeleteStrategy`] and [`DeleteResult`], which are used
//! when removing notes from a [`Workspace`](super::workspace::Workspace).
//!
//! ## Strategies
//!
//! Two strategies are supported:
//!
//! - [`DeleteStrategy::DeleteAll`] — removes the target note and all of its
//!   descendants recursively.
//! - [`DeleteStrategy::PromoteChildren`] — removes only the target note and
//!   re-parents its direct children to the deleted note's parent, preserving
//!   their relative order.
//!
//! ## Serialization
//!
//! Both types are serde-serializable so they can cross the Tauri IPC boundary:
//!
//! - `DeleteStrategy` variants serialize as PascalCase strings
//!   (`"DeleteAll"`, `"PromoteChildren"`), matching the values sent by the
//!   TypeScript front-end.
//! - `DeleteResult` fields serialize in camelCase (`deletedCount`,
//!   `affectedIds`), consistent with all other return types in this project.
//!
//! ## Examples
//!
//! ```rust
//! use krillnotes_core::{DeleteStrategy, DeleteResult};
//!
//! let strategy = DeleteStrategy::PromoteChildren;
//! let json = serde_json::to_string(&strategy).unwrap();
//! assert_eq!(json, r#""PromoteChildren""#);
//!
//! let result = DeleteResult {
//!     deleted_count: 3,
//!     affected_ids: vec!["a".to_string(), "b".to_string(), "c".to_string()],
//! };
//! let json = serde_json::to_string(&result).unwrap();
//! assert!(json.contains("deletedCount"));
//! assert!(json.contains("affectedIds"));
//! ```

// Rust guideline compliant 2026-02-19

use serde::{Deserialize, Serialize};

/// Determines how children are handled when a note is deleted.
///
/// This enum is serialized as a PascalCase string so that TypeScript can send
/// `"DeleteAll"` or `"PromoteChildren"` directly over the Tauri IPC channel
/// without any mapping layer.
///
/// # Examples
///
/// ```rust
/// use krillnotes_core::DeleteStrategy;
///
/// let s = DeleteStrategy::DeleteAll;
/// let json = serde_json::to_string(&s).unwrap();
/// assert_eq!(json, r#""DeleteAll""#);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum DeleteStrategy {
    /// Delete the target note and all of its descendants recursively.
    DeleteAll,

    /// Delete only the target note and re-parent its children to its former parent.
    PromoteChildren,
}

/// The outcome of a delete operation performed on a [`Workspace`](super::workspace::Workspace).
///
/// Contains a count of removed notes and the IDs of every note whose position
/// in the tree was affected — either because it was deleted or because it was
/// re-parented as a result of [`DeleteStrategy::PromoteChildren`].
///
/// # Examples
///
/// ```rust
/// use krillnotes_core::DeleteResult;
///
/// let result = DeleteResult {
///     deleted_count: 1,
///     affected_ids: vec!["note-id-abc".to_string()],
/// };
/// assert_eq!(result.deleted_count, 1);
/// assert_eq!(result.affected_ids.len(), 1);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteResult {
    /// The total number of notes that were permanently removed.
    pub deleted_count: usize,

    /// IDs of all notes that were deleted or structurally affected by the operation.
    pub affected_ids: Vec<String>,
}
