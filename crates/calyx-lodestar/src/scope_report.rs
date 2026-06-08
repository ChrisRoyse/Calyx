use serde::{Deserialize, Serialize};

use crate::{Kernel, Scope, scope_hash};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScopeKernelReport {
    pub scope_name: String,
    pub scope_hash: [u8; 32],
    pub kernel_size: usize,
    pub kernel_graph_size: usize,
    pub kernel_only_recall: f32,
    pub grounded_fraction: f32,
    pub approx_factor: f64,
}

impl ScopeKernelReport {
    pub fn from_scope_kernel(scope: &Scope, kernel: &Kernel) -> Self {
        Self {
            scope_name: format!("{scope:?}"),
            scope_hash: scope_hash(scope),
            kernel_size: kernel.members.len(),
            kernel_graph_size: kernel.kernel_graph.len(),
            kernel_only_recall: kernel.recall.kernel_only,
            grounded_fraction: kernel.groundedness.reached_anchor,
            approx_factor: kernel.recall.approx_factor,
        }
    }
}

pub fn report_all_scopes(kernels: &[(Scope, Kernel)]) -> Vec<ScopeKernelReport> {
    kernels
        .iter()
        .map(|(scope, kernel)| ScopeKernelReport::from_scope_kernel(scope, kernel))
        .collect()
}
