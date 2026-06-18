//! DiskANN on-disk graph index (PH68, server-only).
//!
//! Embedded vaults keep using the in-RAM HNSW from PH23; this module is the
//! NVMe-resident Vamana graph used by server-scale slots.

pub mod build;
pub mod concat;
#[cfg(feature = "cuda")]
mod cuvs_cagra;
pub mod dual;
pub mod graph;
pub mod pq;
pub mod search;
pub mod token;
mod token_sidecar;

pub use build::{
    DiskAnnBuildBackend, DiskAnnBuildParams, build_diskann_graph, build_diskann_graph_with_backend,
};
pub use concat::{ConcatCrossTermDiskAnn, ConcatCrossTermHit, ConcatCrossTermKey};
pub use dual::{
    Direction, DirectionalBoost, DualDiskAnnSearch, build_dual, build_dual_with_search,
    dual_graph_path, open_dual,
};
pub use graph::{
    DiskAnnGraphReader, DiskAnnGraphWriter, DiskAnnHeader, DiskAnnNodeRef, node_block_size,
    open_diskann_graph,
};
pub use pq::{DiskAnnPqBuildParams, DiskAnnPqIndex};
pub use search::{DiskAnnPqSearchBuild, DiskAnnSearch, DiskAnnSearchParams};
pub use token::TokenDiskAnnMaxSim;
