//! Core Calyx identifiers, model contracts, and shared types.

pub mod ids;

pub use ids::{CxId, LensId, ParseIdError, SlotId, SlotKey, VaultId, content_address};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-core");
    }
}
