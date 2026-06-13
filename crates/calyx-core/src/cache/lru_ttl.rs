//! Generic LRU + TTL, byte-capped cache (PH56 · T03).
//!
//! [`LruTtlCache`] bounds itself three ways: a hard byte cap (sum of entry sizes
//! never exceeds it), LRU eviction when the cap is hit, and a per-entry TTL
//! measured against an injected [`Clock`](crate::Clock). Recency order is an
//! intrusive doubly-linked list over a node arena (O(1) get/insert/evict) — no
//! external map crate, no `SystemTime::now()` in logic. Optional TTL jitter
//! de-synchronizes expiry to avoid a cache-stampede herd (hazard 15).

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use std::time::Duration;

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::alloc::alloc_cap_exceeded;
use crate::{Clock, Result, Ts};

/// Emitted (as a structured log event, never a panic) whenever an entry is
/// evicted to honor the byte cap.
pub const CALYX_CACHE_EVICTED: &str = "CALYX_CACHE_EVICTED";

/// Fixed seed for the jitter RNG so jittered TTLs are reproducible in FSV.
const JITTER_SEED: u64 = 0xCA17_8C0F_FEE5_1D0F;

/// Outcome of an [`LruTtlCache::insert`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InsertResult {
    /// Number of LRU entries evicted to make room for the new one.
    pub evicted: usize,
}

struct Node<K, V> {
    key: K,
    value: V,
    size_bytes: usize,
    expires_at: Ts,
    prev: Option<usize>,
    next: Option<usize>,
}

/// A byte-capped LRU cache with per-entry TTL.
pub struct LruTtlCache<K, V> {
    map: HashMap<K, usize>,
    nodes: Vec<Option<Node<K, V>>>,
    free: Vec<usize>,
    /// Most-recently-used end of the recency list.
    head: Option<usize>,
    /// Least-recently-used end (evicted first).
    tail: Option<usize>,
    byte_cap: usize,
    used_bytes: usize,
    ttl_ms: u64,
    jitter_ms: u64,
    rng: ChaCha8Rng,
    clock: Arc<dyn Clock>,
    hits: u64,
    misses: u64,
    evictions: u64,
    expired: u64,
}

impl<K, V> LruTtlCache<K, V>
where
    K: Clone + Eq + Hash,
{
    /// Builds a cache with a hard `byte_cap`, uniform `ttl`, and injected clock.
    /// Errors with [`CALYX_ALLOC_CAP_EXCEEDED`](crate::alloc::CALYX_ALLOC_CAP_EXCEEDED)
    /// if `byte_cap == 0`.
    pub fn new(byte_cap: usize, ttl: Duration, clock: Arc<dyn Clock>) -> Result<Self> {
        Self::with_jitter(byte_cap, ttl, Duration::ZERO, clock)
    }

    /// Like [`new`](Self::new) but each entry's TTL is randomized by `±jitter/2`
    /// to prevent synchronized mass expiry (cache stampede). Errors with
    /// [`CALYX_ALLOC_CAP_EXCEEDED`](crate::alloc::CALYX_ALLOC_CAP_EXCEEDED) if
    /// `byte_cap == 0`.
    pub fn with_jitter(
        byte_cap: usize,
        ttl: Duration,
        jitter: Duration,
        clock: Arc<dyn Clock>,
    ) -> Result<Self> {
        if byte_cap == 0 {
            return Err(alloc_cap_exceeded("cache byte_cap must be > 0"));
        }
        Ok(Self {
            map: HashMap::new(),
            nodes: Vec::new(),
            free: Vec::new(),
            head: None,
            tail: None,
            byte_cap,
            used_bytes: 0,
            ttl_ms: ttl.as_millis().min(u128::from(u64::MAX)) as u64,
            jitter_ms: jitter.as_millis().min(u128::from(u64::MAX)) as u64,
            rng: ChaCha8Rng::seed_from_u64(JITTER_SEED),
            clock,
            hits: 0,
            misses: 0,
            evictions: 0,
            expired: 0,
        })
    }

    /// Looks up `key`. An entry past its TTL is evicted and reported as a miss;
    /// a live hit is promoted to most-recently-used.
    pub fn get(&mut self, key: &K) -> Option<&V> {
        let now = self.clock.now();
        let idx = match self.map.get(key).copied() {
            Some(i) => i,
            None => {
                self.misses += 1;
                return None;
            }
        };
        if self.node(idx).expires_at <= now {
            self.remove_index(idx);
            self.expired += 1;
            self.misses += 1;
            return None;
        }
        self.move_to_front(idx);
        self.hits += 1;
        Some(&self.node(idx).value)
    }

    /// Inserts `key -> value` accounted at `size_bytes`. Evicts LRU entries (and
    /// any TTL-expired entries) so `used_bytes` never exceeds the cap.
    ///
    /// # Errors
    /// [`CALYX_ALLOC_CAP_EXCEEDED`](crate::alloc::CALYX_ALLOC_CAP_EXCEEDED) if a
    /// single entry is larger than the whole cap (it could never fit).
    pub fn insert(&mut self, key: K, value: V, size_bytes: usize) -> Result<InsertResult> {
        if size_bytes > self.byte_cap {
            return Err(alloc_cap_exceeded(format!(
                "cache entry of {size_bytes} bytes exceeds byte_cap {}",
                self.byte_cap
            )));
        }
        self.evict_expired();
        // Replacing an existing key: drop the old entry first (not an eviction).
        if let Some(old) = self.map.get(&key).copied() {
            self.remove_index(old);
        }
        let mut evicted = 0;
        while self.used_bytes + size_bytes > self.byte_cap {
            let lru = self.tail.expect("over-cap cache must have a tail");
            self.emit_evicted(lru);
            self.remove_index(lru);
            self.evictions += 1;
            evicted += 1;
        }
        let now = self.clock.now();
        let expires_at = self.compute_expiry(now);
        let idx = self.alloc_node(Node {
            key: key.clone(),
            value,
            size_bytes,
            expires_at,
            prev: None,
            next: None,
        });
        self.map.insert(key, idx);
        self.used_bytes += size_bytes;
        self.push_front(idx);
        Ok(InsertResult { evicted })
    }

    /// Sweeps and removes every TTL-expired entry. Returns how many were removed.
    pub fn evict_expired(&mut self) -> usize {
        let now = self.clock.now();
        let expired: Vec<usize> = (0..self.nodes.len())
            .filter(|&i| {
                self.nodes[i]
                    .as_ref()
                    .is_some_and(|n| now >= n.expires_at)
            })
            .collect();
        let count = expired.len();
        for i in expired {
            self.remove_index(i);
            self.expired += 1;
        }
        count
    }

    /// Live entry count.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True when no live entries remain.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Bytes currently accounted (never exceeds the cap) — the FSV Source of Truth.
    pub fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    /// Hard byte cap.
    pub fn byte_cap(&self) -> usize {
        self.byte_cap
    }

    /// Hit rate `hits / (hits + misses)`; `0.0` before any access.
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }

    /// Total LRU evictions performed (monotonic) — the `cache_evictions_total` SoT.
    pub fn evictions(&self) -> u64 {
        self.evictions
    }

    /// Total TTL-expired removals (monotonic).
    pub fn expired_total(&self) -> u64 {
        self.expired
    }

    fn node(&self, idx: usize) -> &Node<K, V> {
        self.nodes[idx].as_ref().expect("live node index")
    }

    fn compute_expiry(&mut self, now: Ts) -> Ts {
        let base = now.saturating_add(self.ttl_ms);
        if self.jitter_ms == 0 {
            return base;
        }
        let half = (self.jitter_ms / 2) as i64;
        let offset = self.rng.gen_range(-half..=half);
        if offset >= 0 {
            base.saturating_add(offset as u64)
        } else {
            base.saturating_sub(offset.unsigned_abs())
        }
    }

    fn emit_evicted(&self, idx: usize) {
        let n = self.node(idx);
        tracing::debug!(
            target: "calyx::cache",
            event = CALYX_CACHE_EVICTED,
            key_type = std::any::type_name::<K>(),
            size_bytes = n.size_bytes,
            "lru eviction"
        );
    }

    fn alloc_node(&mut self, node: Node<K, V>) -> usize {
        if let Some(i) = self.free.pop() {
            self.nodes[i] = Some(node);
            i
        } else {
            self.nodes.push(Some(node));
            self.nodes.len() - 1
        }
    }

    fn push_front(&mut self, idx: usize) {
        let old_head = self.head;
        {
            let n = self.nodes[idx].as_mut().expect("live node");
            n.prev = None;
            n.next = old_head;
        }
        if let Some(h) = old_head {
            self.nodes[h].as_mut().expect("live head").prev = Some(idx);
        }
        self.head = Some(idx);
        if self.tail.is_none() {
            self.tail = Some(idx);
        }
    }

    fn unlink(&mut self, idx: usize) {
        let (prev, next) = {
            let n = self.nodes[idx].as_ref().expect("live node");
            (n.prev, n.next)
        };
        match prev {
            Some(p) => self.nodes[p].as_mut().expect("live prev").next = next,
            None => self.head = next,
        }
        match next {
            Some(nx) => self.nodes[nx].as_mut().expect("live next").prev = prev,
            None => self.tail = prev,
        }
        let n = self.nodes[idx].as_mut().expect("live node");
        n.prev = None;
        n.next = None;
    }

    fn move_to_front(&mut self, idx: usize) {
        if self.head == Some(idx) {
            return;
        }
        self.unlink(idx);
        self.push_front(idx);
    }

    fn remove_index(&mut self, idx: usize) {
        self.unlink(idx);
        let node = self.nodes[idx].take().expect("removing a live node");
        self.used_bytes -= node.size_bytes;
        self.map.remove(&node.key);
        self.free.push(idx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::FixedClock;
    use std::sync::Arc;

    fn cache_at(now: Ts, cap: usize, ttl_ms: u64) -> LruTtlCache<u32, u32> {
        LruTtlCache::new(cap, Duration::from_millis(ttl_ms), Arc::new(FixedClock::new(now)))
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
        assert!(c.get(&u32::from(b'A')).is_some(), "A survived (was promoted)");
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
        println!("flood: max_used={max_used} cap={} evictions={}", c.byte_cap(), c.evictions());
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
        let (lo, hi) = (*expiries.iter().next().unwrap(), *expiries.iter().next_back().unwrap());
        println!("jittered expiry range = [{lo}, {hi}] over {} distinct", expiries.len());
        assert!(expiries.len() > 1, "jitter produced distinct expiries");
        assert!(lo >= 800 && hi <= 1200, "expiry within base ± jitter/2");
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
}
