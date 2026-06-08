use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use calyx_core::CxId;
use calyx_mincut::{
    AgreementEdge, CitationEdge, ConstraintSense, FrequencyEntry, LpConstraint, LpProblem,
    LpSolution, LpVariable, MincutError, OptSense, SolveStatus, betweenness, betweenness_top_k,
    build_assoc_graph, condensate, mfvs_lp_problem, tarjan_scc,
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

fn write_readback(name: &str, value: serde_json::Value) {
    let Ok(root) = std::env::var("CALYX_FSV_ROOT") else {
        return;
    };
    let path = PathBuf::from(root).join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create fsv root");
    }
    fs::write(&path, serde_json::to_vec_pretty(&value).expect("json")).expect("write readback");
    println!("PH31_MINCUT_READBACK={}", path.display());
}

#[test]
fn scc_planted_cycle_and_condensation_match_known_partition() {
    let mut builder = builder_with_nodes(&[1, 2, 3, 4]);
    builder
        .add_edge(cx(1), cx(2), 1.0)
        .unwrap()
        .add_edge(cx(2), cx(3), 1.0)
        .unwrap()
        .add_edge(cx(3), cx(1), 1.0)
        .unwrap()
        .add_edge(cx(4), cx(1), 1.0)
        .unwrap();
    let graph = builder.build();
    let scc = tarjan_scc(&graph);
    let condensed = condensate(&graph, &scc).expect("condensate");

    println!(
        "SCC_PLANTED_READBACK components={:?} condensed_edges={:?}",
        scc.components, condensed.edges
    );
    write_readback(
        "ph31-scc-readback.json",
        json!({ "components": scc.components, "condensed_edges": condensed.edges }),
    );
    assert_eq!(scc.components, vec![vec![cx(1), cx(2), cx(3)], vec![cx(4)]]);
    assert_eq!(condensed.edges.len(), 1);
    assert_eq!(condensed.edges[0].src_component, 1);
    assert_eq!(condensed.edges[0].dst_component, 0);
    assert!(condensed.is_dag());
}

#[test]
fn scc_dag_singletons_clique_and_mismatch_edges() {
    let mut dag_builder = builder_with_nodes(&[1, 2, 3]);
    dag_builder
        .add_edge(cx(1), cx(2), 1.0)
        .unwrap()
        .add_edge(cx(2), cx(3), 1.0)
        .unwrap();
    let dag = dag_builder.build();
    assert_eq!(tarjan_scc(&dag).components.len(), 3);

    let mut clique_builder = builder_with_nodes(&[1, 2, 3, 4, 5]);
    for a in 1..=5 {
        for b in 1..=5 {
            if a != b {
                clique_builder.add_edge(cx(a), cx(b), 1.0).unwrap();
            }
        }
    }
    let clique = clique_builder.build();
    assert_eq!(tarjan_scc(&clique).components.len(), 1);

    let mut bad = tarjan_scc(&dag);
    bad.component_of.remove(&cx(3));
    assert_eq!(
        condensate(&dag, &bad).unwrap_err().code(),
        "CALYX_SCC_GRAPH_MISMATCH"
    );
}

#[test]
fn betweenness_path_scores_match_reference_and_top_k_is_stable() {
    let mut builder = builder_with_nodes(&[1, 2, 3, 4, 5]);
    for seed in 1..5 {
        builder.add_edge(cx(seed), cx(seed + 1), 1.0).unwrap();
    }
    let graph = builder.build();
    let scores = betweenness(&graph).expect("betweenness");
    let top = betweenness_top_k(&graph, 2).expect("top k");
    let table: BTreeMap<_, _> = scores
        .iter()
        .map(|(id, score)| (id.to_string(), *score))
        .collect();

    println!("BETWEENNESS_PATH_READBACK scores={table:?} top={top:?}");
    write_readback(
        "ph31-betweenness-readback.json",
        json!({ "path_scores": table, "top": top }),
    );
    assert!((scores[&cx(1)] - 0.0).abs() <= 1e-6);
    assert!((scores[&cx(2)] - 0.25).abs() <= 1e-6);
    assert!((scores[&cx(3)] - (1.0 / 3.0)).abs() <= 1e-6);
    assert!((scores[&cx(4)] - 0.25).abs() <= 1e-6);
    assert!((scores[&cx(5)] - 0.0).abs() <= 1e-6);
    assert_eq!(top[0].0, cx(3));
    assert_eq!(top[1].0, cx(2));
}

#[test]
fn betweenness_star_hub_is_maximal_and_empty_fails_closed() {
    let mut builder = builder_with_nodes(&[1, 2, 3, 4, 5]);
    for leaf in 2..=5 {
        builder.add_edge(cx(1), cx(leaf), 1.0).unwrap();
        builder.add_edge(cx(leaf), cx(1), 1.0).unwrap();
    }
    let graph = builder.build();
    let scores = betweenness(&graph).expect("betweenness");

    println!(
        "BETWEENNESS_STAR_READBACK hub={} leaves={:?}",
        scores[&cx(1)],
        &scores
    );
    assert!(scores[&cx(1)] > 0.99);
    for leaf in 2..=5 {
        assert_eq!(scores[&cx(leaf)], 0.0);
    }
    assert!(matches!(
        betweenness(&AssocGraph::builder().build()),
        Err(MincutError::BetweennessEmptyGraph)
    ));
}

#[test]
fn graph_builder_weights_frequency_and_citation_merge_are_exact() {
    let agreements = [
        AgreementEdge {
            src: cx(1),
            dst: cx(2),
            agreement: 0.8,
            directional_confidence: 0.9,
        },
        AgreementEdge {
            src: cx(2),
            dst: cx(3),
            agreement: 0.6,
            directional_confidence: 0.7,
        },
        AgreementEdge {
            src: cx(3),
            dst: cx(1),
            agreement: 1.0,
            directional_confidence: 1.0,
        },
        AgreementEdge {
            src: cx(1),
            dst: cx(3),
            agreement: 0.3,
            directional_confidence: 0.5,
        },
    ];
    let frequencies = [FrequencyEntry {
        cx_id: cx(1),
        frequency: 2.0,
    }];
    let citations = [CitationEdge {
        src: cx(1),
        dst: cx(3),
    }];
    let graph = build_assoc_graph(&agreements, &frequencies, &citations).expect("build graph");
    let edge_weights: BTreeMap<_, _> = graph
        .edges()
        .iter()
        .map(|edge| {
            let (src, dst) = graph.edge_endpoints(*edge);
            ((src.to_string(), dst.to_string()), edge.weight)
        })
        .collect();
    let edge_readback: Vec<_> = edge_weights
        .iter()
        .map(|((src, dst), weight)| json!({ "src": src, "dst": dst, "weight": weight }))
        .collect();
    let rounded_weights: Vec<_> = edge_weights
        .iter()
        .map(|((src, dst), weight)| {
            json!({ "src": src, "dst": dst, "weight_rounded": format!("{weight:.2}") })
        })
        .collect();

    println!(
        "GRAPH_BUILDER_READBACK weights={edge_weights:?} node_a={}",
        graph.node_weight(cx(1)).unwrap()
    );
    write_readback(
        "ph31-graph-builder-readback.json",
        json!({
            "edge_weights": edge_readback,
            "rounded_weights": rounded_weights,
            "node_a": graph.node_weight(cx(1)).unwrap()
        }),
    );
    assert!((edge_weights[&(cx(1).to_string(), cx(2).to_string())] - 0.72).abs() <= 1e-6);
    assert!((edge_weights[&(cx(2).to_string(), cx(3).to_string())] - 0.42).abs() <= 1e-6);
    assert!((edge_weights[&(cx(1).to_string(), cx(3).to_string())] - 1.0).abs() <= 1e-6);
    assert_eq!(graph.node_weight(cx(1)).unwrap(), 2.0);
}

#[test]
fn lp_scaffold_roundtrips_and_triangle_problem_has_unit_bounds() {
    let vars = vec![
        LpVariable::new(0, "x_a", 0.0, 1.0).unwrap(),
        LpVariable::new(1, "x_b", 0.0, 1.0).unwrap(),
        LpVariable::new(2, "x_c", 0.0, 1.0).unwrap(),
    ];
    let problem = LpProblem {
        vars,
        constraints: vec![LpConstraint {
            coeffs: vec![(0, 1.0), (1, 1.0)],
            sense: ConstraintSense::Geq,
            rhs: 1.0,
        }],
        objective: vec![(0, 1.0), (1, 1.0), (2, 1.0)],
        sense: OptSense::Minimize,
    };
    problem.validate().unwrap();
    let json = serde_json::to_string(&problem).unwrap();
    let restored: LpProblem = serde_json::from_str(&json).unwrap();
    assert_eq!(problem, restored);

    let mut builder = builder_with_nodes(&[1, 2, 3]);
    builder
        .add_edge(cx(1), cx(2), 1.0)
        .unwrap()
        .add_edge(cx(2), cx(3), 1.0)
        .unwrap()
        .add_edge(cx(3), cx(1), 1.0)
        .unwrap();
    let triangle_lp = mfvs_lp_problem(&builder.build()).unwrap();
    let solution = LpSolution {
        values: vec![0.0, 1.0, 0.5],
        objective_value: 1.5,
        status: SolveStatus::NotSolved,
    };
    let solution_json = serde_json::to_string(&solution).unwrap();
    let solution_restored: LpSolution = serde_json::from_str(&solution_json).unwrap();

    println!("LP_SCAFFOLD_READBACK problem={json} triangle={triangle_lp:?}");
    write_readback(
        "ph31-lp-readback.json",
        json!({ "problem": problem, "triangle_lp": triangle_lp, "solution": solution_restored }),
    );
    assert_eq!(triangle_lp.vars.len(), 3);
    assert!(
        triangle_lp
            .vars
            .iter()
            .all(|var| var.lb == 0.0 && var.ub == 1.0)
    );
    assert_eq!(triangle_lp.objective, vec![(0, 1.0), (1, 1.0), (2, 1.0)]);
    assert_eq!(solution, solution_restored);
}

#[test]
fn fail_closed_graph_builder_and_lp_edges() {
    let bad_agreement = [AgreementEdge {
        src: cx(1),
        dst: cx(2),
        agreement: 1.1,
        directional_confidence: 1.0,
    }];
    assert_eq!(
        build_assoc_graph(&bad_agreement, &[], &[])
            .unwrap_err()
            .code(),
        "CALYX_GRAPH_INVALID_WEIGHT"
    );
    assert_eq!(
        build_assoc_graph(
            &[],
            &[FrequencyEntry {
                cx_id: cx(1),
                frequency: 0.5
            }],
            &[]
        )
        .unwrap_err()
        .code(),
        "CALYX_GRAPH_INVALID_WEIGHT"
    );
    assert_eq!(
        LpVariable::new(0, "bad", 1.0, 0.0).unwrap_err().code(),
        "CALYX_LP_INVALID"
    );
    let invalid = LpProblem {
        vars: vec![LpVariable::new(0, "x", 0.0, 1.0).unwrap()],
        constraints: vec![LpConstraint {
            coeffs: vec![(5, 1.0)],
            sense: ConstraintSense::Leq,
            rhs: 0.0,
        }],
        objective: Vec::new(),
        sense: OptSense::Minimize,
    };
    assert_eq!(invalid.validate().unwrap_err().code(), "CALYX_LP_INVALID");
}

proptest! {
    #[test]
    fn random_dag_sccs_are_singletons(n in 1u8..12) {
        let mut builder = builder_with_nodes(&(1..=n).collect::<Vec<_>>());
        for src in 1..=n {
            for dst in src + 1..=n {
                if (src + dst) % 3 == 0 {
                    builder.add_edge(cx(src), cx(dst), 1.0).unwrap();
                }
            }
        }
        let graph = builder.build();
        prop_assert_eq!(tarjan_scc(&graph).components.len(), n as usize);
    }

    #[test]
    fn graph_builder_edge_count_matches_non_overlapping_edges(n in 1u8..20) {
        let agreements: Vec<_> = (1..=n)
            .map(|seed| AgreementEdge {
                src: cx(seed),
                dst: cx(seed.saturating_add(40)),
                agreement: 0.5,
                directional_confidence: 0.5,
            })
            .collect();
        let graph = build_assoc_graph(&agreements, &[], &[]).unwrap();
        prop_assert_eq!(graph.edge_count(), n as usize);
    }
}
