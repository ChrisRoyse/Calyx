use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::collection::{
    Collection, CollectionMode, DedupPolicy, RetentionPolicy, TemporalPolicy, TenantId, TxnPolicy,
};
use calyx_aster::layers::{RecordKey, RecordValue, RelationalLayer, Row};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::VaultId;
use serde_json::json;

use crate::query::{AskSpec, FieldOp, FieldPredicate, KvLookup, RelationalFilter, UniversalQuery};

use super::plan;

#[test]
#[ignore = "manual FSV for issue #464"]
fn issue464_query_planner_fsv_writes_readback_artifacts() {
    let root = std::env::var_os("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .expect("set CALYX_FSV_ROOT to the issue #464 FSV directory");
    fs::remove_dir_all(&root).ok();
    fs::create_dir_all(&root).unwrap();
    let vault_dir = root.join("vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue464-query-planner-fsv-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let collection = collection("orders", CollectionMode::Records);
    let before_rows = relational_rows(&vault);
    println!("[BEFORE] relational rows = {}", before_rows.len());

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
    vault.flush().unwrap();
    let after_rows = relational_rows(&vault);

    let query = UniversalQuery {
        relational: Some(RelationalFilter {
            collection: collection.clone(),
            predicates: vec![FieldPredicate {
                field: "qty".to_string(),
                op: FieldOp::Gte,
                value: json!(1),
            }],
            estimated_rows: None,
        }),
        kv: Some(KvLookup {
            ns: "sessions".to_string(),
            key: b"sess-1".to_vec(),
        }),
        explain: true,
        cost_cap_ms: Some(100),
        ..UniversalQuery::default()
    };
    let happy = plan(&vault, &query).unwrap();
    let empty = plan(&vault, &UniversalQuery::default()).unwrap();
    let ask_only = plan(
        &vault,
        &UniversalQuery {
            ask: Some(AskSpec {
                question: "which orders need review?".to_string(),
                context_cx_ids: Vec::new(),
                top_k: 10,
                oracle: false,
            }),
            ..UniversalQuery::default()
        },
    )
    .unwrap();
    let cap_zero = plan(
        &vault,
        &UniversalQuery {
            kv: query.kv.clone(),
            cost_cap_ms: Some(0),
            ..UniversalQuery::default()
        },
    )
    .unwrap_err();
    let unbounded = plan(
        &vault,
        &UniversalQuery {
            relational: Some(RelationalFilter {
                collection,
                predicates: query.relational.as_ref().unwrap().predicates.clone(),
                estimated_rows: Some(1_000_000),
            }),
            ..UniversalQuery::default()
        },
    )
    .unwrap_err();

    println!(
        "[AFTER ] relational rows = {}; happy steps = {:?}; cost = {:.1}",
        after_rows.len(),
        happy
            .steps
            .iter()
            .map(|step| step.kind())
            .collect::<Vec<_>>(),
        happy.estimated_cost_ms
    );
    println!(
        "[EDGE empty] steps = {}, cost = {:.1}",
        empty.steps.len(),
        empty.estimated_cost_ms
    );
    println!("[EDGE ask_only] kind = {:?}", ask_only.steps[0].kind());
    println!("[EDGE cap_zero] error = {}", cap_zero.code);
    println!("[EDGE unbounded] error = {}", unbounded.code);

    let readback = json!({
        "before_relational_rows": before_rows.len(),
        "after_relational_rows": after_rows.len(),
        "relational_row_keys_hex": after_rows.iter().map(|(key, _)| hex_bytes(key)).collect::<Vec<_>>(),
        "happy_plan": happy,
        "edge_empty": empty,
        "edge_ask_only": ask_only,
        "edge_cap_zero_error": {
            "code": cap_zero.code,
            "message": cap_zero.message,
        },
        "edge_unbounded_error": {
            "code": unbounded.code,
            "message": unbounded.message,
        },
        "physical_relational_cf_files": physical_files(&vault_dir.join("cf").join("relational")),
    });
    fs::write(
        root.join("issue464-query-planner-readback.json"),
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();
    println!("issue464_fsv_root={}", root.display());
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

fn relational_rows(vault: &AsterVault) -> Vec<(Vec<u8>, Vec<u8>)> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Relational)
        .unwrap()
}

fn physical_files(dir: &Path) -> Vec<String> {
    let mut files = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn hex_bytes(bytes: &[u8]) -> String {
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
