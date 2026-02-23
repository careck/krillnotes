//! Hook registry for global / lifecycle hooks (on_load, on_export, menu hooks, …).
//!
//! Schema-bound hooks (`on_save`, `on_view`) are managed by
//! [`SchemaRegistry`](super::schema::SchemaRegistry) and registered via the
//! `schema()` Rhai host function.

/// Registry for global event hooks not tied to a specific schema.
///
/// Currently a placeholder — global hooks (on_load, on_export, menu hooks, …)
/// will be added here in a future task.
///
/// Constructed only by `ScriptRegistry::new()`.
#[derive(Debug)]
pub struct HookRegistry {}

impl HookRegistry {
    // Intentionally unused until global hooks are wired in a future task.
    #[allow(dead_code)]
    pub(super) fn new() -> Self {
        Self {}
    }
}
