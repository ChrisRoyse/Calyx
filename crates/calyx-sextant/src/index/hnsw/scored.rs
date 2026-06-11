use std::cmp::Ordering;

pub(super) fn top_k_indices(scored: Vec<(usize, f32)>, k: usize) -> Vec<usize> {
    let mut scored = scored;
    sort_scored(&mut scored);
    scored.truncate(k);
    scored.into_iter().map(|(idx, _)| idx).collect()
}

pub(super) fn diversified_neighbors(
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

pub(super) fn worst_scored(scored: &[(usize, f32)]) -> Option<(usize, f32)> {
    scored.iter().copied().reduce(|worst, candidate| {
        if score_worse(candidate, worst) {
            candidate
        } else {
            worst
        }
    })
}

pub(super) fn worst_position(scored: &[(usize, f32)]) -> Option<usize> {
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

pub(super) fn sort_scored(scored: &mut [(usize, f32)]) {
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
}

pub(super) fn score_better(candidate: (usize, f32), incumbent: (usize, f32)) -> bool {
    candidate.1 > incumbent.1 || (candidate.1 == incumbent.1 && candidate.0 < incumbent.0)
}

fn score_worse(candidate: (usize, f32), incumbent: (usize, f32)) -> bool {
    candidate.1 < incumbent.1 || (candidate.1 == incumbent.1 && candidate.0 > incumbent.0)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ScoredIndex {
    pub idx: usize,
    pub score: f32,
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
