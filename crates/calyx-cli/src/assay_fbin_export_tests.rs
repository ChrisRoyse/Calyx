use std::fs;

use serde_json::Value;

use super::args::Args;
use super::export_fbin;

#[path = "assay_fbin_export_tests/support.rs"]
mod support;

use support::{Fixture, assert_fbin_header, mark_bits_as_leaked, write_algorithmic_manifests};

#[test]
fn export_fbin_writes_headers_plan_and_readback_report() {
    let fixture = Fixture::new("export-fbin-happy", 10, 6);
    let args = fixture.args(2);

    let evidence = export_fbin(&args).unwrap();

    assert_eq!(evidence.rows, 6);
    assert_eq!(evidence.query_count, 2);
    assert_eq!(evidence.lens_roster.len(), 10);
    assert_fbin_header(&fixture.out.join("fbin/slot_00_lens-0_corpus.fbin"), 3, 6);
    assert_fbin_header(&fixture.out.join("fbin/slot_00_lens-0_queries.fbin"), 3, 2);
    let plan: Value =
        serde_json::from_slice(&fs::read(fixture.out.join("partitioned_rrf_plan.json")).unwrap())
            .unwrap();
    assert_eq!(plan["slots"].as_array().unwrap().len(), 10);
    assert_eq!(
        plan["timeline"].as_str().unwrap(),
        fixture.out.join("timeline.jsonl").display().to_string()
    );
    assert_eq!(plan["temporal_counts_toward_a35"], false);
    assert_eq!(plan["slots"][0]["name"], "lens-0");
    assert_eq!(plan["slots"][0]["signal_kind"], "learned_encoder");
    let bits = plan["slots"][0]["bits_about"].as_f64().unwrap();
    assert!((bits - 0.2).abs() < 0.00001);
    let timeline = fs::read_to_string(fixture.out.join("timeline.jsonl")).unwrap();
    let timeline_rows = timeline
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(timeline_rows.len(), 6);
    assert_eq!(
        timeline_rows[0]["source_event_time_secs"],
        1_704_153_600_i64
    );
    assert_eq!(timeline_rows[0]["query_row"], true);
    assert_eq!(timeline_rows[2]["query_row"], false);
    assert!(fixture.out.join("export_report.json").is_file());
    let report: Value =
        serde_json::from_slice(&fs::read(fixture.out.join("export_report.json")).unwrap()).unwrap();
    assert_eq!(report["out_dir"], fixture.out.display().to_string());
    assert_eq!(
        report["timeline_path"].as_str().unwrap(),
        fixture.out.join("timeline.jsonl").display().to_string()
    );
    assert_eq!(report["temporal"]["active_rows"], 6);
    assert_eq!(report["lens_roster"][0]["signal_kind"], "learned_encoder");
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn export_fbin_preserves_corpus_build_lens_order() {
    let names = vec![
        "zulu-lens",
        "alpha-lens",
        "mercury-lens",
        "bravo-lens",
        "theta-lens",
        "charlie-lens",
        "omega-lens",
        "delta-lens",
        "kappa-lens",
        "echo-lens",
    ];
    let fixture = Fixture::with_names("export-fbin-corpus-order", &names, 10, 4);
    let args = fixture.args(2);

    let evidence = export_fbin(&args).unwrap();

    let evidence_names = evidence
        .lens_roster
        .iter()
        .map(|lens| lens.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(evidence_names, names);
    let plan: Value =
        serde_json::from_slice(&fs::read(fixture.out.join("partitioned_rrf_plan.json")).unwrap())
            .unwrap();
    let plan_names = plan["slots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|slot| slot["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(plan_names, names);
    assert!(
        fixture
            .out
            .join("fbin/slot_00_zulu-lens_corpus.fbin")
            .is_file()
    );
    assert!(
        fixture
            .out
            .join("fbin/slot_01_alpha-lens_corpus.fbin")
            .is_file()
    );
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn export_fbin_rejects_query_count_above_rows() {
    let fixture = Fixture::new("export-fbin-query-too-large", 10, 3);
    let args = fixture.args(4);

    let error = export_fbin(&args).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_ASSAY_FBIN_EXPORT_QUERY_TOO_LARGE");
    assert!(!fixture.out.exists());
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn export_fbin_rejects_existing_output_before_scanning_vectors() {
    let fixture = Fixture::new("export-fbin-output-exists-early", 10, 3);
    let args = fixture.args(2);
    fs::create_dir_all(&fixture.out).unwrap();
    fs::write(fixture.corpus.join("vectors.jsonl"), "{not json}\n").unwrap();

    let error = export_fbin(&args).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_ASSAY_FBIN_EXPORT_OUTPUT_EXISTS");
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn export_fbin_rejects_panel_below_a35_floor() {
    let fixture = Fixture::new("export-fbin-too-small", 3, 6);
    let args = fixture.args(2);

    let error = export_fbin(&args).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_ASSAY_FBIN_EXPORT_PANEL_TOO_SMALL");
    assert!(!fixture.out.exists());
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn export_fbin_accepts_deterministic_content_feature_manifest() {
    let fixture = Fixture::new_algorithmic("export-fbin-algorithmic", 10, 6);
    let args = fixture.args(2);

    let evidence = export_fbin(&args).unwrap();
    let plan: Value =
        serde_json::from_slice(&fs::read(fixture.out.join("partitioned_rrf_plan.json")).unwrap())
            .unwrap();

    assert_eq!(evidence.lens_roster.len(), 10);
    assert_eq!(
        plan["slots"][0]["signal_kind"],
        "deterministic_content_feature"
    );
    assert_fbin_header(&fixture.out.join("fbin/slot_00_lens-0_corpus.fbin"), 3, 6);
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn export_fbin_rejects_temporal_sidecar_as_content_feature() {
    let mut names = (0..10).map(|idx| format!("lens-{idx}")).collect::<Vec<_>>();
    names[0] = "temporal-as-of-time-manipulation-sidecar".to_string();
    let fixture = Fixture::with_names_and_writer(
        "export-fbin-temporal-sidecar",
        &names,
        10,
        6,
        write_algorithmic_manifests,
    );
    let args = fixture.args(2);

    let error = export_fbin(&args).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_A35_TEMPORAL_SIDECAR_NOT_CONTENT");
    assert!(!fixture.out.exists());
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn export_fbin_rejects_leaked_anchor_bits_before_writing() {
    let fixture = Fixture::new("export-fbin-leaked-anchor", 10, 6);
    mark_bits_as_leaked(&fixture.bits);
    let args = fixture.args(2);

    let error = export_fbin(&args).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_ASSAY_TRIVIAL_ANCHOR");
    assert!(!fixture.out.exists());
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn export_fbin_rejects_inconsistent_vector_dimensions() {
    let fixture = Fixture::new("export-fbin-bad-dim", 10, 6);
    let mut lines = fs::read_to_string(fixture.corpus.join("vectors.jsonl")).unwrap();
    lines.push_str(
        &serde_json::json!({
            "id": "bad-row",
            "lenses": {
                "lens-0": [1.0, 2.0],
                "lens-1": [1.0, 2.0, 3.0],
                "lens-2": [1.0, 2.0, 3.0],
                "lens-3": [1.0, 2.0, 3.0],
                "lens-4": [1.0, 2.0, 3.0],
                "lens-5": [1.0, 2.0, 3.0],
                "lens-6": [1.0, 2.0, 3.0],
                "lens-7": [1.0, 2.0, 3.0],
                "lens-8": [1.0, 2.0, 3.0],
                "lens-9": [1.0, 2.0, 3.0]
            }
        })
        .to_string(),
    );
    lines.push('\n');
    fs::write(fixture.corpus.join("vectors.jsonl"), lines).unwrap();
    let args = fixture.args(2);

    let error = export_fbin(&args).unwrap_err();

    assert_eq!(
        error.code(),
        "CALYX_FSV_ASSAY_FBIN_EXPORT_LENS_SET_MISMATCH"
    );
    assert!(!fixture.out.exists());
    let _ = fs::remove_dir_all(fixture.root);
}
