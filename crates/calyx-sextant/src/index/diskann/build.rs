//! Vamana graph construction for the DiskANN on-disk format (PH68 T01/T02).
//!
//! Two-pass build per the DiskANN paper: seeded random init edges, then for
//! each point greedy-search from the medoid and RobustPrune — alpha=1.0 on the
//! first pass, `params.alpha` on the second — with backward edges re-pruned on
//! overflow.
//!
//! Construction geometry is selected per build metric: unit-L2 builds operate
//! on normalized copies so graph topology matches search-time cosine distance,
//! while raw-L2 builds operate on the source coordinates directly. Unit-L2
//! graphs store compact v3 signed-int8 directional payloads; raw-L2 graphs
//! store compact v2 f32 payloads. Each pass advances in batches: every point in
//! a batch greedy-searches the *same frozen snapshot* of the graph in parallel
//! (read-only), then edge updates apply sequentially in batch order — so the
//! build is both parallel and fully deterministic regardless of thread count.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::str::FromStr;

use calyx_core::Result;
use rand::SeedableRng;
use rand::seq::SliceRandom;
use rand_chacha::ChaCha8Rng;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

mod metric;

use super::graph::{
    DISKANN_F32_FORMAT_VERSION, DISKANN_FORMAT_VERSION, DISKANN_MAX_DIM, DISKANN_MAX_M,
    DiskAnnGraphWriter, DiskAnnHeader, invalid,
};

pub use metric::DiskAnnBuildMetric;
#[cfg(feature = "cuda")]
pub(super) use metric::normalize;
use metric::{build_space, dist};

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiskAnnBuildBackend {
    #[default]
    CpuVamana,
    CuvsCagra,
}

impl DiskAnnBuildBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CpuVamana => "cpu-vamana",
            Self::CuvsCagra => "cuvs-cagra",
        }
    }
}

impl FromStr for DiskAnnBuildBackend {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value {
            "cpu" | "cpu-vamana" => Ok(Self::CpuVamana),
            "cuvs" | "cagra" | "gpu" | "cuvs-cagra" => Ok(Self::CuvsCagra),
            other => Err(format!(
                "unknown diskann build backend {other:?}; expected cpu-vamana or cuvs-cagra"
            )),
        }
    }
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
    build_diskann_graph_with_backend(path, vectors, params, DiskAnnBuildBackend::CpuVamana)
}

pub fn build_diskann_graph_with_backend(
    path: &Path,
    vectors: &[(u32, Vec<f32>)],
    params: DiskAnnBuildParams,
    backend: DiskAnnBuildBackend,
) -> Result<()> {
    build_diskann_graph_with_metric(path, vectors, params, backend, DiskAnnBuildMetric::UnitL2)
}

pub fn build_diskann_graph_raw_l2_with_backend(
    path: &Path,
    vectors: &[(u32, Vec<f32>)],
    params: DiskAnnBuildParams,
    backend: DiskAnnBuildBackend,
) -> Result<()> {
    build_diskann_graph_with_metric(path, vectors, params, backend, DiskAnnBuildMetric::RawL2)
}

fn build_diskann_graph_with_metric(
    path: &Path,
    vectors: &[(u32, Vec<f32>)],
    params: DiskAnnBuildParams,
    backend: DiskAnnBuildBackend,
    metric: DiskAnnBuildMetric,
) -> Result<()> {
    params.validate()?;
    validate_build_input(vectors, &params)?;
    match backend {
        DiskAnnBuildBackend::CpuVamana => build_diskann_graph_cpu(path, vectors, params, metric),
        DiskAnnBuildBackend::CuvsCagra => {
            build_diskann_graph_cuvs_cagra(path, vectors, params, metric)
        }
    }
}

fn build_diskann_graph_cpu(
    path: &Path,
    vectors: &[(u32, Vec<f32>)],
    params: DiskAnnBuildParams,
    metric: DiskAnnBuildMetric,
) -> Result<()> {
    let (entry, adjacency) = vamana(vectors, &params, metric);
    match metric {
        DiskAnnBuildMetric::UnitL2 => {
            write_graph_from_adjacency(path, vectors, params, entry, &adjacency)
        }
        DiskAnnBuildMetric::RawL2 => {
            write_graph_from_adjacency_f32(path, vectors, params, entry, &adjacency)
        }
    }
}

pub(super) fn validate_build_input(
    vectors: &[(u32, Vec<f32>)],
    params: &DiskAnnBuildParams,
) -> Result<()> {
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
    Ok(())
}

pub(super) fn write_graph_from_adjacency(
    path: &Path,
    vectors: &[(u32, Vec<f32>)],
    params: DiskAnnBuildParams,
    entry: u32,
    adjacency: &[Vec<u32>],
) -> Result<()> {
    write_graph_from_adjacency_with_format(
        path,
        vectors,
        params,
        entry,
        adjacency,
        DISKANN_FORMAT_VERSION,
    )
}

pub(super) fn write_graph_from_adjacency_f32(
    path: &Path,
    vectors: &[(u32, Vec<f32>)],
    params: DiskAnnBuildParams,
    entry: u32,
    adjacency: &[Vec<u32>],
) -> Result<()> {
    write_graph_from_adjacency_with_format(
        path,
        vectors,
        params,
        entry,
        adjacency,
        DISKANN_F32_FORMAT_VERSION,
    )
}

fn write_graph_from_adjacency_with_format(
    path: &Path,
    vectors: &[(u32, Vec<f32>)],
    params: DiskAnnBuildParams,
    entry: u32,
    adjacency: &[Vec<u32>],
    format_version: u32,
) -> Result<()> {
    if adjacency.len() != vectors.len() {
        return Err(invalid(format!(
            "adjacency len {} != vector len {}",
            adjacency.len(),
            vectors.len()
        )));
    }
    for (id, neighbors) in adjacency.iter().enumerate() {
        if neighbors.len() > params.m_max {
            return Err(invalid(format!(
                "node {id} degree {} > m_max {}",
                neighbors.len(),
                params.m_max
            )));
        }
    }
    let max_degree = adjacency.iter().map(Vec::len).max().unwrap_or(0);
    let header = DiskAnnHeader {
        format_version,
        dim: u32::try_from(params.dim).expect("dim <= 8192"),
        m_max: u32::try_from(params.m_max).expect("m_max <= 512"),
        max_degree: u32::try_from(max_degree).expect("<= m_max"),
        entry_point_id: entry,
        node_count: adjacency.len() as u64,
    };
    let mut writer = DiskAnnGraphWriter::create(path, header)?;
    for (id, vector) in vectors {
        writer.write_node(*id, vector, &adjacency[*id as usize])?;
    }
    writer.finish()
}

#[cfg(feature = "cuda")]
fn build_diskann_graph_cuvs_cagra(
    path: &Path,
    vectors: &[(u32, Vec<f32>)],
    params: DiskAnnBuildParams,
    metric: DiskAnnBuildMetric,
) -> Result<()> {
    super::cuvs_cagra::build_diskann_graph_cuvs_cagra(path, vectors, params, metric)
}

#[cfg(not(feature = "cuda"))]
fn build_diskann_graph_cuvs_cagra(
    _path: &Path,
    _vectors: &[(u32, Vec<f32>)],
    _params: DiskAnnBuildParams,
    _metric: DiskAnnBuildMetric,
) -> Result<()> {
    Err(invalid(
        "cuvs-cagra backend requires building calyx-sextant with --features cuda",
    ))
}

/// Two-pass Vamana over an in-memory adjacency list, batched + parallel.
fn vamana(
    vectors: &[(u32, Vec<f32>)],
    params: &DiskAnnBuildParams,
    metric: DiskAnnBuildMetric,
) -> (u32, Vec<Vec<u32>>) {
    let n = vectors.len();
    if n == 1 {
        return (0, vec![Vec::new()]);
    }
    let space = build_space(vectors, metric);
    let entry = medoid(&space, metric);
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
        }
    }
    (entry, adjacency)
}

/// Point closest to the active build-space centroid — the DiskANN entry.
pub(super) fn medoid(space: &[Vec<f32>], metric: DiskAnnBuildMetric) -> u32 {
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
