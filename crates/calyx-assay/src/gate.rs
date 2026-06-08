//! AssayGate facade for lens signal and pair gain.

use calyx_core::Result;
use serde::{Deserialize, Serialize};

use crate::estimate::{EstimatorKind, MiEstimate, TrustTag};
use crate::logistic::logistic_probe_mi;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LensSignal {
    pub estimate: MiEstimate,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PairGain {
    pub left_bits: f32,
    pub right_bits: f32,
    pub pair_bits: f32,
    pub gain_bits: f32,
    pub n_samples: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssayGate {
    pub min_samples: usize,
}

impl Default for AssayGate {
    fn default() -> Self {
        Self { min_samples: 50 }
    }
}

impl AssayGate {
    pub fn lens_signal(&self, samples: &[Vec<f32>], labels: &[bool]) -> Result<LensSignal> {
        let report = logistic_probe_mi(samples, labels)?;
        Ok(LensSignal {
            estimate: report.estimate,
        })
    }

    pub fn pair_gain(
        &self,
        left: &[Vec<f32>],
        right: &[Vec<f32>],
        labels: &[bool],
    ) -> Result<PairGain> {
        let left_bits = self.lens_signal(left, labels)?.estimate.bits;
        let right_bits = self.lens_signal(right, labels)?.estimate.bits;
        let combined: Vec<Vec<f32>> = left
            .iter()
            .zip(right)
            .map(|(a, b)| a.iter().chain(b).copied().collect())
            .collect();
        let pair_bits = self.lens_signal(&combined, labels)?.estimate.bits;
        Ok(PairGain {
            left_bits,
            right_bits,
            pair_bits,
            gain_bits: (pair_bits - left_bits.max(right_bits)).max(0.0),
            n_samples: labels.len(),
        })
    }

    pub fn pair_gain_estimate(&self, gain: &PairGain) -> MiEstimate {
        MiEstimate::new(
            gain.gain_bits,
            (gain.gain_bits - 0.02).max(0.0),
            gain.gain_bits + 0.02,
            gain.n_samples,
            EstimatorKind::PairGain,
            TrustTag::Trusted,
        )
    }
}
