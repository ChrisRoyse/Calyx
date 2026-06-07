use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

use crate::Result;
use crate::cpu::guard::check_finite;

pub fn topk_f32(scores: &[f32], k: usize) -> Result<Vec<(usize, f32)>> {
    if k == 0 || scores.is_empty() {
        return Ok(Vec::new());
    }
    check_finite(scores, "topk")?;

    let mut heap: BinaryHeap<Reverse<RankedScore>> = BinaryHeap::with_capacity(k.min(scores.len()));
    for (index, score) in scores.iter().copied().enumerate() {
        let ranked = RankedScore { index, score };
        if heap.len() < k {
            heap.push(Reverse(ranked));
        } else if heap.peek().is_some_and(|worst| ranked > worst.0) {
            heap.pop();
            heap.push(Reverse(ranked));
        }
    }

    let mut ranked: Vec<_> = heap.into_iter().map(|Reverse(score)| score).collect();
    ranked.sort_by(|left, right| right.cmp(left));
    Ok(ranked
        .into_iter()
        .map(|score| (score.index, score.score))
        .collect())
}

#[derive(Clone, Copy, Debug)]
struct RankedScore {
    index: usize,
    score: f32,
}

impl PartialEq for RankedScore {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.score.to_bits() == other.score.to_bits()
    }
}

impl Eq for RankedScore {}

impl PartialOrd for RankedScore {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RankedScore {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score
            .total_cmp(&other.score)
            .then_with(|| other.index.cmp(&self.index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Backend, CpuBackend, ForgeError};
    use proptest::prelude::*;

    #[test]
    fn topk_tie_break_deterministic() -> Result<()> {
        let result = topk_f32(&[0.1, 0.9, 0.5, 0.9], 2)?;
        println!("TOPK_TIE {:?}", result);
        assert_eq!(result, vec![(1, 0.9), (3, 0.9)]);
        Ok(())
    }

    #[test]
    fn topk_edges_single_equal_empty_and_backend() -> Result<()> {
        assert_eq!(topk_f32(&[42.0], 1)?, vec![(0, 42.0)]);
        assert_eq!(
            topk_f32(&[1.0, 1.0, 1.0, 1.0], 3)?,
            vec![(0, 1.0), (1, 1.0), (2, 1.0)]
        );
        assert!(topk_f32(&[], 5)?.is_empty());
        assert!(topk_f32(&[1.0, 2.0], 0)?.is_empty());

        let cpu = CpuBackend::new();
        assert_eq!(cpu.topk(&[3.0, 5.0, 4.0], 2)?, vec![(1, 5.0), (2, 4.0)]);
        Ok(())
    }

    #[test]
    fn topk_fail_closed_nan() {
        let err = topk_f32(&[1.0, f32::NAN], 1).expect_err("NaN must fail closed");
        println!("TOPK_FAIL_NAN {err}");
        assert!(matches!(err, ForgeError::NumericalInvariant { .. }));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        #[test]
        fn topk_subset_sorted_and_bounded(
            scores in proptest::collection::vec(-100.0f32..100.0, 0..=128),
            k in 0usize..160
        ) {
            let result = topk_f32(&scores, k);
            prop_assert!(result.is_ok(), "topk failed: {:?}", result.err());
            let result = result.expect("checked ok");
            prop_assert_eq!(result.len(), k.min(scores.len()));
            for (index, score) in &result {
                prop_assert!(*index < scores.len());
                prop_assert_eq!(*score, scores[*index]);
            }
            for pair in result.windows(2) {
                let (left_index, left_score) = pair[0];
                let (right_index, right_score) = pair[1];
                prop_assert!(
                    left_score > right_score
                        || (left_score == right_score && left_index < right_index)
                );
            }
        }
    }
}
