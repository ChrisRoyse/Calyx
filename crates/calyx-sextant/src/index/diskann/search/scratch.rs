use std::cell::RefCell;
use std::cmp::Ordering;

use calyx_core::Result;

use super::super::pq::DiskAnnPqQuery;
use super::helpers::{Candidate, DiskAnnDistanceMode, distance_to_node, invalid};
use super::{DiskAnnSearch, DiskAnnSearchParams};

thread_local! {
    static SEARCH_SCRATCH: RefCell<SearchScratch> = RefCell::new(SearchScratch::default());
}

pub(super) fn search_ids(
    index: &DiskAnnSearch,
    query: &[f32],
    k: usize,
    params: &DiskAnnSearchParams,
) -> Result<Vec<(u32, f32)>> {
    SEARCH_SCRATCH.with_borrow_mut(|scratch| scratch.search(index, query, k, params))
}

#[derive(Default)]
struct SearchScratch {
    epoch: u32,
    node_count: usize,
    expanded_count: usize,
    expanded_epoch: Vec<u32>,
    scored_epoch: Vec<u32>,
    scores: Vec<f32>,
    scored_ids: Vec<u32>,
    candidates: Vec<Candidate>,
    ranked: Vec<(u32, f32)>,
}

impl SearchScratch {
    fn search(
        &mut self,
        index: &DiskAnnSearch,
        query: &[f32],
        k: usize,
        params: &DiskAnnSearchParams,
    ) -> Result<Vec<(u32, f32)>> {
        if k == 0 || index.ids.is_empty() {
            return Ok(Vec::new());
        }
        index.validate_query(query)?;
        params.validate()?;
        let reader = match &index.reader {
            Some(reader) => reader,
            None => return Ok(Vec::new()),
        };
        let want = k.min(index.ids.len());
        if params.ef_search < want {
            return Err(invalid(format!(
                "ef_search {} below requested candidate count {want}",
                params.ef_search
            )));
        }
        let graph_query = index.graph_query(query);
        let pq_query = match (&index.pq, index.distance_mode) {
            (Some(pq), DiskAnnDistanceMode::UnitL2) => Some(pq.query(graph_query.as_ref())?),
            _ => None,
        };
        let pq_scored = pq_query.is_some();
        let rescore_k = params.rescore_k.max(want).min(index.ids.len());
        let candidate_limit = params.ef_search.max(rescore_k).min(index.ids.len());
        self.prepare(index.ids.len(), candidate_limit);

        let entry = reader.header().entry_point_id;
        let entry_distance = score_node(
            index,
            reader,
            pq_query.as_ref(),
            graph_query.as_ref(),
            entry,
        )?;
        self.record_score(entry, entry_distance)?;
        self.insert_candidate(entry, entry_distance, candidate_limit)?;
        while self.expanded_count < params.ef_search.min(index.ids.len()) {
            index.prefetch(&self.candidates, params.beamwidth, reader)?;
            let Some(next) = self.pop_next_unexpanded() else {
                break;
            };
            if !self.mark_expanded(next.id)? {
                continue;
            }
            let node = reader.read_node(next.id)?;
            for &neighbor in node.neighbors {
                if self.is_expanded(neighbor)? {
                    continue;
                }
                let d = score_node(
                    index,
                    reader,
                    pq_query.as_ref(),
                    graph_query.as_ref(),
                    neighbor,
                )?;
                self.record_score(neighbor, d)?;
                self.insert_candidate(neighbor, d, candidate_limit)?;
            }
            if self.stop_search(rescore_k) {
                break;
            }
        }
        let mut hits = self.sorted_hits(rescore_k);
        if params.rescore_from_raw {
            hits = index.rescore_final(query, graph_query.as_ref(), &hits, pq_scored)?;
        }
        hits.truncate(want);
        Ok(hits)
    }

    fn prepare(&mut self, node_count: usize, candidate_limit: usize) {
        self.bump_epoch();
        self.node_count = node_count;
        self.expanded_count = 0;
        self.expanded_epoch.resize(node_count, 0);
        self.scored_epoch.resize(node_count, 0);
        self.scores.resize(node_count, f32::INFINITY);
        self.scored_ids.clear();
        self.candidates.clear();
        self.ranked.clear();
        self.candidates.reserve(candidate_limit);
        self.scored_ids.reserve(candidate_limit);
        self.ranked.reserve(candidate_limit);
    }

    fn bump_epoch(&mut self) {
        if self.epoch == u32::MAX {
            self.expanded_epoch.fill(0);
            self.scored_epoch.fill(0);
            self.epoch = 1;
        } else {
            self.epoch += 1;
        }
    }

    fn record_score(&mut self, id: u32, distance: f32) -> Result<()> {
        let idx = self.idx(id)?;
        if self.scored_epoch[idx] == self.epoch {
            self.scores[idx] = self.scores[idx].min(distance);
        } else {
            self.scored_epoch[idx] = self.epoch;
            self.scores[idx] = distance;
            self.scored_ids.push(id);
        }
        Ok(())
    }

    fn insert_candidate(&mut self, id: u32, distance: f32, limit: usize) -> Result<()> {
        if self.is_expanded(id)? {
            return Ok(());
        }
        if let Some(pos) = self
            .candidates
            .iter()
            .position(|candidate| candidate.id == id)
        {
            if self.candidates[pos].distance <= distance {
                return Ok(());
            }
            self.candidates.remove(pos);
        }
        let candidate = Candidate::new(id, distance);
        let pos = self
            .candidates
            .binary_search_by(|probe| probe.cmp(&candidate))
            .unwrap_or_else(|pos| pos);
        if pos >= limit {
            return Ok(());
        }
        self.candidates.insert(pos, candidate);
        if self.candidates.len() > limit {
            self.candidates.truncate(limit);
        }
        Ok(())
    }

    fn pop_next_unexpanded(&mut self) -> Option<Candidate> {
        while !self.candidates.is_empty() {
            let next = self.candidates.remove(0);
            if !self.is_expanded_fast(next.id) {
                return Some(next);
            }
        }
        None
    }

    fn mark_expanded(&mut self, id: u32) -> Result<bool> {
        let idx = self.idx(id)?;
        if self.expanded_epoch[idx] == self.epoch {
            return Ok(false);
        }
        self.expanded_epoch[idx] = self.epoch;
        self.expanded_count += 1;
        Ok(true)
    }

    fn is_expanded(&self, id: u32) -> Result<bool> {
        Ok(self.expanded_epoch[self.idx(id)?] == self.epoch)
    }

    fn is_expanded_fast(&self, id: u32) -> bool {
        let idx = id as usize;
        idx < self.node_count && self.expanded_epoch[idx] == self.epoch
    }

    fn stop_search(&mut self, want: usize) -> bool {
        if self.scored_ids.len() < want {
            return false;
        }
        let worst = self.worst_score(want);
        self.candidates
            .iter()
            .find(|candidate| !self.is_expanded_fast(candidate.id))
            .is_none_or(|best| best.distance >= worst)
    }

    fn worst_score(&mut self, want: usize) -> f32 {
        self.ranked_scores();
        let idx = want - 1;
        self.ranked.select_nth_unstable_by(idx, cmp_hit);
        self.ranked[idx].1
    }

    fn sorted_hits(&mut self, limit: usize) -> Vec<(u32, f32)> {
        self.ranked_scores();
        self.ranked.sort_by(cmp_hit);
        self.ranked.truncate(limit);
        self.ranked.clone()
    }

    fn ranked_scores(&mut self) {
        self.ranked.clear();
        self.ranked.extend(
            self.scored_ids
                .iter()
                .map(|&id| (id, self.scores[id as usize])),
        );
    }

    fn idx(&self, id: u32) -> Result<usize> {
        let idx = id as usize;
        if idx >= self.node_count {
            return Err(invalid(format!(
                "graph references node {id} outside node_count {}",
                self.node_count
            )));
        }
        Ok(idx)
    }
}

fn cmp_hit(a: &(u32, f32), b: &(u32, f32)) -> Ordering {
    a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0))
}

fn score_node(
    index: &DiskAnnSearch,
    reader: &crate::index::diskann::DiskAnnGraphReader,
    pq_query: Option<&DiskAnnPqQuery<'_>>,
    graph_query: &[f32],
    id: u32,
) -> Result<f32> {
    if let Some(pq_query) = pq_query {
        return Ok(0.5 * pq_query.distance_l2(id)?);
    }
    Ok(distance_to_node(
        graph_query,
        reader.read_node(id)?.vector,
        index.distance_mode,
    ))
}
