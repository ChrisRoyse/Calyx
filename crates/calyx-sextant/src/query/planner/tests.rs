use calyx_aster::collection::{
    Collection, CollectionMode, DedupPolicy, RetentionPolicy, SecondaryIndexKind,
    SecondaryIndexSpec, TemporalPolicy, TenantId, TxnPolicy,
};
use calyx_aster::layers::{RecordKey, RecordValue, RelationalLayer, Row};
use calyx_aster::vault::AsterVault;
use calyx_core::{CxId, LensId, VaultId};
use proptest::prelude::*;
use serde_json::json;

use crate::error::CALYX_PLANNER_COST_CAP;
use crate::query::{
    AggOp, AggSpec, AskSpec, FieldOp, FieldPredicate, GraphHop, KvLookup, PlanStepKind,
    RelationalFilter, TsRange, UniversalQuery, VectorQuery,
};

use super::{DEFAULT_COST_CAP_MS, plan};

fn vault() -> AsterVault {
    AsterVault::new(vault_id(), b"query-planner-test-salt".to_vec())
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn collection(name: &str, mode: CollectionMode) -> Collection {
    Collection {
        name: name.to_string(),
        mode,
        schema: None,
        panel: None,
        indexes: Vec::new(),
        dedup: DedupPolicy::Off,
        temporal: TemporalPolicy::default(),
        retention: RetentionPolicy::Forever,
        txn_policy: TxnPolicy::default(),
        tenant: TenantId::default(),
    }
}

fn relational_filter(rows: u64) -> RelationalFilter {
    RelationalFilter {
        collection: collection("orders", CollectionMode::Records),
        predicates: vec![FieldPredicate {
            field: "qty".to_string(),
            op: FieldOp::Gte,
            value: json!(1),
        }],
        estimated_rows: Some(rows),
    }
}

#[test]
fn relational_and_kv_order_relational_first() {
    let vault = vault();
    let query = UniversalQuery {
        relational: Some(relational_filter(1)),
        kv: Some(KvLookup {
            ns: "sessions".to_string(),
            key: b"sess-1".to_vec(),
        }),
        cost_cap_ms: Some(100),
        ..UniversalQuery::default()
    };

    let planned = plan(&vault, &query).unwrap();
    let kinds = planned
        .steps
        .iter()
        .map(|step| step.kind())
        .collect::<Vec<_>>();

    assert_eq!(
        kinds,
        vec![PlanStepKind::RelationalScan, PlanStepKind::KvGet]
    );
    assert!(planned.estimated_cost_ms > 0.0);
}

#[test]
fn relational_scan_rejects_when_explicit_cap_is_too_low() {
    let vault = vault();
    let query = UniversalQuery {
        relational: Some(relational_filter(1)),
        cost_cap_ms: Some(1),
        ..UniversalQuery::default()
    };

    let error = plan(&vault, &query).unwrap_err();

    assert_eq!(error.code, CALYX_PLANNER_COST_CAP);
    assert!(error.message.contains("exceeds cap 1 ms"));
}

#[test]
fn explain_has_one_entry_per_step_and_total_matches_parts() {
    let vault = vault();
    let query = UniversalQuery {
        relational: Some(relational_filter(10)),
        kv: Some(KvLookup {
            ns: "sessions".to_string(),
            key: b"sess-1".to_vec(),
        }),
        explain: true,
        cost_cap_ms: Some(100),
        ..UniversalQuery::default()
    };

    let planned = plan(&vault, &query).unwrap();
    let explain = planned.explain.as_ref().unwrap();
    let sum = explain
        .steps
        .iter()
        .map(|step| step.estimated_cost_ms)
        .sum::<f32>();

    assert_eq!(explain.steps.len(), planned.steps.len());
    assert!(
        explain
            .steps
            .iter()
            .all(|step| step.estimated_cost_ms > 0.0)
    );
    assert!((sum - explain.total_cost_ms).abs() < f32::EPSILON);
}

#[test]
fn unbounded_default_cap_rejects_large_relational_full_scan() {
    let vault = vault();
    let query = UniversalQuery {
        relational: Some(relational_filter(1_000_000)),
        cost_cap_ms: None,
        ..UniversalQuery::default()
    };

    let error = plan(&vault, &query).unwrap_err();

    assert_eq!(error.code, CALYX_PLANNER_COST_CAP);
    assert!(error.message.contains(&DEFAULT_COST_CAP_MS.to_string()));
}

#[test]
fn empty_query_is_zero_cost_and_accepted() {
    let vault = vault();

    let planned = plan(&vault, &UniversalQuery::default()).unwrap();

    assert!(planned.steps.is_empty());
    assert_eq!(planned.estimated_cost_ms, 0.0);
}

#[test]
fn ask_only_plans_single_ask_step() {
    let vault = vault();
    let query = UniversalQuery {
        ask: Some(AskSpec {
            question: "which orders need review?".to_string(),
            context_cx_ids: Vec::new(),
            top_k: 10,
            oracle: false,
        }),
        ..UniversalQuery::default()
    };

    let planned = plan(&vault, &query).unwrap();

    assert_eq!(planned.steps.len(), 1);
    assert_eq!(planned.steps[0].kind(), PlanStepKind::Ask);
}

#[test]
fn all_modes_keep_dependency_order() {
    let vault = vault();
    let query = UniversalQuery {
        relational: Some(relational_filter(10)),
        document: Some(crate::query::DocFilter {
            collection: collection("docs", CollectionMode::Documents),
            path: vec!["status".to_string()],
            value: Some(json!("open")),
            estimated_docs: Some(1),
        }),
        kv: Some(KvLookup {
            ns: "sessions".to_string(),
            key: b"sess-1".to_vec(),
        }),
        timeseries: Some(TsRange {
            series: "orders.latency".to_string(),
            start: 10,
            end: 20,
            estimated_points: Some(10),
        }),
        graph_hop: Some(GraphHop {
            from_cx_ids: vec![CxId::from_input(b"order", 1, b"salt")],
            hop_kind: "related".to_string(),
            max_hops: 2,
        }),
        vector: Some(VectorQuery {
            lens_ids: vec![LensId::from_parts("sem", b"w", b"c", b"shape")],
            query_vec: vec![0.1, 0.2],
            limit: 3,
        }),
        aggregate: Some(AggSpec {
            op: AggOp::Count,
            field: None,
        }),
        ask: Some(AskSpec {
            question: "summarize".to_string(),
            context_cx_ids: Vec::new(),
            top_k: 10,
            oracle: false,
        }),
        cost_cap_ms: Some(1_000),
        ..UniversalQuery::default()
    };

    let planned = plan(&vault, &query).unwrap();
    let kinds = planned
        .steps
        .iter()
        .map(|step| step.kind())
        .collect::<Vec<_>>();

    assert_eq!(
        kinds,
        vec![
            PlanStepKind::RelationalScan,
            PlanStepKind::KvGet,
            PlanStepKind::DocScan,
            PlanStepKind::TsRangeScan,
            PlanStepKind::GraphHop,
            PlanStepKind::VectorFusion,
            PlanStepKind::Aggregate,
            PlanStepKind::Ask,
        ]
    );
}

#[test]
fn cost_cap_zero_fails_closed_for_nonzero_plan() {
    let vault = vault();
    let query = UniversalQuery {
        kv: Some(KvLookup {
            ns: "sessions".to_string(),
            key: b"sess-1".to_vec(),
        }),
        cost_cap_ms: Some(0),
        ..UniversalQuery::default()
    };

    let error = plan(&vault, &query).unwrap_err();

    assert_eq!(error.code, CALYX_PLANNER_COST_CAP);
}

#[test]
fn relational_estimate_reads_visible_vault_rows_when_absent() {
    let vault = vault();
    let collection = collection("orders", CollectionMode::Records);
    let layer = RelationalLayer::new(&vault);
    layer
        .put_record(
            &collection,
            &RecordKey::from_u64(1),
            &Row::new([("qty", RecordValue::I64(2))]),
        )
        .unwrap();
    layer
        .put_record(
            &collection,
            &RecordKey::from_u64(2),
            &Row::new([("qty", RecordValue::I64(3))]),
        )
        .unwrap();
    let query = UniversalQuery {
        relational: Some(RelationalFilter {
            collection,
            predicates: Vec::new(),
            estimated_rows: None,
        }),
        cost_cap_ms: Some(100),
        ..UniversalQuery::default()
    };

    let planned = plan(&vault, &query).unwrap();

    assert_eq!(planned.steps[0].kind(), PlanStepKind::RelationalScan);
    assert_eq!(planned.estimated_cost_ms, 50.0);
}

#[test]
fn btree_index_uses_index_cost_and_explain_names_index() {
    let vault = vault();
    let mut collection = collection("orders", CollectionMode::Records);
    collection.indexes.push(SecondaryIndexSpec {
        name: "orders_qty".to_string(),
        kind: SecondaryIndexKind::Btree,
        fields: vec!["qty".to_string()],
    });
    let query = UniversalQuery {
        relational: Some(RelationalFilter {
            collection,
            predicates: vec![FieldPredicate {
                field: "qty".to_string(),
                op: FieldOp::Eq,
                value: json!(7),
            }],
            estimated_rows: Some(1_000_000),
        }),
        explain: true,
        cost_cap_ms: Some(10),
        ..UniversalQuery::default()
    };

    let planned = plan(&vault, &query).unwrap();
    let explain = planned.explain.unwrap();

    assert_eq!(planned.estimated_cost_ms, 5.0);
    assert_eq!(
        explain.steps[0].chosen_index.as_ref().unwrap().name,
        "orders_qty"
    );
}

proptest! {
    #[test]
    fn accepted_plans_never_exceed_explicit_cap(
        cap in 0_u32..1_000,
        rows in 0_u64..20_000,
        include_kv in any::<bool>(),
        include_ask in any::<bool>(),
    ) {
        let vault = vault();
        let query = UniversalQuery {
            relational: (rows > 0).then(|| relational_filter(rows)),
            kv: include_kv.then(|| KvLookup {
                ns: "sessions".to_string(),
                key: b"sess-1".to_vec(),
            }),
            ask: include_ask.then(|| AskSpec {
                question: "summarize".to_string(),
                context_cx_ids: Vec::new(),
                top_k: 10,
                oracle: false,
            }),
            cost_cap_ms: Some(cap),
            ..UniversalQuery::default()
        };

        if let Ok(planned) = plan(&vault, &query) {
            prop_assert!(planned.estimated_cost_ms <= cap as f32);
        }
    }
}
