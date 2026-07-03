use std::collections::BTreeMap;

use serde_json::Value;

use super::LincsSourceReport;
use super::root::{RootIndex, link_artifact};
use super::rows::{add_row_node, fields, read_jsonl_rows, row_meta, str_field};
use crate::cmd::lincs_reversal::model::{LincsGraphDraft, clean_label, normalize_key};
use crate::error::CliResult;

pub(super) fn ingest_metadata_mappings(
    index: &RootIndex,
    draft: &mut LincsGraphDraft,
    report: &mut LincsSourceReport,
) -> CliResult {
    ingest_perturbation_mappings(index, draft, report)?;
    ingest_placeholder_cases(index, draft, report)?;
    ingest_resolved_repeated_leads(index, draft, report)?;
    Ok(())
}

fn ingest_perturbation_mappings(
    index: &RootIndex,
    draft: &mut LincsGraphDraft,
    report: &mut LincsSourceReport,
) -> CliResult {
    let rel = "parsed/perturbation_id_mappings.jsonl";
    for row in read_jsonl_rows(index, rel, report)? {
        let row_key = add_row_node(index, draft, rel, "perturbation_metadata_row", &row)?;
        let perturbation = perturbation_node(draft, &row.value);
        draft.add_edge(
            &perturbation,
            "has_perturbation_metadata",
            &row_key,
            row_meta(&row),
        );
        if let Some(name) = resolved_name_node(draft, &row.value) {
            draft.add_edge(
                &perturbation,
                "resolved_to_drug_name",
                &name,
                row_meta(&row),
            );
            link_pubchem(draft, &perturbation, &row.value);
        } else {
            let status = mapping_status_node(draft, &row.value);
            draft.add_edge(&perturbation, "has_mapping_status", &status, row_meta(&row));
        }
        link_artifact(
            index,
            draft,
            &row_key,
            "raw/Drugs_metadata.csv",
            "derived_from",
        )?;
    }
    Ok(())
}

fn ingest_placeholder_cases(
    index: &RootIndex,
    draft: &mut LincsGraphDraft,
    report: &mut LincsSourceReport,
) -> CliResult {
    let rel = "parsed/placeholder_pert_desc_cases.jsonl";
    for row in read_jsonl_rows(index, rel, report)? {
        let row_key = add_row_node(index, draft, rel, "placeholder_pert_desc_case_row", &row)?;
        let case = placeholder_case_node(draft, &row.value);
        let score = score_node_key(&row.value);
        let perturbation = perturbation_node(draft, &row.value);
        draft.add_edge(&case, "derived_from", &row_key, row_meta(&row));
        draft.add_edge(&case, "placeholder_for_score", &score, row_meta(&row));
        draft.add_edge(
            &case,
            "placeholder_for_perturbation",
            &perturbation,
            row_meta(&row),
        );
        if let Some(name) = resolved_name_node(draft, &row.value) {
            draft.add_edge(&case, "resolved_placeholder_to_name", &name, row_meta(&row));
            draft.record_path(
                "placeholder_label_to_resolved_name",
                vec![case.clone(), perturbation, name],
            );
        } else {
            let status = mapping_status_node(draft, &row.value);
            draft.add_edge(
                &case,
                "placeholder_unresolved_status",
                &status,
                row_meta(&row),
            );
        }
        link_artifact(
            index,
            draft,
            &row_key,
            "raw/Drugs_metadata.csv",
            "derived_from",
        )?;
    }
    Ok(())
}

fn ingest_resolved_repeated_leads(
    index: &RootIndex,
    draft: &mut LincsGraphDraft,
    report: &mut LincsSourceReport,
) -> CliResult {
    let rel = "parsed/resolved_repeated_leads.jsonl";
    for row in read_jsonl_rows(index, rel, report)? {
        let row_key = add_row_node(index, draft, rel, "resolved_repeated_lead_row", &row)?;
        let lead = resolved_lead_node(draft, &row.value);
        draft.add_edge(&lead, "derived_from", &row_key, row_meta(&row));
        for pert_id in array_strings(&row.value, "pert_ids") {
            let pert = draft.add_node(
                format!("perturbation:{pert_id}"),
                "perturbation",
                pert_id.clone(),
                BTreeMap::from([("pert_id".to_string(), pert_id)]),
            );
            draft.add_edge(&lead, "summarizes_perturbation", &pert, row_meta(&row));
        }
        link_artifact(
            index,
            draft,
            &row_key,
            "raw/Drugs_metadata.csv",
            "derived_from",
        )?;
    }
    Ok(())
}

fn perturbation_node(draft: &mut LincsGraphDraft, value: &Value) -> String {
    let pert_id = str_field(value, "pert_id").unwrap_or_else(|| "unknown".to_string());
    let label = str_field(value, "pert_iname")
        .or_else(|| str_field(value, "resolved_pert_iname"))
        .filter(|name| name != "NULL" && name != &pert_id)
        .unwrap_or_else(|| pert_id.clone());
    draft.add_node(
        format!("perturbation:{pert_id}"),
        "perturbation",
        label,
        fields(
            value,
            &[
                "pert_id",
                "mapping_status",
                "resolved_status",
                "pert_iname",
                "resolved_pert_iname",
                "LSM_id",
                "pubchem_cid",
                "inchi_key",
                "molecular_formula",
                "molecular_wt",
            ],
        ),
    )
}

fn resolved_name_node(draft: &mut LincsGraphDraft, value: &Value) -> Option<String> {
    let status =
        str_field(value, "mapping_status").or_else(|| str_field(value, "resolved_status"))?;
    if status != "resolved_name" {
        return None;
    }
    let name =
        str_field(value, "pert_iname").or_else(|| str_field(value, "resolved_pert_iname"))?;
    if name == "NULL" || name.starts_with("BRD-") {
        return None;
    }
    Some(draft.add_node(
        format!("resolved_drug_name:{}", normalize_key(&name)),
        "resolved_drug_name",
        name.clone(),
        BTreeMap::from([("resolved_name".to_string(), clean_label(&name))]),
    ))
}

fn mapping_status_node(draft: &mut LincsGraphDraft, value: &Value) -> String {
    let status = str_field(value, "mapping_status")
        .or_else(|| str_field(value, "resolved_status"))
        .unwrap_or_else(|| "unknown".to_string());
    draft.add_node(
        format!("mapping_status:{}", normalize_key(&status)),
        "mapping_status",
        status.clone(),
        BTreeMap::from([("mapping_status".to_string(), status)]),
    )
}

fn placeholder_case_node(draft: &mut LincsGraphDraft, value: &Value) -> String {
    let sig_id = str_field(value, "sig_id").unwrap_or_else(|| "unknown".to_string());
    let pert_id = str_field(value, "pert_id").unwrap_or_else(|| "unknown".to_string());
    draft.add_node(
        format!("placeholder_pert_desc_case:{sig_id}:{pert_id}"),
        "placeholder_pert_desc_case",
        format!("placeholder pert_desc {pert_id}"),
        fields(
            value,
            &[
                "signature_id",
                "sig_id",
                "rank",
                "score",
                "original_pert_desc",
                "pert_id",
                "resolved_status",
                "resolved_pert_iname",
                "pubchem_cid",
                "cell_id",
                "disease_name",
                "geo_id",
            ],
        ),
    )
}

fn score_node_key(value: &Value) -> String {
    format!(
        "lincs_reversal_score:{}:{}:{}",
        str_field(value, "signature_id").unwrap_or_else(|| "unknown".to_string()),
        str_field(value, "rank").unwrap_or_else(|| "unknown".to_string()),
        str_field(value, "sig_id").unwrap_or_else(|| "unknown".to_string())
    )
}

fn resolved_lead_node(draft: &mut LincsGraphDraft, value: &Value) -> String {
    let name = str_field(value, "resolved_name_or_id").unwrap_or_else(|| "unknown".to_string());
    let status = str_field(value, "mapping_status").unwrap_or_else(|| "unknown".to_string());
    draft.add_node(
        format!(
            "resolved_repeated_lead:{}:{}",
            normalize_key(&status),
            normalize_key(&name)
        ),
        "resolved_repeated_lead",
        name,
        fields(
            value,
            &[
                "resolved_name_or_id",
                "mapping_status",
                "rows",
                "min_rank",
                "max_score",
            ],
        ),
    )
}

fn link_pubchem(draft: &mut LincsGraphDraft, perturbation: &str, value: &Value) {
    let Some(pubchem) = str_field(value, "pubchem_cid") else {
        return;
    };
    let node = draft.add_node(
        format!("pubchem:{pubchem}"),
        "chemical_identifier",
        pubchem.clone(),
        BTreeMap::from([("pubchem_cid".to_string(), pubchem)]),
    );
    draft.add_edge(perturbation, "has_pubchem_id", &node, BTreeMap::new());
}

fn array_strings(value: &Value, field: &str) -> Vec<String> {
    value
        .get(field)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}
