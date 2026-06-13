#![deny(warnings)]

//! Directed graph primitives for Calyx grounding kernels.

pub mod betweenness;
mod error;
pub mod graph_builder;
pub mod lp_scaffold;
pub mod scc;
pub mod spectral;
mod spectral_linalg;

pub use betweenness::{betweenness, betweenness_top_k};
pub use error::{MincutError, Result};
pub use graph_builder::{AgreementEdge, CitationEdge, FrequencyEntry, build_assoc_graph};
pub use lp_scaffold::{
    ConstraintSense, LpConstraint, LpProblem, LpSolution, LpVariable, OptSense, SolveStatus,
    mfvs_lp_problem,
};
pub use scc::{CondensedEdge, CondensedGraph, SccResult, condensate, tarjan_scc};
pub use spectral::{
    EigenPair, SparseGraph, SpectralCache, SpectralCacheEntry, SpectralCacheKey, SpectralError,
    SpectralResult, eigenvector_centrality, gft_project, gft_reconstruct, laplacian_eigenmaps,
    laplacian_eigenmaps_with_max_iter, spectral_gap,
};
