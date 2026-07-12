//! Small repeated-dataset smoke audit for mixed KSG; this does not claim 95% coverage.

use rand::{Rng, SeedableRng, seq::SliceRandom};
use rand_chacha::ChaCha8Rng;

use super::*;

const OUTER_SEEDS: u64 = 4;
const COVERAGE_SAMPLES: usize = 60;
const COVERAGE_K: usize = 3;

#[test]
fn mixed_ksg_repeated_dataset_null_and_disjoint_support_audit() {
    audit_class_balance(30);
    audit_class_balance(15);
}

fn audit_class_balance(positive_count: usize) {
    let truth = binary_entropy_bits(positive_count as f32 / COVERAGE_SAMPLES as f32);
    let mut planted_interval_hits = 0;
    let mut null_compatible = 0;
    let mut null_successes = 0;
    let mut null_bits_sum = 0.0;

    for outer_seed in 0..OUTER_SEEDS {
        let (null_x, null_labels) = mixed_dataset(positive_count, false, 0x1380_0000 + outer_seed);
        match ksg_mi_continuous_discrete(&null_x, &null_labels, COVERAGE_K) {
            Ok(estimate) => {
                assert_point_contract(&estimate);
                null_successes += 1;
                null_bits_sum += estimate.bits;
                if estimate.ci_low <= 1e-6 {
                    null_compatible += 1;
                }
            }
            Err(error) if error.code == "CALYX_ASSAY_LOW_SIGNAL" => null_compatible += 1,
            Err(error) => panic!("unexpected null failure: {error}"),
        }

        let (planted_x, planted_labels) =
            mixed_dataset(positive_count, true, 0x1380_0000 + outer_seed);
        let estimate = ksg_mi_continuous_discrete(&planted_x, &planted_labels, COVERAGE_K)
            .expect("disjoint-support signal must estimate");
        assert_point_contract(&estimate);
        assert!(
            (estimate.bits - truth).abs() < 0.03,
            "positive_count={positive_count} truth={truth} estimate={estimate:?}"
        );
        if estimate.ci_low <= truth && truth <= estimate.ci_high {
            planted_interval_hits += 1;
        }
    }

    assert!(
        planted_interval_hits >= 3,
        "positive_count={positive_count} planted_interval_hits={planted_interval_hits}"
    );
    assert!(
        null_successes >= 3,
        "positive_count={positive_count} null_successes={null_successes}"
    );
    assert!(
        null_compatible >= 3,
        "positive_count={positive_count} null_compatible={null_compatible}"
    );
    let null_mean_bits = null_bits_sum / null_successes as f32;
    assert!(
        null_mean_bits < 0.25,
        "positive_count={positive_count} null_mean_bits={null_mean_bits}"
    );
}

fn assert_point_contract(estimate: &MiEstimate) {
    assert_eq!(estimate.estimator, EstimatorKind::Ksg);
    assert_eq!(estimate.trust, TrustTag::Provisional);
    assert_eq!(estimate.bound, crate::estimate::EstimateBound::Point);
    assert_eq!(estimate.n_samples, COVERAGE_SAMPLES);
    assert!(estimate.bits.is_finite());
    assert!(estimate.ci_low.is_finite());
    assert!(estimate.ci_high.is_finite());
    assert!(0.0 <= estimate.ci_low);
    assert!(estimate.ci_low <= estimate.bits);
    assert!(estimate.bits <= estimate.ci_high);
}

fn mixed_dataset(positive_count: usize, planted: bool, seed: u64) -> (Vec<Vec<f32>>, Vec<usize>) {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut labels = vec![0; COVERAGE_SAMPLES - positive_count];
    labels.extend(std::iter::repeat_n(1, positive_count));
    labels.shuffle(&mut rng);
    let x = labels
        .iter()
        .map(|label| {
            let base = rng.random_range(0.0..1.0);
            vec![base + if planted { *label as f32 * 3.0 } else { 0.0 }]
        })
        .collect();
    (x, labels)
}

fn binary_entropy_bits(p: f32) -> f32 {
    -p * p.log2() - (1.0 - p) * (1.0 - p).log2()
}
