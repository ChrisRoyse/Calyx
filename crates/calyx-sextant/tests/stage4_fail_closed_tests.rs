use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId,
    SlotState, SlotVector, VaultId,
};
use calyx_sextant::fusion::{
    profiles::{AP60_TEMPORAL_PRIMARY_SLOTS, is_ap60_temporal_primary_slot, lookup},
    weighted_rrf_fuse,
};
use calyx_sextant::{
    CALYX_SEXTANT_NO_LENSES, CALYX_SEXTANT_PLAN_COST_EXCEEDED, CALYX_SEXTANT_PLAN_UNBOUNDED,
    CALYX_SEXTANT_SLOT_ALREADY_REGISTERED, CALYX_SEXTANT_SLOT_INACTIVE, CALYX_SEXTANT_SLOT_MISSING,
    FusionContext, FusionStrategy, HnswIndex, IndexSearchHit, PlanLimits, Query, QueryPlanner,
    RrfProfile, SearchEngine, SlotIndexMap, weighted_profiles,
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
fn explain_provenance_tracks_attached_constellation_provenance() {
    let expected_hash = [0xab; 32];
    let (hit_hash, explain_hash, per_lens_count) = provenance_probe(expected_hash);

    println!(
        "STAGE4_EXPLAIN_PROVENANCE hit={} explain={} per_lens={}",
        hit_hash, explain_hash, per_lens_count
    );
    write_readback(
        "stage4-explain-provenance-readback.json",
        json!({
            "hit_provenance_hex": hit_hash,
            "explain_provenance_hex": explain_hash,
            "expected_provenance_hex": hex32(&expected_hash),
            "per_lens_count": per_lens_count,
        }),
    );

    assert_eq!(hit_hash, hex32(&expected_hash));
    assert_eq!(explain_hash, hex32(&expected_hash));
    assert_eq!(per_lens_count, 1);
}

#[test]
fn weighted_rrf_temporal_profiles_exclude_ap60_slots_and_skip_unlisted_slots() {
    let temporal = lookup(RrfProfile::Temporal).expect("temporal profile");
    let temporal_slots: Vec<_> = temporal.weights.keys().map(|slot| slot.get()).collect();
    let profiles_temporal_free = weighted_profiles().iter().all(|profile| {
        AP60_TEMPORAL_PRIMARY_SLOTS
            .iter()
            .all(|slot| !profile.weights.contains_key(slot))
    });
    let strict = weighted_rrf_strict_probe();

    println!(
        "STAGE4_TEMPORAL_GUARD slots={:?} temporal_free={} strict_hit={}",
        temporal_slots, profiles_temporal_free, strict
    );
    write_readback(
        "stage4-temporal-weighted-rrf-readback.json",
        json!({
            "temporal_profile_slots": temporal_slots,
            "ap60_temporal_primary_slots": AP60_TEMPORAL_PRIMARY_SLOTS
                .iter()
                .map(|slot| slot.get())
                .collect::<Vec<_>>(),
            "profiles_temporal_free": profiles_temporal_free,
            "slot_20_is_temporal_primary": is_ap60_temporal_primary_slot(SlotId::new(20)),
            "weighted_rrf_strict_hit": strict.to_string(),
        }),
    );

    assert_eq!(temporal.weights.len(), 1);
    assert!(temporal.weights.contains_key(&SlotId::new(8)));
    assert!(profiles_temporal_free);
    assert_eq!(strict, _cx(8));
}

#[test]
#[ignore = "manual FSV writes source-of-truth artifacts"]
fn stage4_provenance_temporal_guard_fsv() {
    let expected_hash = [0xab; 32];
    let (hit_hash, explain_hash, per_lens_count) = provenance_probe(expected_hash);
    let temporal = lookup(RrfProfile::Temporal).expect("temporal profile");
    let temporal_slots: Vec<_> = temporal.weights.keys().map(|slot| slot.get()).collect();
    let profiles_temporal_free = weighted_profiles().iter().all(|profile| {
        AP60_TEMPORAL_PRIMARY_SLOTS
            .iter()
            .all(|slot| !profile.weights.contains_key(slot))
    });
    let strict = weighted_rrf_strict_probe();

    write_readback(
        "stage4-provenance-temporal-guard-fsv.json",
        json!({
            "source_of_truth": "stored Sextant hit/explain readback JSON after search and fusion",
            "hit_provenance_hex": hit_hash,
            "explain_provenance_hex": explain_hash,
            "expected_provenance_hex": hex32(&expected_hash),
            "explain_matches_hit": hit_hash == explain_hash,
            "per_lens_count": per_lens_count,
            "temporal_profile_slots": temporal_slots,
            "ap60_temporal_primary_slots": AP60_TEMPORAL_PRIMARY_SLOTS
                .iter()
                .map(|slot| slot.get())
                .collect::<Vec<_>>(),
            "profiles_temporal_free": profiles_temporal_free,
            "weighted_rrf_strict_hit": strict.to_string(),
            "weighted_rrf_skipped_slot_20": strict == _cx(8),
        }),
    );

    assert_eq!(hit_hash, explain_hash);
    assert_eq!(hit_hash, hex32(&expected_hash));
    assert_eq!(per_lens_count, 1);
    assert_eq!(temporal_slots, [8]);
    assert!(profiles_temporal_free);
    assert_eq!(strict, _cx(8));
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
fn inactive_slots_are_excluded_and_fail_closed_when_explicit() {
    let map = SlotIndexMap::new();
    map.register(HnswIndex::new(SlotId::new(8), 3, 42)).unwrap();
    let cx_id = _cx(0x44);
    map.insert(SlotId::new(8), cx_id, dense_vec(1.0, 3), 1)
        .unwrap();

    map.set_slot_state(SlotId::new(8), SlotState::Parked)
        .unwrap();
    let direct = map
        .search(SlotId::new(8), &dense_vec(1.0, 3), 1, Some(4))
        .unwrap_err();
    let default_search = SearchEngine::new(map.clone())
        .search(&Query::new("parked slot").with_vector(dense_vec(1.0, 3)))
        .unwrap_err();
    let mut explicit = Query::new("parked slot")
        .with_vector(dense_vec(1.0, 3))
        .with_slots(vec![SlotId::new(8)]);
    explicit.k = 1;
    explicit.ef = Some(4);
    let explicit_search = SearchEngine::new(map.clone())
        .search(&explicit)
        .unwrap_err();

    println!(
        "STAGE4_SLOT_INACTIVE direct={} default={} explicit={}",
        direct.code, default_search.code, explicit_search.code
    );
    write_readback(
        "stage4-slot-inactive-readback.json",
        json!({
            "registered_slots": map.registered_slots(),
            "active_slots": map.slots(),
            "slot_state": format!("{:?}", map.slot_state(SlotId::new(8)).unwrap()),
            "direct_search": direct.code,
            "default_search": default_search.code,
            "explicit_search": explicit_search.code,
        }),
    );

    assert_eq!(direct.code, CALYX_SEXTANT_SLOT_INACTIVE);
    assert_eq!(default_search.code, CALYX_SEXTANT_NO_LENSES);
    assert_eq!(explicit_search.code, CALYX_SEXTANT_SLOT_INACTIVE);
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

fn provenance_probe(expected_hash: [u8; 32]) -> (String, String, usize) {
    let map = SlotIndexMap::new();
    map.register(HnswIndex::new(SlotId::new(8), 3, 42)).unwrap();
    let cx_id = _cx(0xa8);
    map.insert(SlotId::new(8), cx_id, dense_vec(1.0, 3), 77)
        .unwrap();
    let mut engine = SearchEngine::new(map);
    engine.put_constellation(sample_constellation(cx_id, 77, expected_hash));

    let mut query = Query::new("explain provenance")
        .with_vector(dense_vec(1.0, 3))
        .with_slots(vec![SlotId::new(8)])
        .explain(true);
    query.k = 1;
    query.ef = Some(4);
    let hits = engine.search(&query).unwrap();
    let hit = hits.first().expect("search hit");
    let explain = hit.explain.as_ref().expect("explain");
    (
        hex32(&hit.provenance.hash),
        explain.provenance_hex.clone(),
        explain.per_lens_count,
    )
}

fn weighted_rrf_strict_probe() -> CxId {
    let mut results = BTreeMap::new();
    results.insert(
        SlotId::new(8),
        vec![IndexSearchHit {
            cx_id: _cx(8),
            score: 1.0,
            rank: 1,
        }],
    );
    results.insert(
        SlotId::new(20),
        vec![IndexSearchHit {
            cx_id: _cx(20),
            score: 100.0,
            rank: 1,
        }],
    );
    let mut weights = BTreeMap::new();
    weights.insert(SlotId::new(8), 1.0);
    let hits = weighted_rrf_fuse(
        &results,
        &FusionContext {
            k: 10,
            explain: true,
            strategy: FusionStrategy::WeightedRrf {
                profile: RrfProfile::Semantic,
            },
            weights,
            stage1_slots: Vec::new(),
        },
    );
    assert_eq!(hits.len(), 1);
    hits[0].cx_id
}

fn sample_constellation(
    cx_id: CxId,
    seq: u64,
    provenance_hash: [u8; 32],
) -> calyx_core::Constellation {
    calyx_core::Constellation {
        cx_id,
        vault_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse::<VaultId>().unwrap(),
        panel_version: 1,
        created_at: seq,
        input_ref: InputRef {
            hash: [seq as u8; 32],
            pointer: Some(format!("zfs://calyx/stage4/{cx_id}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: vec![Anchor {
            kind: AnchorKind::Label("stage4".to_string()),
            value: AnchorValue::Text("ok".to_string()),
            source: "stage4-provenance-fsv".to_string(),
            observed_at: seq,
            confidence: 1.0,
        }],
        provenance: LedgerRef {
            seq,
            hash: provenance_hash,
        },
        flags: CxFlags::default(),
    }
}

fn hex32(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
