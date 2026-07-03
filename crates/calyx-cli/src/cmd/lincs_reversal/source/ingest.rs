use std::collections::BTreeMap;

use serde_json::Value;

use super::LincsSourceReport;
use super::root::{RootIndex, link_artifact, link_artifact_field};
use super::rows::{
    add_row_node, fields, nested_array_len, read_jsonl_rows, required_field, row_meta, str_field,
};
use crate::cmd::lincs_reversal::model::{LincsGraphDraft, clean_label, normalize_key};
use crate::error::CliResult;

pub(super) fn ingest_disease_signatures(
    index: &RootIndex,
    draft: &mut LincsGraphDraft,
    report: &mut LincsSourceReport,
) -> CliResult {
    let rel = "parsed/disease_signature_inputs.jsonl";
    for row in read_jsonl_rows(index, rel, report)? {
        let row_key = add_row_node(index, draft, rel, "creeds_disease_signature_row", &row)?;
        let signature = signature_node(draft, &row.value)?;
        let disease = disease_node(draft, &row.value);
        draft.add_edge(&disease, "has_creeds_signature", &signature, row_meta(&row));
        draft.add_edge(&signature, "derived_from", &row_key, row_meta(&row));
        link_source_file(index, draft, &signature, &row.value)?;
        link_optional_concepts(draft, &signature, &disease, &row.value);
    }
    Ok(())
}

pub(super) fn ingest_request_records(
    index: &RootIndex,
    draft: &mut LincsGraphDraft,
    report: &mut LincsSourceReport,
) -> CliResult {
    let rel = "parsed/l1000cds2_request_records.jsonl";
    for row in read_jsonl_rows(index, rel, report)? {
        let row_key = add_row_node(index, draft, rel, "l1000cds2_request_row", &row)?;
        let signature = signature_node(draft, &row.value)?;
        let query = query_node(draft, &row.value);
        draft.add_edge(
            &signature,
            "queried_against_l1000cds2",
            &query,
            row_meta(&row),
        );
        draft.add_edge(&query, "derived_from", &row_key, row_meta(&row));
        draft.add_edge(
            &query,
            "queried_source",
            "source:l1000cds2",
            BTreeMap::new(),
        );
        link_artifact_field(
            index,
            draft,
            &query,
            &row.value,
            "request_path",
            "used_request",
        )?;
        link_artifact_field(
            index,
            draft,
            &query,
            &row.value,
            "response_path",
            "produced_response",
        )?;
    }
    Ok(())
}

pub(super) fn ingest_scores(
    index: &RootIndex,
    draft: &mut LincsGraphDraft,
    report: &mut LincsSourceReport,
) -> CliResult {
    let rel = "parsed/lincs_reversal_scores.jsonl";
    for row in read_jsonl_rows(index, rel, report)? {
        let row_key = add_row_node(index, draft, rel, "lincs_reversal_score_row", &row)?;
        let signature = signature_node(draft, &row.value)?;
        let query = query_node(draft, &row.value);
        let score = score_node(draft, &row.value, &row.sha256);
        let perturbation = perturbation_node(draft, &row.value);
        let cell = optional_node(draft, "cell_line", "cell_line", &row.value, "cell_id");
        let lincs_sig = optional_node(
            draft,
            "lincs_signature",
            "lincs_signature",
            &row.value,
            "sig_id",
        );
        draft.add_edge(
            &signature,
            "has_lincs_reversal_score",
            &score,
            score_meta(&row.value),
        );
        draft.add_edge(&query, "returned_reversal_score", &score, row_meta(&row));
        draft.add_edge(
            &score,
            "scores_perturbation",
            &perturbation,
            score_meta(&row.value),
        );
        draft.add_edge(&score, "derived_from", &row_key, row_meta(&row));
        if let Some(cell) = cell {
            draft.add_edge(&score, "measured_in_cell_line", &cell, BTreeMap::new());
        }
        if let Some(lincs_sig) = lincs_sig {
            draft.add_edge(&score, "has_lincs_signature", &lincs_sig, BTreeMap::new());
        }
        link_chemical_ids(draft, &perturbation, &row.value);
        link_matched_candidates(draft, &score, &row.value);
        record_score_paths(draft, &signature, &query, &score, &perturbation, &row.value);
    }
    Ok(())
}

pub(super) fn ingest_unsupported(
    index: &RootIndex,
    draft: &mut LincsGraphDraft,
    report: &mut LincsSourceReport,
) -> CliResult {
    let rel = "parsed/unsupported_unmapped_cases.jsonl";
    for row in read_jsonl_rows(index, rel, report)? {
        let row_key = add_row_node(index, draft, rel, "unsupported_lincs_candidate_row", &row)?;
        let signature = signature_node(draft, &row.value)?;
        let candidate = candidate_node(draft, &row.value);
        let case = unsupported_node(draft, &row.value, &row.sha256);
        draft.add_edge(&case, "against_signature", &signature, row_meta(&row));
        draft.add_edge(&case, "unsupported_candidate", &candidate, row_meta(&row));
        draft.add_edge(&case, "derived_from", &row_key, row_meta(&row));
        draft.add_edge(
            &candidate,
            "absent_from_l1000cds2_top50_reverse_results",
            &signature,
            row_meta(&row),
        );
        draft.record_path(
            "unsupported_current_candidate_absence",
            vec![candidate, case, signature],
        );
    }
    Ok(())
}

fn signature_node(draft: &mut LincsGraphDraft, value: &Value) -> CliResult<String> {
    let signature_id = required_field(value, "signature_id")?;
    let disease = str_field(value, "disease_name").unwrap_or_else(|| "unknown disease".to_string());
    let mut metadata = fields(
        value,
        &[
            "signature_id",
            "disease_name",
            "creeds_family",
            "geo_id",
            "do_id",
            "umls_cui",
            "cell_type",
            "platform",
            "version",
            "input_up_gene_count",
            "input_down_gene_count",
            "raw_up_gene_count",
            "raw_down_gene_count",
        ],
    );
    metadata.insert("source".to_string(), "CREEDS".to_string());
    Ok(draft.add_node(
        format!("disease_signature:{signature_id}"),
        "disease_signature",
        format!("{signature_id} {disease}"),
        metadata,
    ))
}

fn disease_node(draft: &mut LincsGraphDraft, value: &Value) -> String {
    let disease = str_field(value, "disease_name").unwrap_or_else(|| "unknown disease".to_string());
    draft.add_node(
        format!("disease:{}", normalize_key(&disease)),
        "disease",
        disease.clone(),
        BTreeMap::from([("name".to_string(), clean_label(&disease))]),
    )
}

fn query_node(draft: &mut LincsGraphDraft, value: &Value) -> String {
    let signature = str_field(value, "signature_id").unwrap_or_else(|| "unknown".to_string());
    let mut metadata = fields(
        value,
        &[
            "signature_id",
            "status_code",
            "top_meta_count",
            "share_id",
            "request_sha256",
            "response_sha256",
            "response_bytes",
        ],
    );
    metadata.insert("mode".to_string(), "reverse_gene_set".to_string());
    draft.add_node(
        format!("l1000cds2_query:{signature}"),
        "l1000cds2_query",
        format!("L1000CDS2 reverse query {signature}"),
        metadata,
    )
}

fn score_node(draft: &mut LincsGraphDraft, value: &Value, row_sha: &str) -> String {
    let signature = str_field(value, "signature_id").unwrap_or_else(|| "unknown".to_string());
    let rank = str_field(value, "rank").unwrap_or_else(|| "unknown".to_string());
    let sig_id = str_field(value, "sig_id").unwrap_or_else(|| row_sha.to_string());
    draft.add_node(
        format!("lincs_reversal_score:{signature}:{rank}:{sig_id}"),
        "lincs_reversal_score",
        format!(
            "{} rank {} score {}",
            str_field(value, "pert_desc").unwrap_or_else(|| "unknown perturbation".to_string()),
            rank,
            str_field(value, "score").unwrap_or_else(|| "unknown".to_string())
        ),
        score_node_meta(value, row_sha),
    )
}

fn perturbation_node(draft: &mut LincsGraphDraft, value: &Value) -> String {
    let desc = str_field(value, "pert_desc").unwrap_or_else(|| "unknown perturbation".to_string());
    let key = str_field(value, "pert_id")
        .map(|id| format!("perturbation:{id}"))
        .unwrap_or_else(|| format!("perturbation_text:{}", normalize_key(&desc)));
    let mut metadata = fields(
        value,
        &[
            "pert_id",
            "pert_desc",
            "pert_dose",
            "pert_dose_unit",
            "pert_time",
            "pert_time_unit",
            "pubchem_id",
            "drugbank_id",
        ],
    );
    metadata.insert("source".to_string(), "L1000CDS2".to_string());
    draft.add_node(key, "perturbation", desc, metadata)
}

fn candidate_node(draft: &mut LincsGraphDraft, value: &Value) -> String {
    let candidate = str_field(value, "candidate_drug").unwrap_or_else(|| "unknown".to_string());
    draft.add_node(
        format!("current_candidate_drug:{}", normalize_key(&candidate)),
        "current_candidate_drug",
        candidate.clone(),
        BTreeMap::from([("candidate_drug".to_string(), clean_label(&candidate))]),
    )
}

fn unsupported_node(draft: &mut LincsGraphDraft, value: &Value, row_sha: &str) -> String {
    let signature = str_field(value, "signature_id").unwrap_or_else(|| "unknown".to_string());
    let candidate = str_field(value, "candidate_drug").unwrap_or_else(|| "unknown".to_string());
    let reason = str_field(value, "reason").unwrap_or_else(|| "unknown".to_string());
    let mut metadata = fields(
        value,
        &[
            "signature_id",
            "candidate_drug",
            "reason",
            "disease_name",
            "geo_id",
        ],
    );
    metadata.insert("row_sha256".to_string(), row_sha.to_string());
    draft.add_node(
        format!(
            "unsupported_lincs_case:{}:{}:{}",
            signature,
            normalize_key(&candidate),
            normalize_key(&reason)
        ),
        "unsupported_lincs_case",
        format!("{candidate} absent for {signature}"),
        metadata,
    )
}

fn optional_node(
    draft: &mut LincsGraphDraft,
    node_type: &str,
    namespace: &str,
    value: &Value,
    field: &str,
) -> Option<String> {
    let text = str_field(value, field)?;
    Some(draft.add_node(
        format!("{namespace}:{}", normalize_key(&text)),
        node_type,
        text.clone(),
        BTreeMap::from([(field.to_string(), clean_label(&text))]),
    ))
}

fn link_optional_concepts(
    draft: &mut LincsGraphDraft,
    signature: &str,
    disease: &str,
    value: &Value,
) {
    if let Some(do_id) = str_field(value, "do_id") {
        let node = draft.add_node(
            format!("disease_ontology:{do_id}"),
            "disease_ontology",
            do_id.clone(),
            BTreeMap::from([("do_id".to_string(), do_id)]),
        );
        draft.add_edge(disease, "has_ontology_id", &node, BTreeMap::new());
        draft.add_edge(
            signature,
            "uses_disease_ontology_id",
            &node,
            BTreeMap::new(),
        );
    }
    if let Some(umls) = str_field(value, "umls_cui") {
        let node = draft.add_node(
            format!("umls:{umls}"),
            "umls_concept",
            umls.clone(),
            BTreeMap::from([("umls_cui".to_string(), umls)]),
        );
        draft.add_edge(disease, "has_umls_cui", &node, BTreeMap::new());
    }
    for (field, node_type, edge_type) in [
        ("geo_id", "geo_series", "reported_in_geo_series"),
        ("cell_type", "biosample", "sampled_from_biosample"),
        ("platform", "geo_platform", "measured_on_platform"),
    ] {
        if let Some(node) = optional_node(draft, node_type, node_type, value, field) {
            draft.add_edge(signature, edge_type, &node, BTreeMap::new());
        }
    }
}

fn link_source_file(
    index: &RootIndex,
    draft: &mut LincsGraphDraft,
    from_key: &str,
    value: &Value,
) -> CliResult {
    let Some(source_file) = str_field(value, "source_file") else {
        return Ok(());
    };
    link_artifact(
        index,
        draft,
        from_key,
        &format!("raw/creeds/{source_file}"),
        "derived_from_creeds_file",
    )
}

fn link_chemical_ids(draft: &mut LincsGraphDraft, perturbation: &str, value: &Value) {
    for (field, node_type, namespace, edge_type) in [
        (
            "pubchem_id",
            "chemical_identifier",
            "pubchem",
            "has_pubchem_id",
        ),
        (
            "drugbank_id",
            "chemical_identifier",
            "drugbank",
            "has_drugbank_id",
        ),
    ] {
        if let Some(node) = optional_node(draft, node_type, namespace, value, field) {
            draft.add_edge(perturbation, edge_type, &node, BTreeMap::new());
        }
    }
}

fn link_matched_candidates(draft: &mut LincsGraphDraft, score: &str, value: &Value) {
    let Some(items) = value
        .get("matched_current_candidate_drugs")
        .and_then(Value::as_array)
    else {
        return;
    };
    for item in items.iter().filter_map(Value::as_str) {
        let candidate = draft.add_node(
            format!("current_candidate_drug:{}", normalize_key(item)),
            "current_candidate_drug",
            item,
            BTreeMap::from([("candidate_drug".to_string(), clean_label(item))]),
        );
        draft.add_edge(
            score,
            "matches_current_candidate_drug",
            &candidate,
            BTreeMap::new(),
        );
    }
}

fn record_score_paths(
    draft: &mut LincsGraphDraft,
    signature: &str,
    query: &str,
    score: &str,
    perturbation: &str,
    value: &Value,
) {
    draft.record_path(
        "disease_signature_to_reversal_score",
        vec![signature.to_string(), query.to_string(), score.to_string()],
    );
    draft.record_path(
        "reversal_score_to_perturbation",
        vec![score.to_string(), perturbation.to_string()],
    );
    if str_field(value, "rank").as_deref() == Some("1") {
        let signature_id =
            str_field(value, "signature_id").unwrap_or_else(|| "unknown".to_string());
        draft.record_path(
            &format!("rank1_reversal:{signature_id}"),
            vec![
                signature.to_string(),
                score.to_string(),
                perturbation.to_string(),
            ],
        );
    }
}

fn score_node_meta(value: &Value, row_sha: &str) -> BTreeMap<String, String> {
    let mut metadata = fields(
        value,
        &[
            "signature_id",
            "rank",
            "score",
            "cell_id",
            "deg_count",
            "sig_id",
            "pert_id",
            "pert_desc",
            "pert_dose",
            "pert_dose_unit",
            "pert_time",
            "pert_time_unit",
            "pubchem_id",
            "drugbank_id",
            "disease_name",
            "geo_id",
        ],
    );
    metadata.insert("row_sha256".to_string(), row_sha.to_string());
    metadata.insert("boundary".to_string(), "lead_signal_only".to_string());
    metadata.insert(
        "overlap_dn_up_count".to_string(),
        nested_array_len(value, "overlap", "dn/up").to_string(),
    );
    metadata.insert(
        "overlap_up_dn_count".to_string(),
        nested_array_len(value, "overlap", "up/dn").to_string(),
    );
    metadata
}

fn score_meta(value: &Value) -> BTreeMap<String, String> {
    fields(value, &["rank", "score", "cell_id", "pert_id", "sig_id"])
}
