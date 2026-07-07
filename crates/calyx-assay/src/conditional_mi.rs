//! Gaussian conditional mutual information oracle (#63).
//!
//! For scalar continuous lenses under the Gaussian/linear assumption,
//! `I(X;Y|Z) = -0.5 * log2(1 - r_xy.z^2)`, where `r_xy.z` is the partial
//! correlation after conditioning on one or more controls. This module delegates
//! the hard validation and precision-matrix math to the existing fail-closed
//! partial-correlation implementation.

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

use crate::partial_correlation::{PartialReport, partial_correlation_controlling};

pub const DEFAULT_CMI_ALPHA: f32 = 0.05;
pub const GAUSSIAN_CMI_FORMULA: &str = "-0.5 * log2(1 - partial_r^2)";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConditionalIndependence {
    Independent,
    Dependent,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConditionalMiReport {
    pub estimator: String,
    pub formula: String,
    pub cmi_bits: f32,
    pub partial_r: f32,
    pub zero_order_r: f32,
    pub p_value: f32,
    pub alpha: f32,
    pub decision: ConditionalIndependence,
    pub ci_low: f32,
    pub ci_high: f32,
    pub n_controls: usize,
    pub n_samples: usize,
}

pub fn conditional_mutual_information_gaussian(
    x: &[f32],
    y: &[f32],
    z: &[f32],
) -> Result<ConditionalMiReport> {
    conditional_mutual_information_gaussian_with_alpha(x, y, &[z], DEFAULT_CMI_ALPHA)
}

pub fn conditional_mutual_information_gaussian_with_alpha(
    x: &[f32],
    y: &[f32],
    controls: &[&[f32]],
    alpha: f32,
) -> Result<ConditionalMiReport> {
    if !(alpha > 0.0 && alpha < 1.0) {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "Gaussian CMI alpha must be in (0,1); got {alpha}"
        )));
    }
    let partial = partial_correlation_controlling(x, y, controls)?;
    gaussian_cmi_from_partial(partial, alpha)
}

fn gaussian_cmi_from_partial(partial: PartialReport, alpha: f32) -> Result<ConditionalMiReport> {
    let r = partial.partial_r as f64;
    let unexplained = 1.0 - r * r;
    if unexplained <= f64::EPSILON {
        return Err(CalyxError::assay_degenerate_input(
            "Gaussian CMI is unbounded when residual correlation is perfect",
        ));
    }
    let cmi_bits = (-0.5 * unexplained.ln() / std::f64::consts::LN_2) as f32;
    let decision = if partial.p_value >= alpha {
        ConditionalIndependence::Independent
    } else {
        ConditionalIndependence::Dependent
    };
    Ok(ConditionalMiReport {
        estimator: "gaussian_partial_correlation".to_string(),
        formula: GAUSSIAN_CMI_FORMULA.to_string(),
        cmi_bits,
        partial_r: partial.partial_r,
        zero_order_r: partial.zero_order_r,
        p_value: partial.p_value,
        alpha,
        decision,
        ci_low: partial.ci_low,
        ci_high: partial.ci_high,
        n_controls: partial.n_controls,
        n_samples: partial.n_samples,
    })
}
