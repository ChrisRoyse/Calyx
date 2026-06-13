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
            .filter(|&i| self.nodes[i].as_ref().is_some_and(|n| now >= n.expires_at))
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
mod tests;
