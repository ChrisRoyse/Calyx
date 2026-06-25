//! Registry runtimes for frozen Calyx lenses.

pub mod backfill;
pub mod commission;
pub mod compression;
pub mod drift;
pub mod explain;
pub mod frozen;
pub mod ingest_microbatch;
pub mod lens;
pub mod measure;
pub mod panel_ops;
pub mod panels;
pub mod persistence;
pub mod placement;
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
    CommissionRequest, CommissionedLens, CommissionedLensArtifact, LensForgeFile,
    LensForgeManifest, LensForgeShape, commission_lens, lens_spec_from_manifest,
    lens_spec_from_manifest_path, lens_spec_from_manifest_with_license_override,
    lens_spec_metadata_from_manifest, lens_spec_metadata_from_manifest_path, register_commissioned,
};
pub use compression::{
    CALYX_VECTOR_COMPRESSION_EMPTY, CALYX_VECTOR_COMPRESSION_INVALID, COMPRESSED_SLOT_TAG,
    SlotCompressionReport, SlotCompressionRow, StoredSlotCodec, StoredSlotEnvelope,
    compress_slot_batch, decode_stored_slot_envelope, matryoshka_truncate_renormalize,
    write_compressed_slot_batch,
};
pub use drift::{DriftDecision, RuntimeGolden};
pub use explain::{LensExplanation, explain_lens, explain_lens_from_card};
pub use frozen::{FrozenLensContract, LensDType, NormPolicy};
pub use ingest_microbatch::{
    DEFAULT_INGEST_MICROBATCH_CAP_BYTES, INGEST_MICROBATCH_INPUT_OVERHEAD_BYTES, IngestLensOutcome,
    IngestLensOutcomeStatus, IngestMicrobatchConfig, IngestMicrobatchController,
    IngestMicrobatchPermit, IngestMicrobatchStats, IngestPanelReadout, estimate_microbatch_bytes,
};
pub use lens::{
    DeterminismProof, DualMeasurement, FrozenLensSnapshot, Registry, RegistryLensSnapshot,
    ensure_input_modality, ensure_vector_shape,
};
pub use panel_ops::{
    AppliedPanelTemplate, CALYX_PANEL_LENS_MISSING, PanelCapabilityGateOutcome, PanelDiff,
    PanelSlotListing, ResolvedPanelLens, apply_capability_gate, apply_panel_template, list_panel,
    list_panel_with_assay, swap_panel, swap_panel_to_target,
};
pub use panels::{
    AlgorithmicPanelLens, InstantiatedPanel, PanelLensRuntime, PanelSlotSpec, PanelTemplate,
    bio_default, civic_default, code_default, instantiate_panel, legal_default, media_default,
    medical_default, text_default,
};
pub use persistence::{
    VaultPanelState, VaultPanelWrite, VaultRegistrySnapshot, load_vault_panel_state,
    persist_vault_panel_state,
};
pub use placement::{
    CALYX_RAM_BUDGET_EXCEEDED, CALYX_VRAM_BUDGET_EXCEEDED, CpuLensPool, CpuPoolAdmission,
    LENS_RAM_REMEDIATION, LENS_VRAM_REMEDIATION, PlacementBudget, PlacementPlan, choose_placement,
};
pub use profile::{
    CAPABILITY_MAX_PAIRWISE_CORR_ENV, CAPABILITY_MIN_SIGNAL_BITS_ENV, CapabilityCard,
    CapabilityGateDecision, CapabilityGateEvaluation, CapabilityGateThresholds,
    CapabilitySignalKind, CapabilitySignalReliability, CostMetrics, CoverageMetrics, MetricSource,
    ProfileOptions, ProfileProbe, Profiler, SeparationMetrics, SpreadMetrics,
    append_capability_gate_ledger, apply_assay_metrics, capability_gate_json,
    evaluate_capability_gate, max_panel_pairwise_correlation, profile_lens,
    profile_slot_with_assay, signal_kind_from_runtime,
};
pub use runtime::adapters::{
    CALYX_ALLOW_NONCOMMERCIAL_LENSES_ENV, CALYX_LICENSE_DENIED, MultimodalAdapterLens,
    MultimodalAdapterProvider, MultimodalAdapterSpec, MultimodalAxis, MultimodalLensPackEntry,
    allow_noncommercial_from_env, default_multimodal_lens_specs, ensure_license_allowed,
    is_non_commercial_license, register_multimodal_lens_pack,
};
pub use runtime::algorithmic::{AlgorithmicEncoder, AlgorithmicLens};
pub use runtime::candle::{
    CandleDevicePolicy, CandleFileSpec, CandleLens, CandleModelFiles, CandlePoolingPolicy,
    CandlePrecision, DEFAULT_CANDLE_MODEL,
};
pub use runtime::external_cmd::ExternalCmdLens;
pub use runtime::onnx::{
    DEFAULT_ANSWERAI_COLBERT_MODEL, FastembedBgem3Lens, FastembedRerankerLens, FastembedSparseLens,
    OnnxColbertFileSpec, OnnxColbertLens, OnnxFileSpec, OnnxLens, OnnxModelFiles,
    OnnxProviderPolicy, PoolingPolicy,
};
pub use runtime::qwen3::{
    DEFAULT_QWEN3_MAX_TOKENS, DEFAULT_QWEN3_MODEL, FastembedQwen3Lens, Qwen3FileSpec,
    Qwen3ModelFiles,
};
pub use runtime::static_lookup::{
    StaticLookupDType, StaticLookupFileSpec, StaticLookupFiles, StaticLookupLens,
};
pub use runtime::tei_http::{DEFAULT_TEI_ENDPOINT, TeiHttpLens};
pub use spec::{FastembedBgem3Output, LensHealth, LensRuntime, LensSpec};
pub use swap::{BackfillCandidate, BackfillQueue, SlotSpec, SwapController};
pub use temporal::{
    DecayFunction, E2RecencyConfig, E2RecencyLens, E3PeriodicConfig, E3PeriodicLens,
    E4PositionalConfig, E4PositionalLens, MultiAnchorMode, PeriodicOptions, SequenceDirection,
    SequenceOptions, TEMPORAL_FLAGS, TemporalLensFlags,
};
