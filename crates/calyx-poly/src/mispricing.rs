//! Mispricing detection: market vs structural-neighbor consensus (issue #89).
//!
//! A market is a mispricing candidate when its public price disagrees with the empirical consensus
//! of its nearest **resolved** structural neighbors (the kNN base rate, #81) by more than a
//! threshold. This is a forecast-quality flag — a candidate for closer analysis — never a trade
//! instruction. Fail closed on a non-finite price or an out-of-range threshold. The flag is only
//! trusted when the neighbor consensus itself is reliable (enough similar neighbors).

use serde::{Deserialize, Serialize};

use crate::error::{PolyError, Result};
use crate::knn_base_rate::KnnBaseRate;

/// The market price or threshold was invalid.
pub const ERR_MISPRICING_INPUT: &str = "CALYX_POLY_MISPRICING_INPUT";

/// Minimum neighbor reliability for a mispricing flag to be actionable.
pub const MIN_MISPRICING_RELIABILITY: f64 = 0.3;

/// A mispricing assessment for one market.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MispricingFlag {
    /// Public market implied YES probability.
    pub market_price: f64,
    /// Structural-neighbor consensus YES-rate (kNN base rate).
    pub neighbor_consensus: f64,
    /// `|market_price − neighbor_consensus|`.
    pub divergence: f64,
    /// Divergence threshold used.
    pub threshold: f64,
    /// Whether the market is flagged as a mispricing candidate.
    pub flagged: bool,
    /// `overpriced` (market > neighbors), `underpriced` (market < neighbors), or `aligned`.
    pub direction: String,
    /// Neighbor-consensus reliability (from the kNN base rate).
    pub reliability: f64,
    /// Neighbors that formed the consensus.
    pub n_neighbors: usize,
}

/// Flags a market as a mispricing candidate if its price diverges from the kNN neighbor consensus by
/// more than `threshold` and the consensus is reliable enough to act on.
pub fn detect_mispricing(
    base_rate: &KnnBaseRate,
    market_price: f64,
    threshold: f64,
) -> Result<MispricingFlag> {
    if !market_price.is_finite() || !(0.0..=1.0).contains(&market_price) {
        return Err(PolyError::diagnostics(
            ERR_MISPRICING_INPUT,
            format!("market price {market_price} must be finite in [0, 1]"),
        ));
    }
    if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
        return Err(PolyError::diagnostics(
            ERR_MISPRICING_INPUT,
            format!("mispricing threshold {threshold} must be finite in [0, 1]"),
        ));
    }
    let divergence = (market_price - base_rate.p_yes).abs();
    let reliable = base_rate.reliability >= MIN_MISPRICING_RELIABILITY;
    let flagged = divergence > threshold && reliable;
    let direction = if divergence <= threshold {
        "aligned"
    } else if market_price > base_rate.p_yes {
        "overpriced"
    } else {
        "underpriced"
    }
    .to_string();

    Ok(MispricingFlag {
        market_price,
        neighbor_consensus: base_rate.p_yes,
        divergence,
        threshold,
        flagged,
        direction,
        reliability: base_rate.reliability,
        n_neighbors: base_rate.k,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knn_base_rate::KnnBaseRate;

    fn base_rate(p_yes: f64, reliability: f64) -> KnnBaseRate {
        KnnBaseRate {
            p_yes,
            k: 10,
            n_corpus: 100,
            neighbors: Vec::new(),
            mean_similarity: reliability,
            reliability,
        }
    }

    #[test]
    fn flags_overpriced_market() {
        // Neighbors resolve YES 30% of the time; the market prices it at 0.75 → overpriced.
        let flag = detect_mispricing(&base_rate(0.30, 0.9), 0.75, 0.15).unwrap();
        assert!(flag.flagged);
        assert_eq!(flag.direction, "overpriced");
        assert!((flag.divergence - 0.45).abs() < 1e-9);
    }

    #[test]
    fn aligned_market_not_flagged() {
        let flag = detect_mispricing(&base_rate(0.60, 0.9), 0.62, 0.15).unwrap();
        assert!(!flag.flagged);
        assert_eq!(flag.direction, "aligned");
    }

    #[test]
    fn unreliable_consensus_not_flagged() {
        // Big divergence but the neighbor consensus is unreliable → do not flag.
        let flag = detect_mispricing(&base_rate(0.30, 0.1), 0.80, 0.15).unwrap();
        assert!(!flag.flagged, "unreliable consensus must not raise a flag");
    }

    #[test]
    fn invalid_price_fails_closed() {
        assert_eq!(
            detect_mispricing(&base_rate(0.5, 0.9), 1.5, 0.1)
                .unwrap_err()
                .code(),
            ERR_MISPRICING_INPUT
        );
    }
}
