use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use calyx_core::{CxId, Result};

use super::DiskAnnSearchParams;
use crate::error::{
    CALYX_INDEX_DIM_MISMATCH, CALYX_INDEX_INVALID_PARAMS, CALYX_INDEX_IO, sextant_error,
};
use crate::index::diskann::graph::{DiskAnnGraphReader, DiskAnnVectorRef, open_diskann_graph};
use crate::index::distance::{cosine_distance, l2_sq, unit_l2_cosine_distance};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DiskAnnDistanceMode {
    RawCosine,
    UnitL2,
    RawL2,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Candidate {
    pub(super) id: u32,
    pub(super) distance: f32,
}

impl Candidate {
    pub(super) fn new(id: u32, distance: f32) -> Self {
        Self { id, distance }
    }
}

impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.distance.to_bits() == other.distance.to_bits()
    }
}

impl Eq for Candidate {}

impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.distance
            .total_cmp(&other.distance)
            .then_with(|| self.id.cmp(&other.id))
    }
}

impl DiskAnnSearchParams {
    pub(super) fn validate(&self) -> Result<()> {
        if self.beamwidth == 0 || self.ef_search == 0 || self.rescore_k == 0 {
            return Err(invalid(
                "beamwidth, ef_search, and rescore_k must be positive",
            ));
        }
        Ok(())
    }
}

pub(super) fn distance(a: &[f32], b: &[f32], mode: DiskAnnDistanceMode) -> f32 {
    match mode {
        DiskAnnDistanceMode::RawCosine => cosine_distance(a, b),
        DiskAnnDistanceMode::UnitL2 => unit_l2_cosine_distance(a, b),
        DiskAnnDistanceMode::RawL2 => l2_sq(a, b),
    }
}

pub(super) fn distance_to_node(
    a: &[f32],
    b: DiskAnnVectorRef<'_>,
    mode: DiskAnnDistanceMode,
) -> f32 {
    match b {
        DiskAnnVectorRef::F32(values) => distance(a, values, mode),
        DiskAnnVectorRef::I8(values) => match mode {
            DiskAnnDistanceMode::RawL2 => l2_sq_i8(a, values),
            DiskAnnDistanceMode::RawCosine | DiskAnnDistanceMode::UnitL2 => cosine_i8(a, values),
        },
    }
}

fn cosine_i8(a: &[f32], b: &[i8]) -> f32 {
    let len = a.len().min(b.len());
    let mut dot = 0.0;
    let mut an = 0.0;
    let mut bn = 0.0;
    for i in 0..len {
        let x = a[i];
        let y = f32::from(b[i]);
        dot += x * y;
        an += x * x;
        bn += y * y;
    }
    if an == 0.0 || bn == 0.0 {
        1.0
    } else {
        (1.0 - dot / (an.sqrt() * bn.sqrt())).max(0.0)
    }
}

fn l2_sq_i8(a: &[f32], b: &[i8]) -> f32 {
    let len = a.len().min(b.len());
    let mut sum = 0.0;
    for i in 0..len {
        let d = a[i] - f32::from(b[i]);
        sum += d * d;
    }
    sum
}

pub(super) fn sorted(mut hits: Vec<(u32, f32)>) -> Vec<(u32, f32)> {
    hits.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    hits
}

pub(super) fn dense_rows(rows: &[(CxId, Vec<f32>)], dim: usize) -> Result<Vec<(u32, Vec<f32>)>> {
    rows.iter()
        .enumerate()
        .map(|(idx, (_, vector))| {
            if vector.len() != dim {
                return Err(sextant_error(
                    CALYX_INDEX_DIM_MISMATCH,
                    format!("vector {idx} dim {} expected {dim}", vector.len()),
                ));
            }
            let id = u32::try_from(idx)
                .map_err(|_| invalid("diskann graph exceeds u32 node id space"))?;
            Ok((id, vector.clone()))
        })
        .collect()
}

pub(super) fn positions(ids: &[CxId]) -> HashMap<CxId, u32> {
    ids.iter()
        .enumerate()
        .filter_map(|(idx, cx_id)| u32::try_from(idx).ok().map(|id| (*cx_id, id)))
        .collect()
}

pub(super) fn open_for_search(path: &Path) -> Result<DiskAnnGraphReader> {
    open_diskann_graph(path)
}

pub(super) fn invalid(detail: impl std::fmt::Display) -> calyx_core::CalyxError {
    sextant_error(
        CALYX_INDEX_INVALID_PARAMS,
        format!("diskann search invalid params: {detail}"),
    )
}

pub(super) fn io(stage: &str, error: std::io::Error) -> calyx_core::CalyxError {
    sextant_error(CALYX_INDEX_IO, format!("diskann search {stage}: {error}"))
}

#[cfg(unix)]
pub(super) fn prefetch_node(file: &File, offset: u64, len: usize) {
    use std::os::fd::AsRawFd;

    const POSIX_FADV_WILLNEED: i32 = 3;
    unsafe extern "C" {
        fn posix_fadvise(fd: i32, offset: i64, len: i64, advice: i32) -> i32;
    }
    let _ = unsafe {
        posix_fadvise(
            file.as_raw_fd(),
            offset as i64,
            len as i64,
            POSIX_FADV_WILLNEED,
        )
    };
}

#[cfg(not(unix))]
pub(super) fn prefetch_node(_file: &File, _offset: u64, _len: usize) {}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::error::CALYX_INDEX_CORRUPT;
    use crate::index::diskann::graph::{DISKANN_FORMAT_VERSION, DiskAnnGraphWriter, DiskAnnHeader};

    #[test]
    fn open_for_search_preserves_corrupt_truncated_graph_code() {
        let root = temp_root("diskann-search-truncated");
        let path = root.join("graph.cda");
        let header = DiskAnnHeader {
            format_version: DISKANN_FORMAT_VERSION,
            dim: 2,
            m_max: 1,
            max_degree: 0,
            entry_point_id: 0,
            node_count: 1,
        };
        let mut writer = DiskAnnGraphWriter::create(&path, header).unwrap();
        writer.write_node(0, &[1.0, 0.0], &[]).unwrap();
        writer.finish().unwrap();
        let len = fs::metadata(&path).unwrap().len();
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(len - 1)
            .unwrap();

        let error = open_for_search(&path).unwrap_err();

        assert_eq!(error.code, CALYX_INDEX_CORRUPT);
        assert!(error.message.contains("file len"));
        assert!(error.message.contains("expected"));
        let _ = fs::remove_dir_all(root);
    }

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
