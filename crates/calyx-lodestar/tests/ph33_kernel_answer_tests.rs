use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use calyx_core::{CxId, LedgerRef};
use calyx_lodestar::{
    AnswerHop, AnswerPath, GroundednessReport, Kernel, RecallReport, build_kernel_index,
    kernel_answer, kernel_search,
};
use calyx_paths::AssocGraph;
use serde_json::json;

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
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
    BTreeMap::from([(cx(9), vec![0.99, 0.01]), (cx(10), vec![1.0, 0.0])])
}

fn ranked_anchor_embeddings() -> BTreeMap<CxId, Vec<f32>> {
    let mut embeddings = BTreeMap::new();
    for seed in 1..=12 {
        embeddings.insert(cx(seed), vec![1.0, seed as f32 * 0.001]);
    }
    embeddings.insert(cx(13), vec![0.0, 1.0]);
    embeddings
}

fn chain_graph() -> AssocGraph {
    let mut builder = AssocGraph::builder();
    for seed in [10, 11, 12, 13] {
        builder.add_node(cx(seed), 1.0).unwrap();
    }
    builder
        .add_edge(cx(10), cx(11), 1.0)
        .unwrap()
        .add_edge(cx(11), cx(12), 1.0)
        .unwrap()
        .add_edge(cx(12), cx(13), 1.0)
        .unwrap();
    builder.build()
}

fn far_rank_anchor_graph() -> AssocGraph {
    let mut builder = AssocGraph::builder();
    builder.add_node(cx(13), 1.0).unwrap();
    builder.add_node(cx(200), 1.0).unwrap();
    builder.add_edge(cx(13), cx(200), 1.0).unwrap();
    builder.build()
}

fn competing_anchor_graph() -> AssocGraph {
    let mut builder = AssocGraph::builder();
    for seed in [9, 10, 11, 12, 13] {
        builder.add_node(cx(seed), 1.0).unwrap();
    }
    builder
        .add_edge(cx(10), cx(11), 1.0)
        .unwrap()
        .add_edge(cx(11), cx(12), 1.0)
        .unwrap()
        .add_edge(cx(12), cx(13), 1.0)
        .unwrap();
    builder.build()
}

fn fsv_root(case: &str) -> PathBuf {
    let base = std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph33-t02"));
    base.join(case)
}

fn write_readback(case: &str, name: &str, value: serde_json::Value) {
    let root = fsv_root(case);
    fs::create_dir_all(&root).expect("create readback root");
    let path = root.join(name);
    fs::write(&path, serde_json::to_vec_pretty(&value).expect("json")).expect("readback write");
    println!("PH33_T02_READBACK={}", path.display());
}

#[test]
fn kernel_answer_chain_scores_and_provenance_are_deterministic() {
    let graph = chain_graph();
    let index = build_kernel_index(&kernel(vec![cx(9), cx(10)]), &embeddings()).unwrap();
    let answer = kernel_answer(&index, &graph, cx(13), &[0.99, 0.01], &[cx(10)], 3).unwrap();
    let scores: Vec<_> = answer.hops.iter().map(|hop| hop.hop_score).collect();
    let seqs: Vec<_> = answer.provenance.iter().map(|ledger| ledger.seq).collect();

    println!("KERNEL_ANSWER_CHAIN scores={scores:?} seqs={seqs:?} answer={answer:?}");
    write_readback(
        "chain",
        "kernel-answer-chain.json",
        json!({
            "answer": answer,
            "scores": scores,
            "provenance_seqs": seqs,
        }),
    );

    assert_eq!(answer.anchor_kernel_node, cx(10));
    assert_eq!(answer.hops.len(), 3);
    assert_eq!(scores, vec![1.0, 0.9, 0.80999994]);
    assert_eq!(seqs, vec![1, 2, 3]);
    assert!(
        answer
            .provenance
            .iter()
            .all(|ledger| ledger.hash != [0; 32])
    );
    assert!((answer.total_score - 2.71).abs() <= 1e-5);
}

#[test]
fn kernel_answer_max_hops_fails_closed_and_anchor_self_path_works() {
    let graph = chain_graph();
    let index = build_kernel_index(&kernel(vec![cx(10)]), &embeddings()).unwrap();
    let max_hops = kernel_answer(&index, &graph, cx(13), &[1.0, 0.0], &[cx(10)], 2).unwrap_err();
    let anchored = kernel_answer(&index, &graph, cx(10), &[1.0, 0.0], &[cx(10)], 3).unwrap();

    println!(
        "KERNEL_ANSWER_MAX_HOPS error={} anchored_total={}",
        max_hops.code(),
        anchored.total_score
    );
    write_readback(
        "edges",
        "kernel-answer-max-hops.json",
        json!({
            "max_hops_error": max_hops.code(),
            "anchored": anchored,
        }),
    );

    assert_eq!(max_hops.code(), "CALYX_PATHS_MAX_HOPS");
    assert_eq!(anchored.hops, Vec::new());
    assert_eq!(anchored.total_score, 1.0);
}

#[test]
fn kernel_answer_finds_anchor_ranked_beyond_old_top10_window() {
    let graph = far_rank_anchor_graph();
    let members: Vec<_> = (1..=13).map(cx).collect();
    let index = build_kernel_index(&kernel(members), &ranked_anchor_embeddings()).unwrap();
    let query_vec = [1.0, 0.0];
    let all_hits = kernel_search(&index, &query_vec, index.rows().len()).unwrap();
    let anchor_rank = all_hits
        .iter()
        .position(|(cx_id, _)| *cx_id == cx(13))
        .map(|idx| idx + 1)
        .expect("anchor should be present in exhausted search");
    let answer = kernel_answer(&index, &graph, cx(200), &query_vec, &[cx(13)], 1).unwrap();

    println!(
        "KERNEL_ANSWER_ANCHOR_RANK anchor_rank={} anchor={} hops={}",
        anchor_rank,
        answer.anchor_kernel_node,
        answer.hops.len()
    );
    write_readback(
        "anchor-rank",
        "kernel-answer-anchor-rank.json",
        json!({
            "anchor_rank": anchor_rank,
            "old_candidate_window": 10,
            "answer": answer,
            "hit_count": all_hits.len(),
        }),
    );

    assert!(anchor_rank > 10);
    assert_eq!(answer.anchor_kernel_node, cx(13));
    assert_eq!(answer.hops.len(), 1);
    assert_eq!(answer.hops[0].to, cx(200));
}

#[test]
fn kernel_answer_continues_to_next_reachable_anchor() {
    let graph = competing_anchor_graph();
    let index = build_kernel_index(&kernel(vec![cx(9), cx(10)]), &embeddings()).unwrap();
    let answer = kernel_answer(&index, &graph, cx(13), &[0.99, 0.01], &[cx(9), cx(10)], 3).unwrap();

    write_readback(
        "anchor-rank",
        "kernel-answer-next-reachable-anchor.json",
        json!({
            "unreachable_nearer_anchor": cx(9),
            "selected_anchor": answer.anchor_kernel_node,
            "query_cx": answer.query_cx,
            "hops": answer.hops,
            "total_score": answer.total_score,
        }),
    );

    assert_eq!(answer.anchor_kernel_node, cx(10));
    assert_eq!(answer.hops.len(), 3);
    assert!((answer.total_score - 2.71).abs() <= 1e-5);
}

#[test]
fn kernel_answer_fail_closed_edges_report_catalog_codes() {
    let graph = chain_graph();
    let index = build_kernel_index(&kernel(vec![cx(10)]), &embeddings()).unwrap();
    let no_anchor = kernel_answer(&index, &graph, cx(13), &[1.0, 0.0], &[], 3).unwrap_err();
    let no_path = kernel_answer(&index, &graph, cx(99), &[1.0, 0.0], &[cx(10)], 3).unwrap_err();
    let invalid = AnswerPath::checked(
        cx(13),
        cx(10),
        vec![AnswerHop {
            from: cx(10),
            to: cx(11),
            edge_weight: 1.0,
            hop_index: 0,
            hop_score: f32::NAN,
            ledger_ref: LedgerRef {
                seq: 1,
                hash: [1; 32],
            },
        }],
        f32::NAN,
    )
    .unwrap_err();

    println!(
        "KERNEL_ANSWER_ERRORS no_anchor={} no_path={} invalid={}",
        no_anchor.code(),
        no_path.code(),
        invalid.code()
    );
    write_readback(
        "edges",
        "kernel-answer-errors.json",
        json!({
            "no_anchor": no_anchor.code(),
            "no_path": no_path.code(),
            "invalid": invalid.code(),
        }),
    );

    assert_eq!(no_anchor.code(), "CALYX_KERNEL_NO_ANCHORED_NODE");
    assert_eq!(no_path.code(), "CALYX_PATHS_NODE_NOT_FOUND");
    assert_eq!(invalid.code(), "CALYX_KERNEL_SCORE_INVALID");
}
