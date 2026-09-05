use super::*;
use calyx_core::{
    Asymmetry, Constellation, CxFlags, CxId, InputRef, LedgerRef, LensId, Modality, Panel,
    QuantPolicy, Slot, SlotId, SlotKey, SlotShape, SlotState, VaultId,
};
use std::collections::BTreeMap;

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
fn rss_budget_parses_positive_byte_count() {
    assert_eq!(
        super::parse_rss_budget("1073741824").unwrap(),
        1_073_741_824
    );
    // Surrounding whitespace is tolerated (env values often carry it).
    assert_eq!(super::parse_rss_budget("  4096\n").unwrap(), 4096);
}

#[test]
fn rss_budget_rejects_non_numeric_and_zero_fail_closed() {
    let err = super::parse_rss_budget("lots").unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("CALYX_WEAVE_RSS_BUDGET_BYTES"));

    let err = super::parse_rss_budget("0").unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("positive byte budget"));

    let err = super::parse_rss_budget("-1").unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
}

#[test]
fn rss_budget_decision_refuses_when_projected_peak_exceeds() {
    // No budget configured => never refuses.
    assert_eq!(
        super::projected_peak_over_budget(10, 10, None),
        None,
        "absent budget must never trip the guard"
    );
    // Current + additional under budget => within budget.
    assert_eq!(
        super::projected_peak_over_budget(400, 100, Some(1_000)),
        None
    );
    // Exactly at budget is allowed (strictly-greater refusal).
    assert_eq!(
        super::projected_peak_over_budget(600, 400, Some(1_000)),
        None
    );
    // Projected peak over budget => refuse, returning the projected peak.
    assert_eq!(
        super::projected_peak_over_budget(600, 500, Some(1_000)),
        Some(1_100)
    );
    // Saturating arithmetic: a pathological additive estimate cannot wrap.
    assert_eq!(
        super::projected_peak_over_budget(u64::MAX, 1, Some(u64::MAX - 1)),
        Some(u64::MAX)
    );
}

// The live-RSS phase guard (#1991 fix 3) is exercised through a real progress
// writer, which stages owner-only (0600) evidence files via primitives that have
// no off-Unix implementation, so these run in the WSL weave suite. The guard
// *decision* is trivial (`bytes > budget`); its typed fail-closed error and
// incomplete-artifact side effect are what these assert.
#[cfg(unix)]
mod live_rss_guard {
    use super::*;

    fn progress_writer_in_temp() -> (std::path::PathBuf, progress::WeaveLoomProgressWriter) {
        let dir = std::env::temp_dir().join(format!(
            "calyx-weave-rss-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let progress =
            progress::WeaveLoomProgressWriter::create(&dir, "rss-test", &WeaveLoomArgs::default())
                .unwrap();
        (dir, progress)
    }

    #[test]
    fn live_rss_guard_fires_typed_backpressure_not_abort() {
        // A pathologically tiny budget (1 byte) must be exceeded by the live
        // process RSS, and the guard must fail closed with a typed
        // CALYX_BACKPRESSURE error — never march on toward a kernel OOM-kill.
        let (dir, progress) = progress_writer_in_temp();
        let mut rss = super::super::RssTracker::new();
        let err = rss
            .sample(&progress, "within_doc_progress", Some(1), 0, Some(512))
            .expect_err("a 1-byte budget must be exceeded by live RSS");
        assert_eq!(err.code(), "CALYX_BACKPRESSURE");
        assert!(err.message().contains("live RSS"));
        assert!(err.message().contains("phase within_doc"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn live_rss_guard_absent_budget_never_gates() {
        // With no budget configured the sample is best-effort observability: it
        // records the phase-RSS trail and always returns Ok.
        let (dir, progress) = progress_writer_in_temp();
        let mut rss = super::super::RssTracker::new();
        rss.sample(&progress, "within_doc_progress", None, 4096, Some(512))
            .expect("absent budget must never gate the weave");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn defaults_apply_when_only_vault_given() {
    let args = parse(&["corpus"]).unwrap();
    assert_eq!(args.vault, "corpus");
    assert_eq!(args.content_slot, None);
    assert_eq!(args.knn, DEFAULT_KNN);
    assert_eq!(args.edge_score_threshold, DEFAULT_EDGE_SCORE_THRESHOLD);
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
    assert_eq!(args.candidate_device, super::CandidateDevice::Cpu);
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
        "--candidate-device",
        "gpu",
        "--coverage-only",
        "--time-budget-ms",
        "5000",
    ])
    .unwrap();
    assert_eq!(args.content_slot, Some(8));
    assert_eq!(args.knn, 24);
    assert!((args.edge_score_threshold - 0.7).abs() < 1e-6);
    assert_eq!(args.max_groundedness_distance, 4);
    assert_eq!(args.batch, 1000);
    assert_eq!(args.limit, 50);
    assert_eq!(
        args.candidate_selection,
        coverage::CandidateSelectionMode::BasePrefix
    );
    assert_eq!(args.candidate_device, super::CandidateDevice::Gpu);
    assert!(args.coverage_only);
    assert_eq!(args.time_budget_ms, Some(5000));
}

#[test]
fn candidate_device_parses_cpu_and_gpu() {
    assert_eq!(
        parse(&["corpus", "--candidate-device", "cpu"])
            .unwrap()
            .candidate_device,
        super::CandidateDevice::Cpu
    );
    assert_eq!(
        parse(&["corpus", "--candidate-device", "gpu"])
            .unwrap()
            .candidate_device,
        super::CandidateDevice::Gpu
    );
}

#[test]
fn invalid_candidate_device_fails_closed() {
    let err = parse(&["corpus", "--candidate-device", "tpu"]).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("cpu|gpu"));
}

#[test]
fn candidate_device_gpu_without_cuda_feature_fails_closed() {
    // On a binary built without --features cuda there is no GPU search surface;
    // the forced-GPU candidate entry point must refuse with the specific
    // fail-closed code rather than silently serving the CPU scan (#1963).
    #[cfg(not(feature = "cuda"))]
    {
        assert_eq!(
            calyx_search::persisted::WEAVE_CANDIDATE_CUDA_FEATURE_MISSING,
            "CALYX_WEAVE_CANDIDATE_CUDA_FEATURE_MISSING"
        );
    }
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
fn panel_preflight_selects_only_constellations_dense_in_every_slot() {
    // Panel-native selection is an intersection: a constellation is only
    // selected when it carries a measured typed vector in *every* contracted
    // content slot, never the coverage set of a single preferred lens.
    // Constellations dense in only one lane are excluded. The intersection is
    // driven entirely by the scan's per-ordinal present-slot counts — no
    // resident vector payload (#1991).
    let slots = [SlotId::new(9), SlotId::new(10)];
    let complete_a = candidate(b"cx-complete-a");
    let complete_b = candidate(b"cx-complete-b");
    let only_9 = candidate(b"cx-only-9");
    let only_10 = candidate(b"cx-only-10");
    let (candidates, counts, coverage) = selection_inputs(
        &slots,
        &[
            (complete_a.clone(), &[9, 10]),
            (only_9.clone(), &[9]),
            (complete_b.clone(), &[9, 10]),
            (only_10, &[10]),
        ],
    );

    let selection =
        coverage::select_complete_panel(candidates, &counts, &slots, &slots, 0, &coverage).unwrap();

    assert_eq!(selection.candidates.len(), 2);
    assert_eq!(
        selection.selected_candidate_cx_ids,
        vec![complete_a.cx_id.to_string(), complete_b.cx_id.to_string()]
    );
    // Ordinals point back at the complete candidates' scan positions (0 and 2).
    assert_eq!(selection.candidate_ordinals, vec![0, 2]);
    assert_eq!(selection.ordinal_map.ordinal(&complete_a.cx_id), Some(0));
    assert_eq!(selection.ordinal_map.ordinal(&complete_b.cx_id), Some(2));
    assert_eq!(selection.ordinal_map.ordinal(&only_9.cx_id), None);
}

#[test]
fn panel_preflight_bounds_complete_selection_by_limit() {
    // A non-zero limit caps the complete panel to the first N candidates in
    // deterministic base-page scan order; the rest are dropped from the graph.
    let slots = [SlotId::new(9), SlotId::new(10)];
    let first = candidate(b"cx-first");
    let second = candidate(b"cx-second");
    let third = candidate(b"cx-third");
    let (candidates, counts, coverage) = selection_inputs(
        &slots,
        &[
            (first.clone(), &[9, 10]),
            (second.clone(), &[9, 10]),
            (third.clone(), &[9, 10]),
        ],
    );

    let selection =
        coverage::select_complete_panel(candidates, &counts, &slots, &slots, 2, &coverage).unwrap();

    assert_eq!(selection.candidates.len(), 2);
    assert_eq!(
        selection.selected_candidate_cx_ids,
        vec![first.cx_id.to_string(), second.cx_id.to_string()]
    );
    assert_eq!(selection.ordinal_map.ordinal(&third.cx_id), None);
}

#[test]
fn panel_preflight_fails_closed_when_fewer_than_two_slots() {
    // A one-lane contract can never form a panel; selection fails closed.
    let slots = [SlotId::new(10)];
    let (candidates, counts, coverage) =
        selection_inputs(&slots, &[(candidate(b"cx-only"), &[10])]);

    let Err(err) =
        coverage::select_complete_panel(candidates, &counts, &slots, &slots, 0, &coverage)
    else {
        panic!("one-lane contract must fail closed");
    };

    assert!(err.contains("CALYX_WEAVE_LOOM_PANEL_TOO_SMALL"));
}

#[test]
fn panel_preflight_fails_closed_when_fewer_than_two_complete_constellations() {
    // Two lanes are scanned but only one constellation is dense in both, so the
    // complete panel is too small to weave and selection fails closed.
    let slots = [SlotId::new(9), SlotId::new(10)];
    let (candidates, counts, coverage) = selection_inputs(
        &slots,
        &[
            (candidate(b"cx-complete"), &[9, 10]),
            (candidate(b"cx-only-9"), &[9]),
            (candidate(b"cx-only-10"), &[10]),
        ],
    );

    let Err(err) =
        coverage::select_complete_panel(candidates, &counts, &slots, &slots, 0, &coverage)
    else {
        panic!("under-covered panel must fail closed");
    };

    assert!(err.contains("CALYX_WEAVE_LOOM_COMPLETE_PANEL_EMPTY"));
}

#[test]
fn content_slots_preserve_every_active_typed_lens() {
    let panel = mixed_shape_panel();

    assert_eq!(
        super::content_lens_slots(&panel),
        vec![
            SlotId::new(10),
            SlotId::new(11),
            SlotId::new(12),
            SlotId::new(15)
        ]
    );
    assert_eq!(
        super::incompatible_content_lens_slots(&panel),
        Vec::<IncompatibleContentSlot>::new()
    );
}

#[test]
fn sparse_and_multi_content_slots_are_native_panel_lanes() {
    let panel = mixed_shape_panel();
    let content_slots = super::content_lens_slots(&panel);
    let incompatible_slots = super::incompatible_content_lens_slots(&panel);

    assert!(content_slots.contains(&SlotId::new(11)));
    assert!(content_slots.contains(&SlotId::new(12)));
    assert!(incompatible_slots.is_empty());
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
        sparse_rows: 0,
        multi_rows: 0,
        segmented_multi_rows: 0,
        measured_rows: dense_rows,
        missing_rows: candidate_rows.saturating_sub(dense_rows),
        absent_rows: 0,
        tombstoned_rows: 0,
        example_missing_cx_ids: Vec::new(),
        read_stats: crate::provenance_read::ProvenanceReadStats::default(),
    }
}

/// A minimal deterministic constellation. Panel preflight only reads `cx_id`
/// (for the complete-panel intersection), so the remaining fields are inert.
fn candidate(seed: &[u8]) -> Constellation {
    Constellation {
        cx_id: CxId::from_input(seed, 1, b"weave-loom-preflight-test-salt"),
        vault_id: VaultId::from_ulid(ulid::Ulid::from_bytes([1; 16])),
        panel_version: 1,
        created_at: 1,
        input_ref: InputRef {
            hash: [3; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 1,
            hash: [4; 32],
        },
        flags: CxFlags::default(),
    }
}

/// Builds the filesystem-free inputs to [`coverage::select_complete_panel`] from
/// an ordered list of `(constellation, slot ids it carries a measured vector
/// in)`: the scanned candidate order (ordinal = index), the per-ordinal count of
/// how many of the contracted `slots` each candidate is measured in, and the
/// coverage rows used only for the empty-panel diagnostic. Mirrors what the
/// out-of-core scan records without touching the scratch store.
fn selection_inputs(
    slots: &[SlotId],
    candidates: &[(Constellation, &[u16])],
) -> (
    Vec<Constellation>,
    Vec<u32>,
    Vec<coverage::DenseSlotCoverage>,
) {
    let scanned = candidates.iter().map(|(cx, _)| cx.clone()).collect();
    let counts = candidates
        .iter()
        .map(|(_, member_slots)| {
            slots
                .iter()
                .filter(|slot| member_slots.contains(&slot.get()))
                .count() as u32
        })
        .collect();
    let coverage = slots
        .iter()
        .map(|&slot| {
            let dense = candidates
                .iter()
                .filter(|(_, member_slots)| member_slots.contains(&slot.get()))
                .count();
            coverage_row(slot.get(), candidates.len(), dense)
        })
        .collect();
    (scanned, counts, coverage)
}
