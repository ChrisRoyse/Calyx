//! Issue #600 manual FSV driver: agree/disagree/traverse/skills/search_skill.
//!
//! Writes the full navigation readback to `$CALYX_FSV_ROOT`, re-reads the
//! bytes from disk (the Source of Truth), and asserts every hand-computed
//! expectation against the *re-read* JSON, never the in-memory values.

use std::collections::BTreeMap;
use std::fs;

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, VaultId,
    content_address,
};
use calyx_paths::AssocGraphBuilder;
use calyx_sextant::{
    CALYX_SEXTANT_ASSOC_GRAPH_MISSING, CALYX_SEXTANT_CONSENSUS_INSUFFICIENT_LENSES,
    CALYX_SEXTANT_CX_MISSING, CALYX_SEXTANT_SKILL_BUDGET_EXCEEDED, CALYX_SEXTANT_SKILL_UNKNOWN,
    CALYX_SEXTANT_TRAVERSE_HOPS, HnswIndex, Query, SearchEngine, SextantIndex, SkillParams,
    SlotIndexMap, TraverseDirection, agree, disagree, search_skill, skills, traverse,
};
use serde_json::json;
use sextant_support::{cx_u128_be as cx, dense};

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
            source: "issue600-fsv".to_string(),
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

/// Two-lens consensus vault + skills vault + association graph, all seeded
/// with hand-computed structure (see the sibling unit-test files).
fn fsv_engine() -> SearchEngine {
    let mut index_a = HnswIndex::new(slot_a(), 2, 7);
    let mut index_b = HnswIndex::new(slot_b(), 2, 7);
    let vectors_a: Vec<(CxId, Vec<f32>)> = vec![
        (cx(1), vec![1.0, 0.0]),
        (cx(2), vec![0.96, 0.28]),
        (cx(3), vec![0.96, -0.28]),
        (cx(4), vec![0.0, 1.0]),
        (cx(5), vec![0.28, 0.96]),
        (cx(6), vec![-0.28, 0.96]),
    ];
    for (seq, (id, data)) in vectors_a.iter().enumerate() {
        index_a
            .insert(*id, dense(data.clone()), seq as u64 + 1)
            .unwrap();
        index_b
            .insert(*id, dense(data.clone()), seq as u64 + 1)
            .unwrap();
    }
    // cx(7) disagrees across lenses: aligned with cx(1) on lens A, orthogonal
    // on lens B -> agree score 0.0, disagree score 1.0.
    index_a.insert(cx(7), dense(vec![1.0, 0.0]), 7).unwrap();
    index_b.insert(cx(7), dense(vec![0.0, 1.0]), 7).unwrap();
    let indexes = SlotIndexMap::new();
    indexes.register(index_a).unwrap();
    indexes.register(index_b).unwrap();
    let mut engine = SearchEngine::new(indexes);
    for seq in 1..=7 {
        engine.put_constellation(row(cx(seq as u128), seq));
    }
    let mut builder = AssocGraphBuilder::default();
    for seq in 1..=3 {
        builder.add_node(cx(seq), 1.0).unwrap();
    }
    builder.add_edge(cx(1), cx(2), 0.8).unwrap();
    builder.add_edge(cx(2), cx(3), 0.5).unwrap();
    builder.add_edge(cx(3), cx(1), 0.25).unwrap();
    engine.set_assoc_graph(builder.build());
    engine
}

#[test]
#[ignore = "manual FSV writes issue #600 navigation source-of-truth artifacts"]
fn navigation_manual_fsv() {
    let root = std::env::var("CALYX_FSV_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-issue600-navigation-fsv"));
    fs::create_dir_all(&root).unwrap();
    let engine = fsv_engine();

    let agree_report = agree(&engine, cx(1), 10, None).unwrap();
    let disagree_report = disagree(&engine, cx(1), 10, None).unwrap();
    let forward = traverse(&engine, cx(1), TraverseDirection::Forward, 2).unwrap();
    let backward = traverse(&engine, cx(1), TraverseDirection::Backward, 2).unwrap();
    let both = traverse(&engine, cx(1), TraverseDirection::Both, 2).unwrap();
    let params = SkillParams {
        min_cluster_size: 2,
        min_samples: 1,
        ..SkillParams::default()
    };
    let tree = skills(&engine, &params).unwrap();
    let tree_again = skills(&engine, &params).unwrap();
    let q_skill = tree
        .selected
        .iter()
        .find(|name| tree.nodes[*name].members.contains(&cx(4)))
        .unwrap()
        .clone();
    let query = Query::new("issue600-fsv")
        .with_vector(dense(vec![1.0, 0.0]))
        .with_slots(vec![slot_a()]);
    let query = Query { k: 2, ..query };
    let skill_hits = search_skill(&engine, &tree, &q_skill, &query).unwrap();

    let edges = json!({
        "agree_unknown_anchor": agree(&engine, cx(99), 10, None).unwrap_err().code,
        "traverse_hops_zero":
            traverse(&engine, cx(1), TraverseDirection::Forward, 0).unwrap_err().code,
        "traverse_hops_eleven":
            traverse(&engine, cx(1), TraverseDirection::Forward, 11).unwrap_err().code,
        "traverse_no_graph": traverse(
            &SearchEngine::new(SlotIndexMap::new()),
            cx(1),
            TraverseDirection::Forward,
            2,
        )
        .unwrap_err()
        .code,
        "skills_over_budget": skills(
            &engine,
            &SkillParams { max_constellations: 2, ..params.clone() },
        )
        .unwrap_err()
        .code,
        "search_skill_unknown":
            search_skill(&engine, &tree, "skill-nope", &query).unwrap_err().code,
        "empty_vault_skills_root":
            skills(&SearchEngine::new(SlotIndexMap::new()), &params).unwrap().root,
    });
    let consensus_insufficient = {
        let mut single = SearchEngine::new(SlotIndexMap::new());
        let mut only = HnswIndex::new(slot_a(), 2, 7);
        only.insert(cx(1), dense(vec![1.0, 0.0]), 1).unwrap();
        let map = SlotIndexMap::new();
        map.register(only).unwrap();
        single.indexes = map;
        single.put_constellation(row(cx(1), 1));
        agree(&single, cx(1), 10, None).unwrap_err().code
    };

    let report = json!({
        "issue": 600,
        "agree": agree_report,
        "disagree": disagree_report,
        "traverse_forward": forward,
        "traverse_backward": backward,
        "traverse_both": both,
        "skill_tree": tree,
        "skill_tree_deterministic_bytes":
            serde_json::to_vec(&tree).unwrap() == serde_json::to_vec(&tree_again).unwrap(),
        "search_skill_scope": q_skill,
        "search_skill_hits": skill_hits,
        "edges": edges,
        "edge_consensus_insufficient_lenses": consensus_insufficient,
    });
    let path = root.join("navigation-readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();

    // Execute & Inspect: re-read the SoT bytes from disk and assert on those.
    let bytes = fs::read(&path).unwrap();
    let readback: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let digest: String = content_address([bytes.as_slice()])
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    println!("ISSUE600_FSV_ROOT={}", root.display());
    println!("ISSUE600_NAVIGATION_REPORT={}", path.display());
    println!("ISSUE600_NAVIGATION_REPORT_BLAKE3={digest}");
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    // agree: cx(7) shares lens A direction with cx(1) but is orthogonal on
    // lens B -> min cosine 0.0; best agree hit is cx(2) (cos 0.96 both).
    let agree_hits = readback["agree"]["hits"].as_array().unwrap();
    assert_eq!(agree_hits[0]["cx_id"], json!(cx(2).to_string()));
    assert!((agree_hits[0]["score"].as_f64().unwrap() - 0.96).abs() < 1e-3);
    // disagree: cx(7) spread 1.0 dominates.
    let disagree_hits = readback["disagree"]["hits"].as_array().unwrap();
    assert_eq!(disagree_hits[0]["cx_id"], json!(cx(7).to_string()));
    assert!((disagree_hits[0]["score"].as_f64().unwrap() - 1.0).abs() < 1e-3);
    // traverse forward: b at 0.8*0.9, c at 0.4*0.81; backward differs.
    let fwd = readback["traverse_forward"]["steps"].as_array().unwrap();
    assert_eq!(fwd.len(), 2);
    assert!((fwd[0]["score"].as_f64().unwrap() - 0.72).abs() < 1e-3);
    assert!((fwd[1]["score"].as_f64().unwrap() - 0.324).abs() < 1e-3);
    let bwd = readback["traverse_backward"]["steps"].as_array().unwrap();
    assert!((bwd[0]["score"].as_f64().unwrap() - 0.225).abs() < 1e-3);
    assert_ne!(fwd[0]["cx_id"], bwd[0]["cx_id"]);
    // skills: exactly two selected planted clusters; deterministic bytes.
    assert_eq!(
        readback["skill_tree"]["selected"].as_array().unwrap().len(),
        2
    );
    assert_eq!(readback["skill_tree_deterministic_bytes"], json!(true));
    // search_skill stays inside the Q skill scope.
    let hits = readback["search_skill_hits"].as_array().unwrap();
    assert_eq!(hits[0]["cx_id"], json!(cx(5).to_string()));
    assert_eq!(hits[1]["cx_id"], json!(cx(4).to_string()));
    // fail-closed catalog.
    assert_eq!(
        readback["edges"]["agree_unknown_anchor"],
        CALYX_SEXTANT_CX_MISSING
    );
    assert_eq!(
        readback["edges"]["traverse_hops_zero"],
        CALYX_SEXTANT_TRAVERSE_HOPS
    );
    assert_eq!(
        readback["edges"]["traverse_hops_eleven"],
        CALYX_SEXTANT_TRAVERSE_HOPS
    );
    assert_eq!(
        readback["edges"]["traverse_no_graph"],
        CALYX_SEXTANT_ASSOC_GRAPH_MISSING
    );
    assert_eq!(
        readback["edges"]["skills_over_budget"],
        CALYX_SEXTANT_SKILL_BUDGET_EXCEEDED
    );
    assert_eq!(
        readback["edges"]["search_skill_unknown"],
        CALYX_SEXTANT_SKILL_UNKNOWN
    );
    assert_eq!(readback["edges"]["empty_vault_skills_root"], json!(null));
    assert_eq!(
        readback["edge_consensus_insufficient_lenses"],
        CALYX_SEXTANT_CONSENSUS_INSUFFICIENT_LENSES
    );
}
