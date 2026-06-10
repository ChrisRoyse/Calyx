//! Assay signal-bit measurement, panel sufficiency, and persistence contracts.

pub mod attribution;
pub mod bootstrap;
pub mod contract;
pub mod estimate;
pub mod gate;
pub mod ksg;
pub mod logistic;
pub mod loom_adapter;
pub mod n_eff;
pub mod nmi;
pub mod projection;
pub mod recurrence_anchor;
mod samples;
pub mod store;
pub mod stratified;
pub mod sufficiency;

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
pub use gate::{AssayGate, LensSignal, PairGain};
pub use ksg::{
    MIN_ASSAY_SAMPLES, ksg_mi_continuous, ksg_mi_continuous_discrete,
    ksg_mi_continuous_discrete_with_anchor, ksg_mi_continuous_with_anchor,
};
pub use logistic::{LogisticProbeReport, logistic_probe_mi, logistic_probe_mi_with_anchor};
pub use loom_adapter::AsterAssayMaterializationGate;
pub use n_eff::{NeffReport, stable_rank};
pub use nmi::{NmiReport, partitioned_histogram_nmi};
pub use projection::{ProjectionReport, project_cpu, project_gpu, target_projection_dim};
pub use recurrence_anchor::{
    CALYX_ASSAY_MISSING_OUTCOME_SLOT, CONSISTENT_AGREEMENT_THRESHOLD, DEFAULT_OUTCOME_ANCHOR_LABEL,
    Domain, OutcomeAgreement, RecurrenceAnchor, default_outcome_anchor, frequency_anchor_for,
    measure_outcome_agreement, measure_outcome_agreement_for, oracle_self_consistency,
    oracle_self_consistency_from_agreements, outcome_agreement_from_observations,
    outcome_occurrence_context,
};
pub use store::{AssayCacheKey, AssayRow, AssayStore, AssaySubject};
pub use stratified::{StratifiedBits, StratumBits, stratified_bits};
pub use sufficiency::{
    DeficitRoutingContext, DeficitSuggestedAction, InMemoryDeficitSink, PanelSufficiency,
    SufficiencyDeficit, SufficiencyDeficitSink, entropy_bits, panel_sufficiency,
    panel_sufficiency_with_anchor, panel_sufficiency_with_anchor_and_context,
    panel_sufficiency_with_context,
};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-assay");
    }
}
