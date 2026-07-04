use std::fs;
use std::path::{Path, PathBuf};

use calyx_lodestar::{
    DiscoveryRunManifest, DiscoveryRunStage, HypothesisEvaluationParams, RankedHypothesisParams,
    aggregate_hypothesis_evaluations, rank_traceable_hypotheses,
};
use serde::de::DeserializeOwned;
use serde_json::json;

use super::super::discovery_run_preflight::DiscoveryRunPreflightArgs;
use super::types::{
    EvaluateRankBridgeArgs, EvaluationBridgeOutput, FalsificationEvaluateBridgeArgs,
    RankBridgeOutput,
};
use super::*;

#[test]
fn falsification_bridge_output_is_accepted_by_evaluator() {
    let root = temp_root("falsification-evaluate");
    let miner = root.join("typed_association_miner_report.json");
    let falsification = root.join("falsification_sweep_report.json");
    let out = root.join("hypothesis_evaluate.input.json");
    write_json(&miner, &miner_report());
    write_json(&falsification, &falsification_report());

    run_bridge_falsification_evaluate(FalsificationEvaluateBridgeArgs {
        miner_report: miner,
        falsification_report: falsification,
        out: out.clone(),
        preflight: DiscoveryRunPreflightArgs::default(),
    })
    .unwrap();

    let output: EvaluationBridgeOutput = read_json(&out);
    let report =
        aggregate_hypothesis_evaluations(&output.inputs, &HypothesisEvaluationParams::default())
            .unwrap();
    let sidecar: serde_json::Value = read_json(&out.with_extension("readback.json"));
    assert_eq!(output.inputs.len(), 1);
    assert_eq!(report.retained_count, 1);
    assert_eq!(sidecar["readback_input_count"], 1);
    assert_eq!(sidecar["research_lead_only"], true);
    cleanup(root);
}

#[test]
fn evaluate_rank_bridge_output_is_accepted_by_ranker() {
    let root = temp_root("evaluate-rank");
    let evaluation = root.join("hypothesis_evaluation_report.json");
    let out = root.join("rank.input.json");
    let eval_input = evaluation_input_file();
    let report = aggregate_hypothesis_evaluations(
        &eval_input.inputs,
        &HypothesisEvaluationParams::default(),
    )
    .unwrap();
    write_json(&evaluation, &json!({"schema_version": 1, "report": report}));

    run_bridge_evaluate_rank(EvaluateRankBridgeArgs {
        evaluation_report: evaluation,
        out: out.clone(),
        preflight: DiscoveryRunPreflightArgs::default(),
    })
    .unwrap();

    let output: RankBridgeOutput = read_json(&out);
    let ranked =
        rank_traceable_hypotheses(&output.inputs, &RankedHypothesisParams::default()).unwrap();
    let sidecar: serde_json::Value = read_json(&out.with_extension("readback.json"));
    assert_eq!(output.inputs.len(), 1);
    assert_eq!(ranked.ranked_count, 1);
    assert_eq!(ranked.human_review_count, 1);
    assert_eq!(sidecar["readback_input_count"], 1);
    cleanup(root);
}

#[test]
fn stale_preflight_fails_before_output() {
    let root = temp_root("stale-preflight");
    let miner = root.join("typed_association_miner_report.json");
    let falsification = root.join("falsification_sweep_report.json");
    let manifest = root.join("manifest.json");
    let out = root.join("hypothesis_evaluate.input.json");
    write_json(&miner, &miner_report());
    write_json(&falsification, &falsification_report());
    write_json(
        &manifest,
        &manifest_for_stage("bridge-falsification-evaluate"),
    );

    let err = run_bridge_falsification_evaluate(FalsificationEvaluateBridgeArgs {
        miner_report: miner,
        falsification_report: falsification,
        out: out.clone(),
        preflight: DiscoveryRunPreflightArgs {
            manifest: Some(manifest),
            stage_id: Some("bridge-falsification-evaluate".to_string()),
        },
    })
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_DISCOVERY_RUN_MANIFEST_CHAIN_BROKEN");
    assert!(!out.exists());
    assert!(!out.with_extension("readback.json").exists());
    cleanup(root);
}

#[derive(serde::Deserialize)]
struct EvalInputFile {
    inputs: Vec<calyx_lodestar::HypothesisEvaluationInput>,
}

fn evaluation_input_file() -> EvalInputFile {
    let root = temp_root("eval-input-source");
    let miner = root.join("typed_association_miner_report.json");
    let falsification = root.join("falsification_sweep_report.json");
    let out = root.join("hypothesis_evaluate.input.json");
    write_json(&miner, &miner_report());
    write_json(&falsification, &falsification_report());
    run_bridge_falsification_evaluate(FalsificationEvaluateBridgeArgs {
        miner_report: miner,
        falsification_report: falsification,
        out: out.clone(),
        preflight: DiscoveryRunPreflightArgs::default(),
    })
    .unwrap();
    let file = read_json(&out);
    cleanup(root);
    file
}

fn miner_report() -> serde_json::Value {
    json!({
        "hypotheses": [{
            "hypothesis_id": "typed-assoc:drug:metformin::disease:cancer",
            "source_id": "concept:drug:metformin",
            "source_name": "metformin",
            "source_type": "chemical",
            "target_id": "concept:disease:cancer",
            "target_name": "cancer",
            "target_type": "disease",
            "path_count": 3,
            "support_count": 5,
            "score": 0.9,
            "novelty_score": 0.7,
            "clinical_boundary": "Association hypothesis only."
        }]
    })
}

fn falsification_report() -> serde_json::Value {
    json!({
        "hypothesis_flags": [{
            "hypothesis_id": "typed-assoc:drug:metformin::disease:cancer",
            "support_evidence_count": 1,
            "counter_evidence_count": 0,
            "support_weight": 0.9,
            "counter_weight": 0.0,
            "falsification_score": 0.0,
            "reason_codes": ["supporting_literature_exact_pair"],
            "sweep_status": "complete_no_counterevidence_found_in_current_sources",
            "human_review_atlas_status": "falsification_sweep_complete",
            "clinical_boundary": "Research lead only."
        }],
        "support_evidence": [{
            "hypothesis_id": "typed-assoc:drug:metformin::disease:cancer",
            "evidence_kind": "support",
            "source_system": "PubTator",
            "reason_code": "supporting_literature_exact_pair",
            "source_path": "/fsv/pubtator/support.jsonl",
            "source_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "source_row_index": 7,
            "weight": 2.0,
            "summary": "Persisted literature row links metformin and cancer as an association signal."
        }],
        "counter_evidence": []
    })
}

fn manifest_for_stage(stage_id: &str) -> DiscoveryRunManifest {
    DiscoveryRunManifest {
        schema_version: 1,
        run_id: "issue1220-bridge-test".to_string(),
        corpus_vault_id: "issue1220-vault".to_string(),
        panel_manifest_sha256: sha('a'),
        stages: vec![DiscoveryRunStage {
            stage_id: stage_id.to_string(),
            command: format!("calyx {stage_id}"),
            args: Vec::new(),
            upstream_stage_id: None,
            input_sha256: sha('f'),
            output_sha256: sha('b'),
            git_sha: sha('c'),
        }],
    }
}

fn write_json(path: &Path, value: &impl serde::Serialize) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn read_json<T: DeserializeOwned>(path: &Path) -> T {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "calyx-discovery-bridge-{name}-{}-{}",
        std::process::id(),
        ulid::Ulid::new()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn cleanup(path: PathBuf) {
    fs::remove_dir_all(path).unwrap();
}

fn sha(ch: char) -> String {
    std::iter::repeat_n(ch, 64).collect()
}
