use std::collections::BTreeMap;

use calyx_assay::EnsembleCard;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct InputReport {
    pub(crate) target_class: usize,
    pub(crate) domain: String,
    pub(crate) card: EnsembleCard,
}

#[derive(Clone, Debug)]
pub(crate) struct LoadedReport {
    pub(crate) path: std::path::PathBuf,
    pub(crate) report: InputReport,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct MultiAnchorReport {
    pub(crate) schema_version: u32,
    pub(crate) role: String,
    pub(crate) status: String,
    pub(crate) mode: String,
    pub(crate) gate_passed: bool,
    pub(crate) report_count: usize,
    pub(crate) lens_count: usize,
    pub(crate) passing_lens_count: usize,
    pub(crate) min_lenses: usize,
    pub(crate) min_marginal_bits: f32,
    pub(crate) max_redundancy: f32,
    pub(crate) family_span_pass: bool,
    pub(crate) redundancy_bound_pass: bool,
    pub(crate) no_collapse_pass: bool,
    pub(crate) association_family_count: usize,
    pub(crate) association_families: BTreeMap<String, Vec<u16>>,
    pub(crate) min_best_marginal_bits: f32,
    pub(crate) max_best_marginal_bits: f32,
    pub(crate) weakest_lens: String,
    pub(crate) target_summaries: Vec<TargetSummary>,
    pub(crate) lenses: Vec<LensEvidence>,
    pub(crate) source_reports: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct TargetSummary {
    pub(crate) target_class: usize,
    pub(crate) domain: String,
    pub(crate) report_path: String,
    pub(crate) status: String,
    pub(crate) no_collapse_pass: bool,
    pub(crate) family_span_pass: bool,
    pub(crate) redundancy_bound_pass: bool,
    pub(crate) n_eff: f32,
    pub(crate) panel_bits: f32,
    pub(crate) max_marginal_bits: f32,
    pub(crate) keep_count: usize,
    pub(crate) park_count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct LensEvidence {
    pub(crate) slot: u16,
    pub(crate) name: String,
    pub(crate) association_family: String,
    pub(crate) passed: bool,
    pub(crate) best_target_class: usize,
    pub(crate) best_domain: String,
    pub(crate) best_marginal_bits: f32,
    pub(crate) best_solo_bits: f32,
    pub(crate) target_values: Vec<TargetLensValue>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct TargetLensValue {
    pub(crate) target_class: usize,
    pub(crate) domain: String,
    pub(crate) marginal_bits: f32,
    pub(crate) solo_bits: f32,
    pub(crate) decision: String,
}
