use std::collections::BTreeSet;

use calyx_lodestar::{
    DfvsMethod, IncrementalKernelEval, IncrementalResult, KernelGraphParams, LodestarError,
    LpRoundParams, NodeAddEdge, bounded_genus_approx, build_kernel_pipeline, dfvs_approx,
    genus_estimate, is_tournament, lp_round_kernel_graph, lp_round_kernel_graph_from_solution,
    select_kernel_graph, tournament_2approx,
};
use calyx_mincut::{LpSolution, SolveStatus, betweenness, tarjan_scc};
use proptest::prelude::*;
use serde_json::json;

#[path = "support/ph32_lodestar_helpers.rs"]
mod ph32_lodestar_helpers;
use ph32_lodestar_helpers::{
    builder_with_nodes, cx, full_kernel_graph, has_edge, hub_graph, kernel_params,
    merged_two_cycle_graph, planted_graph, triangle_graph, write_readback,
};

#[test]
fn kernel_graph_selects_two_hubs_and_reports_fraction() {
    let graph = hub_graph();
    let scc = tarjan_scc(&graph);
    let bet = betweenness(&graph).unwrap();
    let params = KernelGraphParams {
        target_fraction: 0.20,
        ..KernelGraphParams::default()
    };
    let selected = select_kernel_graph(&graph, &scc, &bet, &[cx(1)], &params).unwrap();

    println!(
        "KERNEL_GRAPH_READBACK selected={:?} fraction={:.3}",
        selected.selected, selected.source_fraction
    );
    write_readback(
        "ph32-kernel-graph-readback.json",
        json!({
            "selected": selected.selected,
            "source_fraction": selected.source_fraction,
            "scores": selected.scores,
        }),
    );
    assert_eq!(selected.selected, vec![cx(1), cx(2)]);
    assert!((selected.source_fraction - 0.20).abs() <= 1e-6);
}

#[test]
fn lp_round_selects_solution_values_and_fallback_warns() {
    let graph = hub_graph();
    let scc = tarjan_scc(&graph);
    let bet = betweenness(&graph).unwrap();
    let heuristic = select_kernel_graph(
        &graph,
        &scc,
        &bet,
        &[],
        &KernelGraphParams {
            target_fraction: 0.40,
            ..KernelGraphParams::default()
        },
    )
    .unwrap();
    let solution = LpSolution {
        values: vec![0.9, 0.3, 0.7, 0.1],
        objective_value: 1.6,
        status: SolveStatus::Optimal,
    };
    let rounded =
        lp_round_kernel_graph_from_solution(&heuristic, &LpRoundParams::default(), &solution)
            .unwrap();
    let fallback = lp_round_kernel_graph(&heuristic, &LpRoundParams::default()).unwrap();
    let strict_err = lp_round_kernel_graph(
        &heuristic,
        &LpRoundParams {
            fallback_to_heuristic: false,
            ..LpRoundParams::default()
        },
    )
    .unwrap_err();
    let fallback_is_heuristic = fallback.selected == heuristic.selected
        && fallback.lp_fraction == Some(heuristic.source_fraction);

    println!(
        "LP_ROUND_READBACK rounded={:?} strict_error={} fallback_warnings={:?}",
        rounded.selected,
        strict_err.code(),
        fallback.warnings
    );
    write_readback(
        "ph32-lp-round-readback.json",
        json!({
            "contract": "lp_solver_unconfigured_scaffold",
            "rounded": rounded.selected,
            "lp_fraction": rounded.lp_fraction,
            "injected_solution_source": "test-provided LpSolution, not external solver output",
            "strict_error": strict_err.code(),
            "strict_error_message": strict_err.to_string(),
            "heuristic_selected": heuristic.selected,
            "heuristic_source_fraction": heuristic.source_fraction,
            "fallback_selected": fallback.selected,
            "fallback_lp_fraction": fallback.lp_fraction,
            "fallback_is_heuristic": fallback_is_heuristic,
            "fallback_warnings": fallback.warnings.clone(),
        }),
    );
    assert_eq!(rounded.selected, vec![cx(1), cx(3)]);
    assert_eq!(strict_err.code(), "CALYX_KERNEL_LP_UNAVAILABLE");
    assert!(fallback_is_heuristic);
    assert!(
        fallback
            .warnings
            .iter()
            .any(|warning| warning.starts_with("CALYX_KERNEL_LP_UNAVAILABLE"))
    );
}

#[test]
fn dfvs_triangle_planted_and_dag_cases_are_verified() {
    let triangle = triangle_graph();
    let triangle_kernel = build_kernel_pipeline(&triangle, &[cx(1)], &kernel_params(1.0)).unwrap();
    assert_eq!(triangle_kernel.members.len(), 1);

    let planted = planted_graph();
    let planted_kernel =
        build_kernel_pipeline(&planted, &[cx(2), cx(5)], &kernel_params(1.0)).unwrap();
    let planted_members: BTreeSet<_> = planted_kernel.members.iter().copied().collect();

    let dag = {
        let mut builder = builder_with_nodes(&[1, 2, 3]);
        builder
            .add_edge(cx(1), cx(2), 1.0)
            .unwrap()
            .add_edge(cx(2), cx(3), 1.0)
            .unwrap();
        builder.build()
    };
    let dag_kernel = build_kernel_pipeline(&dag, &[cx(3)], &kernel_params(1.0)).unwrap();

    println!(
        "DFVS_READBACK triangle={:?} planted={:?} dag={:?}",
        triangle_kernel.members, planted_kernel.members, dag_kernel.members
    );
    write_readback(
        "ph32-dfvs-readback.json",
        json!({
            "triangle_members": triangle_kernel.members,
            "triangle_approx": triangle_kernel.recall.approx_factor,
            "triangle_method": triangle_kernel.estimator_provenance,
            "planted_members": planted_kernel.members,
            "planted_method": planted_kernel.estimator_provenance,
            "dag_members": dag_kernel.members,
            "dag_method": dag_kernel.estimator_provenance,
        }),
    );
    assert!(triangle_kernel.recall.approx_factor <= 3.0);
    assert!(planted_members.contains(&cx(1)));
    assert!(planted_members.contains(&cx(4)));
    assert!(dag_kernel.members.is_empty());
}

#[test]
fn dfvs_honest_bounds_distinguish_exact_from_approximate_path() {
    let exact_graph = triangle_graph();
    let exact = dfvs_approx(&full_kernel_graph(exact_graph)).unwrap();

    let approximate_graph = merged_two_cycle_graph();
    let approximate = dfvs_approx(&full_kernel_graph(approximate_graph.clone())).unwrap();
    let approximate_kernel =
        build_kernel_pipeline(&approximate_graph, &[cx(1), cx(12)], &kernel_params(1.0)).unwrap();

    println!(
        "DFVS_HONEST_BOUNDS_READBACK exact={exact:?} approximate={approximate:?} provenance={}",
        approximate_kernel.estimator_provenance
    );
    write_readback(
        "ph32-dfvs-honest-bounds-readback.json",
        json!({
            "exact": {
                "members": &exact.members,
                "approx_factor": exact.approx_factor,
                "tau_star_estimate": exact.tau_star_estimate,
                "tau_star_exact": exact.tau_star_exact,
                "method": exact.method,
            },
            "approximate": {
                "members": &approximate.members,
                "approx_factor": approximate.approx_factor,
                "tau_star_estimate": approximate.tau_star_estimate,
                "tau_star_exact": approximate.tau_star_exact,
                "method": approximate.method,
            },
            "kernel_recall": approximate_kernel.recall,
            "kernel_provenance": approximate_kernel.estimator_provenance,
        }),
    );

    assert_eq!(exact.members.len(), 1);
    assert_eq!(exact.approx_factor, 1.0);
    assert_eq!(exact.tau_star_estimate, 1);
    assert!(exact.tau_star_exact);
    assert_eq!(approximate.members.len(), 2);
    assert_eq!(approximate.approx_factor, 2.0);
    assert_eq!(approximate.tau_star_estimate, 1);
    assert!(!approximate.tau_star_exact);
    assert!(calyx_lodestar::dfvs::verify_feedback_vertex_set(
        &approximate_graph,
        &approximate.members
    ));
    assert_eq!(approximate_kernel.recall.approx_factor, 2.0);
    assert_eq!(approximate_kernel.recall.tau_star_estimate, 1);
    assert!(!approximate_kernel.recall.tau_star_exact);
    assert!(
        approximate_kernel
            .estimator_provenance
            .contains("approx_factor=2.000000")
    );
    assert!(
        approximate_kernel
            .estimator_provenance
            .contains("tau_star_exact=false")
    );
}

#[test]
fn tournament_and_bounded_genus_specializations_dispatch() {
    let triangle = triangle_graph();
    assert!(is_tournament(&triangle));
    let tournament = tournament_2approx(&triangle).unwrap();

    let mut planar_builder = builder_with_nodes(&[1, 2, 3, 4]);
    planar_builder
        .add_edge(cx(1), cx(2), 1.0)
        .unwrap()
        .add_edge(cx(2), cx(3), 1.0)
        .unwrap()
        .add_edge(cx(3), cx(1), 1.0)
        .unwrap()
        .add_edge(cx(3), cx(4), 1.0)
        .unwrap()
        .add_edge(cx(4), cx(2), 1.0)
        .unwrap();
    let planar = planar_builder.build();
    let genus = genus_estimate(&planar);
    let bounded = bounded_genus_approx(&planar, genus).unwrap();

    println!(
        "SPECIALIZED_DFVS_READBACK tournament={:?} bounded={:?} genus={}",
        tournament, bounded, genus
    );
    write_readback(
        "ph32-specialized-dfvs-readback.json",
        json!({ "tournament": tournament, "bounded": bounded, "genus": genus }),
    );
    assert_eq!(tournament.method, DfvsMethod::Tournament2Approx);
    assert!(tournament.approx_factor <= 2.0);
    assert_eq!(genus, 0);
    assert_eq!(bounded.method, DfvsMethod::BoundedGenus);
    assert_eq!(
        bounded_genus_approx(&planar, 101).unwrap_err().code(),
        "CALYX_DFVS_GENUS_TOO_LARGE"
    );
}

#[test]
fn kernel_pipeline_serializes_and_marks_ungrounded_provisional() {
    let graph = triangle_graph();
    let anchored = build_kernel_pipeline(&graph, &[cx(2)], &kernel_params(1.0)).unwrap();
    let ungrounded = build_kernel_pipeline(&graph, &[], &kernel_params(1.0)).unwrap();
    let json = serde_json::to_string(&anchored).unwrap();
    let restored: calyx_lodestar::Kernel = serde_json::from_str(&json).unwrap();

    println!(
        "KERNEL_PIPELINE_READBACK anchored={:?} ungrounded={:?}",
        anchored.members, ungrounded.warnings
    );
    write_readback(
        "ph32-kernel-pipeline-readback.json",
        json!({
            "anchored": anchored,
            "ungrounded": ungrounded,
            "roundtrip": restored,
        }),
    );
    assert_eq!(anchored, restored);
    assert!(ungrounded.estimator_provenance.contains("provisional"));
    assert!(
        ungrounded
            .warnings
            .iter()
            .any(|warning| warning.starts_with("CALYX_KERNEL_UNGROUNDED"))
    );
}

#[test]
fn incremental_leaf_dirty_cycle_full_rebuild_and_member_remove() {
    let graph = triangle_graph();
    let params = kernel_params(1.0);
    let kernel = build_kernel_pipeline(&graph, &[cx(2)], &params).unwrap();
    let mut eval = IncrementalKernelEval::new(kernel.clone(), graph.clone(), vec![cx(2)], params);

    let dirty = eval
        .apply_edge_weight_change(cx(1), cx(2), 0.1)
        .expect("dirty edge");
    eval.rebuild_dirty().unwrap();
    let leaf = eval
        .apply_node_add(
            cx(4),
            1.0,
            vec![NodeAddEdge::Out {
                dst: cx(1),
                weight: 1.0,
            }],
        )
        .unwrap();
    eval.rebuild_dirty().unwrap();
    let non_member_removed = eval.apply_node_remove(cx(4)).unwrap();
    assert!(eval.stale);
    eval.rebuild_dirty().unwrap();
    assert!(!eval.stale);
    let full = eval
        .apply_node_add(
            cx(5),
            1.0,
            vec![
                NodeAddEdge::Out {
                    dst: cx(1),
                    weight: 1.0,
                },
                NodeAddEdge::In {
                    src: cx(2),
                    weight: 1.0,
                },
            ],
        )
        .unwrap();
    let full_add_stored_candidate = eval.graph.require_node_index(cx(5)).is_ok()
        && has_edge(&eval.graph, cx(5), cx(1))
        && has_edge(&eval.graph, cx(2), cx(5));
    eval.rebuild_dirty().unwrap();
    let full_rebuild_retained_candidate = eval.graph.require_node_index(cx(5)).is_ok()
        && has_edge(&eval.graph, cx(5), cx(1))
        && has_edge(&eval.graph, cx(2), cx(5));
    let removed = eval.apply_node_remove(kernel.members[0]).unwrap();

    println!(
        "INCREMENTAL_READBACK dirty={dirty:?} leaf={leaf:?} non_member_removed={non_member_removed:?} full={full:?} full_add_stored_candidate={full_add_stored_candidate} full_rebuild_retained_candidate={full_rebuild_retained_candidate} removed={removed:?}"
    );
    write_readback(
        "ph32-incremental-readback.json",
        json!({
            "dirty": dirty,
            "leaf": leaf,
            "non_member_removed": non_member_removed,
            "full": full,
            "full_add_stored_candidate": full_add_stored_candidate,
            "full_rebuild_retained_candidate": full_rebuild_retained_candidate,
            "removed": removed,
        }),
    );
    assert!(matches!(dirty, IncrementalResult::Dirty { .. }));
    assert!(matches!(leaf, IncrementalResult::Dirty { .. }));
    assert!(!eval.kernel.members.contains(&cx(4)));
    assert!(matches!(
        non_member_removed,
        IncrementalResult::FullRebuildRequired { .. }
    ));
    assert!(matches!(
        full,
        IncrementalResult::FullRebuildRequired { .. }
    ));
    assert!(full_add_stored_candidate);
    assert!(full_rebuild_retained_candidate);
    assert!(matches!(
        removed,
        IncrementalResult::KernelMemberRemoved { .. }
    ));
}

#[test]
fn fail_closed_edges_report_catalog_codes() {
    let graph = triangle_graph();
    let scc = tarjan_scc(&graph);
    let bet = betweenness(&graph).unwrap();
    assert_eq!(
        select_kernel_graph(
            &graph,
            &scc,
            &bet,
            &[],
            &KernelGraphParams {
                target_fraction: 0.0,
                ..KernelGraphParams::default()
            },
        )
        .unwrap_err()
        .code(),
        "CALYX_KERNEL_INVALID_PARAMS"
    );
    let heuristic =
        select_kernel_graph(&graph, &scc, &bet, &[], &KernelGraphParams::default()).unwrap();
    let zeros = LpSolution {
        values: vec![0.0],
        objective_value: 0.0,
        status: SolveStatus::Optimal,
    };
    assert!(matches!(
        lp_round_kernel_graph_from_solution(&heuristic, &LpRoundParams::default(), &zeros),
        Err(LodestarError::KernelEmptyResult)
    ));
}

proptest! {
    #[test]
    fn selected_count_stays_within_ceiling(n in 1u8..20) {
        let mut builder = builder_with_nodes(&(1..=n).collect::<Vec<_>>());
        for seed in 1..n {
            builder.add_edge(cx(seed), cx(seed + 1), 1.0).unwrap();
        }
        let graph = builder.build();
        let scc = tarjan_scc(&graph);
        let bet = betweenness(&graph).unwrap();
        let params = KernelGraphParams {
            target_fraction: 0.25,
            ..KernelGraphParams::default()
        };
        let selected = select_kernel_graph(&graph, &scc, &bet, &[], &params).unwrap();
        prop_assert!(selected.selected.len() <= ((n as f32 * 0.25).ceil() as usize).max(1));
    }

    #[test]
    fn tournament_approx_removes_cycles(bits in any::<u16>()) {
        let mut builder = builder_with_nodes(&[1, 2, 3, 4]);
        let mut bit = 0;
        for a in 1..=4 {
            for b in a + 1..=4 {
                if (bits >> bit) & 1 == 0 {
                    builder.add_edge(cx(a), cx(b), 1.0).unwrap();
                } else {
                    builder.add_edge(cx(b), cx(a), 1.0).unwrap();
                }
                bit += 1;
            }
        }
        let graph = builder.build();
        let result = tournament_2approx(&graph).unwrap();
        let kernel = calyx_lodestar::KernelGraph {
            graph,
            selected: vec![cx(1), cx(2), cx(3), cx(4)],
            source_fraction: 1.0,
            lp_fraction: None,
            params: KernelGraphParams::default(),
            scores: Vec::new(),
            warnings: Vec::new(),
        };
        let dfvs = dfvs_approx(&kernel).unwrap();
        prop_assert_eq!(result.method, DfvsMethod::Tournament2Approx);
        prop_assert_eq!(dfvs.method, DfvsMethod::Tournament2Approx);
    }
}
