use calyx_aster::cf::full_content_hash;
use calyx_core::{Anchor, AnchorKind, AnchorValue, CalyxError, CxId, Result};
use serde::{Deserialize, Serialize};

use super::{
    FrozenLensCheck, HeadPromotionGate, HeadStorage, MistakeLog, MistakeRef, MistakeStorage,
    OnlineHeadState, ReplayBuffer, ReplayEntry, ReplayStorage, record_mistake_for_replay,
};
use crate::{
    AnnealLedgerAction, ChangeId, DEFAULT_MISTAKE_SURPRISE_THRESHOLD,
    DEFAULT_SLEEP_PASS_MIN_SURPRISE, HeadUpdateOutcome, LogicalTime,
};

mod queue;

pub use queue::{
    AsterOutcomeStorage, OutcomeQueue, OutcomeStorage, decode_outcome_queue_entry,
    encode_outcome_queue_entry, outcome_queue_key, outcome_queue_seq_from_key,
};

pub const DEFAULT_OUTCOME_ACTION_COST: f64 = 1.0;
pub const DEFAULT_OUTCOME_LR: f32 = 1.0;
pub const DEFAULT_OUTCOME_FISHER_WEIGHT: f32 = 0.0;
pub const CALYX_ANNEAL_OUTCOME_INVALID_CONFIG: &str = "CALYX_ANNEAL_OUTCOME_INVALID_CONFIG";
pub const CALYX_ANNEAL_OUTCOME_INVALID_ANCHOR: &str = "CALYX_ANNEAL_OUTCOME_INVALID_ANCHOR";
pub const CALYX_ANNEAL_OUTCOME_INVALID_ROW: &str = "CALYX_ANNEAL_OUTCOME_INVALID_ROW";
pub const CALYX_ANNEAL_OUTCOME_APPEND_ONLY: &str = "CALYX_ANNEAL_OUTCOME_APPEND_ONLY";

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutcomePrediction {
    pub value: f64,
    pub trusted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordOutcomeConfig {
    pub contradiction_threshold: f64,
    pub replay_min_surprise: f64,
    pub lr: f32,
    pub fisher_weight: f32,
    pub action_cost: f64,
}

impl Default for RecordOutcomeConfig {
    fn default() -> Self {
        Self {
            contradiction_threshold: DEFAULT_MISTAKE_SURPRISE_THRESHOLD,
            replay_min_surprise: DEFAULT_SLEEP_PASS_MIN_SURPRISE,
            lr: DEFAULT_OUTCOME_LR,
            fisher_weight: DEFAULT_OUTCOME_FISHER_WEIGHT,
            action_cost: DEFAULT_OUTCOME_ACTION_COST,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutcomeQueueEntry {
    pub seq: u64,
    pub cx_id: CxId,
    pub anchor: Anchor,
    pub observed: f64,
    pub predicted: Option<f64>,
    pub trusted_prediction: bool,
    pub surprise: f64,
    pub reward: f64,
    pub head_target: f64,
    pub expected_delta_j: f64,
    pub action_cost: f64,
    pub delta_j_per_cost: f64,
    pub ts: LogicalTime,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutcomeQueueReadback {
    pub seq: u64,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub entry: OutcomeQueueEntry,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordOutcomeReward {
    pub queue_seq: u64,
    pub observed: f64,
    pub reward: f64,
    pub expected_delta_j: f64,
    pub head_update: HeadUpdateOutcome,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordOutcomeContradiction {
    pub mistake_ref: MistakeRef,
    pub replay_added: bool,
    pub predicted: f64,
    pub observed: f64,
    pub surprise: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RecordOutcomeResult {
    Reward(RecordOutcomeReward),
    Contradiction(RecordOutcomeContradiction),
}

pub struct RecordOutcomeContext<'a, M, R, H, G, F, O>
where
    M: MistakeStorage,
    R: ReplayStorage,
    H: HeadStorage,
    G: HeadPromotionGate,
    F: FrozenLensCheck,
    O: OutcomeStorage,
{
    pub log: &'a MistakeLog<M>,
    pub replay: &'a mut ReplayBuffer<R>,
    pub heads: &'a mut OnlineHeadState<H, G, F>,
    pub outcomes: &'a OutcomeQueue<O>,
}

impl<'a, M, R, H, G, F, O> RecordOutcomeContext<'a, M, R, H, G, F, O>
where
    M: MistakeStorage,
    R: ReplayStorage,
    H: HeadStorage,
    G: HeadPromotionGate,
    F: FrozenLensCheck,
    O: OutcomeStorage,
{
    pub fn new(
        log: &'a MistakeLog<M>,
        replay: &'a mut ReplayBuffer<R>,
        heads: &'a mut OnlineHeadState<H, G, F>,
        outcomes: &'a OutcomeQueue<O>,
    ) -> Self {
        Self {
            log,
            replay,
            heads,
            outcomes,
        }
    }
}

pub fn record_outcome<M, R, H, G, F, O>(
    cx_id: CxId,
    anchor: Anchor,
    prediction: Option<OutcomePrediction>,
    context: &mut RecordOutcomeContext<'_, M, R, H, G, F, O>,
    config: RecordOutcomeConfig,
) -> Result<RecordOutcomeResult>
where
    M: MistakeStorage,
    R: ReplayStorage,
    H: HeadStorage,
    G: HeadPromotionGate,
    F: FrozenLensCheck,
    O: OutcomeStorage,
{
    validate_config(config)?;
    validate_prediction(prediction)?;
    let observed = observed_scalar(&anchor)?;
    context.outcomes.record_anchor(cx_id, &anchor)?;
    if let Some(trusted) = prediction.filter(|prediction| prediction.trusted) {
        let surprise = (trusted.value - observed).abs();
        if surprise >= config.contradiction_threshold {
            let record = record_mistake_for_replay(
                context.log,
                context.replay,
                cx_id,
                trusted.value,
                observed,
                anchor.kind.clone(),
                config.replay_min_surprise,
            )?;
            let result = RecordOutcomeContradiction {
                mistake_ref: record.mistake_ref,
                replay_added: record.replay_added,
                predicted: trusted.value,
                observed,
                surprise,
            };
            let bytes = queue::encode_ledger_value(&anchor, observed, surprise)?;
            context.heads.record_outcome_event(
                AnnealLedgerAction::OutcomeContradiction,
                change_id(queue::logical_ts(&anchor), record.mistake_ref.seq),
                artifact_id(cx_id, &anchor.kind),
                full_content_hash([bytes.as_slice()]),
                format!(
                    "record_outcome contradiction cx={cx_id} predicted={:.6} observed={observed:.6} surprise={surprise:.6} replay_added={}",
                    trusted.value, record.replay_added
                ),
            )?;
            return Ok(RecordOutcomeResult::Contradiction(result));
        }
    }

    let entry = build_reward_entry(cx_id, anchor, observed, prediction, config)?;
    let entry = context.outcomes.push(entry)?;
    let training = ReplayEntry::new(
        cx_id,
        entry.head_target,
        MistakeRef {
            seq: entry.seq,
            surprise: entry.head_target,
        },
        entry.ts,
    )?;
    let update = context
        .heads
        .update(&[training], config.lr, config.fisher_weight)?;
    let hash = full_content_hash([encode_outcome_queue_entry(&entry)?.as_slice()]);
    context.heads.record_outcome_event(
        AnnealLedgerAction::OutcomeReward,
        change_id(entry.ts, entry.seq),
        artifact_id(cx_id, &entry.anchor.kind),
        hash,
        format!(
            "record_outcome reward cx={cx_id} observed={:.6} reward={:.6} delta_j_per_cost={:.6} queue_seq={}",
            entry.observed, entry.reward, entry.delta_j_per_cost, entry.seq
        ),
    )?;
    Ok(RecordOutcomeResult::Reward(RecordOutcomeReward {
        queue_seq: entry.seq,
        observed: entry.observed,
        reward: entry.reward,
        expected_delta_j: entry.expected_delta_j,
        head_update: update,
    }))
}

fn build_reward_entry(
    cx_id: CxId,
    anchor: Anchor,
    observed: f64,
    prediction: Option<OutcomePrediction>,
    config: RecordOutcomeConfig,
) -> Result<OutcomeQueueEntry> {
    let predicted = prediction.map(|prediction| prediction.value);
    let surprise = predicted.map_or(0.0, |value| (value - observed).abs());
    let reward = reward_score(&anchor, observed, surprise)?;
    let expected_delta_j = reward * (1.0 - surprise).max(0.0);
    Ok(OutcomeQueueEntry {
        seq: 0,
        cx_id,
        anchor,
        observed,
        predicted,
        trusted_prediction: prediction.is_some_and(|prediction| prediction.trusted),
        surprise,
        reward,
        head_target: observed,
        expected_delta_j,
        action_cost: config.action_cost,
        delta_j_per_cost: expected_delta_j / config.action_cost,
        ts: 0,
    })
}

fn observed_scalar(anchor: &Anchor) -> Result<f64> {
    validate_anchor(anchor)?;
    match &anchor.value {
        AnchorValue::Bool(value) => Ok(if *value { 1.0 } else { 0.0 }),
        AnchorValue::Number(value) if (0.0..=1.0).contains(value) => Ok(*value),
        AnchorValue::Number(_) => Err(invalid_anchor("numeric outcomes must be in [0, 1]")),
        AnchorValue::Enum(value) | AnchorValue::Text(value) => label_scalar(value),
        AnchorValue::OneHot(values) if values.len() == 1 => label_scalar(&values[0]),
        AnchorValue::OneHot(_) => Err(invalid_anchor(
            "one-hot outcome must have exactly one label",
        )),
        AnchorValue::Vector(_) => Err(invalid_anchor("vector anchors cannot be scalar outcomes")),
    }
}

fn label_scalar(value: &str) -> Result<f64> {
    match value.trim().to_ascii_lowercase().as_str() {
        "pass" | "passed" | "true" | "yes" | "ok" | "positive" | "thumbs_up" | "tie" => Ok(1.0),
        "fail" | "failed" | "false" | "no" | "negative" | "thumbs_down" | "untied" => Ok(0.0),
        _ => Err(invalid_anchor(format!(
            "unsupported scalar outcome label {value:?}"
        ))),
    }
}

fn reward_score(anchor: &Anchor, observed: f64, surprise: f64) -> Result<f64> {
    if !surprise.is_finite() || surprise < 0.0 {
        return Err(invalid_row("outcome surprise must be finite and >= 0"));
    }
    Ok((observed * f64::from(anchor.confidence)).clamp(0.0, 1.0))
}

fn validate_config(config: RecordOutcomeConfig) -> Result<()> {
    if !config.contradiction_threshold.is_finite() || config.contradiction_threshold < 0.0 {
        return Err(invalid_config(
            "contradiction threshold must be finite and >= 0",
        ));
    }
    if !config.replay_min_surprise.is_finite() || config.replay_min_surprise < 0.0 {
        return Err(invalid_config(
            "replay_min_surprise must be finite and >= 0",
        ));
    }
    if !config.lr.is_finite() || config.lr < 0.0 {
        return Err(invalid_config(
            "outcome learning rate must be finite and >= 0",
        ));
    }
    if !config.fisher_weight.is_finite() || config.fisher_weight < 0.0 {
        return Err(invalid_config(
            "outcome fisher_weight must be finite and >= 0",
        ));
    }
    if !config.action_cost.is_finite() || config.action_cost <= 0.0 {
        return Err(invalid_config("outcome action_cost must be finite and > 0"));
    }
    Ok(())
}

fn validate_prediction(prediction: Option<OutcomePrediction>) -> Result<()> {
    if let Some(prediction) = prediction
        && (!prediction.value.is_finite() || !(0.0..=1.0).contains(&prediction.value))
    {
        return Err(invalid_config("outcome prediction value must be in [0, 1]"));
    }
    Ok(())
}

pub(super) fn validate_anchor(anchor: &Anchor) -> Result<()> {
    anchor
        .validate_schema()
        .map_err(|error| invalid_anchor(format!("{}: {}", error.code, error.message)))
}

pub(super) fn validate_entry(entry: &OutcomeQueueEntry) -> Result<()> {
    if entry.seq == 0 {
        return Err(invalid_row("outcome queue seq must be > 0"));
    }
    validate_entry_without_seq(entry)
}

pub(super) fn validate_entry_without_seq(entry: &OutcomeQueueEntry) -> Result<()> {
    validate_anchor(&entry.anchor)?;
    for (name, value) in [
        ("observed", entry.observed),
        ("surprise", entry.surprise),
        ("reward", entry.reward),
        ("head_target", entry.head_target),
        ("expected_delta_j", entry.expected_delta_j),
        ("action_cost", entry.action_cost),
        ("delta_j_per_cost", entry.delta_j_per_cost),
    ] {
        if !value.is_finite() {
            return Err(invalid_row(format!("{name} must be finite")));
        }
    }
    if entry.action_cost <= 0.0 {
        return Err(invalid_row("action_cost must be > 0"));
    }
    if !(0.0..=1.0).contains(&entry.observed)
        || !(0.0..=1.0).contains(&entry.reward)
        || !(0.0..=1.0).contains(&entry.head_target)
    {
        return Err(invalid_row(
            "observed, reward, and head_target must be in [0, 1]",
        ));
    }
    if let Some(predicted) = entry.predicted
        && (!predicted.is_finite() || !(0.0..=1.0).contains(&predicted))
    {
        return Err(invalid_row("predicted outcome must be in [0, 1]"));
    }
    Ok(())
}

fn change_id(ts: LogicalTime, seq: u64) -> ChangeId {
    ChangeId(ts.saturating_mul(1_000_000).saturating_add(seq).max(1))
}

fn artifact_id(_cx_id: CxId, kind: &AnchorKind) -> String {
    format!("outcome_{}", anchor_kind_label(kind))
}

fn anchor_kind_label(kind: &AnchorKind) -> String {
    match kind {
        AnchorKind::TestPass => "test_pass".to_string(),
        AnchorKind::TieFormed => "tie_formed".to_string(),
        AnchorKind::Thumbs => "thumbs".to_string(),
        AnchorKind::Label(_) => "label".to_string(),
        AnchorKind::Reward => "reward".to_string(),
        AnchorKind::SpeakerMatch => "speaker_match".to_string(),
        AnchorKind::StyleHold => "style_hold".to_string(),
        AnchorKind::Recurrence => "recurrence".to_string(),
    }
}

fn invalid_config(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_OUTCOME_INVALID_CONFIG,
        message: message.into(),
        remediation: "use finite non-negative record_outcome tuning values",
    }
}

fn invalid_anchor(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_OUTCOME_INVALID_ANCHOR,
        message: message.into(),
        remediation: "record a scalar grounded outcome anchor before anneal reward training",
    }
}

pub(super) fn invalid_row(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_OUTCOME_INVALID_ROW,
        message: message.into(),
        remediation: "repair or quarantine online outcome queue rows before reward training",
    }
}
