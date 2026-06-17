use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::{Modality, QuantPolicy};
use calyx_registry::LensForgeManifest;
use serde_json::Value;

use super::data;
use super::lens;
use super::request::CorpusBuildRequest;
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
    };

    let rows = data::load_rows(&request).unwrap();
    let lenses = lens::load_lenses(&request).unwrap();
    let measured = lens::measure_lenses(&request, &rows, lenses).unwrap();
    let evidence = write::write_outputs(&request, &rows, &measured).unwrap();

    assert_eq!(evidence.n_samples, 60);
    assert!(out_dir.join("manifest.json").is_file());
    assert!(out_dir.join("vectors.jsonl").is_file());
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
    assert_eq!(row["lenses"]["code-ast-style"].as_array().unwrap().len(), 8);
    let sparse_vec = row["lenses"]["code-sparse-keywords"].as_array().unwrap();
    assert_eq!(sparse_vec.len(), 128);
    assert!(
        sparse_vec.iter().any(|value| value.as_f64().unwrap() > 0.0),
        "projected sparse vector must retain non-zero lexical evidence"
    );

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
                "label": label
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
