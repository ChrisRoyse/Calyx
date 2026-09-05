//! The Polymarket domain roster and the **data-density-first** selection strategy.
//!
//! Decision (owner-directed): build for data density first. We stand up `Crypto` first — it has the
//! densest, most regular data (per-minute prices, deep books, high resolution cadence), the cleanest
//! oracle, and the easiest historical backfill — so the whole ingest → associate → ground → predict
//! loop can be proven on real data quickly. **Once the data engine is proven on crypto, we compare
//! `Politics` against it** (politics carries the strongest documented calibration edge — chronic
//! under-confidence — but is sparser and more event-driven).

use serde::{Deserialize, Serialize};

/// A Polymarket trading domain. Each maps to its own Calyx vault (bits/kernel/guard are per-domain).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Domain {
    /// Crypto price/threshold markets. Densest, most regular data — the launch domain.
    Crypto,
    /// Political/election markets. Strongest calibration edge, sparser data — the comparison domain.
    Politics,
    /// Sports markets.
    Sports,
    /// Macro/economics (rates, CPI, NFP).
    Economics,
    /// Weather markets.
    Weather,
    /// Culture/entertainment markets.
    Culture,
    /// Geopolitical/world-event markets.
    Geopolitics,
    /// "Mentions"/quote-count style markets.
    Mentions,
    /// Anything uncategorized.
    Other,
}

impl Domain {
    /// Stable lowercase slug used for vault names, metadata, and config keys.
    pub fn slug(self) -> &'static str {
        match self {
            Domain::Crypto => "crypto",
            Domain::Politics => "politics",
            Domain::Sports => "sports",
            Domain::Economics => "economics",
            Domain::Weather => "weather",
            Domain::Culture => "culture",
            Domain::Geopolitics => "geopolitics",
            Domain::Mentions => "mentions",
            Domain::Other => "other",
        }
    }
}

/// A qualitative 0..1 score of a domain along the axes that matter for *building the data engine*.
/// These are design priors (documented in `docs/prd/`), not live measurements — they order the
/// build, after which real per-domain sufficiency/kernel-recall/Brier numbers take over.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DomainDensityScore {
    /// How dense/regular the raw data is (price cadence, book depth, event frequency).
    pub data_density: f64,
    /// How clean/reliable the UMA oracle tends to be (fewer disputes/ambiguity).
    pub oracle_cleanliness: f64,
    /// How easy it is to backfill a large resolved-market history to reach the ≥50-anchor floor.
    pub backfill_ease: f64,
    /// Documented calibration edge potential (e.g. politics under-confidence).
    pub edge_potential: f64,
}

impl DomainDensityScore {
    /// Composite score used to order the build. Data-density-first weighting: density and backfill
    /// dominate so we reach a working, grounded loop fastest; edge potential is secondary until we
    /// have the data to exploit it.
    pub fn build_priority(&self) -> f64 {
        0.45 * self.data_density
            + 0.25 * self.backfill_ease
            + 0.20 * self.oracle_cleanliness
            + 0.10 * self.edge_potential
    }
}

/// The design-prior density score for a domain.
pub fn density_score(domain: Domain) -> DomainDensityScore {
    match domain {
        Domain::Crypto => DomainDensityScore {
            data_density: 0.95,
            oracle_cleanliness: 0.90,
            backfill_ease: 0.90,
            edge_potential: 0.45,
        },
        Domain::Politics => DomainDensityScore {
            data_density: 0.55,
            oracle_cleanliness: 0.70,
            backfill_ease: 0.75,
            edge_potential: 0.95,
        },
        Domain::Sports => DomainDensityScore {
            data_density: 0.85,
            oracle_cleanliness: 0.85,
            backfill_ease: 0.80,
            edge_potential: 0.55,
        },
        Domain::Economics => DomainDensityScore {
            data_density: 0.60,
            oracle_cleanliness: 0.85,
            backfill_ease: 0.55,
            edge_potential: 0.65,
        },
        Domain::Weather => DomainDensityScore {
            data_density: 0.65,
            oracle_cleanliness: 0.80,
            backfill_ease: 0.55,
            edge_potential: 0.60,
        },
        Domain::Geopolitics => DomainDensityScore {
            data_density: 0.40,
            oracle_cleanliness: 0.55,
            backfill_ease: 0.50,
            edge_potential: 0.60,
        },
        Domain::Culture => DomainDensityScore {
            data_density: 0.35,
            oracle_cleanliness: 0.60,
            backfill_ease: 0.45,
            edge_potential: 0.30,
        },
        Domain::Mentions => DomainDensityScore {
            data_density: 0.40,
            oracle_cleanliness: 0.65,
            backfill_ease: 0.45,
            edge_potential: 0.35,
        },
        Domain::Other => DomainDensityScore {
            data_density: 0.30,
            oracle_cleanliness: 0.50,
            backfill_ease: 0.40,
            edge_potential: 0.30,
        },
    }
}

/// All domains ordered by build priority (data-density-first). The head is the launch domain.
pub fn build_order() -> Vec<Domain> {
    let mut all = vec![
        Domain::Crypto,
        Domain::Politics,
        Domain::Sports,
        Domain::Economics,
        Domain::Weather,
        Domain::Geopolitics,
        Domain::Culture,
        Domain::Mentions,
        Domain::Other,
    ];
    all.sort_by(|a, b| {
        density_score(*b)
            .build_priority()
            .partial_cmp(&density_score(*a).build_priority())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    all
}

/// The primary launch domain (data-density-first).
pub fn primary_domain() -> Domain {
    build_order()[0]
}

/// The domain to compare against the primary once the data engine is proven.
pub fn comparison_domain() -> Domain {
    Domain::Politics
}

/// One-paragraph rationale, surfaced in logs and reports.
pub fn selection_rationale() -> &'static str {
    "Data-density-first: launch on Crypto (densest/most-regular data, cleanest oracle, easiest \
     backfill) to prove the ingest→associate→ground→predict loop on real data fastest; then compare \
     Politics (strongest calibration edge, sparser data) against the proven engine."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crypto_is_the_launch_domain() {
        assert_eq!(primary_domain(), Domain::Crypto);
        assert_eq!(build_order()[0], Domain::Crypto);
    }

    #[test]
    fn politics_is_the_comparison_domain() {
        assert_eq!(comparison_domain(), Domain::Politics);
    }

    #[test]
    fn crypto_outranks_politics_for_build() {
        assert!(
            density_score(Domain::Crypto).build_priority()
                > density_score(Domain::Politics).build_priority()
        );
    }

    #[test]
    fn slugs_are_stable() {
        assert_eq!(Domain::Crypto.slug(), "crypto");
        assert_eq!(Domain::Politics.slug(), "politics");
    }
}
