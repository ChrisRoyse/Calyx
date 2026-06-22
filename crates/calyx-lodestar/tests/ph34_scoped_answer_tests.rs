use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use calyx_core::{AnchorKind, CxId};
use calyx_lodestar::{
    CollectionId, GroundednessReport, Kernel, RecallReport, Scope, build_kernel_index,
    kernel_answer_scoped, kernel_search, materialize_scope,
};
use calyx_paths::AssocGraph;
use serde_json::json;

mod memory_assoc_support;
use memory_assoc_support::{MemoryAssocStore, cx, ids};

fn scoped_ranking_store() -> MemoryAssocStore {
    let mut builder = AssocGraph::builder();
    for seed in [1, 2, 9] {
        builder.add_node(cx(seed), 1.0).unwrap();
    }
    builder
        .add_edge(cx(1), cx(2), 0.8)
        .unwrap()
        .add_edge(cx(9), cx(2), 1.0)
        .unwrap();
    MemoryAssocStore::with_scope_data(
        builder.build(),
        BTreeMap::from([(CollectionId::from("answer-scope"), ids([1, 2]))]),
        BTreeMap::from([(domain_anchor(), vec![cx(9), cx(1)])]),
    )
}

fn domain_anchor() -> AnchorKind {
    AnchorKind::Label("domain".to_string())
}

fn answer_scope() -> Scope {
    Scope::Collection {
        id: CollectionId::from("answer-scope"),
    }
}

fn kernel(members: Vec<CxId>) -> Kernel {
    Kernel {
        kernel_id: cx(88),
        panel_version: 1,
        anchor_kind: Some("synthetic_anchor".to_string()),
        corpus_shard_hash: [8; 32],
        members: members.clone(),
        kernel_graph: members,
        groundedness: GroundednessReport {
            reached_anchor: 1.0,
            unanchored_members: Vec::new(),
        },
        recall: RecallReport::default(),
        built_at_millis: 1,
        estimator_provenance: "test".to_string(),
        warnings: Vec::new(),
    }
}

fn embeddings() -> BTreeMap<CxId, Vec<f32>> {
    BTreeMap::from([(cx(9), vec![1.0, 0.0]), (cx(1), vec![0.0, 1.0])])
}

fn fsv_root(case: &str) -> PathBuf {
    let base = std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph34-scoped-answer"));
    base.join(case)
}

fn write_readback(case: &str, name: &str, value: serde_json::Value) {
    let root = fsv_root(case);
    fs::create_dir_all(&root).expect("create readback root");
    let path = root.join(name);
    fs::write(&path, serde_json::to_vec_pretty(&value).expect("json")).expect("write readback");
    println!("PH34_SCOPED_ANSWER_READBACK={}", path.display());
}

#[test]
fn scoped_answer_filters_global_index_before_ranking() {
    let readback = scoped_answer_readback();
    write_readback(
        "rank",
        "ph34-scoped-answer-rank-readback.json",
        readback.clone(),
    );

    assert_eq!(readback["global_top_candidate"], json!(cx(9).to_string()));
    assert_eq!(readback["scoped_index_rows"], json!([cx(1).to_string()]));
    assert_eq!(readback["scoped_anchors"], json!([cx(1).to_string()]));
    assert_eq!(readback["selected_anchor"], json!(cx(1).to_string()));
    assert_eq!(readback["hop_count"], json!(1));
}

#[test]
fn scoped_answer_fails_closed_when_scope_removes_all_anchor_candidates() {
    let store = scoped_ranking_store();
    let index = build_kernel_index(&kernel(vec![cx(9)]), &embeddings()).unwrap();
    let err = kernel_answer_scoped(
        &index,
        &store,
        cx(2),
        &[1.0, 0.0],
        &answer_scope(),
        &[cx(9)],
        2,
    )
    .unwrap_err();
    write_readback(
        "no-anchor",
        "ph34-scoped-answer-no-anchor-readback.json",
        json!({
            "global_index_rows": row_ids(&index),
            "anchored_kernel_nodes": [cx(9).to_string()],
            "scope_nodes": scoped_node_ids(&store),
            "error": err.code(),
        }),
    );

    assert_eq!(err.code(), "CALYX_KERNEL_NO_ANCHORED_NODE");
}

#[test]
#[ignore = "manual FSV for #646 scoped answer candidate narrowing"]
fn ph34_scoped_answer_candidate_narrowing_manual_fsv() {
    let readback = scoped_answer_readback();
    write_readback(
        "fsv",
        "ph34-scoped-answer-candidate-narrowing-readback.json",
        readback.clone(),
    );

    assert_eq!(readback["global_top_candidate"], json!(cx(9).to_string()));
    assert_eq!(readback["scoped_index_rows"], json!([cx(1).to_string()]));
    assert_eq!(readback["selected_anchor"], json!(cx(1).to_string()));
}

fn scoped_answer_readback() -> serde_json::Value {
    let store = scoped_ranking_store();
    let scope = answer_scope();
    let index = build_kernel_index(&kernel(vec![cx(9), cx(1)]), &embeddings()).unwrap();
    let global_hits = kernel_search(&index, &[1.0, 0.0], index.rows().len()).unwrap();
    let scoped_graph = materialize_scope(&scope, &store).unwrap();
    let scoped_nodes: BTreeSet<_> = scoped_graph.node_ids().collect();
    let scoped_index = index.filter_to_nodes(&scoped_nodes).unwrap();
    let scoped_anchors = [cx(9), cx(1)]
        .into_iter()
        .filter(|anchor| scoped_nodes.contains(anchor))
        .collect::<Vec<_>>();
    let answer = kernel_answer_scoped(
        &index,
        &store,
        cx(2),
        &[1.0, 0.0],
        &scope,
        &[cx(9), cx(1)],
        2,
    )
    .unwrap();

    json!({
        "scope": scope,
        "query": cx(2).to_string(),
        "global_index_rows": row_ids(&index),
        "global_hits": hit_ids(&global_hits),
        "global_top_candidate": global_hits[0].0.to_string(),
        "scope_nodes": ids_to_strings(scoped_nodes.iter().copied()),
        "scoped_index_rows": row_ids(&scoped_index),
        "scoped_anchors": ids_to_strings(scoped_anchors.into_iter()),
        "selected_anchor": answer.anchor_kernel_node.to_string(),
        "hop_count": answer.hops.len(),
        "answer": answer,
    })
}

fn scoped_node_ids(store: &MemoryAssocStore) -> Vec<String> {
    let graph = materialize_scope(&answer_scope(), store).unwrap();
    ids_to_strings(graph.node_ids())
}

fn row_ids(index: &calyx_lodestar::KernelIndex) -> Vec<String> {
    ids_to_strings(index.rows().iter().map(|row| row.cx_id))
}

fn hit_ids(hits: &[(CxId, f32)]) -> Vec<String> {
    ids_to_strings(hits.iter().map(|(id, _)| *id))
}

fn ids_to_strings(ids: impl IntoIterator<Item = CxId>) -> Vec<String> {
    ids.into_iter().map(|id| id.to_string()).collect()
}
