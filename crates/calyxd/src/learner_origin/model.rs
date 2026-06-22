use serde::Deserialize;
use serde_json::Value;

pub const ENDPOINT_SIGNALS: &str = "learner_signals_batches";
pub const ENDPOINT_DECIDE: &str = "interventions_decide";
pub const ENDPOINT_OUTCOMES: &str = "intervention_outcomes";

pub const KIND_SIGNAL_BATCH: &str = "learner_signal_batch";
pub const KIND_DECISION: &str = "intervention_decision";
pub const KIND_OUTCOME: &str = "intervention_outcome";

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
