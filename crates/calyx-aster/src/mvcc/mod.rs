//! Vault-wide MVCC sequence and snapshot scaffolding.

mod lease;
mod read_barrier;
mod store;

pub use lease::{Freshness, ReaderLease, SeqAllocator, Snapshot};
pub use read_barrier::{CALYX_ASTER_BASE_CORRUPT, ReadBarrier};
pub use store::{CfRead, VersionedCfStore};

#[cfg(test)]
mod tests;
