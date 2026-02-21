//! Internal domain modules for the Krillnotes core library.
//!
//! All public types from these modules are re-exported at the crate root
//! with `#[doc(inline)]`; import from there in preference to this module.

pub mod delete;
pub mod export;
pub mod device;
pub mod error;
pub mod note;
pub mod operation;
pub mod operation_log;
pub mod scripting;
pub mod storage;
pub mod user_script;
pub mod workspace;

#[doc(inline)]
pub use delete::{DeleteResult, DeleteStrategy};
#[doc(inline)]
pub use export::{
    export_workspace, ExportError, ExportNotes, ImportResult, ScriptManifest, ScriptManifestEntry,
    APP_VERSION,
};
#[doc(inline)]
pub use device::get_device_id;
#[doc(inline)]
pub use error::{KrillnotesError, Result};
#[doc(inline)]
pub use note::{FieldValue, Note};
#[doc(inline)]
pub use operation::Operation;
#[doc(inline)]
pub use operation_log::{OperationLog, OperationSummary, PurgeStrategy};
#[doc(inline)]
pub use scripting::{FieldDefinition, HookRegistry, Schema, ScriptRegistry};
#[doc(inline)]
pub use storage::Storage;
#[doc(inline)]
pub use user_script::UserScript;
#[doc(inline)]
pub use workspace::{AddPosition, Workspace};
