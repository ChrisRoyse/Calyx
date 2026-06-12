use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use calyx_core::{CalyxError, Clock, LedgerRef, LensId, Result};
use calyx_ledger::LedgerCfStore;
use serde::{Deserialize, Serialize};

use crate::{
    AnnealLedger, AnnealLedgerAction, AnnealLedgerEntry, ChangeId, JValue, LogicalTime,
    MetricSnapshot,
};

pub const CALYX_ANNEAL_GOODHART_INVALID_METRIC: &str = "CALYX_ANNEAL_GOODHART_INVALID_METRIC";
pub const CALYX_ANNEAL_GOODHART_INVALID_CONFIG: &str = "CALYX_ANNEAL_GOODHART_INVALID_CONFIG";
pub const DEFAULT_GTAU_THRESHOLD: f64 = 0.95;
pub const DEFAULT_CROSS_LENS_DOMINANCE_THRESHOLD: f64 = 0.80;
pub const DEFAULT_HELD_OUT_MIN_GAIN_FRACTION: f64 = 0.01;
pub const DEFAULT_GOODHART_VIOLATION_PENALTY_WEIGHT: f64 = 1.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "violation", rename_all = "snake_case")]
pub enum GoodhartViolation {
    HeldOutRegression {
        j_train_delta: f64,
        j_heldout_delta: f64,
    },
    GtauViolation {
        in_region_frac: f64,
        threshold: f64,
    },
    CrossLensAnomaly {
        anomalous_lens: LensId,
        delta_fraction: f64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GoodhartReport {
    pub passed: bool,
    pub violations: Vec<GoodhartViolation>,
    pub p_goodhart_increment: f64,
    pub j_train_delta: f64,
    pub j_heldout_delta: Option<f64>,
    pub in_region_frac: Option<f64>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeldOutSet {
    pub id: String,
    pub grounded_anchor_count: usize,
    pub sealed: bool,
    pub before: Option<JValue>,
    pub after: Option<JValue>,
}

impl HeldOutSet {
    pub fn empty(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            grounded_anchor_count: 0,
            sealed: false,
            before: None,
            after: None,
        }
    }

    pub fn sealed(
        id: impl Into<String>,
        grounded_anchor_count: usize,
        before: JValue,
        after: JValue,
    ) -> Self {
        Self {
            id: id.into(),
            grounded_anchor_count,
            sealed: true,
            before: Some(before),
            after: Some(after),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LensContributionDelta {
    pub lens_id: LensId,
    pub delta: f64,
}

pub trait WardGtau: Send + Sync {
    fn in_region_fraction(&self, held_out_set: &HeldOutSet) -> Result<Option<f64>>;
}

#[derive(Clone)]
pub struct GoodhartChecker {
    pub held_out_set: HeldOutSet,
    pub ward: Arc<dyn WardGtau>,
    pub gtau_threshold: f64,
    pub cross_lens_threshold: f64,
    pub held_out_min_gain_fraction: f64,
    pub violation_penalty_weight: f64,
}

impl GoodhartChecker {
    pub fn new(held_out_set: HeldOutSet, ward: Arc<dyn WardGtau>) -> Self {
        Self {
            held_out_set,
            ward,
            gtau_threshold: DEFAULT_GTAU_THRESHOLD,
            cross_lens_threshold: DEFAULT_CROSS_LENS_DOMINANCE_THRESHOLD,
            held_out_min_gain_fraction: DEFAULT_HELD_OUT_MIN_GAIN_FRACTION,
            violation_penalty_weight: DEFAULT_GOODHART_VIOLATION_PENALTY_WEIGHT,
        }
    }

    pub fn with_gtau_threshold(mut self, threshold: f64) -> Self {
        self.gtau_threshold = threshold;
        self
    }

    pub fn with_cross_lens_threshold(mut self, threshold: f64) -> Self {
        self.cross_lens_threshold = threshold;
        self
    }

    pub fn with_held_out_min_gain_fraction(mut self, fraction: f64) -> Self {
        self.held_out_min_gain_fraction = fraction;
        self
    }

    pub fn with_violation_penalty_weight(mut self, weight: f64) -> Self {
        self.violation_penalty_weight = weight;
        self
    }

    pub fn check(
        &self,
        before: &JValue,
        after: &JValue,
        lens_deltas: &[LensContributionDelta],
    ) -> Result<GoodhartReport> {
        self.validate_config()?;
        validate_j_value(before, "before")?;
        validate_j_value(after, "after")?;
        let j_train_delta = validate_finite(after.j - before.j, "j_train_delta")?;
        let mut warnings = Vec::new();
        let mut violations = Vec::new();
        let j_heldout_delta = self.check_held_out(j_train_delta, &mut warnings, &mut violations)?;
        let in_region_frac = self.check_gtau(&mut warnings, &mut violations)?;
        self.check_cross_lens(j_train_delta, lens_deltas, &mut violations)?;
        let passed = violations.is_empty();
        let p_goodhart_increment = if passed {
            0.0
        } else {
            (j_train_delta.abs() * self.violation_penalty_weight).max(0.0)
        };
        Ok(GoodhartReport {
            passed,
            violations,
            p_goodhart_increment,
            j_train_delta,
            j_heldout_delta,
            in_region_frac,
            warnings,
        })
    }

    fn validate_config(&self) -> Result<()> {
        validate_fraction(self.gtau_threshold, "gtau_threshold").map_err(invalid_config)?;
        validate_fraction(self.cross_lens_threshold, "cross_lens_threshold")
            .map_err(invalid_config)?;
        validate_fraction(
            self.held_out_min_gain_fraction,
            "held_out_min_gain_fraction",
        )
        .map_err(invalid_config)?;
        validate_nonnegative(self.violation_penalty_weight, "violation_penalty_weight")
            .map_err(invalid_config)?;
        Ok(())
    }

    fn check_held_out(
        &self,
        j_train_delta: f64,
        warnings: &mut Vec<String>,
        violations: &mut Vec<GoodhartViolation>,
    ) -> Result<Option<f64>> {
        if self.held_out_set.grounded_anchor_count == 0 {
            warnings.push("held_out_set_empty_skip_held_out_check".to_string());
            return Ok(None);
        }
        if !self.held_out_set.sealed {
            return Err(invalid_config(
                "held_out_set must be sealed before Goodhart validation",
            ));
        }
        let before = self
            .held_out_set
            .before
            .as_ref()
            .ok_or_else(|| invalid_config("held_out_set missing before JValue"))?;
        let after = self
            .held_out_set
            .after
            .as_ref()
            .ok_or_else(|| invalid_config("held_out_set missing after JValue"))?;
        validate_j_value(before, "held_out_before")?;
        validate_j_value(after, "held_out_after")?;
        let j_heldout_delta = validate_finite(after.j - before.j, "j_heldout_delta")?;
        if j_heldout_delta <= self.held_out_min_gain_fraction * j_train_delta {
            violations.push(GoodhartViolation::HeldOutRegression {
                j_train_delta,
                j_heldout_delta,
            });
        }
        Ok(Some(j_heldout_delta))
    }

    fn check_gtau(
        &self,
        warnings: &mut Vec<String>,
        violations: &mut Vec<GoodhartViolation>,
    ) -> Result<Option<f64>> {
        let in_region_frac = match self.ward.in_region_fraction(&self.held_out_set) {
            Ok(Some(value)) => {
                validate_fraction(value, "in_region_frac").map_err(invalid_metric)?
            }
            Ok(None) => {
                warnings.push("ward_gtau_unavailable_treated_as_zero".to_string());
                0.0
            }
            Err(error) => {
                warnings.push(format!("ward_gtau_error_treated_as_zero: {}", error.code));
                0.0
            }
        };
        if in_region_frac < self.gtau_threshold {
            violations.push(GoodhartViolation::GtauViolation {
                in_region_frac,
                threshold: self.gtau_threshold,
            });
        }
        Ok(Some(in_region_frac))
    }

    fn check_cross_lens(
        &self,
        j_train_delta: f64,
        lens_deltas: &[LensContributionDelta],
        violations: &mut Vec<GoodhartViolation>,
    ) -> Result<()> {
        if j_train_delta.abs() <= f64::EPSILON {
            return Ok(());
        }
        for delta in lens_deltas {
            validate_finite(delta.delta, "lens_contribution_delta")?;
            let delta_fraction = (delta.delta / j_train_delta).abs();
            if delta_fraction > self.cross_lens_threshold {
                violations.push(GoodhartViolation::CrossLensAnomaly {
                    anomalous_lens: delta.lens_id,
                    delta_fraction,
                });
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoodhartState {
    pub p_goodhart: f64,
}

impl Default for GoodhartState {
    fn default() -> Self {
        Self { p_goodhart: 0.0 }
    }
}

pub fn goodhart_state_path(vault: &Path) -> PathBuf {
    vault.join(".anneal").join("goodhart_state.toml")
}

pub fn read_goodhart_state_from_vault(vault: &Path) -> Result<GoodhartState> {
    let path = goodhart_state_path(vault);
    if !path.exists() {
        return Ok(GoodhartState::default());
    }
    let bytes = fs::read(&path).map_err(|error| {
        invalid_config(format!("read Goodhart state {}: {error}", path.display()))
    })?;
    let text = std::str::from_utf8(&bytes).map_err(|error| {
        invalid_config(format!(
            "read Goodhart state {} as UTF-8: {error}",
            path.display()
        ))
    })?;
    validate_state(toml::from_str::<GoodhartState>(text).map_err(|error| {
        invalid_config(format!("parse Goodhart state {}: {error}", path.display()))
    })?)
}

pub fn add_goodhart_penalty_to_vault(vault: &Path, increment: f64) -> Result<GoodhartState> {
    validate_nonnegative(increment, "p_goodhart_increment").map_err(invalid_metric)?;
    let mut state = read_goodhart_state_from_vault(vault)?;
    state.p_goodhart =
        validate_nonnegative(state.p_goodhart + increment, "p_goodhart").map_err(invalid_metric)?;
    write_goodhart_state(vault, state)
}

pub fn write_goodhart_state(vault: &Path, state: GoodhartState) -> Result<GoodhartState> {
    let state = validate_state(state)?;
    let path = goodhart_state_path(vault);
    let parent = path
        .parent()
        .ok_or_else(|| invalid_config("Goodhart state path has no parent"))?;
    fs::create_dir_all(parent).map_err(|error| {
        invalid_config(format!(
            "create Goodhart state dir {}: {error}",
            parent.display()
        ))
    })?;
    let encoded = toml::to_string_pretty(&state)
        .map_err(|error| invalid_config(format!("encode Goodhart state: {error}")))?;
    fs::write(&path, encoded.as_bytes()).map_err(|error| {
        invalid_config(format!("write Goodhart state {}: {error}", path.display()))
    })?;
    Ok(state)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoodhartLedgerContext {
    pub change_id: ChangeId,
    pub artifact_id: String,
    pub prior_ptr_hash: [u8; 32],
    pub candidate_ptr_hash: [u8; 32],
    pub ts: LogicalTime,
}

pub fn record_goodhart_report<S, C>(
    report: &GoodhartReport,
    context: GoodhartLedgerContext,
    ledger: &mut AnnealLedger<S, C>,
) -> Result<LedgerRef>
where
    S: LedgerCfStore,
    C: Clock,
{
    let entry = AnnealLedgerEntry {
        action: if report.passed {
            AnnealLedgerAction::GoodhartPassed
        } else {
            AnnealLedgerAction::GoodhartFailed
        },
        change_id: context.change_id,
        artifact_id: context.artifact_id,
        prior_ptr_hash: context.prior_ptr_hash,
        candidate_ptr_hash: context.candidate_ptr_hash,
        metrics: MetricSnapshot::empty(context.ts),
        ts: context.ts,
        description: goodhart_ledger_description(report),
        fault: None,
        proposal: None,
        details: None,
        prev_hash: None,
    };
    ledger.write(entry)
}

fn goodhart_ledger_description(report: &GoodhartReport) -> String {
    let heldout = report
        .j_heldout_delta
        .map(|value| format!("{value:.6}"))
        .unwrap_or_else(|| "none".to_string());
    let gtau = report
        .in_region_frac
        .map(|value| format!("{value:.6}"))
        .unwrap_or_else(|| "none".to_string());
    format!(
        "Goodhart report v1 passed {} violations {} train_delta {:.6} heldout_delta {} gtau_in_region {} penalty {:.6}",
        report.passed,
        report.violations.len(),
        report.j_train_delta,
        heldout,
        gtau,
        report.p_goodhart_increment
    )
}

fn validate_state(state: GoodhartState) -> Result<GoodhartState> {
    validate_nonnegative(state.p_goodhart, "p_goodhart").map_err(invalid_config)?;
    Ok(state)
}

fn validate_j_value(value: &JValue, name: &str) -> Result<()> {
    validate_finite(value.j, &format!("{name}.j"))?;
    validate_nonnegative(value.dpi_ceiling, &format!("{name}.dpi_ceiling"))
        .map_err(invalid_metric)?;
    validate_finite(value.dpi_headroom, &format!("{name}.dpi_headroom"))?;
    for (term_name, term) in [
        ("w1_info", value.terms.w1_info),
        ("w2_n_eff", value.terms.w2_n_eff),
        ("w3_sufficiency", value.terms.w3_sufficiency),
        ("w4_kernel_recall", value.terms.w4_kernel_recall),
        ("w5_oracle_accuracy", value.terms.w5_oracle_accuracy),
        ("w6_mistake_rate", value.terms.w6_mistake_rate),
        ("w7_compression", value.terms.w7_compression),
        ("w8_coverage", value.terms.w8_coverage),
        ("p_redundant", value.terms.p_redundant),
        ("p_ungrounded", value.terms.p_ungrounded),
        ("p_goodhart", value.terms.p_goodhart),
    ] {
        validate_nonnegative(term, &format!("{name}.terms.{term_name}")).map_err(invalid_metric)?;
    }
    Ok(())
}

fn validate_fraction(value: f64, name: &str) -> std::result::Result<f64, String> {
    validate_nonnegative(value, name)?;
    if value > 1.0 {
        return Err(format!("{name} must be <= 1.0, got {value}"));
    }
    Ok(value)
}

fn validate_nonnegative(value: f64, name: &str) -> std::result::Result<f64, String> {
    validate_finite_string(value, name)?;
    if value < 0.0 {
        return Err(format!("{name} must be non-negative, got {value}"));
    }
    Ok(value)
}

fn validate_finite(value: f64, name: &str) -> Result<f64> {
    validate_finite_string(value, name).map_err(invalid_metric)?;
    Ok(value)
}

fn validate_finite_string(value: f64, name: &str) -> std::result::Result<f64, String> {
    if !value.is_finite() {
        return Err(format!("{name} must be finite, got {value}"));
    }
    Ok(value)
}

fn invalid_metric(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_GOODHART_INVALID_METRIC,
        message: message.into(),
        remediation: "re-measure held-out, Gtau, and cross-lens Goodhart evidence",
    }
}

fn invalid_config(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_GOODHART_INVALID_CONFIG,
        message: message.into(),
        remediation: "correct sealed held-out Goodhart configuration before promotion",
    }
}
