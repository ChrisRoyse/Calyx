use std::fs;
use std::path::PathBuf;

use calyx_core::CxId;
use calyx_lodestar::{
    KernelGraph, KernelGraphParams, LpRoundParams, lp_round_kernel_graph,
    lp_round_kernel_graph_from_solution, select_kernel_graph,
};
use calyx_mincut::{
    LpSolution, SolveStatus, betweenness, mfvs_lp_problem, solve_mfvs_lp, tarjan_scc,
    verify_feedback_vertex_set,
};
use calyx_paths::AssocGraph;
use serde_json::{Value, json};

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn readback_path() -> PathBuf {
    let root = calyx_fsv::fsv_root_or_else("CALYX_FSV_ROOT", || {
        PathBuf::from("target/fsv/issue1013-lp-mincut-solver")
    });
    root.join("issue1013-lp-mincut-solver-readback.json")
}

fn write_readback(value: &Value) -> PathBuf {
    let path = readback_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    path
}

fn builder_with_nodes(seeds: &[u8]) -> calyx_paths::AssocGraphBuilder {
    let mut builder = AssocGraph::builder();
    for seed in seeds {
        builder.add_node(cx(*seed), 1.0).unwrap();
    }
    builder
}

fn triangle_graph() -> AssocGraph {
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

fn dag_graph() -> AssocGraph {
    let mut builder = builder_with_nodes(&[4, 5, 6]);
    builder
        .add_edge(cx(4), cx(5), 1.0)
        .unwrap()
        .add_edge(cx(5), cx(6), 1.0)
        .unwrap();
    builder.build()
}

fn large_cycle_graph() -> AssocGraph {
    let mut builder = builder_with_nodes(&(10..=34).collect::<Vec<_>>());
    for seed in 10..34 {
        builder.add_edge(cx(seed), cx(seed + 1), 1.0).unwrap();
    }
    builder.add_edge(cx(34), cx(10), 1.0).unwrap();
    builder.build()
}

fn selected_kernel_graph(graph: &AssocGraph) -> KernelGraph {
    let scc = tarjan_scc(graph);
    let bet = betweenness(graph).unwrap();
    select_kernel_graph(
        graph,
        &scc,
        &bet,
        &[],
        &KernelGraphParams {
            target_fraction: 1.0,
            ..KernelGraphParams::default()
        },
    )
    .unwrap()
}

fn full_kernel_graph(graph: AssocGraph) -> KernelGraph {
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

fn graph_state(graph: &AssocGraph, fvs: &[CxId]) -> Value {
    json!({
        "nodes": graph.node_count(),
        "edges": graph.edge_count(),
        "node_order": graph.node_ids().collect::<Vec<_>>(),
        "fvs": fvs,
        "residual_acyclic": verify_feedback_vertex_set(graph, fvs).unwrap(),
    })
}

#[test]
fn issue1013_lp_mincut_solver_fsv() {
    let path = readback_path();
    if path.exists() {
        fs::remove_file(&path).unwrap();
    }

    let triangle = triangle_graph();
    let triangle_kernel = selected_kernel_graph(&triangle);
    let triangle_problem = mfvs_lp_problem(&triangle).unwrap();
    let triangle_solution = solve_mfvs_lp(&triangle).unwrap();
    let before_triangle = graph_state(&triangle_kernel.graph, &[]);
    let rounded_triangle =
        lp_round_kernel_graph(&triangle_kernel, &LpRoundParams::default()).unwrap();
    let after_triangle = graph_state(&triangle_kernel.graph, &rounded_triangle.selected);

    let dag = dag_graph();
    let dag_kernel = selected_kernel_graph(&dag);
    let before_dag = graph_state(&dag_kernel.graph, &[]);
    let rounded_dag = lp_round_kernel_graph(&dag_kernel, &LpRoundParams::default()).unwrap();
    let after_dag = graph_state(&dag_kernel.graph, &rounded_dag.selected);

    let all_zero = LpSolution {
        values: vec![0.0, 0.0, 0.0],
        objective_value: 0.0,
        status: SolveStatus::Optimal,
    };
    let before_all_zero = json!({
        "kernel_selected": triangle_kernel.selected,
        "lp_fraction": triangle_kernel.lp_fraction,
        "empty_fvs_state": graph_state(&triangle_kernel.graph, &[]),
    });
    let all_zero_error =
        lp_round_kernel_graph_from_solution(&triangle_kernel, &LpRoundParams::default(), &all_zero)
            .unwrap_err();
    let after_all_zero = json!({
        "kernel_selected": triangle_kernel.selected,
        "lp_fraction": triangle_kernel.lp_fraction,
        "empty_fvs_state": graph_state(&triangle_kernel.graph, &[]),
    });

    let large = large_cycle_graph();
    let large_kernel = full_kernel_graph(large.clone());
    let before_large = graph_state(&large_kernel.graph, &[]);
    let large_error = lp_round_kernel_graph(&large_kernel, &LpRoundParams::default()).unwrap_err();
    let after_large = graph_state(&large_kernel.graph, &[]);

    println!(
        "ISSUE1013_STATE happy_before={before_triangle} happy_after={after_triangle} dag_before={before_dag} dag_after={after_dag} all_zero_before={before_all_zero} all_zero_after={after_all_zero} large_before={before_large} large_after={after_large}"
    );

    let readback = json!({
        "source_of_truth": "issue1013-lp-mincut-solver-readback.json reread after running real calyx-mincut and calyx-lodestar solver code",
        "issue": 1013,
        "lp_problem": {
            "vars": triangle_problem.vars,
            "constraints": triangle_problem.constraints,
            "objective": triangle_problem.objective,
            "sense": triangle_problem.sense,
        },
        "happy_path": {
            "name": "triangle_minimum_fvs",
            "before": before_triangle,
            "solver_solution": triangle_solution,
            "after": after_triangle,
            "rounded_selected": rounded_triangle.selected,
            "lp_fraction": rounded_triangle.lp_fraction,
            "expected_selected": [cx(1)],
        },
        "edge_cases": [
            {
                "name": "dag_empty_fvs",
                "before": before_dag,
                "after": after_dag,
                "rounded_selected": rounded_dag.selected,
                "lp_fraction": rounded_dag.lp_fraction,
            },
            {
                "name": "all_zero_cyclic_solution",
                "before": before_all_zero,
                "after": after_all_zero,
                "error_code": all_zero_error.code(),
                "error_message": all_zero_error.to_string(),
            },
            {
                "name": "cyclic_graph_over_exact_solver_bound",
                "before": before_large,
                "after": after_large,
                "error_code": large_error.code(),
                "error_message": large_error.to_string(),
            }
        ],
    });
    let written = write_readback(&readback);
    let stored: Value = serde_json::from_slice(&fs::read(&written).unwrap()).unwrap();
    assert_eq!(stored, readback);
    assert_eq!(rounded_triangle.selected, vec![cx(1)]);
    assert!(after_triangle["residual_acyclic"].as_bool().unwrap());
    assert!(rounded_dag.selected.is_empty());
    assert!(after_dag["residual_acyclic"].as_bool().unwrap());
    assert_eq!(all_zero_error.code(), "CALYX_KERNEL_LP_INFEASIBLE");
    assert_eq!(large_error.code(), "CALYX_KERNEL_LP_UNAVAILABLE");
    println!("ISSUE1013_READBACK={}", written.display());
}
