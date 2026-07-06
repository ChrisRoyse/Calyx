use std::fs;
use std::path::{Path, PathBuf};

use super::engine::{enforce_a37_mode, evaluate};
use super::metrics::write_outputs;
use super::request::{A37CardMode, I8binEnsembleRequest};

const DIM: usize = 4;

#[test]
fn i8bin_card_reads_vector_bytes_and_persists_payload() {
    let root = temp_root("i8bin-ensemble-card-pass");
    fs::create_dir_all(&root).unwrap();
    let rows = root.join("rows.jsonl");
    write_rows(&rows, 120);
    let plan = write_plan(&root, 10, 120);
    let report_path = write_stream_report(&root, 10, 120);
    let request = request_for(&root, plan, rows, Some(report_path), 80, Some(60));

    let report = evaluate(&request).unwrap();
    let evidence = write_outputs(&request, &report).unwrap();

    assert_eq!(report.card.panel_lens_count, 10);
    assert_eq!(report.card.pairs.len(), 45);
    assert_eq!(report.sample_rows_selected, 80);
    assert_eq!(report.signature_rows, 60);
    assert_eq!(report.a37_mode, "diagnostic");
    assert!(!report.a37_gate_required);
    assert_eq!(report.diversity.association_family_count, 1);
    assert!(
        report
            .diversity
            .association_families
            .contains_key("dense_semantic_general")
    );
    assert_eq!(report.diversity.status, "diagnostic_only");
    assert_eq!(report.card.a37_diversity.status, "diagnostic_only");
    assert_eq!(evidence.assay_cf_rows_persisted, 58);
    assert_eq!(evidence.assay_cf_subject_counts["lens"], 10);
    assert_eq!(evidence.assay_cf_subject_counts["ensemble_card"], 1);
    assert!(evidence.ensemble_card_payload_readback);
    assert!(Path::new(&evidence.a37_report_path).is_file());
    assert!(Path::new(&evidence.matrix_path).is_file());

    let matrix_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&evidence.matrix_path).unwrap()).unwrap();
    assert_eq!(matrix_json["pairs"].as_array().unwrap().len(), 45);
    let card_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&evidence.ensemble_card_path).unwrap()).unwrap();
    assert_eq!(card_json["panel_lens_count"], 10);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn i8bin_card_db_only_persists_without_metrics_artifacts() {
    let root = temp_root("i8bin-ensemble-card-db-only");
    fs::create_dir_all(&root).unwrap();
    let rows = root.join("rows.jsonl");
    write_rows(&rows, 120);
    let plan = write_plan(&root, 10, 120);
    let mut request = request_for(&root, plan, rows, None, 80, Some(60));
    request.emit_artifacts = false;
    request.metrics_dir = PathBuf::new();
    request.cf_root = root.join("shared-assay-cf");

    let report = evaluate(&request).unwrap();
    let evidence = write_outputs(&request, &report).unwrap();

    assert_eq!(evidence.artifact_mode, "db_only");
    assert_eq!(evidence.assay_cf_rows_persisted, 58);
    assert!(evidence.ensemble_card_payload_readback);
    assert!(evidence.a37_report_path.is_empty());
    assert!(evidence.matrix_path.is_empty());
    assert!(!root.join("metrics").exists());
    assert!(request.cf_root.join("cf").join("assay").is_dir());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn i8bin_card_reads_plan_from_graph_cf() {
    let root = temp_root("i8bin-ensemble-card-plan-db");
    fs::create_dir_all(&root).unwrap();
    let rows = root.join("rows.jsonl");
    write_rows(&rows, 120);
    let plan = write_plan(&root, 10, 120);
    let loaded = crate::partitioned_bench::rrf_plan::load_from_file(&plan).unwrap();
    let plan_cf_root = root.join("plan-cf");
    crate::partitioned_bench::rrf_plan::write(
        &plan_cf_root,
        crate::partitioned_bench::rrf_plan::DEFAULT_ASSOCIATION_KEY,
        &crate::partitioned_bench::rrf_plan::PartitionedRrfPlanRecord {
            format: crate::partitioned_bench::rrf_plan::FORMAT.to_string(),
            mode: crate::partitioned_bench::rrf_plan::MODE.to_string(),
            imported_plan_sha256: loaded.plan_sha256,
            base_dir: loaded.base_dir,
            plan: loaded.plan,
        },
    )
    .unwrap();
    let mut request = request_for(&root, PathBuf::new(), rows, None, 80, Some(60));
    request.plan = None;
    request.plan_cf_root = Some(plan_cf_root.clone());
    request.emit_artifacts = false;
    request.metrics_dir = PathBuf::new();
    request.cf_root = root.join("shared-assay-cf");

    let report = evaluate(&request).unwrap();
    let evidence = write_outputs(&request, &report).unwrap();

    assert_eq!(report.plan_source.mode, "aster_graph_cf");
    let expected_cf_root = plan_cf_root.display().to_string();
    assert_eq!(
        report.plan_source.cf_root.as_deref(),
        Some(expected_cf_root.as_str())
    );
    assert!(report.plan_source.db_readback.unwrap().readback_matches);
    assert_eq!(evidence.artifact_mode, "db_only");
    assert_eq!(evidence.assay_cf_subject_counts["ensemble_card"], 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn i8bin_card_reads_labels_from_graph_cf() {
    let root = temp_root("i8bin-ensemble-card-labels-db");
    fs::create_dir_all(&root).unwrap();
    let rows = root.join("rows.jsonl");
    write_rows(&rows, 120);
    let labels_cf_root = write_labels_db(&root, &rows, 1);
    let plan = write_plan(&root, 10, 120);
    let mut request = request_for(&root, plan, PathBuf::new(), None, 80, Some(60));
    request.labels_cf_root = Some(labels_cf_root.clone());
    request.labels_key = "unit_labels".to_string();
    request.emit_artifacts = false;
    request.metrics_dir = PathBuf::new();
    request.cf_root = root.join("shared-assay-cf");

    let report = evaluate(&request).unwrap();
    let evidence = write_outputs(&request, &report).unwrap();

    assert_eq!(report.label_source.mode, "aster_graph_cf");
    assert_eq!(report.label_source.row_count, 120);
    assert_eq!(report.label_source.positive_count, 60);
    assert_eq!(report.rows_jsonl, "");
    assert!(
        report
            .label_source
            .db_readback
            .as_ref()
            .unwrap()
            .readback_matches
    );
    assert_eq!(evidence.artifact_mode, "db_only");
    assert_eq!(evidence.assay_cf_subject_counts["ensemble_card"], 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn label_import_limit_per_class_matches_stream_selection() {
    let root = temp_root("i8bin-label-limit-per-class");
    fs::create_dir_all(&root).unwrap();
    let rows = root.join("rows.jsonl");
    write_rows(&rows, 10);

    let imported = super::label_store::load_rows_jsonl(
        &rows,
        1,
        &super::label_store::AnchorSpec::Label,
        Some(3),
    )
    .unwrap();

    assert_eq!(imported.labels.len(), 6);
    assert_eq!(imported.labels.iter().filter(|value| **value).count(), 3);
    assert_eq!(imported.label_counts["0"], 3);
    assert_eq!(imported.label_counts["1"], 3);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn default_gate_mode_refuses_homogeneous_panel_before_outputs() {
    let root = temp_root("i8bin-ensemble-card-a37-gate-refuses");
    fs::create_dir_all(&root).unwrap();
    let rows = root.join("rows.jsonl");
    write_rows(&rows, 120);
    let labels_cf_root = write_labels_db(&root, &rows, 1);
    let plan = write_plan(&root, 10, 120);
    let mut request = request_for(&root, plan, PathBuf::new(), None, 80, Some(60));
    request.labels_cf_root = Some(labels_cf_root);
    request.labels_key = "unit_labels".to_string();
    request.mode = A37CardMode::Gate;

    let report = evaluate(&request).unwrap();
    let error = enforce_a37_mode(&request, &report).unwrap_err();

    assert_eq!(report.diversity.status, "diagnostic_only");
    assert!(error.starts_with("CALYX_FSV_ASSAY_A37_DIVERSITY_GATE_REFUSED"));
    assert!(!request.metrics_dir.exists());
    assert!(!request.cf_root.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gate_mode_rejects_file_label_authority_at_parse() {
    let args = [
        "--plan",
        "plan.json",
        "--rows-jsonl",
        "rows.jsonl",
        "--cf-root",
        "assay-cf",
    ]
    .iter()
    .map(|value| value.to_string())
    .collect::<Vec<_>>();

    let error = I8binEnsembleRequest::parse(&args).unwrap_err();

    assert!(error.contains("gate mode requires --labels-cf-root"));
}

#[test]
fn sub_ten_plan_fails_closed_before_card() {
    let root = temp_root("i8bin-ensemble-card-small");
    fs::create_dir_all(&root).unwrap();
    let rows = root.join("rows.jsonl");
    write_rows(&rows, 80);
    let plan = write_plan(&root, 9, 80);
    let request = request_for(&root, plan, rows, None, 60, Some(60));

    let error = evaluate(&request).unwrap_err();

    assert!(error.starts_with(calyx_assay::CALYX_ASSAY_PANEL_TOO_SMALL));
    assert!(error.contains("at least 10"));
    assert!(!root.join("metrics").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn vector_row_mismatch_fails_closed() {
    let root = temp_root("i8bin-ensemble-card-row-mismatch");
    fs::create_dir_all(&root).unwrap();
    let rows = root.join("rows.jsonl");
    write_rows(&rows, 80);
    let plan = write_plan(&root, 10, 79);
    let request = request_for(&root, plan, rows, None, 60, Some(60));

    let error = evaluate(&request).unwrap_err();

    assert!(error.starts_with("CALYX_FSV_ASSAY_I8BIN_CARD_VECTOR_MISMATCH"));
    assert!(error.contains("rows 79 != labels 80"));
    assert!(!root.join("metrics").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn existing_metrics_dir_fails_closed_before_overwrite() {
    let root = temp_root("i8bin-ensemble-card-output-exists");
    fs::create_dir_all(&root).unwrap();
    let rows = root.join("rows.jsonl");
    write_rows(&rows, 80);
    let plan = write_plan(&root, 10, 80);
    let request = request_for(&root, plan, rows, None, 60, Some(60));
    let report = evaluate(&request).unwrap();
    fs::create_dir_all(&request.metrics_dir).unwrap();
    fs::write(request.metrics_dir.join("sentinel.txt"), "preserve").unwrap();

    let error = write_outputs(&request, &report).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_ASSAY_I8BIN_CARD_OUTPUT_EXISTS");
    assert_eq!(
        fs::read_to_string(request.metrics_dir.join("sentinel.txt")).unwrap(),
        "preserve"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn existing_cf_root_fails_closed_before_mixing_rows() {
    let root = temp_root("i8bin-ensemble-card-cf-exists");
    fs::create_dir_all(&root).unwrap();
    let rows = root.join("rows.jsonl");
    write_rows(&rows, 80);
    let plan = write_plan(&root, 10, 80);
    let mut request = request_for(&root, plan, rows, None, 60, Some(60));
    request.cf_root = root.join("existing_assay_cf");
    let report = evaluate(&request).unwrap();
    fs::create_dir_all(&request.cf_root).unwrap();
    fs::write(request.cf_root.join("sentinel.txt"), "preserve-cf").unwrap();

    let error = write_outputs(&request, &report).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_ASSAY_I8BIN_CARD_OUTPUT_EXISTS");
    assert_eq!(
        fs::read_to_string(request.cf_root.join("sentinel.txt")).unwrap(),
        "preserve-cf"
    );
    assert!(!request.metrics_dir.exists());
    let _ = fs::remove_dir_all(root);
}

fn request_for(
    root: &Path,
    plan: PathBuf,
    rows: PathBuf,
    stream_report: Option<PathBuf>,
    sample_rows: usize,
    signature_rows: Option<usize>,
) -> I8binEnsembleRequest {
    let metrics = root.join("metrics");
    I8binEnsembleRequest {
        plan: Some(plan),
        plan_cf_root: None,
        plan_key: crate::partitioned_bench::rrf_plan::DEFAULT_ASSOCIATION_KEY.to_string(),
        rows_jsonl: rows,
        labels_cf_root: None,
        labels_key: super::label_store::DEFAULT_ASSOCIATION_KEY.to_string(),
        stream_report,
        metrics_dir: metrics.clone(),
        cf_root: metrics.join("assay_cf"),
        target_class: 1,
        domain: "i8bin_ensemble_test".to_string(),
        sample_rows,
        signature_rows,
        min_lenses: 10,
        min_marginal_bits: 0.05,
        max_redundancy: 0.6,
        nmi_bins: 8,
        mode: A37CardMode::Diagnostic,
        emit_artifacts: true,
    }
}

fn write_labels_db(root: &Path, rows: &Path, target_class: usize) -> PathBuf {
    let imported = super::label_store::load_rows_jsonl(
        rows,
        target_class,
        &super::label_store::AnchorSpec::Label,
        None,
    )
    .unwrap();
    let labels_cf_root = root.join("labels-cf");
    super::label_store::write(
        &labels_cf_root,
        "unit_labels",
        "unit",
        target_class,
        &imported.source_sha256,
        &imported.label_counts,
        &imported.labels,
        32,
    )
    .unwrap();
    labels_cf_root
}

fn write_rows(path: &Path, rows: usize) {
    let mut out = String::new();
    for row in 0..rows {
        out.push_str(&format!(
            "{{\"id\":\"row-{row}\",\"label\":{},\"event_time\":\"2024-01-01T00:00:00Z\"}}\n",
            row % 2
        ));
    }
    fs::write(path, out).unwrap();
}

fn write_plan(root: &Path, lens_count: usize, rows: usize) -> PathBuf {
    let vector_dir = root.join("i8bin");
    fs::create_dir_all(&vector_dir).unwrap();
    let mut slots = Vec::new();
    for slot in 0..lens_count {
        let corpus = vector_dir.join(format!("slot_{slot:02}_lens_{slot}_corpus.i8bin"));
        let queries = vector_dir.join(format!("slot_{slot:02}_lens_{slot}_queries.i8bin"));
        write_i8bin(&corpus, rows, slot);
        write_i8bin(&queries, 2, slot);
        let lens_id = format!("{:032x}", slot + 1);
        let weights_sha256 = format!("{:064x}", slot + 1);
        let vault = root.join(format!("vault-{slot}"));
        slots.push(format!(
            "{{\"slot\":{slot},\"name\":\"semantic-fastembed-lens-{slot}\",\"lens_id\":\"{}\",\"weights_sha256\":\"{}\",\"bits_about\":0.5,\"corpus\":\"{}\",\"queries\":\"{}\",\"vault\":\"{}\"}}",
            lens_id,
            weights_sha256,
            json_path(&corpus),
            json_path(&queries),
            json_path(&vault)
        ));
    }
    let plan = root.join("partitioned_rrf_plan.json");
    fs::write(&plan, format!("{{\"slots\":[{}]}}", slots.join(","))).unwrap();
    plan
}

fn write_stream_report(root: &Path, lens_count: usize, rows: usize) -> PathBuf {
    let mut roster = Vec::new();
    for slot in 0..lens_count {
        let manifest = root.join(format!("manifest-{slot}.json"));
        fs::write(&manifest, "{\"runtime\":\"onnx-int8\"}").unwrap();
        roster.push(format!(
            "{{\"slot\":{slot},\"manifest\":\"{}\",\"dim\":{DIM},\"max_batch\":8,\"elapsed_ms\":1000,\"corpus_rows_written\":{rows},\"query_rows_written\":2}}",
            json_path(&manifest)
        ));
    }
    let path = root.join("stream_fbin_report.json");
    fs::write(&path, format!("{{\"lens_roster\":[{}]}}", roster.join(","))).unwrap();
    path
}

fn write_i8bin(path: &Path, rows: usize, slot: usize) {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(rows as u32).to_le_bytes());
    bytes.extend_from_slice(&(DIM as u32).to_le_bytes());
    for row in 0..rows {
        let signal = if row % 2 == 1 { 28_i8 } else { -28_i8 };
        let jitter = ((row + slot) % 5) as i8 - 2;
        let row_bytes = [
            signal,
            signal.saturating_add(jitter),
            (slot as i8 + 1).saturating_mul(2),
            jitter,
        ];
        bytes.extend(row_bytes.into_iter().map(|value| value as u8));
    }
    fs::write(path, bytes).unwrap();
}

fn json_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "\\\\")
}

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{name}-{}", std::process::id()))
}
