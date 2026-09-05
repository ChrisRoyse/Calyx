//! Frozen deterministic encoders for explicit structured record atoms.

mod binding;
mod codec;
mod date;
mod encode;
mod error;

pub use binding::{
    DateComponent, NumericDerivation, NumericScaling, NumericSourceType, StructuredBinding,
    StructuredEncoder, StructuredSelector, StructuredSourceType,
};
pub use error::{
    CALYX_STRUCTURED_BINDING_INVALID, CALYX_STRUCTURED_CATEGORY_UNKNOWN,
    CALYX_STRUCTURED_DATE_INVALID, CALYX_STRUCTURED_DERIVATION_INVALID,
};

pub(crate) use error::structured_runtime_error;
