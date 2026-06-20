//! Binary outcome logistic-probe MI estimator.

use calyx_core::{Anchor, CalyxError, Result};
use serde::{Deserialize, Serialize};

use crate::bootstrap::{
    BootstrapConfig, DEFAULT_BOOTSTRAP_RESAMPLES, DEFAULT_BOOTSTRAP_SEED, bootstrap_paired_ci,
};
use crate::calibration::{
    DEFAULT_MIN_POWER_RECOVERY_RATIO, PowerCalibration, ensure_informative_binary_labels,
};
use crate::estimate::{EstimateReliability, EstimatorKind, MiEstimate, TrustTag, trust_for_anchor};
use crate::group_split::{GroupSplit, group_holdout_split, row_groups};
use crate::ksg::MIN_ASSAY_SAMPLES;
use crate::samples::validate_rectangular_finite;

const LOGISTIC_BOOTSTRAP_CONFIG: BootstrapConfig =
    BootstrapConfig::new(DEFAULT_BOOTSTRAP_RESAMPLES, DEFAULT_BOOTSTRAP_SEED);
pub const DEFAULT_ASSAY_SEEDS: [u64; 5] = [20_260_612, 7, 101, 2_024, 99_999];
pub const DEFAULT_HOLDOUT_FRACTION: f32 = 0.2;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LogisticProbeReport {
    pub estimate: MiEstimate,
    pub accuracy: f32,
    pub selected_field: &'static str,
}

pub fn logistic_probe_mi(samples: &[Vec<f32>], labels: &[bool]) -> Result<LogisticProbeReport> {
    logistic_probe_mi_with_trust(samples, labels, TrustTag::Provisional)
}

pub fn logistic_probe_mi_calibrated(
    samples: &[Vec<f32>],
    labels: &[bool],
) -> Result<LogisticProbeReport> {
    ensure_informative_binary_labels(labels)?;
    let calibration = logistic_power_calibration(samples, labels, None, TrustTag::Provisional)?;
    let mut report = logistic_probe_mi_with_trust(samples, labels, TrustTag::Provisional)?;
    report.estimate = report.estimate.with_power_calibration(calibration);
    Ok(report)
}

pub fn logistic_probe_mi_with_anchor(
    samples: &[Vec<f32>],
    labels: &[bool],
    anchor: &Anchor,
) -> Result<LogisticProbeReport> {
    logistic_probe_mi_with_trust(samples, labels, trust_for_anchor(Some(anchor)))
}

pub fn logistic_probe_mi_multiseed(
    samples: &[Vec<f32>],
    labels: &[bool],
    groups: Option<&[String]>,
) -> Result<LogisticProbeReport> {
    logistic_probe_mi_multiseed_with_trust(samples, labels, groups, TrustTag::Provisional)
}

pub fn logistic_probe_mi_multiseed_calibrated(
    samples: &[Vec<f32>],
    labels: &[bool],
    groups: Option<&[String]>,
) -> Result<LogisticProbeReport> {
    logistic_probe_mi_multiseed_calibrated_with_trust(
        samples,
        labels,
        groups,
        TrustTag::Provisional,
    )
}

pub fn logistic_probe_mi_multiseed_with_anchor(
    samples: &[Vec<f32>],
    labels: &[bool],
    groups: Option<&[String]>,
    anchor: &Anchor,
) -> Result<LogisticProbeReport> {
    logistic_probe_mi_multiseed_with_trust(samples, labels, groups, trust_for_anchor(Some(anchor)))
}

pub fn logistic_probe_mi_multiseed_calibrated_with_anchor(
    samples: &[Vec<f32>],
    labels: &[bool],
    groups: Option<&[String]>,
    anchor: &Anchor,
) -> Result<LogisticProbeReport> {
    logistic_probe_mi_multiseed_calibrated_with_trust(
        samples,
        labels,
        groups,
        trust_for_anchor(Some(anchor)),
    )
}

pub(crate) fn logistic_probe_mi_with_min_samples(
    samples: &[Vec<f32>],
    labels: &[bool],
    min_samples: usize,
) -> Result<LogisticProbeReport> {
    logistic_probe_mi_with_trust_and_min_samples(
        samples,
        labels,
        TrustTag::Provisional,
        min_samples,
    )
}

pub(crate) fn logistic_probe_mi_with_anchor_and_min_samples(
    samples: &[Vec<f32>],
    labels: &[bool],
    anchor: &Anchor,
    min_samples: usize,
) -> Result<LogisticProbeReport> {
    logistic_probe_mi_with_trust_and_min_samples(
        samples,
        labels,
        trust_for_anchor(Some(anchor)),
        min_samples,
    )
}

fn logistic_probe_mi_with_trust(
    samples: &[Vec<f32>],
    labels: &[bool],
    trust: TrustTag,
) -> Result<LogisticProbeReport> {
    logistic_probe_mi_with_trust_and_min_samples(samples, labels, trust, MIN_ASSAY_SAMPLES)
}

fn logistic_probe_mi_multiseed_with_trust(
    samples: &[Vec<f32>],
    labels: &[bool],
    groups: Option<&[String]>,
    trust: TrustTag,
) -> Result<LogisticProbeReport> {
    if samples.len() != labels.len() || samples.len() < MIN_ASSAY_SAMPLES {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "need at least {MIN_ASSAY_SAMPLES} labeled samples"
        )));
    }
    let dim = validate_rectangular_finite("logistic", samples)?;
    let owned_groups;
    let groups = match groups {
        Some(groups) => groups,
        None => {
            owned_groups = row_groups(labels.len());
            &owned_groups
        }
    };
    let mut seed_summaries = Vec::with_capacity(DEFAULT_ASSAY_SEEDS.len());
    for seed in DEFAULT_ASSAY_SEEDS {
        let split = group_holdout_split(labels, groups, DEFAULT_HOLDOUT_FRACTION, seed)?;
        seed_summaries.push(logistic_heldout_summary(samples, labels, dim, &split));
    }
    let seed_bits = seed_summaries
        .iter()
        .map(|summary| summary.bits)
        .collect::<Vec<_>>();
    let bits = mean(&seed_bits);
    let seed_sigma = sample_sigma(&seed_bits);
    let (ci_low, ci_high) = seed_ci(bits, seed_sigma, seed_bits.len());
    let reliability =
        EstimateReliability::new(seed_bits.len(), seed_sigma, seed_sigma >= bits.abs())?;
    Ok(LogisticProbeReport {
        estimate: MiEstimate::new(
            bits,
            ci_low,
            ci_high,
            labels.len(),
            EstimatorKind::LogisticProbe,
            trust,
        )
        .with_reliability(reliability),
        accuracy: mean(
            &seed_summaries
                .iter()
                .map(|summary| summary.accuracy)
                .collect::<Vec<_>>(),
        ),
        selected_field: "logistic_probe_multiseed_group_holdout",
    })
}

fn logistic_probe_mi_multiseed_calibrated_with_trust(
    samples: &[Vec<f32>],
    labels: &[bool],
    groups: Option<&[String]>,
    trust: TrustTag,
) -> Result<LogisticProbeReport> {
    ensure_informative_binary_labels(labels)?;
    let calibration = logistic_power_calibration(samples, labels, groups, trust)?;
    let mut report = logistic_probe_mi_multiseed_with_trust(samples, labels, groups, trust)?;
    report.estimate = report.estimate.with_power_calibration(calibration);
    Ok(report)
}

fn logistic_probe_mi_with_trust_and_min_samples(
    samples: &[Vec<f32>],
    labels: &[bool],
    trust: TrustTag,
    min_samples: usize,
) -> Result<LogisticProbeReport> {
    if samples.len() != labels.len() || samples.len() < min_samples {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "need at least {min_samples} labeled samples"
        )));
    }
    let dim = validate_rectangular_finite("logistic", samples)?;
    let summary = logistic_summary(samples, labels, dim);
    let ci = bootstrap_paired_ci(
        samples,
        labels,
        summary.bits,
        LOGISTIC_BOOTSTRAP_CONFIG,
        |sampled_samples, sampled_labels| {
            let dim = validate_rectangular_finite("logistic", sampled_samples)?;
            Ok(logistic_summary(sampled_samples, sampled_labels, dim).bits)
        },
    )?
    .ok_or_else(|| CalyxError::assay_insufficient_samples("bootstrap CI requires samples"))?;
    Ok(LogisticProbeReport {
        estimate: MiEstimate::new(
            summary.bits,
            ci.ci_low,
            ci.ci_high,
            labels.len(),
            EstimatorKind::LogisticProbe,
            trust,
        ),
        accuracy: summary.accuracy,
        selected_field: "logistic_probe",
    })
}

struct LogisticSummary {
    bits: f32,
    accuracy: f32,
}

fn logistic_summary(samples: &[Vec<f32>], labels: &[bool], dim: usize) -> LogisticSummary {
    let (pos_mean, neg_mean) = class_means(samples, labels, dim);
    let direction: Vec<f32> = pos_mean
        .iter()
        .zip(&neg_mean)
        .map(|(pos, neg)| pos - neg)
        .collect();
    let midpoint: Vec<f32> = pos_mean
        .iter()
        .zip(&neg_mean)
        .map(|(pos, neg)| (pos + neg) * 0.5)
        .collect();
    let threshold = dot(&midpoint, &direction);
    let predictions: Vec<bool> = samples
        .iter()
        .map(|row| dot(row, &direction) >= threshold)
        .collect();
    let accuracy = predictions
        .iter()
        .zip(labels)
        .filter(|(prediction, label)| **prediction == **label)
        .count() as f32
        / labels.len() as f32;
    let bits = binary_mi(labels, &predictions);
    LogisticSummary { bits, accuracy }
}

fn logistic_heldout_summary(
    samples: &[Vec<f32>],
    labels: &[bool],
    dim: usize,
    split: &GroupSplit,
) -> LogisticSummary {
    let train_samples = split
        .train
        .iter()
        .map(|&idx| samples[idx].clone())
        .collect::<Vec<_>>();
    let train_labels = split
        .train
        .iter()
        .map(|&idx| labels[idx])
        .collect::<Vec<_>>();
    let test_samples = split
        .test
        .iter()
        .map(|&idx| samples[idx].clone())
        .collect::<Vec<_>>();
    let test_labels = split
        .test
        .iter()
        .map(|&idx| labels[idx])
        .collect::<Vec<_>>();
    logistic_train_test_summary(
        &train_samples,
        &train_labels,
        &test_samples,
        &test_labels,
        dim,
    )
}

fn logistic_train_test_summary(
    train_samples: &[Vec<f32>],
    train_labels: &[bool],
    test_samples: &[Vec<f32>],
    test_labels: &[bool],
    dim: usize,
) -> LogisticSummary {
    let (pos_mean, neg_mean) = class_means(train_samples, train_labels, dim);
    let direction: Vec<f32> = pos_mean
        .iter()
        .zip(&neg_mean)
        .map(|(pos, neg)| pos - neg)
        .collect();
    let midpoint: Vec<f32> = pos_mean
        .iter()
        .zip(&neg_mean)
        .map(|(pos, neg)| (pos + neg) * 0.5)
        .collect();
    let threshold = dot(&midpoint, &direction);
    let predictions = test_samples
        .iter()
        .map(|row| dot(row, &direction) >= threshold)
        .collect::<Vec<_>>();
    let accuracy = predictions
        .iter()
        .zip(test_labels)
        .filter(|(prediction, label)| **prediction == **label)
        .count() as f32
        / test_labels.len().max(1) as f32;
    LogisticSummary {
        bits: binary_mi(test_labels, &predictions),
        accuracy,
    }
}

fn class_means(samples: &[Vec<f32>], labels: &[bool], dim: usize) -> (Vec<f32>, Vec<f32>) {
    let mut pos = vec![0.0; dim];
    let mut neg = vec![0.0; dim];
    let mut pos_n = 0_usize;
    let mut neg_n = 0_usize;
    for (row, label) in samples.iter().zip(labels) {
        let target = if *label {
            pos_n += 1;
            &mut pos
        } else {
            neg_n += 1;
            &mut neg
        };
        for (slot, value) in target.iter_mut().zip(row) {
            *slot += value;
        }
    }
    scale(&mut pos, pos_n);
    scale(&mut neg, neg_n);
    (pos, neg)
}

fn scale(values: &mut [f32], count: usize) {
    let count = count.max(1) as f32;
    for value in values {
        *value /= count;
    }
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(left, right)| left * right).sum()
}

fn mean(values: &[f32]) -> f32 {
    values.iter().sum::<f32>() / values.len().max(1) as f32
}

fn sample_sigma(values: &[f32]) -> f32 {
    if values.len() < 2 {
        return 0.0;
    }
    let mean = mean(values);
    let variance = values
        .iter()
        .map(|value| {
            let delta = *value - mean;
            delta * delta
        })
        .sum::<f32>()
        / (values.len() - 1) as f32;
    variance.sqrt()
}

fn seed_ci(mean: f32, sigma: f32, n: usize) -> (f32, f32) {
    let t = match n.saturating_sub(1) {
        0 => 0.0,
        1 => 12.706,
        2 => 4.303,
        3 => 3.182,
        4 => 2.776,
        _ => 1.960,
    };
    let half_width = t * sigma / (n.max(1) as f32).sqrt();
    ((mean - half_width).max(0.0), mean + half_width)
}

fn binary_mi(labels: &[bool], predictions: &[bool]) -> f32 {
    let n = labels.len().max(1) as f32;
    let mut joint = [[0.0_f32; 2]; 2];
    for (label, prediction) in labels.iter().zip(predictions) {
        joint[*label as usize][*prediction as usize] += 1.0;
    }
    let py = [
        (joint[0][0] + joint[0][1]) / n,
        (joint[1][0] + joint[1][1]) / n,
    ];
    let pp = [
        (joint[0][0] + joint[1][0]) / n,
        (joint[0][1] + joint[1][1]) / n,
    ];
    let mut mi = 0.0;
    for y in 0..2 {
        for p in 0..2 {
            let joint_p = joint[y][p] / n;
            if joint_p > 0.0 && py[y] > 0.0 && pp[p] > 0.0 {
                mi += joint_p * (joint_p / (py[y] * pp[p])).log2();
            }
        }
    }
    mi.max(0.0)
}

fn logistic_power_calibration(
    samples: &[Vec<f32>],
    labels: &[bool],
    groups: Option<&[String]>,
    trust: TrustTag,
) -> Result<PowerCalibration> {
    let planted_bits = ensure_informative_binary_labels(labels)?;
    let dim = validate_rectangular_finite("logistic power calibration", samples)?;
    if dim == 0 {
        return Err(crate::calibration::underpowered(
            "power calibration requires at least one feature column",
        ));
    }
    let planted_column = dim - 1;
    let planted = plant_binary_signal(samples, labels, planted_column);
    let report = match groups {
        Some(groups) => {
            logistic_probe_mi_multiseed_with_trust(&planted, labels, Some(groups), trust)?
        }
        None => logistic_probe_mi_with_trust(&planted, labels, trust)?,
    };
    let calibration = PowerCalibration::new(
        planted_bits,
        report.estimate.bits,
        DEFAULT_MIN_POWER_RECOVERY_RATIO,
        labels.len(),
        dim,
        planted_column,
    )?;
    calibration.require_passed()?;
    Ok(calibration)
}

fn plant_binary_signal(samples: &[Vec<f32>], labels: &[bool], column: usize) -> Vec<Vec<f32>> {
    let mut planted = samples.to_vec();
    for (row, label) in planted.iter_mut().zip(labels) {
        row[column] = if *label { 1.0 } else { -1.0 };
    }
    planted
}
