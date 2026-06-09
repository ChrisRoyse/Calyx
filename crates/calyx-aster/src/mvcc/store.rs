//! In-memory MVCC row table used to define the cross-CF snapshot contract.

use crate::cf::{CfRouter, ColumnFamily};
use crate::mvcc::{Freshness, ReaderLease, SeqAllocator, Snapshot};
use crate::sst::SstSummary;
use calyx_core::{Clock, Result, Seq};
use std::collections::HashMap;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Debug, PartialEq, Eq)]
struct VersionedValue {
    seq: Seq,
    value: Vec<u8>,
}

type CfKey = (ColumnFamily, Vec<u8>);
type VersionChain = Vec<VersionedValue>;
type RowTable = HashMap<CfKey, VersionChain>;

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

/// Versioned row table with a single vault-wide sequence.
#[derive(Debug)]
pub struct VersionedCfStore {
    seqs: SeqAllocator,
    next_lease_id: AtomicU64,
    rows: RwLock<RowTable>,
    router: RwLock<Option<CfRouter>>,
}

impl VersionedCfStore {
    pub fn new(start_seq: Seq) -> Self {
        Self {
            seqs: SeqAllocator::new(start_seq),
            next_lease_id: AtomicU64::new(0),
            rows: RwLock::new(HashMap::new()),
            router: RwLock::new(None),
        }
    }

    pub fn new_with_router(start_seq: Seq, router: CfRouter) -> Self {
        Self {
            seqs: SeqAllocator::new(start_seq),
            next_lease_id: AtomicU64::new(0),
            rows: RwLock::new(HashMap::new()),
            router: RwLock::new(Some(router)),
        }
    }

    /// Latest committed sequence.
    pub fn current_seq(&self) -> Seq {
        self.seqs.current()
    }

    pub fn set_start_seq(&self, seq: Seq) -> Result<()> {
        self.seqs.set_start_seq(seq)
    }

    /// Pins a snapshot at the latest committed sequence.
    pub fn pin_snapshot(
        &self,
        freshness: Freshness,
        clock: &dyn Clock,
        max_age_ms: u64,
    ) -> Snapshot {
        let seq = self.current_seq();
        let lease_id = self.next_lease_id.fetch_add(1, Ordering::AcqRel) + 1;
        let lease = ReaderLease::new(lease_id, seq, clock.now(), max_age_ms);
        Snapshot::new(seq, freshness, lease)
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

    /// Reads one CF/key at the pinned sequence.
    pub fn read_at(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
        key: &[u8],
        clock: &dyn Clock,
    ) -> Result<Option<Vec<u8>>> {
        snapshot.ensure_live(clock)?;
        let table = self.rows.read().expect("mvcc row table poisoned");
        Ok(table
            .get(&(cf, key.to_vec()))
            .and_then(|versions| visible_value(versions, snapshot.seq())))
    }

    /// Resolves all requested CF/key rows at the same pinned sequence.
    pub fn read_batch(
        &self,
        snapshot: Snapshot,
        reads: &[CfRead],
        clock: &dyn Clock,
    ) -> Result<Vec<Option<Vec<u8>>>> {
        snapshot.ensure_live(clock)?;
        let table = self.rows.read().expect("mvcc row table poisoned");
        Ok(reads
            .iter()
            .map(|read| {
                table
                    .get(&(read.cf, read.key.clone()))
                    .and_then(|versions| visible_value(versions, snapshot.seq()))
            })
            .collect())
    }

    /// Scans visible rows for one CF at the pinned sequence, ordered by key.
    pub fn scan_cf_at(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
        clock: &dyn Clock,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        snapshot.ensure_live(clock)?;
        let table = self.rows.read().expect("mvcc row table poisoned");
        let mut rows = table
            .iter()
            .filter_map(|((row_cf, key), versions)| {
                if *row_cf != cf {
                    return None;
                }
                visible_value(versions, snapshot.seq()).map(|value| (key.clone(), value))
            })
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(rows)
    }
}

impl Default for VersionedCfStore {
    fn default() -> Self {
        Self::new(0)
    }
}

fn visible_value(versions: &[VersionedValue], seq: Seq) -> Option<Vec<u8>> {
    versions
        .iter()
        .rev()
        .find(|version| version.seq <= seq)
        .map(|version| version.value.clone())
}
