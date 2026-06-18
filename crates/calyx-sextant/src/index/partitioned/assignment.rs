use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::Path;

use calyx_core::Result;
use rayon::prelude::*;

use crate::error::{
    CALYX_INDEX_CORRUPT, CALYX_INDEX_INVALID_PARAMS, CALYX_INDEX_IO, sextant_error,
};
use crate::index::SpannCentroidIndex;

use super::{VectorSource, ids_rel};

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
                };
                (idx, region)
            })
            .collect();
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
) -> Result<Vec<AssignmentRegion>> {
    let r = centroids.centroid_count();
    let n = source.len();
    if config.cap == 0 || config.routing_probe == 0 {
        return Err(sextant_error(
            CALYX_INDEX_INVALID_PARAMS,
            "bounded assignment requires cap > 0 and routing_probe > 0",
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
    let mut counts = vec![0usize; r];
    clear_stale_ids(root, sink, r)?;
    let mut start = 0u64;
    while start < n {
        let end = (start + chunk).min(n);
        let rayon_assigned: Vec<(u64, Vec<u32>)> = (start..end)
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
                (idx, candidates)
            })
            .collect();
        let mut assigned = Vec::with_capacity(rayon_assigned.len());
        for (idx, candidates) in rayon_assigned {
            let region = choose_bounded_region(&counts, config.cap, &candidates).ok_or_else(|| {
                sextant_error(
                    CALYX_INDEX_INVALID_PARAMS,
                    format!(
                        "bounded assignment exhausted the top {probe} routed regions for row {idx}; increase regions or cap"
                    ),
                )
            })?;
            counts[region] += 1;
            assigned.push((idx, region as u32));
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

fn choose_bounded_region(counts: &[usize], cap: usize, candidates: &[u32]) -> Option<usize> {
    candidates.iter().find_map(|&region| {
        let region = region as usize;
        (region < counts.len() && counts[region] < cap).then_some(region)
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
