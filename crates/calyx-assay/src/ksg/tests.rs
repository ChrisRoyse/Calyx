use std::{
    collections::{BTreeMap, HashSet},
    fs,
};

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde_json::json;

use super::*;

#[test]
fn ksg_no_replacement_ci_rejects_duplicate_bootstrap_pathology() {
    let (x, y) = independent_samples(160, 12_080);
    let point = ksg_bits_from_validated_samples(&x, &y, 3);
    let old_ci = old_with_replacement_ci(&x, &y, point, 3, KSG_BOOTSTRAP_CONFIG);
    let new_ci = ksg_subsample_ci(&x, &y, point, 3, KSG_BOOTSTRAP_CONFIG).unwrap();
    let old_estimate = MiEstimate::new(
        point,
        old_ci.ci_low,
        old_ci.ci_high,
        x.len(),
        EstimatorKind::Ksg,
        TrustTag::Provisional,
    );
    let new_estimate = MiEstimate::new(
        point,
        new_ci.ci_low,
        new_ci.ci_high,
        x.len(),
        EstimatorKind::Ksg,
        TrustTag::Provisional,
    );
    let duplicate_stats = old_replacement_duplicate_stats(x.len(), x.len(), 25, 12_081);
    let no_replacement = no_replacement_duplicate_free(x.len(), 3, 25, 12_082);

    assert!(new_estimate.ci_low <= 0.02, "{new_estimate:?}");
    assert!(
        old_estimate.ci_high > new_estimate.ci_high * 4.0,
        "old={old_estimate:?} new={new_estimate:?}"
    );
    assert!(duplicate_stats["max_duplicates"].as_u64().unwrap() > 0);
    assert!(no_replacement);

    let planted = planted_signal_coverage_readback();
    assert_eq!(planted["finite_seed_count"].as_u64().unwrap(), 5);
    let short = ksg_mi_continuous(&x[..60], &y[..60], 3).unwrap_err();
    assert_eq!(short.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    assert!(short.message.contains("m=48"));

    maybe_write_issue1208_fsv(json!({
        "source_of_truth": "calyx-assay KSG unit test readback from estimator internals and persisted JSON bytes",
        "independent_true_mi_bits": 0.0,
        "independent": {
            "samples": x.len(),
            "point_bits": point,
            "old_with_replacement": old_estimate,
            "new_no_replacement": new_estimate,
            "new_ci_low_lte_0_02": new_estimate.ci_low <= 0.02,
        },
        "duplicate_invariant": {
            "old_with_replacement": duplicate_stats,
            "new_no_replacement_duplicate_free": no_replacement,
            "subsample_m": m_out_of_n_size(x.len(), 3, MIN_ASSAY_SAMPLES, "KSG").unwrap(),
        },
        "planted_signal": planted,
        "edge_case": {
            "case": "n_just_above_min_but_subsample_below_min",
            "before": {"n": 60, "k": 3, "subsample_m": 48},
            "after": {"error": short.code, "message": short.message.clone()},
        },
    }));
}

fn old_with_replacement_ci(
    x: &[Vec<f32>],
    y: &[Vec<f32>],
    point_estimate: f32,
    k: usize,
    config: BootstrapConfig,
) -> BootstrapCi {
    let mut rng = ChaCha8Rng::seed_from_u64(config.seed);
    let mut estimates = Vec::with_capacity(config.resamples);
    for _ in 0..config.resamples {
        let mut sampled_x = Vec::with_capacity(x.len());
        let mut sampled_y = Vec::with_capacity(y.len());
        for _ in 0..x.len() {
            let index = rng.random_range(0..x.len());
            sampled_x.push(x[index].clone());
            sampled_y.push(y[index].clone());
        }
        estimates.push(ksg_bits_from_validated_samples(&sampled_x, &sampled_y, k));
    }
    ci_from_resample_estimates(estimates, point_estimate, 1.0)
}

#[test]
fn continuous_ksg_zero_joint_radius_fails_closed() {
    let mut x = Vec::new();
    let mut y = Vec::new();
    for i in 0..MIN_ASSAY_SAMPLES {
        let value = (i / 4) as f32;
        x.push(vec![value]);
        y.push(vec![value]);
    }

    let error = ksg_mi_continuous(&x, &y, 3).expect_err("k exact duplicates must fail closed");

    assert_eq!(error.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    assert!(error.message.contains("kth joint radius is zero"));
}

#[test]
fn mixed_ksg_zero_same_class_radius_fails_closed() {
    let x = vec![vec![0.0]; 80];
    let labels = (0..80).map(|index| index % 2).collect::<Vec<_>>();

    let error = ksg_mi_continuous_discrete(&x, &labels, 3)
        .expect_err("k same-class duplicates must fail closed");

    assert_eq!(error.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    assert!(error.message.contains("kth same-class radius is zero"));
}

#[test]
fn mixed_ksg_internal_subsample_preserves_full_estimate_quorum() {
    let labels = (0..60).map(|index| index % 2).collect::<Vec<_>>();
    let x = labels
        .iter()
        .enumerate()
        .map(|(index, label)| {
            let center = if *label == 0 { -3.0 } else { 3.0 };
            vec![center + (index / 2) as f32 * 0.01]
        })
        .collect::<Vec<_>>();

    let estimate = ksg_mi_continuous_discrete(&x, &labels, 3)
        .expect("the 50-row quorum applies to the full estimate, not each internal root");

    assert_eq!(estimate.n_samples, 60);
    assert_eq!(estimate.bound, crate::estimate::EstimateBound::Point);
    assert!(estimate.ci_low.is_finite());
    assert!(estimate.ci_high.is_finite());
    assert!(estimate.ci_low <= estimate.bits);
    assert!(estimate.bits <= estimate.ci_high);
}

#[test]
fn mixed_ksg_subsamples_are_conditionally_uniform_not_quota_forced() {
    let labels = (0..100)
        .map(|index| usize::from(index >= 80))
        .collect::<Vec<_>>();
    let m = 30;
    let mut minority_total = 0;
    let resamples = 4_096;

    for seed in 0..resamples {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let indices = mixed_ci::sample_mixed_indices(&labels, m, 3, &mut rng).unwrap();
        let unique = indices.iter().copied().collect::<HashSet<_>>();
        let sampled_labels = indices
            .iter()
            .map(|index| labels[*index])
            .collect::<Vec<_>>();
        let counts = mixed_ci::validate_classes(&sampled_labels, 3).unwrap();

        assert_eq!(indices.len(), m);
        assert_eq!(unique.len(), m);
        assert!(counts.values().all(|count| *count > 3));
        minority_total += counts[&1];
    }

    let minority_mean = minority_total as f64 / resamples as f64;
    assert!(
        (6.15..=6.45).contains(&minority_mean),
        "conditional SRSWOR mean should be about 6.298, got {minority_mean}"
    );
}

#[test]
fn mixed_ksg_rare_class_without_supported_half_sample_plan_fails_closed() {
    let x = (0..100).map(|index| vec![index as f32]).collect::<Vec<_>>();
    let mut labels = vec![0; 96];
    labels.extend([1, 1, 1, 1]);

    let error = ksg_mi_continuous_discrete(&x, &labels, 3)
        .expect_err("a barely valid full class cannot support an honest half-sample CI");

    assert_eq!(error.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    assert!(error.message.contains("class-support"));
}

#[test]
fn mixed_ksg_duplicate_guard_uses_the_exact_k_threshold_and_label_scope() {
    let mut threshold_x = (0..80).map(|index| vec![index as f32]).collect::<Vec<_>>();
    let threshold_labels = (0..80).map(|index| index % 2).collect::<Vec<_>>();
    threshold_x[0] = vec![0.0];
    threshold_x[2] = vec![0.0];
    threshold_x[4] = vec![0.0];
    mixed_ci::validate_radius_defined(&threshold_x, &threshold_labels, 3)
        .expect("k-1 same-class duplicates leave a positive kth radius");
    threshold_x[6] = vec![0.0];
    let error = mixed_ci::validate_radius_defined(&threshold_x, &threshold_labels, 3)
        .expect_err("exactly k same-class duplicates make the kth radius zero");
    assert_eq!(error.code, "CALYX_ASSAY_DEGENERATE_INPUT");

    let cross_class_x = (0..80)
        .map(|index| vec![(index / 2) as f32])
        .collect::<Vec<_>>();
    let cross_class_labels = (0..80).map(|index| index % 2).collect::<Vec<_>>();
    mixed_ci::validate_radius_defined(&cross_class_x, &cross_class_labels, 3)
        .expect("cross-class coordinate ties do not collapse a same-class radius");
}

#[test]
fn mixed_ksg_positive_boundary_ties_use_observed_same_class_mass() {
    let x = (0..60).map(|index| vec![index as f32]).collect::<Vec<_>>();
    let labels = vec![0; x.len()];
    let counts = mixed_ci::validate_classes(&labels, 3).unwrap();

    let raw = mixed_ci::raw_bits_from_validated_samples(&x, &labels, 3, &counts);

    assert!(
        raw.abs() < 1e-12,
        "one constant label has exactly zero MI even when the kth distance has positive ties; got {raw}"
    );
}

#[test]
fn mixed_ksg_support_plan_uses_exact_hypergeometric_tail_math() {
    let probability = mixed_ci::hypergeometric_cdf_at_most(10, 4, 5, 1).unwrap();
    assert!((probability - 66.0 / 252.0).abs() < 1e-12);

    let balanced = BTreeMap::from([(0, 30), (1, 30)]);
    let balanced_plan = mixed_ci::subsample_plan(60, 3, &balanced).unwrap();
    assert_eq!(balanced_plan.m, 16);
    assert!(balanced_plan.support_failure_upper <= 0.01);

    let imbalanced = BTreeMap::from([(0, 80), (1, 20)]);
    let imbalanced_plan = mixed_ci::subsample_plan(100, 3, &imbalanced).unwrap();
    assert_eq!(imbalanced_plan.m, 40);
    assert!(imbalanced_plan.support_failure_upper <= 0.01);
}

#[test]
fn mixed_ksg_root_interval_reverses_tails_and_preserves_raw_negatives() {
    let interval = mixed_ci::ci_from_roots(vec![-4.0, -1.0, 0.0, 2.0, 9.0], 2.0, 100).unwrap();
    assert!((interval.ci_low - 1.1).abs() < 1e-6);
    assert!((interval.ci_high - 2.4).abs() < 1e-6);

    let negative = mixed_ci::ci_from_roots(vec![-1.0, 0.0, 1.0], -0.2, 100).unwrap();
    assert!((negative.mean + 0.2).abs() < 1e-6);
    assert!(negative.ci_low < 0.0);
    assert!(negative.ci_high < 0.0);

    let error = mixed_ci::ci_from_roots(vec![0.0, f64::NAN], 0.0, 100).unwrap_err();
    assert_eq!(error.code, "CALYX_ASSAY_LOW_SIGNAL");
}

#[test]
fn mixed_ksg_finite_population_scale_and_production_root_count_are_pinned() {
    let scale = mixed_ci::finite_population_root_scale(100, 50).unwrap();
    assert!((scale - 10.0).abs() < 1e-12);
    assert_eq!(mixed_ci::MIXED_KSG_SUBSAMPLE_RESAMPLES, 999);
}

fn old_replacement_duplicate_stats(
    n: usize,
    draws: usize,
    resamples: usize,
    seed: u64,
) -> serde_json::Value {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut max_duplicates = 0;
    let mut total_duplicates = 0;
    for _ in 0..resamples {
        let mut seen = vec![false; n];
        let mut unique = 0;
        for _ in 0..draws {
            let index = rng.random_range(0..n);
            if !seen[index] {
                seen[index] = true;
                unique += 1;
            }
        }
        let duplicates = draws - unique;
        max_duplicates = max_duplicates.max(duplicates);
        total_duplicates += duplicates;
    }
    json!({
        "resamples": resamples,
        "draws_per_resample": draws,
        "max_duplicates": max_duplicates,
        "mean_duplicates": total_duplicates as f32 / resamples as f32,
    })
}

fn no_replacement_duplicate_free(n: usize, k: usize, resamples: usize, seed: u64) -> bool {
    let m = m_out_of_n_size(n, k, MIN_ASSAY_SAMPLES, "KSG").unwrap();
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    (0..resamples).all(|_| {
        let mut indices = sample_without_replacement_indices(n, m, &mut rng).unwrap();
        indices.sort_unstable();
        indices.dedup();
        indices.len() == m
    })
}

fn planted_signal_coverage_readback() -> serde_json::Value {
    let (x, y) = planted_samples(180, 12_083);
    let point = ksg_bits_from_validated_samples(&x, &y, 3);
    let known = gaussian_mi_bits(&x, &y);
    let mut covered_seed_count = 0;
    let mut finite_seed_count = 0;
    let mut seed_rows = Vec::new();
    for seed in 0..5 {
        let config = BootstrapConfig::new(80, seed);
        let ci = ksg_subsample_ci(&x, &y, point, 3, config).unwrap();
        let estimate = MiEstimate::new(
            point,
            ci.ci_low,
            ci.ci_high,
            x.len(),
            EstimatorKind::Ksg,
            TrustTag::Provisional,
        );
        let covers = estimate.ci_low <= known && known <= estimate.ci_high;
        covered_seed_count += usize::from(covers);
        finite_seed_count +=
            usize::from(estimate.ci_low.is_finite() && estimate.ci_high.is_finite());
        seed_rows.push(json!({
            "seed": seed,
            "ci_low": estimate.ci_low,
            "ci_high": estimate.ci_high,
            "covers_known": covers,
        }));
    }
    json!({
        "samples": x.len(),
        "point_bits": point,
        "known_gaussian_bits": known,
        "covered_seed_count": covered_seed_count,
        "finite_seed_count": finite_seed_count,
        "seed_rows": seed_rows,
    })
}

fn independent_samples(n: usize, seed: u64) -> (Vec<Vec<f32>>, Vec<Vec<f32>>) {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut x = Vec::with_capacity(n);
    let mut y = Vec::with_capacity(n);
    for _ in 0..n {
        x.push(vec![rng.random_range(-1.0..1.0)]);
        y.push(vec![rng.random_range(-1.0..1.0)]);
    }
    (x, y)
}

fn planted_samples(n: usize, seed: u64) -> (Vec<Vec<f32>>, Vec<Vec<f32>>) {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut x = Vec::with_capacity(n);
    let mut y = Vec::with_capacity(n);
    for _ in 0..n {
        let signal = rng.random_range(-1.0..1.0);
        let noise = rng.random_range(-0.18..0.18);
        x.push(vec![signal]);
        y.push(vec![0.75 * signal + noise]);
    }
    (x, y)
}

fn gaussian_mi_bits(x: &[Vec<f32>], y: &[Vec<f32>]) -> f32 {
    let x_mean = x.iter().map(|row| row[0]).sum::<f32>() / x.len() as f32;
    let y_mean = y.iter().map(|row| row[0]).sum::<f32>() / y.len() as f32;
    let mut cov = 0.0;
    let mut xv = 0.0;
    let mut yv = 0.0;
    for (left, right) in x.iter().zip(y) {
        let dx = left[0] - x_mean;
        let dy = right[0] - y_mean;
        cov += dx * dy;
        xv += dx * dx;
        yv += dy * dy;
    }
    let r2 = (cov * cov / (xv * yv)).clamp(0.0, 0.999);
    -0.5 * (1.0 - r2).log2()
}

#[test]
fn continuous_ksg_rejects_finite_inputs_with_infinite_derived_distance() {
    let mut x: Vec<_> = (0..80).map(|i| vec![i as f32 * 0.01]).collect();
    let y: Vec<_> = (0..80).map(|i| vec![i as f32 * 0.02]).collect();
    x[0][0] = f32::MAX;
    x[1][0] = -f32::MAX;

    let err = ksg_mi_continuous(&x, &y, 3).expect_err("infinite derived distance fails");
    assert_eq!(err.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    assert!(err.message.contains("finite f32 Chebyshev distance"));
    assert!(err.message.contains("x dimension 0"));
}

#[test]
fn mixed_ksg_rejects_finite_inputs_with_infinite_derived_distance() {
    let mut x: Vec<_> = (0..80).map(|i| vec![i as f32 * 0.01]).collect();
    let labels: Vec<_> = (0..80).map(|i| i % 2).collect();
    x[0][0] = f32::MAX;
    x[1][0] = -f32::MAX;

    let err =
        ksg_mi_continuous_discrete(&x, &labels, 3).expect_err("infinite derived distance fails");
    assert_eq!(err.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    assert!(err.message.contains("finite f32 Chebyshev distance"));
    assert!(err.message.contains("x dimension 0"));
}

fn maybe_write_issue1208_fsv(readback: serde_json::Value) {
    let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
        return;
    };
    let dir = root.join("issue1208-ksg-subsample-ci");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ksg-subsample-ci-readback.json");
    let bytes = serde_json::to_vec_pretty(&readback).unwrap();
    fs::write(&path, &bytes).unwrap();
    let stored = fs::read(&path).unwrap();
    assert_eq!(stored, bytes);
    println!("ISSUE1208_KSG_SUBSAMPLE_CI_READBACK={}", path.display());
}
