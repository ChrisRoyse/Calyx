//! Forecast superiority predicate — the six-tier readiness gate (issue #88).
//!
//! Per PRD `04_ASSOCIATION_ENGINE_NO_EMBEDDERS.md` §1.6 and the handbook "readiness predicate"
//! (§7.7), a Calyx-native forecast is admissible for a market/domain only when **all six**
//! independently-measured tiers hold. It is a conjunction, never an average, and it emits an
//! auditable admission decision, not a trade instruction. This module gathers the six measurements
//! and evaluates them through the real `calyx_oracle::super_intelligence` predicate.
//!
//! 1. **Oracle-clean** — `oracle_self_consistency ≥ 0.7`.
//! 2. **Panel-sufficient** — `I(panel;outcome) ≥ H(outcome)` (#79).
//! 3. **Kernel-exists** — a grounding kernel at recall ≥ 0.95 (#82).
//! 4. **Calibrated** — domain×horizon calibration within tolerance, not stale (#86/#91).
//! 5. **Goodhart-defended** — no gamed objective in the meta-learning ledger (#113).
//! 6. **Mistake-closed** — outstanding mistake-closure heads for the domain are closed (#106).

use calyx_oracle::{SuperIntelligenceEvidence, super_intelligence_formula};
use serde::{Deserialize, Serialize};

use crate::error::{PolyError, Result};

/// Tier-1 oracle self-consistency floor.
pub const ORACLE_CLEAN_THRESHOLD: f64 = 0.7;

/// A tier input was not finite in `[0, 1]`.
pub const ERR_SUPERIORITY_INPUT: &str = "CALYX_POLY_SUPERIORITY_INPUT";
/// The composed oracle predicate rejected the evidence.
pub const ERR_SUPERIORITY_ORACLE: &str = "CALYX_POLY_SUPERIORITY_ORACLE";

/// The six independently-measured tiers for a market/domain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SuperiorityTiers {
    /// Oracle self-consistency `validity·(1−flakiness)` for the domain.
    pub oracle_self_consistency: f64,
    /// Panel sufficiency (`I(panel;outcome) ≥ H(outcome)`).
    pub panel_sufficient: bool,
    /// Grounding-kernel recall ratio.
    pub kernel_recall_ratio: f64,
    /// Minimum acceptable kernel recall (policy floor, 0.95).
    pub min_kernel_recall_ratio: f64,
    /// Domain×horizon calibration is within tolerance and not stale.
    pub calibrated: bool,
    /// No gamed/degenerate objective for the domain (meta-learning ledger).
    pub goodhart_defended: bool,
    /// Outstanding mistake-closure heads for the domain are closed.
    pub mistake_closed: bool,
}

/// The superiority verdict: an auditable admission decision (never a trade instruction).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SuperiorityVerdict {
    /// True only if every tier holds.
    pub pass: bool,
    /// The tiers that failed (empty on pass).
    pub failing_tiers: Vec<String>,
    /// Whether tier 1 (oracle-clean ≥ 0.7) held.
    pub oracle_clean: bool,
}

/// Evaluates the six-tier superiority predicate through the real oracle conjunction. Fail closed on a
/// non-finite / out-of-range recall ratio.
pub fn evaluate_superiority(tiers: &SuperiorityTiers) -> Result<SuperiorityVerdict> {
    if !tiers.oracle_self_consistency.is_finite()
        || !(0.0..=1.0).contains(&tiers.oracle_self_consistency)
    {
        return Err(PolyError::diagnostics(
            ERR_SUPERIORITY_INPUT,
            format!(
                "oracle_self_consistency {} must be finite in [0, 1]",
                tiers.oracle_self_consistency
            ),
        ));
    }
    let oracle_clean = tiers.oracle_self_consistency >= ORACLE_CLEAN_THRESHOLD;
    let evidence = SuperIntelligenceEvidence {
        clean: oracle_clean,
        sufficient: tiers.panel_sufficient,
        kernel_recall_ratio: tiers.kernel_recall_ratio as f32,
        min_kernel_recall_ratio: tiers.min_kernel_recall_ratio as f32,
        calibrated: tiers.calibrated,
        goodhart_defended: tiers.goodhart_defended,
        mistake_closed: tiers.mistake_closed,
    };
    let verdict = super_intelligence_formula(evidence).map_err(|err| {
        PolyError::diagnostics(
            ERR_SUPERIORITY_ORACLE,
            format!("calyx_oracle::super_intelligence failed: {err}"),
        )
    })?;
    Ok(SuperiorityVerdict {
        pass: verdict.pass,
        failing_tiers: verdict.failing_tiers,
        oracle_clean,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strong() -> SuperiorityTiers {
        SuperiorityTiers {
            oracle_self_consistency: 0.9,
            panel_sufficient: true,
            kernel_recall_ratio: 0.97,
            min_kernel_recall_ratio: 0.95,
            calibrated: true,
            goodhart_defended: true,
            mistake_closed: true,
        }
    }

    #[test]
    fn all_tiers_pass() {
        let v = evaluate_superiority(&strong()).unwrap();
        assert!(v.pass);
        assert!(v.failing_tiers.is_empty());
        assert!(v.oracle_clean);
    }

    #[test]
    fn each_failing_tier_refuses() {
        let mut t = strong();
        t.oracle_self_consistency = 0.5;
        let v = evaluate_superiority(&t).unwrap();
        assert!(!v.pass && v.failing_tiers.contains(&"clean".to_string()) && !v.oracle_clean);

        let mut t = strong();
        t.panel_sufficient = false;
        assert!(
            evaluate_superiority(&t)
                .unwrap()
                .failing_tiers
                .contains(&"sufficient".to_string())
        );

        let mut t = strong();
        t.kernel_recall_ratio = 0.80;
        assert!(
            evaluate_superiority(&t)
                .unwrap()
                .failing_tiers
                .contains(&"kernel".to_string())
        );

        let mut t = strong();
        t.mistake_closed = false;
        assert!(
            evaluate_superiority(&t)
                .unwrap()
                .failing_tiers
                .contains(&"mistake_closed".to_string())
        );
    }

    #[test]
    fn non_finite_fails_closed() {
        let mut t = strong();
        t.oracle_self_consistency = f64::NAN;
        assert_eq!(
            evaluate_superiority(&t).unwrap_err().code(),
            ERR_SUPERIORITY_INPUT
        );
    }
}
