use std::collections::{BinaryHeap, HashSet};

use calyx_core::CxId;

use super::HnswIndex;
use super::scored::{
    ScoredIndex, diversified_neighbors, score_better, sort_scored, top_k_indices, worst_position,
    worst_scored,
};
use crate::util::cosine;

const EXACT_CONSTRUCTION_ROWS: usize = 4_096;
const CONSTRUCTION_EF: usize = 64;
const RECENT_CONSTRUCTION_SCAN: usize = 128;
const CONSTRUCTION_VISIT_LIMIT_EF_MULTIPLIER: usize = 1;
const SEARCH_VISIT_LIMIT_EF_MULTIPLIER: usize = 16;

impl HnswIndex {
    pub(super) fn connect_new_row(&mut self, index: usize) {
        if index == 0 {
            return;
        }
        let query = self.rows[index].vector.clone();
        let mut neighbors = self.construction_candidates(index, &query);
        append_stride_neighbors(index, &mut neighbors);
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

    pub(super) fn refresh_entry_after_insert(&mut self, index: usize) {
        if self
            .entry_point
            .map(|entry| self.rows[index].level > self.rows[entry].level)
            .unwrap_or(true)
        {
            self.entry_point = Some(index);
        }
    }

    fn construction_candidates(&self, index: usize, query: &[f32]) -> Vec<usize> {
        if index <= EXACT_CONSTRUCTION_ROWS {
            return self.exhaustive_candidates(index, query);
        }
        let candidates = self.approximate_candidate_set(index, query);
        let mut neighbors = top_k_indices(
            scored_from_indices(self, &candidates, query),
            self.max_neighbors,
        );
        for level in 1..=self.rows[index].level {
            let level_scored = candidates
                .iter()
                .copied()
                .filter(|idx| self.rows[*idx].level >= level)
                .map(|idx| (idx, cosine(query, &self.rows[idx].vector)))
                .collect();
            neighbors.extend(top_k_indices(level_scored, self.max_neighbors));
        }
        neighbors
    }

    fn exhaustive_candidates(&self, index: usize, query: &[f32]) -> Vec<usize> {
        let mut neighbors = top_k_indices(
            self.rows[..index]
                .iter()
                .enumerate()
                .map(|(idx, row)| (idx, cosine(query, &row.vector)))
                .collect(),
            self.max_neighbors,
        );
        for level in 1..=self.rows[index].level {
            neighbors.extend(top_k_indices(
                self.rows[..index]
                    .iter()
                    .enumerate()
                    .filter(|(_, row)| row.level >= level)
                    .map(|(idx, row)| (idx, cosine(query, &row.vector)))
                    .collect(),
                self.max_neighbors,
            ));
        }
        neighbors
    }

    fn approximate_candidate_set(&self, index: usize, query: &[f32]) -> Vec<usize> {
        let mut candidates = Vec::with_capacity(CONSTRUCTION_EF + RECENT_CONSTRUCTION_SCAN + 24);
        let ef = CONSTRUCTION_EF.min(index).max(self.max_neighbors);
        if let Some(entry) = self.entry_point_before(index) {
            let start = self.greedy_descent_before(query, entry, index);
            candidates.extend(
                self.beam_search_indices(
                    query,
                    start,
                    ef,
                    index,
                    CONSTRUCTION_VISIT_LIMIT_EF_MULTIPLIER,
                )
                .into_iter()
                .map(|(idx, _)| idx),
            );
        }
        let recent_start = index.saturating_sub(RECENT_CONSTRUCTION_SCAN);
        candidates.extend(recent_start..index);
        append_stride_neighbors(index, &mut candidates);
        candidates.sort_unstable();
        candidates.dedup();
        candidates
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

    pub(super) fn entry_point(&self) -> Option<usize> {
        self.entry_point
    }

    fn entry_point_before(&self, limit: usize) -> Option<usize> {
        if let Some(entry) = self.entry_point
            && entry < limit
        {
            return Some(entry);
        }
        self.rows[..limit]
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.level.cmp(&right.level))
            .map(|(idx, _)| idx)
    }

    pub(super) fn greedy_descent(&self, query: &[f32], current: usize) -> usize {
        self.greedy_descent_before(query, current, self.rows.len())
    }

    fn greedy_descent_before(&self, query: &[f32], mut current: usize, limit: usize) -> usize {
        if current >= limit {
            return current;
        }
        let max_level = self.rows[current].level;
        for level in (1..=max_level).rev() {
            loop {
                let current_score = cosine(query, &self.rows[current].vector);
                let mut best = (current, current_score);
                for &neighbor in &self.rows[current].neighbors {
                    if neighbor >= limit || self.rows[neighbor].level < level {
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

    pub(super) fn beam_search(&self, query: &[f32], entry: usize, ef: usize) -> Vec<(CxId, f32)> {
        let effective_ef = ef
            .saturating_add(self.tombstone_count())
            .min(self.rows.len());
        self.beam_search_indices(
            query,
            entry,
            effective_ef,
            self.rows.len(),
            SEARCH_VISIT_LIMIT_EF_MULTIPLIER,
        )
        .into_iter()
        .filter(|(idx, _)| !self.rows[*idx].deleted)
        .map(|(idx, score)| (self.rows[idx].cx_id, score))
        .collect()
    }

    fn beam_search_indices(
        &self,
        query: &[f32],
        entry: usize,
        ef: usize,
        limit: usize,
        visit_multiplier: usize,
    ) -> Vec<(usize, f32)> {
        let entry_score = cosine(query, &self.rows[entry].vector);
        let max_visits = ef.saturating_mul(visit_multiplier).min(limit);
        let mut visited = HashSet::with_capacity(max_visits);
        let mut candidates = BinaryHeap::new();
        let mut best = vec![(entry, entry_score)];
        candidates.push(ScoredIndex {
            idx: entry,
            score: entry_score,
        });
        visited.insert(entry);

        while let Some(candidate) = candidates.pop() {
            if visited.len() >= max_visits && best.len() >= ef {
                break;
            }
            let candidate = (candidate.idx, candidate.score);
            if best.len() >= ef {
                let worst = worst_scored(&best).unwrap_or(candidate);
                if !score_better(candidate, worst) {
                    break;
                }
            }
            for &neighbor in &self.rows[candidate.0].neighbors {
                if visited.len() >= max_visits {
                    break;
                }
                if neighbor >= limit || !visited.insert(neighbor) {
                    continue;
                }
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
        best
    }
}

fn scored_from_indices(
    index: &HnswIndex,
    candidates: &[usize],
    query: &[f32],
) -> Vec<(usize, f32)> {
    candidates
        .iter()
        .copied()
        .map(|idx| (idx, cosine(query, &index.rows[idx].vector)))
        .collect()
}

fn append_stride_neighbors(index: usize, neighbors: &mut Vec<usize>) {
    let mut stride = 1;
    while index >= stride {
        neighbors.push(index - stride);
        stride *= 2;
    }
}
