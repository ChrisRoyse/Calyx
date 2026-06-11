//! Anneal self-optimization contracts for reversible tuning loops.

mod budget;
mod heal;
mod integration_fsv;
mod ledger_anneal;
mod recurrence_schedule;
mod rollback;
mod rollback_codec;
mod shadow;
mod tripwire;

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
pub use integration_fsv::{AnnealStatus, AnnealSubstrate, CALYX_LEDGER_WRITE_FAIL, ChangeOutcome};
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

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-anneal");
    }
}
