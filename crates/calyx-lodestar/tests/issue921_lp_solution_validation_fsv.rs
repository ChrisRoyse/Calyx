use std::fs;
use std::path::PathBuf;

use calyx_lodestar::{
    KernelGraphParams, LpRoundParams, lp_round_kernel_graph_from_solution, select_kernel_graph,
};
use calyx_mincut::{LpSolution, SolveStatus, betweenness, tarjan_scc};
use calyx_paths::AssocGraph;
use serde_json::{Value, json};

fn cx(seed: u8) -> calyx_core::CxId {
    calyx_core::CxId::from_bytes([seed; 16])
}

fn test_graph() -> AssocGraph {
    let mut builder = AssocGraph::builder();
    for seed in 1..=4 {
        builder.add_node(cx(seed), 1.0).unwrap();
    }
    builder
        .add_edge(cx(1), cx(2), 1.0)
        .unwrap()
        .add_edge(cx(2), cx(3), 1.0)
        .unwrap()
        .add_edge(cx(3), cx(1), 1.0)
        .unwrap()
        .add_edge(cx(2), cx(4), 0.5)
        .unwrap()
        .add_edge(cx(4), cx(1), 0.5)
        .unwrap();
    builder.build()
}

fn readback_path() -> PathBuf {
    let root = calyx_fsv::fsv_root_or_else("CALYX_FSV_ROOT", || {
        PathBuf::from("target/fsv/issue921-lp-solution-validation")
    });
    root.join("issue921-lp-solution-validation-readback.json")
}

fn write_readback(value: &Value) -> PathBuf {
    let path = readback_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    path
}

fn edge_case(
    name: &str,
    solution: LpSolution,
    expected_code: &str,
    expected_message: &str,
    before_selected: &[calyx_core::CxId],
) -> Value {
    let graph = test_graph();
    let scc = tarjan_scc(&graph);
    let bet = betweenness(&graph).unwrap();
    let heuristic = select_kernel_graph(
        &graph,
        &scc,
        &bet,
        &[cx(1)],
        &KernelGraphParams {
            target_fraction: 1.0,
            ..KernelGraphParams::default()
        },
    )
    .unwrap();
    assert_eq!(heuristic.selected, before_selected);
    let before = json!({
        "selected": heuristic.selected,
        "lp_fraction": heuristic.lp_fraction,
    });
    let err = lp_round_kernel_graph_from_solution(&heuristic, &LpRoundParams::default(), &solution)
        .expect_err("invalid LP solution must fail closed");
    assert_eq!(err.code(), expected_code);
    assert!(
        err.to_string().contains(expected_message),
        "expected {name} error to contain {expected_message:?}, got {err}"
    );
    let after = json!({
        "selected": heuristic.selected,
        "lp_fraction": heuristic.lp_fraction,
    });
    assert_eq!(
        before, after,
        "failed edge case must not mutate source graph"
    );
    json!({
        "case": name,
        "before": before,
        "after": after,
        "code": err.code(),
        "message": err.to_string(),
    })
}

#[test]
fn issue921_lp_solution_validation_fsv() {
    let path = readback_path();
    if path.exists() {
        fs::remove_file(&path).unwrap();
    }

    let graph = test_graph();
    let scc = tarjan_scc(&graph);
    let bet = betweenness(&graph).unwrap();
    let heuristic = select_kernel_graph(
        &graph,
        &scc,
        &bet,
        &[cx(1)],
        &KernelGraphParams {
            target_fraction: 1.0,
            ..KernelGraphParams::default()
        },
    )
    .unwrap();
    let before_selected = heuristic.selected.clone();
    let optimal = LpSolution {
        values: vec![0.9, 0.2, 0.8, 0.1],
        objective_value: 2.0,
        status: SolveStatus::Optimal,
    };
    let rounded =
        lp_round_kernel_graph_from_solution(&heuristic, &LpRoundParams::default(), &optimal)
            .unwrap();
    assert_eq!(rounded.selected, vec![cx(1), cx(3)]);
    assert_eq!(rounded.lp_fraction, Some(0.5));

    let edge_cases = vec![
        edge_case(
            "not_solved_status",
            LpSolution {
                values: vec![0.9, 0.2, 0.8, 0.1],
                objective_value: 2.0,
                status: SolveStatus::NotSolved,
            },
            "CALYX_KERNEL_LP_UNAVAILABLE",
            "not optimal",
            &before_selected,
        ),
        edge_case(
            "nan_value",
            LpSolution {
                values: vec![0.9, f64::NAN, 0.8, 0.1],
                objective_value: 1.2,
                status: SolveStatus::Optimal,
            },
            "CALYX_KERNEL_INVALID_PARAMS",
            "must be finite",
            &before_selected,
        ),
        edge_case(
            "inf_objective",
            LpSolution {
                values: vec![0.9, 0.2, 0.8, 0.1],
                objective_value: f64::INFINITY,
                status: SolveStatus::Optimal,
            },
            "CALYX_KERNEL_INVALID_PARAMS",
            "objective_value must be finite",
            &before_selected,
        ),
        edge_case(
            "out_of_range_value",
            LpSolution {
                values: vec![0.9, -0.01, 0.8, 0.1],
                objective_value: 1.79,
                status: SolveStatus::Optimal,
            },
            "CALYX_KERNEL_INVALID_PARAMS",
            "outside [0, 1]",
            &before_selected,
        ),
        edge_case(
            "wrong_value_count",
            LpSolution {
                values: vec![0.9, 0.2, 0.8],
                objective_value: 1.9,
                status: SolveStatus::Optimal,
            },
            "CALYX_KERNEL_INVALID_PARAMS",
            "3 values for 4 nodes",
            &before_selected,
        ),
        edge_case(
            "objective_mismatch",
            LpSolution {
                values: vec![0.9, 0.2, 0.8, 0.1],
                objective_value: 1.2,
                status: SolveStatus::Optimal,
            },
            "CALYX_KERNEL_INVALID_PARAMS",
            "does not match sum(values)",
            &before_selected,
        ),
        edge_case(
            "all_zero_cyclic_solution",
            LpSolution {
                values: vec![0.0, 0.0, 0.0, 0.0],
                objective_value: 0.0,
                status: SolveStatus::Optimal,
            },
            "CALYX_KERNEL_LP_INFEASIBLE",
            "does not hit every directed cycle",
            &before_selected,
        ),
    ];

    let readback = json!({
        "source_of_truth": "issue921 FSV JSON persisted after running lp_round_kernel_graph_from_solution on the real Lodestar KernelGraph",
        "input_graph": {
            "nodes": graph.node_count(),
            "edges": graph.edge_count(),
            "node_order": graph.node_ids().collect::<Vec<_>>(),
        },
        "before_success": {
            "selected": before_selected,
            "lp_fraction": heuristic.lp_fraction,
        },
        "success": {
            "status": "optimal",
            "values": optimal.values,
            "objective_value": optimal.objective_value,
            "selected": rounded.selected,
            "lp_fraction": rounded.lp_fraction,
        },
        "edge_cases": edge_cases,
    });
    let written = write_readback(&readback);
    let stored: Value = serde_json::from_slice(&fs::read(&written).unwrap()).unwrap();
    assert_eq!(stored, readback);
    println!("ISSUE921_READBACK={}", written.display());
}
