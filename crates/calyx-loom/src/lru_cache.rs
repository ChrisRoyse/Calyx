//! Small deterministic LRU cache for lazy cross-terms.

use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Debug)]
pub struct LruCache<K, V> {
    capacity: usize,
    map: BTreeMap<K, CacheEntry<V>>,
    order: VecDeque<(K, u64)>,
    next_stamp: u64,
}

#[derive(Clone, Debug)]
struct CacheEntry<V> {
    value: V,
    stamp: u64,
}

impl<K, V> LruCache<K, V>
where
    K: Clone + Ord,
    V: Clone,
{
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            map: BTreeMap::new(),
            order: VecDeque::new(),
            next_stamp: 0,
        }
    }

    pub fn get(&mut self, key: &K) -> Option<V> {
        let value = self.map.get(key)?.value.clone();
        self.touch(key);
        Some(value)
    }

    pub fn put(&mut self, key: K, value: V) {
        let stamp = self.next_stamp();
        if let Some(entry) = self.map.get_mut(&key) {
            entry.value = value;
            entry.stamp = stamp;
            self.order.push_back((key, stamp));
            self.compact_stale_order();
            return;
        }
        while self.map.len() >= self.capacity {
            if !self.evict_one() {
                break;
            }
        }
        self.order.push_back((key.clone(), stamp));
        self.map.insert(key, CacheEntry { value, stamp });
        self.compact_stale_order();
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    fn touch(&mut self, key: &K) {
        let stamp = self.next_stamp();
        if let Some(entry) = self.map.get_mut(key) {
            entry.stamp = stamp;
            self.order.push_back((key.clone(), stamp));
            self.compact_stale_order();
        }
    }

    fn evict_one(&mut self) -> bool {
        while let Some((oldest, stamp)) = self.order.pop_front() {
            if self
                .map
                .get(&oldest)
                .is_some_and(|entry| entry.stamp == stamp)
            {
                self.map.remove(&oldest);
                return true;
            }
        }
        false
    }

    fn next_stamp(&mut self) -> u64 {
        let stamp = self.next_stamp;
        self.next_stamp = self.next_stamp.wrapping_add(1);
        stamp
    }

    fn compact_stale_order(&mut self) {
        let max_order_len = self.capacity.saturating_mul(4).max(4);
        if self.order.len() <= max_order_len {
            return;
        }
        let mut compact = VecDeque::with_capacity(self.map.len());
        while let Some((key, stamp)) = self.order.pop_front() {
            if self.map.get(&key).is_some_and(|entry| entry.stamp == stamp) {
                compact.push_back((key, stamp));
            }
        }
        self.order = compact;
    }
}

#[cfg(test)]
mod tests {
    use super::LruCache;

    #[test]
    fn get_refreshes_recency_without_duplicate_eviction() {
        let mut cache = LruCache::new(2);
        cache.put("a", 1);
        cache.put("b", 2);

        assert_eq!(cache.get(&"a"), Some(1));
        cache.put("c", 3);

        assert_eq!(cache.get(&"a"), Some(1));
        assert_eq!(cache.get(&"b"), None);
        assert_eq!(cache.get(&"c"), Some(3));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn put_existing_refreshes_recency_and_value() {
        let mut cache = LruCache::new(2);
        cache.put("a", 1);
        cache.put("b", 2);
        cache.put("a", 10);
        cache.put("c", 3);

        assert_eq!(cache.get(&"a"), Some(10));
        assert_eq!(cache.get(&"b"), None);
        assert_eq!(cache.get(&"c"), Some(3));
    }

    #[test]
    fn repeated_touches_do_not_grow_stale_order_without_bound() {
        let mut cache = LruCache::new(2);
        cache.put("a", 1);
        cache.put("b", 2);

        for _ in 0..32 {
            assert_eq!(cache.get(&"a"), Some(1));
        }

        assert!(cache.order.len() <= 8);
    }
}
