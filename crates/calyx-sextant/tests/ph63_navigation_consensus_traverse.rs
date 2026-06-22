//! PH63 engine-level agree/disagree + traverse (issue #600).
//!
//! Synthetic known-I/O: every expected score is hand-computed from the
//! planted vectors / edge weights before the engine runs.

use std::collections::BTreeMap;

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, VaultId,
};
use calyx_paths::{AssocGraphBuilder, attenuate};
use calyx_sextant::{
    CALYX_SEXTANT_ASSOC_GRAPH_MISSING, CALYX_SEXTANT_CONSENSUS_INSUFFICIENT_LENSES,
    CALYX_SEXTANT_CX_MISSING, CALYX_SEXTANT_QUERY_SHAPE, CALYX_SEXTANT_SLOT_MISSING,
    CALYX_SEXTANT_TRAVERSE_HOPS, HnswIndex, SearchEngine, SextantIndex, SlotIndexMap,
    TraverseDirection, agree, disagree, traverse,
};
use sextant_support::{cx_u128_be as cx, dense};

const EPS: f32 = 1e-6;

fn slot_a() -> SlotId {
    SlotId::new(1)
}

fn slot_b() -> SlotId {
    SlotId::new(2)
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

/// Anchor cx(1) = [1,0] on both lenses. Candidates:
/// cx(2): [1,0]/[1,0]      -> cosines (1.0, 1.0)  min 1.0  spread 0.0
/// cx(3): [1,0]/[0,1]      -> cosines (1.0, 0.0)  min 0.0  spread 1.0
/// cx(4): [.6,.8]/[.6,.8]  -> cosines (0.6, 0.6)  min 0.6  spread 0.0
/// cx(5): slot A only      -> skipped (single shared lens)
fn consensus_engine() -> SearchEngine {
    let mut index_a = HnswIndex::new(slot_a(), 2, 7);
    let mut index_b = HnswIndex::new(slot_b(), 2, 7);
    index_a.insert(cx(1), dense(vec![1.0, 0.0]), 1).unwrap();
    index_b.insert(cx(1), dense(vec![1.0, 0.0]), 1).unwrap();
    index_a.insert(cx(2), dense(vec![1.0, 0.0]), 2).unwrap();
    index_b.insert(cx(2), dense(vec![1.0, 0.0]), 2).unwrap();
    index_a.insert(cx(3), dense(vec![1.0, 0.0]), 3).unwrap();
    index_b.insert(cx(3), dense(vec![0.0, 1.0]), 3).unwrap();
    index_a.insert(cx(4), dense(vec![0.6, 0.8]), 4).unwrap();
    index_b.insert(cx(4), dense(vec![0.6, 0.8]), 4).unwrap();
    index_a.insert(cx(5), dense(vec![1.0, 0.0]), 5).unwrap();
    let indexes = SlotIndexMap::new();
    indexes.register(index_a).unwrap();
    indexes.register(index_b).unwrap();
    let mut engine = SearchEngine::new(indexes);
    for seq in 1..=5 {
        engine.put_constellation(row(cx(seq as u128), seq));
    }
    engine
}

#[test]
fn agree_ranks_by_weakest_lens_consensus() {
    let engine = consensus_engine();
    let report = agree(&engine, cx(1), 10, None).unwrap();

    assert_eq!(report.slots, vec![slot_a(), slot_b()]);
    let ids: Vec<CxId> = report.hits.iter().map(|hit| hit.cx_id).collect();
    assert_eq!(ids, vec![cx(2), cx(4), cx(3)]);
    assert!((report.hits[0].score - 1.0).abs() < EPS);
    assert!((report.hits[1].score - 0.6).abs() < EPS);
    assert!(report.hits[2].score.abs() < EPS);
    assert_eq!(report.hits[0].rank, 1);
    assert_eq!(report.hits[2].per_slot.len(), 2);
    assert!((report.hits[2].mean_cosine - 0.5).abs() < EPS);
    assert_eq!(report.skipped_insufficient_overlap, vec![cx(5)]);
}

#[test]
fn disagree_ranks_by_cross_lens_spread() {
    let engine = consensus_engine();
    let report = disagree(&engine, cx(1), 10, None).unwrap();

    let ids: Vec<CxId> = report.hits.iter().map(|hit| hit.cx_id).collect();
    // cx(3) spread 1.0; cx(2)/cx(4) spread 0.0 tie-break by id.
    assert_eq!(ids, vec![cx(3), cx(2), cx(4)]);
    assert!((report.hits[0].score - 1.0).abs() < EPS);
    assert!((report.hits[0].spread - 1.0).abs() < EPS);
    assert!(report.hits[1].score.abs() < EPS);
}

#[test]
fn agree_and_disagree_disagree_on_top_result() {
    let engine = consensus_engine();
    let top_agree = agree(&engine, cx(1), 1, None).unwrap().hits[0].cx_id;
    let top_disagree = disagree(&engine, cx(1), 1, None).unwrap().hits[0].cx_id;
    assert_ne!(top_agree, top_disagree);
}

#[test]
fn consensus_edges_fail_closed() {
    let engine = consensus_engine();

    let k_zero = agree(&engine, cx(1), 0, None).unwrap_err();
    assert_eq!(k_zero.code, CALYX_SEXTANT_QUERY_SHAPE);

    let unknown_anchor = agree(&engine, cx(99), 10, None).unwrap_err();
    assert_eq!(unknown_anchor.code, CALYX_SEXTANT_CX_MISSING);

    // cx(5) only exposes slot A: cross-lens consensus is undefined.
    let one_lens = disagree(&engine, cx(5), 10, None).unwrap_err();
    assert_eq!(one_lens.code, CALYX_SEXTANT_CONSENSUS_INSUFFICIENT_LENSES);

    let bad_filter = agree(&engine, cx(1), 10, Some(&[slot_a(), SlotId::new(9)])).unwrap_err();
    assert_eq!(bad_filter.code, CALYX_SEXTANT_SLOT_MISSING);

    let narrow_filter = agree(&engine, cx(1), 10, Some(&[slot_a()])).unwrap_err();
    assert_eq!(
        narrow_filter.code,
        CALYX_SEXTANT_CONSENSUS_INSUFFICIENT_LENSES
    );
}

/// a -> b (0.8), b -> c (0.5), c -> a (0.25).
fn traverse_engine() -> SearchEngine {
    let mut builder = AssocGraphBuilder::default();
    builder.add_node(cx(1), 1.0).unwrap();
    builder.add_node(cx(2), 1.0).unwrap();
    builder.add_node(cx(3), 1.0).unwrap();
    builder.add_edge(cx(1), cx(2), 0.8).unwrap();
    builder.add_edge(cx(2), cx(3), 0.5).unwrap();
    builder.add_edge(cx(3), cx(1), 0.25).unwrap();
    let mut engine = SearchEngine::new(SlotIndexMap::new());
    engine.set_assoc_graph(builder.build());
    engine
}

#[test]
fn traverse_forward_attenuates_by_hop() {
    let engine = traverse_engine();
    let path = traverse(&engine, cx(1), TraverseDirection::Forward, 2).unwrap();

    assert_eq!(path.steps.len(), 2);
    assert_eq!(path.steps[0].cx_id, cx(2));
    assert_eq!(path.steps[0].hop, 1);
    assert_eq!(path.steps[0].via, cx(1));
    assert!((path.steps[0].score - attenuate(0.8, 1)).abs() < EPS);
    assert_eq!(path.steps[1].cx_id, cx(3));
    assert_eq!(path.steps[1].hop, 2);
    assert_eq!(path.steps[1].via, cx(2));
    assert!((path.steps[1].score - attenuate(0.8 * 0.5, 2)).abs() < EPS);
}

#[test]
fn traverse_backward_is_asymmetric() {
    let engine = traverse_engine();
    let path = traverse(&engine, cx(1), TraverseDirection::Backward, 2).unwrap();

    // Backward from a: c reached via reversed c->a, then b via reversed b->c.
    assert_eq!(path.steps.len(), 2);
    assert_eq!(path.steps[0].cx_id, cx(3));
    assert_eq!(path.steps[0].hop, 1);
    assert!((path.steps[0].score - attenuate(0.25, 1)).abs() < EPS);
    assert_eq!(path.steps[1].cx_id, cx(2));
    assert_eq!(path.steps[1].hop, 2);
    assert!((path.steps[1].score - attenuate(0.25 * 0.5, 2)).abs() < EPS);

    let forward = traverse(&engine, cx(1), TraverseDirection::Forward, 2).unwrap();
    let forward_ids: Vec<(CxId, u32)> = forward.steps.iter().map(|s| (s.cx_id, s.hop)).collect();
    let backward_ids: Vec<(CxId, u32)> = path.steps.iter().map(|s| (s.cx_id, s.hop)).collect();
    assert_ne!(forward_ids, backward_ids);
}

#[test]
fn traverse_both_reports_each_direction() {
    let engine = traverse_engine();
    let path = traverse(&engine, cx(1), TraverseDirection::Both, 1).unwrap();

    assert_eq!(path.steps.len(), 2);
    assert_eq!(path.steps[0].direction, TraverseDirection::Forward);
    assert_eq!(path.steps[0].cx_id, cx(2));
    assert_eq!(path.steps[1].direction, TraverseDirection::Backward);
    assert_eq!(path.steps[1].cx_id, cx(3));
}

#[test]
fn traverse_edges_fail_closed() {
    let engine = traverse_engine();

    let zero = traverse(&engine, cx(1), TraverseDirection::Forward, 0).unwrap_err();
    assert_eq!(zero.code, CALYX_SEXTANT_TRAVERSE_HOPS);

    let eleven = traverse(&engine, cx(1), TraverseDirection::Forward, 11).unwrap_err();
    assert_eq!(eleven.code, CALYX_SEXTANT_TRAVERSE_HOPS);

    let unknown = traverse(&engine, cx(99), TraverseDirection::Forward, 2).unwrap_err();
    assert_eq!(unknown.code, CALYX_SEXTANT_CX_MISSING);

    let no_graph = SearchEngine::new(SlotIndexMap::new());
    let missing = traverse(&no_graph, cx(1), TraverseDirection::Forward, 2).unwrap_err();
    assert_eq!(missing.code, CALYX_SEXTANT_ASSOC_GRAPH_MISSING);
}

#[test]
fn consensus_and_traverse_are_deterministic() {
    let engine = consensus_engine();
    let first = serde_json::to_string(&agree(&engine, cx(1), 10, None).unwrap()).unwrap();
    let second = serde_json::to_string(&agree(&engine, cx(1), 10, None).unwrap()).unwrap();
    assert_eq!(first, second);

    let graph_engine = traverse_engine();
    let lhs =
        serde_json::to_string(&traverse(&graph_engine, cx(1), TraverseDirection::Both, 2).unwrap())
            .unwrap();
    let rhs =
        serde_json::to_string(&traverse(&graph_engine, cx(1), TraverseDirection::Both, 2).unwrap())
            .unwrap();
    assert_eq!(lhs, rhs);
}
