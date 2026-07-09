use std::sync::OnceLock;

use crate::quant::QuantLevel;

use super::level_steps;

const SQRT_2: f64 = std::f64::consts::SQRT_2;
const INV_SQRT_2PI: f64 = 0.398_942_280_401_432_7;

#[derive(Debug)]
pub(super) struct LloydCodebook {
    centroids: Vec<f32>,
    thresholds: Vec<f32>,
}

static BITS3P5_CODEBOOK: OnceLock<LloydCodebook> = OnceLock::new();
static BITS2P5_CODEBOOK: OnceLock<LloydCodebook> = OnceLock::new();

pub(super) fn quantize_unit(value: f32, level: QuantLevel) -> u16 {
    let thresholds = &codebook(level).thresholds;
    thresholds.partition_point(|threshold| value > *threshold) as u16
}

pub(super) fn centroid(level: QuantLevel, code: u16) -> f32 {
    codebook(level).centroids[usize::from(code)]
}

pub(super) fn centroid_product_sum(level: QuantLevel, left: &[u8], right: &[u8]) -> f32 {
    let centroids = &codebook(level).centroids;
    left.iter()
        .zip(right.iter())
        .map(|(left, right)| centroids[usize::from(*left)] * centroids[usize::from(*right)])
        .sum()
}

fn codebook(level: QuantLevel) -> &'static LloydCodebook {
    match level {
        QuantLevel::Bits3p5 => BITS3P5_CODEBOOK
            .get_or_init(|| build_standard_normal_codebook(usize::from(level_steps(level)))),
        QuantLevel::Bits2p5 => BITS2P5_CODEBOOK
            .get_or_init(|| build_standard_normal_codebook(usize::from(level_steps(level)))),
        _ => unreachable!("TurboQuant level validated before Lloyd-Max lookup"),
    }
}

fn build_standard_normal_codebook(levels: usize) -> LloydCodebook {
    debug_assert!(levels >= 2);
    let tail = if levels <= 5 { 2.5 } else { 4.0 };
    let mut centroids = (0..levels)
        .map(|idx| -tail + (2.0 * tail * idx as f64 / (levels - 1) as f64))
        .collect::<Vec<_>>();
    for _ in 0..100 {
        let thresholds = thresholds(&centroids);
        let next = (0..levels)
            .map(|idx| {
                let left = if idx == 0 {
                    f64::NEG_INFINITY
                } else {
                    thresholds[idx - 1]
                };
                let right = if idx + 1 == levels {
                    f64::INFINITY
                } else {
                    thresholds[idx]
                };
                interval_centroid(left, right)
            })
            .collect::<Vec<_>>();
        let max_delta = centroids
            .iter()
            .zip(next.iter())
            .map(|(left, right)| (left - right).abs())
            .fold(0.0_f64, f64::max);
        centroids = next;
        if max_delta <= 1e-10 {
            break;
        }
    }
    let thresholds = thresholds(&centroids);
    LloydCodebook {
        centroids: centroids.into_iter().map(|value| value as f32).collect(),
        thresholds: thresholds.into_iter().map(|value| value as f32).collect(),
    }
}

fn thresholds(centroids: &[f64]) -> Vec<f64> {
    centroids
        .windows(2)
        .map(|pair| 0.5 * (pair[0] + pair[1]))
        .collect()
}

fn interval_centroid(left: f64, right: f64) -> f64 {
    let mass = normal_cdf(right) - normal_cdf(left);
    if mass <= 1e-14 {
        return finite_midpoint(left, right);
    }
    (normal_pdf(left) - normal_pdf(right)) / mass
}

fn finite_midpoint(left: f64, right: f64) -> f64 {
    match (left.is_finite(), right.is_finite()) {
        (true, true) => 0.5 * (left + right),
        (true, false) => left,
        (false, true) => right,
        (false, false) => 0.0,
    }
}

fn normal_pdf(value: f64) -> f64 {
    if value.is_finite() {
        INV_SQRT_2PI * (-0.5 * value * value).exp()
    } else {
        0.0
    }
}

fn normal_cdf(value: f64) -> f64 {
    if value == f64::NEG_INFINITY {
        return 0.0;
    }
    if value == f64::INFINITY {
        return 1.0;
    }
    0.5 * (1.0 + erf(value / SQRT_2))
}

fn erf(value: f64) -> f64 {
    let sign = if value < 0.0 { -1.0 } else { 1.0 };
    let x = value.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly = (((((1.061_405_429 * t - 1.453_152_027) * t) + 1.421_413_741) * t - 0.284_496_736)
        * t
        + 0.254_829_592)
        * t;
    sign * (1.0 - poly * (-x * x).exp())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_bit_codebooks_match_known_standard_normal_centroids() {
        let two = build_standard_normal_codebook(2);
        let b1 = (2.0 / std::f64::consts::PI).sqrt() as f32;
        assert!((two.centroids[0] + b1).abs() <= 0.005);
        assert!((two.centroids[1] - b1).abs() <= 0.005);

        let four = build_standard_normal_codebook(4);
        let expected = [-1.51_f32, -0.453, 0.453, 1.51];
        for (actual, expected) in four.centroids.iter().zip(expected.iter()) {
            assert!((actual - expected).abs() <= 0.03, "{actual} {expected}");
        }
    }

    #[test]
    fn lloyd_thresholds_are_ordered_and_symmetric() {
        let book = build_standard_normal_codebook(5);
        assert!(book.thresholds.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(book.centroids.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(book.centroids[2].abs() <= 1e-5);
        assert!((book.centroids[0] + book.centroids[4]).abs() <= 1e-5);
        assert!((book.centroids[1] + book.centroids[3]).abs() <= 1e-5);
    }
}
