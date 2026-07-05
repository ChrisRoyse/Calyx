use std::fs;
use std::path::{Path, PathBuf};

use super::{
    HypothesisFalsificationArgs, build_report, parse_hypothesis_falsification_sweep,
    persist::persist,
};
use crate::cmd::Subcommand;

#[test]
fn parses_required_roots_and_repeated_reports() {
    let tokens = [
        "--hypotheses-report",
        "/fsv/broad/report.json",
        "--hypotheses-report",
        "/fsv/scoped/report.json",
        "--pubtator-root",
        "/pubtator",
        "--clinicaltrials-root",
        "/ctg",
        "--dgidb-root",
        "/dgidb",
        "--open-targets-root",
        "/ot",
        "--out-dir",
        "/out",
        "--max-hypotheses",
        "500",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    let parsed = parse_hypothesis_falsification_sweep(&tokens).expect("parse");
    let Subcommand::HypothesisFalsificationSweep(args) = parsed else {
        panic!("wrong command");
    };
    assert_eq!(args.hypotheses_reports.len(), 2);
    assert_eq!(args.max_hypotheses, 500);
}

#[test]
fn sweep_flags_counter_evidence_and_persists_readback() {
    let root = temp_root("falsification-ok");
    seed(&root);
    let args = args(&root);
    let report = build_report(&args).expect("build report");
    assert_eq!(report.input_hypothesis_count, 1);
    assert_eq!(report.deduped_hypothesis_count, 1);
    assert_eq!(report.support_evidence_count, 2);
    assert_eq!(report.counter_evidence_count, 1);
    assert_eq!(report.skipped_evidence_count, 0);
    assert_eq!(report.flagged_with_counter_evidence_count, 1);
    assert_eq!(
        report.hypothesis_flags[0].sweep_status,
        "complete_counterevidence_found"
    );
    let readback = persist(&args.out_dir, &report).expect("persist");
    assert_eq!(readback.flag_count, 1);
    assert_eq!(readback.report_sha256.len(), 64);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn stopped_trial_comention_does_not_count_without_asserted_pair() {
    let root = temp_root("falsification-trial-comention");
    seed_hypothesis(
        &root,
        HypothesisFixture {
            hypothesis_id: "typed-assoc:metformin::kidney",
            source_id: "concept:metformin",
            source_name: "metformin",
            source_type: "chemical",
            target_id: "concept:kidney-disease",
            target_name: "kidney disease",
            target_type: "disease",
        },
    );
    seed_empty_sources(&root);
    write(
        root.join("clinicaltrials/parsed/clinicaltrials_trial_rows.jsonl"),
        r#"{"query_intervention":"metformin","query_condition":"type 2 diabetes","brief_title":"metformin kidney disease exploratory note","overall_status":"TERMINATED","why_stopped":"administrative"}"#,
    );
    let report = build_report(&args(&root)).expect("build report");
    assert_eq!(report.counter_evidence_count, 0);
    assert_eq!(report.hypothesis_flags[0].counter_evidence_count, 0);
    assert_eq!(
        report.hypothesis_flags[0].sweep_status,
        "complete_no_counterevidence_found_in_current_sources"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn identifier_match_requires_asserted_endpoint_id_not_numeric_tail() {
    let root = temp_root("falsification-id-match");
    seed_hypothesis(
        &root,
        HypothesisFixture {
            hypothesis_id: "typed-assoc:chembl123::alk",
            source_id: "CHEMBL:CHEMBL123",
            source_name: "test drug",
            source_type: "chemical",
            target_id: "HGNC:427",
            target_name: "ALK",
            target_type: "gene",
        },
    );
    seed_empty_sources(&root);
    write(
        root.join("dgidb/parsed/seed_pair_graphql_interactions.jsonl"),
        concat!(
            r#"{"source_overlay_id":"CHEMBL:CHEMBL999","drug":"test drug","#,
            r#""target_overlay_id":"HGNC:427","gene":"ALK","interaction_score":1.0}"#,
            "\n",
            r#"{"source_overlay_id":"CHEMBL:CHEMBL123","drug":"test drug","#,
            r#""target_overlay_id":"HGNC:427","gene":"ALK","interaction_score":1.0}"#,
        ),
    );
    let report = build_report(&args(&root)).expect("build report");
    assert_eq!(report.support_evidence_count, 1);
    assert_eq!(report.support_evidence[0].source_row_index, 2);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn internal_concept_ids_can_match_structured_endpoint_labels() {
    let root = temp_root("falsification-concept-label");
    seed_hypothesis(
        &root,
        HypothesisFixture {
            hypothesis_id: "typed-assoc:cd4::cd8a",
            source_id: "concept:ncbi_gene:920",
            source_name: "CD4",
            source_type: "gene",
            target_id: "concept:ncbi_gene:925",
            target_name: "CD8A",
            target_type: "gene",
        },
    );
    seed_empty_sources(&root);
    write(
        root.join("pubtator/parsed/supporting_literature.jsonl"),
        r#"{"pmid":"38179747","left_id":"@GENE_CD4","right_id":"@GENE_CD8A","relation_count":7,"support_basis":"pubtator_export_contains_both_selected_annotations"}"#,
    );
    let report = build_report(&args(&root)).expect("build report");
    assert_eq!(report.support_evidence_count, 1);
    assert_eq!(report.support_evidence[0].source_row_index, 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn unstructured_classifiable_row_is_skipped_not_counted() {
    let root = temp_root("falsification-unstructured");
    seed_hypothesis(
        &root,
        HypothesisFixture {
            hypothesis_id: "typed-assoc:metformin::diabetes",
            source_id: "concept:metformin",
            source_name: "metformin",
            source_type: "chemical",
            target_id: "concept:type2diabetes",
            target_name: "type 2 diabetes",
            target_type: "disease",
        },
    );
    seed_empty_sources(&root);
    write(
        root.join("pubtator/parsed/supporting_literature.jsonl"),
        r#"{"pmid":"99","relation_count":3,"support_basis":"mentions metformin and type 2 diabetes but has no asserted endpoints"}"#,
    );
    let report = build_report(&args(&root)).expect("build report");
    assert_eq!(report.support_evidence_count, 0);
    assert_eq!(report.counter_evidence_count, 0);
    assert_eq!(report.skipped_evidence_count, 1);
    assert_eq!(
        report.skipped_evidence[0].reason_code,
        "CALYX_FALSIFY_UNSTRUCTURED_ROW"
    );
    let _ = fs::remove_dir_all(root);
}

fn args(root: &Path) -> HypothesisFalsificationArgs {
    HypothesisFalsificationArgs {
        hypotheses_reports: vec![root.join("miner_report.json")],
        pubtator_root: root.join("pubtator"),
        clinicaltrials_root: root.join("clinicaltrials"),
        dgidb_root: root.join("dgidb"),
        open_targets_root: root.join("open_targets"),
        out_dir: root.join("out"),
        max_hypotheses: 10,
        preflight: Default::default(),
    }
}

fn seed(root: &Path) {
    seed_hypothesis(
        root,
        HypothesisFixture {
            hypothesis_id: "typed-assoc:metformin::diabetes",
            source_id: "concept:metformin",
            source_name: "metformin",
            source_type: "chemical",
            target_id: "concept:type2diabetes",
            target_name: "type 2 diabetes",
            target_type: "disease",
        },
    );
    seed_empty_sources(root);
    write(
        root.join("pubtator/parsed/supporting_literature.jsonl"),
        r#"{"pmid":"1","left_id":"@CHEMICAL_Metformin","right_id":"@DISEASE_Diabetes_Mellitus_Type_2","relation_count":2,"support_basis":"fixture Metformin type 2 diabetes support"}"#,
    );
    write(
        root.join("pubtator/parsed/contradicting_or_negative_literature.jsonl"),
        r#"{"pmid":"2","left_id":"@CHEMICAL_Metformin","right_id":"@DISEASE_Diabetes_Mellitus_Type_2","negative_signal_match":"not significantly associated"}"#,
    );
    write(
        root.join("clinicaltrials/parsed/clinicaltrials_seed_summaries.jsonl"),
        r#"{"intervention":"metformin","condition":"type 2 diabetes","total_count":5,"with_results_count":2,"exact_intervention_match_count":5,"stopped_status_count":0}"#,
    );
}

struct HypothesisFixture<'a> {
    hypothesis_id: &'a str,
    source_id: &'a str,
    source_name: &'a str,
    source_type: &'a str,
    target_id: &'a str,
    target_name: &'a str,
    target_type: &'a str,
}

fn seed_hypothesis(root: &Path, fixture: HypothesisFixture<'_>) {
    let HypothesisFixture {
        hypothesis_id,
        source_id,
        source_name,
        source_type,
        target_id,
        target_name,
        target_type,
    } = fixture;
    write(
        root.join("miner_report.json"),
        &format!(
            r#"{{"hypotheses":[{{"hypothesis_id":"{hypothesis_id}","source_id":"{source_id}","source_name":"{source_name}","source_type":"{source_type}","target_id":"{target_id}","target_name":"{target_name}","target_type":"{target_type}","support_count":3,"score":0.9}}]}}"#
        ),
    );
}

fn seed_empty_sources(root: &Path) {
    write(root.join("pubtator/parsed/supporting_literature.jsonl"), "");
    write(
        root.join("pubtator/parsed/contradicting_or_negative_literature.jsonl"),
        "",
    );
    write(
        root.join("clinicaltrials/parsed/clinicaltrials_seed_summaries.jsonl"),
        "",
    );
    write(
        root.join("clinicaltrials/parsed/clinicaltrials_trial_rows.jsonl"),
        "",
    );
    write(
        root.join("dgidb/parsed/seed_pair_graphql_interactions.jsonl"),
        "",
    );
    write(root.join("dgidb/parsed/unmapped_rows.jsonl"), "");
    write(
        root.join("open_targets/open_targets_validation_edges.jsonl"),
        "",
    );
}

fn write(path: PathBuf, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, text).unwrap();
}

fn temp_root(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "calyx-{name}-{}-{}",
        std::process::id(),
        ulid::Ulid::new()
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}
