//! Vault-wide MVCC sequence and snapshot scaffolding.

mod lease;
mod store;

pub use lease::{Freshness, ReaderLease, SeqAllocator, Snapshot};
pub use store::{CfRead, VersionedCfStore};

#[cfg(test)]
mod tests;
