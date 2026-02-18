//! Internal domain modules for the Krillnotes core library.
//!
//! All public types from these modules are re-exported at the crate root
//! with `#[doc(inline)]`; import from there in preference to this module.

pub mod device;
pub mod error;
pub mod note;
pub mod operation;
pub mod operation_log;
pub mod scripting;
pub mod storage;
pub mod workspace;

#[doc(inline)]
pub use device::get_device_id;
#[doc(inline)]
pub use error::{KrillnotesError, Result};
#[doc(inline)]
pub use note::{FieldValue, Note};
#[doc(inline)]
pub use operation::Operation;
#[doc(inline)]
pub use operation_log::{OperationLog, PurgeStrategy};
#[doc(inline)]
pub use scripting::{FieldDefinition, Schema, SchemaRegistry};
#[doc(inline)]
pub use storage::Storage;
#[doc(inline)]
pub use workspace::{AddPosition, Workspace};
