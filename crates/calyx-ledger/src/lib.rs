//! Append-only Ledger provenance primitives.

pub mod append;
pub mod codec;
pub mod entry;
pub mod group_commit;
pub mod kind;
pub mod merkle;
pub mod redaction;

pub use append::{
    DirectoryLedgerStore, LedgerAppender, LedgerCfStore, LedgerRow, MemoryLedgerStore,
    reject_delete, reject_tombstone,
};
pub use codec::{decode, decode_header, encode};
pub use entry::{ActorId, LedgerEntry, SubjectId, compute_entry_hash};
pub use group_commit::{
    DefaultLedgerHook, LedgerBatchRow, LedgerGroupCommitHook, LedgerWriteBatch, WriteBatch,
    WriteOp, ingest_kind_for, ledger_batch_key,
};
pub use kind::EntryKind;
pub use merkle::{
    MERKLE_EMPTY_ROOT, MERKLE_SIGNING_DOMAIN, MerkleExportBundle, combine_hash, leaf_hash,
    merkle_root, merkle_root_of_hashes, sign_root, verify_signature,
};
pub use redaction::{PayloadBuilder, RedactedInput, RedactionPolicy};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-ledger");
    }
}
