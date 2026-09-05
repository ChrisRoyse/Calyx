use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::{Lens, Modality, QuantPolicy, SlotShape, SlotVector, StructuredRecord};
use calyx_registry::{AlgorithmicLens, LensForgeManifest, StructuredBinding};
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
        lens_parallelism: 1,
        worker_gpu_mem_limit_mib: None,
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
        lens_parallelism: 1,
        worker_gpu_mem_limit_mib: None,
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
        lens_parallelism: 1,
        worker_gpu_mem_limit_mib: None,
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
        lens_parallelism: 1,
        worker_gpu_mem_limit_mib: None,
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
        max_tokens: None,
        representation: None,
        special_tokens: None,
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

// ---------------------------------------------------------------------------
// #1914 / #1913: structured lens wiring on the assay corpus-build path.
//
// Acceptance bar: a structured lens measured through the ASSAY path must produce
// byte-identical slot vectors to the same lens through the INGEST path on the
// same record. These fixtures build real courtlistener-opinion-v1-shaped records
// with known values (synthetic known-outcome inputs; the measurement machinery is
// the real registry runtime, never mocked).
// ---------------------------------------------------------------------------

const COURTLISTENER_SCHEMA: &str = "courtlistener-opinion-v1";

fn numeric_citation_binding() -> StructuredBinding {
    // Roster lane 5: /citation_count Log1pSigned -> Dense(1).
    let json = serde_json::json!({
        "schema_id": COURTLISTENER_SCHEMA,
        "selector": {"kind": "field", "path": "/citation_count"},
        "source_type": "integer",
        "encoder": {"kind": "numeric", "scaling": {"kind": "log1p_signed"}}
    });
    StructuredBinding::from_json_bytes(&serde_json::to_vec(&json).unwrap()).unwrap()
}

fn one_hot_status_binding() -> StructuredBinding {
    // Roster lane 4: /precedential_status OneHot(frozen vocab) -> Dense(2).
    let json = serde_json::json!({
        "schema_id": COURTLISTENER_SCHEMA,
        "selector": {"kind": "field", "path": "/precedential_status"},
        "source_type": "string",
        "encoder": {"kind": "one_hot", "categories": ["Published", "Unpublished"]}
    });
    StructuredBinding::from_json_bytes(&serde_json::to_vec(&json).unwrap()).unwrap()
}

fn shape_dim(shape: SlotShape) -> u32 {
    match shape {
        SlotShape::Dense(dim) | SlotShape::Sparse(dim) => dim,
        SlotShape::Multi { token_dim } => token_dim,
    }
}

/// Writes a structured algorithmic lens manifest bound to `binding`, mirroring the
/// manifest a `calyx lens add --runtime algorithmic --modality structured` freeze
/// would persist (runtime kind carries the frozen binding).
fn write_structured_manifest(
    root: &Path,
    file_name: &str,
    name: &str,
    binding: &StructuredBinding,
) -> PathBuf {
    let kind = binding.runtime_kind().unwrap();
    write_manifest_for_modality(
        root,
        file_name,
        name,
        &format!("algorithmic:{kind}"),
        shape_dim(binding.shape()),
        Modality::Structured,
    )
}

fn courtlistener_record_json(idx: usize) -> Value {
    // Known values: citation_count spans zero and larger counts so Log1pSigned is
    // exercised across its range; precedential_status alternates over the frozen
    // OneHot vocabulary.
    serde_json::json!({
        "citation_count": (idx as i64) * 3,
        "precedential_status": if idx.is_multiple_of(2) { "Published" } else { "Unpublished" },
        "court_id": "scotus",
        "docket_number": format!("No. {idx}")
    })
}

/// Writes a real courtlistener-opinion-v1 row set with `record_schema`+`record`.
/// `schema_override` lets a test inject a schema id that disagrees with the frozen
/// binding to prove the schema-mismatch failure is closed; `with_record=false`
/// omits the typed record entirely to prove the missing-record failure is closed.
fn write_courtlistener_rows(
    path: &Path,
    rows: usize,
    with_record: bool,
    schema_override: Option<&str>,
) {
    let mut lines = String::new();
    for idx in 0..rows {
        let mut row = serde_json::json!({
            "id": format!("cl-op-{idx}"),
            "split": "train",
            "text": format!("real opinion body {idx}: the court holds and remands with enough lexical content"),
            "label": idx % 2,
        });
        if with_record {
            let object = row.as_object_mut().unwrap();
            object.insert(
                "record_schema".to_string(),
                Value::String(schema_override.unwrap_or(COURTLISTENER_SCHEMA).to_string()),
            );
            object.insert("record".to_string(), courtlistener_record_json(idx));
        }
        lines.push_str(&serde_json::to_string(&row).unwrap());
        lines.push('\n');
    }
    fs::write(path, lines).unwrap();
}

/// Measures `records` through the INGEST input-preparation shape, verbatim mirror
/// of `cmd::ingest::command::retention::prepared_hybrid_text_input`
/// (`prepare_text_input(text).with_structured(record)`), using the same frozen
/// algorithmic runtime the registry materializes from the lens spec. Returns the
/// dense slot data per row.
fn ingest_path_dense_vectors(
    binding: &StructuredBinding,
    records: &[StructuredRecord],
) -> Vec<Vec<f32>> {
    let kind = binding.runtime_kind().unwrap();
    let lens = AlgorithmicLens::from_runtime_kind(
        "ingest-ref",
        Modality::Structured,
        &kind,
        binding.shape(),
    )
    .unwrap();
    records
        .iter()
        .map(|record| {
            let input = calyx_aster::retained_input::prepare_text_input("ingest opinion body")
                .with_structured(record.clone());
            match lens.measure(&input).unwrap() {
                SlotVector::Dense { data, .. } => data,
                other => panic!("structured dense lens produced non-dense slot: {other:?}"),
            }
        })
        .collect()
}

#[test]
fn structured_lens_assay_path_matches_ingest_path_byte_for_byte() {
    let root = temp_root("assay-corpus-structured-identity");
    let rows = root.join("rows.jsonl");
    let out_dir = root.join("out");
    let n = 60;
    write_courtlistener_rows(&rows, n, true, None);

    let citation_binding = numeric_citation_binding();
    let status_binding = one_hot_status_binding();
    let citation_manifest = write_structured_manifest(
        &root,
        "citation.json",
        "citation-count-v1",
        &citation_binding,
    );
    let status_manifest = write_structured_manifest(
        &root,
        "status.json",
        "precedential-status-v1",
        &status_binding,
    );

    let request = CorpusBuildRequest {
        rows_jsonl: rows.clone(),
        out_dir: out_dir.clone(),
        dataset: "courtlistener-structured".to_string(),
        target_class: 0,
        manifests: vec![citation_manifest, status_manifest],
        limit_per_class: None,
        batch_size: 7,
        cost_override_json: None,
        embedding_model_id: Some("calyx-structured-courtlistener".to_string()),
        worker_report: None,
        lens_parallelism: 1,
        worker_gpu_mem_limit_mib: None,
    };

    // ASSAY path: full load_rows + measure_requested_lenses pipeline.
    let loaded = data::load_rows(&request).unwrap();
    assert_eq!(loaded.rows.len(), n);
    assert!(
        loaded.rows.iter().all(|row| row.structured.is_some()),
        "every courtlistener row must carry its canonical typed record on the assay path"
    );
    let measured = lens::measure_requested_lenses(&request, &loaded).unwrap();

    // INGEST reference: the same records measured through the ingest input shape.
    let records: Vec<StructuredRecord> = (0..n)
        .map(|idx| {
            StructuredRecord::from_json_value(COURTLISTENER_SCHEMA, &courtlistener_record_json(idx))
                .unwrap()
        })
        .collect();

    for (binding, lens_name) in [
        (&citation_binding, "citation-count-v1"),
        (&status_binding, "precedential-status-v1"),
    ] {
        let assay = measured
            .iter()
            .find(|lens| lens.name == lens_name)
            .unwrap_or_else(|| panic!("assay measured lens {lens_name}"));
        let ingest = ingest_path_dense_vectors(binding, &records);
        assert_eq!(assay.vectors.len(), ingest.len());
        for (idx, (assay_vec, ingest_vec)) in assay.vectors.iter().zip(&ingest).enumerate() {
            let assay_bits: Vec<u32> = assay_vec.iter().map(|value| value.to_bits()).collect();
            let ingest_bits: Vec<u32> = ingest_vec.iter().map(|value| value.to_bits()).collect();
            assert_eq!(
                assay_bits, ingest_bits,
                "lens {lens_name} row {idx}: assay slot vector must be byte-identical to the ingest slot vector"
            );
        }
    }

    let _ = fs::remove_dir_all(root);
}

#[test]
fn structured_lens_assay_missing_record_fails_closed() {
    let root = temp_root("assay-corpus-structured-missing");
    let rows = root.join("rows.jsonl");
    // Rows carry NO typed record, but a structured lens is requested: the assay
    // path must fail closed with the stable missing-field code, never Absent.
    write_courtlistener_rows(&rows, 60, false, None);
    let binding = numeric_citation_binding();
    let manifest = write_structured_manifest(&root, "citation.json", "citation-count-v1", &binding);
    let request = CorpusBuildRequest {
        rows_jsonl: rows,
        out_dir: root.join("out"),
        dataset: "courtlistener-missing-record".to_string(),
        target_class: 0,
        manifests: vec![manifest],
        limit_per_class: None,
        batch_size: 8,
        cost_override_json: None,
        embedding_model_id: None,
        worker_report: None,
        lens_parallelism: 1,
        worker_gpu_mem_limit_mib: None,
    };

    let loaded = data::load_rows(&request).unwrap();
    let error = lens::measure_requested_lenses(&request, &loaded).unwrap_err();
    assert!(
        error.contains(calyx_core::CALYX_STRUCTURED_FIELD_MISSING),
        "expected CALYX_STRUCTURED_FIELD_MISSING, got: {error}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn structured_lens_assay_schema_mismatch_fails_closed() {
    let root = temp_root("assay-corpus-structured-schema");
    let rows = root.join("rows.jsonl");
    // Records declare a schema id that disagrees with the frozen binding schema.
    write_courtlistener_rows(&rows, 60, true, Some("wrong-opinion-schema-v9"));
    let binding = numeric_citation_binding();
    let manifest = write_structured_manifest(&root, "citation.json", "citation-count-v1", &binding);
    let request = CorpusBuildRequest {
        rows_jsonl: rows,
        out_dir: root.join("out"),
        dataset: "courtlistener-schema-mismatch".to_string(),
        target_class: 0,
        manifests: vec![manifest],
        limit_per_class: None,
        batch_size: 8,
        cost_override_json: None,
        embedding_model_id: None,
        worker_report: None,
        lens_parallelism: 1,
        worker_gpu_mem_limit_mib: None,
    };

    // load_rows accepts the record (schema id is caller-declared); measurement is
    // where the frozen binding rejects the mismatched schema, failing closed.
    let loaded = data::load_rows(&request).unwrap();
    let error = lens::measure_requested_lenses(&request, &loaded).unwrap_err();
    assert!(
        error.contains(calyx_core::CALYX_STRUCTURED_SCHEMA_MISMATCH),
        "expected CALYX_STRUCTURED_SCHEMA_MISMATCH, got: {error}"
    );
    let _ = fs::remove_dir_all(root);
}
