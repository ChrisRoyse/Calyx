//! Vamana graph construction for the DiskANN on-disk format (PH68 T01/T02).
//!
//! Two-pass build per the DiskANN paper: seeded random init edges, then for
//! each point greedy-search from the medoid and RobustPrune — alpha=1.0 on the
//! first pass, `params.alpha` on the second — with backward edges re-pruned on
//! overflow.
//!
//! Construction geometry runs on L2-normalized copies so the distance kernel
//! is a bare dot product (cosine is scale-invariant, so neighbor topology is
//! identical to the search-time `1 - cosine`); the graph file still stores the
//! original vectors verbatim. Each pass advances in batches: every point in a
//! batch greedy-searches the *same frozen snapshot* of the graph in parallel
//! (read-only), then edge updates apply sequentially in batch order — so the
//! build is both parallel and fully deterministic regardless of thread count.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use calyx_core::Result;
use rand::SeedableRng;
use rand::seq::SliceRandom;
use rand_chacha::ChaCha8Rng;
use rayon::prelude::*;

use super::graph::{
    DISKANN_FORMAT_VERSION, DISKANN_MAX_DIM, DISKANN_MAX_M, DiskAnnGraphWriter, DiskAnnHeader,
    invalid,
};

/// Deterministic build seed (Vamana insert order + random init edges).
const BUILD_SEED: u64 = 42;
/// First synchronization round size. Batches grow geometrically from here
/// (ParlayANN prefix-doubling): early points refine the graph at near-
/// sequential quality, later points parallelize over the larger snapshot.
const BUILD_BATCH_MIN: usize = 256;
/// Batches never exceed `n / BUILD_BATCH_DIVISOR` so that no single
/// synchronization round connects more than a small fraction of the graph
/// against one stale snapshot — keeping graph quality scale-independent.
const BUILD_BATCH_DIVISOR: usize = 32;

/// Vamana build parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiskAnnBuildParams {
    pub dim: usize,
    pub m_max: usize,
    pub ef_construction: usize,
    pub alpha: f32,
}

impl DiskAnnBuildParams {
    fn validate(&self) -> Result<()> {
        if self.dim == 0 || self.dim > DISKANN_MAX_DIM {
            return Err(invalid(format!(
                "dim {} out of 1..={DISKANN_MAX_DIM}",
                self.dim
            )));
        }
        if self.m_max == 0 || self.m_max > DISKANN_MAX_M {
            return Err(invalid(format!(
                "m_max {} out of 1..={DISKANN_MAX_M}",
                self.m_max
            )));
        }
        if self.ef_construction == 0 {
            return Err(invalid("ef_construction must be >= 1"));
        }
        if !self.alpha.is_finite() || self.alpha < 1.0 || self.alpha > 4.0 {
            return Err(invalid(format!("alpha {} out of 1.0..=4.0", self.alpha)));
        }
        Ok(())
    }
}

/// Build a Vamana graph from `(id, vector)` rows (ids must be dense `0..n`)
/// and publish it atomically at `path` (the `graph.cda` file).
pub fn build_diskann_graph(
    path: &Path,
    vectors: &[(u32, Vec<f32>)],
    params: DiskAnnBuildParams,
) -> Result<()> {
    params.validate()?;
    if vectors.is_empty() {
        return Err(invalid("empty input: at least one vector is required"));
    }
    let n = vectors.len();
    if u32::try_from(n).is_err() {
        return Err(invalid(format!("{n} vectors exceed u32 id space")));
    }
    for (at, (id, vector)) in vectors.iter().enumerate() {
        if *id as usize != at {
            return Err(invalid(format!(
                "ids must be dense 0..n; slot {at} holds id {id}"
            )));
        }
        if vector.len() != params.dim {
            return Err(invalid(format!(
                "vector {id} len {} != dim {}",
                vector.len(),
                params.dim
            )));
        }
        if vector.iter().any(|v| !v.is_finite()) {
            return Err(invalid(format!("vector {id} has non-finite component")));
        }
    }
    let (entry, adjacency) = vamana(vectors, &params);
    let max_degree = adjacency.iter().map(Vec::len).max().unwrap_or(0);
    let header = DiskAnnHeader {
        format_version: DISKANN_FORMAT_VERSION,
        dim: u32::try_from(params.dim).expect("dim <= 8192"),
        m_max: u32::try_from(params.m_max).expect("m_max <= 512"),
        max_degree: u32::try_from(max_degree).expect("<= m_max"),
        entry_point_id: entry,
        node_count: n as u64,
    };
    let mut writer = DiskAnnGraphWriter::create(path, header)?;
    for (id, vector) in vectors {
        writer.write_node(*id, vector, &adjacency[*id as usize])?;
    }
    writer.finish()
}

/// L2-normalize every vector; a zero vector stays all-zero (dot == 0 with
/// anything, i.e. distance 1 — matching cosine's zero-vector convention).
/// `norm[id]` lines up with the dense id space validated above.
fn normalize(vectors: &[(u32, Vec<f32>)]) -> Vec<Vec<f32>> {
    vectors
        .par_iter()
        .map(|(_, v)| {
            let mag = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            if mag == 0.0 {
                v.clone()
            } else {
                v.iter().map(|x| x / mag).collect()
            }
        })
        .collect()
}

/// Distance between two unit vectors: `1 - dot` (equals `1 - cosine`).
fn dist(a: &[f32], b: &[f32]) -> f32 {
    1.0 - a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>()
}

/// Two-pass Vamana over an in-memory adjacency list, batched + parallel.
fn vamana(vectors: &[(u32, Vec<f32>)], params: &DiskAnnBuildParams) -> (u32, Vec<Vec<u32>>) {
    let n = vectors.len();
    if n == 1 {
        return (0, vec![Vec::new()]);
    }
    let norm = normalize(vectors);
    let entry = medoid(&norm);
    let mut rng = ChaCha8Rng::seed_from_u64(BUILD_SEED);
    let mut all: Vec<u32> = (0..n as u32).collect();
    let mut adjacency: Vec<Vec<u32>> = Vec::with_capacity(n);
    for i in 0..n as u32 {
        all.shuffle(&mut rng);
        adjacency.push(
            all.iter()
                .copied()
                .filter(|&j| j != i)
                .take(params.m_max.min(n - 1))
                .collect(),
        );
    }
    let ef = params.ef_construction.max(params.m_max);
    let mut order: Vec<u32> = (0..n as u32).collect();
    let batch_cap = (n / BUILD_BATCH_DIVISOR).max(BUILD_BATCH_MIN);
    for alpha in [1.0_f32, params.alpha] {
        order.shuffle(&mut rng);
        let mut start = 0;
        let mut batch_size = BUILD_BATCH_MIN;
        while start < order.len() {
            let end = (start + batch_size).min(order.len());
            let batch = &order[start..end];
            start = end;
            batch_size = (batch_size * 2).min(batch_cap);
            // Parallel, read-only against the frozen `adjacency` snapshot.
            let pruned: Vec<(u32, Vec<u32>)> = batch
                .par_iter()
                .map(|&i| {
                    let mut candidates = greedy_search(&norm, &adjacency, entry, i, ef);
                    candidates.extend(adjacency[i as usize].iter().copied());
                    (i, robust_prune(&norm, i, candidates, alpha, params.m_max))
                })
                .collect();
            // Forward edges: sequential, cheap (assignment only).
            for (i, neighbors) in &pruned {
                adjacency[*i as usize] = neighbors.clone();
            }
            // Back-edges grouped by target (BTreeMap → deterministic key order,
            // add-lists in batch order). Each affected node is re-pruned ONCE
            // for the whole batch, and the re-prunes run in parallel — this is
            // the build's hot path, so it must not serialize.
            let mut back: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
            for (i, neighbors) in &pruned {
                for &j in neighbors {
                    back.entry(j).or_default().push(*i);
                }
            }
            let updates: Vec<(u32, Vec<u32>)> = back
                .into_iter()
                .collect::<Vec<_>>()
                .par_iter()
                .map(|(j, adds)| {
                    let mut merged = adjacency[*j as usize].clone();
                    for &i in adds {
                        if !merged.contains(&i) {
                            merged.push(i);
                        }
                    }
                    let neighbors = if merged.len() > params.m_max {
                        robust_prune(&norm, *j, merged, alpha, params.m_max)
                    } else {
                        merged
                    };
                    (*j, neighbors)
                })
                .collect();
            for (j, neighbors) in updates {
                adjacency[j as usize] = neighbors;
            }
        }
    }
    (entry, adjacency)
}

/// Point closest to the (normalized) dataset centroid — the DiskANN entry.
fn medoid(norm: &[Vec<f32>]) -> u32 {
    let dim = norm[0].len();
    let mut centroid = vec![0.0_f32; dim];
    for v in norm {
        for (c, x) in centroid.iter_mut().zip(v) {
            *c += x;
        }
    }
    let inv = 1.0 / norm.len() as f32;
    for c in &mut centroid {
        *c *= inv;
    }
    let mut best = (0_u32, f32::INFINITY);
    for (id, v) in norm.iter().enumerate() {
        let d = dist(&centroid, v);
        if d < best.1 {
            best = (id as u32, d);
        }
    }
    best.0
}

/// Greedy beam search over the in-memory adjacency from `entry` toward
/// `query` (a node id); returns every expanded node (the prune candidate set).
fn greedy_search(
    norm: &[Vec<f32>],
    adjacency: &[Vec<u32>],
    entry: u32,
    query: u32,
    ef: usize,
) -> Vec<u32> {
    let q = &norm[query as usize];
    let mut pool: Vec<(u32, f32)> = vec![(entry, dist(q, &norm[entry as usize]))];
    let mut seen: HashSet<u32> = HashSet::from([entry]);
    let mut expanded: HashSet<u32> = HashSet::new();
    let mut visited: Vec<u32> = Vec::new();
    while let Some(&(next, _)) = pool.iter().find(|(id, _)| !expanded.contains(id)) {
        expanded.insert(next);
        visited.push(next);
        for &nb in &adjacency[next as usize] {
            if seen.insert(nb) {
                pool.push((nb, dist(q, &norm[nb as usize])));
            }
        }
        pool.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        pool.truncate(ef);
    }
    visited
}

/// RobustPrune(p, candidates, alpha, r): keep the closest candidate, drop any
/// other whose distance to it (scaled by alpha) undercuts its distance to p.
fn robust_prune(norm: &[Vec<f32>], p: u32, candidates: Vec<u32>, alpha: f32, r: usize) -> Vec<u32> {
    let q = &norm[p as usize];
    let mut pool: Vec<(u32, f32)> = candidates
        .into_iter()
        .collect::<HashSet<_>>()
        .into_iter()
        .filter(|&c| c != p)
        .map(|c| (c, dist(q, &norm[c as usize])))
        .collect();
    pool.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    let mut result: Vec<u32> = Vec::with_capacity(r);
    while let Some((star, _)) = pool.first().copied() {
        result.push(star);
        if result.len() >= r {
            break;
        }
        let star_vec = &norm[star as usize];
        pool.retain(|&(c, d_pc)| c != star && alpha * dist(star_vec, &norm[c as usize]) > d_pc);
    }
    result
}
