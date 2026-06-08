//! Planner-enriched explain metadata.

use serde::{Deserialize, Serialize};

use crate::fusion::FusionStrategy;
use crate::hit::Hit;
use crate::planner::{IntentLabel, PlannedQuery};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlannerExplain {
    pub intent: IntentLabel,
    pub strategy: FusionStrategy,
    pub override_used: bool,
    pub cost_estimate: u64,
    pub timeout_ms: u64,
    pub hits: Vec<Hit>,
}

impl PlannerExplain {
    pub fn new(plan: &PlannedQuery, hits: Vec<Hit>) -> Self {
        Self {
            intent: plan.intent,
            strategy: plan.strategy.clone(),
            override_used: plan.override_used,
            cost_estimate: plan.cost_estimate,
            timeout_ms: plan.timeout_ms,
            hits,
        }
    }
}
