use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::super::model::{EvidenceGraphDraft, concept_node, normalize_key, simple_node};
use super::{
    RootIndex, SourceLoadReport, add_row_node, array_strings, array_values, bool_field, edge_meta,
    field_or, ingest_generic_unhandled_jsonl, link_raw_path, read_json_file, read_jsonl_rows,
    str_field,
};
use crate::error::CliResult;

pub(super) fn ingest(
    index: &RootIndex,
    draft: &mut EvidenceGraphDraft,
    report: &mut SourceLoadReport,
) -> CliResult {
    let mut handled = BTreeSet::new();
    ingest_seed_summaries(index, draft, report, &mut handled)?;
    ingest_trials(index, draft, report, &mut handled)?;
    ingest_outcomes(index, draft)?;
    ingest_generic_unhandled_jsonl(index, draft, report, &handled)
}

fn ingest_seed_summaries(
    index: &RootIndex,
    draft: &mut EvidenceGraphDraft,
    report: &mut SourceLoadReport,
    handled: &mut BTreeSet<String>,
) -> CliResult {
    let rel = "parsed/clinicaltrials_seed_summaries.jsonl";
    handled.insert(rel.to_string());
    for row in read_jsonl_rows(index, rel, report)? {
        let row_key = add_row_node(index, draft, rel, row.line, "trial_seed_summary", &row)?;
        link_raw_path(index, draft, &row_key, &row.value)?;
        let drug = concept_node(
            draft,
            None,
            &field_or(&row.value, "intervention", "intervention"),
            "drug",
        );
        let condition = concept_node(
            draft,
            None,
            &field_or(&row.value, "condition", "condition"),
            "disease",
        );
        draft.add_edge(
            &drug,
            "clinical_trial_seed",
            &row_key,
            edge_meta(&row, "clinicaltrials"),
        );
        draft.add_edge(
            &row_key,
            "observed_in",
            &condition,
            edge_meta(&row, "clinicaltrials"),
        );
        draft.add_edge(
            &drug,
            "clinical_trial_seed_association",
            &condition,
            edge_meta(&row, "clinicaltrials"),
        );
    }
    Ok(())
}

fn ingest_trials(
    index: &RootIndex,
    draft: &mut EvidenceGraphDraft,
    report: &mut SourceLoadReport,
    handled: &mut BTreeSet<String>,
) -> CliResult {
    let rel = "parsed/clinicaltrials_trial_rows.jsonl";
    handled.insert(rel.to_string());
    for row in read_jsonl_rows(index, rel, report)? {
        let row_key = add_row_node(index, draft, rel, row.line, "trial_row", &row)?;
        link_raw_path(index, draft, &row_key, &row.value)?;
        let drug = concept_node(
            draft,
            None,
            &field_or(&row.value, "query_intervention", "intervention"),
            "drug",
        );
        let condition = concept_node(
            draft,
            None,
            &field_or(&row.value, "query_condition", "condition"),
            "disease",
        );
        let nct = field_or(&row.value, "nct_id", "unknown_nct");
        let trial = simple_node(draft, "trial", "nct", &nct, &nct);
        draft.add_edge(
            &drug,
            "tested_in_trial_row",
            &row_key,
            edge_meta(&row, "clinicaltrials"),
        );
        draft.add_edge(
            &row_key,
            "trial_row_condition",
            &condition,
            edge_meta(&row, "clinicaltrials"),
        );
        draft.add_edge(
            &row_key,
            "observed_in_trial",
            &trial,
            edge_meta(&row, "clinicaltrials"),
        );
        draft.add_edge(
            &drug,
            "clinical_trial_association",
            &condition,
            edge_meta(&row, "clinicaltrials"),
        );
        if bool_field(&row.value, "has_results").unwrap_or(false) {
            let result = simple_node(
                draft,
                "result_state",
                "trial_result_state",
                "has_results",
                "has_results",
            );
            draft.add_edge(&trial, "has_result_state", &result, BTreeMap::new());
        }
        if let Some(status) = str_field(&row.value, "overall_status") {
            let status_node = simple_node(draft, "trial_status", "trial_status", &status, &status);
            draft.add_edge(&trial, "has_trial_status", &status_node, BTreeMap::new());
            if is_stopped_status(&status) || str_field(&row.value, "why_stopped").is_some() {
                draft.add_edge(
                    &drug,
                    "negative_or_caution_trial_signal",
                    &row_key,
                    edge_meta(&row, "clinicaltrials"),
                );
                draft.add_edge(
                    &row_key,
                    "cautions_against",
                    &condition,
                    edge_meta(&row, "clinicaltrials"),
                );
                draft.record_path(
                    "clinicaltrials_negative",
                    vec![drug.clone(), row_key.clone(), condition.clone()],
                );
            }
        }
        for phase in array_strings(&row.value, "phases") {
            let phase_node = simple_node(draft, "trial_phase", "trial_phase", &phase, &phase);
            draft.add_edge(&trial, "has_trial_phase", &phase_node, BTreeMap::new());
        }
        if let Some(sponsor) = str_field(&row.value, "lead_sponsor_name") {
            let sponsor_node = simple_node(draft, "sponsor", "sponsor", &sponsor, &sponsor);
            draft.add_edge(&trial, "sponsored_by", &sponsor_node, BTreeMap::new());
        }
        draft.record_path(
            "clinicaltrials_positive",
            vec![drug, row_key, trial, condition],
        );
    }
    Ok(())
}

fn ingest_outcomes(index: &RootIndex, draft: &mut EvidenceGraphDraft) -> CliResult {
    for rel in index.artifacts.keys().filter(|rel| rel.starts_with("raw/")) {
        let raw = read_json_file(&index.root.join(rel))?;
        for study in array_values(&raw, "studies") {
            let Some(nct) = study
                .pointer("/protocolSection/identificationModule/nctId")
                .and_then(Value::as_str)
            else {
                continue;
            };
            let trial = simple_node(draft, "trial", "nct", nct, nct);
            for (kind, pointer) in [
                ("primary", "/protocolSection/outcomesModule/primaryOutcomes"),
                (
                    "secondary",
                    "/protocolSection/outcomesModule/secondaryOutcomes",
                ),
                ("other", "/protocolSection/outcomesModule/otherOutcomes"),
                (
                    "results",
                    "/resultsSection/outcomeMeasuresModule/outcomeMeasures",
                ),
            ] {
                let Some(outcomes) = study.pointer(pointer).and_then(Value::as_array) else {
                    continue;
                };
                for (idx, outcome) in outcomes.iter().enumerate() {
                    let measure = outcome_measure(outcome);
                    if measure.is_empty() {
                        continue;
                    }
                    let outcome_key = draft.add_node(
                        format!(
                            "outcome:clinicaltrials:{nct}:{kind}:{idx}:{}",
                            normalize_key(&measure)
                        ),
                        "outcome",
                        &measure,
                        BTreeMap::from([
                            ("family".to_string(), index.family.clone()),
                            ("nct_id".to_string(), nct.to_string()),
                            ("outcome_kind".to_string(), kind.to_string()),
                            ("raw_path".to_string(), rel.clone()),
                        ]),
                    );
                    let instrument = simple_node(
                        draft,
                        "measurement_instrument",
                        "clinical_measure",
                        &measure,
                        &measure,
                    );
                    draft.add_edge(&trial, "measures_outcome", &outcome_key, BTreeMap::new());
                    draft.add_edge(&outcome_key, "measured_with", &instrument, BTreeMap::new());
                    if let Some(artifact) = index.artifacts.get(rel) {
                        draft.add_edge(
                            &outcome_key,
                            "derived_from",
                            &artifact.stable_key,
                            BTreeMap::new(),
                        );
                    }
                    draft.record_path(
                        "clinicaltrials_outcome",
                        vec![trial.clone(), outcome_key, instrument],
                    );
                }
            }
        }
    }
    Ok(())
}

fn outcome_measure(value: &Value) -> String {
    str_field(value, "measure")
        .or_else(|| str_field(value, "title"))
        .unwrap_or_default()
}

fn is_stopped_status(status: &str) -> bool {
    matches!(
        status,
        "TERMINATED" | "WITHDRAWN" | "SUSPENDED" | "NO_LONGER_AVAILABLE"
    )
}
