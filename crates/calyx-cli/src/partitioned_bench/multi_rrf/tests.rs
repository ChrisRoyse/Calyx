use super::args::Args;
use super::*;
use std::path::PathBuf;

#[test]
fn args_parse_plan_truth_depth_and_tuner_vault() {
    let args = Args::parse(&strings([
        "--plan",
        "plan.json",
        "--timeline-cf-root",
        "timeline-db",
        "--timeline-key",
        "issue791_timeline",
        "--n",
        "12",
        "--k",
        "4",
        "--n-probe",
        "3",
        "--region-beam",
        "32",
        "--ground-truth",
        "5",
        "--truth-depth",
        "40",
        "--fused-ground-truth-file",
        "truth.i32bin",
        "--fused-ground-truth-manifest",
        "truth.manifest.json",
        "--ensemble-card",
        "ensemble_card.json",
        "--a37-admission-card",
        "multi_anchor_ensemble_card.json",
        "--recall-floor",
        "0.8",
        "--anneal-vault",
        "anneal-out",
        "--tuner-slo-us",
        "100",
        "--report-cf-root",
        "report-db",
        "--report-key",
        "issue791_report",
    ]))
    .unwrap();

    assert_eq!(args.plan, Some(PathBuf::from("plan.json")));
    assert_eq!(args.plan_cf_root, None);
    assert_eq!(args.timeline_cf_root, Some(PathBuf::from("timeline-db")));
    assert_eq!(args.timeline_key, "issue791_timeline");
    assert_eq!(args.n, 12);
    assert_eq!(args.k, 4);
    assert_eq!(args.truth_depth, Some(40));
    assert_eq!(
        args.fused_ground_truth_file,
        Some(PathBuf::from("truth.i32bin"))
    );
    assert_eq!(
        args.fused_ground_truth_manifest,
        Some(PathBuf::from("truth.manifest.json"))
    );
    assert_eq!(args.slot_ground_truth_manifest, None);
    assert_eq!(
        args.ensemble_card,
        Some(PathBuf::from("ensemble_card.json"))
    );
    assert_eq!(
        args.a37_admission_card,
        Some(PathBuf::from("multi_anchor_ensemble_card.json"))
    );
    assert_eq!(args.recall_floor, Some(0.8));
    assert_eq!(args.out, None);
    assert_eq!(args.report_cf_root, Some(PathBuf::from("report-db")));
    assert_eq!(args.report_key, "issue791_report");
    assert!(!args.report_db_only);
    assert_eq!(args.anneal_vault, Some(PathBuf::from("anneal-out")));
    assert_eq!(args.tuner_slo_us, Some(100));
}

#[test]
fn args_parse_report_db_only() {
    let args = Args::parse(&strings([
        "--plan",
        "plan.json",
        "--report-cf-root",
        "report-db",
        "--report-db-only",
    ]))
    .unwrap();

    assert_eq!(args.plan, Some(PathBuf::from("plan.json")));
    assert_eq!(args.report_cf_root, Some(PathBuf::from("report-db")));
    assert!(args.report_db_only);
}

#[test]
fn args_parse_plan_cf_root() {
    let args = Args::parse(&strings([
        "--plan-cf-root",
        "plan-db",
        "--plan-key",
        "issue791_plan",
        "--report-cf-root",
        "report-db",
        "--report-db-only",
    ]))
    .unwrap();

    assert_eq!(args.plan, None);
    assert_eq!(args.plan_cf_root, Some(PathBuf::from("plan-db")));
    assert_eq!(args.plan_key, "issue791_plan");
}

#[test]
fn args_reject_plan_file_and_db_together() {
    let err = Args::parse(&strings([
        "--plan",
        "plan.json",
        "--plan-cf-root",
        "plan-db",
    ]))
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("exactly one"));
}

#[test]
fn args_reject_report_db_only_without_cf_root() {
    let err = Args::parse(&strings(["--plan", "plan.json", "--report-db-only"])).unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("--report-cf-root"));
}

#[test]
fn args_reject_report_db_only_with_out_file() {
    let err = Args::parse(&strings([
        "--plan",
        "plan.json",
        "--report-cf-root",
        "report-db",
        "--report-db-only",
        "--out",
        "report.json",
    ]))
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("mutually exclusive"));
}

#[test]
fn args_parse_slot_ground_truth_manifest() {
    let args = Args::parse(&strings([
        "--plan",
        "plan.json",
        "--ground-truth",
        "5",
        "--slot-ground-truth-manifest",
        "slot-truth.manifest.json",
    ]))
    .unwrap();

    assert_eq!(
        args.slot_ground_truth_manifest,
        Some(PathBuf::from("slot-truth.manifest.json"))
    );
}

#[test]
fn args_parse_slot_ground_truth_cf_root() {
    let args = Args::parse(&strings([
        "--plan",
        "plan.json",
        "--ground-truth",
        "5",
        "--slot-ground-truth-cf-root",
        "slot-truth-db",
        "--slot-ground-truth-key",
        "issue791_truth",
    ]))
    .unwrap();

    assert_eq!(
        args.slot_ground_truth_cf_root,
        Some(PathBuf::from("slot-truth-db"))
    );
    assert_eq!(args.slot_ground_truth_key, "issue791_truth");
}

#[test]
fn args_parse_fused_ground_truth_cf_root_and_write_key() {
    let args = Args::parse(&strings([
        "--plan",
        "plan.json",
        "--ground-truth",
        "5",
        "--slot-ground-truth-cf-root",
        "slot-truth-db",
        "--write-fused-ground-truth-cf-root",
        "fused-truth-db",
        "--write-fused-ground-truth-key",
        "issue791_fused",
    ]))
    .unwrap();

    assert_eq!(
        args.write_fused_ground_truth_cf_root,
        Some(PathBuf::from("fused-truth-db"))
    );
    assert_eq!(args.write_fused_ground_truth_key, "issue791_fused");
}

#[test]
fn args_parse_fused_ground_truth_db_source() {
    let args = Args::parse(&strings([
        "--plan",
        "plan.json",
        "--ground-truth",
        "5",
        "--fused-ground-truth-cf-root",
        "fused-truth-db",
        "--fused-ground-truth-key",
        "issue791_fused",
    ]))
    .unwrap();

    assert_eq!(
        args.fused_ground_truth_cf_root,
        Some(PathBuf::from("fused-truth-db"))
    );
    assert_eq!(args.fused_ground_truth_key, "issue791_fused");
}

#[test]
fn args_parse_a37_admission_cf_root() {
    let args = Args::parse(&strings([
        "--plan",
        "plan.json",
        "--a37-admission-cf-root",
        "admission-db",
        "--a37-admission-key",
        "issue791-gate",
    ]))
    .unwrap();

    assert_eq!(
        args.a37_admission_cf_root,
        Some(PathBuf::from("admission-db"))
    );
    assert_eq!(args.a37_admission_key, "issue791-gate");
}

#[test]
fn args_reject_a37_file_and_cf_root_together() {
    let err = Args::parse(&strings([
        "--plan",
        "plan.json",
        "--a37-admission-card",
        "card.json",
        "--a37-admission-cf-root",
        "admission-db",
    ]))
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("mutually exclusive"));
}

#[test]
fn args_reject_zero_tuner_slo() {
    let err = Args::parse(&strings(["--plan", "plan.json", "--tuner-slo-us", "0"])).unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("--tuner-slo-us must be > 0"));
}

#[test]
fn args_require_fused_truth_file_and_manifest_pair() {
    let err = Args::parse(&strings([
        "--plan",
        "plan.json",
        "--fused-ground-truth-file",
        "truth.i32bin",
    ]))
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(
        err.message()
            .contains("--fused-ground-truth-file requires --fused-ground-truth-manifest")
    );
}

#[test]
fn args_reject_consuming_and_writing_fused_truth_in_one_run() {
    let err = Args::parse(&strings([
        "--plan",
        "plan.json",
        "--fused-ground-truth-file",
        "truth.i32bin",
        "--fused-ground-truth-manifest",
        "truth.manifest.json",
        "--write-fused-ground-truth-file",
        "new.i32bin",
        "--write-fused-ground-truth-manifest",
        "new.manifest.json",
    ]))
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("mutually exclusive"));
}

#[test]
fn args_reject_fused_and_slot_truth_sources_together() {
    let err = Args::parse(&strings([
        "--plan",
        "plan.json",
        "--fused-ground-truth-file",
        "truth.i32bin",
        "--fused-ground-truth-manifest",
        "truth.manifest.json",
        "--slot-ground-truth-manifest",
        "slot-truth.manifest.json",
    ]))
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(
        err.message()
            .contains("precomputed fused file, fused DB, slot manifest, and slot DB ground truth")
    );
}

#[test]
fn args_reject_fused_db_and_slot_truth_sources_together() {
    let err = Args::parse(&strings([
        "--plan",
        "plan.json",
        "--fused-ground-truth-cf-root",
        "fused-truth-db",
        "--slot-ground-truth-cf-root",
        "slot-truth-db",
    ]))
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(
        err.message()
            .contains("precomputed fused file, fused DB, slot manifest, and slot DB ground truth")
    );
}

#[test]
fn args_reject_fused_db_source_and_write_in_one_run() {
    let err = Args::parse(&strings([
        "--plan",
        "plan.json",
        "--fused-ground-truth-cf-root",
        "fused-truth-db",
        "--write-fused-ground-truth-cf-root",
        "new-fused-truth-db",
    ]))
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(
        err.message()
            .contains("precomputed and generated fused ground truth")
    );
}

#[test]
fn args_reject_slot_manifest_and_db_truth_sources_together() {
    let err = Args::parse(&strings([
        "--plan",
        "plan.json",
        "--slot-ground-truth-manifest",
        "slot-truth.manifest.json",
        "--slot-ground-truth-cf-root",
        "slot-truth-db",
    ]))
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(
        err.message()
            .contains("precomputed fused file, fused DB, slot manifest, and slot DB ground truth")
    );
}

#[test]
fn to_index_hits_preserves_rank_and_cx_id() {
    let hits = to_index_hits(vec![(9, 0.1), (3, 0.2)]);

    assert_eq!(hits[0].rank, 1);
    assert_eq!(low_u64(hits[0].cx_id), 9);
    assert_eq!(hits[1].rank, 2);
    assert_eq!(low_u64(hits[1].cx_id), 3);
}

fn strings(items: impl IntoIterator<Item = &'static str>) -> Vec<String> {
    items.into_iter().map(str::to_string).collect()
}
