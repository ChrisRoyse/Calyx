use serde_json::Value;

use super::model::InputHypothesis;

pub(super) const UNSTRUCTURED_ROW: &str = "CALYX_FALSIFY_UNSTRUCTURED_ROW";

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

#[derive(Clone, Debug)]
pub(super) struct AssertedRelation {
    left: Endpoint,
    right: Endpoint,
}

#[derive(Clone, Debug, Default)]
struct Endpoint {
    ids: Vec<String>,
    names: Vec<String>,
}

pub(super) fn asserted_relation(
    system: &str,
    role: &str,
    row: &Value,
) -> Result<AssertedRelation, &'static str> {
    match (system, role) {
        ("pubtator", _) => endpoints_from_fields(
            row,
            &["left_id", "left_term", "left"],
            &["right_id", "right_term", "right"],
        ),
        ("clinicaltrials", "seed_summaries") => {
            endpoints_from_fields(row, &["intervention"], &["condition"])
        }
        ("clinicaltrials", "trial_rows") => endpoints_from_fields(
            row,
            &["query_intervention", "intervention"],
            &["query_condition", "condition"],
        ),
        ("dgidb", _) => endpoints_from_fields(
            row,
            &["source_overlay_id", "drug", "drug_name"],
            &["target_overlay_id", "gene", "gene_name"],
        ),
        ("open_targets", _) => endpoints_from_fields(
            row,
            &["overlay_target_concepts", "target_id", "target_name"],
            &["overlay_disease_concepts", "disease_id", "disease_name"],
        ),
        _ => Err(UNSTRUCTURED_ROW),
    }
}

pub(super) fn relation_matches(hypothesis: &InputHypothesis, relation: &AssertedRelation) -> bool {
    let forward = endpoint_matches(
        &relation.left,
        &hypothesis.source_name,
        &hypothesis.source_id,
    ) && endpoint_matches(
        &relation.right,
        &hypothesis.target_name,
        &hypothesis.target_id,
    );
    let reverse = endpoint_matches(
        &relation.right,
        &hypothesis.source_name,
        &hypothesis.source_id,
    ) && endpoint_matches(
        &relation.left,
        &hypothesis.target_name,
        &hypothesis.target_id,
    );
    forward || reverse
}

pub(super) fn needs_safety_triage(hypothesis: &InputHypothesis) -> bool {
    hypothesis.source_type == "chemical" || hypothesis.target_type == "chemical"
}

fn has_pair_types(hypothesis: &InputHypothesis, left: &str, right: &str) -> bool {
    (hypothesis.source_type == left && hypothesis.target_type == right)
        || (hypothesis.source_type == right && hypothesis.target_type == left)
}

fn endpoints_from_fields(
    row: &Value,
    left_fields: &[&str],
    right_fields: &[&str],
) -> Result<AssertedRelation, &'static str> {
    let left = endpoint_from_fields(row, left_fields);
    let right = endpoint_from_fields(row, right_fields);
    if left.is_empty() || right.is_empty() {
        return Err(UNSTRUCTURED_ROW);
    }
    Ok(AssertedRelation { left, right })
}

fn endpoint_from_fields(row: &Value, fields: &[&str]) -> Endpoint {
    let mut endpoint = Endpoint::default();
    for field in fields {
        if let Some(value) = row.get(field) {
            endpoint.add_value(field, value);
        }
    }
    endpoint.dedup();
    endpoint
}

impl Endpoint {
    fn add_value(&mut self, field: &str, value: &Value) {
        match value {
            Value::String(text) => self.add_text(field, text),
            Value::Number(number) => self.add_text(field, &number.to_string()),
            Value::Bool(flag) => self.add_text(field, &flag.to_string()),
            Value::Array(items) => {
                for item in items {
                    self.add_value(field, item);
                }
            }
            _ => {}
        }
    }

    fn add_text(&mut self, field: &str, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        if field.contains("id")
            || field.contains("concept")
            || text.contains(':')
            || text.starts_with('@')
        {
            self.ids.push(text.trim().to_string());
            if let Some(label) = label_from_identifier(text) {
                self.names.push(label);
            }
        } else {
            self.names.push(text.trim().to_string());
        }
    }

    fn dedup(&mut self) {
        self.ids.sort();
        self.ids.dedup();
        self.names.sort();
        self.names.dedup();
    }

    fn is_empty(&self) -> bool {
        self.ids.is_empty() && self.names.is_empty()
    }
}

fn endpoint_matches(endpoint: &Endpoint, name: &str, id: &str) -> bool {
    let hypothesis_ids = id_candidates(id);
    let endpoint_ids = endpoint
        .ids
        .iter()
        .flat_map(|value| id_candidates(value))
        .collect::<Vec<_>>();
    if !hypothesis_ids.is_empty()
        && !endpoint_ids.is_empty()
        && hypothesis_ids
            .iter()
            .any(|candidate| endpoint_ids.contains(candidate))
    {
        return true;
    }
    if is_strict_external_id(id) && !endpoint_ids.is_empty() {
        return false;
    }
    endpoint
        .names
        .iter()
        .any(|endpoint_name| name_matches(name, endpoint_name))
}

fn id_candidates(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let whole = normalize(value);
    if whole.len() >= 5 && !whole.chars().all(|ch| ch.is_ascii_digit()) {
        out.push(whole);
    }
    for token in value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(normalize)
        .filter(|token| {
            token.len() >= 5
                && !token.chars().all(|ch| ch.is_ascii_digit())
                && !is_identifier_namespace_token(token)
        })
    {
        out.push(token);
    }
    out.sort();
    out.dedup();
    out
}

fn name_matches(hypothesis_name: &str, endpoint_name: &str) -> bool {
    let hypo_phrase = normalize(hypothesis_name);
    let endpoint_phrase = normalize(endpoint_name);
    if !hypo_phrase.is_empty() && hypo_phrase == endpoint_phrase {
        return true;
    }
    let hypo_tokens = name_tokens(hypothesis_name);
    let endpoint_tokens = name_tokens(endpoint_name);
    !hypo_tokens.is_empty()
        && hypo_tokens
            .iter()
            .all(|token| endpoint_tokens.contains(token))
}

fn name_tokens(value: &str) -> Vec<String> {
    let mut out = value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(normalize)
        .filter(|token| !token.is_empty() && !token.chars().all(|ch| ch.is_ascii_digit()))
        .collect::<Vec<_>>();
    out.sort();
    out.dedup();
    out
}

fn label_from_identifier(value: &str) -> Option<String> {
    let tokens = value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.trim().is_empty())
        .collect::<Vec<_>>();
    if tokens.len() < 2 {
        return None;
    }
    let label = tokens.iter().skip(1).copied().collect::<Vec<_>>().join(" ");
    if label.trim().is_empty() {
        None
    } else {
        Some(label)
    }
}

fn is_strict_external_id(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("concept:") {
        return false;
    }
    [
        "chembl", "hgnc", "mondo", "mesh", "ncbi", "ensembl", "rxnorm", "pubchem", "uniprot",
        "doid", "efo", "orphanet",
    ]
    .iter()
    .any(|prefix| lower.contains(prefix))
}

fn is_identifier_namespace_token(token: &str) -> bool {
    matches!(
        token,
        "concept"
            | "chemical"
            | "disease"
            | "gene"
            | "protein"
            | "target"
            | "chembl"
            | "hgnc"
            | "mondo"
            | "mesh"
            | "ncbi"
            | "ensembl"
            | "rxnorm"
            | "pubchem"
            | "uniprot"
            | "doid"
            | "efo"
            | "orphanet"
    )
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
