//! Deterministic in-RAM dense HNSW-style index.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use calyx_core::{CxId, Result, SlotId, SlotShape, SlotVector};

use super::{IndexSearchHit, IndexStats, QuantConfig, SextantIndex, ranked};
use crate::error::{
    CALYX_SEXTANT_DIM_MISMATCH, CALYX_SEXTANT_EF_TOO_SMALL, CALYX_SEXTANT_INDEX_EMPTY,
    CALYX_SEXTANT_VECTOR_SHAPE, sextant_error,
};
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
            max_neighbors: 32,
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
        let mut neighbors = top_k_indices(
            self.rows[..index]
                .iter()
                .enumerate()
                .map(|(idx, row)| (idx, cosine(&query, &row.vector)))
                .collect(),
            self.max_neighbors,
        );
        for level in 1..=self.rows[index].level {
            neighbors.extend(top_k_indices(
                self.rows[..index]
                    .iter()
                    .enumerate()
                    .filter(|(_, row)| row.level >= level)
                    .map(|(idx, row)| (idx, cosine(&query, &row.vector)))
                    .collect(),
                self.max_neighbors,
            ));
        }
        let mut stride = 1;
        while index >= stride {
            neighbors.push(index - stride);
            stride *= 2;
        }
        neighbors.sort_unstable();
        neighbors.dedup();
        self.rows[index].neighbors = neighbors.clone();
        self.prune_neighbors(index);
        for neighbor in neighbors.drain(..) {
            if !self.rows[neighbor].neighbors.contains(&index) {
                self.rows[neighbor].neighbors.push(index);
            }
            self.prune_neighbors(neighbor);
        }
    }

    fn prune_neighbors(&mut self, index: usize) {
        let vector = self.rows[index].vector.clone();
        let mut candidates = self.rows[index].neighbors.clone();
        candidates.sort_unstable();
        candidates.dedup();
        candidates.retain(|neighbor| *neighbor != index && *neighbor < self.rows.len());
        let scored: Vec<_> = candidates
            .into_iter()
            .map(|neighbor| (neighbor, cosine(&vector, &self.rows[neighbor].vector)))
            .collect();
        self.rows[index].neighbors = diversified_neighbors(scored, index, self.max_neighbors);
    }

    fn entry_point(&self) -> Option<usize> {
        let mut best: Option<usize> = None;
        for (idx, row) in self.rows.iter().enumerate() {
            if best
                .map(|best_idx| row.level > self.rows[best_idx].level)
                .unwrap_or(true)
            {
                best = Some(idx);
            }
        }
        best
    }

    fn greedy_descent(&self, query: &[f32], mut current: usize) -> usize {
        let Some(max_level) = self.rows.iter().map(|row| row.level).max() else {
            return current;
        };
        for level in (1..=max_level).rev() {
            loop {
                let current_score = cosine(query, &self.rows[current].vector);
                let mut best = (current, current_score);
                for &neighbor in &self.rows[current].neighbors {
                    if self.rows[neighbor].level < level {
                        continue;
                    }
                    let score = cosine(query, &self.rows[neighbor].vector);
                    if score_better((neighbor, score), best) {
                        best = (neighbor, score);
                    }
                }
                if best.0 == current {
                    break;
                }
                current = best.0;
            }
        }
        current
    }

    fn beam_search(&self, query: &[f32], entry: usize, ef: usize) -> Vec<(CxId, f32)> {
        let entry_score = cosine(query, &self.rows[entry].vector);
        let mut visited = vec![false; self.rows.len()];
        let mut candidates = BinaryHeap::new();
        let mut best = vec![(entry, entry_score)];
        candidates.push(ScoredIndex {
            idx: entry,
            score: entry_score,
        });
        visited[entry] = true;

        while let Some(candidate) = candidates.pop() {
            let candidate = (candidate.idx, candidate.score);
            if best.len() >= ef {
                let worst = worst_scored(&best).unwrap_or(candidate);
                if !score_better(candidate, worst) {
                    break;
                }
            }
            for &neighbor in &self.rows[candidate.0].neighbors {
                if visited[neighbor] {
                    continue;
                }
                visited[neighbor] = true;
                let scored = (neighbor, cosine(query, &self.rows[neighbor].vector));
                candidates.push(ScoredIndex {
                    idx: scored.0,
                    score: scored.1,
                });
                best.push(scored);
                if best.len() > ef
                    && let Some(worst) = worst_position(&best)
                {
                    best.swap_remove(worst);
                }
            }
        }

        sort_scored(&mut best);
        best.into_iter()
            .map(|(idx, score)| (self.rows[idx].cx_id, score))
            .collect()
    }

    fn checked_query<'a>(&self, query: &'a SlotVector) -> Result<&'a [f32]> {
        let values = dense(query)?;
        if values.len() != self.dim as usize {
            return Err(sextant_error(
                CALYX_SEXTANT_DIM_MISMATCH,
                format!("query dim {} expected {}", values.len(), self.dim),
            ));
        }
        Ok(values)
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
            return Err(sextant_error(
                CALYX_SEXTANT_VECTOR_SHAPE,
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
        ef: Option<usize>,
    ) -> Result<Vec<IndexSearchHit>> {
        if self.rows.is_empty() {
            return Err(sextant_error(
                CALYX_SEXTANT_INDEX_EMPTY,
                "hnsw search requested on an empty index",
            ));
        }
        if k == 0 {
            return Err(sextant_error(
                CALYX_SEXTANT_EF_TOO_SMALL,
                "hnsw search requires k > 0",
            ));
        }
        let query = self.checked_query(query)?;
        let needed = k.min(self.rows.len());
        let ef = ef
            .unwrap_or_else(|| needed.max(self.max_neighbors * 2))
            .min(self.rows.len());
        if ef < needed {
            return Err(sextant_error(
                CALYX_SEXTANT_EF_TOO_SMALL,
                format!("ef {ef} below requested result count {needed}"),
            ));
        }
        let entry = self.entry_point().ok_or_else(|| {
            sextant_error(
                CALYX_SEXTANT_INDEX_EMPTY,
                "hnsw search requested on an empty index",
            )
        })?;
        let start = self.greedy_descent(query, entry);
        let mut results = self.beam_search(query, start, ef);
        results.truncate(k);
        Ok(ranked(results))
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

fn top_k_indices(scored: Vec<(usize, f32)>, k: usize) -> Vec<usize> {
    let mut scored = scored;
    sort_scored(&mut scored);
    scored.truncate(k);
    scored.into_iter().map(|(idx, _)| idx).collect()
}

fn diversified_neighbors(
    mut scored: Vec<(usize, f32)>,
    origin: usize,
    max_neighbors: usize,
) -> Vec<usize> {
    sort_scored(&mut scored);
    let nearest_cap = (max_neighbors / 2).max(1);
    let mut chosen: Vec<usize> = scored
        .iter()
        .take(nearest_cap)
        .map(|(idx, _)| *idx)
        .collect();
    scored.sort_by(|a, b| {
        ordinal_distance(b.0, origin)
            .cmp(&ordinal_distance(a.0, origin))
            .then_with(|| b.1.total_cmp(&a.1))
            .then_with(|| a.0.cmp(&b.0))
    });
    for (idx, _) in scored {
        if chosen.len() >= max_neighbors {
            break;
        }
        if !chosen.contains(&idx) {
            chosen.push(idx);
        }
    }
    chosen.sort_unstable();
    chosen
}

fn ordinal_distance(left: usize, right: usize) -> usize {
    left.max(right) - left.min(right)
}

fn worst_scored(scored: &[(usize, f32)]) -> Option<(usize, f32)> {
    scored.iter().copied().reduce(|worst, candidate| {
        if score_worse(candidate, worst) {
            candidate
        } else {
            worst
        }
    })
}

fn worst_position(scored: &[(usize, f32)]) -> Option<usize> {
    let mut worst = None;
    for (idx, candidate) in scored.iter().copied().enumerate() {
        if worst
            .map(|worst_idx| score_worse(candidate, scored[worst_idx]))
            .unwrap_or(true)
        {
            worst = Some(idx);
        }
    }
    worst
}

fn sort_scored(scored: &mut [(usize, f32)]) {
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
}

fn score_better(candidate: (usize, f32), incumbent: (usize, f32)) -> bool {
    candidate.1 > incumbent.1 || (candidate.1 == incumbent.1 && candidate.0 < incumbent.0)
}

fn score_worse(candidate: (usize, f32), incumbent: (usize, f32)) -> bool {
    candidate.1 < incumbent.1 || (candidate.1 == incumbent.1 && candidate.0 > incumbent.0)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ScoredIndex {
    idx: usize,
    score: f32,
}

impl Eq for ScoredIndex {}

impl Ord for ScoredIndex {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score
            .total_cmp(&other.score)
            .then_with(|| other.idx.cmp(&self.idx))
    }
}

impl PartialOrd for ScoredIndex {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
