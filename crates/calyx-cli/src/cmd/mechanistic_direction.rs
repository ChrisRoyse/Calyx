use serde::{Deserialize, Serialize};
use serde_json::Value;

mod parse;

use parse::{
    dedup_strings, parse_action_direction, parse_mutation_consequence, parse_trait_effect,
};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MutationConsequence {
    LossOfFunction,
    GainOfFunction,
    DosageLoss,
    DosageGain,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TargetModulation {
    Inhibit,
    Activate,
    ReplaceOrRestore,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TraitEffect {
    Risk,
    Protective,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct MechanisticDirectionEvidence {
    pub status: String,
    pub mutation_consequence: MutationConsequence,
    pub trait_effect: TraitEffect,
    pub required_target_modulation: TargetModulation,
    pub observed_target_modulation: TargetModulation,
    pub reason_codes: Vec<String>,
    pub source_fields: Vec<String>,
}

impl Default for MechanisticDirectionEvidence {
    fn default() -> Self {
        Self {
            status: "direction_missing".to_string(),
            mutation_consequence: MutationConsequence::Unknown,
            trait_effect: TraitEffect::Unknown,
            required_target_modulation: TargetModulation::Unknown,
            observed_target_modulation: TargetModulation::Unknown,
            reason_codes: Vec::new(),
            source_fields: Vec::new(),
        }
    }
}

impl MechanisticDirectionEvidence {
    pub(crate) fn is_required_direction_known(&self) -> bool {
        self.required_target_modulation != TargetModulation::Unknown
            && self.status == "direction_inferred"
    }

    pub(crate) fn is_observed_action_known(&self) -> bool {
        self.observed_target_modulation != TargetModulation::Unknown
            && self.status != "direction_conflict"
    }

    pub(crate) fn required_target_modulation_name(&self) -> Option<String> {
        modulation_name(self.required_target_modulation)
            .filter(|_| self.is_required_direction_known())
            .map(str::to_string)
    }

    pub(crate) fn observed_target_modulation_name(&self) -> Option<String> {
        modulation_name(self.observed_target_modulation)
            .filter(|_| self.is_observed_action_known())
            .map(str::to_string)
    }
}

pub(crate) fn infer_required_target_modulation(row: &Value) -> MechanisticDirectionEvidence {
    let mut evidence = MechanisticDirectionEvidence::default();
    let consequence = parse_mutation_consequence(row, &mut evidence);
    let trait_effect = parse_trait_effect(row, &mut evidence);
    evidence.mutation_consequence = consequence;
    evidence.trait_effect = trait_effect;
    evidence.required_target_modulation = required_modulation(consequence, trait_effect);
    if evidence.required_target_modulation == TargetModulation::Unknown {
        evidence.status = "direction_missing".to_string();
        if consequence == MutationConsequence::Unknown {
            evidence
                .reason_codes
                .push("CALYX_MECH_TARGET_CONSEQUENCE_MISSING".to_string());
        }
        if trait_effect == TraitEffect::Unknown {
            evidence
                .reason_codes
                .push("CALYX_MECH_TRAIT_EFFECT_MISSING".to_string());
        }
    } else {
        evidence.status = "direction_inferred".to_string();
        evidence
            .reason_codes
            .push("CALYX_MECH_REQUIRED_DIRECTION_INFERRED".to_string());
    }
    dedup_strings(&mut evidence.reason_codes);
    dedup_strings(&mut evidence.source_fields);
    evidence
}

pub(crate) fn infer_observed_target_modulation(row: &Value) -> MechanisticDirectionEvidence {
    let mut evidence = MechanisticDirectionEvidence::default();
    let observed = parse_action_direction(row, &mut evidence);
    evidence.observed_target_modulation = observed;
    if observed == TargetModulation::Unknown {
        evidence.status = if evidence
            .reason_codes
            .iter()
            .any(|code| code == "CALYX_MECH_ACTION_DIRECTION_CONFLICT")
        {
            "direction_conflict".to_string()
        } else {
            "direction_missing".to_string()
        };
        if evidence.reason_codes.is_empty() {
            evidence
                .reason_codes
                .push("CALYX_MECH_ACTION_DIRECTION_MISSING".to_string());
        }
    } else {
        evidence.status = "action_direction_inferred".to_string();
        evidence
            .reason_codes
            .push("CALYX_MECH_ACTION_DIRECTION_INFERRED".to_string());
    }
    dedup_strings(&mut evidence.reason_codes);
    dedup_strings(&mut evidence.source_fields);
    evidence
}

pub(crate) fn required_modulation(
    consequence: MutationConsequence,
    trait_effect: TraitEffect,
) -> TargetModulation {
    match (consequence, trait_effect) {
        (
            MutationConsequence::GainOfFunction | MutationConsequence::DosageGain,
            TraitEffect::Risk,
        ) => TargetModulation::Inhibit,
        (
            MutationConsequence::GainOfFunction | MutationConsequence::DosageGain,
            TraitEffect::Protective,
        ) => TargetModulation::Activate,
        (
            MutationConsequence::LossOfFunction | MutationConsequence::DosageLoss,
            TraitEffect::Risk,
        ) => TargetModulation::ReplaceOrRestore,
        (
            MutationConsequence::LossOfFunction | MutationConsequence::DosageLoss,
            TraitEffect::Protective,
        ) => TargetModulation::Inhibit,
        _ => TargetModulation::Unknown,
    }
}

pub(crate) fn modulation_compatible(
    required: TargetModulation,
    observed: TargetModulation,
) -> bool {
    match (required, observed) {
        (TargetModulation::ReplaceOrRestore, TargetModulation::Activate)
        | (TargetModulation::Activate, TargetModulation::ReplaceOrRestore) => true,
        (left, right) => left != TargetModulation::Unknown && left == right,
    }
}

pub(crate) fn modulation_name(value: TargetModulation) -> Option<&'static str> {
    match value {
        TargetModulation::Inhibit => Some("inhibit"),
        TargetModulation::Activate => Some("activate"),
        TargetModulation::ReplaceOrRestore => Some("replace_or_restore"),
        TargetModulation::Unknown => None,
    }
}

pub(crate) fn mutation_consequence_name(value: MutationConsequence) -> Option<&'static str> {
    match value {
        MutationConsequence::LossOfFunction => Some("loss_of_function"),
        MutationConsequence::GainOfFunction => Some("gain_of_function"),
        MutationConsequence::DosageLoss => Some("dosage_loss"),
        MutationConsequence::DosageGain => Some("dosage_gain"),
        MutationConsequence::Unknown => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        MutationConsequence, TargetModulation, TraitEffect, infer_observed_target_modulation,
        infer_required_target_modulation, modulation_compatible,
    };

    #[test]
    fn gof_risk_requires_inhibition() {
        let evidence = infer_required_target_modulation(&json!({
            "directionOnTarget": "Gain of Function",
            "directionOnTrait": "Risk"
        }));
        assert_eq!(
            evidence.mutation_consequence,
            MutationConsequence::GainOfFunction
        );
        assert_eq!(evidence.trait_effect, TraitEffect::Risk);
        assert_eq!(
            evidence.required_target_modulation,
            TargetModulation::Inhibit
        );
    }

    #[test]
    fn lof_risk_requires_replacement_or_restoration() {
        let evidence = infer_required_target_modulation(&json!({
            "direction_on_target": "LoF",
            "direction_on_trait": "risk"
        }));
        assert_eq!(
            evidence.required_target_modulation,
            TargetModulation::ReplaceOrRestore
        );
    }

    #[test]
    fn dosage_sensitivity_requires_expected_modulation() {
        let haplo = infer_required_target_modulation(&json!({
            "dosage_sensitivity": "haploinsufficiency",
            "direction_on_trait": "risk"
        }));
        let triplo = infer_required_target_modulation(&json!({
            "dosageSensitivity": "triplosensitivity",
            "directionOnTrait": "risk"
        }));
        assert_eq!(
            haplo.required_target_modulation,
            TargetModulation::ReplaceOrRestore
        );
        assert_eq!(triplo.required_target_modulation, TargetModulation::Inhibit);
    }

    #[test]
    fn ambiguous_association_text_does_not_imply_trait_direction() {
        let evidence = infer_required_target_modulation(&json!({
            "directionOnTarget": "Gain of Function",
            "directionOnTrait": "association"
        }));
        assert_eq!(evidence.trait_effect, TraitEffect::Unknown);
        assert_eq!(
            evidence.required_target_modulation,
            TargetModulation::Unknown
        );
        assert!(
            evidence
                .reason_codes
                .contains(&"CALYX_MECH_TRAIT_EFFECT_UNRECOGNIZED:directionOnTrait".to_string())
        );
    }

    #[test]
    fn chembl_and_dgidb_action_vocabularies_normalize() {
        let negative = infer_observed_target_modulation(&json!({"action_type": "INHIBITOR"}));
        let positive = infer_observed_target_modulation(&json!({
            "interactionTypes": [{"type": "agonist", "directionality": "activating"}]
        }));
        assert_eq!(
            negative.observed_target_modulation,
            TargetModulation::Inhibit
        );
        assert_eq!(
            positive.observed_target_modulation,
            TargetModulation::Activate
        );
    }

    #[test]
    fn conflicting_action_direction_is_not_accepted() {
        let evidence = infer_observed_target_modulation(&json!({
            "interaction_types": ["inhibitor", "activator"]
        }));
        assert_eq!(evidence.status, "direction_conflict");
        assert_eq!(
            evidence.observed_target_modulation,
            TargetModulation::Unknown
        );
    }

    #[test]
    fn restoration_and_activation_are_compatible_for_lof_risk() {
        assert!(modulation_compatible(
            TargetModulation::ReplaceOrRestore,
            TargetModulation::Activate
        ));
    }
}
