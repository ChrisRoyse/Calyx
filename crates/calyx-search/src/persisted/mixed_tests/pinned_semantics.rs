//! Verify-once-then-pin semantics for persisted slot indexes (issue #1102).
//!
//! Contract under test: sidecar content is fully hashed and validated on the
//! first search per manifest generation, then served from a pinned in-memory
//! copy keyed by the manifest entry sha256. On-disk presence and byte size
//! are still verified per bounded check, and any manifest-generation change
//! forces a fresh verified load.

use std::fs;

use calyx_core::SlotId;
use calyx_sextant::index::MaxSimIndex;

use super::super::{PersistedSearchIndexes, rebuild_from_docs};
use super::helpers::*;

#[test]
fn pinned_multi_scores_are_bit_identical_to_streaming_maxsim() {
    let root = scratch("pinned-parity");
    let docs = mixed_docs();
    rebuild_from_docs(&root, &docs, 40).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let query = vec![vec![0.6f32, 0.4f32]];

    let hits = indexes
        .search(SlotId::new(2), &multi(2, [[0.6, 0.4]]), 3)
        .expect("multi search");

    assert_eq!(hits.len(), 3);
    for hit in &hits {
        let doc = docs.get(&hit.cx_id).expect("hit doc");
        let calyx_core::SlotVector::Multi { tokens, .. } = &doc.slots[&SlotId::new(2)] else {
            panic!("slot 2 must be multi");
        };
        let expected = MaxSimIndex::maxsim(&query, tokens);
        assert_eq!(
            hit.score.to_bits(),
            expected.to_bits(),
            "pinned score for {} must be bit-identical to MaxSimIndex::maxsim",
            hit.cx_id
        );
    }
    cleanup(root);
}

#[test]
fn pinned_multi_serves_verified_generation_and_stat_checks_catch_truncation() {
    let root = scratch("pinned-generation");
    rebuild_from_docs(&root, &mixed_docs(), 41).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let query = multi(2, [[0.6, 0.4]]);

    let first = indexes
        .search(SlotId::new(2), &query, 3)
        .expect("first search pins the verified index");

    let multi_entry = indexes
        .manifest
        .slots
        .iter()
        .find(|entry| entry.slot == 2)
        .expect("multi entry");
    let segment_rel = first_segment_rel(&read_multi_segment_manifest(&root, multi_entry));
    let segment_path = root.join(&segment_rel);
    let original = fs::read(&segment_path).expect("segment bytes");

    // Same-size in-place corruption: the pinned generation keeps serving the
    // bytes that were verified against the manifest sha256 at load time.
    let mut corrupted = original.clone();
    let last = corrupted.len() - 1;
    corrupted[last] ^= 0xFF;
    fs::write(&segment_path, &corrupted).expect("corrupt segment in place");
    let second = indexes
        .search(SlotId::new(2), &query, 3)
        .expect("pinned search serves the verified generation");
    assert_eq!(first, second);

    // Truncation changes the on-disk byte size: the memoized bounded check
    // still stats every file per call and fails closed.
    fs::write(&segment_path, &corrupted[..corrupted.len() - 1]).expect("truncate segment");
    let err = indexes
        .ensure_search_bounded_for_slots(None)
        .expect_err("bounded check must detect truncation");
    assert_eq!(err.code(), "CALYX_STALE_DERIVED");
    assert!(err.message().contains("bytes, expected"));

    fs::write(&segment_path, &original).expect("restore segment");
    cleanup(root);
}

#[test]
fn new_manifest_generation_forces_fresh_verified_load() {
    let root = scratch("pinned-repin");
    let docs = mixed_docs();
    rebuild_from_docs(&root, &docs, 42).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    indexes
        .search(SlotId::new(1), &sparse(8, [1]), 2)
        .expect("first sparse search pins");
    indexes
        .search(SlotId::new(2), &multi(2, [[0.6, 0.4]]), 2)
        .expect("first multi search pins");

    // New generation: rebuild at a later seq, then corrupt the new sparse
    // sidecar. The pin is keyed by entry sha256, so the next search must
    // re-read the new file from disk and fail closed on the corruption.
    rebuild_from_docs(&root, &docs, 43).expect("rebuild new generation");
    let reopened = PersistedSearchIndexes::open(&root).expect("reopen");
    let sparse_entry = reopened
        .manifest
        .slots
        .iter()
        .find(|entry| entry.slot == 1)
        .expect("sparse entry");
    let sparse_path = root.join(sparse_entry.index_rel.as_ref().unwrap());
    let mut bytes = fs::read(&sparse_path).expect("sparse sidecar");
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    fs::write(&sparse_path, &bytes).expect("corrupt new sparse sidecar");

    let err = reopened
        .search(SlotId::new(1), &sparse(8, [1]), 2)
        .expect_err("new generation must be re-verified from disk");
    assert_eq!(err.code(), "CALYX_STALE_DERIVED");
    assert!(err.message().contains("sha256"));

    // The multi slot's new generation loads and pins cleanly.
    reopened
        .search(SlotId::new(2), &multi(2, [[0.6, 0.4]]), 2)
        .expect("multi repin at new generation");
    cleanup(root);
}
