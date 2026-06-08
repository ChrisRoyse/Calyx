//! Shared Assay estimate types.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustTag {
    Trusted,
    Provisional,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstimatorKind {
    Ksg,
    HistogramNmi,
    LogisticProbe,
    Bootstrap,
    PanelSufficiency,
    PairGain,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MiEstimate {
    pub bits: f32,
    pub ci_low: f32,
    pub ci_high: f32,
    pub n_samples: usize,
    pub estimator: EstimatorKind,
    pub trust: TrustTag,
}

impl MiEstimate {
    pub fn new(
        bits: f32,
        ci_low: f32,
        ci_high: f32,
        n_samples: usize,
        estimator: EstimatorKind,
        trust: TrustTag,
    ) -> Self {
        let bits = bits.max(0.0);
        let ci_low = ci_low.min(bits).max(0.0);
        let ci_high = ci_high.max(bits);
        Self {
            bits,
            ci_low,
            ci_high,
            n_samples,
            estimator,
            trust,
        }
    }

    pub fn point(bits: f32, n_samples: usize, estimator: EstimatorKind, trust: TrustTag) -> Self {
        let band = (bits.abs() * 0.15).max(0.02);
        Self::new(bits, bits - band, bits + band, n_samples, estimator, trust)
    }
}
