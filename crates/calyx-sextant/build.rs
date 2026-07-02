//! Emits `cfg(sextant_cuvs)` when the cuVS GPU index paths are actually
//! compiled into this build (#1130): the `cuda` feature is enabled AND the
//! target OS ships libcuvs (Linux only — RAPIDS provides no native
//! Windows/macOS packages, #1016).
//!
//! Source code must gate cuVS usage on `cfg(sextant_cuvs)`, never on
//! `cfg(feature = "cuda")` alone: feature flags are target-independent, so on
//! a non-Linux target the feature can be "on" while the `cuvs-sys`/`cudarc`
//! dependencies (target-gated in Cargo.toml) do not exist.
//!
//! `CARGO_CFG_TARGET_OS` (not `cfg!`) is read because build scripts compile
//! for the host while this decision is about the target.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(sextant_cuvs)");
    let cuda_feature = std::env::var_os("CARGO_FEATURE_CUDA").is_some();
    let target_os = std::env::var("CARGO_CFG_TARGET_OS")
        .expect("CALYX_SEXTANT_BUILD: cargo did not set CARGO_CFG_TARGET_OS");
    if cuda_feature && target_os == "linux" {
        println!("cargo::rustc-cfg=sextant_cuvs");
    }
}
