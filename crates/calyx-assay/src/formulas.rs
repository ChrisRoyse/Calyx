//! PRD-22 Assay formula-name wrappers.

use calyx_core::{CalyxError, Result};

use crate::contract::{AdmissionDecision, MAX_PAIRWISE_CORR, admit_lens};

pub fn lens_signal(signal_bits: f32, max_pairwise_corr: f32) -> Result<AdmissionDecision> {
    admit_lens(signal_bits, max_pairwise_corr)
}

pub fn pair_redundancy(correlation: f32) -> Result<f32> {
    if !correlation.is_finite() {
        return Err(CalyxError::assay_redundant(
            "pair redundancy requires finite correlation",
        ));
    }
    let redundancy = correlation.abs();
    if redundancy > MAX_PAIRWISE_CORR {
        return Err(CalyxError::assay_redundant(format!(
            "pair redundancy {redundancy:.4} above {MAX_PAIRWISE_CORR:.4}"
        )));
    }
    Ok(redundancy)
}

pub fn marginal_value(panel_bits: f32, panel_without_lens_bits: f32) -> Result<f32> {
    validate_non_negative_bits(panel_bits, "panel_bits")?;
    validate_non_negative_bits(panel_without_lens_bits, "panel_without_lens_bits")?;
    Ok((panel_bits - panel_without_lens_bits).max(0.0))
}

pub fn dpi_ceiling(panel_outcome_bits: f32) -> Result<f32> {
    validate_non_negative_bits(panel_outcome_bits, "panel_outcome_bits")?;
    Ok(panel_outcome_bits)
}

fn validate_non_negative_bits(value: f32, field: &str) -> Result<()> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(CalyxError::assay_insufficient_samples(format!(
            "{field} must be finite non-negative bits"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formula_wrappers_match_prd_thresholds() {
        assert!(lens_signal(0.05, 0.6).unwrap().admitted);
        assert_eq!(pair_redundancy(-0.6).unwrap(), 0.6);
        assert_eq!(marginal_value(1.25, 0.75).unwrap(), 0.5);
        assert_eq!(dpi_ceiling(2.0).unwrap(), 2.0);
    }

    #[test]
    fn formula_wrappers_fail_closed() {
        assert_eq!(
            lens_signal(0.049, 0.1).unwrap_err().code,
            "CALYX_ASSAY_LOW_SIGNAL"
        );
        assert_eq!(
            pair_redundancy(0.61).unwrap_err().code,
            "CALYX_ASSAY_REDUNDANT"
        );
        assert_eq!(
            marginal_value(f32::NAN, 0.0).unwrap_err().code,
            "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
        );
    }
}
