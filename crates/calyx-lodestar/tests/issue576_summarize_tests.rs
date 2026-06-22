//! Synthetic, deterministic tests for PH72 · T06 universal summarization (#576).
//!
//! Known input → hand-computed expectation (the 2+2=4 discipline). The Source of
//! Truth for every "Ledger entry present" assertion is an INDEPENDENT
//! `scan_entries()` read of the ledger store — never the `summarize` return value.

use std::collections::{BTreeMap, BTreeSet};

use calyx_core::{AnchorKind, CxId, FixedClock};
use calyx_ledger::{EntryKind, LedgerAppender, MemoryLedgerStore, SubjectId};
use calyx_lodestar::{
    CALYX_SCOPE_INVALID_TIME_WINDOW, CALYX_SUMMARIZE_INSUFFICIENT_GROUNDING,
    CALYX_TIMETRAVEL_BEFORE_HORIZON, CollectionId, SUMMARIZE_INVOKED_MARKER, Scope, ScopeCache,
    SummarizeCtx, SummarizeParams, scope_hash, summarize, summarize_as_of,
};
use calyx_paths::AssocGraph;
use proptest::prelude::*;

mod memory_assoc_support;
use memory_assoc_support::{MemoryAssocStore, cx, ids};

/// Bundles the engine plumbing into a [`SummarizeCtx`] for one call. The returned
/// context's borrows of `cache`/`ledger` end with the enclosing statement.
fn ctx<'a>(
    cache: &'a mut ScopeCache,
    clock: &'a FixedClock,
    ledger: &'a mut LedgerAppender<MemoryLedgerStore, FixedClock>,
) -> SummarizeCtx<'a, MemoryLedgerStore, FixedClock> {
    SummarizeCtx {
        cache,
        clock,
        ledger,
    }
}

/// A 20-node corpus: two grounded collections (anchored at cx(1)) plus a planted
/// "bridge" between them and an isolated pair. Timestamps are monotone in seed.
fn corpus(anchored: bool, temporal_ready: bool) -> MemoryAssocStore {
    let mut builder = AssocGraph::builder();
    for seed in 1..=20u8 {
        builder.add_node(cx(seed), 1.0 + (seed % 5) as f32).unwrap();
    }
    // collection A cycle, collection B cycle, a bridge A->B, and a tail chain.
    let edges: &[(u8, u8)] = &[
        (1, 2),
        (2, 3),
        (3, 4),
        (4, 5),
        (5, 1), // collection A (anchored at 1)
        (6, 7),
        (7, 8),
        (8, 9),
        (9, 10),
        (10, 6), // collection B
        (5, 6),  // bridge A -> B
        (11, 12),
        (12, 13),
        (13, 11), // isolated triangle
        (14, 15),
        (15, 16),
        (16, 17),
        (17, 18),
        (18, 19),
        (19, 20),
    ];
    for (a, b) in edges {
        builder.add_edge(cx(*a), cx(*b), 1.0).unwrap();
    }
    let domain = AnchorKind::Label("domain".to_string());
    MemoryAssocStore::with_indexes(
        builder.build(),
        BTreeMap::from([
            (CollectionId::from("coll-a"), ids([1, 2, 3, 4, 5])),
            (CollectionId::from("coll-b"), ids([6, 7, 8, 9, 10])),
            (CollectionId::from("empty"), BTreeSet::new()),
        ]),
        if anchored {
            BTreeMap::from([(domain, vec![cx(1)])])
        } else {
            BTreeMap::new()
        },
        temporal_ready.then(|| {
            (1..=20u8)
                .map(|seed| (cx(seed), 1_000_u64 + seed as u64))
                .collect()
        }),
        BTreeMap::new(),
        BTreeMap::new(),
    )
}

fn empty_store() -> MemoryAssocStore {
    MemoryAssocStore::with_indexes(
        AssocGraph::builder().build(),
        BTreeMap::new(),
        BTreeMap::new(),
        Some(BTreeMap::new()),
        BTreeMap::new(),
        BTreeMap::new(),
    )
}

fn appender() -> LedgerAppender<MemoryLedgerStore, FixedClock> {
    LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(1_000)).unwrap()
}

/// Reads the LAST ledger entry independently (the Source of Truth).
fn last_entry(
    ledger: &LedgerAppender<MemoryLedgerStore, FixedClock>,
) -> (EntryKind, SubjectId, serde_json::Value) {
    let entries = ledger.scan_entries().expect("scan ledger");
    let entry = entries.last().expect("at least one ledger entry").clone();
    let payload: serde_json::Value =
        serde_json::from_slice(&entry.payload).expect("ledger payload is JSON");
    (entry.kind, entry.subject, payload)
}

#[test]
fn all_associations_summary_is_strict_subset_with_ledger_proof() {
    let store = corpus(true, false);
    let mut cache = ScopeCache::new(8);
    let clock = FixedClock::new(7_000);
    let mut ledger = appender();

    // max_kernel_size=20 over 20 nodes drives target_fraction=1.0 → the full
    // feedback-vertex-set kernel (non-trivial on a cyclic corpus).
    let result = summarize(
        &store,
        Scope::AllAssociations,
        Some(SummarizeParams {
            max_kernel_size: Some(20),
            ..Default::default()
        }),
        &mut ctx(&mut cache, &clock, &mut ledger),
    )
    .expect("summarize");

    // kernel ids are a strict subset of the 20-node corpus.
    let all: BTreeSet<CxId> = (1..=20u8).map(cx).collect();
    let kernel: BTreeSet<CxId> = result.kernel_ids.iter().copied().collect();
    assert!(kernel.is_subset(&all), "kernel ⊆ corpus");
    assert!(result.kernel_size <= 20);
    assert!(
        result.kernel_size > 0,
        "non-trivial corpus yields a non-empty kernel"
    );
    assert!((0.0..=1.0).contains(&result.kernel_only_recall));
    assert!((0.0..=1.0).contains(&result.grounded_fraction));

    // SoT: independent ledger read — exactly the SUMMARIZE_INVOKED entry.
    let (kind, subject, payload) = last_entry(&ledger);
    assert_eq!(kind, EntryKind::Kernel, "kind is Kernel");
    assert_eq!(
        subject,
        SubjectId::Kernel(result.scope_hash.to_vec()),
        "subject carries scope hash"
    );
    assert_eq!(payload["marker"], SUMMARIZE_INVOKED_MARKER);
    assert_eq!(
        payload["kernel_size"].as_u64().unwrap() as usize,
        result.kernel_size
    );
    assert_eq!(result.ledger_ref.seq, 0, "first ledger entry has seq 0");
}

#[test]
fn collection_scope_isolates_its_nodes() {
    let store = corpus(true, false);
    let mut cache = ScopeCache::new(8);
    let clock = FixedClock::new(1);
    let mut ledger = appender();

    let result = summarize(
        &store,
        Scope::Collection {
            id: CollectionId::from("coll-a"),
        },
        None,
        &mut ctx(&mut cache, &clock, &mut ledger),
    )
    .expect("summarize collection");

    let coll_a = ids([1, 2, 3, 4, 5]);
    let coll_b = ids([6, 7, 8, 9, 10]);
    for id in &result.kernel_ids {
        assert!(coll_a.contains(id), "kernel id {id} is in coll-a");
        assert!(
            !coll_b.contains(id),
            "no coll-b node leaks into coll-a summary"
        );
    }
}

#[test]
fn require_grounded_fails_closed_without_leaking_kernel() {
    // An unanchored corpus: grounded_fraction collapses below 0.5.
    let store = corpus(false, false);
    let mut cache = ScopeCache::new(8);
    let clock = FixedClock::new(1);
    let mut ledger = appender();

    let err = summarize(
        &store,
        Scope::AllAssociations,
        // max_kernel_size=20 forces a non-empty kernel; with no anchors its
        // grounded_fraction collapses to 0.0 (< 0.5), tripping the guard.
        Some(SummarizeParams {
            require_grounded: true,
            max_kernel_size: Some(20),
            ..Default::default()
        }),
        &mut ctx(&mut cache, &clock, &mut ledger),
    )
    .expect_err("ungrounded corpus must fail closed");

    assert_eq!(err.code, CALYX_SUMMARIZE_INSUFFICIENT_GROUNDING);
    // Fail-closed: NO ledger entry was written (the summary is fully withheld).
    assert!(
        ledger.scan_entries().unwrap().is_empty(),
        "no provenance for a withheld summary"
    );
}

#[test]
fn summarize_as_of_before_horizon_fails_closed() {
    let store = corpus(true, true);
    let mut cache = ScopeCache::new(8);
    let clock = FixedClock::new(1);
    let mut ledger = appender();

    // Retention horizon is 1_010; ask for t = 1_005 (before it).
    let err = summarize_as_of(
        &store,
        Scope::AllAssociations,
        1_005,
        Some(1_010),
        None,
        &mut ctx(&mut cache, &clock, &mut ledger),
    )
    .expect_err("before-horizon read must fail closed");

    assert_eq!(err.code, CALYX_TIMETRAVEL_BEFORE_HORIZON);
    assert!(
        ledger.scan_entries().unwrap().is_empty(),
        "no kernel built before horizon"
    );
}

#[test]
fn empty_vault_yields_empty_summary_with_provenance() {
    let store = empty_store();
    let mut cache = ScopeCache::new(8);
    let clock = FixedClock::new(1);
    let mut ledger = appender();

    let result = summarize(
        &store,
        Scope::AllAssociations,
        None,
        &mut ctx(&mut cache, &clock, &mut ledger),
    )
    .expect("summarize empty");

    assert_eq!(result.kernel_size, 0);
    assert!(result.kernel_ids.is_empty());
    assert_eq!(result.kernel_only_recall, 0.0);
    // Provenance is still written — an empty summary is a real, audited event.
    let (kind, _, payload) = last_entry(&ledger);
    assert_eq!(kind, EntryKind::Kernel);
    assert_eq!(payload["kernel_size"].as_u64().unwrap(), 0);
}

#[test]
fn no_cache_recomputes_and_reprovenances() {
    let store = corpus(true, false);
    let mut cache = ScopeCache::new(8);
    let clock = FixedClock::new(1);
    let mut ledger = appender();
    let params = || {
        Some(SummarizeParams {
            cache_ttl_secs: Some(0),
            ..Default::default()
        })
    };

    let first = summarize(
        &store,
        Scope::AllAssociations,
        params(),
        &mut ctx(&mut cache, &clock, &mut ledger),
    )
    .expect("first");
    let second = summarize(
        &store,
        Scope::AllAssociations,
        params(),
        &mut ctx(&mut cache, &clock, &mut ledger),
    )
    .expect("second");

    // Two distinct ledger entries (distinct seq) → both recomputed + reprovenanced.
    assert_ne!(
        first.ledger_ref.seq, second.ledger_ref.seq,
        "distinct provenance per call"
    );
    assert_eq!(ledger.scan_entries().unwrap().len(), 2);
    // Kernel result is consistent across the two recomputations.
    assert_eq!(first.kernel_ids, second.kernel_ids);
    assert_eq!(first.scope_hash, second.scope_hash);
}

#[test]
fn inverted_time_window_fails_closed_without_building() {
    let store = corpus(true, true);
    let mut cache = ScopeCache::new(8);
    let clock = FixedClock::new(1);
    let mut ledger = appender();

    let err = summarize(
        &store,
        Scope::TimeWindow {
            t0: 2_000,
            t1: 1_000,
        },
        None,
        &mut ctx(&mut cache, &clock, &mut ledger),
    )
    .expect_err("inverted window must fail closed");

    assert_eq!(err.code, CALYX_SCOPE_INVALID_TIME_WINDOW);
    assert!(
        ledger.scan_entries().unwrap().is_empty(),
        "no kernel built for an invalid window"
    );
}

#[test]
fn scope_hash_is_stable_and_matches_subject() {
    // scope_hash is deterministic and the ledger subject carries exactly it.
    let store = corpus(true, false);
    let mut cache = ScopeCache::new(8);
    let clock = FixedClock::new(1);
    let mut ledger = appender();
    let scope = Scope::Collection {
        id: CollectionId::from("coll-b"),
    };
    let expected = scope_hash(&scope);

    let result = summarize(
        &store,
        scope,
        None,
        &mut ctx(&mut cache, &clock, &mut ledger),
    )
    .expect("summarize");
    assert_eq!(
        result.scope_hash, expected,
        "scope_hash matches the canonical hash"
    );
    let (_, subject, _) = last_entry(&ledger);
    assert_eq!(subject, SubjectId::Kernel(expected.to_vec()));
}

proptest! {
    /// ∀ scope ∈ {AllAssociations, Collection, TimeWindow}: metrics stay in bounds
    /// and the call never panics.
    #[test]
    fn metrics_in_bounds_for_any_scope(which in 0u8..3, lo in 1_000u64..1_010, span in 0u64..30) {
        let store = corpus(true, true);
        let mut cache = ScopeCache::new(8);
        let clock = FixedClock::new(1);
        let mut ledger = appender();
        let scope = match which {
            0 => Scope::AllAssociations,
            1 => Scope::Collection { id: CollectionId::from("coll-a") },
            _ => Scope::TimeWindow { t0: lo, t1: lo + span },
        };
        let result = summarize(&store, scope, None, &mut ctx(&mut cache, &clock, &mut ledger)).expect("summarize");
        prop_assert!((0.0..=1.0).contains(&result.kernel_only_recall));
        prop_assert!((0.0..=1.0).contains(&result.grounded_fraction));
        prop_assert!(result.kernel_size <= 20);
    }
}
