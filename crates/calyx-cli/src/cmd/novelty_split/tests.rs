use std::fs;
use std::path::{Path, PathBuf};

use calyx_lodestar::{DiscoveryRunManifest, DiscoveryRunStage};
use serde_json::{Value, json};

use super::super::artifact_hash::sha256_hex;
use super::super::discovery_run_preflight::DiscoveryRunPreflightArgs;
use super::*;

#[test]
fn parse_accepts_repeated_atlases_and_preflight() {
    let args = parse_novelty_split(&[
        "--atlas".to_string(),
        "1188|infectious|target/a.jsonl".to_string(),
        "--atlas".to_string(),
        "1187|neuro|target/b.jsonl".to_string(),
        "--out-dir".to_string(),
        "target/out".to_string(),
        "--top-k".to_string(),
        "7".to_string(),
        "--run-manifest".to_string(),
        "target/manifest.json".to_string(),
        "--run-stage-id".to_string(),
        "novelty-calibration-split".to_string(),
    ])
    .unwrap();

    assert_eq!(args.atlases.len(), 2);
    assert_eq!(args.atlases[0].issue, "1188");
    assert_eq!(args.atlases[1].domain, "neuro");
    assert_eq!(args.out_dir, PathBuf::from("target/out"));
    assert_eq!(args.top_k, 7);
    assert_eq!(
        args.preflight.stage_id.as_deref(),
        Some("novelty-calibration-split")
    );
}

#[test]
fn split_routes_calibration_and_novelty_rows() {
    let root = temp_root("split");
    let input = root.join("atlas.jsonl");
    let out = root.join("out");
    write_jsonl(
        &input,
        &[
            json!({
                "candidate_id": "oncology-civic:11176",
                "candidate_type": "civic_precision_oncology_evidence",
                "evidence_level": "A",
                "external_validation": {"source": "CIViC"},
                "nct_ids": ["NCT01362803"],
                "gene": "NF1",
                "therapies": ["Selumetinib"],
                "cancer_type": "Plexiform Neurofibroma",
                "rank_score": 6.8
            }),
            json!({
                "candidate_id": "typed-assoc:tnf-proteinuria",
                "candidate_type": "typed_gene_disease_association",
                "target_name": "TNF",
                "disease_name": "Proteinuria",
                "rank": 2,
                "rank_score": 6.1
            }),
            json!({
                "hypothesis_id": "typed:chem-chem",
                "source_class": "typed_all_pair_miner",
                "source": {"name": "Steroids", "type": "chemical"},
                "target": {"name": "Theophylline", "type": "chemical"},
                "normalized_names": ["Steroids", "Theophylline"],
                "rank": 3,
                "rank_score": 5.0
            }),
        ],
    );

    run_novelty_split(NoveltySplitArgs {
        atlases: vec![AtlasInputArg {
            issue: "118x".to_string(),
            domain: "synthetic".to_string(),
            path: input,
        }],
        out_dir: out.clone(),
        top_k: 3,
        preflight: DiscoveryRunPreflightArgs::default(),
    })
    .unwrap();

    let calibration = read_jsonl(&out.join("calibration_known_positive_rows.jsonl"));
    let novelty = read_jsonl(&out.join("novelty_prioritized_research_leads.jsonl"));
    let readback: Value =
        serde_json::from_slice(&fs::read(out.join("persisted_readback.json")).unwrap()).unwrap();

    assert_eq!(calibration.len(), 2);
    assert_eq!(novelty.len(), 1);
    assert_eq!(novelty[0]["candidate_id"], "typed-assoc:tnf-proteinuria");
    assert_eq!(readback["assertions"]["split_rows_sum_to_total"], true);
    cleanup(root);
}

#[test]
fn stale_manifest_preflight_fails_before_output() {
    let root = temp_root("stale");
    let input = root.join("atlas.jsonl");
    let out = root.join("out");
    let manifest = root.join("manifest.json");
    write_jsonl(&input, &[json!({"candidate_id": "h1", "rank": 1})]);
    write_manifest(
        &manifest,
        &manifest_for_stage("novelty-calibration-split", &sha256_hex(b"fresh")),
    );

    let err = run_novelty_split(NoveltySplitArgs {
        atlases: vec![AtlasInputArg {
            issue: "118x".to_string(),
            domain: "synthetic".to_string(),
            path: input,
        }],
        out_dir: out.clone(),
        top_k: 3,
        preflight: DiscoveryRunPreflightArgs {
            manifest: Some(manifest),
            stage_id: Some("novelty-calibration-split".to_string()),
        },
    })
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_DISCOVERY_RUN_MANIFEST_CHAIN_BROKEN");
    assert!(!out.join("combined_original_ranked.jsonl").exists());
    cleanup(root);
}

fn write_jsonl(path: &Path, rows: &[Value]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut bytes = Vec::new();
    for row in rows {
        serde_json::to_writer(&mut bytes, row).unwrap();
        bytes.push(b'\n');
    }
    fs::write(path, bytes).unwrap();
}

fn read_jsonl(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn manifest_for_stage(stage_id: &str, input_sha256: &str) -> DiscoveryRunManifest {
    DiscoveryRunManifest {
        schema_version: 1,
        run_id: "issue1227-test".to_string(),
        corpus_vault_id: "clinical-vault".to_string(),
        panel_manifest_sha256: sha('a'),
        stages: vec![DiscoveryRunStage {
            stage_id: stage_id.to_string(),
            command: format!("calyx {stage_id}"),
            args: Vec::new(),
            upstream_stage_id: None,
            input_sha256: input_sha256.to_string(),
            output_sha256: sha('b'),
            git_sha: "issue1227".to_string(),
        }],
    }
}

fn write_manifest(path: &Path, manifest: &DiscoveryRunManifest) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec_pretty(manifest).unwrap()).unwrap();
}

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "calyx-novelty-split-{name}-{}-{}",
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
