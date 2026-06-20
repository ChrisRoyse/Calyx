use std::fs;

use calyx_sextant::index::I8BinVectors;
use serde_json::Value;

use super::args::StreamMode;
use super::format::VectorFormat;
use super::write;

#[path = "tests/support.rs"]
mod support;

use support::{
    Fixture, staging_dir, write_bits_with_gate, write_bits_with_panel_names, write_legacy_bits,
};

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
    assert_eq!(progress["current_lens_elapsed_ms"], Value::Null);
    assert_eq!(progress["streaming_fbin_source"], true);
    assert_eq!(progress["temporal_counts_toward_a35"], false);
    assert_eq!(
        progress["temporal_lane_role"],
        "time_manipulation_walk_forward_backward_as_of_sidecar"
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
    assert_eq!(report["pre_encode_gate"]["sufficient"], true);
    assert_eq!(
        report["pre_encode_gate"]["estimate_bound"]
            .as_str()
            .unwrap(),
        "lower_bound"
    );
    assert_eq!(
        report["pre_encode_gate"]["power_calibration_status"]
            .as_str()
            .unwrap(),
        "passed"
    );
    assert_eq!(
        report["pre_encode_gate"]["streamed_lenses"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
    assert_eq!(report["temporal_counts_toward_a35"], false);
    assert_eq!(
        report["temporal_lane_role"],
        "time_manipulation_walk_forward_backward_as_of_sidecar"
    );
    let first_lens = &report["lens_roster"][0];
    assert_eq!(first_lens["signal_kind"], "learned_encoder");
    assert_eq!(first_lens["effective_batch_size"], 7);
    assert!(first_lens["elapsed_ms"].as_u64().is_some());
    let ms_per_input = first_lens["ms_per_input"].as_f64().unwrap();
    assert!(ms_per_input.is_finite());
    assert!(ms_per_input >= 0.0);

    let plan: Value =
        serde_json::from_slice(&fs::read(fixture.out.join("partitioned_rrf_plan.json")).unwrap())
            .unwrap();
    assert_eq!(plan["temporal_counts_toward_a35"], false);
    assert_eq!(plan["slots"][0]["signal_kind"], "learned_encoder");
    assert_eq!(
        plan["temporal_lane_role"],
        "time_manipulation_walk_forward_backward_as_of_sidecar"
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
    assert_eq!(plan["slots"][0]["name"], "lens-0");
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
fn stream_fbin_accepts_deterministic_content_feature_manifest() {
    let fixture = Fixture::new_algorithmic("stream-fbin-algorithmic-content", 10, 10, 50);
    let args = fixture.args(8);

    let evidence = write::run(&args).unwrap();
    let plan: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.out.join("partitioned_rrf_plan.json")).unwrap())
            .unwrap();

    assert_eq!(evidence.lens_roster.len(), 10);
    assert_eq!(
        plan["slots"][0]["signal_kind"],
        "deterministic_content_feature"
    );
    assert!(fixture.out.join("fbin/slot_00_lens-0_corpus.fbin").exists());
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn stream_fbin_rejects_temporal_sidecar_as_content_feature() {
    let fixture = Fixture::new_algorithmic("stream-fbin-temporal-sidecar", 10, 10, 50);
    let temporal_manifest = fixture.root.join("algorithmic/lens-0.json");
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(&temporal_manifest).unwrap()).unwrap();
    manifest["name"] = serde_json::json!("temporal-as-of-time-manipulation-sidecar");
    fs::write(
        &temporal_manifest,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let args = fixture.args(8);

    let error = write::run(&args).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_A35_TEMPORAL_SIDECAR_NOT_CONTENT");
    assert!(!fixture.out.exists());
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn stream_fbin_rejects_leaked_anchor_bits_before_rows() {
    let fixture = Fixture::new("stream-fbin-leaked-anchor", 10, 10, 50);
    mark_bits_as_leaked(&fixture.bits);
    let mut args = fixture.args(8);
    args.rows_jsonl = fixture.root.join("missing-rows.jsonl");

    let error = write::run(&args).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_ASSAY_TRIVIAL_ANCHOR");
    assert!(!fixture.out.exists());
    assert!(!staging_dir(&fixture).exists());
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn stream_fbin_diagnostic_mode_records_insufficient_panel() {
    let fixture = Fixture::new("stream-fbin-diagnostic-insufficient", 10, 10, 50);
    write_bits_with_gate(&fixture.bits, 10, 10, 0.42, "passed", 1.0);
    let mut args = fixture.args(8);
    args.mode = StreamMode::Diagnostic;

    write::run(&args).unwrap();

    let report: Value =
        serde_json::from_slice(&fs::read(fixture.out.join("stream_fbin_report.json")).unwrap())
            .unwrap();
    assert_eq!(report["pre_encode_gate"]["mode"], "diagnostic");
    assert_eq!(report["pre_encode_gate"]["diagnostic_only"], true);
    assert_eq!(report["pre_encode_gate"]["sufficient"], false);
    let deficit = report["pre_encode_gate"]["deficit_bits"].as_f64().unwrap();
    assert!((deficit - 0.58).abs() < 0.000001);
    assert_eq!(report["lens_roster"].as_array().unwrap().len(), 10);
    assert!(fixture.out.join("partitioned_rrf_plan.json").exists());
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn stream_fbin_diagnostic_mode_records_leaked_anchor() {
    let fixture = Fixture::new("stream-fbin-diagnostic-anchor-leak", 10, 10, 50);
    mark_bits_as_leaked(&fixture.bits);
    let mut args = fixture.args(8);
    args.mode = StreamMode::Diagnostic;

    write::run(&args).unwrap();

    let report: Value =
        serde_json::from_slice(&fs::read(fixture.out.join("stream_fbin_report.json")).unwrap())
            .unwrap();
    assert_eq!(report["pre_encode_gate"]["mode"], "diagnostic");
    assert_eq!(report["pre_encode_gate"]["diagnostic_only"], true);
    assert_eq!(report["pre_encode_gate"]["grounded_gate_eligible"], false);
    assert_eq!(
        report["pre_encode_gate"]["anchor_audit"]["anchor_leaks_into_input"],
        true
    );
    assert_eq!(report["lens_roster"].as_array().unwrap().len(), 10);
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

#[test]
fn stream_fbin_pre_gate_refuses_before_row_scan() {
    let fixture = Fixture::new("stream-fbin-pre-gate-refused", 10, 10, 8);
    write_bits_with_gate(&fixture.bits, 10, 10, 0.42, "passed", 1.0);
    let mut args = fixture.args(2);
    args.rows_jsonl = fixture.root.join("missing-rows.jsonl");

    let error = write::run(&args).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_ASSAY_STREAM_FBIN_PRE_GATE_REFUSED");
    assert!(!fixture.out.exists());
    assert!(!staging_dir(&fixture).exists());
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn stream_fbin_pre_gate_fails_closed_on_legacy_bits_report() {
    let fixture = Fixture::new("stream-fbin-pre-gate-missing", 10, 10, 50);
    write_legacy_bits(&fixture.bits, 10, 10);
    let args = fixture.args(8);

    let error = write::run(&args).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_ASSAY_STREAM_FBIN_PRE_GATE_MISSING");
    assert!(!fixture.out.exists());
    assert!(!staging_dir(&fixture).exists());
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn stream_fbin_pre_gate_rejects_unpowered_panel() {
    let fixture = Fixture::new("stream-fbin-pre-gate-unpowered", 10, 10, 50);
    write_bits_with_gate(&fixture.bits, 10, 10, 1.25, "underpowered", 0.25);
    let args = fixture.args(8);

    let error = write::run(&args).unwrap_err();

    assert_eq!(
        error.code(),
        "CALYX_FSV_ASSAY_STREAM_FBIN_PRE_GATE_UNPOWERED"
    );
    assert!(!fixture.out.exists());
    assert!(!staging_dir(&fixture).exists());
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn stream_fbin_pre_gate_rejects_mismatched_panel() {
    let fixture = Fixture::new("stream-fbin-pre-gate-mismatch", 10, 10, 50);
    write_bits_with_panel_names(
        &fixture.bits,
        10,
        10,
        (0..9)
            .map(|idx| format!("lens-{idx}"))
            .chain(std::iter::once("lens-other".to_string()))
            .collect(),
        1.25,
        "passed",
        1.0,
    );
    let args = fixture.args(8);

    let error = write::run(&args).unwrap_err();

    assert_eq!(
        error.code(),
        "CALYX_FSV_ASSAY_STREAM_FBIN_PRE_GATE_PANEL_MISMATCH"
    );
    assert!(!fixture.out.exists());
    assert!(!staging_dir(&fixture).exists());
    let _ = fs::remove_dir_all(fixture.root);
}

fn mark_bits_as_leaked(path: &std::path::Path) {
    let mut report: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    report["anchor_audit"] = serde_json::json!({
        "anchor_leaks_into_input": true,
        "trivial_anchor": true,
        "grounded_gate_eligible": false,
        "label_recoverable_from_input": true,
        "reason": "unit fixture label is present in text"
    });
    fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
}
