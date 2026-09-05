use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::Result;

use super::helpers::DiskAnnDistanceMode;
use super::raw_sidecar;
use crate::error::{CALYX_INDEX_IO, sextant_error};
use crate::index::diskann::build::{
    DiskAnnBuildBackend, DiskAnnBuildParams, DiskAnnBuildProgress,
    build_diskann_graph_raw_l2_with_backend_and_progress,
    build_diskann_graph_with_backend_and_progress, diskann_metric_sidecar_path,
};
use crate::index::distance::l2_normalize;

const DISTANCE_MODE_UNIT_L2: &str = "unit_l2\n";
const DISTANCE_MODE_RAW_COSINE: &str = "raw_cosine\n";
const DISTANCE_MODE_RAW_L2: &str = "raw_l2\n";

pub(super) fn build_search_graph_with_backend(
    graph_path: &Path,
    rows: &[(u32, Vec<f32>)],
    build_params: DiskAnnBuildParams,
    raw_sidecar: Option<PathBuf>,
    write_raw_sidecar: bool,
    backend: DiskAnnBuildBackend,
) -> Result<Option<PathBuf>> {
    build_search_graph_with_backend_and_progress(
        graph_path,
        rows,
        build_params,
        raw_sidecar,
        write_raw_sidecar,
        backend,
        |_| Ok(()),
    )
}

pub(super) fn build_search_graph_with_backend_and_progress<F>(
    graph_path: &Path,
    rows: &[(u32, Vec<f32>)],
    build_params: DiskAnnBuildParams,
    raw_sidecar: Option<PathBuf>,
    write_raw_sidecar: bool,
    backend: DiskAnnBuildBackend,
    progress: F,
) -> Result<Option<PathBuf>>
where
    F: FnMut(DiskAnnBuildProgress) -> Result<()>,
{
    build_search_graph_with_distance_backend(
        graph_path,
        rows,
        build_params,
        raw_sidecar,
        write_raw_sidecar,
        backend,
        DiskAnnDistanceMode::UnitL2,
        progress,
    )
}

pub(super) fn build_search_graph_raw_l2_with_backend(
    graph_path: &Path,
    rows: &[(u32, Vec<f32>)],
    build_params: DiskAnnBuildParams,
    raw_sidecar: Option<PathBuf>,
    write_raw_sidecar: bool,
    backend: DiskAnnBuildBackend,
) -> Result<Option<PathBuf>> {
    build_search_graph_with_distance_backend(
        graph_path,
        rows,
        build_params,
        raw_sidecar,
        write_raw_sidecar,
        backend,
        DiskAnnDistanceMode::RawL2,
        |_| Ok(()),
    )
}

#[allow(clippy::too_many_arguments)]
fn build_search_graph_with_distance_backend<F>(
    graph_path: &Path,
    rows: &[(u32, Vec<f32>)],
    build_params: DiskAnnBuildParams,
    raw_sidecar: Option<PathBuf>,
    write_raw_sidecar: bool,
    backend: DiskAnnBuildBackend,
    distance_mode: DiskAnnDistanceMode,
    mut progress: F,
) -> Result<Option<PathBuf>>
where
    F: FnMut(DiskAnnBuildProgress) -> Result<()>,
{
    match distance_mode {
        DiskAnnDistanceMode::UnitL2 => {
            let graph_rows = normalized_rows(rows);
            build_diskann_graph_with_backend_and_progress(
                graph_path,
                &graph_rows,
                build_params,
                backend,
                &mut progress,
            )?;
        }
        DiskAnnDistanceMode::RawL2 => {
            build_diskann_graph_raw_l2_with_backend_and_progress(
                graph_path,
                rows,
                build_params,
                backend,
                &mut progress,
            )?;
        }
        DiskAnnDistanceMode::RawCosine => {
            return Err(sextant_error(
                crate::error::CALYX_INDEX_INVALID_PARAMS,
                "new raw-cosine DiskANN generations are unsupported; build a unit-L2 graph",
            ));
        }
    }
    let raw_sidecar = match raw_sidecar {
        Some(path) => {
            if write_raw_sidecar {
                raw_sidecar::write_packed(&path, rows, build_params.dim)?;
            }
            Some(path)
        }
        None => {
            if write_raw_sidecar {
                let path = default_raw_sidecar(graph_path);
                raw_sidecar::write_packed(&path, rows, build_params.dim)?;
                Some(path)
            } else {
                None
            }
        }
    };
    Ok(raw_sidecar)
}

pub(super) fn default_raw_sidecar(graph_path: &Path) -> PathBuf {
    graph_path.with_extension("raw")
}

pub(super) fn read_distance_mode(graph_path: &Path) -> Result<DiskAnnDistanceMode> {
    let path = distance_mode_path(graph_path);
    let marker = fs::read_to_string(&path).map_err(|error| {
        sextant_error(
            CALYX_INDEX_IO,
            format!(
                "read required DiskANN distance mode marker {}: {error}; rebuild the index",
                path.display()
            ),
        )
    })?;
    match marker.as_str() {
        DISTANCE_MODE_UNIT_L2 => Ok(DiskAnnDistanceMode::UnitL2),
        DISTANCE_MODE_RAW_COSINE => Ok(DiskAnnDistanceMode::RawCosine),
        DISTANCE_MODE_RAW_L2 => Ok(DiskAnnDistanceMode::RawL2),
        other => Err(sextant_error(
            CALYX_INDEX_IO,
            format!(
                "DiskANN distance mode marker {} has unsupported bytes {other:?}; expected an exact newline-terminated metric marker",
                path.display()
            ),
        )),
    }
}

fn normalized_rows(rows: &[(u32, Vec<f32>)]) -> Vec<(u32, Vec<f32>)> {
    rows.iter()
        .map(|(id, vector)| (*id, l2_normalize(vector)))
        .collect()
}

fn distance_mode_path(graph_path: &Path) -> PathBuf {
    diskann_metric_sidecar_path(graph_path)
}
