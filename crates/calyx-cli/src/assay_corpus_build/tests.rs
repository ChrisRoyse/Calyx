use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::{Modality, QuantPolicy};
use calyx_registry::LensForgeManifest;
use serde_json::Value;

use super::data;
use super::lens;
use super::request::CorpusBuildRequest;
use super::worker;
use super::write;

#[test]
fn corpus_build_measures_algorithmic_code_and_sparse_lenses() {
    let root = temp_root("assay-corpus-algorithmic");
    let rows = root.join("rows.jsonl");
    let out_dir = root.join("out");
    write_code_rows(&rows, 60);
    let ast_manifest = write_manifest(
        &root,
        "code-ast.json",
        "code-ast-style",
        "algorithmic:ast-style",
        8,
    );
    let sparse_manifest = write_manifest(
        &root,
        "code-sparse.json",
        "code-sparse-keywords",
        "algorithmic:sparse-keywords",
        128,
    );
    let request = CorpusBuildRequest {
        rows_jsonl: rows,
        out_dir: out_dir.clone(),
        dataset: "code-fixture".to_string(),
        target_class: 0,
        manifests: vec![ast_manifest, sparse_manifest],
        limit_per_class: None,
        batch_size: 7,
        cost_override_json: None,
        embedding_model_id: Some("calyx-algorithmic-code+sparse".to_string()),
        worker_report: None,
    };

    let rows = data::load_rows(&request).unwrap();
    let measured = lens::measure_requested_lenses(&request, &rows).unwrap();
    let evidence = write::write_outputs(&request, &rows, &measured).unwrap();

    assert_eq!(evidence.n_samples, 60);
    assert!(out_dir.join("manifest.json").is_file());
    assert!(out_dir.join("vectors.jsonl").is_file());
    let persisted_report: Value =
        serde_json::from_slice(&fs::read(out_dir.join("corpus_build_report.json")).unwrap())
            .unwrap();
    assert_eq!(persisted_report["out_dir"], out_dir.display().to_string());
    let ast = evidence
        .lenses
        .iter()
        .find(|lens| lens.name == "code-ast-style")
        .unwrap();
    assert_eq!(ast.output_shape, "dense:8");
    assert_eq!(ast.assay_projection, "native_dense");
    assert_eq!(ast.vram_mb, 0.0);
    let sparse = evidence
        .lenses
        .iter()
        .find(|lens| lens.name == "code-sparse-keywords")
        .unwrap();
    assert_eq!(sparse.output_shape, "sparse:128");
    assert_eq!(sparse.assay_projection, "sparse_to_dense");
    assert_eq!(sparse.vram_mb, 0.0);

    let first_line = fs::read_to_string(out_dir.join("vectors.jsonl"))
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    let row: Value = serde_json::from_str(&first_line).unwrap();
    assert_eq!(row["source_event_time_secs"], 1_704_153_600_i64);
    assert_eq!(row["source_event_time_raw"], "2024-01-02T00:00:00Z");
    assert_eq!(row["temporal_lane_state"], "active");
    assert_eq!(row["lenses"]["code-ast-style"].as_array().unwrap().len(), 8);
    let sparse_vec = row["lenses"]["code-sparse-keywords"].as_array().unwrap();
    assert_eq!(sparse_vec.len(), 128);
    assert!(
        sparse_vec.iter().any(|value| value.as_f64().unwrap() > 0.0),
        "projected sparse vector must retain non-zero lexical evidence"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn corpus_build_accepts_source_rows_with_string_labels() {
    let root = temp_root("assay-corpus-source-labels");
    let rows = root.join("rows.jsonl");
    let mut lines = String::new();
    for idx in 0..60 {
        lines.push_str(&format!(
            "{}\n",
            serde_json::json!({
                "source": format!("ag_news://train.parquet#row={idx}"),
                "row": idx,
                "text": format!("real news row {idx} with enough lexical content"),
                "label": (idx % 2).to_string(),
                "anchor_audit": {
                    "anchor_leaks_into_input": true,
                    "trivial_anchor": true,
                    "grounded_gate_eligible": false,
                    "reason": "fixture label is present in input text"
                }
            })
        ));
    }
    fs::write(&rows, lines).unwrap();
    let request = rows_request(&rows, &root.join("out"));

    let loaded = data::load_rows(&request).unwrap();

    assert_eq!(loaded.rows.len(), 60);
    assert_eq!(loaded.rows[0].id, "ag_news://train.parquet#row=0");
    assert_eq!(loaded.rows[1].label, 1);
    assert_eq!(loaded.rows[0].event_time_secs, None);
    assert_eq!(loaded.rows[0].temporal_lane_state, "inactive");
    assert_eq!(
        loaded.rows[0].temporal_inactive_reason.as_deref(),
        Some("source_missing_created_at")
    );
    assert!(loaded.anchor_audit.anchor_leaks_into_input);
    assert!(loaded.anchor_audit.trivial_anchor);
    assert!(!loaded.anchor_audit.grounded_gate_eligible);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn corpus_build_measures_input_path_bytes_for_image_rows() {
    let root = temp_root("assay-corpus-image-path");
    let rows = root.join("rows.jsonl");
    let out_dir = root.join("out");
    let png_a = root.join("a.png");
    let png_b = root.join("b.png");
    fs::write(&png_a, sample_png(17)).unwrap();
    fs::write(&png_b, sample_png(29)).unwrap();
    write_image_path_rows(&rows, 60);
    let byte_manifest = write_manifest_for_modality(
        &root,
        "image-byte.json",
        "image-byte-features",
        "algorithmic:byte-features",
        16,
        Modality::Image,
    );
    let scalar_manifest = write_manifest_for_modality(
        &root,
        "image-scalar.json",
        "image-scalar",
        "algorithmic:scalar",
        1,
        Modality::Image,
    );
    let request = CorpusBuildRequest {
        rows_jsonl: rows,
        out_dir: out_dir.clone(),
        dataset: "image-path-fixture".to_string(),
        target_class: 0,
        manifests: vec![byte_manifest, scalar_manifest],
        limit_per_class: None,
        batch_size: 9,
        cost_override_json: None,
        embedding_model_id: Some("calyx-image-path-bytes".to_string()),
        worker_report: None,
    };

    let loaded = data::load_rows(&request).unwrap();
    assert!(loaded.rows[0].text.is_empty());
    assert!(loaded.rows[0].input_bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(loaded.rows[0].input_pointer.ends_with("a.png"));
    let measured = lens::measure_requested_lenses(&request, &loaded).unwrap();
    let evidence = write::write_outputs(&request, &loaded, &measured).unwrap();

    assert_eq!(evidence.n_samples, 60);
    let image_byte = evidence
        .lenses
        .iter()
        .find(|lens| lens.name == "image-byte-features")
        .unwrap();
    assert_eq!(image_byte.output_shape, "dense:16");
    assert_eq!(image_byte.assay_projection, "native_dense");
    let first_line = fs::read_to_string(out_dir.join("vectors.jsonl"))
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    let row: Value = serde_json::from_str(&first_line).unwrap();
    assert_eq!(
        row["lenses"]["image-byte-features"]
            .as_array()
            .unwrap()
            .len(),
        16
    );
    assert_eq!(row["lenses"]["image-scalar"].as_array().unwrap().len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn corpus_build_worker_writes_single_lens_report() {
    let root = temp_root("assay-corpus-worker");
    let rows = root.join("rows.jsonl");
    let report = root.join("worker.json");
    write_code_rows(&rows, 60);
    let manifest = write_manifest(
        &root,
        "code-ast.json",
        "code-ast-style",
        "algorithmic:ast-style",
        8,
    );
    let request = CorpusBuildRequest {
        rows_jsonl: rows,
        out_dir: root.join("out"),
        dataset: "worker-fixture".to_string(),
        target_class: 0,
        manifests: vec![manifest],
        limit_per_class: None,
        batch_size: 7,
        cost_override_json: None,
        embedding_model_id: None,
        worker_report: Some(report.clone()),
    };

    worker::run_worker(&request).unwrap();

    let persisted: lens::MeasuredLens =
        serde_json::from_slice(&fs::read(&report).unwrap()).unwrap();
    assert_eq!(persisted.name, "code-ast-style");
    assert_eq!(persisted.vectors.len(), 60);
    assert_eq!(persisted.vectors[0].len(), 8);
    assert_eq!(persisted.worker_pid, Some(std::process::id()));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn corpus_build_rejects_ambiguous_text_and_input_path() {
    let root = temp_root("assay-corpus-ambiguous-input");
    let rows = root.join("rows.jsonl");
    let image = root.join("image.png");
    fs::write(&image, sample_png(3)).unwrap();
    fs::write(
        &rows,
        format!(
            "{}\n",
            serde_json::json!({
                "source": "image://fixture#row=0",
                "text": "text bytes",
                "input_path": "image.png",
                "label": 0
            })
        ),
    )
    .unwrap();
    let request = rows_request(&rows, &root.join("out"));

    let error = data::load_rows(&request).unwrap_err();

    assert!(error.contains("requires exactly one of text or input_path"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn corpus_build_rejects_malformed_timestamp() {
    let root = temp_root("assay-corpus-bad-time");
    let rows = root.join("rows.jsonl");
    fs::write(
        &rows,
        format!(
            "{}\n",
            serde_json::json!({
                "source": "ag_news://train.parquet#row=0",
                "text": "bad timestamp row with enough lexical content",
                "label": "0",
                "event_time": "not-a-timestamp"
            })
        ),
    )
    .unwrap();
    let request = rows_request(&rows, &root.join("out"));

    let error = data::load_rows(&request).unwrap_err();

    assert!(error.contains("CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_TIMESTAMP"));
    assert!(error.contains("not-a-timestamp"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn corpus_build_rejects_invalid_string_label() {
    let root = temp_root("assay-corpus-bad-label");
    let rows = root.join("rows.jsonl");
    fs::write(
        &rows,
        format!(
            "{}\n",
            serde_json::json!({
                "source": "ag_news://train.parquet#row=0",
                "text": "bad label row",
                "label": "business"
            })
        ),
    )
    .unwrap();
    let request = rows_request(&rows, &root.join("out"));

    let error = data::load_rows(&request).unwrap_err();

    assert!(error.contains("label must be usize"));
    let _ = fs::remove_dir_all(root);
}

fn write_code_rows(path: &Path, rows: usize) {
    let mut lines = String::new();
    for idx in 0..rows {
        let label = idx % 2;
        let text = if label == 0 {
            format!(
                "fn parse_order_{idx}(input: &str) -> Result<Order, Error> {{ let token = input.trim(); parse_order(token) }}"
            )
        } else {
            format!(
                "struct LedgerEntry{idx} {{ amount: u64, account: String }} impl LedgerEntry{idx} {{ fn debit(&self) -> u64 {{ self.amount }} }}"
            )
        };
        lines.push_str(&format!(
            "{}\n",
            serde_json::json!({
                "id": format!("row-{idx}"),
                "split": "train",
                "text": text,
                "label": label,
                "event_time": format!("2024-01-02T00:{:02}:00Z", idx % 60)
            })
        ));
    }
    fs::write(path, lines).unwrap();
}

fn write_image_path_rows(path: &Path, rows: usize) {
    let mut lines = String::new();
    for idx in 0..rows {
        lines.push_str(&format!(
            "{}\n",
            serde_json::json!({
                "id": format!("image-row-{idx}"),
                "split": "train",
                "input_path": if idx % 2 == 0 { "a.png" } else { "b.png" },
                "label": idx % 2,
                "event_time": format!("2024-01-02T01:{:02}:00Z", idx % 60)
            })
        ));
    }
    fs::write(path, lines).unwrap();
}

fn sample_png(extra: u8) -> Vec<u8> {
    let mut bytes = vec![
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 2,
        0, 0, 0, 144, 119, 83, 222, 0, 0, 0, 13, 73, 68, 65, 84, 120, 218, 99, 100, 248, 207, 80,
        15, 0, 3, 134, 1, 128, 90, 52, 125, 107, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    bytes.push(extra);
    bytes
}

fn rows_request(rows_jsonl: &Path, out_dir: &Path) -> CorpusBuildRequest {
    CorpusBuildRequest {
        rows_jsonl: rows_jsonl.to_path_buf(),
        out_dir: out_dir.to_path_buf(),
        dataset: "rows-fixture".to_string(),
        target_class: 0,
        manifests: Vec::new(),
        limit_per_class: None,
        batch_size: 8,
        cost_override_json: None,
        embedding_model_id: None,
        worker_report: None,
    }
}

fn write_manifest(root: &Path, file_name: &str, name: &str, runtime: &str, dim: u32) -> PathBuf {
    write_manifest_for_modality(root, file_name, name, runtime, dim, Modality::Code)
}

fn write_manifest_for_modality(
    root: &Path,
    file_name: &str,
    name: &str,
    runtime: &str,
    dim: u32,
    modality: Modality,
) -> PathBuf {
    let manifest = LensForgeManifest {
        name: name.to_string(),
        modality,
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
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    root
}
