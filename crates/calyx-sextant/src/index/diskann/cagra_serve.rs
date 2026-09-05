//! Fail-closed, device-resident CAGRA serving for persisted DiskANN graphs.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use calyx_core::Result;
use serde::Serialize;

use crate::error::{
    CALYX_INDEX_DIM_MISMATCH, CALYX_INDEX_INVALID_PARAMS, CALYX_SEXTANT_GPU_SERVING_UNAVAILABLE,
    sextant_error,
};

/// Largest query matrix admitted to one CAGRA launch. cuVS sizes its search
/// workspace from `max_queries`; 4,096 keeps that workspace bounded while
/// giving corpus-scale callers the large-batch occupancy CAGRA is designed for.
pub const CAGRA_SERVING_MAX_BATCH: usize = 4096;
pub const CAGRA_SERVING_MAX_K: usize = 1024;
pub const CAGRA_PARTITIONED_MAX_REGIONS: usize = 4096;
pub const CAGRA_PARTITIONED_MAX_DIM: usize = 65_536;
pub const CAGRA_PARTITIONED_MAX_SCRATCH_BYTES: u64 = (CAGRA_PARTITIONED_MAX_REGIONS as u64)
    * (8 + 4 + 8 + 4 + 8 + 8)
    + (CAGRA_PARTITIONED_MAX_DIM as u64) * 4
    + (CAGRA_PARTITIONED_MAX_REGIONS as u64) * (CAGRA_SERVING_MAX_K as u64) * (8 + 4)
    + (CAGRA_SERVING_MAX_K as u64) * (8 + 4);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CagraServingMetric {
    UnitL2,
    RawL2,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct CagraServingDiagnostics {
    pub backend: &'static str,
    pub cache_entries: usize,
    pub cache_max_entries: usize,
    pub resident_bytes: u64,
    pub resident_max_bytes: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_evictions: u64,
    pub cache_invalidations: u64,
    pub batches: u64,
    pub queries: u64,
    pub cagra_kernel_launches: u64,
    pub exact_filter_kernel_launches: u64,
    pub partitioned_exact_kernel_launches: u64,
    pub partitioned_merge_kernel_launches: u64,
    pub partitioned_prepare_us: u64,
    pub partitioned_execute_us: u64,
    pub partitioned_scratch_bytes: u64,
    pub partitioned_scratch_max_bytes: u64,
    pub partitioned_i8_dataset_loads: u64,
    pub partitioned_f32_dataset_loads: u64,
    pub partitioned_pool_reserved_bytes: u64,
    pub partitioned_pool_reserved_max_bytes: u64,
    pub partitioned_pool_used_bytes: u64,
    pub partitioned_pool_used_max_bytes: u64,
    pub query_uploads: u64,
    pub filter_uploads: u64,
    pub h2d_bytes: u64,
    pub d2h_bytes: u64,
    pub final_readback_pairs: u64,
    pub intermediate_readback_pairs: u64,
    pub failures: u64,
}

pub struct CagraSearchRequest<'a> {
    pub graph_path: &'a Path,
    pub metric: CagraServingMetric,
    pub queries: &'a [f32],
    pub query_count: usize,
    pub k: usize,
    pub ef_search: usize,
    /// Local row ids that may be returned. One device bitset is shared by the batch.
    pub allowed_ids: Option<&'a [u32]>,
}

/// Immutable identity for one validated serving sidecar. Partitioned search
/// captures this with its manifest snapshot so hot queries do not repeat
/// canonicalization and metadata I/O for every touched region.
#[derive(Clone, Debug)]
#[cfg_attr(not(sextant_cuvs), allow(dead_code))]
pub struct CagraServingAsset {
    path: PathBuf,
    len: u64,
    modified_ns: u128,
    generation_digest: [u8; 32],
}

/// Snapshot identity for a partition region and its local-to-global map.
#[derive(Clone, Debug)]
#[cfg_attr(not(sextant_cuvs), allow(dead_code))]
pub struct CagraServingRegion {
    asset: CagraServingAsset,
    global_ids_digest: [u8; 32],
    metric: CagraServingMetric,
}

pub struct CagraPartitionRegion<'a> {
    pub serving: &'a CagraServingRegion,
    pub global_ids: &'a [u64],
}

pub struct CagraPartitionSearchRequest<'a> {
    pub metric: CagraServingMetric,
    pub query: &'a [f32],
    pub k: usize,
    pub regions: &'a [CagraPartitionRegion<'a>],
}

pub fn cagra_serving_region(
    graph_path: &Path,
    global_ids: &[u64],
    metric: CagraServingMetric,
    expected_dim: usize,
) -> Result<CagraServingRegion> {
    let sidecar = super::cagra_dataset_sidecar_path(graph_path);
    let path = sidecar.canonicalize().map_err(|error| {
        serving_unavailable(format!(
            "required CAGRA serving asset {} is unavailable: {error}",
            sidecar.display()
        ))
    })?;
    let metadata = path.metadata().map_err(|error| {
        serving_unavailable(format!("stat CAGRA asset {}: {error}", path.display()))
    })?;
    if metadata.len() == 0 {
        return Err(serving_unavailable(format!(
            "required CAGRA serving asset {} is empty",
            path.display()
        )));
    }
    let header = super::cagra_dataset::read_header(&path)?;
    let header_metric = match header.metric {
        super::cagra_dataset::DatasetMetric::UnitL2 => CagraServingMetric::UnitL2,
        super::cagra_dataset::DatasetMetric::RawL2 => CagraServingMetric::RawL2,
    };
    if header_metric != metric || header.rows != global_ids.len() || header.dim != expected_dim {
        return Err(serving_unavailable(format!(
            "CAGRA dataset generation metric/rows/dim {:?}/{}/{} != manifest {:?}/{}/{}",
            header_metric,
            header.rows,
            header.dim,
            metric,
            global_ids.len(),
            expected_dim
        )));
    }
    let modified_ns = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos());
    Ok(CagraServingRegion {
        asset: CagraServingAsset {
            path,
            len: metadata.len(),
            modified_ns,
            generation_digest: header.payload_digest,
        },
        global_ids_digest: digest_global_ids(global_ids),
        metric,
    })
}

fn digest_global_ids(values: &[u64]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    for value in values {
        hasher.update(&value.to_le_bytes());
    }
    *hasher.finalize().as_bytes()
}

fn serving_unavailable(detail: impl Into<String>) -> calyx_core::CalyxError {
    sextant_error(CALYX_SEXTANT_GPU_SERVING_UNAVAILABLE, detail)
}

pub fn cagra_search_batch(request: CagraSearchRequest<'_>) -> Result<Vec<Vec<(u32, f32)>>> {
    validate(&request)?;
    #[cfg(sextant_cuvs)]
    {
        cache::search(request)
    }
    #[cfg(not(sextant_cuvs))]
    {
        Err(sextant_error(
            crate::error::CALYX_SEXTANT_GPU_SERVING_UNAVAILABLE,
            crate::cuvs_unavailable_reason("persisted CAGRA search serving"),
        ))
    }
}

pub fn cagra_serving_diagnostics() -> CagraServingDiagnostics {
    #[cfg(sextant_cuvs)]
    {
        cache::diagnostics()
    }
    #[cfg(not(sextant_cuvs))]
    {
        CagraServingDiagnostics {
            backend: "unavailable",
            ..CagraServingDiagnostics::default()
        }
    }
}

/// Bind the process-wide CUDA generation cache to an admitted runtime budget.
///
/// Reconfiguration evicts only idle generations and fails closed if active
/// generations prevent the requested bound. Callers must reserve CUDA driver
/// and workspace overhead outside `max_bytes`.
pub fn configure_cagra_serving_cache(max_bytes: u64, max_entries: usize) -> Result<()> {
    #[cfg(sextant_cuvs)]
    {
        cache::configure(max_bytes, max_entries)
    }
    #[cfg(not(sextant_cuvs))]
    {
        let _ = (max_bytes, max_entries);
        Err(sextant_error(
            crate::error::CALYX_SEXTANT_GPU_SERVING_UNAVAILABLE,
            crate::cuvs_unavailable_reason("persisted CAGRA cache configuration"),
        ))
    }
}

pub fn cagra_partitioned_search(
    request: CagraPartitionSearchRequest<'_>,
) -> Result<Vec<(u64, f32)>> {
    if request.query.is_empty()
        || request.query.len() > CAGRA_PARTITIONED_MAX_DIM
        || request.query.iter().any(|value| !value.is_finite())
        || request.k == 0
        || request.k > CAGRA_SERVING_MAX_K
        || request.regions.is_empty()
        || request.regions.len() > CAGRA_PARTITIONED_MAX_REGIONS
    {
        return Err(sextant_error(
            CALYX_INDEX_INVALID_PARAMS,
            format!(
                "partitioned CAGRA serving requires a finite query with 0<dim<={CAGRA_PARTITIONED_MAX_DIM}, 0<k<={CAGRA_SERVING_MAX_K}, and 0<regions<={CAGRA_PARTITIONED_MAX_REGIONS}"
            ),
        ));
    }
    #[cfg(sextant_cuvs)]
    {
        cache::search_partitioned(request)
    }
    #[cfg(not(sextant_cuvs))]
    {
        Err(sextant_error(
            crate::error::CALYX_SEXTANT_GPU_SERVING_UNAVAILABLE,
            crate::cuvs_unavailable_reason("partitioned CAGRA search serving"),
        ))
    }
}

fn validate(request: &CagraSearchRequest<'_>) -> Result<()> {
    if request.query_count == 0
        || request.query_count > CAGRA_SERVING_MAX_BATCH
        || request.k == 0
        || request.k > CAGRA_SERVING_MAX_K
        || request.ef_search == 0
    {
        return Err(sextant_error(
            CALYX_INDEX_INVALID_PARAMS,
            format!(
                "CAGRA serving requires 0<query_count<={CAGRA_SERVING_MAX_BATCH}, 0<k<={CAGRA_SERVING_MAX_K}, and ef_search>0; received query_count={}, k={}, ef_search={}",
                request.query_count, request.k, request.ef_search
            ),
        ));
    }
    if !request.queries.len().is_multiple_of(request.query_count) || request.queries.is_empty() {
        return Err(sextant_error(
            CALYX_INDEX_DIM_MISMATCH,
            "CAGRA serving query matrix is not divisible by query_count",
        ));
    }
    if request.queries.iter().any(|value| !value.is_finite()) {
        return Err(sextant_error(
            CALYX_INDEX_INVALID_PARAMS,
            "CAGRA serving query matrix contains a non-finite value",
        ));
    }
    Ok(())
}

#[cfg(sextant_cuvs)]
#[path = "cagra_serve/cache.rs"]
mod cache;
#[cfg(sextant_cuvs)]
mod cache_config;
#[cfg(sextant_cuvs)]
#[path = "cagra_serve/cuda.rs"]
mod cuda;
#[cfg(sextant_cuvs)]
#[path = "cagra_serve/output.rs"]
mod output;
#[cfg(sextant_cuvs)]
#[path = "cagra_serve/partition_asset.rs"]
mod partition_asset;
#[cfg(sextant_cuvs)]
#[path = "cagra_serve/partitioned.rs"]
mod partitioned;
#[cfg(sextant_cuvs)]
#[path = "cagra_serve/telemetry.rs"]
mod telemetry;
