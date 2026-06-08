//! Append-only Ledger provenance primitives.

pub mod append;
pub mod codec;
pub mod entry;
pub mod kind;
pub mod redaction;

pub use append::{
    DirectoryLedgerStore, LedgerAppender, LedgerCfStore, LedgerRow, MemoryLedgerStore,
    reject_delete, reject_tombstone,
};
pub use codec::{decode, decode_header, encode};
pub use entry::{ActorId, LedgerEntry, SubjectId, compute_entry_hash};
pub use kind::EntryKind;
pub use redaction::{PayloadBuilder, RedactedInput, RedactionPolicy};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-ledger");
    }
}
