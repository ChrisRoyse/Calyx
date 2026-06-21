use std::collections::BTreeSet;

use calyx_core::{
    Asymmetry, CalyxError, LensCost, LensId, Modality, Panel, Placement, QuantPolicy, Slot, SlotId,
    SlotKey, SlotResource, SlotShape, SlotState, content_address,
};
use calyx_registry::{LensHealth, lens_spec_metadata_from_manifest_path};
use serde::{Deserialize, Serialize};

use crate::error::{CliError, CliResult};
use crate::lens_commands::support::runtime_name;

pub(super) const MIN_CONTENT_LENSES: usize = 10;
pub(super) const CATALOG_VERSION: u16 = 1;
pub(super) const OBJECT_VERSION: u16 = 1;
pub(super) const CARD_VERSION: u16 = 1;
pub(super) const A37_ADMISSION_VERSION: u16 = 1;
pub(super) const TEMPLATE_INVALID: &str = "CALYX_PANEL_TEMPLATE_INVALID";
pub(super) const TEMPLATE_NOT_FOUND: &str = "CALYX_PANEL_TEMPLATE_NOT_FOUND";
pub(super) const TEMPLATE_A37_GATE_REFUSED: &str = "CALYX_PANEL_TEMPLATE_A37_GATE_REFUSED";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct PanelTemplateCatalog {
    pub schema_version: u16,
    pub templates: Vec<PanelTemplateIndexEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct PanelTemplateIndexEntry {
    pub name: String,
    pub active_template_id: String,
    pub versions: Vec<PanelTemplateVersionRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct PanelTemplateVersionRef {
    pub version: u32,
    pub template_id: String,
    pub object_path: String,
    pub blake3_hex: String,
    pub size_bytes: u64,
    pub saved_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct SavedPanelTemplate {
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
pub(super) struct TemplateLensRef {
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
    pub counts_toward_a35: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct TemplateTimeControl {
    pub slot_key: String,
    pub kind: String,
    pub shape: SlotShape,
    pub purpose: String,
    pub counts_toward_a35: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct TemplateEnsembleCard {
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
pub(super) struct CapabilityCardRef {
    pub path: String,
    pub blake3_hex: String,
    pub lens_id: LensId,
    pub probe_count: usize,
    pub coverage_rate: f32,
    pub failed: usize,
    pub health: LensHealth,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct TemplateA37Admission {
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
pub(super) struct TemplateA37CardRef {
    pub path: String,
    pub blake3_hex: String,
    pub card_schema_version: u32,
    pub card_source: String,
    pub panel_lens_count: usize,
    pub status: String,
}

#[derive(Clone, Debug)]
pub(super) struct TemplateDraft {
    pub name: String,
    pub notes: String,
    pub lenses: Vec<TemplateLensRef>,
    pub ensemble_card: Option<TemplateEnsembleCard>,
}

impl SavedPanelTemplate {
    pub(super) fn content_lens_count(&self) -> usize {
        self.lenses
            .iter()
            .filter(|lens| lens.counts_toward_a35)
            .count()
    }

    pub(super) fn validate(&self) -> CliResult {
        if self.schema_version != OBJECT_VERSION {
            return Err(template_error(
                TEMPLATE_INVALID,
                format!("unsupported panel template object {}", self.schema_version),
                "migrate the panel template object through a compatible reader",
            ));
        }
        if self.name.trim().is_empty() || self.name.contains(['/', '\\']) {
            return Err(template_error(
                TEMPLATE_INVALID,
                "panel template name must be non-empty and path-safe",
                "choose a stable template name such as text-deep",
            ));
        }
        if self.content_lens_count() < MIN_CONTENT_LENSES {
            return Err(template_error(
                TEMPLATE_INVALID,
                format!(
                    "panel template {} has {} content lenses; minimum is {MIN_CONTENT_LENSES}",
                    self.name,
                    self.content_lens_count()
                ),
                "add real frozen content lenses until the template has at least ten",
            ));
        }
        validate_lenses(self)?;
        validate_time_controls(self)
    }

    pub(super) fn a37_admission(&self) -> TemplateA37Admission {
        self.ensemble_card
            .as_ref()
            .map(|card| card.a37_admission.clone())
            .unwrap_or_default()
    }

    pub(super) fn a37_gate_eligible(&self) -> bool {
        self.a37_admission().gate_eligible
    }

    pub(super) fn require_a37_gate(&self) -> CliResult {
        let admission = self.a37_admission();
        if admission.gate_eligible {
            return Ok(());
        }
        Err(template_error(
            TEMPLATE_A37_GATE_REFUSED,
            format!(
                "template {} is not A37 gate eligible: {}",
                self.name, admission.verdict
            ),
            "profile the template with an Assay EnsembleCard whose A37 status is gate_passed",
        ))
    }

    pub(super) fn to_target_panel(&self, created_at: u64) -> Panel {
        let mut slots = Vec::with_capacity(self.lenses.len() + self.time_controls.len());
        for lens in &self.lenses {
            let slot_id = SlotId::new(slots.len() as u16);
            slots.push(Slot {
                slot_id,
                slot_key: SlotKey::new(slot_id, lens.slot_key.clone()),
                lens_id: lens.runtime_lens_id.unwrap_or(lens.lens_id),
                shape: lens.shape,
                modality: lens.modality,
                asymmetry: Asymmetry::None,
                quant: QuantPolicy::turboquant_default(),
                resource: SlotResource {
                    cost: lens.cost,
                    placement: lens.placement,
                },
                axis: Some(lens.slot_key.clone()),
                retrieval_only: false,
                excluded_from_dedup: false,
                bits_about: Default::default(),
                state: SlotState::Active,
                added_at_panel_version: (slots.len() + 1) as u32,
            });
        }
        for control in &self.time_controls {
            let slot_id = SlotId::new(slots.len() as u16);
            slots.push(Slot {
                slot_id,
                slot_key: SlotKey::new(slot_id, control.slot_key.clone()),
                lens_id: time_control_id(&self.name, control),
                shape: control.shape,
                modality: Modality::Structured,
                asymmetry: Asymmetry::None,
                quant: QuantPolicy::None,
                resource: SlotResource::default(),
                axis: Some(control.slot_key.clone()),
                retrieval_only: true,
                excluded_from_dedup: true,
                bits_about: Default::default(),
                state: SlotState::Active,
                added_at_panel_version: (slots.len() + 1) as u32,
            });
        }
        Panel {
            version: slots.len() as u32,
            slots,
            created_at,
            kernel_ref: None,
            guard_ref: None,
        }
    }
}

impl Default for TemplateA37Admission {
    fn default() -> Self {
        Self {
            schema_version: A37_ADMISSION_VERSION,
            source: "missing_assay_ensemble_card".to_string(),
            gate_eligible: false,
            status: "missing_a37_ensemble_card".to_string(),
            verdict: "A37 gate not evaluated; template has no Assay EnsembleCard".to_string(),
            content_lens_count: 0,
            temporal_sidecar_count: 0,
            temporal_counts_toward_content_floor: false,
            association_family_count: 0,
            n_eff: None,
            mean_pairwise_corr: None,
            mean_pairwise_nmi: None,
            sum_unique_pid_bits: None,
        }
    }
}

pub(super) fn default_time_controls() -> Vec<TemplateTimeControl> {
    vec![
        time_control("E2_recency", "temporal_recent", SlotShape::Dense(1)),
        time_control("E3_periodic", "temporal_periodic", SlotShape::Dense(2)),
        time_control("E4_positional", "temporal_positional", SlotShape::Dense(4)),
    ]
}

pub(super) fn lens_ref_from_catalog(entry: &super::LensCatalogEntry) -> CliResult<TemplateLensRef> {
    let spec = lens_spec_metadata_from_manifest_path(&entry.manifest)?;
    Ok(TemplateLensRef {
        slot_key: slug(&entry.name),
        lens_name: entry.name.clone(),
        lens_id: entry
            .lens_id
            .parse()
            .map_err(|err| CliError::usage(format!("parse lens_id {}: {err}", entry.lens_id)))?,
        runtime_lens_id: None,
        weights_sha256: entry.weights_sha256.clone(),
        runtime: runtime_name(&spec.runtime).to_string(),
        modality: spec.modality,
        shape: spec.output,
        placement: entry.placement,
        cost: entry.cost,
        manifest: entry.manifest.display().to_string(),
        counts_toward_a35: true,
    })
}

pub(super) fn template_error(
    code: &'static str,
    message: impl Into<String>,
    remediation: &'static str,
) -> CliError {
    CliError::from(CalyxError {
        code,
        message: message.into(),
        remediation,
    })
}

fn validate_lenses(template: &SavedPanelTemplate) -> CliResult {
    let mut ids = BTreeSet::new();
    let mut runtime_ids = BTreeSet::new();
    for lens in &template.lenses {
        if !ids.insert(lens.lens_id) {
            return Err(template_error(
                TEMPLATE_INVALID,
                format!("template {} repeats lens {}", template.name, lens.lens_id),
                "remove duplicate lens ids from the template",
            ));
        }
        if let Some(runtime_lens_id) = lens.runtime_lens_id
            && !runtime_ids.insert(runtime_lens_id)
        {
            return Err(template_error(
                TEMPLATE_INVALID,
                format!(
                    "template {} repeats runtime lens {}",
                    template.name, runtime_lens_id
                ),
                "remove duplicate runtime lens ids from the template",
            ));
        }
        validate_weight_hash(&lens.weights_sha256)?;
        if !lens.counts_toward_a35 {
            return Err(template_error(
                TEMPLATE_INVALID,
                format!("template {} has a non-counting content lens", template.name),
                "store non-content time controls in time_controls, not lenses",
            ));
        }
    }
    Ok(())
}

fn validate_time_controls(template: &SavedPanelTemplate) -> CliResult {
    for control in &template.time_controls {
        if control.counts_toward_a35 {
            return Err(template_error(
                TEMPLATE_INVALID,
                format!("time control {} counts toward A35", control.slot_key),
                "temporal/time capture is a control sidecar and must not count as an embedder",
            ));
        }
    }
    Ok(())
}

fn validate_weight_hash(value: &str) -> CliResult {
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(());
    }
    Err(template_error(
        TEMPLATE_INVALID,
        format!("weights_sha256 must be 64 hex chars, got {value}"),
        "rebuild the template from frozen lens manifests",
    ))
}

fn time_control(slot_key: &str, kind: &str, shape: SlotShape) -> TemplateTimeControl {
    TemplateTimeControl {
        slot_key: slot_key.to_string(),
        kind: kind.to_string(),
        shape,
        purpose: "walk_forward_backward_as_of_time_control".to_string(),
        counts_toward_a35: false,
    }
}

fn time_control_id(template: &str, control: &TemplateTimeControl) -> LensId {
    LensId::from_bytes(content_address([
        b"panel-template-time-control-v1".as_slice(),
        template.as_bytes(),
        control.slot_key.as_bytes(),
        control.kind.as_bytes(),
    ]))
}

fn slug(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

#[cfg(test)]
mod tests;
