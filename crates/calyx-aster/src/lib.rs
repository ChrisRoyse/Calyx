//! Aster storage engine skeleton for Calyx column families and WAL.

pub mod base_page_index;
pub mod cf;
pub mod collection;
pub mod compaction;
pub mod dedup;
pub mod erase;
mod file_lock;
mod fsync;
pub mod gc;
pub mod index;
pub mod layers;
pub mod ledger_head;
pub mod ledger_view;
pub mod manifest;
pub mod media_artifact;
pub mod memtable;
pub mod mmap_col;
pub mod mvcc;
pub mod olap;
pub mod plain_column;
pub mod plain_graph;
pub mod pressure;
pub mod recurrence;
pub mod redaction;
pub mod residency;
pub mod resource;
pub mod retained_input;
pub mod retention;
pub mod security;
pub mod sst;
pub mod storage_names;
pub mod stream;
pub mod stride_fsv;
pub mod supply_chain;
pub mod timetravel;
pub mod txn;
pub mod vault;
pub mod verify_restore;
pub mod wal;

pub use dedup::{
    CompressionRatio, Domain, DomainCompressionStats, compression_ratio, domain_compression_stats,
};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-aster");
    }
}
