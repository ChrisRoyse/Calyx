use std::path::PathBuf;

use calyx_core::{
    Constellation, CxFlags, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId,
};

use super::physical_slots::PhysicalSlotState;
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
fn slot_summary_counts_physical_states_including_tombstones() {
    let states = [
        PhysicalSlotState::Vector {
            vector: SlotVector::Dense {
                dim: 2,
                data: vec![1.0, 2.0],
            },
            payload_source: "slot_cf",
        },
        PhysicalSlotState::Vector {
            vector: SlotVector::Absent {
                reason: calyx_core::AbsentReason::LensInactive,
            },
            payload_source: "slot_cf",
        },
        PhysicalSlotState::Tombstoned {
            payload_source: "slot_cf_tombstone",
        },
    ];

    let summary = slot_summary(states.iter());

    assert_eq!(summary["slot_count"], 3);
    assert_eq!(summary["dense_slots"], 1);
    assert_eq!(summary["absent_slots"], 1);
    assert_eq!(summary["tombstoned_slots"], 1);
    assert_eq!(summary["absent_reasons"]["lens_inactive"], 1);
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

#[test]
fn cx_list_rows_emit_decoded_base_metadata_and_provenance() {
    let cx_id = CxId::from_bytes([0x33; 16]);
    let mut slots = std::collections::BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![0.25, 0.75],
        },
    );
    let mut metadata = std::collections::BTreeMap::new();
    metadata.insert("derived.kind".to_string(), "transcript".to_string());
    metadata.insert("derived.runtime".to_string(), "whisper.cpp".to_string());
    let cx = Constellation {
        cx_id,
        vault_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse::<VaultId>().unwrap(),
        panel_version: 7,
        created_at: 1_786_000_000,
        input_ref: InputRef {
            hash: [0xab; 32],
            pointer: Some("calyx-vault://inputs/derived_text/transcript/example.txt".to_string()),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: std::collections::BTreeMap::new(),
        metadata,
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 42,
            hash: [0xcd; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    };
    let mut rows = std::collections::BTreeMap::new();
    rows.insert(
        base_key(cx_id),
        calyx_aster::vault::encode::encode_constellation_base(&cx).unwrap(),
    );
    let mut progress = ProgressSink::Disabled;
    let rendered = cx_list_rows(
        std::path::Path::new("unused-vault"),
        rows,
        false,
        &Deadline::new(None),
        &mut progress,
    )
    .unwrap();
    let row = &rendered[0];

    assert_eq!(row["modality"], "text");
    assert_eq!(row["input_ref"]["hash"], "ab".repeat(32));
    assert_eq!(
        row["input_ref"]["pointer"],
        "calyx-vault://inputs/derived_text/transcript/example.txt"
    );
    assert_eq!(row["metadata"]["derived.kind"], "transcript");
    assert_eq!(row["metadata"]["derived.runtime"], "whisper.cpp");
    assert_eq!(row["provenance"]["seq"], 42);
    assert_eq!(row["provenance"]["hash"], "cd".repeat(32));
}
