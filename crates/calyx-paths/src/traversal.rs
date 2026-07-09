use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, VecDeque};

use calyx_core::CxId;

use crate::{AssocGraph, PathsError, Result, attenuate};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BidirectionalPath {
    pub forward: Option<Vec<CxId>>,
    pub reverse: Option<Vec<CxId>>,
}

pub fn reach(
    graph: &AssocGraph,
    src: CxId,
    dst: CxId,
    max_hops: usize,
) -> Result<Option<Vec<CxId>>> {
    if graph.is_empty() {
        return Err(PathsError::NodeNotFound { id: src });
    }
    let src_idx = require_present(graph, src)?;
    let dst_idx = require_present(graph, dst)?;
    if src_idx == dst_idx {
        return Ok(Some(vec![src]));
    }

    match shortest_path_indices(graph, src_idx, dst_idx, max_hops) {
        PathSearch::Found(path) => Ok(Some(path_to_ids(graph, &path))),
        PathSearch::Exhausted => Ok(None),
        PathSearch::BeyondMax { required } => Err(PathsError::MaxHops { required, max_hops }),
    }
}

pub fn bidirectional(
    graph: &AssocGraph,
    question: CxId,
    answer: CxId,
    max_hops: usize,
) -> Result<BidirectionalPath> {
    Ok(BidirectionalPath {
        forward: reach(graph, question, answer, max_hops)?,
        reverse: reach(graph, answer, question, max_hops)?,
    })
}

pub fn reach_scored(graph: &AssocGraph, src: CxId, max_hops: usize) -> Result<Vec<(CxId, f32)>> {
    if graph.is_empty() {
        return Err(PathsError::NodeNotFound { id: src });
    }
    let src_idx = require_present(graph, src)?;
    let start = ScoredReach {
        node: src_idx,
        hops: 0,
        raw_score: 1.0,
    };
    let mut best_by_node = vec![None; graph.node_count()];
    let mut best_by_state = vec![vec![None; max_hops.saturating_add(1)]; graph.node_count()];
    let mut queue = BinaryHeap::from([start]);
    best_by_node[src_idx] = Some(start);
    best_by_state[src_idx][0] = Some(start);

    while let Some(current) = queue.pop() {
        let Some(known) = best_by_state[current.node][current.hops] else {
            continue;
        };
        if known.is_better_than(current) {
            continue;
        }
        if current.hops == max_hops {
            continue;
        }
        for edge in graph.out_edges_by_index(current.node) {
            let hops = current.hops + 1;
            let raw_score = current.raw_score * edge.weight;
            let next = ScoredReach {
                node: edge.dst,
                hops,
                raw_score,
            };
            if best_by_node[edge.dst].is_none_or(|known| next.is_better_than(known)) {
                best_by_node[edge.dst] = Some(next);
            }
            if best_by_state[edge.dst][hops].is_none_or(|known| next.is_better_than(known)) {
                best_by_state[edge.dst][hops] = Some(next);
                if hops < max_hops {
                    queue.push(next);
                }
            }
        }
    }

    Ok(best_by_node
        .into_iter()
        .flatten()
        .filter(|entry| entry.node != src_idx)
        .map(|entry| {
            (
                graph.node_id(entry.node).expect("reachable node id"),
                attenuate(entry.raw_score, entry.hops as u32),
            )
        })
        .collect())
}

fn require_present(graph: &AssocGraph, id: CxId) -> Result<usize> {
    graph.node_index(id).ok_or(PathsError::NodeNotFound { id })
}

fn shortest_path_indices(
    graph: &AssocGraph,
    src: usize,
    dst: usize,
    max_hops: usize,
) -> PathSearch {
    let mut stats = SearchStats::default();
    shortest_path_indices_with_stats(graph, src, dst, max_hops, &mut stats)
}

fn shortest_path_indices_with_stats(
    graph: &AssocGraph,
    src: usize,
    dst: usize,
    max_hops: usize,
    stats: &mut SearchStats,
) -> PathSearch {
    let mut forward = Frontier::new(src);
    let mut backward = Frontier::new(dst);

    loop {
        if forward.frontier.is_empty() || backward.frontier.is_empty() {
            return PathSearch::Exhausted;
        }
        if forward.depth + backward.depth >= max_hops {
            return PathSearch::BeyondMax {
                required: max_hops.saturating_add(1),
            };
        }

        if forward.frontier.len() <= backward.frontier.len() {
            let expansion = expand_forward(graph, &mut forward, &backward.parents);
            stats.forward_levels += 1;
            stats.forward_nodes += expansion.expanded_nodes;
            if let Some(meet) = expansion.meet {
                return PathSearch::Found(reconstruct(
                    src,
                    dst,
                    meet,
                    &forward.parents,
                    &backward.parents,
                ));
            }
        } else {
            let expansion = expand_backward(graph, &mut backward, &forward.parents);
            stats.backward_levels += 1;
            stats.backward_nodes += expansion.expanded_nodes;
            if let Some(meet) = expansion.meet {
                return PathSearch::Found(reconstruct(
                    src,
                    dst,
                    meet,
                    &forward.parents,
                    &backward.parents,
                ));
            }
        }
    }
}

fn expand_forward(
    graph: &AssocGraph,
    state: &mut Frontier,
    other: &HashMap<usize, Option<usize>>,
) -> LevelExpansion {
    let mut next = VecDeque::new();
    let mut expanded_nodes = 0;
    while let Some(node) = state.frontier.pop_front() {
        expanded_nodes += 1;
        for edge in graph.out_edges_by_index(node) {
            if state.parents.contains_key(&edge.dst) {
                continue;
            }
            state.parents.insert(edge.dst, Some(node));
            if other.contains_key(&edge.dst) {
                state.frontier = next;
                state.depth += 1;
                return LevelExpansion {
                    meet: Some(edge.dst),
                    expanded_nodes,
                };
            }
            next.push_back(edge.dst);
        }
    }
    state.frontier = next;
    state.depth += 1;
    LevelExpansion {
        meet: None,
        expanded_nodes,
    }
}

fn expand_backward(
    graph: &AssocGraph,
    state: &mut Frontier,
    other: &HashMap<usize, Option<usize>>,
) -> LevelExpansion {
    let mut next = VecDeque::new();
    let mut expanded_nodes = 0;
    while let Some(node) = state.frontier.pop_front() {
        expanded_nodes += 1;
        for edge in graph.incoming_edges_by_index(node) {
            if state.parents.contains_key(&edge.src) {
                continue;
            }
            state.parents.insert(edge.src, Some(node));
            if other.contains_key(&edge.src) {
                state.frontier = next;
                state.depth += 1;
                return LevelExpansion {
                    meet: Some(edge.src),
                    expanded_nodes,
                };
            }
            next.push_back(edge.src);
        }
    }
    state.frontier = next;
    state.depth += 1;
    LevelExpansion {
        meet: None,
        expanded_nodes,
    }
}

fn reconstruct(
    src: usize,
    dst: usize,
    meet: usize,
    forward: &HashMap<usize, Option<usize>>,
    backward: &HashMap<usize, Option<usize>>,
) -> Vec<usize> {
    let mut left = Vec::new();
    let mut cursor = meet;
    left.push(cursor);
    while cursor != src {
        cursor = forward[&cursor].expect("forward parent");
        left.push(cursor);
    }
    left.reverse();

    cursor = meet;
    while cursor != dst {
        cursor = backward[&cursor].expect("backward parent");
        left.push(cursor);
    }
    left
}

fn path_to_ids(graph: &AssocGraph, path: &[usize]) -> Vec<CxId> {
    path.iter()
        .map(|index| graph.node_id(*index).expect("path node id"))
        .collect()
}

#[derive(Clone, Copy, Debug)]
struct ScoredReach {
    node: usize,
    hops: usize,
    raw_score: f32,
}

impl ScoredReach {
    fn ranked_score(self) -> f32 {
        attenuate(self.raw_score, self.hops as u32)
    }

    fn is_better_than(self, known: Self) -> bool {
        match self.ranked_score().total_cmp(&known.ranked_score()) {
            Ordering::Greater => true,
            Ordering::Equal => self.hops < known.hops,
            Ordering::Less => false,
        }
    }
}

impl PartialEq for ScoredReach {
    fn eq(&self, other: &Self) -> bool {
        self.node == other.node
            && self.hops == other.hops
            && self.ranked_score().total_cmp(&other.ranked_score()).is_eq()
    }
}

impl Eq for ScoredReach {}

impl PartialOrd for ScoredReach {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScoredReach {
    fn cmp(&self, other: &Self) -> Ordering {
        self.ranked_score()
            .total_cmp(&other.ranked_score())
            .then_with(|| other.hops.cmp(&self.hops))
            .then_with(|| other.node.cmp(&self.node))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PathSearch {
    Found(Vec<usize>),
    Exhausted,
    BeyondMax { required: usize },
}

#[derive(Clone, Debug, Default)]
struct SearchStats {
    forward_levels: usize,
    backward_levels: usize,
    forward_nodes: usize,
    backward_nodes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LevelExpansion {
    meet: Option<usize>,
    expanded_nodes: usize,
}

#[derive(Clone, Debug)]
struct Frontier {
    frontier: VecDeque<usize>,
    parents: HashMap<usize, Option<usize>>,
    depth: usize,
}

impl Frontier {
    fn new(root: usize) -> Self {
        Self {
            frontier: VecDeque::from([root]),
            parents: HashMap::from([(root, None)]),
            depth: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use calyx_core::CxId;

    use super::*;
    use crate::AssocGraph;

    fn cx(seed: u8) -> CxId {
        CxId::from_bytes([seed; 16])
    }

    fn linear_graph(len: u8) -> AssocGraph {
        let mut builder = AssocGraph::builder();
        for seed in 1..=len {
            builder.add_node(cx(seed), 1.0).expect("add node");
        }
        for seed in 1..len {
            builder
                .add_edge(cx(seed), cx(seed + 1), 1.0)
                .expect("add edge");
        }
        builder.build()
    }

    #[test]
    fn max_hops_caps_bidirectional_expansion_depth() {
        let graph = linear_graph(32);
        let src = graph.node_index(cx(1)).expect("src index");
        let dst = graph.node_index(cx(32)).expect("dst index");
        let mut stats = SearchStats::default();

        let result = shortest_path_indices_with_stats(&graph, src, dst, 1, &mut stats);

        assert_eq!(result, PathSearch::BeyondMax { required: 2 });
        assert_eq!(stats.forward_levels + stats.backward_levels, 1);
        assert_eq!(stats.forward_nodes + stats.backward_nodes, 1);
    }
}
