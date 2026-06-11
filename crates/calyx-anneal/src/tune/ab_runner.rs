mod errors;
mod types;
mod writer;

pub use types::{
    ABLedgerEvent, ABPromotionConfig, ABResult, ABSummary, ABTrial, ABVerdict, ABVerdictRecord,
    CALYX_ANNEAL_AB_CACHE_WRITE_FAIL, CALYX_ANNEAL_TRIAL_ALREADY_ACTIVE,
    CALYX_ANNEAL_TRIAL_INVALID_RESULT, CALYX_ANNEAL_TRIAL_NOT_ACTIVE, DEFAULT_AB_MIN_SAMPLES,
};
pub use writer::{ABLedgerWriter, ABTrialBudget, NoopABBudget, NoopABLedgerWriter};

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use calyx_core::{Clock, Result};
use calyx_forge::AutotuneCache;

use crate::{
    AnnealLedgerAction, CALYX_ANNEAL_BUDGET_EXHAUSTED, ChangeId, ConfigBandit, ShapeKey,
    TripwireMetric, TripwireRegistry, TripwireResult,
};

use errors::{already_active, cache_write_fail, invalid_result, not_active};

const CHANGE_ID_START: u64 = 416_000;

pub struct ABRunner<W = NoopABLedgerWriter, B = NoopABBudget>
where
    W: ABLedgerWriter,
    B: ABTrialBudget,
{
    pub active_trials: HashMap<ShapeKey, ABTrial>,
    pub tripwires: TripwireRegistry,
    pub writer: W,
    pub budget: B,
    pub clock: Arc<dyn Clock>,
    cache: Option<Arc<Mutex<AutotuneCache>>>,
    next_change_id: u64,
}

impl<W, B> ABRunner<W, B>
where
    W: ABLedgerWriter,
    B: ABTrialBudget,
{
    pub fn new(tripwires: TripwireRegistry, writer: W, budget: B, clock: Arc<dyn Clock>) -> Self {
        Self {
            active_trials: HashMap::new(),
            tripwires,
            writer,
            budget,
            clock,
            cache: None,
            next_change_id: CHANGE_ID_START,
        }
    }

    pub fn with_cache(mut self, cache: AutotuneCache) -> Self {
        self.cache = Some(Arc::new(Mutex::new(cache)));
        self
    }

    pub fn start_trial(
        &mut self,
        key: ShapeKey,
        candidate_arm: usize,
        incumbent_arm: usize,
    ) -> Result<()> {
        self.start_trial_with_config(
            key,
            candidate_arm,
            incumbent_arm,
            DEFAULT_AB_MIN_SAMPLES,
            None,
        )
    }

    pub fn start_trial_with_config(
        &mut self,
        key: ShapeKey,
        candidate_arm: usize,
        incumbent_arm: usize,
        min_samples: usize,
        promotion_config: Option<ABPromotionConfig>,
    ) -> Result<()> {
        if self
            .active_trials
            .get(&key)
            .is_some_and(|trial| trial.verdict.is_none())
        {
            return Err(already_active(key.label()));
        }
        let mut trial =
            ABTrial::with_min_samples(key.clone(), candidate_arm, incumbent_arm, min_samples);
        trial.promotion_config = promotion_config;
        self.active_trials.insert(key, trial);
        Ok(())
    }

    pub fn record_query(
        &mut self,
        key: &ShapeKey,
        incumbent_result: ABResult,
        candidate_result: ABResult,
        bandit: &mut ConfigBandit,
    ) -> Result<Option<ABVerdict>> {
        if let Some(verdict) = self
            .active_trials
            .get(key)
            .and_then(|trial| trial.verdict.clone())
        {
            return Ok(Some(verdict));
        }
        let mut handle = match self.budget.acquire_shadow() {
            Ok(handle) => handle,
            Err(error) if error.code == CALYX_ANNEAL_BUDGET_EXHAUSTED => {
                return self.abandon_trial(key, "shadow budget exhausted");
            }
            Err(error) => return Err(error),
        };
        if handle.try_consume() {
            self.record_query_under_budget(key, incumbent_result, candidate_result, bandit)
        } else {
            self.abandon_trial(key, "shadow budget exhausted")
        }
    }

    pub fn abandon_trial(
        &mut self,
        key: &ShapeKey,
        reason: impl Into<String>,
    ) -> Result<Option<ABVerdict>> {
        if let Some(verdict) = self
            .active_trials
            .get(key)
            .and_then(|trial| trial.verdict.clone())
        {
            return Ok(Some(verdict));
        }
        let Some(trial) = self.active_trials.get(key).cloned() else {
            return Err(not_active(key.label()));
        };
        let record = self.record_for(&trial, empty_summary(), empty_summary(), reason.into());
        let event = self.event_for(AnnealLedgerAction::AutotuneAbandoned, &record, None);
        self.writer.write_ab_event(&event)?;
        let verdict = ABVerdict::Abandoned(record);
        self.active_trials
            .get_mut(key)
            .expect("trial exists")
            .verdict = Some(verdict.clone());
        Ok(Some(verdict))
    }

    pub fn declare_winner(
        &mut self,
        key: &ShapeKey,
        bandit: &mut ConfigBandit,
    ) -> Result<ABVerdict> {
        if let Some(verdict) = self
            .active_trials
            .get(key)
            .and_then(|trial| trial.verdict.clone())
        {
            return Ok(verdict);
        }
        let Some(trial) = self.active_trials.get(key).cloned() else {
            return Err(not_active(key.label()));
        };
        let incumbent = summarize_arm(&trial, trial.incumbent_idx)?;
        let candidate = summarize_arm(&trial, trial.candidate_idx)?;
        let recall_ok = matches!(
            self.tripwires
                .check(TripwireMetric::RecallAtK, candidate.mean_recall_k)?,
            TripwireResult::Ok
        );
        let latency_ok = matches!(
            self.tripwires
                .check(TripwireMetric::SearchP99, candidate.p99_latency_ns as f64)?,
            TripwireResult::Ok
        );
        let recall_regression_ok =
            candidate.mean_recall_k + f64::EPSILON >= incumbent.mean_recall_k;
        let bits_ok =
            candidate.mean_bits_per_anchor + f64::EPSILON >= incumbent.mean_bits_per_anchor;
        let faster = candidate.p99_latency_ns < incumbent.p99_latency_ns;
        let candidate_won = faster && recall_ok && recall_regression_ok && latency_ok && bits_ok;

        let mut candidate_bandit = bandit.clone();
        candidate_bandit.record_result(trial.candidate_idx, candidate_won)?;
        let promoted = candidate_won
            && candidate_bandit.incumbent_idx == trial.candidate_idx
            && bandit.incumbent_idx != trial.candidate_idx;
        let record = self.record_for(
            &trial,
            incumbent,
            candidate,
            verdict_reason(
                candidate_won,
                recall_ok,
                recall_regression_ok,
                latency_ok,
                bits_ok,
                promoted,
            ),
        );
        let action = if promoted {
            AnnealLedgerAction::AutotunePromote
        } else {
            AnnealLedgerAction::AutotuneAB
        };
        let event = self.event_for(action, &record, Some(bandit));
        self.writer.write_ab_event(&event)?;
        if promoted {
            self.persist_cache(&trial)?;
        }
        *bandit = candidate_bandit;
        let verdict = if promoted {
            ABVerdict::Promoted(record)
        } else {
            ABVerdict::Kept(record)
        };
        self.active_trials
            .get_mut(key)
            .expect("trial exists")
            .verdict = Some(verdict.clone());
        Ok(verdict)
    }

    fn record_query_under_budget(
        &mut self,
        key: &ShapeKey,
        incumbent_result: ABResult,
        candidate_result: ABResult,
        bandit: &mut ConfigBandit,
    ) -> Result<Option<ABVerdict>> {
        let Some(trial) = self.active_trials.get_mut(key) else {
            return Err(not_active(key.label()));
        };
        validate_result(incumbent_result)?;
        validate_result(candidate_result)?;
        if incumbent_result.arm_idx != trial.incumbent_idx {
            return Err(invalid_result("incumbent result arm does not match trial"));
        }
        if candidate_result.arm_idx != trial.candidate_idx {
            return Err(invalid_result("candidate result arm does not match trial"));
        }
        trial.results.push(incumbent_result);
        trial.results.push(candidate_result);
        if trial.query_pairs() < trial.min_samples {
            return Ok(None);
        }
        self.declare_winner(key, bandit).map(Some)
    }

    fn persist_cache(&self, trial: &ABTrial) -> Result<()> {
        let Some(promotion) = &trial.promotion_config else {
            return Ok(());
        };
        let Some(cache) = &self.cache else {
            return Ok(());
        };
        let mut cache = cache
            .lock()
            .map_err(|_| invalid_result("autotune cache lock poisoned"))?;
        cache.insert(promotion.key.clone(), promotion.config.clone());
        cache.persist().map_err(cache_write_fail)
    }

    fn record_for(
        &mut self,
        trial: &ABTrial,
        incumbent: ABSummary,
        candidate: ABSummary,
        reason: String,
    ) -> types::ABVerdictRecord {
        let change_id = ChangeId(self.next_change_id);
        self.next_change_id = self.next_change_id.saturating_add(1);
        types::ABVerdictRecord {
            key: trial.key.clone(),
            incumbent_idx: trial.incumbent_idx,
            candidate_idx: trial.candidate_idx,
            samples: incumbent.sample_count.max(candidate.sample_count),
            latency_before_ns: incumbent.p99_latency_ns,
            latency_after_ns: candidate.p99_latency_ns,
            recall_before: incumbent.mean_recall_k,
            recall_after: candidate.mean_recall_k,
            bits_before: incumbent.mean_bits_per_anchor,
            bits_after: candidate.mean_bits_per_anchor,
            reason,
            change_id,
            ts: self.clock.now(),
        }
    }

    fn event_for(
        &self,
        action: AnnealLedgerAction,
        record: &types::ABVerdictRecord,
        bandit: Option<&ConfigBandit>,
    ) -> ABLedgerEvent {
        ABLedgerEvent {
            action,
            record: record.clone(),
            prior_ptr_hash: arm_hash(bandit, record.incumbent_idx),
            candidate_ptr_hash: arm_hash(bandit, record.candidate_idx),
        }
    }
}

fn summarize_arm(trial: &ABTrial, arm_idx: usize) -> Result<ABSummary> {
    let mut latencies = Vec::new();
    let mut recall_sum = 0.0;
    let mut bits_sum = 0.0;
    for result in trial
        .results
        .iter()
        .filter(|result| result.arm_idx == arm_idx)
    {
        validate_result(*result)?;
        latencies.push(result.latency_ns);
        recall_sum += result.recall_k;
        bits_sum += result.bits_per_anchor;
    }
    if latencies.is_empty() {
        return Err(invalid_result("A/B trial has no samples for arm"));
    }
    latencies.sort_unstable();
    let p99_idx = (latencies.len() * 99).div_ceil(100).saturating_sub(1);
    let latency_sum: u128 = latencies.iter().map(|latency| u128::from(*latency)).sum();
    Ok(ABSummary {
        sample_count: latencies.len(),
        p99_latency_ns: latencies[p99_idx],
        mean_latency_ns: (latency_sum / latencies.len() as u128).min(u128::from(u64::MAX)) as u64,
        mean_recall_k: recall_sum / latencies.len() as f64,
        mean_bits_per_anchor: bits_sum / latencies.len() as f64,
    })
}

fn empty_summary() -> ABSummary {
    ABSummary {
        sample_count: 0,
        p99_latency_ns: 0,
        mean_latency_ns: 0,
        mean_recall_k: 0.0,
        mean_bits_per_anchor: 0.0,
    }
}

fn verdict_reason(
    candidate_won: bool,
    recall_ok: bool,
    recall_regression_ok: bool,
    latency_ok: bool,
    bits_ok: bool,
    promoted: bool,
) -> String {
    if promoted {
        return "promoted".to_string();
    }
    if !candidate_won && !recall_ok {
        return "recall_tripwire".to_string();
    }
    if !candidate_won && !recall_regression_ok {
        return "recall_regression".to_string();
    }
    if !candidate_won && !latency_ok {
        return "latency_tripwire".to_string();
    }
    if !candidate_won && !bits_ok {
        return "bits_regression".to_string();
    }
    "candidate_not_faster_or_hysteresis_pending".to_string()
}

fn arm_hash(bandit: Option<&ConfigBandit>, arm_idx: usize) -> [u8; 32] {
    bandit
        .and_then(|bandit| bandit.arms.get(arm_idx))
        .map(|arm| *blake3::hash(&arm.config).as_bytes())
        .unwrap_or([0; 32])
}

fn validate_result(result: ABResult) -> Result<()> {
    if !result.recall_k.is_finite() || !result.bits_per_anchor.is_finite() {
        return Err(invalid_result("A/B result metrics must be finite"));
    }
    if !(0.0..=1.0).contains(&result.recall_k) {
        return Err(invalid_result("A/B result recall_k must be in 0..=1"));
    }
    Ok(())
}
