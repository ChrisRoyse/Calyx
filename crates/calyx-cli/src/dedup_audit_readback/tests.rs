use std::path::PathBuf;

use super::*;

#[test]
fn cx_list_args_parse_bounded_filters() {
    let cx_id = "00000000000000000000000000000001";
    let args = parse_cx_list_args(&[
        "--vault".to_string(),
        "vault-dir".to_string(),
        "--cx-id".to_string(),
        cx_id.to_string(),
        "--limit".to_string(),
        "1".to_string(),
    ])
    .unwrap();

    assert_eq!(args.vault, PathBuf::from("vault-dir"));
    assert_eq!(args.cx_id.unwrap().to_string(), cx_id);
    assert_eq!(args.limit, Some(1));
    assert!(!args.include_slots);
    assert!(!args.allow_unbounded);
    assert!(args.progress_jsonl.is_none());
    assert!(args.time_budget_ms.is_none());
    assert!(!args.rebuild_base_page_index);
    assert_eq!(
        args.base_page_index_page_size,
        DEFAULT_BASE_PAGE_INDEX_PAGE_SIZE
    );
}

#[test]
fn cx_list_rejects_zero_limit() {
    let err = parse_cx_list_args(&[
        "--vault".to_string(),
        "vault-dir".to_string(),
        "--limit".to_string(),
        "0".to_string(),
    ])
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("at least 1"));
}

#[test]
fn cx_list_unbounded_does_not_decode_slots_unless_explicit() {
    let base_only = parse_cx_list_args(&[
        "--vault".to_string(),
        "vault-dir".to_string(),
        "--allow-unbounded".to_string(),
    ])
    .unwrap();
    let with_slots = parse_cx_list_args(&[
        "--vault".to_string(),
        "vault-dir".to_string(),
        "--allow-unbounded".to_string(),
        "--include-slots".to_string(),
    ])
    .unwrap();

    assert!(!base_only.include_slots);
    assert!(with_slots.include_slots);
}

#[test]
fn cx_list_progress_and_budget_parse() {
    let args = parse_cx_list_args(&[
        "--vault".to_string(),
        "vault-dir".to_string(),
        "--progress-jsonl".to_string(),
        "stderr".to_string(),
        "--time-budget-ms".to_string(),
        "50".to_string(),
        "--rebuild-base-page-index".to_string(),
        "--base-page-index-page-size".to_string(),
        "7".to_string(),
    ])
    .unwrap();

    assert_eq!(args.progress_jsonl, Some("stderr".to_string()));
    assert_eq!(args.time_budget_ms, Some(50));
    assert!(args.rebuild_base_page_index);
    assert_eq!(args.base_page_index_page_size, 7);
}

#[test]
fn cx_list_tombstone_row_reports_tombstoned_not_corrupt() {
    let cx_id = CxId::from_bytes([0x17; 16]);
    let row = tombstone_row(&base_key(cx_id));

    assert_eq!(row["cx_id"], cx_id.to_string());
    assert_eq!(row["base_visible"], false);
    assert_eq!(row["tombstoned"], true);
    assert_eq!(row["slot_payload_decode_mode"], "mvcc_tombstone");
}
