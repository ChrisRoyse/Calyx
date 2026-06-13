use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::path::Path;

use calyx_core::{CxId, Result};

use super::DiskAnnSearchParams;
use crate::error::{
    CALYX_INDEX_DIM_MISMATCH, CALYX_INDEX_INVALID_PARAMS, CALYX_INDEX_IO, sextant_error,
};
use crate::index::diskann::graph::{DiskAnnGraphReader, open_diskann_graph};
use crate::util::cosine;

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

pub(super) fn distance(a: &[f32], b: &[f32]) -> f32 {
    (1.0 - cosine(a, b)).max(0.0)
}

pub(super) fn sorted(mut hits: Vec<(u32, f32)>) -> Vec<(u32, f32)> {
    hits.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    hits
}

pub(super) fn stop_search(
    candidates: &[Candidate],
    scores: &BTreeMap<u32, f32>,
    expanded: &BTreeSet<u32>,
    want: usize,
) -> bool {
    if scores.len() < want {
        return false;
    }
    let worst = sorted(scores.iter().map(|(&id, &d)| (id, d)).collect())[want - 1].1;
    candidates
        .iter()
        .filter(|candidate| !expanded.contains(&candidate.id))
        .map(|candidate| candidate.distance)
        .min_by(f32::total_cmp)
        .is_none_or(|best| best >= worst)
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
    open_diskann_graph(path).map_err(|err| {
        if err.code == crate::error::CALYX_INDEX_CORRUPT
            && err.message.contains("file len")
            && err.message.contains("expected")
        {
            sextant_error(CALYX_INDEX_IO, err.message)
        } else {
            err
        }
    })
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
