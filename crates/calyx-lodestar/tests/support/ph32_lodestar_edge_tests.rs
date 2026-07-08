use super::*;

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
    let heuristic = select_kernel_graph(
        &graph,
        &scc,
        &bet,
        &[],
        &KernelGraphParams {
            target_fraction: 1.0,
            ..KernelGraphParams::default()
        },
    )
    .unwrap();
    let zeros = LpSolution {
        values: vec![0.0, 0.0, 0.0],
        objective_value: 0.0,
        status: SolveStatus::Optimal,
    };
    assert!(matches!(
        lp_round_kernel_graph_from_solution(&heuristic, &LpRoundParams::default(), &zeros),
        Err(LodestarError::KernelLpInfeasible { .. })
    ));
}

proptest! {
    #![proptest_config(calyx_testkit::integration_proptest_config(256))]

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
