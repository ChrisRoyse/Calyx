use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap, VecDeque};

use calyx_core::CxId;
use calyx_paths::AssocGraph;
use rayon::prelude::*;

use crate::{MincutError, Result};

const DIST_EPSILON: f64 = 1.0e-12;

/// Exact betweenness centrality (normalized to `[0,1]`).
///
/// Brandes' algorithm with a **binary-heap Dijkstra** per source — O(V·(E+V·log V)).
/// The previous dense `min_unvisited` scan was O(V²) per source → O(V³) overall,
/// intractable past a few thousand nodes; this is correct-identical but far cheaper.
/// For graphs too large even for this, use [`betweenness_sampled`] /
/// [`betweenness_auto`].
pub fn betweenness(graph: &AssocGraph) -> Result<BTreeMap<CxId, f64>> {
    if graph.is_empty() {
        return Err(MincutError::BetweennessEmptyGraph);
    }
    let n = graph.node_count();
    let sources: Vec<usize> = (0..n).collect();
    Ok(accumulate(graph, &sources, n, 1.0))
}

/// Sampled (pivot) betweenness estimator for large graphs.
///
/// Runs a full Brandes source-iteration (SSSP + dependency accumulation) from
/// `num_pivots` deterministically-chosen pivots and rescales scores by
/// `n / num_pivots`. Cost drops from O(V·(E+V·log V)) to O(k·(E+V·log V)). The
/// kernel only needs the *relative ranking* of nodes by centrality, which pivot
/// sampling preserves (Brandes & Pich 2007; Geisberger et al. 2008; Riondato &
/// Kornaropoulos 2014 give the formal ε/δ guarantees). Pivots are chosen by a
/// deterministic hash of each node id, so the estimate is reproducible (no RNG
/// state — required by the honesty/repro contract).
pub fn betweenness_sampled(graph: &AssocGraph, num_pivots: usize) -> Result<BTreeMap<CxId, f64>> {
    if graph.is_empty() {
        return Err(MincutError::BetweennessEmptyGraph);
    }
    if num_pivots == 0 {
        return Err(MincutError::lp_invalid(
            "betweenness num_pivots must be > 0",
        ));
    }
    let n = graph.node_count();
    if num_pivots >= n {
        return betweenness(graph);
    }
    let pivots = sample_pivots(graph, num_pivots);
    Ok(accumulate(graph, &pivots, n, n as f64 / num_pivots as f64))
}

/// Single policy point: exact betweenness when `node_count <= exact_max_nodes`,
/// else the sampled estimator with `num_pivots` pivots. `exact_max_nodes == 0`
/// forces sampling; `num_pivots == 0` (with sampling selected) is an error.
pub fn betweenness_auto(
    graph: &AssocGraph,
    exact_max_nodes: usize,
    num_pivots: usize,
) -> Result<BTreeMap<CxId, f64>> {
    if graph.node_count() <= exact_max_nodes {
        betweenness(graph)
    } else {
        betweenness_sampled(graph, num_pivots)
    }
}

pub fn betweenness_top_k(graph: &AssocGraph, k: usize) -> Result<Vec<(CxId, f64)>> {
    let scores = betweenness(graph)?;
    Ok(rank(scores, k))
}

/// Top-k by the sampled estimator (large-graph counterpart of [`betweenness_top_k`]).
pub fn betweenness_top_k_sampled(
    graph: &AssocGraph,
    k: usize,
    num_pivots: usize,
) -> Result<Vec<(CxId, f64)>> {
    let scores = betweenness_sampled(graph, num_pivots)?;
    Ok(rank(scores, k))
}

fn rank(scores: BTreeMap<CxId, f64>, k: usize) -> Vec<(CxId, f64)> {
    let mut ranked: Vec<_> = scores.into_iter().collect();
    ranked.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.as_bytes().cmp(right.0.as_bytes()))
    });
    ranked.truncate(k.min(ranked.len()));
    ranked
}

fn accumulate(graph: &AssocGraph, sources: &[usize], n: usize, scale: f64) -> BTreeMap<CxId, f64> {
    let mut scores = vec![0.0_f64; n];
    let chunk_size = source_chunk_size(sources.len());
    let unit_weight = graph_is_unit_weight(graph);
    let mut partials: Vec<(usize, Vec<f64>)> = sources
        .par_chunks(chunk_size)
        .enumerate()
        .map(|(chunk_idx, chunk)| {
            let mut partial = vec![0.0_f64; n];
            for &source in chunk {
                let shortest = if unit_weight {
                    shortest_paths_unit_from(graph, source)
                } else {
                    shortest_paths_from(graph, source)
                };
                accumulate_dependencies(source, &shortest, &mut partial);
            }
            (chunk_idx, partial)
        })
        .collect();
    partials.sort_by_key(|(chunk_idx, _)| *chunk_idx);
    for (_, partial) in partials {
        for (score, contribution) in scores.iter_mut().zip(partial) {
            *score += contribution;
        }
    }
    let norm = if n > 2 {
        ((n - 1) * (n - 2)) as f64
    } else {
        1.0
    };
    (0..n)
        .map(|index| {
            (
                graph.node_id(index).expect("betweenness node id"),
                scores[index] * scale / norm,
            )
        })
        .collect()
}

fn source_chunk_size(source_count: usize) -> usize {
    let threads = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .max(1);
    let target_chunks = (threads * 4).max(1);
    source_count.div_ceil(target_chunks).max(1)
}

fn graph_is_unit_weight(graph: &AssocGraph) -> bool {
    graph
        .edges()
        .iter()
        .all(|edge| approx_eq(edge.weight as f64, 1.0))
}

fn shortest_paths_unit_from(graph: &AssocGraph, source: usize) -> ShortestPaths {
    let n = graph.node_count();
    let mut dist = vec![usize::MAX; n];
    let mut sigma = vec![0.0_f64; n];
    let mut predecessors = vec![Vec::<usize>::new(); n];
    let mut stack = Vec::with_capacity(n);
    let mut queue = VecDeque::new();
    dist[source] = 0;
    sigma[source] = 1.0;
    queue.push_back(source);

    while let Some(node) = queue.pop_front() {
        stack.push(node);
        let next_dist = dist[node] + 1;
        for edge in graph.out_edges_by_index(node) {
            let dst = edge.dst;
            if dist[dst] == usize::MAX {
                dist[dst] = next_dist;
                queue.push_back(dst);
            }
            if dist[dst] == next_dist {
                sigma[dst] += sigma[node];
                predecessors[dst].push(node);
            }
        }
    }

    ShortestPaths {
        stack,
        sigma,
        predecessors,
    }
}

fn shortest_paths_from(graph: &AssocGraph, source: usize) -> ShortestPaths {
    let n = graph.node_count();
    let mut dist = vec![f64::INFINITY; n];
    let mut sigma = vec![0.0_f64; n];
    let mut predecessors = vec![Vec::<usize>::new(); n];
    let mut visited = vec![false; n];
    let mut stack = Vec::with_capacity(n);
    let mut heap = BinaryHeap::new();
    dist[source] = 0.0;
    sigma[source] = 1.0;
    heap.push(HeapEntry {
        dist: 0.0,
        node: source,
    });

    while let Some(HeapEntry { node, .. }) = heap.pop() {
        if visited[node] {
            continue;
        }
        visited[node] = true;
        stack.push(node);
        for edge in graph.out_edges_by_index(node) {
            if edge.weight <= 0.0 {
                continue;
            }
            let dst = edge.dst;
            let candidate = dist[node] + 1.0 / edge.weight as f64;
            if candidate + DIST_EPSILON < dist[dst] {
                dist[dst] = candidate;
                sigma[dst] = sigma[node];
                predecessors[dst].clear();
                predecessors[dst].push(node);
                heap.push(HeapEntry {
                    dist: candidate,
                    node: dst,
                });
            } else if approx_eq(candidate, dist[dst]) {
                sigma[dst] += sigma[node];
                predecessors[dst].push(node);
            }
        }
    }

    ShortestPaths {
        stack,
        sigma,
        predecessors,
    }
}

fn accumulate_dependencies(source: usize, paths: &ShortestPaths, scores: &mut [f64]) {
    let mut delta = vec![0.0_f64; scores.len()];
    for &node in paths.stack.iter().rev() {
        for &pred in &paths.predecessors[node] {
            if paths.sigma[node] > 0.0 {
                delta[pred] += (paths.sigma[pred] / paths.sigma[node]) * (1.0 + delta[node]);
            }
        }
        if node != source {
            scores[node] += delta[node];
        }
    }
}

/// Deterministic pivot selection: rank nodes by a stable hash of their id and
/// take the lowest `k`. Reproducible across runs (no RNG), well-spread because
/// the hash decorrelates from id order.
fn sample_pivots(graph: &AssocGraph, k: usize) -> Vec<usize> {
    let mut ranked: Vec<(u64, usize)> = (0..graph.node_count())
        .map(|index| {
            let id = graph.node_id(index).expect("pivot node id");
            (fnv1a64(id.as_bytes()), index)
        })
        .collect();
    ranked.sort_unstable();
    ranked.into_iter().take(k).map(|(_, index)| index).collect()
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn approx_eq(left: f64, right: f64) -> bool {
    (left - right).abs() <= DIST_EPSILON
}

#[derive(Clone, Debug)]
struct ShortestPaths {
    stack: Vec<usize>,
    sigma: Vec<f64>,
    predecessors: Vec<Vec<usize>>,
}

/// Min-heap entry: ordered by `dist` ascending (so `BinaryHeap`'s max-pop yields
/// the smallest distance), ties broken by node index for determinism.
#[derive(Clone, Copy, Debug)]
struct HeapEntry {
    dist: f64,
    node: usize,
}

impl PartialEq for HeapEntry {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for HeapEntry {}

impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .dist
            .total_cmp(&self.dist)
            .then_with(|| self.node.cmp(&other.node))
    }
}

impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cx(seed: u8) -> CxId {
        CxId::from_bytes([seed; 16])
    }

    /// Path a->b->c->d: b and c are bridges, a and d are not.
    fn line() -> AssocGraph {
        let mut builder = AssocGraph::builder();
        for seed in [1, 2, 3, 4] {
            builder.add_node(cx(seed), 1.0).unwrap();
        }
        builder.add_edge(cx(1), cx(2), 1.0).unwrap();
        builder.add_edge(cx(2), cx(3), 1.0).unwrap();
        builder.add_edge(cx(3), cx(4), 1.0).unwrap();
        builder.build()
    }

    #[test]
    fn heap_betweenness_ranks_bridges_above_leaves() {
        let scores = betweenness(&line()).unwrap();
        // Interior nodes lie on more shortest paths than the endpoints.
        assert!(scores[&cx(2)] > scores[&cx(1)]);
        assert!(scores[&cx(3)] > scores[&cx(4)]);
        assert!(scores[&cx(2)] > 0.0 && scores[&cx(3)] > 0.0);
    }

    #[test]
    fn sampled_with_all_pivots_equals_exact() {
        let graph = line();
        let exact = betweenness(&graph).unwrap();
        // num_pivots >= n falls back to exact.
        let sampled = betweenness_sampled(&graph, 4).unwrap();
        for (id, value) in &exact {
            assert!((value - sampled[id]).abs() < 1e-9);
        }
    }

    #[test]
    fn sampled_preserves_top_ranking_on_a_star() {
        // Star: hub (1) is on every shortest path between the 8 leaves; leaves
        // are on none. Even a single pivot must rank the hub strictly highest.
        let mut builder = AssocGraph::builder();
        for seed in 1..=9u8 {
            builder.add_node(cx(seed), 1.0).unwrap();
        }
        for leaf in 2..=9u8 {
            builder.add_edge(cx(1), cx(leaf), 1.0).unwrap();
            builder.add_edge(cx(leaf), cx(1), 1.0).unwrap();
        }
        let graph = builder.build();
        let sampled = betweenness_sampled(&graph, 3).unwrap();
        let hub = sampled[&cx(1)];
        assert!(hub > 0.0);
        for leaf in 2..=9u8 {
            assert!(hub > sampled[&cx(leaf)]);
        }
    }

    #[test]
    fn pivot_selection_is_deterministic() {
        let graph = line();
        assert_eq!(sample_pivots(&graph, 2), sample_pivots(&graph, 2));
    }

    #[test]
    fn auto_switches_on_node_count() {
        let graph = line();
        // exact_max_nodes below n forces the sampled path; result is finite + ranked.
        let scores = betweenness_auto(&graph, 2, 2).unwrap();
        assert!(scores[&cx(2)] >= 0.0);
        // large threshold takes the exact path.
        let exact = betweenness_auto(&graph, 100, 2).unwrap();
        assert!((exact[&cx(2)] - betweenness(&graph).unwrap()[&cx(2)]).abs() < 1e-9);
    }
}
