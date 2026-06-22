use std::fs;
use std::path::Path;
use std::str::FromStr;

use calyx_core::SlotId;
use calyx_ward::{GuardId, NoveltyAction, SlotVerdict};
use proptest::prelude::*;
use serde::Serialize;

use super::*;
use crate::{
    CALYX_ORACLE_FLAKY_ANCHOR, CALYX_ORACLE_INSUFFICIENT, CALYX_ORACLE_NO_RECURRENCE,
    CALYX_ORACLE_SLOT_CONFLICT, OracleError,
};

#[test]
fn prediction_json_roundtrips_with_known_fields() {
    let prediction = prediction_fixture();

    let json = serde_json::to_string(&prediction).expect("serialize prediction");
    let decoded: Prediction = serde_json::from_str(&json).expect("deserialize prediction");

    assert_eq!(decoded, prediction);
    assert!(json.contains("\"I_panel_oracle\":1.05"));
}

#[test]
fn self_consistency_ceiling_matches_known_values() {
    assert_close(OracleSelfConsistency::measured(0.0, 1.0).ceiling, 1.0);
    assert_close(OracleSelfConsistency::measured(0.1, 0.8).ceiling, 0.72);
    assert_close(OracleSelfConsistency::measured(0.5, 0.5).ceiling, 0.25);
}

proptest! {
    #[test]
    fn self_consistency_ceiling_stays_in_unit_interval(
        flakiness in 0.0f32..=1.0,
        validity in 0.0f32..=1.0,
    ) {
        let consistency = OracleSelfConsistency::measured(flakiness, validity);

        prop_assert!(consistency.ceiling >= 0.0);
        prop_assert!(consistency.ceiling <= 1.0);
    }
}

#[test]
fn empty_per_sensor_deficit_still_serializes() {
    let bound = SufficiencyBound {
        i_panel_oracle: 0.0,
        dpi_ceiling: 0.0,
        sufficient: false,
        per_sensor_deficit: Vec::new(),
    };

    let json = serde_json::to_string(&bound).expect("serialize bound");
    let decoded: SufficiencyBound = serde_json::from_str(&json).expect("deserialize bound");

    assert_eq!(decoded, bound);
    assert!(json.contains("\"per_sensor_deficit\":[]"));
}

#[test]
fn completion_result_filters_measured_inferred_and_provisional_slots() {
    let all = slot_set(&[1, 2, 3, 4, 5, 6, 7]);
    let clamp = slot_set(&[1, 2, 3]);
    let free = slot_set(&[4, 5, 6, 7]);
    let result = CompletionResult::new(
        vec![
            tagged_slot(1, SlotTag::Measured),
            tagged_slot(2, SlotTag::Measured),
            tagged_slot(3, SlotTag::Measured),
            tagged_slot(4, SlotTag::Inferred),
            tagged_slot(5, SlotTag::Inferred),
            tagged_slot(6, SlotTag::Inferred),
            tagged_slot(7, SlotTag::Inferred),
        ],
        0.81,
        true,
        -1.5,
        ledger(51),
        CompletionSlotPartition::new(&all, &clamp, &free),
    )
    .expect("valid completion result");

    println!(
        "completion_counts measured={} inferred={} provisional={}",
        result.measured_slots().len(),
        result.inferred_slots().len(),
        result.provisional_slots().len()
    );
    assert_eq!(result.measured_slots().len(), 3);
    assert_eq!(result.inferred_slots().len(), 4);
    assert!(result.provisional_slots().is_empty());
}

#[test]
fn slot_tag_serde_roundtrip_is_byte_identical() {
    for (tag, expected) in [
        (SlotTag::Measured, "\"measured\""),
        (SlotTag::Inferred, "\"inferred\""),
        (SlotTag::Provisional, "\"provisional\""),
    ] {
        let encoded = serde_json::to_string(&tag).expect("serialize slot tag");
        println!("slot_tag_encoding={encoded}");
        let decoded: SlotTag = serde_json::from_str(&encoded).expect("deserialize slot tag");

        assert_eq!(encoded, expected);
        assert_eq!(decoded, tag);
    }
}

#[test]
fn completion_result_rejects_overlapping_slot_sets() {
    let all = slot_set(&[1, 2]);
    let clamp = slot_set(&[1]);
    let free = slot_set(&[1, 2]);
    let error = CompletionResult::new(
        vec![
            tagged_slot(1, SlotTag::Measured),
            tagged_slot(2, SlotTag::Inferred),
        ],
        0.5,
        true,
        -0.25,
        ledger(52),
        CompletionSlotPartition::new(&all, &clamp, &free),
    )
    .expect_err("overlap must fail closed");

    println!("slot_conflict_overlap code={} error={error}", error.code());
    assert_eq!(error.code(), CALYX_ORACLE_SLOT_CONFLICT);
    assert!(error.to_string().contains(&lens(1).to_string()));
    assert!(error.to_string().contains("remediation:"));
}

#[test]
fn completion_result_rejects_missing_slots_with_ids_listed() {
    let all = slot_set(&[1, 2]);
    let clamp = slot_set(&[1]);
    let free = SlotSet::new();
    let error = CompletionResult::new(
        vec![tagged_slot(1, SlotTag::Measured)],
        0.5,
        false,
        -0.25,
        ledger(53),
        CompletionSlotPartition::new(&all, &clamp, &free),
    )
    .expect_err("missing union slot must fail closed");

    println!("slot_conflict_missing code={} error={error}", error.code());
    assert_eq!(error.code(), CALYX_ORACLE_SLOT_CONFLICT);
    assert!(error.to_string().contains(&lens(2).to_string()));
}

#[test]
fn completion_result_edges_cover_all_clamped_all_free_and_zero_slots() {
    let all_clamped = CompletionResult::new(
        vec![
            tagged_slot(1, SlotTag::Measured),
            tagged_slot(2, SlotTag::Measured),
        ],
        1.0,
        true,
        0.0,
        ledger(54),
        CompletionSlotPartition::new(&slot_set(&[1, 2]), &slot_set(&[1, 2]), &SlotSet::new()),
    )
    .expect("all clamped allowed");
    println!(
        "edge_all_clamped before_free=0 after_inferred={}",
        all_clamped.inferred_slots().len()
    );
    assert!(all_clamped.inferred_slots().is_empty());

    let all_free = CompletionResult::new(
        vec![
            tagged_slot(1, SlotTag::Inferred),
            tagged_slot(2, SlotTag::Provisional),
        ],
        0.4,
        false,
        1.0,
        ledger(55),
        CompletionSlotPartition::new(&slot_set(&[1, 2]), &SlotSet::new(), &slot_set(&[1, 2])),
    )
    .expect("all free allowed");
    println!(
        "edge_all_free before_clamp=0 after_measured={}",
        all_free.measured_slots().len()
    );
    assert!(all_free.measured_slots().is_empty());

    let zero = CompletionResult::new(
        Vec::new(),
        0.0,
        true,
        0.0,
        ledger(56),
        CompletionSlotPartition::new(&SlotSet::new(), &SlotSet::new(), &SlotSet::new()),
    )
    .expect("zero slots allowed");
    println!("edge_zero_slots after_filled={}", zero.filled_cx.len());
    assert!(zero.filled_cx.is_empty());
}

proptest! {
    #[test]
    fn slot_tag_filters_partition_the_filled_constellation(
        tags in prop::collection::vec(0u8..3, 0..64),
    ) {
        let filled_cx: Vec<_> = tags
            .iter()
            .enumerate()
            .map(|(index, tag)| tagged_slot(index as u8, tag_from_index(*tag)))
            .collect();
        let result = CompletionResult {
            filled_cx,
            confidence: 0.5,
            converged: true,
            energy: -1.0,
            provenance: ledger(57),
        };

        prop_assert_eq!(
            result.measured_slots().len()
                + result.inferred_slots().len()
                + result.provisional_slots().len(),
            result.filled_cx.len()
        );
    }
}

#[test]
fn root_consequence_keeps_hop_zero() {
    let root = consequence(2, "root-action", 0, 0.9);

    assert_eq!(root.hop, 0);
}

#[test]
fn max_depth_zero_allows_empty_tree() {
    let tree = ConsequenceTree {
        root: consequence(3, "terminal", 0, 0.6),
        children: Vec::new(),
        max_depth: 0,
    };

    assert_eq!(tree.max_depth, 0);
    assert!(tree.children.is_empty());
}

#[test]
fn oracle_error_display_contains_codes_and_remediation() {
    let insufficient = OracleError::Insufficient {
        bound: SufficiencyBound {
            i_panel_oracle: 0.46,
            dpi_ceiling: 0.46,
            sufficient: false,
            per_sensor_deficit: Vec::new(),
        },
    };
    let flaky = OracleError::FlakyAnchor {
        self_consistency: 0.25,
    };
    let recurrence = OracleError::NoRecurrence {
        domain: DomainId::from("fixture"),
    };

    assert_display_has_code_and_remediation(&insufficient, CALYX_ORACLE_INSUFFICIENT);
    assert_display_has_code_and_remediation(&flaky, CALYX_ORACLE_FLAKY_ANCHOR);
    assert_display_has_code_and_remediation(&recurrence, CALYX_ORACLE_NO_RECURRENCE);
}

#[test]
#[ignore = "manual FSV for issue #429 Oracle contract readbacks"]
fn issue429_oracle_types_fsv_writes_readbacks() {
    let root = std::env::var_os("CALYX_ORACLE_TYPES_FSV_DIR")
        .map(std::path::PathBuf::from)
        .expect("set CALYX_ORACLE_TYPES_FSV_DIR");
    fs::create_dir_all(&root).expect("create oracle types fsv root");

    let prediction = prediction_fixture();
    write_json(&root.join("prediction.json"), &prediction);
    let decoded: Prediction =
        serde_json::from_slice(&fs::read(root.join("prediction.json")).expect("read prediction"))
            .expect("decode prediction");
    write_json(&root.join("prediction-roundtrip.json"), &decoded);

    write_json(
        &root.join("edge-empty-deficit.json"),
        &SufficiencyBound {
            i_panel_oracle: 0.0,
            dpi_ceiling: 0.0,
            sufficient: false,
            per_sensor_deficit: Vec::new(),
        },
    );
    write_json(
        &root.join("edge-hop-zero.json"),
        &consequence(2, "root-action", 0, 0.9),
    );
    write_json(
        &root.join("edge-max-depth-zero.json"),
        &ConsequenceTree {
            root: consequence(3, "terminal", 0, 0.6),
            children: Vec::new(),
            max_depth: 0,
        },
    );
    fs::write(
        root.join("oracle-error-catalog.txt"),
        [
            CALYX_ORACLE_INSUFFICIENT,
            CALYX_ORACLE_FLAKY_ANCHOR,
            CALYX_ORACLE_NO_RECURRENCE,
        ]
        .join("\n"),
    )
    .expect("write oracle catalog");
}

fn assert_display_has_code_and_remediation(error: &OracleError, code: &'static str) {
    let display = error.to_string();
    assert!(display.contains(code));
    assert!(display.contains("remediation:"));
    assert!(!error.remediation().is_empty());
}

fn prediction_fixture() -> Prediction {
    Prediction {
        outcome: AnchorValue::Bool(true),
        confidence: 0.72,
        consequences: vec![consequence(1, "compile-pass", 1, 0.5)],
        bound: SufficiencyBound {
            i_panel_oracle: 1.05,
            dpi_ceiling: 1.05,
            sufficient: true,
            per_sensor_deficit: vec![(LensId::from_bytes([7; 16]), 0.0)],
        },
        provenance: ledger(9),
        guard: guard(true),
    }
}

fn tagged_slot(seed: u8, tag: SlotTag) -> TaggedSlot {
    TaggedSlot {
        lens_id: lens(seed),
        vector: vec![f32::from(seed), 1.0],
        tag,
    }
}

fn tag_from_index(index: u8) -> SlotTag {
    match index % 3 {
        0 => SlotTag::Measured,
        1 => SlotTag::Inferred,
        _ => SlotTag::Provisional,
    }
}

fn slot_set(ids: &[u8]) -> SlotSet {
    ids.iter().map(|id| lens(*id)).collect()
}

fn lens(seed: u8) -> LensId {
    LensId::from_bytes([seed; 16])
}

fn write_json<T: Serialize>(path: &Path, value: &T) {
    let json = serde_json::to_vec_pretty(value).expect("serialize fsv json");
    fs::write(path, json).expect("write fsv json");
}

fn consequence(seed: u8, action_or_event: &str, hop: u8, confidence: f32) -> Consequence {
    Consequence {
        action_or_event: action_or_event.to_string(),
        domain: DomainId::from("fixture"),
        outcome: AnchorValue::Text(format!("outcome-{seed}")),
        confidence,
        hop,
        provenance: ledger(u64::from(seed)),
    }
}

fn guard(pass: bool) -> GuardVerdict {
    GuardVerdict {
        guard_id: GuardId::from_str("018f48a4-9a79-74d2-8a5c-9ad7f6b8c101").expect("guard id"),
        overall_pass: pass,
        provisional: false,
        per_slot: vec![SlotVerdict {
            slot: SlotId::new(1),
            cos: 0.9,
            tau: 0.7,
            pass,
        }],
        action: Some(NoveltyAction::RejectClosed),
    }
}

fn ledger(seed: u64) -> LedgerRef {
    LedgerRef {
        seq: seed,
        hash: [seed as u8; 32],
    }
}

fn assert_close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 1.0e-6);
}
