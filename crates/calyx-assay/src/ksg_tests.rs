use std::fs;

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
    assert!(planted["covered_seed_count"].as_u64().unwrap() >= 4);
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
            "subsample_m": ksg_subsample_size(x.len(), 3).unwrap(),
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
            let index = rng.gen_range(0..x.len());
            sampled_x.push(x[index].clone());
            sampled_y.push(y[index].clone());
        }
        estimates.push(ksg_bits_from_validated_samples(&sampled_x, &sampled_y, k));
    }
    ci_from_resample_estimates(estimates, point_estimate)
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
            let index = rng.gen_range(0..n);
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
    let m = ksg_subsample_size(n, k).unwrap();
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    (0..resamples).all(|_| {
        let indices = sample_without_replacement_indices(n, m, &mut rng);
        indices_are_distinct(&indices, n)
    })
}

fn planted_signal_coverage_readback() -> serde_json::Value {
    let (x, y) = planted_samples(180, 12_083);
    let point = ksg_bits_from_validated_samples(&x, &y, 3);
    let known = gaussian_mi_bits(&x, &y);
    let mut covered_seed_count = 0;
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
        "seed_rows": seed_rows,
    })
}

fn independent_samples(n: usize, seed: u64) -> (Vec<Vec<f32>>, Vec<Vec<f32>>) {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut x = Vec::with_capacity(n);
    let mut y = Vec::with_capacity(n);
    for _ in 0..n {
        x.push(vec![rng.gen_range(-1.0..1.0)]);
        y.push(vec![rng.gen_range(-1.0..1.0)]);
    }
    (x, y)
}

fn planted_samples(n: usize, seed: u64) -> (Vec<Vec<f32>>, Vec<Vec<f32>>) {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut x = Vec::with_capacity(n);
    let mut y = Vec::with_capacity(n);
    for _ in 0..n {
        let signal = rng.gen_range(-1.0..1.0);
        let noise = rng.gen_range(-0.18..0.18);
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
