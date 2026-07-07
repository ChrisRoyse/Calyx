use std::fs;
use std::path::{Path, PathBuf};

use super::{
    TypedAssociationMinerArgs, build_report, parse_typed_association_miner, persist::persist,
};
use crate::cmd::Subcommand;

#[test]
fn parses_scope_filters() {
    let tokens = [
        "--typed-root",
        "/typed",
        "--validation-report",
        "/validation/report.json",
        "--out-dir",
        "/out",
        "--source-type",
        "chemical",
        "--target-type",
        "disease",
        "--name-contains",
        "asthma",
        "--source-issue",
        "1173",
        "--min-support",
        "2",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();

    let parsed = parse_typed_association_miner(&tokens).expect("parse miner");
    let Subcommand::TypedAssociationMiner(args) = parsed else {
        panic!("wrong command");
    };
    assert_eq!(args.source_type.as_deref(), Some("chemical"));
    assert_eq!(args.target_type.as_deref(), Some("disease"));
    assert_eq!(args.name_contains.as_deref(), Some("asthma"));
    assert_eq!(args.source_issue, Some(1173));
    assert_eq!(args.min_support, 2);
}

#[test]
fn miner_requires_passing_validation_report() {
    let root = temp_root("typed-miner-fail");
    seed_typed(&root.join("typed"));
    write(
        root.join("validation.json"),
        r#"{"gate_passed":false,"schema_version":1}"#,
    );
    let args = args(&root);
    let error = build_report(&args).expect_err("failed gate must refuse");
    assert!(error.message().contains("did not pass"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn miner_persists_hypotheses_with_readback() {
    let root = temp_root("typed-miner-ok");
    seed_typed(&root.join("typed"));
    write(
        root.join("validation.json"),
        r#"{"gate_passed":true,"schema_version":1}"#,
    );
    let args = args(&root);
    let report = build_report(&args).expect("build report");
    assert_eq!(report.emitted_hypothesis_count, 1);
    let readback = persist(&args.out_dir, &report).expect("persist");
    assert_eq!(readback.hypothesis_count, 1);
    assert_eq!(readback.report_sha256.len(), 64);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn miner_deduplicates_reversed_edges_into_filtered_orientation() {
    let root = temp_root("typed-miner-dedup");
    seed_reversed_typed(&root.join("typed"));
    write(
        root.join("validation.json"),
        r#"{"gate_passed":true,"schema_version":1}"#,
    );
    let args = args(&root);
    let report = build_report(&args).expect("build report");
    assert_eq!(report.candidate_pair_count, 1);
    assert_eq!(report.hypotheses[0].source_type, "chemical");
    assert_eq!(report.hypotheses[0].target_type, "disease");
    assert_eq!(report.hypotheses[0].path_count, 2);
    assert_eq!(report.hypotheses[0].support_count, 5);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gene_disease_edge_without_direction_is_blocked_not_emitted() {
    let root = temp_root("typed-miner-direction-block");
    seed_gene_disease_typed(&root.join("typed"), None);
    write(
        root.join("validation.json"),
        r#"{"gate_passed":true,"schema_version":2}"#,
    );
    let args = TypedAssociationMinerArgs {
        typed_root: root.join("typed"),
        validation_report: root.join("validation.json"),
        out_dir: root.join("out"),
        source_type: Some("gene".to_string()),
        target_type: Some("disease".to_string()),
        min_support: 1,
        ..TypedAssociationMinerArgs::default()
    };
    let report = build_report(&args).expect("build blocked report");
    assert_eq!(report.emitted_hypothesis_count, 0);
    assert_eq!(report.blocked_candidate_count, 1);
    assert_eq!(
        report.blocked_candidates[0].reason_codes,
        vec![
            "CALYX_MECH_TARGET_CONSEQUENCE_MISSING".to_string(),
            "CALYX_MECH_TRAIT_EFFECT_MISSING".to_string()
        ]
    );
    let readback = persist(&args.out_dir, &report).expect("persist blocked report");
    assert_eq!(readback.hypothesis_count, 0);
    assert_eq!(readback.blocked_candidate_count, 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gene_disease_direction_is_preserved_and_emitted() {
    let root = temp_root("typed-miner-direction-ok");
    seed_gene_disease_typed(
        &root.join("typed"),
        Some(r#","directionOnTarget":"Gain of Function","directionOnTrait":"Risk""#),
    );
    write(
        root.join("validation.json"),
        r#"{"gate_passed":true,"schema_version":2}"#,
    );
    let args = TypedAssociationMinerArgs {
        typed_root: root.join("typed"),
        validation_report: root.join("validation.json"),
        out_dir: root.join("out"),
        source_type: Some("gene".to_string()),
        target_type: Some("disease".to_string()),
        min_support: 1,
        ..TypedAssociationMinerArgs::default()
    };
    let report = build_report(&args).expect("build direction report");
    assert_eq!(report.emitted_hypothesis_count, 1);
    assert_eq!(report.blocked_candidate_count, 0);
    assert_eq!(
        serde_json::to_value(report.hypotheses[0].required_target_modulation)
            .unwrap()
            .as_str(),
        Some("inhibit")
    );
    assert_eq!(
        report.hypotheses[0].mechanistic_direction_status,
        "direction_inferred"
    );
    let _ = fs::remove_dir_all(root);
}

fn args(root: &Path) -> TypedAssociationMinerArgs {
    TypedAssociationMinerArgs {
        typed_root: root.join("typed"),
        validation_report: root.join("validation.json"),
        out_dir: root.join("out"),
        source_type: Some("chemical".to_string()),
        target_type: Some("disease".to_string()),
        min_support: 2,
        ..TypedAssociationMinerArgs::default()
    }
}

fn seed_typed(root: &Path) {
    write(
        root.join("typed_nodes.jsonl"),
        concat!(
            r#"{"node_id":"concept:drug","node_type":"concept","normalized_name":"metformin","concept_type":"chemical"}"#,
            "\n",
            r#"{"node_id":"concept:disease","node_type":"concept","normalized_name":"type 2 diabetes","concept_type":"disease"}"#,
            "\n"
        ),
    );
    write(
        root.join("typed_edges.jsonl"),
        r#"{"edge_id":"edge:1","edge_type":"associated_with","source":"concept:drug","target":"concept:disease","source_issue":1173,"support_count":3,"source_hash":["abc"],"support_cx_ids":["cx1","cx2","cx3"]}"#,
    );
}

fn seed_reversed_typed(root: &Path) {
    write(
        root.join("typed_nodes.jsonl"),
        concat!(
            r#"{"node_id":"concept:drug","node_type":"concept","normalized_name":"metformin","concept_type":"chemical"}"#,
            "\n",
            r#"{"node_id":"concept:disease","node_type":"concept","normalized_name":"type 2 diabetes","concept_type":"disease"}"#,
            "\n"
        ),
    );
    write(
        root.join("typed_edges.jsonl"),
        concat!(
            r#"{"edge_id":"edge:1","edge_type":"associated_with","source":"concept:drug","target":"concept:disease","source_issue":1173,"support_count":3,"source_hash":["abc"],"support_cx_ids":["cx1"]}"#,
            "\n",
            r#"{"edge_id":"edge:2","edge_type":"associated_with","source":"concept:disease","target":"concept:drug","source_issue":1173,"support_count":2,"source_hash":["def"],"support_cx_ids":["cx2"]}"#,
            "\n"
        ),
    );
}

fn seed_gene_disease_typed(root: &Path, direction_json_suffix: Option<&str>) {
    write(
        root.join("typed_nodes.jsonl"),
        concat!(
            r#"{"node_id":"concept:gene:TNF","node_type":"concept","normalized_name":"TNF","concept_type":"gene"}"#,
            "\n",
            r#"{"node_id":"concept:disease:psoriasis","node_type":"concept","normalized_name":"psoriasis","concept_type":"disease"}"#,
            "\n"
        ),
    );
    let suffix = direction_json_suffix.unwrap_or_default();
    write(
        root.join("typed_edges.jsonl"),
        &format!(
            r#"{{"edge_id":"edge:gene-disease","edge_type":"associated_with","source":"concept:gene:TNF","target":"concept:disease:psoriasis","support_count":3,"source_hash":["abc"],"support_cx_ids":["cx1"]{suffix}}}"#
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
