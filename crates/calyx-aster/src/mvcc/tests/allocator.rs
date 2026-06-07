use super::*;
use proptest::prelude::*;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::thread;

#[test]
fn allocator_and_snapshot_pin_latest_committed_sequence() {
    let store = VersionedCfStore::default();
    let clock = FixedClock::new(100);

    let initial = store.pin_snapshot(Freshness::FreshDerived, &clock, 10);
    assert_eq!(initial.seq(), 0);
    assert_eq!(initial.lease().pinned_seq(), 0);

    let seq = store
        .commit_batch([(ColumnFamily::Ledger, vec![0], b"ledger-v1".to_vec())])
        .expect("commit");
    let after = store.pin_snapshot(Freshness::FreshDerived, &clock, 10);

    assert_eq!(seq, 1);
    assert_eq!(store.current_seq(), 1);
    assert_eq!(after.seq(), 1);
}

#[test]
fn allocator_concurrent_stress_produces_contiguous_unique_range() {
    let allocator = Arc::new(SeqAllocator::default());
    let mut handles = Vec::new();
    for _ in 0..8 {
        let allocator = Arc::clone(&allocator);
        handles.push(thread::spawn(move || {
            (0..100).map(|_| allocator.allocate()).collect::<Vec<_>>()
        }));
    }

    let mut seqs = Vec::new();
    for handle in handles {
        seqs.extend(handle.join().expect("allocator thread joins"));
    }
    seqs.sort_unstable();
    let unique = seqs.iter().copied().collect::<BTreeSet<_>>();

    assert_eq!(seqs.len(), 800);
    assert_eq!(unique.len(), 800);
    assert_eq!(seqs.first().copied(), Some(1));
    assert_eq!(seqs.last().copied(), Some(800));
    assert_eq!(allocator.current(), 800);
    println!("MVCC_ALLOC_UNIQUE count=800 min=1 max=800");
}

#[test]
fn allocator_current_set_start_and_overflow_edges() {
    let allocator = SeqAllocator::new(42);
    assert_eq!(allocator.current(), 42);
    assert_eq!(allocator.allocate(), 43);
    assert_eq!(allocator.current(), 43);

    let recovered = SeqAllocator::default();
    recovered.set_start_seq(99).expect("set recovered seq");
    assert_eq!(recovered.allocate(), 100);
    let error = recovered
        .set_start_seq(7)
        .expect_err("set after allocation fails closed");
    assert_eq!(error.code, "CALYX_BACKPRESSURE");

    let near_max = SeqAllocator::new(u64::MAX - 1);
    assert_eq!(near_max.allocate(), u64::MAX);
}

proptest! {
    #[test]
    fn allocator_sequential_allocations_are_contiguous(start in 0u64..1_000_000, n in 1usize..=100) {
        let allocator = SeqAllocator::new(start);
        for offset in 1..=n as u64 {
            prop_assert_eq!(allocator.allocate(), start + offset);
        }
        prop_assert_eq!(allocator.current(), start + n as u64);
    }
}
