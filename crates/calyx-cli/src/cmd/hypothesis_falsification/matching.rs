use serde_json::Value;

use super::model::InputHypothesis;

pub(super) fn source_applicable(system: &str, hypothesis: &InputHypothesis) -> bool {
    match system {
        "clinicaltrials" => has_pair_types(hypothesis, "chemical", "disease"),
        "dgidb" => {
            has_pair_types(hypothesis, "chemical", "gene")
                || has_pair_types(hypothesis, "chemical", "gene_protein")
        }
        "open_targets" => {
            has_pair_types(hypothesis, "gene", "disease")
                || has_pair_types(hypothesis, "gene_protein", "disease")
        }
        _ => true,
    }
}

pub(super) fn pair_matches(hypothesis: &InputHypothesis, row: &Value) -> bool {
    let Ok(text) = serde_json::to_string(row) else {
        return false;
    };
    let normalized = normalize(&text);
    entity_matches(&normalized, &hypothesis.source_name, &hypothesis.source_id)
        && entity_matches(&normalized, &hypothesis.target_name, &hypothesis.target_id)
}

pub(super) fn needs_safety_triage(hypothesis: &InputHypothesis) -> bool {
    hypothesis.source_type == "chemical" || hypothesis.target_type == "chemical"
}

fn has_pair_types(hypothesis: &InputHypothesis, left: &str, right: &str) -> bool {
    (hypothesis.source_type == left && hypothesis.target_type == right)
        || (hypothesis.source_type == right && hypothesis.target_type == left)
}

fn entity_matches(text: &str, name: &str, id: &str) -> bool {
    let name_norm = normalize(name);
    if name_norm.len() >= 3 && text.contains(&name_norm) {
        return true;
    }
    let id_tail = id.rsplit(':').next().map(normalize).unwrap_or_default();
    if id_tail.len() >= 3 && text.contains(&id_tail) {
        return true;
    }
    let tokens = name
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(normalize)
        .filter(|token| token.len() >= 3)
        .collect::<Vec<_>>();
    !tokens.is_empty() && tokens.iter().all(|token| text.contains(token))
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
