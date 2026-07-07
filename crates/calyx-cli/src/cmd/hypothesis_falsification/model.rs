use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::cmd::discovery_run_preflight::DiscoveryRunPreflightArgs;
use crate::cmd::mechanistic_direction::{
    MechanisticDirectionEvidence, MutationConsequence, TargetModulation,
};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HypothesisFalsificationArgs {
    pub hypotheses_reports: Vec<PathBuf>,
    pub pubtator_root: PathBuf,
    pub clinicaltrials_root: PathBuf,
    pub dgidb_root: PathBuf,
    pub open_targets_root: PathBuf,
    pub out_dir: PathBuf,
    pub max_hypotheses: usize,
    pub preflight: DiscoveryRunPreflightArgs,
}

impl Default for HypothesisFalsificationArgs {
    fn default() -> Self {
        Self {
            hypotheses_reports: Vec::new(),
            pubtator_root: PathBuf::new(),
            clinicaltrials_root: PathBuf::new(),
            dgidb_root: PathBuf::new(),
            open_targets_root: PathBuf::new(),
            out_dir: PathBuf::new(),
            max_hypotheses: 10_000,
            preflight: DiscoveryRunPreflightArgs::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct InputHypothesis {
    pub hypothesis_id: String,
    pub source_id: String,
    pub source_name: String,
    pub source_type: String,
    pub target_id: String,
    pub target_name: String,
    pub target_type: String,
    pub support_count: usize,
    pub score: f64,
    #[serde(default)]
    pub mechanistic_direction_status: String,
    #[serde(default)]
    pub required_target_modulation: Option<TargetModulation>,
    #[serde(default)]
    pub observed_target_modulation: Option<TargetModulation>,
    #[serde(default)]
    pub mutation_consequence: Option<MutationConsequence>,
    #[serde(default)]
    pub direction_reason_codes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct EvidenceRow {
    pub hypothesis_id: String,
    pub evidence_kind: String,
    pub source_system: String,
    pub reason_code: String,
    pub source_path: String,
    pub source_sha256: String,
    pub source_row_index: usize,
    pub weight: f64,
    pub summary: String,
    pub mechanistic_direction: Option<MechanisticDirectionEvidence>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct SkippedEvidenceRow {
    pub source_system: String,
    pub role: String,
    pub reason_code: String,
    pub source_path: String,
    pub source_sha256: String,
    pub source_row_index: usize,
    pub summary: String,
    pub mechanistic_direction: Option<MechanisticDirectionEvidence>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct HypothesisFlag {
    pub hypothesis_id: String,
    pub source_name: String,
    pub source_type: String,
    pub target_name: String,
    pub target_type: String,
    pub support_evidence_count: usize,
    pub counter_evidence_count: usize,
    pub support_weight: f64,
    pub counter_weight: f64,
    pub falsification_score: f64,
    pub reason_codes: Vec<String>,
    pub mechanistic_direction_status: String,
    pub required_target_modulation: Option<TargetModulation>,
    pub observed_target_modulation: Option<TargetModulation>,
    pub mutation_consequence: Option<MutationConsequence>,
    pub sweep_status: String,
    pub human_review_atlas_status: String,
    pub clinical_boundary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct RawQueryManifestRow {
    pub source_system: String,
    pub source_path: String,
    pub source_sha256: String,
    pub bytes: u64,
    pub role: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct FalsificationReport {
    pub schema_version: u32,
    pub status: String,
    pub hypotheses_reports: Vec<String>,
    pub pubtator_root: String,
    pub clinicaltrials_root: String,
    pub dgidb_root: String,
    pub open_targets_root: String,
    pub input_hypothesis_count: usize,
    pub deduped_hypothesis_count: usize,
    pub raw_query_manifest_count: usize,
    pub support_evidence_count: usize,
    pub counter_evidence_count: usize,
    pub skipped_evidence_count: usize,
    pub flagged_with_counter_evidence_count: usize,
    pub hypothesis_flags: Vec<HypothesisFlag>,
    pub support_evidence: Vec<EvidenceRow>,
    pub counter_evidence: Vec<EvidenceRow>,
    pub skipped_evidence: Vec<SkippedEvidenceRow>,
    pub raw_query_manifest: Vec<RawQueryManifestRow>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct FalsificationSummary {
    pub status: &'static str,
    pub out_dir: String,
    pub report: String,
    pub report_sha256: String,
    pub support_evidence_jsonl: String,
    pub support_evidence_sha256: String,
    pub counter_evidence_jsonl: String,
    pub counter_evidence_sha256: String,
    pub skipped_evidence_jsonl: String,
    pub skipped_evidence_sha256: String,
    pub hypothesis_flags_jsonl: String,
    pub hypothesis_flags_sha256: String,
    pub raw_query_manifest_jsonl: String,
    pub raw_query_manifest_sha256: String,
    pub input_hypothesis_count: usize,
    pub deduped_hypothesis_count: usize,
    pub support_evidence_count: usize,
    pub counter_evidence_count: usize,
    pub skipped_evidence_count: usize,
    pub flagged_with_counter_evidence_count: usize,
    pub readback_flag_count: usize,
}

#[derive(Default)]
pub(super) struct LoadedSources {
    pub support_evidence: Vec<EvidenceRow>,
    pub counter_evidence: Vec<EvidenceRow>,
    pub skipped_evidence: Vec<SkippedEvidenceRow>,
    pub raw_query_manifest: Vec<RawQueryManifestRow>,
}
