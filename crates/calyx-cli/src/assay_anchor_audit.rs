use calyx_core::CalyxError;
use serde::{Deserialize, Serialize};

use crate::error::{CliError, CliResult};

pub(crate) const CALYX_FSV_ASSAY_TRIVIAL_ANCHOR: &str = "CALYX_FSV_ASSAY_TRIVIAL_ANCHOR";

const TRIVIAL_ANCHOR_REMEDIATION: &str = "use a validity-audited non-linguistic outcome anchor; leaked/trivial labels may be measured as controls but cannot satisfy grounded gates";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AnchorAudit {
    #[serde(default)]
    pub(crate) anchor_leaks_into_input: bool,
    #[serde(default)]
    pub(crate) trivial_anchor: bool,
    #[serde(default = "default_grounded_gate_eligible")]
    pub(crate) grounded_gate_eligible: bool,
    #[serde(default)]
    pub(crate) label_recoverable_from_input: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) audit_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) label_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) embedded_text_fields: Vec<String>,
}

impl Default for AnchorAudit {
    /// Fail-closed baseline (#1140): absence of an audit is not evidence of
    /// eligibility. Gate eligibility must be claimed explicitly by the row or
    /// report producer.
    fn default() -> Self {
        Self {
            anchor_leaks_into_input: false,
            trivial_anchor: false,
            grounded_gate_eligible: false,
            label_recoverable_from_input: false,
            audit_kind: None,
            source: None,
            reason: None,
            label_fields: Vec::new(),
            embedded_text_fields: Vec::new(),
        }
    }
}

impl AnchorAudit {
    pub(crate) fn gdelt_country_text_leak() -> Self {
        Self {
            anchor_leaks_into_input: true,
            trivial_anchor: true,
            grounded_gate_eligible: false,
            label_recoverable_from_input: true,
            audit_kind: Some("source_text_label_overlap".to_string()),
            source: Some("calyx assay gdelt-rows".to_string()),
            reason: Some(
                "GDELT positive label is computed from actor/action country fields that are embedded verbatim in the measured text"
                    .to_string(),
            ),
            label_fields: vec![
                "gdelt_actor1_country".to_string(),
                "gdelt_actor2_country".to_string(),
                "gdelt_action_geo_country".to_string(),
                "gdelt_action_geo_fullname".to_string(),
            ],
            embedded_text_fields: vec![
                "Actor1 country".to_string(),
                "Actor2 country".to_string(),
                "ActionGeo country".to_string(),
                "ActionGeo fullname".to_string(),
            ],
        }
    }

    pub(crate) fn from_parts(
        audit: Option<Self>,
        anchor_leaks_into_input: Option<bool>,
        trivial_anchor: Option<bool>,
        grounded_gate_eligible: Option<bool>,
    ) -> Self {
        let missing = audit.is_none()
            && anchor_leaks_into_input.is_none()
            && trivial_anchor.is_none()
            && grounded_gate_eligible.is_none();
        let mut out = audit.unwrap_or_default();
        if let Some(value) = anchor_leaks_into_input {
            out.anchor_leaks_into_input = value;
        }
        if let Some(value) = trivial_anchor {
            out.trivial_anchor = value;
        }
        if let Some(value) = grounded_gate_eligible {
            out.grounded_gate_eligible = value;
        }
        if missing {
            // #1140 fail-closed: no audit information at all — record why the
            // gate refuses so readbacks are self-explaining.
            out.audit_kind = Some("missing".to_string());
            out.source = Some("calyx anchor-audit fail-closed default".to_string());
            out.reason = Some(
                "anchor audit absent from source rows/report; gate eligibility requires an explicit affirmative audit"
                    .to_string(),
            );
        }
        if out.anchor_leaks_into_input || out.trivial_anchor || out.label_recoverable_from_input {
            out.grounded_gate_eligible = false;
        }
        out
    }

    pub(crate) fn merge_rows<'a>(audits: impl IntoIterator<Item = &'a AnchorAudit>) -> Self {
        let mut out = Self::default();
        let mut merged_any = false;
        let mut all_eligible = true;
        for audit in audits {
            merged_any = true;
            out.anchor_leaks_into_input |= audit.anchor_leaks_into_input;
            out.trivial_anchor |= audit.trivial_anchor;
            out.label_recoverable_from_input |= audit.label_recoverable_from_input;
            all_eligible &= audit.grounded_gate_eligible;
            if out.audit_kind.is_none() {
                out.audit_kind = audit.audit_kind.clone();
            }
            if out.source.is_none() {
                out.source = audit.source.clone();
            }
            if out.reason.is_none() {
                out.reason = audit.reason.clone();
            }
            extend_unique(&mut out.label_fields, &audit.label_fields);
            extend_unique(&mut out.embedded_text_fields, &audit.embedded_text_fields);
        }
        out.grounded_gate_eligible = merged_any && all_eligible;
        if !merged_any {
            // #1140 fail-closed: nothing to merge — say so in the readback.
            out.audit_kind = Some("missing".to_string());
            out.source = Some("calyx anchor-audit fail-closed default".to_string());
            out.reason = Some(
                "no anchor audits present in source rows; gate eligibility requires explicit affirmative audits"
                    .to_string(),
            );
        }
        if out.anchor_leaks_into_input || out.trivial_anchor || out.label_recoverable_from_input {
            out.grounded_gate_eligible = false;
        }
        out
    }

    pub(crate) fn require_gate_eligible(&self, gate: &'static str) -> CliResult {
        if self.grounded_gate_eligible
            && !self.anchor_leaks_into_input
            && !self.trivial_anchor
            && !self.label_recoverable_from_input
        {
            return Ok(());
        }
        Err(CliError::Calyx(CalyxError {
            code: CALYX_FSV_ASSAY_TRIVIAL_ANCHOR,
            message: format!(
                "{gate} refused bits report: anchor_leaks_into_input={} trivial_anchor={} label_recoverable_from_input={} grounded_gate_eligible={} reason={}",
                self.anchor_leaks_into_input,
                self.trivial_anchor,
                self.label_recoverable_from_input,
                self.grounded_gate_eligible,
                self.reason.as_deref().unwrap_or("not provided")
            ),
            remediation: TRIVIAL_ANCHOR_REMEDIATION,
        }))
    }
}

fn default_grounded_gate_eligible() -> bool {
    // #1140 fail-closed: an audit object that does not state eligibility is
    // not eligible.
    false
}

fn extend_unique(target: &mut Vec<String>, values: &[String]) {
    for value in values {
        if !target.iter().any(|existing| existing == value) {
            target.push(value.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AnchorAudit;

    fn affirmative_eligible() -> AnchorAudit {
        AnchorAudit {
            grounded_gate_eligible: true,
            audit_kind: Some("unit_affirmative".to_string()),
            source: Some("unit test".to_string()),
            reason: Some("audited eligible".to_string()),
            ..AnchorAudit::default()
        }
    }

    #[test]
    fn missing_audit_fails_closed_with_reason() {
        let audit = AnchorAudit::from_parts(None, None, None, None);
        assert!(!audit.grounded_gate_eligible);
        assert_eq!(audit.audit_kind.as_deref(), Some("missing"));
        assert!(audit.require_gate_eligible("unit gate").is_err());
    }

    #[test]
    fn explicit_eligibility_is_honored() {
        let audit = AnchorAudit::from_parts(Some(affirmative_eligible()), None, None, None);
        assert!(audit.grounded_gate_eligible);
        assert!(audit.require_gate_eligible("unit gate").is_ok());
    }

    #[test]
    fn leak_flags_clamp_explicit_eligibility() {
        let audit =
            AnchorAudit::from_parts(Some(affirmative_eligible()), Some(true), None, Some(true));
        assert!(!audit.grounded_gate_eligible);
    }

    #[test]
    fn merge_of_no_rows_fails_closed() {
        let audit = AnchorAudit::merge_rows([]);
        assert!(!audit.grounded_gate_eligible);
        assert_eq!(audit.audit_kind.as_deref(), Some("missing"));
    }

    #[test]
    fn merge_requires_every_row_eligible() {
        let eligible = affirmative_eligible();
        let ineligible = AnchorAudit::from_parts(None, None, None, None);
        let merged = AnchorAudit::merge_rows([&eligible, &ineligible]);
        assert!(!merged.grounded_gate_eligible);
        let merged_all = AnchorAudit::merge_rows([&eligible, &eligible]);
        assert!(merged_all.grounded_gate_eligible);
    }

    #[test]
    fn serde_defaults_are_fail_closed() {
        let audit: AnchorAudit = serde_json::from_str("{}").unwrap();
        assert!(!audit.grounded_gate_eligible);
    }
}
