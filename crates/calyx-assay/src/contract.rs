//! Lens differentiation contract enforcement.

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

use crate::estimate::MiEstimate;
use crate::stratified::StratifiedBits;

pub const MIN_SIGNAL_BITS: f32 = 0.05;
pub const MAX_PAIRWISE_CORR: f32 = 0.6;
pub const MIN_RELIABILITY_SEEDS: usize = 5;
pub const CALYX_ASSAY_UNRESOLVED: &str = "CALYX_ASSAY_UNRESOLVED";
pub const LEARNED_SIGNAL_KIND: &str = "learned_encoder";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdmissionDecision {
    pub admitted: bool,
    pub signal_bits: f32,
    pub max_pairwise_corr: f32,
    pub stratified_override: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CorrelationEvidence {
    pub corr: f32,
    pub ci_low: f32,
    pub ci_high: f32,
}

impl CorrelationEvidence {
    pub fn point(corr: f32) -> Self {
        Self {
            corr,
            ci_low: corr,
            ci_high: corr,
        }
    }

    pub fn new(corr: f32, ci_low: f32, ci_high: f32) -> Result<Self> {
        for (name, value) in [("corr", corr), ("ci_low", ci_low), ("ci_high", ci_high)] {
            if !value.is_finite() || value < 0.0 {
                return Err(CalyxError::assay_redundant(format!(
                    "correlation evidence {name} must be finite and non-negative"
                )));
            }
        }
        Ok(Self {
            corr: corr.min(1.0),
            ci_low: ci_low.min(corr).min(1.0),
            ci_high: ci_high.max(corr).min(1.0),
        })
    }
}

pub fn admit_lens(signal_bits: f32, max_pairwise_corr: f32) -> Result<AdmissionDecision> {
    decide(signal_bits, max_pairwise_corr, false)
}

pub fn admit_lens_estimate(
    signal: &MiEstimate,
    corr: CorrelationEvidence,
) -> Result<AdmissionDecision> {
    decide_estimate(signal, corr, false)
}

pub fn admit_lens_estimate_with_signal_kind(
    signal: &MiEstimate,
    corr: CorrelationEvidence,
    signal_kind: &str,
) -> Result<AdmissionDecision> {
    validate_learned_signal_kind(signal_kind)?;
    decide_estimate(signal, corr, false)
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

fn decide_estimate(
    signal: &MiEstimate,
    corr: CorrelationEvidence,
    stratified_override: bool,
) -> Result<AdmissionDecision> {
    validate_signal_evidence(signal)?;
    validate_corr_evidence(corr)?;

    let reliability = signal
        .reliability
        .as_ref()
        .ok_or_else(|| unresolved("assay signal lacks multi-seed reliability evidence"))?;
    if reliability.seed_count < MIN_RELIABILITY_SEEDS {
        return Err(unresolved(format!(
            "assay signal used {} seeds; need at least {MIN_RELIABILITY_SEEDS}",
            reliability.seed_count
        )));
    }
    if reliability.unresolved || reliability.seed_sigma >= signal.bits.abs() {
        return Err(unresolved(format!(
            "assay signal unresolved: bits={:.6} seed_sigma={:.6}",
            signal.bits, reliability.seed_sigma
        )));
    }
    if signal.ci_high < MIN_SIGNAL_BITS {
        return Err(CalyxError::assay_low_signal(format!(
            "lens signal upper CI {:.4} bits below {MIN_SIGNAL_BITS:.4}",
            signal.ci_high
        )));
    }
    if signal.ci_low < MIN_SIGNAL_BITS {
        return Err(unresolved(format!(
            "lens signal CI [{:.4}, {:.4}] spans {MIN_SIGNAL_BITS:.4}",
            signal.ci_low, signal.ci_high
        )));
    }
    if corr.ci_low > MAX_PAIRWISE_CORR {
        return Err(CalyxError::assay_redundant(format!(
            "pairwise correlation lower CI {:.4} above {MAX_PAIRWISE_CORR:.4}",
            corr.ci_low
        )));
    }
    if corr.ci_high > MAX_PAIRWISE_CORR {
        return Err(unresolved(format!(
            "pairwise correlation CI [{:.4}, {:.4}] spans {MAX_PAIRWISE_CORR:.4}",
            corr.ci_low, corr.ci_high
        )));
    }
    Ok(AdmissionDecision {
        admitted: true,
        signal_bits: signal.bits,
        max_pairwise_corr: corr.corr,
        stratified_override,
    })
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

fn validate_signal_evidence(signal: &MiEstimate) -> Result<()> {
    for (name, value) in [
        ("bits", signal.bits),
        ("ci_low", signal.ci_low),
        ("ci_high", signal.ci_high),
    ] {
        if !value.is_finite() || value < 0.0 {
            return Err(CalyxError::assay_low_signal(format!(
                "assay signal {name} must be finite and non-negative"
            )));
        }
    }
    if signal.ci_low > signal.bits || signal.ci_high < signal.bits {
        return Err(CalyxError::assay_low_signal(
            "assay signal CI must contain the point estimate",
        ));
    }
    Ok(())
}

fn validate_corr_evidence(corr: CorrelationEvidence) -> Result<()> {
    if corr.ci_low > corr.corr || corr.ci_high < corr.corr {
        return Err(CalyxError::assay_redundant(
            "correlation CI must contain the point estimate",
        ));
    }
    Ok(())
}

fn validate_learned_signal_kind(signal_kind: &str) -> Result<()> {
    if signal_kind == LEARNED_SIGNAL_KIND {
        return Ok(());
    }
    Err(CalyxError {
        code: "CALYX_ASSAY_NON_LEARNED_LENS",
        message: format!(
            "lens signal_kind={signal_kind}; Assay A35 admission requires learned_encoder content lenses"
        ),
        remediation: "use frozen learned content encoders; algorithmic, placeholder, unknown, and temporal lenses are diagnostic/sidecar only",
    })
}

fn unresolved(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ASSAY_UNRESOLVED,
        message: message.into(),
        remediation: "collect more grouped anchors and re-run multi-seed Assay measurement",
    }
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

    #[test]
    fn admit_lens_estimate_uses_lower_bound_and_unresolved_band() {
        let reliable = crate::estimate::EstimateReliability::new(5, 0.01, false).unwrap();
        let pass = MiEstimate::new(
            0.08,
            0.052,
            0.11,
            120,
            crate::estimate::EstimatorKind::LogisticProbe,
            crate::estimate::TrustTag::Trusted,
        )
        .with_reliability(reliable.clone());
        assert!(admit_lens_estimate(&pass, CorrelationEvidence::point(0.2)).is_ok());

        let spans = MiEstimate::new(
            0.08,
            0.049,
            0.11,
            120,
            crate::estimate::EstimatorKind::LogisticProbe,
            crate::estimate::TrustTag::Trusted,
        )
        .with_reliability(reliable);
        assert_eq!(
            admit_lens_estimate(&spans, CorrelationEvidence::point(0.2))
                .unwrap_err()
                .code,
            CALYX_ASSAY_UNRESOLVED
        );
    }

    #[test]
    fn correlation_ci_must_clear_redundancy_threshold() {
        let signal = MiEstimate::new(
            0.20,
            0.10,
            0.25,
            120,
            crate::estimate::EstimatorKind::LogisticProbe,
            crate::estimate::TrustTag::Trusted,
        )
        .with_reliability(crate::estimate::EstimateReliability::new(5, 0.01, false).unwrap());

        let redundant = CorrelationEvidence::new(0.80, 0.70, 0.90).unwrap();
        assert_eq!(
            admit_lens_estimate(&signal, redundant).unwrap_err().code,
            "CALYX_ASSAY_REDUNDANT"
        );

        let unresolved_corr = CorrelationEvidence::new(0.65, 0.55, 0.70).unwrap();
        assert_eq!(
            admit_lens_estimate(&signal, unresolved_corr)
                .unwrap_err()
                .code,
            CALYX_ASSAY_UNRESOLVED
        );
    }

    #[test]
    fn estimate_admission_with_signal_kind_rejects_non_learned() {
        let signal = MiEstimate::new(
            0.20,
            0.10,
            0.25,
            120,
            crate::estimate::EstimatorKind::LogisticProbe,
            crate::estimate::TrustTag::Trusted,
        )
        .with_reliability(crate::estimate::EstimateReliability::new(5, 0.01, false).unwrap());

        let err = admit_lens_estimate_with_signal_kind(
            &signal,
            CorrelationEvidence::point(0.2),
            "algorithmic",
        )
        .unwrap_err();

        assert_eq!(err.code, "CALYX_ASSAY_NON_LEARNED_LENS");
    }
}
