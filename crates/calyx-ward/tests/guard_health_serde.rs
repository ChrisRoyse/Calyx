use calyx_core::SlotId;
use calyx_ward::{GuardHealth, GuardId};
use serde_json::{Value, json};

const GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101";

#[test]
fn guard_health_deserializes_pre_per_slot_far_bound_json() {
    let health = legacy_health();

    assert_eq!(health.guard_id, guard_id());
    assert_eq!(health.per_slot_rejection_rate.get(&slot(1)), Some(&0.02));
    assert!(health.per_slot_calibrated_far_bound.is_empty());
    assert_eq!(health.per_slot_frr.get(&slot(1)), Some(&0.10));
    assert!(health.drift);
    assert_eq!(health.last_calibrated, 1_786_233_600);
    assert_eq!(health.dropped_events, 0);
}

#[test]
#[ignore = "manual FSV fixture; set CALYX_WARD_GUARD_HEALTH_SERDE_FSV_DIR"]
fn guard_health_serde_fsv_fixture_writes_readback_artifacts() {
    let root = std::env::var("CALYX_WARD_GUARD_HEALTH_SERDE_FSV_DIR")
        .expect("CALYX_WARD_GUARD_HEALTH_SERDE_FSV_DIR is required");
    std::fs::create_dir_all(&root).expect("create fsv root");
    let legacy = legacy_health_json();
    let health: GuardHealth =
        serde_json::from_value(legacy.clone()).expect("deserialize legacy health");
    let readback = serde_json::to_value(&health).expect("serialize readback health");

    write_json(&root, "legacy-health.json", &legacy);
    write_json(&root, "readback-health.json", &readback);
    write_json(
        &root,
        "case-summary.json",
        &json!({
            "legacy_has_per_slot_calibrated_far_bound": legacy
                .get("per_slot_calibrated_far_bound")
                .is_some(),
            "deserialized_bound_count": health.per_slot_calibrated_far_bound.len(),
            "readback_has_per_slot_calibrated_far_bound": readback
                .get("per_slot_calibrated_far_bound")
                .is_some(),
            "slot1_rejection_rate": health.per_slot_rejection_rate.get(&slot(1)),
            "slot1_frr": health.per_slot_frr.get(&slot(1)),
            "drift": health.drift,
            "last_calibrated": health.last_calibrated,
        }),
    );

    println!(
        "FSV_GUARD_HEALTH_SERDE legacy_has_bound={} deserialized_bound_count={} readback_has_bound={} slot1_rejection_rate={:?} slot1_frr={:?}",
        legacy.get("per_slot_calibrated_far_bound").is_some(),
        health.per_slot_calibrated_far_bound.len(),
        readback.get("per_slot_calibrated_far_bound").is_some(),
        health.per_slot_rejection_rate.get(&slot(1)),
        health.per_slot_frr.get(&slot(1)),
    );
}

fn legacy_health() -> GuardHealth {
    serde_json::from_value(legacy_health_json()).expect("deserialize legacy health")
}

fn legacy_health_json() -> Value {
    json!({
        "guard_id": GUARD_UUID,
        "per_slot_rejection_rate": {
            "1": 0.02
        },
        "per_slot_frr": {
            "1": 0.10
        },
        "drift": true,
        "last_calibrated": 1_786_233_600,
        "dropped_events": 0
    })
}

fn write_json<T: serde::Serialize>(root: &str, name: &str, value: &T) {
    let path = std::path::Path::new(root).join(name);
    let file = std::fs::File::create(path).expect("create fsv json");
    serde_json::to_writer_pretty(file, value).expect("write fsv json");
}

fn guard_id() -> GuardId {
    GUARD_UUID.parse().expect("guard id")
}

const fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}
