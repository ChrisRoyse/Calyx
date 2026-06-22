//! PH63 engine-level skills()/search_skill() (issue #600).
//!
//! Plants two well-separated direction clusters on one lens:
//!   P = {cx1 [1,0], cx2 [.96,.28], cx3 [.96,-.28]}
//!   Q = {cx4 [0,1], cx5 [.28,.96], cx6 [-.28,.96]}
//! Within-cluster cosine distance <= 0.157, cross-cluster best 0.4624, so
//! deterministic HDBSCAN* (mcs=2, min_samples=1) must select exactly the two
//! planted skills with no noise.

use std::collections::BTreeMap;

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, VaultId,
};
use calyx_sextant::{
    CALYX_SEXTANT_CX_MISSING, CALYX_SEXTANT_SKILL_BUDGET_EXCEEDED,
    CALYX_SEXTANT_SKILL_PAIR_NO_OVERLAP, CALYX_SEXTANT_SKILL_PARAMS, CALYX_SEXTANT_SKILL_UNKNOWN,
    HnswIndex, Query, SearchEngine, SextantIndex, SkillParams, SlotIndexMap, search_skill, skills,
};
use sextant_support::{cx_u128_be as cx, dense};

fn slot() -> SlotId {
    SlotId::new(1)
}

fn row(cx_id: CxId, seq: u64) -> calyx_core::Constellation {
    calyx_core::Constellation {
        cx_id,
        vault_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse::<VaultId>().unwrap(),
        panel_version: 1,
        created_at: seq,
        input_ref: InputRef {
            hash: [seq as u8; 32],
            pointer: Some(format!("zfs://calyx/issue600/{seq}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: vec![Anchor {
            kind: AnchorKind::Label("issue600".to_string()),
            value: AnchorValue::Text("ok".to_string()),
            source: "issue600-tests".to_string(),
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

fn planted_vectors() -> Vec<(CxId, Vec<f32>)> {
    vec![
        (cx(1), vec![1.0, 0.0]),
        (cx(2), vec![0.96, 0.28]),
        (cx(3), vec![0.96, -0.28]),
        (cx(4), vec![0.0, 1.0]),
        (cx(5), vec![0.28, 0.96]),
        (cx(6), vec![-0.28, 0.96]),
    ]
}

fn skills_engine() -> SearchEngine {
    let mut index = HnswIndex::new(slot(), 2, 7);
    for (seq, (id, data)) in planted_vectors().into_iter().enumerate() {
        index.insert(id, dense(data), seq as u64 + 1).unwrap();
    }
    let indexes = SlotIndexMap::new();
    indexes.register(index).unwrap();
    let mut engine = SearchEngine::new(indexes);
    for seq in 1..=6 {
        engine.put_constellation(row(cx(seq as u128), seq));
    }
    engine
}

fn test_params() -> SkillParams {
    SkillParams {
        min_cluster_size: 2,
        min_samples: 1,
        ..SkillParams::default()
    }
}

#[test]
fn skills_recovers_planted_clusters() {
    let engine = skills_engine();
    let tree = skills(&engine, &test_params()).unwrap();

    assert_eq!(tree.root.as_deref(), Some("skill-root"));
    assert_eq!(tree.selected.len(), 2);
    assert!(tree.noise.is_empty());

    let mut member_sets: Vec<Vec<CxId>> = tree
        .selected
        .iter()
        .map(|name| tree.nodes[name].members.clone())
        .collect();
    member_sets.sort();
    assert_eq!(member_sets[0], vec![cx(1), cx(2), cx(3)]);
    assert_eq!(member_sets[1], vec![cx(4), cx(5), cx(6)]);

    let root = &tree.nodes["skill-root"];
    assert_eq!(root.members.len(), 6);
    assert_eq!(root.children.len(), 2);
    assert!(!root.selected);
    for name in &tree.selected {
        let node = &tree.nodes[name];
        assert_eq!(node.parent.as_deref(), Some("skill-root"));
        assert_eq!(node.depth, 1);
        assert!(node.stability > 0.0);
        // Hand-computed: birth lambda = 1 / 0.4624 ~= 2.1626 (cross-cluster
        // mutual-reachability split distance).
        assert!((node.lambda_birth - 1.0 / 0.4624).abs() < 1e-3);
    }
}

#[test]
fn skills_tree_is_byte_deterministic() {
    let engine = skills_engine();
    let first = serde_json::to_vec(&skills(&engine, &test_params()).unwrap()).unwrap();
    let second = serde_json::to_vec(&skills(&engine, &test_params()).unwrap()).unwrap();
    assert_eq!(first, second);
}

#[test]
fn search_skill_restricts_to_member_scope() {
    let engine = skills_engine();
    let tree = skills(&engine, &test_params()).unwrap();
    let q_skill = tree
        .selected
        .iter()
        .find(|name| tree.nodes[*name].members.contains(&cx(4)))
        .unwrap();

    // Query points at [1,0]: globally the P cluster dominates, but inside the
    // Q skill the best match is cx(5) [.28,.96] (cosine 0.28), then cx(4) (0).
    let query = Query::new("issue600")
        .with_vector(dense(vec![1.0, 0.0]))
        .with_slots(vec![slot()]);
    let query = Query { k: 2, ..query };
    let hits = search_skill(&engine, &tree, q_skill, &query).unwrap();

    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].cx_id, cx(5));
    assert_eq!(hits[0].rank, 1);
    assert_eq!(hits[1].cx_id, cx(4));
    assert_eq!(hits[1].rank, 2);
    let members = &tree.nodes[q_skill].members;
    assert!(hits.iter().all(|hit| members.contains(&hit.cx_id)));
}

#[test]
fn skills_empty_engine_returns_empty_tree() {
    let engine = SearchEngine::new(SlotIndexMap::new());
    let tree = skills(&engine, &test_params()).unwrap();
    assert!(tree.root.is_none());
    assert!(tree.nodes.is_empty());
    assert!(tree.selected.is_empty());
    assert!(tree.noise.is_empty());
}

#[test]
fn skills_single_constellation_is_noise() {
    let mut index = HnswIndex::new(slot(), 2, 7);
    index.insert(cx(1), dense(vec![1.0, 0.0]), 1).unwrap();
    let indexes = SlotIndexMap::new();
    indexes.register(index).unwrap();
    let mut engine = SearchEngine::new(indexes);
    engine.put_constellation(row(cx(1), 1));

    let tree = skills(&engine, &test_params()).unwrap();
    assert_eq!(tree.root.as_deref(), Some("skill-root"));
    assert!(tree.selected.is_empty());
    assert_eq!(tree.noise, vec![cx(1)]);
    assert_eq!(tree.nodes["skill-root"].members, vec![cx(1)]);
}

#[test]
fn skills_edges_fail_closed() {
    let engine = skills_engine();

    let bad_mcs = skills(
        &engine,
        &SkillParams {
            min_cluster_size: 1,
            ..test_params()
        },
    )
    .unwrap_err();
    assert_eq!(bad_mcs.code, CALYX_SEXTANT_SKILL_PARAMS);

    let over_budget = skills(
        &engine,
        &SkillParams {
            max_constellations: 2,
            ..test_params()
        },
    )
    .unwrap_err();
    assert_eq!(over_budget.code, CALYX_SEXTANT_SKILL_BUDGET_EXCEEDED);

    // A stored constellation with no indexed vector cannot be placed.
    let mut engine_with_orphan = skills_engine();
    engine_with_orphan.put_constellation(row(cx(7), 7));
    let orphan = skills(&engine_with_orphan, &test_params()).unwrap_err();
    assert_eq!(orphan.code, CALYX_SEXTANT_CX_MISSING);

    let tree = skills(&engine, &test_params()).unwrap();
    let query = Query::new("issue600")
        .with_vector(dense(vec![1.0, 0.0]))
        .with_slots(vec![slot()]);
    let unknown = search_skill(&engine, &tree, "skill-nope", &query).unwrap_err();
    assert_eq!(unknown.code, CALYX_SEXTANT_SKILL_UNKNOWN);
}

#[test]
fn skills_pair_without_shared_lens_fails_closed() {
    let slot_b = SlotId::new(2);
    let mut index_a = HnswIndex::new(slot(), 2, 7);
    let mut index_b = HnswIndex::new(slot_b, 2, 7);
    index_a.insert(cx(1), dense(vec![1.0, 0.0]), 1).unwrap();
    index_b.insert(cx(2), dense(vec![0.0, 1.0]), 2).unwrap();
    let indexes = SlotIndexMap::new();
    indexes.register(index_a).unwrap();
    indexes.register(index_b).unwrap();
    let mut engine = SearchEngine::new(indexes);
    engine.put_constellation(row(cx(1), 1));
    engine.put_constellation(row(cx(2), 2));

    let err = skills(&engine, &test_params()).unwrap_err();
    assert_eq!(err.code, CALYX_SEXTANT_SKILL_PAIR_NO_OVERLAP);
}
