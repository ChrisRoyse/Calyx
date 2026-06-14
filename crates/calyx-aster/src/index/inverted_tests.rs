use proptest::prelude::*;

use super::*;
use crate::collection::{
    Collection, CollectionMode, DedupPolicy, FieldDef, IsolationLevel, RetentionPolicy, Schema,
    TemporalPolicy, TenantId, TxnPolicy,
};
use crate::index::IndexId;
use crate::layers::{RelationalLayer, Row};
use calyx_core::VaultId;

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn texts() -> Collection {
    Collection {
        name: "texts".to_string(),
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
        "body_idx",
        IndexKind::Inverted,
        "body",
        FieldType::Text,
    )
}

fn put_doc(vault: &AsterVault, col: &Collection, spec: &IndexSpec, pk: u64, text: &str) {
    let key = RecordKey::from_u64(pk);
    let row = Row::new([("body", RecordValue::Text(text.to_string()))]);
    RelationalLayer::new(vault)
        .put_record(col, &key, &row)
        .unwrap();
    inverted_put(vault, col, spec, &RecordValue::Text(text.to_string()), &key).unwrap();
}

fn pk_nums(rows: &[(RecordKey, f32)]) -> Vec<u64> {
    rows.iter()
        .map(|(pk, _)| u64::from_be_bytes(pk.as_bytes().try_into().unwrap()))
        .collect()
}

#[test]
fn term_match_and_bm25_rank_known_docs() {
    let vault = AsterVault::new(vault_id(), b"inverted-unit");
    let col = texts();
    let spec = body_index();
    assert_eq!(CF_INDEX_INVERTED, ColumnFamily::IndexInverted.name());

    put_doc(&vault, &col, &spec, 1, "the quick brown fox");
    put_doc(&vault, &col, &spec, 2, "quick lazy dog");

    let quick = inverted_match(&vault, &col, &spec, "quick").unwrap();
    assert_eq!(
        pk_nums(&quick).into_iter().collect::<BTreeSet<_>>(),
        [1, 2].into()
    );
    assert!(
        quick
            .iter()
            .all(|(_, weight)| *weight > 0.0 && weight.is_finite())
    );

    let bm25 = inverted_bm25(&vault, &col, &spec, &["quick", "fox"], 2, 10).unwrap();
    assert_eq!(pk_nums(&bm25), [1, 2]);
    assert!(bm25[0].1 > bm25[1].1);
}

#[test]
fn edges_and_invalid_values_fail_closed() {
    let vault = AsterVault::new(vault_id(), b"inverted-edges");
    let col = texts();
    let spec = body_index();

    put_doc(&vault, &col, &spec, 1, "");
    assert!(
        inverted_match(&vault, &col, &spec, "missing")
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        vault
            .scan_cf_at(vault.latest_seq(), ColumnFamily::IndexInverted)
            .unwrap()
            .len(),
        0
    );

    put_doc(&vault, &col, &spec, 2, "same text");
    put_doc(&vault, &col, &spec, 3, "same text");
    assert_eq!(
        pk_nums(&inverted_match(&vault, &col, &spec, "same").unwrap()),
        [2, 3]
    );

    let error = inverted_put(
        &vault,
        &col,
        &spec,
        &RecordValue::I64(7),
        &RecordKey::from_u64(4),
    )
    .unwrap_err();
    assert_eq!(error.code, "CALYX_INVALID_ARGUMENT");

    let wrong_spec = IndexSpec::new(
        IndexId::new(2),
        "qty",
        IndexKind::Btree,
        "qty",
        FieldType::I64,
    );
    let error = inverted_match(&vault, &col, &wrong_spec, "same").unwrap_err();
    assert_eq!(error.code, "CALYX_INVALID_ARGUMENT");
}

proptest! {
    #[test]
    fn term_match_has_no_false_positives(flags in proptest::collection::vec(any::<bool>(), 1..16)) {
        let vault = AsterVault::new(vault_id(), b"inverted-prop");
        let col = texts();
        let spec = body_index();
        for (idx, has_needle) in flags.iter().enumerate() {
            let text = if *has_needle { "alpha needle zed" } else { "alpha haystack zed" };
            put_doc(&vault, &col, &spec, idx as u64 + 1, text);
        }
        let hits = inverted_match(&vault, &col, &spec, "needle").unwrap();
        for (pk, _) in hits {
            let idx = u64::from_be_bytes(pk.as_bytes().try_into().unwrap()) as usize - 1;
            prop_assert!(flags[idx]);
        }
    }
}
