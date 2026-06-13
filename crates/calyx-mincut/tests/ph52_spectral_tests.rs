use std::fs;
use std::path::PathBuf;

use calyx_core::CxId;
use calyx_mincut::{
    EigenPair, SpectralCache, SpectralCacheEntry, SpectralCacheKey, SpectralError,
    eigenvector_centrality, gft_project, gft_reconstruct, laplacian_eigenmaps,
    laplacian_eigenmaps_with_max_iter, spectral_gap,
};
use calyx_paths::AssocGraph;
use proptest::prelude::*;
use serde_json::json;

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn builder_with_nodes(seeds: &[u8]) -> calyx_paths::AssocGraphBuilder {
    let mut builder = AssocGraph::builder();
    for seed in seeds {
        builder.add_node(cx(*seed), 1.0).expect("add node");
    }
    builder
}

fn add_undirected(builder: &mut calyx_paths::AssocGraphBuilder, left: u8, right: u8) {
    builder.add_edge(cx(left), cx(right), 1.0).unwrap();
    builder.add_edge(cx(right), cx(left), 1.0).unwrap();
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
    println!("PH52_SPECTRAL_READBACK={}", path.display());
}

#[test]
fn cycle_graph_centrality_is_uniform() {
    let mut builder = builder_with_nodes(&[1, 2, 3, 4]);
    add_undirected(&mut builder, 1, 2);
    add_undirected(&mut builder, 2, 3);
    add_undirected(&mut builder, 3, 4);
    add_undirected(&mut builder, 4, 1);
    let ranked = eigenvector_centrality(&builder.build(), 64, 1e-6).expect("centrality");
    let first = ranked[0].1;
    assert!(
        ranked
            .iter()
            .all(|(_, score)| (score - first).abs() <= 1e-3)
    );
    write_readback(
        "ph52-cycle-centrality.json",
        json!({ "case": "cycle_uniform", "scores": ranked }),
    );
}

#[test]
fn planted_two_community_graph_bisects_by_second_eigenvector() {
    let graph = two_community_graph();
    let eigenmaps = laplacian_eigenmaps(&graph, 3).expect("eigenmaps");
    let fiedler = &eigenmaps[1].eigenvector;
    let left_signs = sign_count(&fiedler[0..5]);
    let right_signs = sign_count(&fiedler[5..10]);
    let gap = spectral_gap(&eigenmaps);

    println!("community_bisection left={left_signs:?} right={right_signs:?} spectral_gap={gap:.6}");
    write_readback(
        "ph52-community-eigenmaps.json",
        json!({
            "case": "community_bisection",
            "eigenvalues": eigenmaps.iter().map(|pair| pair.eigenvalue).collect::<Vec<_>>(),
            "fiedler": fiedler,
            "left_signs": left_signs,
            "right_signs": right_signs,
            "spectral_gap": gap,
        }),
    );
    assert!(gap > 0.0);
    assert!(
        (left_signs == (5, 0) && right_signs == (0, 5))
            || (left_signs == (0, 5) && right_signs == (5, 0))
    );
}

#[test]
fn gft_roundtrip_and_low_pass_filter_match_planted_signal() {
    let graph = two_community_graph();
    let eigenmaps = laplacian_eigenmaps(&graph, graph.node_count()).expect("basis");
    let smooth = vec![1.0, 1.0, 1.0, 1.0, 1.0, -1.0, -1.0, -1.0, -1.0, -1.0];
    let checker = vec![
        0.25, -0.25, 0.25, -0.25, 0.25, -0.25, 0.25, -0.25, 0.25, -0.25,
    ];
    let signal: Vec<_> = smooth.iter().zip(&checker).map(|(a, b)| a + b).collect();
    let coeffs = gft_project(&signal, &eigenmaps);
    let reconstructed = gft_reconstruct(&coeffs, &eigenmaps);
    let roundtrip_error = max_abs_delta(&signal, &reconstructed);
    let mut low_pass = coeffs.clone();
    for value in low_pass.iter_mut().skip(2) {
        *value = 0.0;
    }
    let filtered = gft_reconstruct(&low_pass, &eigenmaps);
    let smooth_error = max_abs_delta(&smooth, &filtered);

    println!("gft_roundtrip_error={roundtrip_error:.8} smooth_error={smooth_error:.8}");
    write_readback(
        "ph52-gft-readback.json",
        json!({
            "case": "gft_roundtrip_low_pass",
            "roundtrip_error": roundtrip_error,
            "smooth_error": smooth_error,
            "coefficients": coeffs,
        }),
    );
    assert!(roundtrip_error <= 1e-3);
    assert!(smooth_error <= 0.35);
}

#[test]
fn spectral_edges_fail_closed_and_star_hub_ranks_highest() {
    let one = builder_with_nodes(&[1]).build();
    assert_eq!(
        eigenvector_centrality(&one, 32, 1e-6).unwrap_err().code(),
        "CALYX_SPECTRAL_GRAPH_TOO_SMALL"
    );

    let mut disconnected = builder_with_nodes(&[1, 2, 3, 4]);
    add_undirected(&mut disconnected, 1, 2);
    add_undirected(&mut disconnected, 3, 4);
    let disconnected_eigs = laplacian_eigenmaps(&disconnected.build(), 2).expect("disconnected");
    assert_eq!(spectral_gap(&disconnected_eigs), 0.0);

    let mut star = builder_with_nodes(&[1, 2, 3, 4, 5]);
    for leaf in 2..=5 {
        add_undirected(&mut star, 1, leaf);
    }
    let star_graph = star.build();
    let ranked = eigenvector_centrality(&star_graph, 64, 1e-6).expect("star centrality");
    assert_eq!(ranked[0].0, cx(1));
    assert!(ranked[0].1 >= 2.0 * ranked[1].1);
    assert!(matches!(
        laplacian_eigenmaps_with_max_iter(&star_graph, 2, 0),
        Err(SpectralError::NotConverged { iterations: 0 })
    ));

    write_readback(
        "ph52-edge-readback.json",
        json!({
            "one_node_error": "CALYX_SPECTRAL_GRAPH_TOO_SMALL",
            "disconnected_gap": spectral_gap(&disconnected_eigs),
            "star_ranked": ranked,
            "not_converged_error": "CALYX_SPECTRAL_NOT_CONVERGED",
        }),
    );
}

#[test]
fn spectral_cache_roundtrips_and_invalidates_scope() {
    let key = SpectralCacheKey {
        scope: "ph52:fsv".to_string(),
        panel_version: 7,
    };
    let entry = SpectralCacheEntry {
        centrality: vec![(cx(1), 1.0)],
        eigenpairs: vec![EigenPair {
            eigenvalue: 0.0,
            eigenvector: vec![1.0],
        }],
        refreshed_at_seq: 42,
    };
    let mut cache = SpectralCache::default();
    cache.insert(key.clone(), entry.clone());
    assert_eq!(cache.get(&key), Some(&entry));
    cache.invalidate_scope("ph52:fsv");
    assert!(cache.is_empty());
}

proptest! {
    #[test]
    fn centrality_scores_are_normalized(n in 2u8..10) {
        let mut builder = builder_with_nodes(&(1..=n).collect::<Vec<_>>());
        for left in 1..n {
            add_undirected(&mut builder, left, left + 1);
        }
        let ranked = eigenvector_centrality(&builder.build(), 128, 1e-5).unwrap();
        prop_assert!(ranked.iter().all(|(_, score)| (0.0..=1.0).contains(score)));
        prop_assert!((ranked[0].1 - 1.0).abs() <= 1e-5);
        prop_assert!(ranked.iter().map(|(_, score)| score).sum::<f32>() > 0.0);
    }
}

fn two_community_graph() -> AssocGraph {
    let mut builder = builder_with_nodes(&(1..=10).collect::<Vec<_>>());
    for cluster in [1..=5, 6..=10] {
        let nodes: Vec<_> = cluster.collect();
        for (index, left) in nodes.iter().enumerate() {
            for right in nodes.iter().skip(index + 1) {
                add_undirected(&mut builder, *left, *right);
            }
        }
    }
    add_undirected(&mut builder, 5, 6);
    builder.build()
}

fn sign_count(values: &[f32]) -> (usize, usize) {
    (
        values.iter().filter(|value| **value >= 0.0).count(),
        values.iter().filter(|value| **value < 0.0).count(),
    )
}

fn max_abs_delta(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max)
}
