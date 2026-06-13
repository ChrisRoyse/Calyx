use super::*;
use crate::time::FixedClock;
use std::sync::Arc;

fn cache_at(now: Ts, cap: usize, ttl_ms: u64) -> LruTtlCache<u32, u32> {
    LruTtlCache::new(
        cap,
        Duration::from_millis(ttl_ms),
        Arc::new(FixedClock::new(now)),
    )
    .expect("cache")
}

#[test]
fn byte_cap_evicts_lru_to_make_room() {
    // 500-byte cap; ten 100-byte entries -> only 5 live, LRU evicted.
    let mut c = cache_at(0, 500, 60_000);
    for k in 0..10u32 {
        let r = c.insert(k, k, 100).expect("insert");
        println!("insert {k}: used={} evicted={}", c.used_bytes(), r.evicted);
    }
    assert_eq!(c.used_bytes(), 500, "used stays at the cap");
    assert_eq!(c.len(), 5);
    // Earliest keys were evicted; only the last 5 survive.
    for k in 0..5u32 {
        assert!(c.get(&k).is_none(), "old key {k} evicted");
    }
    for k in 5..10u32 {
        assert!(c.get(&k).is_some(), "recent key {k} present");
    }
    assert!(c.evictions() >= 5, "evictions counted: {}", c.evictions());
}

#[test]
fn ttl_expiry_via_advancing_clock() {
    // Use an Arc<Mutex<Ts>>-backed clock so we can advance time in place.
    use std::sync::Mutex;
    struct MovableClock(Mutex<Ts>);
    impl Clock for MovableClock {
        fn now(&self) -> Ts {
            *self.0.lock().unwrap()
        }
    }
    let clock = Arc::new(MovableClock(Mutex::new(1000)));
    let mut c: LruTtlCache<u32, u32> =
        LruTtlCache::new(1000, Duration::from_millis(50), clock.clone()).expect("cache");
    c.insert(1, 7, 100).expect("insert");
    assert_eq!(c.get(&1), Some(&7), "live before TTL");
    *clock.0.lock().unwrap() = 1000 + 51; // advance past TTL
    let got = c.get(&1).copied();
    println!("after advance: get -> {got:?} used={}", c.used_bytes());
    assert_eq!(got, None, "expired after TTL");
    assert_eq!(c.used_bytes(), 0, "expired entry frees its bytes");
    assert_eq!(c.expired_total(), 1);
}

#[test]
fn lru_order_promotes_on_get() {
    let mut c = cache_at(0, 300, 60_000);
    c.insert(b'A'.into(), 1, 100).unwrap();
    c.insert(b'B'.into(), 2, 100).unwrap();
    c.insert(b'C'.into(), 3, 100).unwrap(); // full: A(LRU) B C(MRU)
    assert_eq!(c.get(&u32::from(b'A')), Some(&1)); // promote A -> MRU
    // Insert D -> must evict LRU which is now B (not A).
    let r = c.insert(b'D'.into(), 4, 100).unwrap();
    assert_eq!(r.evicted, 1);
    assert!(c.get(&u32::from(b'B')).is_none(), "B evicted, not A");
    assert!(
        c.get(&u32::from(b'A')).is_some(),
        "A survived (was promoted)"
    );
    assert!(c.get(&u32::from(b'D')).is_some());
    assert_eq!(c.len(), 3);
}

#[test]
fn hit_rate_accounting() {
    let mut c = cache_at(0, 10_000, 60_000);
    for k in 0..10u32 {
        c.insert(k, k, 100).unwrap();
    }
    for k in 0..10u32 {
        assert!(c.get(&k).is_some());
    }
    assert_eq!(c.hit_rate(), 1.0, "10 hits, 0 misses");

    let mut c2 = cache_at(0, 10_000, 60_000);
    for k in 100..110u32 {
        assert!(c2.get(&k).is_none());
    }
    assert_eq!(c2.hit_rate(), 0.0, "10 misses");
}

#[test]
fn single_entry_exactly_cap_then_next_evicts() {
    let mut c = cache_at(0, 256, 60_000);
    c.insert(1, 1, 256).unwrap();
    assert_eq!(c.used_bytes(), 256);
    let r = c.insert(2, 2, 256).unwrap();
    assert_eq!(r.evicted, 1, "first entry evicted to fit the second");
    assert_eq!(c.used_bytes(), 256);
    assert!(c.get(&1).is_none());
    assert!(c.get(&2).is_some());
}

#[test]
fn entry_larger_than_cap_rejected() {
    let mut c = cache_at(0, 256, 60_000);
    let err = c.insert(1, 1, 257).expect_err("too large");
    assert_eq!(err.code, crate::alloc::CALYX_ALLOC_CAP_EXCEEDED);
    assert_eq!(c.used_bytes(), 0, "cache unmodified");
    assert_eq!(c.len(), 0);
}

#[test]
fn zero_cap_rejected() {
    // `Arc<dyn Clock>` is not `Debug`, so match rather than `expect_err`.
    match LruTtlCache::<u32, u32>::new(0, Duration::from_secs(1), Arc::new(FixedClock::new(0))) {
        Ok(_) => panic!("zero byte_cap must be rejected"),
        Err(e) => assert_eq!(e.code, crate::alloc::CALYX_ALLOC_CAP_EXCEEDED),
    }
}

#[test]
fn flood_keeps_used_bytes_bounded() {
    // Insert 10x the cap worth of entries; used_bytes never exceeds cap.
    let mut c = cache_at(0, 1000, 60_000);
    let mut max_used = 0;
    for k in 0..200u32 {
        c.insert(k, k, 100).unwrap();
        max_used = max_used.max(c.used_bytes());
    }
    println!(
        "flood: max_used={max_used} cap={} evictions={}",
        c.byte_cap(),
        c.evictions()
    );
    assert!(max_used <= 1000, "used_bytes bounded by cap under flood");
    assert_eq!(c.used_bytes(), 1000);
    assert!(c.evictions() > 0, "eviction ran under flood");
}

#[test]
fn jitter_spreads_expiry() {
    let mut c: LruTtlCache<u32, u32> = LruTtlCache::with_jitter(
        100_000,
        Duration::from_millis(1000),
        Duration::from_millis(400),
        Arc::new(FixedClock::new(0)),
    )
    .expect("cache");
    let mut expiries = std::collections::BTreeSet::new();
    for k in 0..50u32 {
        c.insert(k, k, 100).unwrap();
        // Inspect the node's expiry directly (SoT).
        let idx = c.map[&k];
        expiries.insert(c.node(idx).expires_at);
    }
    let (lo, hi) = (
        *expiries.iter().next().unwrap(),
        *expiries.iter().next_back().unwrap(),
    );
    println!(
        "jittered expiry range = [{lo}, {hi}] over {} distinct",
        expiries.len()
    );
    assert!(expiries.len() > 1, "jitter produced distinct expiries");
    assert!(lo >= 800 && hi <= 1200, "expiry within base +/- jitter/2");
}

proptest::proptest! {
    #[test]
    fn used_bytes_never_exceeds_cap(
        byte_cap in 1usize..=10_000,
        entries in proptest::collection::vec((0u32..64, 1usize..512), 0..256),
    ) {
        let mut c: LruTtlCache<u32, u32> = LruTtlCache::new(
            byte_cap,
            Duration::from_secs(3600),
            Arc::new(FixedClock::new(0)),
        ).expect("cache");
        for (k, size) in entries {
            if size > byte_cap {
                proptest::prop_assert!(c.insert(k, k, size).is_err());
            } else {
                c.insert(k, k, size).expect("insert within cap");
            }
            proptest::prop_assert!(
                c.used_bytes() <= byte_cap,
                "used {} exceeded cap {}", c.used_bytes(), byte_cap
            );
        }
    }
}
