use calyx_assay::TrustTag;
use calyx_core::{AnchorValue, Clock};
use calyx_lodestar::{LodestarError, RecallReport};
use calyx_oracle::{
    CalibrationMeasurement, CalibrationSource, DomainId, GoodhartDefenseMeasurement,
    GoodhartDefenseSource, HeldOutSplit, KERNEL_RECALL_RATIO, KernelRecallSource,
    MIN_VALIDITY_SAMPLES, MistakeClosureMeasurement, MistakeClosureSource, OracleError,
};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub(super) struct OracleFixture {
    #[serde(default = "default_occurrence_count")]
    pub(super) occurrence_count: usize,
    #[serde(default = "default_anchor_value")]
    pub(super) oracle_verdict: AnchorValue,
    #[serde(default = "default_anchor_value")]
    pub(super) ground_truth_anchor: AnchorValue,
}

impl Default for OracleFixture {
    fn default() -> Self {
        Self {
            occurrence_count: default_occurrence_count(),
            oracle_verdict: default_anchor_value(),
            ground_truth_anchor: default_anchor_value(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct KernelFixture {
    pub(super) ratio: f32,
    #[serde(default = "default_kernel_only")]
    pub(super) kernel_only: f32,
    #[serde(default = "default_full_recall")]
    pub(super) full: f32,
    pub(super) n_queries_tested: usize,
}

impl KernelFixture {
    pub(super) fn validate(&self) -> Result<(), String> {
        validate_fraction(self.ratio, "kernel.ratio")?;
        validate_fraction(self.kernel_only, "kernel.kernel_only")?;
        validate_fraction(self.full, "kernel.full")
    }
}

pub(super) struct KernelSource(pub(super) KernelFixture);

impl KernelRecallSource for KernelSource {
    fn kernel_recall_report(
        &self,
        held_out: &HeldOutSplit,
        _clock: &dyn Clock,
    ) -> Result<RecallReport, LodestarError> {
        Ok(RecallReport {
            kernel_only: self.0.kernel_only,
            full: self.0.full,
            ratio: self.0.ratio,
            n_queries_tested: self.0.n_queries_tested,
            held_out: held_out.held_out_ids.clone(),
            ..RecallReport::default()
        })
    }
}

pub(super) struct CalibrationSourceFixture(pub(super) CalibrationMeasurement);

impl CalibrationSource for CalibrationSourceFixture {
    fn calibration_measurement(
        &self,
        _domain: &DomainId,
        held_out: &HeldOutSplit,
        _clock: &dyn Clock,
    ) -> Result<CalibrationMeasurement, OracleError> {
        let mut measurement = self.0.clone();
        measurement.held_out_count = held_out.held_out_count();
        Ok(measurement)
    }
}

pub(super) struct GoodhartSourceFixture(pub(super) GoodhartDefenseMeasurement);

impl GoodhartDefenseSource for GoodhartSourceFixture {
    fn goodhart_defense_measurement(
        &self,
        _domain: &DomainId,
        held_out: &HeldOutSplit,
        _clock: &dyn Clock,
    ) -> Result<GoodhartDefenseMeasurement, OracleError> {
        let mut measurement = self.0.clone();
        measurement.held_out_count = held_out.held_out_count();
        Ok(measurement)
    }
}

pub(super) struct MistakeSourceFixture(pub(super) MistakeClosureMeasurement);

impl MistakeClosureSource for MistakeSourceFixture {
    fn mistake_closure_measurement(
        &self,
        _domain: &DomainId,
        _clock: &dyn Clock,
    ) -> Result<MistakeClosureMeasurement, OracleError> {
        Ok(self.0.clone())
    }
}

pub(super) fn validate_bits(value: f32, name: &str) -> Result<(), String> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(format!("{name} must be finite and non-negative"))
    }
}

pub(super) fn validate_fraction(value: f32, name: &str) -> Result<(), String> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(format!("{name} must be finite and in [0, 1]"))
    }
}

pub(super) fn default_occurrence_count() -> usize {
    MIN_VALIDITY_SAMPLES
}

pub(super) fn default_anchor_value() -> AnchorValue {
    AnchorValue::Text("pass".to_string())
}

pub(super) fn default_samples() -> usize {
    120
}

pub(super) fn default_trust() -> TrustTag {
    TrustTag::Trusted
}

fn default_kernel_only() -> f32 {
    KERNEL_RECALL_RATIO
}

fn default_full_recall() -> f32 {
    1.0
}
