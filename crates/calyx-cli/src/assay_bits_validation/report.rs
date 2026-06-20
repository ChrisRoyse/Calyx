use calyx_assay::{MiEstimate, PanelPackingReport, ResourceDensity, ResourceUsage};
use calyx_core::Placement;
use serde::Serialize;

use crate::assay_anchor_audit::AnchorAudit;

use super::comparison::PanelComparisonReport;
use super::selection::SignalDensityReport;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct AssayBitsReport {
    pub(crate) dataset: String,
    pub(crate) embedding_model_id: String,
    pub(crate) domain: String,
    pub(crate) n_samples: usize,
    pub(crate) target_class: usize,
    pub(crate) anchor_audit: AnchorAudit,
    pub(crate) anchor_leaks_into_input: bool,
    pub(crate) trivial_anchor: bool,
    pub(crate) grounded_gate_eligible: bool,
    pub(crate) anchor_entropy_bits: f32,
    pub(crate) min_informative_target_entropy_bits: f32,
    pub(crate) min_bits: f32,
    pub(crate) max_corr: f32,
    pub(crate) lenses: Vec<LensReport>,
    pub(crate) panel: PanelReport,
    pub(crate) strata: Vec<StratumReport>,
    /// Present only when `--cost-json` was supplied: per-lens signal density
    /// (bits per resource) ranked for panel selection (#717 signal-density).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) signal_density: Option<SignalDensityReport>,
    /// Present only when `--panel-budget-json` was supplied: the actual
    /// density-ordered panel packing verdict under the fixed resource budget.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) packed_panel: Option<PanelPackingReport>,
    /// Present only when resource packing runs: the density panel compared
    /// with best raw-signal one-/two-lens controls under the same budget.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) panel_comparison: Option<PanelComparisonReport>,
    pub(crate) cf_root: String,
    pub(crate) assay_cf_rows_persisted: usize,
    pub(crate) assay_cf_rows_readback: usize,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct LensReport {
    pub(crate) name: String,
    pub(crate) redundant: bool,
    pub(crate) bits_about: f32,
    pub(crate) anchor_leaks_into_input: bool,
    pub(crate) trivial_anchor: bool,
    pub(crate) grounded_gate_eligible: bool,
    pub(crate) ci: [f32; 2],
    pub(crate) estimate_bound: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) power_calibration_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) power_recovery_ratio: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) power_recovered_bits: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) power_planted_bits: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) seed_sigma_bits: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) seed_count: Option<usize>,
    pub(crate) unresolved: bool,
    pub(crate) estimator: String,
    pub(crate) max_pairwise_corr: f32,
    pub(crate) max_pairwise_corr_ci: [f32; 2],
    pub(crate) admitted: bool,
    pub(crate) rejection_reason: Option<String>,
    /// Per-lens signal density, present only when `--cost-json` was supplied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) density: Option<ResourceDensity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) usage: Option<ResourceUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) placement: Option<Placement>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct PanelReport {
    pub(crate) admitted_lenses: Vec<String>,
    pub(crate) i_panel_anchor: f32,
    pub(crate) ci_95: [f32; 2],
    pub(crate) estimate_bound: String,
    pub(crate) sufficiency_basis_bits: f32,
    pub(crate) power_calibration_status: Option<String>,
    pub(crate) power_recovery_ratio: Option<f32>,
    pub(crate) power_recovered_bits: Option<f32>,
    pub(crate) power_planted_bits: Option<f32>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct StratumReport {
    pub(crate) name: String,
    pub(crate) bits: f32,
    pub(crate) frequency: f32,
}

pub(crate) struct LensMeasurement {
    pub(crate) index: usize,
    pub(crate) name: String,
    pub(crate) redundant: bool,
    pub(crate) estimate: MiEstimate,
}
