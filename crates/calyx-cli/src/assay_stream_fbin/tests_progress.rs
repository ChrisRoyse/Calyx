use std::fs;

use serde_json::Value;

use super::tests_support::Fixture;
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
    assert_eq!(
        report["plan_cf_root"].as_str().unwrap(),
        fixture
            .out
            .join("partitioned_rrf_plan_cf")
            .display()
            .to_string()
    );
    assert_eq!(
        report["plan_association_key"].as_str().unwrap(),
        crate::partitioned_bench::rrf_plan::DEFAULT_ASSOCIATION_KEY
    );
    assert!(
        report["plan_db_readback"]["readback_matches"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(
        report["timeline_cf_root"].as_str().unwrap(),
        fixture
            .out
            .join("partitioned_rrf_timeline_cf")
            .display()
            .to_string()
    );
    assert_eq!(
        report["timeline_association_key"].as_str().unwrap(),
        crate::partitioned_bench::timeline_store::DEFAULT_ASSOCIATION_KEY
    );
    assert!(
        report["timeline_db_readback"]["readback_matches"]
            .as_bool()
            .unwrap()
    );
    let (db_plan, db_readback) = crate::partitioned_bench::rrf_plan::read(
        &fixture.out.join("partitioned_rrf_plan_cf"),
        crate::partitioned_bench::rrf_plan::DEFAULT_ASSOCIATION_KEY,
    )
    .unwrap();
    assert!(db_readback.readback_matches);
    assert_eq!(db_plan.plan.slots.len(), 10);
    assert_eq!(db_plan.plan.slots[0].name.as_deref(), Some("lens-0"));
    assert!(db_plan.plan.slots[0].corpus.is_file());
    let db_timeline = crate::partitioned_bench::timeline_store::read(
        &fixture.out.join("partitioned_rrf_timeline_cf"),
        crate::partitioned_bench::timeline_store::DEFAULT_ASSOCIATION_KEY,
    )
    .unwrap();
    assert!(db_timeline.db_readback.readback_matches);
    assert_eq!(db_timeline.manifest.row_count, 50);
    assert_eq!(db_timeline.rows[0].id, "row-0");
    assert_eq!(report["pre_encode_gate"]["sufficient"], true);
    assert_eq!(
        report["pre_encode_gate"]["estimate_bound"]
            .as_str()
            .unwrap(),
        "a37_multi_anchor_best_target"
    );
    assert_eq!(
        report["pre_encode_gate"]["power_calibration_status"]
            .as_str()
            .unwrap(),
        "db_readback_passed"
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
