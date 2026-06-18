use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use calyx_core::Result;
use rayon::prelude::*;

use crate::error::{CALYX_INDEX_CORRUPT, CALYX_INDEX_IO, sextant_error};
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
}

#[derive(Debug, Clone, Copy)]
pub(super) enum AssignmentSink {
    Final,
    Provisional,
}

/// Stream assignment into compact per-region id files. The final region->cx mapping
/// is the on-disk source of truth; graph build reads it back instead of retaining
/// `Vec<Vec<u64>>` buckets in heap.
pub(super) fn stream_assign_to_ids(
    root: &Path,
    sink: AssignmentSink,
    centroids: &SpannCentroidIndex,
    source: &dyn VectorSource,
    chunk: usize,
) -> Result<Vec<AssignmentRegion>> {
    stream_assign_to_ids_with_routing(
        root,
        sink,
        centroids,
        source,
        chunk,
        AssignmentRouting::Hnsw,
    )
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
    let mut writers: Vec<Option<BufWriter<File>>> = (0..r).map(|_| None).collect();
    clear_stale_ids(root, sink, r)?;
    let mut start = 0u64;
    while start < n {
        let end = (start + chunk).min(n);
        let assigned: Vec<(u64, u32)> = (start..end)
            .into_par_iter()
            .map(|idx| {
                let row = source.row(idx);
                let region = match routing {
                    AssignmentRouting::Exact => centroids.assign(&row),
                    AssignmentRouting::Hnsw => centroids.assign_hnsw(&row),
                };
                (idx, region)
            })
            .collect();
        for (idx, region) in assigned {
            let region = region as usize;
            let writer = writer_for_region(root, sink, region as u32, &mut writers[region])?;
            writer.write_all(&idx.to_le_bytes()).map_err(|e| {
                sextant_error(
                    CALYX_INDEX_IO,
                    format!("write region {region} id {idx}: {e}"),
                )
            })?;
            counts[region] += 1;
        }
        start = end;
    }
    for writer in writers.iter_mut().flatten() {
        writer
            .flush()
            .map_err(|e| sextant_error(CALYX_INDEX_IO, format!("flush ids: {e}")))?;
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

fn writer_for_region<'a>(
    root: &Path,
    sink: AssignmentSink,
    region: u32,
    slot: &'a mut Option<BufWriter<File>>,
) -> Result<&'a mut BufWriter<File>> {
    if slot.is_none() {
        let path = root.join(assignment_ids_rel(sink, region));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| sextant_error(CALYX_INDEX_IO, format!("create ids dir: {e}")))?;
        }
        let file = File::create(&path).map_err(|e| {
            sextant_error(
                CALYX_INDEX_IO,
                format!("create ids {}: {e}", path.display()),
            )
        })?;
        *slot = Some(BufWriter::new(file));
    }
    Ok(slot.as_mut().expect("writer initialized"))
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
