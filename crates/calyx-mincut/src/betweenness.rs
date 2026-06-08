use std::cmp::Ordering;
use std::collections::BTreeMap;

use calyx_core::CxId;
use calyx_paths::AssocGraph;

use crate::{MincutError, Result};

const DIST_EPSILON: f64 = 1.0e-12;

pub fn betweenness(graph: &AssocGraph) -> Result<BTreeMap<CxId, f64>> {
    if graph.is_empty() {
        return Err(MincutError::BetweennessEmptyGraph);
    }
    let n = graph.node_count();
    let mut scores = vec![0.0_f64; n];

    for source in 0..n {
        let shortest = shortest_paths_from(graph, source);
        accumulate_dependencies(source, &shortest, &mut scores);
    }

    let norm = if n > 2 {
        ((n - 1) * (n - 2)) as f64
    } else {
        1.0
    };
    Ok((0..n)
        .map(|index| {
            (
                graph.node_id(index).expect("betweenness node id"),
                scores[index] / norm,
            )
        })
        .collect())
}

pub fn betweenness_top_k(graph: &AssocGraph, k: usize) -> Result<Vec<(CxId, f64)>> {
    let scores = betweenness(graph)?;
    let mut ranked: Vec<_> = scores.into_iter().collect();
    ranked.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.as_bytes().cmp(right.0.as_bytes()))
    });
    ranked.truncate(k.min(ranked.len()));
    Ok(ranked)
}

fn shortest_paths_from(graph: &AssocGraph, source: usize) -> ShortestPaths {
    let n = graph.node_count();
    let mut visited = vec![false; n];
    let mut dist = vec![f64::INFINITY; n];
    let mut sigma = vec![0.0_f64; n];
    let mut predecessors = vec![Vec::<usize>::new(); n];
    let mut stack = Vec::with_capacity(n);
    dist[source] = 0.0;
    sigma[source] = 1.0;

    while let Some(node) = min_unvisited(&dist, &visited) {
        visited[node] = true;
        stack.push(node);
        for edge in graph.out_edges_by_index(node) {
            if edge.weight <= 0.0 {
                continue;
            }
            let candidate = dist[node] + 1.0 / edge.weight as f64;
            match candidate.total_cmp(&dist[edge.dst]) {
                Ordering::Less if !approx_eq(candidate, dist[edge.dst]) => {
                    dist[edge.dst] = candidate;
                    sigma[edge.dst] = sigma[node];
                    predecessors[edge.dst].clear();
                    predecessors[edge.dst].push(node);
                }
                _ if approx_eq(candidate, dist[edge.dst]) => {
                    sigma[edge.dst] += sigma[node];
                    predecessors[edge.dst].push(node);
                }
                _ => {}
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

fn min_unvisited(dist: &[f64], visited: &[bool]) -> Option<usize> {
    dist.iter()
        .enumerate()
        .filter(|(index, value)| !visited[*index] && value.is_finite())
        .min_by(|left, right| left.1.total_cmp(right.1).then_with(|| left.0.cmp(&right.0)))
        .map(|(index, _)| index)
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
