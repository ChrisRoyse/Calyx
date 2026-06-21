use super::request::CorpusBuildRequest;
use super::{data, lens, write};
use calyx_core::{Modality, QuantPolicy};
use calyx_registry::LensForgeManifest;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn corpus_build_accepts_single_manifest_for_candidate_assay() {
    let request = CorpusBuildRequest::parse(&[
        "--rows-jsonl".to_string(),
        "rows.jsonl".to_string(),
        "--out-dir".to_string(),
        "out".to_string(),
        "--dataset".to_string(),
        "single-manifest".to_string(),
        "--target-class".to_string(),
        "0".to_string(),
        "--manifest".to_string(),
        "one.json".to_string(),
    ])
    .unwrap();

    assert_eq!(request.manifests.len(), 1);
}

#[test]
fn corpus_build_writes_single_manifest_candidate_outputs() {
    let root = temp_root("assay-corpus-single-manifest-write");
    let rows = root.join("rows.jsonl");
    let out_dir = root.join("out");
    write_code_rows(&rows, 60);
    let manifest = write_manifest(
        &root,
        "code-ast.json",
        "code-ast-single-candidate",
        "algorithmic:ast-style",
        8,
    );
    let request = CorpusBuildRequest {
        rows_jsonl: rows,
        out_dir: out_dir.clone(),
        dataset: "single-candidate-fixture".to_string(),
        target_class: 0,
        manifests: vec![manifest],
        limit_per_class: None,
        batch_size: 7,
        cost_override_json: None,
        embedding_model_id: None,
        worker_report: None,
    };

    let rows = data::load_rows(&request).unwrap();
    let measured = lens::measure_requested_lenses(&request, &rows).unwrap();
    let evidence = write::write_outputs(&request, &rows, &measured).unwrap();

    assert_eq!(evidence.n_samples, 60);
    assert_eq!(evidence.lenses.len(), 1);
    assert!(out_dir.join("manifest.json").is_file());
    assert!(out_dir.join("vectors.jsonl").is_file());
    let first_line = fs::read_to_string(out_dir.join("vectors.jsonl"))
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    let row: Value = serde_json::from_str(&first_line).unwrap();
    assert_eq!(
        row["lenses"]["code-ast-single-candidate"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn corpus_build_rejects_zero_manifests_for_candidate_assay() {
    let error = CorpusBuildRequest::parse(&[
        "--rows-jsonl".to_string(),
        "rows.jsonl".to_string(),
        "--out-dir".to_string(),
        "out".to_string(),
        "--dataset".to_string(),
        "zero-manifest".to_string(),
        "--target-class".to_string(),
        "0".to_string(),
    ])
    .unwrap_err();

    assert!(error.contains("provide at least one --manifest entry"));
}

fn write_code_rows(path: &Path, rows: usize) {
    let mut lines = String::new();
    for idx in 0..rows {
        lines.push_str(&format!(
            "{}\n",
            serde_json::json!({
                "id": format!("single-row-{idx}"),
                "split": "train",
                "text": format!("fn candidate_{idx}(input: &str) -> usize {{ input.len() + {idx} }}"),
                "label": idx % 2,
            })
        ));
    }
    fs::write(path, lines).unwrap();
}

fn write_manifest(root: &Path, file_name: &str, name: &str, runtime: &str, dim: u32) -> PathBuf {
    let manifest = LensForgeManifest {
        name: name.to_string(),
        modality: Modality::Code,
        runtime: runtime.to_string(),
        dim,
        shape: None,
        dtype: "f32".to_string(),
        weights_sha256: String::new(),
        artifact_set_sha256: None,
        files: Vec::new(),
        pooling: "algorithmic".to_string(),
        norm: "none".to_string(),
        source_hf_id: format!("calyx/{name}"),
        endpoint: None,
        license: Some("apache-2.0".to_string()),
        non_commercial: false,
        quant_default: QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: calyx_registry::spec::default_recall_delta(),
        max_batch: None,
    };
    let path = root.join(file_name);
    fs::write(&path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    path
}

fn temp_root(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("{name}-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    root
}
