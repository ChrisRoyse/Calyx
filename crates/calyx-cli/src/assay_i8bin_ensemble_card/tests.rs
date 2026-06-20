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
fn default_gate_mode_refuses_homogeneous_panel_before_outputs() {
    let root = temp_root("i8bin-ensemble-card-a37-gate-refuses");
    fs::create_dir_all(&root).unwrap();
    let rows = root.join("rows.jsonl");
    write_rows(&rows, 120);
    let plan = write_plan(&root, 10, 120);
    let mut request = request_for(&root, plan, rows, None, 80, Some(60));
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

    assert!(error.starts_with("CALYX_FSV_ASSAY_I8BIN_CARD_OUTPUT_EXISTS"));
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

    assert!(error.starts_with("CALYX_FSV_ASSAY_I8BIN_CARD_OUTPUT_EXISTS"));
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
        plan,
        rows_jsonl: rows,
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
    }
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
