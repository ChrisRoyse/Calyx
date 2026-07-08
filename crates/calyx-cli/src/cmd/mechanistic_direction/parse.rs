use serde_json::Value;

use super::{MechanisticDirectionEvidence, MutationConsequence, TargetModulation, TraitEffect};

pub(super) fn parse_mutation_consequence(
    row: &Value,
    evidence: &mut MechanisticDirectionEvidence,
) -> MutationConsequence {
    let fields = [
        "direction_on_target",
        "directionOnTarget",
        "target_direction",
        "target_effect",
        "mutation_consequence",
        "mutational_consequence",
        "variant_functional_consequence",
        "variantFunctionalConsequence",
        "functional_consequence",
        "mechanism",
        "disease_mechanism",
        "dosage_sensitivity",
        "dosageSensitivity",
    ];
    let mut parsed = Vec::new();
    for field in fields {
        for text in text_values(row.get(field)) {
            evidence.source_fields.push(field.to_string());
            match mutation_consequence_from_text(&text) {
                Some(value) => parsed.push(value),
                None => evidence.reason_codes.push(format!(
                    "CALYX_MECH_TARGET_CONSEQUENCE_UNRECOGNIZED:{field}"
                )),
            }
        }
    }
    unique_or_unknown(parsed, evidence, "CALYX_MECH_TARGET_CONSEQUENCE_CONFLICT")
}

pub(super) fn parse_trait_effect(
    row: &Value,
    evidence: &mut MechanisticDirectionEvidence,
) -> TraitEffect {
    let fields = [
        "direction_on_trait",
        "directionOnTrait",
        "trait_effect",
        "traitEffect",
        "disease_effect",
        "clinical_significance",
        "clinicalSignificance",
    ];
    let mut parsed = Vec::new();
    for field in fields {
        for text in text_values(row.get(field)) {
            evidence.source_fields.push(field.to_string());
            match trait_effect_from_text(&text) {
                Some(value) => parsed.push(value),
                None => evidence
                    .reason_codes
                    .push(format!("CALYX_MECH_TRAIT_EFFECT_UNRECOGNIZED:{field}")),
            }
        }
    }
    if parsed.is_empty() {
        if let Some(value) = signed_number(row, "beta") {
            evidence.source_fields.push("beta".to_string());
            parsed.push(if value < 0.0 {
                TraitEffect::Protective
            } else if value > 0.0 {
                TraitEffect::Risk
            } else {
                TraitEffect::Unknown
            });
        }
        if let Some(value) =
            signed_number(row, "odds_ratio").or_else(|| signed_number(row, "oddsRatio"))
        {
            evidence.source_fields.push("odds_ratio".to_string());
            parsed.push(if value < 1.0 {
                TraitEffect::Protective
            } else if value > 1.0 {
                TraitEffect::Risk
            } else {
                TraitEffect::Unknown
            });
        }
    }
    parsed.retain(|value| *value != TraitEffect::Unknown);
    unique_or_unknown(parsed, evidence, "CALYX_MECH_TRAIT_EFFECT_CONFLICT")
}

pub(super) fn parse_action_direction(
    row: &Value,
    evidence: &mut MechanisticDirectionEvidence,
) -> TargetModulation {
    let fields = [
        "action_type",
        "actionType",
        "interaction_type",
        "interaction_types",
        "interactionTypes",
        "directionality",
        "directionalities",
        "relation",
        "moa",
        "mechanism_of_action",
        "mechanismOfAction",
    ];
    let mut parsed = Vec::new();
    for field in fields {
        for text in text_values(row.get(field)) {
            evidence.source_fields.push(field.to_string());
            if let Some(value) = action_direction_from_text(&text) {
                parsed.push(value);
            }
        }
    }
    unique_or_unknown(parsed, evidence, "CALYX_MECH_ACTION_DIRECTION_CONFLICT")
}

fn mutation_consequence_from_text(text: &str) -> Option<MutationConsequence> {
    let value = normalized_token(text);
    if value.is_empty() {
        return None;
    }
    if contains_any(
        &value,
        &[
            "lossoffunction",
            "lossfunction",
            "lof",
            "reducedfunction",
            "decreasedfunction",
            "inactivating",
            "nullvariant",
        ],
    ) {
        Some(MutationConsequence::LossOfFunction)
    } else if contains_any(
        &value,
        &[
            "gainoffunction",
            "gainfunction",
            "gof",
            "increasedfunction",
            "activatingmutation",
            "activatingvariant",
        ],
    ) {
        Some(MutationConsequence::GainOfFunction)
    } else if contains_any(
        &value,
        &[
            "haploinsufficiency",
            "haploinsufficient",
            "dosageloss",
            "deletion",
            "copyloss",
        ],
    ) {
        Some(MutationConsequence::DosageLoss)
    } else if contains_any(
        &value,
        &[
            "triplosensitivity",
            "triplosensitive",
            "dosagegain",
            "duplication",
            "copygain",
        ],
    ) {
        Some(MutationConsequence::DosageGain)
    } else {
        None
    }
}

fn trait_effect_from_text(text: &str) -> Option<TraitEffect> {
    let value = normalized_token(text);
    if value.is_empty() {
        return None;
    }
    if contains_any(
        &value,
        &[
            "risk",
            "pathogenic",
            "likelypathogenic",
            "establishedriskallele",
            "riskfactor",
            "predisposing",
        ],
    ) {
        Some(TraitEffect::Risk)
    } else if contains_any(&value, &["protective", "protection"]) {
        Some(TraitEffect::Protective)
    } else {
        None
    }
}

fn action_direction_from_text(text: &str) -> Option<TargetModulation> {
    let value = normalized_token(text);
    if value.is_empty() {
        return None;
    }
    if contains_any(
        &value,
        &[
            "exogenousgene",
            "exogenousprotein",
            "replacement",
            "restore",
            "restoration",
            "supplement",
        ],
    ) {
        Some(TargetModulation::ReplaceOrRestore)
    } else if contains_any(
        &value,
        &[
            "inhibitor",
            "inhibiting",
            "inhibitory",
            "antagonist",
            "blocker",
            "degrader",
            "negativemodulator",
            "negativeallostericmodulator",
            "inverseagonist",
            "rnaiinhibitor",
            "antisenseinhibitor",
            "geneditingnegativemodulator",
            "downregulator",
            "suppressor",
        ],
    ) {
        Some(TargetModulation::Inhibit)
    } else if contains_any(
        &value,
        &[
            "activator",
            "activating",
            "agonist",
            "positivemodulator",
            "positiveallostericmodulator",
            "opener",
            "partialagonist",
            "upregulator",
            "stimulator",
        ],
    ) {
        Some(TargetModulation::Activate)
    } else {
        None
    }
}

fn unique_or_unknown<T>(
    mut values: Vec<T>,
    evidence: &mut MechanisticDirectionEvidence,
    conflict_code: &str,
) -> T
where
    T: Copy + Default + Ord + PartialEq,
{
    values.sort();
    values.dedup();
    match values.as_slice() {
        [value] => *value,
        [] => T::default(),
        _ => {
            evidence.reason_codes.push(conflict_code.to_string());
            T::default()
        }
    }
}

fn text_values(value: Option<&Value>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(value) = value {
        collect_text_values(value, &mut out);
    }
    out
}

fn collect_text_values(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) if !text.trim().is_empty() => {
            out.push(text.trim().to_string());
        }
        Value::Number(number) => out.push(number.to_string()),
        Value::Array(values) => {
            for value in values {
                collect_text_values(value, out);
            }
        }
        Value::Object(map) => {
            for key in [
                "type",
                "directionality",
                "action_type",
                "value",
                "label",
                "term",
            ] {
                if let Some(value) = map.get(key) {
                    collect_text_values(value, out);
                }
            }
        }
        _ => {}
    }
}

fn signed_number(row: &Value, field: &str) -> Option<f64> {
    row.get(field).and_then(|raw| {
        raw.as_f64()
            .or_else(|| raw.as_i64().map(|value| value as f64))
            .or_else(|| raw.as_str().and_then(|text| text.parse::<f64>().ok()))
    })
}

fn normalized_token(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

pub(super) fn dedup_strings(values: &mut Vec<String>) {
    values.sort();
    values.dedup();
}
