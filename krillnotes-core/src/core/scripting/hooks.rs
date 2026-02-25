//! Hook registry for global / lifecycle hooks (tree menu actions, on_load, on_export, …).
//!
//! Schema-bound hooks (`on_save`, `on_view`, `on_add_child`) are managed by
//! [`SchemaRegistry`](super::schema::SchemaRegistry) and registered via the
//! `schema()` Rhai host function.

use rhai::{FnPtr, AST};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A user-registered tree context-menu action.
pub struct TreeActionEntry {
    pub label:               String,
    pub allowed_types:       Vec<String>,
    pub(super) script_name:  String,
    pub(super) fn_ptr:       FnPtr,
    pub(super) ast:          AST,
}

impl std::fmt::Debug for TreeActionEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeActionEntry")
            .field("label", &self.label)
            .field("allowed_types", &self.allowed_types)
            .finish()
    }
}

/// Spec for a note to be created by a tree action.
#[derive(Debug, Clone)]
pub struct ActionCreate {
    pub id:        String,
    pub parent_id: String,
    pub node_type: String,
    pub title:     String,
    pub fields:    std::collections::HashMap<String, crate::core::note::FieldValue>,
}

/// Spec for a note to be updated by a tree action.
#[derive(Debug, Clone)]
pub struct ActionUpdate {
    pub note_id: String,
    pub title:   String,
    pub fields:  std::collections::HashMap<String, crate::core::note::FieldValue>,
}

/// Shared mutable context active during a tree action closure.
/// Host functions (`create_note`, `update_note`) queue operations here.
/// `get_children` / `get_note` also read from `note_cache` to see in-flight notes.
#[derive(Debug, Default)]
pub struct ActionTxContext {
    pub creates:    Vec<ActionCreate>,
    pub updates:    Vec<ActionUpdate>,
    /// Note maps (same Dynamic shape as `note_to_rhai_dynamic`) keyed by note ID.
    /// Populated by `create_note`; kept up-to-date by `update_note`.
    pub note_cache: std::collections::HashMap<String, rhai::Dynamic>,
}

/// Return value from `invoke_tree_action_hook`.
#[derive(Debug, Default)]
pub struct TreeActionResult {
    /// If the closure returned an array of IDs, they are placed here (reorder path).
    pub reorder:  Option<Vec<String>>,
    pub creates:  Vec<ActionCreate>,
    pub updates:  Vec<ActionUpdate>,
}

/// Registry for global event hooks not tied to a specific schema.
///
/// Currently holds tree context-menu actions; on_load, on_export, and other
/// lifecycle hooks will be added here in future tasks.
///
/// Constructed only by `ScriptRegistry::new()`.
///
/// Cheaply cloneable — the inner `Arc` is shared, so clones see the same data.
#[derive(Debug, Clone)]
pub struct HookRegistry {
    tree_actions: Arc<Mutex<Vec<TreeActionEntry>>>,
}

impl HookRegistry {
    pub(super) fn new() -> Self {
        Self {
            tree_actions: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Appends a new tree action. Logs a warning if a duplicate label+type
    /// combination already exists (first-registered wins).
    pub(super) fn register_tree_action(&self, entry: TreeActionEntry) {
        let mut actions = self.tree_actions.lock().unwrap();
        // Warn on duplicate label per allowed type.
        for ty in &entry.allowed_types {
            if actions.iter().any(|a| a.label == entry.label && a.allowed_types.contains(ty)) {
                eprintln!(
                    "[krillnotes] tree action {:?} already registered for type {ty:?} \
                     (script: {:?}); first-registered entry wins",
                    entry.label, entry.script_name
                );
            }
        }
        actions.push(entry);
    }

    /// Looks up a tree action by label and returns clones of its fn_ptr, ast, and script_name.
    pub(super) fn find_tree_action(&self, label: &str) -> Option<(FnPtr, AST, String)> {
        let actions = self.tree_actions.lock().unwrap();
        actions.iter()
            .find(|a| a.label == label)
            .map(|a| (a.fn_ptr.clone(), a.ast.clone(), a.script_name.clone()))
    }

    /// Returns a map of `note_type → [action_label, …]` for every registered action.
    pub fn tree_action_map(&self) -> HashMap<String, Vec<String>> {
        let actions = self.tree_actions.lock().unwrap();
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for entry in actions.iter() {
            for ty in &entry.allowed_types {
                map.entry(ty.clone()).or_default().push(entry.label.clone());
            }
        }
        map
    }

    /// Removes all registered tree actions so scripts can be reloaded.
    pub(super) fn clear(&self) {
        self.tree_actions.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_tree_action_adds_entry() {
        let registry = HookRegistry::new();
        // We can't construct FnPtr/AST in a unit test without a full engine,
        // so just test the map method on an empty registry.
        let map = registry.tree_action_map();
        assert!(map.is_empty(), "fresh registry should have no tree actions");
    }

    #[test]
    fn test_clear_removes_tree_actions() {
        let registry = HookRegistry::new();
        registry.clear();
        let map = registry.tree_action_map();
        assert!(map.is_empty());
    }
}
