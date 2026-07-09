use serde::Deserialize;
use serde_json::Value;

use calyx_core::AnchorValue;

pub const ENDPOINT_SIGNALS: &str = "learner_signals_batches";
pub const ENDPOINT_DECIDE: &str = "interventions_decide";
pub const ENDPOINT_OUTCOMES: &str = "intervention_outcomes";
pub const ENDPOINT_MASTERY_ESTIMATE: &str = "mastery_estimate";
pub const ENDPOINT_ORACLE_FORECAST: &str = "oracle_forecast";
pub const ENDPOINT_REACTIVE_AFFECT: &str = "reactive_affect_signals";
pub const ENDPOINT_TRACK_SPINES: &str = "kernel_track_spines";

pub const KIND_SIGNAL_BATCH: &str = "learner_signal_batch";
pub const KIND_DECISION: &str = "intervention_decision";
pub const KIND_OUTCOME: &str = "intervention_outcome";
pub const KIND_MASTERY_ESTIMATE: &str = "mastery_estimate";
pub const KIND_ORACLE_FORECAST: &str = "oracle_forecast";
pub const KIND_REACTIVE_AFFECT: &str = "reactive_affect_signal";
pub const KIND_TRACK_SPINES: &str = "track_spines";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalBatchRequest {
    #[serde(alias = "batch_id")]
    pub batch_id: String,
    #[serde(default, alias = "idempotency_key")]
    pub idempotency_key: Option<String>,
    #[serde(alias = "learner_id")]
    pub learner_id: String,
    #[serde(default, alias = "session_id")]
    pub session_id: Option<String>,
    #[serde(default, alias = "privacy_class")]
    pub privacy_class: Option<String>,
    #[serde(default, alias = "signals")]
    pub events: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionRequest {
    #[serde(default, alias = "decision_id")]
    pub decision_id: Option<String>,
    #[serde(default, alias = "idempotency_key")]
    pub idempotency_key: Option<String>,
    #[serde(alias = "learner_id")]
    pub learner_id: String,
    #[serde(alias = "concept_id")]
    pub concept_id: String,
    #[serde(default, alias = "session_id")]
    pub session_id: Option<String>,
    #[serde(default, alias = "privacy_class")]
    pub privacy_class: Option<String>,
    #[serde(default)]
    pub need: Option<String>,
    #[serde(default)]
    pub trigger: Option<String>,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default, alias = "evidence_ids")]
    pub evidence_ids: Vec<String>,
    #[serde(default, alias = "allowed_widget_kinds")]
    pub allowed_widget_kinds: Vec<String>,
    #[serde(default, alias = "cooldown_until")]
    pub cooldown_until: Option<u64>,
    #[serde(default, alias = "now_millis")]
    pub now_millis: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutcomeRequest {
    #[serde(default, alias = "outcome_id")]
    pub outcome_id: Option<String>,
    #[serde(default, alias = "decision_id")]
    pub decision_id: Option<String>,
    #[serde(alias = "learner_id")]
    pub learner_id: String,
    #[serde(default)]
    pub outcome: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default, alias = "privacy_class")]
    pub privacy_class: Option<String>,
    #[serde(default)]
    pub evidence: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MasteryEstimateRequest {
    #[serde(default, alias = "request_id")]
    pub request_id: Option<String>,
    #[serde(default, alias = "idempotency_key")]
    pub idempotency_key: Option<String>,
    #[serde(alias = "learner_id")]
    pub learner_id: String,
    #[serde(default, alias = "session_id")]
    pub session_id: Option<String>,
    #[serde(default, alias = "privacy_class")]
    pub privacy_class: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub concepts: Vec<MasteryConceptRequest>,
    #[serde(alias = "panel_bits")]
    pub panel_bits: f32,
    #[serde(alias = "anchor_entropy_bits")]
    pub anchor_entropy_bits: f32,
    #[serde(alias = "oracle_self_consistency")]
    pub oracle_self_consistency: OracleSelfConsistencyRequest,
    #[serde(alias = "trust_gate")]
    pub trust_gate: MasteryTrustGateRequest,
    #[serde(default, alias = "now_millis")]
    pub now_millis: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MasteryConceptRequest {
    #[serde(alias = "concept_id")]
    pub concept_id: String,
    #[serde(default)]
    pub mastery: Option<f32>,
    #[serde(default, alias = "trusted_mastery")]
    pub trusted_mastery: Option<f32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OracleSelfConsistencyRequest {
    pub flakiness: f32,
    pub validity: f32,
    #[serde(default)]
    pub provisional: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MasteryTrustGateRequest {
    #[serde(alias = "held_out_count")]
    pub held_out_count: usize,
    #[serde(alias = "kernel_recall_ratio")]
    pub kernel_recall_ratio: f32,
    #[serde(alias = "calibration_error")]
    pub calibration_error: f32,
    #[serde(alias = "goodhart_pass_rate")]
    pub goodhart_pass_rate: f32,
    #[serde(default, alias = "goodhart_passed")]
    pub goodhart_passed: Option<bool>,
    #[serde(default, alias = "goodhart_violations")]
    pub goodhart_violations: Option<usize>,
    #[serde(default, alias = "recurring_mistakes")]
    pub recurring_mistakes: usize,
    #[serde(default, alias = "replayed_mistakes")]
    pub replayed_mistakes: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OracleForecastRequest {
    #[serde(default, alias = "request_id")]
    pub request_id: Option<String>,
    #[serde(default, alias = "idempotency_key")]
    pub idempotency_key: Option<String>,
    #[serde(alias = "learner_id")]
    pub learner_id: String,
    #[serde(default, alias = "session_id")]
    pub session_id: Option<String>,
    #[serde(default, alias = "privacy_class")]
    pub privacy_class: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(alias = "action_id")]
    pub action_id: String,
    #[serde(default, alias = "panel_concepts")]
    pub panel_concepts: Vec<String>,
    #[serde(alias = "panel_bits")]
    pub panel_bits: f32,
    #[serde(alias = "anchor_entropy_bits")]
    pub anchor_entropy_bits: f32,
    #[serde(default)]
    pub observations: Vec<OracleObservationRequest>,
    #[serde(default, alias = "desired_outcome")]
    pub desired_outcome: Option<AnchorValue>,
    #[serde(alias = "reverse_answer")]
    pub reverse_answer: AnchorValue,
    #[serde(alias = "transfer_entropy")]
    pub transfer_entropy: TransferEntropyRequest,
    #[serde(default, alias = "now_millis")]
    pub now_millis: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OracleObservationRequest {
    #[serde(alias = "action", alias = "action_id")]
    pub action_id: String,
    pub outcome: AnchorValue,
    #[serde(default, alias = "ground_truth")]
    pub ground_truth: Option<AnchorValue>,
    #[serde(default)]
    pub consequences: Vec<OracleConsequenceRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OracleConsequenceRequest {
    #[serde(alias = "action_or_event")]
    pub action_or_event: String,
    #[serde(default)]
    pub domain: Option<String>,
    pub outcome: AnchorValue,
    #[serde(default = "default_grounded")]
    pub grounded: bool,
    #[serde(default)]
    pub provisional: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferEntropyRequest {
    #[serde(alias = "source_concept_id")]
    pub source_concept_id: String,
    #[serde(alias = "target_concept_id")]
    pub target_concept_id: String,
    #[serde(alias = "source_series")]
    pub source_series: Vec<TransferEntropySampleRequest>,
    #[serde(alias = "target_series")]
    pub target_series: Vec<TransferEntropySampleRequest>,
    #[serde(default)]
    pub lags: Vec<usize>,
    #[serde(default, alias = "window_size")]
    pub window_size: Option<usize>,
    #[serde(default)]
    pub k: Option<usize>,
    #[serde(default, alias = "bootstrap_resamples")]
    pub bootstrap_resamples: Option<usize>,
    #[serde(default, alias = "bootstrap_seed")]
    pub bootstrap_seed: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferEntropySampleRequest {
    #[serde(alias = "timestamp", alias = "time")]
    pub t: u64,
    pub value: f32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactiveAffectRequest {
    #[serde(default, alias = "request_id")]
    pub request_id: Option<String>,
    #[serde(default, alias = "idempotency_key")]
    pub idempotency_key: Option<String>,
    #[serde(alias = "learner_id")]
    pub learner_id: String,
    #[serde(default, alias = "session_id")]
    pub session_id: Option<String>,
    #[serde(default, alias = "privacy_class")]
    pub privacy_class: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(alias = "concept_id")]
    pub concept_id: String,
    #[serde(default, alias = "slot_id")]
    pub slot_id: Option<u16>,
    #[serde(alias = "matched_vector")]
    pub matched_vector: Vec<f32>,
    #[serde(alias = "baseline_vector")]
    pub baseline_vector: Vec<f32>,
    #[serde(alias = "current_vector")]
    pub current_vector: Vec<f32>,
    #[serde(default)]
    pub tau: Option<f32>,
    #[serde(
        default = "default_reactive_drift_threshold",
        alias = "drift_threshold"
    )]
    pub drift_threshold: f32,
    pub recurrence: ReactiveRecurrenceRequest,
    pub mmd: ReactiveMmdRequest,
    #[serde(default, alias = "now_millis")]
    pub now_millis: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactiveRecurrenceRequest {
    #[serde(default, alias = "current_occurrences_secs")]
    pub current_occurrences_secs: Vec<u64>,
    #[serde(default, alias = "known_pattern_frequency")]
    pub known_pattern_frequency: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactiveMmdRequest {
    #[serde(default, alias = "baseline_samples")]
    pub baseline_samples: Vec<Vec<f64>>,
    #[serde(default, alias = "recent_samples")]
    pub recent_samples: Vec<Vec<f64>>,
    #[serde(default, alias = "change_point_stream")]
    pub change_point_stream: Vec<Vec<f64>>,
    #[serde(default, alias = "min_window")]
    pub min_window: Option<usize>,
    #[serde(default)]
    pub bandwidth: Option<f64>,
    #[serde(default)]
    pub permutations: Option<usize>,
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(default)]
    pub alpha: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackSpinesRequest {
    #[serde(default, alias = "request_id")]
    pub request_id: Option<String>,
    #[serde(default, alias = "idempotency_key")]
    pub idempotency_key: Option<String>,
    #[serde(alias = "learner_id")]
    pub learner_id: String,
    #[serde(default, alias = "session_id")]
    pub session_id: Option<String>,
    #[serde(default, alias = "privacy_class")]
    pub privacy_class: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub nodes: Vec<TrackNodeRequest>,
    #[serde(default)]
    pub edges: Vec<TrackEdgeRequest>,
    #[serde(default)]
    pub tracks: Vec<TrackRequest>,
    #[serde(default, alias = "mastery_labels")]
    pub mastery_labels: Vec<TrackMasteryLabelRequest>,
    #[serde(default)]
    pub params: TrackSpineParamsRequest,
    #[serde(default, alias = "now_millis")]
    pub now_millis: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackNodeRequest {
    #[serde(alias = "concept_id")]
    pub concept_id: String,
    #[serde(default)]
    pub weight: Option<f32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackEdgeRequest {
    #[serde(alias = "from_concept_id")]
    pub from_concept_id: String,
    #[serde(alias = "to_concept_id")]
    pub to_concept_id: String,
    #[serde(default)]
    pub weight: Option<f32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackRequest {
    #[serde(alias = "track_id")]
    pub track_id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub regions: Vec<TrackRegionRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackRegionRequest {
    #[serde(alias = "region_id")]
    pub region_id: String,
    #[serde(alias = "centroid_concept_id")]
    pub centroid_concept_id: String,
    #[serde(default, alias = "concept_ids")]
    pub concept_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackMasteryLabelRequest {
    #[serde(alias = "concept_id")]
    pub concept_id: String,
    pub mastery: f32,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackSpineParamsRequest {
    #[serde(default)]
    pub max_regions: Option<usize>,
    #[serde(default)]
    pub drill_radius: Option<usize>,
    #[serde(default)]
    pub min_region_size: Option<usize>,
    #[serde(default)]
    pub max_iter: Option<usize>,
    #[serde(default)]
    pub tol: Option<f32>,
    #[serde(default, alias = "decay_lambda")]
    pub decay_lambda: Option<f32>,
}

fn default_grounded() -> bool {
    false
}

fn default_reactive_drift_threshold() -> f32 {
    0.1
}
