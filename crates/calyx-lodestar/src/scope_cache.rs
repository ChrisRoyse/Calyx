use std::collections::{BTreeMap, VecDeque};

use calyx_core::{AnchorKind, CxId};
use serde::{Deserialize, Serialize};

use crate::Kernel;

const DEFAULT_MAX_ENTRIES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ScopeCacheKey {
    pub scope_hash: [u8; 32],
    pub panel_version: u64,
    pub anchor_identity: [u8; 32],
    pub corpus_identity: [u8; 32],
}

impl ScopeCacheKey {
    pub const fn new(
        scope_hash: [u8; 32],
        panel_version: u64,
        anchor_identity: [u8; 32],
        corpus_identity: [u8; 32],
    ) -> Self {
        Self {
            scope_hash,
            panel_version,
            anchor_identity,
            corpus_identity,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub current_size: usize,
    pub max_entries: usize,
}

#[derive(Clone, Debug)]
pub struct ScopeCache {
    entries: BTreeMap<ScopeCacheKey, Kernel>,
    lru: VecDeque<ScopeCacheKey>,
    max_entries: usize,
    hits: u64,
    misses: u64,
}

impl ScopeCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            lru: VecDeque::new(),
            max_entries,
            hits: 0,
            misses: 0,
        }
    }

    pub fn get(&mut self, key: &ScopeCacheKey) -> Option<&Kernel> {
        if self.entries.contains_key(key) {
            self.hits += 1;
            self.touch(*key);
            self.entries.get(key)
        } else {
            self.misses += 1;
            None
        }
    }

    pub fn insert(&mut self, key: ScopeCacheKey, kernel: Kernel) {
        if self.max_entries == 0 {
            log_eviction(&key, "zero_capacity");
            return;
        }
        self.entries.insert(key, kernel);
        self.touch(key);
        while self.entries.len() > self.max_entries {
            let Some(evicted) = self.lru.pop_front() else {
                break;
            };
            if self.entries.remove(&evicted).is_some() {
                log_eviction(&evicted, "capacity");
            }
        }
    }

    pub fn invalidate_panel_version(&mut self, old_version: u64) -> usize {
        let keys: Vec<_> = self
            .entries
            .keys()
            .copied()
            .filter(|key| key.panel_version == old_version)
            .collect();
        for key in &keys {
            self.entries.remove(key);
        }
        self.lru
            .retain(|key| key.panel_version != old_version && self.entries.contains_key(key));
        keys.len()
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits,
            misses: self.misses,
            current_size: self.entries.len(),
            max_entries: self.max_entries,
        }
    }

    fn touch(&mut self, key: ScopeCacheKey) {
        self.lru.retain(|known| known != &key);
        if self.entries.contains_key(&key) {
            self.lru.push_back(key);
        }
    }
}

impl Default for ScopeCache {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_ENTRIES)
    }
}

fn log_eviction(key: &ScopeCacheKey, reason: &str) {
    eprintln!(
        "CALYX_SCOPE_CACHE_EVICT reason={reason} scope_hash={} panel_version={} anchor_identity={} corpus_identity={}",
        hex32(&key.scope_hash),
        key.panel_version,
        hex32(&key.anchor_identity),
        hex32(&key.corpus_identity)
    );
}

pub fn scope_cache_anchor_identity(anchor_kinds: &[AnchorKind], anchors: &[CxId]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"calyx-lodestar-scope-anchor-identity-v1");
    for kind in anchor_kinds {
        let bytes = serde_json::to_vec(kind).expect("anchor kind serializes");
        frame(&mut hasher, &bytes);
    }
    for anchor in anchors {
        frame(&mut hasher, anchor.as_bytes());
    }
    *hasher.finalize().as_bytes()
}

fn frame(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn hex32(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
