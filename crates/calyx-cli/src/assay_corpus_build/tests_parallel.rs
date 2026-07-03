use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::{Modality, QuantPolicy};
use calyx_registry::LensForgeManifest;

use super::data;
use super::lens;
use super::request::CorpusBuildRequest;
use super::write;

#[test]
fn corpus_build_lens_parallelism_preserves_persisted_outputs() {
    let root = temp_root("assay-corpus-parallel");
    let rows = root.join("rows.jsonl");
    write_code_rows(&rows, 60);
    let manifests: Vec<PathBuf> = (0..10)
        .map(|slot| {
            write_manifest(
                &root,
                &format!("lens-{slot}.json"),
                &format!("code-lens-{slot}"),
                if slot % 3 == 0 {
                    "algorithmic:sparse-keywords"
                } else {
                    "algorithmic:ast-style"
                },
                if slot % 3 == 0 { 128 } else { 8 },
            )
        })
        .collect();

    let sequential = CorpusBuildRequest {
        rows_jsonl: rows.clone(),
        out_dir: root.join("out-k1"),
        dataset: "parallel-fixture".to_string(),
        target_class: 0,
        manifests: manifests.clone(),
        limit_per_class: None,
        batch_size: 7,
        cost_override_json: None,
        embedding_model_id: Some("calyx-corpus-parallel".to_string()),
        worker_report: None,
        lens_parallelism: 1,
        worker_gpu_mem_limit_mib: None,
    };
    let loaded = data::load_rows(&sequential).unwrap();
    let measured = lens::measure_requested_lenses(&sequential, &loaded).unwrap();
    let sequential_evidence = write::write_outputs(&sequential, &loaded, &measured).unwrap();

    let mut parallel = sequential.clone();
    parallel.out_dir = root.join("out-k3");
    parallel.lens_parallelism = 3;
    parallel.worker_gpu_mem_limit_mib = Some(1024);
    let loaded = data::load_rows(&parallel).unwrap();
    let measured = lens::measure_requested_lenses(&parallel, &loaded).unwrap();
    let parallel_evidence = write::write_outputs(&parallel, &loaded, &measured).unwrap();

    assert_eq!(
        fs::read(sequential.out_dir.join("vectors.jsonl")).unwrap(),
        fs::read(parallel.out_dir.join("vectors.jsonl")).unwrap()
    );
    assert_eq!(
        fs::read(sequential.out_dir.join("manifest.json")).unwrap(),
        fs::read(parallel.out_dir.join("manifest.json")).unwrap()
    );
    assert_eq!(
        sequential_evidence
            .lenses
            .iter()
            .map(|lens| lens.name.as_str())
            .collect::<Vec<_>>(),
        parallel_evidence
            .lenses
            .iter()
            .map(|lens| lens.name.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(parallel_evidence.lenses.len(), 10);

    let _ = fs::remove_dir_all(root);
}

fn write_code_rows(path: &Path, rows: usize) {
    let mut lines = String::new();
    for idx in 0..rows {
        let label = idx % 2;
        let text = if label == 0 {
            format!(
                "fn parse_order_{idx}(input: &str) -> Result<Order, Error> {{ input.trim(); Ok(Order) }}"
            )
        } else {
            format!(
                "struct LedgerEntry{idx} {{ amount: u64 }} impl LedgerEntry{idx} {{ fn debit(&self) -> u64 {{ self.amount }} }}"
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
        batch_policy: None,
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
