use std::fs;
use std::path::{Path, PathBuf};

use super::{AssociationValidationArgs, parse_association_validation_gates};
use crate::cmd::Subcommand;

#[test]
fn parses_required_roots_and_thresholds() {
    let tokens = [
        "--typed-root",
        "/fsv/typed",
        "--open-targets-root",
        "/fsv/open-targets",
        "--pubtator-root",
        "/fsv/pubtator",
        "--clinicaltrials-root",
        "/fsv/clinical",
        "--dgidb-root",
        "/fsv/dgidb",
        "--out-dir",
        "/fsv/out",
        "--cutoff-year",
        "2018",
        "--score-threshold",
        "0.4",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();

    let parsed = parse_association_validation_gates(&tokens).expect("parse command");

    assert_eq!(
        parsed,
        Subcommand::AssociationValidationGates(AssociationValidationArgs {
            typed_root: "/fsv/typed".into(),
            open_targets_root: "/fsv/open-targets".into(),
            pubtator_root: "/fsv/pubtator".into(),
            clinicaltrials_root: "/fsv/clinical".into(),
            dgidb_root: "/fsv/dgidb".into(),
            out_dir: "/fsv/out".into(),
            cutoff_year: 2018,
            score_threshold: 0.4,
            ..AssociationValidationArgs::default()
        })
    );
}

#[test]
fn rejects_missing_required_roots() {
    let err = parse_association_validation_gates(&[]).expect_err("missing roots");
    assert!(err.message().contains("--typed-root"));
}

#[test]
fn validation_gate_persists_readback_artifacts() {
    let root = temp_root("association-validation");
    let typed = root.join("typed");
    let open_targets = root.join("open_targets");
    let pubtator = root.join("pubtator");
    let clinical = root.join("clinical");
    let dgidb = root.join("dgidb");
    let out = root.join("out");
    seed_fixture(&typed, &open_targets, &pubtator, &clinical, &dgidb);

    let args = AssociationValidationArgs {
        typed_root: typed,
        open_targets_root: open_targets,
        pubtator_root: pubtator,
        clinicaltrials_root: clinical,
        dgidb_root: dgidb,
        out_dir: out.clone(),
        cutoff_year: 2016,
        ..AssociationValidationArgs::default()
    };

    let report = super::build_report(&args).expect("build report");
    assert!(report.gate_passed, "{:?}", report.gate_decision.reasons);
    let readback = super::model::persist_report_set(&out, &report).expect("persist report");

    assert!(readback.readback_gate_passed);
    assert_eq!(
        readback.benchmark_source_rows,
        report.benchmark_source_rows.len()
    );
    assert_eq!(
        readback.train_test_split_rows,
        report.train_test_split.len()
    );
    assert_eq!(readback.scored_output_rows, report.scored_outputs.len());
    assert!(readback.report_sha256.len() == 64);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn open_targets_rows_without_direction_fail_gate_with_blocked_readback() {
    let root = temp_root("association-validation-direction-block");
    let typed = root.join("typed");
    let open_targets = root.join("open_targets");
    let pubtator = root.join("pubtator");
    let clinical = root.join("clinical");
    let dgidb = root.join("dgidb");
    let out = root.join("out");
    seed_fixture(&typed, &open_targets, &pubtator, &clinical, &dgidb);
    write(
        open_targets.join("open_targets_validation_edges.jsonl"),
        r#"{"edge_id":"ot1","target_name":"TNF","disease_name":"psoriasis","score":0.8,"overlay_target_concepts":["concept:hgnc:11892"],"overlay_disease_concepts":["concept:mesh:D011565"]}"#,
    );

    let args = AssociationValidationArgs {
        typed_root: typed,
        open_targets_root: open_targets,
        pubtator_root: pubtator,
        clinicaltrials_root: clinical,
        dgidb_root: dgidb,
        out_dir: out.clone(),
        cutoff_year: 2016,
        ..AssociationValidationArgs::default()
    };

    let report = super::build_report(&args).expect("build report");
    assert!(!report.gate_passed);
    assert_eq!(
        report.mechanistic_direction_counts.blocked_direction_rows,
        1
    );
    assert!(report.gate_decision.reasons.iter().any(|reason| {
        reason.contains("Open Targets rows lacked usable mechanistic direction")
    }));
    let readback = super::model::persist_report_set(&out, &report).expect("persist report");
    assert_eq!(readback.mechanistic_direction_blocked_rows, 1);

    let _ = fs::remove_dir_all(root);
}

fn seed_fixture(typed: &Path, open: &Path, pubtator: &Path, clinical: &Path, dgidb: &Path) {
    write(typed.join("typed_graph_summary.json"), "{}\n");
    write(
        open.join("open_targets_validation_edges.jsonl"),
        r#"{"edge_id":"ot1","target_name":"TNF","disease_name":"psoriasis","score":0.8,"overlay_target_concepts":["concept:hgnc:11892"],"overlay_disease_concepts":["concept:mesh:D011565"],"directionOnTarget":"Gain of Function","directionOnTrait":"Risk"}"#,
    );
    write(
        pubtator.join("parsed/association_evidence_edges.jsonl"),
        r#"{"seed_id":"tnf_psoriasis","left_term":"TNF","right_term":"psoriasis","relation_publication_sum":500,"relation_endpoint_exact_match_count":3,"export_docs_with_both_entities":8}"#,
    );
    write(
        clinical.join("parsed/clinicaltrials_seed_summaries.jsonl"),
        r#"{"seed_id":"metformin_type2_diabetes","intervention":"metformin","condition":"type 2 diabetes","total_count":100,"exact_intervention_match_count":25,"with_results_count":10,"stopped_status_count":0,"max_trial_evidence_score":5.6}"#,
    );
    write(
        clinical.join("parsed/clinicaltrials_trial_rows.jsonl"),
        concat!(
            r#"{"seed_id":"metformin_type2_diabetes","start_date":"2010-01-01","trial_evidence_score":5.0,"exact_intervention_match":true,"condition_match":true,"has_results":true,"overall_status":"COMPLETED"}"#,
            "\n",
            r#"{"seed_id":"metformin_type2_diabetes","start_date":"2018-01-01","trial_evidence_score":5.6,"exact_intervention_match":true,"condition_match":true,"has_results":true,"overall_status":"COMPLETED"}"#,
            "\n",
            r#"{"seed_id":"weak_control","start_date":"2010-01-01","trial_evidence_score":0.1,"exact_intervention_match":false,"condition_match":true,"has_results":false,"overall_status":"UNKNOWN"}"#,
            "\n"
        ),
    );
    write(
        dgidb.join("parsed/seed_pair_tsv_interactions.jsonl"),
        r#"{"seed_id":"tnf_adalimumab","drug_name":"ADALIMUMAB","gene_name":"TNF","interaction_score":"5.0"}"#,
    );
    write(
        dgidb.join("parsed/unmapped_rows.jsonl"),
        concat!(
            r#"{"seed_id":"dpp4_metformin","drug":"metformin","gene":"DPP4","reason":"no_dgidb_graphql_interaction_for_exact_pair","total_count":0}"#,
            "\n",
            r#"{"seed_id":"pla2r1_rituximab","drug":"rituximab","gene":"PLA2R1","reason":"no_dgidb_graphql_interaction_for_exact_pair","total_count":0}"#,
            "\n"
        ),
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
