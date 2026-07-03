//! Resolved compile-time capability map for the `calyx` binary (#1130).
//!
//! Each entry is (capability name, compiled-in), where the value is a `cfg!`
//! const exported by the crate that OWNS the capability — so the map reports
//! what the linked crate instances actually compiled, not what feature names
//! were requested at the top level. #1130: a `--features cuda` binary passed
//! the #1116 deploy feature gate while calyx-sextant's cuVS path was compiled
//! out; deploy gates now assert these values instead.
//!
//! Adding a GPU (or otherwise deploy-critical) surface to a dependency crate?
//! Export a `cfg!` const there and add it here, then require it in the
//! deployment manifest generator's `required_capabilities_for` mapping.

pub(crate) const COMPILED: &[(&str, bool)] = &[
    ("forge-cuda", calyx_forge::CUDA_COMPILED),
    ("registry-candle-cuda", calyx_registry::CANDLE_CUDA_COMPILED),
    ("sextant-cuvs", calyx_sextant::CUVS_COMPILED),
];
