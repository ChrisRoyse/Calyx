//! Query request types and freshness policy.

use calyx_core::{SlotId, SlotVector};
use serde::{Deserialize, Serialize};

use crate::fusion::FusionStrategy;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessRequirement {
    #[default]
    FreshDerived,
    StaleOk {
        seq_lag: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Query {
    pub text: String,
    pub vector: Option<SlotVector>,
    pub slots: Vec<SlotId>,
    pub k: usize,
    pub ef: Option<usize>,
    pub explain: bool,
    pub freshness: FreshnessRequirement,
    pub fusion: Option<FusionStrategy>,
}

impl Query {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            vector: None,
            slots: Vec::new(),
            k: 10,
            ef: Some(64),
            explain: false,
            freshness: FreshnessRequirement::FreshDerived,
            fusion: None,
        }
    }

    pub fn with_vector(mut self, vector: SlotVector) -> Self {
        self.vector = Some(vector);
        self
    }

    pub fn with_slots(mut self, slots: impl Into<Vec<SlotId>>) -> Self {
        self.slots = slots.into();
        self
    }

    pub fn explain(mut self, explain: bool) -> Self {
        self.explain = explain;
        self
    }
}
