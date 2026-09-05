//! Confidence ceiling: `confidence = min(raw, oracle self-consistency, DPI)` (issue #87).
//!
//! A Calyx-native forecast never sells confidence it did not earn. Three independent caps bound it,
//! and the ceiling is their **minimum** (the tightest binding constraint), always strictly `< 1`:
//!
//! 1. **raw** — the support-driven confidence `n/(n+1)` style value, always below certainty.
//! 2. **oracle self-consistency** — `validity·(1 − flakiness)` for the domain, composed through the
//!    real `calyx_oracle::oracle_ceiling` (which caps `raw` by the oracle's self-consistency).
//! 3. **DPI** — the data-processing-inequality ceiling from the panel's measured bits: you cannot be
//!    more confident than the information `I(panel;outcome)` supports.
//!
//! Fail closed on non-finite / out-of-range inputs.

use calyx_oracle::oracle_ceiling;
use serde::{Deserialize, Serialize};

use crate::error::{PolyError, Result};

/// A ceiling input was not finite in `[0, 1]`.
pub const ERR_CEILING_INPUT: &str = "CALYX_POLY_FORECAST_CEILING_INPUT";
/// The oracle self-consistency inputs were invalid.
pub const ERR_CEILING_ORACLE: &str = "CALYX_POLY_FORECAST_CEILING_ORACLE";

/// The largest confidence a grounded forecast may carry — strictly below certainty.
pub const CONFIDENCE_HARD_CAP: f64 = 1.0 - 1e-9;

/// The three caps and the final confidence (their minimum), with the binding constraint named.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfidenceCeiling {
    /// Support-driven raw confidence (`n/(n+1)` style).
    pub raw: f64,
    /// Oracle self-consistency `validity·(1 − flakiness)`.
    pub self_consistency: f64,
    /// DPI ceiling from measured panel bits.
    pub dpi: f64,
    /// `raw` after the oracle caps it by self-consistency (from `oracle_ceiling`).
    pub capped_by_oracle: f64,
    /// Final confidence: `min(raw, self_consistency, dpi)`, `< 1`.
    pub confidence: f64,
    /// Which cap bound the confidence (`raw` | `self_consistency` | `dpi`).
    pub binding: String,
}

fn unit(value: f64, field: &str) -> Result<()> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(PolyError::diagnostics(
            ERR_CEILING_INPUT,
            format!("confidence ceiling input {field}={value} must be finite in [0, 1]"),
        ));
    }
    Ok(())
}

/// Computes the confidence ceiling. `raw` is the support-driven confidence; `flakiness`/`validity`
/// are the oracle self-consistency inputs (composed through `calyx_oracle::oracle_ceiling`); `dpi`
/// is the panel-bits DPI ceiling (see [`dpi_ceiling_from_bits`]).
pub fn confidence_ceiling(
    raw: f64,
    flakiness: f64,
    validity: f64,
    dpi: f64,
) -> Result<ConfidenceCeiling> {
    unit(raw, "raw")?;
    unit(flakiness, "flakiness")?;
    unit(validity, "validity")?;
    unit(dpi, "dpi")?;

    // Compose the real oracle ceiling: caps `raw` (tau_corr) by validity·(1−flakiness).
    let oracle = oracle_ceiling(raw as f32, flakiness as f32, validity as f32).map_err(|err| {
        PolyError::diagnostics(
            ERR_CEILING_ORACLE,
            format!("calyx_oracle::oracle_ceiling failed: {err}"),
        )
    })?;
    let self_consistency = oracle.oracle_self_consistency as f64;
    let capped_by_oracle = oracle.capped_tau as f64;

    let mut confidence = capped_by_oracle.min(dpi);
    let mut binding = if capped_by_oracle <= dpi {
        if raw <= self_consistency {
            "raw"
        } else {
            "self_consistency"
        }
    } else {
        "dpi"
    }
    .to_string();
    if confidence >= 1.0 {
        confidence = CONFIDENCE_HARD_CAP;
        binding = "hard_cap".to_string();
    }
    confidence = confidence.min(CONFIDENCE_HARD_CAP);

    Ok(ConfidenceCeiling {
        raw,
        self_consistency,
        dpi,
        capped_by_oracle,
        confidence,
        binding,
    })
}

/// Maps measured panel bits about a binary outcome to a DPI confidence ceiling in `[0.5, 1)`. Zero
/// bits → a coin flip (0.5); reaching the outcome's own entropy (sufficient) → the hard cap. Below-
/// zero or non-finite bits fail closed upstream (this is a monotone, saturating map).
pub fn dpi_ceiling_from_bits(panel_bits: f64, anchor_entropy_bits: f64) -> f64 {
    if anchor_entropy_bits <= 0.0 {
        return CONFIDENCE_HARD_CAP;
    }
    let ratio = (panel_bits.max(0.0) / anchor_entropy_bits).min(1.0);
    (0.5 + 0.5 * ratio).min(CONFIDENCE_HARD_CAP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_of_three_binds() {
        // Strong raw, clean oracle, but weak DPI → DPI binds.
        let c = confidence_ceiling(0.99, 0.0, 1.0, 0.7).unwrap();
        assert!((c.confidence - 0.7).abs() < 1e-6, "dpi must bind: {c:?}");
        assert_eq!(c.binding, "dpi");

        // Flaky oracle caps a high raw.
        let c2 = confidence_ceiling(0.99, 0.5, 1.0, 0.99).unwrap();
        // self-consistency = 1.0*(1-0.5)=0.5 → capped_by_oracle = min(0.99,0.5)=0.5
        assert!(
            (c2.confidence - 0.5).abs() < 1e-6,
            "self-consistency must bind: {c2:?}"
        );
    }

    #[test]
    fn never_reaches_one() {
        let c = confidence_ceiling(1.0, 0.0, 1.0, 1.0).unwrap();
        assert!(c.confidence < 1.0);
    }

    #[test]
    fn dpi_map_endpoints() {
        assert!((dpi_ceiling_from_bits(0.0, 1.0) - 0.5).abs() < 1e-9);
        assert!(dpi_ceiling_from_bits(1.0, 1.0) >= CONFIDENCE_HARD_CAP - 1e-6);
        assert!((dpi_ceiling_from_bits(0.5, 1.0) - 0.75).abs() < 1e-9);
    }

    #[test]
    fn non_finite_fails_closed() {
        assert_eq!(
            confidence_ceiling(f64::NAN, 0.0, 1.0, 0.9)
                .unwrap_err()
                .code(),
            ERR_CEILING_INPUT
        );
    }
}
