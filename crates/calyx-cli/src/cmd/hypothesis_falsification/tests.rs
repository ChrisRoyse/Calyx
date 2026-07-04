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

fn args(root: &Path) -> HypothesisFalsificationArgs {
    HypothesisFalsificationArgs {
        hypotheses_reports: vec![root.join("miner_report.json")],
        pubtator_root: root.join("pubtator"),
        clinicaltrials_root: root.join("clinicaltrials"),
        dgidb_root: root.join("dgidb"),
        open_targets_root: root.join("open_targets"),
        out_dir: root.join("out"),
        max_hypotheses: 10,
    }
}

fn seed(root: &Path) {
    write(
        root.join("miner_report.json"),
        r#"{"hypotheses":[{"hypothesis_id":"typed-assoc:metformin::diabetes","source_id":"concept:metformin","source_name":"metformin","source_type":"chemical","target_id":"concept:type2diabetes","target_name":"type 2 diabetes","target_type":"disease","support_count":3,"score":0.9}]}"#,
    );
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
