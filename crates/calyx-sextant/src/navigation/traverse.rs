//! Asymmetric hop-attenuated traversal (PRD 10 §4, 18 §4).
//!
//! Walks the vault association graph from an anchor constellation. Forward
//! follows edge direction ("what did X cause / lead to"), Backward follows
//! reversed edges ("what caused / led to X"), Both reports each direction
//! separately — the asymmetry is the point, the two walks legitimately
//! return different result sets. Scores are the best path-weight product
//! attenuated by `0.9^hop` (calyx-paths `attenuate`).

use std::collections::{BTreeMap, VecDeque};

use calyx_core::{CxId, Result};
use calyx_paths::{AssocGraph, attenuate};
use serde::{Deserialize, Serialize};

use crate::error::{
    CALYX_SEXTANT_ASSOC_GRAPH_MISSING, CALYX_SEXTANT_CX_MISSING, CALYX_SEXTANT_TRAVERSE_HOPS,
    sextant_error,
};
use crate::search::SearchEngine;

/// Upper bound on traversal depth (MCP/CLI contract: hops in 1..=10).
pub const MAX_TRAVERSE_HOPS: u32 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraverseDirection {
    Forward,
    Backward,
    Both,
}

/// One reached constellation, with the hop count and best attenuated score.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraverseStep {
    pub cx_id: CxId,
    pub hop: u32,
    /// The direction this step was reached in (never `Both`).
    pub direction: TraverseDirection,
    /// Best path-weight product to this node, attenuated by `0.9^hop`.
    pub score: f32,
    /// Predecessor on the best path (provenance for explain).
    pub via: CxId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraversePath {
    pub anchor: CxId,
    pub direction: TraverseDirection,
    pub max_hops: u32,
    pub steps: Vec<TraverseStep>,
}

/// Walks the engine's association graph from `anchor`.
pub fn traverse(
    engine: &SearchEngine,
    anchor: CxId,
    direction: TraverseDirection,
    max_hops: u32,
) -> Result<TraversePath> {
    let graph = engine.assoc_graph().ok_or_else(|| {
        sextant_error(
            CALYX_SEXTANT_ASSOC_GRAPH_MISSING,
            "no association graph is set on this engine",
        )
    })?;
    traverse_graph(graph, anchor, direction, max_hops)
}

/// Walks an explicit association graph from `anchor`.
pub fn traverse_graph(
    graph: &AssocGraph,
    anchor: CxId,
    direction: TraverseDirection,
    max_hops: u32,
) -> Result<TraversePath> {
    if !(1..=MAX_TRAVERSE_HOPS).contains(&max_hops) {
        return Err(sextant_error(
            CALYX_SEXTANT_TRAVERSE_HOPS,
            format!("max_hops {max_hops} outside 1..={MAX_TRAVERSE_HOPS}"),
        ));
    }
    let anchor_idx = graph.node_index(anchor).ok_or_else(|| {
        sextant_error(
            CALYX_SEXTANT_CX_MISSING,
            format!("anchor {anchor} is not a node in the association graph"),
        )
    })?;
    let mut steps = Vec::new();
    if matches!(
        direction,
        TraverseDirection::Forward | TraverseDirection::Both
    ) {
        steps.extend(scored_walk(
            graph,
            anchor_idx,
            max_hops,
            TraverseDirection::Forward,
        ));
    }
    if matches!(
        direction,
        TraverseDirection::Backward | TraverseDirection::Both
    ) {
        steps.extend(scored_walk(
            graph,
            anchor_idx,
            max_hops,
            TraverseDirection::Backward,
        ));
    }
    steps.sort_by(|a, b| {
        a.hop
            .cmp(&b.hop)
            .then_with(|| b.score.total_cmp(&a.score))
            .then_with(|| a.cx_id.cmp(&b.cx_id))
            .then_with(|| direction_order(a.direction).cmp(&direction_order(b.direction)))
    });
    Ok(TraversePath {
        anchor,
        direction,
        max_hops,
        steps,
    })
}

fn direction_order(direction: TraverseDirection) -> u8 {
    match direction {
        TraverseDirection::Forward => 0,
        TraverseDirection::Backward => 1,
        TraverseDirection::Both => 2,
    }
}

#[derive(Clone, Copy)]
struct WalkState {
    hop: u32,
    raw_score: f32,
    via: usize,
}

fn scored_walk(
    graph: &AssocGraph,
    src: usize,
    max_hops: u32,
    direction: TraverseDirection,
) -> Vec<TraverseStep> {
    let reverse_adj = match direction {
        TraverseDirection::Backward => Some(reverse_adjacency(graph)),
        _ => None,
    };
    let mut best = BTreeMap::<usize, WalkState>::new();
    let mut queue = VecDeque::from([(
        src,
        WalkState {
            hop: 0,
            raw_score: 1.0,
            via: src,
        },
    )]);
    while let Some((node, state)) = queue.pop_front() {
        if state.hop == max_hops {
            continue;
        }
        let neighbors: Vec<(usize, f32)> = match &reverse_adj {
            Some(adj) => adj.get(&node).cloned().unwrap_or_default(),
            None => graph
                .out_edges_by_index(node)
                .iter()
                .map(|edge| (edge.dst, edge.weight))
                .collect(),
        };
        for (next, weight) in neighbors {
            if next == src {
                continue;
            }
            let candidate = WalkState {
                hop: state.hop + 1,
                raw_score: state.raw_score * weight,
                via: node,
            };
            let ranked = attenuate(candidate.raw_score, candidate.hop);
            let improves = best
                .get(&next)
                .is_none_or(|known| ranked > attenuate(known.raw_score, known.hop));
            if improves {
                best.insert(next, candidate);
                queue.push_back((next, candidate));
            }
        }
    }
    best.into_iter()
        .map(|(node, state)| TraverseStep {
            cx_id: graph.node_id(node).expect("walked node id"),
            hop: state.hop,
            direction,
            score: attenuate(state.raw_score, state.hop),
            via: graph.node_id(state.via).expect("walk predecessor id"),
        })
        .collect()
}

/// Builds the reversed adjacency once per backward walk (deterministic order).
fn reverse_adjacency(graph: &AssocGraph) -> BTreeMap<usize, Vec<(usize, f32)>> {
    let mut adj = BTreeMap::<usize, Vec<(usize, f32)>>::new();
    for edge in graph.edges() {
        adj.entry(edge.dst)
            .or_default()
            .push((edge.src, edge.weight));
    }
    for targets in adj.values_mut() {
        targets.sort_by_key(|entry| entry.0);
    }
    adj
}
