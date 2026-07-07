use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cmd::discovery_run_preflight::DiscoveryRunPreflightArgs;
use crate::cmd::mechanistic_direction::{
    MechanisticDirectionEvidence, MutationConsequence, TargetModulation,
};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TypedAssociationMinerArgs {
    pub typed_root: PathBuf,
    pub validation_report: PathBuf,
    pub out_dir: PathBuf,
    pub source_type: Option<String>,
    pub target_type: Option<String>,
    pub name_contains: Option<String>,
    pub source_issue: Option<u64>,
    pub min_support: usize,
    pub max_pairs: usize,
    pub max_input_edges: usize,
    pub max_paths_per_pair: usize,
    pub preflight: DiscoveryRunPreflightArgs,
}

impl Default for TypedAssociationMinerArgs {
    fn default() -> Self {
        Self {
            typed_root: PathBuf::new(),
            validation_report: PathBuf::new(),
            out_dir: PathBuf::new(),
            source_type: None,
            target_type: None,
            name_contains: None,
            source_issue: None,
            min_support: 1,
            max_pairs: 100,
            max_input_edges: 100_000,
            max_paths_per_pair: 8,
            preflight: DiscoveryRunPreflightArgs::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ConceptNode {
    pub node_id: String,
    pub normalized_name: String,
    pub concept_type: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct TypedPath {
    pub edge_id: String,
    pub edge_type: String,
    pub support_count: usize,
    pub source_issue: Option<u64>,
    pub source_hashes: Vec<String>,
    pub support_cx_ids: Vec<String>,
    pub mechanistic_direction: Option<MechanisticDirectionEvidence>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct AssociationHypothesis {
    pub hypothesis_id: String,
    pub source_id: String,
    pub source_name: String,
    pub source_type: String,
    pub target_id: String,
    pub target_name: String,
    pub target_type: String,
    pub typed_paths: Vec<TypedPath>,
    pub path_count: usize,
    pub support_count: usize,
    pub score: f64,
    pub novelty_score: f64,
    pub mechanistic_direction_status: String,
    pub required_target_modulation: Option<TargetModulation>,
    pub observed_target_modulation: Option<TargetModulation>,
    pub mutation_consequence: Option<MutationConsequence>,
    pub direction_reason_codes: Vec<String>,
    pub validation_gate_report_sha256: String,
    pub counter_evidence_hooks: Vec<String>,
    pub clinical_boundary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct BlockedAssociationCandidate {
    pub edge_id: String,
    pub source_id: String,
    pub source_name: String,
    pub source_type: String,
    pub target_id: String,
    pub target_name: String,
    pub target_type: String,
    pub reason_codes: Vec<String>,
    pub mechanistic_direction: MechanisticDirectionEvidence,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct MinerReport {
    pub schema_version: u32,
    pub status: String,
    pub typed_root: String,
    pub validation_report: String,
    pub validation_report_sha256: String,
    pub validation_gate_passed: bool,
    pub input_node_count: usize,
    pub input_edge_count: usize,
    pub scan_limit_reached: bool,
    pub candidate_pair_count: usize,
    pub blocked_candidate_count: usize,
    pub emitted_hypothesis_count: usize,
    pub filters: Value,
    pub hypotheses: Vec<AssociationHypothesis>,
    pub blocked_candidates: Vec<BlockedAssociationCandidate>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct MinerCliSummary {
    pub status: &'static str,
    pub out_dir: String,
    pub report: String,
    pub report_sha256: String,
    pub hypotheses_jsonl: String,
    pub hypotheses_sha256: String,
    pub blocked_candidates_jsonl: String,
    pub blocked_candidates_sha256: String,
    pub score_summary_json: String,
    pub score_summary_sha256: String,
    pub emitted_hypothesis_count: usize,
    pub candidate_pair_count: usize,
    pub blocked_candidate_count: usize,
    pub readback_hypothesis_count: usize,
    pub readback_blocked_candidate_count: usize,
    pub scan_limit_reached: bool,
}

pub(super) struct ScanOutput {
    pub input_edges: usize,
    pub limit_reached: bool,
    pub max_support: usize,
    pub candidates: Vec<AssociationHypothesis>,
    pub blocked_candidates: Vec<BlockedAssociationCandidate>,
}

pub(super) fn new_hypothesis(source: &ConceptNode, target: &ConceptNode) -> AssociationHypothesis {
    AssociationHypothesis {
        hypothesis_id: format!("typed-assoc:{}::{}", source.node_id, target.node_id),
        source_id: source.node_id.clone(),
        source_name: source.normalized_name.clone(),
        source_type: source.concept_type.clone(),
        target_id: target.node_id.clone(),
        target_name: target.normalized_name.clone(),
        target_type: target.concept_type.clone(),
        typed_paths: Vec::new(),
        path_count: 0,
        support_count: 0,
        score: 0.0,
        novelty_score: 0.0,
        mechanistic_direction_status: "not_mechanistic".to_string(),
        required_target_modulation: None,
        observed_target_modulation: None,
        mutation_consequence: None,
        direction_reason_codes: Vec::new(),
        validation_gate_report_sha256: String::new(),
        counter_evidence_hooks: vec![
            "requires_1184_falsification_sweep".to_string(),
            "requires_safety_triage_for_drug_or_intervention_claims".to_string(),
        ],
        clinical_boundary:
            "Association hypothesis only; not efficacy, safety, actionability, or cure evidence."
                .to_string(),
    }
}

pub(super) fn score(support: usize, max_support: usize) -> f64 {
    ((support as f64).ln_1p() / (max_support as f64).ln_1p()).clamp(0.0, 1.0)
}

pub(super) fn novelty(support: usize) -> f64 {
    (1.0 / (1.0 + support as f64)).clamp(0.0, 1.0)
}
