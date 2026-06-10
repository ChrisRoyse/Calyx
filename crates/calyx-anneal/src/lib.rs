//! Anneal self-optimization contracts for reversible tuning loops.

mod recurrence_schedule;
mod tripwire;

pub use recurrence_schedule::{
    CALYX_ANNEAL_INVALID_CADENCE, FREQ_BONUS_MAX, RecurrenceSchedule, RefreshPriority,
    RetentionTier, anneal_retention_tier, frequency_kernel_bonus, recurrence_schedule_for,
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
