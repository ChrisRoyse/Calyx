//! Assay signal-bit measurement, panel sufficiency, and persistence contracts.

pub mod attribution;
pub mod bootstrap;
pub mod contract;
pub mod estimate;
pub mod gate;
pub mod ksg;
pub mod logistic;
pub mod n_eff;
pub mod nmi;
pub mod projection;
pub mod store;
pub mod stratified;
pub mod sufficiency;

pub use attribution::{BitsReport, SlotAttribution, bits_report, per_sensor_attribution};
pub use bootstrap::{BootstrapCi, bootstrap_mean_ci};
pub use contract::{AdmissionDecision, admit_lens, admit_lens_with_strata};
pub use estimate::{EstimatorKind, MiEstimate, TrustTag};
pub use gate::{AssayGate, LensSignal, PairGain};
pub use ksg::{MIN_ASSAY_SAMPLES, ksg_mi_continuous, ksg_mi_continuous_discrete};
pub use logistic::{LogisticProbeReport, logistic_probe_mi};
pub use n_eff::{NeffReport, stable_rank};
pub use nmi::{NmiReport, partitioned_histogram_nmi};
pub use projection::{ProjectionReport, project_cpu, project_gpu, target_projection_dim};
pub use store::{AssayCacheKey, AssayRow, AssayStore, AssaySubject};
pub use stratified::{StratifiedBits, StratumBits, stratified_bits};
pub use sufficiency::{
    DeficitRoutingContext, DeficitSuggestedAction, InMemoryDeficitSink, PanelSufficiency,
    SufficiencyDeficit, SufficiencyDeficitSink, entropy_bits, panel_sufficiency,
    panel_sufficiency_with_context,
};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-assay");
    }
}
