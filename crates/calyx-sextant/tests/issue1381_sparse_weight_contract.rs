use calyx_core::{CxId, SlotId, SlotVector, SparseEntry};
use calyx_sextant::{InvertedIndex, SextantIndex};

#[test]
fn sparse_document_weights_control_ranking_before_and_after_rebuild() {
    let weak_id = cx(0x11);
    let strong_id = cx(0x22);
    let mut index = InvertedIndex::new(SlotId::new(1));
    index
        .insert(weak_id, sparse(&[(7, 0.25)]), 1)
        .expect("insert weak sparse document");
    index
        .insert(strong_id, sparse(&[(7, 2.5)]), 2)
        .expect("insert strong sparse document");

    assert_strong_document_wins(&index, strong_id);
    index.rebuild().expect("rebuild weighted sparse index");
    assert_strong_document_wins(&index, strong_id);
}

#[test]
fn sparse_query_weights_control_ranking() {
    let decoy_id = cx(0x21);
    let relevant_id = cx(0x42);
    let mut index = InvertedIndex::new(SlotId::new(1));
    index
        .insert(decoy_id, sparse(&[(3, 0.25), (9, 4.0)]), 1)
        .expect("insert decoy sparse document");
    index
        .insert(relevant_id, sparse(&[(3, 4.0), (9, 0.25)]), 2)
        .expect("insert relevant sparse document");

    let hits = index
        .search(&sparse(&[(3, 3.0), (9, 0.5)]), 2, None)
        .expect("search weighted sparse query");

    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].cx_id, relevant_id);
    assert!(hits[0].score > hits[1].score);
}

#[test]
fn invalid_sparse_weights_fail_before_mutation() {
    let retained_id = cx(0x31);
    let mut index = InvertedIndex::new(SlotId::new(1));
    index
        .insert(retained_id, sparse(&[(1, 1.0)]), 1)
        .expect("insert retained document");

    for (seed, weight) in [(0x32, 0.0), (0x33, -1.0)] {
        let rejected_id = cx(seed);
        let err = index
            .insert(rejected_id, sparse(&[(2, weight)]), 2)
            .unwrap_err();
        assert_eq!(err.code, "CALYX_SEXTANT_VECTOR_SHAPE");
        assert!(index.vector(rejected_id).is_none());
        assert_eq!(index.total_docs(), 1);
    }

    let overflow_id = cx(0x34);
    let err = index
        .insert(overflow_id, sparse(&[(2, f32::MAX), (3, f32::MAX)]), 3)
        .unwrap_err();
    assert_eq!(err.code, "CALYX_SEXTANT_VECTOR_SHAPE");
    assert!(index.vector(overflow_id).is_none());
    assert_eq!(index.total_docs(), 1);
}

#[test]
fn invalid_query_and_score_overflow_fail_closed() {
    let mut index = InvertedIndex::new(SlotId::new(1));
    index
        .insert(cx(0x41), sparse(&[(7, 1.0)]), 1)
        .expect("insert searchable document");

    let err = index.search(&sparse(&[(7, 0.0)]), 1, None).unwrap_err();
    assert_eq!(err.code, "CALYX_SEXTANT_VECTOR_SHAPE");

    let extreme_id = cx(0x42);
    index
        .insert(extreme_id, sparse(&[(8, f32::MAX)]), 2)
        .expect("finite extreme weight is structurally valid");
    let err = index.search(&sparse(&[(8, 1.0)]), 1, None).unwrap_err();
    assert_eq!(err.code, "CALYX_SEXTANT_VECTOR_SHAPE");
    assert!(err.message.contains("score overflowed"));
}

fn assert_strong_document_wins(index: &InvertedIndex, strong_id: CxId) {
    let hits = index
        .search(&sparse(&[(7, 1.0)]), 2, None)
        .expect("search weighted sparse documents");
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].cx_id, strong_id);
    assert!(hits[0].score > hits[1].score);
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

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}
