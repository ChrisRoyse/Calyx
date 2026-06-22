use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::collection::{
    Collection, CollectionMode, DedupPolicy, FieldDef, FieldType, RetentionPolicy, TemporalPolicy,
    TenantId, TxnPolicy,
};
use calyx_aster::layers::kv::kv_key;
use calyx_aster::layers::{KvLayer, RecordKey, RecordValue, RelationalLayer, Row, TimeSeriesLayer};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, LensId, VaultId};
use serde_json::json;

use crate::query::{AggOp, AggSpec, CrossModelPlan, FieldOp, FieldPredicate, PlanStep, execute};

use super::execute_at_snapshot;

#[test]
#[ignore = "manual FSV for issue #465"]
fn issue465_query_executor_fsv_writes_readback_artifacts() {
    let root = std::env::var_os("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .expect("set CALYX_FSV_ROOT to the issue #465 FSV directory");
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
        }]),
    )
    .unwrap();
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
        "edge_graph_stub_keys": rows_json(&graph.rows),
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
