use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::Path;

use calyx_core::Result;
use rayon::prelude::*;

use crate::error::{
    CALYX_INDEX_CORRUPT, CALYX_INDEX_INVALID_PARAMS, CALYX_INDEX_IO, sextant_error,
};
use crate::index::SpannCentroidIndex;
use crate::index::distance::l2_sq;

use super::{ClosureAssignmentStats, VectorSource, ids_rel};

#[derive(Debug, Clone)]
pub(super) struct AssignmentRegion {
    pub id: u32,
    pub count: usize,
    pub ids_rel: String,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum AssignmentRouting {
    Exact,
    Hnsw,
    RawL2Graph,
}

impl AssignmentRouting {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact-l2",
            Self::Hnsw => "hnsw",
            Self::RawL2Graph => "raw-l2-graph",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum AssignmentSink {
    Final,
    Provisional,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct BoundedAssignmentConfig {
    pub cap: usize,
    pub routing_probe: usize,
    pub routing: AssignmentRouting,
    pub boundary_epsilon: f32,
    pub max_replication: usize,
    /// SPANN RNG rule (arXiv:2111.08566 §4.1): skip a replica region when an
    /// already-selected region's centroid is closer to that region's centroid
    /// than the row is — the replica would duplicate boundary coverage the
    /// selected region already provides.
    pub apply_rng_rule: bool,
    /// SPTAG `RNGFactor` parity (#1129): skip when
    /// `rng_factor * d²(c_sel, c_cand) < d²(x, c_cand)`. 1.0 is the strict
    /// paper rule; larger values keep more replicas. At coarse Calyx
    /// geometries the point-to-centroid distance exceeds the inter-centroid
    /// spacing for ~all rows, so 1.0 prunes essentially every replica.
    pub rng_factor: f32,
}

pub(super) fn stream_assign_to_ids_with_routing(
    root: &Path,
    sink: AssignmentSink,
    centroids: &SpannCentroidIndex,
    source: &dyn VectorSource,
    chunk: usize,
    routing: AssignmentRouting,
) -> Result<Vec<AssignmentRegion>> {
    let r = centroids.centroid_count();
    let n = source.len();
    let chunk = chunk.max(1) as u64;
    let mut counts = vec![0usize; r];
    clear_stale_ids(root, sink, r)?;
    let mut start = 0u64;
    while start < n {
        let end = (start + chunk).min(n);
        let mut assigned: Vec<(u64, u32)> = (start..end)
            .into_par_iter()
            .map(|idx| {
                let row = source.row(idx);
                let region = match routing {
                    AssignmentRouting::Exact => centroids.assign(&row),
                    AssignmentRouting::Hnsw => centroids.assign_hnsw(&row),
                    AssignmentRouting::RawL2Graph => centroids.assign_raw_l2_graph(&row),
                }?;
                Ok((idx, region))
            })
            .collect::<Result<_>>()?;
        for &(idx, region) in &assigned {
            let region = region as usize;
            if region >= counts.len() {
                return Err(sextant_error(
                    CALYX_INDEX_CORRUPT,
                    format!(
                        "assignment returned region {region} >= centroid count {r} for row {idx}"
                    ),
                ));
            }
            counts[region] += 1;
        }
        append_assigned_chunk(root, sink, &mut assigned)?;
        start = end;
    }
    Ok(counts
        .into_iter()
        .enumerate()
        .filter(|(_, count)| *count > 0)
        .map(|(region, count)| AssignmentRegion {
            id: region as u32,
            count,
            ids_rel: assignment_ids_rel(sink, region as u32),
        })
        .collect())
}

pub(super) fn stream_assign_to_ids_bounded(
    root: &Path,
    sink: AssignmentSink,
    centroids: &SpannCentroidIndex,
    source: &dyn VectorSource,
    chunk: usize,
    config: BoundedAssignmentConfig,
) -> Result<(Vec<AssignmentRegion>, ClosureAssignmentStats)> {
    let r = centroids.centroid_count();
    let n = source.len();
    if config.cap == 0
        || config.routing_probe == 0
        || config.max_replication == 0
        || !config.boundary_epsilon.is_finite()
        || config.boundary_epsilon < 0.0
        || !config.rng_factor.is_finite()
        || config.rng_factor <= 0.0
    {
        return Err(sextant_error(
            CALYX_INDEX_INVALID_PARAMS,
            "bounded assignment requires cap > 0, routing_probe > 0, max_replication > 0, finite nonnegative boundary_epsilon, and finite positive rng_factor",
        ));
    }
    let total_capacity = (config.cap as u128) * (r as u128);
    if total_capacity < n as u128 {
        return Err(sextant_error(
            CALYX_INDEX_INVALID_PARAMS,
            format!("bounded assignment capacity {total_capacity} < n_cx {n}"),
        ));
    }
    let probe = config.routing_probe.min(r);
    let chunk = chunk.max(1) as u64;
    let mut primary_counts = vec![0usize; r];
    let mut stored_counts = vec![0usize; r];
    let mut duplicate_budget = usize::try_from(total_capacity - n as u128).unwrap_or(usize::MAX);
    let mut stats = ClosureAssignmentStats::default();
    clear_stale_ids(root, sink, r)?;
    let mut start = 0u64;
    while start < n {
        let end = (start + chunk).min(n);
        let rayon_assigned: Vec<(u64, Vec<(usize, f32)>)> = (start..end)
            .into_par_iter()
            .map(|idx| {
                let row = source.row(idx);
                let candidates = match config.routing {
                    AssignmentRouting::Exact => centroids.nearest_centroids_exact_l2(&row, probe),
                    AssignmentRouting::Hnsw => centroids.nearest_centroids(&row, probe),
                    AssignmentRouting::RawL2Graph => {
                        centroids.nearest_centroids_raw_l2_graph(&row, probe)
                    }
                };
                (idx, score_candidates(centroids, &row, &candidates))
            })
            .collect();
        let mut assigned = Vec::with_capacity(rayon_assigned.len());
        for (idx, candidates) in rayon_assigned {
            let regions = choose_bounded_regions(
                &primary_counts,
                &stored_counts,
                config.cap,
                &candidates,
                config.boundary_epsilon,
                config.max_replication,
                duplicate_budget,
                config.apply_rng_rule.then_some((centroids, config.rng_factor)),
                &mut stats,
            )
            .ok_or_else(|| {
                sextant_error(
                    CALYX_INDEX_INVALID_PARAMS,
                    format!(
                        "bounded assignment exhausted the top {probe} routed regions for row {idx}; increase regions or cap"
                    ),
                )
            })?;
            for (pos, region) in regions.into_iter().enumerate() {
                if pos == 0 {
                    primary_counts[region] += 1;
                } else {
                    duplicate_budget = duplicate_budget.saturating_sub(1);
                }
                stored_counts[region] += 1;
                assigned.push((idx, region as u32));
            }
        }
        append_assigned_chunk(root, sink, &mut assigned)?;
        start = end;
    }
    let regions = stored_counts
        .into_iter()
        .enumerate()
        .filter(|(_, count)| *count > 0)
        .map(|(region, count)| AssignmentRegion {
            id: region as u32,
            count,
            ids_rel: assignment_ids_rel(sink, region as u32),
        })
        .collect();
    Ok((regions, stats))
}

fn score_candidates(
    centroids: &SpannCentroidIndex,
    row: &[f32],
    candidates: &[u32],
) -> Vec<(usize, f32)> {
    let mut scored = Vec::with_capacity(candidates.len());
    for &candidate in candidates {
        let region = candidate as usize;
        let Some(centroid) = centroids.centroids().get(region) else {
            continue;
        };
        if !scored.iter().any(|(seen, _)| *seen == region) {
            scored.push((region, l2_sq(centroid, row)));
        }
    }
    scored.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    scored
}

#[allow(clippy::too_many_arguments)]
fn choose_bounded_regions(
    primary_counts: &[usize],
    stored_counts: &[usize],
    cap: usize,
    candidates: &[(usize, f32)],
    boundary_epsilon: f32,
    max_replication: usize,
    duplicate_budget: usize,
    rng_rule: Option<(&SpannCentroidIndex, f32)>,
    stats: &mut ClosureAssignmentStats,
) -> Option<Vec<usize>> {
    let &(primary, primary_distance) = candidates.iter().find(|(region, _)| {
        primary_counts
            .get(*region)
            .is_some_and(|count| *count < cap)
            && stored_counts.get(*region).is_some_and(|count| *count < cap)
    })?;
    // Candidates carry SQUARED distances; epsilon is defined on the distance
    // scale (SPANN's closure rule), so the squared threshold is (1 + eps)^2.
    let factor = (1.0 + boundary_epsilon) * (1.0 + boundary_epsilon);
    let threshold = primary_distance * factor;
    let duplicate_cap = cap.saturating_mul(max_replication.saturating_sub(1));
    let mut selected = vec![primary];
    for &(region, distance) in candidates {
        if selected.len() >= max_replication {
            break;
        }
        if selected.len() > duplicate_budget {
            stats.budget_stopped_rows += 1;
            break;
        }
        if region == primary {
            continue;
        }
        if distance > threshold {
            stats.epsilon_filtered += 1;
            continue;
        }
        let duplicates = stored_counts[region].saturating_sub(primary_counts[region]);
        if stored_counts[region] >= cap || duplicates >= duplicate_cap {
            stats.cap_skipped += 1;
            continue;
        }
        if rng_rule_skips(rng_rule, &selected, region, distance) {
            stats.rng_skipped += 1;
            continue;
        }
        selected.push(region);
    }
    stats.rows += 1;
    stats.replicas_stored += selected.len() as u64 - 1;
    let bucket = selected.len() - 1;
    if stats.replica_histogram.len() <= bucket {
        stats.replica_histogram.resize(bucket + 1, 0);
    }
    stats.replica_histogram[bucket] += 1;
    Some(selected)
}

/// SPANN RNG rule: replica candidate `region` (squared distance `distance_sq`
/// from the row) is redundant when some already-selected region's centroid is
/// closer to the candidate's centroid than the row itself is. `rng_factor`
/// relaxes the comparison on the squared scale (SPTAG `RNGFactor` parity).
fn rng_rule_skips(
    rng_rule: Option<(&SpannCentroidIndex, f32)>,
    selected: &[usize],
    region: usize,
    distance_sq: f32,
) -> bool {
    let Some((index, rng_factor)) = rng_rule else {
        return false;
    };
    let all = index.centroids();
    let Some(candidate) = all.get(region) else {
        return false;
    };
    selected.iter().any(|&chosen| {
        all.get(chosen)
            .is_some_and(|centroid| rng_factor * l2_sq(centroid, candidate) < distance_sq)
    })
}

fn append_assigned_chunk(
    root: &Path,
    sink: AssignmentSink,
    assigned: &mut [(u64, u32)],
) -> Result<()> {
    assigned.sort_by_key(|(_, region)| *region);
    let mut offset = 0usize;
    while offset < assigned.len() {
        let region = assigned[offset].1;
        let start = offset;
        while offset < assigned.len() && assigned[offset].1 == region {
            offset += 1;
        }
        append_region_ids(root, sink, region, &assigned[start..offset])?;
    }
    Ok(())
}

fn append_region_ids(
    root: &Path,
    sink: AssignmentSink,
    region: u32,
    assigned: &[(u64, u32)],
) -> Result<()> {
    let path = root.join(assignment_ids_rel(sink, region));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| sextant_error(CALYX_INDEX_IO, format!("create ids dir: {e}")))?;
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| {
            sextant_error(
                CALYX_INDEX_IO,
                format!("open ids {} for append: {e}", path.display()),
            )
        })?;
    let mut writer = BufWriter::new(file);
    for &(idx, _) in assigned {
        writer.write_all(&idx.to_le_bytes()).map_err(|e| {
            sextant_error(
                CALYX_INDEX_IO,
                format!("write region {region} id {idx}: {e}"),
            )
        })?;
    }
    writer
        .flush()
        .map_err(|e| sextant_error(CALYX_INDEX_IO, format!("flush ids {}: {e}", path.display())))
}

fn clear_stale_ids(root: &Path, sink: AssignmentSink, regions: usize) -> Result<()> {
    for region in 0..regions {
        let path = root.join(assignment_ids_rel(sink, region as u32));
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(sextant_error(
                    CALYX_INDEX_IO,
                    format!("remove stale ids {}: {e}", path.display()),
                ));
            }
        }
    }
    Ok(())
}

fn assignment_ids_rel(sink: AssignmentSink, region: u32) -> String {
    match sink {
        AssignmentSink::Final => ids_rel(region),
        AssignmentSink::Provisional => format!("idx/assign-initial/region_{region:05}.ids"),
    }
}

pub(super) fn read_ids(path: &Path) -> Result<Vec<u64>> {
    let bytes = std::fs::read(path)
        .map_err(|e| sextant_error(CALYX_INDEX_IO, format!("read ids {}: {e}", path.display())))?;
    if bytes.len() % 8 != 0 {
        return Err(sextant_error(
            CALYX_INDEX_CORRUPT,
            format!(
                "ids {} len {} is not multiple of 8",
                path.display(),
                bytes.len()
            ),
        ));
    }
    Ok(bytes
        .chunks_exact(8)
        .map(|c| u64::from_le_bytes(c.try_into().expect("8 bytes")))
        .collect())
}
