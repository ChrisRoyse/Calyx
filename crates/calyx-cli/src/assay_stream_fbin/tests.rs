use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::{Modality, QuantPolicy};
use calyx_registry::LensForgeManifest;
use calyx_sextant::index::I8BinVectors;
use serde_json::Value;

use super::args::Args;
use super::format::VectorFormat;
use super::write;

#[test]
fn stream_fbin_writes_structured_progress_snapshot() {
    let fixture = Fixture::new("stream-fbin-progress", 10, 10, 50);
    let args = fixture.args(8);

    write::run(&args).unwrap();

    let progress_path = fixture.out.join("stream_fbin_progress.json");
    let progress: Value = serde_json::from_slice(&fs::read(&progress_path).unwrap()).unwrap();
    assert_eq!(progress["schema"], "calyx-assay-stream-fbin-progress-v1");
    assert_eq!(progress["state"], "complete");
    assert_eq!(progress["event"], "export_complete");
    assert_eq!(progress["dataset"], "unit_stream_fbin");
    assert_eq!(progress["rows_total"], 50);
    assert_eq!(progress["query_count"], 8);
    assert_eq!(progress["lens_total"], 10);
    assert_eq!(progress["lenses_completed"], 10);
    assert_eq!(progress["completed_corpus_rows"], 500);
    assert_eq!(progress["completed_query_rows"], 80);
    assert_eq!(progress["vector_format"], "fbin");
    assert_eq!(
        progress["vector_storage_contract"],
        "f32-row-major-calyx-fbin"
    );
    assert_eq!(progress["total_lens_corpus_rows_expected"], 500);
    assert_eq!(progress["total_lens_query_rows_expected"], 80);
    assert_eq!(progress["current_lens"], Value::Null);
    assert_eq!(progress["streaming_fbin_source"], true);
    assert_eq!(progress["temporal_counts_toward_a35"], false);
    assert_eq!(
        progress["temporal_lane_role"],
        "event_time_forward_backward_as_of_sidecar"
    );
    assert_eq!(
        progress["progress_path"].as_str().unwrap(),
        progress_path.display().to_string()
    );

    let report: Value =
        serde_json::from_slice(&fs::read(fixture.out.join("stream_fbin_report.json")).unwrap())
            .unwrap();
    assert_eq!(
        report["progress_path"].as_str().unwrap(),
        progress_path.display().to_string()
    );
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn stream_fbin_can_emit_i8bin_vector_sources() {
    let fixture = Fixture::new("stream-i8bin-output", 10, 10, 50);
    let mut args = fixture.args(8);
    args.vector_format = VectorFormat::I8Bin;

    write::run(&args).unwrap();

    let corpus_path = fixture.out.join("i8bin/slot_00_lens-0_corpus.i8bin");
    let queries_path = fixture.out.join("i8bin/slot_00_lens-0_queries.i8bin");
    let corpus = I8BinVectors::open(&corpus_path).unwrap();
    let queries = I8BinVectors::open(&queries_path).unwrap();
    assert_eq!(corpus.count(), 50);
    assert_eq!(corpus.dim(), 4);
    assert_eq!(queries.count(), 8);
    assert_eq!(queries.dim(), 4);
    assert_eq!(fs::metadata(&corpus_path).unwrap().len(), 8 + 50 * 4);

    let plan: Value =
        serde_json::from_slice(&fs::read(fixture.out.join("partitioned_rrf_plan.json")).unwrap())
            .unwrap();
    assert!(
        plan["slots"][0]["corpus"]
            .as_str()
            .unwrap()
            .ends_with(".i8bin")
    );
    assert!(
        plan["slots"][0]["queries"]
            .as_str()
            .unwrap()
            .ends_with(".i8bin")
    );

    let report: Value =
        serde_json::from_slice(&fs::read(fixture.out.join("stream_fbin_report.json")).unwrap())
            .unwrap();
    assert_eq!(report["vector_format"], "i8bin");
    assert_eq!(report["fbin_dir"], Value::Null);
    assert!(
        report["vector_dir"]
            .as_str()
            .unwrap()
            .replace('\\', "/")
            .ends_with("/i8bin")
    );
    assert_eq!(
        report["vector_storage_contract"],
        "per-row-directional-symmetric-int8-normalized-on-read"
    );
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn stream_fbin_rejects_panel_below_a35_floor() {
    let fixture = Fixture::new("stream-fbin-too-small", 3, 3, 50);
    let args = fixture.args(8);

    let error = write::run(&args).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_ASSAY_STREAM_FBIN_PANEL_TOO_SMALL");
    assert!(!fixture.out.exists());
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn stream_fbin_rejects_panel_floor_before_row_floor() {
    let fixture = Fixture::new("stream-fbin-too-small-before-rows", 4, 4, 8);
    let args = fixture.args(2);

    let error = write::run(&args).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_ASSAY_STREAM_FBIN_PANEL_TOO_SMALL");
    assert!(!fixture.out.exists());
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn stream_fbin_rejects_existing_output_before_loading_inputs() {
    let fixture = Fixture::new("stream-fbin-output-exists-first", 10, 10, 50);
    fs::create_dir_all(&fixture.out).unwrap();
    let mut args = fixture.args(8);
    args.rows_jsonl = fixture.root.join("missing-rows.jsonl");

    let error = write::run(&args).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_ASSAY_STREAM_FBIN_OUTPUT_EXISTS");
    assert!(fixture.out.exists());
    assert_eq!(fs::read_dir(&fixture.out).unwrap().count(), 0);
    let _ = fs::remove_dir_all(fixture.root);
}

struct Fixture {
    root: PathBuf,
    rows: PathBuf,
    out: PathBuf,
    bits: PathBuf,
    manifests: Vec<PathBuf>,
}

impl Fixture {
    fn new(name: &str, manifest_count: usize, admitted_lenses: usize, rows: usize) -> Self {
        let root = temp_root(name);
        let manifest_root = root.join("manifests");
        fs::create_dir_all(&manifest_root).unwrap();
        let manifests = write_manifests(&manifest_root, manifest_count);
        let rows_path = root.join("rows.jsonl");
        write_rows(&rows_path, rows);
        let bits = root.join("assay_abundance.json");
        write_bits(&bits, manifest_count, admitted_lenses);
        Self {
            out: root.join("out"),
            root,
            rows: rows_path,
            bits,
            manifests,
        }
    }

    fn args(&self, query_count: usize) -> Args {
        Args {
            rows_jsonl: self.rows.clone(),
            out_dir: self.out.clone(),
            dataset: "unit_stream_fbin".to_string(),
            target_class: 1,
            manifests: self.manifests.clone(),
            bits_report: self.bits.clone(),
            query_count,
            limit_per_class: None,
            batch_size: 7,
            cost_override_json: None,
            embedding_model_id: None,
            min_bits: 0.05,
            vector_format: VectorFormat::Fbin,
        }
    }
}

fn write_rows(path: &Path, rows: usize) {
    let mut lines = String::new();
    for row in 0..rows {
        lines.push_str(
            &serde_json::json!({
                "id": format!("row-{row}"),
                "text": format!("unit stream fbin row {row}"),
                "label": row % 2,
                "event_time": 1_704_153_600_i64 + row as i64
            })
            .to_string(),
        );
        lines.push('\n');
    }
    fs::write(path, lines).unwrap();
}

fn write_bits(path: &Path, lenses: usize, admitted: usize) {
    let lenses = (0..lenses)
        .map(|idx| {
            serde_json::json!({
                "name": format!("lens-{idx}"),
                "bits_about": 0.2,
                "admitted": idx < admitted
            })
        })
        .collect::<Vec<_>>();
    fs::write(
        path,
        serde_json::to_vec_pretty(&serde_json::json!({ "lenses": lenses })).unwrap(),
    )
    .unwrap();
}

fn write_manifests(root: &Path, count: usize) -> Vec<PathBuf> {
    (0..count)
        .map(|idx| {
            let path = root.join(format!("lens-{idx}.json"));
            let manifest = LensForgeManifest {
                name: format!("lens-{idx}"),
                modality: Modality::Text,
                runtime: "algorithmic:one-hot:4".to_string(),
                dim: 4,
                dtype: "f32".to_string(),
                weights_sha256: String::new(),
                artifact_set_sha256: None,
                files: Vec::new(),
                pooling: "algorithmic".to_string(),
                norm: "none".to_string(),
                source_hf_id: format!("calyx/lens-{idx}"),
                endpoint: None,
                license: Some("apache-2.0".to_string()),
                non_commercial: false,
                quant_default: QuantPolicy::turboquant_default(),
                truncate_dim: None,
                recall_delta: calyx_registry::spec::default_recall_delta(),
                max_batch: None,
            };
            fs::write(&path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
            path
        })
        .collect()
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
