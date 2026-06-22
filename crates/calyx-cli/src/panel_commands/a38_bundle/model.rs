use std::collections::BTreeMap;

use calyx_core::{LensCost, Placement};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    A38_BUNDLE_BASE_A37_REFUSED, A38_BUNDLE_BUDGET_EXCEEDED, A38_BUNDLE_INCOMPLETE,
    A38_BUNDLE_INVALID, bundle_error,
};
use crate::error::{CliError, CliResult};
use crate::panel_commands::template_store::SavedPanelTemplate;

pub(super) const BUNDLE_CATALOG_VERSION: u16 = 1;
pub(super) const BUNDLE_OBJECT_VERSION: u16 = 1;
pub(super) const A38_COVERAGE_STATUS: &str = "a38_coverage_passed";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct A38BundleCatalog {
    pub schema_version: u16,
    pub bundles: Vec<A38BundleIndexEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct A38BundleIndexEntry {
    pub name: String,
    pub active_bundle_id: String,
    pub versions: Vec<A38BundleVersionRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct A38BundleVersionRef {
    pub version: u32,
    pub bundle_id: String,
    pub object_path: String,
    pub blake3_hex: String,
    pub size_bytes: u64,
    pub saved_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct SavedA38Bundle {
    pub schema_version: u16,
    pub name: String,
    pub version: u32,
    pub base_template: BaseTemplateRef,
    pub registry_ref: RegistryRef,
    pub required_modalities: Vec<String>,
    pub evidence_refs: Vec<EvidenceRef>,
    pub lenses: Vec<BundleLensRef>,
    pub modality_counts: BTreeMap<String, usize>,
    pub content_lens_count: usize,
    pub total_vram_bytes: u64,
    pub total_vram_mib: f32,
    pub budget_vram_bytes: u64,
    pub budget_vram_mib: u64,
    pub under_budget: bool,
    pub coverage_status: String,
    pub created_at_ms: u64,
}

#[derive(Clone, Debug)]
pub(super) struct BundleDraft {
    pub name: String,
    pub base_template: BaseTemplateRef,
    pub registry_ref: RegistryRef,
    pub required_modalities: Vec<String>,
    pub evidence_refs: Vec<EvidenceRef>,
    pub lenses: Vec<BundleLensRef>,
    pub budget_vram_mib: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct BaseTemplateRef {
    pub name: String,
    pub version: u32,
    pub template_id: String,
    pub content_lens_count: usize,
    pub a37_gate_eligible: bool,
    pub a37_status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct RegistryRef {
    pub path: String,
    pub sha256_hex: String,
    pub size_bytes: u64,
    pub lens_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct EvidenceRef {
    pub path: String,
    pub sha256_hex: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct BundleLensRef {
    pub lens_id: String,
    pub name: String,
    pub modality: String,
    pub runtime: String,
    pub weights_sha256: String,
    pub manifest: String,
    pub placement: Placement,
    pub cost: LensCost,
}

#[derive(Serialize)]
pub(super) struct BundleSummary {
    pub name: String,
    pub active_bundle_id: String,
    pub version: u32,
    pub base_template_id: String,
    pub content_lens_count: usize,
    pub modality_counts: BTreeMap<String, usize>,
    pub evidence_ref_count: usize,
    pub total_vram_bytes: u64,
    pub budget_vram_mib: u64,
    pub under_budget: bool,
    pub coverage_status: String,
    pub object_path: String,
}

impl SavedA38Bundle {
    pub(super) fn validate(&self) -> CliResult {
        if self.schema_version != BUNDLE_OBJECT_VERSION {
            return Err(bundle_error(
                A38_BUNDLE_INVALID,
                format!("unsupported A38 bundle object {}", self.schema_version),
                "migrate the A38 bundle object through a compatible reader",
            ));
        }
        validate_name(&self.name)?;
        if self.lenses.len() < 10 || self.content_lens_count != self.lenses.len() {
            return Err(bundle_error(
                A38_BUNDLE_INCOMPLETE,
                format!(
                    "A38 bundle {} has {} content lenses; minimum is 10",
                    self.name,
                    self.lenses.len()
                ),
                "include the base A35 roster plus admitted cross-modal lenses",
            ));
        }
        if !base_gate_passed(&self.base_template) {
            return Err(bundle_error(
                A38_BUNDLE_BASE_A37_REFUSED,
                format!(
                    "base template {} is not A37 gate_passed",
                    self.base_template.name
                ),
                "profile the base template with an A37 gate_passed ensemble card first",
            ));
        }
        if self.evidence_refs.is_empty() {
            return Err(bundle_error(
                A38_BUNDLE_INVALID,
                format!("A38 bundle {} has no evidence refs", self.name),
                "attach at least one persisted FSV/admission evidence artifact",
            ));
        }
        validate_required_modalities(&self.required_modalities, &self.modality_counts)?;
        if !self.under_budget || self.total_vram_bytes > self.budget_vram_bytes {
            return Err(bundle_error(
                A38_BUNDLE_BUDGET_EXCEEDED,
                format!(
                    "A38 bundle {} uses {} bytes VRAM over budget {}",
                    self.name, self.total_vram_bytes, self.budget_vram_bytes
                ),
                "remove heavy lenses or raise the explicit budget only after FSV",
            ));
        }
        if self.coverage_status != A38_COVERAGE_STATUS {
            return Err(bundle_error(
                A38_BUNDLE_INVALID,
                format!("unknown A38 coverage status {}", self.coverage_status),
                "save the bundle through this command so coverage state is recomputed",
            ));
        }
        Ok(())
    }
}

pub(super) fn validate_required_modalities(
    required: &[String],
    counts: &BTreeMap<String, usize>,
) -> CliResult {
    if required.is_empty() {
        return Err(bundle_error(
            A38_BUNDLE_INCOMPLETE,
            "A38 bundle has no required modalities",
            "declare at least one required modality",
        ));
    }
    let missing = required
        .iter()
        .filter(|modality| counts.get(*modality).copied().unwrap_or_default() == 0)
        .cloned()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    Err(bundle_error(
        A38_BUNDLE_INCOMPLETE,
        format!(
            "A38 bundle is missing required modalities: {}",
            missing.join(",")
        ),
        "include at least one admitted registry lens for every required modality",
    ))
}

pub(super) fn modality_counts(lenses: &[BundleLensRef]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for lens in lenses {
        *counts.entry(lens.modality.clone()).or_insert(0) += 1;
    }
    counts
}

pub(super) fn validate_name(name: &str) -> CliResult {
    if !name.trim().is_empty() && !name.contains(['/', '\\']) {
        return Ok(());
    }
    Err(bundle_error(
        A38_BUNDLE_INVALID,
        "A38 bundle name must be non-empty and path-safe",
        "choose a stable bundle name such as constellation-24-general",
    ))
}

pub(super) fn base_gate_passed(base: &BaseTemplateRef) -> bool {
    base.a37_gate_eligible && base.a37_status == "gate_passed"
}

pub(super) fn template_id(template: &SavedPanelTemplate) -> CliResult<String> {
    Ok(blake3::hash(&serde_json::to_vec_pretty(template)?)
        .to_hex()
        .to_string())
}

pub(super) fn parse_modality(value: &str) -> CliResult<&'static str> {
    match value {
        "text" => Ok("text"),
        "code" => Ok("code"),
        "image" => Ok("image"),
        "audio" => Ok("audio"),
        "video" => Ok("video"),
        "protein" => Ok("protein"),
        "dna" => Ok("dna"),
        "molecule" => Ok("molecule"),
        "structured" => Ok("structured"),
        "mixed" => Ok("mixed"),
        other => Err(CliError::usage(format!("unknown modality {other}"))),
    }
}

pub(super) fn object_bytes(bundle: &SavedA38Bundle) -> CliResult<Vec<u8>> {
    Ok(serde_json::to_vec_pretty(bundle)?)
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}

pub(super) fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

pub(super) fn mib_to_bytes(value: u64) -> u64 {
    value.saturating_mul(1024 * 1024)
}

pub(super) fn mib(bytes: u64) -> f32 {
    bytes as f32 / (1024.0 * 1024.0)
}
