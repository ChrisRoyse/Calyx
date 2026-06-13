//! DiskANN on-disk graph index (PH68, server-only).
//!
//! Embedded vaults keep using the in-RAM HNSW from PH23; this module is the
//! NVMe-resident Vamana graph used by server-scale slots.

pub mod build;
pub mod concat;
pub mod graph;
pub mod search;
pub mod token;
mod token_sidecar;

pub use build::{DiskAnnBuildParams, build_diskann_graph};
pub use concat::{ConcatCrossTermDiskAnn, ConcatCrossTermHit, ConcatCrossTermKey};
pub use graph::{
    DiskAnnGraphReader, DiskAnnGraphWriter, DiskAnnHeader, DiskAnnNodeRef, node_block_size,
    open_diskann_graph,
};
pub use search::{DiskAnnSearch, DiskAnnSearchParams};
pub use token::TokenDiskAnnMaxSim;
