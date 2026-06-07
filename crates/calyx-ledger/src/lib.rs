//! Ledger provenance skeleton for append-only hash-chain audit state.

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-ledger");
    }
}
