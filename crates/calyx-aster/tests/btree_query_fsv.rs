//! FSV for PH54 T02 btree range/point/count queries against a real `AsterVault`.
//!
//! Source of truth: the `index_btree` and `relational` column families in the
//! vault's MVCC row store. We write rows + index entries, then perform separate
//! read-backs (raw CF scan for the physical bytes; `btree_range`/`point`/`count`
//! for the query path) and assert hand-computed expectations. Run:
//!
//! ```text
//! cargo test -p calyx-aster --test btree_query_fsv -- --nocapture
//! ```

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
use calyx_aster::vault::AsterVault;
use calyx_core::{FixedClock, VaultId};

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
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(10));
    let layer = RelationalLayer::new(&vault);
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
        layer.put_record(&col, &pk, &row).unwrap();
        btree_index_put(&vault, &col, &spec, &RecordValue::I64(qty), &pk).unwrap();
    }

    // --- SoT read-back #1: raw index_btree CF bytes (physical proof) ---------
    let raw = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::IndexBtree)
        .unwrap();
    println!("index_btree CF holds {} keys:", raw.len());
    for (key, val) in &raw {
        println!("  key={} val_len={}", hex(key), val.len());
        assert_eq!(key[0], 0x10, "btree index discriminant");
        assert!(
            val.is_empty(),
            "index value must be empty (existence is signal)"
        );
    }
    assert_eq!(raw.len(), 5);
    // Keys are stored in ascending order ⇒ ascending qty.
    let stored_order: Vec<Vec<u8>> = raw.iter().map(|(k, _)| k.clone()).collect();
    let mut sorted = stored_order.clone();
    sorted.sort();
    assert_eq!(
        stored_order, sorted,
        "index_btree CF physically sorted ascending"
    );

    // --- SoT read-back #2: query path (2+2=4 synthetic known I/O) ------------
    let range = btree_range(
        &vault,
        &col,
        &spec,
        Some(&RecordValue::I64(3)),
        Some(&RecordValue::I64(7)),
        0,
    )
    .unwrap();
    println!(
        "range(gte=3,lte=7) -> pks {:?} (expected [3,5,7])",
        pks(&range)
    );
    assert_eq!(pks(&range), vec![3, 5, 7]);

    let point = btree_point(&vault, &col, &spec, &RecordValue::I64(5)).unwrap();
    println!("point(5) -> pks {:?} (expected [5])", pks(&point));
    assert_eq!(pks(&point), vec![5]);

    let count = btree_count(
        &vault,
        &col,
        &spec,
        Some(&RecordValue::I64(1)),
        Some(&RecordValue::I64(9)),
    )
    .unwrap();
    println!("count(1..=9) -> {count} (expected 5)");
    assert_eq!(count, 5);

    // --- Edge 1: no records match -> empty -----------------------------------
    let none = btree_range(
        &vault,
        &col,
        &spec,
        Some(&RecordValue::I64(100)),
        Some(&RecordValue::I64(200)),
        0,
    )
    .unwrap();
    println!("Edge[range 100..=200] -> {:?} (expected [])", pks(&none));
    assert!(none.is_empty());

    // --- Edge 2: limit=2 over 5 matching -> first 2 --------------------------
    let limited = btree_range(
        &vault,
        &col,
        &spec,
        Some(&RecordValue::I64(1)),
        Some(&RecordValue::I64(9)),
        2,
    )
    .unwrap();
    println!(
        "Edge[range 1..=9 limit 2] -> {:?} (expected [1,3])",
        pks(&limited)
    );
    assert_eq!(pks(&limited), vec![1, 3]);

    // --- Edge 3: stale index entry (index key present, data row absent) ------
    // Write an index entry for qty=11/pk=11 WITHOUT a matching data row.
    let stale_pk = RecordKey::from_u64(11);
    btree_index_put(&vault, &col, &spec, &RecordValue::I64(11), &stale_pk).unwrap();
    let snap = vault.latest_seq();
    let raw_after = vault.scan_cf_at(snap, ColumnFamily::IndexBtree).unwrap();
    // Independent data-CF read-back: pk=5 present, pk=11 absent (the SoT proof).
    let live = vault
        .read_cf_at(
            snap,
            ColumnFamily::Relational,
            &relational::record_key(&col, &RecordKey::from_u64(5)).unwrap(),
        )
        .unwrap();
    let stale = vault
        .read_cf_at(
            snap,
            ColumnFamily::Relational,
            &relational::record_key(&col, &stale_pk).unwrap(),
        )
        .unwrap();
    println!(
        "Edge[stale] index_btree CF now holds {} keys (was 5); data row pk=5 present={}, pk=11 present={}",
        raw_after.len(),
        live.is_some(),
        stale.is_some()
    );
    assert_eq!(
        raw_after.len(),
        6,
        "stale index key physically present in CF"
    );
    assert!(live.is_some() && stale.is_none(), "pk=11 has no data row");
    let with_stale = btree_range(
        &vault,
        &col,
        &spec,
        Some(&RecordValue::I64(1)),
        Some(&RecordValue::I64(20)),
        0,
    )
    .unwrap();
    println!(
        "range(1..=20) after stale insert -> {:?} (expected [1,3,5,7,9], pk=11 skipped)",
        pks(&with_stale)
    );
    assert_eq!(
        pks(&with_stale),
        vec![1, 3, 5, 7, 9],
        "stale entry must be skipped"
    );
    let count_after = btree_count(
        &vault,
        &col,
        &spec,
        Some(&RecordValue::I64(1)),
        Some(&RecordValue::I64(20)),
    )
    .unwrap();
    assert_eq!(
        count_after, 5,
        "count excludes stale entry, agrees with range len"
    );

    // --- Edge 4: point on absent value -> empty ------------------------------
    let absent = btree_point(&vault, &col, &spec, &RecordValue::I64(2)).unwrap();
    println!("Edge[point absent=2] -> {:?} (expected [])", pks(&absent));
    assert!(absent.is_empty());

    println!("=== FSV PASS: index_btree CF is the verified source of truth ===\n");
}
