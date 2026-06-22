use std::fs;

#[path = "support/dedup_ingest_at_readback.rs"]
mod dedup_ingest_at_readback_support;

use dedup_ingest_at_readback_support::{
    anchor_conflict_scenario, event_time_edges_scenario, event_time_fallback_signature_scenario,
    exact_duplicate_scenario, fsv_root, list_files, missing_temporal_signature_scenario,
    negative_time_scenario, recurrence_scenario, reset_dir, same_temporal_signature_scenario,
    write_blake3_sums, write_json,
};
use serde_json::json;

#[test]
fn dedup_ingest_at_writes_event_time_merge_and_ledger_bytes() {
    let (root, keep_root) = fsv_root(
        "CALYX_DEDUP_INGEST_AT_FSV_ROOT",
        "calyx-dedup-ingest-at-fsv",
    );
    let before = json!({
        "root_exists_before_reset": root.exists(),
        "files_before_reset": list_files(&root),
    });
    reset_dir(&root);

    let recurrence = recurrence_scenario(&root);
    let same_time = same_temporal_signature_scenario(&root);
    let fallback = event_time_fallback_signature_scenario(&root);
    let missing_temporal = missing_temporal_signature_scenario(&root);
    let exact = exact_duplicate_scenario(&root);
    let conflict = anchor_conflict_scenario(&root);
    let edges = event_time_edges_scenario(&root);
    let negative = negative_time_scenario(&root);

    let readback = json!({
        "before": before,
        "recurrence": recurrence,
        "same_temporal_signature": same_time,
        "event_time_fallback_signature": fallback,
        "missing_temporal_signature": missing_temporal,
        "exact_duplicate": exact,
        "anchor_conflict": conflict,
        "event_time_edges": edges,
        "negative_time": negative,
        "after": {
            "files": list_files(&root),
        }
    });
    write_json(&root.join("dedup-ingest-at-readback.json"), &readback);
    write_blake3_sums(&root);

    assert_eq!(readback["recurrence"]["base_row_count"], json!(1));
    assert_eq!(
        readback["recurrence"]["ledger_payloads"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        readback["recurrence"]["occurrence_times"],
        json!([100, 200, 300])
    );
    assert_eq!(
        readback["recurrence"]["results"][1]["DedupMerge"]["occurrence"],
        json!(1)
    );
    assert_eq!(
        readback["recurrence"]["ledger_payloads"][1]["recurrence_signature"],
        json!(true)
    );
    assert_eq!(
        readback["same_temporal_signature"]["second_result"]["ExactDuplicate"],
        readback["same_temporal_signature"]["cx_id"]
    );
    assert_eq!(
        readback["same_temporal_signature"]["recurrence_row_count"],
        json!(1)
    );
    assert_eq!(
        readback["same_temporal_signature"]["ledger_payloads"][1]["recurrence_signature"],
        json!(false)
    );
    assert_eq!(
        readback["event_time_fallback_signature"]["occurrence_times"],
        json!([100, 200])
    );
    assert_eq!(
        readback["event_time_fallback_signature"]["results"][1]["DedupMerge"]["occurrence"],
        json!(1)
    );
    assert_eq!(
        readback["event_time_fallback_signature"]["recurrence_row_count"],
        json!(2)
    );
    assert_eq!(
        readback["event_time_fallback_signature"]["ledger_payloads"][1]["recurrence_signature"],
        json!(true)
    );
    assert_eq!(
        readback["event_time_fallback_signature"]["ledger_payloads"][1]["new_time"],
        json!(200)
    );
    assert_eq!(
        readback["missing_temporal_signature"]["error_code"],
        json!("CALYX_RECURRENCE_SLOT_MISSING")
    );
    assert_eq!(
        readback["missing_temporal_signature"]["ledger_row_count"],
        json!(1)
    );
    assert_eq!(
        readback["missing_temporal_signature"]["recurrence_row_count"],
        json!(1)
    );
    assert_eq!(readback["exact_duplicate"]["base_row_count"], json!(1));
    assert_eq!(
        readback["exact_duplicate"]["second_result"]["ExactDuplicate"],
        readback["exact_duplicate"]["cx_id"]
    );
    assert_eq!(readback["anchor_conflict"]["base_row_count"], json!(2));
    assert!(
        readback["anchor_conflict"]["second_result"]
            .get("New")
            .is_some()
    );
    assert_eq!(
        readback["event_time_edges"]["stored_times"],
        json!([0, 4_102_444_800_i64])
    );
    assert_eq!(
        readback["negative_time"]["error_code"],
        json!("CALYX_DEDUP_INVALID_EVENT_TIME")
    );
    assert_eq!(readback["negative_time"]["base_stdout"], json!(""));

    println!("dedup_ingest_at_fsv_root={}", root.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    if !keep_root {
        fs::remove_dir_all(root).expect("cleanup temp root");
    }
}
