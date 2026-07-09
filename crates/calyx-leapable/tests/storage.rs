//! End-to-end stdio tests for Leapable storage-layer RPCs.

use calyx_aster::layers::blob::BLOB_CHUNK_SIZE;
use serde_json::{Value, json};

use storage_support::{
    TestRoot, assert_calyx_code, assert_no_json_on_stderr, hex, json_lines, request, run_engine,
    storage_dir, wal_files,
};

mod storage_support;

#[test]
fn storage_layers_round_trip_and_txn_rolls_back_staged_writes() {
    let root = TestRoot::new("round-trip");
    let vault_ref = "storage";
    let vault_dir = storage_dir(&root.path, vault_ref);
    assert!(!vault_dir.exists(), "before: vault bytes absent");

    let large_blob = "z".repeat(BLOB_CHUNK_SIZE + 1);
    let blob_hash = blake3::hash(large_blob.as_bytes());
    let blob_id = hex(&blob_hash.as_bytes()[..16]);

    let input = [
        request(
            1,
            "vault.create",
            json!({"vault_ref": vault_ref, "ts": 1_785_600_000_000_u64}),
        ),
        request(
            2,
            "rel.insert",
            json!({
                "vault_ref": vault_ref,
                "ts": 1_785_600_001_000_u64,
                "collection_name": "orders",
                "collection": orders_collection(),
                "pk": {"u64": 1},
                "row": order_row("bolt", 3, 1_785_600_001_000_i64)
            }),
        ),
        request(
            3,
            "rel.get",
            json!({"vault_ref": vault_ref, "ts": 1_785_600_002_000_u64, "collection_name": "orders", "pk": {"u64": 1}}),
        ),
        request(
            4,
            "rel.update_row",
            json!({
                "vault_ref": vault_ref,
                "ts": 1_785_600_003_000_u64,
                "collection_name": "orders",
                "pk": {"u64": 1},
                "set": {"qty": {"i64": 5}, "updated": {"timestamp": 1_785_600_003_000_i64}}
            }),
        ),
        request(
            5,
            "rel.insert",
            json!({
                "vault_ref": vault_ref,
                "ts": 1_785_600_004_000_u64,
                "collection_name": "orders",
                "pk": {"u64": 2},
                "row": order_row("nut", 9, 1_785_600_004_000_i64)
            }),
        ),
        request(
            6,
            "rel.query",
            json!({
                "vault_ref": vault_ref,
                "ts": 1_785_600_005_000_u64,
                "collection_name": "orders",
                "index_name": "qty_idx",
                "gte": {"i64": 5},
                "lte": {"i64": 9},
                "limit": 10
            }),
        ),
        request(
            7,
            "rel.scan",
            json!({"vault_ref": vault_ref, "ts": 1_785_600_006_000_u64, "collection_name": "orders", "limit": 10}),
        ),
        request(
            8,
            "kv.set",
            json!({
                "vault_ref": vault_ref,
                "ts": 1_785_600_007_000_u64,
                "collection_name": "locks",
                "collection": {},
                "ns": 7,
                "key": {"text": "doc-1"},
                "value": {"text": "held"},
                "ttl_ms": 1_000
            }),
        ),
        request(
            9,
            "kv.get",
            json!({"vault_ref": vault_ref, "ts": 1_785_600_007_500_u64, "collection_name": "locks", "ns": 7, "key": {"text": "doc-1"}, "include_text": true}),
        ),
        request(
            10,
            "kv.get",
            json!({"vault_ref": vault_ref, "ts": 1_785_600_009_000_u64, "collection_name": "locks", "ns": 7, "key": {"text": "doc-1"}}),
        ),
        request(
            11,
            "kv.set",
            json!({
                "vault_ref": vault_ref,
                "ts": 1_785_600_010_000_u64,
                "collection_name": "locks",
                "ns": 7,
                "key": {"text": "durable"},
                "value": {"text": "alpha"}
            }),
        ),
        request(
            12,
            "kv.get",
            json!({"vault_ref": vault_ref, "ts": 1_785_600_011_000_u64, "collection_name": "locks", "ns": 7, "key": {"text": "durable"}, "include_text": true}),
        ),
        request(
            13,
            "kv.delete",
            json!({"vault_ref": vault_ref, "ts": 1_785_600_012_000_u64, "collection_name": "locks", "ns": 7, "key": {"text": "durable"}}),
        ),
        request(
            14,
            "kv.get",
            json!({"vault_ref": vault_ref, "ts": 1_785_600_013_000_u64, "collection_name": "locks", "ns": 7, "key": {"text": "durable"}}),
        ),
        request(
            15,
            "ts.write",
            json!({"vault_ref": vault_ref, "ts": 1_785_600_014_000_u64, "collection_name": "metrics", "collection": {}, "series": 42, "point_ts": 100, "value": 1.5}),
        ),
        request(
            16,
            "ts.write",
            json!({"vault_ref": vault_ref, "ts": 1_785_600_015_000_u64, "collection_name": "metrics", "series": 42, "point_ts": 200, "value": 2.5}),
        ),
        request(
            17,
            "ts.range",
            json!({"vault_ref": vault_ref, "ts": 1_785_600_016_000_u64, "collection_name": "metrics", "series": 42, "start_ts": 50, "end_ts": 250}),
        ),
        request(
            18,
            "blob.put",
            json!({"vault_ref": vault_ref, "ts": 1_785_600_017_000_u64, "collection_name": "payloads", "collection": {}, "input": {"text": large_blob}}),
        ),
        request(
            19,
            "blob.get",
            json!({"vault_ref": vault_ref, "ts": 1_785_600_018_000_u64, "collection_name": "payloads", "blob_id": blob_id, "include_data": true}),
        ),
        request(
            20,
            "txn.commit",
            json!({
                "vault_ref": vault_ref,
                "ts": 1_785_600_019_000_u64,
                "cost_cap_ms": 1_000,
                "ops": [
                    {"op": "rel.insert", "collection_name": "orders", "pk": {"u64": 3}, "row": order_row("washer", 11, 1_785_600_019_000_i64)},
                    {"op": "kv.set", "collection_name": "locks", "ns": 7, "key": {"text": "txn-ok"}, "value": {"text": "yes"}},
                    {"op": "ts.write", "collection_name": "metrics", "series": 42, "point_ts": 300, "value": 3.5}
                ]
            }),
        ),
        request(
            21,
            "rel.get",
            json!({"vault_ref": vault_ref, "ts": 1_785_600_020_000_u64, "collection_name": "orders", "pk": {"u64": 3}}),
        ),
        request(
            22,
            "kv.get",
            json!({"vault_ref": vault_ref, "ts": 1_785_600_021_000_u64, "collection_name": "locks", "ns": 7, "key": {"text": "txn-ok"}, "include_text": true}),
        ),
        request(
            23,
            "ts.range",
            json!({"vault_ref": vault_ref, "ts": 1_785_600_022_000_u64, "collection_name": "metrics", "series": 42, "start_ts": 300, "end_ts": 300}),
        ),
        request(
            24,
            "txn.commit",
            json!({
                "vault_ref": vault_ref,
                "ts": 1_785_600_023_000_u64,
                "cost_cap_ms": 1_000,
                "inject_crash_after_stage": true,
                "ops": [
                    {"op": "rel.insert", "collection_name": "orders", "pk": {"u64": 4}, "row": order_row("failed", 12, 1_785_600_023_000_i64)},
                    {"op": "kv.set", "collection_name": "locks", "ns": 7, "key": {"text": "txn-fail"}, "value": {"text": "no"}},
                    {"op": "ts.write", "collection_name": "metrics", "series": 42, "point_ts": 400, "value": 4.5}
                ]
            }),
        ),
        request(
            25,
            "rel.get",
            json!({"vault_ref": vault_ref, "ts": 1_785_600_024_000_u64, "collection_name": "orders", "pk": {"u64": 4}}),
        ),
        request(
            26,
            "kv.get",
            json!({"vault_ref": vault_ref, "ts": 1_785_600_025_000_u64, "collection_name": "locks", "ns": 7, "key": {"text": "txn-fail"}}),
        ),
        request(
            27,
            "ts.range",
            json!({"vault_ref": vault_ref, "ts": 1_785_600_026_000_u64, "collection_name": "metrics", "series": 42, "start_ts": 400, "end_ts": 400}),
        ),
        request(
            28,
            "rel.delete",
            json!({"vault_ref": vault_ref, "ts": 1_785_600_027_000_u64, "collection_name": "orders", "pk": {"u64": 2}}),
        ),
        request(
            29,
            "rel.get",
            json!({"vault_ref": vault_ref, "ts": 1_785_600_028_000_u64, "collection_name": "orders", "pk": {"u64": 2}}),
        ),
        request(
            30,
            "rel.query",
            json!({"vault_ref": vault_ref, "ts": 1_785_600_029_000_u64, "collection_name": "orders", "index_name": "qty_idx", "gte": {"i64": 9}, "lte": {"i64": 12}, "limit": 10}),
        ),
    ]
    .concat();

    let (stdout, stderr, ok) = run_engine(&input, &root.path);
    let responses = json_lines(&stdout);
    assert_eq!(responses.len(), 30);
    for (idx, response) in responses.iter().enumerate() {
        if idx != 23 {
            assert!(response.get("error").is_none(), "{response}");
        }
    }
    assert_calyx_code(&responses[23], "CALYX_LEAPABLE_TXN_INJECTED_CRASH");
    assert_no_json_on_stderr(&stderr);
    assert!(ok);

    assert_eq!(responses[2]["result"]["row"]["qty"]["i64"], 3);
    assert_eq!(responses[3]["result"]["row"]["qty"]["i64"], 5);
    assert_eq!(responses[5]["result"]["items"].as_array().unwrap().len(), 2);
    assert_eq!(responses[5]["result"]["items"][0]["row"]["qty"]["i64"], 5);
    assert_eq!(responses[5]["result"]["items"][1]["row"]["qty"]["i64"], 9);
    assert_eq!(responses[6]["result"]["items"].as_array().unwrap().len(), 2);
    assert!(responses[7]["result"].get("value_hex").is_none());
    assert_eq!(responses[7]["result"]["value_len"], 4);
    assert_eq!(responses[8]["result"]["status"], "found");
    assert_eq!(responses[8]["result"]["value"]["text"], "held");
    assert_eq!(responses[9]["result"]["status"], "absent");
    assert_eq!(responses[11]["result"]["value"]["text"], "alpha");
    assert_eq!(responses[13]["result"]["status"], "absent");
    assert_eq!(
        responses[16]["result"]["points"].as_array().unwrap().len(),
        2
    );
    assert_eq!(responses[16]["result"]["points"][0]["value"], 1.5);
    assert_eq!(responses[17]["result"]["blob_id"], blob_id);
    assert_eq!(responses[17]["result"]["chunk_count"], 2);
    assert_eq!(
        responses[17]["result"]["content_hash"],
        hex(blob_hash.as_bytes())
    );
    assert_eq!(responses[18]["result"]["manifest"]["chunk_count"], 2);
    assert_eq!(responses[18]["result"]["data"]["len"], BLOB_CHUNK_SIZE + 1);
    assert_eq!(responses[20]["result"]["status"], "found");
    assert_eq!(responses[20]["result"]["row"]["sku"]["text"], "washer");
    assert_eq!(responses[21]["result"]["value"]["text"], "yes");
    assert_eq!(
        responses[22]["result"]["points"].as_array().unwrap().len(),
        1
    );
    assert_eq!(responses[24]["result"]["status"], "absent");
    assert_eq!(responses[25]["result"]["status"], "absent");
    assert_eq!(
        responses[26]["result"]["points"].as_array().unwrap().len(),
        0
    );
    assert_eq!(responses[28]["result"]["status"], "absent");
    let post_delete = responses[29]["result"]["items"].as_array().unwrap();
    assert_eq!(post_delete.len(), 1);
    assert_eq!(post_delete[0]["row"]["sku"]["text"], "washer");

    assert!(vault_dir.join("cf/relational").exists());
    assert!(vault_dir.join("cf/kv").exists());
    assert!(vault_dir.join("cf/timeseries").exists());
    assert!(vault_dir.join("cf/blob").exists());
    assert!(vault_dir.join("cf/index_btree").exists());
    assert!(vault_dir.join("cf/ledger").exists());
    assert!(!wal_files(&vault_dir).is_empty(), "WAL bytes were written");
}

#[test]
fn storage_rpc_edges_fail_closed() {
    let root = TestRoot::new("edges");
    let input = [
        request(
            1,
            "rel.get",
            json!({"vault_ref": "ghost", "ts": 1_785_700_000_000_u64, "collection_name": "strict", "pk": {"u64": 1}}),
        ),
        request(
            2,
            "vault.create",
            json!({"vault_ref": "edges", "ts": 1_785_700_001_000_u64}),
        ),
        request(
            3,
            "rel.insert",
            json!({
                "vault_ref": "edges",
                "ts": 1_785_700_002_000_u64,
                "collection_name": "strict",
                "collection": strict_collection(),
                "pk": {"u64": 1, "text": "ambiguous"},
                "row": {"qty": {"i64": 1}}
            }),
        ),
        request(
            4,
            "rel.get",
            json!({"vault_ref": "edges", "ts": 1_785_700_003_000_u64, "collection_name": "strict", "pk": {"u64": 1}}),
        ),
        request(
            5,
            "rel.insert",
            json!({
                "vault_ref": "edges",
                "ts": 1_785_700_004_000_u64,
                "collection_name": "strict",
                "collection": strict_collection(),
                "pk": {"u64": 1},
                "row": {"qty": {"i64": 1}}
            }),
        ),
        request(
            6,
            "rel.insert",
            json!({
                "vault_ref": "edges",
                "ts": 1_785_700_005_000_u64,
                "collection_name": "strict",
                "pk": {"u64": 2},
                "row": {"qty": {"text": "wrong"}}
            }),
        ),
        request(
            7,
            "kv.get",
            json!({"vault_ref": "edges", "ts": 1_785_700_006_000_u64, "collection_name": "strict", "key": {"text": "not-kv"}}),
        ),
        request(
            8,
            "rel.query",
            json!({"vault_ref": "edges", "ts": 1_785_700_007_000_u64, "collection_name": "strict", "index_name": "missing_idx", "gte": {"i64": 1}, "limit": 10}),
        ),
        request(
            9,
            "blob.put",
            json!({"vault_ref": "edges", "ts": 1_785_700_008_000_u64, "collection_name": "payloads", "collection": {}, "input": {"text": "edge"}}),
        ),
        request(
            10,
            "blob.get",
            json!({"vault_ref": "edges", "ts": 1_785_700_009_000_u64, "collection_name": "payloads", "blob_id": "abc"}),
        ),
        request(
            11,
            "rel.scan",
            json!({"vault_ref": "edges", "ts": 1_785_700_010_000_u64, "collection_name": "strict", "limit": 0}),
        ),
    ]
    .concat();

    let (stdout, stderr, ok) = run_engine(&input, &root.path);
    let responses = json_lines(&stdout);
    assert_eq!(responses.len(), 11);
    assert_calyx_code(&responses[0], "CALYX_LEAPABLE_VAULT_NOT_OPEN");
    assert!(responses[1].get("error").is_none());
    assert_calyx_code(&responses[2], "CALYX_LEAPABLE_STORAGE_INPUT_INVALID");
    assert_calyx_code(&responses[3], "CALYX_COLLECTION_NOT_FOUND");
    assert!(responses[4].get("error").is_none());
    assert_calyx_code(&responses[5], "CALYX_SCHEMA_VIOLATION");
    assert_calyx_code(&responses[6], "CALYX_LEAPABLE_COLLECTION_MISMATCH");
    assert_calyx_code(&responses[7], "CALYX_LEAPABLE_INDEX_NOT_FOUND");
    assert!(responses[8].get("error").is_none());
    assert_calyx_code(&responses[9], "CALYX_LEAPABLE_STORAGE_INPUT_INVALID");
    assert_calyx_code(&responses[10], "CALYX_LEAPABLE_STORAGE_INPUT_INVALID");
    assert_no_json_on_stderr(&stderr);
    assert!(ok);
}

fn orders_collection() -> Value {
    json!({
        "schema": [
            {"name": "sku", "ty": "text"},
            {"name": "qty", "ty": "i64"},
            {"name": "updated", "ty": "timestamp"}
        ],
        "indexes": [{"name": "qty_idx", "kind": "btree", "fields": ["qty"]}]
    })
}

fn strict_collection() -> Value {
    json!({"schema": [{"name": "qty", "ty": "i64"}]})
}

fn order_row(sku: &str, qty: i64, updated: i64) -> Value {
    json!({
        "sku": {"text": sku},
        "qty": {"i64": qty},
        "updated": {"timestamp": updated}
    })
}
