mod types;
mod writer;

use std::sync::{Arc, Mutex};

use calyx_core::{CalyxError, Result};
use calyx_forge::AutotuneCache;

use crate::{AssayMetrics, BanditPolicy, ConfigBandit, SignalSample, shape_key_hash};

pub use types::{
    ConcatKey, DEFAULT_LOOM_RECALL_TARGET, MAX_LOOM_CANDIDATES, MAX_LOOM_EAGER_PAIRS,
    MIN_LOOM_PAIR_BITS, MatPlanConfig, PlanScore, QueryLog, QueryObservation,
    decode_mat_plan_config, encode_mat_plan_config, evaluate_plan, generate_candidate_plan,
    loom_plan_label, loom_plan_shape_key, loom_plan_tune_key, plan_hash, validate_mat_plan_config,
};
pub use writer::{
    LoomBanditPersistence, LoomPromotionWriter, NoopLoomBanditStore, NoopLoomPromotionWriter,
};

pub const CALYX_LOOM_PLAN_WRITE_FAIL: &str = "CALYX_LOOM_PLAN_WRITE_FAIL";
pub const CALYX_LOOM_SCOPE_INVALID_CONFIG: &str = "CALYX_LOOM_SCOPE_INVALID_CONFIG";

const NEXT_CHANGE_ID_START: u64 = 415_000;
const SCORE_EPSILON: f64 = 1e-12;

#[derive(Default)]
pub struct NoopLoomAssayMetrics;

impl AssayMetrics for NoopLoomAssayMetrics {
    fn signal_samples(&self) -> Result<Vec<SignalSample>> {
        Ok(Vec::new())
    }
}

pub trait LoomMaterializer: Send + Sync {
    fn apply_plan(&self, old_plan: &MatPlanConfig, new_plan: &MatPlanConfig) -> Result<()>;
}

#[derive(Clone, Copy, Default)]
pub struct NoopLoomMaterializer;

impl LoomMaterializer for NoopLoomMaterializer {
    fn apply_plan(&self, _old_plan: &MatPlanConfig, _new_plan: &MatPlanConfig) -> Result<()> {
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LoomPromotionRecord {
    pub change_id: crate::ChangeId,
    pub old_plan: MatPlanConfig,
    pub new_plan: MatPlanConfig,
    pub latency_before_ns: u64,
    pub latency_after_ns: u64,
    pub bits_before: f64,
    pub bits_after: f64,
    pub plan_key_hash: [u8; 32],
    pub old_plan_hash: [u8; 32],
    pub new_plan_hash: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LoomTuneDecision {
    pub evaluated_arm: usize,
    pub won: bool,
    pub incumbent: MatPlanConfig,
    pub incumbent_score: PlanScore,
    pub promoted: Option<LoomPromotionRecord>,
    pub shadow_arm: Option<usize>,
    pub shadow_candidate: Option<MatPlanConfig>,
}

pub struct LoomScopeTuner<
    W = NoopLoomPromotionWriter,
    B = NoopLoomBanditStore,
    L = NoopLoomMaterializer,
> where
    W: LoomPromotionWriter,
    B: LoomBanditPersistence,
    L: LoomMaterializer,
{
    pub bandit: ConfigBandit,
    pub current_plan: MatPlanConfig,
    pub assay: Arc<dyn AssayMetrics>,
    pub loom: Arc<L>,
    pub cache: Arc<Mutex<AutotuneCache>>,
    promotion_writer: W,
    bandit_store: B,
    pending_arm: Option<usize>,
    next_change_id: u64,
    promotions: Vec<LoomPromotionRecord>,
}

impl LoomScopeTuner {
    pub fn new(cache: AutotuneCache, current_plan: MatPlanConfig) -> Self {
        Self::with_parts(
            cache,
            current_plan,
            NoopLoomPromotionWriter,
            NoopLoomBanditStore,
            NoopLoomMaterializer,
        )
    }
}

impl<W, B, L> LoomScopeTuner<W, B, L>
where
    W: LoomPromotionWriter,
    B: LoomBanditPersistence,
    L: LoomMaterializer,
{
    pub fn with_parts(
        cache: AutotuneCache,
        current_plan: MatPlanConfig,
        promotion_writer: W,
        bandit_store: B,
        loom: L,
    ) -> Self {
        Self::with_assay_parts(
            cache,
            current_plan,
            Arc::new(NoopLoomAssayMetrics),
            promotion_writer,
            bandit_store,
            loom,
        )
    }

    pub fn with_assay_parts(
        cache: AutotuneCache,
        current_plan: MatPlanConfig,
        assay: Arc<dyn AssayMetrics>,
        promotion_writer: W,
        bandit_store: B,
        loom: L,
    ) -> Self {
        Self {
            bandit: ConfigBandit::new(BanditPolicy::Thompson, types::seed_for_loom()),
            current_plan,
            assay,
            loom: Arc::new(loom),
            cache: Arc::new(Mutex::new(cache)),
            promotion_writer,
            bandit_store,
            pending_arm: None,
            next_change_id: NEXT_CHANGE_ID_START,
            promotions: Vec::new(),
        }
    }

    pub fn install_candidates(&mut self, configs: Vec<MatPlanConfig>) -> Result<()> {
        if configs.is_empty() || configs.len() > MAX_LOOM_CANDIDATES {
            return Err(invalid_config(
                "Loom candidate set must contain 1..=8 plans",
            ));
        }
        let mut bandit = ConfigBandit::new(BanditPolicy::Thompson, types::seed_for_loom());
        for config in configs {
            bandit.add_arm(encode_mat_plan_config(&config)?);
        }
        self.current_plan = decode_mat_plan_config(&bandit.incumbent()?.config)?;
        self.bandit_store
            .save_bandit(shape_key_hash(loom_plan_shape_key()), &bandit)?;
        self.bandit = bandit;
        Ok(())
    }

    pub fn on_query_tick(&mut self, query_log: &QueryLog) -> Result<LoomTuneDecision> {
        self.ensure_bandit(query_log)?;
        let arm = self.pending_arm.take().unwrap_or(self.bandit.incumbent_idx);
        self.on_query_tick_for_arm(query_log, arm)
    }

    pub fn on_query_tick_for_arm(
        &mut self,
        query_log: &QueryLog,
        arm_idx: usize,
    ) -> Result<LoomTuneDecision> {
        self.ensure_bandit(query_log)?;
        let prior_idx = self.bandit.incumbent_idx;
        let prior_plan = self.config_for_arm(prior_idx)?;
        let candidate_plan = self.config_for_arm(arm_idx)?;
        let incumbent_score = evaluate_plan(&prior_plan, query_log, self.assay.as_ref());
        let candidate_score = evaluate_plan(&candidate_plan, query_log, self.assay.as_ref());
        let won = arm_won(arm_idx, prior_idx, candidate_score, incumbent_score);
        self.bandit.record_result(arm_idx, won)?;

        let new_idx = self.bandit.incumbent_idx;
        let promoted = if new_idx != prior_idx {
            let new_plan = self.config_for_arm(new_idx)?;
            let record = self.promote(&prior_plan, &new_plan, incumbent_score, candidate_score)?;
            self.current_plan = new_plan;
            Some(record)
        } else {
            self.current_plan = prior_plan;
            None
        };

        self.save_bandit()?;
        self.ensure_candidate_arm(query_log)?;
        let shadow_arm = self.next_shadow_arm();
        self.pending_arm = Some(shadow_arm);
        let shadow_candidate = (shadow_arm != self.bandit.incumbent_idx)
            .then(|| self.config_for_arm(shadow_arm))
            .transpose()?;
        let incumbent = self.config_for_arm(self.bandit.incumbent_idx)?;
        let score = evaluate_plan(&incumbent, query_log, self.assay.as_ref());

        Ok(LoomTuneDecision {
            evaluated_arm: arm_idx,
            won,
            incumbent,
            incumbent_score: score,
            promoted,
            shadow_arm: Some(shadow_arm),
            shadow_candidate,
        })
    }

    pub fn promotions(&self) -> &[LoomPromotionRecord] {
        &self.promotions
    }

    fn ensure_bandit(&mut self, query_log: &QueryLog) -> Result<()> {
        if !self.bandit.arms.is_empty() {
            return self.ensure_candidate_arm(query_log);
        }
        if let Some(saved) = self
            .bandit_store
            .load_bandit(shape_key_hash(loom_plan_shape_key()))?
        {
            self.bandit = saved;
        }
        if self.bandit.arms.is_empty() {
            self.bandit
                .add_arm(encode_mat_plan_config(&self.current_plan)?);
        }
        self.ensure_candidate_arm(query_log)?;
        self.save_bandit()
    }

    fn ensure_candidate_arm(&mut self, query_log: &QueryLog) -> Result<()> {
        let candidate = generate_candidate_plan(&self.current_plan, self.assay.as_ref(), query_log);
        if candidate == self.current_plan || self.has_plan_arm(&candidate)? {
            return Ok(());
        }
        if self.bandit.arms.len() >= MAX_LOOM_CANDIDATES {
            return Ok(());
        }
        self.bandit.add_arm(encode_mat_plan_config(&candidate)?);
        self.pending_arm.get_or_insert(self.bandit.arms.len() - 1);
        Ok(())
    }

    fn has_plan_arm(&self, plan: &MatPlanConfig) -> Result<bool> {
        for arm in &self.bandit.arms {
            if decode_mat_plan_config(&arm.config)? == *plan {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn next_shadow_arm(&self) -> usize {
        self.bandit
            .arms
            .iter()
            .enumerate()
            .find_map(|(idx, _)| (idx != self.bandit.incumbent_idx).then_some(idx))
            .unwrap_or(self.bandit.incumbent_idx)
    }

    fn config_for_arm(&self, arm_idx: usize) -> Result<MatPlanConfig> {
        let arm = self
            .bandit
            .arms
            .get(arm_idx)
            .ok_or_else(|| invalid_config(format!("Loom arm {arm_idx} out of range")))?;
        decode_mat_plan_config(&arm.config)
    }

    fn promote(
        &mut self,
        old_plan: &MatPlanConfig,
        new_plan: &MatPlanConfig,
        old_score: PlanScore,
        new_score: PlanScore,
    ) -> Result<LoomPromotionRecord> {
        let change_id = crate::ChangeId(self.next_change_id);
        self.next_change_id = self.next_change_id.saturating_add(1);
        let record = LoomPromotionRecord {
            change_id,
            old_plan: old_plan.clone(),
            new_plan: new_plan.clone(),
            latency_before_ns: old_score.avg_latency_ns,
            latency_after_ns: new_score.avg_latency_ns,
            bits_before: old_score.bits_sum,
            bits_after: new_score.bits_sum,
            plan_key_hash: shape_key_hash(loom_plan_shape_key()),
            old_plan_hash: plan_hash(old_plan)?,
            new_plan_hash: plan_hash(new_plan)?,
        };
        self.write_cache(&record)?;
        self.loom.apply_plan(old_plan, new_plan)?;
        self.promotion_writer.write_autotune_promote(&record)?;
        self.promotions.push(record.clone());
        Ok(record)
    }

    fn write_cache(&self, record: &LoomPromotionRecord) -> Result<()> {
        let score = PlanScore {
            avg_latency_ns: record.latency_after_ns,
            bits_sum: record.bits_after,
            query_count: 0,
            eager_pair_count: record.new_plan.eager_pairs.len(),
        };
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| invalid_config("autotune cache lock poisoned"))?;
        cache.insert(loom_plan_tune_key(), record.new_plan.to_best_config(score)?);
        cache.persist().map_err(cache_write_fail)
    }

    fn save_bandit(&self) -> Result<()> {
        self.bandit_store
            .save_bandit(shape_key_hash(loom_plan_shape_key()), &self.bandit)
    }
}

pub(super) fn invalid_config(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_LOOM_SCOPE_INVALID_CONFIG,
        message: message.into(),
        remediation: "repair Loom materialization plan/query-log metadata before promotion",
    }
}

fn cache_write_fail(error: calyx_forge::ForgeError) -> CalyxError {
    CalyxError {
        code: CALYX_LOOM_PLAN_WRITE_FAIL,
        message: format!("Loom materialization plan cache write failed: {error}"),
        remediation: "repair the PH16 autotune cache path before persisting a Loom plan promotion",
    }
}

fn arm_won(
    arm_idx: usize,
    incumbent_idx: usize,
    candidate: PlanScore,
    incumbent: PlanScore,
) -> bool {
    arm_idx == incumbent_idx
        || (candidate.avg_latency_ns < incumbent.avg_latency_ns
            && candidate.bits_sum + SCORE_EPSILON >= incumbent.bits_sum)
}
