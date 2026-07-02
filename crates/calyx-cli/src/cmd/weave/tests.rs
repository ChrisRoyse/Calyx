use super::*;
use calyx_core::{
    Asymmetry, LensId, Modality, Panel, QuantPolicy, Slot, SlotId, SlotKey, SlotShape, SlotState,
};

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
    assert_eq!(
        args.candidate_selection,
        coverage::CandidateSelectionMode::BasePrefix
    );
    assert!(!args.coverage_only);
    assert_eq!(args.time_budget_ms, None);
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
        "--candidate-selection",
        "base-prefix",
        "--coverage-only",
        "--time-budget-ms",
        "5000",
    ])
    .unwrap();
    assert_eq!(args.content_slot, Some(8));
    assert_eq!(args.knn, 24);
    assert!((args.edge_cos_threshold - 0.7).abs() < 1e-6);
    assert_eq!(args.max_groundedness_distance, 4);
    assert_eq!(args.batch, 1000);
    assert_eq!(args.limit, 50);
    assert_eq!(
        args.candidate_selection,
        coverage::CandidateSelectionMode::BasePrefix
    );
    assert!(args.coverage_only);
    assert_eq!(args.time_budget_ms, Some(5000));
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
    assert_eq!(args.time_budget_ms, None);
}

#[test]
fn invalid_candidate_selection_fails_closed() {
    let err = parse(&["corpus", "--candidate-selection", "partial"]).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("covered|base-prefix"));
}

#[test]
fn covered_selection_picks_largest_viable_lower_slot_tie() {
    let coverage = vec![
        coverage_row(8, 5, 1),
        coverage_row(9, 5, 4),
        coverage_row(10, 5, 4),
    ];

    let selected = coverage::select_slot_from_coverage(
        None,
        coverage::CandidateSelectionMode::Covered,
        3,
        &coverage,
    )
    .unwrap();

    assert_eq!(selected.slot, SlotId::new(9));
    assert_eq!(selected.reason, "largest_dense_covered_candidate_set");
    assert_eq!(selected.selected_rows, 3);
    assert_eq!(selected.scanned_rows, 5);
    assert_eq!(selected.excluded_uncovered_rows, 1);
}

#[test]
fn covered_selection_allows_requested_partial_slot_with_visible_exclusions() {
    let coverage = vec![coverage_row(10, 25, 2)];

    let selected = coverage::select_slot_from_coverage(
        Some(SlotId::new(10)),
        coverage::CandidateSelectionMode::Covered,
        25,
        &coverage,
    )
    .unwrap();

    assert_eq!(selected.slot, SlotId::new(10));
    assert_eq!(selected.reason, "requested_slot_covered_candidate_set");
    assert_eq!(selected.selected_rows, 2);
    assert_eq!(selected.excluded_uncovered_rows, 23);
}

#[test]
fn base_prefix_selection_still_fails_when_prefix_not_fully_covered() {
    let coverage = vec![coverage_row(10, 5, 4)];

    let err = coverage::select_slot_from_coverage(
        None,
        coverage::CandidateSelectionMode::BasePrefix,
        5,
        &coverage,
    )
    .unwrap_err();

    assert!(err.contains("CALYX_WEAVE_LOOM_NO_FULL_DENSE_SLOT"));
}

#[test]
fn covered_selection_fails_closed_when_no_slot_has_two_rows() {
    let coverage = vec![coverage_row(10, 5, 1)];

    let err = coverage::select_slot_from_coverage(
        None,
        coverage::CandidateSelectionMode::Covered,
        5,
        &coverage,
    )
    .unwrap_err();

    assert!(err.contains("CALYX_WEAVE_LOOM_NO_COVERED_DENSE_SET"));
}

#[test]
fn content_slots_filter_to_active_dense_lenses() {
    let panel = mixed_shape_panel();

    assert_eq!(
        super::content_lens_slots(&panel),
        vec![SlotId::new(10), SlotId::new(15)]
    );
    assert_eq!(
        super::incompatible_content_lens_slots(&panel),
        vec![
            IncompatibleContentSlot {
                slot_id: 11,
                shape: "sparse:30522".to_string(),
                reason: "active_content_slot_shape_is_not_dense",
            },
            IncompatibleContentSlot {
                slot_id: 12,
                shape: "multi:384".to_string(),
                reason: "active_content_slot_shape_is_not_dense",
            },
        ]
    );
}

#[test]
fn requested_sparse_content_slot_fails_with_incompatible_readback() {
    let panel = mixed_shape_panel();
    let content_slots = super::content_lens_slots(&panel);
    let incompatible_slots = super::incompatible_content_lens_slots(&panel);

    let err = super::resolve_requested_slot(Some(11), &content_slots, &incompatible_slots)
        .expect_err("sparse content slot must fail before vault mutation");
    let message = err.to_string();

    assert!(message.contains("--content-slot 11 is not an active dense content lens"));
    assert!(message.contains("choose one of [10, 15]"));
    assert!(message.contains("slot_id: 11"));
    assert!(message.contains("sparse:30522"));
}

fn mixed_shape_panel() -> Panel {
    Panel {
        version: 1,
        slots: vec![
            slot(10, SlotShape::Dense(768), false, SlotState::Active),
            slot(11, SlotShape::Sparse(30_522), false, SlotState::Active),
            slot(
                12,
                SlotShape::Multi { token_dim: 384 },
                false,
                SlotState::Active,
            ),
            slot(13, SlotShape::Dense(768), true, SlotState::Active),
            slot(14, SlotShape::Dense(768), false, SlotState::Parked),
            slot(15, SlotShape::Dense(768), false, SlotState::Active),
        ],
        created_at: 1,
        kernel_ref: None,
        guard_ref: None,
    }
}

fn slot(id: u16, shape: SlotShape, retrieval_only: bool, state: SlotState) -> Slot {
    let slot_id = SlotId::new(id);
    Slot {
        slot_id,
        slot_key: SlotKey::new(slot_id, format!("slot-{id}")),
        lens_id: LensId::from_bytes([id as u8; 16]),
        shape,
        modality: Modality::Text,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::None,
        resource: Default::default(),
        axis: None,
        retrieval_only,
        excluded_from_dedup: false,
        bits_about: Default::default(),
        state,
        added_at_panel_version: 1,
    }
}

fn coverage_row(
    slot_id: u16,
    candidate_rows: usize,
    dense_rows: usize,
) -> coverage::DenseSlotCoverage {
    coverage::DenseSlotCoverage {
        slot_id,
        candidate_rows,
        dense_rows,
        missing_rows: candidate_rows.saturating_sub(dense_rows),
        non_dense_rows: 0,
        absent_rows: 0,
        tombstoned_rows: 0,
        example_missing_cx_ids: Vec::new(),
        read_stats: crate::provenance_read::ProvenanceReadStats::default(),
    }
}
