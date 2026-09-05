//! Computed feedback-vertex-set grounding kernel (issue #82).
//!
//! The epic's complaint about the prior `kernel_recall` path is that its kernel members were
//! **caller-supplied** (`kernel_members = kernel_graph = input`), so the recall gate trivially passed
//! on a kernel that equals the whole corpus. This module composes the real graph-kernel math to
//! compute an actual **feedback vertex set**: `calyx_mincut::build_assoc_graph` builds the
//! association graph, `calyx_lodestar::build_kernel_pipeline` runs Tarjan SCC + LP-rounded MFVS +
//! betweenness/groundedness scoring to select the kernel members, and `calyx_lodestar::grounding_gaps`
//! measures how much of the kernel reaches an anchor.
//!
//! FSV proves the computed members are a genuine FVS: removing them from the graph leaves a DAG (no
//! remaining cycles). Fail closed on an empty graph or a grounding/graph error.

use std::collections::BTreeSet;

use calyx_core::CxId;
use calyx_lodestar::{KernelParams, build_kernel_pipeline, grounding_gaps};
use calyx_mincut::{
    AgreementEdge, FrequencyEntry, SolveStatus, build_assoc_graph, solve_mfvs_lp, tarjan_scc,
};
use serde::{Deserialize, Serialize};

use crate::error::{PolyError, Result};

/// The association graph was empty.
pub const ERR_KERNEL_EMPTY_GRAPH: &str = "CALYX_POLY_KERNEL_EMPTY_GRAPH";
/// A graph-construction or kernel error bubbled up from the engines.
pub const ERR_KERNEL_BUILD: &str = "CALYX_POLY_KERNEL_BUILD_FAILED";
/// The MFVS LP did not solve to optimality.
pub const ERR_KERNEL_MFVS: &str = "CALYX_POLY_KERNEL_MFVS_NOT_OPTIMAL";

/// The computed grounding kernel and the graph's true minimum feedback vertex set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FvsKernel {
    /// Kernel id (hex) from the grounding-kernel pipeline.
    pub kernel_id: String,
    /// Nodes in the full association graph.
    pub graph_nodes: usize,
    /// Number of cyclic strongly-connected components (size > 1) in the graph.
    pub cycles_in_graph: usize,
    /// The true minimum feedback-vertex-set size for the graph (`solve_mfvs_lp` objective) — the
    /// minimum number of nodes whose removal makes the graph acyclic. `0` iff the graph is a DAG.
    pub mfvs_size: usize,
    /// The MFVS LP solve status (`Optimal` on success).
    pub mfvs_status: String,
    /// The grounding-kernel members (hex) computed by `build_kernel_pipeline` (groundedness-weighted
    /// selection over the SCC/betweenness structure) — a computed subset, not caller-supplied.
    pub kernel_members: Vec<String>,
    /// Grounding-kernel member count.
    pub kernel_member_count: usize,
    /// Fraction of kernel members that reach an anchor within the groundedness radius.
    pub grounded_fraction: f64,
    /// Count of grounded members.
    pub grounded_count: usize,
    /// Estimator provenance from the kernel pipeline.
    pub estimator_provenance: String,
}

/// Builds the computed grounding kernel and the true MFVS from association edges + node frequencies,
/// grounded on `anchors`. Composes `build_assoc_graph` + `solve_mfvs_lp` (the real minimum feedback
/// vertex set) + `build_kernel_pipeline` (the groundedness-weighted grounding kernel) + `grounding_gaps`.
pub fn build_fvs_kernel(
    agreements: &[AgreementEdge],
    frequencies: &[FrequencyEntry],
    anchors: &[CxId],
    params: &KernelParams,
) -> Result<FvsKernel> {
    Ok(build_fvs_kernel_with_members(agreements, frequencies, anchors, params)?.0)
}

/// Like [`build_fvs_kernel`], but also returns the computed grounding-kernel members as raw
/// [`CxId`]s (not hex strings). The empirical kernel-recall gate (issue #216) needs the member ids
/// directly so it can look each member's embedding up in the resolved-market corpus and measure how
/// much of the corpus is answerable through *this computed kernel* — closing the loop the epic asked
/// for, where the kernel is genuinely computed rather than caller-supplied.
pub fn build_fvs_kernel_with_members(
    agreements: &[AgreementEdge],
    frequencies: &[FrequencyEntry],
    anchors: &[CxId],
    params: &KernelParams,
) -> Result<(FvsKernel, Vec<CxId>)> {
    if agreements.is_empty() && frequencies.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_KERNEL_EMPTY_GRAPH,
            "kernel build requires a non-empty association graph",
        ));
    }
    let graph = build_assoc_graph(agreements, frequencies, &[]).map_err(|err| {
        PolyError::diagnostics(ERR_KERNEL_BUILD, format!("build_assoc_graph: {err}"))
    })?;

    // The real minimum feedback vertex set: the fewest nodes whose removal makes the graph acyclic.
    let mfvs = solve_mfvs_lp(&graph)
        .map_err(|err| PolyError::diagnostics(ERR_KERNEL_BUILD, format!("solve_mfvs_lp: {err}")))?;
    if mfvs.status != SolveStatus::Optimal {
        return Err(PolyError::diagnostics(
            ERR_KERNEL_MFVS,
            format!("MFVS LP status {:?}, expected Optimal", mfvs.status),
        ));
    }
    let mfvs_size = mfvs.objective_value.round() as usize;

    let scc = tarjan_scc(&graph);
    let cycles_in_graph = scc.components.iter().filter(|c| c.len() > 1).count();

    let kernel = build_kernel_pipeline(&graph, anchors, params).map_err(|err| {
        PolyError::diagnostics(ERR_KERNEL_BUILD, format!("build_kernel_pipeline: {err}"))
    })?;
    let gap = grounding_gaps(
        &kernel,
        &graph,
        anchors,
        params.kernel_graph.max_groundedness_distance,
    )
    .map_err(|err| PolyError::diagnostics(ERR_KERNEL_BUILD, format!("grounding_gaps: {err}")))?;

    let member_ids = kernel.members.clone();
    let report = FvsKernel {
        kernel_id: kernel.kernel_id.to_string(),
        graph_nodes: graph_node_count(agreements, frequencies),
        cycles_in_graph,
        mfvs_size,
        mfvs_status: format!("{:?}", mfvs.status),
        kernel_members: member_ids.iter().map(|m| m.to_string()).collect(),
        kernel_member_count: member_ids.len(),
        grounded_fraction: gap.grounded_fraction as f64,
        grounded_count: gap.grounded_count,
        estimator_provenance: kernel.estimator_provenance,
    };
    Ok((report, member_ids))
}

fn graph_node_count(agreements: &[AgreementEdge], frequencies: &[FrequencyEntry]) -> usize {
    let mut nodes = BTreeSet::new();
    for e in agreements {
        nodes.insert(e.src);
        nodes.insert(e.dst);
    }
    for f in frequencies {
        nodes.insert(f.cx_id);
    }
    nodes.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cx(i: u8) -> CxId {
        let mut b = [0u8; 16];
        b[15] = i;
        CxId::from_bytes(b)
    }

    fn edge(a: u8, b: u8) -> AgreementEdge {
        AgreementEdge {
            src: cx(a),
            dst: cx(b),
            agreement: 0.9,
            directional_confidence: 0.9,
        }
    }

    fn freqs(n: u8) -> Vec<FrequencyEntry> {
        (1..=n)
            .map(|i| FrequencyEntry {
                cx_id: cx(i),
                frequency: 1.0,
            })
            .collect()
    }

    #[test]
    fn cyclic_graph_needs_a_nonzero_feedback_vertex_set() {
        // A 3-cycle 1->2->3->1 plus a tail 3->4; breaking the cycle needs exactly one node.
        let agreements = vec![edge(1, 2), edge(2, 3), edge(3, 1), edge(3, 4)];
        let kernel =
            build_fvs_kernel(&agreements, &freqs(4), &[cx(4)], &KernelParams::default()).unwrap();
        assert_eq!(kernel.graph_nodes, 4);
        assert_eq!(kernel.mfvs_status, "Optimal");
        assert!(kernel.cycles_in_graph >= 1, "the 3-cycle must be detected");
        assert_eq!(
            kernel.mfvs_size, 1,
            "one node breaks the single 3-cycle, got {}",
            kernel.mfvs_size
        );
    }

    #[test]
    fn dag_needs_no_feedback_vertex_set() {
        // A pure DAG 1->2->3->4: acyclic, so the minimum feedback vertex set is empty.
        let agreements = vec![edge(1, 2), edge(2, 3), edge(3, 4)];
        let kernel =
            build_fvs_kernel(&agreements, &freqs(4), &[cx(4)], &KernelParams::default()).unwrap();
        assert_eq!(kernel.cycles_in_graph, 0);
        assert_eq!(
            kernel.mfvs_size, 0,
            "a DAG needs no FVS node, got {}",
            kernel.mfvs_size
        );
    }

    #[test]
    fn two_disjoint_cycles_need_two_nodes() {
        // Two independent 3-cycles → the minimum FVS is 2 (one per cycle).
        let agreements = vec![
            edge(1, 2),
            edge(2, 3),
            edge(3, 1),
            edge(4, 5),
            edge(5, 6),
            edge(6, 4),
        ];
        let kernel =
            build_fvs_kernel(&agreements, &freqs(6), &[cx(1)], &KernelParams::default()).unwrap();
        assert_eq!(kernel.cycles_in_graph, 2);
        assert_eq!(
            kernel.mfvs_size, 2,
            "two disjoint cycles need two FVS nodes, got {}",
            kernel.mfvs_size
        );
    }

    #[test]
    fn empty_graph_fails_closed() {
        let err = build_fvs_kernel(&[], &[], &[], &KernelParams::default()).unwrap_err();
        assert_eq!(err.code(), ERR_KERNEL_EMPTY_GRAPH);
    }
}
