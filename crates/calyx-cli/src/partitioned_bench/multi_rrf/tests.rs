use super::*;

#[test]
fn args_parse_plan_truth_depth_and_tuner_vault() {
    let args = Args::parse(&strings([
        "--plan",
        "plan.json",
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
        "--recall-floor",
        "0.8",
        "--anneal-vault",
        "anneal-out",
        "--tuner-slo-us",
        "100",
    ]))
    .unwrap();

    assert_eq!(args.plan, PathBuf::from("plan.json"));
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
    assert_eq!(args.recall_floor, Some(0.8));
    assert_eq!(args.out, None);
    assert_eq!(args.anneal_vault, Some(PathBuf::from("anneal-out")));
    assert_eq!(args.tuner_slo_us, Some(100));
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
            .contains("--fused-ground-truth-file and --slot-ground-truth-manifest")
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
