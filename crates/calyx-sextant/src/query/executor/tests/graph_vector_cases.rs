use super::*;

#[test]
fn graph_hop_fails_closed_without_wired_association_graph() {
    let first = CxId::from_input(b"first", 1, b"salt");
    let second = CxId::from_input(b"second", 1, b"salt");
    let graph = execute(
        &vault(),
        plan(vec![PlanStep::GraphHop {
            from_cx_ids: vec![first, second],
            hop_kind: "related".to_string(),
            max_hops: 1,
        }]),
    )
    .unwrap_err();

    assert_eq!(graph.code, CALYX_SEXTANT_ASSOC_GRAPH_MISSING);
    assert!(graph.message.contains("no persisted nodes"));

    let vector = execute(
        &vault(),
        plan(vec![PlanStep::VectorFusion {
            lens_ids: vec![LensId::from_parts("sem", b"w", b"c", b"shape")],
            query_vec: vec![0.1, 0.2],
            limit: 3,
        }]),
    )
    .unwrap();
    assert!(vector.rows.is_empty());
}

#[test]
fn graph_hop_reads_persisted_edges_and_filters_hop_kind() {
    let vault = vault();
    let graph = PlainGraph::new(&vault, "default").unwrap();
    for id in [cx(1), cx(2), cx(3), cx(4)] {
        graph.put_node(id, b"{}").unwrap();
    }
    graph.put_edge(cx(1), "assoc", cx(2), b"12").unwrap();
    graph.put_edge(cx(2), "assoc", cx(3), b"23").unwrap();
    graph.put_edge(cx(1), "blocks", cx(4), b"14").unwrap();
    let before_rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Graph)
        .unwrap()
        .len();

    let result = execute(
        &vault,
        plan(vec![PlanStep::GraphHop {
            from_cx_ids: vec![cx(1)],
            hop_kind: "assoc".to_string(),
            max_hops: 2,
        }]),
    )
    .unwrap();

    let keys = result
        .rows
        .iter()
        .map(|row| CxId::from_bytes(row.key.as_bytes().try_into().unwrap()))
        .collect::<Vec<_>>();
    let graph_readback = graph
        .traverse(
            vault.latest_seq(),
            cx(1),
            TraverseOptions {
                edge_type: Some("assoc"),
                direction: PlainGraphDirection::Out,
                max_hops: 2,
                cost_cap: 32,
            },
        )
        .unwrap();
    let after_rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Graph)
        .unwrap()
        .len();

    assert_eq!(keys, vec![cx(2), cx(3)]);
    assert_eq!(keys, graph_readback);
    assert_eq!(before_rows, after_rows);
    assert!(result.rows.iter().all(|row| {
        row.value
            .as_ref()
            .unwrap()
            .get("hop_kind")
            .is_some_and(|value| value == &RecordValue::Text("assoc".to_string()))
    }));
}

#[test]
fn graph_hop_unknown_hop_kind_and_invalid_hops_fail_closed() {
    let vault = vault();
    let graph = PlainGraph::new(&vault, "default").unwrap();
    for id in [cx(1), cx(2)] {
        graph.put_node(id, b"{}").unwrap();
    }
    graph.put_edge(cx(1), "assoc", cx(2), b"12").unwrap();
    let before_seq = vault.latest_seq();

    let unknown = execute(
        &vault,
        plan(vec![PlanStep::GraphHop {
            from_cx_ids: vec![cx(1)],
            hop_kind: "blocks".to_string(),
            max_hops: 1,
        }]),
    )
    .unwrap_err();
    let invalid_hops = execute(
        &vault,
        plan(vec![PlanStep::GraphHop {
            from_cx_ids: vec![cx(1)],
            hop_kind: "assoc".to_string(),
            max_hops: 0,
        }]),
    )
    .unwrap_err();

    assert_eq!(unknown.code, CALYX_SEXTANT_GRAPH_HOP_KIND_UNKNOWN);
    assert!(unknown.message.contains("known hop kinds"));
    assert_eq!(invalid_hops.code, CALYX_SEXTANT_TRAVERSE_HOPS);
    assert_eq!(vault.latest_seq(), before_seq);
}

#[test]
fn vector_empty_candidates_stays_empty() {
    let vector = execute(
        &vault(),
        plan(vec![PlanStep::VectorFusion {
            lens_ids: vec![LensId::from_parts("sem", b"w", b"c", b"shape")],
            query_vec: vec![0.1, 0.2],
            limit: 3,
        }]),
    )
    .unwrap();
    assert!(vector.rows.is_empty());
}

#[test]
fn vector_candidates_fail_closed_without_wired_slot_indexes() {
    let vault = vault();
    let cx = CxId::from_input(b"candidate", 1, b"salt");
    let mut state = super::super::ExecState {
        rows: Vec::new(),
        candidates: BTreeSet::from([cx]),
        total_scanned: 0,
    };
    let before_candidates = state.candidates.clone();
    let err = super::super::execute_vector_fusion(
        &vault,
        vault.latest_seq(),
        &mut state,
        1,
        &[0.1, 0.2],
        3,
    )
    .unwrap_err();

    assert_eq!(err.code, CALYX_SEXTANT_VECTOR_FUSION_UNWIRED);
    assert!(err.message.contains("refusing synthetic ranking"));
    assert_eq!(state.candidates, before_candidates);
    assert!(state.rows.is_empty());
    assert_eq!(vault.latest_seq(), 0);
}

#[test]
fn relational_btree_index_path_filters_candidates() {
    let vault = vault();
    let mut orders = orders();
    orders.indexes.push(SecondaryIndexSpec {
        name: "orders_qty".to_string(),
        kind: SecondaryIndexKind::Btree,
        fields: vec!["qty".to_string()],
    });
    for qty in [1, 3, 5, 7] {
        put_order(&vault, &orders, qty as u64, qty);
    }

    let result = execute(
        &vault,
        plan(vec![PlanStep::RelationalScan {
            collection: orders.clone(),
            filter: vec![FieldPredicate {
                field: "qty".to_string(),
                op: FieldOp::Gte,
                value: json!(5),
            }],
            index: Some(orders.indexes[0].clone()),
        }]),
    )
    .unwrap();
    let pks = result
        .rows
        .iter()
        .map(|row| key_u64(&row.key))
        .collect::<Vec<_>>();

    assert_eq!(pks, vec![5, 7]);
}
