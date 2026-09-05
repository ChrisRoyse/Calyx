use std::path::PathBuf;

use calyx_core::{LensCost, LensId, Modality, Placement, SlotShape};
use calyx_registry::{FrozenLensContract, LensForgeManifest, LensHealth, LensSpec};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(in crate::panel_commands) struct PanelTemplateCatalog {
    pub schema_version: u16,
    pub templates: Vec<PanelTemplateIndexEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(in crate::panel_commands) struct PanelTemplateIndexEntry {
    pub name: String,
    pub active_template_id: String,
    pub versions: Vec<PanelTemplateVersionRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(in crate::panel_commands) struct PanelTemplateVersionRef {
    pub version: u32,
    pub template_id: String,
    pub object_path: String,
    pub blake3_hex: String,
    pub size_bytes: u64,
    pub saved_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(in crate::panel_commands) struct SavedPanelTemplate {
    pub schema_version: u16,
    pub name: String,
    pub version: u32,
    pub notes: String,
    pub min_content_lenses: usize,
    pub lenses: Vec<TemplateLensRef>,
    pub time_controls: Vec<TemplateTimeControl>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ensemble_card: Option<TemplateEnsembleCard>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(in crate::panel_commands) struct TemplateLensRef {
    pub slot_key: String,
    pub lens_name: String,
    pub lens_id: LensId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_lens_id: Option<LensId>,
    pub weights_sha256: String,
    pub runtime: String,
    pub modality: Modality,
    pub shape: SlotShape,
    pub placement: Placement,
    pub cost: LensCost,
    pub manifest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub immutable_snapshot: Option<TemplateLensSnapshot>,
    pub counts_toward_a35: bool,
}

/// Self-contained, content-addressed deployment input for one template lens.
///
/// The source manifest path remains on `TemplateLensRef` for audit display only.
/// Materialization uses this embedded manifest/spec/contract snapshot and the
/// frozen artifact base, then re-hashes every manifest artifact before loading a
/// runtime. A mutable commissioned-manifest alias can therefore never change a
/// saved template's meaning.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(in crate::panel_commands) struct TemplateLensSnapshot {
    pub schema_version: u16,
    pub manifest: LensForgeManifest,
    pub manifest_base_dir: PathBuf,
    pub manifest_blake3: String,
    pub spec: LensSpec,
    pub spec_blake3: String,
    pub runtime_contract: FrozenLensContract,
    pub runtime_contract_blake3: String,
    /// Lens-identity derivation version that produced this snapshot's ids.
    ///
    /// Sidecar only, never a hash input (see `calyx_registry::derivation`).
    /// `None` marks a template published before derivation stamping existed.
    /// Skipped when absent so pre-stamp templates reserialize byte-identically
    /// and their `spec_blake3` / `runtime_contract_blake3` stay valid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derivation_version: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(in crate::panel_commands) struct TemplateTimeControl {
    pub slot_key: String,
    pub kind: String,
    pub shape: SlotShape,
    pub purpose: String,
    pub counts_toward_a35: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(in crate::panel_commands) struct TemplateEnsembleCard {
    pub schema_version: u16,
    pub source: String,
    pub content_lens_count: usize,
    pub measured_lens_count: usize,
    pub all_loaded: bool,
    pub min_coverage_rate: f32,
    pub total_vram_bytes: u64,
    pub total_ram_bytes: u64,
    pub mean_ms_per_input: f32,
    pub card_refs: Vec<CapabilityCardRef>,
    #[serde(default)]
    pub a37_admission: TemplateA37Admission,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a37_ensemble_card_ref: Option<TemplateA37CardRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a37_admission_card_ref: Option<TemplateA37CardRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(in crate::panel_commands) struct CapabilityCardRef {
    pub path: String,
    pub blake3_hex: String,
    pub lens_id: LensId,
    pub probe_count: usize,
    pub coverage_rate: f32,
    pub failed: usize,
    pub health: LensHealth,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(in crate::panel_commands) struct TemplateA37Admission {
    pub schema_version: u16,
    pub source: String,
    pub gate_eligible: bool,
    pub status: String,
    pub verdict: String,
    pub content_lens_count: usize,
    pub temporal_sidecar_count: usize,
    pub temporal_counts_toward_content_floor: bool,
    pub association_family_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n_eff: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_pairwise_corr: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_pairwise_nmi: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sum_unique_pid_bits: Option<f32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(in crate::panel_commands) struct TemplateA37CardRef {
    pub path: String,
    pub blake3_hex: String,
    pub card_schema_version: u32,
    pub card_source: String,
    pub panel_lens_count: usize,
    pub status: String,
}

#[derive(Clone, Debug)]
pub(in crate::panel_commands) struct TemplateDraft {
    pub name: String,
    pub notes: String,
    pub lenses: Vec<TemplateLensRef>,
    pub ensemble_card: Option<TemplateEnsembleCard>,
}
