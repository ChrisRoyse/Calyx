use calyx_forge::{AutotuneKey, BestConfig};
use serde::{Deserialize, Serialize};

use crate::{AnnealLedgerAction, ChangeId, LogicalTime, ShapeKey};

pub const DEFAULT_AB_MIN_SAMPLES: usize = 100;
pub const CALYX_ANNEAL_TRIAL_ALREADY_ACTIVE: &str = "CALYX_ANNEAL_TRIAL_ALREADY_ACTIVE";
pub const CALYX_ANNEAL_TRIAL_NOT_ACTIVE: &str = "CALYX_ANNEAL_TRIAL_NOT_ACTIVE";
pub const CALYX_ANNEAL_TRIAL_INVALID_RESULT: &str = "CALYX_ANNEAL_TRIAL_INVALID_RESULT";
pub const CALYX_ANNEAL_AB_CACHE_WRITE_FAIL: &str = "CALYX_ANNEAL_AB_CACHE_WRITE_FAIL";

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ABResult {
    pub arm_idx: usize,
    pub latency_ns: u64,
    pub recall_k: f64,
    pub bits_per_anchor: f64,
    pub ts: LogicalTime,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ABTrial {
    pub key: ShapeKey,
    pub incumbent_idx: usize,
    pub candidate_idx: usize,
    pub results: Vec<ABResult>,
    pub min_samples: usize,
    pub verdict: Option<ABVerdict>,
    pub promotion_config: Option<ABPromotionConfig>,
}

impl ABTrial {
    pub fn new(key: ShapeKey, candidate_idx: usize, incumbent_idx: usize) -> Self {
        Self::with_min_samples(key, candidate_idx, incumbent_idx, DEFAULT_AB_MIN_SAMPLES)
    }

    pub fn with_min_samples(
        key: ShapeKey,
        candidate_idx: usize,
        incumbent_idx: usize,
        min_samples: usize,
    ) -> Self {
        Self {
            key,
            incumbent_idx,
            candidate_idx,
            results: Vec::new(),
            min_samples: min_samples.max(1),
            verdict: None,
            promotion_config: None,
        }
    }

    pub fn with_promotion_config(mut self, promotion_config: ABPromotionConfig) -> Self {
        self.promotion_config = Some(promotion_config);
        self
    }

    pub fn query_pairs(&self) -> usize {
        self.results.len() / 2
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ABPromotionConfig {
    pub key: AutotuneKey,
    pub config: BestConfig,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum ABVerdict {
    Promoted(ABVerdictRecord),
    Kept(ABVerdictRecord),
    Abandoned(ABVerdictRecord),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ABVerdictRecord {
    pub key: ShapeKey,
    pub incumbent_idx: usize,
    pub candidate_idx: usize,
    pub samples: usize,
    pub latency_before_ns: u64,
    pub latency_after_ns: u64,
    pub recall_before: f64,
    pub recall_after: f64,
    pub bits_before: f64,
    pub bits_after: f64,
    pub reason: String,
    pub change_id: ChangeId,
    pub ts: LogicalTime,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ABSummary {
    pub sample_count: usize,
    pub p99_latency_ns: u64,
    pub mean_latency_ns: u64,
    pub mean_recall_k: f64,
    pub mean_bits_per_anchor: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ABLedgerEvent {
    pub action: AnnealLedgerAction,
    pub record: ABVerdictRecord,
    pub prior_ptr_hash: [u8; 32],
    pub candidate_ptr_hash: [u8; 32],
}
