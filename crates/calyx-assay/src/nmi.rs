//! Partitioned histogram normalized mutual information.

use std::collections::BTreeMap;

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

use crate::ksg::MIN_ASSAY_SAMPLES;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NmiReport {
    pub nmi: f32,
    pub mi_bits: f32,
    pub x_entropy_bits: f32,
    pub y_entropy_bits: f32,
    pub bins: usize,
    pub n_samples: usize,
}

pub fn partitioned_histogram_nmi(x: &[f32], y: &[f32], bins: usize) -> Result<NmiReport> {
    validate_nmi_samples(x, y)?;
    let bins = bins.max(2);
    let xb = bin_values(x, bins);
    let yb = bin_values(y, bins);
    let hx = entropy(&xb);
    let hy = entropy(&yb);
    let joint: Vec<_> = xb
        .iter()
        .zip(&yb)
        .map(|(left, right)| (*left, *right))
        .collect();
    let hxy = entropy(&joint);
    let mi = (hx + hy - hxy).max(0.0);
    let denom = (hx * hy).sqrt();
    Ok(NmiReport {
        nmi: if denom > 0.0 { mi / denom } else { 0.0 },
        mi_bits: mi,
        x_entropy_bits: hx,
        y_entropy_bits: hy,
        bins,
        n_samples: x.len(),
    })
}

fn validate_nmi_samples(x: &[f32], y: &[f32]) -> Result<()> {
    if x.len() != y.len() {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "NMI requires paired samples: x={} y={}",
            x.len(),
            y.len()
        )));
    }
    if x.len() < MIN_ASSAY_SAMPLES {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "NMI requires at least {MIN_ASSAY_SAMPLES} paired samples; got {}",
            x.len()
        )));
    }
    ensure_finite("x", x)?;
    ensure_finite("y", y)?;
    Ok(())
}

fn ensure_finite(name: &str, values: &[f32]) -> Result<()> {
    if let Some((idx, _)) = values
        .iter()
        .enumerate()
        .find(|(_, value)| !value.is_finite())
    {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "NMI {name} sample {idx} contains NaN or infinity"
        )));
    }
    Ok(())
}

fn bin_values(values: &[f32], bins: usize) -> Vec<usize> {
    if values.is_empty() {
        return Vec::new();
    }
    let min = values.iter().copied().fold(f32::INFINITY, f32::min);
    let max = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let width = (max - min).max(f32::EPSILON);
    values
        .iter()
        .map(|value| {
            let scaled = ((*value - min) / width * bins as f32).floor() as usize;
            scaled.min(bins - 1)
        })
        .collect()
}

fn entropy<T>(values: &[T]) -> f32
where
    T: Ord + Copy,
{
    let mut counts = BTreeMap::<T, usize>::new();
    for value in values {
        *counts.entry(*value).or_default() += 1;
    }
    let n = values.len().max(1) as f32;
    counts
        .values()
        .map(|count| {
            let p = *count as f32 / n;
            -p * p.log2()
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nmi_redundant_signal_is_high_and_independent_is_low() -> Result<()> {
        let x: Vec<f32> = (0..100).map(|i| (i % 10) as f32).collect();
        let y: Vec<f32> = (0..100).map(|i| (i / 10) as f32).collect();

        let redundant = partitioned_histogram_nmi(&x, &x, 10)?;
        let independent = partitioned_histogram_nmi(&x, &y, 10)?;

        assert!(redundant.nmi >= 0.8);
        assert!(independent.nmi <= 0.1);
        Ok(())
    }

    #[test]
    fn nmi_mismatched_samples_fail_closed() {
        let err = partitioned_histogram_nmi(&[1.0, 2.0], &[1.0], 10)
            .expect_err("mismatched paired samples must fail closed");

        assert_eq!(err.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
        assert!(err.message.contains("x=2 y=1"));
    }

    #[test]
    fn nmi_quorum_and_nonfinite_inputs_fail_closed() {
        let empty =
            partitioned_histogram_nmi(&[], &[], 10).expect_err("empty NMI input must fail closed");
        assert_eq!(empty.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
        assert!(empty.message.contains("got 0"));

        let short: Vec<f32> = (0..30).map(|value| value as f32).collect();
        let err = partitioned_histogram_nmi(&short, &short, 10)
            .expect_err("short NMI input must fail closed");
        assert_eq!(err.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
        assert!(err.message.contains("got 30"));

        let exact: Vec<f32> = (0..MIN_ASSAY_SAMPLES)
            .map(|value| (value % 10) as f32)
            .collect();
        let report =
            partitioned_histogram_nmi(&exact, &exact, 10).expect("n=50 should meet the NMI quorum");
        assert_eq!(report.n_samples, MIN_ASSAY_SAMPLES);

        let mut nonfinite = exact.clone();
        nonfinite[7] = f32::NAN;
        let err = partitioned_histogram_nmi(&nonfinite, &exact, 10)
            .expect_err("NaN NMI input must fail closed");
        assert_eq!(err.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
        assert!(err.message.contains("sample 7"));
    }
}
