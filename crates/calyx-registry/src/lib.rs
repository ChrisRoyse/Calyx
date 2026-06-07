//! Registry runtimes for frozen Calyx lenses.

pub mod frozen;
pub mod lens;
pub mod panels;
pub mod profile;
pub mod runtime;
pub mod swap;
pub mod temporal;

pub use calyx_core::{Input, Lens};
pub use frozen::{FrozenLensContract, LensDType, NormPolicy};
pub use lens::{Registry, ensure_input_modality, ensure_vector_shape};
pub use panels::{
    AlgorithmicPanelLens, InstantiatedPanel, PanelLensRuntime, PanelSlotSpec, PanelTemplate,
    civic_default, code_default, instantiate_panel, media_default, text_default,
};
pub use profile::{
    CapabilityCard, CostMetrics, CoverageMetrics, MetricSource, ProfileOptions, ProfileProbe,
    Profiler, SeparationMetrics, SpreadMetrics, profile_lens,
};
pub use runtime::algorithmic::{AlgorithmicEncoder, AlgorithmicLens};
pub use runtime::candle::{CandleLens, CandleModelFiles, DEFAULT_CANDLE_MODEL};
pub use runtime::onnx::{OnnxLens, OnnxModelFiles};
pub use runtime::tei_http::{DEFAULT_TEI_ENDPOINT, TeiHttpLens};
pub use swap::{BackfillCandidate, BackfillQueue, SlotSpec, SwapController};
pub use temporal::{
    DecayFunction, E2RecencyConfig, E2RecencyLens, E3PeriodicConfig, E3PeriodicLens,
    E4PositionalConfig, E4PositionalLens, MultiAnchorMode, PeriodicOptions, SequenceDirection,
    SequenceOptions, TEMPORAL_FLAGS, TemporalLensFlags,
};
