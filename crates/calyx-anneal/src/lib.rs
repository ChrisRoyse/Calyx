//! Anneal self-optimization contracts for reversible tuning loops.

mod budget;
mod heal;
mod integration_fsv;
mod learn;
mod ledger_anneal;
mod recurrence_schedule;
mod rollback;
mod rollback_codec;
mod shadow;
mod tripwire;
mod tune;

pub use budget::{
    BACKGROUND_NICE, BudgetConfig, BudgetConfigReadback, BudgetEnforcer, BudgetHandle, BudgetProbe,
    BudgetProbeSample, BudgetStatus, CALYX_ANNEAL_BUDGET_EXHAUSTED,
    CALYX_ANNEAL_BUDGET_INVALID_CONFIG, CALYX_ANNEAL_BUDGET_NVML_UNAVAILABLE, ProcStatBudgetProbe,
    budget_config_path, read_budget_config_from_vault,
};
pub use heal::degrade::{
    ANNEAL_HEALTH_TAG, AsterHealthStore, CALYX_ANNEAL_HEAL_CONFIRMATION_REQUIRED,
    CALYX_ANNEAL_HEALTH_INVALID_ROW, ComponentHealth, ComponentKind, DegradeRegistry,
    HealthRowReadback, HealthStorage, LensRoute, ScopeId, decode_health_value,
};
pub use heal::rebuild::{
    AnnIndexRebuilder, AsterRebuildSource, CALYX_ANNEAL_REBUILD_INVALID_TARGET,
    CALYX_ANNEAL_REBUILD_IO, CALYX_ANNEAL_REBUILD_SOURCE_VIOLATION,
    CALYX_ANNEAL_REBUILD_TRIPWIRE_FAILED, CALYX_ASTER_SNAPSHOT_UNAVAILABLE, GuardProfileRebuilder,
    KernelIndexRebuilder, MvccSnapshot, RebuildOutcome, RebuildPriority, RebuildScheduler,
    RebuildTarget, Rebuilder,
};
pub use heal::recalibrate::{
    CALYX_ANNEAL_PARK_THRESHOLD_NOT_MET, CALYX_ANNEAL_TAU_INVALID,
    CALYX_ANNEAL_UNPARK_THRESHOLD_NOT_MET, CALYX_WARD_RECALIBRATE_FAILED, FileWardTauStore,
    LensParkOutcome, NewTau, RecalibrationOutcome, SIGNAL_DECAY_FLOOR_BITS, TauDriftEvent,
    WARD_TAU_TAG, WardRecalibrate, WardTauReadback, WardTauStore, park_decayed_lens,
    trigger_tau_recalibration, unpark_lens, ward_tau_path,
};
pub use heal::restore::{
    BASE_SHARD_CHECKSUM_TAG, BaseFaultEvent, BaseShard, CALYX_ANNEAL_ALERT_WRITE_FAILED,
    CALYX_ANNEAL_CHECKSUM_INVALID_ROW, CALYX_ANNEAL_RESTORE_FAILED, RestoreCommand, RestoreConfig,
    RestoreOutcome, ShardId, alert_operator, attempt_restore, base_shard_checksum,
    clear_reads_on_range, fail_reads_on_range, install_recorded_read_barriers, load_base_shards,
    record_base_shard_checksum, verify_base_shards, write_base_restored_event,
};
pub use heal::triggers::{
    AssayMetrics, CALYX_ANNEAL_FAULT_INVALID_EVENT, ChecksumDetector, ChecksumEntry, EndpointUrl,
    FaultDetector, FaultEvent, FaultKind, FaultMonitor, HttpProbe, LensProbeDetector, ProbeStatus,
    SignalDecayDetector, SignalSample, StaleDetector, StaleEntry, TauDriftDetector, TauDriftSample,
    WardMetrics,
};
pub use integration_fsv::{
    AnnealLedgerActionPair, AnnealStatus, AnnealSubstrate, CALYX_LEDGER_WRITE_FAIL, ChangeOutcome,
};
pub use learn::{
    AsterHeadStorage, AsterMistakeStorage, AsterOutcomeStorage, AsterReplayStorage,
    CALYX_ANNEAL_HEAD_INVALID_ROW, CALYX_ANNEAL_HEAD_TOO_LARGE, CALYX_ANNEAL_HEAD_UPDATE_REVERTED,
    CALYX_ANNEAL_INVALID_CAPACITY, CALYX_ANNEAL_INVALID_WINDOW, CALYX_ANNEAL_MISTAKE_APPEND_ONLY,
    CALYX_ANNEAL_MISTAKE_INVALID_ROW, CALYX_ANNEAL_OUTCOME_APPEND_ONLY,
    CALYX_ANNEAL_OUTCOME_INVALID_ANCHOR, CALYX_ANNEAL_OUTCOME_INVALID_CONFIG,
    CALYX_ANNEAL_OUTCOME_INVALID_ROW, CALYX_ANNEAL_REGRESSION_INVALID_CONFIG,
    CALYX_ANNEAL_REGRESSION_NAN_PREDICTION, CALYX_ANNEAL_REGRESSION_RECURRED,
    CALYX_ANNEAL_REGRESSION_SOURCE_UNAVAILABLE, CALYX_ANNEAL_REPLAY_INVALID_ROW,
    CALYX_ANNEAL_SLEEP_PASS_INVALID_CONFIG, CALYX_REGISTRY_UNAVAILABLE,
    DEFAULT_MAX_REGRESSION_RATE, DEFAULT_MISTAKE_SURPRISE_THRESHOLD, DEFAULT_OUTCOME_ACTION_COST,
    DEFAULT_OUTCOME_FISHER_WEIGHT, DEFAULT_OUTCOME_LR, DEFAULT_REPLAY_CAPACITY,
    DEFAULT_SLEEP_PASS_BATCH_SIZE, DEFAULT_SLEEP_PASS_MIN_SURPRISE, FrozenCheckReport,
    FrozenLensCheck, FrozenLensGuard, FrozenLensReportRow, FrozenLensSource, FrozenLensStatus,
    HeadKind, HeadPromotionGate, HeadReadback, HeadRegressionRollback, HeadShadowProposal,
    HeadStorage, HeadUpdateOutcome, HeadUpdateSummary, MAX_ONLINE_HEAD_PARAMS, MistakeEntry,
    MistakeLog, MistakeReadback, MistakeRef, MistakeStorage, NoFrozenLensGuard, OnlineHead,
    OnlineHeadState, OutcomePrediction, OutcomeQueue, OutcomeQueueEntry, OutcomeQueueReadback,
    OutcomeStorage, RecordOutcomeConfig, RecordOutcomeContext, RecordOutcomeContradiction,
    RecordOutcomeResult, RecordOutcomeReward, RegressionConfig, RegressionContextSource,
    RegressionPredictor, RegressionReport, RegressionResult, RegressionUpdateOutcome, ReplayBuffer,
    ReplayEntry, ReplaySnapshot, ReplayStorage, SleepPassConfig, SleepPassOutcome,
    SleepPassReplayRecord, assert_no_regression, decode_head_rows, decode_mistake_entry,
    decode_online_head, decode_outcome_queue_entry, decode_replay_snapshot, encode_mistake_entry,
    encode_online_head, encode_outcome_queue_entry, encode_replay_snapshot, head_key,
    head_state_artifact_key, mistake_key, mistake_seq_from_key, outcome_queue_key,
    outcome_queue_seq_from_key, record_mistake_for_replay, record_outcome, record_regression,
    regression_rate, regression_recurred, replay_snapshot_key, run_sleep_pass,
};
pub use ledger_anneal::{
    ANNEAL_LEDGER_PAYLOAD_TAG, AnnealFaultLedgerDetails, AnnealLedger, AnnealLedgerAction,
    AnnealLedgerEntry, AnnealLedgerReadback, AsterAnnealLedgerStore,
    CALYX_ANNEAL_LEDGER_INVALID_ENTRY, CALYX_ASTER_CF_UNAVAILABLE, CALYX_LEDGER_ENTRY_TOO_LARGE,
    MAX_ANNEAL_LEDGER_PAYLOAD_BYTES, decode_anneal_ledger_payload,
};
pub use recurrence_schedule::{
    CALYX_ANNEAL_INVALID_CADENCE, FREQ_BONUS_MAX, RecurrenceSchedule, RefreshPriority,
    RetentionTier, anneal_retention_tier, frequency_kernel_bonus, recurrence_schedule_for,
};
pub use rollback::{
    ArtifactKey, ArtifactPtr, ArtifactSnapshot, AsterRollbackStorage,
    CALYX_ANNEAL_CHANGE_COMMITTED, CALYX_ANNEAL_INVALID_ROLLBACK_STATE,
    CALYX_ANNEAL_UNKNOWN_CHANGE_ID, ChangeId, LogicalTime, RollbackReadback, RollbackStorage,
    RollbackStore, rollback_live_key, rollback_snapshot_key,
};
pub use shadow::{
    ActionMetricSnapshot, AnnealAction, HeldOutReplay, MetricComparison, MetricSide,
    MetricSnapshot, ReplayAnchor, ReplayQuery, ReplaySource, ShadowExecutor, ShadowRevertReason,
    ShadowVerdict, build_replay,
};
pub use tripwire::{
    CALYX_TRIPWIRE_INVALID_CONFIG, CALYX_TRIPWIRE_INVALID_METRIC, ThresholdDir, ThresholdState,
    TripwireConfigReadback, TripwireMetric, TripwireRegistry, TripwireResult, TripwireStatus,
    TripwireThreshold, TripwireThresholdEntry, read_tripwire_config_from_vault,
    tripwire_config_path,
};
pub use tune::{
    Arm, ArmStatus, AsterBanditStorage, BanditPolicy, BanditReadback, BanditStatus, BanditStorage,
    CALYX_ANNEAL_BANDIT_EMPTY, CALYX_ANNEAL_BANDIT_INVALID_CONFIG, CALYX_ANNEAL_BANDIT_INVALID_ROW,
    CALYX_FORGE_CACHE_WRITE_FAIL, CALYX_FORGE_SCOPE_INVALID_CONFIG, ConfigBandit,
    ConfigBanditStore, ConfigVariant, DEFAULT_FORGE_RECALL_TARGET, DEFAULT_HYSTERESIS_WINS, DType,
    ForgeBanditPersistence, ForgeConfig, ForgePromotionRecord, ForgePromotionWriter,
    ForgeScopeTuner, ForgeTuneDecision, MAX_BUCKETED_DIM, MAX_FORGE_CANDIDATES,
    NoopForgeBanditStore, NoopForgePromotionWriter, ShapeKey, bandit_key, bucket_dim, bucket_shape,
    candidate_configs, decode_config_bandit, decode_forge_config, encode_config_bandit,
    encode_forge_config, shape_key_hash,
};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-anneal");
    }
}
