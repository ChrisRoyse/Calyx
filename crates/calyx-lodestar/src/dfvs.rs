use std::collections::BTreeSet;

use calyx_core::CxId;
use calyx_mincut::tarjan_scc;
use calyx_paths::AssocGraph;
use serde::{Deserialize, Serialize};

use crate::{KernelGraph, LodestarError, Result};

const EXACT_SEARCH_MAX_NODES: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DfvsMethod {
    ExactOrGreedyLocalSearch,
    Tournament2Approx,
    BoundedGenus,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DfvsResult {
    pub members: Vec<CxId>,
    pub approx_factor: f64,
    pub tau_star_estimate: usize,
    pub tau_star_exact: bool,
    pub method: DfvsMethod,
}

pub fn dfvs_approx(kernel_graph: &KernelGraph) -> Result<DfvsResult> {
    let graph = &kernel_graph.graph;
    if graph.is_empty() {
        return Ok(empty_result(DfvsMethod::ExactOrGreedyLocalSearch));
    }
    if is_tournament(graph) {
        return tournament_2approx(graph);
    }
    let genus = genus_estimate(graph);
    if genus <= 2 {
        return bounded_genus_approx(graph, genus);
    }
    solve_with_method(graph, DfvsMethod::ExactOrGreedyLocalSearch, None)
}

pub fn is_tournament(graph: &AssocGraph) -> bool {
    let ids: Vec<_> = graph.node_ids().collect();
    for left in 0..ids.len() {
        for right in left + 1..ids.len() {
            let a = graph.node_index(ids[left]).expect("node index");
            let b = graph.node_index(ids[right]).expect("node index");
            let a_to_b = graph.out_edges_by_index(a).iter().any(|edge| edge.dst == b);
            let b_to_a = graph.out_edges_by_index(b).iter().any(|edge| edge.dst == a);
            if !a_to_b && !b_to_a {
                return false;
            }
        }
    }
    true
}

pub fn tournament_2approx(graph: &AssocGraph) -> Result<DfvsResult> {
    solve_with_method(graph, DfvsMethod::Tournament2Approx, Some(2.0))
}

pub fn genus_estimate(graph: &AssocGraph) -> usize {
    let v = graph.node_count() as isize;
    let e = graph.edge_count() as isize;
    if v < 3 {
        return 0;
    }
    ((e - 3 * v + 6).max(0) as usize).div_ceil(6)
}

pub fn bounded_genus_approx(graph: &AssocGraph, genus: usize) -> Result<DfvsResult> {
    if genus > 100 {
        return Err(LodestarError::DfvsGenusTooLarge { genus });
    }
    solve_with_method(
        graph,
        DfvsMethod::BoundedGenus,
        Some((genus + 1).max(1) as f64),
    )
}

pub fn verify_feedback_vertex_set(graph: &AssocGraph, members: &[CxId]) -> bool {
    let removed: BTreeSet<_> = members.iter().copied().collect();
    is_acyclic_after_removing(graph, &removed)
}

fn solve_with_method(
    graph: &AssocGraph,
    method: DfvsMethod,
    theoretical_bound: Option<f64>,
) -> Result<DfvsResult> {
    if graph.is_empty() {
        return Ok(empty_result(method));
    }
    let exact = if graph.node_count() <= EXACT_SEARCH_MAX_NODES {
        exact_min_fvs(graph)
    } else {
        None
    };
    let exact_search_used = exact.is_some();
    let mut members = exact.unwrap_or_else(|| greedy_fvs(graph));
    local_search_shrink(graph, &mut members);
    members.sort();

    if !verify_feedback_vertex_set(graph, &members) {
        return Err(LodestarError::DfvsVerificationFailed {
            detail: "removing computed members leaves a directed cycle".to_string(),
        });
    }
    let (tau_star_estimate, tau_star_exact, approx_factor) =
        approximation_report(graph, members.len(), exact_search_used, theoretical_bound);
    Ok(DfvsResult {
        members,
        approx_factor,
        tau_star_estimate,
        tau_star_exact,
        method,
    })
}

fn approximation_report(
    graph: &AssocGraph,
    member_count: usize,
    exact_search_used: bool,
    theoretical_bound: Option<f64>,
) -> (usize, bool, f64) {
    if member_count == 0 {
        return (0, true, 1.0);
    }
    if exact_search_used {
        return (member_count, true, 1.0);
    }

    let lower_bound = cyclic_scc_lower_bound(graph).max(1);
    let observed_bound = member_count as f64 / lower_bound as f64;
    let lower_bound_is_tight = member_count == lower_bound;
    let approx_factor = if lower_bound_is_tight {
        1.0
    } else {
        theoretical_bound.map_or(observed_bound, |bound| observed_bound.max(bound))
    };
    (lower_bound, lower_bound_is_tight, approx_factor)
}

fn cyclic_scc_lower_bound(graph: &AssocGraph) -> usize {
    let self_loop_nodes: BTreeSet<_> = graph
        .edges()
        .iter()
        .filter_map(|edge| {
            let (src, dst) = graph.edge_endpoints(*edge);
            (src == dst).then_some(src)
        })
        .collect();

    tarjan_scc(graph)
        .components
        .iter()
        .filter(|component| {
            component.len() > 1 || component.iter().any(|node| self_loop_nodes.contains(node))
        })
        .count()
}

fn exact_min_fvs(graph: &AssocGraph) -> Option<Vec<CxId>> {
    let ids: Vec<_> = graph.node_ids().collect();
    for size in 0..=ids.len() {
        let mut current = Vec::new();
        if let Some(solution) = choose_subset(graph, &ids, size, 0, &mut current) {
            return Some(solution);
        }
    }
    None
}

fn choose_subset(
    graph: &AssocGraph,
    ids: &[CxId],
    target: usize,
    start: usize,
    current: &mut Vec<CxId>,
) -> Option<Vec<CxId>> {
    if current.len() == target {
        let removed: BTreeSet<_> = current.iter().copied().collect();
        return is_acyclic_after_removing(graph, &removed).then(|| current.clone());
    }
    for index in start..ids.len() {
        current.push(ids[index]);
        if let Some(solution) = choose_subset(graph, ids, target, index + 1, current) {
            return Some(solution);
        }
        current.pop();
    }
    None
}

fn greedy_fvs(graph: &AssocGraph) -> Vec<CxId> {
    let mut removed = BTreeSet::new();
    while !is_acyclic_after_removing(graph, &removed) {
        let candidate = graph
            .node_ids()
            .filter(|id| !removed.contains(id))
            .max_by_key(|id| {
                graph.in_degree(*id).unwrap_or(0) + graph.out_degree(*id).unwrap_or(0)
            });
        if let Some(id) = candidate {
            removed.insert(id);
        } else {
            break;
        }
    }
    removed.into_iter().collect()
}

fn local_search_shrink(graph: &AssocGraph, members: &mut Vec<CxId>) {
    let mut index = 0;
    while index < members.len() {
        let mut trial: BTreeSet<_> = members.iter().copied().collect();
        trial.remove(&members[index]);
        if is_acyclic_after_removing(graph, &trial) {
            members.remove(index);
        } else {
            index += 1;
        }
    }
}

fn is_acyclic_after_removing(graph: &AssocGraph, removed: &BTreeSet<CxId>) -> bool {
    let mut builder = AssocGraph::builder();
    for node in graph.nodes() {
        if !removed.contains(&node.id) && builder.add_node(node.id, node.frequency_weight).is_err()
        {
            return false;
        }
    }
    for edge in graph.edges() {
        let (src, dst) = graph.edge_endpoints(*edge);
        if removed.contains(&src) || removed.contains(&dst) {
            continue;
        }
        if src == dst {
            return false;
        }
        if builder.add_edge(src, dst, edge.weight).is_err() {
            return false;
        }
    }
    let remaining = builder.build();
    tarjan_scc(&remaining)
        .components
        .iter()
        .all(|component| component.len() == 1)
}

fn empty_result(method: DfvsMethod) -> DfvsResult {
    DfvsResult {
        members: Vec::new(),
        approx_factor: 1.0,
        tau_star_estimate: 0,
        tau_star_exact: true,
        method,
    }
}
