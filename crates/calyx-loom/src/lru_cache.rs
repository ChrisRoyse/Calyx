//! Small deterministic LRU cache for lazy cross-terms.

use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Debug)]
pub struct LruCache<K, V> {
    capacity: usize,
    map: BTreeMap<K, V>,
    order: VecDeque<K>,
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
        }
    }

    pub fn get(&mut self, key: &K) -> Option<V> {
        let value = self.map.get(key).cloned()?;
        self.touch(key);
        Some(value)
    }

    pub fn put(&mut self, key: K, value: V) {
        if self.map.contains_key(&key) {
            self.map.insert(key.clone(), value);
            self.touch(&key);
            return;
        }
        while self.map.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
        self.order.push_back(key.clone());
        self.map.insert(key, value);
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    fn touch(&mut self, key: &K) {
        self.order.retain(|item| item != key);
        self.order.push_back(key.clone());
    }
}
