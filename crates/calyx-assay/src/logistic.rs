//! Binary outcome logistic-probe MI estimator.

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

use crate::estimate::{EstimatorKind, MiEstimate, TrustTag};
use crate::ksg::MIN_ASSAY_SAMPLES;
use crate::samples::validate_rectangular_finite;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LogisticProbeReport {
    pub estimate: MiEstimate,
    pub accuracy: f32,
    pub selected_field: &'static str,
}

pub fn logistic_probe_mi(samples: &[Vec<f32>], labels: &[bool]) -> Result<LogisticProbeReport> {
    if samples.len() != labels.len() || samples.len() < MIN_ASSAY_SAMPLES {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "need at least {MIN_ASSAY_SAMPLES} labeled samples"
        )));
    }
    let dim = validate_rectangular_finite("logistic", samples)?;
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
    Ok(LogisticProbeReport {
        estimate: MiEstimate::new(
            bits,
            (bits - 0.05).max(0.0),
            bits + 0.05,
            labels.len(),
            EstimatorKind::LogisticProbe,
            TrustTag::Trusted,
        ),
        accuracy,
        selected_field: "logistic_probe",
    })
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
