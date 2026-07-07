//! Shared contract for Calyx-native forecast components (EPIC #210, "Compose" step).
//!
//! A Calyx-native forecast is a **blend of independently-measured components** — a kNN-of-resolved
//! base rate (#81), a per-slot bits vote (#84), an oracle prediction (#83), a kernel answer (#82),
//! and structural / baseline-market signals (#89) — reliability-weighted into one `p_model` (#85),
//! calibrated per domain×horizon (#86), and capped by a confidence ceiling (#87). This module owns
//! the component contract every producer emits, so the blend never depends on how a component was
//! computed. Fail closed: a component with a non-finite probability, an out-of-range reliability, or
//! zero support is a construction error, never a silent default.

use calyx_assay::TrustTag;
use serde::{Deserialize, Serialize};

use crate::error::{PolyError, Result};

/// A component probability was not finite in `[0, 1]`.
pub const ERR_COMPONENT_PROBABILITY: &str = "CALYX_POLY_FORECAST_COMPONENT_PROBABILITY";
/// A component reliability was not finite in `[0, 1]`.
pub const ERR_COMPONENT_RELIABILITY: &str = "CALYX_POLY_FORECAST_COMPONENT_RELIABILITY";
/// A component reported zero supporting samples.
pub const ERR_COMPONENT_SUPPORT: &str = "CALYX_POLY_FORECAST_COMPONENT_SUPPORT";

/// The kind of evidence a forecast component carries. Each maps to an engine subsystem.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    /// kNN-of-resolved empirical YES-rate (Sextant), issue #81.
    KnnBaseRate,
    /// Per-slot logistic-probe direction weighted by measured bits (Assay), issue #84.
    BitsVote,
    /// calyx-oracle forward prediction, issue #83.
    Oracle,
    /// Lodestar kernel answer over the grounding kernel, issue #82.
    Kernel,
    /// Structural-neighbor consensus (Loom / mispricing), issue #89.
    Structural,
    /// The public market's own implied probability (a baseline, never trusted blindly).
    BaselineMarket,
}

impl ComponentKind {
    /// Stable slug used in persisted records and provenance.
    pub fn slug(self) -> &'static str {
        match self {
            ComponentKind::KnnBaseRate => "knn_base_rate",
            ComponentKind::BitsVote => "bits_vote",
            ComponentKind::Oracle => "oracle",
            ComponentKind::Kernel => "kernel",
            ComponentKind::Structural => "structural",
            ComponentKind::BaselineMarket => "baseline_market",
        }
    }
}

/// One measured forecast component: a probability for the YES outcome, the reliability weight it
/// earned on held-out data (a Brier-derived `[0,1]` score; higher = more trustworthy), how many
/// samples support it, and the trust of the evidence it was measured on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForecastComponent {
    /// Which evidence kind this is.
    pub kind: ComponentKind,
    /// Probability of the YES outcome in `[0, 1]`.
    pub p: f64,
    /// Held-out reliability weight in `[0, 1]` (Brier-derived; 0 = ignore this component).
    pub reliability: f64,
    /// Supporting sample count.
    pub n_support: usize,
    /// Trust of the grounding evidence (proxy-grounded → Provisional).
    pub trust: TrustTag,
    /// Human-readable provenance detail.
    pub detail: String,
}

impl ForecastComponent {
    /// Builds a validated component. Fails closed on a non-finite / out-of-range probability or
    /// reliability, or zero support.
    pub fn new(
        kind: ComponentKind,
        p: f64,
        reliability: f64,
        n_support: usize,
        trust: TrustTag,
        detail: impl Into<String>,
    ) -> Result<Self> {
        if !p.is_finite() || !(0.0..=1.0).contains(&p) {
            return Err(PolyError::diagnostics(
                ERR_COMPONENT_PROBABILITY,
                format!("{} probability {p} must be finite in [0, 1]", kind.slug()),
            ));
        }
        if !reliability.is_finite() || !(0.0..=1.0).contains(&reliability) {
            return Err(PolyError::diagnostics(
                ERR_COMPONENT_RELIABILITY,
                format!(
                    "{} reliability {reliability} must be finite in [0, 1]",
                    kind.slug()
                ),
            ));
        }
        if n_support == 0 {
            return Err(PolyError::diagnostics(
                ERR_COMPONENT_SUPPORT,
                format!("{} reported zero supporting samples", kind.slug()),
            ));
        }
        Ok(Self {
            kind,
            p,
            reliability,
            n_support,
            trust,
            detail: detail.into(),
        })
    }
}

/// Numerically-safe logit with probabilities clamped away from the `{0,1}` singularities.
pub fn logit(p: f64) -> f64 {
    const EPS: f64 = 1e-6;
    let p = p.clamp(EPS, 1.0 - EPS);
    (p / (1.0 - p)).ln()
}

/// Logistic sigmoid, the inverse of [`logit`].
pub fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logit_sigmoid_round_trip() {
        for p in [0.05, 0.2, 0.5, 0.73, 0.95] {
            assert!((sigmoid(logit(p)) - p).abs() < 1e-6, "round trip at {p}");
        }
    }

    #[test]
    fn component_validation_fails_closed() {
        assert_eq!(
            ForecastComponent::new(ComponentKind::Oracle, 1.5, 0.5, 10, TrustTag::Trusted, "x")
                .unwrap_err()
                .code(),
            ERR_COMPONENT_PROBABILITY
        );
        assert_eq!(
            ForecastComponent::new(ComponentKind::Oracle, 0.5, 2.0, 10, TrustTag::Trusted, "x")
                .unwrap_err()
                .code(),
            ERR_COMPONENT_RELIABILITY
        );
        assert_eq!(
            ForecastComponent::new(ComponentKind::Oracle, 0.5, 0.5, 0, TrustTag::Trusted, "x")
                .unwrap_err()
                .code(),
            ERR_COMPONENT_SUPPORT
        );
        assert!(
            ForecastComponent::new(ComponentKind::Oracle, 0.5, 0.5, 10, TrustTag::Trusted, "ok")
                .is_ok()
        );
    }
}
