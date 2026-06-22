use std::collections::BTreeMap;
use std::fs;

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId,
    SlotVector, VaultId,
};
use calyx_sextant::{HnswIndex, Query, QueryPlanner, RrfProfile, SearchEngine, SlotIndexMap};
use serde_json::json;
use sextant_support::cx_u8_fill as cx;

#[test]
fn planned_explain_search_carries_plan_and_hit_explain() {
    let engine = sample_engine();
    let planner = QueryPlanner::default();
    let explain = engine
        .planned_explain_search(causal_query().explain(false), &planner)
        .unwrap();

    assert_eq!(format!("{:?}", explain.intent), "Causal");
    assert_eq!(explain.strategy.name(), "weighted_rrf:causal");
    assert!(!explain.override_used);
    assert!(explain.cost_estimate > 0);
    assert_eq!(explain.timeout_ms, 5_000);
    assert!(!explain.hits.is_empty());
    let hit_explain = explain.hits[0].explain.as_ref().unwrap();
    assert_eq!(hit_explain.strategy, "weighted_rrf:causal");
    assert!(
        explain.hits[0]
            .provenance
            .hash
            .iter()
            .any(|byte| *byte != 0)
    );
}

#[test]
#[ignore = "manual FSV writes PH26 planned explain source-of-truth artifacts"]
fn planned_explain_search_manual_fsv() {
    let root = std::env::var("CALYX_FSV_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-planner-explain-path-fsv"));
    fs::create_dir_all(&root).unwrap();

    let engine = sample_engine();
    let planner = QueryPlanner::default();
    let explain = engine
        .planned_explain_search(causal_query(), &planner)
        .unwrap();
    let top = explain.hits.first().expect("planned hits");
    let hit_explain = top.explain.as_ref().expect("hit explain");
    let readback = json!({
        "intent": format!("{:?}", explain.intent),
        "strategy": explain.strategy.name(),
        "expected_profile": format!("{:?}", RrfProfile::Causal),
        "override_used": explain.override_used,
        "cost_estimate": explain.cost_estimate,
        "timeout_ms": explain.timeout_ms,
        "hit_count": explain.hits.len(),
        "top_hit": top.cx_id.to_string(),
        "hit_explain_strategy": hit_explain.strategy,
        "hit_explain_per_lens_count": hit_explain.per_lens_count,
        "hit_provenance_hex": hit_explain.provenance_hex,
        "hit_provenance_nonzero": top.provenance.hash.iter().any(|byte| *byte != 0),
        "planned_strategy_matches_hit_explain": explain.strategy.name() == hit_explain.strategy,
    });

    let path = root.join("planner-explain-readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();
    println!("planner_explain_readback={}", path.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(readback["intent"], "Causal");
    assert_eq!(readback["strategy"], "weighted_rrf:causal");
    assert_eq!(readback["timeout_ms"], 5_000);
    assert_eq!(readback["hit_provenance_nonzero"], true);
    assert_eq!(readback["planned_strategy_matches_hit_explain"], true);
}

fn sample_engine() -> SearchEngine {
    let map = SlotIndexMap::new();
    map.register(HnswIndex::new(SlotId::new(8), 3, 42)).unwrap();
    let mut engine = SearchEngine::new(map);
    for (seq, (id, vector)) in [
        (cx(1), basis_vec(0)),
        (cx(2), basis_vec(1)),
        (cx(3), basis_vec(2)),
    ]
    .into_iter()
    .enumerate()
    {
        let seq = seq as u64 + 1;
        engine
            .indexes
            .insert(SlotId::new(8), id, vector, seq)
            .unwrap();
        engine.put_constellation(sample_constellation(id, seq));
    }
    engine
}

fn causal_query() -> Query {
    Query::new("why causal effect")
        .with_vector(basis_vec(2))
        .with_slots(vec![SlotId::new(8)])
        .explain(true)
}

fn basis_vec(index: usize) -> SlotVector {
    let mut data = vec![0.0; 3];
    data[index % 3] = 1.0;
    SlotVector::Dense { dim: 3, data }
}

fn sample_constellation(cx_id: CxId, seq: u64) -> calyx_core::Constellation {
    calyx_core::Constellation {
        cx_id,
        vault_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse::<VaultId>().unwrap(),
        panel_version: 1,
        created_at: seq,
        input_ref: InputRef {
            hash: [seq as u8; 32],
            pointer: Some(format!("zfs://calyx/planned-explain/{cx_id}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: vec![Anchor {
            kind: AnchorKind::Label("planner-explain".to_string()),
            value: AnchorValue::Text("ok".to_string()),
            source: "planner-explain-fsv".to_string(),
            observed_at: seq,
            confidence: 1.0,
        }],
        provenance: LedgerRef {
            seq,
            hash: [seq as u8; 32],
        },
        flags: CxFlags::default(),
    }
}
