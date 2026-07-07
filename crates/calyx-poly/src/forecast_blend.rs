//! Reliability-weighted ensemble blend of forecast components → `p_model` (issue #85).
//!
//! The components (kNN base rate #81, bits vote #84, oracle #83, kernel #82, structural #89) are
//! pooled in **logit space** weighted by each component's held-out reliability (a Brier-derived
//! `[0,1]` weight). Logit pooling is the log-opinion-pool: it is the reliability-weighted geometric
//! mean of odds, which stays calibrated under independent evidence and never lets one component with
//! zero reliability move the answer. Fail closed if no component carries positive reliability.

use calyx_assay::TrustTag;
use serde::{Deserialize, Serialize};

use crate::error::{PolyError, Result};
use crate::forecast::{ForecastComponent, logit, sigmoid};

/// No component carried positive reliability, so there is nothing to blend.
pub const ERR_NO_RELIABLE_COMPONENTS: &str = "CALYX_POLY_FORECAST_NO_RELIABLE_COMPONENTS";
/// The blend received an empty component set.
pub const ERR_EMPTY_BLEND: &str = "CALYX_POLY_FORECAST_EMPTY_BLEND";

/// The pooled model probability and its provenance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlendResult {
    /// The reliability-weighted pooled probability of YES.
    pub p_model: f64,
    /// Sum of the reliability weights that contributed.
    pub total_weight: f64,
    /// Number of components with positive reliability.
    pub contributing: usize,
    /// Trusted only if **every** contributing component was grounded on Trusted evidence.
    pub trust: TrustTag,
    /// The pooled logit before the sigmoid (auditable).
    pub pooled_logit: f64,
}

/// Blends components into a single `p_model` by reliability-weighted logit pooling. Components with
/// zero reliability are dropped (they carry no held-out skill); if none remain, fail closed.
pub fn blend_components(components: &[ForecastComponent]) -> Result<BlendResult> {
    if components.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_EMPTY_BLEND,
            "ensemble blend requires at least one component",
        ));
    }
    let mut weighted_logit = 0.0;
    let mut total_weight = 0.0;
    let mut contributing = 0usize;
    let mut all_trusted = true;
    for c in components {
        if c.reliability <= 0.0 {
            continue;
        }
        weighted_logit += c.reliability * logit(c.p);
        total_weight += c.reliability;
        contributing += 1;
        if c.trust != TrustTag::Trusted {
            all_trusted = false;
        }
    }
    if total_weight <= 0.0 || contributing == 0 {
        return Err(PolyError::diagnostics(
            ERR_NO_RELIABLE_COMPONENTS,
            "no forecast component carried positive held-out reliability",
        ));
    }
    let pooled_logit = weighted_logit / total_weight;
    Ok(BlendResult {
        p_model: sigmoid(pooled_logit),
        total_weight,
        contributing,
        trust: if all_trusted {
            TrustTag::Trusted
        } else {
            TrustTag::Provisional
        },
        pooled_logit,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forecast::ComponentKind;

    fn comp(kind: ComponentKind, p: f64, r: f64, trust: TrustTag) -> ForecastComponent {
        ForecastComponent::new(kind, p, r, 100, trust, "t").unwrap()
    }

    #[test]
    fn equal_reliability_pools_toward_geometric_mean_of_odds() {
        let out = blend_components(&[
            comp(ComponentKind::KnnBaseRate, 0.6, 1.0, TrustTag::Trusted),
            comp(ComponentKind::BitsVote, 0.8, 1.0, TrustTag::Trusted),
        ])
        .unwrap();
        // Logit pool of 0.6 and 0.8 lies between them, above the naive 0.7 arithmetic mean's odds.
        assert!(out.p_model > 0.6 && out.p_model < 0.8);
        assert_eq!(out.trust, TrustTag::Trusted);
        assert_eq!(out.contributing, 2);
    }

    #[test]
    fn zero_reliability_component_is_ignored() {
        let out = blend_components(&[
            comp(ComponentKind::KnnBaseRate, 0.6, 1.0, TrustTag::Trusted),
            comp(ComponentKind::BaselineMarket, 0.1, 0.0, TrustTag::Trusted),
        ])
        .unwrap();
        assert!(
            (out.p_model - 0.6).abs() < 1e-9,
            "zero-reliability must not move p_model"
        );
        assert_eq!(out.contributing, 1);
    }

    #[test]
    fn provisional_component_makes_blend_provisional() {
        let out = blend_components(&[
            comp(ComponentKind::KnnBaseRate, 0.6, 1.0, TrustTag::Trusted),
            comp(ComponentKind::BitsVote, 0.7, 0.5, TrustTag::Provisional),
        ])
        .unwrap();
        assert_eq!(out.trust, TrustTag::Provisional);
    }

    #[test]
    fn no_reliable_components_fails_closed() {
        let err = blend_components(&[comp(
            ComponentKind::BaselineMarket,
            0.5,
            0.0,
            TrustTag::Trusted,
        )])
        .unwrap_err();
        assert_eq!(err.code(), ERR_NO_RELIABLE_COMPONENTS);
    }
}
