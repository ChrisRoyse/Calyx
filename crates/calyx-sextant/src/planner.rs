//! Deterministic intent classifier and bounded query planner.

use calyx_core::{Result, SlotId};
use serde::{Deserialize, Serialize};

use crate::error::{
    CALYX_SEXTANT_NO_LENSES, CALYX_SEXTANT_PLAN_COST_EXCEEDED, CALYX_SEXTANT_PLAN_UNBOUNDED,
    sextant_error,
};
use crate::fusion::{FusionStrategy, RrfProfile};
use crate::query::Query;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentLabel {
    Causal,
    Code,
    Entity,
    Temporal,
    Speaker,
    Style,
    Civic,
    Media,
    Bridge,
    Kernel,
    Semantic,
    Lexical,
    Multimodal,
    General,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanLimits {
    pub max_k: usize,
    pub max_ef: usize,
    pub max_slots: usize,
    pub max_cost: u64,
    pub timeout_ms: u64,
}

impl Default for PlanLimits {
    fn default() -> Self {
        Self {
            max_k: 100,
            max_ef: 512,
            max_slots: 16,
            max_cost: 20_000_000,
            timeout_ms: 5_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlannedQuery {
    pub query: Query,
    pub intent: IntentLabel,
    pub strategy: FusionStrategy,
    pub override_used: bool,
    pub cost_estimate: u64,
    pub timeout_ms: u64,
}

#[derive(Clone, Debug)]
pub struct QueryPlanner {
    limits: PlanLimits,
}

impl QueryPlanner {
    pub fn new(limits: PlanLimits) -> Self {
        Self { limits }
    }

    pub fn classify(&self, query: &Query) -> IntentLabel {
        let text = query.text.to_lowercase();
        if any(
            &text,
            &["fn ", "rust", "stacktrace", "compile", "trait", "function"],
        ) {
            IntentLabel::Code
        } else if any(&text, &["because", "caused", "why", "causal", "leads to"]) {
            IntentLabel::Causal
        } else if any(
            &text,
            &["who", "entity", "person", "company", "organization"],
        ) {
            IntentLabel::Entity
        } else if any(
            &text,
            &["when", "recent", "yesterday", "recurring", "periodic"],
        ) {
            IntentLabel::Temporal
        } else if any(&text, &["speaker", "voice", "said"]) {
            IntentLabel::Speaker
        } else if any(&text, &["style", "tone", "register"]) {
            IntentLabel::Style
        } else if any(&text, &["polis", "vote", "civic", "policy"]) {
            IntentLabel::Civic
        } else if any(&text, &["image", "audio", "video", "media"]) {
            IntentLabel::Media
        } else if any(&text, &["bridge", "connect", "between"]) {
            IntentLabel::Bridge
        } else if any(&text, &["kernel", "grounding", "anchor"]) {
            IntentLabel::Kernel
        } else if any(&text, &["exact", "keyword", "literal", "bm25"]) {
            IntentLabel::Lexical
        } else if any(&text, &["multimodal", "cross modal"]) {
            IntentLabel::Multimodal
        } else if any(&text, &["meaning", "semantic", "similar"]) {
            IntentLabel::Semantic
        } else {
            IntentLabel::General
        }
    }

    pub fn plan(&self, mut query: Query, index_size: usize) -> Result<PlannedQuery> {
        let intent = self.classify(&query);
        let override_used = query.fusion.is_some();
        let strategy = query
            .fusion
            .clone()
            .unwrap_or_else(|| self.strategy_for(intent, &query));
        self.enforce_bounds(&query, index_size)?;
        let cost = self.estimate_cost(&query, index_size);
        self.enforce_cost(cost)?;
        query.fusion = Some(strategy.clone());
        Ok(PlannedQuery {
            query,
            intent,
            strategy,
            override_used,
            cost_estimate: cost,
            timeout_ms: self.limits.timeout_ms,
        })
    }

    pub fn strategy_for(&self, intent: IntentLabel, query: &Query) -> FusionStrategy {
        match intent {
            IntentLabel::Code => FusionStrategy::SingleLens {
                slot: query.slots.first().copied().unwrap_or(SlotId::new(8)),
            },
            IntentLabel::Lexical => FusionStrategy::WeightedRrf {
                profile: RrfProfile::Lexical,
            },
            IntentLabel::Causal => FusionStrategy::WeightedRrf {
                profile: RrfProfile::Causal,
            },
            IntentLabel::Temporal => FusionStrategy::WeightedRrf {
                profile: RrfProfile::Temporal,
            },
            IntentLabel::General => FusionStrategy::Rrf,
            other => FusionStrategy::WeightedRrf {
                profile: profile_for(other),
            },
        }
    }

    pub fn estimate_cost(&self, query: &Query, index_size: usize) -> u64 {
        let slots = query.slots.len().max(1) as u64;
        let ef = query.ef.unwrap_or(64) as u64;
        slots
            .saturating_mul(ef)
            .saturating_mul(query.k as u64)
            .saturating_mul(index_size.max(1) as u64)
            / 100
    }

    fn enforce_bounds(&self, query: &Query, index_size: usize) -> Result<()> {
        if query.k == 0 {
            return Err(sextant_error(
                CALYX_SEXTANT_PLAN_UNBOUNDED,
                "k must be greater than zero",
            ));
        }
        if query.k > self.limits.max_k {
            return Err(sextant_error(
                CALYX_SEXTANT_PLAN_UNBOUNDED,
                "k exceeds max_k",
            ));
        }
        if query.slots.is_empty() && index_size == 0 {
            return Err(sextant_error(
                CALYX_SEXTANT_NO_LENSES,
                "no registered lenses are available for the query",
            ));
        }
        let ef = query.ef.unwrap_or(64);
        if ef == 0 {
            return Err(sextant_error(
                CALYX_SEXTANT_PLAN_UNBOUNDED,
                "ef must be greater than zero",
            ));
        }
        if ef > self.limits.max_ef {
            return Err(sextant_error(
                CALYX_SEXTANT_PLAN_UNBOUNDED,
                "ef exceeds max_ef",
            ));
        }
        if query.slots.len() > self.limits.max_slots {
            return Err(sextant_error(
                CALYX_SEXTANT_PLAN_UNBOUNDED,
                "slot count exceeds max_slots",
            ));
        }
        Ok(())
    }

    fn enforce_cost(&self, cost: u64) -> Result<()> {
        if cost > self.limits.max_cost {
            return Err(sextant_error(
                CALYX_SEXTANT_PLAN_COST_EXCEEDED,
                "estimated cost exceeds cap",
            ));
        }
        Ok(())
    }
}

impl Default for QueryPlanner {
    fn default() -> Self {
        Self::new(PlanLimits::default())
    }
}

fn any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn profile_for(intent: IntentLabel) -> RrfProfile {
    match intent {
        IntentLabel::Causal => RrfProfile::Causal,
        IntentLabel::Code => RrfProfile::Code,
        IntentLabel::Entity => RrfProfile::Entity,
        IntentLabel::Temporal => RrfProfile::Temporal,
        IntentLabel::Speaker => RrfProfile::Speaker,
        IntentLabel::Style => RrfProfile::Style,
        IntentLabel::Civic => RrfProfile::Civic,
        IntentLabel::Media => RrfProfile::Media,
        IntentLabel::Bridge => RrfProfile::Bridge,
        IntentLabel::Kernel => RrfProfile::Kernel,
        IntentLabel::Semantic => RrfProfile::Semantic,
        IntentLabel::Lexical => RrfProfile::Lexical,
        IntentLabel::Multimodal => RrfProfile::Multimodal,
        IntentLabel::General => RrfProfile::General,
    }
}
