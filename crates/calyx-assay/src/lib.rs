//! Assay signal-bit measurement, panel sufficiency, and persistence contracts.

pub mod attribution;
pub mod bayesian;
pub mod bootstrap;
pub mod calibration;
pub mod contract;
pub mod ensemble;
pub mod estimate;
pub mod formula_catalog;
pub mod formulas;
pub mod gate;
pub mod group_split;
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
pub mod resource_contract;
mod samples;
mod special_fn;
pub mod store;
pub mod stratified;
pub mod sufficiency;
pub mod total_correlation;
pub mod transfer_entropy;

pub use attribution::{
    BitsReport, CALYX_ASSAY_INVALID_COVERAGE, CoverageMask, SlotAttribution, bits_report,
    bits_report_with_anchor, per_sensor_attribution, per_sensor_attribution_with_coverage,
};
pub use bayesian::{
    BAYESIAN_POSTERIOR_KEY_PREFIX, BayesianPosteriorRow, BetaBernoulli,
    CALYX_BAYES_INVALID_INTERVAL, DEFAULT_BAYES_PRIOR_ALPHA, DEFAULT_BAYES_PRIOR_BETA,
    GammaPoisson, bayesian_posterior_for_domain, bayesian_posterior_key, beta_bernoulli_for_domain,
    gamma_poisson_for_domain, persist_bayesian_posterior,
};
pub use bootstrap::{
    BootstrapCi, BootstrapConfig, DEFAULT_BOOTSTRAP_RESAMPLES, DEFAULT_BOOTSTRAP_SEED,
    bootstrap_mean_ci, bootstrap_mean_ci_with_config, bootstrap_paired_ci,
};
pub use calibration::{
    CALYX_ASSAY_DEGENERATE_TARGET_ENTROPY, CALYX_ASSAY_ESTIMATOR_UNDERPOWERED,
    DEFAULT_MIN_POWER_RECOVERY_RATIO, MIN_INFORMATIVE_TARGET_ENTROPY_BITS, PowerCalibration,
    PowerCalibrationStatus, ensure_informative_binary_labels,
};
pub use contract::{
    AdmissionDecision, CALYX_ASSAY_UNRESOLVED, CorrelationEvidence, admit_lens,
    admit_lens_estimate, admit_lens_estimate_with_signal_kind, admit_lens_with_strata,
};
pub use ensemble::{
    A37_DIVERSITY_DIAGNOSTIC_ONLY, A37_DIVERSITY_GATE_PASSED, A37_DIVERSITY_SCHEMA_VERSION,
    A37DiversityGate, CALYX_ASSAY_PANEL_TOO_SMALL, DEFAULT_GATE_PANEL_LENSES,
    DEFAULT_MAX_REDUNDANCY, DEFAULT_MIN_MARGINAL_BITS, DeficitProposal, ENSEMBLE_CARD_PID_METHOD,
    ENSEMBLE_CARD_SCHEMA_VERSION, EnsembleCard, EnsembleConfig, EnsembleDecision,
    EnsembleLensInput, EnsembleLensValue, EnsemblePairValue, MIN_ENSEMBLE_PANEL_LENSES, PidBits,
    a37_association_family, a37_diversity_gate, ensemble_card,
};
pub use estimate::{
    EstimateBound, EstimateReliability, EstimatorKind, MiEstimate, TrustTag,
    require_grounded_anchor, trust_for_anchor,
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
pub use group_split::{GroupSplit, group_holdout_split, row_groups};
pub use ksg::{
    MIN_ASSAY_SAMPLES, ksg_mi_continuous, ksg_mi_continuous_discrete,
    ksg_mi_continuous_discrete_with_anchor, ksg_mi_continuous_with_anchor,
};
pub use logistic::{
    DEFAULT_ASSAY_SEEDS, DEFAULT_HOLDOUT_FRACTION, LogisticProbeReport, logistic_probe_mi,
    logistic_probe_mi_calibrated, logistic_probe_mi_multiseed,
    logistic_probe_mi_multiseed_calibrated, logistic_probe_mi_multiseed_calibrated_with_anchor,
    logistic_probe_mi_multiseed_with_anchor, logistic_probe_mi_with_anchor,
};
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
pub use resource_contract::{
    CALYX_ASSAY_INVALID_RESOURCE, CALYX_ASSAY_RESOURCE_BUDGET_EXCEEDED, PanelAdmissionCandidate,
    PanelLensDecision, PanelPackingReport, PanelResourceBudget, ResourceAwareAdmissionDecision,
    ResourceDensity, ResourceUsage, admit_lens_with_resources, admit_lens_with_usage,
    pack_panel_by_density,
};
pub use store::{AssayCacheKey, AssayRow, AssayStore, AssaySubject};
pub use stratified::{StratifiedBits, StratumBits, stratified_bits};
pub use sufficiency::{
    CALYX_ASSAY_INVALID_SCOPE, DeficitRoutingContext, DeficitSuggestedAction, InMemoryDeficitSink,
    ObservationScope, PanelSufficiency, ScopedSufficiencyReport, SufficiencyDeficit,
    SufficiencyDeficitSink, SufficiencyScopeInput, entropy_bits, panel_sufficiency,
    panel_sufficiency_by_scope, panel_sufficiency_from_estimate, panel_sufficiency_with_anchor,
    panel_sufficiency_with_anchor_and_context, panel_sufficiency_with_context,
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
