//! Broker configuration authority, split by boundary:
//!
//! - [`schema`]: the raw, untrusted on-disk shape.
//! - [`validated`]: wrappers whose existence proves validation succeeded.
//! - [`rules`]: the fail-closed policy that converts one into the other.
//! - `primitives`: field-level checks shared by every rule.

use std::path::PathBuf;

use thiserror::Error;

mod primitives;
mod rules;
mod schema;
mod validated;

pub use rules::validate;
pub use schema::{
    BrokerConfig, ContainmentConfig, ExecutionRootConfig, JournalConfig, RootConfig, StateConfig,
};
pub use validated::{
    ValidatedConfig, ValidatedExecutionRootConfig, ValidatedRootConfig, ValidatedStateConfig,
};

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("unsupported schema_version {actual}; expected {expected}")]
    SchemaVersion { expected: u16, actual: u16 },
    #[error("invalid configuration field {field}: {reason}")]
    InvalidField { field: String, reason: String },
    #[error("unsafe configuration field {field}: required {required}")]
    UnsafeSetting {
        field: String,
        required: &'static str,
    },
    #[error("configured authority paths overlap: {first} and {second}")]
    OverlappingPaths { first: PathBuf, second: PathBuf },
}
