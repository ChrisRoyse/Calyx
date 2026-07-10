use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

use calyx_core::CxId;
use calyx_mincut::{
    LpSolution, MincutError, SccResult, SolveStatus, solve_mfvs_lp, verify_feedback_vertex_set,
};
use calyx_paths::AssocGraph;
use serde::{Deserialize, Serialize};

use crate::{LodestarError, Result};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KernelGraphParams {
    pub target_fraction: f32,
    pub max_groundedness_distance: usize,
    pub degree_weight: f32,
    pub betweenness_weight: f32,
    pub groundedness_weight: f32,
}

impl Default for KernelGraphParams {
    fn default() -> Self {
        Self {
            target_fraction: 0.10,
            max_groundedness_distance: 3,
            degree_weight: 0.40,
            betweenness_weight: 0.40,
            groundedness_weight: 0.20,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LpRoundParams {
    pub threshold: f64,
    pub fallback_to_heuristic: bool,
}

impl Default for LpRoundParams {
    fn default() -> Self {
        Self {
            threshold: 0.5,
            fallback_to_heuristic: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeScore {
    pub id: CxId,
    pub degree_score: f64,
    pub betweenness_score: f64,
    pub groundedness_distance: Option<usize>,
    pub groundedness_score: f64,
    #[serde(default)]
    pub frequency_bonus: f32,
    pub total_score: f64,
}

pub type KernelNodeScore = NodeScore;

#[derive(Clone, Debug)]
pub struct KernelGraph {
    pub graph: AssocGraph,
    pub selected: Vec<CxId>,
    pub source_fraction: f32,
    pub lp_fraction: Option<f32>,
    pub params: KernelGraphParams,
    pub scores: Vec<NodeScore>,
    pub warnings: Vec<String>,
}

pub fn select_kernel_graph(
    graph: &AssocGraph,
    scc: &SccResult,
    betweenness: &BTreeMap<CxId, f64>,
    anchors: &[CxId],
    params: &KernelGraphParams,
) -> Result<KernelGraph> {
    validate_params(params)?;
    if graph.is_empty() {
        return Err(LodestarError::KernelEmptyGraph);
    }
    if scc.component_of.len() != graph.node_count() {
        return Err(LodestarError::KernelInvalidParams {
            detail: "SCC result does not cover graph".to_string(),
        });
    }
    let scores = score_nodes(graph, betweenness, anchors, params)?;
    let take = ((params.target_fraction * graph.node_count() as f32).ceil() as usize)
        .max(1)
        .min(graph.node_count());
    let selected: Vec<_> = scores.iter().take(take).map(|score| score.id).collect();
    build_kernel_graph(graph, selected, params.clone(), scores, None, Vec::new())
}

pub fn groundedness_distance(
    graph: &AssocGraph,
    node: CxId,
    anchors: &[CxId],
    max_hops: usize,
) -> Result<Option<usize>> {
    let start = graph.require_node_index(node)?;
    if anchors.contains(&node) {
        return Ok(Some(0));
    }
    let anchor_indices: BTreeSet<_> = anchors
        .iter()
        .filter_map(|anchor| graph.node_index(*anchor))
        .collect();
    if anchor_indices.is_empty() {
        return Ok(None);
    }
    let mut seen = BTreeSet::from([start]);
    let mut queue = VecDeque::from([(start, 0_usize)]);
    while let Some((current, hops)) = queue.pop_front() {
        if hops == max_hops {
            continue;
        }
        for edge in graph.out_edges_by_index(current) {
            if !seen.insert(edge.dst) {
                continue;
            }
            let next_hops = hops + 1;
            if anchor_indices.contains(&edge.dst) {
                return Ok(Some(next_hops));
            }
            queue.push_back((edge.dst, next_hops));
        }
    }
    Ok(None)
}

pub fn lp_round_kernel_graph(
    kernel_graph: &KernelGraph,
    params: &LpRoundParams,
) -> Result<KernelGraph> {
    validate_lp_params(params)?;
    if params.fallback_to_heuristic {
        return Err(LodestarError::KernelLpUnavailable {
            detail: "heuristic fallback is disabled for the bounded exact MFVS solver".to_string(),
        });
    }
    let solution = solve_mfvs_lp(&kernel_graph.graph).map_err(map_lp_solver_error)?;
    lp_round_kernel_graph_from_solution(kernel_graph, params, &solution)
}

pub fn lp_round_kernel_graph_from_solution(
    kernel_graph: &KernelGraph,
    params: &LpRoundParams,
    solution: &LpSolution,
) -> Result<KernelGraph> {
    validate_lp_params(params)?;
    match solution.status {
        SolveStatus::Optimal => {}
        SolveStatus::Infeasible => {
            return Err(LodestarError::KernelLpInfeasible {
                detail: "LP solution status is infeasible".to_string(),
            });
        }
        other => {
            return Err(LodestarError::KernelLpUnavailable {
                detail: format!(
                    "LP solution status {other:?} is not optimal; heuristic fallback is disabled"
                ),
            });
        }
    }
    let ids: Vec<_> = kernel_graph.graph.node_ids().collect();
    validate_lp_solution_values(solution, ids.len())?;
    let selected: Vec<_> = ids
        .iter()
        .zip(&solution.values)
        .filter_map(|(id, value)| (*value >= params.threshold).then_some(*id))
        .collect();
    if !verify_feedback_vertex_set(&kernel_graph.graph, &selected)? {
        return Err(LodestarError::KernelLpInfeasible {
            detail: "rounded LP solution does not hit every directed cycle".to_string(),
        });
    }
    let lp_fraction = if ids.is_empty() {
        0.0
    } else {
        selected.len() as f32 / ids.len() as f32
    };
    build_kernel_graph(
        &kernel_graph.graph,
        selected,
        kernel_graph.params.clone(),
        kernel_graph.scores.clone(),
        Some(lp_fraction),
        kernel_graph.warnings.clone(),
    )
}

fn validate_lp_solution_values(solution: &LpSolution, expected_len: usize) -> Result<()> {
    if solution.values.len() != expected_len {
        return Err(LodestarError::KernelInvalidParams {
            detail: format!(
                "LP solution has {} values for {} nodes",
                solution.values.len(),
                expected_len
            ),
        });
    }
    if !solution.objective_value.is_finite() {
        return Err(LodestarError::KernelInvalidParams {
            detail: format!(
                "LP solution objective_value must be finite, got {}",
                solution.objective_value
            ),
        });
    }
    for (idx, value) in solution.values.iter().copied().enumerate() {
        if !value.is_finite() {
            return Err(LodestarError::KernelInvalidParams {
                detail: format!("LP solution value at index {idx} must be finite, got {value}"),
            });
        }
        if !(0.0..=1.0).contains(&value) {
            return Err(LodestarError::KernelInvalidParams {
                detail: format!("LP solution value at index {idx}={value} is outside [0, 1]"),
            });
        }
    }
    let objective_sum: f64 = solution.values.iter().sum();
    if (solution.objective_value - objective_sum).abs() > 1.0e-6 {
        return Err(LodestarError::KernelInvalidParams {
            detail: format!(
                "LP solution objective_value {} does not match sum(values) {objective_sum}",
                solution.objective_value
            ),
        });
    }
    Ok(())
}

/// Total (in + out) degree per node index, in one O(V+E) pass over the edge
/// list. Replaces per-node `in_degree` calls, each of which is O(E).
fn total_degrees(graph: &AssocGraph) -> Result<Vec<usize>> {
    let mut degrees = vec![0_usize; graph.node_count()];
    for edge in graph.edges() {
        degrees[edge.src] += 1;
        degrees[edge.dst] += 1;
    }
    Ok(degrees)
}

fn score_nodes(
    graph: &AssocGraph,
    betweenness: &BTreeMap<CxId, f64>,
    anchors: &[CxId],
    params: &KernelGraphParams,
) -> Result<Vec<NodeScore>> {
    // Precompute total degree per node in a single O(V+E) pass. `in_degree` is
    // O(E) per call (it scans every edge), so the previous per-node calls were
    // O(V·E) — intractable on the corpus graph. `out_degree` is O(1) (adjacency
    // ranges); in-degrees we count once over all edges.
    let degrees = total_degrees(graph)?;
    let max_degree = degrees.iter().copied().max().unwrap_or(1).max(1) as f64;
    // Anchor membership in O(1); a node that is itself an anchor is grounded at
    // distance 0 without the BFS (corpus is fully anchored → no BFS at all).
    let anchor_set: HashSet<CxId> = anchors.iter().copied().collect();
    let mut scored = Vec::new();
    for (index, id) in graph.node_ids().enumerate() {
        let degree = degrees[index] as f64 / max_degree;
        let bet = *betweenness.get(&id).unwrap_or(&0.0);
        let gnd = if anchor_set.contains(&id) {
            Some(0)
        } else {
            groundedness_distance(graph, id, anchors, params.max_groundedness_distance)?
        };
        let gnd_score = match gnd {
            Some(distance) => {
                1.0 - (distance.min(params.max_groundedness_distance) as f64
                    / params.max_groundedness_distance.max(1) as f64)
            }
            None => 0.0,
        };
        let total = degree * params.degree_weight as f64
            + bet * params.betweenness_weight as f64
            + gnd_score * params.groundedness_weight as f64;
        scored.push(NodeScore {
            id,
            degree_score: degree,
            betweenness_score: bet,
            groundedness_distance: gnd,
            groundedness_score: gnd_score,
            frequency_bonus: 0.0,
            total_score: total,
        });
    }
    sort_node_scores(&mut scored);
    Ok(scored)
}

pub fn sort_node_scores(scores: &mut [NodeScore]) {
    scores.sort_by(|left, right| {
        right
            .total_score
            .total_cmp(&left.total_score)
            .then_with(|| left.id.as_bytes().cmp(right.id.as_bytes()))
    });
}

pub(crate) fn rebuild_kernel_graph(
    source: &AssocGraph,
    kernel_graph: &mut KernelGraph,
    selected: Vec<CxId>,
) -> Result<()> {
    let graph = selected_subgraph(source, &selected)?;
    kernel_graph.graph = graph;
    kernel_graph.selected = selected;
    kernel_graph.source_fraction =
        kernel_graph.selected.len() as f32 / source.node_count().max(1) as f32;
    Ok(())
}

fn build_kernel_graph(
    source: &AssocGraph,
    selected: Vec<CxId>,
    params: KernelGraphParams,
    scores: Vec<NodeScore>,
    lp_fraction: Option<f32>,
    warnings: Vec<String>,
) -> Result<KernelGraph> {
    let graph = selected_subgraph(source, &selected)?;
    let source_fraction = selected.len() as f32 / source.node_count().max(1) as f32;
    Ok(KernelGraph {
        graph,
        selected,
        source_fraction,
        lp_fraction,
        params,
        scores,
        warnings,
    })
}

fn selected_subgraph(source: &AssocGraph, selected: &[CxId]) -> Result<AssocGraph> {
    let selected_set: BTreeSet<_> = selected.iter().copied().collect();
    let mut builder = AssocGraph::builder();
    for id in selected {
        builder.add_node(*id, source.node_weight(*id)?)?;
    }
    for edge in source.edges() {
        let (src, dst) = source.edge_endpoints(*edge);
        if selected_set.contains(&src) && selected_set.contains(&dst) {
            builder.add_edge(src, dst, edge.weight)?;
        }
    }
    Ok(builder.build())
}

fn map_lp_solver_error(error: MincutError) -> LodestarError {
    match error {
        err @ MincutError::LpSolverLimit { .. } => LodestarError::KernelLpUnavailable {
            detail: err.to_string(),
        },
        other => other.into(),
    }
}

fn validate_params(params: &KernelGraphParams) -> Result<()> {
    let weight_sum = params.degree_weight + params.betweenness_weight + params.groundedness_weight;
    if !(0.0..=1.0).contains(&params.target_fraction) || params.target_fraction == 0.0 {
        return Err(LodestarError::KernelInvalidParams {
            detail: format!(
                "target_fraction={} must be in (0,1]",
                params.target_fraction
            ),
        });
    }
    if !weight_sum.is_finite() || (weight_sum - 1.0).abs() > 1e-6 {
        return Err(LodestarError::KernelInvalidParams {
            detail: format!("score weights sum to {weight_sum}, expected 1.0"),
        });
    }
    Ok(())
}

fn validate_lp_params(params: &LpRoundParams) -> Result<()> {
    if params.threshold.is_finite() && (0.0..=1.0).contains(&params.threshold) {
        Ok(())
    } else {
        Err(LodestarError::KernelInvalidParams {
            detail: format!("LP threshold {} must be in [0,1]", params.threshold),
        })
    }
}
