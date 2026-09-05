//! Byte-bounded exact-rerank panel cache for `weave-loom` (#1997/#1999).

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use calyx_core::CxId;
use calyx_lodestar::TypedPanelVectors;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub(super) struct PanelCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub oversized_not_cached: u64,
    pub resident_entries: usize,
    pub resident_bytes: u64,
    pub max_bytes: u64,
}

struct CacheEntry {
    panel: Arc<TypedPanelVectors>,
    bytes: u64,
    stamp: u64,
}

/// Deterministic LRU whose admission and eviction policy is measured in bytes,
/// not row count. Panel sizes differ by orders of magnitude once chunk-Multi
/// lanes are present, so a count-only cache is not a memory bound.
pub(super) struct PanelCache {
    entries: BTreeMap<CxId, CacheEntry>,
    order: VecDeque<(CxId, u64)>,
    next_stamp: u64,
    stats: PanelCacheStats,
}

impl PanelCache {
    pub(super) fn new(max_bytes: u64) -> Self {
        Self {
            entries: BTreeMap::new(),
            order: VecDeque::new(),
            next_stamp: 0,
            stats: PanelCacheStats {
                max_bytes,
                ..PanelCacheStats::default()
            },
        }
    }

    /// Look up one panel while updating deterministic LRU recency and telemetry.
    /// A miss is counted here, before decode, so callers can decode several
    /// independent misses in parallel before admitting them in stable input order.
    pub(super) fn lookup(&mut self, cx_id: CxId) -> Option<Arc<TypedPanelVectors>> {
        if !self.entries.contains_key(&cx_id) {
            self.stats.misses += 1;
            return None;
        }
        self.stats.hits += 1;
        let stamp = self.next_stamp();
        let entry = self.entries.get_mut(&cx_id).expect("entry exists");
        entry.stamp = stamp;
        let panel = Arc::clone(&entry.panel);
        self.order.push_back((cx_id, stamp));
        self.compact_order();
        Some(panel)
    }

    /// Admit a panel that was decoded outside the cache. Admission and eviction are
    /// still serialized and deterministic; only the pure scratch decode may run in
    /// parallel. The returned `Arc` remains valid even if a later admission evicts
    /// this entry from the LRU.
    pub(super) fn admit_loaded(
        &mut self,
        cx_id: CxId,
        bytes: u64,
        panel: TypedPanelVectors,
    ) -> Arc<TypedPanelVectors> {
        let panel = Arc::new(panel);
        if bytes > self.stats.max_bytes {
            self.stats.oversized_not_cached += 1;
            return panel;
        }
        while self.stats.resident_bytes.saturating_add(bytes) > self.stats.max_bytes {
            if !self.evict_one() {
                break;
            }
        }
        let stamp = self.next_stamp();
        self.order.push_back((cx_id, stamp));
        self.entries.insert(
            cx_id,
            CacheEntry {
                panel: Arc::clone(&panel),
                bytes,
                stamp,
            },
        );
        self.stats.resident_bytes = self.stats.resident_bytes.saturating_add(bytes);
        self.stats.resident_entries = self.entries.len();
        self.compact_order();
        panel
    }

    pub(super) fn stats(&self) -> PanelCacheStats {
        self.stats
    }

    fn evict_one(&mut self) -> bool {
        while let Some((cx_id, stamp)) = self.order.pop_front() {
            if self
                .entries
                .get(&cx_id)
                .is_some_and(|entry| entry.stamp == stamp)
            {
                let entry = self.entries.remove(&cx_id).expect("entry exists");
                self.stats.resident_bytes = self.stats.resident_bytes.saturating_sub(entry.bytes);
                self.stats.resident_entries = self.entries.len();
                self.stats.evictions += 1;
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

    fn compact_order(&mut self) {
        let max_len = self.entries.len().saturating_mul(4).max(4);
        if self.order.len() <= max_len {
            return;
        }
        self.order.retain(|(cx_id, stamp)| {
            self.entries
                .get(cx_id)
                .is_some_and(|entry| entry.stamp == *stamp)
        });
    }
}
