use calyx_core::AnchorValue;
use serde::Deserialize;

use crate::DomainId;

#[derive(Clone, Debug, Default, Deserialize)]
pub(super) struct ReverseContext {
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    action_id: Option<String>,
    #[serde(default)]
    consequence: Option<EdgeEvidence>,
    #[serde(default)]
    consequences: Vec<EdgeEvidence>,
}

impl ReverseContext {
    pub(super) fn action(&self) -> Option<&str> {
        self.action_id.as_deref().or(self.action.as_deref())
    }

    pub(super) fn edges(&self) -> impl Iterator<Item = &EdgeEvidence> {
        self.consequence.iter().chain(self.consequences.iter())
    }
}

#[derive(Clone, Debug, Deserialize)]
struct AnchorEvidence {
    value: AnchorValue,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct EdgeEvidence {
    #[serde(default, rename = "action_or_event")]
    _action_or_event: String,
    #[serde(default = "default_edge_domain")]
    domain: String,
    outcome: AnchorEvidence,
    #[serde(default = "default_grounded")]
    grounded: bool,
    #[serde(default)]
    provisional: bool,
}

impl EdgeEvidence {
    pub(super) fn matches_answer(&self, answer: &AnchorValue, domain: &DomainId) -> bool {
        self.outcome.value == *answer && self.domain == domain.as_str()
    }

    pub(super) fn domain_id(&self) -> DomainId {
        DomainId::from(self.domain.clone())
    }

    pub(super) fn is_grounded(&self) -> bool {
        self.grounded && !self.provisional
    }
}

fn default_edge_domain() -> String {
    "oracle".to_string()
}

fn default_grounded() -> bool {
    true
}
