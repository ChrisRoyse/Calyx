use std::fs;

use calyx_sextant::index::I8BinVectors;
use serde_json::Value;

use super::args::StreamMode;
use super::format::VectorFormat;
use super::tests_support::{Fixture, staging_dir, write_bits_with_gate, write_legacy_bits};
use super::write;

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
fn lens_parallelism_produces_identical_per_lens_outputs() {
    let sequential = Fixture::new_algorithmic("stream-fbin-k1-baseline", 10, 10, 50);
    let sequential_args = sequential.args(8);
    write::run(&sequential_args).unwrap();

    let parallel = Fixture::new_algorithmic("stream-fbin-k3-parallel", 10, 10, 50);
    let mut parallel_args = parallel.args(8);
    parallel_args.lens_parallelism = 3;
    parallel_args.worker_gpu_mem_limit_mib = Some(1024);
    let evidence = write::run(&parallel_args).unwrap();
    assert_eq!(evidence.lens_roster.len(), 10);

    let sequential_dir = sequential.out.join("fbin");
    let parallel_dir = parallel.out.join("fbin");
    let mut names: Vec<_> = fs::read_dir(&sequential_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names.sort();
    assert_eq!(names.len(), 20, "10 lenses x corpus+queries");
    for name in names {
        let baseline = fs::read(sequential_dir.join(&name)).unwrap();
        let candidate = fs::read(parallel_dir.join(&name)).unwrap();
        assert_eq!(
            baseline, candidate,
            "lens output {name:?} differs between K=1 and K=3"
        );
    }
    let plan: serde_json::Value =
        serde_json::from_slice(&fs::read(parallel.out.join("partitioned_rrf_plan.json")).unwrap())
            .unwrap();
    for (idx, slot) in plan["slots"].as_array().unwrap().iter().enumerate() {
        assert_eq!(slot["slot"], idx as u64, "roster must stay slot-ordered");
    }
    let _ = fs::remove_dir_all(sequential.root);
    let _ = fs::remove_dir_all(parallel.root);
}

#[test]
fn lens_parallelism_above_one_requires_worker_vram_budget() {
    if std::env::var("CALYX_ONNX_GPU_MEM_LIMIT_MIB").is_ok() {
        // An outer harness budget makes K>1 legitimately runnable here.
        return;
    }
    let fixture = Fixture::new_algorithmic("stream-fbin-k-unbudgeted", 10, 10, 50);
    let mut args = fixture.args(8);
    args.lens_parallelism = 2;

    let error = write::run(&args).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_ASSAY_STREAM_FBIN_PARALLEL_VRAM");
    assert!(!fixture.out.exists());
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
fn stream_fbin_rejects_json_gate_before_rows() {
    let fixture = Fixture::new("stream-fbin-json-gate", 10, 10, 50);
    mark_bits_as_leaked(&fixture.bits);
    let mut args = fixture.json_args(8, StreamMode::Gate);
    args.rows_jsonl = fixture.root.join("missing-rows.jsonl");

    let error = write::run(&args).unwrap_err();

    assert_eq!(
        error.code(),
        "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_TEMPLATE_DB_REQUIRED"
    );
    assert!(!fixture.out.exists());
    assert!(!staging_dir(&fixture).exists());
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn stream_fbin_diagnostic_mode_records_insufficient_panel() {
    let fixture = Fixture::new("stream-fbin-diagnostic-insufficient", 10, 10, 50);
    write_bits_with_gate(&fixture.bits, 10, 10, 0.42, "passed", 1.0);
    let args = fixture.json_args(8, StreamMode::Diagnostic);

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
    let args = fixture.json_args(8, StreamMode::Diagnostic);

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
    fixture.rewrite_a37(9, None, 0.2);
    let mut args = fixture.args(2);
    args.rows_jsonl = fixture.root.join("missing-rows.jsonl");

    let error = write::run(&args).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_REJECTED");
    assert!(!fixture.out.exists());
    assert!(!staging_dir(&fixture).exists());
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn stream_fbin_pre_gate_fails_closed_on_missing_anchor_audit() {
    // #1140: a bits report with no anchor_audit (e.g. produced from rows that
    // predate the audit machinery) must refuse gate mode, not default to
    // eligible.
    let fixture = Fixture::new("stream-fbin-pre-gate-no-audit", 10, 10, 50);
    strip_bits_anchor_audit(&fixture.bits);
    let mut args = fixture.args(8);
    args.a37_admission_cf_root = None;
    args.bits_report = Some(fixture.bits.clone());

    let error = write::run(&args).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_ASSAY_STREAM_FBIN_A37_DB_REQUIRED");
    assert!(!fixture.out.exists());
    assert!(!staging_dir(&fixture).exists());
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn stream_fbin_pre_gate_accepts_power_adjusted_sufficiency() {
    // #1140: the estimator's planted-perfect ceiling (power recovery 0.9)
    // bounds what any panel can measure; a basis of 0.95 >= 1.0 * 0.9 is
    // sufficient even though it is below the raw anchor entropy.
    let fixture = Fixture::new("stream-fbin-pre-gate-power-adjusted", 10, 10, 50);
    let args = fixture.args(8);

    write::run(&args).unwrap();

    let report: Value =
        serde_json::from_slice(&fs::read(fixture.out.join("stream_fbin_report.json")).unwrap())
            .unwrap();
    assert_eq!(report["pre_encode_gate"]["sufficient"], true);
    assert_eq!(report["pre_encode_gate"]["diagnostic_only"], false);
    assert_eq!(
        report["pre_encode_gate"]["power_calibration_status"],
        "db_readback_passed"
    );
    assert_eq!(report["pre_encode_gate"]["deficit_bits"], 0.0);
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn stream_fbin_pre_gate_fails_closed_on_legacy_bits_report() {
    let fixture = Fixture::new("stream-fbin-pre-gate-missing", 10, 10, 50);
    write_legacy_bits(&fixture.bits, 10, 10);
    let mut args = fixture.args(8);
    args.a37_admission_cf_root = None;
    args.bits_report = Some(fixture.bits.clone());

    let error = write::run(&args).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_ASSAY_STREAM_FBIN_A37_DB_REQUIRED");
    assert!(!fixture.out.exists());
    assert!(!staging_dir(&fixture).exists());
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn stream_fbin_pre_gate_rejects_unpowered_panel() {
    let fixture = Fixture::new("stream-fbin-pre-gate-unpowered", 10, 10, 50);
    write_bits_with_gate(&fixture.bits, 10, 10, 1.25, "underpowered", 0.25);
    let mut args = fixture.args(8);
    args.a37_admission_cf_root = None;
    args.bits_report = Some(fixture.bits.clone());

    let error = write::run(&args).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_ASSAY_STREAM_FBIN_A37_DB_REQUIRED");
    assert!(!fixture.out.exists());
    assert!(!staging_dir(&fixture).exists());
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn stream_fbin_pre_gate_rejects_mismatched_panel() {
    let fixture = Fixture::new("stream-fbin-pre-gate-mismatch", 10, 10, 50);
    fixture.rewrite_a37(
        10,
        Some(
            (0..9)
                .map(|idx| format!("lens-{idx}"))
                .chain(std::iter::once("lens-other".to_string()))
                .collect(),
        ),
        0.2,
    );
    let args = fixture.args(8);

    let error = write::run(&args).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_ASSAY_STREAM_FBIN_BITS_MISSING");
    assert!(!fixture.out.exists());
    assert!(!staging_dir(&fixture).exists());
    let _ = fs::remove_dir_all(fixture.root);
}

fn strip_bits_anchor_audit(path: &std::path::Path) {
    let mut report: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    report.as_object_mut().unwrap().remove("anchor_audit");
    fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
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
