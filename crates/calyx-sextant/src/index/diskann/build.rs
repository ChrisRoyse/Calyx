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

use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use calyx_core::Result;
use serde::{Deserialize, Serialize};

mod metric;
mod vamana;

use super::graph::{
    DISKANN_F32_FORMAT_VERSION, DISKANN_FORMAT_VERSION, DISKANN_MAX_DIM, DISKANN_MAX_M,
    DiskAnnGraphWriter, DiskAnnHeader, invalid,
};

pub use metric::DiskAnnBuildMetric;
#[cfg(sextant_cuvs)]
pub(super) use metric::normalize;
#[cfg(sextant_cuvs)]
pub(super) use vamana::medoid;
use vamana::vamana;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskAnnBuildProgress {
    pub phase: &'static str,
    pub rows: usize,
}

impl DiskAnnBuildProgress {
    pub(super) fn new(phase: &'static str, rows: usize) -> Self {
        Self { phase, rows }
    }
}

const GRAPH_WRITE_PROGRESS_ROWS: usize = 4096;
const DISKANN_METRIC_UNIT_L2: &str = "unit_l2\n";
const DISKANN_METRIC_RAW_L2: &str = "raw_l2\n";

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
    build_diskann_graph_with_backend_and_progress(path, vectors, params, backend, |_| Ok(()))
}

pub fn build_diskann_graph_with_backend_and_progress<F>(
    path: &Path,
    vectors: &[(u32, Vec<f32>)],
    params: DiskAnnBuildParams,
    backend: DiskAnnBuildBackend,
    progress: F,
) -> Result<()>
where
    F: FnMut(DiskAnnBuildProgress) -> Result<()>,
{
    build_diskann_graph_with_metric_and_progress(
        path,
        vectors,
        params,
        backend,
        DiskAnnBuildMetric::UnitL2,
        progress,
    )
}

pub fn build_diskann_graph_raw_l2_with_backend(
    path: &Path,
    vectors: &[(u32, Vec<f32>)],
    params: DiskAnnBuildParams,
    backend: DiskAnnBuildBackend,
) -> Result<()> {
    build_diskann_graph_raw_l2_with_backend_and_progress(path, vectors, params, backend, |_| Ok(()))
}

pub fn build_diskann_graph_raw_l2_with_backend_and_progress<F>(
    path: &Path,
    vectors: &[(u32, Vec<f32>)],
    params: DiskAnnBuildParams,
    backend: DiskAnnBuildBackend,
    progress: F,
) -> Result<()>
where
    F: FnMut(DiskAnnBuildProgress) -> Result<()>,
{
    build_diskann_graph_with_metric_and_progress(
        path,
        vectors,
        params,
        backend,
        DiskAnnBuildMetric::RawL2,
        progress,
    )
}

fn build_diskann_graph_with_metric_and_progress<F>(
    path: &Path,
    vectors: &[(u32, Vec<f32>)],
    params: DiskAnnBuildParams,
    backend: DiskAnnBuildBackend,
    metric: DiskAnnBuildMetric,
    mut progress: F,
) -> Result<()>
where
    F: FnMut(DiskAnnBuildProgress) -> Result<()>,
{
    params.validate()?;
    validate_build_input(vectors, &params)?;
    // The metric is part of the graph generation contract. Invalidate it
    // before touching any generation asset so an interrupted or failed
    // rebuild cannot leave a readable graph with a stale distance mode.
    remove_metric_sidecar_if_present(&diskann_metric_sidecar_path(path))?;
    let cagra_sidecar = cagra_sidecar_path(path);
    let cagra_dataset = cagra_dataset_sidecar_path(path);
    match backend {
        DiskAnnBuildBackend::CpuVamana => {
            // A CPU rebuild changes the graph generation. Remove any prior
            // device-serving asset first so serving can only fail closed,
            // never deserialize a stale CAGRA index.
            remove_cagra_sidecar_if_present(&cagra_sidecar)?;
            remove_cagra_sidecar_if_present(&cagra_dataset)?;
            build_diskann_graph_cpu(path, vectors, params, metric, &mut progress)?;
        }
        DiskAnnBuildBackend::CuvsCagra => {
            progress(DiskAnnBuildProgress::new("diskann_cuvs_cagra_start", 0))?;
            build_diskann_graph_cuvs_cagra(path, vectors, params, metric)?;
            progress(DiskAnnBuildProgress::new(
                "diskann_cuvs_cagra_ok",
                vectors.len(),
            ))?;
        }
    }
    publish_diskann_build_metric(path, metric)
}

/// Distance-mode marker paired with every successfully published DiskANN graph.
pub fn diskann_metric_sidecar_path(graph_path: &Path) -> PathBuf {
    graph_path.with_extension("metric")
}

/// Read a builder-owned metric marker. Absence and unknown values are corrupt
/// generation state, never an invitation to guess a legacy distance mode.
pub fn read_diskann_build_metric(graph_path: &Path) -> Result<DiskAnnBuildMetric> {
    let path = diskann_metric_sidecar_path(graph_path);
    let marker = fs::read_to_string(&path).map_err(|error| {
        crate::error::sextant_error(
            crate::error::CALYX_INDEX_IO,
            format!(
                "read required DiskANN metric marker {}: {error}; rebuild the index",
                path.display()
            ),
        )
    })?;
    match marker.as_str() {
        DISKANN_METRIC_UNIT_L2 => Ok(DiskAnnBuildMetric::UnitL2),
        DISKANN_METRIC_RAW_L2 => Ok(DiskAnnBuildMetric::RawL2),
        other => Err(crate::error::sextant_error(
            crate::error::CALYX_INDEX_IO,
            format!(
                "DiskANN metric marker {} has unsupported bytes {other:?}; expected exactly unit_l2\\n or raw_l2\\n",
                path.display()
            ),
        )),
    }
}

fn publish_diskann_build_metric(graph_path: &Path, metric: DiskAnnBuildMetric) -> Result<()> {
    let marker = match metric {
        DiskAnnBuildMetric::UnitL2 => DISKANN_METRIC_UNIT_L2,
        DiskAnnBuildMetric::RawL2 => DISKANN_METRIC_RAW_L2,
    };
    calyx_aster::private_fs::write_private_atomic(
        &diskann_metric_sidecar_path(graph_path),
        marker.as_bytes(),
    )
}

fn remove_metric_sidecar_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(crate::error::sextant_error(
            crate::error::CALYX_INDEX_IO,
            format!(
                "invalidate stale DiskANN metric marker {}: {error}",
                path.display()
            ),
        )),
    }
}

/// Dataset-inclusive cuVS CAGRA asset used by CUDA serving.
pub fn cagra_sidecar_path(graph_path: &Path) -> PathBuf {
    graph_path.with_extension("cagra")
}

/// Compact dataset paired with the CAGRA generation for partitioned CUDA serving.
pub fn cagra_dataset_sidecar_path(graph_path: &Path) -> PathBuf {
    graph_path.with_extension("cagra-data")
}

fn remove_cagra_sidecar_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(crate::error::sextant_error(
            crate::error::CALYX_INDEX_IO,
            format!(
                "remove stale CAGRA serving asset {}: {error}",
                path.display()
            ),
        )),
    }
}

fn build_diskann_graph_cpu<F>(
    path: &Path,
    vectors: &[(u32, Vec<f32>)],
    params: DiskAnnBuildParams,
    metric: DiskAnnBuildMetric,
    progress: &mut F,
) -> Result<()>
where
    F: FnMut(DiskAnnBuildProgress) -> Result<()>,
{
    let (entry, adjacency) = vamana(vectors, &params, metric, progress)?;
    match metric {
        DiskAnnBuildMetric::UnitL2 => write_graph_from_adjacency_with_progress(
            path, vectors, params, entry, &adjacency, progress,
        ),
        DiskAnnBuildMetric::RawL2 => write_graph_from_adjacency_f32_with_progress(
            path, vectors, params, entry, &adjacency, progress,
        ),
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

#[cfg(sextant_cuvs)]
pub(super) fn write_graph_from_adjacency(
    path: &Path,
    vectors: &[(u32, Vec<f32>)],
    params: DiskAnnBuildParams,
    entry: u32,
    adjacency: &[Vec<u32>],
) -> Result<()> {
    let mut no_progress = |_| Ok(());
    write_graph_from_adjacency_with_progress(
        path,
        vectors,
        params,
        entry,
        adjacency,
        &mut no_progress,
    )
}

pub(super) fn write_graph_from_adjacency_with_progress<F>(
    path: &Path,
    vectors: &[(u32, Vec<f32>)],
    params: DiskAnnBuildParams,
    entry: u32,
    adjacency: &[Vec<u32>],
    progress: &mut F,
) -> Result<()>
where
    F: FnMut(DiskAnnBuildProgress) -> Result<()>,
{
    write_graph_from_adjacency_with_format(
        path,
        vectors,
        params,
        entry,
        adjacency,
        DISKANN_FORMAT_VERSION,
        progress,
    )
}

#[cfg(sextant_cuvs)]
pub(super) fn write_graph_from_adjacency_f32(
    path: &Path,
    vectors: &[(u32, Vec<f32>)],
    params: DiskAnnBuildParams,
    entry: u32,
    adjacency: &[Vec<u32>],
) -> Result<()> {
    let mut no_progress = |_| Ok(());
    write_graph_from_adjacency_f32_with_progress(
        path,
        vectors,
        params,
        entry,
        adjacency,
        &mut no_progress,
    )
}

pub(super) fn write_graph_from_adjacency_f32_with_progress<F>(
    path: &Path,
    vectors: &[(u32, Vec<f32>)],
    params: DiskAnnBuildParams,
    entry: u32,
    adjacency: &[Vec<u32>],
    progress: &mut F,
) -> Result<()>
where
    F: FnMut(DiskAnnBuildProgress) -> Result<()>,
{
    write_graph_from_adjacency_with_format(
        path,
        vectors,
        params,
        entry,
        adjacency,
        DISKANN_F32_FORMAT_VERSION,
        progress,
    )
}

fn write_graph_from_adjacency_with_format<F>(
    path: &Path,
    vectors: &[(u32, Vec<f32>)],
    params: DiskAnnBuildParams,
    entry: u32,
    adjacency: &[Vec<u32>],
    format_version: u32,
    progress: &mut F,
) -> Result<()>
where
    F: FnMut(DiskAnnBuildProgress) -> Result<()>,
{
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
    progress(DiskAnnBuildProgress::new("diskann_graph_write_start", 0))?;
    for (idx, (id, vector)) in vectors.iter().enumerate() {
        writer.write_node(*id, vector, &adjacency[*id as usize])?;
        let written = idx + 1;
        if written == vectors.len() || written % GRAPH_WRITE_PROGRESS_ROWS == 0 {
            progress(DiskAnnBuildProgress::new(
                "diskann_graph_write_page",
                written,
            ))?;
        }
    }
    writer.finish()?;
    progress(DiskAnnBuildProgress::new(
        "diskann_graph_write_ok",
        vectors.len(),
    ))
}

#[cfg(sextant_cuvs)]
fn build_diskann_graph_cuvs_cagra(
    path: &Path,
    vectors: &[(u32, Vec<f32>)],
    params: DiskAnnBuildParams,
    metric: DiskAnnBuildMetric,
) -> Result<()> {
    super::cuvs_cagra::build_diskann_graph_cuvs_cagra(path, vectors, params, metric)
}

#[cfg(not(sextant_cuvs))]
fn build_diskann_graph_cuvs_cagra(
    _path: &Path,
    _vectors: &[(u32, Vec<f32>)],
    _params: DiskAnnBuildParams,
    _metric: DiskAnnBuildMetric,
) -> Result<()> {
    Err(invalid(crate::cuvs_unavailable_reason(
        "cuvs-cagra backend",
    )))
}

#[cfg(test)]
#[path = "build_tests.rs"]
mod build_tests;
