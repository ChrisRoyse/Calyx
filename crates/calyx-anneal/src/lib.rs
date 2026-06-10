//! Anneal self-optimization contracts for reversible tuning loops.

mod recurrence_schedule;
mod rollback;
mod rollback_codec;
mod shadow;
mod tripwire;

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
    ActionMetricSnapshot, AnnealAction, BudgetHandle, HeldOutReplay, MetricComparison, MetricSide,
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
