use std::collections::{BinaryHeap, HashSet};

use calyx_sextant::index::{FbinVectors, gen_row};
use rayon::prelude::*;

const CHUNK: u64 = 200_000;

/// Brute-force the true top-`k` neighbours (by L2) for each query over a REAL
/// corpus `.fbin`. Memory-bounded: scans the mmap'd file in row windows.
pub(super) fn brute_force_topk_fbin(
    corpus: &FbinVectors,
    queries: &[Vec<f32>],
    k: usize,
) -> Vec<HashSet<u64>> {
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
                    (OrdF32(l2(q, row)), idx)
                })
                .collect();
            push_scored(&mut heaps[qi], scored, k);
        }
        start = end;
    }
    heaps_to_sets(heaps)
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
                .map(|(idx, row)| (OrdF32(l2(q, row)), *idx))
                .collect();
            push_scored(&mut heaps[qi], scored, k);
        }
        start = end;
    }
    heaps_to_sets(heaps)
}

fn l2(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right)
        .map(|(l, r)| {
            let diff = l - r;
            diff * diff
        })
        .sum()
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
    heaps
        .into_iter()
        .map(|heap| heap.into_iter().map(|(_, idx)| idx).collect())
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
