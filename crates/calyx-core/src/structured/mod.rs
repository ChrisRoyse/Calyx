//! Canonical typed record input for structured and hybrid constellations.

mod codec;
mod error;
mod pointer;
mod record;

pub use error::{
    CALYX_STRUCTURED_FIELD_MISSING, CALYX_STRUCTURED_NON_FINITE, CALYX_STRUCTURED_RECORD_INVALID,
    CALYX_STRUCTURED_RESERVED_METADATA, CALYX_STRUCTURED_SCHEMA_INVALID,
    CALYX_STRUCTURED_SCHEMA_MISMATCH, CALYX_STRUCTURED_SELECTOR_INVALID,
    CALYX_STRUCTURED_TYPE_MISMATCH,
};
pub use pointer::{escape_json_pointer_token, validate_json_field_pointer};
pub use record::{
    METADATA_STRUCTURED_RECORD, METADATA_STRUCTURED_SCHEMA, STRUCTURED_METADATA_PREFIX,
    StructuredRecord, StructuredValue,
};
