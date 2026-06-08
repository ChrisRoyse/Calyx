//! Registry runtimes for frozen Calyx lenses.

pub mod backfill;
pub mod commission;
pub mod drift;
pub mod explain;
pub mod frozen;
pub mod lens;
pub mod panel_ops;
pub mod panels;
pub mod profile;
pub mod runtime;
pub mod spec;
pub mod swap;
pub mod temporal;

pub use backfill::{
    BackfillBatch, BackfillConfig, BackfillPriority, BackfillRequest, BackfillScheduler,
    BackfillWatermark,
};
pub use calyx_core::{Input, Lens};
pub use commission::{
    CommissionRequest, CommissionedLens, CommissionedLensArtifact, commission_lens,
    register_commissioned,
};
pub use drift::{DriftDecision, RuntimeGolden};
pub use explain::{LensExplanation, explain_lens, explain_lens_from_card};
pub use frozen::{FrozenLensContract, LensDType, NormPolicy};
pub use lens::{DualMeasurement, Registry, ensure_input_modality, ensure_vector_shape};
pub use panel_ops::{PanelDiff, PanelSlotListing, list_panel, swap_panel};
pub use panels::{
    AlgorithmicPanelLens, InstantiatedPanel, PanelLensRuntime, PanelSlotSpec, PanelTemplate,
    civic_default, code_default, instantiate_panel, media_default, text_default,
};
pub use profile::{
    CapabilityCard, CostMetrics, CoverageMetrics, MetricSource, ProfileOptions, ProfileProbe,
    Profiler, SeparationMetrics, SpreadMetrics, profile_lens,
};
pub use runtime::algorithmic::{AlgorithmicEncoder, AlgorithmicLens};
pub use runtime::candle::{CandleDevicePolicy, CandleLens, CandleModelFiles, DEFAULT_CANDLE_MODEL};
pub use runtime::external_cmd::ExternalCmdLens;
pub use runtime::onnx::{OnnxLens, OnnxModelFiles, OnnxProviderPolicy};
pub use runtime::tei_http::{DEFAULT_TEI_ENDPOINT, TeiHttpLens};
pub use spec::{LensHealth, LensRuntime, LensSpec};
pub use swap::{BackfillCandidate, BackfillQueue, SlotSpec, SwapController};
pub use temporal::{
    DecayFunction, E2RecencyConfig, E2RecencyLens, E3PeriodicConfig, E3PeriodicLens,
    E4PositionalConfig, E4PositionalLens, MultiAnchorMode, PeriodicOptions, SequenceDirection,
    SequenceOptions, TEMPORAL_FLAGS, TemporalLensFlags,
};
