use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(super) const SCHEMA_VERSION: u32 = 1;
pub(super) const DEFAULT_KNOWN_LITERATURE_THRESHOLD: u64 = 3;
pub(super) const DEFAULT_MIN_STABILITY_FREQUENCY: f64 = 0.67;
pub(super) const DEFAULT_MAX_TRANSCRIPTOMIC_CLASS_BREADTH: u64 = 25;
pub(super) const CLINICAL_BOUNDARY: &str = "Computational biomedical hypothesis audit only; not efficacy, safety, clinical actionability, treatment guidance, dosing guidance, recommendation, pair-interaction proof, or cure evidence.";

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BiomedicalBlindspotAuditArgs {
    pub hypotheses_reports: Vec<PathBuf>,
    pub literature_audit: PathBuf,
    pub stability_audit: PathBuf,
    pub drug_lifecycle: PathBuf,
    pub transcriptomic_audit: PathBuf,
    pub out_dir: PathBuf,
    pub known_literature_threshold: u64,
    pub min_stability_frequency: f64,
    pub max_transcriptomic_class_breadth: u64,
}

impl Default for BiomedicalBlindspotAuditArgs {
    fn default() -> Self {
        Self {
            hypotheses_reports: Vec::new(),
            literature_audit: PathBuf::new(),
            stability_audit: PathBuf::new(),
            drug_lifecycle: PathBuf::new(),
            transcriptomic_audit: PathBuf::new(),
            out_dir: PathBuf::new(),
            known_literature_threshold: DEFAULT_KNOWN_LITERATURE_THRESHOLD,
            min_stability_frequency: DEFAULT_MIN_STABILITY_FREQUENCY,
            max_transcriptomic_class_breadth: DEFAULT_MAX_TRANSCRIPTOMIC_CLASS_BREADTH,
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct CliSummary {
    pub(super) status: &'static str,
    pub(super) out_dir: String,
    pub(super) report: String,
    pub(super) report_sha256: String,
    pub(super) audited_hypotheses_jsonl: String,
    pub(super) audited_hypotheses_sha256: String,
    pub(super) ready_hypotheses_jsonl: String,
    pub(super) ready_hypotheses_sha256: String,
    pub(super) blocked_hypotheses_jsonl: String,
    pub(super) blocked_hypotheses_sha256: String,
    pub(super) benchmark_export_jsonl: String,
    pub(super) benchmark_export_sha256: String,
    pub(super) metrics_json: String,
    pub(super) metrics_sha256: String,
    pub(super) audited_count: usize,
    pub(super) ready_count: usize,
    pub(super) blocked_count: usize,
    pub(super) pending_count: usize,
    pub(super) readback: ReadbackSummary,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct SourceManifest {
    pub(super) label: String,
    pub(super) path: String,
    pub(super) bytes: u64,
    pub(super) rows: Option<usize>,
    pub(super) sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct AuditParams {
    pub(super) known_literature_threshold: u64,
    pub(super) min_stability_frequency: f64,
    pub(super) max_transcriptomic_class_breadth: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Candidate {
    pub(super) hypothesis_id: String,
    pub(super) source_name: String,
    pub(super) source_type: String,
    pub(super) target_name: String,
    pub(super) target_type: String,
    pub(super) drug_names: Vec<String>,
    pub(super) target_names: Vec<String>,
    pub(super) disease_names: Vec<String>,
    pub(super) candidate_type: String,
    pub(super) evidence_type: String,
    pub(super) score: Option<f64>,
    pub(super) novelty_score: Option<f64>,
    pub(super) patient_context: String,
    pub(super) therapeutic_rationale: String,
    pub(super) clinical_boundary: String,
    pub(super) raw: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct LiteratureEvidence {
    pub(super) source_system: String,
    pub(super) publication_count: u64,
    pub(super) co_mention_count: u64,
    pub(super) query: String,
    pub(super) source_ids: Vec<String>,
    pub(super) raw: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct StabilityEvidence {
    pub(super) run_count: u64,
    pub(super) present_count: u64,
    pub(super) frequency: f64,
    pub(super) corpus_count: Option<u64>,
    pub(super) seed_count: Option<u64>,
    pub(super) raw: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct LifecycleEvidence {
    pub(super) drug_name: String,
    pub(super) max_phase: Option<f64>,
    pub(super) lifecycle_status: String,
    pub(super) trial_status: String,
    pub(super) integrity_status: String,
    pub(super) withdrawn_flag: Option<bool>,
    pub(super) source_system: String,
    pub(super) raw: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct TranscriptomicEvidence {
    pub(super) perturbagen_id: String,
    pub(super) signature_id: String,
    pub(super) cell_context: String,
    pub(super) mechanism_class: String,
    pub(super) class_breadth: Option<u64>,
    pub(super) is_gold: Option<bool>,
    pub(super) reproducible: Option<bool>,
    pub(super) self_connected: Option<bool>,
    pub(super) raw: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct AuditedHypothesis {
    pub(super) hypothesis_id: String,
    pub(super) final_status: String,
    pub(super) novelty_promotion_allowed: bool,
    pub(super) benchmark_exportable: bool,
    pub(super) reason_codes: Vec<String>,
    pub(super) warning_codes: Vec<String>,
    pub(super) source_name: String,
    pub(super) source_type: String,
    pub(super) target_name: String,
    pub(super) target_type: String,
    pub(super) drug_names: Vec<String>,
    pub(super) target_names: Vec<String>,
    pub(super) disease_names: Vec<String>,
    pub(super) score: Option<f64>,
    pub(super) novelty_score: Option<f64>,
    pub(super) external_novelty_class: String,
    pub(super) literature_co_mention_count: Option<u64>,
    pub(super) stability_frequency: Option<f64>,
    pub(super) drug_lifecycle_statuses: Vec<String>,
    pub(super) transcriptomic_specificity_status: String,
    pub(super) patient_context_status: String,
    pub(super) clinical_boundary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct BenchmarkExportRow {
    pub(super) hypothesis_id: String,
    pub(super) disease_names: Vec<String>,
    pub(super) target_names: Vec<String>,
    pub(super) drug_names: Vec<String>,
    pub(super) final_status: String,
    pub(super) novelty_score: Option<f64>,
    pub(super) external_novelty_class: String,
    pub(super) stability_frequency: Option<f64>,
    pub(super) reason_codes: Vec<String>,
    pub(super) source_name: String,
    pub(super) source_type: String,
    pub(super) target_name: String,
    pub(super) target_type: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct CorrelationMetric {
    pub(super) n: usize,
    pub(super) pearson: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Metrics {
    pub(super) status_counts: BTreeMap<String, usize>,
    pub(super) reason_code_counts: BTreeMap<String, usize>,
    pub(super) external_novelty_counts: BTreeMap<String, usize>,
    pub(super) novelty_score_external_literature_correlation: Option<CorrelationMetric>,
    pub(super) novelty_score_external_literature_correlation_status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct AuditReport {
    pub(super) schema_version: u32,
    pub(super) status: String,
    pub(super) clinical_boundary: String,
    pub(super) params: AuditParams,
    pub(super) source_manifests: Vec<SourceManifest>,
    pub(super) input_hypothesis_count: usize,
    pub(super) deduped_hypothesis_count: usize,
    pub(super) audited_count: usize,
    pub(super) ready_count: usize,
    pub(super) blocked_count: usize,
    pub(super) pending_count: usize,
    pub(super) audited_hypotheses: Vec<AuditedHypothesis>,
    pub(super) benchmark_export: Vec<BenchmarkExportRow>,
    pub(super) metrics: Metrics,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct ReadbackSummary {
    pub(super) report: String,
    pub(super) report_sha256: String,
    pub(super) audited_hypotheses: String,
    pub(super) audited_hypotheses_rows: usize,
    pub(super) audited_hypotheses_sha256: String,
    pub(super) ready_hypotheses: String,
    pub(super) ready_hypotheses_rows: usize,
    pub(super) ready_hypotheses_sha256: String,
    pub(super) blocked_hypotheses: String,
    pub(super) blocked_hypotheses_rows: usize,
    pub(super) blocked_hypotheses_sha256: String,
    pub(super) benchmark_export: String,
    pub(super) benchmark_export_rows: usize,
    pub(super) benchmark_export_sha256: String,
    pub(super) metrics: String,
    pub(super) metrics_sha256: String,
}

pub(super) struct AuditSources {
    pub(super) manifests: Vec<SourceManifest>,
    pub(super) candidates: Vec<Candidate>,
    pub(super) literature_by_id: BTreeMap<String, LiteratureEvidence>,
    pub(super) literature_by_key: BTreeMap<String, LiteratureEvidence>,
    pub(super) stability_by_id: BTreeMap<String, StabilityEvidence>,
    pub(super) lifecycle_by_drug: BTreeMap<String, LifecycleEvidence>,
    pub(super) transcriptomic_by_id: BTreeMap<String, TranscriptomicEvidence>,
    pub(super) transcriptomic_by_key: BTreeMap<String, TranscriptomicEvidence>,
    pub(super) input_hypothesis_count: usize,
}
