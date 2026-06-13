//! Assay signal-bit measurement, panel sufficiency, and persistence contracts.

pub mod attribution;
pub mod bootstrap;
pub mod contract;
pub mod estimate;
pub mod formula_catalog;
pub mod formulas;
pub mod gate;
pub mod ksg;
pub mod logistic;
pub mod loom_adapter;
pub mod mmd;
pub mod n_eff;
pub mod nmi;
pub mod periodicity;
pub mod projection;
pub mod recurrence_anchor;
pub mod recurrence_hazard;
mod samples;
mod special_fn;
pub mod store;
pub mod stratified;
pub mod sufficiency;
pub mod total_correlation;
pub mod transfer_entropy;

pub use attribution::{
    BitsReport, SlotAttribution, bits_report, bits_report_with_anchor, per_sensor_attribution,
};
pub use bootstrap::{
    BootstrapCi, BootstrapConfig, DEFAULT_BOOTSTRAP_RESAMPLES, DEFAULT_BOOTSTRAP_SEED,
    bootstrap_mean_ci, bootstrap_mean_ci_with_config, bootstrap_paired_ci,
};
pub use contract::{AdmissionDecision, admit_lens, admit_lens_with_strata};
pub use estimate::{
    EstimatorKind, MiEstimate, TrustTag, require_grounded_anchor, trust_for_anchor,
};
pub use formula_catalog::{
    CALYX_FORMULA_COVERAGE_MISSING, FORMULA_COVERAGE_ARTIFACT_KIND,
    FORMULA_COVERAGE_SCHEMA_VERSION, FORMULA_COVERAGE_SOT_KEY, FORMULA_COVERAGE_SURFACE,
    FormulaCoverageArtifact, FormulaCoverageRow, FormulaCoverageStatus, FormulaCoverageSummary,
    FormulaRowSpec, formula_coverage_artifact, formula_coverage_json, prd22_formula_specs,
    validate_formula_coverage,
};
pub use formulas::{dpi_ceiling, lens_signal, marginal_value, pair_redundancy};
pub use gate::{AssayGate, LensSignal, PairGain};
pub use ksg::{
    MIN_ASSAY_SAMPLES, ksg_mi_continuous, ksg_mi_continuous_discrete,
    ksg_mi_continuous_discrete_with_anchor, ksg_mi_continuous_with_anchor,
};
pub use logistic::{LogisticProbeReport, logistic_probe_mi, logistic_probe_mi_with_anchor};
pub use loom_adapter::AsterAssayMaterializationGate;
pub use mmd::{
    ChangePointReport, DEFAULT_MMD_ALPHA, DEFAULT_MMD_PERMUTATIONS, DEFAULT_MMD_SEED, MmdConfig,
    MmdReport, gaussian_mmd, gaussian_mmd_with_config, mmd_change_point,
};
pub use n_eff::{NeffReport, stable_rank};
pub use nmi::{NmiReport, partitioned_histogram_nmi};
pub use periodicity::{
    AutocorrelationReport, DEFAULT_FAP_PERMUTATIONS, DEFAULT_MAX_PEAKS, DEFAULT_PERIODICITY_SEED,
    DEFAULT_PERIODOGRAM_OVERSAMPLE, MAX_ACF_SAMPLES, MAX_FREQUENCY_GRID, MIN_PERIODICITY_SAMPLES,
    PeriodicityReport, PeriodogramConfig, PeriodogramPeak, SIGNIFICANT_PEAK_FAP, autocorrelation,
    bin_event_counts, lomb_scargle, lomb_scargle_with_anchor, lomb_scargle_with_config,
};
pub use projection::{ProjectionReport, project_cpu, project_gpu, target_projection_dim};
pub use recurrence_anchor::{
    CALYX_ASSAY_MISSING_OUTCOME_SLOT, CONSISTENT_AGREEMENT_THRESHOLD, DEFAULT_OUTCOME_ANCHOR_LABEL,
    Domain, OutcomeAgreement, RecurrenceAnchor, default_outcome_anchor, frequency_anchor_for,
    measure_outcome_agreement, measure_outcome_agreement_for, oracle_self_consistency,
    oracle_self_consistency_from_agreements, outcome_agreement_from_observations,
    outcome_occurrence_context,
};
pub use recurrence_hazard::{
    CV_DETERMINISTIC, CusumChangePoint, CusumConfig, CusumReport, DEFAULT_CUSUM_SLACK_K,
    DEFAULT_CUSUM_THRESHOLD_H, DEFAULT_MIN_SIGMA_FRAC, DEFAULT_OVERDUE_ALPHA,
    InterEventHazardReport, MIN_CUSUM_GAPS, MIN_HAZARD_GAPS, RateShift, inter_event_hazard,
    inter_event_hazard_from_series, inter_event_hazard_with_alpha, recurrence_rate_cusum,
    recurrence_rate_cusum_from_series, recurrence_rate_cusum_with_config,
};
pub use store::{AssayCacheKey, AssayRow, AssayStore, AssaySubject};
pub use stratified::{StratifiedBits, StratumBits, stratified_bits};
pub use sufficiency::{
    DeficitRoutingContext, DeficitSuggestedAction, InMemoryDeficitSink, PanelSufficiency,
    SufficiencyDeficit, SufficiencyDeficitSink, entropy_bits, panel_sufficiency,
    panel_sufficiency_with_anchor, panel_sufficiency_with_anchor_and_context,
    panel_sufficiency_with_context,
};
pub use total_correlation::{
    CALYX_TC_INSUFFICIENT_SAMPLES, DEFAULT_TC_BOOTSTRAP_RESAMPLES, DEFAULT_TC_K, IIResult, IISign,
    MIN_QUORUM_TC_PER_SLOT, SlotVectors, TCResult, TotalCorrelationConfig, interaction_information,
    interaction_information_with_config, min_quorum_tc, n_eff_from_tc, total_correlation,
    total_correlation_with_config,
};
pub use transfer_entropy::{
    CALYX_TE_INSUFFICIENT_SAMPLES, DEFAULT_TE_BOOTSTRAP_RESAMPLES, DEFAULT_TE_BOOTSTRAP_SEED,
    DEFAULT_TE_K, DEFAULT_TE_LAGS, DEFAULT_TE_WINDOW, Direction, MIN_TE_QUORUM, RecurrenceStream,
    TEResult, Timestamp, TransferEntropyConfig, max_transfer_entropy_lag, transfer_entropy,
    transfer_entropy_sweep, transfer_entropy_sweep_with_config, transfer_entropy_with_config,
};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-assay");
    }
}
