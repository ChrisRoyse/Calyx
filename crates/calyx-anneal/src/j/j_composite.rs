use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

pub const CALYX_ANNEAL_J_INVALID_METRIC: &str = "CALYX_ANNEAL_J_INVALID_METRIC";
pub const CALYX_ANNEAL_J_INVALID_CONFIG: &str = "CALYX_ANNEAL_J_INVALID_CONFIG";
pub const CALYX_ANNEAL_J_SYNTHETIC_RECURSION: &str = "CALYX_ANNEAL_J_SYNTHETIC_RECURSION";
pub const DEFAULT_J_DOMAIN: &str = "default";
pub const UNIT_PENALTY: f64 = 1.0;
pub const REDUNDANCY_PENALTY: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct JTerms {
    pub w1_info: f64,
    pub w2_n_eff: f64,
    pub w3_sufficiency: f64,
    pub w4_kernel_recall: f64,
    pub w5_oracle_accuracy: f64,
    pub w6_mistake_rate: f64,
    pub w7_compression: f64,
    pub w8_coverage: f64,
    pub p_redundant: f64,
    pub p_ungrounded: f64,
    pub p_goodhart: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JValue {
    pub j: f64,
    pub terms: JTerms,
    pub dpi_ceiling: f64,
    pub dpi_headroom: f64,
    pub provisional_excluded: usize,
    pub weights: JWeights,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JWeights {
    pub w1: f64,
    pub w2: f64,
    pub w3: f64,
    pub w4: f64,
    pub w5: f64,
    pub w6: f64,
    pub w7: f64,
    pub w8: f64,
}

impl Default for JWeights {
    fn default() -> Self {
        Self {
            w1: 1.0,
            w2: 1.0,
            w3: 1.0,
            w4: 1.0,
            w5: 1.0,
            w6: 1.0,
            w7: 1.0,
            w8: 1.0,
        }
    }
}

impl JWeights {
    pub fn zero() -> Self {
        Self {
            w1: 0.0,
            w2: 0.0,
            w3: 0.0,
            w4: 0.0,
            w5: 0.0,
            w6: 0.0,
            w7: 0.0,
            w8: 0.0,
        }
    }

    fn validate(self) -> Result<Self> {
        for (name, value) in [
            ("w1", self.w1),
            ("w2", self.w2),
            ("w3", self.w3),
            ("w4", self.w4),
            ("w5", self.w5),
            ("w6", self.w6),
            ("w7", self.w7),
            ("w8", self.w8),
        ] {
            validate_nonnegative(value, name).map_err(invalid_config)?;
        }
        Ok(self)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JGeneratedPositiveCredit {
    pub count: usize,
    pub w1_info: f64,
    pub w2_n_eff: f64,
    pub w3_sufficiency: f64,
    pub w4_kernel_recall: f64,
    pub w5_oracle_accuracy: f64,
    pub w7_compression: f64,
    pub w8_coverage: f64,
}

impl JGeneratedPositiveCredit {
    fn validate(self) -> Result<Self> {
        for (name, value) in [
            ("generated_positive_credit.w1_info", self.w1_info),
            ("generated_positive_credit.w2_n_eff", self.w2_n_eff),
            (
                "generated_positive_credit.w3_sufficiency",
                self.w3_sufficiency,
            ),
            (
                "generated_positive_credit.w4_kernel_recall",
                self.w4_kernel_recall,
            ),
            (
                "generated_positive_credit.w5_oracle_accuracy",
                self.w5_oracle_accuracy,
            ),
            (
                "generated_positive_credit.w7_compression",
                self.w7_compression,
            ),
            ("generated_positive_credit.w8_coverage", self.w8_coverage),
        ] {
            validate_nonnegative(value, name).map_err(invalid_metric)?;
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JObjectiveContext {
    pub domain: String,
    pub panel_len: usize,
    pub weights: JWeights,
    pub goodhart_penalty: f64,
}

impl JObjectiveContext {
    pub fn new(domain: impl Into<String>, panel_len: usize) -> Self {
        Self {
            domain: domain.into(),
            panel_len,
            weights: JWeights::default(),
            goodhart_penalty: 0.0,
        }
    }

    pub fn with_weights(mut self, weights: JWeights) -> Self {
        self.weights = weights;
        self
    }

    pub fn with_goodhart_penalty(mut self, penalty: f64) -> Self {
        self.goodhart_penalty = penalty;
        self
    }
}

impl Default for JObjectiveContext {
    fn default() -> Self {
        Self::new(DEFAULT_J_DOMAIN, 0)
    }
}

pub trait JMetricSources {
    fn mutual_info_panel_anchor(&self) -> f64;
    fn n_eff(&self) -> f64;
    fn panel_sufficiency(&self, domain: &str) -> f64;
    fn kernel_recall(&self) -> f64;
    fn oracle_accuracy(&self) -> f64;
    fn mistake_rate(&self) -> f64;
    fn compression_yield(&self) -> f64;
    fn coverage(&self) -> f64;
    fn dpi_ceiling(&self) -> f64;
    fn provisional_count(&self) -> usize;
    fn generated_positive_credit(&self) -> JGeneratedPositiveCredit {
        JGeneratedPositiveCredit::default()
    }
    fn synthetic_recursion_credit_attempted(&self) -> bool {
        false
    }
}

pub fn compute_j<S>(context: &JObjectiveContext, sources: &S) -> Result<JValue>
where
    S: JMetricSources,
{
    if sources.synthetic_recursion_credit_attempted() {
        return Err(synthetic_recursion_error(
            "generated/model-output-derived signals cannot receive positive J credit",
        ));
    }
    let weights = context.weights.validate()?;
    let generated_credit = sources.generated_positive_credit().validate()?;
    let dpi_ceiling =
        validate_nonnegative(sources.dpi_ceiling(), "dpi_ceiling").map_err(invalid_metric)?;
    let raw_info = validate_nonnegative(
        sources.mutual_info_panel_anchor(),
        "mutual_info_panel_anchor",
    )
    .map_err(invalid_metric)?;
    let raw_sufficiency = validate_nonnegative(
        sources.panel_sufficiency(&context.domain),
        "panel_sufficiency",
    )
    .map_err(invalid_metric)?;
    let raw_n_eff = validate_nonnegative(sources.n_eff(), "n_eff").map_err(invalid_metric)?;
    let grounded_info = exclude_generated_credit(
        raw_info,
        generated_credit.w1_info,
        "mutual_info_panel_anchor",
    )?;
    let grounded_n_eff = exclude_generated_credit(raw_n_eff, generated_credit.w2_n_eff, "n_eff")?;
    let grounded_sufficiency = exclude_generated_credit(
        raw_sufficiency,
        generated_credit.w3_sufficiency,
        "panel_sufficiency",
    )?;
    let provisional_count = sources.provisional_count();
    let provisional_excluded = provisional_count
        .checked_add(generated_credit.count)
        .ok_or_else(|| invalid_metric("provisional exclusion count overflow"))?;
    let info = if provisional_count == 0 {
        grounded_info.min(dpi_ceiling)
    } else {
        0.0
    };
    let sufficiency = grounded_sufficiency.min(dpi_ceiling);
    let terms = JTerms {
        w1_info: info,
        w2_n_eff: grounded_n_eff,
        w3_sufficiency: sufficiency,
        w4_kernel_recall: exclude_generated_credit(
            validate_nonnegative(sources.kernel_recall(), "kernel_recall")
                .map_err(invalid_metric)?,
            generated_credit.w4_kernel_recall,
            "kernel_recall",
        )?,
        w5_oracle_accuracy: exclude_generated_credit(
            validate_nonnegative(sources.oracle_accuracy(), "oracle_accuracy")
                .map_err(invalid_metric)?,
            generated_credit.w5_oracle_accuracy,
            "oracle_accuracy",
        )?,
        w6_mistake_rate: validate_nonnegative(sources.mistake_rate(), "mistake_rate")
            .map_err(invalid_metric)?,
        w7_compression: exclude_generated_credit(
            validate_nonnegative(sources.compression_yield(), "compression_yield")
                .map_err(invalid_metric)?,
            generated_credit.w7_compression,
            "compression_yield",
        )?,
        w8_coverage: exclude_generated_credit(
            validate_nonnegative(sources.coverage(), "coverage").map_err(invalid_metric)?,
            generated_credit.w8_coverage,
            "coverage",
        )?,
        p_redundant: redundancy_penalty(context.panel_len, grounded_n_eff),
        p_ungrounded: ungrounded_penalty(provisional_excluded)?,
        p_goodhart: validate_nonnegative(context.goodhart_penalty, "p_goodhart")
            .map_err(invalid_metric)?,
    };
    let weighted_positive = weights.w1 * terms.w1_info
        + weights.w2 * terms.w2_n_eff
        + weights.w3 * terms.w3_sufficiency
        + weights.w4 * terms.w4_kernel_recall
        + weights.w5 * terms.w5_oracle_accuracy
        + weights.w7 * terms.w7_compression
        + weights.w8 * terms.w8_coverage;
    let weighted_negative = weights.w6 * terms.w6_mistake_rate
        + terms.p_redundant
        + terms.p_ungrounded
        + terms.p_goodhart;
    let j = weighted_positive - weighted_negative;
    if !j.is_finite() {
        return Err(invalid_metric(format!(
            "computed J must be finite, got {j}"
        )));
    }
    Ok(JValue {
        j,
        terms,
        dpi_ceiling,
        dpi_headroom: (dpi_ceiling - grounded_info).min(dpi_ceiling - grounded_sufficiency),
        provisional_excluded,
        weights,
    })
}

pub fn j_weights_path(vault: &Path) -> PathBuf {
    vault.join(".anneal").join("j_weights.toml")
}

pub fn set_objective_weights(vault: &Path, weights: JWeights) -> Result<JWeights> {
    let weights = weights.validate()?;
    let path = j_weights_path(vault);
    let parent = path
        .parent()
        .ok_or_else(|| invalid_config("j weight path has no parent"))?;
    fs::create_dir_all(parent).map_err(|error| {
        invalid_config(format!(
            "create objective weight dir {}: {error}",
            parent.display()
        ))
    })?;
    let encoded = toml::to_string_pretty(&weights)
        .map_err(|error| invalid_config(format!("encode objective weights: {error}")))?;
    fs::write(&path, encoded.as_bytes()).map_err(|error| {
        invalid_config(format!(
            "write objective weights {}: {error}",
            path.display()
        ))
    })?;
    Ok(weights)
}

pub fn read_objective_weights_from_vault(vault: &Path) -> Result<JWeights> {
    let path = j_weights_path(vault);
    if !path.exists() {
        return Ok(JWeights::default());
    }
    let bytes = fs::read(&path).map_err(|error| {
        invalid_config(format!(
            "read objective weights {}: {error}",
            path.display()
        ))
    })?;
    let text = std::str::from_utf8(&bytes).map_err(|error| {
        invalid_config(format!(
            "read objective weights {} as UTF-8: {error}",
            path.display()
        ))
    })?;
    toml::from_str::<JWeights>(text)
        .map_err(|error| {
            invalid_config(format!(
                "parse objective weights {}: {error}",
                path.display()
            ))
        })?
        .validate()
}

fn redundancy_penalty(panel_len: usize, n_eff: f64) -> f64 {
    ((panel_len as f64) - n_eff).max(0.0) * REDUNDANCY_PENALTY
}

fn exclude_generated_credit(raw: f64, generated: f64, name: &str) -> Result<f64> {
    if generated > raw {
        return Err(invalid_metric(format!(
            "{name} generated-positive credit {generated} exceeds measured input {raw}"
        )));
    }
    Ok(raw - generated)
}

fn ungrounded_penalty(count: usize) -> Result<f64> {
    let penalty = count as f64 * UNIT_PENALTY;
    if !penalty.is_finite() {
        return Err(invalid_metric(format!(
            "provisional exclusion penalty must be finite, got {penalty}"
        )));
    }
    Ok(penalty)
}

fn validate_nonnegative(value: f64, name: &str) -> std::result::Result<f64, String> {
    if !value.is_finite() || value < 0.0 {
        return Err(format!(
            "{name} must be finite and non-negative, got {value}"
        ));
    }
    Ok(value)
}

fn invalid_metric(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_J_INVALID_METRIC,
        message: message.into(),
        remediation: "re-measure grounded objective inputs before computing J",
    }
}

fn invalid_config(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_J_INVALID_CONFIG,
        message: message.into(),
        remediation: "correct PH48 objective configuration before computing J",
    }
}

fn synthetic_recursion_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_J_SYNTHETIC_RECURSION,
        message: message.into(),
        remediation: "tag generated/model-output-derived signals as generated_positive_credit or re-measure real grounded inputs through frozen lenses",
    }
}
