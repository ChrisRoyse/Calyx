mod bandit;
mod scope_forge;
mod scope_index;

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
    NoopIndexPromotionWriter, NoopIndexSlotHealth, candidate_configs as index_candidate_configs,
    decode_index_config, encode_index_config, index_slot_label, quant_win_check, slot_autotune_key,
    validate_index_config,
};
