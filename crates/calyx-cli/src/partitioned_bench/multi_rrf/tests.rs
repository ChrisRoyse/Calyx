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
    assert_eq!(args.recall_floor, Some(0.8));
    assert_eq!(args.out, None);
    assert_eq!(args.anneal_vault, Some(PathBuf::from("anneal-out")));
    assert_eq!(args.tuner_slo_us, Some(100));
}

#[test]
fn args_reject_zero_tuner_slo() {
    let err = Args::parse(&strings(["--plan", "plan.json", "--tuner-slo-us", "0"])).unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("--tuner-slo-us must be > 0"));
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
