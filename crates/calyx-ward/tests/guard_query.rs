use std::collections::BTreeMap;

use calyx_core::{CxId, SlotId};
use calyx_ward::{
    GuardId, GuardPolicy, GuardProfile, KernelFirstQueryVerdict, NoveltyAction, ProducedSlots,
    QueryVerdict, RegionSource, TrustedRegion, WardError, guard_query, guard_query_kernel_first,
};
use serde_json::json;

const GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101";

#[test]
fn in_region_query_passes_with_nearest_cx() {
    let profile = sample_profile();
    let trusted = trusted_regions();
    let query = slot_vectors(&[(slot(1), cos_vector(0.90)), (slot(2), cos_vector(0.85))]);

    let verdict = guard_query(&profile, &query, &trusted).expect("guard query");

    match verdict {
        QueryVerdict::Pass {
            nearest_cx,
            gap,
            per_slot,
        } => {
            assert_eq!(nearest_cx, cx(1));
            assert_eq!(gap, 0.0);
            assert_eq!(per_slot.len(), 2);
            assert!(per_slot.iter().all(|slot| slot.pass));
        }
        QueryVerdict::Ood { .. } => panic!("expected pass"),
    }
}

#[test]
fn ood_query_returns_nearest_gap() {
    let profile = sample_profile();
    let trusted = trusted_regions();
    let query = slot_vectors(&[(slot(1), cos_vector(0.60)), (slot(2), cos_vector(0.62))]);

    let verdict = guard_query(&profile, &query, &trusted).expect("guard query");

    match verdict {
        QueryVerdict::Ood {
            nearest_cx,
            gap,
            per_slot,
            action,
        } => {
            assert_eq!(nearest_cx, Some(cx(1)));
            assert_close(gap.unwrap(), 0.10);
            assert_eq!(action, NoveltyAction::Quarantine);
            assert_eq!(per_slot.len(), 2);
            assert!(per_slot.iter().all(|slot| !slot.pass));
        }
        QueryVerdict::Pass { .. } => panic!("expected ood"),
    }
}

#[test]
fn average_passing_query_still_ood_when_one_slot_fails() {
    let profile = sample_profile();
    let trusted = trusted_regions();
    let query = slot_vectors(&[(slot(1), cos_vector(0.95)), (slot(2), cos_vector(0.45))]);

    let verdict = guard_query(&profile, &query, &trusted).expect("guard query");

    match verdict {
        QueryVerdict::Ood {
            nearest_cx,
            gap,
            per_slot,
            ..
        } => {
            assert_eq!(nearest_cx, Some(cx(1)));
            assert_close(gap.unwrap(), 0.25);
            assert!(
                per_slot
                    .iter()
                    .any(|detail| detail.slot == slot(1) && detail.pass)
            );
            assert!(
                per_slot
                    .iter()
                    .any(|detail| detail.slot == slot(2) && !detail.pass)
            );
        }
        QueryVerdict::Pass { .. } => panic!("average-flattened query must not pass"),
    }
}

#[test]
fn no_trusted_regions_returns_ood_without_nearest() {
    let profile = sample_profile();
    let query = slot_vectors(&[(slot(1), cos_vector(0.90)), (slot(2), cos_vector(0.85))]);

    let verdict = guard_query(&profile, &query, &[]).expect("guard query");

    assert_eq!(
        verdict,
        QueryVerdict::Ood {
            nearest_cx: None,
            gap: None,
            per_slot: Vec::new(),
            action: NoveltyAction::Quarantine,
        }
    );
}

#[test]
fn missing_query_slot_fails_closed() {
    let profile = sample_profile();
    let trusted = trusted_regions();
    let query = slot_vectors(&[(slot(1), cos_vector(0.90))]);

    let error = guard_query(&profile, &query, &trusted).expect_err("missing slot");

    assert_eq!(error, WardError::MissingSlot { slot: slot(2) });
}

#[test]
fn shape_mismatch_returns_ood_gap_without_panic() {
    let profile = sample_profile();
    let trusted = trusted_regions();
    let query = slot_vectors(&[(slot(1), vec![1.0]), (slot(2), cos_vector(0.90))]);

    let verdict = guard_query(&profile, &query, &trusted).expect("guard query");

    match verdict {
        QueryVerdict::Ood { gap, per_slot, .. } => {
            assert_close(gap.unwrap(), 0.70);
            assert!(
                per_slot
                    .iter()
                    .any(|detail| detail.slot == slot(1) && !detail.pass)
            );
        }
        QueryVerdict::Pass { .. } => panic!("shape mismatch must not pass"),
    }
}

#[test]
fn query_source_has_no_aggregate_vector_gate_markers() {
    let source = read_query_source();
    let markers: Vec<_> = aggregate_markers()
        .into_iter()
        .filter(|marker| source.contains(marker))
        .collect();

    assert!(markers.is_empty(), "aggregate markers found: {markers:?}");
}

#[test]
fn kernel_near_pass_wins_over_better_peripheral_pass() {
    let profile = sample_profile();
    let query = unit_query();
    let kernel = vec![region(cx(10), 0.75, 0.75)];
    let peripheral = vec![region(cx(20), 0.95, 0.95)];

    let verdict =
        guard_query_kernel_first(&profile, &query, &kernel, &peripheral).expect("kernel first");

    match verdict {
        KernelFirstQueryVerdict::Pass {
            nearest_cx,
            match_source,
            per_slot,
            ..
        } => {
            assert_eq!(nearest_cx, cx(10));
            assert_eq!(match_source, RegionSource::KernelNear);
            assert!(per_slot.iter().all(|slot| slot.pass));
        }
        KernelFirstQueryVerdict::Ood { .. } => panic!("expected kernel pass"),
    }
}

#[test]
fn peripheral_region_is_used_after_kernel_candidates_fail() {
    let profile = sample_profile();
    let query = unit_query();
    let kernel = vec![region(cx(10), 0.95, 0.55)];
    let peripheral = vec![region(cx(20), 0.90, 0.90)];

    let verdict =
        guard_query_kernel_first(&profile, &query, &kernel, &peripheral).expect("fallback");

    match verdict {
        KernelFirstQueryVerdict::Pass {
            nearest_cx,
            match_source,
            ..
        } => {
            assert_eq!(nearest_cx, cx(20));
            assert_eq!(match_source, RegionSource::Peripheral);
        }
        KernelFirstQueryVerdict::Ood { .. } => panic!("expected peripheral pass"),
    }
}

#[test]
fn ood_verdict_records_nearest_region_source() {
    let profile = sample_profile();
    let query = unit_query();
    let kernel = vec![region(cx(10), 0.66, 0.66)];
    let peripheral = vec![region(cx(20), 0.40, 0.40)];

    let verdict = guard_query_kernel_first(&profile, &query, &kernel, &peripheral).expect("ood");

    match verdict {
        KernelFirstQueryVerdict::Ood {
            nearest_cx,
            match_source,
            gap,
            ..
        } => {
            assert_eq!(nearest_cx, Some(cx(10)));
            assert_eq!(match_source, Some(RegionSource::KernelNear));
            assert_close(gap.unwrap(), 0.04);
        }
        KernelFirstQueryVerdict::Pass { .. } => panic!("expected ood"),
    }
}

#[test]
#[ignore = "manual FSV fixture; set CALYX_WARD_QUERY_FSV_DIR"]
fn guard_query_fsv_fixture_writes_readback_artifacts() {
    let root =
        std::env::var("CALYX_WARD_QUERY_FSV_DIR").expect("CALYX_WARD_QUERY_FSV_DIR is required");
    std::fs::create_dir_all(&root).expect("create fsv root");

    let profile = sample_profile();
    let trusted = trusted_regions();
    let pass_query = slot_vectors(&[(slot(1), cos_vector(0.90)), (slot(2), cos_vector(0.85))]);
    let ood_query = slot_vectors(&[(slot(1), cos_vector(0.60)), (slot(2), cos_vector(0.62))]);
    let average_attack = slot_vectors(&[(slot(1), cos_vector(0.95)), (slot(2), cos_vector(0.45))]);
    let missing_query = slot_vectors(&[(slot(1), cos_vector(0.90))]);
    let pass = guard_query(&profile, &pass_query, &trusted).expect("pass query");
    let ood = guard_query(&profile, &ood_query, &trusted).expect("ood query");
    let average = guard_query(&profile, &average_attack, &trusted).expect("average attack");
    let no_regions = guard_query(&profile, &pass_query, &[]).expect("no trusted regions");
    let missing = guard_query(&profile, &missing_query, &trusted).expect_err("missing query slot");
    let kernel_first = guard_query_kernel_first(
        &profile,
        &unit_query(),
        &[region(cx(10), 0.75, 0.75)],
        &[region(cx(20), 0.95, 0.95)],
    )
    .expect("kernel first");
    let peripheral_fallback = guard_query_kernel_first(
        &profile,
        &unit_query(),
        &[region(cx(10), 0.95, 0.55)],
        &[region(cx(20), 0.90, 0.90)],
    )
    .expect("peripheral fallback");
    let kernel_ood = guard_query_kernel_first(
        &profile,
        &unit_query(),
        &[region(cx(10), 0.66, 0.66)],
        &[region(cx(20), 0.40, 0.40)],
    )
    .expect("kernel ood");
    let source = read_query_source();
    let markers: Vec<_> = aggregate_markers()
        .into_iter()
        .filter(|marker| source.contains(marker))
        .collect();

    write_json(&root, "query-pass.json", &pass);
    write_json(&root, "query-ood.json", &ood);
    write_json(&root, "query-average-attack.json", &average);
    write_json(&root, "query-no-regions.json", &no_regions);
    write_json(&root, "missing-slot-error.json", &error_json(&missing));
    write_json(&root, "query-kernel-first.json", &kernel_first);
    write_json(
        &root,
        "query-peripheral-fallback.json",
        &peripheral_fallback,
    );
    write_json(&root, "query-kernel-ood.json", &kernel_ood);
    write_json(
        &root,
        "source-readback.json",
        &json!({
            "line_count": source.lines().count(),
            "aggregate_vector_gate_markers": markers,
        }),
    );

    println!(
        "FSV_GUARD_QUERY pass={} ood={} average_attack={} no_regions={} missing_code={} kernel_first={} fallback={}",
        pass.is_pass(),
        !ood.is_pass(),
        !average.is_pass(),
        !no_regions.is_pass(),
        missing.code(),
        kernel_first.is_pass(),
        peripheral_fallback.is_pass()
    );
}

fn sample_profile() -> GuardProfile {
    let mut tau = BTreeMap::new();
    tau.insert(slot(1), 0.70);
    tau.insert(slot(2), 0.70);
    GuardProfile {
        guard_id: guard_id(),
        panel_version: 42,
        domain: "synthetic-query".to_string(),
        tau,
        required_slots: vec![slot(1), slot(2)],
        policy: GuardPolicy::AllRequired,
        calibration: None,
        novelty_action: NoveltyAction::Quarantine,
    }
}

fn trusted_regions() -> Vec<TrustedRegion> {
    vec![
        TrustedRegion {
            cx_id: cx(1),
            slots: slot_vectors(&[(slot(1), vec![1.0, 0.0]), (slot(2), vec![1.0, 0.0])]),
        },
        TrustedRegion {
            cx_id: cx(2),
            slots: slot_vectors(&[(slot(1), vec![-1.0, 0.0]), (slot(2), vec![-1.0, 0.0])]),
        },
    ]
}

fn unit_query() -> ProducedSlots {
    slot_vectors(&[(slot(1), vec![1.0, 0.0]), (slot(2), vec![1.0, 0.0])])
}

fn region(cx_id: CxId, slot1_cos: f32, slot2_cos: f32) -> TrustedRegion {
    TrustedRegion {
        cx_id,
        slots: slot_vectors(&[
            (slot(1), cos_vector(slot1_cos)),
            (slot(2), cos_vector(slot2_cos)),
        ]),
    }
}

fn aggregate_markers() -> Vec<&'static str> {
    vec![
        "flatten",
        "concat",
        "extend_from_slice",
        ".append(",
        ".extend(",
        ".chain(",
        "flat_map",
        "collect::<Vec",
    ]
}

fn read_query_source() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("query.rs");
    std::fs::read_to_string(path).expect("read query.rs")
}

fn slot_vectors(entries: &[(SlotId, Vec<f32>)]) -> ProducedSlots {
    entries.iter().cloned().collect()
}

fn cos_vector(cos: f32) -> Vec<f32> {
    vec![cos, (1.0 - cos * cos).sqrt()]
}

fn error_json(error: &WardError) -> serde_json::Value {
    json!({
        "code": error.code(),
        "message": error.to_string(),
    })
}

fn write_json<T: serde::Serialize>(root: &str, name: &str, value: &T) {
    let path = std::path::Path::new(root).join(name);
    let file = std::fs::File::create(path).expect("create fsv json");
    serde_json::to_writer_pretty(file, value).expect("write fsv json");
}

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= 1.0e-5,
        "actual={actual} expected={expected}"
    );
}

fn guard_id() -> GuardId {
    GUARD_UUID.parse().expect("guard id")
}

fn cx(value: u8) -> CxId {
    CxId::from_bytes([value; 16])
}

const fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}
