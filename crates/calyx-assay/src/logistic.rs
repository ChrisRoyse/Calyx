//! Binary outcome logistic-probe MI estimator.

use calyx_core::{Anchor, CalyxError, Result};
use serde::{Deserialize, Serialize};

use crate::bootstrap::{
    BootstrapConfig, DEFAULT_BOOTSTRAP_RESAMPLES, DEFAULT_BOOTSTRAP_SEED, bootstrap_paired_ci,
};
use crate::estimate::{EstimatorKind, MiEstimate, TrustTag, trust_for_anchor};
use crate::ksg::MIN_ASSAY_SAMPLES;
use crate::samples::validate_rectangular_finite;

const LOGISTIC_BOOTSTRAP_CONFIG: BootstrapConfig =
    BootstrapConfig::new(DEFAULT_BOOTSTRAP_RESAMPLES, DEFAULT_BOOTSTRAP_SEED);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LogisticProbeReport {
    pub estimate: MiEstimate,
    pub accuracy: f32,
    pub selected_field: &'static str,
}

pub fn logistic_probe_mi(samples: &[Vec<f32>], labels: &[bool]) -> Result<LogisticProbeReport> {
    logistic_probe_mi_with_trust(samples, labels, TrustTag::Provisional)
}

pub fn logistic_probe_mi_with_anchor(
    samples: &[Vec<f32>],
    labels: &[bool],
    anchor: &Anchor,
) -> Result<LogisticProbeReport> {
    logistic_probe_mi_with_trust(samples, labels, trust_for_anchor(Some(anchor)))
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
