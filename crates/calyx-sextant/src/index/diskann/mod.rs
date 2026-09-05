//! DiskANN on-disk graph index (PH68, server-only).
//!
//! Embedded vaults keep using the in-RAM HNSW from PH23; this module is the
//! NVMe-resident Vamana graph used by server-scale slots.

pub mod build;
mod cagra_dataset;
mod cagra_serve;
pub mod concat;
#[cfg(sextant_cuvs)]
mod cuvs_cagra;
pub mod dual;
pub mod graph;
pub mod pq;
pub mod search;
pub mod token;
mod token_sidecar;

pub use build::{
    DiskAnnBuildBackend, DiskAnnBuildMetric, DiskAnnBuildParams, DiskAnnBuildProgress,
    build_diskann_graph, build_diskann_graph_with_backend,
    build_diskann_graph_with_backend_and_progress, cagra_dataset_sidecar_path, cagra_sidecar_path,
    diskann_metric_sidecar_path, read_diskann_build_metric,
};
pub use cagra_serve::{
    CAGRA_PARTITIONED_MAX_DIM, CAGRA_PARTITIONED_MAX_REGIONS, CAGRA_PARTITIONED_MAX_SCRATCH_BYTES,
    CAGRA_SERVING_MAX_BATCH, CAGRA_SERVING_MAX_K, CagraPartitionRegion,
    CagraPartitionSearchRequest, CagraSearchRequest, CagraServingDiagnostics, CagraServingMetric,
    CagraServingRegion, cagra_partitioned_search, cagra_search_batch, cagra_serving_diagnostics,
    cagra_serving_region, configure_cagra_serving_cache,
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
pub use pq::{
    DISKANN_PQ_SMALL_CORPUS_ROWS, DiskAnnPqBuildDiagnostics, DiskAnnPqBuildExecution,
    DiskAnnPqBuildParams, DiskAnnPqIndex,
};
pub use search::{
    DiskAnnPqSearchBuild, DiskAnnSearch, DiskAnnSearchParams, RAW_SIDECAR_HEADER_SIZE,
    RAW_SIDECAR_MAGIC, RAW_SIDECAR_PACKED_VERSION, write_packed_raw_sidecar,
};
pub use token::TokenDiskAnnMaxSim;
