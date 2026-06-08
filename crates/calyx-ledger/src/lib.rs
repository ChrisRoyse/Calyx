//! Append-only Ledger provenance primitives.

pub mod codec;
pub mod entry;
pub mod kind;

pub use codec::{decode, decode_header, encode};
pub use entry::{ActorId, LedgerEntry, SubjectId, compute_entry_hash};
pub use kind::EntryKind;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-ledger");
    }
}
