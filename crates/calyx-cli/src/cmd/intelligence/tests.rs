use std::collections::BTreeMap;
use std::fs;

use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Asymmetry, CxFlags, CxId, InputRef, LedgerRef, LensId,
    Modality, Panel, QuantPolicy, Slot, SlotId, SlotKey, SlotShape, SlotState, SlotVector, VaultId,
};

use super::*;

#[test]
fn bits_insufficient_samples_has_exact_remediation() {
    let docs = docs_with_signal(30, true, false);
    let err = bits::calculate(
        &panel_two_active(),
        &docs,
        &AnchorKind::TestPass,
        "test_pass",
        false,
        b"bits\0test_pass",
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    assert_eq!(err.remediation(), "anchor ≥50 outcomes first");
}

#[test]
fn bits_planted_signal_reports_high_and_low_slots() {
    let docs = docs_with_signal(100, true, false);
    let report = bits::calculate(
        &panel_two_active(),
        &docs,
        &AnchorKind::TestPass,
        "test_pass",
        true,
        b"bits\0test_pass",
    )
    .unwrap();

    assert_eq!(report.n, 100);
    let high = report.per_slot.iter().find(|slot| slot.slot == 0).unwrap();
    let low = report.per_slot.iter().find(|slot| slot.slot == 1).unwrap();
    assert!(high.bits >= 0.05, "{high:?}");
    assert!(low.bits < 0.05, "{low:?}");
    assert!(low.low_signal);
}

#[test]
fn bits_all_low_signal_fails_closed_with_exact_remediation() {
    let docs = docs_with_signal(100, false, false);
    let err = bits::calculate(
        &panel_two_active(),
        &docs,
        &AnchorKind::TestPass,
        "test_pass",
        false,
        b"bits\0test_pass",
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_ASSAY_LOW_SIGNAL");
    assert_eq!(err.remediation(), "park/retire lens");
}

#[test]
fn bits_no_active_slots_is_structured_error() {
    let docs = docs_with_signal(100, true, false);
    let err = bits::calculate(
        &Panel {
            slots: Vec::new(),
            ..panel_two_active()
        },
        &docs,
        &AnchorKind::TestPass,
        "test_pass",
        false,
        b"bits\0test_pass",
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_ASSAY_LOW_SIGNAL");
}

#[test]
fn abundance_on_100_constellations_and_two_slots_matches_card() {
    let docs = docs_with_signal(100, true, false);
    let report = abundance::calculate(&docs, &[SlotId::new(0), SlotId::new(1)]);

    assert_eq!(report.n, 100);
    assert_eq!(report.pairs, 4950);
    assert_eq!(report.panel_size, 2);
}

#[test]
fn kernel_without_anchors_is_ungrounded() {
    let docs = docs_without_anchors(5);
    let err = kernel::calculate(&docs, Some(&AnchorKind::TestPass)).unwrap_err();

    assert_eq!(err.code(), "CALYX_KERNEL_UNGROUNDED");
    assert_eq!(err.remediation(), "add anchors (grounding_gaps)");
}

#[test]
fn guard_calibration_jsonl_blocks_99_percent_injections() {
    let root = temp_root("guard-calibrate");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("calibrate.jsonl");
    let mut jsonl = String::new();
    for _ in 0..100 {
        jsonl.push_str(r#"{"slot":0,"score":0.99,"class":"good"}"#);
        jsonl.push('\n');
        jsonl.push_str(r#"{"slot":0,"score":0.10,"class":"injection"}"#);
        jsonl.push('\n');
    }
    fs::write(&path, jsonl).unwrap();
    let scores = guard::read_calibration_set(&path).unwrap();
    let inputs = scores
        .into_iter()
        .map(|(slot, scores)| calyx_ward::CalibrationInput {
            slot,
            good_scores: scores.good,
            bad_scores: scores.bad,
            slot_kind: calyx_ward::SlotKind::Identity,
            target_far: 0.01,
        })
        .collect::<Vec<_>>();
    let profile = calyx_ward::GuardProfile {
        guard_id: "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101".parse().unwrap(),
        panel_version: 1,
        domain: "unit".to_string(),
        tau: BTreeMap::new(),
        required_slots: Vec::new(),
        policy: calyx_ward::GuardPolicy::AllRequired,
        calibration: None,
        novelty_action: calyx_ward::NoveltyAction::RejectClosed,
    };
    let calibrated =
        calyx_ward::calibrate(profile, inputs, 0.01, &calyx_core::FixedClock::new(1)).unwrap();
    let meta = calibrated.calibration.as_ref().unwrap();

    assert!(1.0 - meta.far >= 0.99, "{meta:?}");
    fs::remove_dir_all(root).ok();
}

fn docs_with_signal(
    n: usize,
    slot0_separates: bool,
    slot1_separates: bool,
) -> BTreeMap<CxId, calyx_core::Constellation> {
    (0..n)
        .map(|idx| {
            let positive = idx < n / 2;
            let cx = constellation(
                idx as u8,
                Some(AnchorValue::Bool(positive)),
                vector_for(positive, slot0_separates),
                vector_for(positive, slot1_separates),
            );
            (cx.cx_id, cx)
        })
        .collect()
}

fn docs_without_anchors(n: usize) -> BTreeMap<CxId, calyx_core::Constellation> {
    (0..n)
        .map(|idx| {
            let cx = constellation(idx as u8, None, vec![1.0, 0.0], vec![1.0, 0.0]);
            (cx.cx_id, cx)
        })
        .collect()
}

fn vector_for(positive: bool, separates: bool) -> Vec<f32> {
    if !separates || positive {
        vec![1.0, 0.0]
    } else {
        vec![0.0, 1.0]
    }
}

fn panel_two_active() -> Panel {
    Panel {
        version: 1,
        slots: vec![slot(0), slot(1)],
        created_at: 1,
        kernel_ref: None,
        guard_ref: None,
    }
}

fn slot(id: u16) -> Slot {
    let slot_id = SlotId::new(id);
    Slot {
        slot_id,
        slot_key: SlotKey::new(slot_id, format!("slot-{id}")),
        lens_id: LensId::from_bytes([id as u8; 16]),
        shape: SlotShape::Dense(2),
        modality: Modality::Text,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::None,
        resource: Default::default(),
        axis: Some("unit".to_string()),
        retrieval_only: false,
        excluded_from_dedup: false,
        bits_about: BTreeMap::new(),
        state: SlotState::Active,
        added_at_panel_version: 1,
    }
}

fn constellation(
    seed: u8,
    anchor_value: Option<AnchorValue>,
    slot0: Vec<f32>,
    slot1: Vec<f32>,
) -> calyx_core::Constellation {
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: slot0,
        },
    );
    slots.insert(
        SlotId::new(1),
        SlotVector::Dense {
            dim: 2,
            data: slot1,
        },
    );
    calyx_core::Constellation {
        cx_id: CxId::from_bytes([seed; 16]),
        vault_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse::<VaultId>().unwrap(),
        panel_version: 1,
        created_at: u64::from(seed),
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: anchor_value
            .map(|value| {
                vec![Anchor {
                    kind: AnchorKind::TestPass,
                    value,
                    source: "unit".to_string(),
                    observed_at: 1,
                    confidence: 1.0,
                }]
            })
            .unwrap_or_default(),
        provenance: LedgerRef {
            seq: u64::from(seed),
            hash: [seed; 32],
        },
        flags: CxFlags {
            ungrounded: false,
            degraded: false,
            novel_region: false,
            redacted_input: false,
        },
    }
}

fn temp_root(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "calyx-cli-intelligence-{name}-{}",
        std::process::id()
    ))
}
