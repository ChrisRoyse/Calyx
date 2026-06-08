use std::fs;
use std::path::PathBuf;

use calyx_core::{CxId, SlotId, SlotVector};
use calyx_sextant::{
    CALYX_SEXTANT_NO_LENSES, CALYX_SEXTANT_PLAN_COST_EXCEEDED, CALYX_SEXTANT_PLAN_UNBOUNDED,
    CALYX_SEXTANT_SLOT_ALREADY_REGISTERED, CALYX_SEXTANT_SLOT_MISSING, HnswIndex, PlanLimits,
    Query, QueryPlanner, SearchEngine, SlotIndexMap,
};
use serde_json::json;

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-stage4-fail-closed"))
}

fn write_readback(name: &str, value: serde_json::Value) {
    let root = fsv_root();
    fs::create_dir_all(&root).expect("create fsv root");
    let path = root.join(name);
    fs::write(&path, serde_json::to_vec_pretty(&value).expect("json")).expect("write readback");
    println!("STAGE4_FAIL_CLOSED_READBACK={}", path.display());
}

#[test]
fn slot_map_duplicate_registration_and_empty_search_fail_closed() {
    let map = SlotIndexMap::new();
    map.register(HnswIndex::new(SlotId::new(8), 3, 42)).unwrap();
    let duplicate = map
        .register(HnswIndex::new(SlotId::new(8), 4, 43))
        .unwrap_err();
    let missing = map
        .search(SlotId::new(9), &dense_vec(1.0, 3), 1, Some(4))
        .unwrap_err();
    let no_lenses = SearchEngine::new(SlotIndexMap::new())
        .search(&Query::new("empty"))
        .unwrap_err();

    println!(
        "STAGE4_SLOT_EDGES duplicate={} missing={} no_lenses={}",
        duplicate.code, missing.code, no_lenses.code
    );
    write_readback(
        "stage4-slot-map-fail-closed.json",
        json!({
            "registered_slots": map.slots(),
            "duplicate": duplicate.code,
            "missing": missing.code,
            "no_lenses": no_lenses.code,
            "duplicate_remediation": duplicate.remediation,
            "no_lenses_remediation": no_lenses.remediation,
        }),
    );

    assert_eq!(duplicate.code, CALYX_SEXTANT_SLOT_ALREADY_REGISTERED);
    assert_eq!(missing.code, CALYX_SEXTANT_SLOT_MISSING);
    assert_eq!(no_lenses.code, CALYX_SEXTANT_NO_LENSES);
}

#[test]
fn planner_bounds_cost_and_no_lenses_fail_closed_distinctly() {
    let planner = QueryPlanner::new(PlanLimits {
        max_k: 10,
        max_ef: 20,
        max_slots: 2,
        max_cost: 50,
        timeout_ms: 7,
    });
    let mut valid = Query::new("why bounded")
        .with_slots(vec![SlotId::new(8)])
        .with_vector(dense_vec(1.0, 3));
    valid.ef = Some(5);
    let plan = planner.plan(valid.clone(), 10).unwrap();

    let mut k_zero = valid.clone();
    k_zero.k = 0;
    let k_zero = planner.plan(k_zero, 10).unwrap_err();

    let no_lenses = planner.plan(Query::new(""), 0).unwrap_err();

    let mut ef_too_large = valid.clone();
    ef_too_large.ef = Some(21);
    let ef_too_large = planner.plan(ef_too_large, 10).unwrap_err();

    let slots_too_large = planner
        .plan(
            Query::new("too many")
                .with_vector(dense_vec(1.0, 3))
                .with_slots(vec![SlotId::new(1), SlotId::new(2), SlotId::new(3)]),
            10,
        )
        .unwrap_err();

    let mut expensive = valid.clone();
    expensive.k = 10;
    expensive.ef = Some(20);
    let cost_exceeded = planner.plan(expensive, 100).unwrap_err();

    println!(
        "STAGE4_PLANNER_EDGES k_zero={} no_lenses={} ef={} slots={} cost={} valid_cost={}",
        k_zero.code,
        no_lenses.code,
        ef_too_large.code,
        slots_too_large.code,
        cost_exceeded.code,
        plan.cost_estimate
    );
    write_readback(
        "stage4-planner-fail-closed.json",
        json!({
            "valid": {
                "intent": format!("{:?}", plan.intent),
                "timeout_ms": plan.timeout_ms,
                "cost_estimate": plan.cost_estimate,
            },
            "k_zero": k_zero.code,
            "no_lenses": no_lenses.code,
            "ef_too_large": ef_too_large.code,
            "slots_too_large": slots_too_large.code,
            "cost_exceeded": cost_exceeded.code,
            "cost_remediation": cost_exceeded.remediation,
        }),
    );

    assert_eq!(plan.timeout_ms, 7);
    assert_eq!(k_zero.code, CALYX_SEXTANT_PLAN_UNBOUNDED);
    assert_eq!(no_lenses.code, CALYX_SEXTANT_NO_LENSES);
    assert_eq!(ef_too_large.code, CALYX_SEXTANT_PLAN_UNBOUNDED);
    assert_eq!(slots_too_large.code, CALYX_SEXTANT_PLAN_UNBOUNDED);
    assert_eq!(cost_exceeded.code, CALYX_SEXTANT_PLAN_COST_EXCEEDED);
}

fn dense_vec(base: f32, dim: u32) -> SlotVector {
    SlotVector::Dense {
        dim,
        data: (0..dim).map(|idx| base + idx as f32 * 0.01).collect(),
    }
}

fn _cx(value: u8) -> CxId {
    CxId::from_bytes([value; 16])
}
