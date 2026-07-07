//! PH68 T06 memory-bounded partitioned billion-scale vault (#550).

mod assignment;
mod balance;
mod manifest;
mod metric;
mod search;
mod sources;

use std::path::Path;

use calyx_core::{CxId, Result, SlotId};
use rayon::{ThreadPoolBuilder, prelude::*};

use crate::index::{
    DiskAnnBuildBackend, DiskAnnBuildParams, DiskAnnSearch, DiskAnnSearchParams,
    SpannCentroidIndex, build_centroids,
};
use assignment::{
    AssignmentRouting, AssignmentSink, BoundedAssignmentConfig, read_ids,
    stream_assign_to_ids_bounded, stream_assign_to_ids_with_routing,
};
use balance::balance_region_files;
pub use manifest::{
    ClosureAssignmentStats, PartitionedManifest, PartitionedManifestDbReadback, RegionMeta,
};
pub use metric::PartitionDistanceMetric;
pub use search::{PartitionedSearch, PartitionedSearchOptions, PartitionedSearchReadback};
use sources::normalize;
pub use sources::{FbinSource, I8BinSource, SyntheticSource, VectorSource, gen_row};

const CENTROID_DIR: &str = "idx/slot_00.sparse";
const ROOT_GRAPH: &str = "idx/slot_00.ann/graph.cda";
/// Mixing constant for per-index RNG seeding (splitmix64 multiplier).
const IDX_MIX: u64 = 0x9E37_79B9_7F4A_7C15;
/// Floor for the per-region size cap used by region balancing (#713).
const MIN_REGION_CAP: usize = 2_048;
pub const DEFAULT_FINAL_ASSIGNMENT_PROBE: usize = 32;
const FINAL_ASSIGNMENT_BOUNDARY_EPSILON: f32 = 0.10;
/// Closure replication defaults OFF (#1129): under the strict RNG rule the
/// replication factor measured 1.003-1.078 on real SpaceV at every Calyx
/// geometry, with <=1.1pt recall@10 gain at fixed n_probe — pure build/byte
/// cost. Replication is an explicit storage-for-probes trade: opt in with
/// `--max-replication N` (+ `--rng-factor` to actually keep replicas).
const FINAL_ASSIGNMENT_MAX_REPLICATION: usize = 1;
const FINAL_ASSIGNMENT_RNG_FACTOR: f32 = 1.0;

pub fn cx(idx: u64) -> CxId {
    let mut bytes = [0u8; 16];
    bytes[8..16].copy_from_slice(&idx.to_be_bytes());
    CxId::from_bytes(bytes)
}

#[derive(Debug, Clone, Copy)]
pub struct PartitionBuildParams {
    pub n_cx: u64,
    pub dim: usize,
    pub n_regions: usize,
    pub seed: u64,
    pub sample: usize,
    pub chunk: usize,
    pub m_max: usize,
    pub ef_construction: usize,
    pub region_build_parallelism: usize,
    pub final_assignment_probe: usize,
    pub final_assignment_cap: Option<usize>,
    /// Hierarchical balance split cap: regions larger than this are recursively
    /// re-clustered (SPANN hierarchical balanced clustering). Defaults to the
    /// initial-region mean (floored at MIN_REGION_CAP) when unset.
    pub balance_cap: Option<usize>,
    /// Closure-assignment boundary epsilon on the DISTANCE scale (SPANN eps1).
    pub assignment_boundary_epsilon: f32,
    /// Max regions a row may be stored in (SPANN closure replication limit).
    pub assignment_max_replication: usize,
    /// Apply SPANN's RNG rule to skip redundant boundary replicas.
    pub assignment_rng_rule: bool,
    /// SPTAG `RNGFactor` parity: skip a replica when
    /// `rng_factor * d²(c_sel, c_cand) < d²(x, c_cand)`. 1.0 is the strict
    /// SPANN paper rule; larger values keep more replicas (#1129).
    pub assignment_rng_factor: f32,
}

impl PartitionBuildParams {
    pub fn new(n_cx: u64, dim: usize, n_regions: usize, seed: u64) -> Self {
        Self {
            n_cx,
            dim,
            n_regions,
            seed,
            sample: (n_cx as usize).min(200_000),
            chunk: 100_000,
            m_max: 32,
            ef_construction: 96,
            region_build_parallelism: Self::default_region_build_parallelism(n_regions),
            final_assignment_probe: DEFAULT_FINAL_ASSIGNMENT_PROBE,
            final_assignment_cap: None,
            balance_cap: None,
            assignment_boundary_epsilon: FINAL_ASSIGNMENT_BOUNDARY_EPSILON,
            assignment_max_replication: FINAL_ASSIGNMENT_MAX_REPLICATION,
            assignment_rng_rule: true,
            assignment_rng_factor: FINAL_ASSIGNMENT_RNG_FACTOR,
        }
    }

    pub fn default_region_build_parallelism(n_regions: usize) -> usize {
        std::thread::available_parallelism()
            .map(|threads| threads.get())
            .unwrap_or(1)
            .min(n_regions.max(1))
            .max(1)
    }
}

fn effective_region_build_parallelism(requested: usize, region_count: usize) -> Result<usize> {
    if requested == 0 {
        return Err(crate::error::sextant_error(
            crate::error::CALYX_INDEX_INVALID_PARAMS,
            "region_build_parallelism must be > 0",
        ));
    }
    Ok(requested.min(region_count.max(1)).max(1))
}

fn graph_rel(region: u32) -> String {
    format!("idx/region_{region:05}.ann/graph.cda")
}
fn ids_rel(region: u32) -> String {
    format!("idx/region_{region:05}.ids")
}

pub fn build_partitioned_vault(
    root: &Path,
    p: PartitionBuildParams,
) -> Result<PartitionedManifest> {
    build_partitioned_vault_with_backend(root, p, DiskAnnBuildBackend::CpuVamana)
}

pub fn build_partitioned_vault_with_backend(
    root: &Path,
    p: PartitionBuildParams,
    backend: DiskAnnBuildBackend,
) -> Result<PartitionedManifest> {
    if p.n_cx == 0 || p.dim == 0 || p.n_regions == 0 || p.final_assignment_probe == 0 {
        return Err(crate::error::sextant_error(
            crate::error::CALYX_INDEX_INVALID_PARAMS,
            "partitioned vault requires nonzero n_cx, dim, n_regions, final_assignment_probe",
        ));
    }
    let source = SyntheticSource {
        seed: p.seed,
        dim: p.dim,
        n_cx: p.n_cx,
    };
    build_partitioned_vault_from_source_with_backend(root, &source, p, backend)
}

pub fn build_partitioned_vault_from_source(
    root: &Path,
    source: &dyn VectorSource,
    p: PartitionBuildParams,
) -> Result<PartitionedManifest> {
    build_partitioned_vault_from_source_with_backend(
        root,
        source,
        p,
        DiskAnnBuildBackend::CpuVamana,
    )
}

pub fn build_partitioned_vault_from_source_with_backend(
    root: &Path,
    source: &dyn VectorSource,
    p: PartitionBuildParams,
    backend: DiskAnnBuildBackend,
) -> Result<PartitionedManifest> {
    build_partitioned_vault_from_source_with_backend_and_metric(
        root,
        source,
        p,
        backend,
        PartitionDistanceMetric::UnitL2,
    )
}

pub fn build_partitioned_vault_from_source_with_backend_and_metric(
    root: &Path,
    source: &dyn VectorSource,
    p: PartitionBuildParams,
    backend: DiskAnnBuildBackend,
    distance_metric: PartitionDistanceMetric,
) -> Result<PartitionedManifest> {
    let dim = source.dim();
    let n_cx = source.len();
    if n_cx == 0 || dim == 0 || p.n_regions == 0 || p.final_assignment_probe == 0 {
        return Err(crate::error::sextant_error(
            crate::error::CALYX_INDEX_INVALID_PARAMS,
            "partitioned vault requires nonzero source len, dim, n_regions, final_assignment_probe",
        ));
    }
    if p.final_assignment_cap == Some(0) {
        return Err(crate::error::sextant_error(
            crate::error::CALYX_INDEX_INVALID_PARAMS,
            "final_assignment_cap must be > 0 when set",
        ));
    }
    if p.balance_cap == Some(0) {
        return Err(crate::error::sextant_error(
            crate::error::CALYX_INDEX_INVALID_PARAMS,
            "balance_cap must be > 0 when set",
        ));
    }
    if !p.assignment_boundary_epsilon.is_finite()
        || p.assignment_boundary_epsilon < 0.0
        || p.assignment_max_replication == 0
        || !p.assignment_rng_factor.is_finite()
        || p.assignment_rng_factor <= 0.0
    {
        return Err(crate::error::sextant_error(
            crate::error::CALYX_INDEX_INVALID_PARAMS,
            "assignment_boundary_epsilon must be finite and >= 0, assignment_max_replication >= 1, assignment_rng_factor finite and > 0",
        ));
    }
    if p.region_build_parallelism == 0 {
        return Err(crate::error::sextant_error(
            crate::error::CALYX_INDEX_INVALID_PARAMS,
            "region_build_parallelism must be > 0",
        ));
    }
    std::fs::create_dir_all(root.join(CENTROID_DIR))
        .map_err(|e| crate::error::sextant_error(crate::error::CALYX_INDEX_IO, e.to_string()))?;

    let sample = p.sample.min(n_cx as usize).max(1);
    let stride = (n_cx / sample as u64).max(1);
    let sample_rows: Vec<(u32, Vec<f32>)> = (0..sample)
        .into_par_iter()
        .map(|s| {
            let idx = (s as u64 * stride) % n_cx;
            (s as u32, source.row(idx))
        })
        .collect();
    let centroids = build_centroids(&sample_rows, p.n_regions, p.seed);
    let r = centroids.centroid_count();

    const ROUTED_ASSIGN_MIN_CENTROIDS: usize = 256;
    let use_routed_assign = r > ROUTED_ASSIGN_MIN_CENTROIDS;
    let provisional_routing = match distance_metric {
        PartitionDistanceMetric::RawL2 if use_routed_assign => AssignmentRouting::RawL2Graph,
        PartitionDistanceMetric::RawL2 => AssignmentRouting::Exact,
        PartitionDistanceMetric::UnitL2 if use_routed_assign => AssignmentRouting::Hnsw,
        PartitionDistanceMetric::UnitL2 => AssignmentRouting::Exact,
    };
    let provisional = stream_assign_to_ids_with_routing(
        root,
        AssignmentSink::Provisional,
        &centroids,
        source,
        p.chunk,
        provisional_routing,
    )?;

    let mean_region = (n_cx as usize).div_ceil(r.max(1));
    let cap = p
        .balance_cap
        .unwrap_or_else(|| mean_region.max(MIN_REGION_CAP));
    let final_centroids = balance_region_files(
        root,
        &centroids,
        &provisional,
        source,
        p.seed,
        cap,
        distance_metric,
    )?;
    let centroids =
        SpannCentroidIndex::from_parts(dim as u32, final_centroids, Vec::new(), Vec::new())?;
    centroids.save(root.join(CENTROID_DIR))?;

    let final_mean = (n_cx as usize).div_ceil(centroids.centroid_count().max(1));
    let final_cap = p
        .final_assignment_cap
        .unwrap_or_else(|| final_mean.saturating_mul(2).max(MIN_REGION_CAP));
    let use_final_routed_assign = centroids.centroid_count() > ROUTED_ASSIGN_MIN_CENTROIDS;
    let final_routing = match distance_metric {
        PartitionDistanceMetric::RawL2 if use_final_routed_assign => AssignmentRouting::RawL2Graph,
        PartitionDistanceMetric::RawL2 => AssignmentRouting::Exact,
        PartitionDistanceMetric::UnitL2 => AssignmentRouting::Hnsw,
    };
    let (region_ids, closure_stats) = stream_assign_to_ids_bounded(
        root,
        AssignmentSink::Final,
        &centroids,
        source,
        p.chunk,
        BoundedAssignmentConfig {
            cap: final_cap,
            routing_probe: p.final_assignment_probe,
            routing: final_routing,
            boundary_epsilon: p.assignment_boundary_epsilon,
            max_replication: p.assignment_max_replication,
            apply_rng_rule: p.assignment_rng_rule,
            rng_factor: p.assignment_rng_factor,
        },
    )?;
    let region_build_parallelism =
        effective_region_build_parallelism(p.region_build_parallelism, region_ids.len())?;

    // 3. Build one DiskANN graph per region (each fits RAM). Regions are built
    //    in a LOCAL, capped rayon pool (#706). The cap bounds the number of
    //    region row buffers that can exist at once and also contains nested
    //    DiskANN parallelism inside the same worker budget.
    let build_params = DiskAnnBuildParams {
        dim,
        m_max: p.m_max,
        ef_construction: p.ef_construction,
        alpha: 1.2,
    };
    let search_params = DiskAnnSearchParams {
        beamwidth: 64,
        ef_search: 64,
        rescore_k: 64,
        rescore_from_raw: false,
    };
    let pool = ThreadPoolBuilder::new()
        .num_threads(region_build_parallelism)
        .thread_name(|idx| format!("calyx-region-build-{idx}"))
        .build()
        .map_err(|e| {
            crate::error::sextant_error(
                crate::error::CALYX_INDEX_INVALID_PARAMS,
                format!("build region rayon pool: {e}"),
            )
        })?;
    let mut regions: Vec<RegionMeta> = pool.install(|| {
        region_ids
            .par_iter()
            .map(|meta| -> Result<RegionMeta> {
                let region = meta.id;
                let members = read_ids(&root.join(&meta.ids_rel))?;
                if members.len() != meta.count {
                    return Err(crate::error::sextant_error(
                        crate::error::CALYX_INDEX_CORRUPT,
                        format!(
                            "region {region} ids count {} != assignment count {}",
                            members.len(),
                            meta.count
                        ),
                    ));
                }
                let rows: Vec<(CxId, Vec<f32>)> = members
                    .iter()
                    .map(|&idx| (cx(idx), source.row(idx)))
                    .collect();
                let graph_path = root.join(graph_rel(region));
                build_partitioned_graph(
                    &graph_path,
                    &rows,
                    build_params,
                    search_params,
                    backend,
                    distance_metric,
                )?;
                Ok(RegionMeta {
                    id: region,
                    count: members.len(),
                    graph_rel: graph_rel(region),
                    ids_rel: meta.ids_rel.clone(),
                })
            })
            .collect::<Result<Vec<RegionMeta>>>()
    })?;
    // `par_iter().collect()` preserves input order, but make the on-disk manifest
    // order explicit and deterministic regardless of scheduling.
    regions.sort_by_key(|m| m.id);

    // 4. Root DiskANN graph over the region centroids (card's slot_00.ann + a
    //    second routing path). Tiny (R nodes).
    let centroid_rows: Vec<(CxId, Vec<f32>)> = centroids
        .centroids()
        .iter()
        .enumerate()
        .map(|(i, c)| (cx(i as u64), c.clone()))
        .collect();
    build_partitioned_graph(
        &root.join(ROOT_GRAPH),
        &centroid_rows,
        build_params,
        search_params,
        backend,
        distance_metric,
    )?;

    let manifest = PartitionedManifest {
        format: "calyx-partitioned-vault-v1".to_string(),
        n_cx,
        dim,
        n_regions: centroids.centroid_count(),
        seed: p.seed,
        m_max: p.m_max,
        ef_construction: p.ef_construction,
        distance_metric,
        region_build_parallelism,
        graph_build_backend: backend,
        provisional_assignment_routing: provisional_routing.as_str().to_string(),
        final_assignment_routing: final_routing.as_str().to_string(),
        final_assignment_probe: p.final_assignment_probe,
        final_assignment_cap: Some(final_cap),
        final_assignment_boundary_epsilon: p.assignment_boundary_epsilon,
        final_assignment_max_replication: p.assignment_max_replication,
        final_assignment_rng_rule: p.assignment_rng_rule,
        final_assignment_rng_factor: p.assignment_rng_factor,
        final_assignment_closure: Some(closure_stats),
        region_balance_cap: cap,
        stored_region_members: regions.iter().map(|region| region.count).sum(),
        centroids_rel: format!("{CENTROID_DIR}/centroids.spn"),
        root_graph_rel: ROOT_GRAPH.to_string(),
        regions,
    };
    manifest::write_manifest_db(root, &manifest)?;
    Ok(manifest)
}

pub fn partitioned_manifest_db_exists(root: &Path) -> Result<bool> {
    manifest::manifest_db_exists(root)
}

pub fn partitioned_manifest_db_readback(root: &Path) -> Result<PartitionedManifestDbReadback> {
    manifest::read_manifest_db_readback(root)
}

fn build_partitioned_graph(
    graph_path: &Path,
    rows: &[(CxId, Vec<f32>)],
    build_params: DiskAnnBuildParams,
    search_params: DiskAnnSearchParams,
    backend: DiskAnnBuildBackend,
    distance_metric: PartitionDistanceMetric,
) -> Result<()> {
    match distance_metric {
        PartitionDistanceMetric::UnitL2 => {
            DiskAnnSearch::build_without_default_raw_sidecar_with_backend(
                SlotId::new(0),
                graph_path,
                rows,
                build_params,
                None,
                search_params,
                backend,
            )?;
        }
        PartitionDistanceMetric::RawL2 => {
            DiskAnnSearch::build_raw_l2_without_default_raw_sidecar_with_backend(
                SlotId::new(0),
                graph_path,
                rows,
                build_params,
                None,
                search_params,
                backend,
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
