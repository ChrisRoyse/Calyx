use calyx_core::{SlotId, SlotVector, SparseEntry};

use super::*;

#[test]
fn persisted_sparse_ranking_uses_document_weights() {
    let root = scratch("issue1381-document-weight");
    let weak_id = cx(0x11);
    let strong_id = cx(0x22);
    let mut weak = constellation(weak_id, vec![1.0, 0.0]);
    weak.slots.insert(SlotId::new(1), sparse(&[(7, 0.25)]));
    let mut strong = constellation(strong_id, vec![0.0, 1.0]);
    strong.slots.insert(SlotId::new(1), sparse(&[(7, 2.5)]));
    let docs = BTreeMap::from([(weak_id, weak), (strong_id, strong)]);

    rebuild_from_docs(&root, &docs, 7).expect("rebuild weighted sparse index");
    let indexes = PersistedSearchIndexes::open(&root).expect("open weighted sparse index");
    let hits = indexes
        .search(SlotId::new(1), &sparse(&[(7, 1.0)]), 2)
        .expect("search weighted sparse index");

    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].cx_id, strong_id);
    assert!(hits[0].score > hits[1].score);
    let entry = indexes
        .manifest
        .slots
        .iter()
        .find(|entry| entry.slot == 1)
        .expect("sparse manifest entry");
    let sidecar: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join(entry.index_rel.as_ref().expect("sparse sidecar path"))).unwrap(),
    )
    .expect("sparse sidecar json");
    assert_eq!(sidecar["format"], "calyx-search-sparse-index-v2");
    assert_eq!(sidecar["rows"][0]["doc_len"].as_f64(), Some(0.25));
    assert_eq!(sidecar["rows"][1]["doc_len"].as_f64(), Some(2.5));
    assert_eq!(sidecar["postings"]["7"][0]["tf"].as_f64(), Some(0.25));
    assert_eq!(sidecar["postings"]["7"][1]["tf"].as_f64(), Some(2.5));
    fs::remove_dir_all(root).ok();
}

#[test]
fn persisted_sparse_ranking_uses_query_weights() {
    let root = scratch("issue1381-query-weight");
    let decoy_id = cx(0x21);
    let relevant_id = cx(0x42);
    let mut decoy = constellation(decoy_id, vec![1.0, 0.0]);
    decoy
        .slots
        .insert(SlotId::new(1), sparse(&[(3, 0.25), (9, 4.0)]));
    let mut relevant = constellation(relevant_id, vec![0.0, 1.0]);
    relevant
        .slots
        .insert(SlotId::new(1), sparse(&[(3, 4.0), (9, 0.25)]));
    let docs = BTreeMap::from([(decoy_id, decoy), (relevant_id, relevant)]);

    rebuild_from_docs(&root, &docs, 8).expect("rebuild query-weighted sparse index");
    let indexes = PersistedSearchIndexes::open(&root).expect("open query-weighted sparse index");
    let hits = indexes
        .search(SlotId::new(1), &sparse(&[(3, 3.0), (9, 0.5)]), 2)
        .expect("search query-weighted sparse index");

    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].cx_id, relevant_id);
    assert!(hits[0].score > hits[1].score);
    fs::remove_dir_all(root).ok();
}

#[test]
fn invalid_persisted_weights_fail_closed() {
    let invalid_root = scratch("issue1381-invalid-document-weight");
    let invalid_id = cx(0x51);
    let mut invalid = constellation(invalid_id, vec![1.0, 0.0]);
    invalid.slots.insert(SlotId::new(1), sparse(&[(7, 0.0)]));
    let err =
        rebuild_from_docs(&invalid_root, &BTreeMap::from([(invalid_id, invalid)]), 9).unwrap_err();
    assert_eq!(err.code(), "CALYX_STALE_DERIVED");
    assert!(!invalid_root.join("idx/search/manifest.json").exists());
    fs::remove_dir_all(invalid_root).ok();

    let overflow_root = scratch("issue1381-corpus-overflow");
    let mut first = constellation(cx(0x53), vec![1.0, 0.0]);
    first.slots.insert(SlotId::new(1), sparse(&[(7, f32::MAX)]));
    let mut second = constellation(cx(0x54), vec![0.0, 1.0]);
    second
        .slots
        .insert(SlotId::new(1), sparse(&[(7, f32::MAX)]));
    let err = rebuild_from_docs(
        &overflow_root,
        &BTreeMap::from([(cx(0x53), first), (cx(0x54), second)]),
        10,
    )
    .unwrap_err();
    assert_eq!(err.code(), "CALYX_STALE_DERIVED");
    assert!(err.message().contains("corpus length overflowed"));
    assert!(!overflow_root.join("idx/search/manifest.json").exists());
    fs::remove_dir_all(overflow_root).ok();

    let query_root = scratch("issue1381-invalid-query-weight");
    let valid_id = cx(0x52);
    let mut valid = constellation(valid_id, vec![1.0, 0.0]);
    valid.slots.insert(SlotId::new(1), sparse(&[(7, 1.0)]));
    rebuild_from_docs(&query_root, &BTreeMap::from([(valid_id, valid)]), 11)
        .expect("rebuild valid sparse index");
    let indexes = PersistedSearchIndexes::open(&query_root).expect("open valid sparse index");
    let err = indexes
        .search(SlotId::new(1), &sparse(&[(7, 0.0)]), 1)
        .unwrap_err();
    assert_eq!(err.code(), "CALYX_STALE_DERIVED");
    fs::remove_dir_all(query_root).ok();
}

#[test]
fn legacy_sparse_sidecar_format_requires_rebuild() {
    let root = scratch("issue1381-v1-sidecar");
    let id = cx(0x61);
    let mut doc = constellation(id, vec![1.0, 0.0]);
    doc.slots.insert(SlotId::new(1), sparse(&[(7, 1.0)]));
    rebuild_from_docs(&root, &BTreeMap::from([(id, doc)]), 12).expect("rebuild v2 index");

    let manifest_path = root.join("idx/search/manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    let entry = manifest["slots"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry["slot"] == 1)
        .unwrap();
    let sidecar_path = root.join(entry["index_rel"].as_str().unwrap());
    let mut sidecar: serde_json::Value =
        serde_json::from_slice(&fs::read(&sidecar_path).unwrap()).unwrap();
    sidecar["format"] = serde_json::Value::String("calyx-search-sparse-index-v1".into());
    let sidecar_bytes = serde_json::to_vec(&sidecar).unwrap();
    fs::write(&sidecar_path, &sidecar_bytes).unwrap();
    entry["sha256"] = serde_json::Value::String(sha256_hex(&sidecar_bytes));
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

    let indexes = PersistedSearchIndexes::open(&root).expect("open legacy manifest");
    let err = indexes
        .search(SlotId::new(1), &sparse(&[(7, 1.0)]), 1)
        .unwrap_err();
    assert_eq!(err.code(), "CALYX_STALE_DERIVED");
    assert!(
        err.message()
            .contains("expected calyx-search-sparse-index-v2")
    );
    fs::remove_dir_all(root).ok();
}

fn sparse(entries: &[(u32, f32)]) -> SlotVector {
    SlotVector::Sparse {
        dim: 16,
        entries: entries
            .iter()
            .map(|&(idx, val)| SparseEntry { idx, val })
            .collect(),
    }
}
