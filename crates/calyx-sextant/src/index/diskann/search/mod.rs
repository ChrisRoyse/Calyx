//! DiskANN beam search and raw-f32 rescore (PH68 T02).

mod helpers;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use calyx_core::{CxId, Result, SlotId, SlotShape, SlotVector};

use super::build::{DiskAnnBuildParams, build_diskann_graph};
use super::graph::DiskAnnGraphReader;
use crate::error::{CALYX_INDEX_DIM_MISMATCH, CALYX_INDEX_IO, sextant_error};
use crate::index::{IndexSearchHit, IndexStats, SextantIndex, ranked};
use crate::util::dense;

use helpers::{
    Candidate, dense_rows, distance, invalid, io, open_for_search, positions, prefetch_node,
    sorted, stop_search,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskAnnSearchParams {
    pub beamwidth: usize,
    pub ef_search: usize,
    pub rescore_k: usize,
    pub rescore_from_raw: bool,
}

impl Default for DiskAnnSearchParams {
    fn default() -> Self {
        Self {
            beamwidth: 32,
            ef_search: 64,
            rescore_k: 64,
            rescore_from_raw: true,
        }
    }
}

#[derive(Debug)]
pub struct DiskAnnSearch {
    slot: SlotId,
    dim: u32,
    graph_path: PathBuf,
    raw_sidecar: Option<PathBuf>,
    reader: Option<DiskAnnGraphReader>,
    graph_file: Option<File>,
    ids: Vec<CxId>,
    positions: HashMap<CxId, u32>,
    build_params: DiskAnnBuildParams,
    default_search: DiskAnnSearchParams,
    built_at_seq: u64,
    base_seq: u64,
}

impl DiskAnnSearch {
    pub fn open(
        slot: SlotId,
        graph_path: impl Into<PathBuf>,
        ids: Vec<CxId>,
        raw_sidecar: Option<PathBuf>,
        default_search: DiskAnnSearchParams,
    ) -> Result<Self> {
        let graph_path = graph_path.into();
        let reader = open_for_search(&graph_path)?;
        let header = *reader.header();
        if ids.len() != header.node_count as usize {
            return Err(invalid(format!(
                "id map len {} != graph node_count {}",
                ids.len(),
                header.node_count
            )));
        }
        let graph_file = File::open(&graph_path).map_err(|e| io("open graph for prefetch", e))?;
        let build_params = DiskAnnBuildParams {
            dim: header.dim as usize,
            m_max: header.m_max as usize,
            ef_construction: default_search.ef_search.max(header.m_max as usize),
            alpha: 1.2,
        };
        Ok(Self {
            slot,
            dim: header.dim,
            graph_path,
            raw_sidecar,
            reader: Some(reader),
            graph_file: Some(graph_file),
            positions: positions(&ids),
            ids,
            build_params,
            default_search,
            built_at_seq: 0,
            base_seq: 0,
        })
    }

    pub fn build(
        slot: SlotId,
        graph_path: impl Into<PathBuf>,
        rows: &[(CxId, Vec<f32>)],
        build_params: DiskAnnBuildParams,
        raw_sidecar: Option<PathBuf>,
        default_search: DiskAnnSearchParams,
    ) -> Result<Self> {
        let graph_path = graph_path.into();
        let dense_rows = dense_rows(rows, build_params.dim)?;
        build_diskann_graph(&graph_path, &dense_rows, build_params)?;
        Self::open(
            slot,
            graph_path,
            rows.iter().map(|(cx_id, _)| *cx_id).collect(),
            raw_sidecar,
            default_search,
        )
    }

    pub fn empty(slot: SlotId, dim: u32, graph_path: impl Into<PathBuf>) -> Self {
        Self {
            slot,
            dim,
            graph_path: graph_path.into(),
            raw_sidecar: None,
            reader: None,
            graph_file: None,
            ids: Vec::new(),
            positions: HashMap::new(),
            build_params: DiskAnnBuildParams {
                dim: dim as usize,
                m_max: 32,
                ef_construction: 64,
                alpha: 1.2,
            },
            default_search: DiskAnnSearchParams::default(),
            built_at_seq: 0,
            base_seq: 0,
        }
    }

    pub fn persist_path(&self) -> &Path {
        &self.graph_path
    }

    pub fn search_ids(
        &self,
        query: &[f32],
        k: usize,
        params: &DiskAnnSearchParams,
    ) -> Result<Vec<(u32, f32)>> {
        if k == 0 || self.ids.is_empty() {
            return Ok(Vec::new());
        }
        self.validate_query(query)?;
        params.validate()?;
        let reader = match &self.reader {
            Some(reader) => reader,
            None => return Ok(Vec::new()),
        };
        let want = k.min(self.ids.len());
        if params.ef_search < want {
            return Err(invalid(format!(
                "ef_search {} below requested candidate count {want}",
                params.ef_search
            )));
        }
        let rescore_k = params.rescore_k.max(want).min(self.ids.len());
        let mut scores = BTreeMap::<u32, f32>::new();
        let mut expanded = BTreeSet::<u32>::new();
        let mut candidates = vec![Candidate::new(
            reader.header().entry_point_id,
            distance(
                query,
                reader.read_node(reader.header().entry_point_id)?.vector,
            ),
        )];
        while expanded.len() < params.ef_search.min(self.ids.len()) {
            candidates.sort();
            self.prefetch(&candidates, params.beamwidth, reader)?;
            let Some(next) = candidates
                .iter()
                .find(|candidate| !expanded.contains(&candidate.id))
                .copied()
            else {
                break;
            };
            if !expanded.insert(next.id) {
                continue;
            }
            scores
                .entry(next.id)
                .and_modify(|score| *score = score.min(next.distance))
                .or_insert(next.distance);
            let node = reader.read_node(next.id)?;
            for &neighbor in node.neighbors {
                if expanded.contains(&neighbor) {
                    continue;
                }
                let neighbor_node = reader.read_node(neighbor)?;
                let d = distance(query, neighbor_node.vector);
                scores
                    .entry(neighbor)
                    .and_modify(|score| *score = score.min(d))
                    .or_insert(d);
                candidates.push(Candidate::new(neighbor, d));
            }
            candidates.sort();
            candidates.dedup_by_key(|candidate| candidate.id);
            candidates.truncate(params.ef_search.max(rescore_k));
            if stop_search(&candidates, &scores, &expanded, rescore_k) {
                break;
            }
        }
        let mut hits = sorted(scores.into_iter().collect());
        hits.truncate(rescore_k);
        if params.rescore_from_raw {
            hits = self.rescore_from_raw(query, &hits)?;
        }
        hits.truncate(want);
        Ok(hits)
    }

    fn validate_query(&self, query: &[f32]) -> Result<()> {
        if query.len() != self.dim as usize {
            return Err(sextant_error(
                CALYX_INDEX_DIM_MISMATCH,
                format!("query dim {} expected {}", query.len(), self.dim),
            ));
        }
        if query.iter().any(|v| !v.is_finite()) {
            return Err(invalid("query vector has non-finite component"));
        }
        Ok(())
    }

    fn rescore_from_raw(&self, query: &[f32], hits: &[(u32, f32)]) -> Result<Vec<(u32, f32)>> {
        let Some(raw_dir) = &self.raw_sidecar else {
            return Ok(hits.to_vec());
        };
        if !raw_dir.is_dir() {
            return Ok(hits.to_vec());
        }
        let mut rescored = Vec::with_capacity(hits.len());
        for &(id, _) in hits {
            let raw = self.read_raw_vector(raw_dir, id)?;
            rescored.push((id, distance(query, &raw)));
        }
        Ok(sorted(rescored))
    }

    fn read_raw_vector(&self, raw_dir: &Path, id: u32) -> Result<Vec<f32>> {
        let Some(path) = self.raw_path(raw_dir, id) else {
            return Err(sextant_error(
                CALYX_INDEX_IO,
                format!("raw sidecar missing for diskann node {id}"),
            ));
        };
        let bytes = fs::read(&path).map_err(|e| io("read raw sidecar", e))?;
        if bytes.len() != self.dim as usize * 4 {
            return Err(sextant_error(
                CALYX_INDEX_IO,
                format!(
                    "raw sidecar {} is {} B, expected {} B",
                    path.display(),
                    bytes.len(),
                    self.dim as usize * 4
                ),
            ));
        }
        let mut out = Vec::with_capacity(self.dim as usize);
        for chunk in bytes.chunks_exact(4) {
            let value = f32::from_le_bytes(chunk.try_into().expect("4B"));
            if !value.is_finite() {
                return Err(sextant_error(
                    CALYX_INDEX_IO,
                    format!("raw sidecar {} has non-finite f32", path.display()),
                ));
            }
            out.push(value);
        }
        Ok(out)
    }

    fn raw_path(&self, raw_dir: &Path, id: u32) -> Option<PathBuf> {
        let mut names = vec![id.to_string(), format!("{id}.raw"), format!("{id:08}.raw")];
        if let Some(cx_id) = self.ids.get(id as usize) {
            names.push(cx_id.to_string());
            names.push(format!("{cx_id}.raw"));
        }
        names
            .into_iter()
            .map(|name| raw_dir.join(name))
            .find(|p| p.is_file())
    }

    fn prefetch(
        &self,
        candidates: &[Candidate],
        beamwidth: usize,
        reader: &DiskAnnGraphReader,
    ) -> Result<()> {
        let Some(file) = &self.graph_file else {
            return Ok(());
        };
        for candidate in candidates.iter().take(beamwidth) {
            prefetch_node(
                file,
                reader.node_block_offset(candidate.id)?,
                reader.node_block_size(),
            );
        }
        Ok(())
    }

    fn vectors_from_graph(&self) -> Result<Vec<Vec<f32>>> {
        let Some(reader) = &self.reader else {
            return Ok(Vec::new());
        };
        (0..reader.node_count() as u32)
            .map(|id| reader.read_node(id).map(|node| node.vector.to_vec()))
            .collect()
    }
}

impl SextantIndex for DiskAnnSearch {
    fn slot(&self) -> SlotId {
        self.slot
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Dense(self.dim)
    }

    fn insert(&mut self, cx_id: CxId, vector: SlotVector, seq: u64) -> Result<()> {
        let values = dense(&vector)?;
        self.validate_query(values)?;
        let mut vectors = self.vectors_from_graph()?;
        if let Some(&id) = self.positions.get(&cx_id) {
            vectors[id as usize] = values.to_vec();
        } else {
            let id = u32::try_from(self.ids.len())
                .map_err(|_| invalid("diskann graph exceeds u32 node id space"))?;
            self.positions.insert(cx_id, id);
            self.ids.push(cx_id);
            vectors.push(values.to_vec());
        }
        let rows: Vec<_> = self.ids.iter().copied().zip(vectors).collect();
        let dense_rows = dense_rows(&rows, self.dim as usize)?;
        build_diskann_graph(&self.graph_path, &dense_rows, self.build_params)?;
        self.reader = Some(open_for_search(&self.graph_path)?);
        self.graph_file =
            Some(File::open(&self.graph_path).map_err(|e| io("open graph for prefetch", e))?);
        self.built_at_seq = self.built_at_seq.max(seq);
        self.base_seq = self.base_seq.max(seq);
        Ok(())
    }

    fn search(
        &self,
        query: &SlotVector,
        k: usize,
        ef: Option<usize>,
    ) -> Result<Vec<IndexSearchHit>> {
        let query = dense(query)?;
        let mut params = self.default_search;
        if let Some(ef) = ef {
            params.ef_search = ef;
        }
        let scored = self
            .search_ids(query, k, &params)?
            .into_iter()
            .map(|(id, dist)| (self.ids[id as usize], 1.0 - dist))
            .collect();
        Ok(ranked(scored))
    }

    fn rebuild(&mut self) -> Result<()> {
        let vectors = self.vectors_from_graph()?;
        if vectors.is_empty() {
            return Ok(());
        }
        let rows: Vec<_> = self.ids.iter().copied().zip(vectors).collect();
        let dense_rows = dense_rows(&rows, self.dim as usize)?;
        build_diskann_graph(&self.graph_path, &dense_rows, self.build_params)?;
        self.reader = Some(open_for_search(&self.graph_path)?);
        Ok(())
    }

    fn vector(&self, cx_id: CxId) -> Option<SlotVector> {
        let id = *self.positions.get(&cx_id)?;
        let reader = self.reader.as_ref()?;
        let vector = reader.read_node(id).ok()?.vector.to_vec();
        Some(SlotVector::Dense {
            dim: self.dim,
            data: vector,
        })
    }

    fn set_base_seq(&mut self, seq: u64) {
        self.base_seq = seq;
    }

    fn stats(&self) -> IndexStats {
        IndexStats {
            slot: self.slot,
            shape: self.shape(),
            len: self.ids.len(),
            built_at_seq: self.built_at_seq,
            base_seq: self.base_seq,
            kind: "DiskANN",
        }
    }
}
