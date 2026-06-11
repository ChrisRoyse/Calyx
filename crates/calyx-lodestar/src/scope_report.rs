use std::collections::{BTreeMap, BTreeSet};

use calyx_core::CxId;
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
    pub tau_star_estimate: usize,
    pub tau_star_exact: bool,
    pub bridge_count: usize,
}

impl ScopeKernelReport {
    pub fn from_scope_kernel(scope: &Scope, kernel: &Kernel) -> Self {
        Self::from_scope_kernel_with_bridge_count(scope, kernel, 0)
    }

    fn from_scope_kernel_with_bridge_count(
        scope: &Scope,
        kernel: &Kernel,
        bridge_count: usize,
    ) -> Self {
        Self {
            scope_name: format!("{scope:?}"),
            scope_hash: scope_hash(scope),
            kernel_size: kernel.members.len(),
            kernel_graph_size: kernel.kernel_graph.len(),
            kernel_only_recall: kernel.recall.kernel_only,
            grounded_fraction: kernel.groundedness.reached_anchor,
            approx_factor: kernel.recall.approx_factor,
            tau_star_estimate: kernel.recall.tau_star_estimate,
            tau_star_exact: kernel.recall.tau_star_exact,
            bridge_count,
        }
    }
}

pub fn report_all_scopes(kernels: &[(Scope, Kernel)]) -> Vec<ScopeKernelReport> {
    let bridge_counts = bridge_counts(kernels);
    kernels
        .iter()
        .zip(bridge_counts)
        .map(|((scope, kernel), bridge_count)| {
            ScopeKernelReport::from_scope_kernel_with_bridge_count(scope, kernel, bridge_count)
        })
        .collect()
}

fn bridge_counts(kernels: &[(Scope, Kernel)]) -> Vec<usize> {
    let mut occurrences = BTreeMap::<CxId, usize>::new();
    for (_, kernel) in kernels {
        for member in kernel.members.iter().copied().collect::<BTreeSet<_>>() {
            *occurrences.entry(member).or_default() += 1;
        }
    }
    kernels
        .iter()
        .map(|(_, kernel)| {
            kernel
                .members
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .filter(|member| occurrences.get(member).copied().unwrap_or_default() > 1)
                .count()
        })
        .collect()
}
