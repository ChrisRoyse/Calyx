//! Vamana graph construction for the DiskANN on-disk format (PH68 T01).
//!
//! Two-pass build per the DiskANN paper: seeded random init edges, then for
//! each point (seeded random order) greedy-search from the medoid and
//! RobustPrune — alpha=1.0 on the first pass, `params.alpha` on the second —
//! with backward edges re-pruned on overflow. Deterministic for a fixed input.

use std::collections::HashSet;
use std::path::Path;

use calyx_core::Result;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use super::graph::{
    DISKANN_FORMAT_VERSION, DISKANN_MAX_DIM, DISKANN_MAX_M, DiskAnnGraphWriter, DiskAnnHeader,
    invalid,
};
use crate::util::cosine;

/// Deterministic build seed (Vamana insert order + random init edges).
const BUILD_SEED: u64 = 42;

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

fn distance(a: &[f32], b: &[f32]) -> f32 {
    1.0 - cosine(a, b)
}

/// Two-pass Vamana over an in-memory adjacency list.
fn vamana(vectors: &[(u32, Vec<f32>)], params: &DiskAnnBuildParams) -> (u32, Vec<Vec<u32>>) {
    let n = vectors.len();
    if n == 1 {
        return (0, vec![Vec::new()]);
    }
    let entry = medoid(vectors);
    let mut rng = ChaCha8Rng::seed_from_u64(BUILD_SEED);
    let mut adjacency: Vec<Vec<u32>> = Vec::with_capacity(n);
    for i in 0..n as u32 {
        adjacency.push(random_neighbors(&mut rng, n, i, params.m_max.min(n - 1)));
    }
    let ef = params.ef_construction.max(params.m_max);
    let mut order: Vec<u32> = (0..n as u32).collect();
    for alpha in [1.0_f32, params.alpha] {
        order.shuffle(&mut rng);
        for &i in &order {
            let query = &vectors[i as usize].1;
            let mut candidates = greedy_search(vectors, &adjacency, entry, query, ef);
            candidates.extend(adjacency[i as usize].iter().copied());
            adjacency[i as usize] = robust_prune(vectors, i, candidates, alpha, params.m_max);
            for j in adjacency[i as usize].clone() {
                if !adjacency[j as usize].contains(&i) {
                    adjacency[j as usize].push(i);
                    if adjacency[j as usize].len() > params.m_max {
                        let cands = std::mem::take(&mut adjacency[j as usize]);
                        adjacency[j as usize] =
                            robust_prune(vectors, j, cands, alpha, params.m_max);
                    }
                }
            }
        }
    }
    (entry, adjacency)
}

fn random_neighbors(rng: &mut ChaCha8Rng, n: usize, self_id: u32, count: usize) -> Vec<u32> {
    if count == n.saturating_sub(1) {
        return (0..n as u32).filter(|&id| id != self_id).collect();
    }
    let mut seen = HashSet::with_capacity(count);
    let mut out = Vec::with_capacity(count);
    while out.len() < count {
        let candidate = rng.gen_range(0..n as u32);
        if candidate != self_id && seen.insert(candidate) {
            out.push(candidate);
        }
    }
    out
}

/// Point closest to the dataset centroid (standard DiskANN entry point).
fn medoid(vectors: &[(u32, Vec<f32>)]) -> u32 {
    let dim = vectors[0].1.len();
    let mut centroid = vec![0.0_f32; dim];
    for (_, v) in vectors {
        for (c, x) in centroid.iter_mut().zip(v) {
            *c += x;
        }
    }
    let inv = 1.0 / vectors.len() as f32;
    for c in &mut centroid {
        *c *= inv;
    }
    let mut best = (0_u32, f32::INFINITY);
    for (id, v) in vectors {
        let d = distance(&centroid, v);
        if d < best.1 {
            best = (*id, d);
        }
    }
    best.0
}

/// Beam search over the in-memory adjacency; returns every expanded node
/// (the Vamana visited set used as prune candidates).
fn greedy_search(
    vectors: &[(u32, Vec<f32>)],
    adjacency: &[Vec<u32>],
    entry: u32,
    query: &[f32],
    ef: usize,
) -> Vec<u32> {
    let mut pool: Vec<(u32, f32)> = vec![(entry, distance(query, &vectors[entry as usize].1))];
    let mut seen: HashSet<u32> = HashSet::from([entry]);
    let mut expanded: HashSet<u32> = HashSet::new();
    let mut visited: Vec<u32> = Vec::new();
    while let Some(&(next, _)) = pool.iter().find(|(id, _)| !expanded.contains(id)) {
        expanded.insert(next);
        visited.push(next);
        for &nb in &adjacency[next as usize] {
            if seen.insert(nb) {
                pool.push((nb, distance(query, &vectors[nb as usize].1)));
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
    vectors: &[(u32, Vec<f32>)],
    p: u32,
    candidates: Vec<u32>,
    alpha: f32,
    r: usize,
) -> Vec<u32> {
    let query = &vectors[p as usize].1;
    let mut pool: Vec<(u32, f32)> = candidates
        .into_iter()
        .collect::<HashSet<_>>()
        .into_iter()
        .filter(|&c| c != p)
        .map(|c| (c, distance(query, &vectors[c as usize].1)))
        .collect();
    pool.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    let mut result: Vec<u32> = Vec::with_capacity(r);
    while let Some((star, _)) = pool.first().copied() {
        result.push(star);
        if result.len() >= r {
            break;
        }
        let star_vec = &vectors[star as usize].1;
        pool.retain(|&(c, d_pc)| {
            c != star && alpha * distance(star_vec, &vectors[c as usize].1) > d_pc
        });
    }
    result
}
