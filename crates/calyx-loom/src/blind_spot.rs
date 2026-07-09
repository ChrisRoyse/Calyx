//! Cross-lens anomaly detector.

use calyx_core::{CxId, Result, SlotId};
use serde::{Deserialize, Serialize};

use crate::error::{CALYX_LOOM_UNCALIBRATED_BLINDSPOT, loom_error};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlindSpotAlert {
    pub cx_id: CxId,
    pub a: SlotId,
    pub b: SlotId,
    pub delta: f32,
    pub severity: Severity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calibration: Option<BlindSpotCalibrationEvidence>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlindSpotCalibrationParams {
    pub min_samples: usize,
    pub alpha: f32,
}

impl Default for BlindSpotCalibrationParams {
    fn default() -> Self {
        Self {
            min_samples: 50,
            alpha: 0.05,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlindSpotCalibrationEvidence {
    pub sample_count: usize,
    pub alpha: f32,
    pub threshold_delta: f32,
    pub percentile: f32,
    pub p_value: f32,
    pub score: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BlindSpotCalibration {
    params: BlindSpotCalibrationParams,
    sorted_deltas: Vec<f32>,
    threshold_delta: f32,
}

impl BlindSpotCalibration {
    pub fn from_deltas(
        deltas: impl IntoIterator<Item = f32>,
        params: BlindSpotCalibrationParams,
    ) -> Result<Self> {
        validate_calibration_params(params)?;
        let mut sorted_deltas = Vec::new();
        for delta in deltas {
            if !delta.is_finite() {
                return Err(loom_error(
                    CALYX_LOOM_UNCALIBRATED_BLINDSPOT,
                    "blind-spot calibration delta must be finite",
                ));
            }
            sorted_deltas.push(delta);
        }
        if sorted_deltas.len() < params.min_samples {
            return Err(loom_error(
                CALYX_LOOM_UNCALIBRATED_BLINDSPOT,
                format!(
                    "blind-spot calibration needs at least {} samples; got {}",
                    params.min_samples,
                    sorted_deltas.len()
                ),
            ));
        }
        sorted_deltas.sort_by(f32::total_cmp);
        let threshold_delta = percentile_threshold(&sorted_deltas, 1.0 - params.alpha);
        Ok(Self {
            params,
            sorted_deltas,
            threshold_delta,
        })
    }

    pub fn sample_count(&self) -> usize {
        self.sorted_deltas.len()
    }

    pub fn threshold_delta(&self) -> f32 {
        self.threshold_delta
    }

    fn evaluate(&self, delta: f32) -> BlindSpotCalibrationEvidence {
        let n = self.sorted_deltas.len();
        let less_or_equal = self.sorted_deltas.partition_point(|value| *value <= delta);
        let less_than = self.sorted_deltas.partition_point(|value| *value < delta);
        let greater_or_equal = n - less_than;
        let percentile = less_or_equal as f32 / n as f32;
        let p_value = greater_or_equal as f32 / n as f32;
        BlindSpotCalibrationEvidence {
            sample_count: n,
            alpha: self.params.alpha,
            threshold_delta: self.threshold_delta,
            percentile,
            p_value,
            score: percentile,
        }
    }
}

pub fn detect_blind_spot(
    cx_id: CxId,
    a: SlotId,
    b: SlotId,
    lens_a_similarity: f32,
    lens_b_neighbor_mean: f32,
) -> Option<BlindSpotAlert> {
    let delta = lens_a_similarity - lens_b_neighbor_mean;
    if delta < 0.5 {
        return None;
    }
    let severity = if delta >= 0.8 {
        Severity::High
    } else if delta >= 0.65 {
        Severity::Medium
    } else {
        Severity::Low
    };
    Some(BlindSpotAlert {
        cx_id,
        a,
        b,
        delta,
        severity,
        calibration: None,
    })
}

pub fn detect_blind_spot_calibrated(
    cx_id: CxId,
    a: SlotId,
    b: SlotId,
    lens_a_similarity: f32,
    lens_b_neighbor_mean: f32,
    calibration: &BlindSpotCalibration,
) -> Result<Option<BlindSpotAlert>> {
    let delta = lens_a_similarity - lens_b_neighbor_mean;
    if !delta.is_finite() {
        return Err(loom_error(
            CALYX_LOOM_UNCALIBRATED_BLINDSPOT,
            "blind-spot delta must be finite",
        ));
    }
    let evidence = calibration.evaluate(delta);
    if evidence.p_value > evidence.alpha {
        return Ok(None);
    }
    let severity = calibrated_severity(evidence.percentile, evidence.alpha);
    Ok(Some(BlindSpotAlert {
        cx_id,
        a,
        b,
        delta,
        severity,
        calibration: Some(evidence),
    }))
}

fn calibrated_severity(percentile: f32, alpha: f32) -> Severity {
    let high = 1.0 - alpha / 3.0;
    let medium = 1.0 - (2.0 * alpha / 3.0);
    if percentile >= high {
        Severity::High
    } else if percentile >= medium {
        Severity::Medium
    } else {
        Severity::Low
    }
}

fn validate_calibration_params(params: BlindSpotCalibrationParams) -> Result<()> {
    if params.min_samples == 0 {
        return Err(loom_error(
            CALYX_LOOM_UNCALIBRATED_BLINDSPOT,
            "blind-spot calibration min_samples must be greater than zero",
        ));
    }
    if !params.alpha.is_finite() || params.alpha <= 0.0 || params.alpha >= 1.0 {
        return Err(loom_error(
            CALYX_LOOM_UNCALIBRATED_BLINDSPOT,
            "blind-spot calibration alpha must be finite and in (0,1)",
        ));
    }
    Ok(())
}

fn percentile_threshold(sorted_deltas: &[f32], percentile: f32) -> f32 {
    let last = sorted_deltas.len().saturating_sub(1);
    let index = (last as f32 * percentile).ceil() as usize;
    sorted_deltas[index.min(last)]
}
