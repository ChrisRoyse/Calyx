use super::*;

pub(in crate::persisted::multi) struct PinnedCandidateSelection {
    row_indexes: Vec<usize>,
    token_count: usize,
}

impl PinnedCandidateSelection {
    pub(in crate::persisted::multi) fn row_count(&self) -> usize {
        self.row_indexes.len()
    }

    pub(in crate::persisted::multi) fn token_count(&self) -> usize {
        self.token_count
    }

    pub(in crate::persisted::multi) fn row_index(&self, index: usize) -> Option<usize> {
        self.row_indexes.get(index).copied()
    }
}

pub(in crate::persisted::multi) struct PinnedCandidateRow<'a> {
    pub(in crate::persisted::multi) cx_id: CxId,
    pub(in crate::persisted::multi) tokens: &'a [f32],
    pub(in crate::persisted::multi) norms: &'a [f32],
}

impl PinnedMultiIndex {
    pub(in crate::persisted::multi) fn select_candidates(
        &self,
        candidates: &BTreeSet<CxId>,
    ) -> PinnedCandidateSelection {
        let mut row_indexes = Vec::with_capacity(candidates.len());
        let mut token_count = 0usize;
        for candidate in candidates {
            let Ok(found) = self
                .row_lookup
                .binary_search_by_key(candidate, |(cx_id, _)| *cx_id)
            else {
                continue;
            };
            let row_index = self.row_lookup[found].1;
            token_count = token_count.saturating_add(self.rows[row_index].token_count);
            row_indexes.push(row_index);
        }
        PinnedCandidateSelection {
            row_indexes,
            token_count,
        }
    }

    pub(in crate::persisted::multi) fn selected_row(
        &self,
        index: usize,
    ) -> Option<PinnedCandidateRow<'_>> {
        let row = self.rows.get(index)?;
        let dim = self.token_dim as usize;
        Some(PinnedCandidateRow {
            cx_id: row.cx_id,
            tokens: &self.tokens[row.token_start * dim..(row.token_start + row.token_count) * dim],
            norms: &self.norms[row.token_start..row.token_start + row.token_count],
        })
    }

    pub(in crate::persisted::multi) fn score_candidates_exact(
        &self,
        selection: &PinnedCandidateSelection,
        query: &[Vec<f32>],
    ) -> CliResult<Vec<(CxId, f32)>> {
        let mut scored = Vec::with_capacity(selection.row_count());
        for &row_index in &selection.row_indexes {
            let row = self
                .selected_row(row_index)
                .ok_or_else(|| stale("resident MaxSim candidate row index is out of bounds"))?;
            let score = MaxSimIndex::maxsim_flat(query, row.tokens, self.token_dim as usize)?;
            scored.push((row.cx_id, score));
        }
        Ok(scored)
    }
}
