use std::collections::{BTreeMap, HashMap, VecDeque};

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

    let Some(path) = shortest_path_indices(graph, src_idx, dst_idx) else {
        return Ok(None);
    };
    let hops = path.len().saturating_sub(1);
    if hops > max_hops {
        return Err(PathsError::MaxHops {
            required: hops,
            max_hops,
        });
    }
    Ok(Some(path_to_ids(graph, &path)))
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
    let mut best = BTreeMap::<usize, ScoredReach>::new();
    let mut queue = VecDeque::from([ScoredReach {
        node: src_idx,
        hops: 0,
        raw_score: 1.0,
    }]);
    best.insert(
        src_idx,
        ScoredReach {
            node: src_idx,
            hops: 0,
            raw_score: 1.0,
        },
    );

    while let Some(current) = queue.pop_front() {
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
            let should_update = best
                .get(&edge.dst)
                .is_none_or(|known| next.ranked_score() > known.ranked_score());
            if should_update {
                best.insert(edge.dst, next);
                queue.push_back(next);
            }
        }
    }

    Ok(best
        .into_values()
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

fn shortest_path_indices(graph: &AssocGraph, src: usize, dst: usize) -> Option<Vec<usize>> {
    let mut forward = Frontier::new(src);
    let mut backward = Frontier::new(dst);

    while !forward.frontier.is_empty() && !backward.frontier.is_empty() {
        if forward.frontier.len() <= backward.frontier.len() {
            if let Some(meet) = expand_forward(graph, &mut forward, &backward.parents) {
                return Some(reconstruct(
                    src,
                    dst,
                    meet,
                    &forward.parents,
                    &backward.parents,
                ));
            }
        } else if let Some(meet) = expand_backward(graph, &mut backward, &forward.parents) {
            return Some(reconstruct(
                src,
                dst,
                meet,
                &forward.parents,
                &backward.parents,
            ));
        }
    }
    None
}

fn expand_forward(
    graph: &AssocGraph,
    state: &mut Frontier,
    other: &HashMap<usize, Option<usize>>,
) -> Option<usize> {
    let mut next = VecDeque::new();
    while let Some(node) = state.frontier.pop_front() {
        for edge in graph.out_edges_by_index(node) {
            if state.parents.contains_key(&edge.dst) {
                continue;
            }
            state.parents.insert(edge.dst, Some(node));
            if other.contains_key(&edge.dst) {
                return Some(edge.dst);
            }
            next.push_back(edge.dst);
        }
    }
    state.frontier = next;
    None
}

fn expand_backward(
    graph: &AssocGraph,
    state: &mut Frontier,
    other: &HashMap<usize, Option<usize>>,
) -> Option<usize> {
    let mut next = VecDeque::new();
    while let Some(node) = state.frontier.pop_front() {
        for edge in graph.incoming_edges_by_index(node) {
            if state.parents.contains_key(&edge.src) {
                continue;
            }
            state.parents.insert(edge.src, Some(node));
            if other.contains_key(&edge.src) {
                return Some(edge.src);
            }
            next.push_back(edge.src);
        }
    }
    state.frontier = next;
    None
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
}

#[derive(Clone, Debug)]
struct Frontier {
    frontier: VecDeque<usize>,
    parents: HashMap<usize, Option<usize>>,
}

impl Frontier {
    fn new(root: usize) -> Self {
        Self {
            frontier: VecDeque::from([root]),
            parents: HashMap::from([(root, None)]),
        }
    }
}
