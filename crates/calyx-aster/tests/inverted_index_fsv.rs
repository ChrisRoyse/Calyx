//! FSV for PH54 T03 inverted term-match and BM25 against a durable AsterVault.

use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::collection::{
    Collection, CollectionMode, DedupPolicy, FieldDef, FieldType, IsolationLevel, RetentionPolicy,
    Schema, TemporalPolicy, TenantId, TxnPolicy,
};
use calyx_aster::index::inverted::{
    CF_INDEX_INVERTED, inverted_bm25, inverted_bm25_and, inverted_match, inverted_put,
    inverted_stats,
};
use calyx_aster::index::{IndexId, IndexKind, IndexSpec};
use calyx_aster::layers::{RecordKey, RecordValue, RelationalLayer, Row};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Clock, VaultId};
use serde_json::{Value, json};

mod fsv_support;
use fsv_support::collect_physical_file_states;

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn texts() -> Collection {
    Collection {
        name: "text_col".to_string(),
        mode: CollectionMode::Records,
        schema: Some(Schema::SchemaFull(vec![FieldDef::new(
            "body",
            FieldType::Text,
            false,
        )])),
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

fn body_index() -> IndexSpec {
    IndexSpec::new(
        IndexId::new(1),
        "body",
        IndexKind::Inverted,
        "body",
        FieldType::Text,
    )
}

#[test]
fn fsv_inverted_index_term_match_bm25_and_edges() {
    let fsv_root = std::env::var_os("CALYX_FSV_ROOT").map(PathBuf::from);
    let dir = fsv_root
        .as_ref()
        .map(|root| root.join("inverted-index-vault"))
        .unwrap_or_else(|| std::env::temp_dir().join("calyx-inverted-index-fsv-test"));
    fs::remove_dir_all(&dir).ok();

    let salt = b"ph54-t03-inverted-index-fsv".to_vec();
    let options = VaultOptions::default();
    let vault = AsterVault::new_durable(&dir, vault_id(), salt.clone(), options.clone()).unwrap();
    let col = texts();
    let spec = body_index();
    assert_eq!(CF_INDEX_INVERTED, ColumnFamily::IndexInverted.name());

    println!("\n=== PH54 T03 inverted index FSV ===");
    write_doc(&vault, &col, &spec, 1, "the quick brown fox");
    write_doc(&vault, &col, &spec, 2, "quick lazy dog");
    let initial = read_state("initial", &vault, &col, &spec);
    assert_eq!(initial["quick_sorted_pks"], json!([1, 2]));
    assert_eq!(initial["fox_pks"], json!([1]));
    assert_eq!(initial["bm25_or_pks"], json!([1, 2]));
    assert_eq!(initial["bm25_and_pks"], json!([1]));

    let before_empty = raw_index(&vault);
    write_doc(&vault, &col, &spec, 4, "");
    let after_empty = raw_index(&vault);
    assert_eq!(after_empty.len(), before_empty.len());

    let long_text = (0..5000)
        .map(|idx| format!("long{idx:04}"))
        .collect::<Vec<_>>()
        .join(" ");
    write_doc(&vault, &col, &spec, 3, &long_text);
    write_doc(&vault, &col, &spec, 5, "same text");
    write_doc(&vault, &col, &spec, 6, "same text");

    let before_invalid = raw_index(&vault);
    let invalid = inverted_put(
        &vault,
        &col,
        &spec,
        &RecordValue::I64(7),
        &RecordKey::from_u64(7),
    )
    .unwrap_err();
    let after_invalid = raw_index(&vault);
    assert_eq!(invalid.code, "CALYX_INVALID_ARGUMENT");
    assert_eq!(after_invalid.len(), before_invalid.len());

    let before_restart = read_state("before_restart", &vault, &col, &spec);
    assert_edges(&before_restart);
    vault.flush().unwrap();
    drop(vault);

    let reopened = AsterVault::open(&dir, vault_id(), salt, options).unwrap();
    let after_restart = read_state("after_restart", &reopened, &col, &spec);
    assert_eq!(after_restart, before_restart);
    assert_edges(&after_restart);

    let readback = json!({
        "issue": 459,
        "source_of_truth": dir.display().to_string(),
        "initial": initial,
        "before_restart": before_restart,
        "after_restart": after_restart,
        "edges": {
            "empty_field_raw_count_before": before_empty.len(),
            "empty_field_raw_count_after": after_empty.len(),
            "invalid_error_code": invalid.code,
            "invalid_raw_count_before": before_invalid.len(),
            "invalid_raw_count_after": after_invalid.len(),
        },
        "cf_files": physical_files(&dir.join("cf")),
    });
    println!("=== FSV PASS: index_inverted CF is the verified source of truth ===");
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    if let Some(root) = fsv_root {
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("inverted-index-readback.json"),
            serde_json::to_vec_pretty(&readback).unwrap(),
        )
        .unwrap();
    } else {
        fs::remove_dir_all(dir).ok();
    }
}

fn write_doc<C: Clock>(
    vault: &AsterVault<C>,
    col: &Collection,
    spec: &IndexSpec,
    pk: u64,
    text: &str,
) {
    let key = RecordKey::from_u64(pk);
    let row = Row::new([("body", RecordValue::Text(text.to_string()))]);
    RelationalLayer::new(vault)
        .put_record(col, &key, &row)
        .unwrap();
    inverted_put(vault, col, spec, &RecordValue::Text(text.to_string()), &key).unwrap();
}

fn read_state<C: Clock>(
    label: &str,
    vault: &AsterVault<C>,
    col: &Collection,
    spec: &IndexSpec,
) -> Value {
    let raw = raw_index(vault);
    let quick = inverted_match(vault, col, spec, "quick").unwrap();
    let fox = inverted_match(vault, col, spec, "fox").unwrap();
    let missing = inverted_match(vault, col, spec, "missing").unwrap();
    let long = inverted_match(vault, col, spec, "long4999").unwrap();
    let same = inverted_match(vault, col, spec, "same").unwrap();
    let stats = inverted_stats(vault, col, spec).unwrap();
    let bm25_or = inverted_bm25(vault, col, spec, &["quick", "fox"], stats.doc_count, 10).unwrap();
    let bm25_and =
        inverted_bm25_and(vault, col, spec, &["quick", "fox"], stats.doc_count, 10).unwrap();
    let mut quick_sorted = pk_nums(&quick);
    quick_sorted.sort_unstable();

    println!("{label}: index_inverted raw rows = {}", raw.len());
    println!(
        "{label}: quick ranked {:?}, sorted {:?}",
        pk_nums(&quick),
        quick_sorted
    );
    println!(
        "{label}: bm25 OR {:?}, AND {:?}",
        pk_nums(&bm25_or),
        pk_nums(&bm25_and)
    );
    println!(
        "{label}: stats doc_count={} avgdl={}",
        stats.doc_count, stats.avgdl
    );

    json!({
        "raw_entry_count": raw.len(),
        "raw_first_key": raw.first().map(|(key, _)| key.clone()),
        "raw_last_key": raw.last().map(|(key, _)| key.clone()),
        "posting_value_len_count": raw.iter().filter(|(_, len)| *len == 4).count(),
        "stats_value_len_count": raw.iter().filter(|(_, len)| *len == 12).count(),
        "quick_ranked_pks": pk_nums(&quick),
        "quick_sorted_pks": quick_sorted,
        "quick_weights": weights(&quick),
        "fox_pks": pk_nums(&fox),
        "missing_pks": pk_nums(&missing),
        "long4999_pks": pk_nums(&long),
        "same_pks": pk_nums(&same),
        "bm25_or_pks": pk_nums(&bm25_or),
        "bm25_or_scores": weights(&bm25_or),
        "bm25_and_pks": pk_nums(&bm25_and),
        "stats_doc_count": stats.doc_count,
        "stats_avgdl": stats.avgdl,
    })
}

fn raw_index<C: Clock>(vault: &AsterVault<C>) -> Vec<(String, usize)> {
    let raw = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::IndexInverted)
        .unwrap();
    let stored_order: Vec<Vec<u8>> = raw.iter().map(|(key, _)| key.clone()).collect();
    let mut sorted = stored_order.clone();
    sorted.sort();
    assert_eq!(stored_order, sorted);
    raw.into_iter()
        .map(|(key, value)| {
            assert_eq!(key[0], 0x11, "inverted index discriminant");
            assert!(matches!(value.len(), 4 | 12));
            (hex(&key), value.len())
        })
        .collect()
}

fn assert_edges(value: &Value) {
    assert_eq!(value["quick_sorted_pks"], json!([1, 2]));
    assert_eq!(value["fox_pks"], json!([1]));
    assert_eq!(value["missing_pks"], json!([]));
    assert_eq!(value["long4999_pks"], json!([3]));
    assert_eq!(value["same_pks"], json!([5, 6]));
    assert_eq!(value["bm25_or_pks"], json!([1, 2]));
    assert_eq!(value["bm25_and_pks"], json!([1]));
    assert_eq!(value["stats_doc_count"], json!(5));
    assert_eq!(value["posting_value_len_count"], json!(5011));
    assert_eq!(value["stats_value_len_count"], json!(1));
}

fn pk_nums(rows: &[(RecordKey, f32)]) -> Vec<u64> {
    rows.iter()
        .map(|(pk, _)| u64::from_be_bytes(pk.as_bytes().try_into().unwrap()))
        .collect()
}

fn weights(rows: &[(RecordKey, f32)]) -> Vec<String> {
    rows.iter()
        .map(|(_, score)| format!("{score:.6}"))
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

fn physical_files(root: &Path) -> Vec<Value> {
    let mut files = Vec::new();
    if root.exists() {
        collect_physical_file_states(root, &mut files);
    }
    files.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    files
}
