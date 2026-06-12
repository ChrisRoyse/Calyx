use calyx_core::AnchorValue;
use serde::Deserialize;

use super::ChildCandidate;
use crate::DomainId;

#[derive(Clone, Debug, Deserialize)]
pub(super) struct ExpansionContext {
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    action_id: Option<String>,
    #[serde(default)]
    consequence: Option<EdgeEvidence>,
    #[serde(default)]
    consequences: Vec<EdgeEvidence>,
}

impl ExpansionContext {
    pub(super) fn matches_action(&self, expected: &str, base_action_match: bool) -> bool {
        self.action_id
            .as_deref()
            .or(self.action.as_deref())
            .map_or(base_action_match, |actual| actual == expected)
    }

    pub(super) fn consequences(&self) -> Vec<ChildCandidate> {
        self.consequence
            .iter()
            .chain(self.consequences.iter())
            .filter(|edge| !edge.action_or_event.trim().is_empty())
            .map(|edge| ChildCandidate {
                action_or_event: edge.action_or_event.clone(),
                domain: DomainId::from(edge.domain.clone()),
                outcome: edge.outcome.value.clone(),
                grounded: edge.grounded && !edge.provisional,
            })
            .collect()
    }
}

#[derive(Clone, Debug, Deserialize)]
struct AnchorEvidence {
    value: AnchorValue,
}

#[derive(Clone, Debug, Deserialize)]
struct EdgeEvidence {
    action_or_event: String,
    #[serde(default = "default_consequence_domain")]
    domain: String,
    outcome: AnchorEvidence,
    #[serde(default = "default_grounded")]
    grounded: bool,
    #[serde(default)]
    provisional: bool,
}

fn default_consequence_domain() -> String {
    "oracle".to_string()
}

fn default_grounded() -> bool {
    true
}
