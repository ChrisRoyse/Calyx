use calyx_core::{SlotId, SlotVector, SparseEntry};

use super::*;

#[test]
fn persisted_sparse_ranking_uses_document_weights() {
    let root = scratch("issue1381-document-weight");
    let weak_id = cx(0x11);
    let strong_id = cx(0x22);
    let mut weak = constellation(weak_id, vec![1.0, 0.0]);
    weak.slots
        .insert(SlotId::new(1), sparse(&[(7, 0.25)]));
    let mut strong = constellation(strong_id, vec![0.0, 1.0]);
    strong
        .slots
        .insert(SlotId::new(1), sparse(&[(7, 2.5)]));
    let docs = BTreeMap::from([(weak_id, weak), (strong_id, strong)]);

    rebuild_from_docs(&root, &docs, 7).expect("rebuild weighted sparse index");
    let indexes = PersistedSearchIndexes::open(&root).expect("open weighted sparse index");
    let hits = indexes
        .search(SlotId::new(1), &sparse(&[(7, 1.0)]), 2)
        .expect("search weighted sparse index");

    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].cx_id, strong_id);
    assert!(hits[0].score > hits[1].score);
    fs::remove_dir_all(root).ok();
}

#[test]
fn persisted_sparse_ranking_uses_query_weights() {
    let root = scratch("issue1381-query-weight");
    let decoy_id = cx(0x21);
    let relevant_id = cx(0x42);
    let mut decoy = constellation(decoy_id, vec![1.0, 0.0]);
    decoy.slots.insert(
        SlotId::new(1),
        sparse(&[(3, 0.25), (9, 4.0)]),
    );
    let mut relevant = constellation(relevant_id, vec![0.0, 1.0]);
    relevant.slots.insert(
        SlotId::new(1),
        sparse(&[(3, 4.0), (9, 0.25)]),
    );
    let docs = BTreeMap::from([(decoy_id, decoy), (relevant_id, relevant)]);

    rebuild_from_docs(&root, &docs, 8).expect("rebuild query-weighted sparse index");
    let indexes = PersistedSearchIndexes::open(&root).expect("open query-weighted sparse index");
    let hits = indexes
        .search(
            SlotId::new(1),
            &sparse(&[(3, 3.0), (9, 0.5)]),
            2,
        )
        .expect("search query-weighted sparse index");

    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].cx_id, relevant_id);
    assert!(hits[0].score > hits[1].score);
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
