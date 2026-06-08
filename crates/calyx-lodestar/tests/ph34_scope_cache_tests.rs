use std::fs;
use std::path::PathBuf;

use calyx_core::CxId;
use calyx_lodestar::{GroundednessReport, Kernel, RecallReport, ScopeCache, ScopeCacheKey};
use serde_json::json;

fn key(seed: u8, panel_version: u64) -> ScopeCacheKey {
    ScopeCacheKey::new([seed; 32], panel_version, [0; 32], [0; 32])
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn kernel(seed: u8, panel_version: u64) -> Kernel {
    Kernel {
        kernel_id: cx(seed),
        panel_version,
        anchor_kind: Some("scope-cache-test".to_string()),
        corpus_shard_hash: [seed; 32],
        members: vec![cx(seed)],
        kernel_graph: vec![cx(seed)],
        groundedness: GroundednessReport {
            reached_anchor: 1.0,
            unanchored_members: Vec::new(),
        },
        recall: RecallReport::default(),
        built_at_millis: 1,
        estimator_provenance: "scope-cache-test".to_string(),
        warnings: Vec::new(),
    }
}

fn fsv_root(case: &str) -> PathBuf {
    let base = std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph34-t02"));
    base.join(case)
}

fn write_readback(case: &str, name: &str, value: serde_json::Value) {
    let root = fsv_root(case);
    fs::create_dir_all(&root).expect("create readback root");
    let path = root.join(name);
    fs::write(&path, serde_json::to_vec_pretty(&value).expect("json")).expect("write readback");
    println!("PH34_T02_READBACK={}", path.display());
}

#[test]
fn scope_cache_hits_misses_and_stats() {
    let mut cache = ScopeCache::new(4);
    for seed in 1..=3 {
        cache.insert(key(seed, 7), kernel(seed, 7));
    }
    for seed in 1..=3 {
        assert!(cache.get(&key(seed, 7)).is_some());
    }
    assert!(cache.get(&key(99, 7)).is_none());
    let stats = cache.stats();

    println!(
        "PH34_SCOPE_CACHE_STATS hits={} misses={} size={} max={}",
        stats.hits, stats.misses, stats.current_size, stats.max_entries
    );
    write_readback(
        "stats",
        "ph34-scope-cache-stats-readback.json",
        json!({ "stats": stats }),
    );

    assert_eq!(stats.hits, 3);
    assert_eq!(stats.misses, 1);
    assert_eq!(stats.current_size, 3);
}

#[test]
fn scope_cache_capacity_two_evicts_first_inserted() {
    let mut cache = ScopeCache::new(2);
    cache.insert(key(1, 7), kernel(1, 7));
    cache.insert(key(2, 7), kernel(2, 7));
    cache.insert(key(3, 7), kernel(3, 7));
    let first_absent = cache.get(&key(1, 7)).is_none();
    let second_present = cache.get(&key(2, 7)).is_some();
    let third_present = cache.get(&key(3, 7)).is_some();
    let stats = cache.stats();

    println!(
        "PH34_SCOPE_CACHE_EVICTION first_absent={} size={} hits={} misses={}",
        first_absent, stats.current_size, stats.hits, stats.misses
    );
    write_readback(
        "eviction",
        "ph34-scope-cache-eviction-readback.json",
        json!({
            "first_absent": first_absent,
            "second_present": second_present,
            "third_present": third_present,
            "stats": stats,
        }),
    );

    assert!(first_absent);
    assert!(second_present);
    assert!(third_present);
    assert_eq!(stats.current_size, 2);
}

#[test]
fn scope_cache_invalidate_panel_version_removes_only_matching_entries() {
    let mut cache = ScopeCache::new(4);
    cache.insert(key(1, 1), kernel(1, 1));
    cache.insert(key(2, 1), kernel(2, 1));
    cache.insert(key(3, 2), kernel(3, 2));
    let removed = cache.invalidate_panel_version(1);
    let v1_absent = cache.get(&key(1, 1)).is_none() && cache.get(&key(2, 1)).is_none();
    let v2_present = cache.get(&key(3, 2)).is_some();
    let stats = cache.stats();

    println!(
        "PH34_SCOPE_CACHE_INVALIDATE removed={} v1_absent={} v2_present={} size={}",
        removed, v1_absent, v2_present, stats.current_size
    );
    write_readback(
        "invalidate",
        "ph34-scope-cache-invalidate-readback.json",
        json!({
            "removed": removed,
            "v1_absent": v1_absent,
            "v2_present": v2_present,
            "stats": stats,
        }),
    );

    assert_eq!(removed, 2);
    assert!(v1_absent);
    assert!(v2_present);
    assert_eq!(stats.current_size, 1);
}

#[test]
fn scope_cache_zero_capacity_and_max_panel_version_are_safe() {
    assert_send_sync::<ScopeCache>();
    let mut zero = ScopeCache::new(0);
    zero.insert(key(1, 7), kernel(1, 7));
    let zero_stats = zero.stats();

    let mut max_panel = ScopeCache::new(1);
    let max_key = ScopeCacheKey::new([254; 32], u64::MAX, [1; 32], [2; 32]);
    max_panel.insert(max_key, kernel(9, u64::MAX));
    let max_present = max_panel.get(&max_key).is_some();
    let max_stats = max_panel.stats();

    println!(
        "PH34_SCOPE_CACHE_EDGES zero_size={} max_present={} max_panel={}",
        zero_stats.current_size, max_present, max_key.panel_version
    );
    write_readback(
        "edges",
        "ph34-scope-cache-edges-readback.json",
        json!({
            "zero_capacity_size": zero_stats.current_size,
            "max_panel_present": max_present,
            "max_panel_version": max_key.panel_version,
            "max_stats": max_stats,
        }),
    );

    assert_eq!(zero_stats.current_size, 0);
    assert!(max_present);
    assert_eq!(max_stats.current_size, 1);
}

#[test]
fn scope_cache_key_includes_anchor_and_corpus_identity() {
    let base = ScopeCacheKey::new([1; 32], 7, [2; 32], [3; 32]);
    let changed_anchor = ScopeCacheKey::new([1; 32], 7, [4; 32], [3; 32]);
    let changed_corpus = ScopeCacheKey::new([1; 32], 7, [2; 32], [5; 32]);

    assert_ne!(base, changed_anchor);
    assert_ne!(base, changed_corpus);
}

fn assert_send_sync<T: Send + Sync>() {}
