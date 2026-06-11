use std::collections::HashMap;
use std::sync::Arc;

use calyx_core::{CalyxError, Clock, Constellation, Result};
use serde::{Deserialize, Serialize};

mod codec;
mod regression;
mod sleep_pass;
mod storage;
mod update;

pub use codec::{
    decode_head_rows, decode_online_head, encode_online_head, head_key, head_state_artifact_key,
};
pub use regression::{HeadRegressionRollback, RegressionUpdateOutcome};
pub use sleep_pass::{
    CALYX_ANNEAL_SLEEP_PASS_INVALID_CONFIG, DEFAULT_SLEEP_PASS_BATCH_SIZE,
    DEFAULT_SLEEP_PASS_MIN_SURPRISE, SleepPassConfig, SleepPassOutcome, SleepPassReplayRecord,
    record_mistake_for_replay, run_sleep_pass,
};
pub use storage::{AsterHeadStorage, HeadStorage};
pub use update::{HeadPromotionGate, HeadShadowProposal};

use super::{FrozenLensCheck, NoFrozenLensGuard, ReplayEntry};
use crate::{AnnealLedgerAction, ChangeId, ChangeOutcome, LogicalTime};
use codec::{encode_head_rows, heads_hash};
use update::{apply_update, update_reverted, validate_update};

pub const MAX_ONLINE_HEAD_PARAMS: usize = 1024;
pub const CALYX_ANNEAL_HEAD_TOO_LARGE: &str = "CALYX_ANNEAL_HEAD_TOO_LARGE";
pub const CALYX_ANNEAL_HEAD_INVALID_ROW: &str = "CALYX_ANNEAL_HEAD_INVALID_ROW";
pub const CALYX_ANNEAL_HEAD_UPDATE_REVERTED: &str = "CALYX_ANNEAL_HEAD_UPDATE_REVERTED";

const ONLINE_HEAD_TAG: &str = "anneal_online_head_v1";
const HEAD_KEY_PREFIX: &[u8] = b"head/v1/";
const STATE_KEY_HASH_SEED: &[u8] = b"anneal-online-head-state-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeadKind {
    Predictor,
    Calibrator,
    FusionWeights,
}

impl HeadKind {
    pub const ALL: [Self; 3] = [Self::Predictor, Self::Calibrator, Self::FusionWeights];

    pub const fn key(self) -> &'static str {
        match self {
            Self::Predictor => "predictor",
            Self::Calibrator => "calibrator",
            Self::FusionWeights => "fusion_weights",
        }
    }

    pub fn from_label(value: &str) -> Result<Self> {
        match value {
            "Predictor" | "predictor" => Ok(Self::Predictor),
            "Calibrator" | "calibrator" => Ok(Self::Calibrator),
            "FusionWeights" | "fusion_weights" | "fusion-weights" => Ok(Self::FusionWeights),
            _ => Err(invalid_row(format!("unknown online head kind {value:?}"))),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OnlineHead {
    pub kind: HeadKind,
    pub params: Vec<f32>,
    pub fisher_diag: Vec<f32>,
    pub version: u64,
    #[serde(default)]
    pub(crate) prior_params: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeadReadback {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub head: OnlineHead,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeadUpdateSummary {
    pub kind: HeadKind,
    pub prior_version: u64,
    pub version: u64,
    pub param_count: usize,
    pub param_norm: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeadUpdateOutcome {
    pub promoted: bool,
    pub change_id: Option<ChangeId>,
    pub batch_len: usize,
    pub updated_at: LogicalTime,
    pub heads: Vec<HeadUpdateSummary>,
}

pub struct OnlineHeadState<S, G, F = NoFrozenLensGuard> {
    heads: HashMap<HeadKind, OnlineHead>,
    storage: S,
    substrate: G,
    clock: Arc<dyn Clock>,
    frozen_guard: F,
}

impl<S, G> OnlineHeadState<S, G, NoFrozenLensGuard>
where
    S: HeadStorage,
    G: HeadPromotionGate,
{
    pub fn open(
        storage: S,
        substrate: G,
        clock: Arc<dyn Clock>,
        heads: impl IntoIterator<Item = OnlineHead>,
    ) -> Result<Self> {
        Self::open_with_guard(storage, substrate, clock, heads, NoFrozenLensGuard)
    }

    pub fn open_default(storage: S, substrate: G, clock: Arc<dyn Clock>) -> Result<Self> {
        Self::open(storage, substrate, clock, default_heads()?)
    }
}

impl<S, G, F> OnlineHeadState<S, G, F>
where
    S: HeadStorage,
    G: HeadPromotionGate,
    F: FrozenLensCheck,
{
    pub fn open_with_guard(
        storage: S,
        substrate: G,
        clock: Arc<dyn Clock>,
        heads: impl IntoIterator<Item = OnlineHead>,
        frozen_guard: F,
    ) -> Result<Self> {
        let mut map = HashMap::new();
        for head in heads {
            validate_head(&head)?;
            if map.insert(head.kind, head).is_some() {
                return Err(invalid_row("duplicate online head kind"));
            }
        }
        for kind in HeadKind::ALL {
            if let Some(bytes) = storage.load_head(kind)? {
                let head = decode_online_head(&bytes)?;
                if head.kind != kind {
                    return Err(invalid_row("anneal_heads row kind does not match key"));
                }
                map.insert(kind, head);
            }
        }
        Ok(Self {
            heads: map,
            storage,
            substrate,
            clock,
            frozen_guard,
        })
    }

    pub fn open_default_with_guard(
        storage: S,
        substrate: G,
        clock: Arc<dyn Clock>,
        frozen_guard: F,
    ) -> Result<Self> {
        Self::open_with_guard(storage, substrate, clock, default_heads()?, frozen_guard)
    }

    pub fn update(
        &mut self,
        batch: &[ReplayEntry],
        lr: f32,
        fisher_weight: f32,
    ) -> Result<HeadUpdateOutcome> {
        self.frozen_guard.assert_no_violation()?;
        let outcome = self.update_inner(batch, lr, fisher_weight);
        post_update_guard(&self.frozen_guard)?;
        outcome
    }

    fn update_inner(
        &mut self,
        batch: &[ReplayEntry],
        lr: f32,
        fisher_weight: f32,
    ) -> Result<HeadUpdateOutcome> {
        validate_update(batch, lr, fisher_weight)?;
        if batch.is_empty() || lr == 0.0 || self.heads.is_empty() {
            return Ok(HeadUpdateOutcome {
                promoted: false,
                change_id: None,
                batch_len: batch.len(),
                updated_at: self.clock.now(),
                heads: self.summaries(),
            });
        }
        let candidate_heads = self.candidate_heads(batch, lr, fisher_weight)?;
        let prior_ptr = crate::ArtifactPtr::ConfigCacheKeyHash(heads_hash(self.sorted_heads()?));
        let candidate_ptr =
            crate::ArtifactPtr::ConfigCacheKeyHash(heads_hash(candidate_heads.clone()));
        let key = head_state_artifact_key();
        self.substrate.ensure_head_prior(key.clone(), prior_ptr)?;
        let description = format!(
            "online_head_update batch={} lr={lr:.6} fisher_weight={fisher_weight:.6}",
            batch.len()
        );
        let proposal = HeadShadowProposal::stable();
        match self.substrate.propose_head_change(
            key,
            candidate_ptr,
            &proposal,
            &proposal,
            &description,
        )? {
            ChangeOutcome::Promoted(change_id) => {
                let rows = encode_head_rows(&candidate_heads)?;
                self.storage.save_heads(rows)?;
                let prior_heads = std::mem::take(&mut self.heads);
                self.heads = candidate_heads
                    .into_iter()
                    .map(|head| (head.kind, head))
                    .collect();
                Ok(HeadUpdateOutcome {
                    promoted: true,
                    change_id: Some(change_id),
                    batch_len: batch.len(),
                    updated_at: self.clock.now(),
                    heads: summaries_from_maps(&prior_heads, &self.heads),
                })
            }
            ChangeOutcome::Reverted { reason, .. } => Err(update_reverted(reason)),
        }
    }

    pub fn predict(&self, cx: &Constellation) -> f64 {
        let Some(head) = self.heads.get(&HeadKind::Predictor) else {
            return 0.0;
        };
        dot(&head.params, &constellation_features(cx, head.params.len())) as f64
    }

    pub fn calibrate(&self, raw_score: f64) -> f64 {
        let Some(head) = self.heads.get(&HeadKind::Calibrator) else {
            return raw_score;
        };
        let slope = head.params.first().copied().unwrap_or(1.0) as f64;
        let intercept = head.params.get(1).copied().unwrap_or(0.0) as f64;
        sigmoid(slope.mul_add(raw_score, intercept))
    }

    pub fn fusion_weights(&self) -> &[f32] {
        self.heads
            .get(&HeadKind::FusionWeights)
            .map(|head| head.params.as_slice())
            .unwrap_or(&[])
    }

    pub fn head(&self, kind: HeadKind) -> Option<&OnlineHead> {
        self.heads.get(&kind)
    }

    pub fn readback(&self) -> Result<Vec<HeadReadback>> {
        decode_head_rows(self.storage.scan_heads()?)
    }

    pub fn record_outcome_event(
        &mut self,
        action: AnnealLedgerAction,
        change_id: ChangeId,
        artifact_id: String,
        candidate_hash: [u8; 32],
        description: String,
    ) -> Result<()> {
        self.substrate.record_outcome_event(
            action,
            change_id,
            artifact_id,
            candidate_hash,
            description,
        )
    }

    fn candidate_heads(
        &self,
        batch: &[ReplayEntry],
        lr: f32,
        fisher_weight: f32,
    ) -> Result<Vec<OnlineHead>> {
        self.sorted_heads()?
            .iter()
            .map(|head| apply_update(head, batch, lr, fisher_weight))
            .collect()
    }

    fn sorted_heads(&self) -> Result<Vec<OnlineHead>> {
        let mut heads = self.heads.values().cloned().collect::<Vec<_>>();
        heads.sort_by_key(|head| head.kind.key());
        for head in &heads {
            validate_head(head)?;
        }
        Ok(heads)
    }

    fn summaries(&self) -> Vec<HeadUpdateSummary> {
        let mut heads = self.heads.values().collect::<Vec<_>>();
        heads.sort_by_key(|head| head.kind.key());
        heads
            .into_iter()
            .map(|head| HeadUpdateSummary::from_head(head, head.version))
            .collect()
    }
}

fn post_update_guard<F: FrozenLensCheck>(guard: &F) -> Result<()> {
    match guard.assert_no_violation() {
        Ok(()) => Ok(()),
        Err(error) => {
            #[cfg(debug_assertions)]
            panic!("frozen lens violation after anneal update: {error}");
            #[cfg(not(debug_assertions))]
            {
                Err(error)
            }
        }
    }
}

impl OnlineHead {
    pub fn new(kind: HeadKind, params: Vec<f32>) -> Result<Self> {
        let fisher_diag = vec![0.0; params.len()];
        Self::with_fisher(kind, params, fisher_diag, 0)
    }

    pub fn with_fisher(
        kind: HeadKind,
        params: Vec<f32>,
        fisher_diag: Vec<f32>,
        version: u64,
    ) -> Result<Self> {
        let head = Self {
            kind,
            prior_params: params.clone(),
            params,
            fisher_diag,
            version,
        };
        validate_head(&head)?;
        Ok(head)
    }

    pub fn param_norm(&self) -> f64 {
        norm(&self.params)
    }
}

impl HeadUpdateSummary {
    fn from_head(head: &OnlineHead, prior_version: u64) -> Self {
        Self {
            kind: head.kind,
            prior_version,
            version: head.version,
            param_count: head.params.len(),
            param_norm: head.param_norm(),
        }
    }
}

pub(crate) fn validate_head(head: &OnlineHead) -> Result<()> {
    if head.params.len() > MAX_ONLINE_HEAD_PARAMS {
        return Err(CalyxError {
            code: CALYX_ANNEAL_HEAD_TOO_LARGE,
            message: format!(
                "{:?} head has {} params; max is {MAX_ONLINE_HEAD_PARAMS}",
                head.kind,
                head.params.len()
            ),
            remediation: "split or shrink the derived online head",
        });
    }
    if head.fisher_diag.len() != head.params.len() || head.prior_params.len() != head.params.len() {
        return Err(invalid_row(
            "params, fisher_diag, and prior_params lengths differ",
        ));
    }
    if !head.params.iter().all(|value| value.is_finite())
        || !head
            .fisher_diag
            .iter()
            .all(|value| value.is_finite() && *value >= 0.0)
        || !head.prior_params.iter().all(|value| value.is_finite())
    {
        return Err(invalid_row(
            "online head params and fisher values must be finite",
        ));
    }
    Ok(())
}

pub(crate) fn invalid_row(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_HEAD_INVALID_ROW,
        message: message.into(),
        remediation: "rewrite the anneal_heads row from a validated OnlineHead",
    }
}

fn summaries_from_maps(
    prior: &HashMap<HeadKind, OnlineHead>,
    updated: &HashMap<HeadKind, OnlineHead>,
) -> Vec<HeadUpdateSummary> {
    let mut heads = updated.values().collect::<Vec<_>>();
    heads.sort_by_key(|head| head.kind.key());
    heads
        .into_iter()
        .map(|head| {
            let prior_version = prior
                .get(&head.kind)
                .map_or(head.version, |old| old.version);
            HeadUpdateSummary::from_head(head, prior_version)
        })
        .collect()
}

fn default_heads() -> Result<Vec<OnlineHead>> {
    Ok(vec![
        OnlineHead::new(HeadKind::Predictor, vec![0.0])?,
        OnlineHead::new(HeadKind::Calibrator, vec![1.0, 0.0])?,
        OnlineHead::new(HeadKind::FusionWeights, vec![1.0, 1.0, 1.0])?,
    ])
}

fn constellation_features(cx: &Constellation, len: usize) -> Vec<f32> {
    let mut features = Vec::with_capacity(len);
    for index in 0..len {
        let value = if index == 0 {
            1.0
        } else if let Some(value) = cx.scalars.values().nth(index - 1) {
            *value as f32
        } else {
            cx.slots
                .values()
                .filter_map(|slot| slot.as_dense())
                .flatten()
                .nth(index.saturating_sub(1 + cx.scalars.len()))
                .copied()
                .unwrap_or(0.0)
        };
        features.push(value);
    }
    features
}

pub(crate) fn dot(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right.iter())
        .map(|(left, right)| left * right)
        .sum()
}

pub(crate) fn norm(values: &[f32]) -> f64 {
    values
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>()
        .sqrt()
}

fn sigmoid(value: f64) -> f64 {
    if value >= 0.0 {
        1.0 / (1.0 + (-value).exp())
    } else {
        let exp = value.exp();
        exp / (1.0 + exp)
    }
}
