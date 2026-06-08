//! Lens differentiation contract enforcement.

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

use crate::stratified::StratifiedBits;

pub const MIN_SIGNAL_BITS: f32 = 0.05;
pub const MAX_PAIRWISE_CORR: f32 = 0.6;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdmissionDecision {
    pub admitted: bool,
    pub signal_bits: f32,
    pub max_pairwise_corr: f32,
    pub stratified_override: bool,
}

pub fn admit_lens(signal_bits: f32, max_pairwise_corr: f32) -> Result<AdmissionDecision> {
    decide(signal_bits, max_pairwise_corr, false)
}

pub fn admit_lens_with_strata(
    strata: &StratifiedBits,
    max_pairwise_corr: f32,
) -> Result<AdmissionDecision> {
    let stratified_override = strata.effective_bits >= MIN_SIGNAL_BITS
        && strata.global_bits < MIN_SIGNAL_BITS
        && strata.strata.iter().any(|stratum| stratum.sole_carrier);
    decide(
        strata.effective_bits,
        max_pairwise_corr,
        stratified_override,
    )
}

fn decide(
    signal_bits: f32,
    max_pairwise_corr: f32,
    stratified_override: bool,
) -> Result<AdmissionDecision> {
    if !signal_bits.is_finite() {
        return Err(CalyxError::assay_low_signal(
            "lens signal bits must be finite",
        ));
    }
    if !max_pairwise_corr.is_finite() {
        return Err(CalyxError::assay_redundant(
            "pairwise correlation must be finite",
        ));
    }
    if signal_bits < MIN_SIGNAL_BITS {
        return Err(CalyxError::assay_low_signal(format!(
            "lens signal {signal_bits:.4} bits below {MIN_SIGNAL_BITS:.4}"
        )));
    }
    if max_pairwise_corr > MAX_PAIRWISE_CORR {
        return Err(CalyxError::assay_redundant(format!(
            "pairwise correlation {max_pairwise_corr:.4} above {MAX_PAIRWISE_CORR:.4}"
        )));
    }
    Ok(AdmissionDecision {
        admitted: true,
        signal_bits,
        max_pairwise_corr,
        stratified_override,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admit_lens_rejects_non_finite_signal_and_corr() {
        assert_eq!(
            admit_lens(f32::NAN, 0.1).unwrap_err().code,
            "CALYX_ASSAY_LOW_SIGNAL"
        );
        assert_eq!(
            admit_lens(f32::INFINITY, 0.1).unwrap_err().code,
            "CALYX_ASSAY_LOW_SIGNAL"
        );
        assert_eq!(
            admit_lens(0.2, f32::NAN).unwrap_err().code,
            "CALYX_ASSAY_REDUNDANT"
        );
        assert_eq!(
            admit_lens(0.2, f32::NEG_INFINITY).unwrap_err().code,
            "CALYX_ASSAY_REDUNDANT"
        );
    }
}
