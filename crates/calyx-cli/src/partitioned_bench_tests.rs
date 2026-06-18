use super::*;

#[test]
fn partitioned_search_parses_recall_floor() {
    let args = strings([
        "--vault",
        "vault",
        "--ground-truth",
        "200",
        "--recall-floor",
        "0.85",
    ]);

    let parsed = SearchArgs::parse(&args).unwrap();

    assert_eq!(parsed.ground_truth, 200);
    assert_eq!(parsed.recall_floor, Some(0.85));
}

#[test]
fn partitioned_build_parses_region_build_parallelism() {
    let args = strings([
        "--vault",
        "vault",
        "--n-cx",
        "1000",
        "--regions",
        "8",
        "--region-build-parallelism",
        "3",
    ]);

    let parsed = BuildArgs::parse(&args).unwrap();

    assert_eq!(parsed.p.region_build_parallelism, 3);
}

#[test]
fn recall_floor_requires_ground_truth_readback() {
    let err = enforce_recall_floor(Some(0.85), 0, None).unwrap_err();

    assert_eq!(err.code(), "CALYX_FSV_PARTITIONED_GROUND_TRUTH_REQUIRED");
    assert!(err.message().contains("--ground-truth > 0"));
}

#[test]
fn recall_floor_rejects_low_true_recall() {
    let err = enforce_recall_floor(Some(0.85), 200, Some(0.84)).unwrap_err();

    assert_eq!(err.code(), "CALYX_FSV_PARTITIONED_RECALL_BELOW_FLOOR");
    assert!(err.message().contains("ground_truth_recall_at_k=0.840000"));
}

#[test]
fn recall_floor_accepts_true_recall_at_floor() {
    enforce_recall_floor(Some(0.85), 200, Some(0.85)).unwrap();
}

#[test]
fn partitioned_search_rejects_zero_probe_count() {
    let args = strings(["--vault", "vault", "--n-probe", "0"]);

    let err = match SearchArgs::parse(&args) {
        Ok(_) => panic!("zero n-probe accepted"),
        Err(err) => err,
    };

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("--n-probe must be > 0"));
}

fn strings(items: impl IntoIterator<Item = &'static str>) -> Vec<String> {
    items.into_iter().map(str::to_string).collect()
}
