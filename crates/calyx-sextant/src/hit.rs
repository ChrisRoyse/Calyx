//! Provenanced search hit types.

use calyx_core::{CxId, LedgerRef, SlotId};
use serde::{Deserialize, Serialize};

use crate::util::hex32;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PerLensContribution {
    pub slot: SlotId,
    pub rank: usize,
    pub raw_score: f32,
    pub weight: f32,
    pub contribution: f32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreshnessTag {
    pub built_at_seq: u64,
    pub base_seq: u64,
    pub stale_by: u64,
    pub policy: String,
}

impl FreshnessTag {
    pub fn fresh(seq: u64) -> Self {
        Self {
            built_at_seq: seq,
            base_seq: seq,
            stale_by: 0,
            policy: "fresh_derived".to_string(),
        }
    }

    pub fn stale_ok(built_at_seq: u64, base_seq: u64) -> Self {
        Self {
            built_at_seq,
            base_seq,
            stale_by: base_seq.saturating_sub(built_at_seq),
            policy: "stale_ok".to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExplainBreakdown {
    pub strategy: String,
    pub per_lens_count: usize,
    pub provenance_hex: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceSource {
    Stored,
    Stub,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hit {
    pub cx_id: CxId,
    pub score: f32,
    pub rank: usize,
    pub per_lens: Vec<PerLensContribution>,
    pub cross_terms_used: bool,
    pub guard: Option<String>,
    pub provenance: LedgerRef,
    pub provenance_source: ProvenanceSource,
    pub freshness: FreshnessTag,
    pub explain: Option<ExplainBreakdown>,
}

impl Hit {
    pub fn with_explain(mut self, strategy: impl Into<String>) -> Self {
        self.explain = Some(ExplainBreakdown {
            strategy: strategy.into(),
            per_lens_count: self.per_lens.len(),
            provenance_hex: hex32(&self.provenance.hash),
        });
        self
    }
}
