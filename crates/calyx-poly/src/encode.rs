//! Deterministic, embedder-free encoders that turn raw numbers into slot-vectors.
//!
//! Loom's cross-terms and Sextant's kNN use cosine similarity. Raw floats make cosine meaningless
//! (`cos(0.008, 0.009) ≈ 1` for any two small prices). These encoders give a scalar a *geometry*:
//!
//! - [`RffEncoder`] — Random Fourier Features. Maps a scalar to a vector whose inner product
//!   approximates an RBF (Gaussian) kernel `exp(-(x-y)²/(2σ²))`, so "close values are similar".
//!   Frequencies and phases are sampled from a fixed seed, so the encoder is **frozen and
//!   reproducible** — a requirement for a lens and for auditable forecasts.
//! - [`QuantileEncoder`] — piecewise-linear / thermometer encoding over empirical quantile edges,
//!   for heavy-tailed features (volume, liquidity).
//! - [`one_hot`] / [`signed_log`] / [`l2_normalize`] — supporting transforms.
//!
//! All math is pure and deterministic (no `rand`, no floats-from-time); a tiny splitmix64 PRNG plus
//! Box–Muller supplies the frozen randomness.

use std::f64::consts::PI;

/// Deterministic splitmix64 PRNG. Used only to *freeze* encoder parameters at construction; it never
/// touches per-input data, so encoders remain byte-for-byte reproducible.
#[derive(Debug, Clone)]
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    fn next_f64(&mut self) -> f64 {
        // 53-bit mantissa precision.
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Standard normal via Box–Muller.
    fn next_gaussian(&mut self) -> f64 {
        // Guard against log(0).
        let u1 = (self.next_f64()).max(1.0e-12);
        let u2 = self.next_f64();
        (-2.0 * u1.ln()).sqrt() * (2.0 * PI * u2).cos()
    }
}

/// L2-normalizes a vector in place. A zero/degenerate vector is left as-is (callers treat an
/// all-zero encode as an absent signal rather than a direction).
pub fn l2_normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm.is_finite() && norm > f32::EPSILON {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Signed log transform `sign(x)·ln(1+|x|)` — compresses heavy tails while preserving sign and
/// monotonicity. Non-finite input returns 0.0.
pub fn signed_log(x: f64) -> f64 {
    if !x.is_finite() {
        return 0.0;
    }
    x.signum() * (1.0 + x.abs()).ln()
}

/// One-hot encodes `index` into an `n`-dim dense vector. Out-of-range or `n == 0` yields all zeros
/// (an absent categorical), which callers map to `Absent`.
pub fn one_hot(index: usize, n: usize) -> Vec<f32> {
    let mut v = vec![0.0_f32; n];
    if n > 0 && index < n {
        v[index] = 1.0;
    }
    v
}

/// Random Fourier Feature encoder for a single continuous scalar.
///
/// `feature_i(x) = sqrt(2/dim) · cos(freq_i · x + phase_i)`, then L2-normalized. With frequencies
/// `freq_i ~ N(0, 1/σ²)` and phases `phase_i ~ U(0, 2π)`, `⟨φ(x), φ(y)⟩ ≈ exp(-(x-y)²/(2σ²))`.
#[derive(Debug, Clone)]
pub struct RffEncoder {
    dim: usize,
    freqs: Vec<f64>,
    phases: Vec<f64>,
}

impl RffEncoder {
    /// Builds a frozen encoder. `seed` fixes the frequencies/phases; `dim` is the output dimension;
    /// `sigma` is the RBF length scale (in the input's own units). `dim` is clamped to ≥ 1 and
    /// `sigma` to a small positive floor.
    pub fn new(seed: u64, dim: usize, sigma: f64) -> Self {
        let dim = dim.max(1);
        let sigma = if sigma.is_finite() && sigma > 1.0e-9 {
            sigma
        } else {
            1.0e-3
        };
        let mut prng = SplitMix64::new(seed);
        let mut freqs = Vec::with_capacity(dim);
        let mut phases = Vec::with_capacity(dim);
        for _ in 0..dim {
            // N(0, 1/σ²): standard normal scaled by 1/σ.
            freqs.push(prng.next_gaussian() / sigma);
            phases.push(prng.next_f64() * 2.0 * PI);
        }
        Self { dim, freqs, phases }
    }

    /// Output dimension.
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Encodes a scalar. Non-finite input returns an all-zero vector (an absent signal).
    pub fn encode(&self, x: f64) -> Vec<f32> {
        if !x.is_finite() {
            return vec![0.0_f32; self.dim];
        }
        let scale = (2.0 / self.dim as f64).sqrt();
        let mut out: Vec<f32> = self
            .freqs
            .iter()
            .zip(self.phases.iter())
            .map(|(&w, &b)| (scale * (w * x + b).cos()) as f32)
            .collect();
        l2_normalize(&mut out);
        out
    }
}

/// Piecewise-linear ("thermometer with a fractional top bin") quantile encoder for heavy-tailed
/// features. Given sorted quantile edges `b_0 < b_1 < … < b_T`, produces a `T`-dim vector where
/// component `t` is how far `x` has filled bin `[b_t, b_{t+1})` (0 below, 1 above, linear inside).
/// A trailing constant term is appended before normalization so a low value still has a defined
/// direction (avoids a zero vector).
#[derive(Debug, Clone)]
pub struct QuantileEncoder {
    edges: Vec<f64>,
}

impl QuantileEncoder {
    /// Builds an encoder from sorted, strictly-increasing `edges`. Fewer than two edges falls back
    /// to `[0.0, 1.0]` (a single bin).
    pub fn new(mut edges: Vec<f64>) -> Self {
        edges.retain(|e| e.is_finite());
        edges.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        edges.dedup();
        if edges.len() < 2 {
            edges = vec![0.0, 1.0];
        }
        Self { edges }
    }

    /// Output dimension: number of bins (`edges.len() - 1`) plus the trailing bias term.
    pub fn dim(&self) -> usize {
        (self.edges.len() - 1) + 1
    }

    /// Encodes a scalar into the piecewise-linear fill vector, L2-normalized.
    pub fn encode(&self, x: f64) -> Vec<f32> {
        let bins = self.edges.len() - 1;
        let mut out = vec![0.0_f32; bins + 1];
        if !x.is_finite() {
            return out;
        }
        for (t, slot) in out.iter_mut().enumerate().take(bins) {
            let lo = self.edges[t];
            let hi = self.edges[t + 1];
            let fill = if x >= hi {
                1.0
            } else if x <= lo {
                0.0
            } else if hi > lo {
                (x - lo) / (hi - lo)
            } else {
                0.0
            };
            *slot = fill as f32;
        }
        out[bins] = 1.0; // bias term
        l2_normalize(&mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        if na < f32::EPSILON || nb < f32::EPSILON {
            0.0
        } else {
            dot / (na * nb)
        }
    }

    #[test]
    fn rff_is_deterministic() {
        let e1 = RffEncoder::new(42, 32, 0.05);
        let e2 = RffEncoder::new(42, 32, 0.05);
        assert_eq!(e1.encode(0.37), e2.encode(0.37));
    }

    #[test]
    fn rff_close_values_more_similar_than_far() {
        // Prices near 0.5, length scale 0.05.
        let e = RffEncoder::new(7, 128, 0.05);
        let base = e.encode(0.50);
        let near = e.encode(0.52);
        let far = e.encode(0.90);
        let sim_near = cosine(&base, &near);
        let sim_far = cosine(&base, &far);
        assert!(
            sim_near > sim_far,
            "near {sim_near} should exceed far {sim_far}"
        );
        // Identical inputs are ~1.0.
        assert!((cosine(&base, &e.encode(0.50)) - 1.0).abs() < 1.0e-4);
    }

    #[test]
    fn rff_is_unit_norm() {
        let e = RffEncoder::new(1, 16, 0.1);
        let v = e.encode(0.3);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1.0e-4);
    }

    #[test]
    fn rff_non_finite_is_zero() {
        let e = RffEncoder::new(1, 8, 0.1);
        assert!(e.encode(f64::NAN).iter().all(|&x| x == 0.0));
    }

    #[test]
    fn quantile_is_monotone_fill() {
        let q = QuantileEncoder::new(vec![0.0, 10.0, 100.0, 1000.0]);
        let low = q.encode(5.0);
        let high = q.encode(500.0);
        // Higher value fills more bins → larger total fill (excluding bias term).
        let fill_low: f32 = low[..low.len() - 1].iter().sum();
        let fill_high: f32 = high[..high.len() - 1].iter().sum();
        assert!(fill_high > fill_low);
        assert_eq!(q.encode(5.0), q.encode(5.0));
    }

    #[test]
    fn signed_log_preserves_sign_and_monotonicity() {
        assert!(signed_log(-100.0) < 0.0);
        assert!(signed_log(100.0) > 0.0);
        assert!(signed_log(1000.0) > signed_log(10.0));
        assert_eq!(signed_log(f64::INFINITY), 0.0);
    }

    #[test]
    fn one_hot_basic() {
        assert_eq!(one_hot(1, 3), vec![0.0, 1.0, 0.0]);
        assert_eq!(one_hot(5, 3), vec![0.0, 0.0, 0.0]); // out of range → absent
    }
}
