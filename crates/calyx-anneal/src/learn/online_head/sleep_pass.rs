use calyx_core::{AnchorKind, CalyxError, CxId, Result};
use serde::{Deserialize, Serialize};

use crate::{
    CALYX_ANNEAL_HEAD_UPDATE_REVERTED, CALYX_ANNEAL_REGRESSION_RECURRED, ComponentHealth,
    ComponentKind, DegradeRegistry, FrozenLensCheck, HeadPromotionGate, HeadRegressionRollback,
    HeadStorage, HealthStorage, MistakeLog, MistakeRef, MistakeStorage, OnlineHeadState,
    RegressionConfig, RegressionContextSource, RegressionUpdateOutcome, ReplayBuffer,
    ReplayStorage,
};

pub const DEFAULT_SLEEP_PASS_BATCH_SIZE: usize = 16;
pub const DEFAULT_SLEEP_PASS_MIN_SURPRISE: f64 = 0.01;
pub const CALYX_ANNEAL_SLEEP_PASS_INVALID_CONFIG: &str = "CALYX_ANNEAL_SLEEP_PASS_INVALID_CONFIG";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SleepPassConfig {
    pub batch_size: usize,
    pub seed: u64,
    pub lr: f32,
    pub fisher_weight: f32,
    pub regression: RegressionConfig,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SleepPassOutcome {
    Idle {
        reason: String,
        buffer_len: usize,
    },
    Deferred {
        degraded_components: Vec<String>,
        buffer_len: usize,
    },
    Promoted {
        update: RegressionUpdateOutcome,
    },
    Reverted {
        error_code: String,
        message: String,
        batch_len: usize,
        buffer_len: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SleepPassReplayRecord {
    pub mistake_ref: MistakeRef,
    pub surprise: f64,
    pub replay_added: bool,
    pub replay_len: usize,
}

impl Default for SleepPassConfig {
    fn default() -> Self {
        Self {
            batch_size: DEFAULT_SLEEP_PASS_BATCH_SIZE,
            seed: 0xCAFE,
            lr: 1.0,
            fisher_weight: 0.0,
            regression: RegressionConfig::strict(),
        }
    }
}

impl SleepPassConfig {
    pub fn validate(self) -> Result<Self> {
        if !self.lr.is_finite()
            || self.lr < 0.0
            || !self.fisher_weight.is_finite()
            || self.fisher_weight < 0.0
        {
            return Err(invalid_config(
                "sleep pass lr and fisher_weight must be finite and >= 0",
            ));
        }
        Ok(Self {
            regression: self.regression.validate()?,
            ..self
        })
    }
}

pub fn record_mistake_for_replay<M, R>(
    log: &MistakeLog<M>,
    buffer: &mut ReplayBuffer<R>,
    cx_id: CxId,
    predicted: f64,
    observed: f64,
    anchor: AnchorKind,
    min_surprise: f64,
) -> Result<SleepPassReplayRecord>
where
    M: MistakeStorage,
    R: ReplayStorage,
{
    if !min_surprise.is_finite() || min_surprise < 0.0 {
        return Err(invalid_config(
            "sleep pass min_surprise must be finite and >= 0",
        ));
    }
    let mistake_ref = log.append(cx_id, predicted, observed, anchor)?;
    let mut replay_added = false;
    if mistake_ref.surprise >= min_surprise {
        let entry = buffer.entry(cx_id, mistake_ref.surprise, mistake_ref)?;
        replay_added = buffer.push(entry)?;
    }
    Ok(SleepPassReplayRecord {
        mistake_ref,
        surprise: mistake_ref.surprise,
        replay_added,
        replay_len: buffer.len(),
    })
}

pub fn run_sleep_pass<S, G, F, R, M, C, H>(
    heads: &mut OnlineHeadState<S, G, F>,
    buffer: &ReplayBuffer<R>,
    log: &MistakeLog<M>,
    contexts: &C,
    registry: &DegradeRegistry<H>,
    config: SleepPassConfig,
) -> Result<SleepPassOutcome>
where
    S: HeadStorage,
    G: HeadPromotionGate + HeadRegressionRollback,
    F: FrozenLensCheck,
    R: ReplayStorage,
    M: MistakeStorage,
    C: RegressionContextSource,
    H: HealthStorage,
{
    let config = config.validate()?;
    let buffer_len = buffer.len();
    let degraded_components = registry.degraded_components();
    if !degraded_components.is_empty() {
        let labels = degraded_labels(&degraded_components);
        heads
            .substrate
            .record_sleep_pass_deferred(buffer_len, &labels)?;
        return Ok(SleepPassOutcome::Deferred {
            degraded_components: labels,
            buffer_len,
        });
    }
    if buffer_len == 0 {
        return Ok(SleepPassOutcome::Idle {
            reason: "replay buffer empty".to_string(),
            buffer_len,
        });
    }
    if config.batch_size == 0 {
        return Ok(SleepPassOutcome::Idle {
            reason: "sleep pass batch_size is zero".to_string(),
            buffer_len,
        });
    }
    let batch = buffer.sample_batch(config.batch_size, config.seed);
    if batch.is_empty() {
        return Ok(SleepPassOutcome::Idle {
            reason: "sampled replay batch empty".to_string(),
            buffer_len,
        });
    }
    match heads.update_with_regression(
        &batch,
        log,
        contexts,
        config.lr,
        config.fisher_weight,
        config.regression,
    ) {
        Ok(update) if update.update.promoted => Ok(SleepPassOutcome::Promoted { update }),
        Ok(_) => Ok(SleepPassOutcome::Idle {
            reason: "head update made no change".to_string(),
            buffer_len,
        }),
        Err(error)
            if error.code == CALYX_ANNEAL_HEAD_UPDATE_REVERTED
                || error.code == CALYX_ANNEAL_REGRESSION_RECURRED =>
        {
            Ok(SleepPassOutcome::Reverted {
                error_code: error.code.to_string(),
                message: error.message,
                batch_len: batch.len(),
                buffer_len,
            })
        }
        Err(error) => Err(error),
    }
}

fn degraded_labels(rows: &[(ComponentKind, ComponentHealth)]) -> Vec<String> {
    rows.iter()
        .map(|(kind, health)| format!("{kind:?}: {health}"))
        .collect()
}

fn invalid_config(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_SLEEP_PASS_INVALID_CONFIG,
        message: message.into(),
        remediation: "use finite non-negative sleep pass tuning values",
    }
}
