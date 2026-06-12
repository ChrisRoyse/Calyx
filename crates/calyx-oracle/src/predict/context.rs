use calyx_core::AnchorValue;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub(super) struct PredictionContext {
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    action_id: Option<String>,
    #[serde(default, rename = "oracle_verdict")]
    oracle_verdict: Option<AnchorEvidence>,
    #[serde(default, rename = "outcome_anchor")]
    outcome_anchor: Option<AnchorEvidence>,
    #[serde(default)]
    consequence: Option<ConsequenceEvidence>,
    #[serde(default)]
    consequences: Vec<ConsequenceEvidence>,
}

impl PredictionContext {
    pub(super) fn matches_action(&self, expected: &str, base_action_match: bool) -> bool {
        self.action_id
            .as_deref()
            .or(self.action.as_deref())
            .map_or(base_action_match, |actual| actual == expected)
    }

    pub(super) fn outcome(&self) -> Option<AnchorValue> {
        self.outcome_anchor
            .as_ref()
            .or(self.oracle_verdict.as_ref())
            .map(|evidence| evidence.value.clone())
    }

    pub(super) fn consequences(&self) -> Vec<ConsequenceSeed> {
        self.consequence
            .iter()
            .chain(self.consequences.iter())
            .filter(|consequence| !consequence.action_or_event.trim().is_empty())
            .map(|consequence| ConsequenceSeed {
                action_or_event: consequence.action_or_event.clone(),
                domain: consequence.domain.clone(),
                outcome: consequence.outcome.value.clone(),
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ConsequenceSeed {
    pub(super) action_or_event: String,
    pub(super) domain: String,
    pub(super) outcome: AnchorValue,
}

#[derive(Clone, Debug, Deserialize)]
struct AnchorEvidence {
    value: AnchorValue,
}

#[derive(Clone, Debug, Deserialize)]
struct ConsequenceEvidence {
    action_or_event: String,
    #[serde(default = "default_consequence_domain")]
    domain: String,
    outcome: AnchorEvidence,
}

fn default_consequence_domain() -> String {
    "oracle".to_string()
}
