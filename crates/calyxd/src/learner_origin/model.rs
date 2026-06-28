use serde::Deserialize;
use serde_json::Value;

pub const ENDPOINT_SIGNALS: &str = "learner_signals_batches";
pub const ENDPOINT_DECIDE: &str = "interventions_decide";
pub const ENDPOINT_OUTCOMES: &str = "intervention_outcomes";
pub const ENDPOINT_MASTERY_ESTIMATE: &str = "mastery_estimate";

pub const KIND_SIGNAL_BATCH: &str = "learner_signal_batch";
pub const KIND_DECISION: &str = "intervention_decision";
pub const KIND_OUTCOME: &str = "intervention_outcome";
pub const KIND_MASTERY_ESTIMATE: &str = "mastery_estimate";

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
