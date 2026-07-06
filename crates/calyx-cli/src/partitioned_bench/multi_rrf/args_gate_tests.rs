use super::{Args, strings};

#[test]
fn args_reject_recall_floor_without_db_timeline() {
    let err = Args::parse(&strings([
        "--plan-cf-root",
        "plan-db",
        "--a37-admission-cf-root",
        "a37-db",
        "--slot-ground-truth-cf-root",
        "slot-truth-db",
        "--ground-truth",
        "5",
        "--recall-floor",
        "0.8",
    ]))
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("--timeline-cf-root"));
}

#[test]
fn args_reject_recall_floor_without_db_a37_admission() {
    let err = Args::parse(&strings([
        "--plan-cf-root",
        "plan-db",
        "--timeline-cf-root",
        "timeline-db",
        "--slot-ground-truth-cf-root",
        "slot-truth-db",
        "--ground-truth",
        "5",
        "--recall-floor",
        "0.8",
    ]))
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("--a37-admission-cf-root"));
}

#[test]
fn args_reject_recall_floor_with_json_gate_card() {
    let err = Args::parse(&strings([
        "--plan-cf-root",
        "plan-db",
        "--timeline-cf-root",
        "timeline-db",
        "--a37-admission-cf-root",
        "a37-db",
        "--ensemble-card",
        "ensemble_card.json",
        "--slot-ground-truth-cf-root",
        "slot-truth-db",
        "--ground-truth",
        "5",
        "--recall-floor",
        "0.8",
    ]))
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("JSON A37 admission or ensemble"));
}

#[test]
fn args_reject_recall_floor_with_file_truth() {
    let err = Args::parse(&strings([
        "--plan-cf-root",
        "plan-db",
        "--timeline-cf-root",
        "timeline-db",
        "--a37-admission-cf-root",
        "a37-db",
        "--fused-ground-truth-file",
        "truth.i32bin",
        "--fused-ground-truth-manifest",
        "truth.manifest.json",
        "--ground-truth",
        "5",
        "--recall-floor",
        "0.8",
    ]))
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("file or manifest truth"));
}

#[test]
fn args_reject_recall_floor_without_db_truth() {
    let err = Args::parse(&strings([
        "--plan-cf-root",
        "plan-db",
        "--timeline-cf-root",
        "timeline-db",
        "--a37-admission-cf-root",
        "a37-db",
        "--ground-truth",
        "5",
        "--recall-floor",
        "0.8",
    ]))
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("--fused-ground-truth-cf-root"));
}
