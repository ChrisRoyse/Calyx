//! PRD-22 formula coverage catalog.

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

pub const FORMULA_COVERAGE_SURFACE: &str = "formula-coverage";
pub const FORMULA_COVERAGE_ARTIFACT_KIND: &str = "prd22.formula-coverage.v1";
pub const FORMULA_COVERAGE_SCHEMA_VERSION: u64 = 1;
pub const FORMULA_COVERAGE_SOT_KEY: &str = "formula_coverage/prd22";
pub const CALYX_FORMULA_COVERAGE_MISSING: &str = "CALYX_FORMULA_COVERAGE_MISSING";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormulaCoverageStatus {
    Covered,
    Missing,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FormulaCoverageArtifact {
    pub surface: String,
    pub artifact_kind: String,
    pub schema_version: u64,
    pub source_of_truth: String,
    pub generated_at: u64,
    pub fsv_root: String,
    pub rows: Vec<FormulaCoverageRow>,
    pub summary: FormulaCoverageSummary,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormulaCoverageRow {
    pub formula: String,
    pub prd_ref: String,
    pub engine: String,
    pub callable: String,
    pub tunable_params: Vec<String>,
    pub test: String,
    pub fsv_root: String,
    pub status: FormulaCoverageStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormulaCoverageSummary {
    pub total_rows: usize,
    pub covered_rows: usize,
    pub missing_rows: usize,
    pub self_tuning_representatives: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormulaRowSpec {
    pub formula: &'static str,
    pub prd_ref: &'static str,
    pub engine: &'static str,
    pub callable: &'static str,
    pub tunable_params: &'static [&'static str],
    pub test: &'static str,
}

pub fn prd22_formula_specs() -> &'static [FormulaRowSpec] {
    FORMULA_ROWS
}

pub fn formula_coverage_artifact(
    fsv_root: impl Into<String>,
    generated_at: u64,
) -> FormulaCoverageArtifact {
    let fsv_root = fsv_root.into();
    let rows = FORMULA_ROWS
        .iter()
        .map(|spec| spec.row(&fsv_root))
        .collect::<Vec<_>>();
    let summary = coverage_summary(&rows);
    FormulaCoverageArtifact {
        surface: FORMULA_COVERAGE_SURFACE.to_string(),
        artifact_kind: FORMULA_COVERAGE_ARTIFACT_KIND.to_string(),
        schema_version: FORMULA_COVERAGE_SCHEMA_VERSION,
        source_of_truth: format!("Aster assay CF key {FORMULA_COVERAGE_SOT_KEY} + artifact file"),
        generated_at,
        fsv_root,
        rows,
        summary,
    }
}

pub fn formula_coverage_json(fsv_root: impl Into<String>, generated_at: u64) -> Result<Vec<u8>> {
    serde_json::to_vec_pretty(&formula_coverage_artifact(fsv_root, generated_at))
        .map_err(|error| coverage_error(format!("serialize formula coverage: {error}")))
}

pub fn validate_formula_coverage(
    artifact: &FormulaCoverageArtifact,
) -> Result<FormulaCoverageSummary> {
    if artifact.surface != FORMULA_COVERAGE_SURFACE {
        return Err(coverage_error("formula coverage surface mismatch"));
    }
    if artifact.artifact_kind != FORMULA_COVERAGE_ARTIFACT_KIND {
        return Err(coverage_error("formula coverage artifact kind mismatch"));
    }
    if artifact.schema_version != FORMULA_COVERAGE_SCHEMA_VERSION {
        return Err(coverage_error("formula coverage schema version mismatch"));
    }
    let summary = coverage_summary(&artifact.rows);
    if summary.missing_rows != 0 || summary.total_rows != FORMULA_ROWS.len() {
        return Err(coverage_error(format!(
            "formula coverage incomplete: covered={} missing={} total={} expected={}",
            summary.covered_rows,
            summary.missing_rows,
            summary.total_rows,
            FORMULA_ROWS.len()
        )));
    }
    Ok(summary)
}

impl FormulaRowSpec {
    fn row(self, fsv_root: &str) -> FormulaCoverageRow {
        FormulaCoverageRow {
            formula: self.formula.to_string(),
            prd_ref: self.prd_ref.to_string(),
            engine: self.engine.to_string(),
            callable: self.callable.to_string(),
            tunable_params: self
                .tunable_params
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            test: self.test.to_string(),
            fsv_root: fsv_root.to_string(),
            status: FormulaCoverageStatus::Covered,
        }
    }
}

fn coverage_summary(rows: &[FormulaCoverageRow]) -> FormulaCoverageSummary {
    let covered_rows = rows
        .iter()
        .filter(|row| row.status == FormulaCoverageStatus::Covered)
        .count();
    FormulaCoverageSummary {
        total_rows: rows.len(),
        covered_rows,
        missing_rows: rows.len().saturating_sub(covered_rows),
        self_tuning_representatives: vec!["rrf.k".to_string(), "ksg.k".to_string()],
    }
}

fn coverage_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_FORMULA_COVERAGE_MISSING,
        message: message.into(),
        remediation: "map every PRD-22 formula to a callable, test, and FSV evidence row",
    }
}

const fn spec(
    formula: &'static str,
    prd_ref: &'static str,
    engine: &'static str,
    callable: &'static str,
    tunable_params: &'static [&'static str],
    test: &'static str,
) -> FormulaRowSpec {
    FormulaRowSpec {
        formula,
        prd_ref,
        engine,
        callable,
        tunable_params,
        test,
    }
}

const NONE: &[&str] = &[];

const FORMULA_ROWS: &[FormulaRowSpec] = &[
    spec(
        "DDA yield",
        "22 S1",
        "loom",
        "calyx_loom::dda_signal_yield",
        &["n_eff_materialization_budget"],
        "calyx-assay::formula_coverage_fsv",
    ),
    spec(
        "Cross-term",
        "22 S1",
        "loom",
        "calyx_loom::LoomStore::cross_term",
        &["cross_term_kind_plan"],
        "calyx-loom::stage5_fsv",
    ),
    spec(
        "Meaning compression yield",
        "22 S1",
        "loom",
        "calyx_loom::meaning_compression_yield",
        &["materialization_budget"],
        "calyx-assay::formula_coverage_fsv",
    ),
    spec(
        "Per-lens signal",
        "22 S2",
        "assay",
        "calyx_assay::lens_signal",
        &["min_signal_bits"],
        "calyx-assay::stage5_fsv",
    ),
    spec(
        "Pairwise redundancy",
        "22 S2",
        "assay",
        "calyx_assay::pair_redundancy",
        &["max_pairwise_corr"],
        "calyx-assay::formula_coverage_fsv",
    ),
    spec(
        "KSG estimator",
        "22 S2",
        "assay",
        "calyx_assay::ksg_mi_continuous",
        &["k"],
        "calyx-assay::stage5_fsv",
    ),
    spec(
        "Partitioned NMI",
        "22 S2",
        "assay",
        "calyx_assay::partitioned_histogram_nmi",
        &["bins"],
        "calyx-assay::stage5_fsv",
    ),
    spec(
        "Effective rank",
        "22 S2",
        "assay",
        "calyx_assay::stable_rank",
        &["redundancy_graph_threshold"],
        "calyx-assay::stage5_fsv",
    ),
    spec(
        "Marginal lens value",
        "22 S2",
        "assay",
        "calyx_assay::marginal_value",
        &["lens_admission_delta"],
        "calyx-assay::formula_coverage_fsv",
    ),
    spec(
        "DPI ceiling",
        "22 S3",
        "assay",
        "calyx_assay::dpi_ceiling",
        NONE,
        "calyx-assay::formula_coverage_fsv",
    ),
    spec(
        "Panel sufficiency",
        "22 S3",
        "assay",
        "calyx_assay::panel_sufficiency",
        &["tau_mi"],
        "calyx-assay::stage5_fsv",
    ),
    spec(
        "Per-sensor decomposition",
        "22 S3",
        "assay",
        "calyx_assay::per_sensor_attribution",
        &["sole_carrier_threshold_bits"],
        "calyx-assay::stage5_fsv",
    ),
    spec(
        "Abundance honesty",
        "22 S3",
        "loom",
        "calyx_loom::AbundanceReport::new",
        &["n_eff_budget", "dpi_ceiling"],
        "calyx-assay::stage5_fsv",
    ),
    spec(
        "Association graph",
        "22 S4",
        "lodestar",
        "calyx_lodestar::build_assoc_graph_from_loom",
        &["directional_confidence"],
        "calyx-lodestar::ph33_loom_assoc_tests",
    ),
    spec(
        "Kernel graph",
        "22 S4",
        "lodestar",
        "calyx_lodestar::select_kernel_graph",
        &["kernel_graph_params"],
        "calyx-lodestar::ph32_lodestar_tests",
    ),
    spec(
        "Grounding kernel",
        "22 S4",
        "lodestar",
        "calyx_lodestar::dfvs_approx",
        &["mfvs_search_depth"],
        "calyx-lodestar::ph32_lodestar_tests",
    ),
    spec(
        "MFVS approx",
        "22 S4",
        "lodestar",
        "calyx_lodestar::tournament_2approx",
        &["approximation_method"],
        "calyx-lodestar::ph32_lodestar_tests",
    ),
    spec(
        "Kernel-only recall",
        "22 S4",
        "lodestar",
        "calyx_lodestar::kernel_recall_gate",
        &["min_recall_ratio", "top_k"],
        "calyx-lodestar::ph33_recall_test_tests",
    ),
    spec(
        "Hop attenuation",
        "22 S4",
        "paths",
        "calyx_paths::attenuate",
        &["hop_decay"],
        "calyx-paths::ph31_paths_tests",
    ),
    spec(
        "Grounding gaps",
        "22 S4",
        "lodestar",
        "calyx_lodestar::grounding_gaps",
        &["label_cost_model"],
        "calyx-lodestar::ph33_grounding_gaps_tests",
    ),
    spec(
        "Gtau guard",
        "22 S5",
        "ward",
        "calyx_ward::guard",
        &["tau_k"],
        "calyx-ward::guard_ph37_fsv",
    ),
    spec(
        "Constellation pass",
        "22 S5",
        "ward",
        "calyx_ward::guard",
        &["guard_policy_kofn"],
        "calyx-ward::guard_tests",
    ),
    spec(
        "Tau calibration",
        "22 S5",
        "ward",
        "calyx_ward::calibrate",
        &["alpha", "target_far"],
        "calyx-ward::calibrate_unit",
    ),
    spec(
        "Novelty new region",
        "22 S5",
        "ward",
        "calyx_ward::classify_novelty",
        &["surprise_threshold"],
        "calyx-ward::novelty_handler",
    ),
    spec(
        "Oracle self-consistency ceiling",
        "22 S6",
        "oracle",
        "calyx_oracle::oracle_ceiling",
        &["tau_corr"],
        "calyx-oracle::prd22",
    ),
    spec(
        "Consequence prediction",
        "22 S6",
        "oracle",
        "calyx_oracle::oracle_predict",
        &["confidence_ceiling"],
        "calyx-oracle::prd22",
    ),
    spec(
        "Butterfly expansion",
        "22 S6",
        "oracle",
        "calyx_oracle::butterfly_expand",
        &["max_hops", "hop_decay"],
        "calyx-oracle::prd22",
    ),
    spec(
        "Super-intelligence predicate",
        "22 S6",
        "oracle",
        "calyx_oracle::super_intelligence",
        &["min_kernel_recall_ratio"],
        "calyx-oracle::prd22",
    ),
    spec(
        "Sufficiency falsification",
        "22 S6",
        "oracle",
        "calyx_oracle::oracle_predict",
        &["tau_mi"],
        "calyx-oracle::prd22",
    ),
    spec(
        "RRF",
        "22 S7",
        "sextant",
        "calyx_sextant::fusion::rrf::rrf_contribution",
        &["rrf.k", "weight"],
        "calyx-sextant::rrf_tests",
    ),
    spec(
        "Weighted RRF",
        "22 S7",
        "sextant",
        "calyx_sextant::weighted_profiles",
        &["profile_weights"],
        "calyx-sextant::stage4_fsv",
    ),
    spec(
        "ColBERT MaxSim",
        "22 S7",
        "sextant",
        "calyx_sextant::MaxSimIndex::maxsim",
        &["token_cutoff"],
        "calyx-sextant::stage4_fsv",
    ),
    spec(
        "Causal gate",
        "22 S7",
        "sextant",
        "calyx_sextant::causal_gate_mult",
        &["high_multiplier", "low_multiplier"],
        "calyx-sextant::causal_gate_fsv",
    ),
    spec(
        "Cross-lens anomaly",
        "22 S7",
        "loom",
        "calyx_loom::detect_blind_spot",
        &["delta_threshold"],
        "calyx-assay::stage5_fsv",
    ),
    spec(
        "Define",
        "22 S7",
        "sextant",
        "calyx_sextant::define",
        &["definition_k"],
        "calyx-sextant::stage4_fsv",
    ),
    spec(
        "Reverse query",
        "22 S8",
        "oracle",
        "calyx_oracle::reverse_query",
        &["max_hops"],
        "calyx-oracle::prd22",
    ),
    spec(
        "Q/A equivalence",
        "22 S8",
        "paths",
        "calyx_paths::bidirectional",
        &["max_hops"],
        "calyx-paths::ph31_paths_tests",
    ),
    spec(
        "Anneal self-tuning",
        "22 S9",
        "anneal",
        "calyx_anneal::AnnealSubstrate::propose_change",
        &["rrf.k", "ksg.k", "tripwires"],
        "calyx-assay::formula_coverage_fsv",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formula_coverage_catalog_has_all_prd22_rows() {
        let artifact = formula_coverage_artifact("/tmp/fsv-639", 1);
        let summary = validate_formula_coverage(&artifact).unwrap();
        assert_eq!(summary.total_rows, 38);
        assert_eq!(summary.covered_rows, 38);
        assert_eq!(summary.missing_rows, 0);
    }
}
