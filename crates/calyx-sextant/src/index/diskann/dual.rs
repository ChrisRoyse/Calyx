//! Dual-DiskANN search for asymmetric server-scale slots.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use calyx_core::{CxId, Result, SlotId, SlotShape, SlotVector};

use super::graph::open_diskann_graph;
use super::{DiskAnnBuildParams, DiskAnnSearch, DiskAnnSearchParams, build_diskann_graph};
use crate::error::{
    CALYX_INDEX_CORRUPT, CALYX_INDEX_DIM_MISMATCH, CALYX_INDEX_DIRECTION_UNAVAILABLE,
    CALYX_INDEX_INVALID_PARAMS, CALYX_INDEX_IO, sextant_error,
};
use crate::index::{IndexSearchHit, IndexStats, SextantIndex, ranked};
use crate::util::dense;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Reverse,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectionalBoost {
    pub forward_weight: f32,
    pub reverse_weight: f32,
}

impl Default for DirectionalBoost {
    fn default() -> Self {
        Self {
            forward_weight: 0.5,
            reverse_weight: 0.5,
        }
    }
}

impl DirectionalBoost {
    pub fn new(forward_weight: f32, reverse_weight: f32) -> Result<Self> {
        let boost = Self {
            forward_weight,
            reverse_weight,
        };
        boost.validate()?;
        Ok(boost)
    }

    fn validate(&self) -> Result<()> {
        let sum = self.forward_weight + self.reverse_weight;
        if !self.forward_weight.is_finite()
            || !self.reverse_weight.is_finite()
            || self.forward_weight < 0.0
            || self.reverse_weight < 0.0
            || (sum - 1.0).abs() > 1.0e-6
        {
            return Err(invalid(format!(
                "directional boost must be finite, non-negative, and sum to 1.0; got {} + {}",
                self.forward_weight, self.reverse_weight
            )));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct DualDiskAnnSearch {
    slot: SlotId,
    ids: Vec<u32>,
    cx_ids: Vec<CxId>,
    search_a: DiskAnnSearch,
    search_b: DiskAnnSearch,
    default_search: DiskAnnSearchParams,
    degraded: bool,
}

impl DualDiskAnnSearch {
    pub fn open_dual(vault_path: &Path, slot_id: u8, params: DiskAnnSearchParams) -> Result<Self> {
        let slot = SlotId::new(u16::from(slot_id));
        let a_path = dual_graph_path(vault_path, slot_id, Direction::Forward);
        let b_path = dual_graph_path(vault_path, slot_id, Direction::Reverse);
        let a_count = open_count(&a_path).map_err(open_err)?;
        let b_count = open_count(&b_path).map_err(open_err)?;
        if a_count != b_count {
            return Err(invalid(format!(
                "asymmetric graph node counts differ: forward={a_count} reverse={b_count}"
            )));
        }
        let ids: Vec<u32> = (0..a_count)
            .map(|id| u32::try_from(id).map_err(|_| invalid("dual graph exceeds u32 id space")))
            .collect::<Result<_>>()?;
        let cx_ids: Vec<_> = ids.iter().copied().map(cx_from_local).collect();
        Ok(Self {
            slot,
            ids,
            cx_ids: cx_ids.clone(),
            search_a: DiskAnnSearch::open(slot, a_path, cx_ids.clone(), None, params)
                .map_err(open_err)?,
            search_b: DiskAnnSearch::open(slot, b_path, cx_ids, None, params).map_err(open_err)?,
            default_search: params,
            degraded: false,
        })
    }

    pub fn empty(vault_path: &Path, slot_id: u8, dim: u32, params: DiskAnnSearchParams) -> Self {
        let slot = SlotId::new(u16::from(slot_id));
        Self {
            slot,
            ids: Vec::new(),
            cx_ids: Vec::new(),
            search_a: DiskAnnSearch::empty(
                slot,
                dim,
                dual_graph_path(vault_path, slot_id, Direction::Forward),
            ),
            search_b: DiskAnnSearch::empty(
                slot,
                dim,
                dual_graph_path(vault_path, slot_id, Direction::Reverse),
            ),
            default_search: params,
            degraded: false,
        }
    }

    pub fn is_degraded(&self) -> bool {
        self.degraded
    }

    pub fn search_directional(
        &self,
        query: &[f32],
        direction: Direction,
        k: usize,
    ) -> Result<Vec<(u32, f32)>> {
        let search = self.search_for(direction);
        self.validate_current_graph(direction)?;
        let params = self.params_for(k);
        let mut hits = search
            .search_ids(query, k, &params)?
            .into_iter()
            .map(|(node_id, dist)| (self.ids[node_id as usize], 1.0 - dist))
            .collect::<Vec<_>>();
        sort_scores_desc(&mut hits);
        Ok(hits)
    }

    pub fn search_merged(
        &self,
        query: &[f32],
        k: usize,
        boost: DirectionalBoost,
    ) -> Result<Vec<(u32, f32)>> {
        boost.validate()?;
        if k == 0 || self.ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut merged = BTreeMap::<u32, f32>::new();
        for (direction, weight) in [
            (Direction::Forward, boost.forward_weight),
            (Direction::Reverse, boost.reverse_weight),
        ] {
            for (id, score) in self.search_directional(query, direction, k)? {
                let weighted = score * weight;
                merged
                    .entry(id)
                    .and_modify(|old| *old = old.max(weighted))
                    .or_insert(weighted);
            }
        }
        let mut out: Vec<_> = merged.into_iter().collect();
        sort_scores_desc(&mut out);
        out.truncate(k);
        Ok(out)
    }

    fn search_for(&self, direction: Direction) -> &DiskAnnSearch {
        match direction {
            Direction::Forward => &self.search_a,
            Direction::Reverse => &self.search_b,
        }
    }

    fn params_for(&self, k: usize) -> DiskAnnSearchParams {
        let want = k.min(self.ids.len()).max(1);
        DiskAnnSearchParams {
            ef_search: self.default_search.ef_search.max(want),
            rescore_k: self.default_search.rescore_k.max(want),
            ..self.default_search
        }
    }

    fn validate_current_graph(&self, direction: Direction) -> Result<()> {
        if self.ids.is_empty() {
            return Ok(());
        }
        let path = self.search_for(direction).persist_path();
        if !path.is_file() {
            return Err(sextant_error(
                CALYX_INDEX_DIRECTION_UNAVAILABLE,
                format!(
                    "dual diskann {direction:?} graph missing at {}",
                    path.display()
                ),
            ));
        }
        open_diskann_graph(path).map(|_| ())
    }
}

impl SextantIndex for DualDiskAnnSearch {
    fn slot(&self) -> SlotId {
        self.slot
    }

    fn shape(&self) -> SlotShape {
        self.search_a.shape()
    }

    fn insert(&mut self, cx_id: CxId, vector: SlotVector, seq: u64) -> Result<()> {
        let values = dense(&vector)?;
        if values.len() != self.dim()? as usize {
            return Err(sextant_error(
                CALYX_INDEX_DIM_MISMATCH,
                format!("query dim {} expected {}", values.len(), self.dim()?),
            ));
        }
        self.search_a.insert(cx_id, vector.clone(), seq)?;
        if let Err(err) = self.search_b.insert(cx_id, vector, seq) {
            self.degraded = true;
            return Err(err);
        }
        if !self.cx_ids.contains(&cx_id) {
            let local = u32::try_from(self.ids.len())
                .map_err(|_| invalid("dual diskann local id exceeds u32"))?;
            self.ids.push(local);
            self.cx_ids.push(cx_id);
        }
        Ok(())
    }

    fn search(
        &self,
        query: &SlotVector,
        k: usize,
        _ef: Option<usize>,
    ) -> Result<Vec<IndexSearchHit>> {
        let query = dense(query)?;
        let scored = self
            .search_merged(query, k, DirectionalBoost::default())?
            .into_iter()
            .map(|(id, score)| (self.cx_ids[id as usize], score))
            .collect();
        Ok(ranked(scored))
    }

    fn rebuild(&mut self) -> Result<()> {
        self.search_a.rebuild()?;
        self.search_b.rebuild()?;
        self.degraded = false;
        Ok(())
    }

    fn vector(&self, cx_id: CxId) -> Option<SlotVector> {
        self.search_a
            .vector(cx_id)
            .or_else(|| self.search_b.vector(cx_id))
    }

    fn set_base_seq(&mut self, seq: u64) {
        self.search_a.set_base_seq(seq);
        self.search_b.set_base_seq(seq);
    }

    fn stats(&self) -> IndexStats {
        let mut stats = self.search_a.stats();
        stats.kind = "DualDiskANN";
        stats.len = self.ids.len();
        stats
    }
}

impl DualDiskAnnSearch {
    fn dim(&self) -> Result<u32> {
        match self.shape() {
            SlotShape::Dense(dim) => Ok(dim),
            _ => Err(invalid("dual diskann requires dense slot shape")),
        }
    }
}

pub fn open_dual(
    vault_path: &Path,
    slot_id: u8,
    params: DiskAnnSearchParams,
) -> Result<DualDiskAnnSearch> {
    DualDiskAnnSearch::open_dual(vault_path, slot_id, params)
}

pub fn build_dual(
    vault_path: &Path,
    slot_id: u8,
    a_vectors: &[(u32, Vec<f32>)],
    b_vectors: &[(u32, Vec<f32>)],
    params: DiskAnnBuildParams,
) -> Result<DualDiskAnnSearch> {
    build_dual_with_search(
        vault_path,
        slot_id,
        a_vectors,
        b_vectors,
        params,
        DiskAnnSearchParams::default(),
    )
}

pub fn build_dual_with_search(
    vault_path: &Path,
    slot_id: u8,
    a_vectors: &[(u32, Vec<f32>)],
    b_vectors: &[(u32, Vec<f32>)],
    build_params: DiskAnnBuildParams,
    search_params: DiskAnnSearchParams,
) -> Result<DualDiskAnnSearch> {
    validate_dual_rows(a_vectors, b_vectors, build_params.dim)?;
    if a_vectors.is_empty() {
        return Ok(DualDiskAnnSearch::empty(
            vault_path,
            slot_id,
            build_params.dim as u32,
            search_params,
        ));
    }
    let a_path = dual_graph_path(vault_path, slot_id, Direction::Forward);
    let b_path = dual_graph_path(vault_path, slot_id, Direction::Reverse);
    build_diskann_graph(&a_path, a_vectors, build_params)?;
    build_diskann_graph(&b_path, b_vectors, build_params)?;
    DualDiskAnnSearch::open_dual(vault_path, slot_id, search_params)
}

pub fn dual_graph_path(vault_path: &Path, slot_id: u8, direction: Direction) -> PathBuf {
    let suffix = match direction {
        Direction::Forward => "asym_a",
        Direction::Reverse => "asym_b",
    };
    vault_path
        .join("idx")
        .join(format!("slot_{slot_id:02}.{suffix}"))
        .join("graph.cda")
}

fn validate_dual_rows(
    a_vectors: &[(u32, Vec<f32>)],
    b_vectors: &[(u32, Vec<f32>)],
    dim: usize,
) -> Result<()> {
    if a_vectors.len() != b_vectors.len() {
        return Err(invalid(format!(
            "asymmetric graph row counts differ: {} vs {}",
            a_vectors.len(),
            b_vectors.len()
        )));
    }
    for (idx, ((a_id, a), (b_id, b))) in a_vectors.iter().zip(b_vectors).enumerate() {
        let expected = u32::try_from(idx).map_err(|_| invalid("dual row id exceeds u32"))?;
        if *a_id != expected || *b_id != expected {
            return Err(invalid(format!(
                "dual diskann ids must be dense and aligned at row {idx}: {a_id}/{b_id}"
            )));
        }
        if a.len() != dim || b.len() != dim {
            return Err(sextant_error(
                CALYX_INDEX_DIM_MISMATCH,
                format!(
                    "dual row {idx} dim mismatch: {} and {}, expected {dim}",
                    a.len(),
                    b.len()
                ),
            ));
        }
        if a.iter().chain(b).any(|v| !v.is_finite()) {
            return Err(invalid(format!("dual row {idx} has non-finite component")));
        }
    }
    Ok(())
}

fn open_count(path: &Path) -> Result<u64> {
    Ok(open_diskann_graph(path)?.node_count())
}

fn open_err(err: calyx_core::CalyxError) -> calyx_core::CalyxError {
    if err.code == CALYX_INDEX_CORRUPT || err.code == CALYX_INDEX_DIRECTION_UNAVAILABLE {
        sextant_error(CALYX_INDEX_IO, err.message)
    } else {
        err
    }
}

fn invalid(detail: impl std::fmt::Display) -> calyx_core::CalyxError {
    sextant_error(
        CALYX_INDEX_INVALID_PARAMS,
        format!("dual diskann: {detail}"),
    )
}

fn cx_from_local(local_id: u32) -> CxId {
    let mut bytes = [0_u8; 16];
    bytes[8..16].copy_from_slice(&u64::from(local_id).to_be_bytes());
    CxId::from_bytes(bytes)
}

fn sort_scores_desc(hits: &mut [(u32, f32)]) {
    hits.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
}
