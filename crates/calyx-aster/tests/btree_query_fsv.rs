//! FSV for PH54 T02 btree range/point/count queries against a real `AsterVault`.
//!
//! Source of truth: the durable `index_btree` and `relational` column-family
//! SSTs. We write rows + index entries, perform separate read-backs (raw CF scan
//! for the physical bytes; `btree_range`/`point`/`count` for the query path),
//! reopen the vault, and assert hand-computed expectations. Run:
//!
//! ```text
//! cargo test -p calyx-aster --test btree_query_fsv -- --nocapture
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::collection::{
    Collection, CollectionMode, DedupPolicy, FieldDef, FieldType, IsolationLevel, RetentionPolicy,
    Schema, TemporalPolicy, TenantId, TxnPolicy,
};
use calyx_aster::index::btree::{
    CF_INDEX_BTREE, btree_count, btree_index_put, btree_point, btree_range,
};
use calyx_aster::index::{IndexId, IndexKind, IndexSpec};
use calyx_aster::layers::relational;
use calyx_aster::layers::{RecordKey, RecordValue, RelationalLayer, Row};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Clock, VaultId};
use serde_json::{Value, json};

mod fsv_support;
use fsv_support::collect_physical_file_states;

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn orders() -> Collection {
    Collection {
        name: "orders".to_string(),
        mode: CollectionMode::Records,
        schema: Some(Schema::SchemaFull(vec![
            FieldDef::new("item", FieldType::Text, false),
            FieldDef::new("qty", FieldType::I64, false),
        ])),
        panel: None,
        indexes: Vec::new(),
        dedup: DedupPolicy::Off,
        temporal: TemporalPolicy::default(),
        retention: RetentionPolicy::Forever,
        txn_policy: TxnPolicy {
            isolation: IsolationLevel::ReadCommitted,
            cost_cap_ms: None,
        },
        tenant: TenantId::default(),
    }
}

fn qty_index() -> IndexSpec {
    IndexSpec::new(
        IndexId::new(1),
        "qty_idx",
        IndexKind::Btree,
        "qty",
        FieldType::I64,
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

fn pks(keys: &[RecordKey]) -> Vec<u64> {
    keys.iter()
        .map(|k| u64::from_be_bytes(k.as_bytes().try_into().expect("8-byte pk")))
        .collect()
}

#[test]
fn fsv_btree_range_point_count_with_stale_skip() {
    let fsv_root = std::env::var_os("CALYX_FSV_ROOT").map(PathBuf::from);
    let dir = fsv_root
        .as_ref()
        .map(|root| root.join("btree-query-vault"))
        .unwrap_or_else(|| std::env::temp_dir().join("calyx-btree-query-fsv-test"));
    fs::remove_dir_all(&dir).ok();

    let salt = b"ph54-t02-btree-query-fsv".to_vec();
    let options = VaultOptions::default();
    let vault = AsterVault::new_durable(&dir, vault_id(), salt.clone(), options.clone()).unwrap();
    let col = orders();
    let spec = qty_index();

    // CF-name registry agrees between the const and the ColumnFamily.
    assert_eq!(CF_INDEX_BTREE, ColumnFamily::IndexBtree.name());

    // --- Trigger X: write 5 rows {qty=1,3,5,7,9} + their index entries -------
    println!("\n=== PH54 T02 btree query FSV ===");
    for qty in [1_i64, 3, 5, 7, 9] {
        let pk = RecordKey::from_u64(qty as u64);
        let row = Row::new([
            ("item", RecordValue::Text("bolt".to_string())),
            ("qty", RecordValue::I64(qty)),
        ]);
        RelationalLayer::new(&vault)
            .put_record(&col, &pk, &row)
            .unwrap();
        btree_index_put(&vault, &col, &spec, &RecordValue::I64(qty), &pk).unwrap();
    }

    let initial = read_initial_state(&vault, &col, &spec);
    assert_eq!(initial["raw_key_count"], json!(5));
    assert_eq!(initial["range_3_7_pks"], json!([3, 5, 7]));
    assert_eq!(initial["point_5_pks"], json!([5]));
    assert_eq!(initial["count_1_9"], json!(5));
    assert_eq!(initial["edges"]["no_match_pks"], json!([]));
    assert_eq!(initial["edges"]["limit_2_pks"], json!([1, 3]));

    // --- Edge 3: stale index entry (index key present, data row absent) ------
    // Write an index entry for qty=11/pk=11 WITHOUT a matching data row.
    let stale_pk = RecordKey::from_u64(11);
    btree_index_put(&vault, &col, &spec, &RecordValue::I64(11), &stale_pk).unwrap();

    let before_restart = read_stale_state(&vault, &col, &spec);
    assert_stale_state(&before_restart);
    vault.flush().unwrap();
    drop(vault);

    let reopened = AsterVault::open(&dir, vault_id(), salt, options).unwrap();
    let after_restart = read_stale_state(&reopened, &col, &spec);
    assert_eq!(after_restart, before_restart);
    assert_stale_state(&after_restart);

    let readback = json!({
        "issue": 458,
        "source_of_truth": dir.display().to_string(),
        "initial": initial,
        "before_restart": before_restart,
        "after_restart": after_restart,
        "cf_files": physical_files(&dir.join("cf")),
    });
    println!("=== FSV PASS: index_btree CF is the verified source of truth ===");
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    if let Some(root) = fsv_root {
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("btree-query-readback.json"),
            serde_json::to_vec_pretty(&readback).unwrap(),
        )
        .unwrap();
    } else {
        fs::remove_dir_all(dir).ok();
    }
}

fn read_initial_state<C: Clock>(
    vault: &AsterVault<C>,
    col: &Collection,
    spec: &IndexSpec,
) -> Value {
    let raw = read_raw_index(vault);
    assert_eq!(raw.len(), 5);
    let range = btree_range(
        vault,
        col,
        spec,
        Some(&RecordValue::I64(3)),
        Some(&RecordValue::I64(7)),
        0,
    )
    .unwrap();
    let point = btree_point(vault, col, spec, &RecordValue::I64(5)).unwrap();
    let count = btree_count(
        vault,
        col,
        spec,
        Some(&RecordValue::I64(1)),
        Some(&RecordValue::I64(9)),
    )
    .unwrap();
    let none = btree_range(
        vault,
        col,
        spec,
        Some(&RecordValue::I64(100)),
        Some(&RecordValue::I64(200)),
        0,
    )
    .unwrap();
    let limited = btree_range(
        vault,
        col,
        spec,
        Some(&RecordValue::I64(1)),
        Some(&RecordValue::I64(9)),
        2,
    )
    .unwrap();
    println!("index_btree CF holds {} initial keys:", raw.len());
    for (key, value_len) in &raw {
        println!("  key={key} val_len={value_len}");
    }
    println!(
        "range(gte=3,lte=7) -> pks {:?} (expected [3,5,7])",
        pks(&range)
    );
    println!("point(5) -> pks {:?} (expected [5])", pks(&point));
    println!("count(1..=9) -> {count} (expected 5)");
    println!("Edge[range 100..=200] -> {:?} (expected [])", pks(&none));
    println!(
        "Edge[range 1..=9 limit 2] -> {:?} (expected [1,3])",
        pks(&limited)
    );

    json!({
        "raw_key_count": raw.len(),
        "raw_keys": raw.iter().map(|(key, _)| key).collect::<Vec<_>>(),
        "raw_value_lens": raw.iter().map(|(_, len)| *len).collect::<Vec<_>>(),
        "range_3_7_pks": pks(&range),
        "point_5_pks": pks(&point),
        "count_1_9": count,
        "edges": {
            "no_match_pks": pks(&none),
            "limit_2_pks": pks(&limited),
        },
    })
}

fn read_stale_state<C: Clock>(vault: &AsterVault<C>, col: &Collection, spec: &IndexSpec) -> Value {
    let snap = vault.latest_seq();
    let raw = read_raw_index(vault);
    let live = vault
        .read_cf_at(
            snap,
            ColumnFamily::Relational,
            &relational::record_key(col, &RecordKey::from_u64(5)).unwrap(),
        )
        .unwrap();
    let stale = vault
        .read_cf_at(
            snap,
            ColumnFamily::Relational,
            &relational::record_key(col, &RecordKey::from_u64(11)).unwrap(),
        )
        .unwrap();
    let with_stale = btree_range(
        vault,
        col,
        spec,
        Some(&RecordValue::I64(1)),
        Some(&RecordValue::I64(20)),
        0,
    )
    .unwrap();
    let count_after = btree_count(
        vault,
        col,
        spec,
        Some(&RecordValue::I64(1)),
        Some(&RecordValue::I64(20)),
    )
    .unwrap();
    let absent = btree_point(vault, col, spec, &RecordValue::I64(2)).unwrap();
    println!(
        "Edge[stale] index_btree CF holds {} keys; data row pk=5 present={}, pk=11 present={}",
        raw.len(),
        live.is_some(),
        stale.is_some()
    );
    println!(
        "range(1..=20) after stale insert -> {:?} (expected [1,3,5,7,9], pk=11 skipped)",
        pks(&with_stale)
    );
    println!("Edge[point absent=2] -> {:?} (expected [])", pks(&absent));

    json!({
        "raw_key_count": raw.len(),
        "raw_keys": raw.iter().map(|(key, _)| key).collect::<Vec<_>>(),
        "raw_value_lens": raw.iter().map(|(_, len)| *len).collect::<Vec<_>>(),
        "pk_5_present": live.is_some(),
        "pk_11_present": stale.is_some(),
        "range_1_20_pks": pks(&with_stale),
        "count_1_20": count_after,
        "point_absent_2_pks": pks(&absent),
    })
}

fn read_raw_index<C: Clock>(vault: &AsterVault<C>) -> Vec<(String, usize)> {
    let raw = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::IndexBtree)
        .unwrap();
    let stored_order: Vec<Vec<u8>> = raw.iter().map(|(key, _)| key.clone()).collect();
    let mut sorted = stored_order.clone();
    sorted.sort();
    assert_eq!(
        stored_order, sorted,
        "index_btree CF physically sorted ascending"
    );
    raw.into_iter()
        .map(|(key, value)| {
            assert_eq!(key[0], 0x10, "btree index discriminant");
            assert!(
                value.is_empty(),
                "index value must be empty (existence is signal)"
            );
            (hex(&key), value.len())
        })
        .collect()
}

fn assert_stale_state(value: &Value) {
    assert_eq!(value["raw_key_count"], json!(6));
    assert_eq!(value["pk_5_present"], json!(true));
    assert_eq!(value["pk_11_present"], json!(false));
    assert_eq!(value["range_1_20_pks"], json!([1, 3, 5, 7, 9]));
    assert_eq!(value["count_1_20"], json!(5));
    assert_eq!(value["point_absent_2_pks"], json!([]));
}

fn physical_files(root: &Path) -> Vec<Value> {
    let mut files = Vec::new();
    if !root.exists() {
        return files;
    }
    collect_physical_file_states(root, &mut files);
    files.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    files
}
