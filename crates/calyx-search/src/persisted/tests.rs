use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use calyx_aster::{
    cf::ColumnFamily,
    vault::{AsterVault, VaultOptions},
};
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality,
    SlotId, SlotVector, VaultId, VaultStore,
};
use calyx_sextant::{AnchorPredicate, MetadataPredicate, QueryFilters, ScalarOp, ScalarPredicate};
use ulid::Ulid;

use super::*;

#[path = "tests/flat_dense.rs"]
mod flat_dense;
#[path = "tests/rebuild_resume.rs"]
mod rebuild_resume;
#[path = "tests/streaming_rebuild.rs"]
mod streaming_rebuild;

#[test]
fn load_docs_reads_real_base_and_slot_cf_bytes() {
    let vault_id = VaultId::from_ulid(Ulid::from_bytes([7; 16]));
    let vault = AsterVault::new(vault_id, b"search-load-docs");
    let before_base_rows = vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Base)
        .expect("scan base before")
        .len();
    let before_slot0_rows = vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::slot(SlotId::new(0)))
        .expect("scan slot 0 before")
        .len();
    let before_slot2_rows = vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::slot(SlotId::new(2)))
        .expect("scan slot 2 before")
        .len();
    let mut first = constellation(cx(11), vec![1.0, 0.0]);
    first.vault_id = vault_id;
    first.slots.insert(SlotId::new(2), dense(vec![0.25, 0.75]));
    let mut second = constellation(cx(12), vec![0.0, 1.0]);
    second.vault_id = vault_id;
    second.slots.insert(SlotId::new(2), dense(vec![0.9, 0.1]));
    let first_id = vault.put(first).expect("write first constellation");
    let second_id = vault.put(second).expect("write second constellation");
    let after_base_rows = vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Base)
        .expect("scan base after")
        .len();
    let after_slot0_rows = vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::slot(SlotId::new(0)))
        .expect("scan slot 0 after")
        .len();
    let after_slot2_rows = vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::slot(SlotId::new(2)))
        .expect("scan slot 2 after")
        .len();

    let docs = load_docs(&vault).expect("load docs from physical CF bytes");

    let first_slot2 = docs
        .get(&first_id)
        .and_then(|cx| cx.slots.get(&SlotId::new(2)))
        .and_then(SlotVector::as_dense)
        .expect("first slot 2 dense vector");
    let second_slot0 = docs
        .get(&second_id)
        .and_then(|cx| cx.slots.get(&SlotId::new(0)))
        .and_then(SlotVector::as_dense)
        .expect("second slot 0 dense vector");
    println!(
        "SEARCH_LOAD_DOCS_FSV {}",
        serde_json::json!({
            "source_of_truth": "AsterVault Base CF rows plus Slot CF rows read after VaultStore::put",
            "before": {
                "base_rows": before_base_rows,
                "slot_0_rows": before_slot0_rows,
                "slot_2_rows": before_slot2_rows
            },
            "after": {
                "base_rows": after_base_rows,
                "slot_0_rows": after_slot0_rows,
                "slot_2_rows": after_slot2_rows,
                "loaded_docs": docs.len(),
                "first_id": first_id,
                "second_id": second_id,
                "first_slot2": first_slot2,
                "second_slot0": second_slot0,
            }
        })
    );
    assert_eq!(docs.len(), 2);
    assert_eq!(after_base_rows, 2);
    assert_eq!(after_slot0_rows, 2);
    assert_eq!(after_slot2_rows, 2);
    assert_eq!(first_slot2, &[0.25, 0.75]);
    assert_eq!(second_slot0, &[0.0, 1.0]);
}

#[test]
fn filtered_search_matches_exact_reference_for_scalar_anchor_and_metadata() {
    let root = scratch("filtered");
    let docs = rich_docs();
    rebuild_from_docs(&root, &docs, 11).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let filters = selective_filters();
    let candidates = indexes.filter_candidates(&filters).unwrap().unwrap();
    let query = dense(vec![1.0, 0.0]);
    let hits = indexes
        .search_filtered(SlotId::new(0), &query, candidates.len(), &candidates)
        .expect("filtered search");
    let expected = exact_reference(&docs, &filters, query.as_dense().unwrap());

    assert_eq!(candidates, BTreeSet::from([cx(1), cx(3)]));
    assert_eq!(
        hits.iter().map(|hit| hit.cx_id).collect::<Vec<_>>(),
        expected
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn empty_match_filter_returns_empty_candidate_set() {
    let root = scratch("empty-filter");
    rebuild_from_docs(&root, &rich_docs(), 12).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let filters = QueryFilters {
        scalars: vec![ScalarPredicate {
            name: "quality_score".to_string(),
            op: ScalarOp::Gt,
            value: 100.0,
        }],
        anchors: Vec::new(),
        metadata: Vec::new(),
    };

    let candidates = indexes.filter_candidates(&filters).unwrap().unwrap();

    assert!(candidates.is_empty());
    fs::remove_dir_all(root).ok();
}

#[test]
fn missing_filter_sidecar_fails_closed() {
    let root = scratch("missing-filter");
    rebuild_from_docs(&root, &rich_docs(), 13).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let entry = indexes.manifest.filter.as_ref().unwrap();
    fs::remove_file(root.join(&entry.index_rel)).unwrap();

    let err = indexes.filter_candidates(&selective_filters()).unwrap_err();

    assert_eq!(err.code(), "CALYX_STALE_DERIVED");
    assert!(err.message().contains("filter sidecar missing"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn stale_filter_sidecar_hash_fails_closed() {
    let root = scratch("stale-filter");
    rebuild_from_docs(&root, &rich_docs(), 14).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let entry = indexes.manifest.filter.as_ref().unwrap();
    fs::write(root.join(&entry.index_rel), b"{\"format\":\"tampered\"}").unwrap();

    let err = indexes.filter_candidates(&selective_filters()).unwrap_err();

    assert_eq!(err.code(), "CALYX_STALE_DERIVED");
    assert!(err.message().contains("sha256"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn missing_manifest_fails_closed() {
    let err = PersistedSearchIndexes::open(&scratch("missing")).unwrap_err();

    assert_eq!(err.code(), "CALYX_STALE_DERIVED");
    assert!(err.message().contains("manifest missing"));
}

#[test]
fn manifest_seq_must_cover_derived_content_seq() {
    let root = scratch("manifest-seq");
    rebuild_from_docs(&root, &docs([(1, vec![1.0, 0.0])]), 42).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");

    indexes
        .ensure_fresh_at_snapshot(42, 42)
        .expect("manifest at the pinned seq is fresh");
    // Content-neutral commits (idempotency-ledger appends) advance the pinned
    // seq but not the derived-content watermark: still fresh (issue #1100).
    indexes
        .ensure_fresh_at_snapshot(43, 42)
        .expect("content-neutral seq advance keeps Fresh available");
    indexes
        .ensure_fresh_at_snapshot(43, 40)
        .expect("watermark below manifest base seq is fresh");

    // A commit that changed derived-search inputs after the rebuild: stale.
    let err = indexes.ensure_fresh_at_snapshot(43, 43).unwrap_err();
    assert_eq!(err.code(), "CALYX_STALE_DERIVED");
    assert!(err.message().contains("base seq 42"));
    assert!(err.message().contains("derived content seq 43"));
    assert!(err.message().contains("pinned vault seq 43"));

    // Manifest built after the pinned snapshot: fail closed.
    let err = indexes.ensure_fresh_at_snapshot(41, 41).unwrap_err();
    assert_eq!(err.code(), "CALYX_STALE_DERIVED");
    assert!(err.message().contains("ahead of pinned vault seq 41"));

    // Unclamped watermark (above the pin) is a caller bug: fail closed.
    let err = indexes.ensure_fresh_at_snapshot(43, 44).unwrap_err();
    assert_eq!(err.code(), "CALYX_STALE_DERIVED");
    assert!(err.message().contains("was not clamped"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_dim_mismatch_fails_closed() {
    let root = scratch("dim");
    rebuild_from_docs(&root, &docs([(1, vec![1.0, 0.0])]), 2).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");

    let err = indexes
        .search(SlotId::new(0), &dense(vec![1.0, 0.0, 0.0]), 1)
        .unwrap_err();

    assert_eq!(err.code(), "CALYX_STALE_DERIVED");
    assert!(err.message().contains("dim 2 != query dim 3"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn sidecars_are_streamed_compact_with_matching_hash() {
    // Regression guard for the post-ingest finalization hang: sidecars must be streamed
    // as compact JSON via the shared hashing writer (not materialized with to_vec_pretty),
    // and the manifest sha256 must equal the hash of exactly the bytes written to disk.
    let root = scratch("compact");
    rebuild_from_docs(&root, &rich_docs(), 21).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let entry = indexes.manifest.filter.as_ref().expect("filter entry");
    let path = root.join(&entry.index_rel);
    let bytes = fs::read(&path).expect("read sidecar");

    // The pretty printer emits newlines + indentation; the streamed compact path has none.
    assert!(
        !bytes.contains(&b'\n'),
        "sidecar must be compact (streamed), not pretty-printed"
    );
    serde_json::from_slice::<serde_json::Value>(&bytes).expect("sidecar is valid json");
    // The streamed hash must equal the hash of the bytes actually on disk.
    assert_eq!(sha256_hex(&bytes), entry.sha256);
    fs::remove_dir_all(root).ok();
}

#[test]
fn latest_only_rebuild_reads_checkpoint_plus_wal_without_recheckpointing_base() {
    let root = scratch("latest-only-rebuild");
    let vault_id = VaultId::from_ulid(Ulid::from_bytes([0x31; 16]));
    let salt = b"latest-only-search-rebuild".to_vec();
    let writer = AsterVault::new_durable(&root, vault_id, salt.clone(), VaultOptions::default())
        .expect("open durable writer");
    let mut first = constellation(cx(31), vec![1.0, 0.0]);
    first.vault_id = vault_id;
    let mut second = constellation(cx(32), vec![0.0, 1.0]);
    second.vault_id = vault_id;
    writer
        .put_batch(vec![first, second])
        .expect("write checkpointed rows");
    writer.flush().expect("checkpoint first two rows");
    let checkpoint_ssts = sst_count(root.join("cf/base"));
    let checkpoint_manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("MANIFEST")).unwrap()).unwrap();
    assert_eq!(checkpoint_manifest["durable_seq"], 1);

    let mut third = constellation(cx(33), vec![0.5, 0.5]);
    third.vault_id = vault_id;
    writer.put(third).expect("write wal-only row");
    drop(writer);

    let latest = AsterVault::open(
        &root,
        vault_id,
        salt,
        VaultOptions {
            restore_mvcc_rows: false,
            ..VaultOptions::default()
        },
    )
    .expect("open latest-only reader");
    let docs = load_docs(&latest).expect("load docs from latest-only reader");
    rebuild_for_vault(&root, &latest).expect("rebuild from latest-only reader");
    let after_manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("MANIFEST")).unwrap()).unwrap();
    let index_manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("idx/search/manifest.json")).unwrap()).unwrap();

    assert_eq!(docs.len(), 3);
    assert_eq!(index_manifest["slots"].as_array().unwrap().len(), 1);
    assert_eq!(index_manifest["filter"]["len"], 3);
    assert_eq!(
        sst_count(root.join("cf/base")),
        checkpoint_ssts,
        "read-only search rebuild must not checkpoint or duplicate Base SST files"
    );
    assert_eq!(
        after_manifest["durable_seq"], checkpoint_manifest["durable_seq"],
        "read-only search rebuild must not advance the vault manifest durable_seq"
    );
    fs::remove_dir_all(root).ok();
}

fn selective_filters() -> QueryFilters {
    QueryFilters {
        scalars: vec![ScalarPredicate {
            name: "quality_score".to_string(),
            op: ScalarOp::Gte,
            value: 0.7,
        }],
        anchors: vec![AnchorPredicate {
            kind: AnchorKind::Label("issue735".to_string()),
            value: Some(AnchorValue::Enum("gold".to_string())),
            min_confidence: Some(0.8),
            source: Some("unit".to_string()),
        }],
        metadata: vec![
            MetadataPredicate::Modality(Modality::Text),
            MetadataPredicate::InputPointerContains("north".to_string()),
        ],
    }
}

fn exact_reference(
    docs: &BTreeMap<CxId, Constellation>,
    filters: &QueryFilters,
    query: &[f32],
) -> Vec<CxId> {
    let mut scored = docs
        .values()
        .filter(|cx| crate::filters::matches(cx, filters))
        .filter_map(|cx| {
            cx.slots
                .get(&SlotId::new(0))?
                .as_dense()
                .map(|values| (cx.cx_id, values))
        })
        .map(|(cx_id, values)| (cx_id, cosine(query, values)))
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.to_string().cmp(&right.0.to_string()))
    });
    scored.into_iter().map(|(cx_id, _)| cx_id).collect()
}

fn rich_docs() -> BTreeMap<CxId, Constellation> {
    let mut first = constellation(cx(1), vec![1.0, 0.0]);
    first.scalars.insert("quality_score".to_string(), 0.91);
    first.input_ref.pointer = Some("north/alpha".to_string());
    first.anchors.push(anchor("gold", 0.95, "unit"));

    let mut second = constellation(cx(2), vec![0.0, 1.0]);
    second.scalars.insert("quality_score".to_string(), 0.97);
    second.input_ref.pointer = Some("south/beta".to_string());
    second.anchors.push(anchor("gold", 0.95, "unit"));

    let mut third = constellation(cx(3), vec![0.8, 0.2]);
    third.scalars.insert("quality_score".to_string(), 0.72);
    third.input_ref.pointer = Some("north/gamma".to_string());
    third.anchors.push(anchor("gold", 0.82, "unit"));

    let mut fourth = constellation(cx(4), vec![0.9, 0.1]);
    fourth.scalars.insert("quality_score".to_string(), 0.69);
    fourth.input_ref.pointer = Some("north/delta".to_string());
    fourth.anchors.push(anchor("gold", 0.95, "unit"));

    [first, second, third, fourth]
        .into_iter()
        .map(|cx| (cx.cx_id, cx))
        .collect()
}

fn docs<const N: usize>(rows: [(u8, Vec<f32>); N]) -> BTreeMap<CxId, Constellation> {
    rows.into_iter()
        .map(|(seed, vector)| {
            let id = cx(seed);
            (id, constellation(id, vector))
        })
        .collect()
}

fn anchor(label: &str, confidence: f32, source: &str) -> Anchor {
    Anchor {
        kind: AnchorKind::Label("issue735".to_string()),
        value: AnchorValue::Enum(label.to_string()),
        source: source.to_string(),
        observed_at: 1,
        confidence,
    }
}

fn constellation(cx_id: CxId, vector: Vec<f32>) -> Constellation {
    let mut slots = BTreeMap::new();
    slots.insert(SlotId::new(0), dense(vector));
    Constellation {
        cx_id,
        vault_id: VaultId::from_ulid(Ulid::from_bytes([9; 16])),
        panel_version: 1,
        created_at: 1,
        input_ref: InputRef {
            hash: [0; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 1,
            hash: [1; 32],
        },
        flags: CxFlags::default(),
    }
}

fn dense(data: Vec<f32>) -> SlotVector {
    SlotVector::Dense {
        dim: data.len() as u32,
        data,
    }
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "calyx-cli-persisted-search-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch");
    dir
}

fn sst_count(path: PathBuf) -> usize {
    fs::read_dir(path)
        .expect("read sst dir")
        .filter(|entry| {
            entry
                .as_ref()
                .ok()
                .and_then(|entry| entry.path().extension().map(|ext| ext == "sst"))
                .unwrap_or(false)
        })
        .count()
}

fn cosine(left: &[f32], right: &[f32]) -> f32 {
    let (mut dot, mut left_l2, mut right_l2) = (0.0, 0.0, 0.0);
    for (left, right) in left.iter().zip(right) {
        dot += left * right;
        left_l2 += left * left;
        right_l2 += right * right;
    }
    if left_l2 == 0.0 || right_l2 == 0.0 {
        0.0
    } else {
        dot / (left_l2.sqrt() * right_l2.sqrt())
    }
}
