mod ab_runner;
mod bandit;
mod scope_forge;
mod scope_index;
mod scope_loom;
mod scope_storage;
mod soak_harness;

pub use ab_runner::{
    ABLedgerEvent, ABLedgerWriter, ABPromotionConfig, ABResult, ABRunner, ABSummary, ABTrial,
    ABTrialBudget, ABVerdict, ABVerdictRecord, CALYX_ANNEAL_AB_CACHE_WRITE_FAIL,
    CALYX_ANNEAL_TRIAL_ALREADY_ACTIVE, CALYX_ANNEAL_TRIAL_INVALID_RESULT,
    CALYX_ANNEAL_TRIAL_NOT_ACTIVE, DEFAULT_AB_MIN_SAMPLES, NoopABBudget, NoopABLedgerWriter,
};
pub use bandit::{
    Arm, ArmStatus, AsterBanditStorage, BanditPolicy, BanditReadback, BanditStatus, BanditStorage,
    CALYX_ANNEAL_BANDIT_EMPTY, CALYX_ANNEAL_BANDIT_INVALID_CONFIG, CALYX_ANNEAL_BANDIT_INVALID_ROW,
    ConfigBandit, ConfigBanditStore, ConfigVariant, DEFAULT_HYSTERESIS_WINS, bandit_key,
    decode_config_bandit, encode_config_bandit, shape_key_hash,
};
pub use scope_forge::{
    CALYX_FORGE_CACHE_WRITE_FAIL, CALYX_FORGE_SCOPE_INVALID_CONFIG, DEFAULT_FORGE_RECALL_TARGET,
    DType, ForgeBanditPersistence, ForgeConfig, ForgePromotionRecord, ForgePromotionWriter,
    ForgeScopeTuner, ForgeTuneDecision, MAX_BUCKETED_DIM, MAX_FORGE_CANDIDATES,
    NoopForgeBanditStore, NoopForgePromotionWriter, ShapeKey, bucket_dim, bucket_shape,
    candidate_configs, decode_forge_config, encode_forge_config,
};
pub use scope_index::{
    CALYX_INDEX_CACHE_WRITE_FAIL, CALYX_INDEX_SCOPE_INVALID_CONFIG, DEFAULT_INDEX_RECALL_TARGET,
    DEFAULT_INDEX_VRAM_BUDGET_BYTES, IndexBanditPersistence, IndexConfig, IndexPromotionRecord,
    IndexPromotionWriter, IndexScopeTuner, IndexSlotHealth, IndexTuneDecision, IndexTuneSkip,
    MAX_INDEX_CANDIDATES, MIN_BITS_PER_ANCHOR, NoopIndexAssayMetrics, NoopIndexBanditStore,
    NoopIndexPromotionWriter, NoopIndexSlotHealth, QuantPromotionEvidence,
    candidate_configs as index_candidate_configs, decode_index_config, encode_index_config,
    index_slot_label, quant_win_check, slot_autotune_key, validate_index_config,
    validate_quant_promotion_evidence,
};
pub use scope_loom::{
    CALYX_LOOM_PLAN_WRITE_FAIL, CALYX_LOOM_SCOPE_INVALID_CONFIG, ConcatKey,
    DEFAULT_LOOM_RECALL_TARGET, LoomBanditPersistence, LoomMaterializer, LoomPromotionRecord,
    LoomPromotionWriter, LoomScopeTuner, LoomTuneDecision, MAX_LOOM_CANDIDATES,
    MAX_LOOM_EAGER_PAIRS, MIN_LOOM_PAIR_BITS, MatPlanConfig, NoopLoomBanditStore,
    NoopLoomMaterializer, NoopLoomPromotionWriter, PlanScore, QueryLog, QueryObservation,
    decode_mat_plan_config, encode_mat_plan_config, evaluate_plan, generate_candidate_plan,
    loom_plan_label, loom_plan_shape_key, loom_plan_tune_key, validate_mat_plan_config,
};
pub use scope_storage::{
    CALYX_STORAGE_CACHE_WRITE_FAIL, CALYX_STORAGE_SCOPE_INVALID_CONFIG,
    DEFAULT_STORAGE_RECALL_TARGET, MAX_STORAGE_CANDIDATES, NoopStorageBanditStore,
    NoopStoragePromotionWriter, StorageBanditPersistence, StorageConfig, StorageMetrics,
    StoragePromotionRecord, StoragePromotionWriter, StorageScopeTuner, StorageShapeKey,
    StorageTuneDecision, candidate_storage_configs, decode_storage_config, encode_storage_config,
    storage_autotune_key, storage_shape_label, storage_win_check, validate_storage_config,
    validate_storage_metrics,
};
pub use soak_harness::{
    AsterSoakStorage, CALYX_ANNEAL_SOAK_INVALID_CONFIG, CALYX_ANNEAL_SOAK_INVALID_ROW,
    CALYX_ANNEAL_SOAK_TIME_BUDGET_EXHAUSTED, DEFAULT_SOAK_OSCILLATION_WINDOW,
    DEFAULT_SOAK_P99_TARGET_REDUCTION, DEFAULT_SOAK_QUERIES, DEFAULT_SOAK_SAMPLE_INTERVAL,
    DEFAULT_SOAK_SEED, MetricSample, NoopSoakStorage, SeededSoakProfile, SoakConfig, SoakHarness,
    SoakMetrics, SoakMode, SoakReport, SoakRowKind, SoakStorage, SoakStoredRow, check_oscillation,
    decode_soak_reports, decode_soak_row, encode_soak_row, soak_report_key, soak_sample_key,
};
