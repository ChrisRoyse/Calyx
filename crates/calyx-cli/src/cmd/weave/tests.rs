use super::*;

fn toks(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

fn parse(parts: &[&str]) -> CliResult<WeaveLoomArgs> {
    match super::parse_weave_loom(&toks(parts))? {
        Subcommand::WeaveLoom(args) => Ok(args),
        _ => unreachable!("parse_weave_loom must return WeaveLoom"),
    }
}

#[test]
fn defaults_apply_when_only_vault_given() {
    let args = parse(&["corpus"]).unwrap();
    assert_eq!(args.vault, "corpus");
    assert_eq!(args.content_slot, None);
    assert_eq!(args.knn, DEFAULT_KNN);
    assert_eq!(args.edge_cos_threshold, DEFAULT_EDGE_COS_THRESHOLD);
    assert_eq!(
        args.max_groundedness_distance,
        DEFAULT_MAX_GROUNDEDNESS_DISTANCE
    );
    assert_eq!(args.batch, DEFAULT_BATCH);
    assert_eq!(args.limit, 0);
}

#[test]
fn all_flags_parse() {
    let args = parse(&[
        "corpus",
        "--content-slot",
        "8",
        "--knn",
        "24",
        "--edge-cos-threshold",
        "0.7",
        "--max-groundedness-distance",
        "4",
        "--batch",
        "1000",
        "--limit",
        "50",
    ])
    .unwrap();
    assert_eq!(args.content_slot, Some(8));
    assert_eq!(args.knn, 24);
    assert!((args.edge_cos_threshold - 0.7).abs() < 1e-6);
    assert_eq!(args.max_groundedness_distance, 4);
    assert_eq!(args.batch, 1000);
    assert_eq!(args.limit, 50);
}

#[test]
fn missing_vault_fails_closed() {
    let err = super::parse_weave_loom(&[]).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
}

#[test]
fn knn_below_one_fails_closed() {
    let err = parse(&["corpus", "--knn", "0"]).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("--knn must be >= 1"));
}

#[test]
fn threshold_out_of_range_fails_closed() {
    let err = parse(&["corpus", "--edge-cos-threshold", "1.5"]).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");

    let err = parse(&["corpus", "--edge-cos-threshold", "nan"]).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
}

#[test]
fn max_groundedness_distance_zero_fails_closed() {
    let err = parse(&["corpus", "--max-groundedness-distance", "0"]).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
}

#[test]
fn unknown_flag_fails_closed() {
    let err = parse(&["corpus", "--bogus", "1"]).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("unexpected weave-loom flag"));
}

#[test]
fn limit_zero_is_valid_meaning_all() {
    let args = parse(&["corpus", "--limit", "0"]).unwrap();
    assert_eq!(args.limit, 0);
}
