pub mod core;

// Re-export commonly used types
pub use core::{
    device::get_device_id,
    error::{KrillnotesError, Result},
    note::{FieldValue, Note},
    operation::Operation,
    operation_log::{OperationLog, PurgeStrategy},
    scripting::{FieldDefinition, Schema, SchemaRegistry},
    storage::Storage,
    workspace::{AddPosition, Workspace},
};
