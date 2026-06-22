use calyx_assay::{
    CALYX_ASSAY_INVALID_COVERAGE, CALYX_ASSAY_INVALID_SCOPE, CoverageMask, DeficitRoutingContext,
    ObservationScope, SufficiencyScopeInput, TrustTag, bits_report, panel_sufficiency_by_scope,
    per_sensor_attribution, per_sensor_attribution_with_coverage,
};
use calyx_core::{AnchorKind, CxId, SlotId};
use serde_json::json;

#[test]
fn coverage_mask_gates_slot_bits_by_constellation() {
    let cx_a = cx(0xA1);
    let cx_b = cx(0xB2);
    let cx_c = cx(0xC3);
    let covered_value_slot = CoverageMask::partial(3, [cx_a, cx_c]).unwrap();
    let attributions = per_sensor_attribution_with_coverage(
        &[
            (slot(1), 0.10, CoverageMask::Full),
            (slot(2), 0.40, covered_value_slot),
        ],
        0.10,
    );
    let report = bits_report(attributions, TrustTag::Trusted);

    assert_close(report.total_bits_for(cx_a), 0.50);
    assert_close(report.total_bits_for(cx_b), 0.10);
    assert_close(report.total_bits_for(cx_c), 0.50);
    assert_eq!(report.observed_slots_for(cx_b), vec![slot(1)]);
    assert_eq!(report.slots[1].coverage.observed_count(), Some(2));
    assert_close(report.slots[1].coverage_rate(), 2.0 / 3.0);
}

#[test]
fn sufficiency_report_exposes_observation_scope_as_a_lever() {
    let slots = per_sensor_attribution(&[(slot(1), 0.05), (slot(2), 0.42)], 0.10);
    let narrow = input("narrow_fail_to_pass", 0.0, 1.0, 2, 6, slots.clone());
    let broad = input("broad_pass_and_fail", 1.05, 1.0, 6, 6, slots);
    let report = panel_sufficiency_by_scope(vec![narrow, broad]).unwrap();

    assert_eq!(
        report.best_scope.as_ref().map(|scope| scope.id.as_str()),
        Some("broad_pass_and_fail")
    );
    assert_eq!(report.sufficient_scopes.len(), 1);
    assert_eq!(report.sufficient_scopes[0].id, "broad_pass_and_fail");
    assert!(!report.scopes[0].sufficient);
    assert!(report.scopes[1].sufficient);
    assert_eq!(
        report.scopes[0].deficits[0]
            .observation_scope
            .as_ref()
            .map(|scope| scope.id.as_str()),
        Some("narrow_fail_to_pass")
    );
}

#[test]
fn invalid_coverage_and_duplicate_scopes_fail_closed() {
    let coverage_error = CoverageMask::partial(1, [cx(0x01), cx(0x02)]).unwrap_err();
    assert_eq!(coverage_error.code, CALYX_ASSAY_INVALID_COVERAGE);

    let slots = per_sensor_attribution(&[(slot(1), 0.10)], 0.10);
    let duplicate_error = panel_sufficiency_by_scope(vec![
        input("duplicate", 0.10, 1.0, 1, 2, slots.clone()),
        input("duplicate", 0.20, 1.0, 2, 2, slots),
    ])
    .unwrap_err();
    assert_eq!(duplicate_error.code, CALYX_ASSAY_INVALID_SCOPE);
}

#[test]
#[ignore = "manual FSV writes source-of-truth artifacts"]
fn coverage_scope_manual_fsv() {
    let root =
        std::env::var("CALYX_ISSUE773_FSV_ROOT").expect("CALYX_ISSUE773_FSV_ROOT is required");
    std::fs::create_dir_all(&root).expect("create fsv root");
    let cx_a = cx(0xA1);
    let cx_b = cx(0xB2);
    let covered_value_slot = CoverageMask::partial(3, [cx_a, cx(0xC3)]).unwrap();
    let attributions = per_sensor_attribution_with_coverage(
        &[
            (slot(1), 0.10, CoverageMask::Full),
            (slot(2), 0.40, covered_value_slot),
        ],
        0.10,
    );
    let bits = bits_report(attributions.clone(), TrustTag::Trusted);
    let narrow = input(
        "narrow_fail_to_pass",
        0.0,
        1.0,
        2,
        6,
        per_sensor_attribution(&[(slot(1), 0.05), (slot(2), 0.42)], 0.10),
    );
    let broad = input(
        "broad_pass_and_fail",
        1.05,
        1.0,
        6,
        6,
        per_sensor_attribution(&[(slot(1), 0.05), (slot(2), 0.42)], 0.10),
    );
    let scoped = panel_sufficiency_by_scope(vec![narrow, broad]).unwrap();
    let ward_config = calyx_ward::RequiredSlotDerivation::assay_bits(AnchorKind::Reward);
    let ward_covered = calyx_ward::derive_required_slots_for_observations(
        &observations_for(&attributions, cx_a),
        &ward_config,
    )
    .unwrap();
    let ward_uncovered = calyx_ward::derive_required_slots_for_observations(
        &observations_for(&attributions, cx_b),
        &ward_config,
    )
    .unwrap();
    let invalid_coverage_code = CoverageMask::partial(1, [cx(0x01), cx(0x02)])
        .unwrap_err()
        .code;
    let duplicate_scope_code = panel_sufficiency_by_scope(vec![
        input(
            "duplicate_scope_edge",
            0.10,
            1.0,
            1,
            2,
            per_sensor_attribution(&[(slot(1), 0.10)], 0.10),
        ),
        input(
            "duplicate_scope_edge",
            0.20,
            1.0,
            2,
            2,
            per_sensor_attribution(&[(slot(1), 0.10)], 0.10),
        ),
    ])
    .unwrap_err()
    .code;
    let readback = json!({
        "source_of_truth": "issue773 coverage/scope JSON bytes written by calyx-assay FSV test in a manual verification run",
        "coverage_input": {
            "cx_a": cx_a.to_string(),
            "cx_b": cx_b.to_string(),
            "slot_1": {"bits": 0.10, "coverage": "full"},
            "slot_2": {"bits": 0.40, "coverage": [cx_a.to_string(), cx(0xC3).to_string()]},
        },
        "coverage_expected": {
            "cx_a_total_bits": 0.50,
            "cx_b_total_bits": 0.10,
            "cx_b_observed_slots": [1],
        },
        "coverage_actual": {
            "cx_a_total_bits": bits.total_bits_for(cx_a),
            "cx_b_total_bits": bits.total_bits_for(cx_b),
            "cx_b_observed_slots": slot_values(bits.observed_slots_for(cx_b)),
        },
        "scope_expected": {
            "best_scope": "broad_pass_and_fail",
            "sufficient_scopes": ["broad_pass_and_fail"],
        },
        "scope_actual": {
            "best_scope": scoped.best_scope.as_ref().map(|scope| scope.id.clone()),
            "sufficient_scopes": scoped.sufficient_scopes.iter().map(|scope| scope.id.clone()).collect::<Vec<_>>(),
            "narrow_deficit_scope": scoped.scopes[0].deficits[0].observation_scope.as_ref().map(|scope| scope.id.clone()),
        },
        "ward_expected": {
            "covered_required_slots": [1, 2],
            "uncovered_required_slots": [1],
        },
        "ward_actual": {
            "covered_required_slots": slot_values(ward_covered.iter().map(|entry| entry.slot).collect()),
            "uncovered_required_slots": slot_values(ward_uncovered.iter().map(|entry| entry.slot).collect()),
        },
        "edge_expected": {
            "invalid_coverage_code": CALYX_ASSAY_INVALID_COVERAGE,
            "duplicate_scope_code": CALYX_ASSAY_INVALID_SCOPE,
        },
        "edge_actual": {
            "invalid_coverage_code": invalid_coverage_code,
            "duplicate_scope_code": duplicate_scope_code,
        },
        "edge_cases": [
            {
                "name": "unobserved_constellation_masks_partial_slot",
                "before": {
                    "constellation": cx_b.to_string(),
                    "slot_1": {"bits": 0.10, "coverage": "full"},
                    "slot_2": {"bits": 0.40, "coverage": [cx_a.to_string(), cx(0xC3).to_string()]},
                },
                "expected_after": {"total_bits": 0.10, "observed_slots": [1]},
                "after": {
                    "total_bits": bits.total_bits_for(cx_b),
                    "observed_slots": slot_values(bits.observed_slots_for(cx_b)),
                },
            },
            {
                "name": "invalid_coverage_fails_closed",
                "before": {"total": 1, "observed_count": 2},
                "expected_after": {"error_code": CALYX_ASSAY_INVALID_COVERAGE},
                "after": {"error_code": invalid_coverage_code},
            },
            {
                "name": "duplicate_scope_id_fails_closed",
                "before": {"scope_ids": ["duplicate_scope_edge", "duplicate_scope_edge"]},
                "expected_after": {"error_code": CALYX_ASSAY_INVALID_SCOPE},
                "after": {"error_code": duplicate_scope_code},
            }
        ]
    });
    assert_close(bits.total_bits_for(cx_a), 0.50);
    assert_close(bits.total_bits_for(cx_b), 0.10);
    assert_eq!(bits.observed_slots_for(cx_b), vec![slot(1)]);
    assert_eq!(
        readback["scope_actual"]["best_scope"],
        "broad_pass_and_fail"
    );
    assert_eq!(
        readback["scope_actual"]["sufficient_scopes"],
        json!(["broad_pass_and_fail"])
    );
    assert_eq!(readback["ward_actual"], readback["ward_expected"]);
    assert_eq!(readback["edge_actual"], readback["edge_expected"]);
    let path = std::path::Path::new(&root).join("issue773-coverage-scope-readback.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();
    println!("ISSUE773_COVERAGE_SCOPE_READBACK={}", path.display());
}

fn input(
    scope_id: &str,
    panel_bits: f32,
    anchor_entropy_bits: f32,
    observed: usize,
    total: usize,
    slots: Vec<calyx_assay::SlotAttribution>,
) -> SufficiencyScopeInput {
    SufficiencyScopeInput {
        scope: ObservationScope::new(scope_id, observed, total).unwrap(),
        panel_bits,
        anchor_entropy_bits,
        slots,
        trust: TrustTag::Trusted,
        context: DeficitRoutingContext {
            panel_id: "panel:coverage-unit".to_string(),
            anchor: AnchorKind::Label("passfail".to_string()),
            computed_at_seq: 44,
            observation_scope: None,
        },
    }
}

fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}

fn cx(byte: u8) -> CxId {
    CxId::from_bytes([byte; 16])
}

fn observations_for(
    attributions: &[calyx_assay::SlotAttribution],
    cx: CxId,
) -> Vec<calyx_ward::RequiredSlotObservation> {
    attributions
        .iter()
        .map(|attribution| calyx_ward::RequiredSlotObservation {
            slot: attribution.slot,
            bits: attribution.marginal_bits,
            observed: attribution.is_observed_for(cx),
        })
        .collect()
}

fn slot_values(slots: Vec<SlotId>) -> Vec<u16> {
    slots.into_iter().map(SlotId::get).collect()
}

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= 1.0e-6,
        "actual={actual} expected={expected}"
    );
}
