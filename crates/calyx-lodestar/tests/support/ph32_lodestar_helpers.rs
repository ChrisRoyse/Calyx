use std::fs;
use std::path::PathBuf;

use calyx_core::CxId;
use calyx_lodestar::{KernelGraph, KernelGraphParams, KernelParams, LpRoundParams};
use calyx_paths::AssocGraph;

pub fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

pub fn write_readback(name: &str, value: serde_json::Value) {
    let Ok(root) = std::env::var("CALYX_FSV_ROOT") else {
        return;
    };
    let path = PathBuf::from(root).join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create fsv root");
    }
    fs::write(&path, serde_json::to_vec_pretty(&value).expect("json")).expect("write readback");
    println!("PH32_READBACK={}", path.display());
}

pub fn builder_with_nodes(seeds: &[u8]) -> calyx_paths::AssocGraphBuilder {
    let mut builder = AssocGraph::builder();
    for seed in seeds {
        builder.add_node(cx(*seed), 1.0).unwrap();
    }
    builder
}

pub fn has_edge(graph: &AssocGraph, src: CxId, dst: CxId) -> bool {
    graph
        .edges()
        .iter()
        .any(|edge| graph.edge_endpoints(*edge) == (src, dst))
}

pub fn hub_graph() -> AssocGraph {
    let mut builder = builder_with_nodes(&(1..=10).collect::<Vec<_>>());
    for leaf in 3..=10 {
        builder.add_edge(cx(1), cx(leaf), 1.0).unwrap();
        builder.add_edge(cx(leaf), cx(1), 1.0).unwrap();
        builder.add_edge(cx(2), cx(leaf), 0.9).unwrap();
        builder.add_edge(cx(leaf), cx(2), 0.9).unwrap();
    }
    builder.add_edge(cx(1), cx(2), 1.0).unwrap();
    builder.add_edge(cx(2), cx(1), 1.0).unwrap();
    builder.build()
}

pub fn triangle_graph() -> AssocGraph {
    let mut builder = builder_with_nodes(&[1, 2, 3]);
    builder
        .add_edge(cx(1), cx(2), 1.0)
        .unwrap()
        .add_edge(cx(2), cx(3), 1.0)
        .unwrap()
        .add_edge(cx(3), cx(1), 1.0)
        .unwrap();
    builder.build()
}

pub fn planted_graph() -> AssocGraph {
    let mut builder = builder_with_nodes(&[1, 2, 3, 4, 5, 6]);
    builder
        .add_edge(cx(1), cx(2), 1.0)
        .unwrap()
        .add_edge(cx(2), cx(3), 1.0)
        .unwrap()
        .add_edge(cx(3), cx(1), 1.0)
        .unwrap()
        .add_edge(cx(4), cx(5), 1.0)
        .unwrap()
        .add_edge(cx(5), cx(6), 1.0)
        .unwrap()
        .add_edge(cx(6), cx(4), 1.0)
        .unwrap()
        .add_edge(cx(1), cx(4), 1.0)
        .unwrap()
        .add_edge(cx(4), cx(1), 1.0)
        .unwrap();
    builder.build()
}

pub fn merged_two_cycle_graph() -> AssocGraph {
    let mut builder = builder_with_nodes(&(1..=22).collect::<Vec<_>>());
    for seed in 1..11 {
        builder.add_edge(cx(seed), cx(seed + 1), 1.0).unwrap();
    }
    builder.add_edge(cx(11), cx(1), 1.0).unwrap();
    for seed in 12..22 {
        builder.add_edge(cx(seed), cx(seed + 1), 1.0).unwrap();
    }
    builder.add_edge(cx(22), cx(12), 1.0).unwrap();
    builder
        .add_edge(cx(1), cx(12), 1.0)
        .unwrap()
        .add_edge(cx(12), cx(1), 1.0)
        .unwrap();
    builder.build()
}

pub fn full_kernel_graph(graph: AssocGraph) -> KernelGraph {
    let selected = graph.node_ids().collect();
    KernelGraph {
        graph,
        selected,
        source_fraction: 1.0,
        lp_fraction: None,
        params: KernelGraphParams {
            target_fraction: 1.0,
            ..KernelGraphParams::default()
        },
        scores: Vec::new(),
        warnings: Vec::new(),
    }
}

pub fn kernel_params(target_fraction: f32) -> KernelParams {
    KernelParams {
        panel_version: 7,
        anchor_kind: Some("synthetic_outcome".to_string()),
        corpus_shard_hash: [9; 32],
        built_at_millis: 12345,
        kernel_graph: KernelGraphParams {
            target_fraction,
            max_groundedness_distance: 4,
            ..KernelGraphParams::default()
        },
        lp_round: LpRoundParams::default(),
    }
}
