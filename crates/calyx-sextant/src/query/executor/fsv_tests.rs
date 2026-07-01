use std::fs;
use std::path::Path;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::collection::{
    Collection, CollectionMode, DedupPolicy, FieldDef, FieldType, RetentionPolicy, TemporalPolicy,
    TenantId, TxnPolicy,
};
use calyx_aster::layers::kv::kv_key;
use calyx_aster::layers::{KvLayer, RecordKey, RecordValue, RelationalLayer, Row, TimeSeriesLayer};
use calyx_aster::plain_graph::{PlainGraph, PlainGraphDirection, TraverseOptions};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, LensId, VaultId};
use serde_json::json;

use crate::error::{
    CALYX_SEXTANT_ASSOC_GRAPH_MISSING, CALYX_SEXTANT_GRAPH_HOP_KIND_UNKNOWN,
    CALYX_SEXTANT_TRAVERSE_HOPS,
};
use crate::query::{AggOp, AggSpec, CrossModelPlan, FieldOp, FieldPredicate, PlanStep, execute};

use super::execute_at_snapshot;

#[test]
#[ignore = "manual FSV for issue #465"]
fn issue465_query_executor_fsv_writes_readback_artifacts() {
    let root = calyx_fsv::required_fsv_root("CALYX_FSV_ROOT").join("issue465-query-executor");
    fs::remove_dir_all(&root).ok();
    fs::create_dir_all(&root).unwrap();
    let vault_dir = root.join("vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue465-query-executor-fsv-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let orders = orders();
    let kv = kv_collection();
    let ts = ts_collection();
    let before = raw_state(&vault);
    println!("[BEFORE] {}", before);

    for qty in [0, 1, 3, 5, 7] {
        put_order(&vault, &orders, qty as u64, qty);
    }
    KvLayer::new(&vault)
        .kv_set(&kv, 1, b"sess", b"active", None)
        .unwrap();
    for (stamp, value) in [(10, 1.0), (20, 2.0), (30, 3.0)] {
        TimeSeriesLayer::new(&vault)
            .ts_write(&ts, 1, stamp, value)
            .unwrap();
    }
    vault.flush().unwrap();
    let pinned = vault.latest_seq();

    let happy = execute(
        &vault,
        plan(vec![
            relational_step(orders.clone(), 3),
            PlanStep::KvGet {
                ns: "1".to_string(),
                key: b"sess".to_vec(),
            },
        ]),
    )
    .unwrap();
    let ts_result = execute(
        &vault,
        plan(vec![PlanStep::TsRangeScan {
            series: "1".to_string(),
            start: 0,
            end: i64::MAX,
        }]),
    )
    .unwrap();
    let aggregate = execute(
        &vault,
        plan(vec![
            relational_step(orders.clone(), 3),
            PlanStep::Aggregate {
                spec: AggSpec {
                    op: AggOp::Count,
                    field: None,
                },
            },
        ]),
    )
    .unwrap();

    let empty = execute(
        &vault,
        plan(vec![relational_step(
            collection("empty", CollectionMode::Records),
            1,
        )]),
    )
    .unwrap();
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
    let first = CxId::from_input(b"first", 1, b"salt");
    let second = CxId::from_input(b"second", 1, b"salt");
    let graph = execute(
        &vault,
        plan(vec![PlanStep::GraphHop {
            from_cx_ids: vec![first, second],
            hop_kind: "related".to_string(),
            max_hops: 1,
        }]),
    )
    .unwrap_err();
    let vector_empty = execute(
        &vault,
        plan(vec![PlanStep::VectorFusion {
            lens_ids: vec![LensId::from_parts("sem", b"w", b"c", b"shape")],
            query_vec: vec![0.1, 0.2],
            limit: 3,
        }]),
    )
    .unwrap();
    put_order(&vault, &orders, 99, 99);
    let pinned_result = execute_at_snapshot(
        &vault,
        plan(vec![relational_step(orders.clone(), 1)]),
        pinned,
    )
    .unwrap();
    let fail_closed = execute(
        &vault,
        plan(vec![
            relational_step(orders.clone(), 3),
            PlanStep::Ask {
                question: "which orders?".to_string(),
                context_cx_ids: Vec::new(),
                top_k: 1,
                oracle: false,
            },
        ]),
    )
    .unwrap_err();
    vault.flush().unwrap();
    let after = raw_state(&vault);
    println!("[AFTER ] {}", after);
    println!("[GRAPH] {}", graph.code);

    assert_eq!(graph.code, CALYX_SEXTANT_ASSOC_GRAPH_MISSING);
    assert!(graph.message.contains("no persisted nodes"));

    let readback = json!({
        "source_of_truth": "Aster durable CF rows under vault/cf plus executor readback JSON",
        "trigger": "execute CrossModelPlan steps against one pinned Aster snapshot",
        "snapshot_seq_for_happy_query": pinned,
        "before": before,
        "after": after,
        "happy_relational_plus_kv": rows_json(&happy.rows),
        "expected_happy_relational_pks": [3, 5, 7],
        "expected_happy_kv_value_hex": hex(b"active"),
        "ts_rows": rows_json(&ts_result.rows),
        "aggregate_count_row": rows_json(&aggregate.rows),
        "edge_empty_rows": empty.rows.len(),
        "edge_expired_kv_rows": expired.rows.len(),
        "edge_graph_hop_error": {
            "code": graph.code,
            "message": graph.message,
            "source_ids": [first.to_string(), second.to_string()],
        },
        "edge_vector_empty_rows": vector_empty.rows.len(),
        "pinned_snapshot_after_post_write_keys": rows_json(&pinned_result.rows),
        "fail_closed_code": fail_closed.code,
        "physical_cf_files": {
            "relational": physical_files(&vault_dir.join("cf").join("relational")),
            "kv": physical_files(&vault_dir.join("cf").join("kv")),
            "timeseries": physical_files(&vault_dir.join("cf").join("timeseries")),
            "ledger": physical_files(&vault_dir.join("cf").join("ledger")),
        }
    });
    fs::write(
        root.join("issue465-query-executor-readback.json"),
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();
    println!("issue465_fsv_root={}", root.display());
}

#[test]
#[ignore = "manual FSV for issue #910"]
fn issue910_graph_hop_fsv_writes_readback_artifacts() {
    let root = calyx_fsv::required_fsv_root("CALYX_FSV_ROOT").join("issue910-graph-hop");
    fs::remove_dir_all(&root).ok();
    fs::create_dir_all(&root).unwrap();
    let vault_dir = root.join("vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue910-graph-hop-fsv-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let missing_before = raw_graph_state(&vault);
    println!("[EDGE missing before] {}", missing_before);
    let missing_graph = execute(
        &vault,
        plan(vec![PlanStep::GraphHop {
            from_cx_ids: vec![cx(1)],
            hop_kind: "assoc".to_string(),
            max_hops: 1,
        }]),
    )
    .unwrap_err();
    let missing_after = raw_graph_state(&vault);
    println!("[EDGE missing after ] {}", missing_after);

    let graph = PlainGraph::new(&vault, "default").unwrap();
    for id in [cx(1), cx(2), cx(3), cx(4)] {
        graph.put_node(id, b"{}").unwrap();
    }
    graph.put_edge(cx(1), "assoc", cx(2), b"12").unwrap();
    graph.put_edge(cx(2), "assoc", cx(3), b"23").unwrap();
    graph.put_edge(cx(1), "blocks", cx(4), b"14").unwrap();
    vault.flush().unwrap();
    let seeded = raw_graph_state(&vault);
    println!("[SEEDED] {}", seeded);

    let expected = graph
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
    let happy_before = raw_graph_state(&vault);
    println!("[HAPPY before] {}", happy_before);
    let happy = execute(
        &vault,
        plan(vec![PlanStep::GraphHop {
            from_cx_ids: vec![cx(1)],
            hop_kind: "assoc".to_string(),
            max_hops: 2,
        }]),
    )
    .unwrap();
    let happy_after = raw_graph_state(&vault);
    println!("[HAPPY after ] {}", happy_after);
    let actual = cx_rows(&happy.rows);
    assert_eq!(actual, expected);

    let unknown_before = raw_graph_state(&vault);
    println!("[EDGE unknown before] {}", unknown_before);
    let unknown = execute(
        &vault,
        plan(vec![PlanStep::GraphHop {
            from_cx_ids: vec![cx(1)],
            hop_kind: "missing".to_string(),
            max_hops: 1,
        }]),
    )
    .unwrap_err();
    let unknown_after = raw_graph_state(&vault);
    println!("[EDGE unknown after ] {}", unknown_after);
    assert_eq!(unknown.code, CALYX_SEXTANT_GRAPH_HOP_KIND_UNKNOWN);

    let invalid_before = raw_graph_state(&vault);
    println!("[EDGE invalid before] {}", invalid_before);
    let invalid_hops = execute(
        &vault,
        plan(vec![PlanStep::GraphHop {
            from_cx_ids: vec![cx(1)],
            hop_kind: "assoc".to_string(),
            max_hops: 0,
        }]),
    )
    .unwrap_err();
    let invalid_after = raw_graph_state(&vault);
    println!("[EDGE invalid after ] {}", invalid_after);
    assert_eq!(invalid_hops.code, CALYX_SEXTANT_TRAVERSE_HOPS);

    vault.flush().unwrap();
    let readback = json!({
        "issue": 910,
        "source_of_truth": "Aster durable graph CF rows plus executor readback JSON",
        "trigger": "execute PlanStep::GraphHop against persisted PlainGraph edges",
        "missing_graph_edge": {
            "before": missing_before,
            "after": missing_after,
            "code": missing_graph.code,
            "message": missing_graph.message,
        },
        "seeded_graph": seeded,
        "happy_path": {
            "before": happy_before,
            "after": happy_after,
            "expected_from_plain_graph_traverse": cx_json(&expected),
            "actual_executor_rows": rows_json(&happy.rows),
            "actual_cx_ids": cx_json(&actual),
            "total_scanned": happy.total_scanned,
        },
        "edge_unknown_hop_kind": {
            "before": unknown_before,
            "after": unknown_after,
            "code": unknown.code,
            "message": unknown.message,
        },
        "edge_invalid_max_hops_zero": {
            "before": invalid_before,
            "after": invalid_after,
            "code": invalid_hops.code,
            "message": invalid_hops.message,
        },
        "physical_graph_files": physical_files(&vault_dir.join("cf").join("graph")),
        "physical_wal_files": physical_files(&vault_dir.join("wal")),
    });
    fs::write(
        root.join("issue910-graph-hop-readback.json"),
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();
    println!("issue910_fsv_root={}", root.display());
}

fn plan(steps: Vec<PlanStep>) -> CrossModelPlan {
    CrossModelPlan {
        steps,
        estimated_cost_ms: 1.0,
        explain: None,
    }
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

fn put_order(vault: &AsterVault, collection: &Collection, pk: u64, qty: i64) {
    RelationalLayer::new(vault)
        .put_record(
            collection,
            &RecordKey::from_u64(pk),
            &Row::new([("qty", RecordValue::I64(qty))]),
        )
        .unwrap();
}

fn rows_json(rows: &[crate::query::ProvenancedRow]) -> Vec<serde_json::Value> {
    rows.iter()
        .map(|row| {
            json!({
                "key_hex": hex(row.key.as_bytes()),
                "value": row.value,
                "score": row.score,
                "ledger_ref": row.ledger_ref,
            })
        })
        .collect()
}

fn raw_state(vault: &AsterVault) -> serde_json::Value {
    json!({
        "latest_seq": vault.latest_seq(),
        "relational_rows": vault.scan_cf_at(vault.latest_seq(), ColumnFamily::Relational).unwrap().len(),
        "kv_rows": vault.scan_cf_at(vault.latest_seq(), ColumnFamily::Kv).unwrap().len(),
        "timeseries_rows": vault.scan_cf_at(vault.latest_seq(), ColumnFamily::TimeSeries).unwrap().len(),
        "ledger_rows": vault.scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger).unwrap().len(),
    })
}

fn raw_graph_state(vault: &AsterVault) -> serde_json::Value {
    let rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Graph)
        .unwrap();
    json!({
        "latest_seq": vault.latest_seq(),
        "graph_rows": rows.len(),
        "rows": rows.into_iter().map(|(key, value)| {
            json!({"key_hex": hex(&key), "value_hex": hex(&value)})
        }).collect::<Vec<_>>(),
    })
}

fn cx(byte: u8) -> CxId {
    CxId::from_bytes([byte; 16])
}

fn cx_rows(rows: &[crate::query::ProvenancedRow]) -> Vec<CxId> {
    rows.iter()
        .map(|row| CxId::from_bytes(row.key.as_bytes().try_into().unwrap()))
        .collect()
}

fn cx_json(ids: &[CxId]) -> Vec<String> {
    ids.iter().map(ToString::to_string).collect()
}

fn expired_kv_value(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(9 + payload.len());
    out.push(1);
    out.extend_from_slice(&1_u64.to_be_bytes());
    out.extend_from_slice(payload);
    out
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

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn physical_files(dir: &Path) -> Vec<String> {
    let mut files = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + value - 10),
        _ => unreachable!("nibble out of range"),
    }
}
