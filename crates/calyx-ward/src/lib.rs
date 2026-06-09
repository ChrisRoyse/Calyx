//! Ward guard profile types for per-slot cosine policy enforcement.

pub mod profile;

pub use profile::{CalibrationMeta, GuardId, GuardPolicy, GuardProfile, NoveltyAction};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-ward");
    }
}
