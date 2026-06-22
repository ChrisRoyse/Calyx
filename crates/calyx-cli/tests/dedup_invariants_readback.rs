use std::fs;

#[path = "support/dedup_invariants_readback.rs"]
mod support;

use serde_json::json;
use support::{
    anchor_conflict_scenario, frequency_count_scenario, fsv_root, list_files,
    near_distinct_scenario, recurring_reversible_scenario, reset_dir, temporal_excluded_scenario,
    write_blake3_sums, write_json,
};

#[test]
fn dedup_invariants_readback_proves_ph41_exit_gate() {
    let (root, keep_root) = fsv_root(
        "CALYX_DEDUP_INVARIANTS_FSV_ROOT",
        "calyx-dedup-invariants-fsv",
    );
    let before = json!({
        "root_exists_before_reset": root.exists(),
        "files_before_reset": list_files(&root),
    });
    reset_dir(&root);

    let near_distinct = near_distinct_scenario(&root);
    let anchor_conflict = anchor_conflict_scenario(&root);
    let recurring_reversible = recurring_reversible_scenario(&root);
    let temporal_excluded = temporal_excluded_scenario(&root);
    let frequency_count = frequency_count_scenario(&root);

    let readback = json!({
        "before": before,
        "near_but_distinct_not_merged": near_distinct,
        "conflicting_anchor_stays_separate": anchor_conflict,
        "recurring_event_series_reversible": recurring_reversible,
        "temporal_excluded_from_dedup_agreement": temporal_excluded,
        "frequency_count_accurate": frequency_count,
        "after": {"files": list_files(&root)},
    });
    write_json(&root.join("dedup-invariants-readback.json"), &readback);
    write_blake3_sums(&root);

    assert_eq!(
        readback["near_but_distinct_not_merged"]["cx_list"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        readback["conflicting_anchor_stays_separate"]["audit_second"]["anchor_conflict_blocks"][0],
        readback["conflicting_anchor_stays_separate"]["first_id"]
    );
    for expected in readback["recurring_event_series_reversible"]["expected_base_hex"]
        .as_array()
        .unwrap()
    {
        let cx_id = expected["cx_id"].as_str().unwrap();
        let restored = readback["recurring_event_series_reversible"]["cx_list_after"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["cx_id"].as_str() == Some(cx_id))
            .expect("restored row");
        assert_eq!(restored["base_hex"], expected["base_hex"]);
    }
    assert_eq!(
        readback["temporal_excluded_from_dedup_agreement"]["audit"]["merges"][0]["per_slot_cos"][0]
            [0],
        json!(0)
    );
    assert_eq!(
        readback["frequency_count_accurate"]["series"]["frequency"],
        json!(10)
    );
    assert_eq!(
        readback["frequency_count_accurate"]["store_occurrence_count"],
        json!(10)
    );

    println!("dedup_invariants_fsv_root={}", root.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    if !keep_root {
        fs::remove_dir_all(root).expect("cleanup temp root");
    }
}
