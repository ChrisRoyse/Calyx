//! Lens proposal primitives for Anneal.

pub mod admission_record;
pub mod candidate_synth;
pub mod deficit_localize;
pub mod differentiation_gate;
pub mod propose_lens;
pub mod registry_hot_add;

pub use admission_record::{
    AdmissionRecord, LensAdmittedEntry, LensRejectedEntry, ProposalHistoryReadback,
    proposal_history, proposal_history_with_refs, record_admitted, record_from_entry,
    record_outcome, record_rejected,
};
pub use candidate_synth::{
    AlgParams, AlgorithmicKind, CALYX_ANNEAL_CANDIDATE_INVALID_DEFICIT, CandidateLens,
    CommissionSpec, CorpusSampleSource, MAX_SYNTHESIS_CORPUS_SAMPLE, build_commission_spec,
    describe, synthesize, synthesize_algorithmic, synthesize_from_source,
};
pub use deficit_localize::{
    AnchorGap, AnchorId, AssayAttribution, CALYX_ANNEAL_DEFICIT_INVALID_CONFIG,
    CALYX_ASSAY_INVALID_METRIC, CALYX_ASSAY_UNAVAILABLE, DEFAULT_DEFICIT_THRESHOLD_BITS,
    DeficitLocalizer, DeficitLocalizerConfig, DeficitMap, MODALITY_COVERAGE_THRESHOLD_BITS,
    ModalityId, has_deficit, top_gap_description,
};
pub use differentiation_gate::{
    CALYX_REGISTRY_PROFILE_TIMEOUT, DIFFERENTIATION_MAX_CORR, DIFFERENTIATION_MIN_BITS,
    DifferentiationGate, GateOutcome, LensProfiler, PROFILE_TIMEOUT_MS, PairNMI, RejectReason,
    describe_gate_outcome, gate,
};
pub use propose_lens::{
    CALYX_REGISTRY_HOT_ADD_FAIL, HotAddAction, HotAddPlan, HotAddReceipt, LensHotAdder,
    ProposalOutcome, ProposalSubstrate, ProposalTerminalState, ProposeLens, ProposeLensRequest,
    propose_lens,
};
pub use registry_hot_add::RegistryHotAdder;
