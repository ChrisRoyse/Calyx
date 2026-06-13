//! Stage 4 search query request types and freshness policy.

use std::collections::{BTreeMap, BTreeSet};

use calyx_core::{AnchorKind, AnchorValue, Modality, Result, SlotId, SlotVector, VaultId};
use calyx_ward::GuardProfile;
use serde::{Deserialize, Serialize};

use crate::error::{CALYX_SEXTANT_QUERY_SHAPE, sextant_error};
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
#[serde(rename_all = "snake_case")]
pub enum QueryGuard {
    InRegionOnly(GuardProfile),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Query {
    pub text: String,
    pub vector: Option<SlotVector>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub guard_vectors: BTreeMap<SlotId, SlotVector>,
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
    #[serde(default)]
    pub guard: Option<QueryGuard>,
}

impl Query {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            vector: None,
            guard_vectors: BTreeMap::new(),
            slots: Vec::new(),
            k: 10,
            ef: Some(64),
            recall_k: None,
            explain: false,
            require_stored_provenance: false,
            freshness: FreshnessRequirement::FreshDerived,
            fusion: None,
            filters: QueryFilters::default(),
            guard: None,
        }
    }

    pub fn with_vector(mut self, vector: SlotVector) -> Self {
        self.vector = Some(vector);
        self
    }

    pub fn with_guard_vectors(mut self, guard_vectors: BTreeMap<SlotId, SlotVector>) -> Self {
        self.guard_vectors = guard_vectors;
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

    pub fn with_guard(mut self, guard: QueryGuard) -> Self {
        self.guard = Some(guard);
        self
    }

    /// Validates caller-supplied query shape before index/search execution.
    pub fn validate(&self) -> Result<()> {
        if self.k == 0 {
            return Err(query_shape_error("query k must be greater than zero"));
        }
        if self.ef == Some(0) {
            return Err(query_shape_error("query ef must be greater than zero"));
        }
        if self.recall_k == Some(0) {
            return Err(query_shape_error(
                "query recall_k must be greater than zero",
            ));
        }
        let mut seen_slots = BTreeSet::new();
        for slot in &self.slots {
            if !seen_slots.insert(*slot) {
                return Err(query_shape_error(format!("duplicate query slot {slot}")));
            }
        }
        if let Some(vector) = &self.vector {
            validate_query_vector("query vector", vector)?;
        }
        for (slot, vector) in &self.guard_vectors {
            validate_query_vector(&format!("guard vector {slot}"), vector)?;
        }
        self.filters.validate()?;
        if let Some(QueryGuard::InRegionOnly(profile)) = &self.guard {
            validate_guard_profile(profile)?;
        }
        Ok(())
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

    pub fn validate(&self) -> Result<()> {
        for scalar in &self.scalars {
            scalar.validate()?;
        }
        for anchor in &self.anchors {
            anchor.validate()?;
        }
        for metadata in &self.metadata {
            metadata.validate()?;
        }
        Ok(())
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

impl ScalarPredicate {
    fn validate(&self) -> Result<()> {
        if self.name.is_empty() {
            return Err(query_shape_error("scalar predicate name must not be empty"));
        }
        if !self.value.is_finite() {
            return Err(query_shape_error(format!(
                "scalar predicate {:?} is NaN or Inf",
                self.name
            )));
        }
        Ok(())
    }
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

impl AnchorPredicate {
    fn validate(&self) -> Result<()> {
        if let Some(value) = &self.value {
            validate_anchor_value(value)?;
        }
        if let Some(confidence) = self.min_confidence
            && (!confidence.is_finite() || !(0.0..=1.0).contains(&confidence))
        {
            return Err(query_shape_error(
                "anchor min_confidence must be finite and within [0, 1]",
            ));
        }
        Ok(())
    }
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

impl MetadataPredicate {
    fn validate(&self) -> Result<()> {
        match self {
            Self::CreatedAt {
                min: Some(min),
                max: Some(max),
            } if min > max => Err(query_shape_error(
                "created_at minimum must not exceed maximum",
            )),
            Self::InputPointerContains(fragment) if fragment.is_empty() => Err(query_shape_error(
                "input pointer substring must not be empty",
            )),
            Self::Vault(_)
            | Self::Modality(_)
            | Self::PanelVersion(_)
            | Self::CreatedAt { .. }
            | Self::InputRedacted(_)
            | Self::InputPointerContains(_) => Ok(()),
        }
    }
}

fn validate_query_vector(field: &str, vector: &SlotVector) -> Result<()> {
    match vector {
        SlotVector::Dense { dim, data } => {
            if *dim == 0 {
                return Err(query_shape_error(format!("{field} dense dim must be > 0")));
            }
            if data.len() != *dim as usize {
                return Err(query_shape_error(format!(
                    "{field} dense dim {dim} does not match {} values",
                    data.len()
                )));
            }
            ensure_finite(field, data)
        }
        SlotVector::Sparse { dim, entries } => {
            if *dim == 0 {
                return Err(query_shape_error(format!("{field} sparse dim must be > 0")));
            }
            let mut seen = BTreeSet::new();
            for entry in entries {
                if entry.idx >= *dim {
                    return Err(query_shape_error(format!(
                        "{field} sparse index {} outside dim {dim}",
                        entry.idx
                    )));
                }
                if !seen.insert(entry.idx) {
                    return Err(query_shape_error(format!(
                        "{field} sparse index {} is duplicated",
                        entry.idx
                    )));
                }
                if !entry.val.is_finite() {
                    return Err(query_shape_error(format!(
                        "{field} sparse index {} is NaN or Inf",
                        entry.idx
                    )));
                }
            }
            Ok(())
        }
        SlotVector::Multi { token_dim, tokens } => {
            if *token_dim == 0 {
                return Err(query_shape_error(format!("{field} token_dim must be > 0")));
            }
            if tokens.is_empty() {
                return Err(query_shape_error(format!(
                    "{field} must contain at least one token"
                )));
            }
            for (idx, token) in tokens.iter().enumerate() {
                if token.len() != *token_dim as usize {
                    return Err(query_shape_error(format!(
                        "{field} token {idx} length {} does not match token_dim {token_dim}",
                        token.len()
                    )));
                }
                ensure_finite(field, token)?;
            }
            Ok(())
        }
        SlotVector::Absent { .. } => Err(query_shape_error(format!(
            "{field} must be a concrete vector, not absent"
        ))),
    }
}

fn validate_anchor_value(value: &AnchorValue) -> Result<()> {
    match value {
        AnchorValue::Number(value) if !value.is_finite() => {
            Err(query_shape_error("anchor predicate number is NaN or Inf"))
        }
        AnchorValue::Vector(values) if values.is_empty() => Err(query_shape_error(
            "anchor predicate vector must not be empty",
        )),
        AnchorValue::Vector(values) if values.iter().any(|value| !value.is_finite()) => Err(
            query_shape_error("anchor predicate vector contains NaN or Inf"),
        ),
        AnchorValue::Bool(_)
        | AnchorValue::Enum(_)
        | AnchorValue::Number(_)
        | AnchorValue::OneHot(_)
        | AnchorValue::Text(_)
        | AnchorValue::Vector(_) => Ok(()),
    }
}

fn validate_guard_profile(profile: &GuardProfile) -> Result<()> {
    let mut required = BTreeSet::new();
    for slot in &profile.required_slots {
        if !required.insert(*slot) {
            return Err(query_shape_error(format!(
                "guard profile repeats required slot {slot}"
            )));
        }
    }
    for (slot, tau) in &profile.tau {
        if !tau.is_finite() || !(0.0..=1.0).contains(tau) {
            return Err(query_shape_error(format!(
                "guard profile tau for slot {slot} must be finite and within [0, 1]"
            )));
        }
    }
    Ok(())
}

fn ensure_finite(field: &str, values: &[f32]) -> Result<()> {
    if values.iter().all(|value| value.is_finite()) {
        return Ok(());
    }
    Err(query_shape_error(format!("{field} contains NaN or Inf")))
}

fn query_shape_error(message: impl Into<String>) -> calyx_core::CalyxError {
    sextant_error(CALYX_SEXTANT_QUERY_SHAPE, message)
}
