use std::collections::BTreeMap;

use calyx_core::{Constellation, CxId, SlotId};
use calyx_sextant::{FusionStrategy, Hit, RrfProfile};

use crate::error::{CliResult, SearchError};

/// Fusion strategy choice (transport-agnostic; the CLI flag parser and the HTTP
/// request both map onto this, then it resolves to a concrete `FusionStrategy`
/// against the live slot set).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FusionChoice {
    Rrf,
    WeightedRrf,
    WeightedRrfProfile(RrfProfile),
    SingleLens,
    SingleLensSlot(SlotId),
    KernelFirst,
    Pipeline,
}

impl FusionChoice {
    pub fn to_strategy(self, slots: &[SlotId]) -> CliResult<FusionStrategy> {
        match self {
            Self::Rrf => Ok(FusionStrategy::Rrf),
            Self::WeightedRrf => Ok(FusionStrategy::WeightedRrf {
                profile: RrfProfile::General,
            }),
            Self::WeightedRrfProfile(profile) => Ok(FusionStrategy::WeightedRrf { profile }),
            Self::SingleLens => slots
                .first()
                .copied()
                .map(|slot| FusionStrategy::SingleLens { slot })
                .ok_or_else(|| SearchError::usage("single-lens search has no active lens slot")),
            Self::SingleLensSlot(slot) => {
                if slots.contains(&slot) {
                    Ok(FusionStrategy::SingleLens { slot })
                } else {
                    Err(SearchError::usage(format!(
                        "single-lens search requested slot {slot}, but the slot has no active persisted search results"
                    )))
                }
            }
            Self::KernelFirst => Ok(FusionStrategy::WeightedRrf {
                profile: RrfProfile::Kernel,
            }),
            Self::Pipeline => Ok(FusionStrategy::Pipeline),
        }
    }
}

/// Guard choice for a search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardChoice {
    Off,
    InRegion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchFreshness {
    Fresh,
    StaleOk,
}

/// The result of a search: ranked hits (each carrying score + stored
/// provenance) and the guard tau actually applied (if any).
pub struct SearchOutcome {
    pub hits: Vec<Hit>,
    pub guard_tau: Option<f32>,
    pub docs: BTreeMap<CxId, Constellation>,
}

impl SearchOutcome {
    pub(super) fn empty() -> Self {
        Self {
            hits: Vec::new(),
            guard_tau: None,
            docs: BTreeMap::new(),
        }
    }
}
