//! Query request types and freshness policy.

use calyx_core::{AnchorKind, AnchorValue, Modality, SlotId, SlotVector, VaultId};
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
    #[serde(default)]
    pub recall_k: Option<usize>,
    pub explain: bool,
    #[serde(default)]
    pub require_stored_provenance: bool,
    pub freshness: FreshnessRequirement,
    pub fusion: Option<FusionStrategy>,
    #[serde(default)]
    pub filters: QueryFilters,
}

impl Query {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            vector: None,
            slots: Vec::new(),
            k: 10,
            ef: Some(64),
            recall_k: None,
            explain: false,
            require_stored_provenance: false,
            freshness: FreshnessRequirement::FreshDerived,
            fusion: None,
            filters: QueryFilters::default(),
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

    pub fn require_stored_provenance(mut self, required: bool) -> Self {
        self.require_stored_provenance = required;
        self
    }

    pub fn with_filters(mut self, filters: QueryFilters) -> Self {
        self.filters = filters;
        self
    }

    pub fn with_recall_k(mut self, recall_k: usize) -> Self {
        self.recall_k = Some(recall_k);
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct QueryFilters {
    #[serde(default)]
    pub scalars: Vec<ScalarPredicate>,
    #[serde(default)]
    pub anchors: Vec<AnchorPredicate>,
    #[serde(default)]
    pub metadata: Vec<MetadataPredicate>,
}

impl QueryFilters {
    pub fn is_empty(&self) -> bool {
        self.scalars.is_empty() && self.anchors.is_empty() && self.metadata.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScalarOp {
    Eq,
    Gt,
    Gte,
    Lt,
    Lte,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScalarPredicate {
    pub name: String,
    pub op: ScalarOp,
    pub value: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnchorPredicate {
    pub kind: AnchorKind,
    #[serde(default)]
    pub value: Option<AnchorValue>,
    #[serde(default)]
    pub min_confidence: Option<f32>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataPredicate {
    Vault(VaultId),
    Modality(Modality),
    PanelVersion(u32),
    CreatedAt {
        #[serde(default)]
        min: Option<u64>,
        #[serde(default)]
        max: Option<u64>,
    },
    InputRedacted(bool),
    InputPointerContains(String),
}
