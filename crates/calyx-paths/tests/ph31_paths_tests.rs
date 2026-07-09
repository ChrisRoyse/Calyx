use std::collections::BTreeMap;
use std::fs;

use calyx_core::CxId;
use calyx_paths::{
    AssocGraph, PathsError, attenuate, bidirectional, deattenuate, reach, reach_scored,
};
use proptest::prelude::*;
use serde_json::json;

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn linear_graph(len: u8) -> AssocGraph {
    let mut builder = AssocGraph::builder();
    for seed in 1..=len {
        builder.add_node(cx(seed), 1.0).expect("add node");
    }
    for seed in 1..len {
        builder
            .add_edge(cx(seed), cx(seed + 1), 1.0)
            .expect("add edge");
    }
    builder.build()
}

fn graph_from_edge_specs(edge_specs: &[(usize, usize, u16)]) -> AssocGraph {
    const NODE_COUNT: u8 = 12;
    let mut builder = AssocGraph::builder();
    for seed in 1..=NODE_COUNT {
        builder.add_node(cx(seed), 1.0).expect("add node");
    }
    for &(src, dst, weight_milli) in edge_specs {
        builder
            .add_edge(
                cx((src as u8 % NODE_COUNT) + 1),
                cx((dst as u8 % NODE_COUNT) + 1),
                f32::from(weight_milli) / 1000.0,
            )
            .expect("add edge");
    }
    builder.build()
}

fn write_readback(name: &str, value: serde_json::Value) {
    let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
        return;
    };
    let path = root.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create fsv root");
    }
    fs::write(&path, serde_json::to_vec_pretty(&value).expect("json")).expect("write readback");
    println!("PH31_PATHS_READBACK={}", path.display());
}

fn score_map(scores: Vec<(CxId, f32)>) -> BTreeMap<String, f32> {
    scores
        .into_iter()
        .map(|(id, score)| (id.to_string(), score))
        .collect()
}

fn reference_reach_scored(graph: &AssocGraph, src: CxId, max_hops: usize) -> BTreeMap<String, f32> {
    #[derive(Clone, Copy)]
    struct Entry {
        node: usize,
        hops: usize,
        raw_score: f32,
    }
    impl Entry {
        fn score(self) -> f32 {
            attenuate(self.raw_score, self.hops as u32)
        }

        fn is_better_than(self, known: Self) -> bool {
            match self.score().total_cmp(&known.score()) {
                std::cmp::Ordering::Greater => true,
                std::cmp::Ordering::Equal => self.hops < known.hops,
                std::cmp::Ordering::Less => false,
            }
        }
    }

    let src_idx = graph.node_index(src).expect("src index");
    let start = Entry {
        node: src_idx,
        hops: 0,
        raw_score: 1.0,
    };
    let mut best_by_node = vec![None; graph.node_count()];
    let mut best_by_hop = vec![vec![None; graph.node_count()]; max_hops.saturating_add(1)];
    best_by_node[src_idx] = Some(start);
    best_by_hop[0][src_idx] = Some(start);

    for hop in 0..max_hops {
        for current in best_by_hop[hop].clone().into_iter().flatten() {
            for edge in graph.out_edges_by_index(current.node) {
                let next = Entry {
                    node: edge.dst,
                    hops: current.hops + 1,
                    raw_score: current.raw_score * edge.weight,
                };
                if best_by_node[edge.dst].is_none_or(|known| next.is_better_than(known)) {
                    best_by_node[edge.dst] = Some(next);
                }
                if best_by_hop[next.hops][edge.dst].is_none_or(|known| next.is_better_than(known)) {
                    best_by_hop[next.hops][edge.dst] = Some(next);
                }
            }
        }
    }

    best_by_node
        .into_iter()
        .flatten()
        .filter(|entry| entry.node != src_idx)
        .map(|entry| {
            (
                graph.node_id(entry.node).expect("node id").to_string(),
                attenuate(entry.raw_score, entry.hops as u32),
            )
        })
        .collect()
}

#[test]
fn attenuation_matches_hop_power_reference() {
    let a0 = attenuate(1.0, 0);
    let a1 = attenuate(1.0, 1);
    let a10 = attenuate(1.0, 10);
    let restored = deattenuate(attenuate(0.42, 7), 7);

    println!("ATTENUATION_READBACK k0={a0:.6} k1={a1:.6} k10={a10:.6}");
    assert!((a0 - 1.0).abs() <= 1e-6);
    assert!((a1 - 0.9).abs() <= 1e-6);
    assert!((a10 - 0.34867844).abs() <= 1e-6);
    assert!((restored - 0.42).abs() <= 1e-6);
}

#[test]
fn graph_triangle_weights_and_frequency_are_stable() {
    let a = cx(1);
    let b = cx(2);
    let c = cx(3);
    let mut builder = AssocGraph::builder();
    builder
        .add_node(a, 3.0)
        .unwrap()
        .add_node(b, 1.0)
        .unwrap()
        .add_node(c, 1.0)
        .unwrap()
        .add_edge(a, b, 0.8)
        .unwrap()
        .add_edge(b, c, 0.6)
        .unwrap()
        .add_edge(c, a, 0.9)
        .unwrap();
    let graph = builder.build();
    let edge_table: Vec<_> = graph
        .edges()
        .iter()
        .map(|edge| {
            let (src, dst) = graph.edge_endpoints(*edge);
            (src.to_string(), dst.to_string(), edge.weight)
        })
        .collect();

    println!(
        "GRAPH_TRIANGLE_READBACK edges={edge_table:?} weight_a={}",
        graph.node_weight(a).unwrap()
    );
    write_readback(
        "ph31-paths-graph-readback.json",
        json!({ "edges": edge_table, "node_weight_a": graph.node_weight(a).unwrap() }),
    );
    assert_eq!(graph.edge_count(), 3);
    assert_eq!(graph.in_degree(b).unwrap(), 1);
    assert_eq!(graph.node_weight(a).unwrap(), 3.0);
}

#[test]
fn reverse_csr_incoming_edges_match_filter_readback() {
    let graph = graph_from_edge_specs(&[(0, 1, 400), (2, 1, 700), (3, 1, 600), (2, 2, 900)]);
    let b_idx = graph.node_index(cx(2)).expect("b index");
    let incoming: Vec<_> = graph
        .incoming_edges_by_index(b_idx)
        .map(|edge| {
            let (src, dst) = graph.edge_endpoints(edge);
            (src.to_string(), dst.to_string(), edge.weight)
        })
        .collect();
    let reference: Vec<_> = graph
        .edges()
        .iter()
        .copied()
        .filter(|edge| edge.dst == b_idx)
        .map(|edge| {
            let (src, dst) = graph.edge_endpoints(edge);
            (src.to_string(), dst.to_string(), edge.weight)
        })
        .collect();

    println!("REVERSE_CSR_READBACK incoming={incoming:?}");
    write_readback(
        "ph31-paths-reverse-csr-readback.json",
        json!({ "incoming": incoming, "reference": reference }),
    );
    assert_eq!(incoming, reference);
    assert_eq!(graph.in_degree(cx(2)).unwrap(), reference.len());
}

#[test]
fn traversal_reaches_linear_chain_and_scores_hops() {
    let graph = linear_graph(4);
    let path = reach(&graph, cx(1), cx(4), 3)
        .expect("reach result")
        .expect("path");
    let scored: BTreeMap<_, _> = reach_scored(&graph, cx(1), 3)
        .expect("scored")
        .into_iter()
        .map(|(id, score)| (id.to_string(), score))
        .collect();

    println!("TRAVERSAL_READBACK path={path:?} scored={scored:?}");
    write_readback(
        "ph31-paths-traversal-readback.json",
        json!({ "path": path, "scored": scored }),
    );
    assert_eq!(path, vec![cx(1), cx(2), cx(3), cx(4)]);
    assert!((scored[&cx(2).to_string()] - 0.9).abs() <= 1e-6);
    assert!((scored[&cx(3).to_string()] - 0.81).abs() <= 1e-6);
    assert!((scored[&cx(4).to_string()] - 0.729).abs() <= 1e-6);
}

#[test]
fn reach_scored_prefers_late_higher_ranked_path() {
    let graph = graph_from_edge_specs(&[(0, 1, 500), (0, 2, 900), (2, 1, 900)]);
    let scored = score_map(reach_scored(&graph, cx(1), 2).expect("scored"));
    let expected_b = attenuate(0.81, 2);

    println!(
        "SCORED_DIJKSTRA_READBACK b_score={:.6} expected={expected_b:.6}",
        scored[&cx(2).to_string()]
    );
    assert!((scored[&cx(2).to_string()] - expected_b).abs() <= 1e-6);
}

#[test]
fn bidirectional_reports_forward_and_reverse_paths() {
    let mut builder = AssocGraph::builder();
    builder
        .add_node(cx(1), 1.0)
        .unwrap()
        .add_node(cx(2), 1.0)
        .unwrap()
        .add_edge(cx(1), cx(2), 0.8)
        .unwrap()
        .add_edge(cx(2), cx(1), 0.7)
        .unwrap();
    let graph = builder.build();

    let paths = bidirectional(&graph, cx(1), cx(2), 1).expect("bidirectional path");

    println!(
        "BIDIRECTIONAL_READBACK forward={:?} reverse={:?}",
        paths.forward, paths.reverse
    );
    write_readback(
        "ph31-paths-bidirectional-readback.json",
        json!({ "forward": paths.forward, "reverse": paths.reverse }),
    );
    assert_eq!(paths.forward, Some(vec![cx(1), cx(2)]));
    assert_eq!(paths.reverse, Some(vec![cx(2), cx(1)]));
}

#[test]
fn traversal_edges_fail_closed_or_return_none() {
    let graph = linear_graph(2);
    assert_eq!(
        reach(&graph, cx(1), cx(1), 0).expect("self reach"),
        Some(vec![cx(1)])
    );
    assert_eq!(
        reach(&graph, cx(1), cx(2), 0).unwrap_err().code(),
        "CALYX_PATHS_MAX_HOPS"
    );

    let mut builder = AssocGraph::builder();
    builder
        .add_node(cx(9), 1.0)
        .unwrap()
        .add_node(cx(10), 1.0)
        .unwrap();
    let disconnected = builder.build();
    assert_eq!(reach(&disconnected, cx(9), cx(10), 100).unwrap(), None);

    let empty = AssocGraph::builder().build();
    assert!(matches!(
        reach(&empty, cx(1), cx(2), 1),
        Err(PathsError::NodeNotFound { .. })
    ));
}

#[test]
fn graph_parallel_self_loop_and_invalid_weights_are_handled() {
    let a = cx(1);
    let b = cx(2);
    let mut builder = AssocGraph::builder();
    builder
        .add_node(a, 1.0)
        .unwrap()
        .add_node(b, 1.0)
        .unwrap()
        .add_edge(a, b, 0.3)
        .unwrap()
        .add_edge(a, b, 0.7)
        .unwrap()
        .add_edge(a, a, 0.4)
        .unwrap();
    let graph = builder.build();
    let weights: Vec<_> = graph
        .out_neighbors(a)
        .unwrap()
        .iter()
        .map(|edge| edge.weight)
        .collect();

    println!("GRAPH_DEDUP_READBACK weights={weights:?}");
    assert_eq!(graph.edge_count(), 2);
    assert!(weights.contains(&0.7));
    assert!(weights.contains(&0.4));
    assert_eq!(
        AssocGraph::builder().add_node(a, -1.0).unwrap_err().code(),
        "CALYX_GRAPH_INVALID_WEIGHT"
    );
}

proptest! {
    #![proptest_config(calyx_testkit::integration_proptest_config(256))]

    #[test]
    fn uniform_chain_scores_decrease_with_hops(len in 2u8..20) {
        let graph = linear_graph(len);
        let scores = reach_scored(&graph, cx(1), len as usize).expect("scores");
        for pair in scores.windows(2) {
            prop_assert!(pair[0].1 > pair[1].1);
        }
    }

    #[test]
    fn reverse_csr_matches_filter_reference(
        edge_specs in prop::collection::vec((0usize..12, 0usize..12, 0u16..=1000), 0..96)
    ) {
        let graph = graph_from_edge_specs(&edge_specs);
        for node in 0..graph.node_count() {
            let incoming: Vec<_> = graph
                .incoming_edges_by_index(node)
                .map(|edge| (edge.src, edge.dst, edge.weight.to_bits()))
                .collect();
            let reference: Vec<_> = graph
                .edges()
                .iter()
                .copied()
                .filter(|edge| edge.dst == node)
                .map(|edge| (edge.src, edge.dst, edge.weight.to_bits()))
                .collect();
            let id = graph.node_id(node).expect("node id");
            let reference_len = reference.len();
            prop_assert_eq!(incoming, reference);
            prop_assert_eq!(graph.in_degree(id).unwrap(), reference_len);
        }
    }

    #[test]
    fn reach_scored_matches_bounded_dp_reference(
        edge_specs in prop::collection::vec((0usize..12, 0usize..12, 0u16..=1000), 0..96),
        max_hops in 0usize..6,
    ) {
        let graph = graph_from_edge_specs(&edge_specs);
        let actual = score_map(reach_scored(&graph, cx(1), max_hops).expect("scores"));
        let expected = reference_reach_scored(&graph, cx(1), max_hops);

        prop_assert_eq!(actual.len(), expected.len());
        for (id, expected_score) in expected {
            let actual_score = actual[&id];
            prop_assert!(
                (actual_score - expected_score).abs() <= 1e-6,
                "{id}: actual={actual_score} expected={expected_score}"
            );
        }
    }
}
