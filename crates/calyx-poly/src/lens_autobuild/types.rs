use calyx_assay::{EstimateBound, TrustTag};
use serde::{Deserialize, Serialize};

use crate::panel_sufficiency::PolyPanelSufficiencyReport;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LensDeficit {
    pub domain: String,
    pub panel_id: String,
    pub panel_version: u32,
    pub source_artifact: String,
    pub proposal_action: String,
    pub deficit_bits: f32,
    pub weakest_slots: Vec<u16>,
    pub reason: String,
    pub trust: TrustTag,
}

impl LensDeficit {
    /// Builds a deficit reference from a persisted panel-sufficiency report.
    pub fn from_panel_sufficiency_report(
        report: &PolyPanelSufficiencyReport,
        source_artifact: impl Into<String>,
    ) -> Option<Self> {
        let proposal = report.assay_card.deficit_proposal.as_ref()?;
        if proposal.action != "propose_lens" {
            return None;
        }
        Some(Self {
            domain: report.domain.clone(),
            panel_id: report.panel_id.clone(),
            panel_version: report.panel_version,
            source_artifact: source_artifact.into(),
            proposal_action: proposal.action.clone(),
            deficit_bits: proposal.deficit_bits,
            weakest_slots: proposal
                .weakest_slots
                .iter()
                .map(|slot| slot.get())
                .collect(),
            reason: proposal.reason.clone(),
            trust: report.assay_card.sufficiency.trust,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LensCandidateMeasurement {
    pub lens_key: String,
    pub encoder_kind: String,
    pub source_fields: Vec<String>,
    pub measured_gain_bits: f32,
    pub ci_low_bits: f32,
    pub ci_high_bits: f32,
    pub n_samples: usize,
    pub trust: TrustTag,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate_bound: Option<EstimateBound>,
    pub evidence_artifact: String,
    pub requested_action: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LensAutobuildRequest {
    pub domain: String,
    pub panel_id: String,
    pub panel_version: u32,
    pub existing_lens_keys: Vec<String>,
    pub deficits: Vec<LensDeficit>,
    pub candidates: Vec<LensCandidateMeasurement>,
    pub min_gain_bits: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LensAutobuildStatus {
    Admitted,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BuiltLensSpec {
    pub lens_id: String,
    pub lens_key: String,
    pub encoder_kind: String,
    pub source_fields: Vec<String>,
    pub registry_patch_kind: String,
    pub target_slots: Vec<u16>,
    pub expected_gain_bits: f32,
    pub ci_low_bits: f32,
    pub n_samples: usize,
    pub trust: TrustTag,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate_bound: Option<EstimateBound>,
    pub deficit_source_artifact: String,
    pub evidence_artifact: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LensCandidateRejection {
    pub lens_key: String,
    pub code: String,
    pub reason: String,
    pub measured_gain_bits: f32,
    pub trust: TrustTag,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate_bound: Option<EstimateBound>,
    pub requested_action: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LensAutobuildReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub domain: String,
    pub panel_id: String,
    pub panel_version: u32,
    pub min_gain_bits: f32,
    pub existing_lens_count: usize,
    pub deficit_count: usize,
    pub candidate_count: usize,
    pub admitted_count: usize,
    pub rejected_count: usize,
    pub status: LensAutobuildStatus,
    pub primary_deficit: LensDeficit,
    pub admitted: Vec<BuiltLensSpec>,
    pub rejected: Vec<LensCandidateRejection>,
    pub decision_hash: String,
}
