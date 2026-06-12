//! Active reader-lease registry for oldest-pinned-seq gap accounting.

use crate::mvcc::ReaderLease;
use calyx_core::{Seq, Ts};
use std::collections::BTreeMap;
use std::sync::Mutex;

/// Registry of live reader leases pinned through `VersionedCfStore::pin_snapshot`.
///
/// Bounded by construction (A26): every lease carries an expiry, and expired
/// entries are pruned on every register/view, so the registry can never grow
/// past the set of leases that are still within their `max_age_ms` window.
/// Ad-hoc internal snapshot handles (lease id 0, vault-internal reads) are
/// intentionally not registered: the oldest-pinned-seq gap tracks explicit
/// long readers, the hazard PRD 24 §7 row 6 cares about.
#[derive(Debug, Default)]
pub struct LeaseRegistry {
    entries: Mutex<BTreeMap<u64, LeaseEntry>>,
}

#[derive(Clone, Copy, Debug)]
struct LeaseEntry {
    pinned_seq: Seq,
    expires_at: Ts,
}

/// Live-lease view used by the resource status collector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LeaseView {
    /// Number of unexpired leases.
    pub active_leases: usize,
    /// Smallest pinned sequence among unexpired leases.
    pub oldest_pinned_seq: Option<Seq>,
}

impl LeaseRegistry {
    /// Registers a freshly issued lease, pruning entries already expired.
    pub fn register(&self, lease: ReaderLease) {
        let mut entries = self.lock();
        prune_expired(&mut entries, lease.issued_at());
        entries.insert(
            lease.id(),
            LeaseEntry {
                pinned_seq: lease.pinned_seq(),
                expires_at: lease.expires_at(),
            },
        );
    }

    /// Releases one lease; returns whether it was still registered.
    pub fn release(&self, lease_id: u64) -> bool {
        self.lock().remove(&lease_id).is_some()
    }

    /// Returns the live view at `now`, pruning expired leases first.
    pub fn live_view(&self, now: Ts) -> LeaseView {
        let mut entries = self.lock();
        prune_expired(&mut entries, now);
        LeaseView {
            active_leases: entries.len(),
            oldest_pinned_seq: entries.values().map(|entry| entry.pinned_seq).min(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<u64, LeaseEntry>> {
        self.entries.lock().expect("lease registry poisoned")
    }
}

fn prune_expired(entries: &mut BTreeMap<u64, LeaseEntry>, now: Ts) {
    entries.retain(|_, entry| now < entry.expires_at);
}
