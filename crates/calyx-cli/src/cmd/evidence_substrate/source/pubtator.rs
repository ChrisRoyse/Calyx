use std::collections::{BTreeMap, BTreeSet};

use super::super::model::{EvidenceGraphDraft, concept_node, simple_node};
use super::{
    RootIndex, SourceLoadReport, add_row_node, array_values, edge_meta, field_or,
    ingest_generic_unhandled_jsonl, link_raw_path, link_raw_paths, read_jsonl_rows, str_field,
};
use crate::error::CliResult;

pub(super) fn ingest(
    index: &RootIndex,
    draft: &mut EvidenceGraphDraft,
    report: &mut SourceLoadReport,
) -> CliResult {
    let mut handled = BTreeSet::new();
    ingest_associations(index, draft, report, &mut handled)?;
    ingest_literature(
        index,
        draft,
        report,
        &mut handled,
        "parsed/supporting_literature.jsonl",
        false,
    )?;
    ingest_literature(
        index,
        draft,
        report,
        &mut handled,
        "parsed/contradicting_or_negative_literature.jsonl",
        true,
    )?;
    ingest_generic_unhandled_jsonl(index, draft, report, &handled)
}

fn ingest_associations(
    index: &RootIndex,
    draft: &mut EvidenceGraphDraft,
    report: &mut SourceLoadReport,
    handled: &mut BTreeSet<String>,
) -> CliResult {
    let rel = "parsed/association_evidence_edges.jsonl";
    handled.insert(rel.to_string());
    for row in read_jsonl_rows(index, rel, report)? {
        let row_key = add_row_node(index, draft, rel, row.line, "association_evidence", &row)?;
        link_raw_paths(index, draft, &row_key, &row.value)?;
        let left = concept_node(
            draft,
            str_field(&row.value, "left_id").as_deref(),
            &field_or(&row.value, "left_term", "left"),
            "biomedical",
        );
        let right = concept_node(
            draft,
            str_field(&row.value, "right_id").as_deref(),
            &field_or(&row.value, "right_term", "right"),
            &field_or(&row.value, "right_concept", "biomedical"),
        );
        draft.add_edge(
            &left,
            "has_evidence_row",
            &row_key,
            edge_meta(&row, "pubtator"),
        );
        draft.add_edge(&row_key, "supports", &right, edge_meta(&row, "pubtator"));
        draft.add_edge(
            &left,
            "supports_association",
            &right,
            edge_meta(&row, "pubtator"),
        );
        draft.record_path(
            "pubtator_positive",
            vec![left.clone(), row_key.clone(), right.clone()],
        );
    }
    Ok(())
}

fn ingest_literature(
    index: &RootIndex,
    draft: &mut EvidenceGraphDraft,
    report: &mut SourceLoadReport,
    handled: &mut BTreeSet<String>,
    rel: &str,
    negative: bool,
) -> CliResult {
    handled.insert(rel.to_string());
    for row in read_jsonl_rows(index, rel, report)? {
        let row_type = if negative {
            "negative_literature"
        } else {
            "supporting_literature"
        };
        let row_key = add_row_node(index, draft, rel, row.line, row_type, &row)?;
        link_raw_path(index, draft, &row_key, &row.value)?;
        let left = concept_node(
            draft,
            str_field(&row.value, "left_id").as_deref(),
            &field_or(&row.value, "left_id", "left"),
            "biomedical",
        );
        let right = concept_node(
            draft,
            str_field(&row.value, "right_id").as_deref(),
            &field_or(&row.value, "right_id", "right"),
            "biomedical",
        );
        if let Some(pmid) = str_field(&row.value, "pmid") {
            let pub_key = simple_node(draft, "publication", "pmid", &pmid, &pmid);
            draft.add_edge(
                &row_key,
                "published_in",
                &pub_key,
                edge_meta(&row, "pubtator"),
            );
            draft.add_edge(&pub_key, "mentions", &left, BTreeMap::new());
            draft.add_edge(&pub_key, "mentions", &right, BTreeMap::new());
        }
        for annotation in array_values(&row.value, "annotations") {
            if let Some(accession) = str_field(annotation, "accession") {
                let label = field_or(annotation, "name", &accession);
                let concept = concept_node(
                    draft,
                    Some(&accession),
                    &label,
                    &field_or(annotation, "type", "annotation"),
                );
                if let Some(pmid) = str_field(&row.value, "pmid") {
                    let pub_key = simple_node(draft, "publication", "pmid", &pmid, &pmid);
                    draft.add_edge(&pub_key, "publication_mentions", &concept, BTreeMap::new());
                }
            }
        }
        if negative {
            draft.add_edge(
                &left,
                "has_negative_evidence",
                &row_key,
                edge_meta(&row, "pubtator"),
            );
            draft.add_edge(
                &row_key,
                "refutes_or_cautions",
                &right,
                edge_meta(&row, "pubtator"),
            );
            draft.add_edge(
                &left,
                "negative_evidence_association",
                &right,
                edge_meta(&row, "pubtator"),
            );
            draft.record_path("pubtator_negative", vec![left, row_key, right]);
        } else {
            draft.add_edge(
                &left,
                "has_supporting_literature",
                &row_key,
                edge_meta(&row, "pubtator"),
            );
            draft.add_edge(
                &row_key,
                "supports_association_evidence",
                &right,
                edge_meta(&row, "pubtator"),
            );
        }
    }
    Ok(())
}
