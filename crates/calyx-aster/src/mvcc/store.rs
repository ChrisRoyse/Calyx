//! In-memory MVCC row table used to define the cross-CF snapshot contract.

mod gc;
mod read;
mod scan_pages;
use crate::cf::{CfRouter, ColumnFamily, KeyRange};
use crate::gc::{SnapshotGcCounters, SnapshotGcReclaimer, SnapshotGcTick};
use crate::mvcc::{
    Freshness, ReadBarrier, ReaderLease, SeqAllocator, Snapshot, read_barrier::first_blocking,
};
use crate::resource::{
    LeaseRegistry, LeaseView, MemtableCfStatus, MemtableStatus, ResourceCounters,
};
use crate::sst::{SstEntry, SstSummary};
use calyx_core::{Clock, Result, Seq, Ts};
use std::collections::BTreeMap;
use std::ops::Bound;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

const TOMBSTONE_VALUE: &[u8] = b"\0CALYX_ASTER_TOMBSTONE_V1";

#[derive(Clone, Debug, PartialEq, Eq)]
struct VersionedValue {
    seq: Seq,
    value: Vec<u8>,
}

type CfKey = (ColumnFamily, Vec<u8>);
type VersionChain = Vec<VersionedValue>;
type RowTable = BTreeMap<CfKey, VersionChain>;

/// One CF/key read requested against a snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CfRead {
    pub cf: ColumnFamily,
    pub key: Vec<u8>,
}

impl CfRead {
    pub fn new(cf: ColumnFamily, key: impl Into<Vec<u8>>) -> Self {
        Self {
            cf,
            key: key.into(),
        }
    }
}

pub fn tombstone_value() -> Vec<u8> {
    TOMBSTONE_VALUE.to_vec()
}

pub fn is_tombstone_value(value: &[u8]) -> bool {
    value == TOMBSTONE_VALUE
}

/// Versioned row table with a single vault-wide sequence.
#[derive(Debug)]
pub struct VersionedCfStore {
    seqs: SeqAllocator,
    /// Max committed seq whose batch wrote at least one row in a CF that
    /// feeds derived search content (issue #1100). Advances inside the row
    /// write lock *before* the seq becomes visible, so any reader that
    /// observes a content commit's seq also observes its watermark.
    derived_content_seq: AtomicU64,
    next_lease_id: AtomicU64,
    rows: RwLock<RowTable>,
    router: RwLock<Option<CfRouter>>,
    router_latest_readback: AtomicBool,
    read_barriers: RwLock<Vec<ReadBarrier>>,
    leases: LeaseRegistry,
    resource_counters: Arc<ResourceCounters>,
    snapshot_gc: SnapshotGcReclaimer,
    snapshot_gc_counters: SnapshotGcCounters,
}

impl VersionedCfStore {
    pub fn new(start_seq: Seq) -> Self {
        Self {
            seqs: SeqAllocator::new(start_seq),
            derived_content_seq: AtomicU64::new(0),
            next_lease_id: AtomicU64::new(0),
            rows: RwLock::new(BTreeMap::new()),
            router: RwLock::new(None),
            router_latest_readback: AtomicBool::new(false),
            read_barriers: RwLock::new(Vec::new()),
            leases: LeaseRegistry::default(),
            resource_counters: Arc::new(ResourceCounters::default()),
            snapshot_gc: SnapshotGcReclaimer::default(),
            snapshot_gc_counters: SnapshotGcCounters::default(),
        }
    }

    pub fn new_with_router(start_seq: Seq, router: CfRouter) -> Self {
        let resource_counters = router.resource_counters();
        Self {
            seqs: SeqAllocator::new(start_seq),
            derived_content_seq: AtomicU64::new(0),
            next_lease_id: AtomicU64::new(0),
            rows: RwLock::new(BTreeMap::new()),
            router: RwLock::new(Some(router)),
            router_latest_readback: AtomicBool::new(false),
            read_barriers: RwLock::new(Vec::new()),
            leases: LeaseRegistry::default(),
            resource_counters,
            snapshot_gc: SnapshotGcReclaimer::default(),
            snapshot_gc_counters: SnapshotGcCounters::default(),
        }
    }

    pub fn new_with_router_latest_readback(start_seq: Seq, router: CfRouter) -> Self {
        let store = Self::new_with_router(start_seq, router);
        store.router_latest_readback.store(true, Ordering::Release);
        store
    }

    /// Latest committed sequence.
    pub fn current_seq(&self) -> Seq {
        self.seqs.current()
    }

    pub fn set_start_seq(&self, seq: Seq) -> Result<()> {
        self.seqs.set_start_seq(seq)
    }

    pub fn advance_to_at_least(&self, seq: Seq) {
        self.seqs.advance_to_at_least(seq);
    }

    /// Latest committed seq whose batch wrote derived-search-content inputs.
    /// See [`crate::cf::ColumnFamily::feeds_derived_search_content`].
    pub fn derived_content_seq(&self) -> Seq {
        self.derived_content_seq.load(Ordering::Acquire)
    }

    /// Raises the derived-content watermark to a durably recorded floor
    /// (vault MANIFEST readback, foreign-process checkpoint refresh).
    pub fn advance_derived_content_seq_to_at_least(&self, seq: Seq) {
        self.derived_content_seq.fetch_max(seq, Ordering::AcqRel);
    }

    /// Pins a snapshot at the latest committed sequence.
    ///
    /// The lease is registered for oldest-pinned-seq gap accounting; it leaves
    /// the registry on [`Self::release_lease`] or when its `max_age_ms` expires.
    pub fn pin_snapshot(
        &self,
        freshness: Freshness,
        clock: &dyn Clock,
        max_age_ms: u64,
    ) -> Snapshot {
        let seq = self.current_seq();
        let lease_id = self.next_lease_id.fetch_add(1, Ordering::AcqRel) + 1;
        let lease = ReaderLease::new(lease_id, seq, clock.now(), max_age_ms);
        self.leases.register(lease);
        Snapshot::new(seq, freshness, lease)
            .with_derived_content_seq(self.derived_content_seq_at(seq))
    }

    /// Pins a reader lease at an explicit historical `seq` (time-travel). The
    /// lease participates in oldest-pinned-seq accounting so version GC cannot
    /// reclaim versions at or below `seq` until it is released.
    pub fn pin_snapshot_at(
        &self,
        seq: Seq,
        freshness: Freshness,
        clock: &dyn Clock,
        max_age_ms: u64,
    ) -> Snapshot {
        let lease_id = self.next_lease_id.fetch_add(1, Ordering::AcqRel) + 1;
        let lease = ReaderLease::new(lease_id, seq, clock.now(), max_age_ms);
        self.leases.register(lease);
        Snapshot::new(seq, freshness, lease)
            .with_derived_content_seq(self.derived_content_seq_at(seq))
    }

    /// Derived-content watermark as knowable for a pin at `seq`, clamped
    /// fail-closed: if the live watermark exceeds `seq` (content committed
    /// after the pin, or a historical pin below the watermark), the watermark
    /// at `seq` is unknowable from the live counter and the pin falls back to
    /// `seq` itself — the pre-#1100 exact-equality behavior, never laxer.
    fn derived_content_seq_at(&self, seq: Seq) -> Seq {
        self.derived_content_seq().min(seq)
    }

    /// Releases one pinned reader lease; returns whether it was still live.
    pub fn release_lease(&self, lease_id: u64) -> bool {
        self.leases.release(lease_id)
    }

    /// Live reader-lease view at `now` for resource accounting.
    pub fn lease_view(&self, now: Ts) -> LeaseView {
        self.leases.live_view(now)
    }

    /// Background snapshot-GC tick hook, intended for the 1 s GC scheduler.
    pub fn snapshot_gc_tick(&self, clock: &dyn Clock, max_gap_seqs: u64) -> SnapshotGcTick {
        let now = clock.now();
        let aborted_readers = self.leases.check_and_abort_expired(now);
        let gap_alert = self.leases.check_gap(self.current_seq(), now, max_gap_seqs);
        let metrics = self.leases.metrics(self.current_seq(), now);
        SnapshotGcTick {
            aborted_readers,
            gap_alert,
            metrics,
        }
    }

    /// Backpressure counters shared with this store's CF router.
    pub fn resource_counters(&self) -> &ResourceCounters {
        &self.resource_counters
    }

    /// Live memtable byte-cap status shared with resource readback.
    pub fn memtable_status(&self) -> MemtableStatus {
        let router = self.router.read().expect("mvcc router poisoned");
        let Some(router) = router.as_ref() else {
            return MemtableStatus::default();
        };
        let per_cf = router
            .memtable_usage_by_cf()
            .into_iter()
            .map(|(cf, usage)| MemtableCfStatus {
                cf: cf.name().to_string(),
                used_bytes: usage.used_bytes as u64,
                cap_bytes: usage.cap_bytes as u64,
                high_water_bytes: usage.high_water_bytes as u64,
                flush_triggered: usage.flush_triggered,
            })
            .collect::<Vec<_>>();
        let total_used_bytes = per_cf.iter().map(|cf| cf.used_bytes).sum();
        let total_cap_bytes = per_cf.iter().map(|cf| cf.cap_bytes).sum();
        MemtableStatus {
            total_used_bytes,
            total_cap_bytes,
            per_cf,
        }
    }

    /// Admission check for rows that cannot fit even in an empty memtable.
    pub fn ensure_memtable_admission<I, K, V>(&self, rows: I) -> Result<()>
    where
        I: IntoIterator<Item = (ColumnFamily, K, V)>,
        K: AsRef<[u8]>,
        V: AsRef<[u8]>,
    {
        self.router
            .read()
            .expect("mvcc router poisoned")
            .as_ref()
            .map_or(Ok(()), |router| router.ensure_batch_admitted(rows))
    }

    /// Atomically commits one write group across any number of CFs.
    pub fn commit_batch<I, K, V>(&self, rows: I) -> Result<Seq>
    where
        I: IntoIterator<Item = (ColumnFamily, K, V)>,
        K: Into<Vec<u8>>,
        V: Into<Vec<u8>>,
    {
        let rows: Vec<_> = rows
            .into_iter()
            .map(|(cf, key, value)| (cf, key.into(), value.into()))
            .collect();
        if rows.is_empty() {
            return Ok(self.current_seq());
        }

        let mut table = self.rows.write().expect("mvcc row table poisoned");
        if let Some(router) = self.router.write().expect("mvcc router poisoned").as_mut() {
            for (cf, key, value) in &rows {
                router.put(*cf, key, value)?;
            }
        }
        // Advance the derived-content watermark BEFORE allocating the seq:
        // readers pin without taking the row lock, so a reader that observes
        // this commit's seq must already observe its watermark (issue #1100).
        // All allocations happen under the row write lock held here, so the
        // next allocated seq is exactly current + 1 (asserted by the vault
        // commit path's time-index seqno prediction).
        if rows
            .iter()
            .any(|(cf, _, _)| cf.feeds_derived_search_content())
        {
            self.derived_content_seq
                .fetch_max(self.current_seq() + 1, Ordering::AcqRel);
        }
        let seq = self.seqs.allocate();
        for (cf, key, value) in rows {
            table
                .entry((cf, key))
                .or_default()
                .push(VersionedValue { seq, value });
        }
        Ok(seq)
    }

    /// Restores one durable write group at its original sequence before live writes begin.
    pub fn restore_batch<I, K, V>(&self, seq: Seq, rows: I) -> Result<()>
    where
        I: IntoIterator<Item = (ColumnFamily, K, V)>,
        K: Into<Vec<u8>>,
        V: Into<Vec<u8>>,
    {
        let rows: Vec<_> = rows
            .into_iter()
            .map(|(cf, key, value)| (cf, key.into(), value.into()))
            .collect();
        let mut table = self.rows.write().expect("mvcc row table poisoned");
        if rows
            .iter()
            .any(|(cf, _, _)| cf.feeds_derived_search_content())
        {
            self.derived_content_seq.fetch_max(seq, Ordering::AcqRel);
        }
        for (cf, key, value) in rows {
            table
                .entry((cf, key))
                .or_default()
                .push(VersionedValue { seq, value });
        }
        Ok(())
    }

    pub fn flush_all_cfs(&self) -> Result<Vec<SstSummary>> {
        self.router
            .write()
            .expect("mvcc router poisoned")
            .as_mut()
            .map_or(Ok(Vec::new()), CfRouter::flush_pending)
    }

    pub fn install_read_barrier(&self, barrier: ReadBarrier) {
        let mut barriers = self
            .read_barriers
            .write()
            .expect("mvcc read barriers poisoned");
        barriers.retain(|existing| existing.id() != barrier.id());
        barriers.push(barrier);
    }

    pub fn remove_read_barrier(&self, id: &str) -> bool {
        let mut barriers = self
            .read_barriers
            .write()
            .expect("mvcc read barriers poisoned");
        let before = barriers.len();
        barriers.retain(|existing| existing.id() != id);
        barriers.len() != before
    }

    pub fn read_barriers(&self) -> Vec<ReadBarrier> {
        self.read_barriers
            .read()
            .expect("mvcc read barriers poisoned")
            .clone()
    }
}

impl Default for VersionedCfStore {
    fn default() -> Self {
        Self::new(0)
    }
}
