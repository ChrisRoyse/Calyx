use std::collections::{BTreeMap, HashSet};

use calyx_core::Result;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rayon::prelude::*;

use super::metric::{DiskAnnBuildMetric, build_space, dist};
use super::{DiskAnnBuildParams, DiskAnnBuildProgress};

/// Deterministic build seed (Vamana insert order + random init edges).
const BUILD_SEED: u64 = 42;
/// First synchronization round size. Batches grow geometrically from here
/// (ParlayANN prefix-doubling): early points refine the graph at near-
/// sequential quality, later points parallelize over the larger snapshot.
const BUILD_BATCH_MIN: usize = 256;
/// Batches never exceed `n / BUILD_BATCH_DIVISOR` so that no single
/// synchronization round connects more than a small fraction of the graph
/// against one stale snapshot -- keeping graph quality scale-independent.
const BUILD_BATCH_DIVISOR: usize = 32;
const BUILD_PROGRESS_ROWS: usize = 4096;

/// Two-pass Vamana over an in-memory adjacency list, batched + parallel.
pub(super) fn vamana<F>(
    vectors: &[(u32, Vec<f32>)],
    params: &DiskAnnBuildParams,
    metric: DiskAnnBuildMetric,
    progress: &mut F,
) -> Result<(u32, Vec<Vec<u32>>)>
where
    F: FnMut(DiskAnnBuildProgress) -> Result<()>,
{
    let n = vectors.len();
    if n == 1 {
        return Ok((0, vec![Vec::new()]));
    }
    progress(DiskAnnBuildProgress::new("diskann_space_start", 0))?;
    let space = build_space(vectors, metric);
    let entry = medoid(&space, metric);
    progress(DiskAnnBuildProgress::new("diskann_space_ok", n))?;
    let mut rng = ChaCha8Rng::seed_from_u64(BUILD_SEED);
    let init_degree = params.m_max.min(n - 1);
    let mut adjacency: Vec<Vec<u32>> = Vec::with_capacity(n);
    progress(DiskAnnBuildProgress::new("diskann_init_start", 0))?;
    for i in 0..n as u32 {
        adjacency.push(initial_neighbors(n as u32, i, init_degree, entry, &mut rng));
        let initialized = i as usize + 1;
        if initialized == n || initialized.is_multiple_of(BUILD_PROGRESS_ROWS) {
            progress(DiskAnnBuildProgress::new("diskann_init_page", initialized))?;
        }
    }
    progress(DiskAnnBuildProgress::new("diskann_init_ok", n))?;
    let ef = params.ef_construction.max(params.m_max);
    let mut order: Vec<u32> = (0..n as u32).collect();
    let batch_cap = (n / BUILD_BATCH_DIVISOR).max(BUILD_BATCH_MIN);
    for (pass_idx, alpha) in [1.0_f32, params.alpha].into_iter().enumerate() {
        let (pass_start, pass_page, pass_ok) = match pass_idx {
            0 => (
                "diskann_vamana_pass1_start",
                "diskann_vamana_pass1_batch_ok",
                "diskann_vamana_pass1_ok",
            ),
            _ => (
                "diskann_vamana_pass2_start",
                "diskann_vamana_pass2_batch_ok",
                "diskann_vamana_pass2_ok",
            ),
        };
        progress(DiskAnnBuildProgress::new(pass_start, 0))?;
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
                    let mut candidates = greedy_search(&space, &adjacency, entry, i, ef, metric);
                    candidates.extend(adjacency[i as usize].iter().copied());
                    (
                        i,
                        robust_prune(&space, i, candidates, alpha, params.m_max, metric),
                    )
                })
                .collect();
            // Forward edges: sequential, cheap (assignment only).
            for (i, neighbors) in &pruned {
                adjacency[*i as usize] = neighbors.clone();
            }
            // Back-edges grouped by target (BTreeMap -> deterministic key order,
            // add-lists in batch order). Each affected node is re-pruned ONCE
            // for the whole batch, and the re-prunes run in parallel -- this is
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
                        robust_prune(&space, *j, merged, alpha, params.m_max, metric)
                    } else {
                        merged
                    };
                    (*j, neighbors)
                })
                .collect();
            for (j, neighbors) in updates {
                adjacency[j as usize] = neighbors;
            }
            progress(DiskAnnBuildProgress::new(pass_page, end))?;
        }
        progress(DiskAnnBuildProgress::new(pass_ok, n))?;
    }
    Ok((entry, adjacency))
}

fn initial_neighbors(
    node_count: u32,
    node: u32,
    degree: usize,
    entry: u32,
    rng: &mut ChaCha8Rng,
) -> Vec<u32> {
    let mut out = Vec::with_capacity(degree);
    let mut seen = HashSet::with_capacity(degree);
    if node != entry && degree > 0 {
        out.push(entry);
        seen.insert(entry);
    }
    while out.len() < degree {
        let candidate = rng.random_range(0..node_count);
        if candidate != node && seen.insert(candidate) {
            out.push(candidate);
        }
    }
    out
}

/// Point closest to the active build-space centroid — the DiskANN entry.
pub(in crate::index::diskann) fn medoid(space: &[Vec<f32>], metric: DiskAnnBuildMetric) -> u32 {
    let dim = space[0].len();
    let mut centroid = vec![0.0_f32; dim];
    for v in space {
        for (c, x) in centroid.iter_mut().zip(v) {
            *c += x;
        }
    }
    let inv = 1.0 / space.len() as f32;
    for c in &mut centroid {
        *c *= inv;
    }
    let mut best = (0_u32, f32::INFINITY);
    for (id, v) in space.iter().enumerate() {
        let d = dist(&centroid, v, metric);
        if d < best.1 {
            best = (id as u32, d);
        }
    }
    best.0
}

/// Greedy beam search over the in-memory adjacency from `entry` toward
/// `query` (a node id); returns every expanded node (the prune candidate set).
fn greedy_search(
    space: &[Vec<f32>],
    adjacency: &[Vec<u32>],
    entry: u32,
    query: u32,
    ef: usize,
    metric: DiskAnnBuildMetric,
) -> Vec<u32> {
    let q = &space[query as usize];
    let mut pool: Vec<(u32, f32)> = vec![(entry, dist(q, &space[entry as usize], metric))];
    let mut seen: HashSet<u32> = HashSet::from([entry]);
    let mut expanded: HashSet<u32> = HashSet::new();
    let mut visited: Vec<u32> = Vec::new();
    while let Some(&(next, _)) = pool.iter().find(|(id, _)| !expanded.contains(id)) {
        expanded.insert(next);
        visited.push(next);
        for &nb in &adjacency[next as usize] {
            if seen.insert(nb) {
                pool.push((nb, dist(q, &space[nb as usize], metric)));
            }
        }
        pool.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        pool.truncate(ef);
    }
    visited
}

/// RobustPrune(p, candidates, alpha, r): keep the closest candidate, drop any
/// other whose distance to it (scaled by alpha) undercuts its distance to p.
fn robust_prune(
    space: &[Vec<f32>],
    p: u32,
    candidates: Vec<u32>,
    alpha: f32,
    r: usize,
    metric: DiskAnnBuildMetric,
) -> Vec<u32> {
    let q = &space[p as usize];
    let mut pool: Vec<(u32, f32)> = candidates
        .into_iter()
        .collect::<HashSet<_>>()
        .into_iter()
        .filter(|&c| c != p)
        .map(|c| (c, dist(q, &space[c as usize], metric)))
        .collect();
    pool.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    let mut result: Vec<u32> = Vec::with_capacity(r);
    while let Some((star, _)) = pool.first().copied() {
        result.push(star);
        if result.len() >= r {
            break;
        }
        let star_vec = &space[star as usize];
        pool.retain(|&(c, d_pc)| {
            c != star && alpha * dist(star_vec, &space[c as usize], metric) > d_pc
        });
    }
    result
}
