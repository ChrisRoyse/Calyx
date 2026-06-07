//! Aster storage engine skeleton for Calyx column families and WAL.

pub mod cf;
pub mod compaction;
pub mod manifest;
pub mod memtable;
pub mod mvcc;
pub mod sst;
pub mod vault;
pub mod wal;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-aster");
    }
}
