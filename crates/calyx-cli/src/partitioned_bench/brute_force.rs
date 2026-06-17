use std::collections::{BinaryHeap, HashSet};

use calyx_sextant::index::{FbinVectors, gen_row};
use rayon::prelude::*;

const CHUNK: u64 = 200_000;

/// Brute-force the true top-`k` neighbours (by cosine distance) for each query
/// over a REAL corpus `.fbin`. Memory-bounded: scans the mmap'd file in row
/// windows. This must match DiskANN search scoring; raw L2 is only equivalent for
/// already-normalized embeddings.
pub(super) fn brute_force_topk_fbin(
    corpus: &FbinVectors,
    queries: &[Vec<f32>],
    k: usize,
) -> Vec<HashSet<u64>> {
    brute_force_topk_fbin_ranked(corpus, queries, k)
        .into_iter()
        .map(|row| row.into_iter().map(|(id, _)| id).collect())
        .collect()
}

/// Brute-force exact ranked top-k rows (by cosine distance) for each real query
/// over a real corpus `.fbin`. This preserves rank so RRF can build an exact
/// fused ground truth, not just a set-overlap recall verdict.
pub(super) fn brute_force_topk_fbin_ranked(
    corpus: &FbinVectors,
    queries: &[Vec<f32>],
    k: usize,
) -> Vec<Vec<(u64, f32)>> {
    let n_cx = corpus.count();
    let mut heaps: Vec<BinaryHeap<(OrdF32, u64)>> = (0..queries.len())
        .map(|_| BinaryHeap::with_capacity(k + 1))
        .collect();
    let mut start = 0u64;
    while start < n_cx {
        let end = (start + CHUNK).min(n_cx);
        for (qi, q) in queries.iter().enumerate() {
            let scored: Vec<(OrdF32, u64)> = (start..end)
                .into_par_iter()
                .map(|idx| {
                    let row = corpus.row(idx);
                    (OrdF32(cosine_distance(q, row)), idx)
                })
                .collect();
            push_scored(&mut heaps[qi], scored, k);
        }
        start = end;
    }
    heaps_to_ranked(heaps)
}

/// Brute-force the true top-`k` neighbours (by L2) for each query over the
/// generated corpus without materializing all rows at once.
pub(super) fn brute_force_topk(
    seed: u64,
    n_cx: u64,
    dim: usize,
    queries: &[Vec<f32>],
    k: usize,
) -> Vec<HashSet<u64>> {
    let mut heaps: Vec<BinaryHeap<(OrdF32, u64)>> = (0..queries.len())
        .map(|_| BinaryHeap::with_capacity(k + 1))
        .collect();
    let mut start = 0u64;
    while start < n_cx {
        let end = (start + CHUNK).min(n_cx);
        let rows: Vec<(u64, Vec<f32>)> = (start..end)
            .into_par_iter()
            .map(|idx| (idx, gen_row(seed, idx, dim)))
            .collect();
        for (qi, q) in queries.iter().enumerate() {
            let scored: Vec<(OrdF32, u64)> = rows
                .par_iter()
                .map(|(idx, row)| (OrdF32(cosine_distance(q, row)), *idx))
                .collect();
            push_scored(&mut heaps[qi], scored, k);
        }
        start = end;
    }
    heaps_to_sets(heaps)
}

fn cosine_distance(left: &[f32], right: &[f32]) -> f32 {
    let mut dot = 0.0;
    let mut left_norm = 0.0;
    let mut right_norm = 0.0;
    for (left, right) in left.iter().zip(right) {
        dot += left * right;
        left_norm += left * left;
        right_norm += right * right;
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        1.0
    } else {
        (1.0 - dot / (left_norm.sqrt() * right_norm.sqrt())).max(0.0)
    }
}

fn push_scored(heap: &mut BinaryHeap<(OrdF32, u64)>, scored: Vec<(OrdF32, u64)>, k: usize) {
    for item in scored {
        heap.push(item);
        if heap.len() > k {
            heap.pop();
        }
    }
}

fn heaps_to_sets(heaps: Vec<BinaryHeap<(OrdF32, u64)>>) -> Vec<HashSet<u64>> {
    heaps_to_ranked(heaps)
        .into_iter()
        .map(|row| row.into_iter().map(|(idx, _)| idx).collect())
        .collect()
}

fn heaps_to_ranked(heaps: Vec<BinaryHeap<(OrdF32, u64)>>) -> Vec<Vec<(u64, f32)>> {
    heaps
        .into_iter()
        .map(|heap| {
            let mut row = heap
                .into_iter()
                .map(|(distance, idx)| (idx, distance.0))
                .collect::<Vec<_>>();
            row.sort_by(|left, right| {
                left.1
                    .total_cmp(&right.1)
                    .then_with(|| left.0.cmp(&right.0))
            });
            row
        })
        .collect()
}

/// Minimal total-order wrapper over f32 for heap keys.
#[derive(Clone, Copy, PartialEq)]
struct OrdF32(f32);

impl Eq for OrdF32 {}

impl PartialOrd for OrdF32 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrdF32 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_distance_handles_unnormalized_vectors() {
        let query = [10.0, 0.0];
        let same_direction = [100.0, 0.0];
        let l2_closer_but_worse_angle = [9.0, 1.0];

        assert!(
            cosine_distance(&query, &same_direction)
                < cosine_distance(&query, &l2_closer_but_worse_angle)
        );
    }
}
