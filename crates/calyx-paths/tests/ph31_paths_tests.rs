use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

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

fn write_readback(name: &str, value: serde_json::Value) {
    let Ok(root) = std::env::var("CALYX_FSV_ROOT") else {
        return;
    };
    let path = PathBuf::from(root).join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create fsv root");
    }
    fs::write(&path, serde_json::to_vec_pretty(&value).expect("json")).expect("write readback");
    println!("PH31_PATHS_READBACK={}", path.display());
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
    #[test]
    fn uniform_chain_scores_decrease_with_hops(len in 2u8..20) {
        let graph = linear_graph(len);
        let scores = reach_scored(&graph, cx(1), len as usize).expect("scores");
        for pair in scores.windows(2) {
            prop_assert!(pair[0].1 > pair[1].1);
        }
    }
}
