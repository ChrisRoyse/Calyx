//! Loom DDA cross-term and agreement-graph engine.

pub mod abundance;
pub mod agreement_graph;
pub mod blind_spot;
pub mod cross_term;
pub mod lru_cache;
pub mod materialization;

pub use abundance::{AbundanceReport, CeilingEstimate, NeffEstimate};
pub use agreement_graph::{AgreementEdge, LoomStore};
pub use blind_spot::{BlindSpotAlert, Severity, detect_blind_spot};
pub use cross_term::{
    CrossTermKey, CrossTermKind, CrossTermValue, SignalProvenanceTag, agreement_batch_cpu,
    agreement_batch_gpu, agreement_scalar, concat_vec, delta_vec, interaction_vec,
};
pub use lru_cache::LruCache;
pub use materialization::{
    MaterializationAction, MaterializationPlan, PairGainGate, StaticPairGainGate, plan_cross_terms,
};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-loom");
    }
}
