//! Partitioned histogram normalized mutual information.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NmiReport {
    pub nmi: f32,
    pub mi_bits: f32,
    pub x_entropy_bits: f32,
    pub y_entropy_bits: f32,
    pub bins: usize,
    pub n_samples: usize,
}

pub fn partitioned_histogram_nmi(x: &[f32], y: &[f32], bins: usize) -> NmiReport {
    assert_eq!(x.len(), y.len(), "NMI requires paired samples");
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
    NmiReport {
        nmi: if denom > 0.0 { mi / denom } else { 0.0 },
        mi_bits: mi,
        x_entropy_bits: hx,
        y_entropy_bits: hy,
        bins,
        n_samples: x.len(),
    }
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
