use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::super::discovery_run_preflight::DiscoveryRunPreflightArgs;

pub(super) const COMMAND: &str = "novelty-calibration-split";
pub(super) const SCHEMA_VERSION: u32 = 1;
pub(super) const CLINICAL_BOUNDARY: &str = "Novelty-prioritized research triage only; not \
     clinical novelty, efficacy, safety, actionability, treatment guidance, or cure evidence.";

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NoveltySplitArgs {
    pub atlases: Vec<AtlasInputArg>,
    pub out_dir: PathBuf,
    pub top_k: usize,
    pub preflight: DiscoveryRunPreflightArgs,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AtlasInputArg {
    pub issue: String,
    pub domain: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
pub(super) struct LoadedAtlas {
    pub arg: AtlasInputArg,
    pub bytes: Vec<u8>,
    pub sha256: String,
    pub rows: Vec<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct SourceArtifact {
    pub issue: String,
    pub domain: String,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    pub row_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct InputScope {
    pub schema_version: u32,
    pub command: String,
    pub source_artifacts: Vec<SourceArtifact>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct SplitRow {
    pub schema_version: u32,
    pub issue: String,
    pub domain: String,
    pub source_row_index: usize,
    pub source_artifact: String,
    pub source_artifact_sha256: String,
    pub candidate_id: String,
    pub display_names: Vec<String>,
    pub source_class: String,
    pub original_rank: usize,
    pub original_rank_score: f64,
    pub original_rank_percentile_within_domain: f64,
    pub calibration_known_positive: bool,
    pub calibration_flags: Vec<CalibrationFlag>,
    pub evidence_shape: EvidenceShape,
    pub score_components: ScoreComponents,
    pub novelty_priority_score: f64,
    pub falsification_summary: String,
    pub combined_original_order: Option<usize>,
    pub novelty_rank: Option<usize>,
    pub calibration_rank: Option<usize>,
    pub original_row: Value,
    pub clinical_boundary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct CalibrationFlag {
    pub code: String,
    pub strength: String,
    pub detail: Option<String>,
    pub score: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct EvidenceShape {
    pub has_open_targets: bool,
    pub has_dgidb: bool,
    pub has_civic: bool,
    pub has_trial_context: bool,
    pub has_openfda_context: bool,
    pub has_generated_after_falsification: bool,
    pub has_counterevidence: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ScoreComponents {
    pub original_rank_percentile_weighted: f64,
    pub bridge_external_context_bonus: f64,
    pub generated_or_unfalsified_penalty: f64,
    pub safety_trial_gap_penalty: f64,
    pub calibration_view_exclusion_penalty: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct SlimRow {
    pub issue: String,
    pub domain: String,
    pub candidate_id: String,
    pub display_names: Vec<String>,
    pub source_class: String,
    pub original_rank: usize,
    pub original_rank_score: f64,
    pub original_rank_percentile_within_domain: f64,
    pub calibration_known_positive: bool,
    pub calibration_flag_codes: Vec<String>,
    pub novelty_priority_score: f64,
    pub score_components: ScoreComponents,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct BeforeAfterTopK {
    pub schema_version: u32,
    pub before_combined_original_top_k: Vec<SlimRow>,
    pub after_novelty_prioritized_top_k: Vec<SlimRow>,
    pub calibration_known_positive_top_k: Vec<SlimRow>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct SpotCheck {
    pub check: String,
    pub match_count: usize,
    pub examples: Vec<SlimRow>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ManualSpotChecks {
    pub schema_version: u32,
    pub checks: Vec<SpotCheck>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ValidationMetrics {
    pub schema_version: u32,
    pub status: String,
    pub input_row_counts: BTreeMap<String, usize>,
    pub total_rows: usize,
    pub calibration_known_positive_rows: usize,
    pub novelty_prioritized_rows: usize,
    pub calibration_by_issue: BTreeMap<String, usize>,
    pub novelty_by_issue: BTreeMap<String, usize>,
    pub calibration_flag_counts: BTreeMap<String, usize>,
    pub top_original_candidate: Option<SlimRow>,
    pub top_novelty_candidate: Option<SlimRow>,
    pub top_calibration_candidate: Option<SlimRow>,
    pub clinical_boundary: String,
    pub interpretation: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct PersistedReadback {
    pub schema_version: u32,
    pub status: String,
    pub root: String,
    pub artifacts: BTreeMap<String, ArtifactReadback>,
    pub assertions: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ArtifactReadback {
    pub bytes: u64,
    pub sha256: String,
}
