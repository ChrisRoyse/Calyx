//! Deterministic in-RAM dense index with an HNSW-compatible seam.

use calyx_core::{CxId, Result, SlotId, SlotShape, SlotVector};

use super::{IndexSearchHit, IndexStats, QuantConfig, SextantIndex, ranked};
use crate::util::{cosine, dense, top_k};

#[derive(Clone, Debug)]
struct Row {
    cx_id: CxId,
    vector: Vec<f32>,
    seq: u64,
    level: u8,
    neighbors: Vec<usize>,
}

#[derive(Clone, Debug)]
pub struct HnswIndex {
    slot: SlotId,
    dim: u32,
    seed: u64,
    max_neighbors: usize,
    rows: Vec<Row>,
    quant: QuantConfig,
    built_at_seq: u64,
    base_seq: u64,
}

impl HnswIndex {
    pub fn new(slot: SlotId, dim: u32, seed: u64) -> Self {
        Self {
            slot,
            dim,
            seed,
            max_neighbors: 8,
            rows: Vec::new(),
            quant: QuantConfig::none(),
            built_at_seq: 0,
            base_seq: 0,
        }
    }

    pub fn with_quant(mut self, quant: QuantConfig) -> Self {
        self.quant = quant;
        self
    }

    pub fn neighbor_counts(&self) -> Vec<usize> {
        self.rows.iter().map(|row| row.neighbors.len()).collect()
    }

    pub fn layer_histogram(&self) -> Vec<usize> {
        let max = self.rows.iter().map(|row| row.level).max().unwrap_or(0) as usize;
        let mut hist = vec![0; max + 1];
        for row in &self.rows {
            hist[row.level as usize] += 1;
        }
        hist
    }

    pub fn brute_force(&self, query: &[f32], k: usize) -> Vec<(CxId, f32)> {
        top_k(
            self.rows
                .iter()
                .map(|row| (row.cx_id, cosine(query, &row.vector)))
                .collect(),
            k,
        )
    }

    pub fn recall_at(&self, queries: &[Vec<f32>], k: usize, ef: usize) -> f32 {
        if queries.is_empty() {
            return 1.0;
        }
        let mut total = 0.0;
        for query in queries {
            let exact: Vec<_> = self
                .brute_force(query, k)
                .into_iter()
                .map(|x| x.0)
                .collect();
            let got: Vec<_> = self
                .search(
                    &SlotVector::Dense {
                        dim: self.dim,
                        data: query.clone(),
                    },
                    k,
                    Some(ef),
                )
                .unwrap()
                .into_iter()
                .map(|x| x.cx_id)
                .collect();
            let overlap = got.iter().filter(|cx| exact.contains(cx)).count();
            total += overlap as f32 / k.max(1) as f32;
        }
        total / queries.len() as f32
    }

    fn level_for(&self, cx_id: CxId, ordinal: usize) -> u8 {
        let mut hasher = blake3::Hasher::new();
        hasher.update(cx_id.as_bytes());
        hasher.update(&self.seed.to_be_bytes());
        hasher.update(&(ordinal as u64).to_be_bytes());
        let byte = hasher.finalize().as_bytes()[0];
        byte.trailing_zeros().min(6) as u8
    }

    fn connect_new_row(&mut self, index: usize) {
        if index == 0 {
            return;
        }
        let query = self.rows[index].vector.clone();
        let neighbors = top_k(
            self.rows[..index]
                .iter()
                .enumerate()
                .map(|(idx, row)| {
                    (
                        CxId::from_bytes([idx as u8; 16]),
                        cosine(&query, &row.vector),
                    )
                })
                .collect(),
            self.max_neighbors,
        );
        self.rows[index].neighbors = neighbors
            .into_iter()
            .map(|(fake, _)| fake.as_bytes()[0] as usize)
            .collect();
    }
}

impl SextantIndex for HnswIndex {
    fn slot(&self) -> SlotId {
        self.slot
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Dense(self.dim)
    }

    fn insert(&mut self, cx_id: CxId, vector: SlotVector, seq: u64) -> Result<()> {
        let values = dense(&vector)?;
        if values.len() != self.dim as usize {
            return Err(crate::error::sextant_error(
                crate::error::CALYX_SEXTANT_VECTOR_SHAPE,
                format!("dim {} expected {}", values.len(), self.dim),
            ));
        }
        self.quant.lock_after_first_insert();
        if let Some(row) = self.rows.iter_mut().find(|row| row.cx_id == cx_id) {
            row.vector = values.to_vec();
            row.seq = seq;
        } else {
            let level = self.level_for(cx_id, self.rows.len());
            self.rows.push(Row {
                cx_id,
                vector: values.to_vec(),
                seq,
                level,
                neighbors: Vec::new(),
            });
            let index = self.rows.len() - 1;
            self.connect_new_row(index);
        }
        self.built_at_seq = self.built_at_seq.max(seq);
        self.base_seq = self.base_seq.max(seq);
        Ok(())
    }

    fn search(
        &self,
        query: &SlotVector,
        k: usize,
        _ef: Option<usize>,
    ) -> Result<Vec<IndexSearchHit>> {
        Ok(ranked(self.brute_force(dense(query)?, k)))
    }

    fn rebuild(&mut self) -> Result<()> {
        for row in &mut self.rows {
            row.neighbors.clear();
        }
        for idx in 0..self.rows.len() {
            self.connect_new_row(idx);
        }
        self.built_at_seq = self.base_seq;
        Ok(())
    }

    fn vector(&self, cx_id: CxId) -> Option<SlotVector> {
        self.rows
            .iter()
            .find(|row| row.cx_id == cx_id)
            .map(|row| SlotVector::Dense {
                dim: self.dim,
                data: row.vector.clone(),
            })
    }

    fn set_base_seq(&mut self, seq: u64) {
        self.base_seq = seq;
    }

    fn stats(&self) -> IndexStats {
        IndexStats {
            slot: self.slot,
            shape: self.shape(),
            len: self.rows.len(),
            built_at_seq: self.built_at_seq,
            base_seq: self.base_seq,
            kind: "hnsw",
        }
    }
}
