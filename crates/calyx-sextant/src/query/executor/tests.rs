use calyx_aster::cf::ColumnFamily;
use calyx_aster::collection::{
    Collection, CollectionMode, DedupPolicy, FieldDef, FieldType, RetentionPolicy,
    SecondaryIndexKind, SecondaryIndexSpec, TemporalPolicy, TenantId, TxnPolicy,
};
use calyx_aster::layers::document::DocId;
use calyx_aster::layers::kv::kv_key;
use calyx_aster::layers::{
    DocumentLayer, KvLayer, RecordKey, RecordValue, RelationalLayer, Row, TimeSeriesLayer,
};
use calyx_aster::vault::AsterVault;
use calyx_core::{CxId, FixedClock, LensId, VaultId};
use proptest::prelude::*;
use serde_json::json;

use crate::query::{
    AggOp, AggSpec, CrossModelPlan, DocPathFilter, FieldOp, FieldPredicate, PlanStep,
};

use super::{execute, execute_at_snapshot};

mod ask;

fn vault() -> AsterVault {
    AsterVault::new(vault_id(), b"query-executor-test-salt".to_vec())
}

fn fixed_vault(now: u64) -> AsterVault<FixedClock> {
    AsterVault::with_clock(
        vault_id(),
        b"query-executor-test-salt".to_vec(),
        FixedClock::new(now),
    )
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

fn orders() -> Collection {
    let mut collection = collection("orders", CollectionMode::Records);
    collection.schema = Some(calyx_aster::collection::Schema::SchemaFull(vec![
        FieldDef::new("qty", FieldType::I64, false),
    ]));
    collection
}

fn kv_collection() -> Collection {
    collection("kv", CollectionMode::KV)
}

fn ts_collection() -> Collection {
    collection("timeseries", CollectionMode::TimeSeries)
}

fn docs_collection() -> Collection {
    collection("docs", CollectionMode::Documents)
}

fn relational_step(collection: Collection, min_qty: i64) -> PlanStep {
    PlanStep::RelationalScan {
        collection,
        filter: vec![FieldPredicate {
            field: "qty".to_string(),
            op: FieldOp::Gte,
            value: json!(min_qty),
        }],
        index: None,
    }
}

fn plan(steps: Vec<PlanStep>) -> CrossModelPlan {
    CrossModelPlan {
        steps,
        estimated_cost_ms: 1.0,
        explain: None,
    }
}

fn put_order(vault: &AsterVault, collection: &Collection, pk: u64, qty: i64) {
    RelationalLayer::new(vault)
        .put_record(
            collection,
            &RecordKey::from_u64(pk),
            &Row::new([("qty", RecordValue::I64(qty))]),
        )
        .unwrap();
}

fn expired_kv_value(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(9 + payload.len());
    out.push(1);
    out.extend_from_slice(&1_u64.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

fn key_u64(key: &RecordKey) -> u64 {
    u64::from_be_bytes(key.as_bytes().try_into().unwrap())
}

#[test]
fn relational_scan_filters_rows_at_pinned_snapshot() {
    let vault = vault();
    let orders = orders();
    for qty in [0, 1, 3, 5, 7] {
        put_order(&vault, &orders, qty as u64, qty);
    }

    let result = execute(&vault, plan(vec![relational_step(orders, 3)])).unwrap();
    let pks = result
        .rows
        .iter()
        .map(|row| key_u64(&row.key))
        .collect::<Vec<_>>();

    assert_eq!(pks, vec![3, 5, 7]);
    assert!(result.elapsed_ms < 1_000);
}

#[test]
fn multi_mode_relational_then_kv_returns_both_rows() {
    let vault = vault();
    let orders = orders();
    put_order(&vault, &orders, 3, 3);
    KvLayer::new(&vault)
        .kv_set(&kv_collection(), 1, b"sess", b"active", None)
        .unwrap();

    let result = execute(
        &vault,
        plan(vec![
            relational_step(orders, 3),
            PlanStep::KvGet {
                ns: "1".to_string(),
                key: b"sess".to_vec(),
            },
        ]),
    )
    .unwrap();

    assert_eq!(result.rows.len(), 2);
    assert_eq!(
        result.rows[1].value.as_ref().unwrap().get("__value"),
        Some(&RecordValue::Bytes(b"active".to_vec()))
    );
}

#[test]
fn time_series_range_returns_points_in_ascending_order() {
    let vault = vault();
    let ts = ts_collection();
    let layer = TimeSeriesLayer::new(&vault);
    for (ts_value, val) in [(20, 2.0), (10, 1.0), (30, 3.0)] {
        layer.ts_write(&ts, 1, ts_value, val).unwrap();
    }

    let result = execute(
        &vault,
        plan(vec![PlanStep::TsRangeScan {
            series: "1".to_string(),
            start: 0,
            end: i64::MAX,
        }]),
    )
    .unwrap();
    let points = result
        .rows
        .iter()
        .map(|row| row.value.as_ref().unwrap().get("ts").cloned().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(
        points,
        vec![
            RecordValue::U64(10),
            RecordValue::U64(20),
            RecordValue::U64(30)
        ]
    );
}

#[test]
fn document_scan_filters_matching_subtree_values() {
    let vault = vault();
    let docs = docs_collection();
    let layer = DocumentLayer::new(&vault);
    let active = DocId::from_slice(&[1; 16]).unwrap();
    let inactive = DocId::from_slice(&[2; 16]).unwrap();
    layer
        .put_doc(
            &docs,
            active,
            &json!({"profile": {"state": "active", "tier": "gold"}}),
        )
        .unwrap();
    layer
        .put_doc(
            &docs,
            inactive,
            &json!({"profile": {"state": "inactive", "tier": "silver"}}),
        )
        .unwrap();

    let result = execute(
        &vault,
        plan(vec![PlanStep::DocScan {
            collection: docs,
            path_filter: DocPathFilter {
                path: vec!["profile".to_string(), "state".to_string()],
                value: Some(json!("active")),
            },
        }]),
    )
    .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].key.as_bytes(), active.as_bytes());
    assert_eq!(
        result.rows[0].value.as_ref().unwrap().get("document"),
        Some(&RecordValue::Text("active".to_string()))
    );
}

#[test]
fn aggregate_count_collapses_matching_rows() {
    let vault = vault();
    let orders = orders();
    for qty in [0, 1, 3, 5, 7] {
        put_order(&vault, &orders, qty as u64, qty);
    }

    let result = execute(
        &vault,
        plan(vec![
            relational_step(orders, 3),
            PlanStep::Aggregate {
                spec: AggSpec {
                    op: AggOp::Count,
                    field: None,
                },
            },
        ]),
    )
    .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(
        result.rows[0].value.as_ref().unwrap().get("value"),
        Some(&RecordValue::U64(3))
    );
}

#[test]
fn empty_collection_and_expired_kv_are_absent_not_errors() {
    let vault = fixed_vault(10_000);
    let empty = execute(&vault, plan(vec![relational_step(orders(), 1)])).unwrap();
    assert!(empty.rows.is_empty());

    let kv = kv_collection();
    vault
        .write_cf(
            ColumnFamily::Kv,
            kv_key(&kv, 1, b"expired"),
            expired_kv_value(b"gone"),
        )
        .unwrap();
    let expired = execute(
        &vault,
        plan(vec![PlanStep::KvGet {
            ns: "1".to_string(),
            key: b"expired".to_vec(),
        }]),
    )
    .unwrap();
    assert!(expired.rows.is_empty());
}

#[test]
fn graph_stub_passes_input_cx_ids_and_vector_empty_candidates_stays_empty() {
    let first = CxId::from_input(b"first", 1, b"salt");
    let second = CxId::from_input(b"second", 1, b"salt");
    let graph = execute(
        &vault(),
        plan(vec![PlanStep::GraphHop {
            from_cx_ids: vec![first, second],
            hop_kind: "related".to_string(),
        }]),
    )
    .unwrap();
    assert_eq!(graph.rows.len(), 2);
    assert_eq!(graph.rows[0].key.as_bytes(), first.as_bytes());
    assert_eq!(graph.rows[1].key.as_bytes(), second.as_bytes());

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

proptest! {
    #[test]
    fn pinned_snapshot_excludes_rows_committed_after_pin(extra_qty in 10_i64..100) {
        let vault = vault();
        let orders = orders();
        put_order(&vault, &orders, 1, 1);
        let pinned = vault.latest_seq();
        put_order(&vault, &orders, extra_qty as u64, extra_qty);

        let result = execute_at_snapshot(&vault, plan(vec![relational_step(orders, 1)]), pinned).unwrap();
        let pks = result.rows.iter().map(|row| key_u64(&row.key)).collect::<Vec<_>>();

        prop_assert_eq!(pks, vec![1]);
    }

    #[test]
    fn storage_step_combinations_stay_on_pinned_snapshot(
        use_rel in any::<bool>(),
        use_kv in any::<bool>(),
        use_ts in any::<bool>(),
        aggregate in any::<bool>(),
        extra_qty in 10_i64..100,
    ) {
        prop_assume!(use_rel || use_kv || use_ts || aggregate);

        let vault = vault();
        let orders = orders();
        put_order(&vault, &orders, 1, 1);
        put_order(&vault, &orders, 3, 3);
        KvLayer::new(&vault)
            .kv_set(&kv_collection(), 1, b"sess", b"active", None)
            .unwrap();
        TimeSeriesLayer::new(&vault)
            .ts_write(&ts_collection(), 1, 10, 1.0)
            .unwrap();
        TimeSeriesLayer::new(&vault)
            .ts_write(&ts_collection(), 1, 20, 2.0)
            .unwrap();
        let pinned = vault.latest_seq();
        put_order(&vault, &orders, extra_qty as u64, extra_qty);
        KvLayer::new(&vault)
            .kv_set(&kv_collection(), 1, b"sess", b"late", None)
            .unwrap();
        TimeSeriesLayer::new(&vault)
            .ts_write(&ts_collection(), 1, 40, 4.0)
            .unwrap();

        let mut steps = Vec::new();
        if use_rel {
            steps.push(relational_step(orders, 1));
        }
        if use_kv {
            steps.push(PlanStep::KvGet {
                ns: "1".to_string(),
                key: b"sess".to_vec(),
            });
        }
        if use_ts {
            steps.push(PlanStep::TsRangeScan {
                series: "1".to_string(),
                start: 0,
                end: i64::MAX,
            });
        }
        if aggregate {
            steps.push(PlanStep::Aggregate {
                spec: AggSpec {
                    op: AggOp::Count,
                    field: None,
                },
            });
        }

        let result = execute_at_snapshot(&vault, plan(steps), pinned).unwrap();
        if aggregate {
            let expected_count = (if use_rel { 2 } else { 0 })
                + (if use_kv { 1 } else { 0 })
                + (if use_ts { 2 } else { 0 });
            prop_assert_eq!(
                result.rows[0].value.as_ref().unwrap().get("value"),
                Some(&RecordValue::U64(expected_count as u64))
            );
        } else {
            let late_rel_key = RecordKey::from_u64(extra_qty as u64);
            prop_assert!(result.rows.iter().all(|row| row.key != late_rel_key));
            let no_late_values = result.rows.iter().all(|row| {
                row.value.as_ref().is_none_or(|value| {
                    value.get("__value") != Some(&RecordValue::Bytes(b"late".to_vec()))
                        && value.get("ts") != Some(&RecordValue::U64(40))
                })
            });
            prop_assert!(no_late_values);
        }
    }
}
