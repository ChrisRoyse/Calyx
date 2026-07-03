use std::collections::{BTreeMap, BTreeSet};

use super::super::model::{EvidenceGraphDraft, concept_node, simple_node};
use super::{
    RootIndex, SourceLoadReport, add_row_node, array_strings, edge_meta, field_or,
    ingest_generic_unhandled_jsonl, link_raw_path, read_jsonl_rows, str_field,
};
use crate::error::CliResult;

pub(super) fn ingest(
    index: &RootIndex,
    draft: &mut EvidenceGraphDraft,
    report: &mut SourceLoadReport,
) -> CliResult {
    let mut handled = BTreeSet::new();
    ingest_graph_edges(index, draft, report, &mut handled)?;
    ingest_unmapped(index, draft, report, &mut handled)?;
    ingest_source_licenses(index, draft, report, &mut handled)?;
    ingest_generic_unhandled_jsonl(index, draft, report, &handled)
}

fn ingest_graph_edges(
    index: &RootIndex,
    draft: &mut EvidenceGraphDraft,
    report: &mut SourceLoadReport,
    handled: &mut BTreeSet<String>,
) -> CliResult {
    let rel = "parsed/dgidb_graph_edges.jsonl";
    handled.insert(rel.to_string());
    for row in read_jsonl_rows(index, rel, report)? {
        let row_key = add_row_node(index, draft, rel, row.line, "drug_gene_interaction", &row)?;
        link_raw_path(index, draft, &row_key, &row.value)?;
        let drug = concept_node(
            draft,
            str_field(&row.value, "source_overlay_id").as_deref(),
            &field_or(&row.value, "drug", "drug"),
            "drug",
        );
        let gene = concept_node(
            draft,
            str_field(&row.value, "target_overlay_id").as_deref(),
            &field_or(&row.value, "gene", "gene"),
            "gene",
        );
        draft.add_edge(
            &drug,
            "has_drug_gene_evidence",
            &row_key,
            edge_meta(&row, "dgidb"),
        );
        draft.add_edge(
            &row_key,
            "drug_gene_interaction",
            &gene,
            edge_meta(&row, "dgidb"),
        );
        draft.add_edge(
            &drug,
            "drug_gene_source_association",
            &gene,
            edge_meta(&row, "dgidb"),
        );
        for source_db in array_strings(&row.value, "source_dbs") {
            let source_node = simple_node(
                draft,
                "source_database",
                "source_db",
                &source_db,
                &source_db,
            );
            draft.add_edge(
                &row_key,
                "supported_by_source_db",
                &source_node,
                BTreeMap::new(),
            );
        }
        for pmid in array_strings(&row.value, "publication_pmids") {
            let publication = simple_node(draft, "publication", "pmid", &pmid, &pmid);
            draft.add_edge(&row_key, "published_in", &publication, BTreeMap::new());
        }
        draft.record_path("dgidb_positive", vec![drug, row_key, gene]);
    }
    Ok(())
}

fn ingest_unmapped(
    index: &RootIndex,
    draft: &mut EvidenceGraphDraft,
    report: &mut SourceLoadReport,
    handled: &mut BTreeSet<String>,
) -> CliResult {
    let rel = "parsed/unmapped_rows.jsonl";
    handled.insert(rel.to_string());
    for row in read_jsonl_rows(index, rel, report)? {
        let row_key = add_row_node(index, draft, rel, row.line, "unmapped_row", &row)?;
        link_raw_path(index, draft, &row_key, &row.value)?;
        let drug = concept_node(draft, None, &field_or(&row.value, "drug", "drug"), "drug");
        let gene = concept_node(draft, None, &field_or(&row.value, "gene", "gene"), "gene");
        draft.add_edge(
            &drug,
            "has_unmapped_candidate",
            &row_key,
            edge_meta(&row, "dgidb"),
        );
        draft.add_edge(&row_key, "unmapped_from", &gene, edge_meta(&row, "dgidb"));
        draft.add_edge(
            &drug,
            "negative_or_null_dgidb_signal",
            &gene,
            edge_meta(&row, "dgidb"),
        );
        draft.record_path("dgidb_negative", vec![drug, row_key, gene]);
    }
    Ok(())
}

fn ingest_source_licenses(
    index: &RootIndex,
    draft: &mut EvidenceGraphDraft,
    report: &mut SourceLoadReport,
    handled: &mut BTreeSet<String>,
) -> CliResult {
    let rel = "parsed/source_license_rows.jsonl";
    handled.insert(rel.to_string());
    for row in read_jsonl_rows(index, rel, report)? {
        let row_key = add_row_node(index, draft, rel, row.line, "source_license", &row)?;
        link_raw_path(index, draft, &row_key, &row.value)?;
        if let Some(source_db) = str_field(&row.value, "sourceDbName") {
            let source_node = simple_node(
                draft,
                "source_database",
                "source_db",
                &source_db,
                &source_db,
            );
            draft.add_edge(
                &row_key,
                "describes_source_database",
                &source_node,
                BTreeMap::new(),
            );
            if let Some(license) = str_field(&row.value, "license") {
                let license_node = simple_node(draft, "license", "license", &license, &license);
                draft.add_edge(&source_node, "has_license", &license_node, BTreeMap::new());
            }
        }
    }
    Ok(())
}
