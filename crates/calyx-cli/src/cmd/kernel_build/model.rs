use std::collections::BTreeMap;

use calyx_core::CxId;
use calyx_lodestar::TypedPanelVectors;

pub(super) struct KernelBuildNodeProps {
    pub id: CxId,
    pub embeddings: TypedPanelVectors,
    pub anchored: bool,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub(super) struct KernelBuildRefinement {
    pub initial_ratio: f32,
    pub initial_kernel_only: f32,
    pub initial_members: usize,
    pub initial_kernel_graph: usize,
    pub support_members: usize,
    pub support_candidate_hits: usize,
    pub support_queries: usize,
    pub final_members: usize,
    pub final_kernel_graph: usize,
}
