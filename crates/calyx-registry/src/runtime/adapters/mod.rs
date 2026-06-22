//! PH74 multimodal adapter lenses.

mod axis;
mod bridge;
mod config;
mod lens;
mod pack;
mod validate;

pub use axis::MultimodalAxis;
pub use config::{MultimodalAdapterConfig, MultimodalAdapterProvider};
pub use lens::{
    CALYX_ALLOW_NONCOMMERCIAL_LENSES_ENV, CALYX_LICENSE_DENIED, MultimodalAdapterLens,
    MultimodalAdapterSpec, allow_noncommercial_from_env, ensure_license_allowed,
    is_non_commercial_license,
};
pub use pack::{
    MultimodalLensPackEntry, default_multimodal_lens_specs, register_multimodal_lens_pack,
};

#[cfg(test)]
mod tests;
