//! AssayGate facade for lens signal and pair gain.

use calyx_core::{Anchor, Result};
use serde::{Deserialize, Serialize};

use crate::estimate::{EstimatorKind, MiEstimate, TrustTag};
use crate::logistic::{
    logistic_probe_mi_with_anchor_and_min_samples, logistic_probe_mi_with_min_samples,
};

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
    pub ci_low: f32,
    pub ci_high: f32,
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
        let report = logistic_probe_mi_with_min_samples(samples, labels, self.min_samples)?;
        Ok(LensSignal {
            estimate: report.estimate,
        })
    }

    pub fn lens_signal_with_anchor(
        &self,
        samples: &[Vec<f32>],
        labels: &[bool],
        anchor: &Anchor,
    ) -> Result<LensSignal> {
        let report = logistic_probe_mi_with_anchor_and_min_samples(
            samples,
            labels,
            anchor,
            self.min_samples,
        )?;
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
        let left_signal = self.lens_signal(left, labels)?.estimate;
        let right_signal = self.lens_signal(right, labels)?.estimate;
        let combined: Vec<Vec<f32>> = left
            .iter()
            .zip(right)
            .map(|(a, b)| a.iter().chain(b).copied().collect())
            .collect();
        let pair_signal = self.lens_signal(&combined, labels)?.estimate;
        Ok(pair_gain_from_estimates(
            &left_signal,
            &right_signal,
            &pair_signal,
        ))
    }

    pub fn pair_gain_with_anchor(
        &self,
        left: &[Vec<f32>],
        right: &[Vec<f32>],
        labels: &[bool],
        anchor: &Anchor,
    ) -> Result<PairGain> {
        let left_signal = self.lens_signal_with_anchor(left, labels, anchor)?.estimate;
        let right_signal = self
            .lens_signal_with_anchor(right, labels, anchor)?
            .estimate;
        let combined: Vec<Vec<f32>> = left
            .iter()
            .zip(right)
            .map(|(a, b)| a.iter().chain(b).copied().collect())
            .collect();
        let pair_signal = self
            .lens_signal_with_anchor(&combined, labels, anchor)?
            .estimate;
        Ok(pair_gain_from_estimates(
            &left_signal,
            &right_signal,
            &pair_signal,
        ))
    }

    pub fn pair_gain_estimate(&self, gain: &PairGain) -> MiEstimate {
        self.pair_gain_estimate_with_trust(gain, TrustTag::Provisional)
    }

    pub fn pair_gain_estimate_with_anchor(&self, gain: &PairGain, anchor: &Anchor) -> MiEstimate {
        self.pair_gain_estimate_with_trust(gain, crate::estimate::trust_for_anchor(Some(anchor)))
    }

    fn pair_gain_estimate_with_trust(&self, gain: &PairGain, trust: TrustTag) -> MiEstimate {
        MiEstimate::new(
            gain.gain_bits,
            gain.ci_low,
            gain.ci_high,
            gain.n_samples,
            EstimatorKind::PairGain,
            trust,
        )
    }
}

fn pair_gain_from_estimates(left: &MiEstimate, right: &MiEstimate, pair: &MiEstimate) -> PairGain {
    let baseline_low = left.ci_low.max(right.ci_low);
    let baseline_high = left.ci_high.max(right.ci_high);
    let gain_bits = (pair.bits - left.bits.max(right.bits)).max(0.0);
    let ci_low = (pair.ci_low - baseline_high).max(0.0);
    let ci_high = (pair.ci_high - baseline_low).max(gain_bits);
    PairGain {
        left_bits: left.bits,
        right_bits: right.bits,
        pair_bits: pair.bits,
        gain_bits,
        ci_low,
        ci_high,
        n_samples: pair.n_samples,
    }
}
